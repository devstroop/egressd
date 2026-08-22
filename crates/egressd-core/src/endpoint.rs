use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Proxy endpoint — control-plane view of one proxy container.
/// Mirrors `proxyhub/manager/warpgate_manager/pool.py: ProxyEndpoint` but
/// vendor-agnostic: `tunnel_*` not `warp_*` until provider trait scopes it.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct Endpoint {
    pub name: String,
    pub container_id: String,
    pub healthy: bool,
    pub socks5_ok: bool,
    pub http_ok: bool,
    pub tunnel_connected: bool,
    pub tunnel_detail: String,
    pub created_at: DateTime<Utc>,
    /// `true` while a `recreate`/`restart` is in-flight — health checker must not evict.
    pub busy: bool,
}

impl Endpoint {
    pub fn new(name: impl Into<String>, container_id: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            container_id: container_id.into(),
            healthy: false,
            socks5_ok: false,
            http_ok: false,
            tunnel_connected: false,
            tunnel_detail: "unknown".to_string(),
            created_at: Utc::now(),
            busy: false,
        }
    }

    pub fn socks5_url(&self) -> String {
        format!("socks5://{}:1080", self.name)
    }

    pub fn http_url(&self) -> String {
        format!("http://{}:3128", self.name)
    }

    pub fn uptime_secs(&self) -> f64 {
        (Utc::now() - self.created_at).num_milliseconds() as f64 / 1000.0
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn endpoint_urls() {
        let ep = Endpoint::new("egressd-abc123", "cid123");
        assert_eq!(ep.socks5_url(), "socks5://egressd-abc123:1080");
        assert_eq!(ep.http_url(), "http://egressd-abc123:3128");
    }

    #[test]
    fn endpoint_serde_roundtrip() {
        let ep = Endpoint::new("egressd-xyz", "cid-xyz");
        let json = serde_json::to_string(&ep).unwrap();
        let back: Endpoint = serde_json::from_str(&json).unwrap();
        assert_eq!(ep, back);
    }

    #[test]
    fn endpoint_new_defaults_unhealthy() {
        let ep = Endpoint::new("egressd-1", "cid-1");
        assert!(!ep.healthy);
        assert!(!ep.socks5_ok);
        assert!(!ep.tunnel_connected);
        assert_eq!(ep.tunnel_detail, "unknown");
        assert!(!ep.busy);
    }
}
