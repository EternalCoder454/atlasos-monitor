//! One process in full, for the Details panel, and the one-off lookups the
//! Apps page's actions need.
//!
//! Read on request, not by the scan: several of these figures cost a file
//! each, and nobody needs them for every process every second. Run on a
//! short-lived thread, never the GUI thread (DESIGN.md, Threading rule).

use std::ffi::CStr;
use std::fs;
use std::os::unix::fs::MetadataExt;
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use super::parse;

/// Everything the Details panel shows. A figure that couldn't be read is
/// `None`: another user's `smaps_rollup` and `fd/` are closed to us.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Info {
    pub pid: u32,
    /// Start time in clock ticks since boot; see [`super::Proc::start_time`].
    pub start_time: u64,
    pub name: String,
    /// The scheduler's word for what it is doing: "running", "sleeping",
    /// "stopped", "zombie", ...
    pub state: String,
    pub parent: u32,
    pub parent_name: Option<String>,
    pub executable: Option<PathBuf>,
    /// The arguments as given, the program first. Empty for a kernel thread
    /// or a zombie.
    pub command_line: Vec<String>,
    pub uid: Option<u32>,
    /// The owner's user name, or the uid as text if it has none.
    pub user: Option<String>,
    pub started: Option<SystemTime>,
    pub threads: Option<u32>,
    pub nice: i32,
    pub cpu_time: Duration,
    /// Resident bytes. From the same page-table walk as `pss` when that
    /// could be read, so the three agree. `None` for a kernel thread or a
    /// zombie, which have no memory of their own to show.
    pub memory: Option<u64>,
    pub swap: Option<u64>,
    /// Resident bytes with shared pages divided among their users.
    pub pss: Option<u64>,
    /// Bytes that are this process's alone.
    pub private: Option<u64>,
    pub open_files: Option<usize>,
    pub unit: Option<String>,
}

/// Reads what `/proc` knows about one process. `None` if it has gone (or,
/// never seen, its stat line can't be parsed); anything else that can't be
/// read is left out.
///
/// The reads aren't one atomic snapshot, so the start time is checked again
/// at the end: a process that exited part way, its pid taken by another,
/// would otherwise give a panel of two processes' figures, and a start time
/// that [`super::act`] would accept for the newcomer.
pub fn details(pid: u32) -> Option<Info> {
    let dir = PathBuf::from(format!("/proc/{pid}"));
    let stat = fs::read(dir.join("stat")).ok()?;
    let st = parse::parse_stat(&stat)?;
    // Bytes, not a string: the Name line is the raw comm, not always UTF-8.
    let status = fs::read(dir.join("status")).ok()?;
    let hz = rustix::param::clock_ticks_per_second() as f64;

    let mut info = Info {
        pid,
        start_time: st.start_time,
        name: String::from_utf8_lossy(st.name).into_owned(),
        state: String::new(),
        parent: st.ppid,
        parent_name: None,
        executable: fs::read_link(dir.join("exe")).ok(),
        command_line: fs::read(dir.join("cmdline"))
            .map(|b| split_command_line(&b))
            .unwrap_or_default(),
        uid: None,
        user: None,
        started: boot_time().map(|boot| boot + Duration::from_secs_f64(st.start_time as f64 / hz)),
        threads: None,
        nice: st.nice,
        cpu_time: Duration::from_secs_f64(st.jiffies as f64 / hz),
        memory: None,
        swap: None,
        pss: None,
        private: None,
        open_files: fs::read_dir(dir.join("fd")).ok().map(Iterator::count),
        unit: fs::read(dir.join("cgroup")).ok().and_then(|b| {
            parse::unit_from_cgroup(&b).map(|u| String::from_utf8_lossy(u).into_owned())
        }),
    };
    parse_status(&String::from_utf8_lossy(&status), &mut info);
    info.user = info
        .uid
        .map(|uid| user_name(uid).unwrap_or_else(|| uid.to_string()));
    if st.ppid > 0 {
        info.parent_name = fs::read(format!("/proc/{}/comm", st.ppid))
            .ok()
            .map(|b| String::from_utf8_lossy(b.trim_ascii_end()).into_owned());
    }
    if let Ok(rollup) = fs::read_to_string(dir.join("smaps_rollup")) {
        // status's VmRSS is the kernel's running counter, batched per CPU,
        // and can sit below a PSS read an instant later.
        if let Some(rss) = kib_value(&rollup, "Rss") {
            info.memory = Some(rss);
        }
        info.pss = kib_value(&rollup, "Pss");
        info.private = kib_value(&rollup, "Private_Clean")
            .zip(kib_value(&rollup, "Private_Dirty"))
            .map(|(clean, dirty)| clean + dirty);
    }
    (start_time(pid) == Some(st.start_time)).then_some(info)
}

