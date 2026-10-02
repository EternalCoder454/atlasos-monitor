//! Small kernel files held open and re-read without reopening them.
//!
//! Atlas samples the same handful of `/proc` and `/sys` files every second:
//! per-core frequencies, a temperature, `/proc/stat`, `/proc/meminfo`, the
//! GPU's counters. Read with `fs::read` that is four syscalls each (open, read,
//! read, close) plus an allocation; held open and re-read with `pread` from
//! offset zero it is one. In the Go version those opens were a sixth of the
//! app's CPU time.
//!
//! procfs and sysfs regenerate a file's contents on every read, so `pread` at
//! offset zero is how these files are meant to be polled.
//!
//! A [`HeldFile`] belongs to one sampler and is not shared between threads.
//! Readers keep an `Option<HeldFile>`: an attribute that is absent on this
//! machine is `None`, and the reading is simply missing from the page.
//!
//! The byte parsers here ([`parse_uint`], [`field`]) are the ones every reader
//! uses on its hot path: they work on the buffer in place and never allocate.

use std::fs::File;
use std::io;
use std::os::unix::fs::FileExt;
use std::path::Path;

/// Buffer size for a single attribute: a number or a short string. Larger
/// files say how much they need with [`HeldFile::with_capacity`].
const SMALL_VALUE: usize = 256;

/// A kernel file held open for repeated reads.
#[derive(Debug)]
pub struct HeldFile {
    file: File,
    buf: Vec<u8>,
}

impl HeldFile {
    /// Holds `path` open for reading a single small value. `None` if it can't
    /// be opened.
    pub fn open(path: impl AsRef<Path>) -> Option<Self> {
        Self::with_capacity(path, SMALL_VALUE)
    }

    /// Holds `path` open with a buffer of at least `size` bytes. The buffer
    /// still grows if the file outgrows it, so `size` only saves the re-reads
    /// that growing costs.
    pub fn with_capacity(path: impl AsRef<Path>, size: usize) -> Option<Self> {
        let file = File::open(path).ok()?;
        Some(Self {
            file,
            buf: vec![0; size.max(SMALL_VALUE)],
        })
    }

    /// Holds open the first of `paths` that exists, for attributes whose
    /// location differs between drivers.
    pub fn open_first<P: AsRef<Path>>(paths: impl IntoIterator<Item = P>) -> Option<Self> {
        paths.into_iter().find_map(Self::open)
    }

    /// Re-reads the file. The slice borrows the held buffer, so it is valid
    /// until the next read. `None` on an error or an empty file.
    pub fn bytes(&mut self) -> Option<&[u8]> {
        loop {
            let n = match self.file.read_at(&mut self.buf, 0) {
                Ok(n) => n,
                Err(e) if e.kind() == io::ErrorKind::Interrupted => continue,
                Err(_) => return None,
            };
            // A read that fills the buffer exactly may have been cut short:
            // grow and read again, so a file that outgrows its buffer never
            // comes back truncated. A half-read /proc/stat would quietly
            // corrupt every figure.
            if n == self.buf.len() {
                self.buf.resize(n * 2, 0);
                continue;
            }
            if n == 0 {
                return None;
            }
            return Some(&self.buf[..n]);
        }
    }

    /// Re-reads the file as an unsigned integer. Surrounding whitespace is
    /// trimmed first: some drivers write `" 42"`, and without the trim that
    /// figure would silently vanish from the page.
    pub fn uint(&mut self) -> Option<u64> {
        parse_uint(self.bytes()?.trim_ascii())
    }
}

/// Reads the leading ASCII digits of `b`. `None` if there are none, or if the
/// number doesn't fit in a `u64` (no kernel counter is that large, so it is
/// a broken line, not a reading).
pub fn parse_uint(b: &[u8]) -> Option<u64> {
    let digits = b.iter().take_while(|c| c.is_ascii_digit()).count();
    if digits == 0 {
        return None;
    }
    b[..digits].iter().try_fold(0u64, |v, &c| {
        v.checked_mul(10)?.checked_add(u64::from(c - b'0'))
    })
}

/// Returns the `idx`-th field of `line`, fields being separated by runs of
/// spaces or tabs, as in `/proc/diskstats` or `/proc/net/dev`.
pub fn field(line: &[u8], idx: usize) -> Option<&[u8]> {
    line.split(|&c| c == b' ' || c == b'\t')
        .filter(|f| !f.is_empty())
        .nth(idx)
}

/// Reads a value from a file not worth holding open: discovery and static
/// information, read once.
pub fn read_uint(path: impl AsRef<Path>) -> Option<u64> {
    parse_uint(std::fs::read(path).ok()?.trim_ascii())
}

