//! System readers for Atlas Monitor. Plain Rust, no Qt: the app crate runs
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
//! - [`sysmem`]: Atlas Monitor's own memory use, for Settings.

pub mod apps;
pub mod autostart;
pub mod ease;
pub mod files;
pub mod gpu;
pub mod health;
pub mod power;
pub mod process;
pub mod sensors;
pub mod services;
pub mod smart;
pub mod stats;
pub mod sysfs;
pub mod sysmem;
