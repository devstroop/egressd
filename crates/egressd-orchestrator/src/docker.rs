use anyhow::{Context, Result};
use bollard::models::ContainerSummary;
use bollard::Docker;
use std::collections::HashMap;

/// Vendor-agnostic Docker client — thin wrapper over bollard.
/// Control-plane only; data-plane traffic never goes through this client.
#[derive(Clone, Debug)]
pub struct DockerClient {
    docker: Docker,
}

impl DockerClient {
    /// Connect via socket defaults and verify with `ping`. Returns `None` on failure
    /// so callers can run in degraded mode (no Docker) without hard fail.
    pub async fn connect() -> Option<Self> {
        match Docker::connect_with_socket_defaults() {
            Ok(docker) => match docker.ping().await {
                Ok(_) => Some(Self { docker }),
                Err(e) => {
                    tracing::warn!(error = %e, "docker ping failed, pool mode degraded");
                    None
                }
            },
            Err(e) => {
                tracing::warn!(error = %e, "docker socket unavailable, pool mode degraded");
                None
            }
        }
    }

    /// For tests — connect without ping (no socket needed).
    #[cfg(test)]
    pub fn new_fake() -> Self {
        // Use a dummy Docker that won't be used; tests use FakeDockerClient via trait
        // This is only to satisfy type; real tests use FakeDockerClient directly.
        // So we create a Docker with non-existent socket but never call it.
        let docker = Docker::connect_with_socket_defaults().unwrap_or_else(|_| {
            Docker::connect_with_local("unix:///tmp/fake-docker.sock", 120, bollard::API_DEFAULT_VERSION).unwrap()
        });
        Self { docker }
    }

    /// List running containers whose name starts with `prefix`.
    pub async fn list_by_prefix(&self, prefix: &str) -> Result<Vec<ContainerSummary>> {
        self.list_prefix(prefix, false).await
    }

    /// List all containers (including Exit/Created) matching prefix — for stale pruning.
    pub async fn list_all_by_prefix(&self, prefix: &str) -> Result<Vec<ContainerSummary>> {
        self.list_prefix(prefix, true).await
    }

    async fn list_prefix(&self, prefix: &str, all: bool) -> Result<Vec<ContainerSummary>> {
        use bollard::container::ListContainersOptions;
        let mut filters: HashMap<String, Vec<String>> = HashMap::new();
        if !all {
            filters.insert("status".to_string(), vec!["running".to_string()]);
        }
        let options = ListContainersOptions {
            all,
            filters,
            ..Default::default()
        };
        let containers = self.docker.list_containers(Some(options)).await.context("list containers")?;
        Ok(containers
            .into_iter()
            .filter(|c| {
                c.names.as_ref().is_some_and(|names| {
                    names.iter().any(|n| {
                        let name = n.trim_start_matches('/');
                        name.starts_with(prefix)
                    })
                })
            })
            .collect())
    }

    /// Create and start a proxy container with per-proxy volumes preserving `reg.json`.
    /// Mirrors `proxyhub/manager/pool.py: create_proxy_container` — vendor-agnostic
    /// but MVP image is `warpgate:local`.
    pub async fn create_proxy_container(&self, name: &str, image: &str, network: &str) -> Result<String> {
        use bollard::container::{Config, CreateContainerOptions};
        use bollard::models::{DeviceMapping, HostConfig, Mount, MountTypeEnum};

        let mounts = vec![
            Mount {
                target: Some("/var/lib/cloudflare-warp".to_string()),
                source: Some(format!("{name}-data")),
                typ: Some(MountTypeEnum::VOLUME),
                ..Default::default()
            },
            Mount {
                target: Some("/var/cache".to_string()),
                source: Some(format!("{name}-cache")),
                typ: Some(MountTypeEnum::VOLUME),
                ..Default::default()
            },
        ];

        let mut sysctls = HashMap::new();
        sysctls.insert("net.ipv6.conf.all.disable_ipv6".to_string(), "0".to_string());

        let host_config = HostConfig {
            network_mode: Some(network.to_string()),
            cap_add: Some(vec!["NET_ADMIN".to_string()]),
            devices: Some(vec![DeviceMapping {
                path_on_host: Some("/dev/net/tun".to_string()),
                path_in_container: Some("/dev/net/tun".to_string()),
                cgroup_permissions: Some("rwm".to_string()),
            }]),
            mounts: Some(mounts),
            sysctls: Some(sysctls),
            auto_remove: Some(true),
            ..Default::default()
        };

        // bollard HostConfig doesn't expose mem_limit directly in some versions; use container Config resources
        let config = Config {
            image: Some(image.to_string()),
            host_config: Some(host_config),
            env: Some(vec![
                "WARP_WAIT_RETRIES=15".to_string(),
                "WARP_WAIT_INTERVAL=2".to_string(),
            ]),
            ..Default::default()
        };

        let options = CreateContainerOptions {
            name,
            platform: None,
        };

        let resp = self
            .docker
            .create_container(Some(options), config)
            .await
            .with_context(|| format!("create container {name}"))?;
        self.docker
            .start_container(name, None::<bollard::container::StartContainerOptions<String>>)
            .await
            .with_context(|| format!("start container {name}"))?;
        tracing::info!(name, id = %resp.id, "created proxy container");
        Ok(resp.id)
    }

    /// Stop (timeout 10s) and remove container by name or id. Handles `auto_remove` race (NotFound after stop).
    pub async fn remove_container(&self, name: &str) -> Result<()> {
        use bollard::container::{RemoveContainerOptions, StopContainerOptions};
        let _ = self
            .docker
            .stop_container(name, Some(StopContainerOptions { t: 10 }))
            .await;
        let opts = RemoveContainerOptions {
            force: true,
            ..Default::default()
        };
        match self.docker.remove_container(name, Some(opts)).await {
            Ok(()) => {
                tracing::info!(name, "removed proxy container");
                Ok(())
            }
            Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => {
                tracing::debug!(name, "container already removed by auto_remove");
                Ok(())
            }
            Err(e) => Err(e).with_context(|| format!("remove container {name}")),
        }
    }

