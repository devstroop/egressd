use chrono::{DateTime, Utc};
use egressd_core::{
    endpoint::Endpoint,
    pool::PoolSummary,
    task::Task,
};
use regex::Regex;
use serde_json::{json, Value};
use std::sync::OnceLock;

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct ValidationError(pub String);

fn iso8601(dt: DateTime<Utc>) -> String {
    dt.format("%Y-%m-%dT%H:%M:%SZ").to_string()
}

pub fn to_proxy_dict(ep: &Endpoint) -> Value {
    json!({
        "id": ep.name,
        "container_id": ep.container_id,
        "endpoints": {
            "socks5": ep.socks5_url(),
            "http": ep.http_url(),
        },
        "status": {
            "healthy": ep.healthy,
            "socks5": ep.socks5_ok,
            "http": ep.http_ok,
            "warp": {
                "connected": ep.tunnel_connected,
                "detail": ep.tunnel_detail,
            }
        },
        "created_at": iso8601(ep.created_at),
        "uptime_s": (Utc::now() - ep.created_at).num_milliseconds() as f64 / 1000.0,
    })
}

pub fn to_task_dict(task: &Task) -> Value {
    json!({
        "id": task.id,
        "type": task.r#type,
        "status": match task.status {
            egressd_core::task::TaskStatus::Pending => "pending",
            egressd_core::task::TaskStatus::Running => "running",
            egressd_core::task::TaskStatus::Succeeded => "succeeded",
            egressd_core::task::TaskStatus::Failed => "failed",
        },
        "result": task.result,
        "error": task.error,
        "error_code": task.error_code,
        "created_at": iso8601(task.created_at),
        "started_at": task.started_at.map(iso8601),
        "finished_at": task.finished_at.map(iso8601),
    })
}

pub fn to_pool_dict(summary: &PoolSummary, include_proxies: bool) -> Value {
    let mut v = json!({
        "target": summary.target,
        "pool_size": summary.pool_size,
        "healthy": summary.healthy,
        "degraded": summary.degraded,
    });
    if include_proxies {
        v["proxies"] = Value::Array(
            summary
                .endpoints
                .iter()
                .map(to_proxy_dict)
                .collect(),
        );
    }
    v
}

// ── Validation ──────────────────────────────────────────────────────────

pub fn validate_scale_target(body: Option<&Value>) -> Result<usize, ValidationError> {
    let body = body.ok_or_else(|| ValidationError("request body must be a JSON object".to_string()))?;
    let obj = body
        .as_object()
        .ok_or_else(|| ValidationError("request body must be a JSON object".to_string()))?;
    let target = obj
        .get("target")
        .ok_or_else(|| ValidationError("target must be an integer".to_string()))?;
    // Reject bool (serde_json Value::Bool is not Number, but 0/1 via bool check)
    if target.is_boolean() {
        return Err(ValidationError("target must be an integer".to_string()));
    }
    let n = target
        .as_u64()
        .ok_or_else(|| ValidationError("target must be an integer".to_string()))?;
    // Also reject negative via i64? as_u64 already rejects negative floats
    // Check that the raw is integer not float
    if target.as_i64().is_none() && target.as_u64().is_none() {
        return Err(ValidationError("target must be an integer".to_string()));
    }
    const MAX_POOL: usize = 20; // mirrors config MANAGER_MAX_POOL
    if n as usize > MAX_POOL {
        return Err(ValidationError(format!(
            "target exceeds max pool size ({MAX_POOL})"
        )));
    }
    // target is usize, already >=0
    Ok(n as usize)
}

pub fn validate_rotate_scope(body: Option<&Value>) -> Result<String, ValidationError> {
    let scope = if let Some(b) = body {
        if b.is_null() {
            "unhealthy".to_string()
        } else {
            let obj = b
                .as_object()
                .ok_or_else(|| ValidationError("request body must be a JSON object".to_string()))?;
            let s = obj.get("scope").map(|v| v.as_str().unwrap_or("")).unwrap_or("unhealthy");
            s.to_string()
        }
    } else {
        "unhealthy".to_string()
    };
    if scope != "unhealthy" && scope != "all" {
        return Err(ValidationError(
            "scope must be 'all' or 'unhealthy'".to_string(),
        ));
    }
    Ok(scope)
}

static PROXY_NAME_RE: OnceLock<Regex> = OnceLock::new();
const PROXY_NAME_MAX: usize = 64;

fn proxy_name_regex() -> &'static Regex {
    PROXY_NAME_RE.get_or_init(|| Regex::new(r"^[a-zA-Z0-9][a-zA-Z0-9_.-]*$").unwrap())
}

