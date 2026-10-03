//! How much memory this process uses, for the Settings page.
//!
//! `/proc/self/smaps_rollup` gives RSS and PSS in one read. PSS splits shared
//! pages (Qt, Mesa, fonts) between the processes that map them, so it is the
//! fairer number to show; RSS counts every shared page in full.

use std::io;

/// Memory figures in bytes.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct SelfMemory {
    /// Resident set size: every page mapped and in RAM.
    pub rss: u64,
    /// Proportional set size: shared pages divided between their users.
    pub pss: u64,
}

/// Hands the allocator's free memory back to the system: what was freed
/// since a peak (a page closed, a long table dropped), which malloc keeps
/// for reuse. glibc's malloc only; elsewhere it does nothing.
pub fn trim() {
    #[cfg(target_env = "gnu")]
    // SAFETY: malloc_trim takes no pointers and may be called from any
    // thread.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Reads this process's memory use.
pub fn read() -> io::Result<SelfMemory> {
    let text = std::fs::read_to_string("/proc/self/smaps_rollup")?;
    parse_smaps_rollup(&text)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "no Rss/Pss in smaps_rollup"))
}

/// Parses `smaps_rollup`. `None` if either `Rss:` or `Pss:` is missing.
pub fn parse_smaps_rollup(text: &str) -> Option<SelfMemory> {
    let mut rss = None;
    let mut pss = None;
    for line in text.lines() {
        if let Some(v) = line.strip_prefix("Rss:") {
            rss = kib(v);
        } else if let Some(v) = line.strip_prefix("Pss:") {
            pss = kib(v);
        }
    }
    Some(SelfMemory {
        rss: rss?,
        pss: pss?,
    })
}

/// `"   123456 kB"` → bytes.
fn kib(field: &str) -> Option<u64> {
    let n: u64 = field.trim().strip_suffix("kB")?.trim().parse().ok()?;
    n.checked_mul(1024)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = include_str!("../tests/fixtures/smaps_rollup");

    #[test]
    fn parses_the_fixture() {
        let m = parse_smaps_rollup(FIXTURE).unwrap();
        assert_eq!(m.rss, 89_412 * 1024);
        assert_eq!(m.pss, 61_207 * 1024);
    }

    #[test]
    fn missing_fields_are_none() {
        assert_eq!(parse_smaps_rollup(""), None);
        assert_eq!(parse_smaps_rollup("Rss: 10 kB\n"), None);
        assert_eq!(parse_smaps_rollup("Rss: x kB\nPss: 1 kB\n"), None);
    }

    #[test]
    fn live_pss_is_within_rss() {
        // Some sandboxes (gVisor) have no smaps_rollup.
        let m = match read() {
            Err(e) if e.kind() == io::ErrorKind::NotFound => return,
            r => r.unwrap(),
        };
        assert!(m.rss > 0);
        assert!(m.pss <= m.rss);
    }
}
