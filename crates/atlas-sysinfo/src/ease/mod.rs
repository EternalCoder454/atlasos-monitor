//! Energy Saver: easing off an application that keeps the processor busy,
//! by hand ("Ease Off") or automatically, and putting it back.
//!
//! **Mechanism.** An eased application's units get a CPU weight of
//! [`EASED_WEIGHT`] against the default 100, set through the user's own
//! systemd manager as a runtime property (`SetUnitProperties`, runtime): it
//! lasts until the unit ends and writes nothing that outlives the session.
//! Never a nice value: an unprivileged process can't lower a nice value
//! again, so an automatic mode built on it could never take a decision back.
//! A weight acts on the whole application, every process it has and every
//! one it starts later, and only matters when something else wants the
//! processor: an eased application on an idle machine runs as fast as
//! before. Only `app-*` units are ever touched ([`system::SystemdWeights`]
//! refuses anything else).
//!
//! **Policy.** An application is eased automatically after [`EASE_AFTER`]
//! above [`HEAVY_PERCENT`] of one core, so a burst (a page load, a build
//! step) never is, and put back after [`RESTORE_AFTER`] below
//! [`CALM_PERCENT`], so one that pulses doesn't flap. Left alone, whatever
//! they do:
//! - an application playing or recording sound, or with the camera open
//!   ([`audio`]), checked right before easing anything and every
//!   [`AUDIO_EVERY`] while something is eased or busy; an eased one that
//!   starts playing is put back. When PipeWire doesn't answer, nothing is
//!   eased: easing the music is the one thing this must never do;
//! - terminals (a busy terminal is a build someone is waiting on);
//! - the one in use: the window with focus, as the desktop reports it
//!   ([`Controller::set_focused`]; on Plasma a KWin script). An eased one
//!   that gets focus is put back, and counts as busy again only from when
//!   it loses focus;
//! - Telamon Monitor itself, and the applications the user listed as never;
//! - one the user put back by hand, for as long as it runs;
//! - one whose weight someone else set. Fedora (and so Telamon OS) runs
//!   uresourced, which on GNOME gives the *focused* application's unit a
//!   weight of 300 and resets it when focus moves (on Plasma it can't see
//!   focus, checked in the Telamon OS VM), and does the same for one playing
//!   sound (seen in the trial, `--example ease -- --trial`): a raised unit
//!   is the one in use ([`Status::KeptInUse`]). Any unit not at the kernel default is someone
//!   else's, and a unit whose weight changed after Atlas eased it is let go
//!   and never restored over: fighting uresourced could strand an
//!   application at 300 or at 10.
//!
//! **Undo.** Turning automatic off, listing an application as never, and
//! [`Controller::shutdown`] (also on drop: the window closed) put back what
//! was eased automatically: once Atlas stops watching, nothing would notice
//! an eased application start playing. What the user eased by hand stays
//! eased, as a choice they made. Every ease is written to a file in the
//! runtime directory ([`system::state_file`]), which lives exactly as long
//! as the session and so as the eases it describes. After a crash,
//! [`Controller::recover`] puts back the automatic ones and takes up the
//! manual ones again, each only where the weight is still Atlas's.
//!
//! The controller is plain logic over three traits ([`Units`], [`Weights`],
//! [`Audio`]) so the tests can drive it with fakes; [`system::open`]
//! assembles the real one. It is owned by the sampling thread, which calls
//! [`Controller::tick`] every [`TICK_EVERY`] while the window is open.

pub mod audio;
pub mod kwin;
pub mod system;

use std::collections::{HashMap, HashSet};
use std::fmt;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

pub use system::open;

/// The CPU weight an eased application gets, against the default of 100:
/// about a tenth of a share when it competes with a normal one, never none.
pub const EASED_WEIGHT: u64 = 10;
/// The kernel's weight for a cgroup nobody set one on.
pub const DEFAULT_WEIGHT: u64 = 100;
/// systemd's "no weight configured" for `CPUWeight`, which an untouched
/// unit reads as on some versions (others read 100).
pub const UNSET: u64 = u64::MAX;

