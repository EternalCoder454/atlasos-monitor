//! System readers for Atlas Monitor. Plain Rust, no Qt: the app crate runs
//! these on its sampling thread and posts the results to the Qt thread.
//!
//! Every reader is split in two: a function that reads the live system and a
//! pure `parse_*` function over the file's text, which the tests feed with
//! recorded files from `tests/fixtures`. Tests assert invariants (a total is at
//! least its parts, a percentage is within 0..=100), never what one machine
//! happens to have: CI runs in a container with no GPU, battery or system bus.
//!
//! - [`sysmem`]: Atlas Monitor's own memory use, for Settings.

pub mod sysmem;
