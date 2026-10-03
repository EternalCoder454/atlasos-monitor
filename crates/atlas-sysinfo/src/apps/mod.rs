//! Which application each process belongs to, and the Apps table's rows
//! grouped by it.
//!
//! Grouping processes by their own names gets multi-process programs wrong:
//! Firefox is `firefox`, `Isolated Web Co`, `WebExtensions` and a dozen
//! more, spread across rows that don't look related. The desktop already
//! knows better. Plasma and GNOME start every application in a systemd unit
//! of its own, named after the application's desktop ID, which is also the
//! name of its `.desktop` file, where its name and icon are kept. So: unit
//! ([`Proc::unit`]) → application ID ([`desktop::app_id`]) → desktop entry
//! ([`desktop::Index`]) → icon that draws ([`icons::IconLookup`]).
//!
//! A container's processes ([`Proc::container`]) are one row too, under the
//! container's name ([`container`]): a toolbox is one place to look, not a
//! shell, a compiler and a language server scattered through the table.
//!
//! [`Resolver`] answers that per unit and per container and remembers it.
//! [`Grouper`] folds a tick's processes into one row per application or
//! container, and per name for the processes that are neither's. [`Search`] is the table's filter and
//! [`Column`] its sort orders, and [`location`] where Open File Location
//! goes for a row ([`flatpak`] for a Flatpak's).

pub mod container;
pub mod desktop;
pub mod flatpak;
pub mod icons;

use std::cmp::Ordering;
use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Arc;

use crate::process::Proc;
pub use icons::{Icon, IconLookup};

/// An application as the Apps and Energy Saver pages show it, or a
/// container, which the Apps page shows the same way.
#[derive(Debug, Clone, PartialEq)]
pub struct App {
    /// The desktop ID: `org.mozilla.firefox`, `com.discordapp.Discord`. A
    /// container's 64-digit ID.
    pub id: Arc<str>,
    /// The name people know it by, from its desktop file, or made from the
    /// ID ([`desktop::fallback_name`]) when it has none.
    pub name: Arc<str>,
    /// The first icon that draws: the desktop file's, the ID (what Flatpak
    /// apps name their icons), or the lower-case name (`chromium` for
    /// `org.chromium.Chromium`, run without a desktop file by a
    /// browser-automation tool). `None` when none of them does.
    pub icon: Option<Icon>,
    /// A terminal emulator; see [`desktop::Entry::terminal`].
    pub terminal: bool,
    /// Started as a Flatpak: its unit is `app-flatpak-<ID>-…`.
    pub flatpak: bool,
    /// A podman container (toolbox, distrobox, any other), named as podman
    /// knows it, or "Container" and its short ID where its storage can't be
    /// read (a rootful one).
    pub container: bool,
    /// `name` lower-cased, for the search.
    folded: Box<str>,
}

/// A unit's or container's answer, and the tick it was last asked for.
#[derive(Debug)]
struct Known {
    app: Option<Arc<App>>,
    seen: u32,
}

/// Units and containers not asked about for this many ticks are forgotten.
/// Every launch of a Flatpak or a scope gets a unit with a new random
/// suffix, and every `podman run` a container with a new ID, so the cache
/// would otherwise grow for as long as Atlas runs.
const FORGET_AFTER: u32 = 120;

/// How often, in ticks, containers' names are checked against podman's
/// lists.
const CONTAINER_RECHECK: u32 = 30;

/// Turns a process's systemd unit into its application, and its container
/// into a row of its own, remembering the answer per unit and container: a
/// unit's application never changes, a desktop has a few dozen of them, and
/// the table asks about every row every tick.
///
/// Owned by the sampling thread, like the samplers.
#[derive(Debug)]
pub struct Resolver {
    index: desktop::Index,
    /// `None` takes every icon name on trust (tests, and no display).
    icons: Option<IconLookup>,
    /// `None` names every container by its short ID.
    containers: Option<container::Store>,
    /// The store's [`reads`](container::Store::reads) when the containers
    /// known were last checked.
    container_reads: u64,
    /// By unit name, and by container ID: the two never look alike, since
    /// a unit's name ends in `.scope` or `.service`.
    by_unit: HashMap<Arc<str>, Known>,
    tick: u32,
}

