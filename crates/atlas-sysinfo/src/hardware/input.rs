//! Input devices, one block per device. The live read takes each
//! `/sys/class/input/inputN` and reads it as `/proc/bus/input/devices` would
//! print it; that text itself is parsed only for tests and fixtures, since a
//! device's name can hold a line break that forges a block of its own:
//!
//! ```text
//! I: Bus=0003 Vendor=362d Product=0240 Version=0111
//! N: Name="Keychron Lemokey X4"
//! H: Handlers=sysrq kbd leds event3
//! B: PROP=0
//! B: EV=120013
//! B: KEY=1000000000007 ff9f207ac14057ff febeffdfffefffff fffffffffffffffe
//! ```
//!
//! What a device is comes from udev's tags (`ID_INPUT_KEYBOARD=1`) where its
//! database can be read, else from what the block itself says.

use std::fs::File;
use std::io::Read;
use std::path::Path;

use super::clean;

/// What a device is, as the page groups them.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default)]
pub enum InputKind {
    Keyboard,
    Mouse,
    Touchpad,
    Touchscreen,
    Tablet,
    Joystick,
    /// Keys but no keyboard: power and sleep buttons, the video bus.
    Buttons,
    #[default]
    Other,
}

impl InputKind {
    /// The name the page's code uses for the kind.
    pub fn key(self) -> &'static str {
        match self {
            Self::Keyboard => "keyboard",
            Self::Mouse => "mouse",
            Self::Touchpad => "touchpad",
            Self::Touchscreen => "touchscreen",
            Self::Tablet => "tablet",
            Self::Joystick => "joystick",
            Self::Buttons => "buttons",
            Self::Other => "other",
        }
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct InputDevice {
    pub name: String,
    pub kind: InputKind,
    /// "USB", "Bluetooth", "Built-in", "Virtual", or "" when unknown.
    pub bus: String,
}

/// One device's block, parsed. Bitmaps are the words as printed, most
/// significant first.
#[derive(Debug, Default, PartialEq, Eq)]
pub(crate) struct Block {
    pub name: String,
    pub bus: u16,
    pub handlers: Vec<String>,
    pub prop: Vec<u64>,
    pub key: Vec<u64>,
    pub abs: Vec<u64>,
}

/// The kernel's `INPUT_PROP_*` and key codes the heuristics look at.
const PROP_POINTER: usize = 0;
const PROP_DIRECT: usize = 1;
const PROP_BUTTONPAD: usize = 2;
const KEY_Q: usize = 16;
const KEY_P: usize = 25;
const BTN_TOOL_PEN: usize = 0x140;

/// Splits the file into blocks. A block without a name is dropped, and so
/// is one that doesn't start with its `I:` line or has a line twice (other
/// than `B:`): the kernel prints names as they are, so a name with line
/// breaks in it could otherwise add lines of its own.
pub(crate) fn parse_blocks(text: &str) -> Vec<Block> {
    let mut out = Vec::new();
    let mut cur = Block::default();
    let mut named = false;
    // The tags seen in this block, and whether it is still believed.
    let mut seen = String::new();
    let mut sound = true;
    for line in text.lines().chain(std::iter::once("")) {
        let line = line.trim_end();
        if line.is_empty() {
            if named && sound {
                out.push(std::mem::take(&mut cur));
            } else {
                cur = Block::default();
            }
            named = false;
            seen.clear();
            sound = true;
            continue;
        }
        let Some((tag, rest)) = line.split_once(": ") else {
            continue;
        };
        if (seen.is_empty() && tag != "I") || (tag != "B" && seen.split(',').any(|t| t == tag)) {
            sound = false;
        }
        seen.push_str(tag);
        seen.push(',');
        match tag {
            "I" => {
                cur.bus = field(rest, "Bus=")
                    .and_then(|b| u16::from_str_radix(b, 16).ok())
                    .unwrap_or(0);
            }
            "N" => {
                let name = rest.strip_prefix("Name=").unwrap_or(rest);
                cur.name = clean(name.trim_matches('"'));
                named = !cur.name.is_empty();
            }
            "H" => {
                cur.handlers = rest
                    .strip_prefix("Handlers=")
                    .unwrap_or_default()
                    .split_whitespace()
                    .map(str::to_owned)
                    .collect();
            }
            "B" => match rest.split_once('=') {
                Some(("PROP", v)) => cur.prop = bitmap(v),
                Some(("KEY", v)) => cur.key = bitmap(v),
                Some(("ABS", v)) => cur.abs = bitmap(v),
                _ => {}
            },
            _ => {}
        }
    }
    out
}

/// The value after `key` in a line of space-separated `Key=value` pairs.
fn field<'a>(line: &'a str, key: &str) -> Option<&'a str> {
    line.split_whitespace().find_map(|f| f.strip_prefix(key))
}

