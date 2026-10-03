//! The system calls under the process scan: `/proc` held open, files reached
//! with `openat` and a relative path, each read with one `read`.
//!
//! A scan reads three or four files for each of several hundred processes,
//! and walks some of their descriptor tables, every second. What keeps that
//! cheap, all learned on the Go version:
//!
//! - `openat` against a descriptor held on `/proc`, so the kernel doesn't
//!   resolve the mount point again for every file;
//! - one `read` per file. procfs builds these files whole and hands back all
//!   of it at once, so a read that doesn't fill the buffer is the end of the
//!   file; reading again until EOF doubled the reads;
//! - directories listed with `getdents64` into a reused buffer and the names
//!   used as they are, never turned into strings;
//! - a descriptor walk reads each link relative to the opened `fd/`
//!   directory, not by its full path, which made the kernel look the process
//!   up again for every descriptor.

use std::ffi::CStr;
use std::mem::MaybeUninit;
use std::os::fd::OwnedFd;

use rustix::fs::{self as rfs, AtFlags, Mode, OFlags, RawDir, SeekFrom};
use rustix::io::{self as rio, Errno};

use crate::gpu::fdinfo::{self, GpuTime};

/// Room for a `getdents64` batch: about 500 entries of `/proc`.
pub(crate) const DENTS: usize = 16 * 1024;

/// Room for a descriptor's link target. Only its start is looked at
/// (`socket:[`, `/dev/dri/`), so a longer path cut short does no harm.
pub(crate) const LINK: usize = 256;

/// `/proc` held open, with the buffers its reads reuse.
#[derive(Debug)]
pub(crate) struct ProcDir {
    fd: OwnedFd,
    /// `<pid>/<file>` and a NUL, rebuilt for each open.
    path: Vec<u8>,
    /// Every file is read into this; it grows if one doesn't fit.
    buf: Vec<u8>,
}

/// What one walk of a process's descriptors found.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq)]
pub(crate) struct FdWalk {
    /// Descriptors listed, to compare with [`ProcDir::fd_count`] later.
    pub fds: u64,
    pub sockets: u32,
    /// Holds a `/dev/dri` handle: a GPU client, busy or not.
    pub has_drm: bool,
    /// Engine time summed over its distinct DRM clients.
    pub gpu: GpuTime,
}

/// A process's entry in the `/proc` listing.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Entry {
    pub pid: u32,
    /// The inode of `/proc/<pid>`. It stays the same while the process lives
    /// and a new process given the same pid gets a new one, so a pid and inode
    /// already seen are the same process, known without reading anything.
    pub ino: u64,
}

impl ProcDir {
    pub fn open() -> Option<Self> {
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        Some(Self {
            fd: rfs::open("/proc", flags, Mode::empty()).ok()?,
            path: Vec::with_capacity(48),
            buf: vec![0; 8192],
        })
    }

    /// Reads `/proc/<pid><name><suffix>`. The slice borrows the shared
    /// buffer, so it is valid until the next read.
    pub fn read(&mut self, pid: u32, name: &[u8], suffix: &[u8]) -> Result<&[u8], Errno> {
        let path = pid_path(&mut self.path, pid, name, suffix)?;
        let fd = rfs::openat(
            &self.fd,
            path,
            OFlags::RDONLY | OFlags::CLOEXEC,
            Mode::empty(),
        )?;
        read_whole(&fd, &mut self.buf)
    }

    /// Reads `/proc/<pid>/fdinfo/<fd>`, like [`Self::read`].
    pub fn fdinfo(&mut self, pid: u32, fd: u32) -> Result<&[u8], Errno> {
        let mut digits = [0; 10];
        let fd = decimal(fd, &mut digits);
        self.read(pid, b"/fdinfo/", fd)
    }

