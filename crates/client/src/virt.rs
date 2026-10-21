//! Virtualization detection following systemd's `src/basic/virt.c` (`systemd-detect-virt`),
//! including its identifiers. Only Linux is supported; other platforms report nothing.
//!
//! The logic reads its evidence through [`Probe`], so tests can run it against a fake file
//! system on any platform.

use std::fs;
use std::path::PathBuf;

/// Returns the container or VM technology this process runs in, if any.
pub(crate) fn detect() -> Option<&'static str> {
    if cfg!(target_os = "linux") {
        Probe::system().detect()
    } else {
        None
    }
}

const VM_OTHER: &str = "vm-other";
const CONTAINER_OTHER: &str = "container-other";

const CONTAINERS: &[&str] = &[
    "lxc",
    "lxc-libvirt",
    "systemd-nspawn",
    "docker",
    "podman",
    "rkt",
    "wsl",
    "proot",
    "pouch",
];

/// DMI value prefix -> id.
const DMI_VENDORS: &[(&str, &str)] = &[
    ("KVM", "kvm"),
    ("OpenStack", "kvm"),
    ("KubeVirt", "kvm"),
    ("Alibaba Cloud ECS", "kvm"),
    ("Amazon EC2", "amazon"),
    ("QEMU", "qemu"),
    ("VMware", "vmware"),
    ("VMW", "vmware"),
    ("innotek GmbH", "oracle"),
    ("VirtualBox", "oracle"),
    ("Oracle Corporation", "oracle"),
    ("Xen", "xen"),
    ("Bochs", "bochs"),
    ("Parallels", "parallels"),
    ("BHYVE", "bhyve"),
    ("Hyper-V", "microsoft"),
    ("Apple Virtualization", "apple"),
    ("Google Compute Engine", "google"),
];

/// The evidence detection looks at.
struct Probe {
    /// Prefix of every file path read; `/` on a real system.
    root: PathBuf,
    /// Our own process id.
    pid: u32,
    /// Our own `$container` environment variable.
    container_env: Option<String>,
    /// Identifies the hypervisor through CPUID.
    cpuid: fn() -> Option<&'static str>,
}

impl Probe {
    fn system() -> Self {
        Self {
            root: PathBuf::from("/"),
            pid: std::process::id(),
            container_env: std::env::var("container").ok(),
            cpuid: detect_vm_cpuid,
        }
    }