/// A bitmap line: hex words of one `unsigned long` each (64 bits on the
/// machines Atlas runs on). Empty for anything malformed.
fn bitmap(v: &str) -> Vec<u64> {
    let words: Option<Vec<u64>> = v
        .split_whitespace()
        .map(|w| u64::from_str_radix(w, 16).ok())
        .collect();
    words.filter(|w| w.len() <= 64).unwrap_or_default()
}

fn bit(words: &[u64], n: usize) -> bool {
    words
        .len()
        .checked_sub(1 + n / 64)
        .is_some_and(|i| words[i] >> (n % 64) & 1 == 1)
}

fn any(words: &[u64]) -> bool {
    words.iter().any(|&w| w != 0)
}

/// Whether the block has a handler named `prefix` plus digits (`mouse0`,
/// `js1`), or just the name when `prefix` is a bare handler (`kbd`).
fn handler(b: &Block, prefix: &str) -> bool {
    b.handlers.iter().any(|h| {
        h.strip_prefix(prefix)
            .is_some_and(|rest| rest.bytes().all(|c| c.is_ascii_digit()))
    })
}

/// What the device is. `udev` is the `KEY=VALUE` properties of its event
/// node's udev database entry, when that could be read: udev looked at the
/// device's capabilities itself, and an entry without `ID_INPUT=1` says it is
/// not an input device. Without it the block's own bitmaps decide.
pub(crate) fn classify(b: &Block, udev: Option<&[String]>) -> InputKind {
    match udev {
        Some(props) => by_udev(b, props),
        None => by_block(b),
    }
}

/// udev's joystick tag fires for any device with absolute axes and buttons,
/// such as a keyboard's "System Control" interface; a real stick or pad also
/// has a `jsN` handler.
fn by_udev(b: &Block, props: &[String]) -> InputKind {
    let tagged = |name: &str| {
        props
            .iter()
            .any(|p| p.strip_prefix(name).is_some_and(|v| v == "=1"))
    };
    [
        ("ID_INPUT_TOUCHSCREEN", InputKind::Touchscreen),
        ("ID_INPUT_TOUCHPAD", InputKind::Touchpad),
        ("ID_INPUT_TABLET", InputKind::Tablet),
        ("ID_INPUT_JOYSTICK", InputKind::Joystick),
        ("ID_INPUT_MOUSE", InputKind::Mouse),
        ("ID_INPUT_KEYBOARD", InputKind::Keyboard),
        ("ID_INPUT_KEY", InputKind::Buttons),
    ]
    .into_iter()
    .find(|&(name, kind)| tagged(name) && (kind != InputKind::Joystick || handler(b, "js")))
    .map_or(InputKind::Other, |(_, kind)| kind)
}

fn by_block(b: &Block) -> InputKind {
    let abs = any(&b.abs);
    if abs && bit(&b.prop, PROP_DIRECT) {
        return if bit(&b.key, BTN_TOOL_PEN) {
            InputKind::Tablet
        } else {
            InputKind::Touchscreen
        };
    }
    if handler(b, "mouse") {
        let pad = bit(&b.prop, PROP_BUTTONPAD) || (bit(&b.prop, PROP_POINTER) && abs);
        return if pad {
            InputKind::Touchpad
        } else {
            InputKind::Mouse
        };
    }
    if handler(b, "js") {
        return InputKind::Joystick;
    }
    // `/proc` lists the `kbd` handler; sysfs has no node for it, so any key
    // at all says the same there. (A mouse on a kernel without mousedev has
    // no `mouseN` either, and its button keys make it Buttons here; udev's
    // tags, read first on a normal system, name it properly.)
    if handler(b, "kbd") || any(&b.key) {
        // Q to P on a row: a typing keyboard, not a few media keys.
        return if (KEY_Q..=KEY_P).all(|k| bit(&b.key, k)) {
            InputKind::Keyboard
        } else {
            InputKind::Buttons
        };
    }
    InputKind::Other
}

/// The bus's name from the `Bus=` field (`linux/input.h`'s `BUS_*`).
pub(crate) fn bus_name(bus: u16) -> &'static str {
    match bus {
        0x03 => "USB",
        0x05 => "Bluetooth",
        0x06 => "Virtual",
        // PCI, ISA PnP, ISA, i8042, XT keyboard, I2C, host (ACPI and
        // platform buttons), SPI, RMI, Intel ISH, AMD sensor hub.
        0x01 | 0x02 | 0x10 | 0x11 | 0x12 | 0x18 | 0x19 | 0x1c | 0x1d | 0x1f | 0x20 => "Built-in",
        _ => "",
    }
}

