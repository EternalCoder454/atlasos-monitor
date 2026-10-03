use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;
use std::sync::Arc;
use std::time::Instant;

use super::audio::{Procs, Stream, apps_of, parse_pw_dump};
use super::system::{self, CgroupUnits, app_unit, format_state, parse_state, parse_usage};
use super::*;
use crate::apps::desktop;

/// The session the fakes share: each unit's CPU time and systemd weight,
/// what is playing, and what the manager refuses.
#[derive(Default)]
struct World {
    units: HashMap<String, (u64, u64)>,
    audible: Option<HashSet<String>>,
    audio_calls: u32,
    refuse: HashSet<String>,
    sets: u32,
    /// The manager doesn't answer at all.
    mute: bool,
    /// Units the kernel's files can't be read for just now.
    hidden: HashSet<String>,
    /// Sets get no answer: applied all the same, or not.
    set_unanswered: Option<bool>,
}

impl World {
    fn kernel_weight(w: u64) -> u64 {
        if w == UNSET { DEFAULT_WEIGHT } else { w }
    }
}

type Shared = Rc<RefCell<World>>;

struct FakeUnits(Shared);
impl Units for FakeUnits {
    fn sample(&mut self) -> Option<HashMap<Arc<str>, Sample>> {
        let w = self.0.borrow();
        Some(
            w.units
                .iter()
                .filter(|(u, _)| !w.hidden.contains(*u))
                .map(|(u, &(usage, w))| {
                    (
                        Arc::from(u.as_str()),
                        Sample {
                            usage,
                            weight: World::kernel_weight(w),
                        },
                    )
                })
                .collect(),
        )
    }
}

struct FakeWeights(Shared);
impl Weights for FakeWeights {
    fn weight(&mut self, unit: &str) -> Result<u64, Error> {
        if self.0.borrow().mute {
            return Err(Error::NoAnswer);
        }
        self.0
            .borrow()
            .units
            .get(unit)
            .map(|u| u.1)
            .ok_or(Error::Gone)
    }
    fn set_weight(&mut self, unit: &str, weight: u64) -> Result<(), Error> {
        let mut w = self.0.borrow_mut();
        if w.mute {
            return Err(Error::NoAnswer);
        }
        if w.refuse.contains(unit) {
            return Err(Error::Refused("no".into()));
        }
        w.sets += 1;
        let unanswered = w.set_unanswered;
        let u = w.units.get_mut(unit).ok_or(Error::Gone)?;
        match unanswered {
            None => u.1 = weight,
            Some(applied) => {
                if applied {
                    u.1 = weight;
                }
                return Err(Error::NoAnswer);
            }
        }
        Ok(())
    }
}

struct FakeAudio(Shared);
impl Audio for FakeAudio {
    fn audible(&mut self) -> Option<HashSet<String>> {
        let mut w = self.0.borrow_mut();
        w.audio_calls += 1;
        w.audible.clone()
    }
}

fn ident(unit: &str) -> Option<Identity> {
    let id: Arc<str> = desktop::app_id(unit)?.into();
    Some(Identity {
        name: id.clone(),
        terminal: id.contains("konsole"),
        id,
    })
}

struct Rig {
    world: Shared,
    c: Controller,
    now: Instant,
}

const FIREFOX: &str = "app-org.mozilla.firefox-1.scope";
const FIREFOX_2: &str = "app-org.mozilla.firefox-2.scope";
const ELISA: &str = "app-org.kde.elisa@a1.service";
const KONSOLE: &str = "app-org.kde.konsole@b2.service";
const ATLAS: &str = "app-net.eterneon.atlas.monitor-9.scope";

impl Rig {
    fn new(units: &[&str]) -> Self {
        Self::with_state(units, None)
    }

    fn with_state(units: &[&str], state: Option<std::path::PathBuf>) -> Self {
        let world: Shared = Rc::default();
        {
            let mut w = world.borrow_mut();
            w.audible = Some(HashSet::new());
            for u in units {
                w.units.insert((*u).into(), (0, UNSET));
            }
        }
        let c = Controller::new(
            Box::new(FakeUnits(world.clone())),
            Box::new(FakeWeights(world.clone())),
            Box::new(FakeAudio(world.clone())),
            Some("net.eterneon.atlas.monitor".into()),
            state,
        );
        let mut r = Self {
            world,
            c,
            now: Instant::now(),
        };
        r.c.set_automatic(true);
        r.c.tick_at(r.now, &mut ident);
        r
    }

