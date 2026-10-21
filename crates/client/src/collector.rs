use aiom_protocol::{Host, PublicIp, Stats};
use std::path::Path;
use std::time::Instant;
use sysinfo::{
    CpuRefreshKind, DiskRefreshKind, Disks, MemoryRefreshKind, Networks, RefreshKind, System,
};

use crate::metrics::{self, DiskSpace, InterfaceTraffic};
use crate::storage;

/// Samples the metrics of this host. Keeps the sysinfo state between samples, plus the host
/// facts gathered at startup, whose capacities each sample refreshes.
#[derive(Debug)]
pub struct Collector {
    sys: System,
    disks: Disks,
    networks: Networks,
    /// Time of the previous network refresh, to turn byte counts into speeds.
    last_network_refresh: Instant,
    host: Host,
}

impl Default for Collector {
    fn default() -> Self {
        Self::new()
    }
}

impl Collector {
    pub fn new() -> Self {
        let sys = System::new_with_specifics(
            RefreshKind::nothing()
                .with_memory(MemoryRefreshKind::everything())
                .with_cpu(CpuRefreshKind::nothing().with_cpu_usage()),
        );
        let disks =
            Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing().with_storage());
        // Listed interfaces take their current counters as the baseline for the first speed.
        let networks = Networks::new_with_refreshed_list();
        let last_network_refresh = Instant::now();

        let unknown = || "Unknown".to_owned();
        let host = Host {
            agent: env!("CARGO_PKG_VERSION").to_owned(),
            os: System::name().unwrap_or_else(unknown),
            os_version: System::os_version().unwrap_or_else(unknown),
            cpu: sys
                .cpus()
                .first()
                .map_or_else(unknown, |cpu| cpu.brand().trim().to_owned()),
            cpu_count: System::physical_core_count().unwrap_or(0) as u64,
            cpu_arch: System::cpu_arch(),
            total_memory: sys.total_memory(),
            total_swap: sys.total_swap(),
            total_disk: disk_usage(&disks).0,
            v12n: crate::virt::detect().unwrap_or("none").to_owned(),
        };

        Self {
            sys,
            disks,
            networks,
            last_network_refresh,
            host,
        }
    }

    /// Facts about this host. The total memory, swap and disk space are as of the last
    /// sample, since disks get mounted or resized and swap turned on or off at runtime.
    pub fn host(&self) -> &Host {
        &self.host
    }

    /// The public addresses of the physical interfaces as of the last sample, if any. The
    /// server prefers the address the client connects from; these serve when it cannot see
    /// that one, and servers before the rewrite only use these.
    pub fn public_ip(&self) -> PublicIp {
        metrics::public_ip(
            self.networks
                .iter()
                .filter(|(name, _)| !metrics::is_virtual_interface(name))
                .flat_map(|(_, data)| data.ip_networks().iter().map(|network| network.addr)),
        )
    }

    /// Samples the current metrics. CPU usage and network speeds are averaged over the time
    /// since the previous call.
    pub async fn sample(&mut self) -> Stats {
        // Sample CPU first, so usage covers the whole interval since the previous sample
        // instead of a window dominated by this client's own refresh work.
        self.sys.refresh_cpu_usage();

        let connections = tokio::task::spawn_blocking(metrics::connection_counts);

        self.sys.refresh_memory();
        let process_count = self.process_count();
        self.disks
            .refresh_specifics(true, DiskRefreshKind::nothing().with_storage());
        self.networks.refresh(true);
        let now = Instant::now();
        let elapsed = now - std::mem::replace(&mut self.last_network_refresh, now);

        let sys_class_net = cfg!(target_os = "linux").then_some(Path::new("/sys/class/net"));
        let network = metrics::network_usage(
            self.networks
                .iter()
                .filter(|(name, _)| metrics::counts_traffic(name, sys_class_net))
                .map(|(_, data)| InterfaceTraffic {
                    total_received: data.total_received(),
                    total_transmitted: data.total_transmitted(),
                    received: data.received(),
                    transmitted: data.transmitted(),
                }),
            elapsed,
        );
        // Refresh the totals along with the usage, so used never exceeds total.
        let (total_disk, used_disk) = disk_usage(&self.disks);
        self.host.total_disk = total_disk;
        self.host.total_memory = self.sys.total_memory();
        self.host.total_swap = self.sys.total_swap();
        let load = System::load_average();
        let (tcp_count, udp_count) = connections.await.unwrap_or_else(|e| {
            log::error!("Counting connections failed: {e}");
            (0, 0)
        });

        Stats {
            cpu_usage: f64::from(self.sys.global_cpu_usage()),
            used_memory: self.sys.used_memory(),
            used_swap: self.sys.used_swap(),
            used_disk,
            network_in: network.total_received,
            network_out: network.total_transmitted,
            network_in_speed: network.receive_speed,
            network_out_speed: network.transmit_speed,
            uptime: System::uptime(),
            boot_time: System::boot_time(),
            load1: load.one,
            load5: load.five,
            load15: load.fifteen,
            tcp_count,
            udp_count,
            process_count,
        }
    }

    /// Listing `/proc` is enough on Linux, where sysinfo would read two files per process.
    #[cfg(target_os = "linux")]
    fn process_count(&mut self) -> u64 {
        metrics::proc_process_count()
    }

    #[cfg(not(target_os = "linux"))]
    fn process_count(&mut self) -> u64 {
        self.sys.refresh_processes_specifics(
            sysinfo::ProcessesToUpdate::All,
            true,
            sysinfo::ProcessRefreshKind::nothing(),
        );
        self.sys.processes().len() as u64
    }
}

fn disk_usage(disks: &Disks) -> (u64, u64) {
    let storages = storage::identify(disks.list());
    metrics::disk_usage(
        disks
            .list()
            .iter()
            .zip(&storages)
            .map(|(disk, storage)| DiskSpace {
                storage: &storage.id,
                file_system: &storage.file_system,
                total: disk.total_space(),
                available: disk.available_space(),
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use aiom_protocol::is_public;

    #[tokio::test]
    async fn collects_plausible_metrics() {
        let mut collector = Collector::new();
        let host = collector.host().clone();
        assert_eq!(host.agent, env!("CARGO_PKG_VERSION"));
        assert!(!host.os.is_empty());
        assert!(!host.cpu_arch.is_empty());
        assert!(host.total_memory > 0);
        assert!(!host.v12n.is_empty());

        let stats = collector.sample().await;
        assert!(
            (0.0..=100.0).contains(&stats.cpu_usage),
            "{}",
            stats.cpu_usage
        );
        assert!(stats.used_memory > 0);
        // The totals are refreshed with the usage.
        let host = collector.host();
        assert!(stats.used_memory <= host.total_memory);
        assert!(stats.used_swap <= host.total_swap);
        assert!(stats.used_disk <= host.total_disk);
        assert!(stats.process_count > 0);
        assert!(stats.uptime > 0);
        assert!(stats.boot_time > 0);

        let ip = collector.public_ip();
        for ip in [ip.public_ipv4, ip.public_ipv6] {
            assert!(ip.is_empty() || is_public(ip.parse().unwrap()), "{ip}");
        }
    }
}
