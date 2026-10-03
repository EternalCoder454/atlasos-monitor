//! Batteries and power adapters (`/sys/class/power_supply`): charge, draw,
//! time left, health against the design capacity, cycles, the adapter.
//!
//! The kernel gives a battery in one of two shapes, by firmware: energies
//! in microwatt-hours with a rate in microwatts, or charges in
//! microamp-hours with a rate in microamps. Both come out here in
//! watt-hours and watts. A charge is turned into energy at the pack's
//! design voltage (`voltage_min_design`, ACPI's nominal voltage), not the
//! live one, so capacity and health don't drift with the load; the rate
//! uses the live voltage, since it is a live figure. Some drivers give the
//! rate negative while discharging: it is read as a magnitude, and the
//! direction comes from the status.
//!
//! The percentage is the firmware's own (`capacity`) where there is one,
//! the figure Plasma's battery applet shows through UPower; otherwise
//! energy over full. Time left is the driver's own estimate where it has
//! one (`time_to_empty_now`), else the energy left over a rate averaged
//! over about half a minute: an instantaneous rate makes the estimate jump
//! by hours every time something compiles. The average runs on the boot
//! clock, so a resume from suspend starts it over. Charging, "full" is the
//! charge limit when one is set (`charge_control_end_threshold`), and the
//! driver's own time to full, which counts to 100%, is not used then.
//! The charge limit and cycle count are read every ten ticks, not every
//! tick: on a ThinkPad each read runs an ACPI method, on others it is an
//! I2C transfer, and they change over weeks.
//!
//! A peripheral's battery (a wireless mouse, a game controller) is
//! `scope=Device` and left out: it isn't the machine's, and reading a
//! Bluetooth device's capacity can wake it. A hot-swap bay's empty slot
//! (`present=0`) is left out of the reading until a pack is in it. ACPI
//! takes a pulled pack's supply away instead: its held files stop reading,
//! and the pack is left out and the directory listed again at once.
//!
//! [`PowerSampler`] holds every file it reads open, so a tick is a few
//! `pread`s per supply. It lists `/sys/class/power_supply` again every ten
//! ticks and starts over when a supply came or went (a dock, a pack pulled
//! from its bay).

use std::cmp::Ordering;
use std::fs;
use std::os::unix::fs::DirEntryExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::sysfs::{self, HeldFile};

const SUPPLY_DIR: &str = "/sys/class/power_supply";

/// Ticks between looks at `/sys/class/power_supply` for supplies that came
/// or went.
const RESCAN_TICKS: u32 = 10;
/// Time constant of the rate the time estimate uses, in seconds.
const RATE_SECONDS: f64 = 30.0;
/// An estimate past this is a rate near zero, not a forecast.
const MAX_HOURS: f64 = 48.0;
/// More than USB Power Delivery's 240 W is a misread offer.
const MAX_OFFER_WATTS: f64 = 240.0;

/// What a battery is doing, as its driver says.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Status {
    Charging,
    Discharging,
    /// Plugged in and held: at the charge limit, or the firmware decided.
    NotCharging,
    Full,
    #[default]
    Unknown,
}

impl Status {
    fn parse(b: &[u8]) -> Self {
        match b.trim_ascii() {
            b"Charging" => Self::Charging,
            b"Discharging" => Self::Discharging,
            b"Not charging" => Self::NotCharging,
            b"Full" => Self::Full,
            _ => Self::Unknown,
        }
    }

    /// "Charging", "Discharging", "Not charging", "Full", "Unknown".
    pub fn label(self) -> &'static str {
        match self {
            Self::Charging => "Charging",
            Self::Discharging => "Discharging",
            Self::NotCharging => "Not charging",
            Self::Full => "Full",
            Self::Unknown => "Unknown",
        }
    }
}

