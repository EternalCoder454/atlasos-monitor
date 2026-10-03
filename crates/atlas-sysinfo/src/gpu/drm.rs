//! A card's load from its clients' counters, for cards with no busy file:
//! Intel (i915, xe), and any other driver that reports drm-usage-stats.
//!
//! The load is the busiest engine's, as `intel_gpu_top` shows it: a card
//! decoding video while its 3D engine idles isn't twice as busy. Each
//! engine's busy time is the sum of what every client on this card used
//! since the last tick. Deltas are taken per client, so a client opening or
//! closing between ticks doesn't make the sum jump, and a client is counted
//! once by its `drm-client-id` however many processes hold it (a fork, a
//! handle passed over a socket).
//!
//! Finding the clients means walking descriptors, the expensive part. The
//! Go version walked every process every 5 s. Here `/proc` is listed every
//! 5 ticks, and only processes not seen before are walked. One that holds
//! no GPU handle is walked again on its own tick once every 30 (staggered
//! by pid), and only if its number of descriptors changed; one whose
//! descriptors were closed to us is tried again on the same tick. A known
//! client has just the fdinfo of its DRM descriptors re-read each tick, and
//! is walked again at once when one of them stops being a GPU handle. A
//! client found at a listing counts from its first tick after: up to 5 s of
//! a new program's work is not in the figure.

use std::mem::MaybeUninit;
use std::time::Instant;

use rustix::io::Errno;

use super::fdinfo::{self, MAX_ENGINES, PciSlot};
use crate::process::procfs::{DENTS, Entry, FdBuffers, ProcDir};
use crate::stats::elapsed;

/// Ticks between listings of `/proc` for new processes.
const LIST_TICKS: u64 = 5;
/// Listings between re-walks of a process that held no GPU handle.
const SWEEP_LISTINGS: u64 = 6;
/// Engines told apart across all clients of the card.
const ENGINES: usize = MAX_ENGINES;

#[derive(Debug)]
enum Kind {
    /// Holds GPU handles: one descriptor per client, re-read each tick.
    Client(Vec<u32>),
    /// Held none when walked, with this many descriptors.
    Plain(Option<u64>),
    /// Its descriptors weren't ours to read (another user's, a kernel
    /// thread, a non-dumpable program).
    Denied,
}

#[derive(Debug)]
struct Process {
    entry: Entry,
    kind: Kind,
}

/// One client's counters at the last tick, by engine (index into
/// `EngineLoad::names`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Counts {
    /// `drm-client-id`, or the top bit set over the pid and descriptor.
    key: u64,
    busy: [Option<Busy>; ENGINES],
}

/// One of the two: a driver that counts both time and cycles for an
/// engine (msm, panthor) is read by time, or it would count twice.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Busy {
    Ns(u64),
    /// Busy cycles and the GPU's cycle counter.
    Cycles(u64, u64),
}

/// One tick's busy time for one engine, summed over clients.
#[derive(Debug, Clone, Copy, Default)]
struct Tick {
    ns: u64,
    cycles: u64,
    /// The GPU clock's advance over the tick (the same for every client).
    total_cycles: u64,
    capacity: u32,
}

#[derive(Debug)]
pub(super) struct EngineLoad {
    slot: Option<PciSlot>,
    /// Count clients that don't say which card they're on: there is only one.
    only_card: bool,
    dir: ProcDir,
    bufs: FdBuffers,
    dents: Vec<MaybeUninit<u8>>,
    listing: Vec<Entry>,
    procs: Vec<Process>,
    next_procs: Vec<Process>,
    names: Vec<Vec<u8>>,
    counts: Vec<Counts>,
    next_counts: Vec<Counts>,
    /// Processes found holding a closed descriptor this tick, to walk again.
    stale: Vec<usize>,
    tick: u64,
    last: Instant,
}

impl EngineLoad {
    /// Finds the card's clients and takes their first counters. `None`
    /// without a readable `/proc`.
    pub fn new(slot: Option<PciSlot>, only_card: bool) -> Option<Self> {
        let mut load = Self {
            slot,
            only_card,
            dir: ProcDir::open()?,
            bufs: FdBuffers::new(),
            dents: vec![MaybeUninit::uninit(); DENTS],
            listing: Vec::new(),
            procs: Vec::new(),
            next_procs: Vec::new(),
            names: Vec::new(),
            counts: Vec::new(),
            next_counts: Vec::new(),
            stale: Vec::new(),
            tick: 0,
            last: Instant::now(),
        };
        load.sample();
        Some(load)
    }

