//! What a hwmon device and each of its readings are, for people.
//!
//! The kernel names a device by its driver (`spd5118`, `r8169_0_600:00`,
//! `nvme`) and a reading by the chip's own terse label (`junction`, `Tctl`,
//! `SYSTIN`) or by nothing at all. Pure functions over what discovery
//! read, so the tests cover them without a sysfs tree.

use std::collections::HashMap;

use super::{Category, Kind};

/// What discovery read about one hwmon device, the input to [`identify`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Facts {
    /// The hwmon `name`: the driver, sometimes with an instance suffix.
    pub driver: String,
    /// The last component of the device's sysfs path: `0000:03:00.0` for
    /// a PCI card, `10-0051` for a chip on an I2C bus, `nvme0`,
    /// `coretemp.0`. Empty without a device link.
    pub address: String,
    /// The nearest PCI device on the path, `0000:03:00.0`.
    pub pci: Option<String>,
    /// A drive's model, from the device's own `model` (and `vendor`) file.
    pub model: Option<String>,
    /// A drive's kernel name (`nvme0`, `sda`), to tell two drives of the
    /// same model apart.
    pub block: Option<String>,
}

/// What the machine's other parts are called, looked up once per discovery
/// and only when a device needs it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Context {
    /// The processor's model, already through [`processor_name`].
    pub processor: Option<String>,
    /// Graphics cards' names by PCI address, as the GPU page names them.
    pub cards: Vec<(String, String)>,
}

/// A device's name and category, and what tells it apart from another of
/// the same name.
pub(super) fn identify(f: &Facts, ctx: &Context) -> (String, Category, Option<String>) {
    let d = f.driver.to_ascii_lowercase();
    let exact = |names: &[&str]| names.contains(&d.as_str());
    let prefix = |names: &[&str]| names.iter().any(|n| d.starts_with(n));
    let named = |s: &str| s.to_owned();

    if prefix(&["coretemp", "k10temp", "zenpower", "k8temp", "via_cputemp"])
        || exact(&["fam15h_power", "cpu_thermal"])
    {
        // One coretemp device per socket, `coretemp.<package>`.
        let socket = f
            .address
            .strip_prefix("coretemp.")
            .and_then(|n| n.parse::<u32>().ok())
            .map(|n| format!("Socket {}", n + 1));
        let name = ctx.processor.clone().unwrap_or_else(|| named("Processor"));
        return (name, Category::Processor, socket);
    }
    if exact(&["amdgpu", "radeon", "nouveau", "i915", "xe", "nvidia"]) {
        let name = f
            .pci
            .as_ref()
            .and_then(|slot| ctx.cards.iter().find(|(s, _)| s == slot))
            .map_or_else(|| named("Graphics Card"), |(_, n)| n.clone());
        return (name, Category::Graphics, f.pci.clone());
    }
    if exact(&["spd5118", "jc42", "ee1004"]) {
        return memory_slot(&f.address);
    }
    if exact(&["nvme", "drivetemp"]) {
        let name = f.model.clone().unwrap_or_else(|| named("Drive"));
        return (name, Category::Storage, f.block.clone());
    }
    if prefix(&[
        "iwlwifi", "ath9k", "ath10k", "ath11k", "ath12k", "mt76", "mt79", "mt7", "rtw", "brcmfmac",
        "mwifiex",
    ]) {
        return (named("Wi-Fi Adapter"), Category::Network, None);
    }
    if prefix(&[
        "r8169", "r8125", "r8152", "igb", "igc", "e1000e", "atlantic", "tg3", "bnxt", "ixgbe",
        "i40e", "mlx", "aqc",
    ]) || exact(&["ice"])
    {
        return (named("Ethernet Adapter"), Category::Network, None);
    }
    if exact(&["acpitz"]) {
        return (named("Thermal Zone"), Category::Motherboard, None);
    }
    if prefix(&["pch_"]) {
        return (named("Chipset"), Category::Motherboard, None);
    }
    if prefix(&[
        "thinkpad",
        "dell_smm",
        "applesmc",
        "hp_wmi",
        "toshiba",
        "surface_fan",
        "steamdeck",
    ]) || exact(&["asus", "hp"])
    {
        // A laptop's embedded controller: its fans and a few temperatures.
        return (named("Embedded Controller"), Category::Motherboard, None);
    }
    if prefix(&[
        "nct", "it87", "it86", "w83", "f71", "asus_wmi", "asusec", "gigabyte", "lm7",
    ]) {
        return (named("Motherboard"), Category::Motherboard, None);
    }
    if is_battery(&d) {
        return (named("Battery"), Category::PowerSupply, None);
    }
    if exact(&["ac", "acad"]) || prefix(&["adp"]) {
        return (named("Power Adapter"), Category::PowerSupply, None);
    }
    if prefix(&["ucsi_source_psy"]) {
        return (named("USB-C Port"), Category::PowerSupply, None);
    }
    if prefix(&["corsairpsu"]) {
        return (named("Power Supply"), Category::PowerSupply, None);
    }
    if prefix(&["kraken", "nzxt_kraken"]) {
        return (named("Liquid Cooler"), Category::Other, None);
    }
    if prefix(&["corsaircpro", "nzxtsmart2", "nzxt_smart2"]) {
        return (named("Fan Controller"), Category::Other, None);
    }
    (f.driver.clone(), Category::Other, None)
}

/// `BAT0`, `BAT1`, `battery`: not `bat_whatever`.
fn is_battery(d: &str) -> bool {
    d == "battery"
        || d.strip_prefix("bat")
            .is_some_and(|n| n.bytes().all(|b| b.is_ascii_digit()))
}