/// Reads a one-shot string value, trimmed. `None` if the file can't be read.
pub fn read_string(path: impl AsRef<Path>) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    Some(text.trim().to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn write(dir: &Path, name: &str, content: &str) -> std::path::PathBuf {
        let p = dir.join(name);
        fs::write(&p, content).unwrap();
        p
    }

    #[test]
    fn reads_the_same_descriptor_repeatedly() {
        let dir = tempfile::tempdir().unwrap();
        let mut f = HeldFile::open(write(dir.path(), "value", "3456789\n")).unwrap();
        for _ in 0..3 {
            assert_eq!(f.uint(), Some(3_456_789));
        }
        assert_eq!(f.bytes().unwrap().trim_ascii(), b"3456789");
    }

    /// What every sampler depends on: kernel files change between reads, so a
    /// held descriptor must see the new contents.
    #[test]
    fn rereads_changed_content() {
        let dir = tempfile::tempdir().unwrap();
        let path = write(dir.path(), "counter", "1\n");
        let mut f = HeldFile::open(&path).unwrap();
        assert_eq!(f.uint(), Some(1));
        // Rewritten in place (truncate + write), the way the kernel's
        // regenerated contents look to a held descriptor.
        fs::write(&path, "42\n").unwrap();
        assert_eq!(f.uint(), Some(42), "the descriptor is not re-reading");
    }

    #[test]
    fn grows_for_large_files() {
        let dir = tempfile::tempdir().unwrap();
        let big = "0123456789abcdef".repeat(4096); // 64 KiB, far past the default
        let mut f = HeldFile::open(write(dir.path(), "big", &big)).unwrap();
        assert_eq!(f.bytes().unwrap(), big.as_bytes());
    }

    /// Content exactly the buffer's size looks the same as a truncated read.
    #[test]
    fn exact_buffer_boundary() {
        let dir = tempfile::tempdir().unwrap();
        let exact = "x".repeat(SMALL_VALUE);
        let mut f = HeldFile::open(write(dir.path(), "exact", &exact)).unwrap();
        assert_eq!(f.bytes().unwrap().len(), SMALL_VALUE);
    }

    #[test]
    fn missing_and_empty_files() {
        let dir = tempfile::tempdir().unwrap();
        assert!(HeldFile::open(dir.path().join("absent")).is_none());
        let mut empty = HeldFile::open(write(dir.path(), "empty", "")).unwrap();
        assert_eq!(empty.bytes(), None);
        assert_eq!(empty.uint(), None);
        assert_eq!(read_uint(dir.path().join("absent")), None);
        assert_eq!(read_string(dir.path().join("absent")), None);
    }

    #[test]
    fn open_first_takes_the_first_that_exists() {
        let dir = tempfile::tempdir().unwrap();
        let second = write(dir.path(), "second", "7\n");
        let mut f = HeldFile::open_first([dir.path().join("absent"), second]).unwrap();
        assert_eq!(f.uint(), Some(7));
        assert!(HeldFile::open_first([dir.path().join("no"), dir.path().join("nope")]).is_none());
    }

    #[test]
    fn parse_uint_reads_leading_digits() {
        for (input, want) in [
            ("0\n", 0),
            ("42", 42),
            ("18446744073709551615", u64::MAX),
            ("123 456", 123),
            ("99abc", 99),
        ] {
            assert_eq!(parse_uint(input.as_bytes()), Some(want), "{input:?}");
        }
        for input in [
            "",
            "abc",
            "\n",
            " 5",
            "18446744073709551616",
            "99999999999999999999999",
        ] {
            assert_eq!(parse_uint(input.as_bytes()), None, "{input:?}");
        }
    }

    /// `uint` and `read_uint` share one contract: whitespace around the digits
    /// is fine, anything else is no value.
    #[test]
    fn uint_trims_whitespace() {
        let dir = tempfile::tempdir().unwrap();
        for (content, want) in [
            ("42", Some(42)),
            ("42\n", Some(42)),
            (" 42", Some(42)),
            ("  42  \n", Some(42)),
            ("\t42\r\n", Some(42)),
            ("abc", None),
            (" abc ", None),
            ("\n", None),
        ] {
            let path = write(dir.path(), "attr", content);
            let mut f = HeldFile::with_capacity(&path, 8).unwrap();
            assert_eq!(f.uint(), want, "uint of {content:?}");
            assert_eq!(read_uint(&path), want, "read_uint of {content:?}");
        }
    }

    #[test]
    fn field_splits_on_runs_of_blanks() {
        let line = b"   259       0 nvme0n1 123 45 67890 1234 56 7 89012 345";
        for (idx, want) in [
            (0, Some(&b"259"[..])),
            (1, Some(b"0")),
            (2, Some(b"nvme0n1")),
            (5, Some(b"67890")),
            (9, Some(b"89012")),
            (99, None),
        ] {
            assert_eq!(field(line, idx), want, "field {idx}");
        }
        assert_eq!(field(b"a\tb", 1), Some(&b"b"[..]));
        assert_eq!(field(b"", 0), None);
    }

    /// The point of the module, checked on the live system: re-reading a held
    /// procfs file sees it change and keeps returning whole contents.
    #[test]
    fn live_proc_stat_rereads() {
        let Some(mut f) = HeldFile::open("/proc/stat") else {
            return; // no procfs in this sandbox
        };
        let first = f.bytes().unwrap().to_vec();
        assert!(first.starts_with(b"cpu "));
        let second = f.bytes().unwrap();
        assert!(second.starts_with(b"cpu "));
        assert!(second.ends_with(b"\n"), "a re-read came back cut short");
    }
}
