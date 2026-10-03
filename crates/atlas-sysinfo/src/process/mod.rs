//! The process table for the Apps page: every process, how hard it is
//! working, and with what.
//!
//! [`ProcessSampler`] scans `/proc` once per tick. It is the most expensive
//! reader Atlas has (several files for each of several hundred processes),
//! so the app makes one only while the Apps page is on screen, and the scan
//! is arranged to allocate nothing per tick once the first has run: the
//! files are read into reused buffers ([`procfs`]), parsed in place
//! ([`parse`]), and a process's name, unit and container are shared with the previous
//! tick's unless they changed. The costly extras are throttled and carried
//! forward:
//!
//! - each process's `stat`, `statm` and `io` stay open from one tick to the next
//!   and are read again with one `pread`, rather than opened, read and
//!   closed each time: two thirds of the scan's system calls. Up to a
//!   quarter of the descriptor limit is held this way ([`max_held`]);
//! - kernel threads (two thirds of `/proc` on a desktop) are dropped as soon
//!   as their stat line says what they are, unless asked for, and after that
//!   are recognised from the `/proc` listing alone (pid and inode), so they
//!   cost no syscalls at all;
//! - `/proc/<pid>/io` is read only while disk columns are wanted, and one
//!   closed to us is tried again only every 30 ticks;
//! - a process whose CPU time, page faults and rss haven't moved since the
//!   last tick keeps its memory figure without reading `statm`, and shows no
//!   disk traffic without reading `io`, except on a refresh every 5 ticks:
//!   most processes sleep through most ticks, and the stat line, read
//!   anyway, says so;
//! - descriptors are walked (for sockets and GPU handles, in one walk) only
//!   for new processes, a sweep every 30 ticks, and every third tick while
//!   there is network traffic to share out, each process on its own tick so
//!   the walks are spread out rather than all on one, and not even then
//!   while its number of descriptors (one `stat`) is unchanged. A known GPU
//!   client has just the fdinfo of its DRM descriptors re-read; a browser
//!   holds hundreds of descriptors and a handful of those.
//!
//! The state carried between ticks is kept in vectors sorted by pid and
//! merged against the `/proc` listing, which comes in ascending pid order:
//! no hashing, and nothing copied from one tick to the next. Should the
//! listing ever come out of order, a process just looks new for a tick.
//!
//! Per-process network figures are an estimate: the machine's traffic shared
//! out by open socket count. Linux has no per-process network counter
//! without privilege.
//!
//! [`details`] reads one process in full for the Details panel, [`act`]
//! ends, kills, stops or continues one, and [`Proc::impact`] rates its power
//! use.

mod details;
mod impact;
mod parse;
pub(crate) mod procfs;
mod signal;

pub(crate) use details::passwd;
pub use details::{Info, details, executable, start_time};
pub use impact::Impact;
pub use parse::{Stat, container_from_cgroup, parse_io, parse_stat, unit_from_cgroup};
pub use signal::{Action, ActionError, act};

use std::os::fd::OwnedFd;
use std::sync::Arc;
use std::time::Instant;

use rustix::io::Errno;

use crate::gpu::fdinfo::GpuTime;
use crate::stats::{elapsed, net, rate};
use crate::sysfs::HeldFile;
use procfs::{Entry, FdBuffers, ProcDir};

/// Combined receive + send rate (bytes/s) below which the network isn't
/// shared out: with no traffic there is nothing to attribute, and the socket
/// walk is the scan's most expensive part.
const NET_SCAN_THRESHOLD: f64 = 8192.0;

/// While there is traffic, each process's sockets are counted again every
/// this many ticks (staggered by pid); the traffic is shared out every tick
/// by the latest counts.
const NET_SCAN_TICKS: u64 = 3;

/// How often each process's descriptors are walked whatever else says
/// (staggered by pid, like every other throttle here, so the sweep is spread
/// over the ticks rather than a spike on one). New processes are walked when
/// first seen and known GPU clients re-read every tick, so this catches a
/// GPU handle opened long after a process started, a socket count the
/// unchanged descriptor count hid (a file closed and a socket opened), and
/// a process that has lost the right to be read or gained it.
const SWEEP_TICKS: u64 = 30;

/// How often each process's unit is read again. A process can be moved to
/// another cgroup (`systemd-run --scope` does it), but rarely, and nothing
/// breaks for the half minute it takes to notice.
const UNIT_RESCAN_TICKS: u64 = 30;

