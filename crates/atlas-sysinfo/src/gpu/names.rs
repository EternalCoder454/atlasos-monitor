//! A card's name for people: "AMD Radeon RX 7900 XTX", not
//! "Navi 31 [Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M]".
//!
//! The PCI ID database names a chip, and the chip is usually sold as several
//! models, written in brackets after its codename with the models after the
//! first shortened (`Radeon RX 6800/6800 XT / 6900 XT`). The board's
//! subsystem entry, where the database has one, is the vendor's product
//! name and says which of those models it is; it is too irregular to show
//! as it is (`RX-79XMERCB9 [SPEEDSTER MERC 310 RX 7900 XTX]`), so it is only
//! used to pick a model. Without one, every model in the brackets is shown.

use std::fs::File;
use std::io::{BufRead, BufReader};

/// Where Fedora (hwdata) and others keep the PCI ID database.
pub(crate) const PCI_IDS: [&str; 3] = [
    "/usr/share/hwdata/pci.ids",
    "/usr/share/misc/pci.ids",
    "/usr/share/pci.ids",
];

/// A device's entries in the PCI ID database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PciName {
    /// The device line: `Navi 31 [Radeon RX 7900 XT/7900 XTX/7900M]`.
    pub device: String,
    /// The subsystem line for this board, if listed.
    pub subsystem: Option<String>,
}

/// Looks a device up in the system's PCI ID database. Read once per card
/// when the cards are listed; the file is 1.5 MB and isn't kept.
pub fn lookup(vendor: u16, device: u16, subsystem: Option<(u16, u16)>) -> Option<PciName> {
    PCI_IDS.iter().find_map(|path| {
        let file = File::open(path).ok()?;
        scan(BufReader::new(file), vendor, device, subsystem)
    })
}

/// Walks the database for one vendor's block, then the device in it, then
/// the board among the device's subsystems. The layout:
///
/// ```text
/// 1002  Advanced Micro Devices, Inc. [AMD/ATI]
/// \t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX/7900M]
/// \t\t1eae 7901  RX-79XMERCB9 [SPEEDSTER MERC 310 RX 7900 XTX]
/// ```
pub fn scan(
    mut r: impl BufRead,
    vendor: u16,
    device: u16,
    subsystem: Option<(u16, u16)>,
) -> Option<PciName> {
    let vendor = format!("{vendor:04x}");
    let device = format!("{device:04x}");
    let subsystem = subsystem.map(|(v, d)| format!("{v:04x} {d:04x}"));
    let mut line = Vec::with_capacity(128);
    let mut in_vendor = false;
    let mut found: Option<PciName> = None;
    loop {
        line.clear();
        if r.read_until(b'\n', &mut line).ok()? == 0 {
            return found;
        }
        let text = line.trim_ascii_end();
        if text.is_empty() || text[0] == b'#' {
            continue;
        }
        if text[0] != b'\t' {
            // A vendor line (or, at the end, the class list): past ours.
            if in_vendor {
                return found;
            }
            in_vendor = entry(text, vendor.as_bytes()).is_some();
            continue;
        }
        if !in_vendor {
            continue;
        }
        if text.get(1) != Some(&b'\t') {
            // A device line: past ours, or ours.
            if found.is_some() {
                return found;
            }
            if let Some(name) = entry(&text[1..], device.as_bytes()) {
                found = Some(PciName {
                    device: lossy(name),
                    subsystem: None,
                });
            }
        } else if let (Some(f), Some(want)) = (found.as_mut(), &subsystem)
            && let Some(name) = entry(&text[2..], want.as_bytes())
        {
            f.subsystem = Some(lossy(name));
            return found;
        }
    }
}

/// The name after `id` on a line that starts with it, then whitespace.
fn entry<'a>(text: &'a [u8], id: &[u8]) -> Option<&'a [u8]> {
    let rest = text.strip_prefix(id)?;
    if !rest.first().is_some_and(u8::is_ascii_whitespace) {
        return None;
    }
    Some(rest.trim_ascii())
}

fn lossy(b: &[u8]) -> String {
    String::from_utf8_lossy(b).into_owned()
}

