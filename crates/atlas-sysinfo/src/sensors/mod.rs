//! Every temperature, fan, voltage, current and power reading the kernel
//! has (hwmon), each named for what it is.
//!
//! The CPU and GPU pages show the one or two temperatures that matter to
//! them. A machine has a dozen more: drives, memory modules, network cards,
//! the motherboard's chip and its fans. The kernel names them by driver
//! (`spd5118`, `r8169_0_600:00`), so most of the work is saying instead
//! what the hardware is ([`names`]): a drive by its model, a memory module
//! by its slot, a graphics card by the name the GPU page uses, the
//! processor by its model with its cores as P-cores and E-cores on a
//! hybrid chip. A processor with more than four per-core temperatures has
//! them [`Reading::folded`] behind its package reading.
//!
//! [`Sensors`] finds the devices when it is made and holds every reading's
//! file open, so a tick is one `pread` per reading. It lists
//! `/sys/class/hwmon` again every ten ticks and starts over when a device
//! came or went (a dock, a driver reloaded). A node is known by its name
//! and inode, so one that went and came back under the same number in
//! between is noticed too: its held files would read nothing.
//!
//! A graphics card or network adapter that is runtime-suspended (a
//! laptop's sleeping discrete GPU, an Ethernet chip with no cable) is not
//! read: on many kernels reading its hwmon wakes it. It shows as
//! [`Device::asleep`] until it wakes for some other reason, the same rule
//! as [`crate::gpu`]. Other devices are read whatever their bus is doing:
//! the SMBus controller the memory sensors sit behind suspends between
//! every transfer, and waking it is what it is for.
//!
//! A motherboard chip lists every header it has, connected or not: fans at
//! 0 RPM and temperatures from unconnected thermistors (-128 °C, 127 °C).
//! Those are left out, and checked again every 30 ticks in case a fan
//! spins up.

mod names;

use std::collections::HashMap;
use std::fs;
use std::os::unix::fs::DirEntryExt;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::gpu::PciSlot;
use crate::sysfs::{self, HeldFile};
use names::{Context, Facts, Labelling};

const HWMON_DIR: &str = "/sys/class/hwmon";

/// Ticks between looks at `/sys/class/hwmon` for devices that came or went.
const RESCAN_TICKS: u32 = 10;
/// Ticks between checks on the motherboard headers left out as unconnected.
const RECHECK_TICKS: u32 = 30;
/// More per-core temperatures than this are folded behind the package.
const FOLD_OVER: usize = 4;

/// What a reading measures. The order is the order on the page.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Kind {
    #[default]
    Temperature,
    Fan,
    Voltage,
    Current,
    Power,
}

impl Kind {
    /// The unit a value is in: `°C`, `RPM`, `V`, `A`, `W`.
    pub fn unit(self) -> &'static str {
        match self {
            Self::Temperature => "°C",
            Self::Fan => "RPM",
            Self::Voltage => "V",
            Self::Current => "A",
            Self::Power => "W",
        }
    }

    /// Decimal places worth showing: whole degrees and turns, millivolts,
    /// hundredths of an amp, tenths of a watt.
    pub fn decimals(self) -> usize {
        match self {
            Self::Temperature | Self::Fan => 0,
            Self::Voltage => 3,
            Self::Current => 2,
            Self::Power => 1,
        }
    }

    /// The label of a reading the driver doesn't label.
    fn noun(self) -> &'static str {
        match self {
            Self::Temperature => "Temperature",
            Self::Fan => "Fan",
            Self::Voltage => "Voltage",
            Self::Current => "Current",
            Self::Power => "Power",
        }
    }

    /// hwmon's raw units per shown unit: millidegrees, millivolts,
    /// milliamps, microwatts, and fans in plain RPM.
    fn divisor(self) -> f64 {
        match self {
            Self::Fan => 1.0,
            Self::Power => 1e6,
            _ => 1e3,
        }
    }
}

/// What sort of hardware a device is. The order is the order on the page.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Category {
    Processor,
    Graphics,
    Memory,
    Storage,
    Motherboard,
    Network,
    PowerSupply,
    Other,
}

