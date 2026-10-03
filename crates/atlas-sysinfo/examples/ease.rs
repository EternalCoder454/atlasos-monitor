//! Energy Saver against this session.
//!
//! `cargo run -p atlas-sysinfo --example ease` watches every application for
//! 30 s with automatic easing off (it changes nothing) and prints the rows.
//! `-- --bench` times a units sample and a sound check. `-- --trial` is the
//! end-to-end test: it starts throwaway `app-atlastest*` scopes (two busy
//! loops, one also playing silence) and lets a controller that sees only
//! those ease them, then drops it as a crash would and recovers. Nothing
//! else in the session is touched.

use std::path::Path;
use std::process::{Child, Command};
use std::sync::Arc;
use std::thread::sleep;
use std::time::{Duration, Instant};

use atlas_sysinfo::apps::{Resolver, desktop};
use atlas_sysinfo::ease::audio::{PipeWire, ProcFs};
use atlas_sysinfo::ease::system::{CgroupUnits, SystemdWeights, app_slice, state_file};
use atlas_sysinfo::ease::{
    self, Audio, Controller, EASE_AFTER, Identity, TICK_EVERY, Units, Weights,
};

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match args.first().map(String::as_str) {
        Some("--bench") => bench(),
        Some("--trial") => trial(),
        _ => watch(),
    }
}

fn watch() {
    // No state file: this run must not recover or record anything.
    let mut c = match ease::open(None) {
        Ok(c) => c,
        Err(e) => return println!("unavailable: {e}"),
    };
    println!("would use {:?}", state_file());
    let mut resolver = Resolver::for_session("breeze");
    for _ in 0..6 {
        c.tick(&mut resolver);
        for r in c.rows() {
            println!("{:5.1}% {:?} {} ({})", r.cpu, r.status, r.name, r.unit);
        }
        println!("--");
        sleep(TICK_EVERY);
    }
}

fn bench() {
    let slice = match app_slice() {
        Ok(s) => s,
        Err(e) => return println!("unavailable: {e}"),
    };
    let mut units = CgroupUnits::new(slice.to_path_buf());
    let n = units.sample().map_or(0, |s| s.len());
    let t = Instant::now();
    for _ in 0..200 {
        units.sample();
    }
    println!("units sample ({n} units): {:?}", t.elapsed() / 200);
    let Some(mut audio) = PipeWire::new(ProcFs { app_slice: slice }) else {
        return println!("no pw-dump");
    };
    let t = Instant::now();
    let mut heard = None;
    for _ in 0..10 {
        heard = audio.audible();
    }
    println!("sound check: {:?}, audible {heard:?}", t.elapsed() / 10);
    if let Some(mut w) = SystemdWeights::new() {
        let unit = units.sample().and_then(|s| {
            s.keys()
                .find(|u| u.starts_with("app-"))
                .map(|u| u.to_string())
        });
        if let Some(unit) = unit {
            let t = Instant::now();
            let got = w.weight(&unit);
            println!("weight of {unit}: {got:?} in {:?}", t.elapsed());
        }
    }
}

struct Scope {
    unit: String,
    child: Child,
}

impl Scope {
    fn start(name: &str, script: &str) -> Self {
        let unit = format!("app-{name}-{}.scope", std::process::id());
        let child = Command::new("systemd-run")
            .args(["--user", "--scope", "-q", "-u", &unit, "sh", "-c", script])
            .spawn()
            .expect("systemd-run");
        Self { unit, child }
    }
}

impl Drop for Scope {
    fn drop(&mut self) {
        let _ = Command::new("systemctl")
            .args(["--user", "stop", &self.unit])
            .status();
        let _ = self.child.wait();
    }
}

fn controller(slice: &Path, state: &Path) -> Controller {
    Controller::new(
        Box::new(CgroupUnits::new(slice.to_path_buf())),
        Box::new(SystemdWeights::new().expect("user manager")),
        Box::new(
            PipeWire::new(ProcFs {
                app_slice: slice.to_path_buf(),
            })
            .expect("pw-dump"),
        ),
        None,
        Some(state.to_path_buf()),
    )
}

/// Only the trial's own units are applications to this controller.
fn only_trial(unit: &str) -> Option<Identity> {
    let id: Arc<str> = desktop::app_id(unit)?.into();
    id.starts_with("atlastest").then(|| Identity {
        name: id.clone(),
        terminal: false,
        id,
    })
}

fn weight(w: &mut SystemdWeights, s: &Scope) -> String {
    match w.weight(&s.unit) {
        Ok(u64::MAX) => "unset".into(),
        Ok(v) => v.to_string(),
        Err(e) => format!("{e:?}"),
    }
}

fn trial() {
    let slice = app_slice().expect("app.slice");
    let dir = std::env::temp_dir().join(format!("atlas-ease-trial-{}", std::process::id()));
    let state = dir.join("eased");
    let busy = Scope::start("atlastestbusy", "while :; do :; done");
    let play = Scope::start(
        "atlastestplay",
        "paplay --raw --rate 48000 --channels 2 --format s16le /dev/zero & while :; do :; done",
    );
    let manual = Scope::start("atlastestmanual", "while :; do :; done");
    sleep(Duration::from_secs(1));
    let mut w = SystemdWeights::new().expect("user manager");
    println!(
        "before: busy {} play {} manual {}",
        weight(&mut w, &busy),
        weight(&mut w, &play),
        weight(&mut w, &manual)
    );

    let mut c = controller(&slice, &state);
    c.set_automatic(true);
    c.tick(&mut only_trial);
    c.ease("atlastestmanual").expect("manual ease");
    let end = Instant::now() + EASE_AFTER + 2 * TICK_EVERY;
    while Instant::now() < end {
        sleep(TICK_EVERY);
        let t = Instant::now();
        c.tick(&mut only_trial);
        let took = t.elapsed();
        let rows: Vec<String> = c
            .rows()
            .iter()
            .map(|r| format!("{} {:.0}% {:?}", r.id, r.cpu, r.status))
            .collect();
        println!("tick {took:?}: {}", rows.join(", "));
    }
    println!(
        "eased: busy {} play {} manual {}",
        weight(&mut w, &busy),
        weight(&mut w, &play),
        weight(&mut w, &manual)
    );
    println!(
        "state file:\n{}",
        std::fs::read_to_string(&state).unwrap_or_default()
    );

    // A crash: gone without putting anything back.
    std::mem::forget(c);
    let mut c = controller(&slice, &state);
    c.recover();
    c.tick(&mut only_trial);
    println!(
        "recovered: busy {} play {} manual {}; rows {:?}",
        weight(&mut w, &busy),
        weight(&mut w, &play),
        weight(&mut w, &manual),
        c.rows()
            .iter()
            .map(|r| (r.id.clone(), r.status))
            .collect::<Vec<_>>()
    );
    c.restore("atlastestmanual").expect("restore");
    println!(
        "put back by hand: manual {}; state file left: {}",
        weight(&mut w, &manual),
        state.exists()
    );
    drop(c);
    let _ = std::fs::remove_dir_all(&dir);
}
