//! The system's services from systemd over the system bus: the list with
//! each unit's state and whether it starts at boot, one unit's details, and
//! Start, Stop, Restart, Enable and Disable. Sockets, timers, paths, mounts,
//! automounts, swaps and targets come with them (see [`KINDS`]); the page
//! shows them when asked to.
//!
//! Reading needs no privilege. The actions go to systemd as the user, with
//! the message flagged to allow interactive authorization, so systemd asks
//! polkit (`manage-units`, `manage-unit-files`) and polkit's agent asks for
//! the password; without the flag systemd refuses at once. Atlas Monitor
//! adds no privilege of its own (DESIGN.md, Privilege).
//!
//! Listing is costly for PID 1 in two places, so the reader keeps what it
//! can between reads:
//!
//! - Whether each service starts at boot comes from the unit files
//!   (`ListUnitFilesByPatterns`), and systemd works that out by walking
//!   every unit directory for every file: about 240 ms of PID 1's time per
//!   call on the dev machine. The reader lists the files once, and again
//!   after systemd says they changed (`UnitFilesChanged`, sent on enable,
//!   disable, mask and the like, and `Reloading`, on a daemon-reload), or
//!   every 10 minutes in case a signal was lost.
//! - Listing units by pattern goes through every unit systemd has loaded
//!   (576 here, most of them devices, mounts and sockets): about 20 ms of
//!   PID 1's time, whatever the pattern matches. Listing by name costs it
//!   under 1 ms for all 231 services. So every unit is listed once a minute,
//!   and in between the loaded services are listed by name, with `UnitNew`
//!   and `UnitRemoved` keeping the names.
//!
//! The failed services are listed again only after systemd says a service
//! changed (`PropertiesChanged`), or once a minute.
//!
//! systemd sends those signals only while a client is subscribed, so the
//! reader subscribes. The state is eventually right rather than right at
//! once: systemd sends `UnitNew` and `UnitRemoved` a little after the
//! change, so a read just after one can miss it, and the next has it.
//!
//! The list is the loaded services plus the installed ones that aren't
//! loaded but could be switched on or off (a disabled service is often not
//! loaded at all, and is exactly what someone comes to enable). Left out: a
//! name something refers to that has no unit file and isn't running (systemd
//! loads those as `not-found`), templates (`getty@.service`: only their
//! instances run), aliases (the unit is listed under its own name), and
//! unloaded units that can't be enabled (static, masked, generated).
//!
//! The parsers work on the replies as plain values, so the tests feed them
//! replies recorded with `busctl --json`.

use std::collections::{HashMap, HashSet};
use std::fs::File;
use std::future::{Future, poll_fn};
use std::io::Read;
use std::pin::{Pin, pin};
use std::task::{Context, Poll};
use std::time::{Duration, Instant, SystemTime};

use futures_core::Stream;
use zbus::proxy::{CacheProperties, MethodFlags};
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};
use zbus::{Connection, MatchRule, Message, MessageStream};

const DEST: &str = "org.freedesktop.systemd1";
const PATH: &str = "/org/freedesktop/systemd1";
/// Where systemd puts its units' objects.
const UNIT_PATH: &str = "/org/freedesktop/systemd1/unit";
const MANAGER_IF: &str = "org.freedesktop.systemd1.Manager";
const UNIT_IF: &str = "org.freedesktop.systemd1.Unit";
const PROPS_IF: &str = "org.freedesktop.DBus.Properties";
const PEER_IF: &str = "org.freedesktop.DBus.Peer";
const BUS: Option<&str> = Some("org.freedesktop.DBus");
const BUS_PATH: &str = "/org/freedesktop/DBus";
/// The kinds of unit listed: the ones that can be started, stopped or
/// switched on at boot. Devices, slices and scopes are left out: systemd
/// makes and drops them itself, and nothing is done to them by hand.
const KINDS: &[&str] = &[
    "service",
    "socket",
    "timer",
    "path",
    "mount",
    "automount",
    "swap",
    "target",
];
const PATTERN: &[&str] = &[
    "*.service",
    "*.socket",
    "*.timer",
    "*.path",
    "*.mount",
    "*.automount",
    "*.swap",
    "*.target",
];
/// Longest a read call may take. systemd answers in milliseconds; this is
/// for a PID 1 that is busy (a long daemon-reload), so the sampling thread
/// doesn't wait D-Bus's default 25 s.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Longest a whole read may take: two or three calls, normally a few ms.
pub const DEADLINE: Duration = Duration::from_secs(4);
/// How long a reader whose connection broke or timed out first stays
/// quiet; it doubles with each failure in a row, up to [`RETRY_MAX`].
pub const RETRY: Duration = Duration::from_secs(30);
pub const RETRY_MAX: Duration = Duration::from_secs(300);
/// Longest an action waits for systemd's answer, which includes the time
/// someone takes over the password dialog.
pub const AUTH_TIMEOUT: Duration = Duration::from_secs(300);
/// Longest an action waits for the job it queued (a unit's start can take
/// its `TimeoutStartSec`, 90 s by default) before reporting it as still
/// running.
pub const JOB_WAIT: Duration = Duration::from_secs(60);
/// Queue size for the signal streams. They are emptied during every call,
/// so this only has to cover a burst between two polls.
const QUEUE: usize = 256;
/// The most of a unit file read for its description.
const UNIT_FILE_MAX: u64 = 64 * 1024;

/// A property map as systemd sends it.
type Props = HashMap<String, OwnedValue>;

/// A row of `ListUnitsByPatterns`, `a(ssssssouso)`: name, description,
/// load state, active state, sub state, followed, unit path, job ID, job
/// type, job path.
type RawUnit = (
    String,
    String,
    String,
    String,
    String,
    String,
    OwnedObjectPath,
    u32,
    String,
    OwnedObjectPath,
);

/// Whether systemd found and read the unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoadState {
    Loaded,
    /// No unit file: something refers to the name, or the file was removed
    /// while the unit ran.
    NotFound,
    /// The unit file has a setting systemd can't use.
    BadSetting,
    Error,
    Masked,
    Merged,
    Stub,
    /// Not in systemd's memory: listed from its unit file only.
    NotLoaded,
    Other(String),
}