/// One sensor.
#[derive(Debug, Clone, PartialEq)]
pub struct Reading {
    /// What it measures: "Hotspot", "P-core 3", "Fan 2", "Core voltage".
    pub label: String,
    pub kind: Kind,
    /// In the kind's unit. `None` when the read failed or the device is
    /// asleep.
    pub value: Option<f64>,
    /// The hardware's own limits, in °C, for temperatures that have them
    /// (`tempN_max`, `tempN_crit`).
    pub high: Option<f64>,
    pub critical: Option<f64>,
    /// A per-core temperature, one of many: the page folds these behind
    /// the package reading.
    pub folded: bool,
    /// The attribute it comes from, `temp3`: unique within its device, so
    /// with [`Device::node`] it keys the row.
    pub attribute: String,
    /// The N in `temp<N>`, for the order within a kind.
    index: u32,
}

impl Reading {
    /// Grades a temperature against the hardware's limits: 2 at or past
    /// critical, 1 at or past high, else 0. Always 0 without a value or
    /// limits.
    pub fn warmth(&self) -> u8 {
        let Some(v) = self.value.filter(|_| self.kind == Kind::Temperature) else {
            return 0;
        };
        if self.critical.is_some_and(|c| v >= c) {
            2
        } else if self.high.is_some_and(|h| v >= h) {
            1
        } else {
            0
        }
    }

    /// The value with its unit, "52 °C", "0.692 V", or a dash.
    pub fn display(&self) -> String {
        match self.value {
            Some(v) => format!("{v:.*} {}", self.kind.decimals(), self.kind.unit()),
            None => "—".to_owned(),
        }
    }
}

/// One piece of hardware and its sensors.
#[derive(Debug, Clone, PartialEq)]
pub struct Device {
    /// "Samsung SSD 970 EVO Plus 1TB", "Memory Slot 2", "Wi-Fi Adapter".
    pub name: String,
    pub category: Category,
    /// The kernel's name for it (`spd5118`), shown small beside the name:
    /// it is what a search engine finds.
    pub driver: String,
    /// The hwmon node, `hwmon3`. Stable while the device exists.
    pub node: String,
    /// Temperatures first, then fans, voltages, currents and power, each
    /// in the driver's order.
    pub readings: Vec<Reading>,
    /// Behind a runtime-suspended PCI device and left asleep: every value
    /// is `None`.
    pub asleep: bool,
}

/// Whether the machine has any sensor at all. Cheap: for deciding whether
/// the sidebar lists Sensors before anyone opens it.
pub fn available() -> bool {
    available_in(Path::new(HWMON_DIR))
}

fn available_in(root: &Path) -> bool {
    hwmon_dirs(root).iter().any(|(_, dir, _)| {
        fs::read_dir(dir).is_ok_and(|entries| {
            entries
                .flatten()
                .any(|e| e.file_name().to_str().and_then(parse_attribute).is_some())
        })
    })
}

/// Where discovery looks: the live system, or a fake tree in the tests.
#[derive(Debug, Clone)]
struct Roots {
    hwmon: PathBuf,
    /// `/sys/devices/system/cpu`, for core IDs.
    cpu: PathBuf,
    /// The efficiency cores of a hybrid Intel processor, a CPU list.
    atom_cpus: PathBuf,
    cpuinfo: PathBuf,
    /// `/sys/class/drm`, for the graphics cards' names.
    drm: PathBuf,
}

impl Default for Roots {
    fn default() -> Self {
        Self {
            hwmon: HWMON_DIR.into(),
            cpu: "/sys/devices/system/cpu".into(),
            atom_cpus: "/sys/devices/cpu_atom/cpus".into(),
            cpuinfo: "/proc/cpuinfo".into(),
            drm: "/sys/class/drm".into(),
        }
    }
}

/// Every sensor, held open. Owned by the sampling thread while the Sensors
/// page is on screen.
#[derive(Debug)]
pub struct Sensors {
    roots: Roots,
    devices: Vec<Device>,
    /// One per device, in the same order.
    held: Vec<Held>,
    /// The hwmon nodes at discovery, by name and inode.
    nodes: Vec<(String, u64)>,
    /// Motherboard headers left out as unconnected, and their kinds.
    unconnected: Vec<(PathBuf, Kind)>,
    tick: u32,
}