impl Resolver {
    pub fn new(index: desktop::Index, icons: Option<IconLookup>) -> Self {
        Self {
            index,
            icons,
            containers: None,
            container_reads: 0,
            by_unit: HashMap::new(),
            tick: 0,
        }
    }

    /// Names containers from `store`'s lists.
    pub fn with_containers(mut self, store: container::Store) -> Self {
        self.containers = Some(store);
        self
    }

    /// The resolver for this session: desktop files from the XDG data
    /// directories, icons from `theme` (Qt's `QIcon::themeName()`).
    pub fn for_session(theme: &str) -> Self {
        let dirs = desktop::data_dirs();
        let icons = IconLookup::new(theme, &dirs);
        Self::new(desktop::Index::new(dirs), Some(icons))
            .with_containers(container::Store::for_user())
    }

    /// The `icons/` folders Qt must search for the icons this picks; see
    /// [`IconLookup::search_paths`].
    pub fn icon_search_paths(&self) -> Vec<std::path::PathBuf> {
        self.icons
            .as_ref()
            .map(|i| i.search_paths().map(Into::into).collect())
            .unwrap_or_default()
    }

    /// Follows a change of icon theme (Breeze to Breeze Dark, say): every
    /// unit's icon is picked again in the new one. The app calls it when
    /// `QIcon::themeName()` changes.
    pub fn set_icon_theme(&mut self, theme: &str) {
        if let Some(icons) = &self.icons
            && icons.theme() != theme
        {
            self.icons = Some(icons.with_theme(theme));
            self.by_unit.clear();
        }
    }

    /// The application `unit` belongs to, or `None` when it is no
    /// application's: a system service, the session, a kernel thread.
    pub fn of(&mut self, unit: Option<&Arc<str>>) -> Option<&Arc<App>> {
        self.remember(unit?, Self::resolve)
    }

    /// What a process's row stands for: its container, if it runs in one,
    /// whatever unit that runs (distrobox `--init` runs systemd in there),
    /// else the application of its unit ([`of`](Self::of)).
    pub fn of_proc(&mut self, p: &Proc) -> Option<&Arc<App>> {
        match &p.container {
            Some(id) => self.remember(id, Self::resolve_container),
            None => self.of(p.unit.as_ref()),
        }
    }

    fn remember(
        &mut self,
        key: &Arc<str>,
        resolve: fn(&mut Self, &str) -> Option<App>,
    ) -> Option<&Arc<App>> {
        let tick = self.tick;
        if !self.by_unit.contains_key(&**key) {
            let app = resolve(self, key).map(Arc::new);
            self.by_unit.insert(key.clone(), Known { app, seen: tick });
        }
        let known = self.by_unit.get_mut(&**key)?;
        known.seen = tick;
        known.app.as_ref()
    }

    fn resolve_container(&mut self, id: &str) -> Option<App> {
        let name = match self.containers.as_mut().and_then(|s| s.name(id)) {
            Some(name) => name.to_owned(),
            None => format!("Container {}", container::short_id(id)),
        };
        Some(App {
            id: id.into(),
            folded: name.to_lowercase().into(),
            name: name.into(),
            icon: self.pick_icon(container::ICON, "", ""),
            terminal: false,
            flatpak: false,
            container: true,
        })
    }

    fn resolve(&mut self, unit: &str) -> Option<App> {
        let id = desktop::app_id(unit)?;
        let (name, declared, terminal) = match self.index.lookup(&id) {
            Some(e) => (e.name.clone(), e.icon.clone(), e.terminal),
            None => (desktop::fallback_name(&id).to_owned(), String::new(), false),
        };
        let icon = self.pick_icon(&declared, &id, &name);
        Some(App {
            id: id.into(),
            folded: name.to_lowercase().into(),
            name: name.into(),
            icon,
            terminal,
            flatpak: unit.starts_with(flatpak::UNIT_PREFIX),
            container: false,
        })
    }

