//! Disks: whole block devices from `/sys/class/block`, their mounted
//! filesystems from `/proc/self/mounts`, throughput from `/proc/diskstats`
//! and free space from `statvfs`.
//!
//! A mount is traced back to the hardware it lives on through sysfs: a
//! partition's parent is the directory it sits in, and a device-mapper or md
//! device (LUKS, LVM, RAID) leads through its `slaves` to the partitions
//! under it. On AtlasOS the root filesystem is a composefs overlay on `/`; the
//! partition it comes from is mounted at `/sysroot`, so either marks the root
//! disk.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use super::{elapsed, lines, rate};
use crate::sysfs::{self, HeldFile};

/// `/proc/diskstats` and a block device's `size` count 512-byte sectors,
/// whatever the device's real sector size.
const SECTOR: u64 = 512;

/// How deep a stack of device-mapper and md devices may go before the walk
/// down `slaves` gives up. Real stacks (LUKS on LVM on RAID) are three deep.
const MAX_STACK: usize = 8;

/// One whole disk. Read once; see [`DiskSampler`] for throughput and
/// [`space`] for free space.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Disk {
    /// Kernel name: `nvme0n1`, `sda`, `zram0`.
    pub name: String,
    /// Hardware model, e.g. "Samsung SSD 990 PRO 2TB". `None` for virtual
    /// disks and zram.
    pub model: Option<String>,
    /// Compressed swap in RAM (zram), shown as "Swap".
    pub is_swap: bool,
    /// Holds the root filesystem: the primary disk, listed first.
    pub is_root: bool,
    /// Capacity in bytes.
    pub size: u64,
    /// One mount point per filesystem on this disk, for [`space`]. A
    /// filesystem mounted more than once (btrfs subvolumes) appears once.
    pub mounts: Vec<PathBuf>,
}

impl Disk {
    /// The name to show: "Swap" for zram, otherwise the model, otherwise the
    /// kernel name.
    pub fn label(&self) -> &str {
        if self.is_swap {
            "Swap"
        } else {
            self.model.as_deref().unwrap_or(&self.name)
        }
    }
}

/// Finds the disks: root first, swap last, larger before smaller.
pub fn disks() -> Vec<Disk> {
    let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
    discover(Path::new("/sys/class/block"), &mounts, |source| {
        // /dev/mapper/luks-… and /dev/disk/by-uuid/… are links to /dev/dm-0
        // and the like, whose name is the one sysfs uses.
        let real = fs::canonicalize(source).ok()?;
        Some(real.file_name()?.to_str()?.to_owned())
    })
}

/// Builds the disk list from a `/sys/class/block` directory and the text of
/// `/proc/self/mounts`. `kernel_name` turns a mount source (`/dev/…`) into
/// the block device's kernel name.
pub fn discover(
    class_block: &Path,
    mounts: &str,
    kernel_name: impl Fn(&str) -> Option<String>,
) -> Vec<Disk> {
    let mut disks: Vec<Disk> = fs::read_dir(class_block)
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let dir = class_block.join(&name);
            if ["loop", "ram", "dm-", "md"]
                .iter()
                .any(|p| name.starts_with(p))
                || dir.join("partition").exists()
            {
                return None;
            }
            // An empty card reader or optical drive has nothing to show.
            let size = sysfs::read_uint(dir.join("size"))?.saturating_mul(SECTOR);
            if size == 0 {
                return None;
            }
            let is_swap = name.starts_with("zram");
            let model = if is_swap {
                None
            } else {
                sysfs::read_string(dir.join("device/model"))
                    .map(|m| m.split_whitespace().collect::<Vec<_>>().join(" "))
                    .filter(|m| !m.is_empty())
            };
            Some(Disk {
                name,
                model,
                is_swap,
                size,
                ..Disk::default()
            })
        })
        .collect();

    // Each filesystem once, by its device: its first mount point, and whether
    // any of its mount points is the root.
    let mut filesystems: Vec<(String, PathBuf, bool)> = Vec::new();
    for line in mounts.lines() {
        let mut fields = line.split_ascii_whitespace();
        let (Some(source), Some(target)) = (fields.next(), fields.next()) else {
            continue;
        };
        if !source.starts_with("/dev/") {
            continue;
        }
        let Some(device) = kernel_name(source) else {
            continue;
        };
        let target = unescape(target);
        let is_root = target == "/" || target == "/sysroot";
        match filesystems.iter_mut().find(|(d, ..)| *d == device) {
            Some(fs) => fs.2 |= is_root,
            None => filesystems.push((device, PathBuf::from(target), is_root)),
        }
    }
    for (device, target, is_root) in filesystems {
        let mut owners = Vec::new();
        owning_disks(class_block, &device, 0, &mut owners);
        for disk in disks.iter_mut().filter(|d| owners.contains(&d.name)) {
            disk.is_root |= is_root;
            if !disk.mounts.contains(&target) {
                disk.mounts.push(target.clone());
            }
        }
    }

    disks.sort_by(|a, b| {
        rank(a)
            .cmp(&rank(b))
            .then(b.size.cmp(&a.size))
            .then_with(|| a.name.cmp(&b.name))
    });
    disks
}