/// A device's open files.
#[derive(Debug)]
struct Held {
    /// The PCI device's `power/runtime_status`, for a graphics card or
    /// network adapter.
    runtime_status: Option<HeldFile>,
    /// One per reading, in the same order.
    sources: Vec<Source>,
    /// The device was asleep at discovery, so its temperature limits are
    /// still to be read.
    limits_pending: bool,
}

/// Where a reading's value comes from.
#[derive(Debug)]
enum Source {
    /// `<attr>_input`, or for power `_average` then `_input`: some amdgpu
    /// firmware lists both and fails reads of the average.
    Value {
        files: [Option<HeldFile>; 2],
        divisor: f64,
        /// `temp3`, beside which the limits are.
        base: PathBuf,
    },
    /// Power from an energy counter, for a device with no power file
    /// (Intel's discrete cards).
    Energy(Energy),
}

impl Source {
    fn read(&mut self) -> Option<f64> {
        match self {
            Self::Value { files, divisor, .. } => files
                .iter_mut()
                .find_map(|f| f.as_mut()?.int())
                .map(|raw| raw as f64 / *divisor),
            Self::Energy(e) => e.watts(),
        }
    }
}

impl Sensors {
    /// Finds every sensor and holds it open. The first [`Self::sample`]
    /// reads the values.
    pub fn new() -> Self {
        Self::with_roots(Roots::default())
    }

    fn with_roots(roots: Roots) -> Self {
        let mut s = Self {
            roots,
            devices: Vec::new(),
            held: Vec::new(),
            nodes: Vec::new(),
            unconnected: Vec::new(),
            tick: 0,
        };
        s.discover();
        s
    }

    /// Reads every sensor once. Every tenth tick also looks for devices
    /// that came or went.
    pub fn sample(&mut self) -> &[Device] {
        self.tick = self.tick.wrapping_add(1);
        if self.tick.is_multiple_of(RESCAN_TICKS) && self.changed() {
            self.discover();
        }
        for (d, h) in self.devices.iter_mut().zip(&mut self.held) {
            d.asleep = h.asleep();
            if d.asleep {
                for (r, s) in d.readings.iter_mut().zip(&mut h.sources) {
                    r.value = None;
                    if let Source::Energy(e) = s {
                        e.rest();
                    }
                }
                continue;
            }
            if h.limits_pending {
                h.limits_pending = false;
                for (r, s) in d.readings.iter_mut().zip(&h.sources) {
                    if let (Kind::Temperature, Source::Value { base, .. }) = (r.kind, s) {
                        (r.high, r.critical) = limits(base);
                    }
                }
            }
            for (r, s) in d.readings.iter_mut().zip(&mut h.sources) {
                r.value = s.read();
            }
        }
        &self.devices
    }

    /// The devices as last sampled.
    pub fn devices(&self) -> &[Device] {
        &self.devices
    }

    /// Whether a device came or went, or a header left out as unconnected
    /// now reads something.
    fn changed(&self) -> bool {
        let nodes: Vec<(String, u64)> = hwmon_dirs(&self.roots.hwmon)
            .into_iter()
            .map(|(n, _, ino)| (n, ino))
            .collect();
        if nodes != self.nodes {
            return true;
        }
        self.tick.is_multiple_of(RECHECK_TICKS)
            && self.unconnected.iter().any(|(path, kind)| {
                sysfs::read_string(path)
                    .and_then(|s| sysfs::parse_int(s.as_bytes()))
                    .is_some_and(|raw| connected(*kind, raw))
            })
    }