impl LoadState {
    fn parse(s: &str) -> Self {
        match s {
            "loaded" => Self::Loaded,
            "not-found" => Self::NotFound,
            "bad-setting" => Self::BadSetting,
            "error" => Self::Error,
            "masked" => Self::Masked,
            "merged" => Self::Merged,
            "stub" => Self::Stub,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// systemd's high-level state of a unit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActiveState {
    Active,
    Reloading,
    Inactive,
    Failed,
    Activating,
    Deactivating,
    /// Being cleaned (`systemctl clean`).
    Maintenance,
    Refreshing,
    Other(String),
}

impl ActiveState {
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "active" => Self::Active,
            "reloading" => Self::Reloading,
            "inactive" => Self::Inactive,
            "failed" => Self::Failed,
            "activating" => Self::Activating,
            "deactivating" => Self::Deactivating,
            "maintenance" => Self::Maintenance,
            "refreshing" => Self::Refreshing,
            other => Self::Other(other.to_owned()),
        }
    }
}

/// Whether the unit starts at boot, from its unit file (systemctl's
/// "enabled"/"disabled" column).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FileState {
    Enabled,
    /// Enabled in `/run` only, until the next boot (often by a generator).
    EnabledRuntime,
    Linked,
    LinkedRuntime,
    /// Another name for a unit.
    Alias,
    Masked,
    MaskedRuntime,
    /// No `[Install]` section: started by other units, never enabled.
    Static,
    Disabled,
    /// Enabled through another unit (`Also=`), or a template whose instances
    /// are enabled one by one.
    Indirect,
    /// Made by a generator at boot.
    Generated,
    /// Made at run time (`systemd-run`), gone when it stops.
    Transient,
    Bad,
    Other(String),
}

impl FileState {
    pub(crate) fn parse(s: &str) -> Self {
        match s {
            "enabled" => Self::Enabled,
            "enabled-runtime" => Self::EnabledRuntime,
            "linked" => Self::Linked,
            "linked-runtime" => Self::LinkedRuntime,
            "alias" => Self::Alias,
            "masked" => Self::Masked,
            "masked-runtime" => Self::MaskedRuntime,
            "static" => Self::Static,
            "disabled" => Self::Disabled,
            "indirect" => Self::Indirect,
            "generated" => Self::Generated,
            "transient" => Self::Transient,
            "bad" => Self::Bad,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Whether Enable does something for a unit in this state.
    pub fn can_enable(&self) -> bool {
        matches!(self, Self::Disabled)
    }

    /// Whether Disable does something. A unit enabled at run time or
    /// linked stays as it is: Disable removes only `/etc`'s links.
    pub fn can_disable(&self) -> bool {
        matches!(self, Self::Enabled)
    }

    /// Whether this state is worth a row for a unit systemd hasn't loaded.
    fn listed_unloaded(&self) -> bool {
        matches!(
            self,
            Self::Enabled
                | Self::EnabledRuntime
                | Self::Disabled
                | Self::Linked
                | Self::LinkedRuntime
        )
    }
}

/// The state shown as a dot, from the active state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    /// Active with its processes running (or reloading).
    Running,
    /// Active with nothing running: a one-shot that finished, a unit that
    /// only sets something up.
    Active,
    Starting,
    Stopping,
    Stopped,
    Failed,
}

/// One row of the Services list.
#[derive(Debug, Clone, PartialEq)]
pub struct Service {
    /// The unit name, `NetworkManager.service`.
    pub name: String,
    /// The unit's `Description=`; the name when it has none.
    pub description: String,
    pub load: LoadState,
    pub active: ActiveState,
    /// systemd's finer state: `running`, `exited`, `dead`, `auto-restart`...
    pub sub: String,
    /// `None` when the unit has no unit file (a `not-found` unit still
    /// running) or systemd didn't say.
    pub file_state: Option<FileState>,
    /// The type of a job queued for the unit (`start`, `stop`,
    /// `restart`...), while one is.
    pub job: Option<String>,
}

impl Service {
    pub fn status(&self) -> Status {
        Status::of(&self.active, &self.sub)
    }
}

impl Status {
    /// The dot for a unit's active state and sub-state.
    pub fn of(active: &ActiveState, sub: &str) -> Self {
        match active {
            ActiveState::Active if sub == "running" => Status::Running,
            ActiveState::Active => Status::Active,
            ActiveState::Reloading | ActiveState::Refreshing => Status::Running,
            ActiveState::Activating => Status::Starting,
            ActiveState::Deactivating => Status::Stopping,
            ActiveState::Failed => Status::Failed,
            ActiveState::Inactive | ActiveState::Maintenance | ActiveState::Other(_) => {
                Status::Stopped
            }
        }
    }
}

/// One unit's details: what `systemctl status` shows above the log.
#[derive(Debug, Clone, PartialEq)]
pub struct Details {
    pub service: Service,
    /// What the distribution's preset says (`enabled`/`disabled`).
    pub preset: Option<String>,
    /// The unit file.
    pub path: Option<String>,
    /// `man:` pages and URLs.
    pub documentation: Vec<String>,
    /// When the unit entered its current state.
    pub since: Option<SystemTime>,
    pub main_pid: Option<u32>,
    pub tasks: Option<u64>,
    /// Bytes, from the unit's cgroup.
    pub memory: Option<u64>,
    pub cpu_time: Option<Duration>,
    /// How the last run ended when it didn't succeed: `exit-code`,
    /// `signal`, `timeout`, `core-dump`, `start-limit-hit`...
    pub result: Option<String>,
    /// The main process's exit status when it exited with one other than 0.
    pub exit_status: Option<i32>,
    /// Restarts since the unit was started by hand.
    pub restarts: Option<u32>,
    /// Start and Stop are allowed by hand (`RefuseManualStart=` and
    /// `RefuseManualStop=` say otherwise).
    pub can_start: bool,
    pub can_stop: bool,
    /// The sockets, timers and paths that start it.
    pub triggered_by: Vec<String>,
}

/// A unit file from `ListUnitFilesByPatterns`, under its name.
#[derive(Debug, Clone, PartialEq)]
struct UnitFile {
    path: String,
    state: FileState,
}

/// The unit files by name, kept between reads; see the module comment.
#[derive(Default)]
struct Files {
    by_name: HashMap<String, UnitFile>,
    /// Instances (`getty@tty1.service`) have no file of their own: their
    /// state is asked for once and kept with the list.
    instances: HashMap<String, Option<FileState>>,
    /// Descriptions of the units read from their files.
    descriptions: HashMap<String, String>,
    /// The list must be read again before it is used.
    stale: bool,
    /// When the list was last read. It is read again after
    /// [`FILES_EVERY`] even without a signal saying so.
    listed_at: Option<Instant>,
}

/// What the reader keeps between reads; see the module comment.
#[derive(Default)]
struct State {
    files: Files,
    /// The loaded services: from the last full list, then kept by systemd's
    /// `UnitNew` and `UnitRemoved`.
    loaded: HashSet<String>,
    /// When every unit was last listed. `None`: due on the next read.
    listed_at: Option<Instant>,
    /// While every unit is being listed, the `UnitNew` (true) and
    /// `UnitRemoved` signals taken in meanwhile, to apply again over the
    /// answer: some may be newer than it.
    during: Option<Vec<(bool, String)>>,
    /// `Reloading` signals so far, so a list that a reload overtook isn't
    /// taken as fresh.
    reloads: u64,
    /// Changes to services seen so far (and anything else after which they
    /// may have changed), so the failed ones are listed again.
    changes: u64,
    /// The failed services as last listed, when, and [`State::changes`]
    /// then: see [`ServiceReader::failed`].
    failed: Option<(Vec<String>, Instant, u64)>,
}

impl State {
    /// Takes in one of the Manager's signals.
    fn seen(&mut self, msg: &Message) {
        let header = msg.header();
        match header.member().map(|m| m.as_str()) {
            Some("UnitFilesChanged") => self.files.stale = true,
            // A daemon-reload: unit files and units may all have changed.
            Some("Reloading") => {
                self.files.stale = true;
                self.listed_at = None;
                self.changes += 1;
                self.reloads += 1;
            }
            // A unit's state changed: a service's is all that matters to
            // the failed ones. The path is the name escaped, `.` as `_2e`.
            Some("PropertiesChanged") => {
                if header
                    .path()
                    .is_some_and(|p| p.as_str().ends_with("_2eservice"))
                {
                    self.changes += 1;
                }
            }
            Some(member @ ("UnitNew" | "UnitRemoved")) => {
                let Ok((id, _)) = msg.body().deserialize::<(String, OwnedObjectPath)>() else {
                    return;
                };
                if kind(&id).is_none() {
                    return;
                }
                if kind(&id) == Some("service") {
                    self.changes += 1;
                }
                let new = member == "UnitNew";
                if let Some(during) = &mut self.during {
                    during.push((new, id.clone()));
                }
                self.unit(new, id);
            }
            _ => {}
        }
    }

