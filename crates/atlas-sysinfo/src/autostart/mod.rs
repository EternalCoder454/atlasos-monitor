//! What starts at login, for the Startup page: XDG autostart entries and the
//! user's systemd units, each with an on/off switch.
//!
//! **Autostart entries.** Desktop files in `~/.config/autostart` belong to
//! the user; the ones in each `$XDG_CONFIG_DIRS/autostart` (`/etc/xdg` on
//! AtlasOS) are installed for everyone. A file shadows every file of the same
//! name below it: the user's over the system's, and among the system
//! directories the first listed. Switching an entry off writes `Hidden=true`,
//! which the specification says to honour at login: in the user's own file,
//! or in a copy of the system's in the user's directory. The system's file is
//! never touched. Switching it back on removes a copy that only switched it
//! off, so the entry follows the package again.
//!
//! Plasma 6 starts in systemd mode on AtlasOS: at login,
//! `systemd-xdg-autostart-generator` turns each entry into a unit,
//! `app-<name>@autostart.service`, and decides what runs. So this reads the
//! entries the way the generator does: `Hidden`, `OnlyShowIn`/`NotShowIn`
//! against `$XDG_CURRENT_DESKTOP`, `TryExec`, KDE's
//! `X-KDE-autostart-condition`, and `X-systemd-skip`, which says a systemd
//! unit of the desktop's own starts it instead (Plasma's shell, KDE's polkit
//! agent). Hiding a skipped entry changes nothing, so its switch is locked.
//! `X-GNOME-Autostart-enabled` is GNOME's and the generator ignores it. The
//! unit each entry runs as comes from the generator's own output, and its
//! state (running, failed) from the user's systemd manager.
//!
//! **Units.** A unit starts at login when something links it in: the
//! user's or the admin's `.wants` folders (enabled), or a login target's
//! `.wants` in an installed folder or a generator's (KUnifiedPush, drkonqi,
//! a Quadlet), which systemd reports as disabled, static or generated
//! though it starts all the same. So the reader looks through those folders
//! for candidates and asks the user's manager (on the session bus) for just
//! their unit files: listing every unit file costs the manager about 40 ms
//! on the dev machine, a dozen names about 10. Listed are the user's own
//! units (`~/.config/systemd/user`, `~/.local/share/systemd/user`) that can
//! be enabled, the installed ones that start, and the ones the user masked.
//! Left out are installed units that don't start, templates, and the units
//! Plasma's targets pull in by `Wants=`: those are the desktop itself.
//!
//! An own unit is switched with Enable and Disable. An installed unit is
//! enabled for every user (links in `/etc`) or wanted from `/usr/lib`, which
//! the user can't undo, so off masks it for this user and on unmasks it:
//! systemd's own `Mask`, reversible, and in the user's own folder. Neither
//! needs privilege: it is the user's own manager. The manager is reloaded
//! after, as `systemctl` does; a running unit keeps running.
//!
//! **Locks.** Atlas Updater's tray is part of AtlasOS: shown, never switched
//! off. D-Bus, systemd's own and Plasma's units can't be switched off either:
//! the session doesn't start without them. Either can be switched back on.
//!
//! Most of what is listed is plumbing nobody should turn off: on the dev
//! machine, 46 of 49 rows. Those (`NoDisplay` entries, skipped entries, and
//! installed units the user didn't enable by hand) are marked `plumbing`
//! for the page to keep behind a toggle, as the process table keeps back
//! kernel threads.
//!
//! A list reads about 40 small files and makes two calls to the user's
//! manager (the unit files, then every listed unit's state by name): about
//! 10 ms on the dev machine. The page reads it when it opens and after a
//! switch.

use std::collections::{HashMap, HashSet};
use std::ffi::OsStr;
use std::fs::{self, OpenOptions};
use std::io::{self, Read, Write};
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use zbus::proxy::CacheProperties;

use crate::apps::desktop::locale_keys;
use crate::services::{ActiveState, FileState, Status, parse_description};

/// Atlas Updater's tray entry, which can't be switched off.
pub const UPDATER_TRAY: &str = "net.eterneon.atlas.updater-tray.desktop";

const DEST: &str = "org.freedesktop.systemd1";
const PATH: &str = "/org/freedesktop/systemd1";
const MANAGER_IF: &str = "org.freedesktop.systemd1.Manager";
const BUS: &str = "org.freedesktop.DBus";
const BUS_PATH: &str = "/org/freedesktop/DBus";

/// A desktop file or unit file is read up to this much.
const FILE_MAX: u64 = 64 * 1024;
/// One call to the user's manager.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Connecting and both calls of a list.
pub const DEADLINE: Duration = Duration::from_secs(4);
/// A switch: the calls and the manager's reload.
pub const ACT_DEADLINE: Duration = Duration::from_secs(30);
/// The unit types that can start something at login.
const UNIT_TYPES: &[&str] = &[".service", ".socket", ".timer", ".path"];
/// The unit file states asked for. Runtime ones end with the session.
/// Static and generated units count when a login target wants them.
const FILE_STATES: &[&str] = &[
    "enabled",
    "disabled",
    "linked",
    "masked",
    "static",
    "generated",
    "indirect",
];
/// The targets a login reaches, whose `.wants` say what starts.
const LOGIN_TARGETS: &[&str] = &[
    "default.target",
    "basic.target",
    "sockets.target",
    "timers.target",
    "paths.target",
    "graphical-session.target",
    "graphical-session-pre.target",
];

/// Where an item comes from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    /// An XDG autostart desktop file.
    Desktop,
    /// A systemd user unit.
    Unit,
}