    /// Finds every device and opens its readings, replacing what was held.
    fn discover(&mut self) {
        let dirs = hwmon_dirs(&self.roots.hwmon);
        self.nodes = dirs.iter().map(|(n, _, ino)| (n.clone(), *ino)).collect();
        self.unconnected.clear();

        let facts: Vec<(String, PathBuf, Facts, Option<PathBuf>)> = dirs
            .into_iter()
            .map(|(node, dir, _)| {
                let (facts, pci_dir) = facts(&dir);
                (node, dir, facts, pci_dir)
            })
            .collect();
        let ctx = context(&self.roots, facts.iter().map(|(_, _, f, _)| f));

        let mut found: Vec<(Device, Held, Option<String>)> = Vec::new();
        for (node, dir, facts, pci_dir) in facts {
            let (name, category, tell) = names::identify(&facts, &ctx);
            let runtime_status = pci_dir
                .filter(|_| matches!(category, Category::Graphics | Category::Network))
                .and_then(|p| HeldFile::open(p.join("power/runtime_status")));
            let mut held = Held {
                runtime_status,
                sources: Vec::new(),
                limits_pending: false,
            };
            let asleep = held.asleep();
            held.limits_pending = asleep;
            let labelling = labelling(&self.roots, &dir, &facts);
            let readings = readings(
                &dir,
                category,
                asleep,
                &labelling,
                &mut held.sources,
                &mut self.unconnected,
            );
            if readings.is_empty() {
                continue;
            }
            let device = Device {
                name,
                category,
                driver: facts.driver,
                node,
                readings,
                asleep,
            };
            found.push((device, held, tell));
        }

        let order = |found: &mut [(Device, Held, Option<String>)]| {
            found.sort_by(|(a, ..), (b, ..)| {
                (a.category, &a.name, node_number(&a.node)).cmp(&(
                    b.category,
                    &b.name,
                    node_number(&b.node),
                ))
            });
        };
        order(&mut found);
        tell_apart(&mut found);
        // Again with the suffixes: "(nvme0)" before "(nvme1)", whatever
        // their hwmon numbers.
        order(&mut found);
        (self.devices, self.held) = found.into_iter().map(|(d, h, _)| (d, h)).unzip();
    }
}

impl Default for Sensors {
    fn default() -> Self {
        Self::new()
    }
}

impl Held {
    /// Whether the PCI device it is behind is suspended. One `pread` a
    /// tick, and the reason nothing else is read while it is.
    fn asleep(&mut self) -> bool {
        self.runtime_status
            .as_mut()
            .and_then(HeldFile::bytes)
            .is_some_and(|b| b.trim_ascii() == b"suspended")
    }
}

/// The `hwmonN` entries, by number, with their inodes: from the listing
/// itself, no `stat` needed.
fn hwmon_dirs(root: &Path) -> Vec<(String, PathBuf, u64)> {
    let Ok(entries) = fs::read_dir(root) else {
        return Vec::new();
    };
    let mut dirs: Vec<(String, PathBuf, u64)> = entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            node_number(&name)?;
            Some((name, e.path(), e.ino()))
        })
        .collect();
    dirs.sort_by_key(|(n, ..)| node_number(n));
    dirs
}

fn node_number(node: &str) -> Option<u32> {
    node.strip_prefix("hwmon")?.parse().ok()
}

/// What a hwmon directory says about its device, and the PCI device
/// directory it is behind.
fn facts(dir: &Path) -> (Facts, Option<PathBuf>) {
    let driver = sysfs::read_string(dir.join("name")).unwrap_or_default();
    let device = fs::canonicalize(dir.join("device")).ok();
    let file_name = |p: &Path| p.file_name().map(|n| n.to_string_lossy().into_owned());
    let address = device.as_deref().and_then(file_name).unwrap_or_default();
    // The deepest PCI device on the path: the card itself, or the drive's
    // or network chip's controller.
    let pci_dir = device.as_deref().and_then(|d| {
        d.ancestors()
            .find(|a| file_name(a).is_some_and(|n| PciSlot::parse(n.as_bytes()).is_some()))
            .map(Path::to_path_buf)
    });
    let pci = pci_dir.as_deref().and_then(file_name);
    let (model, block) = match (driver.as_str(), &device) {
        ("nvme", Some(d)) => (read_model(d), Some(address.clone())),
        ("drivetemp", Some(d)) => (
            read_model(d),
            fs::read_dir(d.join("block"))
                .ok()
                .and_then(|mut e| e.next()?.ok())
                .map(|e| e.file_name().to_string_lossy().into_owned()),
        ),
        _ => (None, None),
    };
    let facts = Facts {
        driver,
        address,
        pci,
        model,
        block,
    };
    (facts, pci_dir)
}