    /// How many descriptors the process has open: the size `stat` gives its
    /// `fd/` since Linux 6.2. One system call where a walk is one per
    /// descriptor. `None` before 6.2, where the size is 0.
    pub fn fd_count(&mut self, pid: u32) -> Option<u64> {
        let path = pid_path(&mut self.path, pid, b"/fd", b"").ok()?;
        let st = rfs::statat(&self.fd, path, AtFlags::empty()).ok()?;
        u64::try_from(st.st_size).ok().filter(|&n| n > 0)
    }

    /// Appends every process in `/proc` to `pids`, in the order `/proc` lists
    /// them (ascending pid). An error means `/proc` couldn't be listed at
    /// all, which the caller mustn't take for a machine with no processes.
    pub fn list_pids(
        &mut self,
        dents: &mut [MaybeUninit<u8>],
        pids: &mut Vec<Entry>,
    ) -> Result<(), Errno> {
        // The held descriptor was read to the end last tick.
        rfs::seek(&self.fd, SeekFrom::Start(0))?;
        let mut dir = RawDir::new(&self.fd, dents);
        while let Some(entry) = dir.next() {
            // Only /proc/<pid> is a process; the named entries beside them
            // start with a letter.
            let entry = entry?;
            if let Some(pid) = parse_pid(entry.file_name().to_bytes()) {
                pids.push(Entry {
                    pid,
                    ino: entry.ino(),
                });
            }
        }
        Ok(())
    }

    /// Walks a process's open descriptors once, counting sockets and summing
    /// GPU engine time as asked. The two want the same `readlink` of the same
    /// entries, so they share the walk. One DRM client can be reachable
    /// through several descriptors and is counted once, by `drm-client-id`.
    ///
    /// `drm_fds` is set to one descriptor per GPU client found, so the next
    /// tick can re-read just those ([`Self::drm_usage`]) instead of walking
    /// every descriptor again: a browser holds hundreds.
    ///
    /// An error opening `fd/` is returned as it is: `EACCES` means the
    /// process isn't ours to look into, `ENOENT` that it has exited.
    pub fn walk_fds(
        &mut self,
        pid: u32,
        bufs: &mut FdBuffers,
        drm_fds: &mut Vec<u32>,
        sockets: bool,
        gpu: bool,
    ) -> Result<FdWalk, Errno> {
        let path = pid_path(&mut self.path, pid, b"/fd", b"")?;
        let flags = OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC;
        let dir = rfs::openat(&self.fd, path, flags, Mode::empty())?;
        // Only now: a walk that couldn't start leaves the caller's list be.
        drm_fds.clear();
        let mut walk = FdWalk::default();
        bufs.clients.clear();
        let mut entries = RawDir::new(&dir, &mut bufs.dents);
        // An error part way (the process exiting under us) ends the walk with
        // what it has counted.
        while let Some(Ok(entry)) = entries.next() {
            let name = entry.file_name();
            if name.to_bytes().first() == Some(&b'.') {
                continue; // "." and ".."
            }
            walk.fds += 1;
            let Ok(len) = rfs::readlinkat_raw(&dir, name, &mut bufs.link[..]) else {
                continue;
            };
            let target = &bufs.link[..len];
            if sockets && target.starts_with(b"socket:[") {
                walk.sockets += 1;
                continue;
            }
            if !gpu || !target.starts_with(b"/dev/dri/") {
                continue;
            }
            walk.has_drm = true;
            let Ok(info) = self.read(pid, b"/fdinfo/", name.to_bytes()) else {
                continue;
            };
            let Some(client) = fdinfo::parse(info) else {
                continue;
            };
            if let Some(id) = client.id {
                if bufs.clients.contains(&id) {
                    continue;
                }
                bufs.clients.push(id);
            }
            walk.gpu.add(client.time());
            if let Some(fd) = parse_pid(name.to_bytes()) {
                drm_fds.push(fd);
            }
        }
        Ok(walk)
    }

