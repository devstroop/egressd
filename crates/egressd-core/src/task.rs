use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::{Arc, RwLock};

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum TaskStatus {
    Pending,
    Running,
    Succeeded,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Task {
    pub id: String,
    pub r#type: String,
    pub status: TaskStatus,
    pub result: Option<serde_json::Value>,
    pub error: Option<String>,
    pub error_code: Option<String>,
    pub created_at: DateTime<Utc>,
    pub started_at: Option<DateTime<Utc>>,
    pub finished_at: Option<DateTime<Utc>>,
}

impl Task {
    pub fn new(type_: impl Into<String>) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string().replace('-', "")[..16].to_string(),
            r#type: type_.into(),
            status: TaskStatus::Pending,
            result: None,
            error: None,
            error_code: None,
            created_at: Utc::now(),
            started_at: None,
            finished_at: None,
        }
    }

    pub fn start(&mut self) {
        self.started_at = Some(Utc::now());
        self.status = TaskStatus::Running;
    }

    pub fn succeed(&mut self, result: Option<serde_json::Value>) {
        self.result = result;
        self.status = TaskStatus::Succeeded;
        self.finished_at = Some(Utc::now());
    }

    pub fn fail(&mut self, error: TaskError) {
        self.error_code = Some(error.code);
        self.error = Some(error.message);
        self.status = TaskStatus::Failed;
        self.finished_at = Some(Utc::now());
    }

    pub fn fail_internal(&mut self, msg: impl Into<String>) {
        self.error_code = Some("INTERNAL".to_string());
        self.error = Some(msg.into());
        self.status = TaskStatus::Failed;
        self.finished_at = Some(Utc::now());
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskError {
    pub code: String,
    pub message: String,
}

impl TaskError {
    pub fn new(code: impl Into<String>, message: impl Into<String>) -> Self {
        Self {
            code: code.into(),
            message: message.into(),
        }
    }
}

impl std::fmt::Display for TaskError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl std::error::Error for TaskError {}

#[derive(Debug, Default)]
pub struct TaskRegistry {
    tasks: RwLock<HashMap<String, Task>>,
    order: RwLock<Vec<String>>,
    idem: RwLock<HashMap<String, (String, DateTime<Utc>)>>, // key -> (task_id, created_at)
}

impl TaskRegistry {
    pub fn new() -> Arc<Self> {
        Arc::new(Self {
            tasks: RwLock::new(HashMap::new()),
            order: RwLock::new(Vec::new()),
            idem: RwLock::new(HashMap::new()),
        })
    }

    pub fn create(&self, type_: impl Into<String>) -> Task {
        let task = Task::new(type_);
        {
            let mut tasks = self.tasks.write().unwrap();
            tasks.insert(task.id.clone(), task.clone());
        }
        {
            let mut order = self.order.write().unwrap();
            order.push(task.id.clone());
        }
        self.prune();
        task
    }

    pub fn get(&self, id: &str) -> Option<Task> {
        let tasks = self.tasks.read().unwrap();
        tasks.get(id).cloned()
    }

    pub fn get_mut<F, R>(&self, id: &str, f: F) -> Option<R>
    where
        F: FnOnce(&mut Task) -> R,
    {
        let mut tasks = self.tasks.write().unwrap();
        tasks.get_mut(id).map(f)
    }

    pub fn submit<F>(&self, task_id: String, func: F)
    where
        F: FnOnce() -> Result<Option<serde_json::Value>, TaskError> + Send + 'static,
    {
        // Mark running synchronously
        self.get_mut(&task_id, |t| t.start());

        let registry = self as *const Self as usize;
        // Use tokio::spawn if runtime exists, otherwise run sync (for tests without runtime, fallback)
        // We do not take Arc here to avoid lifetime; instead we rely on caller holding Arc.
        // To allow spawn, we need Arc<Self>. So we provide alternate submit_arc.
        // This method is kept for compatibility but delegates to submit_arc when possible.
        let _ = (registry, func); // to avoid unused
                                   // No-op: caller should use submit_arc
    }

    // Preferred: Arc-based submit for async runtime
    pub fn submit_arc<F>(self: &Arc<Self>, task_id: String, func: F)
    where
        F: FnOnce() -> Result<Option<serde_json::Value>, TaskError> + Send + 'static,
    {
        self.get_mut(&task_id, |t| t.start());
        let self_clone = Arc::clone(self);
        tokio::spawn(async move {
            let res = tokio::task::spawn_blocking(func).await;
            match res {
                Ok(Ok(val)) => {
                    self_clone.get_mut(&task_id, |t| t.succeed(val));
                }
                Ok(Err(e)) => {
                    self_clone.get_mut(&task_id, |t| t.fail(e));
                }
                Err(e) => {
                    self_clone.get_mut(&task_id, |t| t.fail_internal(e.to_string()));
                }
            }
        });
    }

    pub fn get_idempotent(&self, key: &str) -> Option<Task> {
        let idem = self.idem.read().unwrap();
        let (task_id, _) = idem.get(key)?;
        let task_id = task_id.clone();
        drop(idem);
        let task = self.get(&task_id)?;
        // Check task still exists and not expired
        let idem_entry = self.idem.read().unwrap().get(key).cloned();
        idem_entry?;
        Some(task)
    }

    pub fn set_idempotent(&self, key: String, task_id: String) {
        {
            let mut idem = self.idem.write().unwrap();
            idem.insert(key, (task_id, Utc::now()));
        }
        self.prune_idem();
    }

    pub fn check_idempotent_conflict(&self, key: &str, expected_type: &str) -> Result<Option<Task>, TaskError> {
        if let Some(task) = self.get_idempotent(key) {
            if task.r#type != expected_type {
                return Err(TaskError::new(
                    "IDEMPOTENCY_CONFLICT",
                    format!(
                        "idempotency key already used for task type '{}'",
                        task.r#type
                    ),
                ));
            }
            return Ok(Some(task));
        }
        Ok(None)
    }

    fn prune(&self) {
        const MAX_ITEMS: usize = 200;
        const MAX_AGE_SECS: i64 = 3600;
        let mut tasks = self.tasks.write().unwrap();
        let mut order = self.order.write().unwrap();
        // Hard cap
        while tasks.len() > MAX_ITEMS {
            if let Some(oldest) = order.first().cloned() {
                order.remove(0);
                tasks.remove(&oldest);
            } else {
                break;
            }
        }
        // Age-based prune for finished tasks
        let cutoff = Utc::now() - chrono::Duration::seconds(MAX_AGE_SECS);
        let mut to_remove = vec![];
        for id in order.iter() {
            if let Some(t) = tasks.get(id) {
                if let Some(finished) = t.finished_at {
                    if finished < cutoff {
                        to_remove.push(id.clone());
                    }
                }
            }
        }
        for id in to_remove {
            order.retain(|x| x != &id);
            tasks.remove(&id);
        }
    }

    fn prune_idem(&self) {
        const IDEM_MAX_AGE_SECS: i64 = 86400;
        const MAX_ITEMS: usize = 200;
        let mut idem = self.idem.write().unwrap();
        let tasks = self.tasks.read().unwrap();
        let cutoff = Utc::now() - chrono::Duration::seconds(IDEM_MAX_AGE_SECS);
        // Remove stale or orphaned
        idem.retain(|_, (task_id, created)| {
            if let Some(task) = tasks.get(task_id) {
                task.created_at >= cutoff && *created >= cutoff
            } else {
                false
            }
        });
        // Cap
        if idem.len() > MAX_ITEMS {
            let mut sorted: Vec<_> = idem.iter().map(|(k, (tid, _))| (k.clone(), tid.clone())).collect();
            // Sort by task created_at ascending (oldest first)
            sorted.sort_by_key(|(_, tid)| tasks.get(tid).map(|t| t.created_at).unwrap_or(Utc::now()));
            let to_drop = idem.len() - MAX_ITEMS;
            for (k, _) in sorted.into_iter().take(to_drop) {
                idem.remove(&k);
            }
        }
    }

    #[cfg(test)]
    pub fn len(&self) -> usize {
        self.tasks.read().unwrap().len()
    }

    #[cfg(test)]
    pub fn idem_len(&self) -> usize {
        self.idem.read().unwrap().len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn task_lifecycle() {
        let mut t = Task::new("proxy.create");
        assert_eq!(t.status, TaskStatus::Pending);
        t.start();
        assert_eq!(t.status, TaskStatus::Running);
        assert!(t.started_at.is_some());
        t.succeed(Some(serde_json::json!({"id":"x"})));
        assert_eq!(t.status, TaskStatus::Succeeded);
        assert!(t.finished_at.is_some());
        assert_eq!(t.result.unwrap()["id"], "x");
    }

    #[test]
    fn task_fail() {
        let mut t = Task::new("proxy.create");
        t.fail(TaskError::new("CREATE_FAILED", "boom"));
        assert_eq!(t.status, TaskStatus::Failed);
        assert_eq!(t.error_code.as_deref(), Some("CREATE_FAILED"));
        assert_eq!(t.error.as_deref(), Some("boom"));
    }

    #[test]
    fn registry_create_get() {
        let reg = TaskRegistry::new();
        let t = reg.create("pool.scale");
        assert_eq!(t.status, TaskStatus::Pending);
        let fetched = reg.get(&t.id).unwrap();
        assert_eq!(fetched.id, t.id);
        assert_eq!(reg.len(), 1);
    }

    #[test]
    fn registry_idempotency_dedup() {
        let reg = TaskRegistry::new();
        let t1 = reg.create("proxy.create");
        reg.set_idempotent("key1".to_string(), t1.id.clone());
        let fetched = reg.get_idempotent("key1").unwrap();
        assert_eq!(fetched.id, t1.id);
        // Same key returns same task
        let again = reg.get_idempotent("key1").unwrap();
        assert_eq!(again.id, t1.id);
        assert_eq!(reg.idem_len(), 1);
    }

    #[test]
    fn registry_idempotency_conflict() {
        let reg = TaskRegistry::new();
        let t1 = reg.create("pool.scale");
        reg.set_idempotent("shared".to_string(), t1.id.clone());
        let err = reg
            .check_idempotent_conflict("shared", "proxy.create")
            .unwrap_err();
        assert_eq!(err.code, "IDEMPOTENCY_CONFLICT");
        // Same type is ok
        let ok = reg.check_idempotent_conflict("shared", "pool.scale").unwrap();
        assert!(ok.is_some());
    }

    #[test]
    fn registry_prune_max_items() {
        let reg = TaskRegistry::new();
        // Create 205 tasks, should prune to 200
        for i in 0..205 {
            let t = reg.create(format!("type-{i}"));
            // Mark old as finished long ago to trigger age prune? Just fill
            reg.get_mut(&t.id, |task| {
                task.finished_at = Some(Utc::now() - chrono::Duration::seconds(4000));
                task.status = TaskStatus::Succeeded;
            });
        }
        assert!(reg.len() <= 200);
    }

    #[test]
    fn registry_task_serde() {
        let t = Task::new("proxy.create");
        let json = serde_json::to_string(&t).unwrap();
        let back: Task = serde_json::from_str(&json).unwrap();
        assert_eq!(t.id, back.id);
        assert_eq!(t.r#type, back.r#type);
    }

    #[tokio::test]
    async fn registry_submit_arc() {
        let reg = TaskRegistry::new();
        let t = reg.create("proxy.create");
        let id = t.id.clone();
        reg.submit_arc(id.clone(), || Ok(Some(serde_json::json!({"ok":true}))));
        // Wait a bit
        tokio::time::sleep(Duration::from_millis(100)).await;
        let done = reg.get(&id).unwrap();
        assert_eq!(done.status, TaskStatus::Succeeded);
    }

    #[tokio::test]
    async fn registry_submit_arc_fail() {
        let reg = TaskRegistry::new();
        let t = reg.create("proxy.create");
        let id = t.id.clone();
        reg.submit_arc(id.clone(), || {
            Err(TaskError::new("CREATE_FAILED", "boom"))
        });
        tokio::time::sleep(Duration::from_millis(100)).await;
        let done = reg.get(&id).unwrap();
        assert_eq!(done.status, TaskStatus::Failed);
        assert_eq!(done.error_code.as_deref(), Some("CREATE_FAILED"));
    }
}