/// Whether a switched-on entry starts at login, and if not, why.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Runs {
    Yes,
    /// `OnlyShowIn`/`NotShowIn` leave this desktop out.
    NotThisDesktop,
    /// `TryExec` names a program that isn't installed.
    MissingProgram,
    /// No `Exec`: nothing to start.
    NoCommand,
    /// The app's own setting is off (`X-KDE-autostart-condition`), as with
    /// file indexing switched off in System Settings.
    TurnedOff,
    /// `X-systemd-skip`: a systemd unit of the desktop's own starts it.
    ByUnit,
}

/// Why a switch can't be moved.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Lock {
    /// Part of AtlasOS (Atlas Updater's tray). It can be switched on, not off.
    Required,
    /// The session needs it to start: D-Bus, systemd's and Plasma's units.
    /// It can be switched on (unmasked), not off.
    Session,
    /// Started by a systemd unit (`X-systemd-skip`): the switch would change
    /// nothing either way.
    ByUnit,
}

/// One row of the Startup page.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Item {
    /// The desktop file's name (`org.kde.kdeconnect.daemon.desktop`) or the
    /// unit's (`wireplumber.service`). Unique within a list.
    pub id: String,
    pub kind: Kind,
    /// The entry's `Name`, or the unit's `Description`; the id without its
    /// suffix when there is none.
    pub name: String,
    /// The entry's `Comment`; empty for a unit.
    pub comment: String,
    /// An icon name or absolute path; empty for none (and for units).
    pub icon: String,
    /// The entry's `Exec` as written; empty for a unit.
    pub command: String,
    /// The file in effect: the user's when there is one. For a masked unit,
    /// the installed file when found.
    pub file: PathBuf,
    /// Installed for every user (a system entry, an installed unit), rather
    /// than made by this one.
    pub system: bool,
    /// Desktop plumbing, for behind a toggle: `NoDisplay` entries and the
    /// installed units the user didn't enable by hand.
    pub plumbing: bool,
    /// Switched on: not hidden, enabled, not masked.
    pub enabled: bool,
    pub runs: Runs,
    pub lock: Option<Lock>,
    /// The unit it runs as: the generated `app-…@autostart.service` for an
    /// entry the generator made one for at login, the id for a unit.
    pub unit: Option<String>,
    /// The unit's state, when the manager has it loaded.
    pub status: Option<Status>,
    /// A unit in the user's own directories (Enable/Disable) rather than an
    /// installed one (Mask/Unmask).
    own: bool,
    /// A unit a login target wants through a link that came with it (in
    /// `/usr/lib`, or from a generator): it starts without being enabled.
    wanted: bool,
}

impl Item {
    /// Whether the switch can be moved from where it is.
    pub fn can_switch(&self) -> bool {
        match self.lock {
            None => true,
            Some(Lock::Required | Lock::Session) => !self.enabled,
            Some(Lock::ByUnit) => false,
        }
    }
}

/// The Startup page's list.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct List {
    /// Sorted by name, without case.
    pub items: Vec<Item>,
    /// Whether the user's systemd manager answered. Without it there are no
    /// units, and entries have no state.
    pub units: bool,
}

/// Why a switch failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Error {
    /// The item can't be switched that way (see [`Lock`]).
    Locked,
    /// Not a desktop file name or unit name; paths are refused.
    InvalidName,
    /// The file is gone.
    NotFound,
    /// A unit without an `[Install]` section can't be enabled.
    NotEnableable,
    /// Writing in `~/.config/autostart` failed.
    Io(io::ErrorKind),
    /// The user's manager didn't answer.
    NoAnswer,
    /// The user's manager answered with an error.
    Refused(String),
}

/// Where to look. [`Paths::from_env`] for the session; tests make their own.
#[derive(Debug, Clone, Default)]
pub struct Paths {
    /// `$XDG_CONFIG_HOME`: `autostart` and `systemd/user` are under it.
    pub config_home: PathBuf,
    /// `$XDG_CONFIG_DIRS`, most important first.
    pub config_dirs: Vec<PathBuf>,
    /// `$XDG_DATA_HOME`: units in its `systemd/user` are the user's own.
    pub data_home: PathBuf,
    /// `$XDG_RUNTIME_DIR`, where the generator writes the session's units.
    pub runtime_dir: Option<PathBuf>,
    /// The admin's unit directory, `/etc/systemd/user`: its links enable
    /// units for every user.
    pub admin_units: PathBuf,
    /// The installed unit directories, for what their login targets want
    /// and a masked unit's description.
    pub unit_dirs: Vec<PathBuf>,
    /// `$XDG_CURRENT_DESKTOP`, split.
    pub desktops: Vec<String>,
    /// `$PATH`, for `TryExec`.
    pub path: Vec<PathBuf>,
    /// The `Name[...]` keys to prefer, best first.
    pub locales: Vec<String>,
}

