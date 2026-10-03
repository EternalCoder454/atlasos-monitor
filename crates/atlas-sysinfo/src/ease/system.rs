//! The real pieces behind [`Controller`]: the user's `app.slice` cgroups
//! for CPU time ([`CgroupUnits`]), the user's systemd manager for weights
//! ([`SystemdWeights`]), PipeWire for sound ([`super::audio::PipeWire`]),
//! and the state file in the runtime directory.

use std::collections::HashMap;
use std::fmt;
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

use super::audio::{PipeWire, ProcFs};
use super::{Controller, Error, Sample, Saved, UNSET, Units, Weights};
use crate::apps::desktop;
use crate::process::unit_from_cgroup;
use crate::sysfs::{self, HeldFile};

const DEST: &str = "org.freedesktop.systemd1";
const PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IF: &str = "org.freedesktop.systemd1.Manager";
const PROPS_IF: &str = "org.freedesktop.DBus.Properties";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";

/// One call to the user's manager.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Connecting, or one read or set (two calls).
const DEADLINE: Duration = Duration::from_secs(4);
/// The quiet period after the manager stops answering, doubling to
/// [`RETRY_MAX`].
const RETRY: Duration = Duration::from_secs(30);
const RETRY_MAX: Duration = Duration::from_secs(300);

/// The state file's directory, under `$XDG_RUNTIME_DIR`.
const STATE_DIR: &str = "net.eterneon.atlas.monitor";
/// The state file is read up to this much.
const STATE_MAX: u64 = 256 * 1024;

/// Why Energy Saver can't work on this session. The page shows it in place
/// of a switch that could never do anything.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Unavailable {
    /// No `app.slice` for this user: applications aren't started in
    /// systemd units.
    NoAppUnits,
    /// The processor controller isn't enabled below `app.slice`, so a
    /// weight would be accepted and do nothing.
    NoCpuController,
    /// No session bus, or no systemd user manager on it.
    NoManager,
    /// No `pw-dump` to tell what is playing.
    NoPipeWire,
}

impl fmt::Display for Unavailable {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Unavailable::NoAppUnits => {
                "Applications are not started in systemd units here, so there is nothing to give a weight to."
            }
            Unavailable::NoCpuController => {
                "This system does not let applications' share of the processor be changed without administrator rights."
            }
            Unavailable::NoManager => "There is no systemd user manager on this session.",
            Unavailable::NoPipeWire => {
                "Telling what is playing needs pw-dump (pipewire-utils), which is not installed."
            }
        })
    }
}

impl std::error::Error for Unavailable {}

/// The controller for this session, with what a previous run left eased
/// recovered ([`Controller::recover`]). `state` is the state file
/// ([`state_file`]; `None` keeps none).
pub fn open(state: Option<PathBuf>) -> Result<Controller, Unavailable> {
    let slice = app_slice()?;
    let weights = SystemdWeights::new().ok_or(Unavailable::NoManager)?;
    let audio = PipeWire::new(ProcFs {
        app_slice: slice.clone(),
    })
    .ok_or(Unavailable::NoPipeWire)?;
    let mut c = Controller::new(
        Box::new(CgroupUnits::new(slice)),
        Box::new(weights),
        Box::new(audio),
        own_app_id().map(Arc::from),
        state,
    );
    c.recover();
    Ok(c)
}

/// The user's `app.slice`, where the desktop starts applications, provided
/// the processor controller is enabled below it.
pub fn app_slice() -> Result<PathBuf, Unavailable> {
    let uid = rustix::process::getuid().as_raw();
    let slice = PathBuf::from(format!(
        "/sys/fs/cgroup/user.slice/user-{uid}.slice/user@{uid}.service/app.slice"
    ));
    let controllers = fs::read_to_string(slice.join("cgroup.subtree_control"))
        .map_err(|_| Unavailable::NoAppUnits)?;
    if controllers.split_ascii_whitespace().any(|c| c == "cpu") {
        Ok(slice)
    } else {
        Err(Unavailable::NoCpuController)
    }
}

/// Atlas Monitor's own application ID, from its own unit.
fn own_app_id() -> Option<String> {
    let b = fs::read("/proc/self/cgroup").ok()?;
    let unit = std::str::from_utf8(unit_from_cgroup(&b)?).ok()?;
    desktop::app_id(unit).map(|id| id.into_owned())
}

