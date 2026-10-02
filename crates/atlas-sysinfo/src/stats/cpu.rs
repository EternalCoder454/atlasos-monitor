//! The processor: `/proc/stat` for load, `/proc/cpuinfo` for what it is, and
//! sysfs for frequency, temperature and cache sizes.

use std::collections::HashSet;
use std::fs;
use std::path::{Path, PathBuf};

use super::lines;
use crate::sysfs::{self, HeldFile};

const CPU_DIR: &str = "/sys/devices/system/cpu";
const HWMON_DIR: &str = "/sys/class/hwmon";

/// What the processor is. Read once.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CpuInfo {
    /// `model name` from `/proc/cpuinfo`, as the kernel writes it.
    pub model: String,
    pub sockets: usize,
    /// Physical cores across all sockets.
    pub physical_cores: usize,
    /// Logical processors (hardware threads) online.
    pub logical: usize,
    /// Rated base clock in MHz. Only intel_pstate reports one.
    pub base_mhz: Option<f64>,
    pub caches: Caches,
}

/// Cache sizes in bytes, from cpu0's cache hierarchy. `None` when the kernel
/// doesn't say (most VMs).
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Caches {
    pub l1d: Option<u64>,
    pub l1i: Option<u64>,
    pub l2: Option<u64>,
    pub l3: Option<u64>,
}

/// Reads what the processor is.
pub fn info() -> CpuInfo {
    let text = fs::read_to_string("/proc/cpuinfo").unwrap_or_default();
    let mut info = parse_cpuinfo(&text);
    info.base_mhz = sysfs::read_uint(format!("{CPU_DIR}/cpu0/cpufreq/base_frequency"))
        .map(|khz| khz as f64 / 1000.0);
    info.caches = read_caches(&Path::new(CPU_DIR).join("cpu0/cache"));
    info
}

/// Parses `/proc/cpuinfo` for the model and topology. Physical cores are the
/// distinct (`physical id`, `core id`) pairs; where the kernel gives neither
/// (VMs, some ARM boards) every logical processor counts as a core.
pub fn parse_cpuinfo(text: &str) -> CpuInfo {
    let mut info = CpuInfo::default();
    let mut sockets = HashSet::new();
    let mut cores = HashSet::new();
    let (mut socket, mut core) = (None, None);
    for line in text.lines().chain([""]) {
        let Some((key, value)) = line.split_once(':') else {
            // A blank line ends one processor's block.
            if line.trim().is_empty() && (socket.is_some() || core.is_some()) {
                cores.insert((socket.take(), core.take()));
            }
            continue;
        };
        let value = value.trim();
        match key.trim() {
            "processor" => info.logical += 1,
            "model name" if info.model.is_empty() => info.model = value.to_owned(),
            "physical id" => {
                sockets.insert(value);
                socket = Some(value);
            }
            "core id" => core = Some(value),
            _ => {}
        }
    }
    info.logical = info.logical.max(1);
    info.sockets = sockets.len().max(1);
    info.physical_cores = if cores.is_empty() {
        info.logical
    } else {
        cores.len()
    };
    info
}

/// Reads L1d, L1i, L2 and L3 sizes from a `cpuN/cache` directory.
pub fn read_caches(cache_dir: &Path) -> Caches {
    let mut caches = Caches::default();
    let Ok(entries) = fs::read_dir(cache_dir) else {
        return caches;
    };
    for entry in entries.flatten() {
        if !entry.file_name().to_string_lossy().starts_with("index") {
            continue;
        }
        let dir = entry.path();
        let read = |name: &str| sysfs::read_string(dir.join(name)).unwrap_or_default();
        let size = parse_cache_size(&read("size"));
        match (read("level").as_str(), read("type").as_str()) {
            ("1", "Data") => caches.l1d = size,
            ("1", "Instruction") => caches.l1i = size,
            ("2", _) => caches.l2 = size,
            ("3", _) => caches.l3 = size,
            _ => {}
        }
    }
    caches
}