/// One battery, or all of them summed ([`Supplies::total`]). A figure the
/// firmware doesn't give is `None`.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Battery {
    /// The kernel's name, `BAT0`: stable while the pack is in, so it keys
    /// the pack's page. Empty for the sum of several packs.
    pub name: String,
    pub vendor: String,
    pub model: String,
    /// "Li-ion", "Li-poly". Empty when the driver says "Unknown".
    pub technology: String,
    pub status: Status,
    /// 0..=100.
    pub percent: Option<f64>,
    /// Energy in the pack now, when full today, and when new.
    pub energy_wh: Option<f64>,
    pub full_wh: Option<f64>,
    pub design_wh: Option<f64>,
    /// The rate energy flows in or out, always positive.
    pub watts: Option<f64>,
    pub volts: Option<f64>,
    /// Charge cycles. `None` where the firmware says 0, which most use for
    /// "not counted".
    pub cycles: Option<u32>,
    /// What full holds against what it held new, as a percentage (at most
    /// 100: a new pack often holds a little over its design).
    pub health: Option<f64>,
    /// The charge limit set in the firmware, 1..=99. `None` when there is
    /// none or it is 100.
    pub charge_limit: Option<u8>,
    /// To empty while discharging, to full (or the charge limit) while
    /// charging. `None` otherwise or when the rate is unknown.
    pub time_left: Option<Duration>,
}

impl Battery {
    /// Grades the wear: 0 at 90% health or more, 1 ("worn") from 70%, 2
    /// ("heavily worn") below. 0 without a health figure.
    pub fn wear(&self) -> u8 {
        match self.health {
            Some(h) if h < 70.0 => 2,
            Some(h) if h < 90.0 => 1,
            _ => 0,
        }
    }
}

/// What a power adapter is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum AdapterKind {
    /// The barrel or AC input (ACPI's `AC`, `ADP1`).
    Mains,
    /// A USB(-C) port that can take power.
    Usb,
    /// A wireless charging pad.
    Wireless,
}

/// A source of power for the machine.
#[derive(Debug, Clone, PartialEq)]
pub struct Adapter {
    /// "AC Adapter", "USB-C Port 2", "USB Charger", "Wireless Charger".
    pub name: String,
    /// The kernel's name, `ADP1`, `ucsi-source-psy-USBC000:001`.
    pub kernel: String,
    pub kind: AdapterKind,
    /// Plugged in and giving power.
    pub online: bool,
    /// The most the charger offers, while online: its highest USB Power
    /// Delivery voltage times that offer's current (`voltage_max` ×
    /// `current_max`). `None` for a plain AC input.
    pub watts: Option<f64>,
}

/// One reading of every supply.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Supplies {
    /// Every pack summed: the battery percentage people mean. The one
    /// pack itself on an ordinary laptop, `None` on a desktop.
    pub total: Option<Battery>,
    /// Each pack on its own, by kernel name. A machine with two (a
    /// ThinkPad's internal and hot-swap packs) drains them one after the
    /// other, which the sum hides.
    pub packs: Vec<Battery>,
    pub adapters: Vec<Adapter>,
}

impl Supplies {
    /// Whether any adapter is giving power. `None` when the machine lists
    /// no adapter at all.
    pub fn on_ac(&self) -> Option<bool> {
        (!self.adapters.is_empty()).then(|| self.adapters.iter().any(|a| a.online))
    }
}

/// Whether the machine has a battery of its own. Cheap: for deciding
/// whether the sidebar lists Battery before anyone opens it.
pub fn available() -> bool {
    available_in(Path::new(SUPPLY_DIR))
}

fn available_in(root: &Path) -> bool {
    supply_dirs(root)
        .iter()
        .any(|(_, dir, _)| matches!(classify(dir), Some(Class::Battery)))
}

