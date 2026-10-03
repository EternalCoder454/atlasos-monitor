//! The sampler against the live `/proc`. CI runs in a container: its own
//! processes only, no kernel threads, no GPU. So these assert what holds
//! anywhere, and make their own processes when they need one.

use super::*;
use std::io::Write;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn find(procs: &[Proc], pid: u32) -> Option<&Proc> {
    procs.iter().find(|p| p.pid == pid)
}

/// Processors online, from sysfs: `available_parallelism` shrinks under
/// taskset or a cpuset, but other processes aren't held to that.
fn online_cpus() -> usize {
    let text = std::fs::read_to_string("/sys/devices/system/cpu/online").unwrap();
    text.trim()
        .split(',')
        .map(|range| match range.split_once('-') {
            Some((a, b)) => b.parse::<usize>().unwrap() - a.parse::<usize>().unwrap() + 1,
            None => 1,
        })
        .sum()
}

fn kill(mut child: Child) {
    let _ = child.kill();
    let _ = child.wait();
}

#[test]
fn finds_this_process() {
    let me = std::process::id();
    let mut s = ProcessSampler::default();
    let procs = s.sample();
    let p = find(procs, me).expect("this process is not in the table");
    let comm = std::fs::read_to_string("/proc/self/comm").unwrap();
    assert_eq!(&*p.name, comm.trim_end());
    assert!(!p.kernel);
    assert!(p.memory > 0);
    assert_eq!(p.parent, std::os::unix::process::parent_id());
    assert_eq!(Some(p.start_time), start_time(me));
    let cgroup = std::fs::read("/proc/self/cgroup").unwrap();
    let unit = unit_from_cgroup(&cgroup).map(|u| String::from_utf8_lossy(u).into_owned());
    assert_eq!(p.unit.as_deref(), unit.as_deref());
    let container = container_from_cgroup(&cgroup).map(|c| String::from_utf8_lossy(c).into_owned());
    assert_eq!(p.container.as_deref(), container.as_deref());
    // Our own io and fd/ are readable, and disk and network are wanted.
    assert!(p.disk_read.is_some() && p.disk_write.is_some());
    assert!(p.net_in.is_some() && p.net_out.is_some());
}

/// The flag read from the stat line, checked against the independent ground
/// truth: a kernel thread has no command line. (The converse doesn't hold: a
/// zombie has none either.) In a container there are no kernel threads, so
/// only the userspace half can be checked there.
#[test]
fn kernel_threads() {
    let all = Wanted {
        kernel_threads: true,
        ..Wanted::default()
    };
    let mut s = ProcessSampler::new(all);
    let procs = s.sample().to_vec();
    assert!(
        procs.iter().any(|p| !p.kernel),
        "everything was a kernel thread"
    );
    for p in &procs {
        if p.pid == 1 {
            assert!(!p.kernel, "init classed as a kernel thread");
        }
        if p.pid == 2 && &*p.name == "kthreadd" {
            assert!(p.kernel, "kthreadd not classed as a kernel thread");
        }
        if p.kernel {
            let cmdline = std::fs::read(format!("/proc/{}/cmdline", p.pid)).unwrap_or_default();
            assert!(
                cmdline.is_empty(),
                "{} ({}) has a command line",
                p.name,
                p.pid
            );
            assert_eq!(p.unit, None);
            assert_eq!(p.gpu, None);
        }
    }

    let mut s = ProcessSampler::default();
    let without = s.sample();
    assert!(
        without.iter().all(|p| !p.kernel),
        "kernel threads not left out"
    );
    let kernel = procs.iter().filter(|p| p.kernel).count();
    // Processes come and go between the two scans (a build alongside
    // starts hundreds).
    let expected = procs.len() - kernel;
    assert!(
        without.len().abs_diff(expected) <= (expected / 5).max(20),
        "{} vs {expected}",
        without.len()
    );
}

#[test]
fn figures_are_sane() {
    let mut s = ProcessSampler::default();
    std::thread::sleep(Duration::from_millis(200));
    let procs = s.sample();
    assert!(!procs.is_empty());
    assert!(procs.is_sorted_by_key(|p| p.pid));
    let limit = online_cpus() as f64 * 100.0;
    let mut total = 0.0;
    for p in procs {
        assert!(
            p.cpu.is_finite() && p.cpu >= 0.0,
            "{} cpu {}",
            p.name,
            p.cpu
        );
        // A process can't use more than every processor (with slack for the
        // two reads of its counters not being exactly the interval apart).
        assert!(p.cpu <= limit * 1.5, "{} cpu {}", p.name, p.cpu);
        total += p.cpu;
        if let Some(g) = p.gpu {
            assert!((0.0..=100.0).contains(&g), "{} gpu {g}", p.name);
        }
        for rate in [p.disk_read, p.disk_write, p.net_in, p.net_out]
            .into_iter()
            .flatten()
        {
            assert!(rate.is_finite() && rate >= 0.0, "{} rate {rate}", p.name);
        }
    }
    assert!(total <= limit * 1.5, "all processes together used {total}%");
}