/// A drive's model, with its vendor unless that is the SCSI layer's
/// placeholder for an ATA drive.
fn read_model(device: &Path) -> Option<String> {
    let model = sysfs::read_string(device.join("model")).filter(|m| !m.is_empty())?;
    let vendor = sysfs::read_string(device.join("vendor")).unwrap_or_default();
    Some(
        if vendor.is_empty() || vendor == "ATA" || model.starts_with(&vendor) {
            model
        } else {
            format!("{vendor} {model}")
        },
    )
}

/// Looks up what the devices' names need, only the parts they need.
fn context<'a>(roots: &Roots, facts: impl Iterator<Item = &'a Facts> + Clone) -> Context {
    let any = |names: &[&str]| facts.clone().any(|f| names.contains(&f.driver.as_str()));
    let processor = any(&["coretemp", "k10temp", "zenpower", "k8temp", "via_cputemp"])
        .then(|| fs::read_to_string(&roots.cpuinfo).ok())
        .flatten()
        .map(|text| names::processor_name(&crate::stats::cpu::parse_cpuinfo(&text).model))
        .filter(|n| !n.is_empty());
    let cards = if any(&["amdgpu", "radeon", "nouveau", "i915", "xe", "nvidia"]) {
        crate::gpu::cards_in(&roots.drm)
            .into_iter()
            .filter(|c| !c.slot.is_empty())
            .map(|c| (c.slot, c.name))
            .collect()
    } else {
        Vec::new()
    };
    Context { processor, cards }
}

/// How one device's readings are labelled: AMD's `Tctl` beside a `Tdie`,
/// and a hybrid Intel processor's cores by type.
fn labelling(roots: &Roots, dir: &Path, facts: &Facts) -> Labelling {
    let labels = || {
        fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                let n = e.file_name();
                let n = n.as_encoded_bytes();
                n.starts_with(b"temp") && n.ends_with(b"_label")
            })
            .filter_map(|e| sysfs::read_string(e.path()))
    };
    match facts.driver.as_str() {
        "k10temp" | "zenpower" => Labelling {
            has_die: labels().any(|l| l.eq_ignore_ascii_case("tdie")),
            ..Labelling::default()
        },
        "coretemp" => {
            let package = labels()
                .find_map(|l| l.strip_prefix("Package id ")?.parse().ok())
                .or_else(|| facts.address.strip_prefix("coretemp.")?.parse().ok())
                .unwrap_or(0);
            Labelling {
                cores: names::core_names(&core_types(roots, package)),
                ..Labelling::default()
            }
        }
        _ => Labelling::default(),
    }
}

/// Every core ID in `package`, with whether it is an efficiency core.
/// Empty on a processor that isn't hybrid.
fn core_types(roots: &Roots, package: u32) -> Vec<(u32, bool)> {
    let Some(atoms) = sysfs::read_string(&roots.atom_cpus).and_then(|s| parse_cpu_list(&s)) else {
        return Vec::new();
    };
    let Ok(entries) = fs::read_dir(&roots.cpu) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let name = e.file_name().into_string().ok()?;
            let cpu: u32 = name.strip_prefix("cpu")?.parse().ok()?;
            let topology = e.path().join("topology");
            if sysfs::read_uint(topology.join("physical_package_id"))? != u64::from(package) {
                return None;
            }
            let core = sysfs::read_uint(topology.join("core_id"))?;
            Some((u32::try_from(core).ok()?, atoms.contains(&cpu)))
        })
        .collect()
}

/// Parses a kernel CPU list, `0-15,32,40-47`. `None` for a malformed one.
fn parse_cpu_list(s: &str) -> Option<Vec<u32>> {
    let mut cpus = Vec::new();
    for part in s.trim().split(',').filter(|p| !p.is_empty()) {
        let (a, b) = part.split_once('-').unwrap_or((part, part));
        let (a, b): (u32, u32) = (a.parse().ok()?, b.parse().ok()?);
        if b < a || b - a > 8192 {
            return None;
        }
        cpus.extend(a..=b);
    }
    Some(cpus)
}