impl Paths {
    pub fn from_env() -> Self {
        let var = |name: &str| std::env::var_os(name).filter(|v| !v.is_empty());
        let absolute = |name: &str| var(name).map(PathBuf::from).filter(|p| p.is_absolute());
        // Not a relative path: the files would be read and written under
        // whatever folder Atlas was started in.
        let home = var("HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .or_else(|| {
                // SAFETY: getuid has no preconditions.
                crate::process::passwd(unsafe { libc::getuid() }).map(|(_, dir)| dir)
            })
            .unwrap_or_default();
        let config_home = absolute("XDG_CONFIG_HOME").unwrap_or_else(|| home.join(".config"));
        let data_home = absolute("XDG_DATA_HOME").unwrap_or_else(|| home.join(".local/share"));
        let mut config_dirs: Vec<PathBuf> = var("XDG_CONFIG_DIRS")
            .map(|v| {
                std::env::split_paths(&v)
                    .filter(|p| p.is_absolute())
                    .collect()
            })
            .unwrap_or_default();
        if config_dirs.is_empty() {
            config_dirs.push("/etc/xdg".into());
        }
        let desktops = std::env::var("XDG_CURRENT_DESKTOP")
            .unwrap_or_default()
            .split(':')
            .filter(|d| !d.is_empty())
            .map(str::to_owned)
            .collect();
        Self {
            config_home,
            config_dirs,
            data_home,
            runtime_dir: absolute("XDG_RUNTIME_DIR"),
            admin_units: "/etc/systemd/user".into(),
            unit_dirs: [
                "/etc/systemd/user",
                "/usr/local/lib/systemd/user",
                "/usr/lib/systemd/user",
                "/usr/share/systemd/user",
            ]
            .map(PathBuf::from)
            .to_vec(),
            desktops,
            path: var("PATH")
                .map(|v| std::env::split_paths(&v).collect())
                .unwrap_or_default(),
            locales: locale_keys(),
        }
    }

    fn user_autostart(&self) -> PathBuf {
        self.config_home.join("autostart")
    }

    fn user_units(&self) -> PathBuf {
        self.config_home.join("systemd/user")
    }

    fn own_unit(&self, path: &Path) -> bool {
        path.starts_with(self.user_units()) || path.starts_with(self.data_home.join("systemd/user"))
    }

    /// Where `systemctl link` put `name` in the user's folders, and the file
    /// it points at.
    fn linked_unit(&self, name: &str) -> Option<(PathBuf, PathBuf)> {
        [self.user_units(), self.data_home.join("systemd/user")]
            .into_iter()
            .filter(|dir| dir.is_absolute())
            .find_map(|dir| {
                let at = dir.join(name);
                let target = fs::read_link(&at).ok()?;
                // A link to /dev/null is a mask, not a linked file.
                (target != Path::new("/dev/null")).then(|| (at, dir.join(target)))
            })
    }
}

/// Everything that starts at login for this session.
pub fn list() -> List {
    let paths = Paths::from_env();
    let items = list_desktop(&paths);
    let units = read_units(&paths, &scan_units(&paths), &items);
    finish(items, units)
}

/// The entries with the units added and every state filled in, sorted.
/// `units` is `None` when the user's manager didn't answer.
fn finish(mut items: Vec<Item>, units: Option<UnitReplies>) -> List {
    let answered = units.is_some();
    if let Some(replies) = units {
        items.extend(replies.items);
        apply_live(&mut items, &replies.live);
    }
    items.sort_by(|a, b| {
        a.name
            .to_lowercase()
            .cmp(&b.name.to_lowercase())
            .then_with(|| a.id.cmp(&b.id))
    });
    List {
        items,
        units: answered,
    }
}

// ---- Autostart entries -------------------------------------------------

/// The keys of a desktop file's `[Desktop Entry]` group that matter here.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Desktop {
    name: Option<String>,
    comment: Option<String>,
    icon: String,
    exec: String,
    try_exec: String,
    no_display: bool,
    hidden: bool,
    skip: bool,
    only_show_in: Option<Vec<String>>,
    not_show_in: Vec<String>,
    /// `X-KDE-autostart-condition`: `file:group:key:default`.
    condition: String,
}

/// Parses the `[Desktop Entry]` group. `None` when the file has none.
/// Booleans are `true` or `false` only, as the specification and the
/// generator have them; a key given twice takes the last value.
fn parse_desktop(text: &str, locales: &[String]) -> Option<Desktop> {
    let mut d = Desktop::default();
    let mut found = false;
    let mut inside = false;
    // The rank of the Name and Comment found so far; lower wins, and
    // locales.len() is the plain key.
    let (mut name_rank, mut comment_rank) = (usize::MAX, usize::MAX);
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            inside = line == "[Desktop Entry]";
            found |= inside;
            continue;
        }
        if !inside {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        let (base, rank) = match key.split_once('[') {
            Some((base, rest)) => {
                let loc = rest.strip_suffix(']').unwrap_or(rest);
                match locales.iter().position(|l| l == loc) {
                    Some(rank) => (base, rank),
                    None => continue,
                }
            }
            None => (key, locales.len()),
        };
        match base {
            "Name" if rank <= name_rank => {
                d.name = Some(value.to_owned());
                name_rank = rank;
            }
            "Comment" if rank <= comment_rank => {
                d.comment = Some(value.to_owned());
                comment_rank = rank;
            }
            _ if rank != locales.len() => {}
            "Icon" => d.icon = value.to_owned(),
            "Exec" => d.exec = value.to_owned(),
            "TryExec" => d.try_exec = value.to_owned(),
            "NoDisplay" => d.no_display = value == "true",
            "Hidden" => d.hidden = value == "true",
            "X-systemd-skip" => d.skip = value == "true",
            "OnlyShowIn" => d.only_show_in = Some(split_list(value)),
            "NotShowIn" => d.not_show_in = split_list(value),
            "X-KDE-autostart-condition" => d.condition = value.to_owned(),
            _ => {}
        }
    }
    found.then_some(d)
}

fn split_list(value: &str) -> Vec<String> {
    value
        .split(';')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect()
}

/// Whether the generator would start this entry, the way it decides.
fn runs(d: &Desktop, paths: &Paths) -> Runs {
    if d.skip {
        return Runs::ByUnit;
    }
    // Compared exactly, as systemd-xdg-autostart-condition does: `kde`
    // is not `KDE`.
    let here = |list: &[String]| list.iter().any(|want| paths.desktops.contains(want));
    if here(&d.not_show_in) || d.only_show_in.as_deref().is_some_and(|only| !here(only)) {
        return Runs::NotThisDesktop;
    }
    if d.exec.is_empty() {
        return Runs::NoCommand;
    }
    if !d.try_exec.is_empty() && !found_program(&d.try_exec, &paths.path) {
        return Runs::MissingProgram;
    }
    if !d.condition.is_empty() && !kde_condition(&d.condition, paths) {
        return Runs::TurnedOff;
    }
    Runs::Yes
}

