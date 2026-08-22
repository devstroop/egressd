use crate::endpoint::Endpoint;

/// Pool summary — control-plane view of desired vs actual pool.
/// `degraded = pool_size - healthy`, `healthy` counts `Endpoint.healthy`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PoolSummary {
    pub target: usize,
    pub pool_size: usize,
    pub healthy: usize,
    pub degraded: usize,
    pub endpoints: Vec<Endpoint>,
}

impl PoolSummary {
    pub fn from_endpoints(target: usize, endpoints: Vec<Endpoint>) -> Self {
        let healthy = endpoints.iter().filter(|e| e.healthy).count();
        let pool_size = endpoints.len();
        Self {
            target,
            pool_size,
            healthy,
            degraded: pool_size.saturating_sub(healthy),
            endpoints,
        }
    }

    pub fn status(&self) -> &'static str {
        if self.pool_size == 0 {
            "initializing"
        } else if self.degraded == 0 {
            "ok"
        } else {
            "degraded"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::endpoint::Endpoint;

    fn ep(name: &str, healthy: bool) -> Endpoint {
        let mut e = Endpoint::new(name, format!("cid-{name}"));
        e.healthy = healthy;
        e
    }

    #[test]
    fn summary_counts() {
        let eps = vec![ep("a", true), ep("b", true), ep("c", false)];
        let s = PoolSummary::from_endpoints(5, eps.clone());
        assert_eq!(s.pool_size, 3);
        assert_eq!(s.healthy, 2);
        assert_eq!(s.degraded, 1);
        assert_eq!(s.target, 5);
        assert_eq!(s.endpoints, eps);
    }

    #[test]
    fn summary_status() {
        assert_eq!(
            PoolSummary::from_endpoints(3, vec![]).status(),
            "initializing"
        );
        assert_eq!(
            PoolSummary::from_endpoints(2, vec![ep("a", true), ep("b", true)]).status(),
            "ok"
        );
        assert_eq!(
            PoolSummary::from_endpoints(2, vec![ep("a", false), ep("b", false)]).status(),
            "degraded"
        );
        assert_eq!(
            PoolSummary::from_endpoints(2, vec![ep("a", true), ep("b", false)]).status(),
            "degraded"
        );
    }

    #[test]
    fn degraded_saturating() {
        let s = PoolSummary {
            target: 0,
            pool_size: 0,
            healthy: 0,
            degraded: 0,
            endpoints: vec![],
        };
        assert_eq!(s.degraded, 0);
    }
}