    fn unit(&mut self, new: bool, id: String) {
        if new {
            self.loaded.insert(id);
        } else {
            self.loaded.remove(&id);
        }
    }

    /// The failed services as last listed, if no service has changed since
    /// and they are under [`FAILED_EVERY`] old at `now`.
    fn listed_failed(&self, now: Instant) -> Option<&[String]> {
        let (names, at, changes) = self.failed.as_ref()?;
        (*changes == self.changes && now.saturating_duration_since(*at) < FAILED_EVERY)
            .then_some(names.as_slice())
    }

    /// Takes in what a signal stream gave: a signal, or an error when one
    /// was lost, after which everything is read again.
    fn take(&mut self, msg: zbus::Result<Message>) {
        match msg {
            Ok(msg) => self.seen(&msg),
            Err(_) => {
                self.files.stale = true;
                self.listed_at = None;
                self.changes += 1;
            }
        }
    }
}

/// The connection and the signal streams: one for each signal in
/// [`SIGNALS`], and one for the units' `PropertiesChanged`.
struct Live {
    conn: Connection,
    signals: Vec<MessageStream>,
}

/// The Manager signals the reader follows.
const SIGNALS: &[&str] = &["UnitFilesChanged", "Reloading", "UnitNew", "UnitRemoved"];
/// How often every unit is listed again, in case a signal was lost: the bus
/// drops signals for a client that falls far behind, as a reader nobody
/// reads for a long time can.
pub const FULL_LIST_EVERY: Duration = Duration::from_secs(60);
/// How often the failed services are listed again with no signal saying a
/// service changed, for the same reason.
pub const FAILED_EVERY: Duration = Duration::from_secs(60);
/// How often the unit files are listed again for the same reason. Rarer:
/// it costs PID 1 about 240 ms.
pub const FILES_EVERY: Duration = Duration::from_secs(600);

/// A connection to systemd for reading. Construct and use it on the
/// sampling thread (not inside a tokio runtime): it runs its own
/// single-threaded one, driven only while a read is in progress. Actions
/// don't go through it: see [`act`].
pub struct ServiceReader {
    // An Option so Drop can let go of it inside the runtime, and so a
    // broken connection can be dropped and made again.
    live: Option<Live>,
    state: State,
    /// When a reader without a connection may try again.
    retry_at: Option<Instant>,
    /// The quiet period the next failure gets.
    retry: Duration,
    rt: tokio::runtime::Runtime,
}

impl ServiceReader {
    /// Connects to the system bus. `None` when there is no system bus or no
    /// systemd on it.
    pub fn new() -> Option<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .thread_keep_alive(Duration::from_secs(1))
            .build()
            .ok()?;
        let live = rt.block_on(async {
            tokio::time::timeout(DEADLINE, connect())
                .await
                .ok()
                .flatten()
        })?;
        Some(Self {
            live: Some(live),
            state: State {
                files: Files {
                    stale: true,
                    ..Files::default()
                },
                ..State::default()
            },
            retry_at: None,
            retry: RETRY,
            rt,
        })
    }

    /// The services, failed ones first, then by name. `None` when systemd
    /// doesn't answer (see [`ServiceReader::failed`] for the back-off).
    pub fn list(&mut self) -> Option<Vec<Service>> {
        self.read(async |live, state| list(live, state).await)
    }

    /// The names of the failed services, and nothing else. It lists the
    /// loaded services as [`ServiceReader::list`] does, without the unit
    /// files, and only when a service has changed since the last time
    /// (systemd's `PropertiesChanged`, `UnitNew` or `UnitRemoved` for one)
    /// or [`FAILED_EVERY`] has passed. Otherwise a read is one round trip,
    /// so it can be asked every few seconds: a list costs PID 1 and the
    /// reader about 2.5 ms each.
    ///
    /// A read that times out drops the connection, and every read in the
    /// next [`RETRY`] is `None` at once; the one after connects again, and
    /// each failure in a row doubles the wait, up to [`RETRY_MAX`]. A
    /// connection that breaks is made again at once, once: the bus
    /// disconnects a client that let its signals pile up, as a reader
    /// nobody reads for hours does.
    pub fn failed(&mut self) -> Option<Vec<String>> {
        self.read(async |live, state| failed(live, state).await)
    }

    /// One unit's details. `None` for a name that isn't one of the listed
    /// kinds (see [`valid_name`]), one systemd doesn't know, and when systemd
    /// doesn't answer.
    pub fn details(&mut self, name: &str) -> Option<Details> {
        if !valid_name(name) {
            return None;
        }
        self.read(async |live, state| details(live, state, name).await)
            .flatten()
    }