/// The name to show, from the database's entries. `brand` ("AMD", "NVIDIA",
/// "Intel") is put in front unless the name already starts with it.
pub fn friendly(brand: &str, name: &PciName) -> String {
    let models = bracketed(&name.device).unwrap_or(&name.device);
    let pick = name
        .subsystem
        .as_deref()
        .and_then(|sub| pick_model(models, sub));
    let model = pick.as_deref().unwrap_or(models);
    if brand.is_empty() || starts_with_word(model, brand) {
        model.to_owned()
    } else {
        format!("{brand} {model}")
    }
}

/// The text in the brackets that end a device line: the marketing name(s)
/// after the codename. `None` without one ("Raphael", "HD Graphics 620").
fn bracketed(device: &str) -> Option<&str> {
    let inner = device.strip_suffix(']')?;
    let open = inner.rfind('[')?;
    let inner = inner[open + 1..].trim();
    (!inner.is_empty()).then_some(inner)
}

/// Which of the models in `models` (`Radeon RX 7900 XT/7900 XTX/7900M`)
/// the subsystem's name mentions, written out in full
/// (`Radeon RX 7900 XTX`). `None` when it mentions none of them, or more
/// than one: a board sold as either. A model mentioned only as part of a
/// longer one (`6800` in `RX 6800 XT`) doesn't count.
fn pick_model(models: &str, subsystem: &str) -> Option<String> {
    let mut parts = models.split('/').map(str::trim).filter(|p| !p.is_empty());
    let base: Vec<&str> = parts.next()?.split_whitespace().collect();
    let mut expanded: Vec<(Vec<&str>, usize)> = Vec::new();
    for part in parts {
        let words: Vec<&str> = part.split_whitespace().collect();
        let at = replace_at(&base, &words);
        let mut full = base[..at].to_vec();
        full.extend(&words);
        expanded.push((full, at));
    }
    if expanded.is_empty() {
        return None; // one model: nothing to choose
    }
    // The first model's distinguishing part starts where the second's does.
    let first_at = expanded[0].1;
    expanded.insert(0, (base.clone(), first_at));

    // Each model's distinguishing words, where the subsystem mentions them.
    let mut mentioned: Vec<(String, Vec<usize>, &Vec<&str>)> = Vec::new();
    for (full, at) in &expanded {
        let key = full[*at..].join(" ");
        if key.is_empty() || mentioned.iter().any(|(k, _, _)| *k == key) {
            continue;
        }
        let starts = word_matches(subsystem, &key);
        if !starts.is_empty() {
            mentioned.push((key, starts, full));
        }
    }
    let within = |key: &str, start: usize, other: &(String, Vec<usize>, _)| {
        other.0.len() > key.len()
            && other
                .1
                .iter()
                .any(|&o| o <= start && start + key.len() <= o + other.0.len())
    };
    let mut alone = mentioned.iter().filter(|(key, starts, _)| {
        !starts
            .iter()
            .all(|&s| mentioned.iter().any(|other| within(key, s, other)))
    });
    let (_, _, full) = alone.next()?;
    alone.next().is_none().then(|| full.join(" "))
}

/// Where a shortened model's words go in the first model's: from the last
/// word that starts with the same character (`7900M` replaces `7900`,
/// `6900 XT` replaces `6800`), else in place of as many trailing words
/// (`64` replaces `56` in `Radeon RX Vega 56`).
fn replace_at(base: &[&str], words: &[&str]) -> usize {
    let first = words.first().and_then(|w| w.chars().next());
    base.iter()
        .rposition(|w| w.chars().next() == first)
        .unwrap_or_else(|| base.len().saturating_sub(words.len()))
}

/// Where `needle` appears in `haystack` as whole words, ignoring case:
/// "7900 XT" is not in "RX 7900 XTX".
fn word_matches(haystack: &str, needle: &str) -> Vec<usize> {
    let (h, n) = (haystack.as_bytes(), needle.as_bytes());
    if n.is_empty() || n.len() > h.len() {
        return Vec::new();
    }
    (0..=h.len() - n.len())
        .filter(|&i| {
            h[i..i + n.len()].eq_ignore_ascii_case(n)
                && (i == 0 || !h[i - 1].is_ascii_alphanumeric())
                && h.get(i + n.len())
                    .is_none_or(|c| !c.is_ascii_alphanumeric())
        })
        .collect()
}