/// The shortest interval figures are worked out over. CPU time counts in
/// clock ticks (10 ms), so over a few milliseconds one tick would read as
/// hundreds of percent.
const MIN_INTERVAL: f64 = 0.1;

/// An idle process's memory and I/O counters are re-read at least this often
/// (staggered by pid, so not all on one tick): reclaim can take its pages
/// without it running, and a process can do a little I/O in less CPU time
/// than one clock tick.
const IDLE_REFRESH_TICKS: u64 = 5;

/// One process, as the Apps table shows it.
///
/// A figure that is `None` is unknown, not zero: another user's `io` and
/// `fd/` can't be read, and the table says so rather than showing 0.
#[derive(Debug, Clone, PartialEq)]
pub struct Proc {
    pub pid: u32,
    /// Start time in clock ticks since boot. With the pid, it names this
    /// process: pass both to [`act`] so a reused pid is never signalled.
    pub start_time: u64,
    /// The kernel's short name (`comm`), or the program's file name where
    /// the kernel cut that at 15 bytes ([`parse::full_name`]).
    pub name: Arc<str>,
    pub parent: u32,
    pub kernel: bool,
    /// The systemd unit the process runs in (`app-org.kde.dolphin@….service`,
    /// `plasma-plasmashell.service`), or `None` (a kernel thread, a
    /// container's processes at the cgroup root). The desktop starts each
    /// application in a unit of its own, so this is what groups an
    /// application's processes.
    pub unit: Option<Arc<str>>,
    /// The podman container it runs in, by ID ([`container_from_cgroup`]):
    /// a toolbox's, a distrobox's or any other. Its unit is podman's
    /// `libpod-<ID>.scope`, which no desktop file names.
    pub container: Option<Arc<str>>,
    /// Percent of one core; above 100 for a process busy on several.
    pub cpu: f64,
    /// Resident bytes, as `ps` and `top` count them.
    pub memory: u64,
    /// Percent of GPU engine time. `None` if it holds no GPU handle, its
    /// descriptors can't be read, or GPU figures weren't wanted.
    pub gpu: Option<f64>,
    /// Estimated bytes/s received and sent: the machine's traffic shared by
    /// open socket count.
    pub net_in: Option<f64>,
    pub net_out: Option<f64>,
    /// Bytes/s read from and written to storage.
    pub disk_read: Option<f64>,
    pub disk_write: Option<f64>,
}

/// Which figures the page shows. Each one left out is work the scan skips.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Wanted {
    /// Kernel threads as rows. Off by default; leaving them out drops three
    /// quarters of the scan at the first file.
    pub kernel_threads: bool,
    /// Disk read and write: one more file per process per tick.
    pub disk: bool,
    /// GPU use: descriptor walks for GPU clients and new processes.
    pub gpu: bool,
    /// Network estimates: a socket walk of every process every third tick
    /// while there is traffic.
    pub network: bool,
}

impl Default for Wanted {
    fn default() -> Self {
        Self {
            kernel_threads: false,
            disk: true,
            gpu: true,
            network: true,
        }
    }
}

/// What the previous tick knew about a process.
#[derive(Debug)]
struct Prev {
    pid: u32,
    start_time: u64,
    name: Arc<str>,
    unit: Option<Arc<str>>,
    container: Option<Arc<str>>,
    /// Its unit is to be read once more on the next tick: it appeared on
    /// this one, maybe before its launcher moved it into its own unit.
    unit_unsure: bool,
    jiffies: u64,
    faults: u64,
    rss: u64,
    memory: u64,
    io: Option<(u64, u64)>,
    /// Seconds from when `io` was read to the previous tick: more than 0
    /// while an idle process's counters are carried instead of read.
    io_age: f64,
    /// Its `io` was unreadable: another user's.
    io_denied: bool,
    /// Summed engine time, if it holds a GPU handle.
    gpu_time: Option<GpuTime>,
    /// One descriptor per GPU client, to re-read without a walk.
    drm_fds: Vec<u32>,
    /// Its `fd/` was unreadable the last time it was walked.
    fd_denied: bool,
    /// Its open sockets when last counted.
    sockets: u32,
    /// How many descriptors it had then.
    fd_count: Option<u64>,
    /// Its `stat`, `statm` and `io`, held open for the next read.
    stat_fd: Option<OwnedFd>,
    statm_fd: Option<OwnedFd>,
    io_fd: Option<OwnedFd>,
}

