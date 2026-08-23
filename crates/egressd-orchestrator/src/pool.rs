use crate::docker::fake::FakeDocker;
use egressd_core::endpoint::Endpoint;
use egressd_core::pool::PoolSummary;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::{Mutex, RwLock};

const MAX_POOL: usize = 20;
const CREATE_RETRIES: usize = 3;

/// Desired-state pool — mirrors `proxyhub/manager/pool.py: target + ensure_count + busy guard`.
#[derive(Debug)]
pub struct Pool {
    pool: RwLock<Vec<Endpoint>>,
    target: AtomicUsize,
    ensure_lock: Mutex<()>,
}

impl Pool {
    pub fn new(target: usize) -> Self {
        Self {
            pool: RwLock::new(Vec::new()),
            target: AtomicUsize::new(target.min(MAX_POOL)),
            ensure_lock: Mutex::new(()),
        }
    }

    pub fn target(&self) -> usize {
        self.target.load(Ordering::Relaxed)
    }

    pub fn set_target(&self, n: usize) {
        self.target.store(n.min(MAX_POOL), Ordering::Relaxed);
    }

    pub async fn pool_size(&self) -> usize {
        self.pool.read().await.len()
    }

    pub async fn snapshot(&self) -> Vec<Endpoint> {
        self.pool.read().await.clone()
    }

    pub async fn summary(&self, include_proxies: bool) -> PoolSummary {
        let eps = self.snapshot().await;
        let mut s = PoolSummary::from_endpoints(self.target(), eps);
        if !include_proxies {
            s.endpoints.clear();
        }
        s
    }

    /// Register a created container into pool and bump target (proxyhub `add_proxy`).
    pub async fn add_proxy(&self, name: String, container_id: String) -> Result<Endpoint, String> {
        let _lock = self.ensure_lock.lock().await;
        let mut pool = self.pool.write().await;
        if pool.iter().any(|e| e.name == name) {
            return Err(format!("proxy {name} already exists"));
        }
        if pool.len() >= MAX_POOL {
            return Err(format!("pool at max size ({MAX_POOL})"));
        }
        let ep = Endpoint::new(name.clone(), container_id);
        pool.push(ep.clone());
        let cur = self.target.load(Ordering::Relaxed);
        self.target.store((cur + 1).min(MAX_POOL), Ordering::Relaxed);
        Ok(ep)
    }

    /// Remove from pool and lower target, cleaning volumes via FakeDocker (or real docker in prod).
    pub async fn remove_proxy(&self, docker: &FakeDocker, name: &str) -> bool {
        let _lock = self.ensure_lock.lock().await;
        let mut pool = self.pool.write().await;
        let Some(pos) = pool.iter().position(|e| e.name == name) else {
            return false;
        };
        pool.remove(pos);
        let cur = self.target.load(Ordering::Relaxed);
        self.target.store(cur.saturating_sub(1), Ordering::Relaxed);
        docker.remove(name);
        true
    }

    /// Ensure pool size == target using FakeDocker (for tests and MVP). Real docker version parallels this.
    pub async fn ensure_count(&self, docker: &FakeDocker, _image: &str, _network: &str, prefix: &str) {
        let _lock = self.ensure_lock.lock().await;
        let target = self.target.load(Ordering::Relaxed);
        let mut pool = self.pool.write().await;
        let current = pool.len();
        if current == target {
            return;
        }
        if current < target {
            let to_create = (target - current).min(MAX_POOL - current);
            for _ in 0..to_create {
                let mut created = None;
                for _ in 0..CREATE_RETRIES {
                    let suffix = format!("{:08x}", rand::random::<u32>());
                    let name = format!("{prefix}{suffix}");
                    if pool.iter().any(|e| e.name == name) {
                        continue;
                    }
                    let id = docker.create(&name, "warpgate:local", "egressd-net");
                    created = Some((name, id));
                    break;
                }
                if let Some((name, id)) = created {
                    pool.push(Endpoint::new(name, id));
                }
            }
        } else {
            let to_remove = current - target;
            // Never evict busy proxies — defer removal
            let mut removable: Vec<usize> = pool
                .iter()
                .enumerate()
                .filter(|(_, e)| !e.busy)
                .map(|(i, _)| i)
                .collect();
            removable.truncate(to_remove);
            removable.sort_unstable_by(|a, b| b.cmp(a)); // remove from end
            for idx in removable {
                let ep = pool.remove(idx);
                docker.remove(&ep.name);
            }
        }
    }

    /// One health cycle: evict gone containers (not busy, not present by name), update healthy flags.
    /// For #15, simplified: uses FakeDocker presence check.
    pub async fn health_cycle(&self, docker: &FakeDocker) {
        let snapshot = self.pool.read().await.clone();
        for ep in snapshot {
            // Simulate probe: if container gone and not busy, evict
            let present = {
                let by_name = docker.by_name.lock().unwrap();
                by_name.contains_key(&ep.name)
            };
            let is_busy = {
                let pool = self.pool.read().await;
                pool.iter().find(|e| e.name == ep.name).map(|e| e.busy).unwrap_or(false)
            };
            if !present && !is_busy {
                // Evict
                let mut pool = self.pool.write().await;
                if let Some(pos) = pool.iter().position(|e| e.name == ep.name) {
                    pool.remove(pos);
                    docker.volumes.lock().unwrap().remove(&format!("{}-data", ep.name));
                    docker.volumes.lock().unwrap().remove(&format!("{}-cache", ep.name));
                }
            }
        }
        // Replenish is done by caller via ensure_count in real loop; here we don't auto-call to keep tests deterministic
    }