    /// Runs `secs` seconds in ticks of TICK_EVERY, `busy` using that
    /// percent of a core.
    fn run(&mut self, secs: u64, busy: &[(&str, u64)]) {
        let step = TICK_EVERY.as_secs();
        for _ in 0..secs / step {
            {
                let mut w = self.world.borrow_mut();
                for (unit, pct) in busy {
                    if let Some(u) = w.units.get_mut(*unit) {
                        u.0 += pct * step * 10_000;
                    }
                }
            }
            self.now += TICK_EVERY;
            self.c.tick_at(self.now, &mut ident);
        }
    }

    fn weight(&self, unit: &str) -> u64 {
        self.world.borrow().units[unit].1
    }

    fn status(&self, id: &str) -> Option<Status> {
        self.c
            .rows()
            .into_iter()
            .find(|r| &*r.id == id)
            .map(|r| r.status)
    }
}

#[test]
fn busy_app_is_eased_and_put_back() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(25, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET, "eased before EASE_AFTER");
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::Busy));
    r.run(10, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedAuto));
    let row = r.c.rows().remove(0);
    assert!(row.eased_at.is_some());
    assert!((row.cpu - 90.0).abs() < 0.5, "cpu {}", row.cpu);

    r.run(55, &[]);
    assert_eq!(
        r.weight(FIREFOX),
        EASED_WEIGHT,
        "put back before RESTORE_AFTER"
    );
    r.run(10, &[]);
    assert_eq!(r.weight(FIREFOX), UNSET, "the weight it had, unset");
}

#[test]
fn a_burst_or_middling_load_is_not_eased() {
    let mut r = Rig::new(&[FIREFOX]);
    for _ in 0..10 {
        r.run(20, &[(FIREFOX, 90)]);
        r.run(5, &[(FIREFOX, 30)]); // between calm and heavy: the count restarts
    }
    assert_eq!(r.weight(FIREFOX), UNSET);
}

#[test]
fn sound_is_left_alone() {
    let mut r = Rig::new(&[ELISA, FIREFOX]);
    r.world.borrow_mut().audible = Some(HashSet::from(["org.kde.elisa".into()]));
    r.run(40, &[(ELISA, 90), (FIREFOX, 90)]);
    assert_eq!(r.weight(ELISA), UNSET);
    assert_eq!(r.status("org.kde.elisa"), Some(Status::KeptSound));
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);

    // An eased application that starts playing is put back within
    // AUDIO_EVERY.
    r.world.borrow_mut().audible = Some(HashSet::from(["org.mozilla.firefox".into()]));
    r.run(10, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
}

/// uresourced raises an application playing sound to 300 as well: the
/// page says sound, which is why it is left alone.
#[test]
fn sound_reads_as_sound_when_raised_too() {
    let mut r = Rig::new(&[ELISA]);
    r.world.borrow_mut().units.get_mut(ELISA).unwrap().1 = 300;
    r.world.borrow_mut().audible = Some(HashSet::from(["org.kde.elisa".into()]));
    r.run(40, &[(ELISA, 90)]);
    assert_eq!(r.status("org.kde.elisa"), Some(Status::KeptSound));
    assert_eq!(r.weight(ELISA), 300);
}

#[test]
fn sound_is_known_before_easing() {
    let mut r = Rig::new(&[ELISA]);
    r.world.borrow_mut().audible = Some(HashSet::from(["org.kde.elisa".into()]));
    r.run(5, &[(ELISA, 90)]);
    assert_eq!(r.status("org.kde.elisa"), Some(Status::KeptSound));
}

#[test]
fn no_audio_answer_means_no_easing() {
    let mut r = Rig::new(&[FIREFOX]);
    r.world.borrow_mut().audible = None;
    r.run(120, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert!(r.world.borrow().audio_calls > 0);
}

#[test]
fn audio_is_not_asked_while_nothing_is_busy() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(120, &[(FIREFOX, 10)]);
    assert_eq!(r.world.borrow().audio_calls, 0);
}

#[test]
fn terminals_self_and_never_are_kept() {
    let mut r = Rig::new(&[KONSOLE, ATLAS, ELISA]);
    r.c.set_never(["org.kde.elisa"]);
    r.run(60, &[(KONSOLE, 90), (ATLAS, 90), (ELISA, 90)]);
    for u in [KONSOLE, ATLAS, ELISA] {
        assert_eq!(r.weight(u), UNSET, "{u}");
    }
    assert_eq!(r.status("org.kde.konsole"), Some(Status::KeptTerminal));
    assert_eq!(r.status("org.kde.elisa"), Some(Status::KeptNever));
    assert_eq!(
        r.status("net.eterneon.atlas.monitor"),
        None,
        "Atlas is left off the page"
    );

    // Never, listed while eased, puts it back at once.
    r.c.set_never(Vec::<String>::new());
    r.run(35, &[(ELISA, 90)]);
    assert_eq!(r.weight(ELISA), EASED_WEIGHT);
    r.c.set_never(["org.kde.elisa"]);
    assert_eq!(r.weight(ELISA), UNSET);
}