    /// Reads the unit files again on the next [`ServiceReader::list`]. For
    /// after an action: systemd's signal says the same, but only once it
    /// has been read.
    pub fn invalidate(&mut self) {
        self.state.files.stale = true;
        self.state.changes += 1;
    }

    /// Whether there is a connection, making one when the back-off allows.
    fn connected(&mut self) -> bool {
        if self.live.is_some() {
            return true;
        }
        if self.retry_at.is_some_and(|t| Instant::now() < t) {
            return false;
        }
        self.live = self.rt.block_on(async {
            tokio::time::timeout(DEADLINE, connect())
                .await
                .ok()
                .flatten()
        });
        if self.live.is_none() {
            self.back_off();
            return false;
        }
        // Changes while there was no connection went unseen.
        self.state.files.stale = true;
        self.state.listed_at = None;
        self.state.changes += 1;
        true
    }

    /// Runs `read` on the connection. `None`, after dropping the
    /// connection and backing off, when it timed out or the bus failed.
    /// A connection that was already up and failed without timing out is
    /// made again and the read tried once more at once (see
    /// [`ServiceReader::failed`]).
    fn read<T>(
        &mut self,
        mut read: impl AsyncFnMut(&mut Live, &mut State) -> zbus::Result<T>,
    ) -> Option<T> {
        for first in [true, false] {
            let fresh = self.live.is_none();
            if !self.connected() {
                return None;
            }
            let Self {
                live, state, rt, ..
            } = self;
            let live = live.as_mut()?;
            let reading =
                rt.block_on(async { tokio::time::timeout(DEADLINE, read(live, state)).await });
            match reading {
                Ok(Ok(value)) => {
                    self.retry = RETRY;
                    return Some(value);
                }
                // A new connection doesn't help when systemd refused.
                Ok(Err(zbus::Error::Failure(e))) if e == REFUSED => {
                    self.disconnect();
                    self.back_off();
                    return None;
                }
                Ok(Err(_)) if first && !fresh => self.disconnect(),
                // Timed out, or the bus failed rather than systemd answering.
                _ => {
                    self.disconnect();
                    self.back_off();
                    return None;
                }
            }
        }
        None
    }

    fn disconnect(&mut self) {
        // zbus's socket reader is a task on this runtime.
        let _guard = self.rt.enter();
        self.live = None;
        // Left set by a full list that timed out.
        self.state.during = None;
    }

    fn back_off(&mut self) {
        self.retry_at = Some(Instant::now() + self.retry);
        self.retry = (self.retry * 2).min(RETRY_MAX);
    }
}

impl Drop for ServiceReader {
    fn drop(&mut self) {
        self.disconnect();
    }
}

async fn connect() -> Option<Live> {
    let conn = zbus::connection::Builder::system()
        .ok()?
        .method_timeout(TIMEOUT)
        .build()
        .await
        .ok()?;
    // Asked first rather than found out from a failed call: a call to a
    // name nobody owns waits for the timeout.
    let running = conn
        .call_method(BUS, BUS_PATH, BUS, "NameHasOwner", &(DEST,))
        .await
        .ok()
        .and_then(|m| m.body().deserialize::<bool>().ok())
        == Some(true);
    if !running {
        return None;
    }
    // The matches go in before Subscribe, so no signal falls between.
    let mut signals = Vec::with_capacity(SIGNALS.len());
    for member in SIGNALS {
        signals.push(signal_stream(&conn, member).await?);
    }
    signals.push(changes_stream(&conn).await?);
    conn.call_method(Some(DEST), PATH, Some(MANAGER_IF), "Subscribe", &())
        .await
        .ok()?;
    Some(Live { conn, signals })
}

/// The Manager's signal `member`, queued from now on.
async fn signal_stream(conn: &Connection, member: &str) -> Option<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DEST)
        .ok()?
        .path(PATH)
        .ok()?
        .interface(MANAGER_IF)
        .ok()?
        .member(member)
        .ok()?
        .build();
    MessageStream::for_match_rule(rule, conn, Some(QUEUE))
        .await
        .ok()
}

/// Every unit's `PropertiesChanged` for its Unit interface, queued from now
/// on. systemd sends one for each change of a unit's state, and another for
/// the kind's own interface, left out here. None at all while nothing
/// changes: two minutes of an idle desktop had not one.
async fn changes_stream(conn: &Connection) -> Option<MessageStream> {
    let rule = MatchRule::builder()
        .msg_type(zbus::message::Type::Signal)
        .sender(DEST)
        .ok()?
        .path_namespace(UNIT_PATH)
        .ok()?
        .interface(PROPS_IF)
        .ok()?
        .member("PropertiesChanged")
        .ok()?
        .arg(0, UNIT_IF)
        .ok()?
        .build();
    MessageStream::for_match_rule(rule, conn, Some(QUEUE))
        .await
        .ok()
}

/// Runs `fut` while taking every message that arrives on `streams`, handing
/// each to `seen` (an error in place of one that was lost). zbus stops
/// reading the socket while a match queue is full, which would hold up the
/// reply `fut` waits for; this keeps the queues empty.
async fn draining<F: Future>(
    streams: &mut [MessageStream],
    mut seen: impl FnMut(zbus::Result<Message>),
    fut: F,
) -> F::Output {
    let mut fut = pin!(fut);
    let mut batch = Vec::new();
    poll_fn(|cx: &mut Context<'_>| {
        for stream in streams.iter_mut() {
            while let Poll::Ready(Some(msg)) = Pin::new(&mut *stream).poll_next(cx) {
                batch.push(msg);
            }
        }
        // Each stream keeps systemd's order, but `UnitRemoved` and a
        // `UnitNew` after it are on two: the serials put them back in
        // order. What is in the streams is all that came before it, as
        // they are emptied every time.
        batch.sort_by_key(|m| m.as_ref().ok().map(|m| m.primary_header().serial_num()));
        for msg in batch.drain(..) {
            seen(msg);
        }
        fut.as_mut().poll(cx)
    })
    .await
}

impl Live {
    /// Calls a Manager or unit method, taking in the signals that arrive
    /// meanwhile. An error reply is `Ok(None)`, and so is a reply that
    /// isn't the expected type; a failure to get any reply is `Err`.
    async fn call<T, B>(
        &mut self,
        state: &mut State,
        path: &str,
        iface: &str,
        method: &str,
        body: &B,
    ) -> zbus::Result<Option<T>>
    where
        T: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type,
        B: serde::Serialize + zbus::zvariant::DynamicType,
    {
        let conn = self.conn.clone();
        let call = conn.call_method(Some(DEST), path, Some(iface), method, body);
        let reply = draining(&mut self.signals, |m| state.take(m), call).await;
        match reply {
            Ok(msg) => Ok(msg.body().deserialize::<T>().ok()),
            Err(zbus::Error::MethodError(..)) => Ok(None),
            Err(e) => Err(e),
        }
    }

