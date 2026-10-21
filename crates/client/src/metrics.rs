//! Aggregation of raw system counters into report fields.

use aiom_protocol::{PublicIp, is_public};
use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs;
use std::net::IpAddr;
use std::path::Path;
use std::time::Duration;

/// Returns the number of established TCP connections and of open UDP sockets.
pub(crate) fn connection_counts() -> (u64, u64) {
    // On Linux, attaching owner PIDs reads the open files of every process; nothing here
    // needs them. Elsewhere the variant without PIDs is missing or IPv4-only.
    #[cfg(not(target_os = "linux"))]
    use netstat2::iterate_sockets_info;
    #[cfg(target_os = "linux")]
    use netstat2::iterate_sockets_info_without_pids as iterate_sockets_info;
    use netstat2::{AddressFamilyFlags, ProtocolFlags, ProtocolSocketInfo, TcpState};

    let families = AddressFamilyFlags::IPV4 | AddressFamilyFlags::IPV6;
    iterate_sockets_info(families, ProtocolFlags::TCP | ProtocolFlags::UDP)
        .and_then(|mut sockets| {
            sockets.try_fold((0, 0), |(tcp, udp), socket| {
                Ok(match socket?.protocol_socket_info {
                    ProtocolSocketInfo::Tcp(info) if info.state == TcpState::Established => {
                        (tcp + 1, udp)
                    }
                    ProtocolSocketInfo::Tcp(_) => (tcp, udp),
                    ProtocolSocketInfo::Udp(_) => (tcp, udp + 1),
                })
            })
        })
        .unwrap_or((0, 0))
}

/// Returns the number of processes, each of which has a numeric directory in `/proc`.
#[cfg(target_os = "linux")]
pub(crate) fn proc_process_count() -> u64 {
    std::fs::read_dir("/proc").map_or(0, |entries| {
        entries
            .filter_map(Result::ok)
            .filter(|entry| {
                let name = entry.file_name();
                name.to_str()
                    .is_some_and(|name| name.parse::<u32>().is_ok())
            })
            .count() as u64
    })
}

/// Capacity of one mounted file system.
#[derive(Debug, Clone, Copy)]
pub(crate) struct DiskSpace<'a> {
    /// The storage behind the mount, see [`crate::storage::identify`].
    pub(crate) storage: &'a OsStr,
    /// File system type of the storage, such as `ext4`.
    pub(crate) file_system: &'a OsStr,
    pub(crate) total: u64,
    pub(crate) available: u64,
}

/// Returns the total and used bytes over all disks.
pub(crate) fn disk_usage<'a>(disks: impl IntoIterator<Item = DiskSpace<'a>>) -> (u64, u64) {
    let mut seen = HashSet::new();
    // ZFS pool -> bytes used by its datasets, and the free space they share.
    let mut pools = HashMap::<&[u8], (u64, u64)>::new();
    let (mut total, mut used) = (0, 0);
    for disk in disks {
        // The same storage can be mounted several times: bind mounts, btrfs subvolumes and
        // overlays of containers on Linux, several paths of one Windows volume, or the volumes
        // of one APFS container on macOS. These share storage and size.
        if !seen.insert((disk.storage, disk.total)) {
            continue;
        }
        let disk_used = disk.total.saturating_sub(disk.available);
        if disk.file_system == "zfs" {
            // Each dataset, named `<pool>/<path>`, reports its own data plus the free space
            // of the whole pool.
            let pool = disk.storage.as_encoded_bytes().split(|&b| b == b'/').next();
            let (pool_used, pool_available) = pools.entry(pool.unwrap_or_default()).or_default();
            *pool_used += disk_used;
            // Quotas leave some datasets less; the largest is the free space of the pool.
            *pool_available = (*pool_available).max(disk.available);
        } else {
            total += disk.total;
            used += disk_used;
        }
    }
    for (pool_used, pool_available) in pools.into_values() {
        total += pool_used + pool_available;
        used += pool_used;
    }
    (total, used)
}

/// Interface name prefixes of loopback, tunnel, bridge and container interfaces, whose
/// traffic is either local or already counted on a physical interface.
const VIRTUAL_INTERFACE_PREFIXES: &[&str] = &[
    "lo",
    "tun",
    "utun",
    "wg",
    "docker",
    "veth",
    "br-",
    "vmbr",
    "vnet",
    "kube",
    "tailscale",
    "fw",
    "tap",
];