#[test]
fn manual_ease_is_the_users_to_undo() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(5, &[]);
    r.c.ease("org.mozilla.firefox").unwrap();
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    r.run(120, &[]); // calm, but the user asked
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedManual));
    r.c.set_automatic(false);
    r.c.shutdown();
    assert_eq!(
        r.weight(FIREFOX),
        EASED_WEIGHT,
        "shutdown keeps what the user eased"
    );

    let mut r = Rig::new(&[FIREFOX]);
    r.c.ease("org.mozilla.firefox").unwrap();
    r.c.restore("org.mozilla.firefox").unwrap();
    assert_eq!(r.weight(FIREFOX), UNSET);
}

#[test]
fn put_back_by_hand_is_not_eased_again() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    r.c.restore("org.mozilla.firefox").unwrap();
    r.run(120, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::KeptByUser));
    // The user can still ease it.
    r.c.ease("org.mozilla.firefox").unwrap();
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedManual));
}

#[test]
fn a_failed_manual_ease_leaves_no_trace() {
    let mut r = Rig::new(&[FIREFOX]);
    r.world.borrow_mut().refuse.insert(FIREFOX.into());
    assert!(r.c.ease("org.mozilla.firefox").is_err());
    r.world.borrow_mut().refuse.clear();
    // Eased automatically later, it must be put back automatically too.
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedAuto));
    r.run(65, &[]);
    assert_eq!(r.weight(FIREFOX), UNSET);
}

#[test]
fn a_new_window_of_an_eased_app_is_eased_and_stays() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(35, &[(FIREFOX, 90)]);
    r.world
        .borrow_mut()
        .units
        .insert(FIREFOX_2.into(), (0, UNSET));
    r.run(5, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX_2), EASED_WEIGHT);
    r.run(20, &[(FIREFOX, 90), (FIREFOX_2, 90)]);
    assert_eq!(r.weight(FIREFOX_2), EASED_WEIGHT);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedAuto));
    // And both go back together.
    r.run(65, &[]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert_eq!(r.weight(FIREFOX_2), UNSET);
}

#[test]
fn shutdown_puts_back_automatic_only() {
    let mut r = Rig::new(&[FIREFOX, ELISA]);
    r.c.ease("org.kde.elisa").unwrap();
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    let world = r.world.clone();
    drop(r); // the window closed
    assert_eq!(world.borrow().units[FIREFOX].1, UNSET);
    assert_eq!(world.borrow().units[ELISA].1, EASED_WEIGHT);
}

#[test]
fn partial_refusal() {
    let mut r = Rig::new(&[FIREFOX, FIREFOX_2]);
    r.world.borrow_mut().refuse.insert(FIREFOX_2.into());
    assert!(r.c.ease("org.mozilla.firefox").is_err());
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedManual));
}

#[test]
fn automatic_off_does_nothing() {
    let mut r = Rig::new(&[FIREFOX]);
    r.c.set_automatic(false);
    r.run(120, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert_eq!(r.world.borrow().audio_calls, 0);

    // Turning it off puts back what it eased.
    r.c.set_automatic(true);
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    r.c.set_automatic(false);
    assert_eq!(r.weight(FIREFOX), UNSET);
}

/// uresourced, which AtlasOS runs, raises the focused application's unit to
/// 300 and resets it when focus moves.
#[test]
fn the_focused_app_is_left_alone() {
    let mut r = Rig::new(&[FIREFOX]);
    r.world.borrow_mut().units.get_mut(FIREFOX).unwrap().1 = 300;
    r.run(60, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), 300);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::KeptInUse));

    // Eased, then focused: let go, and not put back over uresourced's 300.
    let mut r = Rig::new(&[FIREFOX]);
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    r.world.borrow_mut().units.get_mut(FIREFOX).unwrap().1 = 300;
    r.run(5, &[(FIREFOX, 90)]);
    r.c.shutdown();
    assert_eq!(r.weight(FIREFOX), 300);
}

/// On Plasma uresourced can't see focus, so the desktop says which window
/// has it (a KWin script) and Atlas keeps that application itself.
#[test]
fn the_window_with_focus_is_left_alone() {
    let mut r = Rig::new(&[FIREFOX]);
    r.c.set_focused(Some(FIREFOX.into()));
    r.run(60, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::KeptInUse));

    // Focus moves on: busy counts from then, not from when it began.
    r.c.set_focused(None);
    r.run(25, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    r.run(10, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);

    // Eased, then focused (another of its windows): put back on the next
    // tick, though still busy.
    r.c.set_focused(Some(FIREFOX_2.into()));
    r.world
        .borrow_mut()
        .units
        .insert(FIREFOX_2.into(), (0, UNSET));
    r.run(5, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert_eq!(r.weight(FIREFOX_2), UNSET);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::KeptInUse));
    assert!(r.c.saved().is_empty());
}

