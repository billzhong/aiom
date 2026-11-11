//! Wire format shared by the AIO Monitor server and client.
//!
//! Two WebSocket channels exist:
//!
//! * **Agent channel** – each client sends an [`AgentReport`] as a gzip-compressed JSON binary
//!   frame on every tick.
//! * **Dashboard channel** – the web UI sends the text frame [`DASHBOARD_PING`] and the server
//!   answers with a gzip-compressed JSON array of [`NodeSnapshot`]s.
//!
//! Binary frames are encoded with [`encode`], and decoded with [`decode`], or with
//! [`AgentReport::decode`] on the agent channel.

mod codec;

pub use codec::{CodecError, decode, encode};

use serde::{Deserialize, Serialize};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::time::{SystemTime, UNIX_EPOCH};

/// Version of [`AgentReport`]; the server rejects any other value.
pub const PROTOCOL_VERSION: u64 = 1;

/// Text frame the dashboard sends to request a fresh list of [`NodeSnapshot`]s.
pub const DASHBOARD_PING: &str = "ping";

/// Facts about a host that rarely change. The client gathers them when it starts, and
/// refreshes the total memory, swap and disk space with every sample.
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
///
/// Missing fields deserialize to their defaults, as for [`Host`]. JSON has no NaN or infinity,
/// which encoders write as `null`: floats that are not finite are sent as 0, and `null` is
/// read as 0.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
#[serde(default)]
pub struct Stats {
    /// Percent, averaged over all cores.
    #[serde(with = "finite")]
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
    #[serde(with = "finite")]
    pub load1: f64,
    #[serde(with = "finite")]
    pub load5: f64,
    #[serde(with = "finite")]
    pub load15: f64,
    /// Established TCP connections.
    pub tcp_count: u64,
    /// Open UDP sockets.
    pub udp_count: u64,
    pub process_count: u64,
}

/// (De)serializes the floats of [`Stats`], see there.
mod finite {
    use serde::{Deserialize, Deserializer, Serializer};

    pub(super) fn serialize<S: Serializer>(value: &f64, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_f64(if value.is_finite() { *value } else { 0.0 })
    }

    pub(super) fn deserialize<'de, D: Deserializer<'de>>(deserializer: D) -> Result<f64, D::Error> {
        Ok(Option::<f64>::deserialize(deserializer)?.unwrap_or_default())
    }
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
    /// public IPv6 address. A field holding an address of the other family is ignored.
    pub fn preferred(&self) -> Option<IpAddr> {
        let v4 = self.public_ipv4.parse().ok().map(IpAddr::V4);
        let v6 = self.public_ipv6.parse().ok().map(IpAddr::V6);
        [v4, v6].into_iter().flatten().find_map(as_public)
    }
}

/// Whether `ip` may be reachable from the internet, so that geolocating it makes sense.
pub fn is_public(ip: IpAddr) -> bool {
    as_public(ip).is_some()
}

/// `ip` in canonical form when it [is public](is_public): an IPv4 address mapped to IPv6, as
/// a dual-stack listener sees IPv4 clients, is the IPv4 address.
pub fn as_public(ip: IpAddr) -> Option<IpAddr> {
    let ip = ip.to_canonical();
    let public = match ip {
        IpAddr::V4(ip) => is_public_v4(ip),
        IpAddr::V6(ip) => is_public_v6(ip),
    };
    public.then_some(ip)
}

fn is_public_v4(ip: Ipv4Addr) -> bool {
    let [a, b, c, _] = ip.octets();
    // 100.64.0.0/10 is carrier-grade NAT, and used by Tailscale.
    let shared = a == 100 && (b & 0xc0) == 64;
    // 0.0.0.0/8 is "this network", 192.0.0.0/24 for protocol assignments, 198.18.0.0/15 for
    // benchmarks and the fake IPs of proxies, and 240.0.0.0/4, broadcast included, reserved.
    let reserved = a == 0 || (a, b, c) == (192, 0, 0) || (a == 198 && (b & 0xfe) == 18) || a >= 240;
    !(ip.is_private()
        || ip.is_loopback()
        || ip.is_link_local()
        || ip.is_multicast()
        || shared
        || reserved)
}

fn is_public_v6(ip: Ipv6Addr) -> bool {
    match ip.segments() {
        // An address embedding an IPv4 address is as public as that one: NAT64
        // (64:ff9b::/96), 6to4 (2002::/16), and the deprecated IPv4-compatible addresses
        // (::/96), which include `::` and `::1` as 0.0.0.0/8.
        [0x64, 0xff9b, 0, 0, 0, 0, high, low]
        | [0, 0, 0, 0, 0, 0, high, low]
        | [0x2002, high, low, ..] => {
            is_public_v4(Ipv4Addr::from((u32::from(high) << 16) | u32::from(low)))
        }
        // 100::/64 discards what it gets.
        [0x100, 0, 0, 0, ..] => false,
        // fec0::/10 is the deprecated site-local range.
        [first, ..] if (first & 0xffc0) == 0xfec0 => false,
        _ => !(ip.is_multicast() || ip.is_unique_local() || ip.is_unicast_link_local()),
    }
}

/// Message sent by a client on the agent channel. Only the version and the name of the node
/// are required, missing fields deserialize to their defaults.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentReport {
    /// Must equal [`PROTOCOL_VERSION`].
    pub version: u64,
    /// Unix timestamp in seconds, taken by the client.
    #[serde(default)]
    pub timestamp: i64,
    pub data: AgentData,
}