/// Turns sysfs's spelling of a cache size (`"48K"`, `"36864K"`) into bytes.
/// The kernel always writes a whole number with a one-letter suffix; anything
/// else is `None` rather than a guess.
pub fn parse_cache_size(raw: &str) -> Option<u64> {
    let (digits, shift) = match raw.as_bytes().last()? {
        b'K' => (&raw[..raw.len() - 1], 10),
        b'M' => (&raw[..raw.len() - 1], 20),
        b'G' => (&raw[..raw.len() - 1], 30),
        _ => return None,
    };
    if digits.is_empty() || !digits.bytes().all(|c| c.is_ascii_digit()) {
        return None;
    }
    digits.parse::<u64>().ok()?.checked_mul(1 << shift)
}

/// Finds the package temperature input of an Intel (`coretemp`) or AMD
/// (`k10temp`, where `temp1` is Tctl) processor under a hwmon class directory.
pub fn find_temperature(hwmon_dir: &Path) -> Option<PathBuf> {
    let mut dirs: Vec<_> = fs::read_dir(hwmon_dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    dirs.sort(); // hwmon numbering is the only stable order there is
    dirs.into_iter().find_map(|dir| {
        let name = sysfs::read_string(dir.join("name"))?;
        let input = dir.join("temp1_input");
        (matches!(name.as_str(), "coretemp" | "k10temp") && input.exists()).then_some(input)
    })
}

/// One tick of processor load.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct CpuSample {
    /// Whole-processor load, 0..=100.
    pub usage: f64,
    /// Load per logical processor, 0..=100, indexed by its number. An offline
    /// processor reads 0.
    pub cores: Vec<f64>,
    /// The fastest core's current clock in MHz. `None` without cpufreq (VMs).
    pub frequency_mhz: Option<f64>,
    /// Package temperature in °C. `None` without a coretemp/k10temp sensor.
    pub temperature: Option<f64>,
}

/// Idle and total time of one processor as last seen, in jiffies.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct Times {
    idle: u64,
    total: u64,
}

/// Samples processor load, clock and temperature from held files.
#[derive(Debug)]
pub struct CpuSampler {
    stat: Option<HeldFile>,
    freq: Vec<HeldFile>,
    temp: Option<HeldFile>,
    prev_all: Times,
    prev_cores: Vec<Times>,
    sample: CpuSample,
}

/// The highest processor number a `cpuN` line may carry: x86's largest
/// `CONFIG_NR_CPUS`. A line past it is corrupt, and sizing the per-core
/// tables from it would allocate without bound.
const MAX_CORE: usize = 8192;

impl CpuSampler {
    /// Opens the files and takes the first counters.
    pub fn new() -> Self {
        let temp = find_temperature(Path::new(HWMON_DIR));
        Self::open(Path::new("/proc/stat"), Path::new(CPU_DIR), temp.as_deref())
    }

    fn open(stat: &Path, cpu_dir: &Path, temp: Option<&Path>) -> Self {
        // One line per core, about 60 bytes each, before the long intr line
        // the parse never reaches. The buffer grows if this is short.
        let mut stat = HeldFile::with_capacity(stat, 16 * 1024);
        let count = stat.as_mut().and_then(HeldFile::bytes).map_or(0, |data| {
            lines(data)
                .take_while(|l| l.starts_with(b"cpu"))
                .filter_map(|l| parse_stat_line(l)?.0)
                .max()
                .map_or(0, |max| max + 1)
        });
        let freq = (0..count)
            .filter_map(|i| {
                HeldFile::open(cpu_dir.join(format!("cpu{i}/cpufreq/scaling_cur_freq")))
            })
            .collect();
        let mut sampler = Self {
            stat,
            freq,
            temp: temp.and_then(HeldFile::open),
            prev_all: Times::default(),
            prev_cores: vec![Times::default(); count],
            sample: CpuSample {
                cores: vec![0.0; count],
                ..CpuSample::default()
            },
        };
        sampler.read_load();
        sampler.sample.usage = 0.0;
        sampler.sample.cores.fill(0.0);
        sampler
    }