/// What the user eased by hand stays eased when it gets focus.
#[test]
fn focus_leaves_a_manual_ease() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(5, &[(FIREFOX, 90)]);
    r.c.ease("org.mozilla.firefox").unwrap();
    r.c.set_focused(Some(FIREFOX.into()));
    r.run(20, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedManual));
}

#[test]
fn unit_of_a_cgroup_file() {
    let u = |t: &str| system::unit_of_cgroup(t).map(|u| u.to_string());
    let base = "0::/user.slice/user-1000.slice/user@1000.service/app.slice";
    assert_eq!(u(&format!("{base}/{FIREFOX}\n")).as_deref(), Some(FIREFOX));
    // In a sub-slice, and with cgroups of its own below (a Flatpak).
    assert_eq!(
        u(&format!(
            "{base}/app-flatpak.slice/app-flatpak-org.gnome.Maps-7.scope/a/b.scope\n"
        ))
        .as_deref(),
        Some("app-flatpak-org.gnome.Maps-7.scope")
    );
    // Not an application's: the session's own services, or outside app.slice.
    assert_eq!(u(&format!("{base}/dbus-broker.service\n")), None);
    assert_eq!(u("0::/user.slice/user-1000.slice/session-3.scope\n"), None);
    assert_eq!(u("0::/init.scope\n"), None);
    assert_eq!(u(""), None);
    // A v1 line before the unified one is skipped.
    assert_eq!(
        u(&format!("1:name=systemd:/x\n0::{}/{FIREFOX}\n", &base[3..])).as_deref(),
        Some(FIREFOX)
    );
}

#[test]
fn someone_elses_weight_is_not_ours() {
    let mut r = Rig::new(&[FIREFOX]);
    r.world.borrow_mut().units.get_mut(FIREFOX).unwrap().1 = 50;
    r.run(60, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), 50);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::KeptOther));
}

#[test]
fn an_app_that_closes_is_forgotten() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(35, &[(FIREFOX, 90)]);
    r.world.borrow_mut().units.remove(FIREFOX);
    r.run(5, &[]);
    assert!(r.c.rows().is_empty());
    assert!(r.c.saved().is_empty());
}

#[test]
fn the_state_file_follows_the_eases_and_recovers() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("run/eased");
    let mut r = Rig::with_state(&[FIREFOX, ELISA, KONSOLE], Some(path.clone()));
    r.c.ease("org.kde.elisa").unwrap();
    r.run(35, &[(FIREFOX, 90)]);
    let saved = parse_state(&std::fs::read_to_string(&path).unwrap());
    assert_eq!(saved.len(), 2, "{saved:?}");
    assert!(
        saved
            .iter()
            .any(|s| s.unit == FIREFOX && !s.manual && s.prev == UNSET)
    );
    assert!(saved.iter().any(|s| s.unit == ELISA && s.manual));

    // A crash: the controller is gone without putting anything back. The
    // file also lists Konsole, still at 10, and a unit that has ended.
    let world = r.world.clone();
    std::mem::forget(r.c);
    {
        let mut w = world.borrow_mut();
        w.units.get_mut(KONSOLE).unwrap().1 = EASED_WEIGHT;
    }
    let mut lines = std::fs::read_to_string(&path).unwrap();
    lines.push_str(&format!("auto 100 0 {KONSOLE}\n"));
    lines.push_str("auto 100 0 app-gone-1.scope\n");
    std::fs::write(&path, lines).unwrap();

    let mut c = Controller::new(
        Box::new(FakeUnits(world.clone())),
        Box::new(FakeWeights(world.clone())),
        Box::new(FakeAudio(world.clone())),
        None,
        Some(path.clone()),
    );
    c.recover();
    assert_eq!(
        world.borrow().units[FIREFOX].1,
        UNSET,
        "automatic ease put back"
    );
    assert_eq!(world.borrow().units[KONSOLE].1, DEFAULT_WEIGHT);
    assert_eq!(
        world.borrow().units[ELISA].1,
        EASED_WEIGHT,
        "manual ease kept"
    );
    c.tick_at(Instant::now(), &mut ident);
    let rows = c.rows();
    let elisa = rows.iter().find(|r| &*r.id == "org.kde.elisa").unwrap();
    assert_eq!(elisa.status, Status::EasedManual, "taken up again");
    assert!(elisa.eased_at.is_some());
    let saved = parse_state(&std::fs::read_to_string(&path).unwrap());
    assert_eq!(saved.len(), 1);
    assert_eq!(saved[0].unit, ELISA);

    // Put back by hand: the file goes.
    c.restore("org.kde.elisa").unwrap();
    assert!(!path.exists());
}