/// Root first, swap last, everything else between.
fn rank(d: &Disk) -> u8 {
    match (d.is_root, d.is_swap) {
        (true, _) => 0,
        (_, true) => 2,
        _ => 1,
    }
}

/// Adds the whole disks `device` lives on to `out`: itself if it is one, its
/// parent if it is a partition, and for a device-mapper or md device the
/// disks under each of its slaves.
fn owning_disks(class_block: &Path, device: &str, depth: usize, out: &mut Vec<String>) {
    let dir = class_block.join(device);
    if dir.join("partition").exists() {
        // /sys/class/block/nvme0n1p3 links to …/nvme0n1/nvme0n1p3.
        let parent = fs::canonicalize(&dir)
            .ok()
            .and_then(|p| Some(p.parent()?.file_name()?.to_str()?.to_owned()));
        out.extend(parent);
        return;
    }
    let slaves: Vec<String> = fs::read_dir(dir.join("slaves"))
        .into_iter()
        .flatten()
        .flatten()
        .filter_map(|e| e.file_name().into_string().ok())
        .collect();
    if slaves.is_empty() || depth >= MAX_STACK {
        out.push(device.to_owned());
        return;
    }
    for slave in slaves {
        owning_disks(class_block, &slave, depth + 1, out);
    }
}

/// Undoes the octal escapes `/proc/self/mounts` writes for a space, tab,
/// newline or backslash in a path (`\040` is a space).
fn unescape(field: &str) -> String {
    let b = field.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && let Some(oct) = b.get(i + 1..i + 4)
            && oct.iter().all(|c| (b'0'..=b'7').contains(c))
        {
            let v = oct.iter().fold(0u32, |v, c| v * 8 + u32::from(c - b'0'));
            if let Ok(v) = u8::try_from(v) {
                out.push(v);
                i += 4;
                continue;
            }
        }
        out.push(b[i]);
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Used and free bytes on a disk's filesystems.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Space {
    pub used: u64,
    /// What an ordinary user can still write (`f_bavail`), which leaves out
    /// the blocks reserved for root.
    pub free: u64,
}

/// Adds up used and free space over `mounts`. `None` if none of them
/// answered.
///
/// `statvfs` can block for as long as the filesystem takes to answer, so
/// call this on the sampling thread, never while holding a lock, and not
/// every tick: capacity moves slowly.
pub fn space(mounts: &[PathBuf]) -> Option<Space> {
    let mut total: Option<Space> = None;
    for mount in mounts {
        let Ok(st) = rustix::fs::statvfs(mount) else {
            continue;
        };
        let unit = st.f_frsize;
        let s = total.get_or_insert_default();
        s.used = s
            .used
            .saturating_add(st.f_blocks.saturating_sub(st.f_bfree).saturating_mul(unit));
        s.free = s.free.saturating_add(st.f_bavail.saturating_mul(unit));
    }
    total
}

