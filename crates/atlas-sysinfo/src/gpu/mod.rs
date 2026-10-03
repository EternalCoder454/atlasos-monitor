//! Graphics cards: what they are ([`cards`]) and how hard they are working
//! ([`GpuSampler`]), with no vendor SDK at build time.
//!
//! Each card is read the best way its driver allows:
//!
//! - **amdgpu**, straight from sysfs: `gpu_busy_percent`, VRAM and GTT in
//!   use, and the card's hwmon for temperatures, fan, power and clocks. A
//!   handful of `pread`s a tick.
//! - **NVIDIA's driver** through NVML, loaded at run time ([`nvml`]): only
//!   when someone has layered the driver, since AtlasOS ships none.
//! - **Intel** (i915, xe): the load from the time the GPU spent out of its
//!   idle state (RC6), the hwmon node of a discrete card, and the driver's
//!   clock and VRAM files.
//! - **Everything else** (nouveau, virtio...): the load from the clients'
//!   drm-usage-stats counters ([`drm`], [`fdinfo`]), and hwmon if the card
//!   has it.
//!
//! The clients' counters are the last resort for the card's load because
//! they can't see the compositor: `kwin_wayland` has a file capability
//! (`cap_sys_nice`), which makes it non-dumpable, so its descriptors are
//! closed to the user's other processes. On a desktop it does most of the
//! drawing. A busy file or idle residency counts everything.
//!
//! A laptop's discrete GPU sleeps (runtime power management) whenever
//! nothing uses it, and reading its sysfs or asking NVML wakes it up on
//! many kernels and drivers, which costs battery. So [`cards`] reads only
//! what the kernel answers without the card, and the sampler checks
//! `power/runtime_status` first: while the card is suspended it reads
//! nothing else, and the reading says so ([`Gpu::asleep`]). A card asleep
//! when the sampler is made has its files opened, NVML started and its
//! baselines taken on the first tick it is awake. Reading an awake card
//! once a second can still restart its autosuspend timer, so a card that
//! would have dozed off while the GPU page is open may stay up.
//!
//! Per-process GPU use is the Apps table's ([`crate::process`]), from the
//! same fdinfo counters.

pub mod fdinfo;
pub mod names;

mod drm;
mod hwmon;
mod nvml;

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Instant;

use crate::sysfs::{self, HeldFile};
pub use fdinfo::PciSlot;

const DRM_DIR: &str = "/sys/class/drm";

/// Who made the card, from its PCI vendor ID.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Vendor {
    Amd,
    Nvidia,
    Intel,
    Other(u16),
}

impl Vendor {
    fn from_id(id: u16) -> Self {
        match id {
            0x1002 => Self::Amd,
            0x10de => Self::Nvidia,
            0x8086 => Self::Intel,
            other => Self::Other(other),
        }
    }

    /// The brand put before a model name.
    pub fn brand(self) -> &'static str {
        match self {
            Self::Amd => "AMD",
            Self::Nvidia => "NVIDIA",
            Self::Intel => "Intel",
            Self::Other(_) => "",
        }
    }

    /// Between two discrete cards (or two integrated ones), the order they
    /// are listed in.
    fn rank(self) -> u8 {
        match self {
            Self::Nvidia => 0,
            Self::Amd => 1,
            Self::Intel => 2,
            Self::Other(_) => 3,
        }
    }
}

/// A graphics card. Read once, by [`cards`].
#[derive(Debug, Clone, PartialEq)]
pub struct Card {
    /// The DRM node: `card1`.
    pub node: String,
    /// "AMD Radeon RX 7900 XTX": see [`names`].
    pub name: String,
    pub vendor: Vendor,
    /// The kernel driver bound to it: `amdgpu`, `i915`, `xe`, `nvidia`...
    pub driver: String,
    /// The PCI address, `0000:03:00.0`; empty for a card that isn't on PCI.
    pub slot: String,
    /// Dedicated video memory in bytes, where the driver says (amdgpu, xe).
    /// An integrated GPU's is the carve-out the firmware set aside.
    pub memory_total: Option<u64>,
    /// System memory the GPU can map (GTT), in bytes (amdgpu).
    pub gtt_total: Option<u64>,
    /// Part of the processor, sharing system memory: an Intel GPU on the
    /// processor's root bus (`0000:00`, where Intel puts it), or an AMD one whose video
    /// memory has no maker (a firmware carve-out, not chips on a board).
    pub integrated: bool,
    /// The sysfs device directory.
    device: PathBuf,
}