/// A reading's attribute, from a file name: `temp3_input` →
/// (Temperature, 3, "temp", Input).
fn parse_attribute(file: &str) -> Option<(Kind, u32, &str, Suffix)> {
    let (base, suffix) = file.split_once('_')?;
    let suffix = match suffix {
        "input" => Suffix::Input,
        "average" => Suffix::Average,
        _ => return None,
    };
    let digits = base.find(|c: char| c.is_ascii_digit())?;
    let (prefix, n) = base.split_at(digits);
    let index = n.parse().ok()?;
    let kind = match prefix {
        "temp" => Kind::Temperature,
        "fan" => Kind::Fan,
        "in" => Kind::Voltage,
        "curr" => Kind::Current,
        "power" => Kind::Power,
        "energy" if suffix == Suffix::Input => Kind::Power,
        _ => return None,
    };
    Some((kind, index, prefix, suffix))
}

/// What a device has of one attribute.
#[derive(Debug, Default)]
struct Attribute {
    kind: Kind,
    index: u32,
    /// An energy counter, read as power.
    energy: bool,
    /// It has an `_average` file, an `_input` file.
    average: bool,
    input: bool,
    /// The driver's `_label`, empty without one.
    label: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Suffix {
    Input,
    Average,
}

/// Opens a device's readings, sorted, and their sources into `sources`.
/// Motherboard headers that read as unconnected are left out and noted in
/// `unconnected`.
fn readings(
    dir: &Path,
    category: Category,
    asleep: bool,
    labelling: &Labelling,
    sources: &mut Vec<Source>,
    unconnected: &mut Vec<(PathBuf, Kind)>,
) -> Vec<Reading> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    // One entry per attribute (`power1`), with which files it has.
    let mut attributes: HashMap<String, Attribute> = HashMap::new();
    for e in entries.flatten() {
        let Ok(file) = e.file_name().into_string() else {
            continue;
        };
        let Some((kind, index, prefix, suffix)) = parse_attribute(&file) else {
            continue;
        };
        let a = attributes
            .entry(format!("{prefix}{index}"))
            .or_insert_with(|| Attribute {
                kind,
                index,
                energy: prefix == "energy",
                ..Attribute::default()
            });
        match suffix {
            Suffix::Input => a.input = true,
            Suffix::Average => a.average = true,
        }
    }
    // An energy counter only where the device has no power reading of its
    // own with that number.
    let powers: Vec<u32> = attributes
        .values()
        .filter(|a| a.kind == Kind::Power && !a.energy)
        .map(|a| a.index)
        .collect();
    attributes.retain(|_, a| !a.energy || !powers.contains(&a.index));

    let mut attributes: Vec<(String, Attribute)> = attributes.into_iter().collect();
    attributes.sort_by_key(|(_, a)| (a.kind, a.index));
    // Counted over every attribute the chip lists, headers left out as
    // unconnected included, so a label doesn't change when one connects.
    let mut unlabelled: HashMap<Kind, usize> = HashMap::new();
    for (attr, a) in &mut attributes {
        a.label = sysfs::read_string(dir.join(format!("{attr}_label"))).unwrap_or_default();
        if a.label.is_empty() {
            *unlabelled.entry(a.kind).or_default() += 1;
        }
    }

    let mut out = Vec::new();
    let mut cores = Vec::new();
    for (attr, a) in attributes {
        let Attribute {
            kind,
            index,
            energy,
            average,
            input,
            label: given,
        } = a;
        let base = dir.join(&attr);
        let file = |suffix: &str| HeldFile::open(dir.join(format!("{attr}_{suffix}")));
        let mut source = if energy {
            let Some(file) = file("input") else { continue };
            // The first sample takes the baseline: one now would be read
            // again microseconds later.
            Source::Energy(Energy::new(file, false))
        } else {
            // Power prefers the average; anything else reads its input.
            let files = match (kind == Kind::Power && average, input) {
                (true, true) => [file("average"), file("input")],
                (true, false) => [file("average"), None],
                (false, _) => [file("input"), None],
            };
            if files.iter().all(Option::is_none) {
                continue;
            }
            Source::Value {
                files,
                divisor: kind.divisor(),
                base: base.clone(),
            }
        };
        if category == Category::Motherboard
            && !asleep
            && let Source::Value {
                files: [Some(f), _],
                ..
            } = &mut source
            && let Some(raw) = f.int()
            && !connected(kind, raw)
        {
            unconnected.push((dir.join(format!("{attr}_input")), kind));
            continue;
        }
        let (high, critical) = if kind == Kind::Temperature && !asleep {
            limits(&base)
        } else {
            (None, None)
        };
        if kind == Kind::Temperature && names::core_id(&given).is_some() {
            cores.push(out.len());
        }
        out.push(Reading {
            label: names::label(
                &given,
                kind,
                index,
                unlabelled.get(&kind).copied().unwrap_or(0) > 1,
                labelling,
            ),
            kind,
            value: None,
            high,
            critical,
            folded: false,
            attribute: attr,
            index,
        });
        sources.push(source);
    }
    if cores.len() > FOLD_OVER {
        for i in cores {
            out[i].folded = true;
        }
    }
    debug_assert!(
        out.windows(2)
            .all(|w| (w[0].kind, w[0].index) <= (w[1].kind, w[1].index))
    );
    out
}

