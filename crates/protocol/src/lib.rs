//! Wire format shared by the AIO Monitor server and client.
//!
//! Two WebSocket channels exist:
//!
//! * **Agent channel** – each client sends an [`AgentReport`] as a gzip-compressed JSON binary
//!   frame on every tick.
//! * **Dashboard channel** – the web UI sends the text frame [`DASHBOARD_PING`] and the server
//!   answers with a gzip-compressed JSON array of [`NodeSnapshot`]s.
//!
//! Every message is encoded with [`encode`] and decoded with [`decode`].

mod codec;

pub use codec::{CodecError, decode, encode};

use serde::{Deserialize, Serialize};
use std::net::IpAddr;
use std::time::{SystemTime, UNIX_EPOCH};

/// Version of [`AgentReport`]; the server rejects any other value.
pub const PROTOCOL_VERSION: u64 = 1;

/// Text frame the dashboard sends to request a fresh list of [`NodeSnapshot`]s.
pub const DASHBOARD_PING: &str = "ping";

/// Static facts about a host, gathered once when the client starts.
///
/// Missing fields deserialize to their defaults so that clients and servers of slightly
/// different versions stay compatible.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Host {
    /// Client version.
    pub agent: String,
    pub os: String,
    pub os_version: String,
    /// CPU brand string.
    pub cpu: String,
    /// Number of physical cores.
    pub cpu_count: u64,
    pub cpu_arch: String,
    /// Bytes.
    pub total_memory: u64,
    /// Bytes.
    pub total_swap: u64,
    /// Bytes.
    pub total_disk: u64,
    /// Virtualization technology as reported by `systemd-detect-virt`, or `"none"`.
    pub v12n: String,
}

/// Metrics sampled on every tick.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// Percent, averaged over all cores.
    pub cpu_usage: f64,
    /// Bytes.
    pub used_memory: u64,
    /// Bytes.
    pub used_swap: u64,
    /// Bytes.
    pub used_disk: u64,
    /// Bytes received since boot.
    pub network_in: u64,
    /// Bytes sent since boot.
    pub network_out: u64,
    /// Bytes per second.
    pub network_in_speed: u64,
    /// Bytes per second.
    pub network_out_speed: u64,
    /// Seconds.
    pub uptime: u64,
    /// Unix timestamp in seconds.
    pub boot_time: u64,
    pub load1: f64,
    pub load5: f64,
    pub load15: f64,
    /// Established TCP connections.
    pub tcp_count: u64,
    /// Open UDP sockets.
    pub udp_count: u64,
    pub process_count: u64,
}

/// Public addresses of a host as its client found them; an empty string means unknown.
///
/// The server geolocates the address a client connects from, and falls back to these when
/// that one is not public, e.g. behind a reverse proxy that does not pass it on. Servers
/// before the rewrite only use these.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct PublicIp {
    pub public_ipv4: String,
    pub public_ipv6: String,
}

impl PublicIp {
    /// The address to geolocate the host by: its public IPv4 address when known, else its
    /// public IPv6 address.
    pub fn preferred(&self) -> Option<IpAddr> {
        [&self.public_ipv4, &self.public_ipv6]
            .into_iter()
            .filter_map(|ip| ip.parse().ok())
            .find(|&ip| is_public(ip))
    }
}

/// Whether `ip` may be reachable from the internet, so that geolocating it makes sense.
pub fn is_public(ip: IpAddr) -> bool {
    match ip.to_canonical() {
        IpAddr::V4(ip) => {
            let [a, b, ..] = ip.octets();
            // 100.64.0.0/10 is carrier-grade NAT, and used by Tailscale.
            let shared = a == 100 && (b & 0xc0) == 64;
            !(ip.is_private()
                || ip.is_loopback()
                || ip.is_link_local()
                || ip.is_unspecified()
                || ip.is_broadcast()
                || ip.is_multicast()
                || shared)
        }
        IpAddr::V6(ip) => {
            !(ip.is_loopback()
                || ip.is_unspecified()
                || ip.is_multicast()
                || ip.is_unique_local()
                || ip.is_unicast_link_local())
        }
    }
}