    /// Reads the current load, clock and temperature.
    pub fn sample(&mut self) -> &CpuSample {
        self.read_load();
        self.sample.frequency_mhz = self
            .freq
            .iter_mut()
            .filter_map(HeldFile::uint)
            .max()
            .map(|khz| khz as f64 / 1000.0);
        self.sample.temperature = self
            .temp
            .as_mut()
            .and_then(HeldFile::uint)
            .map(|m| m as f64 / 1000.0);
        &self.sample
    }

    fn read_load(&mut self) {
        let Some(data) = self.stat.as_mut().and_then(HeldFile::bytes) else {
            return;
        };
        // The cpu lines lead the file; stop before the long intr line.
        for line in lines(data).take_while(|l| l.starts_with(b"cpu")) {
            match parse_stat_line(line) {
                Some((None, now)) => self.sample.usage = usage(&mut self.prev_all, now),
                Some((Some(i), now)) if i < self.prev_cores.len() => {
                    self.sample.cores[i] = usage(&mut self.prev_cores[i], now);
                }
                _ => {}
            }
        }
    }
}

impl Default for CpuSampler {
    fn default() -> Self {
        Self::new()
    }
}

/// Load since `prev` as a percentage, and moves `prev` on. With nothing to
/// compare against, or a counter that went backwards (across a suspend, or a
/// processor that was offline), the reading is 0 rather than a wrong one.
fn usage(prev: &mut Times, now: Times) -> f64 {
    let mut pct = 0.0;
    if prev.total != 0 && now.total > prev.total && now.idle >= prev.idle {
        let total = (now.total - prev.total) as f64;
        let idle = (now.idle - prev.idle) as f64;
        pct = ((1.0 - idle / total) * 100.0).clamp(0.0, 100.0);
    }
    *prev = now;
    pct
}

/// Parses one `cpu` line of `/proc/stat`: the processor number (`None` for
/// the whole-processor line) and its idle and total times. Idle includes
/// iowait. Total is user through steal; guest and guest_nice are left out
/// because the kernel already counts them in user and nice. `None` for a line
/// that isn't a cpu line or is too short to have an idle column.
fn parse_stat_line(line: &[u8]) -> Option<(Option<usize>, Times)> {
    let mut fields = line.split(|&c| c == b' ').filter(|f| !f.is_empty());
    let suffix = fields.next()?.strip_prefix(b"cpu")?;
    let core = if suffix.is_empty() {
        None
    } else if suffix.iter().all(u8::is_ascii_digit) {
        let n = usize::try_from(sysfs::parse_uint(suffix)?).ok()?;
        if n >= MAX_CORE {
            return None;
        }
        Some(n)
    } else {
        return None;
    };
    let mut times = Times::default();
    let mut columns = 0;
    // user nice system idle iowait irq softirq steal
    for (i, value) in fields.take(8).enumerate() {
        let v = sysfs::parse_uint(value).unwrap_or(0);
        times.total = times.total.saturating_add(v);
        if i == 3 || i == 4 {
            times.idle = times.idle.saturating_add(v);
        }
        columns += 1;
    }
    (columns >= 4).then_some((core, times))
}

#[cfg(test)]
mod tests {
    use super::*;

    const CPUINFO: &str = include_str!("../../tests/fixtures/cpuinfo");
    const STAT: &str = include_str!("../../tests/fixtures/proc_stat");

    #[test]
    fn parses_the_stat_line() {
        let (core, t) = parse_stat_line(b"cpu  2255 34 2290 22625563 6290 127 456 0 0 0").unwrap();
        assert_eq!(core, None);
        assert_eq!(t.idle, 22_625_563 + 6290);
        assert_eq!(t.total, 2255 + 34 + 2290 + 22_625_563 + 6290 + 127 + 456);

        for (line, want) in [
            ("cpu0 1 2 3 4 5", 0),
            ("cpu7 1 2 3 4 5", 7),
            ("cpu31 1 2 3 4", 31),
        ] {
            assert_eq!(
                parse_stat_line(line.as_bytes()).unwrap().0,
                Some(want),
                "{line}"
            );
        }
        for bad in [
            "cpu 1 2 3",
            "intr 1 2 3 4 5 6",
            "cpuX 1 2 3 4 5",
            "cpu1x 1 2 3 4",
            "cpu99999999999999999999999 1 2 3 4",
            "cpu8192 1 2 3 4",
            "",
        ] {
            assert_eq!(parse_stat_line(bad.as_bytes()), None, "{bad:?}");
        }
    }

