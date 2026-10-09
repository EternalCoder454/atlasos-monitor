//! Which applications are playing or recording: PipeWire's running streams
//! from `pw-dump`, traced to their units.
//!
//! A stream node names its client. A native PipeWire client carries the pid
//! the server took from its socket (`pipewire.sec.pid`), which can be
//! trusted. A PulseAudio client is reached through pipewire-pulse, whose own
//! pid is on the client object, so only the pid the client reported for
//! itself (`application.process.id`) is there, and inside a Flatpak that is
//! the sandbox's numbering and names some other process out here. So a
//! claimed pid counts only when `/proc/<pid>/exe` is the program the client
//! said it was (`application.process.binary`). A Flatpak's client carries
//! its app ID (`pipewire.access.portal.app_id`), which is used as it is.
//!
//! A stream that can't be traced for certain marks every application
//! running a program of that name ([`apps_of`]): leaving alone one that
//! could have been eased costs nothing; easing the one playing music is
//! the one thing this must not do.
//!
//! `pw-dump` takes about 10 ms and prints a few hundred KiB; it runs only
//! when something is about to be eased, and every 10 s while something is
//! eased or busy.

use std::collections::{HashMap, HashSet};
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

use serde_json::Value;

use crate::apps::desktop;
use crate::process::unit_from_cgroup;

use super::Audio;

/// How long `pw-dump` may take.
const DEADLINE: Duration = Duration::from_secs(3);
/// More output than this is not a PipeWire graph.
const OUTPUT_MAX: u64 = 32 << 20;

/// The media classes that count: playing, recording, and a camera (a call).
const CLASSES: &[&str] = &[
    "Stream/Output/Audio",
    "Stream/Input/Audio",
    "Stream/Input/Video",
];

/// A running stream and what is known about who owns it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Stream {
    /// The pid the server took from the socket, where there is one.
    pub trusted: u32,
    /// The pid the client reported, and the program it said it was.
    pub claimed: u32,
    pub binary: String,
    /// A Flatpak application's ID.
    pub app_id: String,
}

/// The running streams in `pw-dump`'s JSON. `None` when it isn't that.
pub fn parse_pw_dump(text: &[u8]) -> Option<Vec<Stream>> {
    fn props(o: &Value) -> Option<&serde_json::Map<String, Value>> {
        o.get("info")?.get("props")?.as_object()
    }
    let objects: Vec<Value> = serde_json::from_slice(text).ok()?;
    let clients: HashMap<u64, &serde_json::Map<String, Value>> = objects
        .iter()
        .filter(|o| o["type"] == "PipeWire:Interface:Client")
        .filter_map(|o| Some((o["id"].as_u64()?, props(o)?)))
        .collect();
    let mut out = Vec::new();
    for o in &objects {
        if o["type"] != "PipeWire:Interface:Node" || o["info"]["state"] != "running" {
            continue;
        }
        let Some(p) = props(o) else { continue };
        if !CLASSES.contains(&string(p.get("media.class")).as_str()) {
            continue;
        }
        let mut s = Stream {
            trusted: 0,
            claimed: number(p.get("application.process.id")),
            binary: string(p.get("application.process.binary")),
            app_id: string(p.get("pipewire.access.portal.app_id")),
        };
        if let Some(c) = clients.get(&u64::from(number(p.get("client.id")))) {
            let pulse = string(c.get("client.api")) == "pipewire-pulse"
                || string(c.get("application.name")) == "pipewire-pulse";
            if !pulse {
                s.trusted = number(c.get("pipewire.sec.pid"));
            }
            if s.claimed == 0 {
                s.claimed = number(c.get("application.process.id"));
            }
            if s.binary.is_empty() {
                s.binary = string(c.get("application.process.binary"));
            }
            if s.app_id.is_empty() {
                s.app_id = string(c.get("pipewire.access.portal.app_id"));
            }
        }
        out.push(s);
    }
    Some(out)
}

/// A property as text: PipeWire writes some numbers as strings, and some
/// strings as numbers.
fn string(v: Option<&Value>) -> String {
    match v {
        Some(Value::String(s)) => s.clone(),
        Some(Value::Number(n)) => n.to_string(),
        _ => String::new(),
    }
}

fn number(v: Option<&Value>) -> u32 {
    match v {
        Some(Value::Number(n)) => n.as_u64().and_then(|n| n.try_into().ok()).unwrap_or(0),
        Some(Value::String(s)) => s.trim().parse().unwrap_or(0),
        _ => 0,
    }
}

/// The process table as [`apps_of`] needs it; [`ProcFs`] reads the real one.
pub trait Procs {
    /// The unit a process runs in.
    fn unit(&self, pid: u32) -> Option<String>;
    /// The file name of the program a process runs.
    fn exe_name(&self, pid: u32) -> Option<String>;
    /// Every process in an application unit, with its unit.
    fn all(&self) -> Vec<(u32, String)>;
}