/// Every battery and adapter, held open. Owned by the sampling thread.
#[derive(Debug)]
pub struct PowerSampler {
    root: PathBuf,
    packs: Vec<Pack>,
    adapters: Vec<HeldAdapter>,
    /// The supplies at discovery, by name and inode.
    nodes: Vec<(String, u64)>,
    /// The rate behind the total's estimate, for more than one pack.
    total_rate: RateAverage,
    /// Each pack's rate in the units of its energy, beside
    /// `supplies.packs`, for the total's estimate.
    rates: Vec<Option<f64>>,
    supplies: Supplies,
    tick: u32,
    /// A held pack stopped reading: list the directory on the next tick.
    rescan: bool,
}

impl PowerSampler {
    /// Finds every supply and holds it open. The first [`Self::sample`]
    /// reads the values.
    pub fn new() -> Self {
        Self::with_root(SUPPLY_DIR.into())
    }

    fn with_root(root: PathBuf) -> Self {
        let mut s = Self {
            root,
            packs: Vec::new(),
            adapters: Vec::new(),
            nodes: Vec::new(),
            total_rate: RateAverage::default(),
            rates: Vec::new(),
            supplies: Supplies::default(),
            tick: 0,
            rescan: false,
        };
        s.discover();
        s
    }

    /// Reads every supply once. Every tenth tick also looks for supplies
    /// that came or went.
    pub fn sample(&mut self) -> &Supplies {
        self.sample_at(boot_clock())
    }

    /// One tick at `now` on the boot clock.
    fn sample_at(&mut self, now: Duration) -> &Supplies {
        self.tick = self.tick.wrapping_add(1);
        let slow_tick = self.tick.is_multiple_of(RESCAN_TICKS);
        if (std::mem::take(&mut self.rescan) || slow_tick) && self.changed() {
            self.discover();
        }
        let s = &mut self.supplies;
        s.packs.clear();
        self.rates.clear();
        for p in &mut self.packs {
            match p.read(now, slow_tick) {
                Read::Pack(b, rate) => {
                    s.packs.push(b);
                    self.rates.push(rate);
                }
                Read::Empty => {}
                Read::Gone(first) => self.rescan |= first,
            }
        }
        s.total = total(&s.packs, &self.rates, &mut self.total_rate, now);
        s.adapters.clear();
        s.adapters
            .extend(self.adapters.iter_mut().map(HeldAdapter::read));
        &self.supplies
    }

    /// The supplies as last sampled.
    pub fn supplies(&self) -> &Supplies {
        &self.supplies
    }

    fn changed(&self) -> bool {
        let nodes: Vec<(String, u64)> = supply_dirs(&self.root)
            .into_iter()
            .map(|(n, _, ino)| (n, ino))
            .collect();
        nodes != self.nodes
    }

    /// Finds every supply and opens its files, replacing what was held. A
    /// pack still there (same name and inode) keeps its averaged rate; the
    /// total's starts over.
    fn discover(&mut self) {
        let dirs = supply_dirs(&self.root);
        self.nodes = dirs.iter().map(|(n, _, ino)| (n.clone(), *ino)).collect();
        self.total_rate = RateAverage::default();
        let mut old = std::mem::take(&mut self.packs);
        let mut adapters = Vec::new();
        for (name, dir, ino) in dirs {
            match classify(&dir) {
                Some(Class::Battery) => {
                    let mut pack = Pack::open(name, &dir, ino);
                    if let Some(i) = old
                        .iter()
                        .position(|p| p.ino == ino && p.template.name == pack.template.name)
                    {
                        pack.rate = old.swap_remove(i).rate;
                    }
                    self.packs.push(pack);
                }
                Some(Class::Adapter(kind)) => adapters.push(HeldAdapter::open(name, &dir, kind)),
                None => {}
            }
        }
        name_adapters(&mut adapters);
        self.adapters = adapters;
    }
}

impl Default for PowerSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// What a supply directory is to this reader.
enum Class {
    Battery,
    Adapter(AdapterKind),
}