/// Whether `program` is an executable file: at its path when absolute, else
/// in one of `path`.
fn found_program(program: &str, path: &[PathBuf]) -> bool {
    let executable = |p: &Path| {
        fs::metadata(p).is_ok_and(|m| m.is_file() && m.permissions().mode() & 0o111 != 0)
    };
    if program.starts_with('/') {
        return executable(Path::new(program));
    }
    !program.contains('/') && path.iter().any(|dir| executable(&dir.join(program)))
}

/// Evaluates `X-KDE-autostart-condition` (`file:group:key:default`) as KDE's
/// `kde-systemd-start-condition` does: the boolean `key` in `group` of the
/// config file `file`, the user's first, else `default`.
fn kde_condition(condition: &str, paths: &Paths) -> bool {
    let parts: Vec<&str> = condition.split(':').collect();
    let [file, group @ .., key, default] = parts.as_slice() else {
        return true;
    };
    let group = group.join(":");
    let default = kconfig_bool(default).unwrap_or(true);
    if file.is_empty() || file.contains('/') || key.is_empty() {
        return default;
    }
    std::iter::once(&paths.config_home)
        .chain(&paths.config_dirs)
        .filter(|dir| dir.is_absolute())
        .filter_map(|dir| read_capped(&dir.join(file)))
        .find_map(|text| kconfig_value(&text, &group, key))
        .and_then(|v| kconfig_bool(&v))
        .unwrap_or(default)
}

/// `key`'s value in `[group]` of a KConfig file. Flags such as `[$i]` after a
/// key are ignored.
fn kconfig_value(text: &str, group: &str, key: &str) -> Option<String> {
    let mut inside = false;
    let mut found = None;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            inside = line.strip_prefix('[').and_then(|l| l.strip_suffix(']')) == Some(group);
            continue;
        }
        if !inside {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            let k = k.trim();
            let k = k.split_once("[$").map_or(k, |(k, _)| k);
            if k == key {
                found = Some(v.trim().to_owned());
            }
        }
    }
    found
}

/// KConfig's booleans.
fn kconfig_bool(value: &str) -> Option<bool> {
    match value.trim().to_ascii_lowercase().as_str() {
        "true" | "on" | "yes" | "1" => Some(true),
        "false" | "off" | "no" | "0" => Some(false),
        _ => None,
    }
}

/// A file's text, when it is a regular file of at most [`FILE_MAX`] in
/// UTF-8.
fn read_capped(path: &Path) -> Option<String> {
    read_whole(path).ok().flatten()
}

/// A file's whole text: `None` when nothing is there, an error when
/// something is there but isn't a regular file of at most [`FILE_MAX`] in
/// UTF-8. Opened without blocking, so a FIFO left in a directory can't hang
/// the reader.
fn read_whole(path: &Path) -> Result<Option<String>, Error> {
    let file = match OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
    {
        Ok(file) => file,
        Err(e) if e.kind() == io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(Error::Io(e.kind())),
    };
    if !file.metadata().map_err(|e| Error::Io(e.kind()))?.is_file() {
        return Err(Error::Io(io::ErrorKind::InvalidInput));
    }
    let mut text = String::new();
    file.take(FILE_MAX + 1)
        .read_to_string(&mut text)
        .map_err(|e| Error::Io(e.kind()))?;
    if text.len() as u64 > FILE_MAX {
        return Err(Error::Io(io::ErrorKind::FileTooLarge));
    }
    Ok(Some(text))
}

/// The `.desktop` files in `dir`, parsed, by file name.
fn read_autostart_dir(dir: &Path, locales: &[String]) -> HashMap<String, (PathBuf, Desktop)> {
    let mut out = HashMap::new();
    if !dir.is_absolute() {
        return out;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !valid_desktop_id(&id) {
            continue;
        }
        let path = entry.path();
        if let Some(d) = read_capped(&path).and_then(|t| parse_desktop(&t, locales)) {
            out.insert(id, (path, d));
        }
    }
    out
}

/// Whether `id` is a desktop file name: no path, not hidden.
fn valid_desktop_id(id: &str) -> bool {
    id.len() <= 255
        && id.len() > ".desktop".len()
        && id.ends_with(".desktop")
        && !id.starts_with('.')
        && !id.contains('/')
        && !id.contains('\0')
}