    fn pick_icon(&mut self, declared: &str, id: &str, name: &str) -> Option<Icon> {
        if declared.starts_with('/') {
            if std::path::Path::new(declared).is_file() {
                return Some(Icon::Path(declared.into()));
            }
            return self.pick_icon("", id, name);
        }
        let lower = name.to_lowercase();
        let candidates = [declared, id, &lower].into_iter().filter(|c| !c.is_empty());
        let Some(icons) = &mut self.icons else {
            return candidates.map(|c| Icon::Name(c.to_owned())).next();
        };
        // Every name as it is before any shortened ("foo-bar" drawn as
        // "foo"), so the ID found as it is beats a declared icon that only
        // its prefix stands in for.
        candidates
            .clone()
            .find_map(|c| icons.find_exact(c))
            .or_else(|| candidates.clone().find_map(|c| icons.find(c)))
    }

    /// The row a process belongs to in the grouped table.
    pub fn key_of(&mut self, p: &Proc) -> GroupKey {
        match self.of_proc(p) {
            Some(a) => GroupKey::App(a.id.clone()),
            None => GroupKey::Process(p.name.clone()),
        }
    }

    /// The processes a grouped row stands for, for its actions and Details.
    pub fn members<'a>(
        &'a mut self,
        key: &'a GroupKey,
        procs: &'a [Proc],
    ) -> impl Iterator<Item = &'a Proc> {
        procs.iter().filter(move |p| self.key_of(p) == *key)
    }

    /// Marks the start of a tick, and now and then forgets units that
    /// haven't been seen for a while. [`Grouper::group`] calls it; nothing
    /// else does, so a resolver used without grouping never forgets.
    pub fn next_tick(&mut self) {
        self.tick = self.tick.wrapping_add(1);
        if self.tick.is_multiple_of(FORGET_AFTER) {
            let tick = self.tick;
            self.by_unit
                .retain(|_, k| tick.wrapping_sub(k.seen) <= FORGET_AFTER);
        }
        if self.tick.is_multiple_of(CONTAINER_RECHECK) {
            self.recheck_containers();
        }
    }

    /// Names every container again if podman's lists have changed since
    /// the last check, read now or for a container seen in between: one may
    /// have been renamed, or been seen before podman listed it and named by
    /// its ID. Only while there are containers to name.
    fn recheck_containers(&mut self) {
        let is_container = |k: &Known| k.app.as_ref().is_some_and(|a| a.container);
        let Some(store) = &mut self.containers else {
            return;
        };
        if !self.by_unit.values().any(is_container) {
            return;
        }
        store.refresh();
        if store.reads() != self.container_reads {
            self.container_reads = store.reads();
            self.by_unit.retain(|_, k| !is_container(k));
        }
    }

    #[cfg(test)]
    fn units_known(&self) -> usize {
        self.by_unit.len()
    }
}

/// A row of the grouped table: an application or container by its ID, or,
/// for a process that is neither's, its name. The two are kept apart so a process
/// that merely shares an application's name can't fold into its row.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum GroupKey {
    App(Arc<str>),
    Process(Arc<str>),
}

/// One row of the grouped table.
#[derive(Debug, Clone, PartialEq)]
pub struct Group {
    pub key: GroupKey,
    pub app: Option<Arc<App>>,
    /// How many processes it stands for. A row of one shows that process's
    /// pid; a row of more has no one pid, and showing the first member's
    /// would invite acting on it.
    pub count: u32,
    /// The members summed, as one process, so the table renders, sorts and
    /// rates ([`Proc::impact`]) a group as it does a process. The name is
    /// the application's; pid, start time and unit are the first member's.
    /// A figure is known if any member's is.
    pub total: Proc,
}

impl AsRef<Proc> for Proc {
    fn as_ref(&self) -> &Proc {
        self
    }
}

impl AsRef<Proc> for Group {
    fn as_ref(&self) -> &Proc {
        &self.total
    }
}

/// Folds each tick's processes into groups, reusing its storage.
#[derive(Debug, Default)]
pub struct Grouper {
    groups: Vec<Group>,
    index: HashMap<GroupKey, usize>,
}

