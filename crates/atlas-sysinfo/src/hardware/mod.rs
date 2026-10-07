//! The devices on the PCI and USB buses, and the input devices: the lists
//! KDE's Info Center shows, for Telamon Monitor's Devices page.
//!
//! [`read`] lists `/sys/bus/pci/devices` and `/sys/bus/usb/devices`, then
//! names what it found from `pci.ids` and `usb.ids` in one pass over each
//! file. Root hubs (`usbN`), the controllers' own ports, are left out. The
//! input devices come from `/sys/class/input`, their kinds from udev's
//! database where it is readable. It makes about two hundred small
//! reads and is meant to run once, on a worker thread, when the page opens.
//!
//! Everything a USB device reports about itself (its manufacturer, its
//! product name) is device-controlled text: it is stripped of control
//! characters and cut to 128 characters before it goes anywhere.

mod ids;
mod input;

use std::fs::{self, File};
use std::io::Read;
use std::path::Path;

use ids::{Names, Wanted};
pub use input::{InputDevice, InputKind, parse_input_devices};

const PCI_DEVICES: &str = "/sys/bus/pci/devices";
const USB_DEVICES: &str = "/sys/bus/usb/devices";
const INPUT_CLASS: &str = "/sys/class/input";
const UDEV_DATA: &str = "/run/udev/data";

/// The longest name kept, in characters.
const MAX_TEXT: usize = 128;

/// Everything the Devices page lists.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Hardware {
    pub pci: Vec<PciDevice>,
    pub usb: Vec<UsbDevice>,
    pub input: Vec<InputDevice>,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct PciDevice {
    /// "0000:03:00.0"
    pub slot: String,
    /// The class's name from `pci.ids`: "Display controller". "Other" if the
    /// database doesn't list it.
    pub class: String,
    /// The 24-bit class from sysfs.
    pub class_code: u32,
    /// The vendor's name, else "Vendor 1002".
    pub vendor: String,
    /// The device's name, else its subclass's ("VGA compatible controller"),
    /// else "Device 744c".
    pub name: String,
    /// The bound driver, "" if none.
    pub driver: String,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct UsbDevice {
    /// Sysfs name: "1-3", "2-9.1".
    pub path: String,
    /// The device's `manufacturer`, else `usb.ids`' vendor, else "Vendor 362d".
    pub vendor: String,
    /// The device's `product`, else `usb.ids`' device, else "Device 0240".
    pub name: String,
    /// "362d:0240"
    pub id: String,
    /// "USB 2 (480 Mbit/s)", or "" if sysfs doesn't say.
    pub speed: String,
    pub hub: bool,
}

/// Lists the system's devices. Missing buses, files and databases leave
/// things out or fall back to hex names; nothing here is an error.
pub fn read() -> Hardware {
    Hardware {
        pci: read_pci(Path::new(PCI_DEVICES), &crate::gpu::names::PCI_IDS),
        usb: read_usb(Path::new(USB_DEVICES), &ids::USB_IDS),
        input: input::sysfs_devices(Path::new(INPUT_CLASS), |b| {
            input::udev_props(b, Path::new(INPUT_CLASS), Path::new(UDEV_DATA))
        }),
    }
}

/// Text from a device or the kernel on one line: control and invisible
/// characters taken out, any run of spaces or line breaks one space,
/// trimmed, and cut to 128 characters.
pub(crate) fn clean(s: &str) -> String {
    let mut out = String::new();
    let mut count = 0;
    for word in s
        .split(|c: char| c.is_whitespace() || c.is_control())
        .map(|w| w.chars().filter(|&c| !crate::invisible(c)))
    {
        let mut word = word.peekable();
        if word.peek().is_none() {
            continue;
        }
        if count > 0 {
            if count >= MAX_TEXT {
                break;
            }
            out.push(' ');
            count += 1;
        }
        for c in word.take(MAX_TEXT - count) {
            out.push(c);
            count += 1;
        }
    }
    out.trim_end().to_owned()
}

/// A sysfs attribute as clean text: at most 4 KiB read, lossily decoded.
/// `None` if it can't be read or is empty.
fn attr(dir: &Path, name: &str) -> Option<String> {
    let mut buf = Vec::with_capacity(128);
    File::open(dir.join(name))
        .ok()?
        .take(4096)
        .read_to_end(&mut buf)
        .ok()?;
    let s = clean(&String::from_utf8_lossy(&buf));
    (!s.is_empty()).then_some(s)
}

/// A hex attribute: `0x1002`, or `09`.
fn hex_attr(dir: &Path, name: &str) -> Option<u32> {
    let s = attr(dir, name)?;
    u32::from_str_radix(s.strip_prefix("0x").unwrap_or(&s), 16).ok()
}

// PCI

/// What sysfs says about a PCI device, before it is named.
#[derive(Debug, PartialEq, Eq)]
struct RawPci {
    slot: String,
    vendor: u16,
    device: u16,
    class: u32,
    driver: String,
}

/// Whether `s` looks like a PCI address, `0000:03:00.0`: the domain may be
/// longer than four digits.
pub(crate) fn valid_slot(s: &str) -> bool {
    s.len() <= 16
        && s.split_once('.')
            .is_some_and(|(_, func)| func.len() == 1 && func.bytes().all(|c| c.is_ascii_digit()))
        && s.split(':').count() == 3
        && s.bytes()
            .all(|c| c.is_ascii_hexdigit() || c == b':' || c == b'.')
}

fn scan_pci(dir: &Path) -> Vec<RawPci> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let slot = e.file_name().into_string().ok()?;
            if !valid_slot(&slot) {
                return None;
            }
            let path = e.path();
            let driver = fs::read_link(path.join("driver"))
                .ok()
                .and_then(|l| l.file_name().map(|n| clean(&n.to_string_lossy())))
                .unwrap_or_default();
            Some(RawPci {
                vendor: u16::try_from(hex_attr(&path, "vendor")?).ok()?,
                device: u16::try_from(hex_attr(&path, "device")?).ok()?,
                class: hex_attr(&path, "class")? & 0xff_ffff,
                slot,
                driver,
            })
        })
        .collect()
}