/// Heavy: at least this much of one core, over a tick.
pub const HEAVY_PERCENT: f64 = 50.0;
/// Calm: below this much of one core.
pub const CALM_PERCENT: f64 = 15.0;
/// How long an application must be heavy before it is eased.
pub const EASE_AFTER: Duration = Duration::from_secs(30);
/// How long an eased application must be calm before it is put back.
pub const RESTORE_AFTER: Duration = Duration::from_secs(60);
/// How often sound is checked while something is eased or busy.
pub const AUDIO_EVERY: Duration = Duration::from_secs(10);
/// How often the sampling thread calls [`Controller::tick`]. A tick reads
/// two small held files per application unit.
pub const TICK_EVERY: Duration = Duration::from_secs(5);
/// Applications below this much of one core are left off the page, unless
/// eased.
pub const SHOW_PERCENT: f64 = 5.0;

/// What Energy Saver is doing about an application.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Status {
    Normal,
    /// Heavy, and counting towards [`EASE_AFTER`] (with automatic on).
    Busy,
    EasedAuto,
    EasedManual,
    /// Heavy, but playing or recording.
    KeptSound,
    /// Heavy, but a terminal.
    KeptTerminal,
    /// Heavy, but the user listed it as never.
    KeptNever,
    /// Heavy, but the user put it back by hand while it runs.
    KeptByUser,
    /// Heavy, but the one being used: the window with focus
    /// ([`Controller::set_focused`]), or one whose weight something raised
    /// (uresourced's focused application, on GNOME). One raised for its
    /// sound reads [`KeptSound`](Status::KeptSound).
    KeptInUse,
    /// Heavy, but something else set its weight lower or otherwise.
    KeptOther,
}

impl Status {
    pub fn is_eased(self) -> bool {
        matches!(self, Status::EasedAuto | Status::EasedManual)
    }
}

/// One application as the Energy Saver page shows it.
#[derive(Debug, Clone, PartialEq)]
pub struct Row {
    pub id: Arc<str>,
    pub name: Arc<str>,
    /// One of its units (the first by name), for the app layer's icon
    /// lookup (`apps::Resolver::of`).
    pub unit: Arc<str>,
    /// Percent of one core over the last tick.
    pub cpu: f64,
    pub status: Status,
    /// When it was eased, for the eased statuses.
    pub eased_at: Option<Instant>,
}

/// One unit's reading: cumulative CPU time in microseconds, and the weight
/// the kernel applies to it now (0 when it can't be read).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Sample {
    pub usage: u64,
    pub weight: u64,
}

/// Reads every application unit's CPU time and weight: the cgroup tree
/// ([`system::CgroupUnits`]), or a fake in the tests. `None` is a failed
/// read, which skips the tick.
pub trait Units {
    fn sample(&mut self) -> Option<HashMap<Arc<str>, Sample>>;
}

/// Reads and sets a unit's `CPUWeight` as its service manager has it
/// ([`UNSET`] for none).
pub trait Weights {
    fn weight(&mut self, unit: &str) -> Result<u64, Error>;
    fn set_weight(&mut self, unit: &str, weight: u64) -> Result<(), Error>;
    /// Tries the manager on the next call even while backing off: before
    /// putting weights back, and for the user's own actions.
    fn retry_now(&mut self) {}
}

/// Says which applications are playing or recording, by ID. `None` when
/// that can't be told, which means nothing is eased.
pub trait Audio {
    fn audible(&mut self) -> Option<HashSet<String>>;
}

/// Who a unit's application is.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Identity {
    pub id: Arc<str>,
    pub name: Arc<str>,
    pub terminal: bool,
}

/// Turns a unit into its application; `None` for anything that is not an
/// application's (a D-Bus activated helper, a container's scope).
pub trait Identify {
    fn identify(&mut self, unit: &Arc<str>) -> Option<Identity>;
}