/// A memory module's thermal sensor sits on the module's SPD bus: at 0x50
/// to 0x57 (spd5118, ee1004) or 0x18 to 0x1f (jc42), one address per
/// slot. `address` is `<bus>-<address>`, `10-0051`.
fn memory_slot(address: &str) -> (String, Category, Option<String>) {
    let parsed = address.split_once('-').and_then(|(bus, a)| {
        let n = u16::from_str_radix(a, 16).ok()?;
        (n >= 0x18).then(|| (bus, (n & 7) + 1))
    });
    match parsed {
        Some((bus, slot)) => (
            format!("Memory Slot {slot}"),
            Category::Memory,
            Some(format!("i2c-{bus}")),
        ),
        None => ("Memory Module".to_owned(), Category::Memory, None),
    }
}

/// How a device's readings are labelled beyond the label table.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(super) struct Labelling {
    /// The device has a `Tdie` reading, so its `Tctl` is the fan-control
    /// figure (offset on early Ryzens), not the package temperature.
    pub has_die: bool,
    /// A hybrid processor's per-core names by core ID: "P-core 3".
    pub cores: HashMap<u32, String>,
}

/// What one reading measures. `given` is the driver's label, empty
/// without one; `numbered` says that the device has more than one
/// unlabelled reading of this kind, which then carry their index.
pub(super) fn label(given: &str, kind: Kind, index: u32, numbered: bool, l: &Labelling) -> String {
    let lower = given.to_ascii_lowercase();
    let fixed = match lower.as_str() {
        "edge" => Some("Edge"),
        "junction" | "hotspot" => Some("Hotspot"),
        "mem" | "vram" => Some("Memory"),
        "pkg" | "tdie" => Some("Package"),
        "tctl" if l.has_die => Some("Control"),
        "tctl" => Some("Package"),
        "ppt" => Some("Power draw"),
        "vddgfx" => Some("Core voltage"),
        "vddnb" => Some("SoC voltage"),
        "composite" => Some("Drive"),
        "systin" => Some("System"),
        "cputin" => Some("CPU socket"),
        "pch_chip_temp" => Some("Chipset"),
        _ => None,
    };
    if let Some(s) = fixed {
        return s.to_owned();
    }
    if let Some(n) = number_after(&lower, "tccd") {
        return format!("Chiplet {n}");
    }
    if let Some(n) = number_after(&lower, "auxtin") {
        return format!("Auxiliary {n}");
    }
    if lower.starts_with("package id ") || lower.starts_with("physical id ") {
        return "Package".to_owned();
    }
    if let Some(n) = core_id(given)
        && let Some(name) = l.cores.get(&n)
    {
        return name.clone();
    }
    if given.is_empty() {
        let noun = kind.noun();
        return if numbered {
            format!("{noun} {index}")
        } else {
            noun.to_owned()
        };
    }
    // A driver's lower-case word ("card", "vddcr_soc") reads as a label
    // once capitalised; anything with capitals is the driver's own spelling.
    if given.bytes().all(|b| !b.is_ascii_uppercase()) {
        let mut c = given.chars();
        return c
            .next()
            .map(|f| f.to_uppercase().chain(c).collect())
            .unwrap_or_default();
    }
    given.to_owned()
}

/// The core ID in coretemp's `Core 12`.
pub(super) fn core_id(given: &str) -> Option<u32> {
    given.strip_prefix("Core ")?.parse().ok()
}

/// The number that ends `s` after `prefix`: `tccd3` → 3.
fn number_after(s: &str, prefix: &str) -> Option<u32> {
    let rest = s.strip_prefix(prefix)?;
    if rest.is_empty() || !rest.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    rest.parse().ok()
}

/// Names a hybrid processor's cores by type: "P-core 1".."P-core 8" then
/// "E-core 1".."E-core 16", each in core ID order. `cores` is every core
/// ID in one package with whether it is an efficiency core. Empty when no
/// core is one: coretemp's own "Core N" stays on other processors, where it
/// matches what `sensors` and the BIOS say.
pub(super) fn core_names(cores: &[(u32, bool)]) -> HashMap<u32, String> {
    if !cores.iter().any(|&(_, e)| e) {
        return HashMap::new();
    }
    let mut sorted = cores.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let (mut p, mut e) = (0, 0);
    sorted
        .into_iter()
        .map(|(id, efficient)| {
            let name = if efficient {
                e += 1;
                format!("E-core {e}")
            } else {
                p += 1;
                format!("P-core {p}")
            };
            (id, name)
        })
        .collect()
}

/// The processor's model without trademarks, clock and core count:
/// "Intel(R) Core(TM) i9-14900KF CPU @ 3.20GHz" → "Intel Core i9-14900KF",
/// "AMD Ryzen 9 7950X 16-Core Processor" → "AMD Ryzen 9 7950X".
pub(super) fn processor_name(model: &str) -> String {
    let mut s = model.to_owned();
    for mark in ["(R)", "(r)", "(TM)", "(tm)"] {
        s = s.replace(mark, "");
    }
    for cut in [" @ ", " w/ ", " with "] {
        if let Some(i) = s.find(cut) {
            s.truncate(i);
        }
    }
    let mut words: Vec<&str> = s.split_whitespace().collect();
    if words.last() == Some(&"Processor") {
        words.pop();
        if words.last().is_some_and(|w| w.ends_with("-Core")) {
            words.pop();
        }
    }
    if words.last() == Some(&"CPU") {
        words.pop();
    }
    words.join(" ")
}