/// A battery of the machine's own with a figure to read, or an adapter.
/// Peripherals (`scope=Device`) are neither.
fn classify(dir: &Path) -> Option<Class> {
    let scope = sysfs::read_string(dir.join("scope")).unwrap_or_default();
    if scope.eq_ignore_ascii_case("device") {
        return None;
    }
    let kind = sysfs::read_string(dir.join("type"))?;
    match kind.as_str() {
        "Battery" => [
            "capacity",
            "energy_full",
            "charge_full",
            "energy_now",
            "charge_now",
        ]
        .iter()
        .any(|f| dir.join(f).exists())
        .then_some(Class::Battery),
        "Mains" => Some(Class::Adapter(AdapterKind::Mains)),
        // Older kernels name the USB flavours as types of their own.
        "USB" | "USB_C" | "USB_PD" | "USB_PD_DRP" | "USB_DCP" | "USB_CDP" | "USB_ACA"
        | "BrickID" => Some(Class::Adapter(AdapterKind::Usb)),
        "Wireless" => Some(Class::Adapter(AdapterKind::Wireless)),
        _ => None,
    }
}

/// The supply entries, by name, with their inodes: from the listing
/// itself, no `stat` needed.
fn supply_dirs(root: &Path) -> Vec<(String, PathBuf, u64)> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf, u64)> = entries
        .flatten()
        .filter_map(|e| Some((e.file_name().into_string().ok()?, e.path(), e.ino())))
        .collect();
    dirs.sort_by(|a, b| natural(&a.0, &b.0));
    dirs
}

/// Orders names with their numbers as numbers: `BAT2` before `BAT10`.
/// Names equal but for leading zeros fall back to their bytes.
fn natural(a: &str, b: &str) -> Ordering {
    natural_parts(a.as_bytes(), b.as_bytes()).then_with(|| a.cmp(b))
}

fn natural_parts(mut a: &[u8], mut b: &[u8]) -> Ordering {
    loop {
        match (a.first(), b.first()) {
            (None, None) => return Ordering::Equal,
            (None, Some(_)) => return Ordering::Less,
            (Some(_), None) => return Ordering::Greater,
            (Some(x), Some(y)) if x.is_ascii_digit() && y.is_ascii_digit() => {
                let run = |s: &[u8]| s.iter().take_while(|c| c.is_ascii_digit()).count();
                let (na, nb) = (run(a), run(b));
                // The digits without leading zeros, compared by length
                // then by digit.
                let digits = |s: &'_ [u8], n: usize| -> Vec<u8> {
                    s[..n].iter().copied().skip_while(|&c| c == b'0').collect()
                };
                let (da, db) = (digits(a, na), digits(b, nb));
                let order = da.len().cmp(&db.len()).then_with(|| da.cmp(&db));
                if order != Ordering::Equal {
                    return order;
                }
                (a, b) = (&a[na..], &b[nb..]);
            }
            (Some(x), Some(y)) => {
                if x != y {
                    return x.cmp(y);
                }
                (a, b) = (&a[1..], &b[1..]);
            }
        }
    }
}

/// How a pack gives its rate.
#[derive(Debug)]
enum Rate {
    /// `power_now`, microwatts.
    Watts(HeldFile),
    /// `current_now`, microamps, times the voltage.
    Amps(HeldFile),
}

/// What a pack read as on one tick.
#[expect(
    clippy::large_enum_variant,
    reason = "returned and unpacked at once, never stored"
)]
enum Read {
    /// The pack, and its rate in the units of its energy for the total's
    /// estimate.
    Pack(Battery, Option<f64>),
    /// An empty bay.
    Empty,
    /// Its files stopped reading: the supply went away. True the first
    /// tick it happens, when the directory is worth listing again.
    Gone(bool),
}

