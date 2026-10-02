//! Samples the process table once a second and prints the busiest:
//! `cargo run -p atlas-sysinfo --example processes [ticks] [--kernel]
//! [--no-disk] [--no-gpu] [--no-net] [--quiet] [--bench [--interval ms]]`.
//! For checking numbers against `top` by eye and for counting a tick's
//! syscalls with strace. `--bench` times each scan's on-CPU time from
//! `/proc/self/schedstat` (nanoseconds, kernel time included) and prints
//! the median.

use std::time::Duration;

use atlas_sysinfo::process::{ProcessSampler, Wanted};

fn main() {
    let mut args = std::env::args().skip(1);
    let ticks: usize = args.next().and_then(|a| a.parse().ok()).unwrap_or(3);
    let flag = |f: &str| std::env::args().any(|a| a == f);
    let quiet = flag("--quiet");
    let mut sampler = ProcessSampler::new(Wanted {
        kernel_threads: flag("--kernel"),
        disk: !flag("--no-disk"),
        gpu: !flag("--no-gpu"),
        network: !flag("--no-net"),
    });
    if flag("--bench") {
        // Each scan timed on its own, at the app's own 1 Hz unless
        // --interval says otherwise: back to back, the kernel's caches are
        // warm and a scan looks cheaper than it is.
        let interval = std::env::args()
            .skip_while(|a| a != "--interval")
            .nth(1)
            .and_then(|ms| ms.parse().ok())
            .unwrap_or(1000);
        let ticks = ticks.max(1);
        let mut ns = Vec::with_capacity(ticks);
        for _ in 0..ticks {
            std::thread::sleep(Duration::from_millis(interval));
            let start = on_cpu_ns();
            std::hint::black_box(sampler.sample());
            ns.push(on_cpu_ns() - start);
        }
        ns.sort_unstable();
        let ms = |v: u64| v as f64 / 1e6;
        println!(
            "{} scans of {} processes: median {:.3} ms, p90 {:.3} ms, mean {:.3} ms",
            ns.len(),
            sampler.sample().len(),
            ms(ns[ns.len() / 2]),
            ms(ns[ns.len() * 9 / 10]),
            ms(ns.iter().sum::<u64>() / ns.len() as u64),
        );
        return;
    }
    for _ in 0..ticks {
        std::thread::sleep(Duration::from_secs(1));
        let mut procs = sampler.sample().to_vec();
        if quiet {
            continue;
        }
        procs.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
        println!("{} processes", procs.len());
        for p in procs.iter().take(12) {
            println!(
                "{:>7} {:<16} cpu {:>5.1}% mem {:>6} MiB gpu {:>6} net {:>9} disk {:>9} {:?} {}",
                p.pid,
                p.name,
                p.cpu,
                p.memory >> 20,
                p.gpu.map_or("-".into(), |g| format!("{g:.1}%")),
                p.net_in
                    .zip(p.net_out)
                    .map_or("-".into(), |(i, o)| format!("{:.0}", i + o)),
                p.disk_read
                    .zip(p.disk_write)
                    .map_or("-".into(), |(r, w)| format!("{:.0}", r + w)),
                p.impact(),
                p.unit.as_deref().unwrap_or(""),
            );
        }
    }
}

/// Time this process has spent on a CPU, in ns.
fn on_cpu_ns() -> u64 {
    let s = std::fs::read_to_string("/proc/self/schedstat").unwrap();
    s.split_whitespace().next().unwrap().parse().unwrap()
}