#[test]
fn recovery_leaves_a_weight_changed_since() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eased");
    std::fs::write(
        &path,
        format!("auto 100 0 {FIREFOX}\nmanual 100 0 {ELISA}\n"),
    )
    .unwrap();
    let world: Shared = Rc::default();
    world.borrow_mut().units.insert(FIREFOX.into(), (0, 300));
    world.borrow_mut().units.insert(ELISA.into(), (0, 40));
    let mut c = Controller::new(
        Box::new(FakeUnits(world.clone())),
        Box::new(FakeWeights(world.clone())),
        Box::new(FakeAudio(world.clone())),
        None,
        Some(path.clone()),
    );
    c.recover();
    c.tick_at(Instant::now(), &mut ident);
    assert_eq!(world.borrow().units[FIREFOX].1, 300);
    assert_eq!(world.borrow().units[ELISA].1, 40);
    assert_eq!(world.borrow().sets, 0);
    assert!(c.rows().is_empty());
    assert!(!path.exists());
}

#[test]
fn state_lines() {
    let saved = vec![
        Saved {
            unit: FIREFOX.into(),
            prev: UNSET,
            manual: false,
            since: 1_790_000_000,
        },
        Saved {
            unit: r"app-claude\x2ddesktop@84a2.service".into(),
            prev: 100,
            manual: true,
            since: 0,
        },
    ];
    assert_eq!(parse_state(&format_state(&saved)), saved);
    let bad = "auto 100 0 dbus.service\nmanual x 0 app-a-1.scope\nauto 1 2 app-a-1.scope extra\n\
               paused 1 2 app-a-1.scope\nauto 1 2 app-a/../x.scope\nauto 100 5 app-ok-1.scope\n";
    let got = parse_state(bad);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].unit, "app-ok-1.scope");
    // Weights systemd wouldn't take; a time far off is still a line.
    let odd = format!(
        "auto 0 1 app-a-1.scope\nauto 10001 1 app-a-1.scope\nauto 10000 {} app-a-1.scope\n",
        u64::MAX
    );
    let got = parse_state(&odd);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].since, u64::MAX);
    let now = Instant::now();
    assert!(instant_of(u64::MAX) >= now);
    assert!(instant_of(0) <= now);
}

#[test]
fn state_file_reads() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eased");
    assert_eq!(system::load_state(&path).unwrap(), vec![]);
    std::fs::write(&path, b"\xff\xfe\nauto 100 5 app-ok-1.scope\n").unwrap();
    assert_eq!(system::load_state(&path).unwrap().len(), 1);
    // Unreadable is not the same as empty.
    assert!(system::load_state(dir.path()).is_err());
}

/// A controller over `world` that recovers from `path`, already holding
/// `lines`.
fn recovering(world: &Shared, path: &std::path::Path, lines: &str) -> Controller {
    std::fs::write(path, lines).unwrap();
    let mut c = Controller::new(
        Box::new(FakeUnits(world.clone())),
        Box::new(FakeWeights(world.clone())),
        Box::new(FakeAudio(world.clone())),
        None,
        Some(path.to_path_buf()),
    );
    c.recover();
    c
}

#[test]
fn recovery_waits_for_the_manager() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eased");
    let world: Shared = Rc::default();
    world
        .borrow_mut()
        .units
        .insert(FIREFOX.into(), (0, EASED_WEIGHT));
    world.borrow_mut().mute = true;
    let mut c = recovering(&world, &path, &format!("auto {UNSET} 0 {FIREFOX}\n"));
    c.tick_at(Instant::now(), &mut ident);
    c.tick_at(Instant::now(), &mut ident);
    assert_eq!(world.borrow().units[FIREFOX].1, EASED_WEIGHT);
    assert_eq!(
        parse_state(&std::fs::read_to_string(&path).unwrap()).len(),
        1
    );
    // Still listed when this run ends without an answer.
    c.shutdown();
    assert!(path.exists());
    drop(c);

    let mut c = recovering(&world, &path, &std::fs::read_to_string(&path).unwrap());
    world.borrow_mut().mute = false;
    c.tick_at(Instant::now(), &mut ident);
    assert_eq!(world.borrow().units[FIREFOX].1, UNSET);
    assert!(!path.exists());
}

#[test]
fn a_manual_ease_that_cant_be_taken_up_is_put_back() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eased");
    let world: Shared = Rc::default();
    world
        .borrow_mut()
        .units
        .insert(ELISA.into(), (0, EASED_WEIGHT));
    let mut c = recovering(&world, &path, &format!("manual 100 0 {ELISA}\n"));
    assert_eq!(world.borrow().units[ELISA].1, EASED_WEIGHT);
    // Not an application this time (the desktop file went, say).
    c.tick_at(Instant::now(), &mut |_: &str| -> Option<Identity> { None });
    assert_eq!(world.borrow().units[ELISA].1, DEFAULT_WEIGHT);
    assert!(!path.exists());
}

