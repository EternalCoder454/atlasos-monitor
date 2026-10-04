//! Names from `pci.ids` and `usb.ids`, resolved for a whole set of devices in
//! one pass: the files are 1.5 MB, and a lookup per device would read them
//! thirty times. Both use the layout [`crate::gpu::names`] describes:
//!
//! ```text
//! 1002  Advanced Micro Devices, Inc. [AMD/ATI]
//! \t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX/7900M]
//! \t\t1eae 7901  RX-79XMERCB9 [SPEEDSTER MERC 310 RX 7900 XTX]
//! ...
//! C 03  Display controller
//! \t00  VGA compatible controller
//! \t\t00  VGA controller
//! ```
//!
//! `usb.ids` follows the vendors with more lists (`AT`, `HID`, `R`...) that
//! are skipped here. A missing database is an empty answer, never an error.

use std::collections::{BTreeMap, BTreeSet};
use std::fs::File;
use std::io::{BufRead, BufReader};

use super::clean;

/// The IDs to find.
#[derive(Debug, Default)]
pub(crate) struct Wanted {
    pub vendors: BTreeSet<u16>,
    pub devices: BTreeSet<(u16, u16)>,
    pub classes: BTreeSet<u8>,
    pub subclasses: BTreeSet<(u8, u8)>,
}

impl Wanted {
    fn count(&self) -> usize {
        self.vendors.len() + self.devices.len() + self.classes.len() + self.subclasses.len()
    }
}

/// The names the database has for them; an ID it doesn't list is absent.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Names {
    pub vendors: BTreeMap<u16, String>,
    pub devices: BTreeMap<(u16, u16), String>,
    pub classes: BTreeMap<u8, String>,
    pub subclasses: BTreeMap<(u8, u8), String>,
}

impl Names {
    fn count(&self) -> usize {
        self.vendors.len() + self.devices.len() + self.classes.len() + self.subclasses.len()
    }
}

/// Which block of the file the last column-0 line opened.
enum Section {
    /// Not one we read: an unwanted vendor, or `usb.ids`' trailing lists.
    Skip,
    /// A vendor with wanted devices.
    Vendor(u16),
    /// A class with wanted subclasses.
    Class(u8),
}

/// Reads `r` once and returns the names of everything in `want` that it lists.
/// Stops as soon as all of it is found.
pub(crate) fn resolve(mut r: impl BufRead, want: &Wanted) -> Names {
    let mut names = Names::default();
    let total = want.count();
    if total == 0 {
        return names;
    }
    let mut line = Vec::with_capacity(128);
    let mut section = Section::Skip;
    loop {
        line.clear();
        // A read error ends the pass with what was found: names are a nicety.
        if !matches!(r.read_until(b'\n', &mut line), Ok(n) if n > 0) {
            break;
        }
        let text = line.trim_ascii_end();
        match text.first() {
            None | Some(b'#') => {}
            Some(b'\t') => match (&section, text.get(1)) {
                (_, Some(b'\t')) | (Section::Skip, _) => {}
                (Section::Vendor(v), _) => {
                    if let Some((d, name)) = id(&text[1..], 4)
                        && let Ok(d) = u16::try_from(d)
                        && want.devices.contains(&(*v, d))
                    {
                        names.devices.insert((*v, d), clean(&lossy(name)));
                    }
                }
                (Section::Class(c), _) => {
                    if let Some((s, name)) = id(&text[1..], 2)
                        && let Ok(s) = u8::try_from(s)
                        && want.subclasses.contains(&(*c, s))
                    {
                        names.subclasses.insert((*c, s), clean(&lossy(name)));
                    }
                }
            },
            Some(_) => section = open(text, want, &mut names),
        }
        if names.count() == total {
            break;
        }
    }
    names
}

/// A column-0 line: a vendor or a class opens a block.
fn open(text: &[u8], want: &Wanted, names: &mut Names) -> Section {
    if let Some(rest) = text.strip_prefix(b"C ") {
        let Some((c, name)) = id(rest, 2).and_then(|(c, n)| Some((u8::try_from(c).ok()?, n)))
        else {
            return Section::Skip;
        };
        if want.classes.contains(&c) {
            names.classes.insert(c, clean(&lossy(name)));
        }
        return if want
            .subclasses
            .range((c, 0)..=(c, u8::MAX))
            .next()
            .is_some()
        {
            Section::Class(c)
        } else {
            Section::Skip
        };
    }
    let Some((v, name)) = id(text, 4).and_then(|(v, n)| Some((u16::try_from(v).ok()?, n))) else {
        return Section::Skip;
    };
    if want.vendors.contains(&v) {
        names.vendors.insert(v, clean(&lossy(name)));
    }
    if want.devices.range((v, 0)..=(v, u16::MAX)).next().is_some() {
        Section::Vendor(v)
    } else {
        Section::Skip
    }
}

/// `digits` hex digits, whitespace, then the name. `None` for anything else,
/// which is how `AT 0001` and `HID 00` lines fall through.
fn id(text: &[u8], digits: usize) -> Option<(u32, &[u8])> {
    let hex = text.get(..digits)?;
    if !hex.iter().all(u8::is_ascii_hexdigit) || !text.get(digits)?.is_ascii_whitespace() {
        return None;
    }
    let n = u32::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
    Some((n, text[digits..].trim_ascii()))
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// Resolves `want` in the first database of `paths` that opens.
pub(crate) fn resolve_files(paths: &[&str], want: &Wanted) -> Names {
    if want.count() == 0 {
        return Names::default();
    }
    paths
        .iter()
        .find_map(|p| File::open(p).ok())
        .map(|f| resolve(BufReader::with_capacity(64 * 1024, f), want))
        .unwrap_or_default()
}

/// Where Fedora (hwdata) and others keep the USB ID database.
pub(crate) const USB_IDS: [&str; 3] = [
    "/usr/share/hwdata/usb.ids",
    "/usr/share/misc/usb.ids",
    "/usr/share/usb.ids",
];
