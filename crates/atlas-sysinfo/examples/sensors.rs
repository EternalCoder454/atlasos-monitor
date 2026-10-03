//! Prints every sensor, then samples them once a second:
//! `cargo run -p atlas-sysinfo --example sensors [ticks]`. Folded per-core
//! readings are marked with `·`. For checking names and numbers by eye and
//! counting a tick's syscalls with strace.

use std::time::Duration;

use atlas_sysinfo::sensors::{self, Sensors};

fn main() {
    let ticks: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1);
    println!("available: {}", sensors::available());
    let mut s = Sensors::new();
    for tick in 0..ticks {
        if tick > 0 {
            std::thread::sleep(Duration::from_secs(1));
            println!();
        }
        for d in s.sample() {
            let asleep = if d.asleep { ", asleep" } else { "" };
            println!(
                "{} ({:?}, {} {}{asleep})",
                d.name, d.category, d.driver, d.node
            );
            for r in &d.readings {
                let limits = match (r.high, r.critical) {
                    (None, None) => String::new(),
                    (h, c) => format!("  high {h:?} crit {c:?}"),
                };
                let fold = if r.folded { "·" } else { " " };
                println!(
                    "  {fold} {:<16} {:>10}  {}{limits}",
                    r.label,
                    r.display(),
                    r.attribute
                );
            }
        }
    }
}