#[test]
fn an_unanswered_set_that_happened_is_ours() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eased");
    let mut r = Rig::with_state(&[FIREFOX], Some(path.clone()));
    r.world.borrow_mut().set_unanswered = Some(true);
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::EasedAuto));
    assert_eq!(
        parse_state(&std::fs::read_to_string(&path).unwrap()).len(),
        1
    );
    r.world.borrow_mut().set_unanswered = None;
    r.run(65, &[]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert!(!path.exists());
}

#[test]
fn an_unanswered_set_that_didnt_happen_is_tried_again() {
    let mut r = Rig::new(&[FIREFOX]);
    r.world.borrow_mut().set_unanswered = Some(false);
    r.run(35, &[(FIREFOX, 90)]);
    r.run(5, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), UNSET);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::Busy));
    r.world.borrow_mut().set_unanswered = None;
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
}

#[test]
fn a_new_window_with_its_own_weight_is_not_joined() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(35, &[(FIREFOX, 90)]);
    r.world
        .borrow_mut()
        .units
        .insert(FIREFOX_2.into(), (0, 300));
    r.run(10, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX_2), 300);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
}

#[test]
fn a_window_taken_back_is_not_eased_again() {
    let mut r = Rig::new(&[FIREFOX, FIREFOX_2]);
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX_2), EASED_WEIGHT);
    // Focused, then not: uresourced's 300, then the default.
    r.world.borrow_mut().units.get_mut(FIREFOX_2).unwrap().1 = 300;
    r.run(5, &[(FIREFOX, 90)]);
    r.world.borrow_mut().units.get_mut(FIREFOX_2).unwrap().1 = DEFAULT_WEIGHT;
    r.run(20, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX_2), DEFAULT_WEIGHT);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
}

#[test]
fn only_application_units() {
    for ok in [
        FIREFOX,
        ELISA,
        r"app-youtube\x2dmusic\x2ddesktop\x2dapp-3388142.scope",
        "app-flatpak-com.discordapp.Discord-2370014976.scope",
        "app-dbus-:1.2-org.kde.kdeconnect@0.service",
    ] {
        assert!(app_unit(ok), "{ok}");
    }
    for no in [
        "dbus.service",
        "app.slice",
        "app-foo.slice",
        "plasma-plasmashell.service",
        "app-a/b.scope",
        "app-a b.scope",
        "",
    ] {
        assert!(!app_unit(no), "{no}");
    }
}

#[test]
fn usage_from_cpu_stat() {
    let stat = b"usage_usec 1234567\nuser_usec 1000000\nsystem_usec 234567\nnr_periods 0\n";
    assert_eq!(parse_usage(stat), Some(1_234_567));
    assert_eq!(parse_usage(b"user_usec 5\n"), None);
    assert_eq!(parse_usage(b""), None);
}

#[test]
fn cgroup_units_on_a_tree() {
    let dir = tempfile::tempdir().unwrap();
    let slice = dir.path();
    let mk = |rel: &str, usage: u64, weight: Option<u64>| {
        let d = slice.join(rel);
        std::fs::create_dir_all(&d).unwrap();
        std::fs::write(
            d.join("cpu.stat"),
            format!("usage_usec {usage}\nuser_usec 1\n"),
        )
        .unwrap();
        if let Some(w) = weight {
            std::fs::write(d.join("cpu.weight"), format!("{w}\n")).unwrap();
        }
    };
    mk(FIREFOX, 100, Some(100));
    mk(
        "app-org.kde.foo.slice/app-org.kde.foo@1.service",
        5,
        Some(300),
    );
    mk(
        "app-org.kde.foo.slice/deeper.slice/app-too-deep-1.scope",
        5,
        Some(100),
    );
    mk("dconf.service", 7, None);
    std::fs::write(slice.join("cpu.weight"), "100\n").unwrap();

    let mut u = CgroupUnits::new(slice.to_path_buf());
    let s = u.sample().unwrap();
    assert_eq!(s.len(), 3, "{:?}", s.keys().collect::<Vec<_>>());
    assert_eq!(
        s[FIREFOX],
        Sample {
            usage: 100,
            weight: 100
        }
    );
    assert_eq!(s["app-org.kde.foo@1.service"].weight, 300);
    assert_eq!(s["dconf.service"].weight, 0, "no weight file reads 0");

    // A held file sees a new value; a removed unit is gone.
    mk(FIREFOX, 250, Some(10));
    std::fs::remove_dir_all(slice.join("dconf.service")).unwrap();
    let s = u.sample().unwrap();
    assert_eq!(
        s[FIREFOX],
        Sample {
            usage: 250,
            weight: 10
        }
    );
    assert!(!s.contains_key("dconf.service"));
}