    /// Engine time summed over the GPU clients a walk found, re-read through
    /// the descriptors it remembered. `None` if one of them is no longer a
    /// DRM client (closed, or the number reused), which calls for a walk.
    pub fn drm_usage(
        &mut self,
        pid: u32,
        drm_fds: &[u32],
        clients: &mut Vec<u64>,
    ) -> Option<GpuTime> {
        clients.clear();
        let mut total = GpuTime::default();
        for &fd in drm_fds {
            let info = self.fdinfo(pid, fd).ok()?;
            let client = fdinfo::parse(info)?;
            if let Some(id) = client.id {
                if clients.contains(&id) {
                    continue;
                }
                clients.push(id);
            }
            total.add(client.time());
        }
        Some(total)
    }
}

/// The scratch a descriptor walk needs besides [`ProcDir`]'s own.
#[derive(Debug)]
pub(crate) struct FdBuffers {
    pub dents: Vec<MaybeUninit<u8>>,
    pub link: Vec<u8>,
    pub clients: Vec<u64>,
}

impl FdBuffers {
    pub fn new() -> Self {
        Self {
            dents: vec![MaybeUninit::uninit(); DENTS],
            link: vec![0; LINK],
            clients: Vec::new(),
        }
    }
}

/// Builds `<pid><name><suffix>` and a NUL in `path`, ready for `openat`.
fn pid_path<'a>(
    path: &'a mut Vec<u8>,
    pid: u32,
    name: &[u8],
    suffix: &[u8],
) -> Result<&'a CStr, Errno> {
    path.clear();
    let mut digits = [0; 10];
    path.extend_from_slice(decimal(pid, &mut digits));
    path.extend_from_slice(name);
    path.extend_from_slice(suffix);
    path.push(0);
    CStr::from_bytes_with_nul(path).map_err(|_| Errno::INVAL)
}

/// `n` in decimal, written into `buf` without allocating.
fn decimal(mut n: u32, buf: &mut [u8; 10]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
        if n == 0 {
            return &buf[i..];
        }
    }
}

/// Reads a procfs file in one `read`, growing `buf` only if the file fills
/// it: a full buffer may be a cut-short file, and a truncated line would
/// quietly corrupt a figure.
fn read_whole<'a>(fd: &OwnedFd, buf: &'a mut Vec<u8>) -> Result<&'a [u8], Errno> {
    let mut n = 0;
    loop {
        match rio::read(fd, &mut buf[n..]) {
            Ok(got) => n += got,
            Err(Errno::INTR) => continue,
            Err(e) => return Err(e),
        }
        if n < buf.len() {
            return Ok(&buf[..n]);
        }
        buf.resize(buf.len() * 2, 0);
    }
}

/// A `/proc` entry name that is all digits, as a pid.
fn parse_pid(name: &[u8]) -> Option<u32> {
    if name.is_empty() || !name.iter().all(u8::is_ascii_digit) {
        return None;
    }
    u32::try_from(crate::sysfs::parse_uint(name)?).ok()
}

#[cfg(test)]
mod tests {
    use super::super::parse;
    use super::*;
    use std::os::fd::AsRawFd;
    use std::os::unix::net::UnixDatagram;

    #[test]
    fn paths() {
        let mut p = Vec::new();
        assert_eq!(pid_path(&mut p, 0, b"/stat", b"").unwrap(), c"0/stat");
        assert_eq!(
            pid_path(&mut p, 4_194_304, b"/fdinfo/", b"17").unwrap(),
            c"4194304/fdinfo/17"
        );
        assert_eq!(pid_path(&mut p, u32::MAX, b"", b"").unwrap(), c"4294967295");
        assert!(pid_path(&mut p, 1, b"/a\0b", b"").is_err());
    }

    #[test]
    fn decimals() {
        let mut b = [0; 10];
        assert_eq!(decimal(0, &mut b), b"0");
        assert_eq!(decimal(17, &mut b), b"17");
        assert_eq!(decimal(u32::MAX, &mut b), b"4294967295");
    }