fn starts_with_word(s: &str, word: &str) -> bool {
    s.len() >= word.len()
        && s.as_bytes()[..word.len()].eq_ignore_ascii_case(word.as_bytes())
        && s.as_bytes().get(word.len()).is_none_or(|&c| c == b' ')
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The database's shape: vendor lines at column 0, devices one tab in,
    /// boards two.
    const DB: &str = "\
# comment line
1002  Advanced Micro Devices, Inc. [AMD/ATI]
\t73ff  Navi 23 [Radeon RX 6600/6600 XT/6600M]
\t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M]
\t\t1002 0e3b  RX 7900 XTX / RX 7900 GRE [XFX]
\t\t1eae 7901  RX-79XMERCB9 [SPEEDSTER MERC 310 RX 7900 XTX]
\t\t1eae 790a  RX-79GMERCBR [XFX RX 7900 GRE]
\t164e  Raphael
10de  NVIDIA Corporation
\t2684  AD102 [GeForce RTX 4090]
\t\t1043 889d  ROG Strix GeForce RTX 4090 OC
8086  Intel Corporation
\t56a0  DG2 [Arc A770]
\ta780  Raptor Lake-S GT1 [UHD Graphics 770]

# List of known device classes
C 00  Unclassified device
\t00  Non-VGA unclassified device
";

    fn look(v: u16, d: u16, sub: Option<(u16, u16)>) -> Option<PciName> {
        scan(DB.as_bytes(), v, d, sub)
    }

    #[test]
    fn scans_devices_and_boards() {
        let n = look(0x1002, 0x744c, Some((0x1eae, 0x7901))).unwrap();
        assert_eq!(
            n.device,
            "Navi 31 [Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M]"
        );
        assert_eq!(
            n.subsystem.as_deref(),
            Some("RX-79XMERCB9 [SPEEDSTER MERC 310 RX 7900 XTX]")
        );
        // A board the database doesn't list.
        let n = look(0x1002, 0x744c, Some((0x1234, 0x5678))).unwrap();
        assert_eq!(n.subsystem, None);
        assert_eq!(look(0x1002, 0x73ff, None).unwrap().subsystem, None);
        assert_eq!(
            look(0x8086, 0xa780, None).unwrap().device,
            "Raptor Lake-S GT1 [UHD Graphics 770]"
        );
        // A subsystem ID is never taken for a device ID.
        assert_eq!(look(0x1002, 0x0e3b, None), None);
        assert_eq!(look(0x1002, 0xffff, None), None);
        assert_eq!(look(0xdead, 0x744c, None), None);
        // The class list at the end is not a vendor.
        assert_eq!(look(0x0000, 0x0000, None), None);
    }

    fn named(v: u16, d: u16, sub: Option<(u16, u16)>, brand: &str) -> String {
        friendly(brand, &look(v, d, sub).unwrap())
    }

    #[test]
    fn friendly_names() {
        assert_eq!(
            named(0x1002, 0x744c, Some((0x1eae, 0x7901)), "AMD"),
            "AMD Radeon RX 7900 XTX"
        );
        assert_eq!(
            named(0x1002, 0x744c, Some((0x1eae, 0x790a)), "AMD"),
            "AMD Radeon RX 7900 GRE"
        );
        // A board sold as either model, and one the database doesn't list.
        let all = "AMD Radeon RX 7900 XT/7900 XTX/7900 GRE/7900M";
        assert_eq!(named(0x1002, 0x744c, Some((0x1002, 0x0e3b)), "AMD"), all);
        assert_eq!(named(0x1002, 0x744c, None, "AMD"), all);
        assert_eq!(named(0x1002, 0x164e, None, "AMD"), "AMD Raphael");
        assert_eq!(
            named(0x10de, 0x2684, Some((0x1043, 0x889d)), "NVIDIA"),
            "NVIDIA GeForce RTX 4090"
        );
        assert_eq!(named(0x8086, 0x56a0, None, "Intel"), "Intel Arc A770");
        assert_eq!(
            named(0x8086, 0xa780, None, "Intel"),
            "Intel UHD Graphics 770"
        );
        assert_eq!(named(0x8086, 0xa780, None, ""), "UHD Graphics 770");
        // The brand isn't doubled.
        let n = PciName {
            device: "Foo [AMD Radeon 780M]".into(),
            subsystem: None,
        };
        assert_eq!(friendly("AMD", &n), "AMD Radeon 780M");
        let n = PciName {
            device: "Foo [AMDX 1]".into(),
            subsystem: None,
        };
        assert_eq!(friendly("AMD", &n), "AMD AMDX 1");
    }

    #[test]
    fn picks_models() {
        let cases = [
            (
                "Radeon RX 7900 XT/7900 XTX/7900M",
                "MERC RX 7900 XTX",
                Some("Radeon RX 7900 XTX"),
            ),
            (
                "Radeon RX 7900 XT/7900 XTX/7900M",
                "Radeon RX 7900 XT",
                Some("Radeon RX 7900 XT"),
            ),
            (
                "Radeon RX 7900 XT/7900 XTX/7900M",
                "rx 7900m",
                Some("Radeon RX 7900M"),
            ),
            (
                "Radeon RX 6800/6800 XT / 6900 XT",
                "Radeon RX 6800 XT",
                Some("Radeon RX 6800 XT"),
            ),
            (
                "Radeon RX 6800/6800 XT / 6900 XT",
                "RX 6900 XT Gaming",
                Some("Radeon RX 6900 XT"),
            ),
            (
                "Radeon RX 6800/6800 XT / 6900 XT",
                "Radeon RX 6800",
                Some("Radeon RX 6800"),
            ),
            (
                "Radeon RX 470/480/570/570X/580/580X/590",
                "RX 580 8GB",
                Some("Radeon RX 580"),
            ),
            (
                "Radeon RX 470/480/570/570X/580/580X/590",
                "RX 570X",
                Some("Radeon RX 570X"),
            ),
            (
                "Radeon RX Vega 56/64",
                "Radeon RX Vega 64",
                Some("Radeon RX Vega 64"),
            ),
            (
                "Radeon RX 5500/5500M / Pro 5300/5500M",
                "Radeon Pro 5300",
                Some("Radeon Pro 5300"),
            ),
            (
                "GeForce RTX 3060 Ti / 3070 Ti",
                "RTX 3070 Ti",
                Some("GeForce RTX 3070 Ti"),
            ),
            (
                "Radeon RX 7900 XT/7900 XTX",
                "RX 7900 XTX / RX 7900 XT",
                None,
            ),
            (
                "Radeon RX 5500/5500M / Pro 5300/5500M",
                "RX 5500M",
                Some("Radeon RX 5500M"),
            ),
            (
                "Radeon RX 6800/6800 XT / 6900 XT",
                "RX 6800 XT / 6800",
                None,
            ),
            ("Radeon RX 7900 XT/7900 XTX", "Something else", None),
            ("GeForce RTX 4090", "RTX 4090", None),
        ];
        for (models, sub, want) in cases {
            assert_eq!(
                pick_model(models, sub).as_deref(),
                want,
                "{models:?} / {sub:?}"
            );
        }
    }

    #[test]
    fn whole_words() {
        assert_eq!(word_matches("RX 7900 XT", "7900 XT"), [3]);
        assert!(word_matches("RX 7900 XTX", "7900 XT").is_empty());
        assert!(word_matches("RX17900 XT", "7900 XT").is_empty());
        assert_eq!(word_matches("[7900 XT]", "7900 xt"), [1]);
        assert_eq!(word_matches("XT/XT", "xt"), [0, 3]);
        assert!(word_matches("79", "7900").is_empty());
        assert!(word_matches("anything", "").is_empty());
    }

    #[test]
    fn brackets() {
        assert_eq!(
            bracketed("Navi 31 [Radeon RX 7900 XT]"),
            Some("Radeon RX 7900 XT")
        );
        assert_eq!(
            bracketed("Vega 10 XL/XT [Radeon RX Vega 56/64]"),
            Some("Radeon RX Vega 56/64")
        );
        assert_eq!(bracketed("Raphael"), None);
        assert_eq!(bracketed("Odd []"), None);
    }

    /// The system database, where there is one, is in the layout `scan`
    /// expects.
    #[test]
    fn live_database() {
        let Some(path) = PCI_IDS.iter().find(|p| std::path::Path::new(p).exists()) else {
            return;
        };
        let file = File::open(path).unwrap();
        let n = scan(BufReader::new(file), 0x1002, 0x744c, None);
        // Navi 31 has been in the database since 2022.
        assert!(n.is_some_and(|n| n.device.contains("Navi 31")), "{path}");
    }
}