    fn detect(&self) -> Option<&'static str> {
        self.detect_container().or_else(|| self.detect_vm())
    }

    fn path(&self, path: &str) -> PathBuf {
        self.root.join(path.trim_start_matches('/'))
    }

    fn exists(&self, path: &str) -> bool {
        self.path(path).exists()
    }

    /// Reads the first line, treating NUL as a line end like systemd's `read_one_line_file`.
    fn read_first_line(&self, path: &str) -> Option<String> {
        let content = fs::read_to_string(self.path(path)).ok()?;
        content
            .split(['\n', '\0'])
            .next()
            .map(|line| line.trim_end_matches('\r').to_owned())
    }

    /// Returns the trimmed rest of the first line starting with `prefix`.
    fn read_field(&self, path: &str, prefix: &str) -> Option<String> {
        let content = fs::read_to_string(self.path(path)).ok()?;
        content
            .lines()
            .find_map(|line| line.strip_prefix(prefix))
            .map(|value| value.trim().to_owned())
    }

    // ---- Containers ----

    fn detect_container(&self) -> Option<&'static str> {
        // `/proc/vz` exists both inside and outside OpenVZ containers, `/proc/bc` only outside.
        if self.exists("/proc/vz") && !self.exists("/proc/bc") {
            return Some("openvz");
        }

        if let Some(release) = self.read_first_line("/proc/sys/kernel/osrelease")
            && (release.contains("Microsoft") || release.contains("WSL"))
        {
            return Some("wsl");
        }

        if self.is_traced_by_proot() {
            return Some("proot");
        }

        if let Some(name) = self
            .read_first_line("/run/host/container-manager")
            .filter(|s| !s.is_empty())
        {
            return Some(self.translate_container(&name));
        }

        if self.pid == 1 {
            // As PID 1 our own $container is authoritative; empty means no container.
            match self.container_env.as_deref() {
                Some("") => return None,
                Some(name) => return Some(self.translate_container(name)),
                None => {}
            }
        } else {
            // systemd PID 1 records this in /run, which needs no privileges to read. The
            // fallback for non-systemd PID 1 needs CAP_SYS_PTRACE, so its failures are ignored.
            let name = self
                .read_first_line("/run/systemd/container")
                .filter(|s| !s.is_empty())
                .or_else(|| self.pid1_container_env());
            if let Some(name) = name {
                return Some(self.translate_container(&name));
            }
        }

        // Only checked after container managers, since /.dockerenv can leak into images.
        self.detect_container_files()
            .or_else(|| self.running_in_pid_namespace().then_some(CONTAINER_OTHER))
    }

    fn translate_container(&self, name: &str) -> &'static str {
        // Some images set container=oci, which names no specific manager.
        if name == "oci" {
            return self.detect_container_files().unwrap_or(CONTAINER_OTHER);
        }
        CONTAINERS
            .iter()
            .copied()
            .find(|&c| c == name)
            .unwrap_or(CONTAINER_OTHER)
    }

    fn detect_container_files(&self) -> Option<&'static str> {
        if self.exists("/run/.containerenv") {
            Some("podman")
        } else if self.exists("/.dockerenv") {
            Some("docker")
        } else {
            None
        }
    }

    fn is_traced_by_proot(&self) -> bool {
        self.read_field("/proc/self/status", "TracerPid:")
            .filter(|pid| pid != "0")
            .and_then(|pid| self.read_first_line(&format!("/proc/{pid}/comm")))
            .is_some_and(|comm| comm.starts_with("proot"))
    }

    fn pid1_container_env(&self) -> Option<String> {
        let environ = fs::read(self.path("/proc/1/environ")).ok()?;
        environ
            .split(|b| *b == 0)
            .find_map(|var| var.strip_prefix(b"container="))
            .map(|value| String::from_utf8_lossy(value).into_owned())
    }

    #[cfg(unix)]
    fn running_in_pid_namespace(&self) -> bool {
        use std::os::unix::fs::MetadataExt;
        // The root PID namespace has had a fixed inode number since Linux 3.8.
        const PROC_PID_INIT_INO: u64 = 0xEFFF_FFFC;
        fs::metadata(self.path("/proc/self/ns/pid")).is_ok_and(|m| m.ino() != PROC_PID_INIT_INO)
    }

    #[cfg(not(unix))]
    fn running_in_pid_namespace(&self) -> bool {
        false
    }

    // ---- Virtual machines ----

    fn detect_vm(&self) -> Option<&'static str> {
        // Weak evidence (Hyper-V or an unknown VM), reported only if nothing more specific
        // turns up.
        let mut fallback = None;
        let dmi = self.detect_vm_dmi();

        let found = 'found: {
            // Vendors that run on KVM (or cloak as Hyper-V) are identified by DMI first.
            if matches!(dmi, Some("oracle" | "xen" | "amazon" | "parallels")) {
                break 'found dmi;
            }
            // Without CPUID, GCE VMs can't be told apart from bare metal; report them as VMs.
            if !cfg!(any(target_arch = "x86", target_arch = "x86_64")) && dmi == Some("google") {
                break 'found dmi;
            }

            if let Some(v) = self.detect_vm_uml() {
                break 'found Some(v);
            }

            // A Xen Dom0 is not a VM, unless CPUID shows it is nested in another hypervisor.
            let in_xen = self.exists("/proc/xen");
            let xen_dom0 = in_xen && self.is_xen_dom0();
            if in_xen && !xen_dom0 {
                break 'found Some("xen");
            }

            let cpuid = (self.cpuid)();
            match cpuid {
                // QEMU may expose Hyper-V enlightenments, so prefer other evidence first.
                Some("microsoft" | VM_OTHER) => fallback = cpuid,
                Some("kvm") if dmi == Some("google") => break 'found Some("google"),
                Some(_) => break 'found cpuid,
                None => {}
            }
            if xen_dom0 {
                break 'found cpuid;
            }

            match dmi {
                Some(VM_OTHER) => {
                    fallback.get_or_insert(VM_OTHER);
                }
                // GCE in DMI without KVM in CPUID is a bare-metal instance.
                None | Some("google") => {}
                Some(_) => break 'found dmi,
            }

            for probe in [Self::detect_vm_hypervisor, Self::detect_vm_device_tree] {
                match probe(self) {
                    Some(VM_OTHER) => {
                        fallback.get_or_insert(VM_OTHER);
                    }
                    Some(v) => break 'found Some(v),
                    None => {}
                }
            }

            self.detect_vm_zvm()
        };

        found.or(fallback)
    }

    /// DMI is absent on platforms without it (e.g. POWER, s390x), so this finds nothing there.
    fn detect_vm_dmi(&self) -> Option<&'static str> {
        let vendor = self.detect_vm_dmi_vendor();

        // DMI can't tell EC2 VMs from bare-metal instances, so look at SMBIOS.
        if vendor == Some("amazon") {
            return match self.smbios_vm_bit() {
                Some(true) => vendor,
                Some(false) => None,
                // The SMBIOS table is root-only; fall back to the product name.
                None => match self.read_first_line("/sys/class/dmi/id/product_name") {
                    Some(name) if is_metal_product(&name) => None,
                    _ => vendor,
                },
            };
        }

        if vendor.is_none() && self.smbios_vm_bit() == Some(true) {
            return Some(VM_OTHER);
        }
        vendor
    }

    fn detect_vm_dmi_vendor(&self) -> Option<&'static str> {
        [
            "/sys/class/dmi/id/product_name", // before sys_vendor to report KVM rather than QEMU
            "/sys/class/dmi/id/sys_vendor",
            "/sys/class/dmi/id/board_vendor",
            "/sys/class/dmi/id/bios_vendor",
            "/sys/class/dmi/id/product_version", // for Hyper-V
        ]
        .into_iter()
        .filter_map(|path| self.read_first_line(path))
        .find_map(|line| {
            DMI_VENDORS
                .iter()
                .find(|(prefix, _)| line.starts_with(prefix))
                .map(|(_, id)| *id)
        })
    }

    /// SMBIOS BIOS Characteristics Extension Byte 2, bit 4: "system is a VM".
    /// Returns None when the table is unreadable or too short.
    fn smbios_vm_bit(&self) -> Option<bool> {
        let raw = fs::read(self.path("/sys/firmware/dmi/entries/0-0/raw")).ok()?;
        if raw.len() < 20 || raw[1] < 20 {
            return None;
        }
        Some(raw[19] & (1 << 4) != 0)
    }

    fn detect_vm_uml(&self) -> Option<&'static str> {
        self.read_field("/proc/cpuinfo", "vendor_id\t:")?
            .starts_with("User Mode Linux")
            .then_some("uml")
    }

    fn is_xen_dom0(&self) -> bool {
        const XENFEAT_DOM0: u32 = 11;

        if let Some(features) = self.read_first_line("/sys/hypervisor/properties/features") {
            let hex: String = features
                .chars()
                .take_while(char::is_ascii_hexdigit)
                .collect();
            if let Ok(features) = u64::from_str_radix(&hex, 16) {
                return features & (1 << XENFEAT_DOM0) != 0;
            }
        }

        self.read_first_line("/proc/xen/capabilities")
            .is_some_and(|caps| caps.split(',').any(|cap| cap.trim() == "control_d"))
    }

    fn detect_vm_hypervisor(&self) -> Option<&'static str> {
        let hv_type = self.read_first_line("/sys/hypervisor/type")?;
        Some(if hv_type == "xen" { "xen" } else { VM_OTHER })
    }

    fn detect_vm_device_tree(&self) -> Option<&'static str> {
        if !cfg!(any(
            target_arch = "arm",
            target_arch = "aarch64",
            target_arch = "powerpc",
            target_arch = "powerpc64",
            target_arch = "riscv32",
            target_arch = "riscv64"
        )) {
            return None;
        }

        if let Some(hv_type) = self.read_first_line("/proc/device-tree/hypervisor/compatible") {
            return Some(if hv_type == "linux,kvm" {
                "kvm"
            } else if hv_type.contains("xen") {
                "xen"
            } else if hv_type.contains("vmware") {
                "vmware"
            } else {
                VM_OTHER
            });
        }

        if self.exists("/proc/device-tree/ibm,partition-name")
            && self.exists("/proc/device-tree/hmc-managed?")
            && !self.exists("/proc/device-tree/chosen/qemu,graphic-width")
        {
            return Some("powervm");
        }

        if fs::read_dir(self.path("/proc/device-tree"))
            .ok()?
            .flatten()
            .any(|entry| entry.file_name().to_string_lossy().contains("fw-cfg"))
        {
            return Some("qemu");
        }

        match self
            .read_first_line("/proc/device-tree/compatible")?
            .as_str()
        {
            "qemu,pseries" => Some("qemu"),
            "linux,dummy-virt" => Some(VM_OTHER),
            _ => None,
        }
    }

    fn detect_vm_zvm(&self) -> Option<&'static str> {
        if !cfg!(target_arch = "s390x") {
            return None;
        }

        let program = self.read_field("/proc/sysinfo", "VM00 Control Program:")?;
        let program = program.split_whitespace().next()?;
        Some(if program == "z/VM" { "zvm" } else { "kvm" })
    }
}

