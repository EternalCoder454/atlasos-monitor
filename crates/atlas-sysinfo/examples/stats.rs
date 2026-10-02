//! Prints what the stats readers see on this machine, then samples once a
//! second: `cargo run -p atlas-sysinfo --example stats [ticks]`.
//! For checking numbers by eye and for counting a tick's syscalls with strace.

use std::time::Duration;

use atlas_sysinfo::stats::{cpu, disk, memory, net};

fn main() {
    let ticks: usize = std::env::args()
        .nth(1)
        .and_then(|a| a.parse().ok())
        .unwrap_or(3);

    let info = cpu::info();
    println!("{info:#?}");
    let disks = disk::disks();
    for d in &disks {
        println!(
            "disk {} {:?} root={} swap={} size={} mounts={:?} space={:?}",
            d.name,
            d.label(),
            d.is_root,
            d.is_swap,
            d.size,
            d.mounts,
            disk::space(&d.mounts)
        );
    }
    let addrs = net::addresses().unwrap_or_default();
    for i in net::interfaces() {
        println!("net {i:?} {:?}", addrs.get(&i.index));
    }

    let mut cpu = cpu::CpuSampler::new();
    let mut mem = memory::MemorySampler::new();
    let mut dio = disk::DiskSampler::new(&disks);
    let mut nio = net::NetSampler::new();
    for _ in 0..ticks {
        std::thread::sleep(Duration::from_secs(1));
        let c = cpu.sample();
        println!(
            "cpu {:.1}% freq={:?} temp={:?} cores={:.0?}",
            c.usage, c.frequency_mhz, c.temperature, c.cores
        );
        println!("mem {:?}", mem.sample());
        for (d, io) in disks.iter().zip(dio.sample()) {
            println!("io {} r={:.0} w={:.0}", d.name, io.read_rate, io.write_rate);
        }
        for io in nio.sample() {
            println!("io {} rx={:.0} tx={:.0}", io.name, io.rx_rate, io.tx_rate);
        }
        println!("route {:?}", nio.default_route());
    }
}