/// How many `/proc` files the scan may hold open: a quarter of the soft
/// descriptor limit, so the files a scan opens before it closes last
/// tick's still stay within half of it, and no more than 4096.
fn max_held() -> usize {
    let limit = rustix::process::getrlimit(rustix::process::Resource::Nofile);
    let soft = limit.current.unwrap_or(u64::MAX);
    usize::try_from(soft / 4).unwrap_or(usize::MAX).min(4096)
}

/// Samples every process. Owned by the sampling thread; see the module
/// documentation for what it skips and why.
#[derive(Debug)]
pub struct ProcessSampler {
    wanted: Wanted,
    proc_dir: Option<ProcDir>,
    net_dev: Option<HeldFile>,
    fd_bufs: FdBuffers,
    entries: Vec<Entry>,
    procs: Vec<Proc>,
    /// One per reported process, in pid order.
    prev: Vec<Prev>,
    /// Kernel threads already identified and left out, in pid order.
    kernel: Vec<Entry>,
    /// Emptied buffers of the two above, filled by the next scan and then
    /// swapped in: reused for ever instead of allocated each tick.
    spare_prev: Vec<Prev>,
    spare_kernel: Vec<Entry>,
    net_prev: Option<(u64, u64)>,
    tick: u64,
    last: Instant,
    ticks_per_second: f64,
    page_size: u64,
    /// See [`max_held`].
    max_held: usize,
}

impl ProcessSampler {
    /// Holds `/proc` open and takes the baseline, so the first
    /// [`sample`](Self::sample) has real CPU and rates over the time since.
    pub fn new(wanted: Wanted) -> Self {
        let mut s = Self {
            wanted,
            proc_dir: ProcDir::open(),
            net_dev: HeldFile::with_capacity("/proc/net/dev", 4096),
            fd_bufs: FdBuffers::new(),
            entries: Vec::new(),
            procs: Vec::new(),
            prev: Vec::new(),
            kernel: Vec::new(),
            spare_prev: Vec::new(),
            spare_kernel: Vec::new(),
            net_prev: None,
            tick: 0,
            last: Instant::now(),
            ticks_per_second: rustix::param::clock_ticks_per_second() as f64,
            page_size: rustix::param::page_size() as u64,
            max_held: max_held(),
        };
        s.scan(1.0);
        s
    }

    /// Changes which figures are collected, from the next tick.
    pub fn set_wanted(&mut self, wanted: Wanted) {
        self.wanted = wanted;
    }

    /// Scans every process. The slice is valid until the next call; it is
    /// the previous tick's list if `/proc` couldn't be listed at all.
    pub fn sample(&mut self) -> &[Proc] {
        let seconds = elapsed(&mut self.last);
        self.scan(seconds);
        &self.procs
    }