/// The applications the streams belong to, by ID.
pub fn apps_of(streams: &[Stream], procs: &dyn Procs) -> HashSet<String> {
    fn mark(out: &mut HashSet<String>, unit: Option<String>) {
        if let Some(id) = unit.as_deref().and_then(desktop::app_id) {
            out.insert(id.into_owned());
        }
    }
    let mut out = HashSet::new();
    let mut unknown = Vec::new();
    for s in streams {
        if !s.app_id.is_empty() {
            out.insert(s.app_id.clone());
        } else if s.trusted > 0 {
            mark(&mut out, procs.unit(s.trusted));
        } else if s.claimed > 0
            && !s.binary.is_empty()
            && procs.exe_name(s.claimed).as_deref() == Some(s.binary.as_str())
        {
            mark(&mut out, procs.unit(s.claimed));
        } else if !s.binary.is_empty() {
            unknown.push(s.binary.as_str());
        } else if s.claimed > 0 {
            // Nothing to check it against; marking too much is the safe side.
            mark(&mut out, procs.unit(s.claimed));
        }
    }
    if !unknown.is_empty() {
        for (pid, unit) in procs.all() {
            if procs
                .exe_name(pid)
                .is_some_and(|n| unknown.contains(&n.as_str()))
            {
                mark(&mut out, Some(unit));
            }
        }
    }
    out
}

/// The real process table: `/proc`, and the processes of every unit below
/// the user's `app.slice`.
#[derive(Debug, Clone)]
pub struct ProcFs {
    pub app_slice: PathBuf,
}

impl Procs for ProcFs {
    fn unit(&self, pid: u32) -> Option<String> {
        let b = fs::read(format!("/proc/{pid}/cgroup")).ok()?;
        unit_from_cgroup(&b).map(|u| String::from_utf8_lossy(u).into_owned())
    }

    fn exe_name(&self, pid: u32) -> Option<String> {
        let exe = fs::read_link(format!("/proc/{pid}/exe")).ok()?;
        let name = exe.file_name()?.to_str()?;
        Some(name.strip_suffix(" (deleted)").unwrap_or(name).to_owned())
    }

    fn all(&self) -> Vec<(u32, String)> {
        let mut out = Vec::new();
        for (unit, dir) in super::system::unit_dirs(&self.app_slice) {
            collect_procs(&dir, &unit, 0, &mut out);
        }
        out
    }
}

/// Every pid in `dir`'s cgroup and the cgroups below it (a Flatpak's
/// sandbox, a delegated scope's own).
fn collect_procs(dir: &Path, unit: &str, depth: u32, out: &mut Vec<(u32, String)>) {
    if let Ok(text) = fs::read_to_string(dir.join("cgroup.procs")) {
        out.extend(
            text.split_ascii_whitespace()
                .filter_map(|p| p.parse().ok())
                .map(|pid| (pid, unit.to_owned())),
        );
    }
    if depth >= 8 {
        return;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if e.file_type().is_ok_and(|t| t.is_dir()) {
            collect_procs(&e.path(), unit, depth + 1, out);
        }
    }
}

/// PipeWire through `pw-dump`.
#[derive(Debug, Clone)]
pub struct PipeWire {
    pw_dump: PathBuf,
    procs: ProcFs,
}

impl PipeWire {
    /// `None` when `pw-dump` isn't in `/usr/bin`.
    pub fn new(procs: ProcFs) -> Option<Self> {
        Some(Self {
            pw_dump: find_program("pw-dump")?,
            procs,
        })
    }
}

impl Audio for PipeWire {
    fn audible(&mut self) -> Option<HashSet<String>> {
        let out = run(&self.pw_dump)?;
        let streams = parse_pw_dump(&out)?;
        Some(apps_of(&streams, &self.procs))
    }
}

/// Where the system's programs are. Not the `PATH`: a program in a folder
/// the user can write (`~/.local/bin`, a project's `node_modules/.bin`) that
/// comes first in it would be run in place of the real one (docs/SECURITY.md,
/// "Programs we start").
const SYSTEM_BIN: [&str; 2] = ["/usr/bin", "/bin"];

/// `name`, a plain file name, as a program of the system.
pub fn find_program(name: &str) -> Option<PathBuf> {
    if name.is_empty() || name.contains('/') || name.starts_with('.') {
        return None;
    }
    SYSTEM_BIN
        .iter()
        .map(|d| Path::new(d).join(name))
        .find(|p| fs::metadata(p).is_ok_and(|m| m.is_file()))
}

/// Runs `program` and returns what it printed, if it exits cleanly within
/// [`DEADLINE`]. A hung one is killed.
fn run(program: &Path) -> Option<Vec<u8>> {
    let mut child = Command::new(program)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    let mut stdout = child.stdout.take()?;
    let (tx, rx) = mpsc::channel();
    let output = std::thread::scope(|s| {
        s.spawn(move || {
            let mut buf = Vec::new();
            let read = (&mut stdout).take(OUTPUT_MAX).read_to_end(&mut buf);
            let _ = tx.send(read.ok().map(|_| buf));
        });
        match rx.recv_timeout(DEADLINE) {
            Ok(out) => out,
            Err(_) => {
                // Closes the pipe, which ends the reader.
                let _ = child.kill();
                None
            }
        }
    });
    // Still running: it printed more than OUTPUT_MAX.
    if matches!(child.try_wait(), Ok(None)) {
        let _ = child.kill();
    }
    let status = child.wait().ok()?;
    output.filter(|_| status.success())
}
