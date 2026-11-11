//! Identity of the storage behind a mount, which sysinfo does not expose, and of the file system
//! mounted there. Mounts that show the same storage report the same capacity, which is to be
//! counted once.

#[cfg(any(target_os = "linux", test))]
use std::collections::HashMap;
use std::ffi::OsString;
use std::path::Path;
#[cfg(any(target_os = "linux", test))]
use std::path::PathBuf;
use sysinfo::Disk;

/// The storage behind a mount.
#[derive(Debug)]
pub(crate) struct Storage {
    pub(crate) id: OsString,
    /// File system type, such as `ext4`.
    pub(crate) file_system: OsString,
    /// The device of the file system, which tells mounts of one file system apart from others
    /// when their [`id`](Self::id)s differ: when it is mounted under another name too, such as
    /// `/dev/root`.
    ///
    /// `None` on Windows, where this is the volume serial number, which cloned volumes share;
    /// the volume GUID path is already the same at every path of a volume.
    pub(crate) device: Option<u64>,
}

/// Identifies the storage that each of the disks shows. That is the file system visible at its
/// mount point now, whose capacity sysinfo reports for every disk listed there, including
/// those hidden by a later mount at the point, and those mounted since the list was made.
///
/// - Linux: the mounted device, by the name it was mounted under, or the dataset of a ZFS
///   pool. Bind mounts and btrfs subvolumes repeat the device of the file system. An overlay
///   shows the storage of its upper layer, when that is mounted here too; otherwise it stands
///   for itself.
/// - Windows: the volume GUID path, the same for every path a volume is mounted at.
/// - macOS: the APFS container, whose volumes share its capacity, or else the device.
///
/// Falls back to what sysinfo listed when the platform cannot tell.
pub(crate) fn identify(disks: &[Disk]) -> Vec<Storage> {
    #[cfg(target_os = "linux")]
    if needs_mount_table(disks) {
        let mountinfo = std::fs::read_to_string("/proc/self/mountinfo").unwrap_or_default();
        let mounts = parse_mounts(&mountinfo);
        let table = MountTable::new(&mounts);
        return disks
            .iter()
            .map(|disk| {
                let names = table
                    .shown(disk.mount_point(), |path| path.exists())
                    .map_or_else(
                        || mounted(disk),
                        |mount| (mount.source.into(), mount.file_system.into()),
                    );
                storage(disk, names)
            })
            .collect();
    }
    disks
        .iter()
        .map(|disk| storage(disk, mounted(disk)))
        .collect()
}

/// Whether the disks need the mount table to tell their storage: an overlay does, and so do
/// disks at one point, of which sysinfo cannot tell the visible one. The table is long on a
/// host running containers.
#[cfg(target_os = "linux")]
fn needs_mount_table(disks: &[Disk]) -> bool {
    let mut mount_points = std::collections::HashSet::new();
    disks
        .iter()
        .any(|disk| disk.file_system() == "overlay" || !mount_points.insert(disk.mount_point()))
}

/// The storage at the mount point of `disk`, by the name and file system type given.
fn storage(disk: &Disk, (id, file_system): (OsString, OsString)) -> Storage {
    Storage {
        id,
        file_system,
        device: device(disk.mount_point()),
    }
}

/// The device of the file system at `mount_point` now. sysinfo has the one there when it
/// listed the disk, which a later mount at the point changes.
#[cfg(unix)]
fn device(mount_point: &Path) -> Option<u64> {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(mount_point)
        .ok()
        .map(|metadata| metadata.dev())
}

#[cfg(not(unix))]
fn device(_: &Path) -> Option<u64> {
    None
}

/// The name of the storage at the mount point of `disk`, and its file system type. Linux
/// tells only from the mount table, see [`identify`].
#[cfg(not(any(windows, target_os = "macos")))]
fn mounted(disk: &Disk) -> (OsString, OsString) {
    (disk.name().to_owned(), disk.file_system().to_owned())
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
}

/// Parses `/proc/self/mountinfo`, skipping lines that are not mounts.
#[cfg(any(target_os = "linux", test))]
fn parse_mounts(mountinfo: &str) -> Vec<Mount<'_>> {
    mountinfo.lines().filter_map(Mount::parse).collect()
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

/// The mount table in mount order, with an index of its mount points.
#[cfg(any(target_os = "linux", test))]
struct MountTable<'m, 'a> {
    mounts: &'m [Mount<'a>],
    /// The mounts at each point in mount order, of which the last is the visible one. Looking
    /// mounts up here keeps the work per disk from growing with the table, which is long on a
    /// host running containers.
    at: HashMap<&'m Path, Vec<usize>>,
}

