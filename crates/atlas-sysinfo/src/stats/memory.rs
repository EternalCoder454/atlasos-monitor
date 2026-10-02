//! Memory and swap from `/proc/meminfo`.

use super::lines;
use crate::sysfs::{self, HeldFile};

/// One reading of memory and swap, in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct Memory {
    pub total: u64,
    /// Total minus available, not minus free: page cache is memory the
    /// kernel hands back the moment something asks, and counting it as used
    /// would show a healthy machine as nearly full.
    pub used: u64,
    /// What a new program could get without swapping (`MemAvailable`).
    pub available: u64,
    /// Memory nobody has touched (`MemFree`).
    pub free: u64,
    /// Page cache, reclaimable slab and buffers.
    pub cached: u64,
    pub swap_total: u64,
    pub swap_used: u64,
}

impl Memory {
    /// RAM in use, 0..=100.
    pub fn usage_percent(&self) -> f64 {
        percent(self.used, self.total)
    }

    /// Swap in use, 0..=100; 0 without swap.
    pub fn swap_percent(&self) -> f64 {
        percent(self.swap_used, self.swap_total)
    }
}

fn percent(part: u64, whole: u64) -> f64 {
    if whole == 0 {
        return 0.0;
    }
    part as f64 / whole as f64 * 100.0
}

/// Parses `/proc/meminfo`. `None` without a `MemTotal` line.
pub fn parse_meminfo(data: &[u8]) -> Option<Memory> {
    let (mut total, mut available, mut free, mut cached) = (None, 0, 0, 0);
    let (mut sreclaimable, mut buffers, mut swap_total, mut swap_free) = (0, 0, 0, 0);
    for line in lines(data) {
        let Some((key, kib)) = meminfo_line(line) else {
            continue;
        };
        let bytes = kib.saturating_mul(1024);
        match key {
            b"MemTotal" => total = Some(bytes),
            b"MemAvailable" => available = bytes,
            b"MemFree" => free = bytes,
            b"Cached" => cached = bytes,
            b"SReclaimable" => sreclaimable = bytes,
            b"Buffers" => buffers = bytes,
            b"SwapTotal" => swap_total = bytes,
            b"SwapFree" => swap_free = bytes,
            _ => {}
        }
    }
    let total = total?;
    // A kernel without MemAvailable is older than Fedora has shipped for a
    // decade, but a figure above the total would be nonsense either way.
    let available = available.min(total);
    Some(Memory {
        total,
        used: total - available,
        available,
        free: free.min(total),
        cached: cached.saturating_add(sreclaimable).saturating_add(buffers),
        swap_total,
        swap_used: swap_total.saturating_sub(swap_free),
    })
}

/// Splits one `Key:   1234 kB` line into its key and number.
fn meminfo_line(line: &[u8]) -> Option<(&[u8], u64)> {
    let colon = line.iter().position(|&c| c == b':')?;
    let value = sysfs::field(&line[colon + 1..], 0)?;
    Some((&line[..colon], sysfs::parse_uint(value)?))
}

/// Samples `/proc/meminfo` from a held file.
#[derive(Debug)]
pub struct MemorySampler {
    meminfo: Option<HeldFile>,
}

impl MemorySampler {
    pub fn new() -> Self {
        Self {
            meminfo: HeldFile::with_capacity("/proc/meminfo", 4096),
        }
    }

    /// Reads memory and swap. `None` if `/proc/meminfo` can't be read.
    pub fn sample(&mut self) -> Option<Memory> {
        parse_meminfo(self.meminfo.as_mut()?.bytes()?)
    }
}

impl Default for MemorySampler {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MEMINFO: &[u8] = include_bytes!("../../tests/fixtures/meminfo");

    #[test]
    fn parses_the_fixture() {
        let m = parse_meminfo(MEMINFO).unwrap();
        assert_eq!(m.total, 32_672_072 * 1024);
        assert_eq!(m.available, 22_135_284 * 1024);
        assert_eq!(m.used, m.total - m.available);
        assert_eq!(m.free, 9_913_060 * 1024);
        assert_eq!(m.cached, (11_949_548 + 915_924 + 576) * 1024);
        assert_eq!(m.swap_total, 25_165_816 * 1024);
        assert_eq!(m.swap_used, (25_165_816 - 14_068_884) * 1024);
    }

    #[test]
    fn meminfo_lines() {
        for (line, want) in [
            (
                "MemTotal:       32673528 kB",
                Some((&b"MemTotal"[..], 32_673_528)),
            ),
            ("MemFree: 1 kB", Some((b"MemFree", 1))),
            ("HugePages_Total:       0", Some((b"HugePages_Total", 0))),
            ("not a meminfo line", None),
            ("Empty:", None),
            ("Bad: x kB", None),
        ] {
            assert_eq!(meminfo_line(line.as_bytes()), want, "{line:?}");
        }
    }

    #[test]
    fn broken_files() {
        assert_eq!(parse_meminfo(b""), None);
        assert_eq!(parse_meminfo(b"MemFree: 5 kB\n"), None);
        // Available above total, swap free above swap total: clamped, not
        // wrapped round to a huge figure.
        let m = parse_meminfo(
            b"MemTotal: 10 kB\nMemAvailable: 20 kB\nSwapTotal: 1 kB\nSwapFree: 2 kB\n",
        )
        .unwrap();
        assert_eq!(m.used, 0);
        assert_eq!(m.swap_used, 0);
        assert_eq!(m.usage_percent(), 0.0);
        let huge = parse_meminfo(b"MemTotal: 18446744073709551615 kB\n").unwrap();
        assert_eq!(huge.total, u64::MAX);
    }

    #[test]
    fn percentages() {
        let m = Memory {
            total: 200,
            used: 50,
            swap_total: 0,
            ..Memory::default()
        };
        assert_eq!(m.usage_percent(), 25.0);
        assert_eq!(m.swap_percent(), 0.0);
        assert_eq!(Memory::default().usage_percent(), 0.0);
    }

    /// The live figures hang together.
    #[test]
    fn live_invariants() {
        let m = MemorySampler::new().sample().expect("/proc/meminfo");
        assert!(m.total > 0);
        assert!(m.used <= m.total);
        assert!(m.available <= m.total);
        assert!(m.swap_used <= m.swap_total);
        assert_eq!(m.total % 1024, 0, "kB were not turned into bytes");
        assert!((0.0..=100.0).contains(&m.usage_percent()));
        assert!((0.0..=100.0).contains(&m.swap_percent()));
    }
}