    /// A round trip to systemd: every signal it sent before answering has
    /// then been taken in. Costs PID 1 nothing measurable. Not every
    /// change is in yet: systemd sends `UnitNew` and `UnitRemoved` from its
    /// event loop a little after the change, so one may still come after
    /// the answer, and is taken in on the next read.
    async fn sync(&mut self, state: &mut State) -> zbus::Result<()> {
        let conn = self.conn.clone();
        let ping = conn.call_method(Some(DEST), PATH, Some(PEER_IF), "Ping", &());
        draining(&mut self.signals, |m| state.take(m), ping).await?;
        Ok(())
    }
}

/// The loaded units of the listed kinds as systemd has them now.
///
/// Listing by pattern makes systemd go through every unit it has loaded
/// (devices and slices too: 576 here), about 20 ms of PID 1's time
/// whatever the pattern matches. Listing by name costs it about 0.5 ms for
/// the 474 units of the listed kinds. So every unit is listed now and then
/// (see [`FULL_LIST_EVERY`]), and in between the units are listed by name,
/// the names kept by `UnitNew` and `UnitRemoved`. Only loaded names are
/// asked for: systemd loads a unit that is asked for by name.
async fn units(live: &mut Live, state: &mut State) -> zbus::Result<Vec<Unit>> {
    live.sync(state).await?;
    synced_units(live, state).await
}

/// [`units`] after [`Live::sync`].
async fn synced_units(live: &mut Live, state: &mut State) -> zbus::Result<Vec<Unit>> {
    let full = state
        .listed_at
        .is_none_or(|at| at.elapsed() >= FULL_LIST_EVERY);
    let rows: Vec<RawUnit> = if full {
        let none: &[&str] = &[];
        let reloads = state.reloads;
        state.during = Some(Vec::new());
        let reply = live
            .call(
                state,
                PATH,
                MANAGER_IF,
                "ListUnitsByPatterns",
                &(none, PATTERN),
            )
            .await;
        let during = state.during.take().unwrap_or_default();
        let rows: Vec<RawUnit> = answered(reply)?;
        // The signals taken in while waiting can be newer than the answer
        // (they are read off the socket before it), so they go on top, in
        // order.
        state.loaded = rows.iter().map(|u| u.0.clone()).collect();
        for (new, id) in during {
            state.unit(new, id);
        }
        // A reload meanwhile: the answer may be from before it.
        state.listed_at = (state.reloads == reloads).then(Instant::now);
        rows
    } else {
        // A copy: `call` takes in signals that change `loaded`.
        let names: Vec<String> = state.loaded.iter().cloned().collect();
        let reply = live
            .call(state, PATH, MANAGER_IF, "ListUnitsByNames", &(&names,))
            .await;
        answered(reply)?
    };
    Ok(rows.into_iter().map(Unit::from).collect())
}

/// The error [`answered`] gives for a list systemd refused.
const REFUSED: &str = "systemd refused a list";

/// An answer to a list call. systemd answering with an error is a failure
/// here, not an empty list: an empty list would be kept.
fn answered<T>(reply: zbus::Result<Option<T>>) -> zbus::Result<T> {
    reply?.ok_or_else(|| zbus::Error::Failure(REFUSED.into()))
}

/// Lists the unit files again when a signal said they changed, or when
/// [`FILES_EVERY`] has passed.
async fn unit_files_due(live: &mut Live, state: &mut State) -> zbus::Result<()> {
    let due = state.files.stale
        || state
            .files
            .listed_at
            .is_none_or(|at| at.elapsed() >= FILES_EVERY);
    if !due {
        return Ok(());
    }
    // Cleared first: a change signalled while the call waits marks it
    // again.
    state.files.stale = false;
    let none: &[&str] = &[];
    let reply = live
        .call(
            state,
            PATH,
            MANAGER_IF,
            "ListUnitFilesByPatterns",
            &(none, PATTERN),
        )
        .await;
    let rows: Vec<(String, String)> = match answered(reply) {
        Ok(rows) => rows,
        Err(e) => {
            state.files.stale = true;
            return Err(e);
        }
    };
    let files = &mut state.files;
    files.by_name = unit_files(rows);
    files.instances.clear();
    files.descriptions.clear();
    files.listed_at = Some(Instant::now());
    Ok(())
}

async fn list(live: &mut Live, state: &mut State) -> zbus::Result<Vec<Service>> {
    let units = units(live, state).await?;
    // After the round trip in `units`, so `stale` has the signals sent
    // before it.
    unit_files_due(live, state).await?;
    // An instance's state is the template's links for that instance,
    // which only systemd works out.
    for u in &units {
        if is_instance(&u.name)
            && !state.files.by_name.contains_key(&u.name)
            && !state.files.instances.contains_key(&u.name)
        {
            let file_state: Option<String> = live
                .call(
                    state,
                    PATH,
                    MANAGER_IF,
                    "GetUnitFileState",
                    &(u.name.as_str(),),
                )
                .await?;
            state
                .files
                .instances
                .insert(u.name.clone(), file_state.map(|s| FileState::parse(&s)));
        }
    }
    let files = &mut state.files;
    let loaded: HashSet<&str> = units.iter().map(|u| u.name.as_str()).collect();
    // Instances come and go (`systemd-coredump@...`): only the listed ones
    // are kept.
    files
        .instances
        .retain(|name, _| loaded.contains(name.as_str()));
    for (name, file) in &files.by_name {
        if !loaded.contains(name.as_str())
            && file.state.listed_unloaded()
            && !files.descriptions.contains_key(name)
        {
            let description = read_description(&file.path).unwrap_or_default();
            files.descriptions.insert(name.clone(), description);
        }
    }
    Ok(merge(units, files))
}

async fn failed(live: &mut Live, state: &mut State) -> zbus::Result<Vec<String>> {
    live.sync(state).await?;
    if let Some(names) = state.listed_failed(Instant::now()) {
        return Ok(names.to_vec());
    }
    // Counted from before the list: a service that changes while it is on
    // its way has the next read list again.
    let (at, changes) = (Instant::now(), state.changes);
    let mut names: Vec<String> = synced_units(live, state)
        .await?
        .into_iter()
        // Services only: a failed mount or socket is shown on the page
        // with every unit type, and isn't a failed service.
        .filter(|u| u.active == "failed" && kind(&u.name) == Some("service"))
        .map(|u| u.name)
        .collect();
    names.sort_by(|a, b| by_name(a, b));
    state.failed = Some((names.clone(), at, changes));
    Ok(names)
}