/// One disk's throughput.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DiskIo {
    /// Bytes read and written since boot.
    pub read_total: u64,
    pub write_total: u64,
    /// Bytes per second since the previous sample.
    pub read_rate: f64,
    pub write_rate: f64,
}

/// Samples `/proc/diskstats` for a fixed list of disks.
#[derive(Debug)]
pub struct DiskSampler {
    diskstats: Option<HeldFile>,
    names: Vec<String>,
    io: Vec<DiskIo>,
    last: Instant,
}

impl DiskSampler {
    /// Samples `disks`, in that order. Takes the first counters.
    pub fn new(disks: &[Disk]) -> Self {
        Self::open(Path::new("/proc/diskstats"), disks)
    }

    fn open(diskstats: &Path, disks: &[Disk]) -> Self {
        let mut sampler = Self {
            diskstats: HeldFile::with_capacity(diskstats, 8192),
            names: disks.iter().map(|d| d.name.clone()).collect(),
            io: vec![DiskIo::default(); disks.len()],
            last: Instant::now(),
        };
        sampler.read(None);
        sampler
    }

    /// Reads each disk's counters and its rates since the last sample, in the
    /// order the disks were given.
    pub fn sample(&mut self) -> &[DiskIo] {
        let seconds = elapsed(&mut self.last);
        self.read(Some(seconds));
        &self.io
    }

