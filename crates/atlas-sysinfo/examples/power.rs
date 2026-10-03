//! Prints every battery and adapter, then samples them once a second:
//! `cargo run -p atlas-sysinfo --example power [ticks]`. For checking the
//! numbers by eye on a laptop and counting a tick's syscalls with strace.

use std::time::Duration;

use atlas_sysinfo::power::{self, Battery, PowerSampler};

fn main() {
    let ticks: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(1);
    println!("available: {}", power::available());
    let mut s = PowerSampler::new();
    for tick in 0..ticks {
        if tick > 0 {
            std::thread::sleep(Duration::from_secs(1));
            println!();
        }
        let r = s.sample();
        if let Some(t) = &r.total {
            print("Total", t);
        }
        if r.packs.len() > 1 {
            for p in &r.packs {
                print(&p.name, p);
            }
        }
        for a in &r.adapters {
            let watts = a.watts.map(|w| format!(", {w:.0} W")).unwrap_or_default();
            let online = if a.online { "online" } else { "offline" };
            println!("{} ({}, {:?}): {online}{watts}", a.name, a.kernel, a.kind);
        }
        println!("on AC: {:?}", r.on_ac());
    }
}

fn print(title: &str, b: &Battery) {
    let f = |v: Option<f64>, unit: &str| v.map_or("—".to_owned(), |v| format!("{v:.1} {unit}"));
    println!(
        "{title}: {} {} {} {}",
        b.name, b.vendor, b.model, b.technology
    );
    println!(
        "  {} {}  {} of {} (design {})  {}  {}",
        b.status.label(),
        f(b.percent, "%"),
        f(b.energy_wh, "Wh"),
        f(b.full_wh, "Wh"),
        f(b.design_wh, "Wh"),
        f(b.watts, "W"),
        f(b.volts, "V"),
    );
    println!(
        "  health {} (wear {}), cycles {:?}, limit {:?}, time left {:?}",
        f(b.health, "%"),
        b.wear(),
        b.cycles,
        b.charge_limit,
        b.time_left
    );
}