async fn details(live: &mut Live, state: &mut State, name: &str) -> zbus::Result<Option<Details>> {
    // An unloaded unit is known only from the file list, which may not
    // have been read yet.
    unit_files_due(live, state).await?;
    let unit: Option<OwnedObjectPath> = live
        .call(state, PATH, MANAGER_IF, "GetUnit", &(name,))
        .await?;
    let Some(unit) = unit else {
        // Not loaded: what its file says, if it has one.
        let Some(file) = state.files.by_name.get(name) else {
            return Ok(None);
        };
        let description = state
            .files
            .descriptions
            .get(name)
            .cloned()
            .or_else(|| read_description(&file.path))
            .unwrap_or_default();
        return Ok(Some(unloaded_details(name, file, description)));
    };
    let unit_props: Props = live
        .call(state, unit.as_str(), PROPS_IF, "GetAll", &(UNIT_IF,))
        .await?
        .unwrap_or_default();
    // The kind's own interface (Service, Socket, Mount...) has the result
    // of its last run; a target has none.
    let kind_props: Props = match kind_interface(name) {
        Some(interface) => live
            .call(
                state,
                unit.as_str(),
                PROPS_IF,
                "GetAll",
                &(interface.as_str(),),
            )
            .await?
            .unwrap_or_default(),
        None => Props::default(),
    };
    if unit_props.is_empty() {
        return Ok(None);
    }
    // From the file list when it has the unit, so the details and the
    // list agree.
    let file_state = state
        .files
        .by_name
        .get(name)
        .map(|f| f.state.clone())
        .or_else(|| state.files.instances.get(name).cloned().flatten());
    Ok(Some(parse_details(
        name,
        &unit_props,
        &kind_props,
        file_state,
    )))
}

/// A row of `ListUnitsByPatterns`, the parts used here.
#[derive(Debug, Clone, PartialEq)]
struct Unit {
    name: String,
    description: String,
    load: String,
    active: String,
    sub: String,
    job_type: String,
}

impl From<RawUnit> for Unit {
    fn from(u: RawUnit) -> Self {
        Self {
            name: u.0,
            description: u.1,
            load: u.2,
            active: u.3,
            sub: u.4,
            job_type: u.8,
        }
    }
}

/// The unit files by name. Templates and aliases are left out (see the
/// module comment).
fn unit_files(rows: Vec<(String, String)>) -> HashMap<String, UnitFile> {
    rows.into_iter()
        .filter_map(|(path, state)| {
            let name = path.rsplit('/').next()?.to_owned();
            let state = FileState::parse(&state);
            if is_template(&name) || state == FileState::Alias {
                return None;
            }
            Some((name, UnitFile { path, state }))
        })
        .collect()
}

/// The loaded units and the listed unit files as rows, failed first, then
/// by name.
fn merge(units: Vec<Unit>, files: &Files) -> Vec<Service> {
    let mut out: Vec<Service> = Vec::with_capacity(units.len() + 64);
    let mut seen: HashSet<String> = HashSet::with_capacity(units.len());
    for u in units {
        let load = LoadState::parse(&u.load);
        let active = ActiveState::parse(&u.active);
        // A name with no unit file that nothing runs under.
        if load == LoadState::NotFound && active == ActiveState::Inactive && u.job_type.is_empty() {
            continue;
        }
        let file_state = files
            .by_name
            .get(&u.name)
            .map(|f| f.state.clone())
            .or_else(|| files.instances.get(&u.name).cloned().flatten());
        let description = if u.description.is_empty() {
            u.name.clone()
        } else {
            u.description
        };
        seen.insert(u.name.clone());
        out.push(Service {
            name: u.name,
            description,
            load,
            active,
            sub: u.sub,
            file_state,
            job: (!u.job_type.is_empty()).then_some(u.job_type),
        });
    }
    for (name, file) in &files.by_name {
        if seen.contains(name) || !file.state.listed_unloaded() {
            continue;
        }
        let description = files
            .descriptions
            .get(name)
            .filter(|d| !d.is_empty())
            .cloned()
            .unwrap_or_else(|| name.clone());
        out.push(Service {
            name: name.clone(),
            description,
            load: LoadState::NotLoaded,
            active: ActiveState::Inactive,
            sub: "dead".to_owned(),
            file_state: Some(file.state.clone()),
            job: None,
        });
    }
    out.sort_by(|a, b| {
        let not_failed = |s: &Service| s.active != ActiveState::Failed;
        not_failed(a)
            .cmp(&not_failed(b))
            .then_with(|| by_name(&a.name, &b.name))
    });
    out
}

/// Names compared without case first, so `NetworkManager` sorts among the
/// n's, then byte by byte so the order is total.
fn by_name(a: &str, b: &str) -> std::cmp::Ordering {
    a.bytes()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
        .then_with(|| a.cmp(b))
}

fn unloaded_details(name: &str, file: &UnitFile, description: String) -> Details {
    let description = if description.is_empty() {
        name.to_owned()
    } else {
        description
    };
    Details {
        service: Service {
            name: name.to_owned(),
            description,
            load: LoadState::NotLoaded,
            active: ActiveState::Inactive,
            sub: "dead".to_owned(),
            file_state: Some(file.state.clone()),
            job: None,
        },
        preset: None,
        path: Some(file.path.clone()),
        documentation: Vec::new(),
        since: None,
        main_pid: None,
        tasks: None,
        memory: None,
        cpu_time: None,
        result: None,
        exit_status: None,
        restarts: None,
        can_start: true,
        can_stop: true,
        triggered_by: Vec::new(),
    }
}