fn wanted_pci(raw: &[RawPci]) -> Wanted {
    let mut w = Wanted::default();
    for r in raw {
        let (base, sub) = ((r.class >> 16) as u8, (r.class >> 8) as u8);
        w.vendors.insert(r.vendor);
        w.devices.insert((r.vendor, r.device));
        w.classes.insert(base);
        w.subclasses.insert((base, sub));
    }
    w
}

fn name_pci(raw: Vec<RawPci>, names: &Names) -> Vec<PciDevice> {
    let mut out: Vec<PciDevice> = raw
        .into_iter()
        .map(|r| {
            let (base, sub) = ((r.class >> 16) as u8, (r.class >> 8) as u8);
            let subclass = names.subclasses.get(&(base, sub));
            PciDevice {
                class: names
                    .classes
                    .get(&base)
                    .cloned()
                    .unwrap_or_else(|| "Other".into()),
                vendor: names
                    .vendors
                    .get(&r.vendor)
                    .cloned()
                    .unwrap_or_else(|| format!("Vendor {:04x}", r.vendor)),
                name: names
                    .devices
                    .get(&(r.vendor, r.device))
                    .or(subclass)
                    .cloned()
                    .unwrap_or_else(|| format!("Device {:04x}", r.device)),
                class_code: r.class,
                slot: r.slot,
                driver: r.driver,
            }
        })
        .collect();
    // By class, with the classes pci.ids doesn't name ("Other") last, so
    // each name is one run for the page to group.
    let unnamed = |d: &PciDevice| !names.classes.contains_key(&((d.class_code >> 16) as u8));
    out.sort_by(|a, b| {
        (unnamed(a), a.class_code, &a.slot).cmp(&(unnamed(b), b.class_code, &b.slot))
    });
    out
}

fn read_pci(dir: &Path, db: &[&str]) -> Vec<PciDevice> {
    let raw = scan_pci(dir);
    let names = ids::resolve_files(db, &wanted_pci(&raw));
    name_pci(raw, &names)
}

// USB