/// A battery's open files.
#[derive(Debug)]
struct Pack {
    /// The figures read once: name, vendor, model, technology.
    template: Battery,
    /// The supply's inode, which tells a pack put back under the same name.
    ino: u64,
    /// Its files stopped reading on an earlier tick.
    vanished: bool,
    present: Option<HeldFile>,
    status: Option<HeldFile>,
    capacity: Option<HeldFile>,
    /// Energies (µWh) when true, charges (µAh) when false.
    energy: bool,
    now: Option<HeldFile>,
    full: Option<HeldFile>,
    /// The design capacity, raw, read once.
    design: Option<u64>,
    /// The design voltage in volts, read once, for turning charges into
    /// energies.
    design_volts: Option<f64>,
    flow: Option<Rate>,
    voltage: Option<HeldFile>,
    /// Read on slow ticks only, and the values kept between.
    cycles: Option<HeldFile>,
    limit: Option<HeldFile>,
    cycle_count: Option<u32>,
    charge_limit: Option<u8>,
    to_empty: Option<HeldFile>,
    to_full: Option<HeldFile>,
    rate: RateAverage,
}

impl Pack {
    fn open(name: String, dir: &Path, ino: u64) -> Self {
        let text = |f: &str| {
            sysfs::read_string(dir.join(f))
                .filter(|s| !s.is_empty() && s != "Unknown")
                .unwrap_or_default()
        };
        let held = |f: &str| HeldFile::open(dir.join(f));
        let energy = dir.join("energy_full").exists() || dir.join("energy_now").exists();
        let (prefix, volts_to_wh) = if energy {
            ("energy", false)
        } else {
            ("charge", true)
        };
        // ACPI's design voltage is the nominal one. `voltage_max_design`
        // is the full-charge voltage, too high by about an eighth; the live
        // voltage is closer.
        let design_volts = volts_to_wh
            .then(|| sysfs::read_uint(dir.join("voltage_min_design")))
            .flatten()
            .filter(|&v| v > 0)
            .map(|uv| uv as f64 / 1e6);
        let mut cycles = held("cycle_count");
        let mut limit = held("charge_control_end_threshold");
        let (cycle_count, charge_limit) = slow_figures(&mut cycles, &mut limit);
        let flow = held("power_now")
            .map(Rate::Watts)
            .or_else(|| held("current_now").map(Rate::Amps));
        Self {
            template: Battery {
                name,
                vendor: text("manufacturer"),
                model: text("model_name"),
                technology: text("technology"),
                ..Battery::default()
            },
            ino,
            vanished: false,
            present: held("present"),
            status: held("status"),
            capacity: held("capacity"),
            energy,
            now: held(&format!("{prefix}_now")),
            full: held(&format!("{prefix}_full")),
            design: sysfs::read_uint(dir.join(format!("{prefix}_full_design"))),
            design_volts,
            flow,
            voltage: held("voltage_now"),
            cycles,
            limit,
            cycle_count,
            charge_limit,
            to_empty: held("time_to_empty_now"),
            to_full: held("time_to_full_now"),
            rate: RateAverage::default(),
        }
    }