/// A loaded unit's details from its `Unit` and `Service` properties.
/// `file_state` is the one from the file list, when it has the unit.
fn parse_details(
    name: &str,
    unit: &Props,
    service: &Props,
    file_state: Option<FileState>,
) -> Details {
    let s = |key: &str| text(unit.get(key)).filter(|v| !v.is_empty());
    let description = s("Description").unwrap_or_else(|| name.to_owned());
    let active = ActiveState::parse(&s("ActiveState").unwrap_or_default());
    let file_state = file_state.or_else(|| s("UnitFileState").map(|v| FileState::parse(&v)));
    // systemd gives u64::MAX for "not counted" (accounting off).
    let counted = |key: &str| uint(service.get(key)).filter(|&n| n != u64::MAX);
    let since = uint(unit.get("StateChangeTimestamp"))
        .filter(|&us| us != 0)
        .map(|us| SystemTime::UNIX_EPOCH + Duration::from_micros(us));
    let result = text(service.get("Result")).filter(|r| !r.is_empty() && r != "success");
    // CLD_EXITED: the process exited with a status (rather than a signal).
    let exit_status = match (
        int(service.get("ExecMainCode")),
        int(service.get("ExecMainStatus")),
    ) {
        (Some(1), Some(status)) if status != 0 => i32::try_from(status).ok(),
        _ => None,
    };
    Details {
        service: Service {
            // Under its own name when asked for by an alias.
            name: s("Id").unwrap_or_else(|| name.to_owned()),
            description,
            load: LoadState::parse(&s("LoadState").unwrap_or_default()),
            active,
            sub: s("SubState").unwrap_or_default(),
            file_state,
            // The unit's `Job` property has no type; the list has it.
            job: None,
        },
        preset: s("UnitFilePreset"),
        path: s("FragmentPath"),
        documentation: strings(unit.get("Documentation")),
        since,
        main_pid: uint(service.get("MainPID"))
            .filter(|&p| p != 0)
            .and_then(|p| u32::try_from(p).ok()),
        tasks: counted("TasksCurrent"),
        memory: counted("MemoryCurrent"),
        cpu_time: counted("CPUUsageNSec").map(Duration::from_nanos),
        result,
        exit_status,
        restarts: uint(service.get("NRestarts")).and_then(|n| u32::try_from(n).ok()),
        can_start: boolean(unit.get("CanStart")).unwrap_or(true),
        can_stop: boolean(unit.get("CanStop")).unwrap_or(true),
        triggered_by: strings(unit.get("TriggeredBy")),
    }
}

/// The `Description=` of a unit file's `[Unit]` section (the last one, as
/// systemd takes it). Drop-ins aren't read: this is only for units systemd
/// hasn't loaded, which it would otherwise describe.
fn read_description(path: &str) -> Option<String> {
    let mut text = String::new();
    File::open(path)
        .ok()?
        .take(UNIT_FILE_MAX)
        .read_to_string(&mut text)
        .ok()?;
    parse_description(&text)
}

pub(crate) fn parse_description(text: &str) -> Option<String> {
    let mut in_unit = false;
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_unit = line == "[Unit]";
        } else if in_unit
            && let Some((key, value)) = line.split_once('=')
            && key.trim() == "Description"
        {
            found = Some(value.trim().to_owned());
        }
    }
    found.filter(|d| !d.is_empty())
}

/// Whether `name` is the name of a unit of one of the listed kinds (see
/// [`KINDS`]): what [`act`] and [`ServiceReader::details`] accept. Paths
/// are refused: `EnableUnitFiles` would link a file from anywhere.
pub fn valid_name(name: &str) -> bool {
    let Some((stem, _)) = name.rsplit_once('.').filter(|_| kind(name).is_some()) else {
        return false;
    };
    name.len() <= 255
        && !stem.is_empty()
        && !stem.starts_with('.')
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":-_.\\@".contains(&b))
}

/// A unit name's kind, `service` for `sshd.service`, when it is one of
/// [`KINDS`].
pub fn kind(name: &str) -> Option<&'static str> {
    let (_, suffix) = name.rsplit_once('.')?;
    KINDS.iter().copied().find(|k| *k == suffix)
}

/// The D-Bus interface of a unit's kind, `org.freedesktop.systemd1.Socket`
/// for a socket. `None` for a target, which has none of its own.
fn kind_interface(name: &str) -> Option<String> {
    let kind = kind(name).filter(|k| *k != "target")?;
    let mut chars = kind.chars();
    let first = chars.next()?.to_ascii_uppercase();
    Some(format!(
        "org.freedesktop.systemd1.{first}{}",
        chars.as_str()
    ))
}

/// Whether a unit may be started, stopped or restarted from here. A target
/// may not: starting `poweroff.target` or `rescue.target` turns the
/// computer off or takes the desktop away, and stopping one stops what it
/// pulls in. Targets can still be switched on or off at boot.
pub fn runs_by_hand(name: &str) -> bool {
    kind(name).is_some_and(|k| k != "target")
}

fn is_template(name: &str) -> bool {
    name.rsplit_once('.')
        .is_some_and(|(stem, _)| stem.ends_with('@'))
}

fn is_instance(name: &str) -> bool {
    name.contains('@') && !is_template(name)
}

/// What an action asks systemd to do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Action {
    Start,
    Stop,
    Restart,
    /// Start at boot.
    Enable,
    /// Don't start at boot.
    Disable,
}

/// How an action went when systemd did it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Outcome {
    Done,
    /// Enable did nothing: the unit has no `[Install]` section, so other
    /// units start it.
    NotEnableable,
    /// The job is still running after [`JOB_WAIT`] (a slow start). The
    /// list shows how it ends.
    StillRunning,
}

/// What `EnableUnitFiles` and `DisableUnitFiles` changed: the kind of
/// change (`symlink`, `unlink`), the link, and what it points to.
type Changes = Vec<(String, String, String)>;

/// How a queued job ended when it didn't succeed (systemd's job result).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JobResult {
    /// The unit failed: its process exited with an error, crashed...
    Failed,
    /// A unit it needs failed.
    Dependency,
    /// It took longer than the unit allows.
    Timeout,
    /// Another job replaced it.
    Canceled,
    Other(String),
}

/// Why an action wasn't done.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ActionError {
    /// Not the name of a unit of a listed kind (see [`valid_name`]), or a
    /// start, stop or restart of a unit that may not have one (see
    /// [`runs_by_hand`]).
    InvalidName,
    /// polkit said no: the password dialog was cancelled, or the user may
    /// not manage services.
    NotAllowed,
    NoSuchUnit,
    Masked,
    /// The job ran and didn't succeed.
    Job(JobResult),
    /// systemd refused, with its message ("Operation refused, unit may not
    /// be stopped manually").
    Refused(String),
    /// No system bus, or no answer in [`AUTH_TIMEOUT`].
    NoAnswer,
}

