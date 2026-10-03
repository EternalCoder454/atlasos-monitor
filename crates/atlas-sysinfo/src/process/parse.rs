//! Parsers for the `/proc/<pid>` files the scan reads. They work on the read
//! buffer in place and never allocate: the scan runs them thousands of times
//! a second.

use crate::sysfs;

/// `PF_KTHREAD` in the task flags: a kernel thread (kworker, ksoftirqd, irq
/// handlers), not a program.
const PF_KTHREAD: u64 = 0x0020_0000;

/// What one `/proc/<pid>/stat` line says. `name` borrows the line.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Stat<'a> {
    /// `comm`: the first 15 bytes of the program's name. Not always UTF-8.
    pub name: &'a [u8],
    /// The scheduler state letter: `R`, `S`, `D`, `T`, `Z`, ...
    pub state: u8,
    pub ppid: u32,
    pub kernel: bool,
    /// User plus system time, in clock ticks.
    pub jiffies: u64,
    /// Page faults, minor plus major: a process whose memory grew or came
    /// back from swap took some.
    pub faults: u64,
    pub nice: i32,
    /// When the process started, in clock ticks since boot. Together with
    /// the pid it names one process: pids are reused, this pair is not.
    pub start_time: u64,
    /// The rss field, in pages. Counted differently from `statm` (see
    /// [`parse_statm_resident`]), but it moves when that does.
    pub rss: u64,
}

/// Parses `/proc/<pid>/stat`. `None` if the line is cut short or malformed.
///
/// The name is in brackets and may itself contain spaces and brackets (a
/// program can call itself `") ("`), so the numbered fields start after the
/// *last* `)`. The flags word is on this line, which makes the kernel-thread
/// test free: telling them apart by an empty `cmdline` would be another open
/// per process per tick.
pub fn parse_stat(b: &[u8]) -> Option<Stat<'_>> {
    let open = b.iter().position(|&c| c == b'(')?;
    let close = b.iter().rposition(|&c| c == b')')?;
    if close < open {
        return None;
    }
    // Fields from the state (field 3) up to the rss (field 24).
    let mut f: [&[u8]; 22] = [b""; 22];
    let mut n = 0;
    for field in b
        .get(close + 2..)?
        .split(|&c| c == b' ')
        .filter(|f| !f.is_empty())
    {
        f[n] = field;
        n += 1;
        if n == f.len() {
            break;
        }
    }
    if n < f.len() {
        return None;
    }
    let num = |i: usize| sysfs::parse_uint(f[i]);
    Some(Stat {
        name: &b[open + 1..close],
        state: f[0][0],
        ppid: u32::try_from(num(1)?).ok()?,
        kernel: num(6)? & PF_KTHREAD != 0,
        jiffies: num(11)?.saturating_add(num(12)?),
        faults: num(7)?.saturating_add(num(9)?),
        nice: parse_int(f[16])?,
        start_time: num(19)?,
        rss: num(21)?,
    })
}

/// A signed decimal, as in the nice field.
fn parse_int(b: &[u8]) -> Option<i32> {
    let (negative, digits) = match b.strip_prefix(b"-") {
        Some(rest) => (true, rest),
        None => (false, b),
    };
    let v = i64::try_from(sysfs::parse_uint(digits)?).ok()?;
    i32::try_from(if negative { -v } else { v }).ok()
}

/// Resident pages from `/proc/<pid>/statm` (its second field). This is the
/// figure `ps` and `top` report; the rss field of `stat` is counted
/// differently and would make Atlas disagree with every other tool.
pub fn parse_statm_resident(b: &[u8]) -> Option<u64> {
    sysfs::parse_uint(sysfs::field(b, 1)?)
}

/// `read_bytes` and `write_bytes` from `/proc/<pid>/io`: what reached the
/// storage layer, not what went through `read(2)` (`rchar`), most of which
/// the page cache answers.
pub fn parse_io(b: &[u8]) -> Option<(u64, u64)> {
    let (mut read, mut write) = (None, None);
    for line in b.split(|&c| c == b'\n') {
        if let Some(v) = line.strip_prefix(b"read_bytes:") {
            read = sysfs::parse_uint(v.trim_ascii_start());
        } else if let Some(v) = line.strip_prefix(b"write_bytes:") {
            write = sysfs::parse_uint(v.trim_ascii_start());
        }
    }
    Some((read?, write?))
}

/// The systemd unit in `/proc/<pid>/cgroup`: the deepest path component that
/// is a scope or a service. Deepest unit, not deepest directory: a unit may
/// have cgroups of its own below it (a delegated scope, a Flatpak's sandbox)
/// and those belong to the unit above.
pub fn unit_from_cgroup(b: &[u8]) -> Option<&[u8]> {
    cgroup_path(b)?
        .rsplit(|&c| c == b'/')
        .find(|c| c.ends_with(b".scope") || c.ends_with(b".service"))
}

