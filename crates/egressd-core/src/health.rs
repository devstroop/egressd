#[derive(Debug, Clone)]
pub struct HealthResult {
    pub running: bool,
    pub socks5: bool,
    pub http: bool,
    pub tunnel_connected: bool,
    pub tunnel_detail: String,
}

impl HealthResult {
    pub fn healthy(&self) -> bool {
        self.running && self.socks5 && self.http && self.tunnel_connected
    }
}