    /// Guest time is already inside user time; counting it twice would
    /// overstate the load whenever a VM runs.
    #[test]
    fn guest_time_is_not_counted_twice() {
        let (_, t) = parse_stat_line(b"cpu0 100 0 0 100 0 0 0 0 50 0").unwrap();
        assert_eq!(t.total, 200);
    }

    #[test]
    fn absurd_counters_stay_in_range() {
        let mut prev = Times::default();
        for line in [
            "cpu 18446744073709551615 18446744073709551615 0 0 0",
            "cpu 1 1 1 18446744073709551615 18446744073709551615",
            "cpu 0 0 0 0 0",
            "cpu 5 0 0 1 0",
            "cpu 4 0 0 9 0",
        ] {
            let (_, now) = parse_stat_line(line.as_bytes()).unwrap();
            let u = usage(&mut prev, now);
            assert!((0.0..=100.0).contains(&u), "{line}: {u}");
        }
    }

    #[test]
    fn fixture_stat_has_a_line_per_core() {
        let parsed: Vec<_> = lines(STAT.as_bytes())
            .take_while(|l| l.starts_with(b"cpu"))
            .map(|l| parse_stat_line(l).unwrap())
            .collect();
        assert_eq!(parsed[0].0, None);
        assert_eq!(parsed.len(), 33); // the aggregate line, then cpu0..cpu31
        assert_eq!(parsed.last().unwrap().0, Some(31));
        let sum: u64 = parsed[1..].iter().map(|(_, t)| t.total).sum();
        // The aggregate line is the per-core lines added up, give or take the
        // jiffies that passed while the kernel was writing them.
        assert!(sum.abs_diff(parsed[0].1.total) < 1000);
    }

    #[test]
    fn parses_the_cpuinfo_fixture() {
        // i9-14900KF: 8 performance cores with two threads, 16 efficiency
        // cores with one.
        let info = parse_cpuinfo(CPUINFO);
        assert_eq!(info.model, "Intel(R) Core(TM) i9-14900KF");
        assert_eq!(info.logical, 32);
        assert_eq!(info.physical_cores, 24);
        assert_eq!(info.sockets, 1);
    }

    #[test]
    fn cpuinfo_without_topology() {
        let vm = "processor\t: 0\nmodel name\t: QEMU Virtual CPU\n\nprocessor\t: 1\n";
        let info = parse_cpuinfo(vm);
        assert_eq!((info.logical, info.physical_cores, info.sockets), (2, 2, 1));
        assert_eq!(info.model, "QEMU Virtual CPU");
        let empty = parse_cpuinfo("");
        assert_eq!(
            (empty.logical, empty.physical_cores, empty.sockets),
            (1, 1, 1)
        );
    }

    #[test]
    fn parses_cache_sizes() {
        for (raw, want) in [
            ("48K", Some(48 << 10)),
            ("2048K", Some(2 << 20)),
            ("36864K", Some(36 << 20)),
            ("1M", Some(1 << 20)),
            ("1G", Some(1 << 30)),
            ("", None),
            ("weird", None),
            ("K", None),
            ("12", None),
            ("-1K", None),
            ("99999999999999999999G", None),
        ] {
            assert_eq!(parse_cache_size(raw), want, "{raw:?}");
        }
    }

    #[test]
    fn reads_a_cache_tree() {
        let dir = tempfile::tempdir().unwrap();
        for (index, level, kind, size) in [
            ("index0", "1", "Data", "48K"),
            ("index1", "1", "Instruction", "32K"),
            ("index2", "2", "Unified", "2048K"),
            ("index3", "3", "Unified", "36864K"),
        ] {
            let d = dir.path().join(index);
            fs::create_dir(&d).unwrap();
            fs::write(d.join("level"), format!("{level}\n")).unwrap();
            fs::write(d.join("type"), format!("{kind}\n")).unwrap();
            fs::write(d.join("size"), format!("{size}\n")).unwrap();
        }
        fs::write(dir.path().join("uevent"), "").unwrap();
        let c = read_caches(dir.path());
        assert_eq!(c.l1d, Some(48 << 10));
        assert_eq!(c.l1i, Some(32 << 10));
        assert_eq!(c.l2, Some(2 << 20));
        assert_eq!(c.l3, Some(36 << 20));
        assert_eq!(read_caches(&dir.path().join("absent")), Caches::default());
    }

