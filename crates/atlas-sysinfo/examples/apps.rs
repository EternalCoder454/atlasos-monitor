//! Groups the live process table by application and prints each row with
//! its name, icon and members: `cargo run -p atlas-sysinfo --example apps
//! [theme] [--bench]`. The theme defaults to `breeze`. `--bench` times the
//! first grouping (desktop files read, icon themes walked) and a later one.

use std::time::{Duration, Instant};

use atlas_sysinfo::apps::{Column, Grouper, Resolver, sort};
use atlas_sysinfo::process::ProcessSampler;

fn main() {
    let theme = std::env::args()
        .skip(1)
        .find(|a| !a.starts_with("--"))
        .unwrap_or_else(|| "breeze".into());
    let bench = std::env::args().any(|a| a == "--bench");
    let mut sampler = ProcessSampler::default();
    let mut apps = Resolver::for_session(&theme);
    let mut grouper = Grouper::default();
    std::thread::sleep(Duration::from_secs(1));
    let procs = sampler.sample().to_vec();

    let start = Instant::now();
    let groups = grouper.group(&procs, &mut apps).to_vec();
    let first = start.elapsed();
    if bench {
        let start = Instant::now();
        for _ in 0..100 {
            std::hint::black_box(grouper.group(&procs, &mut apps));
        }
        println!(
            "{} processes in {} rows: first grouping {:.2} ms, then {:.1} µs",
            procs.len(),
            groups.len(),
            first.as_secs_f64() * 1e3,
            start.elapsed().as_secs_f64() * 1e6 / 100.0,
        );
        return;
    }

    let mut order: Vec<usize> = (0..groups.len()).collect();
    sort(&mut order, &groups, Column::Memory, true);
    for &i in &order {
        let g = &groups[i];
        let icon = g.app.as_ref().and_then(|a| a.icon.as_ref());
        println!(
            "{:<28} {:>3} procs {:>6} MiB cpu {:>5.1}%  {:<34} {}",
            g.total.name,
            g.count,
            g.total.memory >> 20,
            g.total.cpu,
            g.app.as_ref().map_or("", |a| &a.id),
            icon.map_or("-".into(), |i| i.source().into_owned()),
        );
    }
    println!("Qt icon search paths: {:?}", apps.icon_search_paths());
}
