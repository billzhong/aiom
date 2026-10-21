//! Identity of the storage behind a mount, which sysinfo does not expose. Mounts that show the
//! same storage report the same capacity, which is to be counted once.

use std::ffi::OsString;
#[cfg(any(target_os = "linux", test))]
use std::path::{Path, PathBuf};
use sysinfo::Disk;

/// The storage behind a mount.
#[derive(Debug)]
pub(crate) struct Storage {
    pub(crate) id: OsString,
    /// File system type, such as `ext4`.
    pub(crate) file_system: OsString,
}

/// Identifies the storage behind each of the disks:
///
/// - Linux: the mounted device, or the dataset of a ZFS pool. Bind mounts and btrfs
///   subvolumes repeat the device of the file system. An overlay shows the storage of its
///   upper layer, when that is mounted here too; otherwise it stands for itself.
/// - Windows: the volume GUID path, the same for every path a volume is mounted at.
/// - macOS: the APFS container, whose volumes share its capacity, or else the device.
///
/// Falls back to the volume name when the platform cannot tell.
pub(crate) fn identify(disks: &[Disk]) -> Vec<Storage> {
    let own = |disk: &Disk| Storage {
        id: id(disk),
        file_system: disk.file_system().to_owned(),
    };
    // Only overlays need the mount table, which is long on a host running containers.
    #[cfg(target_os = "linux")]
    if disks.iter().any(|disk| disk.file_system() == "overlay") {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let mounts: Vec<_> = mountinfo.lines().filter_map(Mount::parse).collect();
        return disks
            .iter()
            .map(|disk| {
                overlay_backing(&mounts, disk.mount_point(), |path| path.exists())
                    .map_or_else(|| own(disk), Mount::storage)
            })
            .collect();
    }
    disks.iter().map(own).collect()
}

#[cfg(not(any(windows, target_os = "macos")))]
fn id(disk: &Disk) -> OsString {
    disk.name().to_owned()
}

/// One line of `/proc/self/mountinfo`, see proc_pid_mountinfo(5).
#[cfg(any(target_os = "linux", test))]
#[derive(Debug)]
struct Mount<'a> {
    mount_point: PathBuf,
    file_system: &'a str,
    /// As in `/proc/mounts`, where sysinfo reads the name of a disk from.
    source: &'a str,
    super_options: &'a str,
}

#[cfg(any(target_os = "linux", test))]
impl<'a> Mount<'a> {
    fn parse(line: &'a str) -> Option<Self> {
        let (mount, file_system) = line.split_once(" - ")?;
        let mut file_system = file_system.split(' ');
        Some(Self {
            mount_point: unescape(mount.split(' ').nth(4)?)?.into(),
            file_system: file_system.next()?,
            source: file_system.next()?,
            super_options: file_system.next().unwrap_or_default(),
        })
    }

    /// The layer of an overlay whose capacity it reports: the upper layer, or the top lower
    /// layer of a read-only overlay.
    fn overlay_layer(&self) -> Option<PathBuf> {
        let option = |name| {
            self.super_options
                .split(',')
                .find_map(|setting: &str| setting.strip_prefix(name))
        };
        let layer = match option("upperdir=") {
            Some(upper) => upper,
            None => option("lowerdir=")?.split(':').next()?,
        };
        unescape(layer).map(PathBuf::from)
    }

    #[cfg(target_os = "linux")]
    fn storage(&self) -> Storage {
        Storage {
            id: self.source.into(),
            file_system: self.file_system.into(),
        }
    }
}