pub(crate) fn is_virtual_interface(name: &str) -> bool {
    VIRTUAL_INTERFACE_PREFIXES
        .iter()
        .any(|prefix| name.starts_with(prefix))
}

/// Name prefixes of interfaces stacked on others, where the kernel names them by kind:
/// bonds, bridges and VLANs on macOS. On Windows, the host's adapters on Hyper-V virtual
/// switches, named `vEthernet (<switch>)`, whose traffic the physical adapter also carries.
const STACKED_INTERFACE_PREFIXES: &[&str] = &["bond", "bridge", "vlan", "vEthernet"];

/// Whether the traffic of an interface counts toward the host's. Of interfaces stacked on
/// each other, only the bottom ones count, as the ones above carry the same traffic again.
///
/// `sys_class_net` is the sysfs directory of network interfaces on Linux, where interfaces
/// are named freely but the stacking is recorded: one above others, such as a bond, bridge,
/// VLAN or VRF, has a `lower_<name>` link to each. Elsewhere the name tells.
pub(crate) fn counts_traffic(name: &str, sys_class_net: Option<&Path>) -> bool {
    if is_virtual_interface(name) {
        return false;
    }
    match sys_class_net {
        Some(dir) => !has_lower_interfaces(&dir.join(name)),
        None => !STACKED_INTERFACE_PREFIXES
            .iter()
            .any(|prefix| name.starts_with(prefix)),
    }
}

fn has_lower_interfaces(interface_dir: &Path) -> bool {
    fs::read_dir(interface_dir).is_ok_and(|entries| {
        entries
            .filter_map(Result::ok)
            .any(|entry| entry.file_name().as_encoded_bytes().starts_with(b"lower_"))
    })
}

/// Byte counters of one network interface.
#[derive(Debug, Clone, Copy)]
pub(crate) struct InterfaceTraffic {
    /// Since boot.
    pub(crate) total_received: u64,
    /// Since boot.
    pub(crate) total_transmitted: u64,
    /// Since the previous refresh.
    pub(crate) received: u64,
    /// Since the previous refresh.
    pub(crate) transmitted: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub(crate) struct NetworkUsage {
    pub(crate) total_received: u64,
    pub(crate) total_transmitted: u64,
    /// Bytes per second.
    pub(crate) receive_speed: u64,
    /// Bytes per second.
    pub(crate) transmit_speed: u64,
}

/// Sums the traffic of the interfaces; `elapsed` is the time since the previous refresh.
pub(crate) fn network_usage(
    interfaces: impl IntoIterator<Item = InterfaceTraffic>,
    elapsed: Duration,
) -> NetworkUsage {
    let mut usage = NetworkUsage::default();
    let (mut received, mut transmitted) = (0, 0);
    for interface in interfaces {
        usage.total_received += interface.total_received;
        usage.total_transmitted += interface.total_transmitted;
        received += interface.received;
        transmitted += interface.transmitted;
    }

    let secs = elapsed.as_secs_f64();
    let rate = |bytes: u64| {
        if secs > 0.0 {
            (bytes as f64 / secs) as u64
        } else {
            0
        }
    };
    usage.receive_speed = rate(received);
    usage.transmit_speed = rate(transmitted);
    usage
}

/// Picks the public IPv4 and IPv6 address among the addresses of the interfaces: the lowest
/// of each, so that the choice stays the same from one report to the next. Behind NAT there
/// is none.
pub(crate) fn public_ip(addrs: impl IntoIterator<Item = IpAddr>) -> PublicIp {
    let (mut v4, mut v6) = (None, None);
    for ip in addrs.into_iter().filter(|&ip| is_public(ip)) {
        let lowest = if ip.is_ipv4() { &mut v4 } else { &mut v6 };
        if lowest.is_none_or(|current| ip < current) {
            *lowest = Some(ip);
        }
    }
    let text = |ip: Option<IpAddr>| ip.map(|ip| ip.to_string()).unwrap_or_default();
    PublicIp {
        public_ipv4: text(v4),
        public_ipv6: text(v6),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn disk<'a>(
        storage: &'a str,
        file_system: &'a str,
        total: u64,
        available: u64,
    ) -> DiskSpace<'a> {
        DiskSpace {
            storage: OsStr::new(storage),
            file_system: OsStr::new(file_system),
            total,
            available,
        }
    }