impl Grouper {
    /// One row per application or container, and per name for the
    /// processes that are neither's, in the order their first members come
    /// in `procs`.
    pub fn group(&mut self, procs: &[Proc], apps: &mut Resolver) -> &[Group] {
        apps.next_tick();
        self.groups.clear();
        self.index.clear();
        for p in procs {
            let app = apps.of_proc(p).cloned();
            let key = match &app {
                Some(a) => GroupKey::App(a.id.clone()),
                None => GroupKey::Process(p.name.clone()),
            };
            if let Some(&i) = self.index.get(&key) {
                let g = &mut self.groups[i];
                g.count += 1;
                add(&mut g.total, p);
                continue;
            }
            let mut total = p.clone();
            if let Some(a) = &app {
                total.name = a.name.clone();
            }
            self.index.insert(key.clone(), self.groups.len());
            self.groups.push(Group {
                key,
                app,
                count: 1,
                total,
            });
        }
        &self.groups
    }
}

fn add(total: &mut Proc, p: &Proc) {
    fn sum(a: &mut Option<f64>, b: Option<f64>) {
        if let Some(b) = b {
            *a = Some(a.unwrap_or(0.0) + b);
        }
    }
    total.cpu += p.cpu;
    total.memory += p.memory;
    sum(&mut total.gpu, p.gpu);
    sum(&mut total.net_in, p.net_in);
    sum(&mut total.net_out, p.net_out);
    sum(&mut total.disk_read, p.disk_read);
    sum(&mut total.disk_write, p.disk_write);
}

/// Where Open File Location goes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Location {
    /// A folder to open: a Flatpak's install folder.
    Folder(PathBuf),
    /// A file to show selected in its folder: a program.
    File(PathBuf),
}

/// Open File Location for a grouped row of `members`, belonging to `app`.
/// A Flatpak opens its install folder: the one a member's sandbox was made
/// from, or else the first installation that has the app. Any other row
/// shows the first member's program that can be read
/// ([`crate::process::executable`]). `None` when nothing can be found.
///
/// A container's row has none: its own programs are files only podman's
/// namespace sees, and what the host can show (a toolbox's init is the
/// host's `toolbox`) is not the container.
///
/// The row is a Flatpak's if any member runs in that app's sandbox, so a
/// row that holds both a Flatpak and a native launch of one ID opens the
/// Flatpak's folder whichever launch [`App::flatpak`] came from.
///
/// Reads `/proc` and the installations: run it off the GUI thread.
pub fn location<'a>(
    app: Option<&App>,
    members: impl IntoIterator<Item = &'a Proc>,
) -> Option<Location> {
    if app.is_some_and(|a| a.container) {
        return None;
    }
    let pids: Vec<u32> = members.into_iter().map(|p| p.pid).collect();
    if let Some(app) = app {
        // The bwrap that sets the sandbox up runs outside it: ask them all.
        let folder = pids
            .iter()
            .filter_map(|&pid| flatpak::Instance::of(pid))
            .find(|i| *i.id == *app.id)
            .map(|i| i.install_folder())
            .or_else(|| {
                let installs = app.flatpak.then(flatpak::installations)?;
                flatpak::install_folder(&app.id, &installs)
            });
        if let Some(folder) = folder {
            return Some(Location::Folder(folder));
        }
    }
    pids.iter()
        .find_map(|&pid| crate::process::executable(pid))
        .map(Location::File)
}

/// The table's search filter.
#[derive(Debug, Clone, Default)]
pub struct Search {
    needle: String,
}