    /// Remove per-proxy volumes `{name}-data` and `{name}-cache`.
    pub async fn cleanup_volumes(&self, name: &str) -> Result<()> {
        for vol in [format!("{name}-data"), format!("{name}-cache")] {
            match self.docker.remove_volume(&vol, None).await {
                Ok(()) => tracing::info!(volume = %vol, "removed volume"),
                Err(bollard::errors::Error::DockerResponseServerError { status_code: 404, .. }) => {}
                Err(e) => tracing::warn!(volume = %vol, error = %e, "remove volume failed"),
            }
        }
        Ok(())
    }

    /// Exec `warp-cli --accept-tos status` and return status line or None.
    pub async fn warp_status(&self, container_id: &str) -> Option<String> {
        use bollard::exec::{CreateExecOptions, StartExecResults};
        use futures_util::StreamExt;
        let exec = self
            .docker
            .create_exec(
                container_id,
                CreateExecOptions {
                    cmd: Some(vec![
                        "warp-cli".to_string(),
                        "--accept-tos".to_string(),
                        "status".to_string(),
                    ]),
                    attach_stdout: Some(true),
                    attach_stderr: Some(true),
                    ..Default::default()
                },
            )
            .await
            .ok()?;
        let start = self.docker.start_exec(&exec.id, None).await.ok()?;
        match start {
            StartExecResults::Attached { mut output, .. } => {
                let mut buf = Vec::new();
                while let Some(Ok(chunk)) = output.next().await {
                    match chunk {
                        bollard::container::LogOutput::StdOut { message }
                        | bollard::container::LogOutput::StdErr { message } => {
                            buf.extend_from_slice(&message)
                        }
                        _ => {}
                    }
                }
                let out = String::from_utf8_lossy(&buf);
                for line in out.lines() {
                    if line.contains("Status update:") {
                        return Some(line.split_once(':')?.1.trim().to_string());
                    }
                }
                None
            }
            StartExecResults::Detached => None,
        }
    }

    pub fn inner(&self) -> &Docker {
        &self.docker
    }
}

// ── Fake for tests (no Docker socket) ─────────────────────────────────────

#[cfg(test)]
pub mod fake {
    use std::collections::HashMap;
    use std::sync::{Arc, Mutex};

    #[derive(Debug, Clone)]
    pub struct FakeContainer {
        pub name: String,
        pub id: String,
        pub running: bool,
        pub mounts: Option<Vec<bollard::models::Mount>>,
    }

    #[derive(Debug, Default, Clone)]
    pub struct FakeDocker {
        pub containers: Arc<Mutex<HashMap<String, FakeContainer>>>, // id -> container
        pub by_name: Arc<Mutex<HashMap<String, String>>>,          // name -> id
        pub volumes: Arc<Mutex<HashMap<String, ()>>>,
    }

    impl FakeDocker {
        pub fn new() -> Self {
            Self::default()
        }

        pub fn create(&self, name: &str, _image: &str, _network: &str) -> String {
            let id = format!("cid-{}", name);
            let c = FakeContainer {
                name: name.to_string(),
                id: id.clone(),
                running: true,
                mounts: None,
            };
            self.containers.lock().unwrap().insert(id.clone(), c);
            self.by_name.lock().unwrap().insert(name.to_string(), id.clone());
            self.volumes.lock().unwrap().insert(format!("{name}-data"), ());
            self.volumes.lock().unwrap().insert(format!("{name}-cache"), ());
            id
        }

        pub fn list_by_prefix(&self, prefix: &str, running_only: bool) -> Vec<FakeContainer> {
            let map = self.containers.lock().unwrap();
            map.values()
                .filter(|c| c.name.starts_with(prefix) && (!running_only || c.running))
                .cloned()
                .collect()
        }

        pub fn remove(&self, name: &str) -> bool {
            let mut by_name = self.by_name.lock().unwrap();
            let mut containers = self.containers.lock().unwrap();
            if let Some(id) = by_name.remove(name) {
                containers.remove(&id);
                self.volumes.lock().unwrap().remove(&format!("{name}-data"));
                self.volumes.lock().unwrap().remove(&format!("{name}-cache"));
                return true;
            }
            // also try id
            if containers.remove(name).is_some() {
                by_name.retain(|_, v| v != name);
                return true;
            }
            false
        }

        pub fn has_volume(&self, name: &str) -> bool {
            self.volumes.lock().unwrap().contains_key(name)
        }
    }

    #[test]
    fn fake_create_list_remove() {
        let fake = FakeDocker::new();
        let id = fake.create("egressd-abc", "warpgate:local", "egressd-net");
        assert!(id.starts_with("cid-"));
        let list = fake.list_by_prefix("egressd-", true);
        assert_eq!(list.len(), 1);
        assert_eq!(list[0].name, "egressd-abc");
        assert!(fake.has_volume("egressd-abc-data"));
        assert!(fake.remove("egressd-abc"));
        assert_eq!(fake.list_by_prefix("egressd-", true).len(), 0);
        assert!(!fake.has_volume("egressd-abc-data"));
    }

    #[test]
    fn fake_prefix_filter() {
        let fake = FakeDocker::new();
        fake.create("egressd-1", "img", "net");
        fake.create("warpgate-1", "img", "net");
        assert_eq!(fake.list_by_prefix("egressd-", true).len(), 1);
        assert_eq!(fake.list_by_prefix("warpgate-", true).len(), 1);
    }
}