    /// The busiest engine's load since the last call, 0..=100. `None` while
    /// no client on this card has reported an engine: a driver that doesn't
    /// count, or nothing has drawn yet.
    pub fn sample(&mut self) -> Option<f64> {
        if self.tick.is_multiple_of(LIST_TICKS) {
            self.discover();
        }
        self.tick += 1;
        let seconds = elapsed(&mut self.last);

        let mut ticks = [Tick::default(); ENGINES];
        self.next_counts.clear();
        self.stale.clear();
        for (index, p) in self.procs.iter().enumerate() {
            let Kind::Client(fds) = &p.kind else {
                continue;
            };
            let pid = p.entry.pid;
            for &fd in fds {
                let Some(client) = self.dir.fdinfo(pid, fd).ok().and_then(fdinfo::parse) else {
                    // Closed, or the number reused.
                    if self.stale.last() != Some(&index) {
                        self.stale.push(index);
                    }
                    continue;
                };
                let on_card = match client.pdev {
                    Some(pdev) => Some(pdev) == self.slot,
                    None => self.only_card,
                };
                let key = client
                    .id
                    .unwrap_or(1 << 63 | u64::from(pid) << 32 | u64::from(fd));
                if !on_card || self.next_counts.iter().any(|c| c.key == key) {
                    continue;
                }
                let mut now = Counts {
                    key,
                    busy: [None; ENGINES],
                };
                let before = self
                    .counts
                    .binary_search_by_key(&key, |c| c.key)
                    .ok()
                    .map(|i| self.counts[i]);
                for e in client.engines() {
                    let busy = match (e.ns, e.cycles.zip(e.total_cycles)) {
                        (Some(ns), _) => Busy::Ns(ns),
                        (None, Some((c, total))) => Busy::Cycles(c, total),
                        (None, None) => continue, // a capacity or clock alone
                    };
                    let Some(i) = engine_index(&mut self.names, e.name()) else {
                        continue;
                    };
                    now.busy[i] = Some(busy);
                    let t = &mut ticks[i];
                    t.capacity = t.capacity.max(e.capacity);
                    match (busy, before.and_then(|b| b.busy[i])) {
                        (Busy::Ns(n), Some(Busy::Ns(p))) => {
                            t.ns = t.ns.saturating_add(n.saturating_sub(p));
                        }
                        (Busy::Cycles(c, total), Some(Busy::Cycles(pc, ptotal))) => {
                            t.cycles = t.cycles.saturating_add(c.saturating_sub(pc));
                            t.total_cycles = t.total_cycles.max(total.saturating_sub(ptotal));
                        }
                        _ => {} // new, or it changed how it counts
                    }
                }
                self.next_counts.push(now);
            }
        }
        // Walked again now, not at the next listing: the clients it still
        // has are counted from the next tick instead of five later.
        for k in 0..self.stale.len() {
            let index = self.stale[k];
            let pid = self.procs[index].entry.pid;
            self.procs[index].kind = self.walk(pid).unwrap_or(Kind::Plain(None));
        }
        self.next_counts.sort_unstable_by_key(|c| c.key);
        std::mem::swap(&mut self.counts, &mut self.next_counts);

        if self.names.is_empty() {
            return None;
        }
        let busiest = ticks.iter().map(|t| t.percent(seconds)).fold(0.0, f64::max);
        Some(busiest.clamp(0.0, 100.0))
    }

    /// Lists `/proc` and walks the processes that need it.
    fn discover(&mut self) {
        self.listing.clear();
        if self
            .dir
            .list_pids(&mut self.dents, &mut self.listing)
            .is_err()
        {
            return;
        }
        let sweep = self.tick / LIST_TICKS;
        let mut old = std::mem::take(&mut self.procs).into_iter().peekable();
        let listing = std::mem::take(&mut self.listing);
        self.next_procs.clear();
        for &entry in &listing {
            while old.next_if(|p| p.entry.pid < entry.pid).is_some() {}
            let prev = old.next_if(|p| p.entry == entry);
            let kind = match prev.map(|p| p.kind) {
                Some(Kind::Plain(count))
                    if !(sweep + u64::from(entry.pid)).is_multiple_of(SWEEP_LISTINGS)
                        || count.is_some_and(|n| self.dir.fd_count(entry.pid) == Some(n)) =>
                {
                    Kind::Plain(count)
                }
                Some(Kind::Denied)
                    if !(sweep + u64::from(entry.pid)).is_multiple_of(SWEEP_LISTINGS) =>
                {
                    Kind::Denied
                }
                Some(kind @ Kind::Client(_)) => kind,
                // New, or due a look.
                _ => match self.walk(entry.pid) {
                    Some(kind) => kind,
                    None => continue, // gone
                },
            };
            self.next_procs.push(Process { entry, kind });
        }
        drop(old);
        self.listing = listing;
        std::mem::swap(&mut self.procs, &mut self.next_procs);
    }