    /// One reading. `slow` also reads the cycle count and charge limit.
    fn read(&mut self, now: Duration, slow: bool) -> Read {
        if self.present.as_mut().and_then(HeldFile::uint) == Some(0) {
            self.rate = RateAverage::default();
            return Read::Empty;
        }
        let uint = |f: &mut Option<HeldFile>| f.as_mut().and_then(HeldFile::uint);
        let mut b = self.template.clone();
        let status = self
            .status
            .as_mut()
            .and_then(HeldFile::bytes)
            .map(Status::parse);
        let level = uint(&mut self.now);
        let capacity = uint(&mut self.capacity);
        if status.is_none() && level.is_none() && capacity.is_none() {
            self.rate = RateAverage::default();
            return Read::Gone(!std::mem::replace(&mut self.vanished, true));
        }
        self.vanished = false;
        b.status = status.unwrap_or_default();
        b.volts = uint(&mut self.voltage)
            .filter(|&v| v > 0)
            .map(|uv| uv as f64 / 1e6);

        let full = uint(&mut self.full).filter(|&v| v > 0);
        let design = self.design.filter(|&v| v > 0);
        // Raw units to watt-hours: µWh, or µAh at the design voltage.
        let volts = if self.energy {
            Some(1.0)
        } else {
            self.design_volts.or(b.volts)
        };
        if let Some(v) = volts {
            let wh = |raw: u64| raw as f64 / 1e6 * v;
            b.energy_wh = level.map(wh);
            b.full_wh = full.map(wh);
            b.design_wh = design.map(wh);
        }
        // Past 100 is a sentinel (255) or a broken driver, not a charge.
        b.percent = capacity
            .filter(|&c| c <= 100)
            .map(|c| c as f64)
            .or_else(|| Some(percent(level? as f64, full? as f64)));
        b.health = full.zip(design).map(|(f, d)| percent(f as f64, d as f64));
        b.watts = match &mut self.flow {
            Some(Rate::Watts(f)) => f.int().map(|uw| uw.unsigned_abs() as f64 / 1e6),
            Some(Rate::Amps(f)) => f
                .int()
                .zip(b.volts.or(self.design_volts))
                .map(|(ua, v)| ua.unsigned_abs() as f64 / 1e6 * v),
            None => None,
        };
        if slow {
            (self.cycle_count, self.charge_limit) = slow_figures(&mut self.cycles, &mut self.limit);
        }
        b.cycles = self.cycle_count;
        b.charge_limit = self.charge_limit;

        // A charge's energy is at the design voltage and the rate at the
        // live one: the estimate needs the rate at the design voltage too,
        // or time left is off by their ratio.
        let rate = match (self.energy, volts, b.volts) {
            (false, Some(design), Some(live)) => b.watts.map(|w| w * design / live),
            _ => b.watts,
        };
        let averaged = self.rate.update(now, b.status, rate);
        let reported = match b.status {
            Status::Discharging => uint(&mut self.to_empty),
            Status::Charging if b.charge_limit.is_none() => uint(&mut self.to_full),
            _ => None,
        };
        b.time_left = reported
            .filter(|&s| s > 0 && (s as f64) <= MAX_HOURS * 3600.0)
            .map(Duration::from_secs)
            .or_else(|| estimate(&b, averaged));
        Read::Pack(b, rate)
    }
}

/// The cycle count (0 is "not counted") and the charge limit (100 is
/// none).
fn slow_figures(
    cycles: &mut Option<HeldFile>,
    limit: &mut Option<HeldFile>,
) -> (Option<u32>, Option<u8>) {
    let cycles = cycles
        .as_mut()
        .and_then(HeldFile::uint)
        .filter(|&c| c > 0)
        .and_then(|c| u32::try_from(c).ok());
    let limit = limit
        .as_mut()
        .and_then(HeldFile::uint)
        .filter(|l| (1..100).contains(l))
        .map(|l| l as u8);
    (cycles, limit)
}

/// `part` of `whole` as a percentage, at most 100.
fn percent(part: f64, whole: f64) -> f64 {
    (part / whole * 100.0).clamp(0.0, 100.0)
}

/// Time left at `watts`: the energy in the pack while discharging, the
/// energy to full (or the charge limit) while charging.
fn estimate(b: &Battery, watts: Option<f64>) -> Option<Duration> {
    let watts = watts.filter(|w| *w > 0.0)?;
    let wh = match b.status {
        Status::Discharging => b.energy_wh?,
        Status::Charging => {
            let target = b.full_wh? * f64::from(b.charge_limit.unwrap_or(100)) / 100.0;
            target - b.energy_wh?
        }
        _ => return None,
    };
    let hours = wh / watts;
    (hours > 0.0 && hours <= MAX_HOURS).then(|| Duration::from_secs_f64(hours * 3600.0))
}

