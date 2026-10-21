//! The latest state of every node, kept in memory.

use aiom_protocol::{Host, NodeSnapshot, Stats};
use std::collections::hash_map::Entry;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

/// The latest report of every node that reported since the server started, and the names
/// clients are reporting as right now.
#[derive(Debug, Default)]
pub(crate) struct Nodes {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    nodes: HashMap<String, NodeSnapshot>,
    /// Names held by open client connections.
    claimed: HashSet<String>,
}

impl Nodes {
    fn lock(&self) -> MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Claims `name` for one client connection, or returns `None` while another connection
    /// holds it. The name is released when the claim is dropped.
    pub(crate) fn claim(self: &Arc<Self>, name: &str) -> Option<Claim> {
        self.lock().claimed.insert(name.to_owned()).then(|| Claim {
            nodes: Arc::clone(self),
            name: name.to_owned(),
        })
    }

    /// All nodes in natural name order.
    pub(crate) fn list(&self) -> Vec<NodeSnapshot> {
        let mut nodes: Vec<_> = self.lock().nodes.values().cloned().collect();
        nodes.sort_by(|a, b| natord::compare(&a.name, &b.name));
        nodes
    }
}

/// The right to report as one node, held by the connection of its client.
#[derive(Debug)]
pub(crate) struct Claim {
    nodes: Arc<Nodes>,
    name: String,
}

impl Claim {
    pub(crate) fn name(&self) -> &str {
        &self.name
    }

    /// Stores the latest report of the node. A `geoip` of `None` keeps the country known
    /// before.
    pub(crate) fn update(&self, timestamp: i64, host: Host, stats: Stats, geoip: Option<String>) {
        match self.nodes.lock().nodes.entry(self.name.clone()) {
            Entry::Occupied(mut entry) => {
                let node = entry.get_mut();
                node.timestamp = timestamp;
                node.host = host;
                node.stats = stats;
                if let Some(geoip) = geoip {
                    node.geoip = geoip;
                }
            }
            Entry::Vacant(entry) => {
                entry.insert(NodeSnapshot {
                    timestamp,
                    name: self.name.clone(),
                    host,
                    stats,
                    geoip: geoip.unwrap_or_default(),
                });
            }
        }
    }
}

impl Drop for Claim {
    fn drop(&mut self) {
        self.nodes.lock().claimed.remove(&self.name);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stats(cpu_usage: f64) -> Stats {
        Stats {
            cpu_usage,
            ..Default::default()
        }
    }

    #[test]
    fn keeps_latest_report_per_node_in_natural_order() {
        let nodes = Arc::new(Nodes::default());
        for name in ["node-10", "node-2", "alpha", "node-1"] {
            let claim = nodes.claim(name).unwrap();
            claim.update(1, Host::default(), stats(1.0), None);
        }
        let claim = nodes.claim("node-2").unwrap();
        claim.update(2, Host::default(), stats(2.0), None);

        let list = nodes.list();
        let names: Vec<_> = list.iter().map(|n| n.name.as_str()).collect();
        assert_eq!(names, ["alpha", "node-1", "node-2", "node-10"]);
        assert_eq!(list[2].timestamp, 2);
        assert_eq!(list[2].stats.cpu_usage, 2.0);
    }

    #[test]
    fn a_name_has_one_holder_at_a_time() {
        let nodes = Arc::new(Nodes::default());
        let first = nodes.claim("web").unwrap();
        assert!(nodes.claim("web").is_none());
        assert!(nodes.claim("db").is_some());

        drop(first);
        assert!(nodes.claim("web").is_some());
    }

    #[test]
    fn nodes_outlive_their_connection() {
        let nodes = Arc::new(Nodes::default());
        nodes
            .claim("web")
            .unwrap()
            .update(1, Host::default(), stats(1.0), None);
        assert_eq!(nodes.list().len(), 1);
    }

    #[test]
    fn missing_country_keeps_the_known_one() {
        let nodes = Arc::new(Nodes::default());
        let claim = nodes.claim("web").unwrap();
        claim.update(1, Host::default(), stats(1.0), None);
        assert_eq!(nodes.list()[0].geoip, "");

        claim.update(2, Host::default(), stats(1.0), Some("NL".into()));
        claim.update(3, Host::default(), stats(1.0), None);
        assert_eq!(nodes.list()[0].geoip, "NL");

        claim.update(4, Host::default(), stats(1.0), Some(String::new()));
        assert_eq!(nodes.list()[0].geoip, "");
    }
}
