//! The node list served to dashboards.

use aiom_protocol::encode;
use bytes::Bytes;
use std::sync::{Mutex, PoisonError};
use std::time::{Duration, Instant};

use crate::nodes::Nodes;

/// Caches the encoded node list for a short time, so the encoding work does not grow with the
/// number of open dashboards.
#[derive(Debug)]
pub(crate) struct SnapshotCache {
    ttl: Duration,
    latest: Mutex<Option<Cached>>,
}

#[derive(Debug)]
struct Cached {
    built: Instant,
    payload: Bytes,
}

impl SnapshotCache {
    pub(crate) fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            latest: Mutex::new(None),
        }
    }

    /// Returns the gzip-compressed JSON list of all nodes in natural name order.
    pub(crate) fn get(&self, nodes: &Nodes) -> Option<Bytes> {
        // Holding the lock while building makes concurrent requests share one build.
        let mut latest = self.latest.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(cached) = latest.as_ref()
            && cached.built.elapsed() < self.ttl
        {
            return Some(cached.payload.clone());
        }
        match encode(&nodes.list()) {
            Ok(payload) => {
                let payload = Bytes::from(payload);
                *latest = Some(Cached {
                    built: Instant::now(),
                    payload: payload.clone(),
                });
                Some(payload)
            }
            Err(e) => {
                tracing::error!("failed to encode node list: {e}");
                None
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiom_protocol::{NodeSnapshot, decode};
    use std::sync::Arc;

    fn report(nodes: &Arc<Nodes>, name: &str) {
        let claim = nodes.claim(name).unwrap();
        claim.update(1, Default::default(), Default::default(), None);
    }

    fn names(cache: &SnapshotCache, nodes: &Nodes) -> Vec<String> {
        let payload = cache.get(nodes).unwrap();
        decode::<Vec<NodeSnapshot>>(&payload, 1 << 20)
            .unwrap()
            .into_iter()
            .map(|n| n.name)
            .collect()
    }

    #[test]
    fn encodes_nodes_in_natural_order() {
        let nodes = Arc::new(Nodes::default());
        for name in ["node-10", "node-2", "node-1"] {
            report(&nodes, name);
        }
        let cache = SnapshotCache::new(Duration::ZERO);
        assert_eq!(names(&cache, &nodes), ["node-1", "node-2", "node-10"]);
    }

    #[test]
    fn no_nodes_give_an_empty_list() {
        let cache = SnapshotCache::new(Duration::ZERO);
        assert!(names(&cache, &Nodes::default()).is_empty());
    }

    #[test]
    fn reuses_list_within_ttl() {
        let nodes = Arc::new(Nodes::default());
        report(&nodes, "a");
        let cache = SnapshotCache::new(Duration::from_secs(3600));
        assert_eq!(names(&cache, &nodes), ["a"]);

        report(&nodes, "b");
        assert_eq!(names(&cache, &nodes), ["a"]);
        assert_eq!(
            names(&SnapshotCache::new(Duration::ZERO), &nodes),
            ["a", "b"]
        );
    }
}