impl Card {
    fn pci_slot(&self) -> Option<PciSlot> {
        PciSlot::parse(self.slot.as_bytes())
    }

    /// Whether reading the card every second costs it no sleep: runtime
    /// power management is turned off for it, or it is awake and showing a
    /// screen, which keeps it awake anyway. A laptop's discrete GPU with no
    /// screen on it is neither, and reading it while it's awake restarts
    /// its autosuspend timer (amdgpu counts a read as use), so it would
    /// never sleep. A card that can't be told apart is left alone.
    ///
    /// Reads only what the kernel keeps without the card: a connector's
    /// `enabled` (a screen is driven from it), never its `status`, whose
    /// read can probe the port and wake the card.
    pub fn stays_awake(&self) -> bool {
        let power = |name: &str| sysfs::read_string(self.device.join("power").join(name));
        match power("control").as_deref() {
            Some("on") => return true,
            Some("auto") => {}
            _ => return false,
        }
        // "unsupported": no runtime PM in the driver (a VM's card), so it
        // never sleeps either.
        if !matches!(
            power("runtime_status").as_deref(),
            Some("active" | "unsupported")
        ) {
            return false;
        }
        // Connectors are `card1-DP-1` beside `device`.
        let Some(dir) = self.device.parent() else {
            return false;
        };
        let prefix = format!("{}-", self.node);
        fs::read_dir(dir)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| {
                e.file_name()
                    .to_str()
                    .is_some_and(|n| n.starts_with(&prefix))
            })
            .any(|e| sysfs::read_string(e.path().join("enabled")).as_deref() == Some("enabled"))
    }
}

/// Every graphics card, the one most worth showing first: a discrete GPU
/// before an integrated one, then NVIDIA, AMD, Intel, then more video
/// memory. Empty when there is none (CI's container, a server).
///
/// Reads nothing that wakes a runtime-suspended card: identity files and
/// the sizes amdgpu keeps in memory.
pub fn cards() -> Vec<Card> {
    cards_in(Path::new(DRM_DIR))
}

pub(crate) fn cards_in(drm: &Path) -> Vec<Card> {
    let Ok(dir) = fs::read_dir(drm) else {
        return Vec::new();
    };
    let mut cards: Vec<(Card, u16)> = dir
        .filter_map(|e| {
            let node = e.ok()?.file_name().into_string().ok()?;
            // card1, not card1-DP-1 (a connector) or renderD128.
            let n = node.strip_prefix("card")?;
            if n.is_empty() || !n.bytes().all(|b| b.is_ascii_digit()) {
                return None;
            }
            let number = n.parse().ok()?;
            Some((card(&drm.join(&node), node)?, number))
        })
        .collect();
    let key = |(c, n): &(Card, u16)| {
        (
            c.integrated,
            c.vendor.rank(),
            std::cmp::Reverse(c.memory_total),
            *n,
        )
    };
    cards.sort_by_key(key);
    cards.into_iter().map(|(c, _)| c).collect()
}