/// The autostart entries, user files over system ones.
fn list_desktop(paths: &Paths) -> Vec<Item> {
    let mut system: HashMap<String, (PathBuf, Desktop)> = HashMap::new();
    for dir in &paths.config_dirs {
        for (id, found) in read_autostart_dir(&dir.join("autostart"), &paths.locales) {
            // The first directory listed wins.
            system.entry(id).or_insert(found);
        }
    }
    let mut user = read_autostart_dir(&paths.user_autostart(), &paths.locales);
    let units = generated_units(paths);

    let mut ids: Vec<String> = system.keys().chain(user.keys()).cloned().collect();
    ids.sort();
    ids.dedup();
    ids.into_iter()
        .map(|id| {
            let sys = system.get(&id);
            let own = user.remove(&id);
            let (file, eff) = match (&own, sys) {
                (Some((p, d)), _) | (None, Some((p, d))) => (p.clone(), d),
                (None, None) => unreachable!("listed from one of them"),
            };
            // A file written only to switch a system entry off carries
            // nothing else: describe it from the system's.
            let marker = own.is_some() && eff.name.is_none() && eff.exec.is_empty();
            let describe = match sys {
                Some((_, s)) if eff.name.is_none() => s,
                _ => eff,
            };
            // Switched on, a marker that hides the entry is removed and the
            // system's runs; one that doesn't hide it has no command.
            let judged = match sys {
                Some((_, s)) if marker && eff.hidden => s,
                _ => eff,
            };
            let updater = id == UPDATER_TRAY;
            let skip = eff.skip || sys.is_some_and(|(_, s)| s.skip);
            // A skipped entry's switch is no switch: behind the toggle,
            // whatever it says.
            let plumbing = !updater && (skip || sys.map_or(eff.no_display, |(_, s)| s.no_display));
            let lock = if updater {
                Some(Lock::Required)
            } else if skip {
                Some(Lock::ByUnit)
            } else {
                None
            };
            Item {
                name: describe
                    .name
                    .clone()
                    .filter(|n| !n.is_empty())
                    .unwrap_or_else(|| id.trim_end_matches(".desktop").to_owned()),
                comment: describe.comment.clone().unwrap_or_default(),
                icon: describe.icon.clone(),
                command: describe.exec.clone(),
                file,
                system: sys.is_some(),
                plumbing,
                enabled: !eff.hidden,
                runs: if skip {
                    Runs::ByUnit
                } else {
                    runs(judged, paths)
                },
                lock,
                unit: units.get(&id).cloned(),
                status: None,
                own: false,
                wanted: false,
                kind: Kind::Desktop,
                id,
            }
        })
        .collect()
}

/// The units the generator made for this session, by desktop file name: its
/// `app-*@autostart.service` files name their entry in `SourcePath=`.
fn generated_units(paths: &Paths) -> HashMap<String, String> {
    let mut out = HashMap::new();
    let Some(dir) = paths
        .runtime_dir
        .as_ref()
        .map(|r| r.join("systemd/generator.late"))
    else {
        return out;
    };
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Some(unit) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if !(unit.starts_with("app-") && unit.ends_with("@autostart.service")) {
            continue;
        }
        let Some(text) = read_capped(&entry.path()) else {
            continue;
        };
        let source = text
            .lines()
            .find_map(|l| l.trim().strip_prefix("SourcePath="));
        if let Some(id) = source
            .and_then(|s| Path::new(s.trim()).file_name())
            .and_then(OsStr::to_str)
        {
            out.insert(id.to_owned(), unit);
        }
    }
    out
}

/// Switches an item on or off. For an entry, from the next login: a running
/// program is left be. For a unit, the manager is reloaded, as `systemctl`
/// does, and a running unit keeps running.
pub fn set_enabled(item: &Item, on: bool) -> Result<(), Error> {
    if !item.can_switch() && item.enabled != on {
        return Err(Error::Locked);
    }
    let paths = Paths::from_env();
    match item.kind {
        Kind::Desktop => set_desktop(&paths, &item.id, on),
        Kind::Unit => set_unit(&paths, &item.id, item.own, item.wanted, on),
    }
}

/// Switches an autostart entry, reading its files again first.
fn set_desktop(paths: &Paths, id: &str, on: bool) -> Result<(), Error> {
    if !valid_desktop_id(id) {
        return Err(Error::InvalidName);
    }
    let user_path = paths.user_autostart().join(id);
    if !user_path.is_absolute() {
        // No home folder: nowhere to write.
        return Err(Error::Io(io::ErrorKind::NotFound));
    }
    // The user's file is rewritten or replaced: one that can't be read
    // whole is left alone rather than lost.
    let user = read_whole(&user_path)?;
    // The system's as the list found it: the first that reads and parses.
    let system = paths.config_dirs.iter().find_map(|dir| {
        let text = read_capped(&dir.join("autostart").join(id))?;
        let d = parse_desktop(&text, &[])?;
        Some((text, d))
    });
    let user_d = user.as_deref().and_then(|t| parse_desktop(t, &[]));
    let (system, system_d) = system.unzip();
    if user_d.is_none() && system_d.is_none() {
        return Err(Error::NotFound);
    }
    if [&user_d, &system_d]
        .iter()
        .any(|d| d.as_ref().is_some_and(|d| d.skip))
        || (id == UPDATER_TRAY && !on)
    {
        return Err(Error::Locked);
    }
    let hidden = !on;
    let write = |text: &str| write_atomic(&user_path, with_hidden(text, hidden).as_bytes());
    match (user, system, system_d) {
        (None, Some(s), Some(sd)) => {
            if sd.hidden == hidden {
                return Ok(());
            }
            write(&s)
        }
        (Some(u), Some(s), Some(sd)) => {
            let marker = user_d
                .as_ref()
                .is_none_or(|d| d.exec.is_empty() && d.name.is_none());
            let copy = marker || with_hidden(&u, false) == with_hidden(&s, false);
            if copy && sd.hidden == hidden {
                // It only switched the entry: hand it back to the package.
                return match fs::remove_file(&user_path) {
                    Err(e) if e.kind() != io::ErrorKind::NotFound => Err(Error::Io(e.kind())),
                    _ => Ok(()),
                };
            }
            write(if marker { &s } else { &u })
        }
        (Some(u), _, _) => write(&u),
        (None, _, _) => Err(Error::NotFound),
    }
}

/// `text` with `Hidden` in its `[Desktop Entry]` group set to `true`, or
/// removed. Other lines are kept as they are.
fn with_hidden(text: &str, hidden: bool) -> String {
    let mut out: Vec<&str> = Vec::new();
    let mut inside = false;
    let mut end = None; // where the main group's last key line ends in `out`
    for line in text.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            inside = trimmed == "[Desktop Entry]";
            if inside && end.is_none() {
                out.push(line);
                end = Some(out.len());
                continue;
            }
        } else if inside {
            if trimmed
                .split_once('=')
                .is_some_and(|(k, _)| k.trim() == "Hidden")
            {
                continue;
            }
            out.push(line);
            if !trimmed.is_empty() && end.is_some() {
                end = Some(out.len());
            }
            continue;
        }
        out.push(line);
    }
    let mut out: Vec<String> = out.into_iter().map(str::to_owned).collect();
    if hidden {
        match end {
            Some(at) => out.insert(at, "Hidden=true".to_owned()),
            None => out.extend(["[Desktop Entry]".to_owned(), "Hidden=true".to_owned()]),
        }
    }
    let mut text = out.join("\n");
    text.push('\n');
    text
}