/// Message sent by a client on the agent channel.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    /// Must equal [`PROTOCOL_VERSION`].
    pub version: u64,
    /// Unix timestamp in seconds, taken by the client.
    pub timestamp: i64,
    pub data: AgentData,
}

/// Payload of an [`AgentReport`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentData {
    /// Unique name of the node, chosen by the operator.
    pub name: String,
    pub host: Host,
    pub stats: Stats,
    pub ip: PublicIp,
}

/// One node as seen by the dashboard.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NodeSnapshot {
    /// Unix timestamp in seconds of the last report the server received.
    pub timestamp: i64,
    pub name: String,
    pub host: Host,
    pub stats: Stats,
    /// ISO 3166-1 alpha-2 country code, or empty when unknown.
    pub geoip: String,
}

/// The current Unix timestamp in seconds, as used by [`AgentReport`] and [`NodeSnapshot`].
pub fn unix_now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| i64::try_from(d.as_secs()).unwrap_or(i64::MAX))
}

/// Validates a token for use as `Authorization: Bearer <token>`: it must be non-empty
/// visible ASCII. Shaped to serve as a clap `value_parser`.
pub fn parse_token(s: &str) -> Result<String, String> {
    if s.is_empty() {
        return Err("must not be empty".into());
    }
    if !s.bytes().all(|b| b.is_ascii_graphic()) {
        return Err("must only contain visible ASCII characters".into());
    }
    Ok(s.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A complete report as the client sends it.
    const FULL_REPORT: &str = r#"{
        "version": 1,
        "timestamp": 1760000000,
        "data": {
            "name": "tokyo-1",
            "host": {
                "agent": "0.1.0", "os": "Ubuntu", "os_version": "24.04", "cpu": "AMD EPYC",
                "cpu_count": 2, "cpu_arch": "x86_64", "total_memory": 2048, "total_swap": 1024,
                "total_disk": 4096, "v12n": "kvm"
            },
            "stats": {
                "cpu_usage": 12.5, "used_memory": 1024, "used_swap": 0, "used_disk": 2048,
                "network_in": 10, "network_out": 20, "network_in_speed": 1, "network_out_speed": 2,
                "uptime": 3600, "boot_time": 1759996400, "load1": 0.1, "load5": 0.2, "load15": 0.3,
                "tcp_count": 5, "udp_count": 3, "process_count": 99
            },
            "ip": {"public_ipv4": "203.0.113.7", "public_ipv6": ""}
        }
    }"#;

    #[test]
    fn parses_full_report() {
        let report: AgentReport = serde_json::from_str(FULL_REPORT).unwrap();
        assert_eq!(report.version, PROTOCOL_VERSION);
        assert_eq!(report.data.name, "tokyo-1");
        assert_eq!(report.data.host.cpu_count, 2);
        assert_eq!(report.data.host.v12n, "kvm");
        assert_eq!(report.data.stats.process_count, 99);
        assert_eq!(report.data.ip.preferred(), "203.0.113.7".parse().ok());
    }

    #[test]
    fn missing_fields_fall_back_to_defaults() {
        let json = r#"{"version":1,"timestamp":1,"data":{"name":"n","host":{"os":"Debian"},"stats":{},"ip":{}}}"#;
        let report: AgentReport = serde_json::from_str(json).unwrap();
        assert_eq!(report.data.host.os, "Debian");
        assert_eq!(report.data.host.total_memory, 0);
        assert_eq!(report.data.stats, Stats::default());
        assert_eq!(report.data.ip, PublicIp::default());
    }

    fn ip(s: &str) -> IpAddr {
        s.parse().unwrap()
    }

    fn public_ip(v4: &str, v6: &str) -> PublicIp {
        PublicIp {
            public_ipv4: v4.into(),
            public_ipv6: v6.into(),
        }
    }

    #[test]
    fn preferred_ip_favours_public_ipv4() {
        let both = public_ip("198.51.100.1", "2001:db8::1");
        assert_eq!(both.preferred(), Some(ip("198.51.100.1")));
        let v6_only = public_ip("", "2001:db8::1");
        assert_eq!(v6_only.preferred(), Some(ip("2001:db8::1")));
        assert_eq!(PublicIp::default().preferred(), None);
    }

    #[test]
    fn preferred_ip_skips_unusable_addresses() {
        let private_v4 = public_ip("192.168.1.2", "2001:db8::1");
        assert_eq!(private_v4.preferred(), Some(ip("2001:db8::1")));
        assert_eq!(public_ip("not an ip", "fe80::1").preferred(), None);
    }

    #[test]
    fn accepts_only_visible_ascii_tokens() {
        assert_eq!(parse_token("s3cret!#%").as_deref(), Ok("s3cret!#%"));
        for invalid in [
            "",
            " ",
            "two words",
            "tab\tbed",
            "naïve",
            "line\nbreak",
            "nul\0",
        ] {
            assert!(parse_token(invalid).is_err(), "{invalid:?}");
        }
    }

    #[test]
    fn tells_public_from_local_addresses() {
        for public in [
            "8.8.8.8",
            "203.0.113.7",
            "100.128.0.1",
            "2001:4860:4860::8888",
            "::ffff:8.8.8.8",
        ] {
            assert!(is_public(ip(public)), "{public}");
        }
        for local in [
            "10.1.2.3",
            "172.16.0.1",
            "192.168.1.1",
            "127.0.0.1",
            "169.254.0.1",
            "100.64.0.1",
            "100.127.255.254",
            "0.0.0.0",
            "255.255.255.255",
            "224.0.0.1",
            "::1",
            "::",
            "fd00::1",
            "fe80::1",
            "ff02::1",
            "::ffff:10.0.0.1",
        ] {
            assert!(!is_public(ip(local)), "{local}");
        }
    }

    /// The node list of the dashboard channel. The web UI checks its types against the same
    /// file, so a change here has to be made there as well.
    #[test]
    fn node_list_matches_shared_fixture() {
        let nodes = vec![
            NodeSnapshot {
                timestamp: 1760000000,
                name: "tokyo-1".into(),
                host: Host {
                    agent: "0.1.0".into(),
                    os: "Ubuntu".into(),
                    os_version: "24.04".into(),
                    cpu: "AMD EPYC 7B13".into(),
                    cpu_count: 2,
                    cpu_arch: "x86_64".into(),
                    total_memory: 4294967296,
                    total_swap: 1073741824,
                    total_disk: 107374182400,
                    v12n: "kvm".into(),
                },
                stats: Stats {
                    cpu_usage: 12.5,
                    used_memory: 1073741824,
                    used_swap: 0,
                    used_disk: 53687091200,
                    network_in: 10737418240,
                    network_out: 5368709120,
                    network_in_speed: 125000,
                    network_out_speed: 250000,
                    uptime: 90061,
                    boot_time: 1759909939,
                    load1: 0.5,
                    load5: 0.25,
                    load15: 0.125,
                    tcp_count: 12,
                    udp_count: 3,
                    process_count: 120,
                },
                geoip: "JP".into(),
            },
            NodeSnapshot {
                timestamp: 1759990000,
                name: "unknown-location".into(),
                host: Host::default(),
                stats: Stats::default(),
                geoip: String::new(),
            },
        ];
        let fixture: serde_json::Value =
            serde_json::from_str(include_str!("../fixtures/node-list.json")).unwrap();
        assert_eq!(
            serde_json::to_value(&nodes).unwrap(),
            fixture,
            "update fixtures/node-list.json and web/src/lib/types.ts"
        );
        let parsed: Vec<NodeSnapshot> = serde_json::from_value(fixture).unwrap();
        assert_eq!(parsed, nodes);
    }
}