/// Reads one `cardN`. `None` without a device that has a vendor ID: a
/// display-only device such as simpledrm's.
fn card(path: &Path, node: String) -> Option<Card> {
    let device = path.join("device");
    let id = |name: &str| {
        let s = sysfs::read_string(device.join(name))?;
        u16::from_str_radix(s.strip_prefix("0x").unwrap_or(&s), 16).ok()
    };
    let vendor_id = id("vendor")?;
    let vendor = Vendor::from_id(vendor_id);
    let link_name = |p: PathBuf| Some(p.file_name()?.to_string_lossy().into_owned());
    let driver = fs::read_link(device.join("driver"))
        .ok()
        .and_then(link_name)
        .unwrap_or_default();
    let slot = fs::canonicalize(&device)
        .ok()
        .and_then(link_name)
        .filter(|s| PciSlot::parse(s.as_bytes()).is_some())
        .unwrap_or_default();

    let memory_total = sysfs::read_uint(device.join("mem_info_vram_total"))
        .or_else(|| sysfs::read_uint(device.join("tile0/physical_vram_size_bytes")))
        .filter(|&b| b > 0);
    let integrated = match vendor {
        Vendor::Intel => PciSlot::parse(slot.as_bytes()).is_some_and(PciSlot::on_root_bus),
        Vendor::Amd => memory_total.is_some() && !device.join("mem_info_vram_vendor").exists(),
        _ => false,
    };
    let name = sysfs::read_string(device.join("product_name"))
        .filter(|n| !n.is_empty())
        .or_else(|| {
            let sub = id("subsystem_vendor").zip(id("subsystem_device"));
            let found = names::lookup(vendor_id, id("device")?, sub)?;
            Some(names::friendly(vendor.brand(), &found))
        })
        // AMD lists its integrated GPUs by codename alone ("Raphael") and
        // sells them as Radeon Graphics.
        .filter(|n| !(integrated && vendor == Vendor::Amd && !n.contains("Radeon")))
        .unwrap_or_else(|| match vendor {
            Vendor::Other(_) => format!("Graphics Card ({node})"),
            Vendor::Amd if integrated => "AMD Radeon Graphics".to_owned(),
            v => format!("{} Graphics", v.brand()),
        });
    Some(Card {
        node,
        name,
        vendor,
        driver,
        slot,
        memory_total,
        gtt_total: sysfs::read_uint(device.join("mem_info_gtt_total")).filter(|&b| b > 0),
        integrated,
        device,
    })
}

/// One reading of a card. A figure its driver doesn't give is `None`, and
/// the page shows a dash.
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct Gpu {
    /// Percent busy, 0..=100: the driver's own figure, or the busiest
    /// engine's.
    pub usage: Option<f64>,
    /// Video memory in use and in all, in bytes.
    pub memory_used: Option<u64>,
    pub memory_total: Option<u64>,
    /// System memory the GPU has mapped (GTT), in bytes.
    pub gtt_used: Option<u64>,
    /// °C: the card's main sensor (`edge` on AMD), the hottest point on the
    /// die (`junction`), and the video memory.
    pub temperature: Option<f64>,
    pub hotspot: Option<f64>,
    pub memory_temperature: Option<f64>,
    pub fan_rpm: Option<u64>,
    /// The fan's duty cycle, for cards that can't count its turns.
    pub fan_percent: Option<f64>,
    /// Watts.
    pub power: Option<f64>,
    /// MHz.
    pub core_clock: Option<f64>,
    pub memory_clock: Option<f64>,
    /// The card is runtime-suspended and was left asleep: usage reads 0 and
    /// nothing else was read.
    pub asleep: bool,
}

/// How the sampler finds the card's load.
#[derive(Debug)]
enum Load {
    /// amdgpu's `gpu_busy_percent`, or another driver's file like it.
    Busy(HeldFile),
    /// Intel's idle residency (RC6), in ms, and the last reading.
    Idle(HeldFile, Option<(u64, Instant)>),
    /// Boxed: the client tables are larger than the other ways.
    Engines(Box<drm::EngineLoad>),
    Nvml(nvml::Nvml),
    None,
}

impl Load {
    /// The load, 0..=100, from a source other than NVML.
    fn sample(&mut self) -> Option<f64> {
        match self {
            Load::Busy(f) => f.uint().map(|p| (p as f64).min(100.0)),
            Load::Idle(f, last) => {
                let ms = f.uint()?;
                let now = Instant::now();
                let (before, then) = last.replace((ms, now))?;
                busy_from_idle(
                    ms.checked_sub(before)?,
                    now.duration_since(then).as_secs_f64(),
                )
            }
            Load::Engines(e) => e.sample(),
            Load::Nvml(_) | Load::None => None,
        }
    }
}