/// Writes `path` through a file beside it and a rename, so a crash leaves
/// the old file or the new one. A symbolic link at `path` is replaced, never
/// written through: it may point at a system file.
fn write_atomic(path: &Path, body: &[u8]) -> Result<(), Error> {
    let io = |e: io::Error| Error::Io(e.kind());
    let dir = path.parent().ok_or(Error::InvalidName)?;
    fs::create_dir_all(dir).map_err(io)?;
    let name = path.file_name().ok_or(Error::InvalidName)?;
    let tmp = dir.join(format!(
        ".{}.atlas-{}",
        name.to_string_lossy(),
        std::process::id()
    ));
    // Left by a write that crashed in a process with this pid.
    let _ = fs::remove_file(&tmp);
    let result = (|| {
        let mut f = OpenOptions::new()
            .write(true)
            .create_new(true)
            .mode(0o644)
            .open(&tmp)?;
        f.write_all(body)?;
        f.sync_all()?;
        fs::rename(&tmp, path)
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result.map_err(io)
}

// ---- Units -------------------------------------------------------------

/// What the user's manager said, made into rows.
#[derive(Debug, Clone, Default)]
struct UnitReplies {
    items: Vec<Item>,
    /// `ListUnitsByNames`: (name, active state, sub-state).
    live: Vec<(String, String, String)>,
}

/// What the unit folders hold, read without the manager.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
struct Scan {
    /// The names to ask the manager about.
    candidates: Vec<String>,
    /// Linked from a `.wants`, `.requires` or `.upholds` folder of the
    /// user's own: enabled by hand.
    user_wants: HashSet<String>,
    /// Wanted by a login target from an installed folder or a generator's.
    login_wants: HashSet<String>,
}

type RawUnit = (
    String,
    String,
    String,
    String,
    String,
    String,
    zbus::zvariant::OwnedObjectPath,
    u32,
    String,
    zbus::zvariant::OwnedObjectPath,
);
type Changes = Vec<(String, String, String)>;

/// Reads the unit folders for the names worth asking about.
///
/// Listing every unit file costs the user's manager about 40 ms on the dev
/// machine, as it works out the state of every file in every folder; asked
/// for a dozen names, about 10. A unit that starts at login is linked from
/// somewhere: the user's or the admin's folders (enabled, aliased, masked),
/// or a login target's `.wants` in an installed or generated folder. The
/// user's own unit files are candidates whatever their state.
fn scan_units(paths: &Paths) -> Scan {
    let mut scan = Scan::default();
    let mut names = HashSet::new();
    let user_dirs = [paths.user_units(), paths.data_home.join("systemd/user")];
    for dir in user_dirs.iter().chain([&paths.admin_units]) {
        // Unit files, aliases and masks at the top.
        names.extend(unit_entries(dir));
        for (_, linked) in wants_dirs(dir) {
            if dir == &paths.user_units() {
                scan.user_wants.extend(linked.iter().cloned());
            }
            names.extend(linked);
        }
    }
    let generated = paths
        .runtime_dir
        .as_ref()
        .map(|r| r.join("systemd/generator"));
    for dir in paths.unit_dirs.iter().chain(generated.as_ref()) {
        if dir == &paths.admin_units {
            continue;
        }
        for (target, linked) in wants_dirs(dir) {
            if login_target(&target) {
                scan.login_wants.extend(linked.iter().cloned());
                names.extend(linked);
            }
        }
    }
    // The XDG entries list these.
    let xdg = |n: &String| n.starts_with("app-") && n.ends_with("@autostart.service");
    names.retain(|n| !xdg(n));
    scan.login_wants.retain(|n| !xdg(n));
    scan.candidates = names.into_iter().collect();
    scan.candidates.sort();
    scan
}

/// Whether a login reaches `target`: the session's own targets and
/// Plasma's.
fn login_target(target: &str) -> bool {
    LOGIN_TARGETS.contains(&target)
        || (target.starts_with("plasma-") && target.ends_with(".target"))
}

/// The unit names at the top of `dir`, and the names its links point to
/// (an alias names the unit it stands for).
fn unit_entries(dir: &Path) -> Vec<String> {
    let mut out = Vec::new();
    if !dir.is_absolute() {
        return out;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        if !valid_unit_name(&name) {
            continue;
        }
        if let Some(target) = fs::read_link(entry.path())
            .ok()
            .and_then(|t| t.file_name()?.to_str().map(str::to_owned))
            .filter(|t| valid_unit_name(t))
        {
            out.push(target);
        }
        out.push(name);
    }
    out
}

/// The `.wants`, `.requires` and `.upholds` folders in `dir`: the unit that
/// wants, and the unit names linked in it (and the names they point to).
fn wants_dirs(dir: &Path) -> Vec<(String, Vec<String>)> {
    let mut out = Vec::new();
    if !dir.is_absolute() {
        return out;
    }
    let Ok(entries) = fs::read_dir(dir) else {
        return out;
    };
    for entry in entries.flatten() {
        let Ok(name) = entry.file_name().into_string() else {
            continue;
        };
        let Some(wanting) = [".wants", ".requires", ".upholds"]
            .iter()
            .find_map(|s| name.strip_suffix(s))
        else {
            continue;
        };
        out.push((wanting.to_owned(), unit_entries(&entry.path())));
    }
    out
}

/// Asks the user's manager for the candidates' unit files, and for the
/// state of every unit listed. `None` when there is no session bus or
/// manager, or it doesn't answer in [`DEADLINE`].
fn read_units(paths: &Paths, scan: &Scan, entries: &[Item]) -> Option<UnitReplies> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .ok()?;
    let replies = rt.block_on(async {
        tokio::time::timeout(DEADLINE, async {
            let conn = connect(paths).await?;
            // No patterns would mean every unit file.
            let files: Vec<(String, String)> = if scan.candidates.is_empty() {
                Vec::new()
            } else {
                let patterns: Vec<String> =
                    scan.candidates.iter().map(|n| glob_literal(n)).collect();
                let rows = conn
                    .call_method(
                        Some(DEST),
                        PATH,
                        Some(MANAGER_IF),
                        "ListUnitFilesByPatterns",
                        &(FILE_STATES, patterns),
                    )
                    .await
                    .ok()?
                    .body()
                    .deserialize()
                    .ok()?;
                asked_for(rows, &scan.candidates)
            };
            let items = merge_units(paths, &files, scan);
            // By name: listing by pattern walks every loaded unit, devices
            // included. A name not loaded is loaded to answer, then
            // dropped again by the manager.
            let names: Vec<&str> = entries
                .iter()
                .chain(&items)
                .filter_map(|i| i.unit.as_deref())
                .collect();
            let live = if names.is_empty() {
                Vec::new()
            } else {
                let units: Vec<RawUnit> = conn
                    .call_method(
                        Some(DEST),
                        PATH,
                        Some(MANAGER_IF),
                        "ListUnitsByNames",
                        &(names,),
                    )
                    .await
                    .ok()?
                    .body()
                    .deserialize()
                    .ok()?;
                units.into_iter().map(|u| (u.0, u.3, u.4)).collect()
            };
            Some(UnitReplies { items, live })
        })
        .await
        .ok()
        .flatten()
    });
    // zbus's socket reader is a task on this runtime; it ends here.
    drop(rt);
    replies
}

/// `name` as an fnmatch pattern that matches only it: the characters that
/// mean something to fnmatch become `?`.
fn glob_literal(name: &str) -> String {
    name.chars()
        .map(|c| if "\\*?[".contains(c) { '?' } else { c })
        .collect()
}

/// The rows for the names asked for: a `?` in a pattern also matches
/// other names.
fn asked_for(mut rows: Vec<(String, String)>, candidates: &[String]) -> Vec<(String, String)> {
    rows.retain(|(path, _)| {
        Path::new(path)
            .file_name()
            .and_then(OsStr::to_str)
            .is_some_and(|name| candidates.iter().any(|c| c == name))
    });
    rows
}

/// A session bus connection with the user's manager on it.
async fn connect(paths: &Paths) -> Option<zbus::Connection> {
    let conn = zbus::connection::Builder::session()
        .ok()?
        .method_timeout(TIMEOUT)
        .build()
        .await
        .ok()?;
    // Asked first: a call to a name nobody owns waits for the timeout. A
    // bus this connection just activated doesn't have the manager on it
    // yet: it joins a moment after, so wait a little for it, when there is
    // a manager to wait for.
    let manager = paths
        .runtime_dir
        .as_ref()
        .is_some_and(|r| r.join("systemd/private").exists());
    let tries = if manager { 10 } else { 1 };
    for attempt in 1..=tries {
        let running = conn
            .call_method(Some(BUS), BUS_PATH, Some(BUS), "NameHasOwner", &(DEST,))
            .await
            .ok()?
            .body()
            .deserialize::<bool>()
            .ok()?;
        if running {
            return Some(conn);
        }
        if attempt < tries {
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    }
    None
}

/// Whether a unit is one the session needs to start.
fn session_unit(name: &str) -> bool {
    name.starts_with("dbus")
        || name.starts_with("systemd-")
        || name.starts_with("plasma-")
        || name.starts_with("xdg-desktop-portal")
}

/// Whether `name` is a unit name this page switches: a service, socket,
/// timer or path, not a template. Paths are refused: `EnableUnitFiles`
/// would link a file from anywhere.
fn valid_unit_name(name: &str) -> bool {
    let Some(stem) = UNIT_TYPES.iter().find_map(|t| name.strip_suffix(t)) else {
        return false;
    };
    name.len() <= 255
        && !stem.is_empty()
        && !stem.starts_with('.')
        && !stem.ends_with('@')
        && stem
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b":-_.\\@".contains(&b))
}

/// The unit rows from `ListUnitFilesByPatterns`' (path, state) rows.
fn merge_units(paths: &Paths, files: &[(String, String)], scan: &Scan) -> Vec<Item> {
    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (path, state) in files {
        let path = Path::new(path);
        let Some(name) = path.file_name().and_then(OsStr::to_str) else {
            continue;
        };
        if !valid_unit_name(name) || !seen.insert(name.to_owned()) {
            continue;
        }
        let state = FileState::parse(state);
        let wanted = scan.login_wants.contains(name);
        let (own, enabled, file) = match state {
            // A mask the user made; one in /etc is the admin's.
            FileState::Masked if path.starts_with(paths.user_units()) => {
                let installed = paths
                    .unit_dirs
                    .iter()
                    .map(|d| d.join(name))
                    .find(|p| p.is_file());
                (false, false, installed.unwrap_or_else(|| path.to_owned()))
            }
            FileState::Masked | FileState::MaskedRuntime => continue,
            FileState::Enabled | FileState::Disabled | FileState::Linked
                if paths.own_unit(path) =>
            {
                (true, state == FileState::Enabled, path.to_owned())
            }
            FileState::Enabled => (false, true, path.to_owned()),
            _ if wanted => (false, true, path.to_owned()),
            _ => continue,
        };
        let text = read_capped(&file).unwrap_or_default();
        // A Quadlet's unit names the .container file it was made from.
        let source = (state == FileState::Generated)
            .then(|| {
                text.lines()
                    .find_map(|l| l.trim().strip_prefix("SourcePath="))
                    .map(|s| PathBuf::from(s.trim()))
            })
            .flatten();
        // A Quadlet the user wrote is theirs, though switched like an
        // installed unit.
        let user_source = source
            .as_ref()
            .is_some_and(|s| s.starts_with(&paths.config_home));
        let mine = own || user_source || scan.user_wants.contains(name);
        out.push(Item {
            id: name.to_owned(),
            kind: Kind::Unit,
            name: parse_description(&text).unwrap_or_else(|| name.to_owned()),
            comment: String::new(),
            icon: String::new(),
            command: String::new(),
            file: source.unwrap_or(file),
            system: !own && !user_source,
            plumbing: !mine,
            enabled,
            runs: Runs::Yes,
            lock: (!own && session_unit(name)).then_some(Lock::Session),
            unit: Some(name.to_owned()),
            status: None,
            own,
            wanted,
        });
    }
    out
}

/// Fills in each item's unit state from `ListUnitsByNames`' rows.
fn apply_live(items: &mut [Item], live: &[(String, String, String)]) {
    let by_name: HashMap<&str, Status> = live
        .iter()
        .map(|(name, active, sub)| (name.as_str(), Status::of(&ActiveState::parse(active), sub)))
        .collect();
    for item in items {
        item.status = item.unit.as_deref().and_then(|u| by_name.get(u)).copied();
    }
}

/// Switches a unit through the user's manager, then reloads it.
fn set_unit(paths: &Paths, name: &str, own: bool, wanted: bool, on: bool) -> Result<(), Error> {
    if !valid_unit_name(name) {
        return Err(Error::InvalidName);
    }
    if !own && !on && session_unit(name) {
        return Err(Error::Locked);
    }
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|_| Error::NoAnswer)?;
    let result = rt.block_on(async {
        tokio::time::timeout(ACT_DEADLINE, set_unit_async(paths, name, own, wanted, on))
            .await
            .unwrap_or(Err(Error::NoAnswer))
    });
    drop(rt);
    result
}