impl Identify for crate::apps::Resolver {
    fn identify(&mut self, unit: &Arc<str>) -> Option<Identity> {
        let app = self.of(Some(unit)).filter(|a| !a.container)?;
        Some(Identity {
            id: app.id.clone(),
            name: app.name.clone(),
            terminal: app.terminal,
        })
    }
}

impl<F: FnMut(&str) -> Option<Identity>> Identify for F {
    fn identify(&mut self, unit: &Arc<str>) -> Option<Identity> {
        self(unit)
    }
}

/// Why a weight could not be read or set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The user's manager didn't answer, or there is no session bus.
    NoAnswer,
    /// The unit has ended.
    Gone,
    /// Not an application's unit: refused before asking.
    NotAnApp,
    /// The manager refused, with its message.
    Refused(String),
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Error::NoAnswer => f.write_str("the service manager did not answer"),
            Error::Gone => f.write_str("the application has closed"),
            Error::NotAnApp => f.write_str("only an application's share can be changed"),
            Error::Refused(m) => f.write_str(m),
        }
    }
}

impl std::error::Error for Error {}

#[derive(Debug)]
struct AppState {
    name: Arc<str>,
    terminal: bool,
    units: HashMap<Arc<str>, UnitState>,
    cpu: f64,
    heavy_for: Duration,
    calm_for: Duration,
    eased: bool,
    manual: bool,
    /// Put back by the user: not eased automatically again while it runs.
    spared: bool,
    since: Option<Instant>,
    /// Something other than Atlas set a weight on one of its units: raised
    /// above the default, or set otherwise. Either way it isn't Atlas's.
    raised: bool,
    foreign: bool,
}

#[derive(Debug)]
struct UnitState {
    usage: u64,
    /// As the kernel last reported it, or as Atlas just set it.
    weight: u64,
    /// First seen this tick: no CPU delta yet.
    fresh: bool,
    eased: bool,
    /// The weight to put back.
    prev: u64,
    /// Someone else set a weight on it after Atlas eased it: it doesn't
    /// rejoin the application's ease. A new ease (after another heavy
    /// spell, or the user's) may take it again.
    let_go: bool,
}

/// One line of the state file: an eased unit and the weight to put back.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Saved {
    pub unit: String,
    pub prev: u64,
    pub manual: bool,
    /// When it was eased, in seconds since the epoch.
    pub since: u64,
}

/// Holds every application seen and carries out the policy. Owned by the
/// sampling thread.
pub struct Controller {
    units: Box<dyn Units>,
    weights: Box<dyn Weights>,
    audio: Box<dyn Audio>,
    /// Telamon Monitor's own application ID, never eased.
    own: Option<Arc<str>>,
    /// Where the eases are written for crash recovery; `None` for nowhere.
    state: Option<PathBuf>,
    automatic: bool,
    never: HashSet<String>,
    apps: HashMap<Arc<str>, AppState>,
    /// Each unit's application, `None` for a unit that is no application's.
    unit_app: HashMap<Arc<str>, Option<Identity>>,
    /// Eases a previous run recorded that aren't settled yet: put back
    /// (automatic), taken up (manual), ended, or someone else's since.
    /// Kept in the state file until then.
    pending: HashMap<String, Saved>,
    last: Option<Instant>,
    last_audio: Option<Instant>,
    audible: HashSet<String>,
    /// The unit of the window with focus, as the desktop last said.
    focused: Option<Arc<str>>,
    shut_down: bool,
}

impl Controller {
    /// A controller that does nothing until [`tick`](Self::tick). `own` is
    /// Telamon Monitor's application ID; `state` the crash-recovery file.
    pub fn new(
        units: Box<dyn Units>,
        weights: Box<dyn Weights>,
        audio: Box<dyn Audio>,
        own: Option<Arc<str>>,
        state: Option<PathBuf>,
    ) -> Self {
        Self {
            units,
            weights,
            audio,
            own,
            state,
            automatic: false,
            never: HashSet::new(),
            apps: HashMap::new(),
            unit_app: HashMap::new(),
            pending: HashMap::new(),
            last: None,
            last_audio: None,
            audible: HashSet::new(),
            focused: None,
            shut_down: false,
        }
    }