    fn scan(&mut self, seconds: f64) {
        let dt = seconds.max(MIN_INTERVAL);
        let (rx_rate, tx_rate) = self.total_net(dt);
        let Some(dir) = self.proc_dir.as_mut() else {
            return;
        };
        self.entries.clear();
        if dir
            .list_pids(&mut self.fd_bufs.dents, &mut self.entries)
            .is_err()
        {
            return;
        }

        let wanted = self.wanted;
        let mut max_held = self.max_held;
        // Descriptors kept into `next` so far.
        let mut held = 0;
        let tick = self.tick;
        self.tick += 1;
        let traffic = rx_rate + tx_rate > NET_SCAN_THRESHOLD;

        self.procs.clear();
        let mut next = std::mem::take(&mut self.spare_prev);
        let mut next_kernel = std::mem::take(&mut self.spare_kernel);
        let mut old = self.prev.drain(..).peekable();
        let mut old_kernel = self.kernel.drain(..).peekable();
        for &entry in &self.entries {
            let pid = entry.pid;
            while old.next_if(|p| p.pid < pid).is_some() {}
            let mut prev = old.next_if(|p| p.pid == pid);
            let mut stat_fd = prev.as_mut().and_then(|p| p.stat_fd.take());
            let mut statm_fd = prev.as_mut().and_then(|p| p.statm_fd.take());
            let mut io_fd = prev.as_mut().and_then(|p| p.io_fd.take());
            // Room for all three files of this process, or none are kept.
            let keep = held + 3 <= max_held;
            while old_kernel.next_if(|k| k.pid < pid).is_some() {}
            let known_kernel = old_kernel.next_if(|k| k.pid == pid) == Some(entry);
            if known_kernel && !wanted.kernel_threads {
                next_kernel.push(entry);
                continue;
            }

            let line = match dir.read_held(&mut stat_fd, keep, pid, b"/stat") {
                Ok(line) => line,
                // Out of descriptors, not exited: hold fewer from now on.
                // The process is left out this tick and reads as new on
                // the next (carried, its CPU time would be two ticks'
                // over one).
                Err(Errno::MFILE | Errno::NFILE) => {
                    max_held /= 2;
                    continue;
                }
                Err(_) => continue, // exited since the listing
            };
            let Some(stat) = parse::parse_stat(line) else {
                continue;
            };
            if stat.kernel && !wanted.kernel_threads {
                next_kernel.push(entry);
                continue;
            }
            let mut prev = prev.filter(|p| p.start_time == stat.start_time);
            if prev.is_none() {
                // Another process's, given the same pid.
                (statm_fd, io_fd) = (None, None);
            }
            let (kernel, jiffies, start_time, parent) =
                (stat.kernel, stat.jiffies, stat.start_time, stat.ppid);
            let (faults, rss) = (stat.faults, stat.rss);
            // The name is copied out of the read buffer only when it changed:
            // a process's name almost never does. One the kernel cut short
            // is looked up whole in `cmdline` when the process appears, and
            // again with the unit (a read that failed, or caught it mid-exec,
            // would otherwise leave it cut, and apart from its siblings).
            let cut = stat.name.len() == parse::COMM_MAX && !kernel;
            let rescan = (tick + u64::from(pid)).is_multiple_of(UNIT_RESCAN_TICKS);
            let unit_again = rescan || prev.as_ref().is_some_and(|p| p.unit_unsure);
            let prev_name = prev.as_ref().map(|p| &p.name);
            let name = match prev_name {
                Some(p)
                    if p.as_bytes().starts_with(stat.name)
                        && (p.len() == stat.name.len() || cut)
                        && !(cut && unit_again) =>
                {
                    Arc::clone(p)
                }
                _ if cut => {
                    let mut comm = [0; parse::COMM_MAX];
                    comm.copy_from_slice(stat.name);
                    match dir.read(pid, b"/cmdline", b"") {
                        Ok(b) => reuse(prev_name, parse::full_name(&comm, b).unwrap_or(&comm)),
                        Err(_) => reuse(prev_name, &comm),
                    }
                }
                _ => reuse(prev_name, stat.name),
            };

            // Read when the process first appears, again on the next tick,
            // and on the occasional rescan, not every tick: one more file per
            // process per tick would be a third more opens. A launcher
            // (`systemd-run --scope`, Plasma's) starts the program and then
            // moves it into the application's unit, and a read in between
            // would file it under the launcher's for half a minute. Those in
            // the baseline scan were running before the page opened: settled.
            let prev_unit = prev.as_ref().and_then(|p| p.unit.as_ref());
            let prev_container = prev.as_ref().and_then(|p| p.container.as_ref());
            let unit_unsure = prev.is_none() && tick > 0;
            let (unit, container) = if kernel {
                (None, None)
            } else if prev.is_none() || unit_again {
                match dir.read(pid, b"/cgroup", b"") {
                    Ok(b) => (
                        parse::unit_from_cgroup(b).map(|u| reuse(prev_unit, u)),
                        parse::container_from_cgroup(b).map(|c| reuse(prev_container, c)),
                    ),
                    Err(_) => (prev_unit.cloned(), prev_container.cloned()),
                }
            } else {
                (prev_unit.cloned(), prev_container.cloned())
            };

            // Most processes sleep through most ticks. One that hasn't run,
            // faulted or had its rss move keeps last tick's memory figure and
            // did no disk I/O, which saves its statm and io reads; the stat
            // line already said all that.
            let idle = prev.as_ref().filter(|p| {
                p.jiffies == jiffies
                    && p.faults == faults
                    && p.rss == rss
                    && !(tick + u64::from(pid)).is_multiple_of(IDLE_REFRESH_TICKS)
            });
            // Should statm fail (the process exiting under us), last tick's
            // figure stands rather than a made-up zero.
            let memory = match idle {
                Some(p) => p.memory,
                None => dir
                    .read_held(&mut statm_fd, keep, pid, b"/statm")
                    .ok()
                    .and_then(parse::parse_statm_resident)
                    .map(|pages| pages.saturating_mul(self.page_size))
                    .or(prev.as_ref().map(|p| p.memory))
                    .unwrap_or(0),
            };

            // Carried counters age, so that when they are read again the
            // rate is over the whole time since, not one tick's worth.
            // A process whose io was closed to us is tried again only on its
            // sweep, like its fd/: it opens up only by changing its
            // credentials, and a busy one was a failed read every tick.
            let skip_io = idle.filter(|_| wanted.disk);
            let carried = skip_io.and_then(|p| p.io.map(|io| (io, p.io_age + dt)));
            let io_closed = prev.as_ref().is_some_and(|p| p.io_denied)
                && !(tick + u64::from(pid)).is_multiple_of(SWEEP_TICKS);
            let (io, io_age, io_denied) = match carried {
                _ if !wanted.disk => (None, 0.0, false),
                Some((io, age)) => (Some(io), age, false),
                None if io_closed => (None, 0.0, true),
                None => match dir.read_held(&mut io_fd, keep, pid, b"/io") {
                    Ok(b) => (parse::parse_io(b), 0.0, false),
                    Err(e) => (None, 0.0, matches!(e, Errno::ACCESS | Errno::PERM)),
                },
            };
            let (disk_read, disk_write) = if carried.is_some() {
                (Some(0.0), Some(0.0))
            } else {
                let before = prev.as_ref().and_then(|p| p.io.map(|io| (io, p.io_age)));
                disk_rates(io, before, dt)
            };

            let cpu = prev.as_ref().map_or(0.0, |p| {
                jiffies.saturating_sub(p.jiffies) as f64 / self.ticks_per_second / dt * 100.0
            });

            // Sockets and GPU handles come from the same descriptors, so they
            // share one walk. A known GPU client between sweeps has only its
            // DRM descriptors re-read. A new process is walked even when
            // neither figure is due, while network figures are wanted, to
            // learn whether its descriptors can be read at all. Sockets due a
            // recount aren't walked again while the number of descriptors
            // holds: on a desktop that skips nine in ten of the links, and
            // the count it would find is the same.
            let mut drm_fds = prev
                .as_mut()
                .map(|p| std::mem::take(&mut p.drm_fds))
                .unwrap_or_default();
            let known_gpu = prev.as_ref().is_some_and(|p| p.gpu_time.is_some());
            let mut fd_denied = prev.as_ref().is_some_and(|p| p.fd_denied);
            let mut sockets = prev.as_ref().map_or(0, |p| p.sockets);
            let mut fd_count = prev.as_ref().and_then(|p| p.fd_count);
            let mut gpu_time = None;
            let sweep = (tick + u64::from(pid)).is_multiple_of(SWEEP_TICKS);
            let mut gpu_walk = wanted.gpu && (sweep || prev.is_none());
            // A process whose fd/ is closed to us is tried again only on its
            // sweep. (SWEEP_TICKS is a multiple of NET_SCAN_TICKS, so a sweep
            // tick is always a recount tick.)
            let mut net_walk = wanted.network
                && traffic
                && (tick + u64::from(pid)).is_multiple_of(NET_SCAN_TICKS)
                && (sweep || !fd_denied);
            if net_walk && !sweep && !gpu_walk && fd_count.is_some() {
                net_walk = dir.fd_count(pid) != fd_count;
            }
            if wanted.gpu && known_gpu && !gpu_walk {
                if net_walk {
                    gpu_walk = true; // walking anyway
                } else {
                    gpu_time = dir.drm_usage(pid, &drm_fds, &mut self.fd_bufs.clients);
                    gpu_walk = gpu_time.is_none();
                }
            }
            let probe = wanted.network && prev.is_none();
            if !kernel && (net_walk || gpu_walk || probe) {
                // Sockets are counted on any walk: the links are read anyway.
                let count = wanted.network;
                match dir.walk_fds(pid, &mut self.fd_bufs, &mut drm_fds, count, gpu_walk) {
                    Ok(walk) => {
                        fd_denied = false;
                        if count {
                            sockets = walk.sockets;
                            fd_count = Some(walk.fds);
                        }
                        gpu_time = walk.has_drm.then_some(walk.gpu);
                    }
                    Err(Errno::ACCESS | Errno::PERM) => {
                        fd_denied = true;
                        sockets = 0;
                        fd_count = None;
                    }
                    // Exited, most likely; or out of descriptors, in which
                    // case a known GPU client stays one until next time.
                    Err(_) => gpu_time = prev.as_ref().and_then(|p| p.gpu_time),
                }
            }
            if !wanted.gpu {
                gpu_time = None;
            }
            let gpu = gpu_time.map(|now| match prev.as_ref().and_then(|p| p.gpu_time) {
                Some(before) => now.percent_since(&before, dt),
                None => 0.0, // a GPU client, but no interval to measure yet
            });

            if !wanted.disk {
                io_fd = None;
            }
            if !keep {
                // Over the budget: held files carried from before go too.
                (stat_fd, statm_fd, io_fd) = (None, None, None);
            }
            held += [&stat_fd, &statm_fd, &io_fd]
                .iter()
                .filter(|f| f.is_some())
                .count();
            next.push(Prev {
                pid,
                start_time,
                name: Arc::clone(&name),
                unit: unit.clone(),
                container: container.clone(),
                unit_unsure,
                jiffies,
                faults,
                rss,
                memory,
                io,
                io_age,
                io_denied,
                gpu_time,
                drm_fds,
                fd_denied,
                sockets,
                fd_count,
                stat_fd,
                statm_fd,
                io_fd,
            });
            self.procs.push(Proc {
                pid,
                start_time,
                name,
                parent,
                kernel,
                unit,
                container,
                cpu,
                memory,
                gpu,
                net_in: None,
                net_out: None,
                disk_read,
                disk_write,
            });
        }
        drop(old);
        drop(old_kernel);
        self.max_held = max_held;

        if wanted.network {
            share_network(&mut self.procs, &next, traffic, (rx_rate, tx_rate));
        }
        // The drained vectors are empty now, with their capacity kept.
        self.spare_prev = std::mem::replace(&mut self.prev, next);
        self.spare_kernel = std::mem::replace(&mut self.kernel, next_kernel);
    }