/// A live read where this session has an app.slice (not in CI's container):
/// invariants only.
#[test]
fn live_cgroup_units() {
    let Ok(slice) = system::app_slice() else {
        eprintln!("no app.slice with the cpu controller here; skipping");
        return;
    };
    let mut u = CgroupUnits::new(slice);
    let a = u.sample().unwrap();
    let b = u.sample().unwrap();
    for (unit, s) in &b {
        assert!(unit.ends_with(".scope") || unit.ends_with(".service"));
        if let Some(prev) = a.get(unit) {
            assert!(s.usage >= prev.usage, "{unit} went backwards");
        }
        assert!(s.weight <= 10_000);
    }
}

/// pw-dump's shape for three running streams, with the properties PipeWire
/// sets in a Plasma 6 session: a native client (pw-cat), traceable by its
/// socket's pid; a PulseAudio client (paplay) through pipewire-pulse, whose
/// client object carries pipewire-pulse's own pid; a Flatpak marked with its
/// app ID. Plus a suspended stream and a sink, which don't count.
const PW_DUMP: &str = r#"[
 {"id":160,"type":"PipeWire:Interface:Client","info":{"props":{"application.name":"pw-cat","application.process.binary":"pw-cat","pipewire.sec.pid":680429}}},
 {"id":201,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"application.name":"pw-cat","client.id":160,"media.class":"Stream/Output/Audio"}}},
 {"id":150,"type":"PipeWire:Interface:Client","info":{"props":{"application.name":"paplay","application.process.binary":"pacat","client.api":"pipewire-pulse","pipewire.sec.pid":2927}}},
 {"id":202,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"application.name":"paplay","application.process.binary":"pacat","application.process.id":"680430","client.api":"pipewire-pulse","client.id":150,"media.class":"Stream/Output/Audio"}}},
 {"id":120,"type":"PipeWire:Interface:Client","info":{"props":{"pipewire.access":"flatpak","pipewire.access.portal.app_id":"com.discordapp.Discord","pipewire.sec.pid":3}}},
 {"id":203,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"client.id":120,"media.class":"Stream/Input/Audio"}}},
 {"id":204,"type":"PipeWire:Interface:Node","info":{"state":"suspended","props":{"application.process.id":"555","media.class":"Stream/Output/Audio"}}},
 {"id":205,"type":"PipeWire:Interface:Node","info":{"state":"running","props":{"media.class":"Audio/Sink","node.name":"speakers"}}},
 {"id":0,"type":"PipeWire:Interface:Core","info":null}
]"#;

#[test]
fn streams_from_pw_dump() {
    let got = parse_pw_dump(PW_DUMP.as_bytes()).unwrap();
    assert_eq!(got.len(), 3, "{got:?}");
    let by = |b: &str| got.iter().find(|s| s.binary == b).unwrap();
    assert_eq!(by("pw-cat").trusted, 680429);
    let pulse = by("pacat");
    assert_eq!(pulse.trusted, 0, "pipewire-pulse's pid is not the app's");
    assert_eq!(pulse.claimed, 680430);
    assert_eq!(by("").app_id, "com.discordapp.Discord");
    assert!(parse_pw_dump(b"not json").is_none());
    assert_eq!(parse_pw_dump(b"[]"), Some(vec![]));
}

struct FakeProcs {
    units: HashMap<u32, String>,
    exes: HashMap<u32, String>,
}

impl Procs for FakeProcs {
    fn unit(&self, pid: u32) -> Option<String> {
        self.units.get(&pid).cloned()
    }
    fn exe_name(&self, pid: u32) -> Option<String> {
        self.exes.get(&pid).cloned()
    }
    fn all(&self) -> Vec<(u32, String)> {
        self.units.iter().map(|(p, u)| (*p, u.clone())).collect()
    }
}

#[test]
fn streams_to_apps() {
    let p = FakeProcs {
        units: HashMap::from([
            (100, "app-org.mozilla.firefox@1.service".into()),
            (200, "app-org.kde.elisa@2.service".into()),
            (300, "app-io.mpv.Mpv-3.scope".into()),
            (400, "app-org.kde.konsole@4.service".into()),
            (7, "app-org.gnome.Nautilus@5.service".into()),
        ]),
        exes: HashMap::from([
            (100, "firefox".into()),
            (200, "elisa".into()),
            (300, "mpv".into()),
            (400, "mpv".into()),
            (7, "nautilus".into()),
        ]),
    };
    let s = |trusted, claimed, binary: &str, app_id: &str| Stream {
        trusted,
        claimed,
        binary: binary.into(),
        app_id: app_id.into(),
    };
    let got = apps_of(
        &[
            s(100, 0, "", ""),                     // native, trusted
            s(0, 200, "elisa", ""),                // pulse, and the pid really is elisa
            s(0, 7, "mpv", ""),                    // pulse from a sandbox: 7 isn't mpv here
            s(0, 0, "", "com.discordapp.Discord"), // Flatpak
        ],
        &p,
    );
    for id in [
        "org.mozilla.firefox",
        "org.kde.elisa",
        "io.mpv.Mpv",      // every app running an mpv...
        "org.kde.konsole", // ...the terminal it was started from too
        "com.discordapp.Discord",
    ] {
        assert!(got.contains(id), "{id} not marked: {got:?}");
    }
    assert!(
        !got.contains("org.gnome.Nautilus"),
        "the process that merely has the sandbox pid's number was marked"
    );
}