/// Every unit cgroup below `app.slice`, by unit name: its scopes and
/// services, and those in the slices directly below it (some desktops sort
/// applications into slices of their own; one level is as deep as any goes).
pub fn unit_dirs(slice: &Path) -> Vec<(String, PathBuf)> {
    let mut out = Vec::new();
    list_units(slice, true, &mut out);
    out
}

fn list_units(dir: &Path, top: bool, out: &mut Vec<(String, PathBuf)>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for e in entries.flatten() {
        if !e.file_type().is_ok_and(|t| t.is_dir()) {
            continue;
        }
        let Ok(name) = e.file_name().into_string() else {
            continue;
        };
        if name.ends_with(".scope") || name.ends_with(".service") {
            out.push((name, e.path()));
        } else if top && name.ends_with(".slice") {
            list_units(&e.path(), false, out);
        }
    }
}

/// Each application unit's CPU time and weight, from its cgroup's
/// `cpu.stat` and `cpu.weight`, held open between ticks.
#[derive(Debug)]
pub struct CgroupUnits {
    slice: PathBuf,
    held: HashMap<Arc<str>, Held>,
}

#[derive(Debug)]
struct Held {
    stat: HeldFile,
    /// Missing when the processor controller isn't enabled above it.
    weight: Option<HeldFile>,
}

impl Held {
    fn open(dir: &Path) -> Option<Self> {
        Some(Self {
            stat: HeldFile::with_capacity(dir.join("cpu.stat"), 1024)?,
            weight: HeldFile::open(dir.join("cpu.weight")),
        })
    }

    fn read(&mut self) -> Option<Sample> {
        let usage = parse_usage(self.stat.bytes()?)?;
        let weight = self.weight.as_mut().and_then(HeldFile::uint).unwrap_or(0);
        Some(Sample { usage, weight })
    }
}

impl CgroupUnits {
    pub fn new(slice: PathBuf) -> Self {
        Self {
            slice,
            held: HashMap::new(),
        }
    }
}

impl Units for CgroupUnits {
    fn sample(&mut self) -> Option<HashMap<Arc<str>, Sample>> {
        let dirs = unit_dirs(&self.slice);
        let mut out = HashMap::with_capacity(dirs.len());
        let mut held = HashMap::with_capacity(dirs.len());
        for (name, dir) in dirs {
            let (key, mut h) = match self.held.remove_entry(name.as_str()) {
                Some(kept) => kept,
                None => match Held::open(&dir) {
                    Some(h) => (Arc::from(name), h),
                    None => continue,
                },
            };
            // A read error is a cgroup removed under the held file: the
            // unit ended, or stopped and started again under the same name.
            let sample = match h.read() {
                Some(s) => s,
                None => match Held::open(&dir) {
                    Some(fresh) => {
                        h = fresh;
                        match h.read() {
                            Some(s) => s,
                            None => continue,
                        }
                    }
                    None => continue,
                },
            };
            out.insert(key.clone(), sample);
            held.insert(key, h);
        }
        self.held = held;
        Some(out)
    }
}

/// `usage_usec` from a cgroup's `cpu.stat`.
pub fn parse_usage(stat: &[u8]) -> Option<u64> {
    stat.split(|&c| c == b'\n').find_map(|line| {
        let v = line.strip_prefix(b"usage_usec ")?;
        sysfs::parse_uint(v.trim_ascii())
    })
}

/// Whether `unit` is one Energy Saver may set a weight on: an application's
/// scope or service, as the desktop names them.
pub fn app_unit(unit: &str) -> bool {
    unit.starts_with("app-")
        && (unit.ends_with(".scope") || unit.ends_with(".service"))
        && unit.len() <= 256
        && unit
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || b":-_.\\@".contains(&c))
}

/// `CPUWeight` through the user's systemd manager, as a runtime property:
/// it lasts until the unit ends.
///
/// Construct and use it on the sampling thread (not inside a tokio
/// runtime): it runs its own single-threaded one, driven only during a
/// call, like `smart::SmartReader`. A call that times out or loses the bus
/// drops the connection, and every call in the next [`RETRY`] fails at
/// once; the one after connects again, each failure in a row doubling the
/// wait up to [`RETRY_MAX`].
pub struct SystemdWeights {
    conn: Option<Connection>,
    retry_at: Option<Instant>,
    retry: Duration,
    rt: tokio::runtime::Runtime,
}