/// Undoes the octal escapes of the mount table, such as `\040` for a space.
#[cfg(any(target_os = "linux", test))]
fn unescape(field: &str) -> Option<String> {
    let mut bytes = Vec::with_capacity(field.len());
    let mut rest = field.as_bytes();
    while let Some((&byte, tail)) = rest.split_first() {
        let octal = tail
            .get(..3)
            .filter(|digits| digits.iter().all(|d| (b'0'..=b'7').contains(d)));
        match octal {
            Some(digits) if byte == b'\\' => {
                let value = digits
                    .iter()
                    .fold(0u32, |value, d| value * 8 + u32::from(d - b'0'));
                bytes.push(u8::try_from(value).ok()?);
                rest = &tail[3..];
            }
            _ => {
                bytes.push(byte);
                rest = tail;
            }
        }
    }
    String::from_utf8(bytes).ok()
}

/// Returns the mount holding the layer whose capacity the overlay mounted at `mount_point`
/// reports, if that is an overlay. `exists` tells whether a path exists here: an overlay
/// mounted from another mount namespace, such as the host's when this runs in a container,
/// names layers that do not, and the mounts holding them are not visible.
#[cfg(any(target_os = "linux", test))]
fn overlay_backing<'m, 'a>(
    mounts: &'m [Mount<'a>],
    mount_point: &Path,
    exists: impl Fn(&Path) -> bool,
) -> Option<&'m Mount<'a>> {
    // Of the mounts at one point, the last is visible.
    let overlay = mounts.iter().rposition(|m| m.mount_point == mount_point)?;
    if mounts[overlay].file_system != "overlay" {
        return None;
    }
    let layer = mounts[overlay]
        .overlay_layer()
        .filter(|layer| exists(layer))?;
    mounts
        .iter()
        .enumerate()
        .filter(|&(i, m)| i != overlay && layer.starts_with(&m.mount_point))
        // The innermost; `max_by_key` takes the last of equals, the visible one.
        .max_by_key(|(_, m)| m.mount_point.components().count())
        .map(|(_, m)| m)
}

#[cfg(windows)]
fn id(disk: &Disk) -> OsString {
    volume_guid_path(disk.mount_point()).unwrap_or_else(|| disk.name().to_owned())
}

/// Returns the volume GUID path of the volume mounted at `mount_point`, such as
/// `\\?\Volume{26a21bda-a627-11d7-9931-806e6f6e6963}\`.
#[cfg(windows)]
#[allow(unsafe_code)]
fn volume_guid_path(mount_point: &std::path::Path) -> Option<OsString> {
    use std::os::windows::ffi::{OsStrExt, OsStringExt};
    use windows_sys::Win32::Storage::FileSystem::GetVolumeNameForVolumeMountPointW;

    let backslash = u16::from(b'\\');
    let mut path: Vec<u16> = mount_point.as_os_str().encode_wide().collect();
    if path.last() != Some(&backslash) {
        path.push(backslash);
    }
    path.push(0);
    // The largest volume GUID path, terminated.
    let mut name = [0u16; 50];
    // SAFETY: `path` is terminated, and `name` holds as many characters as passed.
    let ok = unsafe {
        GetVolumeNameForVolumeMountPointW(path.as_ptr(), name.as_mut_ptr(), name.len() as u32)
    };
    if ok == 0 {
        return None;
    }
    let len = name.iter().position(|&c| c == 0)?;
    Some(OsString::from_wide(&name[..len]))
}

#[cfg(target_os = "macos")]
fn id(disk: &Disk) -> OsString {
    match mounted_device(disk.mount_point()) {
        Some(device) if disk.file_system() == "apfs" => apfs_container(&device).into(),
        Some(device) => device.into(),
        None => disk.name().to_owned(),
    }
}

/// Returns the device a file system is mounted from, such as `/dev/disk3s5`.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn mounted_device(mount_point: &std::path::Path) -> Option<String> {
    use std::os::unix::ffi::OsStrExt;

    let path = std::ffi::CString::new(mount_point.as_os_str().as_bytes()).ok()?;
    let mut stat = std::mem::MaybeUninit::<libc::statfs>::uninit();
    // SAFETY: `path` is terminated, and `statfs` fills `stat` when it succeeds.
    let stat = unsafe {
        if libc::statfs(path.as_ptr(), stat.as_mut_ptr()) != 0 {
            return None;
        }
        stat.assume_init()
    };
    let device = stat
        .f_mntfromname
        .iter()
        .map(|&c| c as u8)
        .take_while(|&c| c != 0)
        .collect();
    String::from_utf8(device).ok()
}

