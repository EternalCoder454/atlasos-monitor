//! One sample of each reader against this machine's live /proc and /sys, to
//! set beside the Go version's benchmarks (BenchmarkCollect,
//! BenchmarkCollectCPU, BenchmarkHeldOpen). Each reader runs once a second
//! while its page is open, so a sample's time is its share of a second.
//!
//! `cargo bench -p atlas-sysinfo --bench readers`. In a container, add
//! `--pid=host` so the process scan sees the machine's processes.

use std::hint::black_box;
use std::io::Write;

use atlas_sysinfo::process::{ProcessSampler, Wanted};
use atlas_sysinfo::sensors::Sensors;
use atlas_sysinfo::stats::cpu::CpuSampler;
use atlas_sysinfo::stats::disk::{self, DiskSampler};
use atlas_sysinfo::stats::memory::MemorySampler;
use atlas_sysinfo::stats::net::NetSampler;
use atlas_sysinfo::sysfs::{self, HeldFile};
use criterion::{Criterion, criterion_group, criterion_main};

fn processes(c: &mut Criterion) {
    // As the Apps page has it by default, then with kernel threads shown.
    let mut s = ProcessSampler::new(Wanted::default());
    c.bench_function("process scan", |b| b.iter(|| black_box(s.sample().len())));
    let mut s = ProcessSampler::new(Wanted {
        kernel_threads: true,
        ..Wanted::default()
    });
    c.bench_function("process scan, kernel threads", |b| {
        b.iter(|| black_box(s.sample().len()))
    });
}

fn system(c: &mut Criterion) {
    let mut cpu = CpuSampler::new();
    c.bench_function("cpu sample", |b| {
        b.iter(|| {
            black_box(cpu.sample());
        })
    });
    let mut memory = MemorySampler::new();
    c.bench_function("memory sample", |b| b.iter(|| black_box(memory.sample())));
    let mut disks = DiskSampler::new(&disk::disks());
    c.bench_function("disk sample", |b| {
        b.iter(|| black_box(disks.sample().len()))
    });
    let mut net = NetSampler::new();
    c.bench_function("net sample", |b| b.iter(|| black_box(net.sample().len())));
    let mut sensors = Sensors::new();
    c.bench_function("sensors sample", |b| {
        b.iter(|| black_box(sensors.sample().len()))
    });
}

fn held(c: &mut Criterion) {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("value");
    std::fs::File::create(&path)
        .unwrap()
        .write_all(b"3456789\n")
        .unwrap();
    let mut f = HeldFile::open(&path).unwrap();
    c.bench_function("held file", |b| b.iter(|| black_box(f.uint())));
    c.bench_function("read file", |b| {
        b.iter(|| black_box(sysfs::read_uint(&path)))
    });
}

criterion_group!(benches, processes, system, held);
criterion_main!(benches);