impl SystemdWeights {
    /// `None` without a session bus or a user manager on it.
    pub fn new() -> Option<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .thread_keep_alive(Duration::from_secs(1))
            .build()
            .ok()?;
        let conn = rt.block_on(async {
            tokio::time::timeout(DEADLINE, connect())
                .await
                .ok()
                .flatten()
        })?;
        Some(Self {
            conn: Some(conn),
            retry_at: None,
            retry: RETRY,
            rt,
        })
    }

    /// Runs `call` on the connection, connecting first if needed.
    fn with_conn<T, F>(&mut self, call: F) -> Result<T, Error>
    where
        F: AsyncFnOnce(&Connection) -> Result<T, zbus::Error>,
    {
        if self.conn.is_none() {
            if self.retry_at.is_some_and(|t| Instant::now() < t) {
                return Err(Error::NoAnswer);
            }
            self.conn = self.rt.block_on(async {
                tokio::time::timeout(DEADLINE, connect())
                    .await
                    .ok()
                    .flatten()
            });
            if self.conn.is_none() {
                self.back_off();
                return Err(Error::NoAnswer);
            }
        }
        let conn = self.conn.as_ref().ok_or(Error::NoAnswer)?;
        let result = self
            .rt
            .block_on(async { tokio::time::timeout(DEADLINE, call(conn)).await });
        match result {
            Ok(Ok(v)) => {
                self.retry = RETRY;
                Ok(v)
            }
            Ok(Err(zbus::Error::MethodError(name, message, _))) => match name.as_str() {
                "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => {
                    self.drop_conn();
                    Err(Error::NoAnswer)
                }
                other => {
                    self.retry = RETRY;
                    Err(match other {
                        "org.freedesktop.systemd1.NoSuchUnit"
                        | "org.freedesktop.systemd1.NoUnitForPID" => Error::Gone,
                        _ => Error::Refused(message.unwrap_or_else(|| other.to_owned())),
                    })
                }
            },
            // An answer that isn't what was asked for: the bus is fine.
            Ok(Err(zbus::Error::Variant(e))) => {
                self.retry = RETRY;
                Err(Error::Refused(e.to_string()))
            }
            // Timed out, or the bus failed rather than the manager answering.
            _ => {
                self.drop_conn();
                Err(Error::NoAnswer)
            }
        }
    }

    fn drop_conn(&mut self) {
        let _guard = self.rt.enter();
        self.conn = None;
        self.back_off();
    }

    fn back_off(&mut self) {
        self.retry_at = Some(Instant::now() + self.retry);
        self.retry = (self.retry * 2).min(RETRY_MAX);
    }
}

impl Drop for SystemdWeights {
    fn drop(&mut self) {
        // zbus's socket reader is a task on this runtime.
        let _guard = self.rt.enter();
        self.conn.take();
    }
}

impl Weights for SystemdWeights {
    fn retry_now(&mut self) {
        self.retry_at = None;
    }

    fn weight(&mut self, unit: &str) -> Result<u64, Error> {
        if !app_unit(unit) {
            return Err(Error::NotAnApp);
        }
        let iface = if unit.ends_with(".scope") {
            "org.freedesktop.systemd1.Scope"
        } else {
            "org.freedesktop.systemd1.Service"
        };
        let value: OwnedValue = self.with_conn(async |conn| {
            let path: OwnedObjectPath = conn
                .call_method(Some(DEST), PATH, Some(MANAGER_IF), "GetUnit", &(unit,))
                .await?
                .body()
                .deserialize()?;
            conn.call_method(
                Some(DEST),
                &path,
                Some(PROPS_IF),
                "Get",
                &(iface, "CPUWeight"),
            )
            .await?
            .body()
            .deserialize()
        })?;
        match &*value {
            Value::U64(w) => Ok(*w),
            Value::Value(v) => match **v {
                Value::U64(w) => Ok(w),
                _ => Err(Error::Refused("CPUWeight is not a number".into())),
            },
            _ => Err(Error::Refused("CPUWeight is not a number".into())),
        }
    }