/// Asks systemd to do `action` to the unit `name`, and waits for the
/// result. Blocks while polkit asks for a password, up to [`AUTH_TIMEOUT`],
/// and then on a start, stop or restart for the job, up to [`JOB_WAIT`]: run
/// it on a thread of its own, never on the sampling thread. It makes its own
/// connection and runtime, gone when it returns.
///
/// Enable and Disable change the links in `/etc` only. Like `systemctl
/// enable` they don't start or stop the unit; unlike it they don't follow
/// with a daemon-reload, which polkit would ask a second password for, and
/// which only matters for a unit's dependencies before the next boot.
pub fn act(name: &str, action: Action) -> Result<Outcome, ActionError> {
    let runs = matches!(action, Action::Start | Action::Stop | Action::Restart);
    if !valid_name(name) || runs && !runs_by_hand(name) {
        return Err(ActionError::InvalidName);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .build()
        .map_err(|_| ActionError::NoAnswer)?;
    let result = rt.block_on(async {
        tokio::time::timeout(AUTH_TIMEOUT + JOB_WAIT, act_async(name, action))
            .await
            .unwrap_or(Err(ActionError::NoAnswer))
    });
    // zbus's socket reader is a task on this runtime; it must end here.
    drop(rt);
    result
}

async fn act_async(name: &str, action: Action) -> Result<Outcome, ActionError> {
    let conn = zbus::connection::Builder::system()
        .map_err(|_| ActionError::NoAnswer)?
        .method_timeout(AUTH_TIMEOUT)
        .build()
        .await
        .map_err(|_| ActionError::NoAnswer)?;
    let manager = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&conn)
        .destination(DEST)
        .and_then(|b| b.path(PATH))
        .and_then(|b| b.interface(MANAGER_IF))
        .map_err(|_| ActionError::NoAnswer)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(|_| ActionError::NoAnswer)?;
    let flags = MethodFlags::AllowInteractiveAuth.into();
    let names = &[name][..];
    match action {
        Action::Enable => {
            let reply: Option<(bool, Changes)> = manager
                .call_with_flags("EnableUnitFiles", flags, &(names, false, false))
                .await
                .map_err(action_error)?;
            let (install_info, changes) = reply.unwrap_or_default();
            if !install_info && changes.is_empty() {
                return Ok(Outcome::NotEnableable);
            }
            Ok(Outcome::Done)
        }
        Action::Disable => {
            let _: Option<Changes> = manager
                .call_with_flags("DisableUnitFiles", flags, &(names, false))
                .await
                .map_err(action_error)?;
            Ok(Outcome::Done)
        }
        Action::Start | Action::Stop | Action::Restart => {
            let method = match action {
                Action::Start => "StartUnit",
                Action::Stop => "StopUnit",
                _ => "RestartUnit",
            };
            // systemd sends JobRemoved to the client that queued the job;
            // the match must be in place before the job can end.
            let Some(mut removed) = signal_stream(&conn, "JobRemoved").await else {
                return Err(ActionError::NoAnswer);
            };
            let mut ended: Vec<(String, String)> = Vec::new();
            let body = (name, "replace");
            let call = manager.call_with_flags(method, flags, &body);
            let job: Option<OwnedObjectPath> = draining(
                std::slice::from_mut(&mut removed),
                |m| {
                    if let Ok(m) = m {
                        ended.extend(job_removed(&m));
                    }
                },
                call,
            )
            .await
            .map_err(action_error)?;
            let job = job.ok_or(ActionError::NoAnswer)?;
            let job = job.as_str();
            let wait = async {
                loop {
                    if let Some((_, result)) = ended.iter().find(|(path, _)| path == job) {
                        return result.clone();
                    }
                    let next = poll_fn(|cx| Pin::new(&mut removed).poll_next(cx)).await;
                    match next {
                        Some(Ok(msg)) => ended.extend(job_removed(&msg)),
                        Some(Err(_)) => {}
                        None => return String::new(),
                    }
                }
            };
            match tokio::time::timeout(JOB_WAIT, wait).await {
                Ok(result) => job_outcome(&result),
                Err(_) => Ok(Outcome::StillRunning),
            }
        }
    }
}

/// The job path and result of a `JobRemoved(u id, o job, s unit, s result)`.
fn job_removed(msg: &Message) -> Option<(String, String)> {
    let (_, job, _, result): (u32, OwnedObjectPath, String, String) =
        msg.body().deserialize().ok()?;
    Some((job.as_str().to_owned(), result))
}

fn job_outcome(result: &str) -> Result<Outcome, ActionError> {
    match result {
        // Skipped: the job didn't apply to the unit's state (stopping a
        // stopped unit), which is what was asked for.
        "done" | "skipped" => Ok(Outcome::Done),
        "failed" => Err(ActionError::Job(JobResult::Failed)),
        "dependency" => Err(ActionError::Job(JobResult::Dependency)),
        "timeout" => Err(ActionError::Job(JobResult::Timeout)),
        "canceled" => Err(ActionError::Job(JobResult::Canceled)),
        // The bus went away before the job ended.
        "" => Err(ActionError::NoAnswer),
        other => Err(ActionError::Job(JobResult::Other(other.to_owned()))),
    }
}

fn action_error(e: zbus::Error) -> ActionError {
    match e {
        zbus::Error::MethodError(name, message, _) => {
            error_from_reply(name.as_str(), message.as_deref())
        }
        _ => ActionError::NoAnswer,
    }
}

/// An action's error reply by its D-Bus error name.
fn error_from_reply(name: &str, message: Option<&str>) -> ActionError {
    match name {
        "org.freedesktop.DBus.Error.AccessDenied"
        | "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired" => ActionError::NotAllowed,
        "org.freedesktop.systemd1.NoSuchUnit"
        | "org.freedesktop.systemd1.LoadFailed"
        | "org.freedesktop.DBus.Error.FileNotFound" => ActionError::NoSuchUnit,
        "org.freedesktop.systemd1.UnitMasked" => ActionError::Masked,
        "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => {
            ActionError::NoAnswer
        }
        _ => ActionError::Refused(message.unwrap_or(name).to_owned()),
    }
}

/// The value inside any variants it is wrapped in.
fn inner<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(v) => inner(v),
        v => v,
    }
}

fn text(v: Option<&OwnedValue>) -> Option<String> {
    match inner(v?) {
        Value::Str(s) => Some(s.as_str().to_owned()),
        _ => None,
    }
}

fn uint(v: Option<&OwnedValue>) -> Option<u64> {
    match inner(v?) {
        Value::U32(n) => Some(u64::from(*n)),
        Value::U64(n) => Some(*n),
        _ => None,
    }
}

fn int(v: Option<&OwnedValue>) -> Option<i64> {
    match inner(v?) {
        Value::I32(n) => Some(i64::from(*n)),
        Value::I64(n) => Some(*n),
        _ => None,
    }
}

fn boolean(v: Option<&OwnedValue>) -> Option<bool> {
    match inner(v?) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn strings(v: Option<&OwnedValue>) -> Vec<String> {
    match v.map(|v| inner(v)) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match inner(v) {
                Value::Str(s) => Some(s.as_str().to_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