/// The packs summed, with `rates` their rates in the units of their
/// energy. With one pack, that pack.
fn total(
    packs: &[Battery],
    rates: &[Option<f64>],
    rate: &mut RateAverage,
    now: Duration,
) -> Option<Battery> {
    match packs {
        [] => {
            *rate = RateAverage::default();
            None
        }
        [one] => Some(one.clone()),
        _ => {
            let sum = |f: fn(&Battery) -> Option<f64>| -> Option<f64> {
                packs.iter().map(f).sum::<Option<f64>>()
            };
            // A pack that is neither charging nor discharging and gives no
            // rate (an idle, full second pack often doesn't) adds nothing.
            // A moving pack without one leaves the sum unknown.
            let flow = |each: &mut dyn Iterator<Item = (&Battery, Option<f64>)>| {
                each.map(|(p, w)| match (w, p.status) {
                    (Some(w), _) => Some(w),
                    (None, Status::Charging | Status::Discharging) => None,
                    (None, _) => Some(0.0),
                })
                .sum::<Option<f64>>()
            };
            let any = |s: Status| packs.iter().any(|p| p.status == s);
            let status = if any(Status::Charging) {
                Status::Charging
            } else if any(Status::Discharging) {
                Status::Discharging
            } else if any(Status::NotCharging) {
                Status::NotCharging
            } else if packs.iter().all(|p| p.status == Status::Full) {
                Status::Full
            } else {
                Status::Unknown
            };
            let mut b = Battery {
                status,
                energy_wh: sum(|p| p.energy_wh),
                full_wh: sum(|p| p.full_wh),
                design_wh: sum(|p| p.design_wh),
                watts: flow(&mut packs.iter().map(|p| (p, p.watts))),
                ..Battery::default()
            };
            // Energy over full; else the packs' percentages weighted by
            // what each holds, never a plain mean of a big and a small pack.
            b.percent = match (b.energy_wh, b.full_wh) {
                (Some(e), Some(f)) if f > 0.0 => Some(percent(e, f)),
                (_, Some(f)) if f > 0.0 => packs
                    .iter()
                    .map(|p| Some(p.percent? * p.full_wh?))
                    .sum::<Option<f64>>()
                    .map(|s| (s / f).clamp(0.0, 100.0)),
                _ => None,
            };
            b.health = b
                .full_wh
                .zip(b.design_wh)
                .filter(|(_, d)| *d > 0.0)
                .map(|(f, d)| percent(f, d));
            let first = packs[0].charge_limit;
            b.charge_limit = first.filter(|_| packs.iter().all(|p| p.charge_limit == first));
            let moving = flow(&mut packs.iter().zip(rates.iter().copied()));
            let averaged = rate.update(now, b.status, moving);
            b.time_left = estimate(&b, averaged);
            Some(b)
        }
    }
}

/// The rate behind a time estimate: an exponential average over
/// [`RATE_SECONDS`], started over when the status changes (plugged in,
/// unplugged) or the rate is unknown.
#[derive(Debug, Default)]
struct RateAverage {
    /// When it was last updated, on the boot clock, and the status then.
    last: Option<(Duration, Status)>,
    watts: Option<f64>,
}

impl RateAverage {
    fn update(&mut self, now: Duration, status: Status, watts: Option<f64>) -> Option<f64> {
        let previous = self.last.replace((now, status));
        let Some(w) = watts.filter(|w| *w > 0.0) else {
            self.watts = None;
            return None;
        };
        self.watts = Some(match (previous, self.watts) {
            (Some((then, s)), Some(avg)) if s == status => {
                let dt = now.saturating_sub(then).as_secs_f64();
                avg + (w - avg) * (1.0 - (-dt / RATE_SECONDS).exp())
            }
            _ => w,
        });
        self.watts
    }
}

/// The time since boot, suspend included (`CLOCK_BOOTTIME`): the rate's
/// average starts over after a suspend, which `Instant` doesn't count.
fn boot_clock() -> Duration {
    let mut ts = libc::timespec {
        tv_sec: 0,
        tv_nsec: 0,
    };
    // SAFETY: `ts` is a valid timespec for the call to fill.
    let ok = unsafe { libc::clock_gettime(libc::CLOCK_BOOTTIME, &mut ts) } == 0;
    if !ok {
        return Duration::ZERO;
    }
    Duration::new(
        u64::try_from(ts.tv_sec).unwrap_or(0),
        u32::try_from(ts.tv_nsec).unwrap_or(0),
    )
}

