//! Prints what starts at login: `cargo run -p atlas-sysinfo --example
//! autostart` lists everything (plumbing marked), `-- --bench [n]` times n
//! lists, and `-- --set <id> on|off` switches one (it writes in
//! `~/.config`: run it in the test VM or a container, not on a desktop
//! someone is using).

use std::time::Instant;

use atlas_sysinfo::autostart::{self, Kind};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let args: Vec<&str> = args.iter().map(String::as_str).collect();
    match args[..] {
        ["--set", id, state] => {
            let on = match state {
                "on" => true,
                "off" => false,
                _ => panic!("on or off, not {state}"),
            };
            let list = autostart::list();
            let Some(item) = list.items.iter().find(|i| i.id == id) else {
                println!("no item {id}");
                return;
            };
            let t = Instant::now();
            let result = autostart::set_enabled(item, on);
            println!("{id} {state}: {result:?} in {:?}", t.elapsed());
        }
        ["--bench", ..] => {
            let n: u32 = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(20);
            let t = Instant::now();
            let mut items = 0;
            for _ in 0..n {
                items = autostart::list().items.len();
            }
            println!("{n} lists of {items} items: {:?} each", t.elapsed() / n);
        }
        _ => {
            let t = Instant::now();
            let list = autostart::list();
            let took = t.elapsed();
            for i in &list.items {
                let kind = match i.kind {
                    Kind::Desktop => "entry",
                    Kind::Unit => "unit ",
                };
                println!(
                    "{} {kind} {:<44} {:<3} {}{}{}runs:{:?} unit:{} {:?}",
                    if i.plumbing { "·" } else { "*" },
                    i.id,
                    if i.enabled { "on" } else { "off" },
                    if i.system { "system " } else { "user " },
                    i.lock.map(|l| format!("lock:{l:?} ")).unwrap_or_default(),
                    if i.can_switch() { "" } else { "fixed " },
                    i.runs,
                    i.unit.as_deref().unwrap_or("-"),
                    i.status,
                );
                println!("      {} — {}", i.name, i.file.display());
            }
            println!(
                "{} items ({} plumbing), user manager {}, in {took:?}",
                list.items.len(),
                list.items.iter().filter(|i| i.plumbing).count(),
                if list.units {
                    "answered"
                } else {
                    "did not answer"
                },
            );
        }
    }
}
