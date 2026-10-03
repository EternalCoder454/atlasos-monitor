//! Health: the short list of what is actually wrong with the machine, worst
//! first. A disk nearly full, a failing drive, failed services, a hot
//! processor; nothing at all on a healthy machine.
//!
//! [`check`] reads nothing itself. The sampling thread passes what its readers
//! last gave: temperatures and memory from this tick, disk space from the last
//! `statvfs` (every 5th tick), and drive health and failed services from their
//! slower D-Bus timers (`smart` once a minute, `services` every few seconds).
//!
//! The thresholds are the Go version's (`internal/health`), unchanged.
//! Changed from Go:
//! - A disk is nearly full when the sum of its mounted filesystems is,
//!   measured against their size, not the whole disk's. Go divided by the
//!   disk, so a Linux partition beside a large unmounted Windows one read as
//!   fuller than it was (a 500 GB partition with 40 GB free on a 2 TB disk
//!   warned). As in Go, a small full `/boot` beside a roomy root isn't seen.
//! - Every graphics card is checked, each named, not only the first.
//! - The wording: details are sentences, and the hot-processor one no longer
//!   says chips slow down from 85 °C (Intel's limit is 100, AMD's 95). The
//!   figures in them are formatted as Go's.

use crate::smart;
use crate::stats::disk::{Disk, Space};
use crate::stats::memory::Memory;

/// Over this, in °C, the processor or a graphics card is running hot.
const HOT: f64 = 85.0;
/// Less memory available than this is running out, whatever the total.
const LOW_MEMORY: u64 = 2 << 30;
/// More than this share of memory used is running out too.
const HIGH_MEMORY_USED: f64 = 0.90;
/// Swap this full is heavy swapping...
const HEAVY_SWAP_USED: f64 = 0.25;
/// ...but only with less than this share of memory available. Fedora's swap
/// is zram, compressed RAM the system is built to use: pages parked there
/// cost nothing while there is memory to spare.
const SWAP_PRESSURE_AVAILABLE: f64 = 0.25;
/// Less than this share free is a disk nearly full.
const NEARLY_FULL: f64 = 0.05;
/// This much of an SSD's rated life used is worth saying. Below it is
/// ordinary ageing.
const WORN: u8 = 90;
/// Failed services named in an alert's detail; the rest are counted.
const SERVICES_NAMED: usize = 3;

/// How loud an alert is. `Critical` sorts above `Warning`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Level {
    /// Worth knowing, not hurting yet.
    Warning,
    /// Doing damage, or about to.
    Critical,
}

/// One thing wrong, with the figures behind it.
#[derive(Debug, Clone, PartialEq)]
pub enum Problem {
    HotProcessor {
        celsius: f64,
    },
    HotGraphics {
        card: String,
        celsius: f64,
    },
    LowMemory {
        available: u64,
        total: u64,
    },
    HeavySwap {
        used: u64,
        total: u64,
    },
    DiskNearlyFull {
        disk: String,
        free: u64,
        size: u64,
    },
    /// The drive's own verdict: it expects to fail.
    DriveFailing {
        drive: String,
    },
    /// Spare blocks below the drive's threshold.
    DriveOutOfSpares {
        drive: String,
    },
    DriveWorn {
        drive: String,
        wear: u8,
    },
    /// Every failed service, in the order given.
    ServicesFailed {
        names: Vec<String>,
    },
}

/// One problem and how loud it is.
#[derive(Debug, Clone, PartialEq)]
pub struct Alert {
    pub level: Level,
    pub problem: Problem,
}

impl Alert {
    /// A short title, for a list: "Disk nearly full: Samsung SSD 990 PRO 2TB".
    pub fn title(&self) -> String {
        match &self.problem {
            Problem::HotProcessor { .. } => "Processor is running hot".into(),
            Problem::HotGraphics { .. } => "Graphics card is running hot".into(),
            Problem::LowMemory { .. } => "Running out of memory".into(),
            Problem::HeavySwap { .. } => "Swapping heavily".into(),
            Problem::DiskNearlyFull { disk, .. } => format!("Disk nearly full: {disk}"),
            Problem::DriveFailing { drive } => format!("Drive is failing: {drive}"),
            Problem::DriveOutOfSpares { drive } => format!("Drive is nearly worn out: {drive}"),
            Problem::DriveWorn { drive, .. } => format!("Drive is wearing out: {drive}"),
            Problem::ServicesFailed { names } if names.len() == 1 => {
                "A background service has failed".into()
            }
            Problem::ServicesFailed { names } => {
                format!("{} background services have failed", names.len())
            }
        }
    }

    /// The figures behind it, in a sentence or two.
    pub fn detail(&self) -> String {
        match &self.problem {
            Problem::HotProcessor { celsius } => format!(
                "{celsius:.0} °C. Close to its limit, a processor slows itself down to cool off."
            ),
            Problem::HotGraphics { card, celsius } => format!("{card} is at {celsius:.0} °C."),
            Problem::LowMemory { available, total } => format!(
                "{} free of {}. Programs may start closing.",
                gib(*available),
                gib(*total)
            ),
            Problem::HeavySwap { used, total } => format!(
                "{} of {} swap in use. The computer will feel slow.",
                gib(*used),
                gib(*total)
            ),
            Problem::DiskNearlyFull { free, size, .. } => {
                format!("{} free of {}.", bytes(*free), bytes(*size))
            }
            Problem::DriveFailing { .. } => {
                "The drive reports that it expects to fail. Back up anything on it that matters."
                    .into()
            }
            Problem::DriveOutOfSpares { .. } => {
                "It has used up its spare blocks, which is how an SSD says it is near the end."
                    .into()
            }
            Problem::DriveWorn { wear, .. } => format!("{wear}% of its rated life is used."),
            Problem::ServicesFailed { names } => {
                let shown = names[..names.len().min(SERVICES_NAMED)].join(", ");
                match names.len().checked_sub(SERVICES_NAMED) {
                    Some(more) if more > 0 => format!("{shown} and {more} more"),
                    _ => shown,
                }
            }
        }
    }
}