    pub async fn set_busy(&self, name: &str, busy: bool) {
        let mut pool = self.pool.write().await;
        if let Some(ep) = pool.iter_mut().find(|e| e.name == name) {
            ep.busy = busy;
        }
    }

    /// Set health flags on an endpoint (used by health checker / tests).
    pub async fn set_healthy(&self, name: &str, healthy: bool) {
        let mut pool = self.pool.write().await;
        if let Some(ep) = pool.iter_mut().find(|e| e.name == name) {
            ep.healthy = healthy;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::docker::fake::FakeDocker;

    #[tokio::test]
    async fn add_bumps_target() {
        let pool = Pool::new(0);
        let ep = pool.add_proxy("egressd-abc".to_string(), "cid-abc".to_string()).await.unwrap();
        assert_eq!(ep.name, "egressd-abc");
        assert_eq!(pool.target(), 1);
        assert_eq!(pool.pool_size().await, 1);
    }

    #[tokio::test]
    async fn add_duplicate_err() {
        let pool = Pool::new(0);
        pool.add_proxy("egressd-abc".to_string(), "cid-1".to_string()).await.unwrap();
        let err = pool.add_proxy("egressd-abc".to_string(), "cid-2".to_string()).await.unwrap_err();
        assert!(err.contains("already exists"));
    }

    #[tokio::test]
    async fn remove_lowers_target_and_cleans_volumes() {
        let pool = Pool::new(0);
        let docker = FakeDocker::new();
        let id = docker.create("egressd-abc", "img", "net");
        pool.add_proxy("egressd-abc".to_string(), id).await.unwrap();
        assert!(docker.has_volume("egressd-abc-data"));
        assert!(pool.remove_proxy(&docker, "egressd-abc").await);
        assert_eq!(pool.target(), 0);
        assert_eq!(pool.pool_size().await, 0);
        assert!(!docker.has_volume("egressd-abc-data"));
    }

    #[tokio::test]
    async fn ensure_scale_up() {
        let pool = Pool::new(2);
        let docker = FakeDocker::new();
        pool.ensure_count(&docker, "img", "net", "egressd-").await;
        assert_eq!(pool.pool_size().await, 2);
        // Should be idempotent
        pool.ensure_count(&docker, "img", "net", "egressd-").await;
        assert_eq!(pool.pool_size().await, 2);
    }

    #[tokio::test]
    async fn ensure_scale_down_skips_busy() {
        let pool = Pool::new(0);
        let docker = FakeDocker::new();
        // Create 2 proxies
        for i in 0..2 {
            let name = format!("egressd-{i}");
            let id = docker.create(&name, "img", "net");
            pool.add_proxy(name, id).await.unwrap();
        }
        assert_eq!(pool.pool_size().await, 2);
        // Mark one busy
        pool.set_busy("egressd-0", true).await;
        pool.set_target(1); // want to scale down by 1
        pool.ensure_count(&docker, "img", "net", "egressd-").await;
        // Should remove non-busy one, keep busy
        let snap = pool.snapshot().await;
        assert_eq!(snap.len(), 1);
        assert_eq!(snap[0].name, "egressd-0");
        // Now free busy and scale down again
        pool.set_busy("egressd-0", false).await;
        pool.set_target(0);
        pool.ensure_count(&docker, "img", "net", "egressd-").await;
        assert_eq!(pool.pool_size().await, 0);
    }

    #[tokio::test]
    async fn health_cycle_evicts_gone() {
        let pool = Pool::new(1);
        let docker = FakeDocker::new();
        let id = docker.create("egressd-abc", "img", "net");
        pool.add_proxy("egressd-abc".to_string(), id).await.unwrap();
        // Remove container behind pool's back (simulate crash)
        docker.remove("egressd-abc");
        // Health cycle should evict
        pool.health_cycle(&docker).await;
        assert_eq!(pool.pool_size().await, 0);
    }

    #[tokio::test]
    async fn health_cycle_skips_busy() {
        let pool = Pool::new(1);
        let docker = FakeDocker::new();
        let id = docker.create("egressd-abc", "img", "net");
        pool.add_proxy("egressd-abc".to_string(), id).await.unwrap();
        pool.set_busy("egressd-abc", true).await;
        docker.remove("egressd-abc");
        pool.health_cycle(&docker).await;
        assert_eq!(pool.pool_size().await, 1); // not evicted
    }

    #[tokio::test]
    async fn health_cycle_cleans_volumes() {
        let pool = Pool::new(1);
        let docker = FakeDocker::new();
        let id = docker.create("egressd-vol", "img", "net");
        pool.add_proxy("egressd-vol".to_string(), id).await.unwrap();
        docker.remove("egressd-vol");
        // Need volume present before cycle? Fake remove already removed volume, so recreate volume to test eviction cleaning
        docker.volumes.lock().unwrap().insert("egressd-vol-data".to_string(), ());
        docker.volumes.lock().unwrap().insert("egressd-vol-cache".to_string(), ());
        pool.health_cycle(&docker).await;
        assert!(!docker.has_volume("egressd-vol-data"));
    }
}