/// The full path of a process's program as the host sees it, for Open File
/// Location. `None` if it can't be read (another user's process, a kernel
/// thread), or if no path on the host leads to it.
///
/// A process names its program as it sees it, from inside its own root,
/// which for a sandbox or a container isn't the host's: a Flatpak's is
/// `/app/discord/Discord`, and a container's `/usr/bin/bash` is the
/// container's bash, where the host has a bash of its own. So a path is
/// kept only if the host's file there is the program itself, the file the
/// kernel runs (same device and inode): the path as given, or for a Flatpak
/// its host path in the app's or runtime's deployed files. A rootless
/// container's files are mounted only in podman's namespace, so its own
/// programs have none; one run from the home folder a toolbox shares does.
/// A program replaced on disk since it started has none either.
pub fn executable(pid: u32) -> Option<PathBuf> {
    let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let running = fs::metadata(format!("/proc/{pid}/exe")).ok()?;
    let is_it = |p: &PathBuf| {
        fs::metadata(p).is_ok_and(|m| m.dev() == running.dev() && m.ino() == running.ino())
    };
    let host = crate::apps::flatpak::Instance::of(pid).and_then(|i| i.host_path(&exe));
    host.into_iter().chain([exe]).find(is_it)
}

/// A process's start time in clock ticks since boot, `None` if it has gone.
/// With the pid it tells one process from a later one given the same pid.
pub fn start_time(pid: u32) -> Option<u64> {
    let stat = fs::read(format!("/proc/{pid}/stat")).ok()?;
    Some(parse::parse_stat(&stat)?.start_time)
}

/// Fills in the fields of `/proc/<pid>/status` the panel shows.
fn parse_status(text: &str, info: &mut Info) {
    for line in text.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let value = value.trim();
        match key {
            // "S (sleeping)": the word in brackets is the readable half.
            "State" => {
                if let Some((_, rest)) = value.split_once('(') {
                    info.state = rest.trim_end_matches(')').to_owned();
                }
            }
            // Real, effective, saved, filesystem: the real one owns it.
            "Uid" => info.uid = value.split_whitespace().next().and_then(|u| u.parse().ok()),
            "Threads" => info.threads = value.parse().ok(),
            "VmRSS" => info.memory = kib(value),
            "VmSwap" => info.swap = kib(value),
            _ => {}
        }
    }
}

/// The value of the line `key:   1234 kB`, in bytes. The key has to start the
/// line: `Pss` is also the end of `SwapPss`.
fn kib_value(text: &str, key: &str) -> Option<u64> {
    text.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix(':'))
        .and_then(kib)
}

/// `"   1234 kB"` to bytes.
fn kib(field: &str) -> Option<u64> {
    let n: u64 = field.trim().strip_suffix("kB")?.trim().parse().ok()?;
    n.checked_mul(1024)
}

/// Splits the NUL-separated `cmdline` into arguments. A program that rewrites
/// its own arguments sometimes leaves one string with spaces and no NULs;
/// that is kept as it is rather than guessed at. One that blanked its
/// argument area leaves a run of NULs at the end; those are no arguments.
fn split_command_line(b: &[u8]) -> Vec<String> {
    let Some(last) = b.iter().rposition(|&c| c != 0) else {
        return Vec::new();
    };
    let trimmed = &b[..=last];
    trimmed
        .split(|&c| c == 0)
        .map(|arg| String::from_utf8_lossy(arg).into_owned())
        .collect()
}