    #[test]
    fn finds_the_package_sensor() {
        let dir = tempfile::tempdir().unwrap();
        for (hwmon, name, input) in [
            ("hwmon0", "nvme", true),
            ("hwmon1", "coretemp", false), // a coretemp with no temp1: skipped
            ("hwmon2", "k10temp", true),
        ] {
            let d = dir.path().join(hwmon);
            fs::create_dir(&d).unwrap();
            fs::write(d.join("name"), format!("{name}\n")).unwrap();
            if input {
                fs::write(d.join("temp1_input"), "45000\n").unwrap();
            }
        }
        assert_eq!(
            find_temperature(dir.path()),
            Some(dir.path().join("hwmon2/temp1_input"))
        );
        assert_eq!(find_temperature(&dir.path().join("absent")), None);
    }

    /// The sampler over recorded files: two /proc/stat readings a known
    /// distance apart give the load between them.
    #[test]
    fn sampler_turns_counters_into_load() {
        let dir = tempfile::tempdir().unwrap();
        let stat = dir.path().join("stat");
        fs::write(&stat, "cpu  100 0 100 800 0 0 0 0 0 0\ncpu0 50 0 50 400 0 0 0 0 0 0\ncpu1 50 0 50 400 0 0 0 0 0 0\nintr 1\n").unwrap();
        let cpu_dir = dir.path().join("cpu");
        fs::create_dir_all(cpu_dir.join("cpu1/cpufreq")).unwrap();
        fs::write(cpu_dir.join("cpu1/cpufreq/scaling_cur_freq"), "4800000\n").unwrap();
        let temp = dir.path().join("temp1_input");
        fs::write(&temp, "51500\n").unwrap();

        let mut s = CpuSampler::open(&stat, &cpu_dir, Some(&temp));
        // cpu0 50% busy, cpu1 fully idle over the next 200 jiffies each.
        fs::write(&stat, "cpu  200 0 100 1100 0 0 0 0 0 0\ncpu0 100 0 100 500 0 0 0 0 0 0\ncpu1 50 0 50 600 0 0 0 0 0 0\n").unwrap();
        let got = s.sample().clone();
        assert_eq!(got.cores, [50.0, 0.0]);
        assert_eq!(got.usage, 25.0);
        assert_eq!(got.frequency_mhz, Some(4800.0));
        assert_eq!(got.temperature, Some(51.5));
    }

    // Live tests: invariants only, CI's container has no cpufreq or sensors.

    /// Processor counts agree with what sysfs says is online.
    #[test]
    fn live_topology_agrees_with_sysfs() {
        let Some(online) = sysfs::read_string(format!("{CPU_DIR}/online")) else {
            return;
        };
        let online: usize = online
            .split(',')
            .map(|r| match r.split_once('-') {
                Some((a, b)) => b.parse::<usize>().unwrap() - a.parse::<usize>().unwrap() + 1,
                None => 1,
            })
            .sum();
        let info = info();
        assert_eq!(info.logical, online);
        assert!(info.physical_cores <= info.logical);
        assert!(info.sockets <= info.physical_cores);
        assert!(!info.model.is_empty() || !std::env::consts::ARCH.starts_with("x86"));
    }

    #[test]
    fn live_load_is_in_range() {
        let mut s = CpuSampler::new();
        let logical = info().logical;
        std::thread::sleep(std::time::Duration::from_millis(50));
        let got = s.sample();
        assert!(got.cores.len() >= logical);
        assert!((0.0..=100.0).contains(&got.usage));
        assert!(got.cores.iter().all(|u| (0.0..=100.0).contains(u)));
        if let Some(mhz) = got.frequency_mhz {
            assert!(mhz > 0.0);
        }
    }
}