#[derive(Debug, PartialEq, Eq)]
struct RawUsb {
    path: String,
    vendor_id: u16,
    product_id: u16,
    manufacturer: Option<String>,
    product: Option<String>,
    speed: String,
    hub: bool,
}

/// A device's position as numbers, so "1-2" sorts before "1-10". `None` for
/// a name that isn't a port path: root hubs (`usb1`) and interfaces (`1-3:1.0`).
fn usb_path_key(name: &str) -> Option<Vec<u32>> {
    if name.is_empty() || name.len() > 32 {
        return None;
    }
    name.split(['-', '.'])
        .map(|p| {
            (!p.is_empty() && p.bytes().all(|c| c.is_ascii_digit()))
                .then(|| p.parse().ok())
                .flatten()
        })
        .collect()
}

fn scan_usb(dir: &Path) -> Vec<(Vec<u32>, RawUsb)> {
    let Ok(entries) = fs::read_dir(dir) else {
        return Vec::new();
    };
    entries
        .flatten()
        .filter_map(|e| {
            let path = e.file_name().into_string().ok()?;
            let key = usb_path_key(&path)?;
            let d = e.path();
            // A device unplugged mid-scan has no ID: leave it out.
            let vendor_id = u16::try_from(hex_attr(&d, "idVendor")?).ok()?;
            let product_id = u16::try_from(hex_attr(&d, "idProduct")?).ok()?;
            Some((
                key,
                RawUsb {
                    path,
                    vendor_id,
                    product_id,
                    manufacturer: attr(&d, "manufacturer"),
                    product: attr(&d, "product"),
                    speed: attr(&d, "speed").map(|s| usb_speed(&s)).unwrap_or_default(),
                    hub: hex_attr(&d, "bDeviceClass") == Some(9),
                },
            ))
        })
        .collect()
}

/// Only what the devices didn't say about themselves is looked up.
fn wanted_usb(raw: &[(Vec<u32>, RawUsb)]) -> Wanted {
    let mut w = Wanted::default();
    for (_, r) in raw {
        if r.manufacturer.is_none() {
            w.vendors.insert(r.vendor_id);
        }
        if r.product.is_none() {
            w.devices.insert((r.vendor_id, r.product_id));
        }
    }
    w
}

fn name_usb(mut raw: Vec<(Vec<u32>, RawUsb)>, names: &Names) -> Vec<UsbDevice> {
    raw.sort_by(|a, b| a.0.cmp(&b.0));
    raw.into_iter()
        .map(|(_, r)| UsbDevice {
            vendor: r
                .manufacturer
                .or_else(|| names.vendors.get(&r.vendor_id).cloned())
                .unwrap_or_else(|| format!("Vendor {:04x}", r.vendor_id)),
            name: r
                .product
                .or_else(|| names.devices.get(&(r.vendor_id, r.product_id)).cloned())
                .unwrap_or_else(|| format!("Device {:04x}", r.product_id)),
            id: format!("{:04x}:{:04x}", r.vendor_id, r.product_id),
            path: r.path,
            speed: r.speed,
            hub: r.hub,
        })
        .collect()
}

fn read_usb(dir: &Path, db: &[&str]) -> Vec<UsbDevice> {
    let raw = scan_usb(dir);
    let names = ids::resolve_files(db, &wanted_usb(&raw));
    name_usb(raw, &names)
}

/// The USB generation and speed from sysfs' `speed`, in Mbit/s: "480" is
/// "USB 2 (480 Mbit/s)". "" for anything that isn't a plain positive number.
pub fn usb_speed(mbps: &str) -> String {
    let Ok(v) = mbps.trim().parse::<f64>() else {
        return String::new();
    };
    if !v.is_finite() || v <= 0.0 || v > 1.0e6 {
        return String::new();
    }
    let generation = match v {
        v if v <= 12.0 => "USB 1",
        v if v <= 480.0 => "USB 2",
        v if v < 40_000.0 => "USB 3",
        _ => "USB4",
    };
    if v >= 1000.0 {
        format!("{generation} ({} Gbit/s)", v / 1000.0)
    } else {
        format!("{generation} ({v} Mbit/s)")
    }
}

#[cfg(test)]
mod tests;
