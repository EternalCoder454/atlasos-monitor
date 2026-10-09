//! System readers for Telamon Monitor. Plain Rust, no Qt: the app crate runs
//! these on its sampling thread and posts the results to the Qt thread.
//!
//! Every reader is split in two: a function that reads the live system and a
//! pure `parse_*` function over the file's text, which the tests feed with
//! recorded files from `tests/fixtures`. Tests assert invariants (a total is at
//! least its parts, a percentage is within 0..=100), never what one machine
//! happens to have: CI runs in a container with no GPU, battery or system bus.
//!
//! - [`sysfs`]: kernel files held open and re-read with one `pread`.
//! - [`stats`]: processor, memory, disks and network for the Hardware pages.
//! - [`gpu`]: graphics cards, their load, memory, temperatures and power.
//! - [`process`]: the Apps page's process table, details and actions.
//! - [`apps`]: which application each process is, and the table grouped by it.
//! - [`sensors`]: every hwmon temperature, fan, voltage and power reading.
//! - [`power`]: batteries and power adapters.
//! - [`smart`]: drive health from udisks2.
//! - [`services`]: systemd's services, their details and actions.
//! - [`autostart`]: what starts at login, and switching it on or off.
//! - [`ease`]: Energy Saver, easing off busy applications and putting them back.
//! - [`health`]: the short list of what is wrong with the machine.
//! - [`sysmem`]: Telamon Monitor's own memory use, for Settings.
//! - [`about`]: what this computer is, for System Info.
//! - [`hardware`]: PCI, USB and input devices, for Devices.
//! - [`text`]: what is done to text from outside before it is shown.

pub mod about;
pub mod apps;
pub mod autostart;
pub mod ease;
pub mod files;
pub mod gpu;
pub mod hardware;
pub mod health;
pub mod power;
pub mod process;
pub mod sensors;
pub mod services;
pub mod smart;
pub mod stats;
pub mod sysfs;
pub mod sysmem;
pub mod text;

#[cfg(test)]
mod hostile;

/// A character that must not reach a name, a path or a line of text: a
/// control character (a line break, an escape), an invisible format character
/// ([`invisible`]), or the line and paragraph separators, which break a line
/// without being controls.
pub(crate) fn unprintable(c: char) -> bool {
    c.is_control() || invisible(c) || joiner(c) || matches!(c, '\u{2028}' | '\u{2029}')
}

/// The zero-width non-joiner and joiner. They are format characters, but
/// Persian, Indic and emoji text needs them *between two letters*: `text`
/// keeps one there and removes it anywhere else (`text::fold_joiners`). In
/// an identifier or a path they are never kept ([`unprintable`]).
pub(crate) fn joiner(c: char) -> bool {
    matches!(c, '\u{200C}' | '\u{200D}')
}

/// A character that changes how text around it is shown without being seen
/// itself: bidirectional overrides and isolates, zero-width characters, the
/// byte order mark, the fillers that draw blank (Hangul and Braille-like
/// fillers, the combining grapheme joiner) and the like (Unicode's format
/// characters). A device could use them to make its name read as another's,
/// or to make a name that looks empty, so text from devices and firmware
/// leaves them out. Not the joiners ([`joiner`]), which depend on their
/// neighbours.
pub(crate) fn invisible(c: char) -> bool {
    matches!(
        c,
        '\u{00AD}'
            | '\u{034F}'
            | '\u{061C}'
            | '\u{115F}'..='\u{1160}'
            | '\u{180E}'
            | '\u{200B}'
            | '\u{200E}'..='\u{200F}'
            | '\u{202A}'..='\u{202E}'
            | '\u{2060}'..='\u{206F}'
            | '\u{3164}'
            | '\u{FEFF}'
            | '\u{FFA0}'
            | '\u{FFF9}'..='\u{FFFB}'
            // Tag characters: invisible, and used to smuggle text.
            | '\u{E0000}'..='\u{E007F}'
    )
}
