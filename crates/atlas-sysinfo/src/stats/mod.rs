//! The Hardware pages' figures: processor, memory, disks and network.
//!
//! Each part has two halves. What doesn't change while Atlas runs (the CPU
//! model, a disk's size, an interface's MAC) is read once by a plain function.
//! What does is read by a sampler: a struct that holds its kernel files open
//! (see [`crate::sysfs`]), keeps the previous counters, and turns them into
//! rates on each `sample()`. Samplers are owned by the sampling thread; they
//! reuse their buffers, so a tick allocates nothing once the first has run.
//!
//! A sampler takes its first counters when it is made, so the first `sample()`
//! is already a real reading over the time since then, never a made-up zero.
//! History (the 60-sample chart rings) lives in the app, not here.

pub mod cpu;
pub mod disk;
pub mod memory;
pub mod net;

use std::time::Instant;

/// Splits a file's contents into lines, without their newlines.
fn lines(data: &[u8]) -> impl Iterator<Item = &[u8]> {
    data.split(|&c| c == b'\n')
}

/// Bytes per second from two readings of a counter that only goes up. A
/// counter that went backwards (a device reset, a wrap) gives 0, not a
/// huge figure.
fn rate(current: u64, previous: u64, seconds: f64) -> f64 {
    if current < previous || seconds <= 0.0 {
        return 0.0;
    }
    (current - previous) as f64 / seconds
}

/// The time since `last` in seconds, and moves `last` to now.
fn elapsed(last: &mut Instant) -> f64 {
    let now = Instant::now();
    let seconds = now.duration_since(*last).as_secs_f64();
    *last = now;
    seconds
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rate_guards_against_resets() {
        assert_eq!(rate(300, 100, 2.0), 100.0);
        assert_eq!(rate(100, 300, 1.0), 0.0);
        assert_eq!(rate(100, 50, 0.0), 0.0);
    }

    #[test]
    fn lines_drop_the_newline() {
        let got: Vec<_> = lines(b"one\ntwo\nthree").collect();
        assert_eq!(got, [&b"one"[..], b"two", b"three"]);
    }
}