    fn walk(&mut self, pid: u32) -> Option<Kind> {
        let mut fds = Vec::new();
        match self
            .dir
            .walk_fds(pid, &mut self.bufs, &mut fds, false, true)
        {
            Ok(_) if !fds.is_empty() => Some(Kind::Client(fds)),
            Ok(walk) => Some(Kind::Plain(Some(walk.fds))),
            Err(Errno::ACCESS | Errno::PERM) => Some(Kind::Denied),
            Err(_) => None,
        }
    }
}

impl Tick {
    /// This engine's load, 0..=100 (more if the counters disagree; the
    /// caller caps it).
    fn percent(&self, seconds: f64) -> f64 {
        let mut busy = 0.0;
        if seconds > 0.0 {
            busy += self.ns as f64 / (seconds * 1e9);
        }
        if self.total_cycles > 0 {
            busy += self.cycles as f64 / self.total_cycles as f64;
        }
        busy / f64::from(self.capacity.max(1)) * 100.0
    }
}

/// The engine's index in `names`, adding it. `None` once `ENGINES` names
/// are known.
fn engine_index(names: &mut Vec<Vec<u8>>, name: &[u8]) -> Option<usize> {
    if let Some(i) = names.iter().position(|n| n == name) {
        return Some(i);
    }
    if names.len() == ENGINES {
        return None;
    }
    names.push(name.to_vec());
    Some(names.len() - 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn engine_percent() {
        let t = Tick {
            ns: 500_000_000,
            capacity: 1,
            ..Tick::default()
        };
        assert_eq!(t.percent(1.0), 50.0);
        assert_eq!(t.percent(0.0), 0.0);
        // Two video engines, one of them busy.
        let t = Tick {
            ns: 1_000_000_000,
            capacity: 2,
            ..Tick::default()
        };
        assert_eq!(t.percent(1.0), 50.0);
        let t = Tick {
            cycles: 300,
            total_cycles: 1_000,
            capacity: 1,
            ..Tick::default()
        };
        assert_eq!(t.percent(1.0), 30.0);
        assert_eq!(Tick::default().percent(1.0), 0.0);
    }

    #[test]
    fn engine_names() {
        let mut names = Vec::new();
        assert_eq!(engine_index(&mut names, b"gfx"), Some(0));
        assert_eq!(engine_index(&mut names, b"compute"), Some(1));
        assert_eq!(engine_index(&mut names, b"gfx"), Some(0));
        for i in 2..ENGINES {
            assert_eq!(
                engine_index(&mut names, format!("e{i}").as_bytes()),
                Some(i)
            );
        }
        assert_eq!(engine_index(&mut names, b"one too many"), None);
        assert_eq!(engine_index(&mut names, b"compute"), Some(1));
    }

    /// Live: whatever this machine's cards are, the load is a percentage,
    /// and repeated samples keep the process list sorted and whole. In CI's
    /// container there are no clients, and so no load.
    #[test]
    fn live_load() {
        let mut load = EngineLoad::new(None, true).expect("/proc");
        for _ in 0..(LIST_TICKS * 2 + 1) {
            if let Some(p) = load.sample() {
                assert!((0.0..=100.0).contains(&p), "{p}");
            }
        }
        assert!(
            load.procs
                .windows(2)
                .all(|w| w[0].entry.pid < w[1].entry.pid)
        );
        assert!(load.counts.windows(2).all(|w| w[0].key < w[1].key));
        // This test process is listed and holds no GPU handle.
        let me = std::process::id();
        let mine = load.procs.iter().find(|p| p.entry.pid == me);
        assert!(matches!(mine.map(|p| &p.kind), Some(Kind::Plain(_))));
    }
}