/// The baseline taken when the sampler is made makes the first sample a
/// real reading: a thread spinning in between shows up.
#[test]
fn the_first_sample_measures_cpu() {
    let me = std::process::id();
    let mut s = ProcessSampler::default();
    let until = std::time::Instant::now() + Duration::from_millis(300);
    let spinner = std::thread::spawn(move || {
        let mut x = 0u64;
        while std::time::Instant::now() < until {
            x = std::hint::black_box(x.wrapping_add(1));
        }
    });
    spinner.join().unwrap();
    let p = find(s.sample(), me).unwrap().clone();
    assert!(p.cpu > 20.0, "a 300 ms spin read as {:.1}% CPU", p.cpu);
}

/// A steady process costs no allocation: its name and unit are the previous
/// tick's, shared.
#[test]
fn names_are_reused() {
    let me = std::process::id();
    let mut s = ProcessSampler::default();
    let first = find(s.sample(), me).unwrap().clone();
    let second = find(s.sample(), me).unwrap().clone();
    assert!(Arc::ptr_eq(&first.name, &second.name));
    if let (Some(a), Some(b)) = (&first.unit, &second.unit) {
        assert!(Arc::ptr_eq(a, b));
    }
}

/// A process that execs another program keeps its pid and start time but
/// changes its name, and the table follows.
#[test]
fn a_renamed_process_is_seen() {
    let mut child = Command::new("sh")
        .args(["-c", "read line; exec sleep 30"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    let pid = child.id();
    let mut s = ProcessSampler::default();
    let before = find(s.sample(), pid).expect("the child").clone();
    assert_eq!(&*before.name, "sh");
    writeln!(child.stdin.as_mut().unwrap(), "go").unwrap();
    let comm = format!("/proc/{pid}/comm");
    let execed = (0..400).any(|_| {
        let done = std::fs::read_to_string(&comm).is_ok_and(|c| c.trim_end() == "sleep");
        if !done {
            std::thread::sleep(Duration::from_millis(5));
        }
        done
    });
    assert!(execed, "the child never became sleep");
    let after = find(s.sample(), pid).expect("the child").clone();
    kill(child);
    assert_eq!(&*after.name, "sleep");
    assert_eq!(after.start_time, before.start_time);
}

#[test]
fn exited_processes_leave_the_table() {
    let child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id();
    let mut s = ProcessSampler::default();
    assert!(find(s.sample(), pid).is_some());
    kill(child);
    assert!(find(s.sample(), pid).is_none());
}

/// What isn't wanted isn't collected, and reads as unknown.
#[test]
fn unwanted_figures_are_not_collected() {
    let me = std::process::id();
    let mut s = ProcessSampler::new(Wanted {
        kernel_threads: false,
        disk: false,
        gpu: false,
        network: false,
    });
    for p in s.sample() {
        assert_eq!((p.disk_read, p.disk_write), (None, None), "{}", p.name);
        assert_eq!(p.gpu, None, "{}", p.name);
        assert_eq!((p.net_in, p.net_out), (None, None), "{}", p.name);
    }
    // Turned on again: disk figures appear without a made-up first rate.
    s.set_wanted(Wanted::default());
    let p = find(s.sample(), me).unwrap().clone();
    assert_eq!(p.disk_read, Some(0.0));
    assert_eq!(p.disk_write, Some(0.0));
}

#[test]
fn reuse_shares_unchanged_text() {
    let name: Arc<str> = Arc::from("plasmashell");
    assert!(Arc::ptr_eq(&reuse(Some(&name), b"plasmashell"), &name));
    assert_eq!(&*reuse(Some(&name), b"kwin_wayland"), "kwin_wayland");
    assert_eq!(&*reuse(None, b"x"), "x");
    // Not UTF-8: replacement characters, and still shared once seen.
    let odd = reuse(None, b"bad\xff");
    assert_eq!(&*odd, "bad\u{fffd}");
    assert!(Arc::ptr_eq(&reuse(Some(&odd), b"bad\xff"), &odd));
}

/// Skipping statm for a process that hasn't moved mustn't hide one that
/// has: memory touched between two samples shows on the second.
#[test]
fn memory_growth_is_seen() {
    let me = std::process::id();
    let mut s = ProcessSampler::default();
    let before = find(s.sample(), me).unwrap().memory;
    let grown = 128 << 20;
    let block = std::hint::black_box(vec![1u8; grown]);
    let after = find(s.sample(), me).unwrap().memory;
    drop(block);
    // Other tests in this binary allocate and free meanwhile.
    assert!(
        after >= before + grown as u64 * 3 / 4,
        "{before} -> {after} after touching {grown} bytes"
    );
}

#[test]
fn network_is_shared_by_socket_count() {
    let state = |sockets, fd_denied| Prev {
        pid: 0,
        start_time: 0,
        name: Arc::from(""),
        unit: None,
        container: None,
        jiffies: 0,
        faults: 0,
        rss: 0,
        memory: 0,
        io: None,
        io_age: 0.0,
        io_denied: false,
        gpu_time: None,
        drm_fds: Vec::new(),
        fd_denied,
        sockets,
        fd_count: None,
        stat_fd: None,
        statm_fd: None,
        io_fd: None,
    };
    let proc = |pid| Proc {
        pid,
        start_time: 0,
        name: Arc::from(""),
        parent: 0,
        kernel: false,
        unit: None,
        container: None,
        cpu: 0.0,
        memory: 0,
        gpu: None,
        net_in: None,
        net_out: None,
        disk_read: None,
        disk_write: None,
    };
    let states = [state(1, false), state(3, false), state(0, true)];
    let mut procs = [proc(1), proc(2), proc(3)];
    share_network(&mut procs, &states, true, (400.0, 800.0));
    let shares: Vec<_> = procs.iter().map(|p| (p.net_in, p.net_out)).collect();
    assert_eq!(
        shares,
        [
            (Some(100.0), Some(200.0)),
            (Some(300.0), Some(600.0)),
            (None, None), // unreadable: unknown, not zero
        ]
    );
    share_network(&mut procs, &states, false, (400.0, 800.0));
    assert_eq!((procs[1].net_in, procs[1].net_out), (Some(0.0), Some(0.0)));
    // No sockets anywhere: nothing to share by.
    let none = [state(0, false)];
    let mut one = [proc(1)];
    share_network(&mut one, &none, true, (400.0, 800.0));
    assert_eq!(one[0].net_in, Some(0.0));
}

/// Disk counters carried through idle ticks give, once read again, the
/// rate over all the time since, not as if it were one tick's.
#[test]
fn disk_rates_cover_the_time_since_the_last_reading() {
    let before = Some(((5_000, 7_000), 2.0));
    assert_eq!(
        disk_rates(Some((3_005_000, 7_000)), before, 1.0),
        (Some(1e6), Some(0.0))
    );
    assert_eq!(
        disk_rates(Some((5_000, 9_000)), Some(((5_000, 7_000), 0.0)), 0.5),
        (Some(0.0), Some(4_000.0))
    );
    assert_eq!(disk_rates(Some((1, 1)), None, 1.0), (Some(0.0), Some(0.0)));
    assert_eq!(disk_rates(None, before, 1.0), (None, None));
}

/// The scan keeps its processes' files open, within its budget, and lets a
/// process that has gone go with them.
#[test]
fn held_files_stay_within_the_budget_and_go_with_their_process() {
    let child = Command::new("sleep").arg("30").spawn().unwrap();
    let pid = child.id();
    let mut s = ProcessSampler::new(Wanted::default());
    // Room for every process, whatever this machine's limit.
    s.max_held = usize::MAX;
    s.sample();
    let held = |s: &ProcessSampler| -> usize {
        s.prev
            .iter()
            .map(|p| {
                [&p.stat_fd, &p.statm_fd, &p.io_fd]
                    .iter()
                    .filter(|f| f.is_some())
                    .count()
            })
            .sum()
    };
    assert!(held(&s) > 0);
    let p = s
        .prev
        .iter()
        .find(|p| p.pid == pid)
        .expect("the child is listed");
    assert!(p.stat_fd.is_some());
    kill(child);
    assert!(find(s.sample(), pid).is_none());
    assert!(s.prev.iter().all(|p| p.pid != pid));

    // A smaller budget lets go of what it can't hold, and still lists
    // every process.
    let listed = s.procs.len();
    s.max_held = 6;
    s.sample();
    assert!(held(&s) <= 6, "{} held", held(&s));
    assert!(find(&s.procs, std::process::id()).is_some());
    assert!(s.procs.len() + 20 > listed);
}
