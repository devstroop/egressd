use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Endpoint {
    pub name: String,
    pub container_id: String,
    pub healthy: bool,
    pub socks5_ok: bool,
    pub http_ok: bool,
    pub tunnel_connected: bool,
    pub tunnel_detail: String,
    pub created_at: DateTime<Utc>,
    pub busy: bool,
}