#[test]
fn rows_are_busiest_first_and_quiet_ones_hidden() {
    let mut r = Rig::new(&[FIREFOX, ELISA, KONSOLE]);
    r.run(5, &[(FIREFOX, 20), (ELISA, 60), (KONSOLE, 2)]);
    let ids: Vec<_> = r.c.rows().into_iter().map(|r| r.id).collect();
    assert_eq!(
        ids,
        vec![Arc::from("org.kde.elisa"), Arc::from("org.mozilla.firefox")]
    );
    assert!(r.c.rows().iter().all(|r| r.cpu <= 100.0 && r.cpu >= 0.0));
}

#[test]
fn an_unreadable_unit_keeps_its_record() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eased");
    let world: Shared = Rc::default();
    world
        .borrow_mut()
        .units
        .insert(FIREFOX.into(), (0, EASED_WEIGHT));
    world
        .borrow_mut()
        .units
        .insert(ELISA.into(), (0, EASED_WEIGHT));
    world.borrow_mut().hidden.insert(ELISA.into());
    world.borrow_mut().mute = true;
    let lines = format!("auto {UNSET} 0 {FIREFOX}\nmanual 100 0 {ELISA}\n");
    let mut c = recovering(&world, &path, &lines);
    world.borrow_mut().mute = false;
    // Firefox missing from a sample, Elisa unreadable: neither is lost.
    world.borrow_mut().hidden = HashSet::from([FIREFOX.into(), ELISA.into()]);
    c.tick_at(Instant::now(), &mut ident);
    assert_eq!(
        world.borrow().units[FIREFOX].1,
        UNSET,
        "put back all the same"
    );
    assert_eq!(world.borrow().units[ELISA].1, EASED_WEIGHT);
    assert_eq!(c.saved().len(), 1);
    world.borrow_mut().hidden.clear();
    c.tick_at(Instant::now(), &mut ident);
    assert_eq!(
        c.rows()
            .iter()
            .find(|r| &*r.id == "org.kde.elisa")
            .map(|r| r.status),
        Some(Status::EasedManual)
    );
    // Ended: the manager's word, and the record goes.
    world.borrow_mut().units.remove(ELISA);
    c.tick_at(Instant::now(), &mut ident);
    assert!(!path.exists());
}

#[test]
fn easing_over_a_previous_runs_ease_puts_back_the_default() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("eased");
    let world: Shared = Rc::default();
    world
        .borrow_mut()
        .units
        .insert(FIREFOX.into(), (0, EASED_WEIGHT));
    world.borrow_mut().mute = true;
    let mut c = recovering(&world, &path, &format!("auto 100 0 {FIREFOX}\n"));
    c.tick_at(Instant::now(), &mut ident);
    world.borrow_mut().mute = false;
    // Eased by hand before the record could be settled: one record, and
    // what it puts back is the default, not 10.
    c.ease("org.mozilla.firefox").unwrap();
    let saved = c.saved();
    assert_eq!(saved.len(), 1, "{saved:?}");
    assert!(saved[0].manual && saved[0].prev == UNSET);
    c.restore("org.mozilla.firefox").unwrap();
    assert_eq!(world.borrow().units[FIREFOX].1, UNSET);
    assert!(!path.exists());
}

#[test]
fn a_window_let_go_is_eased_again_after_another_heavy_spell() {
    let mut r = Rig::new(&[FIREFOX]);
    r.run(35, &[(FIREFOX, 90)]);
    r.world.borrow_mut().units.get_mut(FIREFOX).unwrap().1 = 300;
    r.run(5, &[(FIREFOX, 90)]);
    assert_eq!(r.status("org.mozilla.firefox"), Some(Status::KeptInUse));
    r.world.borrow_mut().units.get_mut(FIREFOX).unwrap().1 = DEFAULT_WEIGHT;
    r.run(35, &[(FIREFOX, 90)]);
    assert_eq!(r.weight(FIREFOX), EASED_WEIGHT);
}