/// EC2 bare-metal instance types end in ".metal" (optionally followed by "-...").
fn is_metal_product(name: &str) -> bool {
    name.split_once(".metal")
        .is_some_and(|(_, rest)| rest.is_empty() || rest.starts_with('-'))
}

/// Maps a CPUID hypervisor vendor signature (leaf 0x40000000) to an id.
#[cfg_attr(
    not(any(target_arch = "x86", target_arch = "x86_64")),
    allow(dead_code)
)]
fn hypervisor_from_signature(signature: &[u8; 12]) -> &'static str {
    match signature {
        b"XenVMMXenVMM" => "xen",
        // The second is KVM with Hyper-V enlightenments.
        b"KVMKVMKVM\0\0\0" | b"Linux KVM Hv" => "kvm",
        b"TCGTCGTCGTCG" => "qemu",
        b"VMwareVMware" => "vmware",
        b"Microsoft Hv" => "microsoft",
        b"bhyve bhyve " => "bhyve",
        b"QNXQVMBSQG\0\0" => "qnx",
        b"ACRNACRNACRN" => "acrn",
        b"SRESRESRESRE" => "sre",
        b"Apple VZ\0\0\0\0" => "apple",
        _ => VM_OTHER,
    }
}

#[cfg(any(target_arch = "x86", target_arch = "x86_64"))]
#[allow(unsafe_code, unused_unsafe)] // `__cpuid` is only safe on newer toolchains.
fn detect_vm_cpuid() -> Option<&'static str> {
    #[cfg(target_arch = "x86")]
    use std::arch::x86::__cpuid;
    #[cfg(target_arch = "x86_64")]
    use std::arch::x86_64::__cpuid;

    // SAFETY: CPUID is available on every x86 CPU Rust targets.
    let features = unsafe { __cpuid(1) };
    // Hypervisor present bit (CPUID.1:ECX[31]).
    if features.ecx & (1 << 31) == 0 {
        return None;
    }

    // SAFETY: as above.
    let leaf = unsafe { __cpuid(0x4000_0000) };
    let mut signature = [0; 12];
    for (chunk, register) in signature
        .as_chunks_mut::<4>()
        .0
        .iter_mut()
        .zip([leaf.ebx, leaf.ecx, leaf.edx])
    {
        *chunk = register.to_le_bytes();
    }
    Some(hypervisor_from_signature(&signature))
}