pub fn validate_proxy_name(body: Option<&Value>) -> Result<Option<String>, ValidationError> {
    let Some(b) = body else {
        return Ok(None);
    };
    if b.is_null() {
        return Ok(None);
    }
    let obj = b
        .as_object()
        .ok_or_else(|| ValidationError("request body must be a JSON object".to_string()))?;
    let Some(name_val) = obj.get("name") else {
        return Ok(None);
    };
    if name_val.is_null() {
        return Ok(None);
    }
    let name = name_val
        .as_str()
        .ok_or_else(|| ValidationError("name must be a string".to_string()))?;
    if name.is_empty() || name.len() > PROXY_NAME_MAX {
        return Err(ValidationError(format!(
            "name must be 1-{PROXY_NAME_MAX} characters"
        )));
    }
    if !proxy_name_regex().is_match(name) {
        return Err(ValidationError(
            "name must match a docker container name ([a-zA-Z0-9][a-zA-Z0-9_.-]*)".to_string(),
        ));
    }
    Ok(Some(name.to_string()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    // ── proxy dict ──────────────────────────────────────────────────────

    #[test]
    fn proxy_dict_shape() {
        let mut ep = Endpoint::new("egressd-abc", "cid123");
        ep.healthy = true;
        ep.socks5_ok = true;
        ep.http_ok = true;
        ep.tunnel_connected = true;
        ep.tunnel_detail = "Connected".to_string();
        let v = to_proxy_dict(&ep);
        assert_eq!(v["id"], "egressd-abc");
        assert_eq!(v["endpoints"]["socks5"], "socks5://egressd-abc:1080");
        assert_eq!(v["status"]["healthy"], true);
        assert!(v["created_at"].is_string());
        assert!(v["uptime_s"].is_number());
    }

    #[test]
    fn pool_dict_include() {
        let ep1 = {
            let mut e = Endpoint::new("a", "cid-a");
            e.healthy = true;
            e
        };
        let ep2 = Endpoint::new("b", "cid-b");
        let summary = PoolSummary::from_endpoints(2, vec![ep1, ep2]);
        let v = to_pool_dict(&summary, true);
        assert_eq!(v["target"], 2);
        assert_eq!(v["pool_size"], 2);
        assert_eq!(v["healthy"], 1);
        assert_eq!(v["degraded"], 1);
        assert_eq!(v["proxies"].as_array().unwrap().len(), 2);

        let v2 = to_pool_dict(&summary, false);
        assert!(v2.get("proxies").is_none());
    }

    // ── scale target ────────────────────────────────────────────────────

    #[test]
    fn scale_valid() {
        assert_eq!(validate_scale_target(Some(&json!({"target":2}))).unwrap(), 2);
        assert_eq!(validate_scale_target(Some(&json!({"target":0}))).unwrap(), 0);
    }

    #[test]
    fn scale_invalid() {
        assert!(validate_scale_target(None).is_err());
        assert!(validate_scale_target(Some(&json!({"target":"many"}))).is_err());
        assert!(validate_scale_target(Some(&json!({"target": -1}))).is_err());
        assert!(validate_scale_target(Some(&json!({"target": 1.5}))).is_err());
        assert!(validate_scale_target(Some(&json!({"target": 100}))).is_err()); // exceeds 20
        assert!(validate_scale_target(Some(&json!({"target": true}))).is_err());
        assert_eq!(
            validate_scale_target(Some(&json!({"target":"many"})))
                .unwrap_err()
                .0,
            "target must be an integer"
        );
    }

    // ── rotate scope ────────────────────────────────────────────────────

    #[test]
    fn rotate_scope_valid() {
        assert_eq!(validate_rotate_scope(None).unwrap(), "unhealthy");
        assert_eq!(
            validate_rotate_scope(Some(&json!({"scope":"all"}))).unwrap(),
            "all"
        );
        assert_eq!(
            validate_rotate_scope(Some(&json!({"scope":"unhealthy"}))).unwrap(),
            "unhealthy"
        );
        assert_eq!(validate_rotate_scope(Some(&json!({}))).unwrap(), "unhealthy");
    }

    #[test]
    fn rotate_scope_invalid() {
        assert!(validate_rotate_scope(Some(&json!({"scope":"everything"}))).is_err());
        assert!(validate_rotate_scope(Some(&json!("not-an-object"))).is_err());
        assert_eq!(
            validate_rotate_scope(Some(&json!({"scope":"everything"})))
                .unwrap_err()
                .0,
            "scope must be 'all' or 'unhealthy'"
        );
    }

    // ── proxy name ──────────────────────────────────────────────────────

    #[test]
    fn proxy_name_valid() {
        assert_eq!(
            validate_proxy_name(Some(&json!({"name":"egressd-custom"}))).unwrap(),
            Some("egressd-custom".to_string())
        );
        assert_eq!(validate_proxy_name(None).unwrap(), None);
        assert_eq!(validate_proxy_name(Some(&json!({}))).unwrap(), None);
        assert_eq!(validate_proxy_name(Some(&json!(null))).unwrap(), None);
    }

    #[test]
    fn proxy_name_invalid() {
        assert!(validate_proxy_name(Some(&json!({"name":"has space"}))).is_err());
        assert!(validate_proxy_name(Some(&json!({"name":12}))).is_err());
        assert!(validate_proxy_name(Some(&json!({"name":"x".repeat(65)}))).is_err());
        assert!(validate_proxy_name(Some(&json!("not-an-object"))).is_err());
        assert!(validate_proxy_name(Some(&json!({"name":""}))).is_err());
        assert!(validate_proxy_name(Some(&json!({"name":"-bad"}))).is_err());
        let err = validate_proxy_name(Some(&json!({"name":"has space"}))).unwrap_err();
        assert_eq!(
            err.0,
            "name must match a docker container name ([a-zA-Z0-9][a-zA-Z0-9_.-]*)"
        );
    }
}