    pub fn automatic(&self) -> bool {
        self.automatic
    }

    /// Turns automatic easing on or off. Off puts back everything it eased
    /// by itself; what the user eased stays.
    pub fn set_automatic(&mut self, on: bool) {
        self.automatic = on;
        if !on {
            self.weights.retry_now();
            self.restore_where(|_, a| !a.manual);
        }
    }

    /// Replaces the applications never to ease automatically, by ID. One
    /// already eased automatically is put back at once.
    pub fn set_never<I: IntoIterator<Item = S>, S: Into<String>>(&mut self, ids: I) {
        self.never = ids.into_iter().map(Into::into).collect();
        let never = std::mem::take(&mut self.never);
        self.weights.retry_now();
        self.restore_where(|id, a| !a.manual && never.contains(&**id));
        self.never = never;
    }

    /// Eases an application at the user's request. It stays eased until
    /// [`restore`](Self::restore), whatever it does meanwhile. Nothing for
    /// an application not seen, or Telamon Monitor itself.
    pub fn ease(&mut self, id: &str) -> Result<(), Error> {
        if self.own.as_deref() == Some(id) {
            return Ok(());
        }
        let Some(a) = self.apps.get_mut(id) else {
            return Ok(());
        };
        self.weights.retry_now();
        let result = ease_app(&mut *self.weights, a, self.last, true);
        // A previous run's record of these units is this one's now.
        for (unit, u) in &a.units {
            if u.eased {
                self.pending.remove(&**unit);
            }
        }
        if a.eased {
            a.manual = true;
            a.spared = false;
        }
        self.persist();
        result
    }

    /// Puts an application back, however it was eased. Put back by hand, it
    /// isn't eased automatically again while it runs.
    pub fn restore(&mut self, id: &str) -> Result<(), Error> {
        let Some(a) = self.apps.get_mut(id) else {
            return Ok(());
        };
        self.weights.retry_now();
        let result = restore_app(&mut *self.weights, a);
        a.spared = true;
        self.persist();
        result
    }

    /// Puts back everything eased automatically, and stops: later calls do
    /// nothing. Called on drop too.
    pub fn shutdown(&mut self) {
        if self.shut_down {
            return;
        }
        self.weights.retry_now();
        self.restore_where(|_, a| !a.manual);
        // What a previous run left and this one couldn't settle stays in
        // the state file for the next.
        if self.settle_pending(None) {
            self.persist();
        }
        self.shut_down = true;
    }

    /// The unit of the window with focus, `None` for none or unknown. Its
    /// application is the one being used: not eased automatically, and put
    /// back on the next tick if it was (by hand stays). It counts as busy
    /// again from when it loses focus. uresourced does this by raising the
    /// weight, but only on GNOME; Plasma says through a KWin script.
    pub fn set_focused(&mut self, unit: Option<Arc<str>>) {
        self.focused = unit;
    }

    /// The application of the window with focus, once a tick has seen its
    /// unit.
    fn focused_app(&self) -> Option<Arc<str>> {
        let unit = self.focused.as_ref()?;
        Some(self.unit_app.get(unit)?.as_ref()?.id.clone())
    }