/// Samples one card. Owned by the sampling thread, made while the GPU page
/// (or the Overview's GPU row) is on screen.
#[derive(Debug)]
pub struct GpuSampler {
    card: Card,
    /// Whether it is the only card, so every DRM client is its client.
    only: bool,
    /// Count the clients even where the driver has a figure of its own.
    count_clients: bool,
    /// The load alone, from the driver's own figure: no other files, and
    /// no counting clients, which reads every process.
    load_only: bool,
    /// Whether the files below are open. Not while the card has slept since
    /// the sampler was made: opening NVML or reading a baseline wakes it.
    ready: bool,
    runtime_status: Option<HeldFile>,
    load: Load,
    hwmon: hwmon::Hwmon,
    memory_used: Option<HeldFile>,
    memory_total: Option<u64>,
    gtt_used: Option<HeldFile>,
    /// The driver's clock file, in MHz, for cards without an hwmon clock.
    clock: Option<HeldFile>,
    power_limit: Option<f64>,
}

impl GpuSampler {
    /// Opens what `card` reports and takes the baseline readings, unless
    /// the card is asleep: then that waits for the first tick it is awake.
    /// `cards` is the whole list, to know whether the card is the only one.
    pub fn new(card: &Card, cards: &[Card]) -> Self {
        Self::with(card, cards, false, false)
    }

    /// Like [`Self::new`], but reads only the load, and only where the
    /// driver keeps a figure (a busy file, idle residency or NVML): the
    /// rest of a reading is `None`. For the sidebar, which also asks it for
    /// [`Self::temperature`].
    pub fn load_only(card: &Card, cards: &[Card]) -> Self {
        Self::with(card, cards, false, true)
    }

    /// Like [`Self::new`], but the load always comes from the clients'
    /// counters, even on a card with a figure of its own. For checking
    /// that path against the driver's (`--example gpu -- --clients`).
    #[doc(hidden)]
    pub fn counting_clients(card: &Card, cards: &[Card]) -> Self {
        Self::with(card, cards, true, false)
    }

    fn with(card: &Card, cards: &[Card], count_clients: bool, load_only: bool) -> Self {
        let mut s = Self {
            card: card.clone(),
            only: cards.len() <= 1,
            count_clients,
            load_only,
            ready: false,
            runtime_status: HeldFile::open(card.device.join("power/runtime_status")),
            load: Load::None,
            hwmon: hwmon::Hwmon::default(),
            memory_used: None,
            memory_total: card.memory_total,
            gtt_used: None,
            clock: None,
            power_limit: None,
        };
        if !s.asleep() {
            s.open();
        }
        s
    }

    /// Opens the card's files and takes the baselines. Only while it is
    /// awake.
    fn open(&mut self) {
        self.ready = true;
        let card = &self.card;
        let dev = |name: &str| card.device.join(name);
        if card.driver == "nvidia"
            && let Some(n) = nvml::Nvml::open(&card.slot)
        {
            self.power_limit = n.power_limit();
            self.load = Load::Nvml(n);
            return;
        }
        let node = card.device.parent().unwrap_or(&card.device);
        if self.load_only {
            self.load = Self::own_figure(card, node).unwrap_or(Load::None);
            // For `temperature`; `sample` leaves it alone.
            self.hwmon = hwmon::Hwmon::open_temperature(&card.device);
            return;
        }
        self.memory_used = HeldFile::open(dev("mem_info_vram_used"));
        self.gtt_used = HeldFile::open(dev("mem_info_gtt_used"));
        self.power_limit = hwmon::power_limit(&card.device);
        self.hwmon = hwmon::Hwmon::open(&card.device);
        self.clock = HeldFile::open_first([
            node.join("gt_act_freq_mhz"),    // i915
            node.join("gt_cur_freq_mhz"),    // i915, older
            dev("tile0/gt0/freq0/act_freq"), // xe
            dev("tile0/gt0/freq0/cur_freq"), // xe, older
        ]);
        self.load = if self.count_clients {
            Self::engines(card, self.only)
        } else {
            Self::own_figure(card, node).unwrap_or_else(|| Self::engines(card, self.only))
        };
    }