/// Whether a motherboard header's raw value says something is connected:
/// a fan that turns, a temperature a thermistor could read.
fn connected(kind: Kind, raw: i64) -> bool {
    match kind {
        Kind::Fan => raw > 0,
        Kind::Temperature => (-40_000..=125_000).contains(&raw) && raw != 0,
        _ => true,
    }
}

/// A temperature's limits in °C from `<base>_max` and `<base>_crit`. A
/// limit that isn't believable (0 or 255 °C for "not set") is none.
fn limits(base: &Path) -> (Option<f64>, Option<f64>) {
    let read = |suffix: &str| {
        let mut path = base.as_os_str().to_owned();
        path.push(suffix);
        sysfs::read_string(PathBuf::from(path))
            .and_then(|s| sysfs::parse_int(s.as_bytes()))
            .filter(|&m| m > 0 && m < 200_000)
            .map(|m| m as f64 / 1000.0)
    };
    (read("_max"), read("_crit"))
}

/// Gives devices that share a name a suffix: what tells them apart
/// ("(nvme1)", "(Socket 2)") where each has something different, else a
/// number.
fn tell_apart(found: &mut [(Device, Held, Option<String>)]) {
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, (d, ..)) in found.iter().enumerate() {
        groups.entry(d.name.clone()).or_default().push(i);
    }
    for members in groups.into_values().filter(|m| m.len() > 1) {
        let tells: Vec<Option<&String>> = members.iter().map(|&i| found[i].2.as_ref()).collect();
        let distinct = tells.iter().all(Option::is_some)
            && tells
                .iter()
                .enumerate()
                .all(|(i, t)| !tells[..i].contains(t));
        let suffixes: Vec<String> = if distinct {
            tells.iter().map(|t| format!(" ({})", t.unwrap())).collect()
        } else {
            (1..=members.len()).map(|n| format!(" {n}")).collect()
        };
        for (&i, suffix) in members.iter().zip(suffixes) {
            found[i].0.name.push_str(&suffix);
        }
    }
}

/// An energy counter (µJ) read as power: the energy used since the last
/// reading. Shared with the GPU page's hwmon reader.
#[derive(Debug)]
pub(crate) struct Energy {
    file: HeldFile,
    last: Option<(u64, Instant)>,
}

impl Energy {
    /// Holds the counter, taking the baseline now if `baseline`: not for a
    /// device asleep, which reading would wake.
    pub(crate) fn new(file: HeldFile, baseline: bool) -> Self {
        let mut e = Self { file, last: None };
        if baseline {
            e.watts();
        }
        e
    }

    /// Watts since the last reading; `None` for the first, and for a
    /// counter that went back (a reset).
    pub(crate) fn watts(&mut self) -> Option<f64> {
        let uj = self.file.uint()?;
        let now = Instant::now();
        let (before, then) = self.last.replace((uj, now))?;
        let seconds = now.duration_since(then).as_secs_f64();
        (uj >= before && seconds > 0.0).then(|| (uj - before) as f64 / 1e6 / seconds)
    }

    /// Forgets the baseline: the device slept, and power over the whole nap
    /// would read as a low figure for a device just woken.
    pub(crate) fn rest(&mut self) {
        self.last = None;
    }
}

#[cfg(test)]
mod tests;