    /// Every application worth showing, busiest first: anything using a
    /// noticeable share of a core, and anything eased, whatever it does.
    /// Telamon Monitor itself is left out.
    pub fn rows(&self) -> Vec<Row> {
        let mut out: Vec<Row> = self
            .apps
            .iter()
            .filter(|(id, a)| (a.cpu >= SHOW_PERCENT || a.eased) && self.own.as_ref() != Some(*id))
            .filter_map(|(id, a)| {
                Some(Row {
                    id: id.clone(),
                    name: a.name.clone(),
                    unit: a.units.keys().min()?.clone(),
                    cpu: a.cpu,
                    status: self.status(id, a),
                    eased_at: a.since.filter(|_| a.eased),
                })
            })
            .collect();
        out.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then_with(|| a.id.cmp(&b.id)));
        out
    }

    fn status(&self, id: &str, a: &AppState) -> Status {
        let focused = self.focused_app();
        match () {
            _ if a.eased && a.manual => Status::EasedManual,
            _ if a.eased => Status::EasedAuto,
            _ if a.cpu < HEAVY_PERCENT => Status::Normal,
            // Before raised: uresourced raises what plays sound too.
            _ if self.audible.contains(id) => Status::KeptSound,
            _ if a.raised => Status::KeptInUse,
            _ if a.foreign => Status::KeptOther,
            _ if self.never.contains(id) => Status::KeptNever,
            _ if a.spared => Status::KeptByUser,
            _ if focused.as_deref() == Some(id) => Status::KeptInUse,
            _ if a.terminal => Status::KeptTerminal,
            _ => Status::Busy,
        }
    }

    /// Reads the units and acts on them. `ident` is the sampling thread's
    /// `apps::Resolver`.
    pub fn tick(&mut self, ident: &mut dyn Identify) {
        self.tick_at(Instant::now(), ident);
    }

    fn tick_at(&mut self, now: Instant, ident: &mut dyn Identify) {
        if self.shut_down {
            return;
        }
        let Some(usage) = self.units.sample() else {
            return;
        };
        let dt = self.last.map(|l| now.saturating_duration_since(l));
        self.last = Some(now);
        let mut changed = false;

        // Units into applications.
        self.unit_app.retain(|u, _| usage.contains_key(u));
        let mut seen = HashSet::new();
        for (unit, s) in &usage {
            let who = self
                .unit_app
                .entry(unit.clone())
                .or_insert_with(|| ident.identify(unit));
            let Some(who) = who else { continue };
            seen.insert(who.id.clone());
            let a = self.apps.entry(who.id.clone()).or_insert_with(|| AppState {
                name: who.name.clone(),
                terminal: who.terminal,
                units: HashMap::new(),
                cpu: 0.0,
                heavy_for: Duration::ZERO,
                calm_for: Duration::ZERO,
                eased: false,
                manual: false,
                spared: false,
                since: None,
                raised: false,
                foreign: false,
            });
            let u = a
                .units
                .entry(unit.clone())
                .and_modify(|u| u.fresh = false)
                .or_insert_with(|| UnitState {
                    usage: s.usage,
                    weight: s.weight,
                    fresh: true,
                    eased: false,
                    prev: UNSET,
                    let_go: false,
                });
            // Eased by hand before a restart: taken up if still at Atlas's
            // weight, else someone else's now. An unreadable weight waits.
            if !u.eased
                && s.weight != 0
                && self.pending.get(&**unit).is_some_and(|p| p.manual)
                && let Some(p) = self.pending.remove(&**unit)
            {
                if s.weight == EASED_WEIGHT {
                    u.eased = true;
                    u.prev = p.prev;
                    a.eased = true;
                    a.manual = true;
                    a.since = Some(instant_of(p.since));
                }
                changed = true;
            }
        }
        changed |= self.settle_pending(Some(&usage));

        // CPU per application, weights set by others, and what has gone. An
        // eased unit that ended is a change too: the state file must stop
        // listing it.
        let weights = &mut *self.weights;
        self.apps.retain(|id, a| {
            if !seen.contains(id) {
                changed |= a.eased;
                return false;
            }
            let (mut busy, mut raised, mut foreign) = (0u64, false, false);
            a.units.retain(|unit, u| {
                let Some(s) = usage.get(unit) else {
                    changed |= u.eased;
                    return false; // that window closed
                };
                if !u.fresh {
                    busy += s.usage.saturating_sub(u.usage);
                }
                u.usage = s.usage;
                u.weight = s.weight;
                // Someone changed a weight Atlas set: uresourced raising the
                // application that got focus, or the user. Theirs wins, and
                // nothing is put back over it. The manager has the last
                // word: the kernel's file can lag a weight just set, and a
                // set that timed out may not have happened.
                if u.eased && s.weight != 0 && s.weight != EASED_WEIGHT {
                    match weights.weight(unit) {
                        Ok(EASED_WEIGHT) => {}
                        Ok(cur) => {
                            u.eased = false;
                            // At the weight it had: Atlas's set never
                            // happened, so it isn't someone else's.
                            u.let_go = kernel(cur) != kernel(u.prev);
                            changed = true;
                        }
                        Err(Error::Gone) => {
                            u.eased = false;
                            changed = true;
                        }
                        // Can't tell: still ours, asked again next tick.
                        Err(_) => {}
                    }
                }
                if !u.eased && s.weight != 0 && s.weight != DEFAULT_WEIGHT {
                    foreign = true;
                    raised |= s.weight > DEFAULT_WEIGHT;
                }
                true
            });
            a.raised = raised;
            a.foreign = foreign;
            if a.eased && !a.units.values().any(|u| u.eased) {
                a.eased = false;
                a.manual = false;
                a.since = None;
                a.heavy_for = Duration::ZERO; // count again, from now
            }
            let dt = dt.unwrap_or_default();
            a.cpu = if dt.is_zero() {
                0.0
            } else {
                busy as f64 / dt.as_micros() as f64 * 100.0
            };
            if a.cpu >= HEAVY_PERCENT {
                a.heavy_for += dt;
                a.calm_for = Duration::ZERO;
            } else if a.cpu < CALM_PERCENT {
                a.calm_for += dt;
                a.heavy_for = Duration::ZERO;
            } else {
                a.heavy_for = Duration::ZERO;
                a.calm_for = Duration::ZERO;
            }
            true
        });

        // A new window of an eased application joins it, from the default
        // weight only (a raised or lowered one is someone else's), and is
        // tried again each tick until it does. After the weights above: the
        // sample predates this ease.
        for a in self.apps.values_mut().filter(|a| a.eased) {
            for (unit, u) in a
                .units
                .iter_mut()
                .filter(|(_, u)| !u.eased && !u.let_go && u.weight == DEFAULT_WEIGHT)
            {
                let _ = ease_unit(&mut *self.weights, unit, u, false);
                changed |= u.eased;
            }
        }

        if self.automatic {
            changed |= self.apply_policy(now);
        }
        if changed {
            self.persist();
        }
    }

    /// Eases what is due and puts back what calmed down or started playing.
    /// Whether anything changed.
    fn apply_policy(&mut self, now: Instant) -> bool {
        // A sound check is always made right before easing, and every
        // AUDIO_EVERY while anything is eased automatically or busy: the
        // first so an eased application is put back soon after it starts
        // playing, the second so the page can say a busy one is kept for
        // its sound rather than about to be eased.
        let focused = self.focused_app();
        if let Some(a) = focused.as_ref().and_then(|f| self.apps.get_mut(f)) {
            a.heavy_for = Duration::ZERO;
        }
        let mut due = Vec::new();
        let mut watch = false;
        for (id, a) in &self.apps {
            if a.eased {
                watch |= !a.manual;
                continue;
            }
            watch |= a.cpu >= HEAVY_PERCENT;
            if a.heavy_for >= EASE_AFTER
                && !a.terminal
                && !a.foreign
                && !a.spared
                && !self.never.contains(&**id)
                && self.own.as_ref() != Some(id)
                && focused.as_ref() != Some(id)
            {
                due.push(id.clone());
            }
        }
        let sound_due = self
            .last_audio
            .is_none_or(|t| now.saturating_duration_since(t) >= AUDIO_EVERY);
        if !due.is_empty() || (watch && sound_due) {
            match self.audio.audible() {
                Some(audible) => {
                    self.audible = audible;
                    self.last_audio = Some(now);
                }
                // Without knowing what is playing, easing anything could be
                // the one thing this must never do. Wait for the next tick.
                None => due.clear(),
            }
        }

        let mut changed = false;
        for id in due {
            if self.audible.contains(&*id) {
                continue;
            }
            if let Some(a) = self.apps.get_mut(&id) {
                let _ = ease_app(&mut *self.weights, a, Some(now), false);
                changed |= a.eased;
            }
        }
        for (id, a) in &mut self.apps {
            if a.eased
                && !a.manual
                && (a.calm_for >= RESTORE_AFTER
                    || self.audible.contains(&**id)
                    || self.never.contains(&**id)
                    || focused.as_ref() == Some(id))
            {
                let _ = restore_app(&mut *self.weights, a);
                changed = true;
            }
        }
        changed
    }

    /// Puts back every eased application `which` picks, then saves.
    fn restore_where(&mut self, which: impl Fn(&Arc<str>, &AppState) -> bool) {
        let mut changed = false;
        for (id, a) in &mut self.apps {
            if a.eased && which(id, a) {
                let _ = restore_app(&mut *self.weights, a);
                changed = true;
            }
        }
        if changed {
            self.persist();
        }
    }

    /// What is eased now, for the state file.
    pub fn saved(&self) -> Vec<Saved> {
        let (wall, mono) = (SystemTime::now(), Instant::now());
        let mut out: Vec<Saved> = self
            .apps
            .values()
            .filter(|a| a.eased)
            .flat_map(|a| {
                let since = a
                    .since
                    .and_then(|s| wall.checked_sub(mono.saturating_duration_since(s)))
                    .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                    .map_or(0, |d| d.as_secs());
                a.units
                    .iter()
                    .filter(|(_, u)| u.eased)
                    .map(move |(unit, u)| Saved {
                        unit: unit.to_string(),
                        prev: u.prev,
                        manual: a.manual,
                        since,
                    })
            })
            .chain(self.pending.values().cloned())
            .collect();
        out.sort_by(|a, b| a.unit.cmp(&b.unit));
        out
    }

    fn persist(&self) {
        if let Some(path) = &self.state {
            system::save_state(path, &self.saved());
        }
    }

    /// Puts back what a previous Telamon Monitor eased automatically and
    /// didn't live to restore, and takes up what the user eased by hand
    /// (on the first tick). Each only where the unit is still at Atlas's
    /// weight: one that has ended needs nothing, and one set otherwise since
    /// is whoever set it's. What can't be settled now (the manager didn't
    /// answer) stays in the state file and is tried again every tick. Call
    /// before the first tick.
    pub fn recover(&mut self) {
        let Some(path) = self.state.clone() else {
            return;
        };
        match system::load_state(&path) {
            Ok(saved) => self
                .pending
                .extend(saved.into_iter().map(|s| (s.unit.clone(), s))),
            Err(e) => {
                // What it lists can't be known, so it isn't written over:
                // this run records nothing.
                log::warn!("energy saver: reading {}: {e}", path.display());
                self.state = None;
                return;
            }
        }
        if self.pending.is_empty() {
            return;
        }
        self.weights.retry_now();
        self.settle_pending(None);
        self.persist();
    }

    /// Settles what a previous run left. An automatic ease is put back
    /// where the unit is still at Atlas's weight. A manual one is taken up
    /// on a tick ([`tick_at`](Self::tick_at)); one still here after a tick
    /// that read its unit's weight belongs to no application, so it is put
    /// back too. Either is dropped once its unit has ended (the manager says
    /// so: a unit missing from one sample may only have been unreadable) or
    /// has a weight someone else set. Whether anything settled.
    fn settle_pending(&mut self, sample: Option<&HashMap<Arc<str>, Sample>>) -> bool {
        let before = self.pending.len();
        let weights = &mut *self.weights;
        self.pending.retain(|unit, p| {
            let seen = sample
                .and_then(|s| s.get(unit.as_str()))
                .map_or(0, |s| s.weight);
            if seen != 0 && seen != EASED_WEIGHT {
                return false;
            }
            let put_back = !p.manual || seen == EASED_WEIGHT;
            match weights.weight(unit) {
                Ok(EASED_WEIGHT) if put_back => {
                    !matches!(weights.set_weight(unit, p.prev), Ok(()) | Err(Error::Gone))
                }
                Ok(EASED_WEIGHT) => true,
                Ok(_) | Err(Error::Gone) | Err(Error::NotAnApp) => false,
                Err(_) => true,
            }
        });
        self.pending.len() != before
    }
}