/// Error returned by [`AgentReport::decode`].
#[derive(Debug, thiserror::Error)]
pub enum ReportError {
    #[error("unsupported protocol version {0}")]
    UnsupportedVersion(u64),
    #[error(transparent)]
    Codec(#[from] CodecError),
}

impl AgentReport {
    /// Decodes a report as [`decode`] does, but tells a report of another version apart from a
    /// malformed one, whatever its data looks like.
    pub fn decode(bytes: &[u8], max_len: usize) -> Result<Self, ReportError> {
        /// What reports of every version have in common.
        #[derive(Deserialize)]
        struct Envelope {
            version: u64,
        }
        let json = codec::inflate(bytes, max_len)?;
        let version = match serde_json::from_slice::<Self>(&json) {
            Ok(report) if report.version == PROTOCOL_VERSION => return Ok(report),
            Ok(report) => report.version,
            // Only a report that does not parse is read again, for its version alone.
            Err(e) => match serde_json::from_slice::<Envelope>(&json) {
                Ok(Envelope { version }) if version != PROTOCOL_VERSION => version,
                _ => return Err(CodecError::from(e).into()),
            },
        };
        Err(ReportError::UnsupportedVersion(version))
    }
}

/// Payload of an [`AgentReport`].
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentData {
    /// Unique name of the node, chosen by the operator, as [`parse_name`] gives it. The server
    /// refuses other names.
    pub name: String,
    #[serde(default)]
    pub host: Host,
    #[serde(default)]
    pub stats: Stats,
    #[serde(default)]
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

/// Validates the name of a node: surrounding whitespace is dropped, and the rest must not be
/// empty. Shaped to serve as a clap `value_parser`.
pub fn parse_name(s: &str) -> Result<String, String> {
    let name = s.trim();
    if name.is_empty() {
        return Err("must not be empty".into());
    }
    Ok(name.to_owned())
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

        let minimal: AgentReport =
            serde_json::from_str(r#"{"version":1,"data":{"name":"n"}}"#).unwrap();
        assert_eq!(minimal.timestamp, 0);
        assert_eq!(minimal.data.host, Host::default());
        assert_eq!(minimal.data.stats, Stats::default());
        assert_eq!(minimal.data.ip, PublicIp::default());
    }

    #[test]
    fn sends_floats_that_are_not_finite_as_zero() {
        let stats = Stats {
            cpu_usage: f64::NAN,
            load1: f64::INFINITY,
            load5: f64::NEG_INFINITY,
            load15: 0.5,
            ..Default::default()
        };
        let json = serde_json::to_value(&stats).unwrap();
        for (field, value) in [
            ("cpu_usage", 0.0),
            ("load1", 0.0),
            ("load5", 0.0),
            ("load15", 0.5),
        ] {
            assert_eq!(json[field], value, "{field}");
        }
    }

    #[test]
    fn reads_null_floats_as_zero() {
        let json = r#"{"cpu_usage":null,"load1":null,"load5":1.5,"load15":2}"#;
        let stats: Stats = serde_json::from_str(json).unwrap();
        assert_eq!((stats.cpu_usage, stats.load1), (0.0, 0.0));
        assert_eq!((stats.load5, stats.load15), (1.5, 2.0));
    }

    #[test]
    fn checks_the_version_of_a_report_before_its_data() {
        let decode =
            |report: serde_json::Value| AgentReport::decode(&encode(&report).unwrap(), 1 << 20);
        // A later version may change the data as well.
        let reshaped = serde_json::json!({"version": PROTOCOL_VERSION + 1, "data": {"id": 7}});
        assert!(
            matches!(decode(reshaped), Err(ReportError::UnsupportedVersion(v)) if v == PROTOCOL_VERSION + 1)
        );
        let malformed = serde_json::json!({"version": PROTOCOL_VERSION, "data": {"id": 7}});
        assert!(matches!(
            decode(malformed),
            Err(ReportError::Codec(CodecError::Json(_)))
        ));
        let unversioned = serde_json::json!({"data": {"name": "n"}});
        assert!(matches!(
            decode(unversioned),
            Err(ReportError::Codec(CodecError::Json(_)))
        ));

        let report: AgentReport = serde_json::from_str(FULL_REPORT).unwrap();
        let decoded = AgentReport::decode(&encode(&report).unwrap(), 1 << 20).unwrap();
        assert_eq!(decoded, report);
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
    fn preferred_ip_takes_each_field_for_its_family() {
        assert_eq!(public_ip("2001:4860::1", "").preferred(), None);
        assert_eq!(public_ip("", "8.8.8.8").preferred(), None);
        let mapped = public_ip("", "::ffff:8.8.8.8").preferred();
        assert_eq!(mapped, Some(IpAddr::V4(Ipv4Addr::new(8, 8, 8, 8))));
    }

    #[test]
    fn names_are_trimmed_and_not_empty() {
        assert_eq!(parse_name(" web-1\t").as_deref(), Ok("web-1"));
        assert_eq!(parse_name("东京 1").as_deref(), Ok("东京 1"));
        for invalid in ["", " ", "\t\n", "\u{3000}"] {
            assert!(parse_name(invalid).is_err(), "{invalid:?}");
        }
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
            "64:ff9b::808:808",
            "2002:808:808::1",
            "::8.8.8.8",
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
            "0.1.2.3",
            "192.0.0.8",
            "198.18.0.1",
            "198.19.255.254",
            "240.0.0.1",
            "fec0::1",
            "64:ff9b::a00:1",
            "2002:a00:1::1",
            "::10.0.0.1",
            "100::1",
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
