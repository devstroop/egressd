/// Triple health probe result — running + SOCKS5 handshake + HTTP CONNECT + tunnel status.
/// Healthy iff all four are true (proxyhub parity: `HealthResult.healthy`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HealthResult {
    pub running: bool,
    pub socks5: bool,
    pub http: bool,
    pub tunnel_connected: bool,
    pub tunnel_detail: String,
}

impl HealthResult {
    pub fn new(
        running: bool,
        socks5: bool,
        http: bool,
        tunnel_connected: bool,
        tunnel_detail: impl Into<String>,
    ) -> Self {
        Self {
            running,
            socks5,
            http,
            tunnel_connected,
            tunnel_detail: tunnel_detail.into(),
        }
    }

    pub fn healthy(&self) -> bool {
        self.running && self.socks5 && self.http && self.tunnel_connected
    }

    pub fn unknown() -> Self {
        Self {
            running: true,
            socks5: false,
            http: false,
            tunnel_connected: false,
            tunnel_detail: "unknown".to_string(),
        }
    }
}

impl Default for HealthResult {
    fn default() -> Self {
        Self::unknown()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn healthy_when_all_true() {
        let h = HealthResult::new(true, true, true, true, "Connected");
        assert!(h.healthy());
    }

    #[test]
    fn unhealthy_when_any_false() {
        assert!(!HealthResult::new(false, true, true, true, "ok").healthy());
        assert!(!HealthResult::new(true, false, true, true, "ok").healthy());
        assert!(!HealthResult::new(true, true, false, true, "ok").healthy());
        assert!(!HealthResult::new(true, true, true, false, "unknown").healthy());
    }

    #[test]
    fn unknown_is_unhealthy() {
        assert!(!HealthResult::unknown().healthy());
        assert_eq!(HealthResult::unknown().tunnel_detail, "unknown");
    }

    #[test]
    fn default_is_unknown() {
        assert_eq!(HealthResult::default(), HealthResult::unknown());
    }
}