    /// The quick re-read gives up on a descriptor that isn't a GPU client,
    /// and costs nothing for a client with none remembered.
    #[test]
    fn drm_usage_needs_drm_descriptors() {
        let mut dir = ProcDir::open().unwrap();
        let me = std::process::id();
        let mut clients = Vec::new();
        assert_eq!(
            dir.drm_usage(me, &[], &mut clients),
            Some(GpuTime::default())
        );
        // stdin is not a GPU handle.
        assert_eq!(dir.drm_usage(me, &[0], &mut clients), None);
        assert_eq!(dir.drm_usage(me, &[999_999], &mut clients), None);
    }

    #[test]
    fn pids() {
        assert_eq!(parse_pid(b"1"), Some(1));
        assert_eq!(parse_pid(b"4194304"), Some(4_194_304));
        for name in [&b""[..], b"self", b"1a", b"-1", b"99999999999", b"."] {
            assert_eq!(parse_pid(name), None, "{name:?}");
        }
    }

    /// The listing agrees with `std::fs::read_dir`, twice over: the second
    /// pass has to rewind the descriptor the first read to the end.
    #[test]
    fn lists_the_same_pids_as_read_dir() {
        let mut dir = ProcDir::open().expect("/proc");
        let mut dents = vec![MaybeUninit::uninit(); DENTS];
        let me = std::process::id();
        let read_dir = || -> Vec<u32> {
            std::fs::read_dir("/proc")
                .unwrap()
                .filter_map(|e| e.ok()?.file_name().to_str()?.parse().ok())
                .collect()
        };
        for pass in 0..2 {
            let mut entries = Vec::new();
            let before = read_dir();
            dir.list_pids(&mut dents, &mut entries).unwrap();
            let after = read_dir();
            let pids: Vec<u32> = entries.iter().map(|e| e.pid).collect();
            assert!(pids.contains(&me), "pass {pass}: this process is missing");
            // The inode is made afresh if the kernel reclaims the entry
            // between the listing and the stat, so allow a retry.
            let ino = |entries: &[Entry]| {
                let stat = std::fs::metadata(format!("/proc/{me}")).unwrap();
                let ours = entries.iter().find(|e| e.pid == me).unwrap().ino;
                (ours, std::os::unix::fs::MetadataExt::ino(&stat))
            };
            let (mut listed, mut actual) = ino(&entries);
            for _ in 0..3 {
                if listed == actual {
                    break;
                }
                let mut again = Vec::new();
                dir.list_pids(&mut dents, &mut again).unwrap();
                (listed, actual) = ino(&again);
            }
            assert_eq!(listed, actual, "pass {pass}: inode");
            assert!(
                pids.is_sorted(),
                "pass {pass}: /proc no longer lists in order"
            );
            // Processes come and go meanwhile (other tests start children),
            // but one listed both before and after was there throughout, so
            // ours must have it.
            for pid in before.iter().filter(|p| after.contains(p)) {
                assert!(
                    pids.binary_search(pid).is_ok(),
                    "pass {pass}: {pid} is missing"
                );
            }
        }
    }

    #[test]
    fn reads_whole_files() {
        let mut dir = ProcDir::open().unwrap();
        let me = std::process::id();
        // Our own counters move between two reads, so compare what doesn't.
        let stat = dir.read(me, b"/stat", b"").unwrap().to_vec();
        assert!(stat.ends_with(b"\n"));
        assert!(stat.starts_with(format!("{me} (").as_bytes()));
        let plain = std::fs::read(format!("/proc/{me}/stat")).unwrap();
        let (a, b) = (
            parse::parse_stat(&stat).unwrap(),
            parse::parse_stat(&plain).unwrap(),
        );
        assert_eq!((a.name, a.start_time), (b.name, b.start_time));
        // A file bigger than the starting buffer comes back whole.
        dir.buf.truncate(16);
        let status = dir.read(me, b"/status", b"").unwrap();
        assert!(status.len() > 16);
        assert!(status.ends_with(b"\n"));
        assert!(status.starts_with(b"Name:"));
        assert_eq!(dir.read(me, b"/no-such-file", b""), Err(Errno::NOENT));
    }