    /// The machine's receive and send rates over every interface but
    /// loopback. Zero on the baseline tick.
    fn total_net(&mut self, dt: f64) -> (f64, f64) {
        let Some(data) = self.net_dev.as_mut().and_then(HeldFile::bytes) else {
            return (0.0, 0.0);
        };
        let (rx, tx) = net::parse_net_dev(data)
            .filter(|(name, ..)| *name != &b"lo"[..])
            .fold((0u64, 0u64), |(r, t), (_, rx, tx)| {
                (r.saturating_add(rx), t.saturating_add(tx))
            });
        let rates = self
            .net_prev
            .map_or((0.0, 0.0), |(pr, pt)| (rate(rx, pr, dt), rate(tx, pt, dt)));
        self.net_prev = Some((rx, tx));
        rates
    }
}

impl Default for ProcessSampler {
    fn default() -> Self {
        Self::new(Wanted::default())
    }
}

/// Fills in the network estimates: the machine's traffic shared out by each
/// process's latest socket count, zero once the network is idle. A process
/// whose descriptors can't be read stays unknown. `procs` and `states` are
/// the same processes in the same order.
fn share_network(procs: &mut [Proc], states: &[Prev], traffic: bool, (rx, tx): (f64, f64)) {
    let total: u32 = states.iter().map(|s| s.sockets).sum();
    for (p, state) in procs.iter_mut().zip(states) {
        if state.fd_denied {
            continue;
        }
        let share = if traffic && total > 0 {
            f64::from(state.sockets) / f64::from(total)
        } else {
            0.0
        };
        (p.net_in, p.net_out) = (Some(rx * share), Some(tx * share));
    }
}

/// Read and write rates from this tick's `io` counters and the last ones
/// read, `before`, with how long before the previous tick they were read.
/// Zero on a process's first reading, unknown where `io` is unreadable.
fn disk_rates(
    io: Option<(u64, u64)>,
    before: Option<((u64, u64), f64)>,
    dt: f64,
) -> (Option<f64>, Option<f64>) {
    match (io, before) {
        (Some((r, w)), Some(((pr, pw), age))) => {
            (Some(rate(r, pr, age + dt)), Some(rate(w, pw, age + dt)))
        }
        (Some(_), None) => (Some(0.0), Some(0.0)),
        (None, _) => (None, None),
    }
}

/// `prev` if it already says `raw`, else a new copy of `raw`. Names that
/// aren't UTF-8 are kept with replacement characters.
fn reuse(prev: Option<&Arc<str>>, raw: &[u8]) -> Arc<str> {
    let text = String::from_utf8_lossy(raw);
    match prev {
        Some(p) if **p == *text => Arc::clone(p),
        _ => Arc::from(&*text),
    }
}

#[cfg(test)]
mod tests;