#[cfg(any(target_os = "linux", test))]
impl<'m, 'a> MountTable<'m, 'a> {
    fn new(mounts: &'m [Mount<'a>]) -> Self {
        let mut at = HashMap::<_, Vec<_>>::new();
        for (i, mount) in mounts.iter().enumerate() {
            at.entry(mount.mount_point.as_path()).or_default().push(i);
        }
        Self { mounts, at }
    }

    /// Returns the mount whose storage `mount_point` shows, if anything is mounted there: the
    /// mount visible at the point, or, if that is an overlay, the mount holding the layer
    /// whose capacity it reports. `exists` tells whether a path exists here: an overlay
    /// mounted from another mount namespace, such as the host's when this runs in a
    /// container, names layers that do not, and the mounts holding them are not visible. Such
    /// an overlay stands for itself.
    fn shown(&self, mount_point: &Path, exists: impl Fn(&Path) -> bool) -> Option<&'m Mount<'a>> {
        let &i = self.at.get(mount_point)?.last()?;
        let visible = &self.mounts[i];
        if visible.file_system != "overlay" {
            return Some(visible);
        }
        let backing = visible
            .overlay_layer()
            .filter(|layer| exists(layer))
            .and_then(|layer| {
                // The innermost mount around the layer: the last at the nearest point that
                // has one other than the overlay itself.
                layer.ancestors().find_map(|point| {
                    let at = self.at.get(point)?;
                    at.iter().rev().copied().find(|&j| j != i)
                })
            });
        Some(backing.map_or(visible, |j| &self.mounts[j]))
    }
}