    #[test]
    fn sums_disks() {
        let disks = [
            disk("/dev/sda1", "ext4", 100, 40),
            disk("/dev/sdb1", "xfs", 50, 50),
        ];
        assert_eq!(disk_usage(disks), (150, 60));
    }

    #[test]
    fn counts_each_storage_once() {
        let bind_mounts = [
            disk("/dev/vda1", "ext4", 100, 30),
            disk("/dev/vda1", "ext4", 100, 30),
            disk("/dev/vda1", "ext4", 100, 30),
        ];
        assert_eq!(disk_usage(bind_mounts), (100, 70));

        // Volumes of one APFS container report its capacity, and the shared free space at
        // slightly different moments.
        let apfs = [
            disk("/dev/disk3", "apfs", 1000, 400),
            disk("/dev/disk3", "apfs", 1000, 398),
        ];
        assert_eq!(disk_usage(apfs), (1000, 600));
    }

    #[test]
    fn counts_distinct_volumes_of_the_same_size() {
        // Two Windows volumes that share label and size.
        let disks = [
            disk(r"\\?\Volume{0a1b}\", "NTFS", 100, 50),
            disk(r"\\?\Volume{2c3d}\", "NTFS", 100, 50),
        ];
        assert_eq!(disk_usage(disks), (200, 100));
    }

    #[test]
    fn counts_overlay_when_backing_storage_is_not_visible() {
        let disks = [disk("overlay", "overlay", 100, 40)];
        assert_eq!(disk_usage(disks), (100, 60));
    }

    #[test]
    fn counts_the_free_space_of_a_zfs_pool_once() {
        let disks = [
            disk("rpool", "zfs", 501, 500),
            disk("rpool/ROOT/pve-1", "zfs", 503, 500),
            disk("rpool/ROOT/pve-1", "zfs", 503, 500),
            disk("rpool/data", "zfs", 510, 500),
            // A quota leaves the dataset less than the pool.
            disk("rpool/limited", "zfs", 50, 40),
            disk("tank/media", "zfs", 300, 200),
        ];
        assert_eq!(disk_usage(disks), (24 + 500 + 300, 24 + 100));
    }

    #[test]
    fn tolerates_more_available_than_total() {
        assert_eq!(disk_usage([disk("x", "ext4", 10, 20)]), (10, 0));
        assert_eq!(disk_usage([]), (0, 0));
    }

    #[test]
    fn recognizes_virtual_interfaces() {
        for name in [
            "lo",
            "lo0",
            "docker0",
            "veth1a2b",
            "br-abc",
            "tailscale0",
            "tun0",
            "utun3",
            "wg0",
        ] {
            assert!(is_virtual_interface(name), "{name}");
        }
        for name in ["eth0", "ens3", "enp0s31f6", "wlan0", "en0", "bond0"] {
            assert!(!is_virtual_interface(name), "{name}");
        }
    }

    fn interface(totals: (u64, u64), delta: (u64, u64)) -> InterfaceTraffic {
        InterfaceTraffic {
            total_received: totals.0,
            total_transmitted: totals.1,
            received: delta.0,
            transmitted: delta.1,
        }
    }

    #[test]
    fn sums_interfaces_and_computes_speed() {
        let interfaces = [
            interface((1000, 2000), (300, 600)),
            interface((10, 20), (100, 0)),
        ];
        assert_eq!(
            network_usage(interfaces, Duration::from_secs(2)),
            NetworkUsage {
                total_received: 1010,
                total_transmitted: 2020,
                receive_speed: 200,
                transmit_speed: 300,
            }
        );
    }

    /// Lays out `/sys/class/net` with an entry per interface, holding its `lower_*` links.
    fn sys_class_net(interfaces: &[(&str, &[&str])]) -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        for (name, lower) in interfaces {
            let interface = dir.path().join(name);
            fs::create_dir(&interface).unwrap();
            fs::write(interface.join("mtu"), "1500").unwrap();
            for lower in *lower {
                fs::create_dir(interface.join(format!("lower_{lower}"))).unwrap();
            }
        }
        dir
    }

    /// Sums the traffic of the interfaces that count, by the stacking in `sys_class_net`.
    fn host_usage(sys_class_net: Option<&Path>, interfaces: &[(&str, u64)]) -> NetworkUsage {
        let counted = interfaces
            .iter()
            .filter(|(name, _)| counts_traffic(name, sys_class_net))
            .map(|&(_, bytes)| interface((bytes, bytes * 2), (bytes / 10, bytes / 5)));
        network_usage(counted, Duration::from_secs(1))
    }

    #[test]
    fn counts_a_bond_and_its_slaves_once() {
        // A bond's counters are the sums of its slaves'.
        let sys = sys_class_net(&[
            ("bond0", &["eth0", "eth1"]),
            ("eth0", &[]),
            ("eth1", &[]),
            ("lo", &[]),
        ]);
        let interfaces = [
            ("bond0", 3000),
            ("eth0", 1000),
            ("eth1", 2000),
            ("lo", 9999),
        ];
        assert_eq!(
            host_usage(Some(sys.path()), &interfaces),
            NetworkUsage {
                total_received: 3000,
                total_transmitted: 6000,
                receive_speed: 300,
                transmit_speed: 600,
            }
        );
    }

    #[test]
    fn counts_the_bottom_of_stacked_interfaces() {
        // A Proxmox host: a bridge on a bond of two NICs, with a VLAN on the bond, and a
        // bridge named freely.
        let sys = sys_class_net(&[
            ("vmbr0", &["bond0", "tap100i0"]),
            ("bond0", &["eno1", "eno2"]),
            ("bond0.100", &["bond0"]),
            ("lan", &["eno3"]),
            ("eno1", &[]),
            ("eno2", &[]),
            ("eno3", &[]),
            ("tap100i0", &[]),
        ]);
        let interfaces = [
            ("vmbr0", 3000),
            ("bond0", 3000),
            ("bond0.100", 500),
            ("lan", 400),
            ("eno1", 1000),
            ("eno2", 2000),
            ("eno3", 400),
            ("tap100i0", 800),
            // Not in sysfs: nothing is known to be below it.
            ("venet0", 50),
        ];
        assert_eq!(
            host_usage(Some(sys.path()), &interfaces).total_received,
            1000 + 2000 + 400 + 50
        );
    }

    #[test]
    fn tells_stacked_interfaces_by_name_without_sysfs() {
        let interfaces = [
            // macOS
            ("en0", 1000),
            ("bridge0", 500),
            ("bond0", 500),
            ("vlan0", 500),
            ("utun4", 500),
            // Windows, with a Hyper-V external switch on the physical adapter.
            ("Ethernet", 2000),
            ("vEthernet (External)", 1500),
        ];
        assert_eq!(host_usage(None, &interfaces).total_received, 3000);
    }

    #[test]
    fn zero_elapsed_time_gives_zero_speed() {
        let usage = network_usage([interface((1, 1), (100, 100))], Duration::ZERO);
        assert_eq!(usage.receive_speed, 0);
        assert_eq!(usage.transmit_speed, 0);
        assert_eq!(usage.total_received, 1);
    }

    #[test]
    fn counts_connections_without_failing() {
        // The values depend on the machine; this only checks that the platform API works.
        let (tcp, udp) = connection_counts();
        assert!(tcp < 10_000_000 && udp < 10_000_000);
    }

    fn ips(addrs: &[&str]) -> PublicIp {
        public_ip(addrs.iter().map(|ip| ip.parse().unwrap()))
    }

    #[test]
    fn picks_the_lowest_public_address_of_each_family() {
        let ip = ips(&[
            "127.0.0.1",
            "203.0.113.9",
            "198.51.100.7",
            "::1",
            "fe80::1",
            "2001:db8::9",
            "2001:db8::2",
        ]);
        assert_eq!(ip.public_ipv4, "198.51.100.7");
        assert_eq!(ip.public_ipv6, "2001:db8::2");
    }

    #[test]
    fn has_no_public_address_behind_nat() {
        assert_eq!(
            ips(&["127.0.0.1", "192.168.1.20", "fd00::20", "fe80::1"]),
            PublicIp::default()
        );
    }
}