    fn read(&mut self, seconds: Option<f64>) {
        for io in &mut self.io {
            io.read_rate = 0.0;
            io.write_rate = 0.0;
        }
        let Some(data) = self.diskstats.as_mut().and_then(HeldFile::bytes) else {
            return;
        };
        for line in lines(data) {
            let (Some(name), Some(read), Some(written)) = (
                sysfs::field(line, 2),
                sysfs::field(line, 5).and_then(sysfs::parse_uint),
                sysfs::field(line, 9).and_then(sysfs::parse_uint),
            ) else {
                continue;
            };
            let Some(i) = self.names.iter().position(|n| n.as_bytes() == name) else {
                continue;
            };
            let io = &mut self.io[i];
            let (read, written) = (read.saturating_mul(SECTOR), written.saturating_mul(SECTOR));
            if let Some(s) = seconds {
                io.read_rate = rate(read, io.read_total, s);
                io.write_rate = rate(written, io.write_total, s);
            }
            io.read_total = read;
            io.write_total = written;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::symlink;

    const DISKSTATS: &str = include_str!("../../tests/fixtures/diskstats");
    const MOUNTS: &str = include_str!("../../tests/fixtures/mounts");
    const MOUNTS_KINOITE: &str = include_str!("../../tests/fixtures/mounts_kinoite");

    /// A fake /sys/class/block: devices under `devices/`, linked from
    /// `class/` the way sysfs links them.
    struct Tree(tempfile::TempDir);

    impl Tree {
        fn new() -> Self {
            let t = Self(tempfile::tempdir().unwrap());
            fs::create_dir_all(t.class()).unwrap();
            t
        }

        fn class(&self) -> PathBuf {
            self.0.path().join("class")
        }

        fn disk(&self, name: &str, sectors: u64, model: Option<&str>) -> &Self {
            let dir = self.0.path().join("devices").join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("size"), format!("{sectors}\n")).unwrap();
            if let Some(m) = model {
                fs::create_dir_all(dir.join("device")).unwrap();
                fs::write(dir.join("device/model"), format!("{m}\n")).unwrap();
            }
            symlink(&dir, self.class().join(name)).unwrap();
            self
        }

        fn partition(&self, disk: &str, name: &str) -> &Self {
            let dir = self.0.path().join("devices").join(disk).join(name);
            fs::create_dir_all(&dir).unwrap();
            fs::write(dir.join("partition"), "1\n").unwrap();
            fs::write(dir.join("size"), "100\n").unwrap();
            symlink(&dir, self.class().join(name)).unwrap();
            self
        }

        /// A device-mapper device over `slaves`.
        fn mapper(&self, name: &str, slaves: &[&str]) -> &Self {
            let dir = self.0.path().join("devices/virtual").join(name);
            fs::create_dir_all(dir.join("slaves")).unwrap();
            fs::write(dir.join("size"), "100\n").unwrap();
            for s in slaves {
                symlink(self.class().join(s), dir.join("slaves").join(s)).unwrap();
            }
            symlink(&dir, self.class().join(name)).unwrap();
            self
        }
    }

    fn plain_names(source: &str) -> Option<String> {
        Some(source.strip_prefix("/dev/")?.to_owned())
    }

    /// This machine, recorded: btrfs root and /home on nvme1n1p3, /boot and
    /// the ESP beside it, a second NVMe drive with nothing mounted, zram swap.
    #[test]
    fn discovers_the_recorded_machine() {
        let t = Tree::new();
        t.disk("nvme0n1", 3_907_029_168, Some("Samsung SSD 990 PRO 2TB   "))
            .disk("nvme1n1", 1_953_525_168, Some("WD_BLACK  SN850X 1000GB"))
            .disk("zram0", 16_777_216, None)
            .disk("loop0", 100, None)
            .disk("sr0", 0, Some("DVD-RW"));
        for p in ["nvme1n1p1", "nvme1n1p2", "nvme1n1p3"] {
            t.partition("nvme1n1", p);
        }
        t.partition("nvme0n1", "nvme0n1p1");

        let disks = discover(&t.class(), MOUNTS, plain_names);
        let names: Vec<_> = disks.iter().map(|d| d.name.as_str()).collect();
        assert_eq!(names, ["nvme1n1", "nvme0n1", "zram0"]);

        let root = &disks[0];
        assert!(root.is_root);
        assert_eq!(root.label(), "WD_BLACK SN850X 1000GB");
        assert_eq!(root.size, 1_953_525_168 * 512);
        // /home is the same btrfs as /, so only / is listed.
        assert_eq!(
            root.mounts,
            [Path::new("/"), Path::new("/boot"), Path::new("/boot/efi")]
        );

        assert!(!disks[1].is_root);
        assert!(disks[1].mounts.is_empty());
        assert_eq!(disks[1].label(), "Samsung SSD 990 PRO 2TB");

        assert!(disks[2].is_swap);
        assert_eq!(disks[2].label(), "Swap");
    }

    /// AtlasOS: composefs on /, the btrfs root at /sysroot through LUKS.
    #[test]
    fn finds_the_root_disk_on_kinoite_with_luks() {
        let t = Tree::new();
        t.disk("vda", 104_857_600, None)
            .disk("vdb", 2_097_152, None);
        for p in ["vda1", "vda2", "vda3"] {
            t.partition("vda", p);
        }
        t.mapper("dm-0", &["vda3"]);
        let names = |source: &str| {
            let name = source.strip_prefix("/dev/")?;
            Some(
                if name.starts_with("mapper/luks-") {
                    "dm-0"
                } else {
                    name
                }
                .to_owned(),
            )
        };
        let disks = discover(&t.class(), MOUNTS_KINOITE, names);
        assert_eq!(disks.len(), 2);
        let root = &disks[0];
        assert_eq!(root.name, "vda");
        assert!(root.is_root);
        assert_eq!(root.label(), "vda");
        assert_eq!(
            root.mounts,
            [
                Path::new("/sysroot"),
                Path::new("/boot"),
                Path::new("/boot/efi")
            ]
        );
        assert!(!disks[1].is_root);
    }

    /// LVM over two disks: a filesystem on it belongs to both.
    #[test]
    fn a_stack_spanning_disks_counts_for_each() {
        let t = Tree::new();
        t.disk("sda", 1000, None).disk("sdb", 2000, None);
        t.partition("sda", "sda1").partition("sdb", "sdb1");
        t.mapper("dm-1", &["sda1", "sdb1"])
            .mapper("dm-2", &["dm-1"]);
        let disks = discover(&t.class(), "/dev/dm-2 /data xfs rw 0 0\n", plain_names);
        assert!(disks.iter().all(|d| d.mounts == [Path::new("/data")]));
    }

    #[test]
    fn unescapes_mount_points() {
        assert_eq!(
            unescape(r"/run/media/me/My\040Disk"),
            "/run/media/me/My Disk"
        );
        assert_eq!(unescape(r"/a\011b\134c"), "/a\tb\\c");
        assert_eq!(unescape(r"/odd\04"), r"/odd\04");
        assert_eq!(unescape(r"/odd\999"), r"/odd\999");
        assert_eq!(unescape(r"/big\777"), r"/big\777");
        assert_eq!(unescape("/"), "/");
    }

    #[test]
    fn sampler_turns_sectors_into_rates() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("diskstats");
        fs::write(&path, DISKSTATS).unwrap();
        let disks = [
            Disk {
                name: "nvme1n1".into(),
                ..Disk::default()
            },
            Disk {
                name: "gone".into(),
                ..Disk::default()
            },
        ];
        let mut s = DiskSampler::open(&path, &disks);
        let first = s.io[0];
        assert_eq!(first.read_total, 1_262_345_056 * 512);
        assert_eq!(first.write_total, 2_695_879_898 * 512);

        // 2048 more sectors read; the second disk isn't in the file at all.
        fs::write(
            &path,
            "259 5 nvme1n1 1 0 1262347104 0 1 0 2695879898 0 0 0 0 0 0 0 0 0 0\n",
        )
        .unwrap();
        let io = s.sample().to_vec();
        assert_eq!(io.len(), 2);
        assert_eq!(io[0].read_total, (1_262_345_056 + 2048) * 512);
        assert!(io[0].read_rate > 0.0);
        assert_eq!(io[0].write_rate, 0.0);
        assert_eq!(io[1], DiskIo::default());

        // A counter that went backwards gives no rate, not a huge one.
        fs::write(&path, "259 5 nvme1n1 1 0 5 0 1 0 5 0\n").unwrap();
        let io = s.sample();
        assert_eq!((io[0].read_rate, io[0].write_rate), (0.0, 0.0));
    }