/// The monotonic instant of a wall-clock time in seconds since the epoch,
/// or now for one that can't be placed.
fn instant_of(since: u64) -> Instant {
    let now = Instant::now();
    UNIX_EPOCH
        .checked_add(Duration::from_secs(since))
        .and_then(|t| SystemTime::now().duration_since(t).ok())
        .and_then(|ago| now.checked_sub(ago))
        .unwrap_or(now)
}

/// A weight as the kernel applies it: unset is the default.
fn kernel(w: u64) -> u64 {
    if w == UNSET { DEFAULT_WEIGHT } else { w }
}

impl Drop for Controller {
    fn drop(&mut self) {
        self.shutdown();
    }
}

/// Lowers every unit of an application. Eased is whatever the units say:
/// if the manager refused one, the application is as eased as the units it
/// accepted.
fn ease_app(
    w: &mut dyn Weights,
    a: &mut AppState,
    now: Option<Instant>,
    by_user: bool,
) -> Result<(), Error> {
    let mut first = Ok(());
    for (unit, u) in &mut a.units {
        if let Err(e) = ease_unit(w, unit, u, by_user)
            && first.is_ok()
        {
            first = Err(e);
        }
    }
    let was = a.eased;
    a.eased = a.units.values().any(|u| u.eased);
    if a.eased && !was {
        a.since = Some(now.unwrap_or_else(Instant::now));
    }
    first
}