    /// The count from `stat` is what a walk lists: the scan skips a walk
    /// when it hasn't moved.
    #[test]
    fn the_descriptor_count_matches_a_walk() {
        // A child, so that no other test's descriptors come and go meanwhile.
        let mut child = std::process::Command::new("sleep")
            .arg("30")
            .spawn()
            .unwrap();
        let pid = child.id();
        let mut dir = ProcDir::open().unwrap();
        let mut bufs = FdBuffers::new();
        // Straight after exec the loader is still opening and closing
        // libraries, so the two can disagree for a moment; once it is
        // asleep they must agree.
        let mut tries = 0;
        let (count, walk) = loop {
            let count = dir.fd_count(pid);
            let walk = dir
                .walk_fds(pid, &mut bufs, &mut Vec::new(), true, false)
                .unwrap();
            tries += 1;
            if count.is_none_or(|n| n == walk.fds) || tries == 200 {
                break (count, walk);
            }
            std::thread::sleep(std::time::Duration::from_millis(5));
        };
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(walk.fds >= 3, "stdin, stdout and stderr: {}", walk.fds);
        match count {
            Some(n) => assert_eq!(n, walk.fds),
            None => eprintln!("kernel before 6.2: no descriptor count"),
        }
        assert_eq!(dir.fd_count(pid), None, "a process that is gone");
    }

    /// The fast walk counts what a plain `read_dir` + `read_link` counts,
    /// with a handful of descriptors and with more than one `getdents`
    /// batch holds, where a listing that stopped after the first batch, or
    /// stepped wrongly between records, would undercount.
    #[test]
    fn walk_counts_sockets() {
        let mut dir = ProcDir::open().unwrap();
        let mut bufs = FdBuffers::new();
        let mut drm = Vec::new();
        let me = std::process::id();
        let slow = || {
            std::fs::read_dir("/proc/self/fd")
                .unwrap()
                .filter_map(|e| std::fs::read_link(e.ok()?.path()).ok())
                .filter(|t| t.to_string_lossy().starts_with("socket:["))
                .count() as u32
        };
        let mut held = Vec::new();
        for _ in 0..3 {
            held.push(UnixDatagram::unbound().unwrap());
        }
        // Other tests in this binary open and close descriptors meanwhile,
        // so allow for their sockets coming and going.
        let walk = dir.walk_fds(me, &mut bufs, &mut drm, true, false).unwrap();
        assert!(walk.sockets >= 3, "found {}", walk.sockets);
        assert!(walk.sockets.abs_diff(slow()) <= 8);
        assert!(
            !walk.has_drm,
            "GPU handles were looked for without being asked"
        );

        // A record is at least 24 bytes.
        let many = DENTS / 24 + 100;
        let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile);
        // Room for these and for the tests running alongside, which spawn
        // children and hold files of their own.
        if limit.current.is_some_and(|c| c < (many * 2 + 512) as u64) {
            eprintln!("descriptor limit too low for {many} sockets; skipping the large walk");
            return;
        }
        for _ in 0..many {
            held.push(UnixDatagram::unbound().unwrap());
        }
        let walk = dir.walk_fds(me, &mut bufs, &mut drm, true, false).unwrap();
        assert!(
            walk.sockets as usize >= held.len(),
            "{} < {}",
            walk.sockets,
            held.len()
        );
        assert!(walk.sockets.abs_diff(slow()) <= 8);
        // Not asked for sockets: none counted.
        let walk = dir.walk_fds(me, &mut bufs, &mut drm, false, false).unwrap();
        assert_eq!(walk.sockets, 0);
        assert!(held.iter().all(|s| s.as_raw_fd() >= 0));
    }

    /// Processes exit between the listing and the walk all the time: that
    /// is an error, not a process with no descriptors and not a panic.
    #[test]
    fn a_process_that_is_gone() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        let mut dir = ProcDir::open().unwrap();
        let mut bufs = FdBuffers::new();
        assert!(dir.read(pid, b"/stat", b"").is_err());
        assert!(
            dir.walk_fds(pid, &mut bufs, &mut Vec::new(), true, true)
                .is_err()
        );
    }
}