#[cfg(windows)]
fn mounted(disk: &Disk) -> (OsString, OsString) {
    let id = volume_guid_path(disk.mount_point()).unwrap_or_else(|| disk.name().to_owned());
    (id, disk.file_system().to_owned())
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
fn mounted(disk: &Disk) -> (OsString, OsString) {
    match mounted_file_system(disk.mount_point()) {
        Some((device, file_system)) => {
            let id = if file_system == "apfs" {
                apfs_container(&device)
            } else {
                &device
            };
            (id.into(), file_system.into())
        }
        None => (disk.name().to_owned(), disk.file_system().to_owned()),
    }
}

/// Returns the device the file system visible at `mount_point` is mounted from, such as
/// `/dev/disk3s5`, and its type, such as `apfs`.
#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn mounted_file_system(mount_point: &Path) -> Option<(String, String)> {
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
    let text = |field: &[libc::c_char]| {
        let bytes = field.iter().map(|&c| c as u8).take_while(|&c| c != 0);
        String::from_utf8(bytes.collect()).ok()
    };
    Some((text(&stat.f_mntfromname)?, text(&stat.f_fstypename)?))
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
        let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing()).unwrap();
        let storages = identify(&disks);
        assert_eq!(storages.len(), disks.len());
        for (disk, storage) in disks.iter().zip(storages) {
            assert!(!storage.id.is_empty(), "{:?}", disk.mount_point());
            // sysinfo took the device when it listed the disk, and nothing was mounted since.
            if cfg!(unix) {
                assert_eq!(storage.device, disk.id(), "{:?}", disk.mount_point());
            }
        }
    }

    /// Returns the source and file system of the mount whose storage `mount_point` shows,
    /// with every path existing.
    fn shown<'a>(mounts: &[Mount<'a>], mount_point: &str) -> Option<(&'a str, &'a str)> {
        MountTable::new(mounts)
            .shown(Path::new(mount_point), |_| true)
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
        let host = parse_mounts(
            "25 1 8:1 / / rw,relatime shared:1 - ext4 /dev/sda1 rw\n\
             153 25 0:38 /docker /var/lib/docker rw,noatime shared:69 - btrfs /dev/vdb1 rw,subvol=/docker\n\
             72 153 0:53 / /var/lib/docker/rootfs/overlayfs/3b2b rw,relatime shared:24 - overlay overlay rw,lowerdir=/var/lib/docker/snapshots/1/fs:/var/lib/docker/snapshots/2/fs,upperdir=/var/lib/docker/snapshots/3/fs,workdir=/var/lib/docker/snapshots/3/work\n\
             80 25 0:54 / /merged rw - overlay overlay rw,lowerdir=/srv/lower,upperdir=/srv/upper,workdir=/srv/work\n\
             81 25 0:55 / /readonly ro - overlay overlay ro,lowerdir=/srv/top:/var/lib/docker/bottom",
        );
        assert_eq!(
            shown(&host, "/var/lib/docker/rootfs/overlayfs/3b2b"),
            Some(("/dev/vdb1", "btrfs"))
        );
        assert_eq!(shown(&host, "/merged"), Some(("/dev/sda1", "ext4")));
        // The top lower layer, without an upper one.
        assert_eq!(shown(&host, "/readonly"), Some(("/dev/sda1", "ext4")));
        // Not overlays, or not mounted.
        assert_eq!(
            shown(&host, "/var/lib/docker"),
            Some(("/dev/vdb1", "btrfs"))
        );
        assert_eq!(shown(&host, "/elsewhere"), None);
    }

    #[test]
    fn finds_the_upper_layer_on_a_ram_disk() {
        // A live system, whose RAM disk sysinfo does not list as a disk: the overlay stays.
        let live = parse_mounts(
            "93 58 0:20 / / rw shared:4 - overlay overlay rw,lowerdir=/rofs,upperdir=/cow/upper,workdir=/cow/work\n\
             94 93 0:44 / /cow rw shared:5 - tmpfs tmpfs rw\n\
             95 93 7:0 / /rofs ro shared:6 - squashfs /dev/loop0 ro",
        );
        assert_eq!(shown(&live, "/"), Some(("tmpfs", "tmpfs")));
    }

    #[test]
    fn leaves_overlays_whose_layers_are_not_here() {
        // In a container, the overlay names layers on the host.
        let container = parse_mounts(
            "207 75 0:53 / / rw,relatime - overlay overlay rw,lowerdir=/var/lib/docker/snapshots/1/fs,upperdir=/var/lib/docker/snapshots/3/fs,workdir=/var/lib/docker/snapshots/3/work\n\
             217 207 0:38 /docker/containers/f1ab/hosts /etc/hosts rw,noatime - btrfs /dev/vdb1 rw,subvol=/docker",
        );
        let table = MountTable::new(&container);
        let source = |exists: bool| {
            let shown = table.shown(Path::new("/"), |_| exists);
            shown.map(|mount| mount.source)
        };
        assert_eq!(source(false), Some("overlay"));
        // Nor would any mount here hold them.
        assert_eq!(source(true), Some("overlay"));
    }

    #[test]
    fn takes_the_visible_mount_of_a_point() {
        let mounts = parse_mounts(
            "25 1 8:1 / / rw - ext4 /dev/sda1 rw\n\
             30 25 8:17 / /data rw - ext4 /dev/sdb1 rw\n\
             31 25 8:33 / /data rw - xfs /dev/sdc1 rw\n\
             40 25 0:60 / /merged rw - overlay overlay rw,upperdir=/data/upper,workdir=/data/work\n\
             41 25 0:61 / /merged rw - ext4 /dev/sdd1 rw",
        );
        // sysinfo lists a disk for each of the mounts at `/data`, which all show `/dev/sdc1`.
        assert_eq!(shown(&mounts, "/data"), Some(("/dev/sdc1", "xfs")));
        // An overlay hidden by a later mount is not what `/merged` shows.
        assert_eq!(shown(&mounts, "/merged"), Some(("/dev/sdd1", "ext4")));
        let mounts = &mounts[..4];
        assert_eq!(shown(mounts, "/merged"), Some(("/dev/sdc1", "xfs")));
    }

    #[test]
    fn finds_the_layer_in_the_mount_an_overlay_hides() {
        // Nothing else holds the upper layer of the overlay at `/` but the file system
        // mounted there before it.
        let mounts = parse_mounts(
            "25 1 8:1 / / rw - ext4 /dev/sda1 rw\n\
             26 25 0:40 / / rw - overlay overlay rw,lowerdir=/lower,upperdir=/upper,workdir=/work",
        );
        assert_eq!(shown(&mounts, "/"), Some(("/dev/sda1", "ext4")));
    }

    #[test]
    fn takes_the_visible_mount_of_a_point_mounted_again() {
        // `/dev/sdb1` mounted again over `/dev/sdc1`: sysinfo merges the two disks of
        // `/dev/sdb1` into the first, so its last disk at `/data` is the hidden one.
        let mounts = parse_mounts(
            "30 25 8:17 / /data rw - ext4 /dev/sdb1 rw\n\
             31 30 8:33 / /data rw - xfs /dev/sdc1 rw\n\
             32 31 8:17 / /data rw - ext4 /dev/sdb1 rw",
        );
        assert_eq!(shown(&mounts, "/data"), Some(("/dev/sdb1", "ext4")));
    }

    #[test]
    fn takes_the_visible_mount_of_a_zfs_mount_point() {
        // A file system hidden by a dataset, which is counted with its pool.
        let mounts = parse_mounts(
            "30 25 8:17 / /rpool/data rw - ext4 /dev/sdb1 rw\n\
             31 30 0:50 / /rpool/data rw - zfs rpool/data rw,xattr,noacl",
        );
        assert_eq!(shown(&mounts, "/rpool/data"), Some(("rpool/data", "zfs")));
        // A dataset hidden by a file system, which is not counted with the pool.
        let mut mounts = mounts;
        mounts.reverse();
        assert_eq!(shown(&mounts, "/rpool/data"), Some(("/dev/sdb1", "ext4")));
    }

    /// The system and data volumes of macOS are in one APFS container.
    #[cfg(target_os = "macos")]
    #[test]
    fn puts_the_system_volumes_in_one_container() {
        let disks = Disks::new_with_refreshed_list_specifics(DiskRefreshKind::nothing()).unwrap();
        let id_at = |mount_point: &str| {
            let disk = disks.iter().find(|d| d.mount_point() == mount_point);
            disk.map(|disk| mounted(disk).0)
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