    // Live tests: invariants only. CI's container has an overlay root and
    // may have no block devices at all.

    #[test]
    fn live_disks_hang_together() {
        let disks = disks();
        // Root may span disks (RAID, LVM); either way it sorts first.
        let roots = disks.iter().take_while(|d| d.is_root).count();
        assert!(
            disks[roots..].iter().all(|d| !d.is_root),
            "a root disk sorted late"
        );
        for d in &disks {
            assert!(d.size > 0);
            assert!(!d.label().is_empty());
            assert!(!(d.is_swap && d.is_root));
            if let Some(s) = space(&d.mounts) {
                assert!(s.used > 0 || s.free > 0, "{}: an empty filesystem", d.name);
            }
        }
        let mut sampler = DiskSampler::new(&disks);
        let io = sampler.sample();
        assert_eq!(io.len(), disks.len());
        assert!(io.iter().all(|i| i.read_rate >= 0.0 && i.write_rate >= 0.0));
    }

    /// When / is a real block device (not a container's overlay), its disk is
    /// found and marked.
    #[test]
    fn live_root_disk_is_found() {
        let mounts = fs::read_to_string("/proc/self/mounts").unwrap_or_default();
        let root_on_device = mounts.lines().any(|l| {
            let mut f = l.split_ascii_whitespace();
            let (src, target) = (f.next().unwrap_or(""), f.next().unwrap_or(""));
            src.starts_with("/dev/") && (target == "/" || target == "/sysroot")
        });
        if !root_on_device {
            return;
        }
        assert!(
            disks().first().is_some_and(|d| d.is_root),
            "no root disk found"
        );
    }

    #[test]
    fn space_of_nothing_is_none() {
        assert_eq!(space(&[]), None);
        assert_eq!(space(&[PathBuf::from("/nonexistent/atlas")]), None);
        let tmp = space(&[std::env::temp_dir()]).unwrap();
        assert!(tmp.used > 0 || tmp.free > 0);
    }
}