    /// The driver's own load figure: a busy file, or idle residency (with
    /// its baseline taken).
    fn own_figure(card: &Card, node: &Path) -> Option<Load> {
        if let Some(f) = HeldFile::open(card.device.join("gpu_busy_percent")) {
            return Some(Load::Busy(f));
        }
        let f = HeldFile::open_first([
            node.join("gt/gt0/rc6_residency_ms"),                   // i915
            node.join("power/rc6_residency_ms"),                    // i915, older
            card.device.join("tile0/gt0/gtidle/idle_residency_ms"), // xe
        ])?;
        let mut load = Load::Idle(f, None);
        load.sample(); // the baseline
        Some(load)
    }

    fn engines(card: &Card, only: bool) -> Load {
        drm::EngineLoad::new(card.pci_slot(), only)
            .map_or(Load::None, |e| Load::Engines(Box::new(e)))
    }

    /// The power limit in force, in watts: NVML's, else hwmon's
    /// (`power1_cap`, `power1_max`). `None` until the card has been awake.
    pub fn power_limit(&self) -> Option<f64> {
        self.power_limit
    }

    /// Reads the card.
    pub fn sample(&mut self) -> Gpu {
        let mut g = Gpu {
            memory_total: self.memory_total,
            ..Gpu::default()
        };
        if self.asleep() {
            g.usage = Some(0.0);
            g.asleep = true;
            self.hwmon.rest();
            if let Load::Idle(_, last) = &mut self.load {
                *last = None;
            }
            return g;
        }
        if !self.ready {
            // Awake for the first time: this tick only takes the baselines
            // (idle residency, energy), and the figures show from the next.
            self.open();
            return g;
        }
        if let Load::Nvml(n) = &mut self.load {
            if self.load_only {
                g.usage = n.usage();
            } else {
                n.read(&mut g);
            }
            return g;
        }
        g.usage = self.load.sample();
        if self.load_only {
            return g;
        }
        let uint = |f: &mut Option<HeldFile>| f.as_mut().and_then(HeldFile::uint);
        g.memory_used = uint(&mut self.memory_used);
        g.gtt_used = uint(&mut self.gtt_used);
        self.hwmon.read(&mut g);
        if g.core_clock.is_none() {
            g.core_clock = uint(&mut self.clock).map(|mhz| mhz as f64);
        }
        g
    }

    /// The card's temperature alone, °C: for the health check on pages
    /// that don't show the card, read on a slower timer than the load.
    /// `None` while it sleeps or before it has been read awake.
    pub fn temperature(&mut self) -> Option<f64> {
        if !self.ready || self.asleep() {
            return None;
        }
        match &mut self.load {
            Load::Nvml(n) => n.temperature(),
            _ => self.hwmon.temperature(),
        }
    }

    /// Whether the kernel has the card suspended. Read every tick: one
    /// `pread`, and the reason no other file is touched while it sleeps.
    fn asleep(&mut self) -> bool {
        self.runtime_status
            .as_mut()
            .and_then(HeldFile::bytes)
            .is_some_and(|b| b.trim_ascii() == b"suspended")
    }
}

/// Percent busy from `idle_ms` of idle residency in `seconds`. `None` for
/// no time at all.
fn busy_from_idle(idle_ms: u64, seconds: f64) -> Option<f64> {
    (seconds > 0.0).then(|| ((1.0 - idle_ms as f64 / (seconds * 1000.0)) * 100.0).clamp(0.0, 100.0))
}

#[cfg(test)]
mod tests;