/// The podman container in `/proc/<pid>/cgroup`, by its 64-digit ID. Podman
/// (and toolbox and distrobox, which run on it) puts a container in
/// `libpod-<ID>.scope`, its monitor where it has a scope of its own in
/// `libpod-conmon-<ID>.scope`, and a Quadlet's container in
/// `libpod-payload-<ID>` inside the unit made for it. The outermost wins, so
/// a container's own units (distrobox `--init` runs systemd) and containers
/// it runs belong to it.
pub fn container_from_cgroup(b: &[u8]) -> Option<&[u8]> {
    cgroup_path(b)?.split(|&c| c == b'/').find_map(|c| {
        let rest = c.strip_prefix(b"libpod-")?;
        let id = match rest.strip_prefix(b"payload-") {
            Some(id) => id,
            None => {
                let rest = rest.strip_suffix(b".scope")?;
                rest.strip_prefix(b"conmon-").unwrap_or(rest)
            }
        };
        (id.len() == 64 && id.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f'))).then_some(id)
    })
}

/// The cgroup path in `/proc/<pid>/cgroup`: the unified hierarchy's `0::`
/// line where there is one; on a legacy or hybrid system, the systemd
/// controller's line.
fn cgroup_path(b: &[u8]) -> Option<&[u8]> {
    const SYSTEMD: &[u8] = b":name=systemd:";
    let mut path = None;
    for line in b.split(|&c| c == b'\n') {
        if let Some(rest) = line.strip_prefix(b"0::") {
            return Some(rest);
        }
        if let Some(i) = line.windows(SYSTEMD.len()).position(|w| w == SYSTEMD) {
            path = Some(&line[i + SYSTEMD.len()..]);
        }
    }
    path
}

#[cfg(test)]
mod tests {
    use super::*;

    const STAT: &[u8] = include_bytes!("../../tests/fixtures/pid_stat");
    const STAT_KTHREAD: &[u8] = include_bytes!("../../tests/fixtures/pid_stat_kthread");
    const IO: &[u8] = include_bytes!("../../tests/fixtures/pid_io");

    #[test]
    fn stat_of_a_program() {
        let s = parse_stat(STAT).unwrap();
        assert_eq!(s.name, b"cat");
        assert_eq!(s.state, b'R');
        assert_eq!(s.ppid, 2_275_422);
        assert!(!s.kernel);
        assert_eq!(s.jiffies, 1234 + 567);
        assert_eq!(s.faults, 93);
        assert_eq!(s.nice, 0);
        assert_eq!(s.start_time, 6_953_729);
        assert_eq!(s.rss, 427);
    }

    #[test]
    fn stat_of_a_kernel_thread() {
        let s = parse_stat(STAT_KTHREAD).unwrap();
        assert_eq!(s.name, b"kthreadd");
        assert!(s.kernel);
        assert_eq!(s.ppid, 0);
        assert_eq!(s.jiffies, 32);
        assert_eq!(s.start_time, 11);
        assert_eq!(s.rss, 0);
    }

    /// The reason the line is split at the last bracket.
    #[test]
    fn names_with_spaces_and_brackets() {
        let line = b"1234 (we ) ird) S 1 1234 1234 0 -1 4194560 100 0 2 0 5 6 0 0 20 -7 1 0 99 0 8";
        let s = parse_stat(line).unwrap();
        assert_eq!(s.name, b"we ) ird");
        assert_eq!(s.jiffies, 11);
        assert_eq!(s.nice, -7);
        assert_eq!(s.faults, 102);
        assert_eq!(s.start_time, 99);
        assert_eq!(s.rss, 8);
        let s = parse_stat(b"5 ()) S 1 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 7 0 0\n").unwrap();
        assert_eq!(s.name, b")");
        assert_eq!(s.start_time, 7);
    }

    #[test]
    fn broken_stat_lines() {
        for line in [
            &b""[..],
            b"1234",
            b"1234 (name",
            b"1234 name) S",
            b")(",
            b"1 (x)",
            b"1 (x) ",
            b"1 (x) S 1 2 3",
            // One field short of the rss.
            b"1 (x) S 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
            b"1 (x) S 0 0 0 0 0 0 0 0 0 0 x 0 0 0 0 0 0 0 0 0 0",
            b"1 (x) S 99999999999 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0 0",
        ] {
            assert_eq!(
                parse_stat(line),
                None,
                "{:?}",
                String::from_utf8_lossy(line)
            );
        }
    }

    /// Every prefix of a real line either parses or is rejected; none panics.
    #[test]
    fn stat_prefixes_never_panic() {
        for end in 0..=STAT.len() {
            let _ = parse_stat(&STAT[..end]);
        }
        for end in 0..=STAT_KTHREAD.len() {
            let _ = parse_stat(&STAT_KTHREAD[..end]);
        }
    }

    #[test]
    fn signed_numbers() {
        assert_eq!(parse_int(b"0"), Some(0));
        assert_eq!(parse_int(b"19"), Some(19));
        assert_eq!(parse_int(b"-20"), Some(-20));
        assert_eq!(parse_int(b"-"), None);
        assert_eq!(parse_int(b""), None);
        assert_eq!(parse_int(b"99999999999"), None);
    }

    #[test]
    fn statm() {
        assert_eq!(
            parse_statm_resident(b"2880021 64299 23600 164 0 449633 0\n"),
            Some(64_299)
        );
        assert_eq!(parse_statm_resident(b"12\n"), None);
        assert_eq!(parse_statm_resident(b""), None);
    }

    #[test]
    fn io() {
        assert_eq!(parse_io(IO), Some((1_281_118_208, 79_998_976)));
        assert_eq!(parse_io(b"rchar: 5\nwchar: 6\n"), None);
        assert_eq!(parse_io(b""), None);
    }

    #[test]
    fn units() {
        for (name, input, want) in [
            (
                "unified, app scope",
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.chromium.Chromium-662486.scope\n",
                Some("app-org.chromium.Chromium-662486.scope"),
            ),
            (
                "unified, app service with an instance",
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-org.kde.dolphin@765eead4667549d0ab239e9e204534a3.service\n",
                Some("app-org.kde.dolphin@765eead4667549d0ab239e9e204534a3.service"),
            ),
            (
                "cgroups below the unit",
                "0::/user.slice/user-1000.slice/user@1000.service/app.slice/app-flatpak-com.discordapp.Discord-3262276743.scope/sandbox/renderer\n",
                Some("app-flatpak-com.discordapp.Discord-3262276743.scope"),
            ),
            (
                "Plasma",
                "0::/user.slice/user-1000.slice/user@1000.service/session.slice/plasma-plasmashell.service\n",
                Some("plasma-plasmashell.service"),
            ),
            (
                "system service",
                "0::/system.slice/sshd.service\n",
                Some("sshd.service"),
            ),
            (
                "the user manager itself",
                "0::/user.slice/user-1000.slice/user@1000.service/init.scope\n",
                Some("init.scope"),
            ),
            ("root", "0::/\n", None),
            ("nothing", "", None),
            (
                "hybrid: the unified line wins",
                "12:cpuset:/\n1:name=systemd:/system.slice/old.service\n0::/system.slice/cups.service\n",
                Some("cups.service"),
            ),
            (
                "legacy",
                "5:cpu,cpuacct:/\n1:name=systemd:/user.slice/user-1000.slice/session-2.scope\n",
                Some("session-2.scope"),
            ),
            ("slices only", "0::/user.slice/user-1000.slice\n", None),
        ] {
            let got = unit_from_cgroup(input.as_bytes());
            assert_eq!(got, want.map(str::as_bytes), "{name}");
        }
    }

    #[test]
    fn containers() {
        const ID: &str = "2414f7b322e3a727aae64037b633eefd3f14b985dff8ddcd38915e66ffd4e539";
        let user = "0::/user.slice/user-1000.slice/user@1000.service";
        for (name, path, want) in [
            (
                "podman run, toolbox, distrobox",
                format!("{user}/user.slice/libpod-{ID}.scope/container"),
                Some(ID),
            ),
            (
                "the container's own cgroup",
                format!("{user}/user.slice/libpod-{ID}.scope"),
                Some(ID),
            ),
            (
                "its monitor",
                format!("{user}/user.slice/libpod-conmon-{ID}.scope"),
                Some(ID),
            ),
            (
                "Quadlet",
                format!("{user}/app.slice/claude-otel.service/libpod-payload-{ID}"),
                Some(ID),
            ),
            (
                "a Quadlet's monitor is its service's",
                format!("{user}/app.slice/claude-otel.service/runtime"),
                None,
            ),
            (
                "systemd inside (distrobox --init)",
                format!("{user}/user.slice/libpod-{ID}.scope/container/system.slice/sshd.service"),
                Some(ID),
            ),
            (
                "rootful",
                format!("0::/machine.slice/libpod-{ID}.scope/container"),
                Some(ID),
            ),
            (
                "an app",
                format!("{user}/app.slice/app-org.kde.konsole-4242.scope"),
                None,
            ),
            (
                "short ID",
                format!("{user}/user.slice/libpod-{}.scope", &ID[..12]),
                None,
            ),
            (
                "not hex",
                format!("{user}/user.slice/libpod-{}.scope", ID.replace('a', "z")),
                None,
            ),
            (
                "no scope suffix",
                format!("{user}/user.slice/libpod-{ID}"),
                None,
            ),
        ] {
            let got = container_from_cgroup(path.as_bytes());
            assert_eq!(got, want.map(str::as_bytes), "{name}");
        }
        let legacy = format!("1:name=systemd:/machine.slice/libpod-{ID}.scope\n");
        assert_eq!(
            container_from_cgroup(legacy.as_bytes()),
            Some(ID.as_bytes())
        );
        assert_eq!(container_from_cgroup(b""), None);
    }
}