    fn set_weight(&mut self, unit: &str, weight: u64) -> Result<(), Error> {
        // Belt and braces: only ever an application's unit. A weight on the
        // session or a system service is not something to get wrong by a
        // bug somewhere else.
        if !app_unit(unit) {
            return Err(Error::NotAnApp);
        }
        self.with_conn(async |conn| {
            let props = vec![("CPUWeight", Value::U64(weight))];
            conn.call_method(
                Some(DEST),
                PATH,
                Some(MANAGER_IF),
                "SetUnitProperties",
                &(unit, true, props),
            )
            .await
            .map(drop)
        })
    }
}

/// A session bus connection with the user's manager on it.
async fn connect() -> Option<Connection> {
    let conn = zbus::connection::Builder::session()
        .ok()?
        .method_timeout(TIMEOUT)
        .build()
        .await
        .ok()?;
    // Asked first: a call to a name nobody owns waits for the timeout.
    let running = conn
        .call_method(Some(BUS), BUS_PATH, Some(BUS), "NameHasOwner", &(DEST,))
        .await
        .ok()?
        .body()
        .deserialize::<bool>()
        .ok()?;
    running.then_some(conn)
}

/// The state file: `$XDG_RUNTIME_DIR/net.eterneon.atlas.monitor/eased`.
/// The runtime directory lives exactly as long as the session, and so as
/// long as the eases it lists.
pub fn state_file() -> Option<PathBuf> {
    let dir = PathBuf::from(std::env::var_os("XDG_RUNTIME_DIR")?);
    // A relative one would put it wherever Atlas was started.
    dir.is_absolute().then(|| dir.join(STATE_DIR).join("eased"))
}

/// One line per eased unit: `auto|manual <weight to put back> <since,
/// seconds since the epoch> <unit>`.
pub fn format_state(saved: &[Saved]) -> String {
    let mut out = String::new();
    for s in saved {
        let kind = if s.manual { "manual" } else { "auto" };
        out.push_str(&format!("{kind} {} {} {}\n", s.prev, s.since, s.unit));
    }
    out
}

/// The lines of a state file that make sense; the rest are skipped.
pub fn parse_state(text: &str) -> Vec<Saved> {
    text.lines()
        .filter_map(|line| {
            let mut f = line.split(' ');
            let manual = match f.next()? {
                "auto" => false,
                "manual" => true,
                _ => return None,
            };
            let prev: u64 = f.next()?.parse().ok()?;
            // Unset, or a weight systemd takes.
            if prev != UNSET && !(1..=10_000).contains(&prev) {
                return None;
            }
            let since = f.next()?.parse().ok()?;
            let unit = f.next()?;
            (f.next().is_none() && app_unit(unit)).then(|| Saved {
                unit: unit.to_owned(),
                prev,
                manual,
                since,
            })
        })
        .collect()
}

/// The state file's eases: none when there is no file, an error when it
/// can't be read (and so mustn't be written over).
pub(super) fn load_state(path: &Path) -> std::io::Result<Vec<Saved>> {
    use std::io::Read;
    let mut bytes = Vec::new();
    let read = OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NOFOLLOW | libc::O_NONBLOCK)
        .open(path)
        .and_then(|f| f.take(STATE_MAX).read_to_end(&mut bytes));
    match read {
        Ok(_) => Ok(parse_state(&String::from_utf8_lossy(&bytes))),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(Vec::new()),
        Err(e) => Err(e),
    }
}

/// Writes the state file through a temporary file and a rename, or removes
/// it when nothing is eased. A failure is logged and otherwise ignored: the
/// eases themselves stand.
pub(super) fn save_state(path: &Path, saved: &[Saved]) {
    if saved.is_empty() {
        match fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => log::warn!("energy saver: removing {}: {e}", path.display()),
        }
        return;
    }
    if let Err(e) = write_state(path, format_state(saved).as_bytes()) {
        log::warn!("energy saver: writing {}: {e}", path.display());
    }
}

fn write_state(path: &Path, body: &[u8]) -> std::io::Result<()> {
    let dir = path.parent().ok_or(std::io::ErrorKind::InvalidInput)?;
    fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let tmp = dir.join(".eased.tmp");
    let mut f = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&tmp)?;
    f.write_all(body)?;
    drop(f);
    fs::rename(&tmp, path)
}
