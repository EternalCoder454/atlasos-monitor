//! Reads this machine once and prints what `health` finds wrong with it:
//! `cargo run -p atlas-sysinfo --example health`.

use atlas_sysinfo::gpu::{self, GpuSampler};
use atlas_sysinfo::health::{self, DiskSpace, Drive, Graphics, Machine};
use atlas_sysinfo::services::ServiceReader;
use atlas_sysinfo::smart::SmartReader;
use atlas_sysinfo::stats::{cpu, disk, memory};

fn main() {
    let cpu_temperature = cpu::CpuSampler::new().sample().temperature;
    let memory = memory::MemorySampler::new().sample();

    let cards = gpu::cards();
    let readings: Vec<_> = cards
        .iter()
        .map(|c| GpuSampler::new(c, &cards).sample())
        .collect();
    let graphics: Vec<Graphics> = cards
        .iter()
        .zip(&readings)
        .map(|(c, g)| Graphics {
            name: &c.name,
            temperature: g.temperature,
        })
        .collect();

    let disks = disk::disks();
    let spaces: Vec<DiskSpace> = disks
        .iter()
        .map(|d| DiskSpace {
            disk: d,
            space: disk::space(&d.mounts),
        })
        .collect();

    let mut smart = SmartReader::new();
    let health: Vec<_> = disks
        .iter()
        .filter(|d| !d.is_swap)
        .filter_map(|d| Some((d, smart.as_mut()?.read(&d.name)?)))
        .collect();
    let drives: Vec<Drive> = health
        .iter()
        .map(|(d, h)| Drive {
            name: d.label(),
            health: h,
        })
        .collect();

    let failed = ServiceReader::new()
        .and_then(|mut r| r.failed())
        .unwrap_or_default();

    let m = Machine {
        cpu_temperature,
        graphics: &graphics,
        memory,
        disks: &spaces,
        drives: &drives,
        failed_services: &failed,
    };
    println!(
        "read: cpu {cpu_temperature:?} °C, {} cards, {} disks, {} drives, {} failed services",
        graphics.len(),
        spaces.len(),
        drives.len(),
        failed.len()
    );
    let alerts = health::check(&m);
    if alerts.is_empty() {
        println!("Nothing wrong.");
    }
    for a in &alerts {
        println!("{:?}: {}\n  {}", a.level, a.title(), a.detail());
    }
}