async fn set_unit_async(
    paths: &Paths,
    name: &str,
    own: bool,
    wanted: bool,
    on: bool,
) -> Result<(), Error> {
    let conn = connect(paths).await.ok_or(Error::NoAnswer)?;
    let manager = zbus::proxy::Builder::<zbus::Proxy<'_>>::new(&conn)
        .destination(DEST)
        .and_then(|b| b.path(PATH))
        .and_then(|b| b.interface(MANAGER_IF))
        .map_err(|_| Error::NoAnswer)?
        .cache_properties(CacheProperties::No)
        .build()
        .await
        .map_err(|_| Error::NoAnswer)?;
    let names = &[name][..];
    let enable = async || -> Result<(), Error> {
        let (install_info, changes): (bool, Changes) = manager
            .call("EnableUnitFiles", &(names, false, false))
            .await
            .map_err(unit_error)?;
        if !install_info && changes.is_empty() {
            return Err(Error::NotEnableable);
        }
        Ok(())
    };
    let changed = async {
        match (own, on) {
            (true, true) => enable().await,
            (true, false) => {
                // Disabling also removes the link `systemctl link` made, and
                // the unit with it: put the link back (in ~/.config, where
                // the manager links, wherever it was before).
                let link = paths.linked_unit(name);
                let _: Changes = manager
                    .call("DisableUnitFiles", &(names, false))
                    .await
                    .map_err(unit_error)?;
                if let Some((at, target)) = link
                    && fs::symlink_metadata(&at).is_err()
                {
                    let target = target.to_string_lossy();
                    let _: Changes = manager
                        .call("LinkUnitFiles", &(&[target.as_ref()][..], false, false))
                        .await
                        .map_err(unit_error)?;
                }
                Ok(())
            }
            (false, false) => {
                let _: Changes = manager
                    .call("MaskUnitFiles", &(names, false, false))
                    .await
                    .map_err(unit_error)?;
                Ok(())
            }
            (false, true) => {
                let _: Changes = manager
                    .call("UnmaskUnitFiles", &(names, false))
                    .await
                    .map_err(unit_error)?;
                // Enabled for everyone or wanted by a login target, it
                // starts again now; enabled by nobody (masked from the
                // command line), it needs enabling.
                let state: String = manager
                    .call("GetUnitFileState", &(name,))
                    .await
                    .map_err(unit_error)?;
                if !wanted && FileState::parse(&state) == FileState::Disabled {
                    enable().await?;
                }
                Ok(())
            }
        }
    }
    .await;
    // Reloaded whatever happened: a step that failed after one that changed
    // the files mustn't leave the manager with the old ones.
    let reloaded = manager.call::<_, _, ()>("Reload", &()).await;
    changed?;
    reloaded.map_err(unit_error)
}

fn unit_error(e: zbus::Error) -> Error {
    match e {
        zbus::Error::MethodError(name, message, _) => match name.as_str() {
            "org.freedesktop.systemd1.NoSuchUnit"
            | "org.freedesktop.systemd1.LoadFailed"
            | "org.freedesktop.DBus.Error.FileNotFound" => Error::NotFound,
            "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => {
                Error::NoAnswer
            }
            other => Error::Refused(message.unwrap_or_else(|| other.to_owned())),
        },
        _ => Error::NoAnswer,
    }
}

#[cfg(test)]
mod tests;
