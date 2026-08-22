use crate::endpoint::Endpoint;

#[derive(Debug, Clone)]
pub struct PoolSummary {
    pub target: usize,
    pub pool_size: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub endpoints: Vec<Endpoint>,
}