/// Returns the APFS container of a volume: `/dev/disk3s5`, and its snapshot `/dev/disk3s5s1`,
/// are in the container `/dev/disk3`.
#[cfg(any(target_os = "macos", test))]
fn apfs_container(device: &str) -> &str {
    let Some(rest) = device.strip_prefix("/dev/disk") else {
        return device;
    };
    match rest.bytes().take_while(u8::is_ascii_digit).count() {
        0 => device,
        digits => &device[..device.len() - rest.len() + digits],
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use sysinfo::{DiskRefreshKind, Disks};

    #[test]
    fn finds_the_apfs_container_of_a_volume() {
        assert_eq!(apfs_container("/dev/disk3s5"), "/dev/disk3");
        assert_eq!(apfs_container("/dev/disk3s1s1"), "/dev/disk3");
        assert_eq!(apfs_container("/dev/disk12s2"), "/dev/disk12");
        assert_eq!(apfs_container("/dev/disk"), "/dev/disk");
        assert_eq!(apfs_container("map auto_home"), "map auto_home");
    }

    #[test]
    fn identifies_every_disk() {
        let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing());
        let storages = identify(disks.list());
        assert_eq!(storages.len(), disks.list().len());
        for (disk, storage) in disks.list().iter().zip(storages) {
            assert!(!storage.id.is_empty(), "{:?}", disk.mount_point());
        }
    }

    fn mounts(mountinfo: &str) -> Vec<Mount<'_>> {
        mountinfo.lines().filter_map(Mount::parse).collect()
    }

    /// Returns the source and file system of the mount holding the layer of the overlay at
    /// `mount_point`, with every path existing.
    fn backing<'a>(mounts: &[Mount<'a>], mount_point: &str) -> Option<(&'a str, &'a str)> {
        overlay_backing(mounts, Path::new(mount_point), |_| true)
            .map(|mount| (mount.source, mount.file_system))
    }

    #[test]
    fn parses_mount_table_lines() {
        let mount = Mount::parse(
            "36 35 98:0 /mnt1 /mnt/my\\040disk rw,noatime master:1 shared:2 - ext4 /dev/sda1 rw,errors=continue",
        )
        .unwrap();
        assert_eq!(mount.mount_point, Path::new("/mnt/my disk"));
        assert_eq!((mount.file_system, mount.source), ("ext4", "/dev/sda1"));
        assert_eq!(mount.super_options, "rw,errors=continue");
        assert!(Mount::parse("36 35 98:0 / /mnt rw").is_none());
    }

    #[test]
    fn unescapes_mount_table_fields() {
        assert_eq!(unescape("/mnt/my\\040disk").unwrap(), "/mnt/my disk");
        assert_eq!(unescape("a\\134b\\011c").unwrap(), "a\\b\tc");
        assert_eq!(unescape("/plain\\x").unwrap(), "/plain\\x");
        assert_eq!(unescape("\\04").unwrap(), "\\04");
    }

    #[test]
    fn finds_the_storage_of_container_overlays_on_the_host() {
        let host = mounts(
            "25 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw\n\
             153 25 0:38 /docker /var/lib/docker rw,noatime shared:69 - btrfs /dev/vdb1 rw,subvol=/docker\n\
             72 153 0:53 / /var/lib/docker/rootfs/overlayfs/3b2b rw,relatime shared:24 - overlay overlay rw,lowerdir=/var/lib/docker/snapshots/1/fs:/var/lib/docker/snapshots/2/fs,upperdir=/var/lib/docker/snapshots/3/fs,workdir=/var/lib/docker/snapshots/3/work\n\
             80 25 0:54 / /merged rw - overlay overlay rw,lowerdir=/srv/lower,upperdir=/srv/upper,workdir=/srv/work\n\
             81 25 0:55 / /readonly ro - overlay overlay ro,lowerdir=/srv/top:/var/lib/docker/bottom",
        );
        assert_eq!(
            backing(&host, "/var/lib/docker/rootfs/overlayfs/3b2b"),
            Some(("/dev/vdb1", "btrfs"))
        );
        assert_eq!(backing(&host, "/merged"), Some(("/dev/sda1", "ext4")));
        // The top lower layer, without an upper one.
        assert_eq!(backing(&host, "/readonly"), Some(("/dev/sda1", "ext4")));
        // Not overlays, or not mounted.
        assert_eq!(backing(&host, "/var/lib/docker"), None);
        assert_eq!(backing(&host, "/elsewhere"), None);
    }

    #[test]
    fn finds_the_upper_layer_on_a_ram_disk() {
        // A live system, whose RAM disk sysinfo does not list as a disk: the overlay stays.
        let live = mounts(
            "93 58 0:20 / / rw shared:4 - overlay overlay rw,lowerdir=/rofs,upperdir=/cow/upper,workdir=/cow/work\n\
             94 93 0:44 / /cow rw shared:5 - tmpfs tmpfs rw\n\
             95 93 7:0 / /rofs ro shared:6 - squashfs /dev/loop0 ro",
        );
        assert_eq!(backing(&live, "/"), Some(("tmpfs", "tmpfs")));
    }

    #[test]
    fn leaves_overlays_whose_layers_are_not_here() {
        // In a container, the overlay names layers on the host.
        let container = mounts(
            "207 75 0:53 / / rw,relatime - overlay overlay rw,lowerdir=/var/lib/docker/snapshots/1/fs,upperdir=/var/lib/docker/snapshots/3/fs,workdir=/var/lib/docker/snapshots/3/work\n\
             217 207 0:38 /docker/containers/f1ab/hosts /etc/hosts rw,noatime - btrfs /dev/vdb1 rw,subvol=/docker",
        );
        let root = Path::new("/");
        assert!(overlay_backing(&container, root, |_| false).is_none());
        // Nor would any mount here hold them.
        assert!(overlay_backing(&container, root, |_| true).is_none());
    }

    #[test]
    fn takes_the_visible_mount_of_a_point() {
        let mounts = mounts(
            "25 1 8:1 / / rw - ext4 /dev/sda1 rw\n\
             30 25 8:17 / /data rw - ext4 /dev/sdb1 rw\n\
             31 25 8:33 / /data rw - xfs /dev/sdc1 rw\n\
             40 25 0:60 / /merged rw - overlay overlay rw,upperdir=/data/upper,workdir=/data/work\n\
             41 25 0:61 / /merged rw - ext4 /dev/sdd1 rw",
        );
        // An overlay hidden by a later mount is not what `/merged` shows.
        assert_eq!(backing(&mounts, "/merged"), None);
        let mounts = &mounts[..4];
        assert_eq!(backing(mounts, "/merged"), Some(("/dev/sdc1", "xfs")));
    }

    /// The system and data volumes of macOS are in one APFS container.
    #[cfg(target_os = "macos")]
    #[test]
    fn puts_the_system_volumes_in_one_container() {
        let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing());
        let id_at = |mount_point: &str| {
            let disk = disks.list().iter().find(|d| d.mount_point() == mount_point);
            disk.map(id)
        };
        if let (Some(system), Some(data)) = (id_at("/"), id_at("/System/Volumes/Data")) {
            assert_eq!(system, data);
            assert!(
                system.to_string_lossy().starts_with("/dev/disk"),
                "{system:?}"
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn identifies_volumes_by_guid() {
        let id = volume_guid_path(std::path::Path::new(r"C:\")).unwrap();
        assert!(id.to_string_lossy().starts_with(r"\\?\Volume{"), "{id:?}");
        assert_eq!(volume_guid_path(std::path::Path::new("C:")), Some(id));
    }
}