#[cfg(not(any(target_arch = "x86", target_arch = "x86_64")))]
fn detect_vm_cpuid() -> Option<&'static str> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    /// A [`Probe`] of a fake root file system.
    struct FakeSystem {
        /// Kept to delete the root when done.
        _dir: TempDir,
        probe: Probe,
    }

    impl FakeSystem {
        fn new() -> Self {
            let dir = TempDir::new().unwrap();
            let probe = Probe {
                root: dir.path().to_owned(),
                pid: 1000,
                container_env: None,
                cpuid: || None,
            };
            Self { _dir: dir, probe }
        }

        fn file(self, path: &str, content: impl AsRef<[u8]>) -> Self {
            let path = self.probe.root.join(path.trim_start_matches('/'));
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, content).unwrap();
            self
        }

        fn cpuid(mut self, cpuid: fn() -> Option<&'static str>) -> Self {
            self.probe.cpuid = cpuid;
            self
        }

        fn pid1(mut self, container_env: Option<&str>) -> Self {
            self.probe.pid = 1;
            self.probe.container_env = container_env.map(str::to_owned);
            self
        }

        fn detect(&self) -> Option<&'static str> {
            self.probe.detect()
        }
    }

    #[test]
    fn bare_metal() {
        assert_eq!(FakeSystem::new().detect(), None);
    }

    #[test]
    fn container_marker_files() {
        assert_eq!(
            FakeSystem::new().file("/.dockerenv", "").detect(),
            Some("docker")
        );
        assert_eq!(
            FakeSystem::new()
                .file("/.dockerenv", "")
                .file("/run/.containerenv", "")
                .detect(),
            Some("podman")
        );
    }

    #[test]
    fn openvz() {
        assert_eq!(
            FakeSystem::new().file("/proc/vz/version", "").detect(),
            Some("openvz")
        );
        // The host has /proc/bc as well.
        assert_eq!(
            FakeSystem::new()
                .file("/proc/vz/version", "")
                .file("/proc/bc/0", "")
                .detect(),
            None
        );
    }

    #[test]
    fn wsl() {
        let system = FakeSystem::new().file(
            "/proc/sys/kernel/osrelease",
            "6.6.87.2-microsoft-standard-WSL2\n",
        );
        assert_eq!(system.detect(), Some("wsl"));
    }

    #[test]
    fn proot() {
        let system = FakeSystem::new()
            .file("/proc/self/status", "Name:\tsh\nTracerPid:\t4242\n")
            .file("/proc/4242/comm", "proot\n");
        assert_eq!(system.detect(), Some("proot"));
    }

    #[test]
    fn container_manager_files() {
        let system = FakeSystem::new().file("/run/host/container-manager", "systemd-nspawn\n");
        assert_eq!(system.detect(), Some("systemd-nspawn"));

        let system = FakeSystem::new().file("/run/systemd/container", "lxc\n");
        assert_eq!(system.detect(), Some("lxc"));

        let system = FakeSystem::new().file("/run/systemd/container", "something-new\n");
        assert_eq!(system.detect(), Some(CONTAINER_OTHER));
    }

    #[test]
    fn oci_resolves_through_marker_files() {
        let system = FakeSystem::new()
            .file("/run/systemd/container", "oci")
            .file("/.dockerenv", "");
        assert_eq!(system.detect(), Some("docker"));

        let system = FakeSystem::new().file("/run/systemd/container", "oci");
        assert_eq!(system.detect(), Some(CONTAINER_OTHER));
    }

    #[test]
    fn pid1_container_environment() {
        let system = FakeSystem::new().file("/proc/1/environ", b"PATH=/bin\0container=podman\0");
        assert_eq!(system.detect(), Some("podman"));
    }

    #[test]
    fn running_as_pid1_trusts_own_environment() {
        assert_eq!(
            FakeSystem::new().pid1(Some("docker")).detect(),
            Some("docker")
        );
        // An empty $container means "not in a container", even with a leaked /.dockerenv.
        assert_eq!(
            FakeSystem::new()
                .pid1(Some(""))
                .file("/.dockerenv", "")
                .detect(),
            None
        );
    }

    #[test]
    fn dmi_vendors() {
        let kvm = FakeSystem::new()
            .file("/sys/class/dmi/id/product_name", "KVM\n")
            .file("/sys/class/dmi/id/sys_vendor", "QEMU\n");
        assert_eq!(kvm.detect(), Some("kvm"));

        let vbox = FakeSystem::new().file("/sys/class/dmi/id/sys_vendor", "innotek GmbH\n");
        assert_eq!(vbox.detect(), Some("oracle"));
    }

    #[test]
    fn containers_take_precedence_over_vms() {
        let system = FakeSystem::new()
            .file("/sys/class/dmi/id/product_name", "KVM\n")
            .file("/.dockerenv", "");
        assert_eq!(system.detect(), Some("docker"));
    }

    #[test]
    fn amazon_ec2_virtual_and_metal() {
        let vm = FakeSystem::new()
            .file("/sys/class/dmi/id/sys_vendor", "Amazon EC2\n")
            .file("/sys/class/dmi/id/product_name", "m5.large\n");
        assert_eq!(vm.detect(), Some("amazon"));

        let metal = FakeSystem::new()
            .file("/sys/class/dmi/id/sys_vendor", "Amazon EC2\n")
            .file("/sys/class/dmi/id/product_name", "m5.metal\n");
        assert_eq!(metal.detect(), None);
    }

    fn smbios(vm: bool) -> Vec<u8> {
        let mut raw = vec![0; 24];
        raw[1] = 24;
        raw[19] = if vm { 1 << 4 } else { 0 };
        raw
    }

    #[test]
    fn smbios_vm_bit() {
        let unknown_vm = FakeSystem::new().file("/sys/firmware/dmi/entries/0-0/raw", smbios(true));
        assert_eq!(unknown_vm.detect(), Some(VM_OTHER));

        // SMBIOS overrides the product name for EC2.
        let metal = FakeSystem::new()
            .file("/sys/class/dmi/id/sys_vendor", "Amazon EC2\n")
            .file("/sys/class/dmi/id/product_name", "m5.large\n")
            .file("/sys/firmware/dmi/entries/0-0/raw", smbios(false));
        assert_eq!(metal.detect(), None);
    }

    #[test]
    fn cpuid_beats_weaker_evidence() {
        let system = FakeSystem::new().cpuid(|| Some("kvm"));
        assert_eq!(system.detect(), Some("kvm"));

        // Hyper-V in CPUID is only a fallback when DMI knows better.
        let system = FakeSystem::new()
            .cpuid(|| Some("microsoft"))
            .file("/sys/class/dmi/id/sys_vendor", "VMware, Inc.\n");
        assert_eq!(system.detect(), Some("vmware"));
        assert_eq!(
            FakeSystem::new().cpuid(|| Some("microsoft")).detect(),
            Some("microsoft")
        );
    }

    #[test]
    fn google_compute_engine() {
        let system = FakeSystem::new()
            .cpuid(|| Some("kvm"))
            .file("/sys/class/dmi/id/product_name", "Google Compute Engine\n");
        assert_eq!(system.detect(), Some("google"));
    }

    #[test]
    fn xen_guest_and_dom0() {
        let guest = FakeSystem::new().file("/proc/xen/capabilities", "");
        assert_eq!(guest.detect(), Some("xen"));

        let dom0 = FakeSystem::new().file("/proc/xen/capabilities", "control_d\n");
        assert_eq!(dom0.detect(), None);

        let dom0_by_features = FakeSystem::new()
            .file("/proc/xen/capabilities", "")
            .file("/sys/hypervisor/properties/features", "00000800\n");
        assert_eq!(dom0_by_features.detect(), None);
    }

    #[test]
    fn hypervisor_type_file() {
        let system = FakeSystem::new().file("/sys/hypervisor/type", "xen\n");
        assert_eq!(system.detect(), Some("xen"));
        let system = FakeSystem::new().file("/sys/hypervisor/type", "other\n");
        assert_eq!(system.detect(), Some(VM_OTHER));
    }

    #[test]
    fn user_mode_linux() {
        let system = FakeSystem::new().file(
            "/proc/cpuinfo",
            "processor\t: 0\nvendor_id\t: User Mode Linux\n",
        );
        assert_eq!(system.detect(), Some("uml"));
    }

    #[test]
    fn metal_product_names() {
        assert!(is_metal_product("m5.metal"));
        assert!(is_metal_product("c7g.metal-48xl"));
        assert!(!is_metal_product("m5.large"));
        assert!(!is_metal_product("m5.metalx"));
        assert!(!is_metal_product(""));
    }

    #[test]
    fn cpuid_signatures() {
        assert_eq!(hypervisor_from_signature(b"KVMKVMKVM\0\0\0"), "kvm");
        assert_eq!(hypervisor_from_signature(b"Linux KVM Hv"), "kvm");
        assert_eq!(hypervisor_from_signature(b"Microsoft Hv"), "microsoft");
        assert_eq!(hypervisor_from_signature(b"VMwareVMware"), "vmware");
        assert_eq!(hypervisor_from_signature(b"Apple VZ\0\0\0\0"), "apple");
        assert_eq!(hypervisor_from_signature(b"SomethingNew"), VM_OTHER);
    }

    #[test]
    fn detects_without_panicking_on_this_machine() {
        let result = detect();
        if !cfg!(target_os = "linux") {
            assert_eq!(result, None);
        }
    }
}
