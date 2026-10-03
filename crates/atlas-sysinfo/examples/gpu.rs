//! Prints the graphics cards and samples them once a second:
//! `cargo run -p atlas-sysinfo --example gpu [ticks] [--clients]`.
//! `--clients` adds a second reading of each card's load from its clients'
//! counters, the path Intel cards take, to compare with amdgpu's own figure.
//! For checking numbers by eye and counting a tick's syscalls with strace.

use std::time::Duration;

use atlas_sysinfo::gpu::{self, GpuSampler};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let ticks: usize = args.iter().find_map(|a| a.parse().ok()).unwrap_or(5);
    let clients = args.iter().any(|a| a == "--clients");

    let cards = gpu::cards();
    if cards.is_empty() {
        println!("no graphics card");
        return;
    }
    for c in &cards {
        println!("{c:#?}");
    }
    let mut samplers: Vec<(String, GpuSampler, Option<GpuSampler>)> = cards
        .iter()
        .map(|c| {
            let other = clients.then(|| GpuSampler::counting_clients(c, &cards));
            (c.node.clone(), GpuSampler::new(c, &cards), other)
        })
        .collect();
    for _ in 0..ticks {
        std::thread::sleep(Duration::from_secs(1));
        for (node, s, other) in &mut samplers {
            let g = s.sample();
            let mib = |b: Option<u64>| b.map_or("-".into(), |b| format!("{}", b >> 20));
            let one = |v: Option<f64>| v.map_or("-".into(), |v| format!("{v:.1}"));
            print!(
                "{node}: load {}% vram {}/{} MiB gtt {} MiB temp {}/{}/{} °C fan {} rpm {}% power {} W clocks {}/{} MHz{}",
                one(g.usage),
                mib(g.memory_used),
                mib(g.memory_total),
                mib(g.gtt_used),
                one(g.temperature),
                one(g.hotspot),
                one(g.memory_temperature),
                g.fan_rpm.map_or("-".into(), |r| r.to_string()),
                one(g.fan_percent),
                one(g.power),
                one(g.core_clock),
                one(g.memory_clock),
                if g.asleep { " (asleep)" } else { "" },
            );
            match other {
                Some(o) => println!("  clients {}%", one(o.sample().usage)),
                None => println!(),
            }
        }
    }
    // Read once the card has been awake.
    for (node, s, _) in &samplers {
        println!("{node}: power limit {:?} W", s.power_limit());
    }
}