impl Search {
    pub fn new(text: &str) -> Self {
        Self {
            needle: text.trim().to_lowercase(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.needle.is_empty()
    }

    /// Whether a process matches: its name, its application's name or its
    /// pid contains the search. A process of an application answers to the
    /// application's name, so searching "discord" finds Discord's helpers
    /// whatever they call themselves.
    pub fn matches(&self, p: &Proc, app: Option<&App>) -> bool {
        self.matches_row(p, app, true)
    }

    /// Whether a grouped row matches: as [`matches`](Self::matches), but by
    /// pid only for a row of one process, since a row of more shows none.
    pub fn matches_group(&self, g: &Group) -> bool {
        self.matches_row(&g.total, g.app.as_deref(), g.count == 1)
    }

    fn matches_row(&self, p: &Proc, app: Option<&App>, by_pid: bool) -> bool {
        let n = self.needle.as_bytes();
        if n.is_empty() || contains_fold(p.name.as_bytes(), n) {
            return true;
        }
        if app.is_some_and(|a| a.folded.contains(&self.needle)) {
            return true;
        }
        let mut buf = [0u8; 10];
        by_pid && contains(digits(p.pid, &mut buf), n)
    }
}

/// `v` in decimal, written into the end of `buf`.
fn digits(mut v: u32, buf: &mut [u8; 10]) -> &[u8] {
    let mut i = buf.len();
    loop {
        i -= 1;
        buf[i] = b'0' + (v % 10) as u8;
        v /= 10;
        if v == 0 {
            return &buf[i..];
        }
    }
}

fn contains(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| w == needle)
}

/// Whether `hay`, ASCII-lower-cased, contains `needle` (already lower case).
/// Process names are the kernel's, ASCII in practice.
fn contains_fold(hay: &[u8], needle: &[u8]) -> bool {
    hay.windows(needle.len()).any(|w| {
        w.iter()
            .zip(needle)
            .all(|(a, b)| a.to_ascii_lowercase() == *b)
    })
}

/// The table's sortable columns.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Column {
    Name,
    Pid,
    Cpu,
    Memory,
    Gpu,
    /// By the score behind the label, so it runs Very Low → High rather than
    /// alphabetically.
    Power,
    NetIn,
    NetOut,
    DiskRead,
    DiskWrite,
}

impl Column {
    /// Ascending order by this column. Rows that read the same compare
    /// equal: there is deliberately no tie-break on pid, see [`sort`].
    /// Unknown figures sort below zero.
    pub fn compare(self, a: &Proc, b: &Proc) -> Ordering {
        fn opt(a: Option<f64>, b: Option<f64>) -> Ordering {
            match (a, b) {
                (Some(a), Some(b)) => a.total_cmp(&b),
                (a, b) => a.is_some().cmp(&b.is_some()),
            }
        }
        match self {
            Column::Name => cmp_fold(&a.name, &b.name),
            Column::Pid => a.pid.cmp(&b.pid),
            Column::Cpu => a.cpu.total_cmp(&b.cpu),
            Column::Memory => a.memory.cmp(&b.memory),
            Column::Gpu => opt(a.gpu, b.gpu),
            Column::Power => a.power_score().total_cmp(&b.power_score()),
            Column::NetIn => opt(a.net_in, b.net_in),
            Column::NetOut => opt(a.net_out, b.net_out),
            Column::DiskRead => opt(a.disk_read, b.disk_read),
            Column::DiskWrite => opt(a.disk_write, b.disk_write),
        }
    }
}

/// Names in ASCII case-insensitive order, without allocating.
fn cmp_fold(a: &str, b: &str) -> Ordering {
    a.bytes()
        .map(|c| c.to_ascii_lowercase())
        .cmp(b.bytes().map(|c| c.to_ascii_lowercase()))
}

/// Re-sorts the table's row order. `order` holds indices into `rows` in the
/// order shown last tick, with rows new this tick at the end.
///
/// Rows that compare equal keep the order they had, so a table sorted by
/// CPU doesn't shuffle its idle rows every second, or reorder them when a
/// process comes or goes: the sort is stable and the columns never break a
/// tie. A descending sort reverses the comparison, not the result, which
/// would flip every tie.
pub fn sort<T: AsRef<Proc>>(order: &mut [usize], rows: &[T], column: Column, descending: bool) {
    order.sort_by(|&a, &b| {
        let o = column.compare(rows[a].as_ref(), rows[b].as_ref());
        if descending { o.reverse() } else { o }
    });
}

#[cfg(test)]
mod tests;