/// Lowers one unit. Automatically (`by_user` false) only from the default
/// weight: one someone else set is theirs. The user's request overrides
/// that.
fn ease_unit(
    w: &mut dyn Weights,
    unit: &str,
    u: &mut UnitState,
    by_user: bool,
) -> Result<(), Error> {
    if u.eased {
        return Ok(());
    }
    let mut prev = w.weight(unit)?;
    if !by_user && kernel(prev) != DEFAULT_WEIGHT {
        return Ok(());
    }
    // Already at Atlas's weight (a previous run's, or set by hand): put
    // back, it goes to the default, never stays at 10.
    if prev == EASED_WEIGHT {
        prev = UNSET;
    }
    let result = w.set_weight(unit, EASED_WEIGHT);
    // A set that got no answer may have happened all the same: it is
    // recorded (state file included), and the next tick asks the manager.
    if matches!(result, Ok(()) | Err(Error::NoAnswer)) {
        u.eased = true;
        u.prev = prev;
        u.weight = EASED_WEIGHT;
        u.let_go = false;
    }
    result
}

/// Puts every unit of an application back to the weight it had, only over
/// Atlas's own: a weight changed since stands.
fn restore_app(w: &mut dyn Weights, a: &mut AppState) -> Result<(), Error> {
    let mut first = Ok(());
    for (unit, u) in &mut a.units {
        if !u.eased {
            continue;
        }
        match w.weight(unit) {
            Ok(cur) if cur != EASED_WEIGHT => {
                u.eased = false;
                u.let_go = kernel(cur) != kernel(u.prev);
                continue;
            }
            Err(Error::Gone) => {
                u.eased = false;
                continue;
            }
            _ => {}
        }
        match w.set_weight(unit, u.prev) {
            Ok(()) | Err(Error::Gone) => {
                u.eased = false;
                u.weight = if u.prev == UNSET {
                    DEFAULT_WEIGHT
                } else {
                    u.prev
                };
            }
            Err(e) => {
                if first.is_ok() {
                    first = Err(e);
                }
            }
        }
    }
    a.eased = a.units.values().any(|u| u.eased);
    if !a.eased {
        a.manual = false;
        a.since = None;
    }
    a.heavy_for = Duration::ZERO;
    a.calm_for = Duration::ZERO;
    first
}

#[cfg(test)]
mod tests;