/// An adapter's open files.
#[derive(Debug)]
struct HeldAdapter {
    template: Adapter,
    /// A USB Type-C port rather than a plain USB charger.
    type_c: bool,
    online: Option<HeldFile>,
    voltage_max: Option<HeldFile>,
    current_max: Option<HeldFile>,
}

impl HeldAdapter {
    fn open(kernel: String, dir: &Path, kind: AdapterKind) -> Self {
        let held = |f: &str| HeldFile::open(dir.join(f));
        // `usb_type` lists what the port can be, the current one in
        // brackets: "[C] PD PD_PPS" on a UCSI port.
        let usb_types = sysfs::read_string(dir.join("usb_type")).unwrap_or_default();
        let kernel_lower = kernel.to_ascii_lowercase();
        let type_c = usb_types
            .split_ascii_whitespace()
            .map(|t| t.trim_matches(['[', ']']))
            .any(|t| t == "C" || t.starts_with("PD"))
            || matches!(
                sysfs::read_string(dir.join("type")).as_deref(),
                Some("USB_C" | "USB_PD" | "USB_PD_DRP")
            )
            || ["ucsi", "tcpm", "typec", "usbpd", "usbc"]
                .iter()
                .any(|n| kernel_lower.contains(n));
        Self {
            type_c,
            template: Adapter {
                name: String::new(),
                kernel,
                kind,
                online: false,
                watts: None,
            },
            online: held("online"),
            voltage_max: held("voltage_max"),
            current_max: held("current_max"),
        }
    }

    fn read(&mut self) -> Adapter {
        let mut a = self.template.clone();
        // 2 is a programmable (PPS) source: online too.
        a.online = self
            .online
            .as_mut()
            .and_then(HeldFile::uint)
            .is_some_and(|v| v > 0);
        if a.online {
            let uv = self.voltage_max.as_mut().and_then(HeldFile::uint);
            let ua = self.current_max.as_mut().and_then(HeldFile::uint);
            a.watts = uv
                .zip(ua)
                .filter(|&(v, c)| v > 0 && c > 0)
                .map(|(v, c)| v as f64 / 1e6 * (c as f64 / 1e6))
                .filter(|&w| w <= MAX_OFFER_WATTS);
        }
        a
    }
}

/// "AC Adapter", "USB-C Port", "USB Charger", "Wireless Charger",
/// numbered from 1 when a machine has several of one, in kernel name order.
fn name_adapters(adapters: &mut [HeldAdapter]) {
    // Type-C ports before plain USB chargers, each by kernel name.
    adapters.sort_by(|a, b| {
        a.template
            .kind
            .cmp(&b.template.kind)
            .then_with(|| b.type_c.cmp(&a.type_c))
            .then_with(|| natural(&a.template.kernel, &b.template.kernel))
    });
    let noun = |a: &HeldAdapter| match a.template.kind {
        AdapterKind::Mains => "AC Adapter",
        AdapterKind::Usb if a.type_c => "USB-C Port",
        AdapterKind::Usb => "USB Charger",
        AdapterKind::Wireless => "Wireless Charger",
    };
    for kind in [
        "AC Adapter",
        "USB-C Port",
        "USB Charger",
        "Wireless Charger",
    ] {
        let count = adapters.iter().filter(|a| noun(a) == kind).count();
        let mut i = 0;
        for a in adapters.iter_mut() {
            if noun(a) != kind {
                continue;
            }
            i += 1;
            let noun = kind;
            a.template.name = if count > 1 {
                format!("{noun} {i}")
            } else {
                noun.to_owned()
            };
        }
    }
}

#[cfg(test)]
mod tests;