/// When the machine started, from the `btime` line of `/proc/stat`.
fn boot_time() -> Option<SystemTime> {
    let stat = fs::read_to_string("/proc/stat").ok()?;
    let secs = stat
        .lines()
        .find_map(|l| l.strip_prefix("btime "))?
        .trim()
        .parse()
        .ok()?;
    Some(SystemTime::UNIX_EPOCH + Duration::from_secs(secs))
}

/// The user name for a uid, through NSS: on Fedora Atomic most system users
/// live in `/usr/lib/passwd` (nss-altfiles) and a service's dynamic user in
/// systemd's userdb, so reading `/etc/passwd` alone would miss them.
fn user_name(uid: u32) -> Option<String> {
    let mut buf = vec![0 as libc::c_char; 1024];
    loop {
        let mut pwd = std::mem::MaybeUninit::<libc::passwd>::uninit();
        let mut result = std::ptr::null_mut();
        // SAFETY: every pointer is valid for the call and `buf.len()` is the
        // buffer's real size; on success `result` points at `pwd`, whose
        // strings live in `buf`, and both outlive the read below.
        let rc = unsafe {
            libc::getpwuid_r(
                uid,
                pwd.as_mut_ptr(),
                buf.as_mut_ptr(),
                buf.len(),
                &mut result,
            )
        };
        if rc == libc::ERANGE && buf.len() < 1 << 20 {
            buf.resize(buf.len() * 2, 0);
            continue;
        }
        if rc != 0 || result.is_null() {
            return None;
        }
        // SAFETY: as above; pw_name is a NUL-terminated string in `buf`.
        let name = unsafe { CStr::from_ptr((*result).pw_name) };
        return Some(name.to_string_lossy().into_owned());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn this_process() {
        let me = std::process::id();
        let info = details(me).expect("our own details");
        assert_eq!(info.pid, me);
        assert_eq!(
            info.name,
            fs::read_to_string("/proc/self/comm").unwrap().trim_end()
        );
        assert_eq!(info.parent, std::os::unix::process::parent_id());
        assert!(info.threads.is_some_and(|n| n >= 1));
        assert!(!info.command_line.is_empty());
        assert_eq!(info.executable, std::env::current_exe().ok());
        assert_eq!(executable(me), std::env::current_exe().ok());
        assert_eq!(info.uid, Some(rustix::process::getuid().as_raw()));
        assert!(info.user.as_deref().is_some_and(|u| !u.is_empty()));
        assert!(
            !info.state.is_empty() && !info.state.contains(')'),
            "{:?}",
            info.state
        );
        let memory = info.memory.expect("our own memory");
        assert!(memory > 0);
        assert!(info.open_files.is_some_and(|n| n >= 1));
        assert_eq!(Some(info.start_time), start_time(me));
        let started = info.started.expect("a start time");
        let now = SystemTime::now();
        assert!(
            started <= now + Duration::from_secs(2),
            "started in the future"
        );
        assert!(now.duration_since(started).unwrap_or_default() < Duration::from_secs(24 * 3600));
        // Some sandboxes have no smaps_rollup.
        if let (Some(pss), Some(private)) = (info.pss, info.private) {
            assert!(pss <= memory, "PSS {pss} above RSS {memory}");
            assert!(private <= memory);
        }
    }

    #[test]
    fn root_is_root() {
        // A bare sandbox can have no user database at all.
        let known = ["/etc/passwd", "/usr/lib/passwd"]
            .iter()
            .any(|f| fs::read_to_string(f).is_ok_and(|t| t.starts_with("root:")));
        if !known {
            eprintln!("no passwd entry for root here; skipping");
            return;
        }
        assert_eq!(user_name(0).as_deref(), Some("root"));
    }

    /// The process has gone: not an error dressed as an empty panel.
    #[test]
    fn a_process_that_is_gone() {
        let mut child = std::process::Command::new("true").spawn().unwrap();
        let pid = child.id();
        child.wait().unwrap();
        assert_eq!(details(pid), None);
        assert_eq!(start_time(pid), None);
        assert_eq!(executable(pid), None);
    }

    /// A program is found at its path while that path holds it, and not
    /// once the file there is another, as after an update replaced it, or
    /// as a container's `/usr/bin/bash` is to the host's.
    #[test]
    fn a_program_is_found_only_where_it_is() {
        let dir = tempfile::tempdir().unwrap();
        let copy = dir.path().join("atlas-sleep");
        fs::copy("/usr/bin/sleep", &copy).unwrap();
        // Another test's fork can hold the copy open for writing a moment.
        let mut child = None;
        for _ in 0..50 {
            match std::process::Command::new(&copy).arg("30").spawn() {
                Ok(c) => {
                    child = Some(c);
                    break;
                }
                Err(e) if e.raw_os_error() == Some(libc::ETXTBSY) => {
                    std::thread::sleep(Duration::from_millis(10));
                }
                Err(e) if e.kind() == std::io::ErrorKind::PermissionDenied => {
                    eprintln!("can't run programs from {}; skipping", dir.path().display());
                    return;
                }
                Err(e) => panic!("{e}"),
            }
        }
        let mut child = child.expect("the copy stayed busy");
        let pid = child.id();
        let path = fs::canonicalize(&copy).unwrap();
        assert_eq!(executable(pid), Some(path));

        fs::remove_file(&copy).unwrap();
        assert_eq!(executable(pid), None, "gone from disk");
        fs::copy("/usr/bin/sleep", &copy).unwrap();
        assert_eq!(executable(pid), None, "a different file at its path");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn command_lines() {
        assert_eq!(split_command_line(b""), Vec::<String>::new());
        assert_eq!(split_command_line(b"\0"), Vec::<String>::new());
        assert_eq!(split_command_line(b"\0\0"), Vec::<String>::new());
        assert_eq!(split_command_line(b"foo\0\0\0"), ["foo"]);
        assert_eq!(split_command_line(b"ls\0-l\0/tmp\0"), ["ls", "-l", "/tmp"]);
        assert_eq!(split_command_line(b"a\0\0b\0"), ["a", "", "b"]);
        // Rewritten by the program: one string, spaces kept.
        assert_eq!(
            split_command_line(b"postgres: writer process   "),
            ["postgres: writer process   "]
        );
    }

    #[test]
    fn kib_values() {
        let rollup =
            "Rss:     2048 kB\nPss:      1024 kB\nSwapPss:   512 kB\nPrivate_Clean: 4 kB\n";
        assert_eq!(kib_value(rollup, "Rss"), Some(2048 * 1024));
        assert_eq!(kib_value(rollup, "Pss"), Some(1024 * 1024));
        assert_eq!(kib_value(rollup, "SwapPss"), Some(512 * 1024));
        assert_eq!(kib_value(rollup, "Private_Dirty"), None);
        assert_eq!(kib_value("Pss: x kB\n", "Pss"), None);
        let swap_only = "SwapPss: 512 kB\n";
        assert_eq!(
            kib_value(swap_only, "Pss"),
            None,
            "matched the end of SwapPss"
        );
    }

    #[test]
    fn status_fields() {
        let mut info = details(std::process::id()).unwrap();
        parse_status(
            "Name:\tbash\nState:\tT (stopped)\nUid:\t1000\t1000\t1000\t1000\nThreads:\t7\nVmRSS:\t  5000 kB\nVmSwap:\t 12 kB\n",
            &mut info,
        );
        assert_eq!(info.state, "stopped");
        assert_eq!(info.uid, Some(1000));
        assert_eq!(info.threads, Some(7));
        assert_eq!(info.memory, Some(5000 * 1024));
        assert_eq!(info.swap, Some(12 * 1024));
        // A zombie's status has no Vm lines: unknown, not zero.
        let mut zombie = details(std::process::id()).unwrap();
        (zombie.memory, zombie.swap) = (None, None);
        parse_status("State:\tZ (zombie)\nThreads:\t1\n", &mut zombie);
        assert_eq!((zombie.memory, zombie.swap), (None, None));
    }
}
