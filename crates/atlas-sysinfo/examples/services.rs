//! Prints the services systemd has, and what reading them costs:
//! `cargo run -p atlas-sysinfo --example services` lists them,
//! `-- --details <name>` shows one, `-- --bench [n]` times n lists and
//! counts PID 1's CPU time for them (from `/proc/1/stat`), and
//! `-- --watch [n]` lists once a second n times and prints what changed,
//! `-- --act start|stop|restart|enable|disable <name>` does an action
//! (polkit asks for a password: run it in the test VM, not on a desktop
//! someone is using).

use std::time::Instant;

use atlas_sysinfo::services::{self, Action, ServiceReader};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    if let ["--act", action, name] = args[..] {
        let action = match action {
            "start" => Action::Start,
            "stop" => Action::Stop,
            "restart" => Action::Restart,
            "enable" => Action::Enable,
            "disable" => Action::Disable,
            _ => panic!("no action {action}"),
        };
        let t = Instant::now();
        let result = services::act(name, action);
        println!("{action:?} {name}: {result:?} in {:?}", t.elapsed());
        return;
    }
    let t = Instant::now();
    let Some(mut reader) = ServiceReader::new() else {
        println!("systemd is not on the system bus");
        return;
    };
    println!("connected in {:?}", t.elapsed());
    match args[..] {
        ["--details", name] => println!("{:#?}", reader.details(name)),
        ["--watch", ..] => {
            let n: u32 = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(30);
            let mut before: Vec<services::Service> = Vec::new();
            for i in 0..n {
                let t = Instant::now();
                let list = reader.list().unwrap_or_default();
                let took = t.elapsed();
                let gone = before
                    .iter()
                    .filter(|b| !list.iter().any(|s| s.name == b.name));
                for s in gone {
                    println!("{i:>3} gone     {}", s.name);
                }
                for s in &list {
                    match before.iter().find(|b| b.name == s.name) {
                        None if i > 0 => println!(
                            "{i:>3} new      {} {:?} {:?}",
                            s.name,
                            s.status(),
                            s.file_state
                        ),
                        Some(b) if b != s => println!(
                            "{i:>3} changed  {} {:?}/{} {:?} -> {:?}/{} {:?} job {:?}",
                            s.name,
                            b.status(),
                            b.sub,
                            b.file_state,
                            s.status(),
                            s.sub,
                            s.file_state,
                            s.job
                        ),
                        _ => {}
                    }
                }
                println!("{i:>3} {} services in {took:?}", list.len());
                before = list;
                std::thread::sleep(std::time::Duration::from_secs(1));
            }
        }
        ["--bench", ..] => {
            let n: u32 = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(20);
            let t = Instant::now();
            let first = reader.list().map_or(0, |l| l.len());
            println!(
                "first list: {first} services in {:?} (with the unit files)",
                t.elapsed()
            );
            let pid1 = pid1_ticks();
            let t = Instant::now();
            for _ in 0..n {
                reader.list().expect("systemd answered");
            }
            let each = t.elapsed() / n;
            let ticks = pid1_ticks().zip(pid1).map(|(b, a)| b - a);
            println!("then {each:?} a list ({n} lists)");
            if let Some(ticks) = ticks {
                // Clock ticks are 10 ms.
                println!(
                    "PID 1: {:.1} ms CPU a list",
                    ticks as f64 * 10.0 / f64::from(n)
                );
            }
            let t = Instant::now();
            for _ in 0..n {
                reader.failed().expect("systemd answered");
            }
            println!("failed(): {:?} each", t.elapsed() / n);
        }
        _ => {
            let t = Instant::now();
            let list = reader.list().unwrap_or_default();
            println!("{} services in {:?}", list.len(), t.elapsed());
            for s in &list {
                let file = s
                    .file_state
                    .as_ref()
                    .map_or("-".to_owned(), |f| format!("{f:?}"));
                let job = s
                    .job
                    .as_deref()
                    .map(|j| format!(" [{j}]"))
                    .unwrap_or_default();
                println!(
                    "{:<9} {:<14} {:<48} {}{job}",
                    format!("{:?}", s.status()),
                    file,
                    s.name,
                    s.description
                );
            }
            println!("failed: {:?}", reader.failed());
        }
    }
}

/// PID 1's user + system time in clock ticks.
fn pid1_ticks() -> Option<u64> {
    let stat = std::fs::read_to_string("/proc/1/stat").ok()?;
    let rest = &stat[stat.rfind(')')? + 2..];
    let fields: Vec<&str> = rest.split(' ').collect();
    // utime and stime are fields 14 and 15; `rest` starts at field 3.
    Some(fields.get(11)?.parse::<u64>().ok()? + fields.get(12)?.parse::<u64>().ok()?)
}