/// A graphics card's main temperature (`edge` on AMD), by the card's name.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Graphics<'a> {
    pub name: &'a str,
    /// `None` while the card sleeps or has no sensor.
    pub temperature: Option<f64>,
}

/// A disk and its filesystems' space, from [`crate::stats::disk::space`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DiskSpace<'a> {
    pub disk: &'a Disk,
    /// `None` when nothing on it is mounted (another system's disk).
    pub space: Option<Space>,
}

/// What a drive says about itself, from [`smart::SmartReader`].
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Drive<'a> {
    /// The name a person reads: [`Disk::label`].
    pub name: &'a str,
    pub health: &'a smart::Health,
}

/// Everything [`check`] looks at. A reading that isn't there is left at its
/// default and checks nothing.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Machine<'a> {
    /// Processor package temperature in °C.
    pub cpu_temperature: Option<f64>,
    pub graphics: &'a [Graphics<'a>],
    pub memory: Option<Memory>,
    pub disks: &'a [DiskSpace<'a>],
    pub drives: &'a [Drive<'a>],
    /// Failed services' names, from [`crate::services::ServiceReader::failed`].
    pub failed_services: &'a [String],
}

/// Everything wrong with the machine, critical first, otherwise in a fixed
/// order: heat, memory, disks, drives, services. Empty when all is well.
pub fn check(m: &Machine) -> Vec<Alert> {
    let mut alerts = Vec::new();
    let mut add = |level, problem| alerts.push(Alert { level, problem });

    if let Some(celsius) = m.cpu_temperature.filter(|&t| t > HOT) {
        add(Level::Critical, Problem::HotProcessor { celsius });
    }
    for g in m.graphics {
        if let Some(celsius) = g.temperature.filter(|&t| t > HOT) {
            add(
                Level::Critical,
                Problem::HotGraphics {
                    card: g.name.to_owned(),
                    celsius,
                },
            );
        }
    }

    if let Some(mem) = m.memory.filter(|mem| mem.total > 0) {
        if mem.available < LOW_MEMORY || ratio(mem.used, mem.total) > HIGH_MEMORY_USED {
            add(
                Level::Warning,
                Problem::LowMemory {
                    available: mem.available,
                    total: mem.total,
                },
            );
        }
        // Swap in use is only a problem when memory is short as well.
        if mem.swap_total > 0
            && ratio(mem.swap_used, mem.swap_total) > HEAVY_SWAP_USED
            && ratio(mem.available, mem.total) < SWAP_PRESSURE_AVAILABLE
        {
            add(
                Level::Warning,
                Problem::HeavySwap {
                    used: mem.swap_used,
                    total: mem.swap_total,
                },
            );
        }
    }

    for d in m.disks {
        // zram is compressed RAM, nearly full by design. A disk with nothing
        // mounted can't be measured, and reads as 0 free of 0, not full.
        if d.disk.is_swap {
            continue;
        }
        let Some(space) = d.space else { continue };
        let size = space.used.saturating_add(space.free);
        if size > 0 && ratio(space.free, size) < NEARLY_FULL {
            add(
                Level::Warning,
                Problem::DiskNearlyFull {
                    disk: d.disk.label().to_owned(),
                    free: space.free,
                    size,
                },
            );
        }
    }

    // A drive that expects to fail is the loudest thing here: everything
    // else is a busy machine, this is one that will stop working.
    for d in m.drives {
        let drive = d.name.to_owned();
        let h = d.health;
        if h.failing {
            add(Level::Critical, Problem::DriveFailing { drive });
        } else if h.spare_low {
            add(Level::Warning, Problem::DriveOutOfSpares { drive });
        } else if let Some(wear) = h.wear.filter(|&w| w >= WORN) {
            add(Level::Warning, Problem::DriveWorn { drive, wear });
        }
    }

    if !m.failed_services.is_empty() {
        add(
            Level::Warning,
            Problem::ServicesFailed {
                names: m.failed_services.to_vec(),
            },
        );
    }

    // Stable, so each level keeps the order above.
    alerts.sort_by_key(|a| std::cmp::Reverse(a.level));
    alerts
}

/// The loudest level among `alerts`; `None` for a healthy machine.
pub fn worst(alerts: &[Alert]) -> Option<Level> {
    alerts.iter().map(|a| a.level).max()
}

fn ratio(part: u64, whole: u64) -> f64 {
    part as f64 / whole as f64
}

const KIB: f64 = 1024.0;
const MIB: f64 = KIB * 1024.0;
const GIB: f64 = MIB * 1024.0;
const TIB: f64 = GIB * 1024.0;

/// "10.71 GiB", for memory, which people know in GiB.
fn gib(b: u64) -> String {
    format!("{:.2} GiB", b as f64 / GIB)
}

/// "476.94 GiB", "1.82 TiB", "512.0 MiB": binary units, as Go's
/// `format.Bytes`.
fn bytes(b: u64) -> String {
    let f = b as f64;
    if f >= TIB {
        format!("{:.2} TiB", f / TIB)
    } else if f >= GIB {
        format!("{:.2} GiB", f / GIB)
    } else if f >= MIB {
        format!("{:.1} MiB", f / MIB)
    } else if f >= KIB {
        format!("{:.0} KiB", f / KIB)
    } else {
        format!("{b} B")
    }
}

#[cfg(test)]
mod tests;