/// The devices in `/proc/bus/input/devices`' text, kinds judged by
/// `udev_of`, which gets each block and returns its udev properties.
/// Sorted by kind then name, exact duplicates collapsed.
pub(crate) fn devices(
    text: &str,
    udev_of: impl Fn(&Block) -> Option<Vec<String>>,
) -> Vec<InputDevice> {
    from_blocks(parse_blocks(text), udev_of)
}

/// The same from `/sys/class/input`, a file per field: what the live read
/// uses, since a name there can't spill into the next field the way a name
/// with line breaks can in `/proc/bus/input/devices`.
pub(crate) fn sysfs_devices(
    class_dir: &Path,
    udev_of: impl Fn(&Block) -> Option<Vec<String>>,
) -> Vec<InputDevice> {
    from_blocks(sysfs_blocks(class_dir), udev_of)
}

/// A block per `inputN` in `class_dir`, read as `/proc/bus/input/devices`
/// would print it. One without a name is left out.
pub(crate) fn sysfs_blocks(class_dir: &Path) -> Vec<Block> {
    let Ok(entries) = std::fs::read_dir(class_dir) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for entry in entries.flatten().take(4096) {
        let file_name = entry.file_name();
        let Some(n) = file_name.to_str().and_then(|n| n.strip_prefix("input")) else {
            continue;
        };
        if !is_number(n) {
            continue;
        }
        let dir = entry.path();
        let name = read_small(&dir.join("name")).map_or_else(String::new, |n| clean(&n));
        if name.is_empty() {
            continue;
        }
        let words = |file: &str| read_small(&dir.join(file)).map_or_else(Vec::new, |v| bitmap(&v));
        // The handlers are the device's own nodes, directories beside its
        // files: event3, js0, mouse1.
        let mut handlers: Vec<String> = std::fs::read_dir(&dir)
            .map(|d| {
                d.flatten()
                    .filter_map(|e| e.file_name().to_str().map(str::to_owned))
                    .filter(|h| {
                        ["event", "js", "mouse"]
                            .iter()
                            .any(|p| h.strip_prefix(p).is_some_and(is_number))
                    })
                    .take(16)
                    .collect()
            })
            .unwrap_or_default();
        handlers.sort();
        out.push(Block {
            name,
            bus: read_small(&dir.join("id/bustype"))
                .and_then(|b| u16::from_str_radix(b.trim(), 16).ok())
                .unwrap_or(0),
            handlers,
            prop: words("properties"),
            key: words("capabilities/key"),
            abs: words("capabilities/abs"),
        });
    }
    out
}

fn from_blocks(
    blocks: Vec<Block>,
    udev_of: impl Fn(&Block) -> Option<Vec<String>>,
) -> Vec<InputDevice> {
    let mut out: Vec<InputDevice> = blocks
        .into_iter()
        .map(|b| InputDevice {
            kind: classify(&b, udev_of(&b).as_deref()),
            bus: bus_name(b.bus).to_owned(),
            name: b.name,
        })
        .collect();
    out.sort_by(|a, b| (a.kind, &a.name, &a.bus).cmp(&(b.kind, &b.name, &b.bus)));
    out.dedup();
    out
}

/// The devices in `/proc/bus/input/devices`' text, kinds from the blocks'
/// own bitmaps. [`super::read`] also asks udev.
pub fn parse_input_devices(text: &str) -> Vec<InputDevice> {
    devices(text, |_| None)
}

/// udev's properties for the device's event node, as `KEY=VALUE` lines.
/// `None` when the node or its database entry can't be read, or the entry has
/// no properties (udev hasn't processed the device yet).
pub(crate) fn udev_props(b: &Block, class_dir: &Path, udev_dir: &Path) -> Option<Vec<String>> {
    let event = b
        .handlers
        .iter()
        .find(|h| h.strip_prefix("event").is_some_and(is_number))?;
    let dev = read_small(&class_dir.join(event).join("dev"))?;
    let (major, minor) = dev.trim().split_once(':')?;
    if !is_number(major) || !is_number(minor) {
        return None;
    }
    let db = read_small(&udev_dir.join(format!("c{major}:{minor}")))?;
    let props = parse_udev(&db);
    (!props.is_empty()).then_some(props)
}

/// The `E:` lines of a udev database entry, without the `E:`.
pub(crate) fn parse_udev(db: &str) -> Vec<String> {
    db.lines()
        .filter_map(|l| l.strip_prefix("E:"))
        .map(str::to_owned)
        .collect()
}

fn is_number(s: &str) -> bool {
    !s.is_empty() && s.len() <= 9 && s.bytes().all(|c| c.is_ascii_digit())
}

/// A small kernel or udev file, at most 64 KiB, lossily decoded.
fn read_small(path: &Path) -> Option<String> {
    let mut buf = Vec::new();
    File::open(path)
        .ok()?
        .take(64 * 1024)
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}
