//! The Apps table: a list model of applications (or processes), sorted,
//! searched and grouped here, and changed by row diffs. A refresh inserts,
//! moves and removes rows and reports changed figures; it never resets the
//! model, so the view keeps its rows, scroll position and selection.
//!
//! While the pointer is over the table (`setHeld(true)`), rows keep their
//! places: new ones come in at the end and gone ones leave, but nothing is
//! re-sorted until the pointer leaves, so the row under it stays put.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qmodelindex.h");
        type QModelIndex = cxx_qt_lib::QModelIndex;
        include!("cxx-qt-lib/qvariant.h");
        type QVariant = cxx_qt_lib::QVariant;
        include!("cxx-qt-lib/qhash.h");
        type QHash_i32_QByteArray = cxx_qt_lib::QHash<cxx_qt_lib::QHashPair_i32_QByteArray>;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!(<QtCore/QAbstractListModel>);
        type QAbstractListModel;
    }

    extern "RustQt" {
        #[qobject]
        #[base = QAbstractListModel]
        /// Rows shown.
        #[qproperty(i32, count)]
        /// One row per application, with its processes under it.
        #[qproperty(bool, grouped)]
        /// Kernel threads are listed (the sampler reads them).
        #[qproperty(bool, kernel_threads, cxx_name = "kernelThreads")]
        /// Column roles the table leaves out.
        #[qproperty(QStringList, hidden_columns, cxx_name = "hiddenColumns")]
        /// The pinned row's name, and how many processes it stands for.
        #[qproperty(QString, pinned_name, cxx_name = "pinnedName")]
        #[qproperty(i32, pinned_count, cxx_name = "pinnedCount")]
        #[namespace = "atlas_monitor"]
        type ProcessModel = super::ProcessModelRust;
    }

    unsafe extern "RustQt" {
        /// Sorts by a column's role ("cpu", "name", ...).
        #[qinvokable]
        #[cxx_name = "sortBy"]
        fn sort_by(self: Pin<&mut ProcessModel>, role: &QString, descending: bool);

        /// Shows only the rows whose name, application or pid contain `text`.
        #[qinvokable]
        #[cxx_name = "setSearch"]
        fn set_search(self: Pin<&mut ProcessModel>, text: &QString);

        #[qinvokable]
        #[cxx_name = "setGrouping"]
        fn set_grouping(self: Pin<&mut ProcessModel>, on: bool);

        #[qinvokable]
        #[cxx_name = "showKernelThreads"]
        fn show_kernel_threads(self: Pin<&mut ProcessModel>, on: bool);

        /// Shows or hides the column with this role.
        #[qinvokable]
        #[cxx_name = "setColumnShown"]
        fn set_column_shown(self: Pin<&mut ProcessModel>, role: &QString, shown: bool);

        /// Keeps the rows in their places while the pointer is over them.
        #[qinvokable]
        #[cxx_name = "setHeld"]
        fn set_held(self: Pin<&mut ProcessModel>, held: bool);

        /// Opens or closes an application's row.
        #[qinvokable]
        fn toggle(self: Pin<&mut ProcessModel>, row: i32);

        /// Takes the row a menu or a confirmation is about: its name and
        /// the processes it is now, so what is acted on later is what the
        /// user saw, wherever the row has gone since. False for no row.
        #[qinvokable]
        fn pin(self: Pin<&mut ProcessModel>, row: i32) -> bool;

        /// End Task (0), Kill (1), Stop (2) or Continue (3) for the pinned
        /// row's processes. "" when done, else "gone" (they had exited),
        /// "denied" (another user's) or "failed".
        #[qinvokable]
        fn act(self: &ProcessModel, action: i32) -> QString;

        /// Opens the Details dialog on the pinned row.
        #[qinvokable]
        #[cxx_name = "showDetails"]
        fn show_details(self: &ProcessModel);

        /// Shows the pinned row's program in the file manager. Answers with
        /// `located` only when the file manager couldn't take it.
        #[qinvokable]
        #[cxx_name = "openLocation"]
        fn open_location(self: Pin<&mut ProcessModel>);

        /// Open File Location couldn't hand the program to a file manager:
        /// `folder` is its folder's URL to open instead, or "" when no path
        /// on this machine leads to the program.
        #[qsignal]
        fn located(self: Pin<&mut ProcessModel>, name: QString, folder: QString);

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(
            self: Pin<&mut ProcessModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut ProcessModel>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(
            self: Pin<&mut ProcessModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut ProcessModel>);
        #[inherit]
        #[cxx_name = "beginMoveRows"]
        fn begin_move_rows(
            self: Pin<&mut ProcessModel>,
            source_parent: &QModelIndex,
            source_first: i32,
            source_last: i32,
            destination_parent: &QModelIndex,
            destination_child: i32,
        ) -> bool;
        #[inherit]
        #[cxx_name = "endMoveRows"]
        fn end_move_rows(self: Pin<&mut ProcessModel>);
        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut ProcessModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut ProcessModel>);
        #[inherit]
        fn index(self: &ProcessModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut ProcessModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        #[cxx_override]
        fn data(self: &ProcessModel, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &ProcessModel) -> QHash_i32_QByteArray;
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &ProcessModel, parent: &QModelIndex) -> i32;
    }

    impl cxx_qt::Threading for ProcessModel {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn process_model_make_unique() -> UniquePtr<ProcessModel>;
    }
}

use std::collections::{HashMap, HashSet};
use std::pin::Pin;

use atlas_sysinfo::apps::{self, Column, GroupKey, Search};
use atlas_sysinfo::files::{self, Shown};
use atlas_sysinfo::process::{self, Action, ActionError, Proc};
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QStringList, QVariant,
};

use crate::details::qobject::ProcessDetails;
use crate::details::{Member, Subject};
use crate::rows::{FIRST_ROLE, Roles, Value, int};
use crate::sampler::qobject::Sampler;
use crate::sampling::AppsTick;
use crate::settings::AppsView;

/// What a row stands for: an application's (or a name's) group, or one
/// process by pid and start time.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
enum RowKey {
    Group(GroupKey),
    Proc(u32, u64),
}

/// For a process with no application icon.
const PROGRAM_ICON: &str = "application-x-executable";

#[derive(Debug, Clone, PartialEq)]
struct Row {
    key: RowKey,
    icon: String,
    /// The figures; a group's are its members' sums, named as the app.
    proc: Proc,
    /// Processes it stands for: 1 for a process.
    count: u32,
    depth: i32,
    expanded: bool,
}

/// How the table is laid out, as the page last asked.
#[derive(Debug, Clone)]
struct View {
    column: Column,
    descending: bool,
    search: Search,
    grouped: bool,
    held: bool,
    expanded: HashSet<GroupKey>,
}

impl Default for View {
    fn default() -> Self {
        Self {
            column: Column::Cpu,
            descending: true,
            search: Search::default(),
            grouped: true,
            held: false,
            expanded: HashSet::new(),
        }
    }
}

pub struct ProcessModelRust {
    count: i32,
    grouped: bool,
    pinned_name: QString,
    pinned_count: i32,
    rows: Vec<Row>,
    data: AppsTick,
    view: View,
    /// The pinned row's processes, by pid and start time.
    pinned: Vec<(u32, u64)>,
    /// What Details shows for the pinned row.
    pinned_subject: Option<Subject>,
    /// The Details dialog's object.
    pub details: Option<Box<CxxQtThread<ProcessDetails>>>,
    kernel_threads: bool,
    hidden_columns: QStringList,
    /// Where Show Kernel Threads goes: the sampler reads the processes.
    pub sampler: Option<Box<CxxQtThread<Sampler>>>,
    /// The hidden columns as saved.
    hidden: Vec<String>,
}

/// The columns View can hide: all but Name.
const HIDEABLE: [&str; 9] = [
    "pid",
    "cpu",
    "memory",
    "diskRead",
    "diskWrite",
    "gpu",
    "power",
    "netIn",
    "netOut",
];

pub(crate) fn string_list(v: &[String]) -> QStringList {
    let mut l = QList::<QString>::default();
    for s in v {
        l.append(QString::from(s));
    }
    QStringList::from(&l)
}

impl Default for ProcessModelRust {
    fn default() -> Self {
        let saved = AppsView::load();
        let hidden: Vec<String> = saved
            .hidden
            .into_iter()
            .filter(|r| HIDEABLE.contains(&r.as_str()))
            .collect();
        Self {
            count: 0,
            grouped: saved.grouped,
            pinned_name: QString::default(),
            pinned_count: 0,
            rows: Vec::new(),
            data: AppsTick::default(),
            view: View {
                grouped: saved.grouped,
                ..View::default()
            },
            pinned: Vec::new(),
            pinned_subject: None,
            details: None,
            kernel_threads: saved.kernel_threads,
            hidden_columns: string_list(&hidden),
            sampler: None,
            hidden,
        }
    }
}

const ROLES: [&str; 15] = [
    "name",
    "icon",
    "pid",
    "cpu",
    "memory",
    "gpu",
    "power",
    "netIn",
    "netOut",
    "diskRead",
    "diskWrite",
    "depth",
    "expandable",
    "expanded",
    "count",
];

fn column(role: &str) -> Option<Column> {
    Some(match role {
        "name" => Column::Name,
        "pid" => Column::Pid,
        "cpu" => Column::Cpu,
        "memory" => Column::Memory,
        "gpu" => Column::Gpu,
        "power" => Column::Power,
        "netIn" => Column::NetIn,
        "netOut" => Column::NetOut,
        "diskRead" => Column::DiskRead,
        "diskWrite" => Column::DiskWrite,
        _ => return None,
    })
}

fn icon_of(app: Option<&apps::App>) -> String {
    app.and_then(|a| a.icon.as_ref())
        .map_or_else(|| PROGRAM_ICON.to_owned(), |i| i.source().into_owned())
}

/// `idx` (indices into `rows`) in the order they were shown last, rows new
/// this tick after them; then sorted, unless the rows are held. The sort is
/// stable, so rows that tie keep their places.
fn ordered<T: AsRef<Proc>>(
    rows: &[T],
    mut idx: Vec<usize>,
    key: impl Fn(usize) -> RowKey,
    shown: &HashMap<RowKey, usize>,
    view: &View,
) -> Vec<usize> {
    idx.sort_by_key(|&i| shown.get(&key(i)).copied().unwrap_or(usize::MAX));
    if !view.held {
        apps::sort(&mut idx, rows, view.column, view.descending);
    }
    idx
}

/// The rows to show for a tick.
fn layout(data: &AppsTick, view: &View, shown: &HashMap<RowKey, usize>) -> Vec<Row> {
    let procs = &data.procs;
    let proc_key = |i: usize| RowKey::Proc(procs[i].pid, procs[i].start_time);
    let proc_row = |i: usize, icon: String, depth: i32| Row {
        key: proc_key(i),
        icon,
        proc: procs[i].clone(),
        count: 1,
        depth,
        expanded: false,
    };
    if !view.grouped {
        let idx = (0..procs.len())
            .filter(|&i| view.search.matches(&procs[i], data.apps[i].as_deref()))
            .collect();
        let mut seen = HashSet::new();
        return ordered(procs, idx, proc_key, shown, view)
            .into_iter()
            .filter(|&i| seen.insert(proc_key(i)))
            .map(|i| proc_row(i, icon_of(data.apps[i].as_deref()), 0))
            .collect();
    }

    let groups = &data.groups;
    let at: HashMap<&GroupKey, usize> = groups
        .iter()
        .enumerate()
        .map(|(g, x)| (&x.key, g))
        .collect();
    let mut members: Vec<Vec<usize>> = vec![Vec::new(); groups.len()];
    for (i, k) in data.keys.iter().enumerate() {
        if let Some(&g) = at.get(k) {
            members[g].push(i);
        }
    }
    let group_key = |g: usize| RowKey::Group(groups[g].key.clone());
    let idx = (0..groups.len())
        .filter(|&g| {
            view.search.is_empty()
                || view.search.matches_group(&groups[g])
                || members[g]
                    .iter()
                    .any(|&i| view.search.matches(&procs[i], data.apps[i].as_deref()))
        })
        .collect();
    let mut rows = Vec::new();
    // A key shows once: the model's steps from one layout to the next are
    // planned by key.
    let mut seen = HashSet::new();
    for g in ordered(groups, idx, group_key, shown, view) {
        if !seen.insert(group_key(g)) {
            continue;
        }
        let group = &groups[g];
        let icon = icon_of(group.app.as_deref());
        let expanded = group.count > 1 && view.expanded.contains(&group.key);
        rows.push(Row {
            key: group_key(g),
            icon: icon.clone(),
            proc: group.total.clone(),
            count: group.count,
            depth: 0,
            expanded,
        });
        if expanded {
            // Found by a member: only the members that match show.
            let mut shown_members = members[g].clone();
            if !view.search.matches_group(group) {
                shown_members.retain(|&i| view.search.matches(&procs[i], data.apps[i].as_deref()));
            }
            for i in ordered(procs, shown_members, proc_key, shown, view) {
                if seen.insert(proc_key(i)) {
                    rows.push(proc_row(i, icon.clone(), 1));
                }
            }
        }
    }
    rows
}

/// What Details shows for `row`: a process, or an application of several.
fn subject_of(data: &AppsTick, row: &Row) -> Subject {
    let application = |i: usize| data.apps[i].as_ref().map(|a| a.name.to_string());
    match &row.key {
        RowKey::Proc(pid, start) => Subject::Process {
            pid: *pid,
            start_time: *start,
            name: row.proc.name.to_string(),
            application: data
                .procs
                .iter()
                .position(|p| p.pid == *pid && p.start_time == *start)
                .and_then(application),
        },
        RowKey::Group(key) => {
            let at: Vec<usize> = (0..data.procs.len())
                .filter(|&i| data.keys[i] == *key)
                .collect();
            if let [i] = at[..] {
                let p = &data.procs[i];
                return Subject::Process {
                    pid: p.pid,
                    start_time: p.start_time,
                    name: p.name.to_string(),
                    application: application(i),
                };
            }
            Subject::Group {
                name: row.proc.name.to_string(),
                app_id: data
                    .groups
                    .iter()
                    .find(|g| g.key == *key)
                    .and_then(|g| g.app.as_ref())
                    .map(|a| a.id.to_string()),
                members: at
                    .into_iter()
                    .map(|i| {
                        let p = &data.procs[i];
                        Member {
                            pid: p.pid,
                            start_time: p.start_time,
                            name: p.name.to_string(),
                            cpu: p.cpu,
                            memory: p.memory,
                            unit: p.unit.as_deref().map(str::to_owned),
                        }
                    })
                    .collect(),
            }
        }
    }
}

fn opt(v: Option<f64>) -> f64 {
    v.unwrap_or(f64::NAN)
}

impl Roles for Row {
    const NAMES: &'static [&'static str] = &ROLES;

    fn value(&self, role: usize) -> Value<'_> {
        let p = &self.proc;
        match ROLES[role] {
            "name" => Value::Text(&p.name),
            "icon" => Value::Text(&self.icon),
            // A row of several processes has no one pid to show.
            "pid" => Value::Int(if self.count == 1 {
                int(p.pid as usize)
            } else {
                0
            }),
            "cpu" => Value::Real(p.cpu),
            "memory" => Value::Real(p.memory as f64),
            "gpu" => Value::Real(opt(p.gpu)),
            "power" => Value::Int(p.impact() as i32),
            "netIn" => Value::Real(opt(p.net_in)),
            "netOut" => Value::Real(opt(p.net_out)),
            "diskRead" => Value::Real(opt(p.disk_read)),
            "diskWrite" => Value::Real(opt(p.disk_write)),
            "depth" => Value::Int(self.depth),
            "expandable" => Value::Bool(self.count > 1),
            "expanded" => Value::Bool(self.expanded),
            "count" => Value::Int(int(self.count as usize)),
            _ => unreachable!("a role in ROLES without a value"),
        }
    }
}

crate::rows::row_model!(qobject::ProcessModel, Row, "Apps");

impl qobject::ProcessModel {
    /// A tick from the sampling thread.
    pub fn apply(mut self: Pin<&mut Self>, data: AppsTick) {
        let after_first = {
            let mut r = self.as_mut().rust_mut();
            // Applications that have gone are forgotten as open.
            let alive: HashSet<&GroupKey> = data.groups.iter().map(|g| &g.key).collect();
            r.view.expanded.retain(|k| alive.contains(k));
            std::mem::replace(&mut r.data, data).first
        };
        // The first reading's order is mostly ties, so the one after sorts
        // even under the pointer, which may have rested there since the
        // page opened. Should the row menu or a question be up already, its
        // answer still goes to its row: that was pinned by pid as it opened.
        if after_first {
            self.relayout_unheld();
        } else {
            self.relayout();
        }
    }

    /// Lays the rows out again and brings the model to them, step by step.
    fn relayout(mut self: Pin<&mut Self>) {
        let new = {
            let r = self.rust();
            let shown = r
                .rows
                .iter()
                .enumerate()
                .map(|(i, row)| (row.key.clone(), i))
                .collect();
            layout(&r.data, &r.view, &shown)
        };
        self.as_mut().replace_rows(new);
        self.update_count();
    }

    pub fn sort_by(mut self: Pin<&mut Self>, role: &QString, descending: bool) {
        let Some(column) = column(&role.to_string()) else {
            return;
        };
        {
            let mut r = self.as_mut().rust_mut();
            r.view.column = column;
            r.view.descending = descending;
        }
        self.relayout_unheld();
    }

    pub fn set_search(mut self: Pin<&mut Self>, text: &QString) {
        self.as_mut().rust_mut().view.search = Search::new(&text.to_string());
        self.relayout_unheld();
    }

    pub fn set_grouping(mut self: Pin<&mut Self>, on: bool) {
        if let Err(e) = AppsView::save_grouped(on) {
            log::warn!("saving Group by App: {e}");
        }
        self.as_mut().rust_mut().view.grouped = on;
        self.as_mut().set_grouped(on);
        self.relayout_unheld();
    }

    pub fn show_kernel_threads(mut self: Pin<&mut Self>, on: bool) {
        if let Err(e) = AppsView::save_kernel_threads(on) {
            log::warn!("saving Show Kernel Threads: {e}");
        }
        self.as_mut().set_kernel_threads(on);
        if let Some(s) = self.rust().sampler.as_deref() {
            let _ = s.queue(move |s| s.show_kernel_threads(on));
        }
    }

    pub fn set_column_shown(mut self: Pin<&mut Self>, role: &QString, shown: bool) {
        let role = role.to_string();
        if !HIDEABLE.contains(&role.as_str()) {
            return;
        }
        let list = {
            let mut r = self.as_mut().rust_mut();
            r.hidden.retain(|h| *h != role);
            if !shown {
                r.hidden.push(role);
                r.hidden.sort();
            }
            r.hidden.clone()
        };
        if let Err(e) = AppsView::save_hidden(&list) {
            log::warn!("saving the Apps columns: {e}");
        }
        self.set_hidden_columns(string_list(&list));
    }

    pub fn set_held(mut self: Pin<&mut Self>, held: bool) {
        let was = std::mem::replace(&mut self.as_mut().rust_mut().view.held, held);
        // Let go: the order the pointer held back catches up at once.
        if was && !held {
            self.relayout();
        }
    }

    /// Something the user asked for (a sort, a search): it reorders now,
    /// even under the pointer, since the pointer is on the control that
    /// asked.
    fn relayout_unheld(mut self: Pin<&mut Self>) {
        let held = std::mem::replace(&mut self.as_mut().rust_mut().view.held, false);
        self.as_mut().relayout();
        self.as_mut().rust_mut().view.held = held;
    }

    pub fn toggle(mut self: Pin<&mut Self>, row: i32) {
        let Some(Row {
            key: RowKey::Group(key),
            count: 2..,
            ..
        }) = self.row(row).cloned()
        else {
            return;
        };
        {
            let expanded = &mut self.as_mut().rust_mut().view.expanded;
            if !expanded.remove(&key) {
                expanded.insert(key);
            }
        }
        self.relayout_unheld();
    }

    fn row(&self, row: i32) -> Option<&Row> {
        self.rust().rows.get(usize::try_from(row).ok()?)
    }

    pub fn pin(mut self: Pin<&mut Self>, row: i32) -> bool {
        let Some(row) = self.row(row).cloned() else {
            // Nothing is acted on that the user didn't just pick.
            let mut r = self.as_mut().rust_mut();
            r.pinned.clear();
            r.pinned_subject = None;
            return false;
        };
        let data = &self.rust().data;
        let targets: Vec<(u32, u64)> = match &row.key {
            RowKey::Proc(pid, start) => vec![(*pid, *start)],
            RowKey::Group(key) => data
                .procs
                .iter()
                .zip(&data.keys)
                .filter(|(_, k)| *k == key)
                .map(|(p, _)| (p.pid, p.start_time))
                .collect(),
        };
        let count = int(targets.len());
        let subject = subject_of(data, &row);
        self.as_mut().rust_mut().pinned_subject = Some(subject);
        self.as_mut().rust_mut().pinned = targets;
        self.as_mut()
            .set_pinned_name(QString::from(&*row.proc.name));
        self.as_mut().set_pinned_count(count);
        true
    }

    pub fn act(&self, action: i32) -> QString {
        let action = match action {
            0 => Action::End,
            1 => Action::Kill,
            2 => Action::Stop,
            3 => Action::Continue,
            _ => return QString::from("failed"),
        };
        // Every member is tried; the worst answer is the one reported. A
        // member that exited in the meantime was what was wanted anyway; a
        // pid reused since fails the start-time check and is left alone.
        let mut done = 0;
        let mut worst = "";
        for &(pid, start) in &self.rust().pinned {
            match process::act(pid, start, action) {
                Ok(()) => done += 1,
                Err(ActionError::Gone) => {}
                Err(ActionError::NotAllowed) => worst = "denied",
                Err(e) => {
                    log::warn!("{action:?} for {pid}: {e}");
                    if worst.is_empty() {
                        worst = "failed";
                    }
                }
            }
        }
        if worst.is_empty() && done == 0 {
            worst = "gone";
        }
        QString::from(worst)
    }

    pub fn show_details(&self) {
        let r = self.rust();
        if let (Some(subject), Some(details)) = (&r.pinned_subject, &r.details) {
            let subject = subject.clone();
            let _ = details.queue(move |d| d.show(subject));
        }
    }

    pub fn open_location(mut self: Pin<&mut Self>) {
        let targets = self.rust().pinned.clone();
        if targets.is_empty() {
            return;
        }
        let name = self.pinned_name().clone().to_string();
        let qt = self.as_mut().qt_thread();
        let named = name.clone();
        let spawned = std::thread::Builder::new()
            .name("open-location".into())
            .spawn(move || {
                let name = named;
                // The first member still running whose program can be found:
                // an application's processes are mostly one program.
                let exe = targets.iter().find_map(|&(pid, start)| {
                    let exe = process::executable(pid)?;
                    (process::start_time(pid) == Some(start)).then_some(exe)
                });
                let folder = match &exe {
                    None => String::new(),
                    Some(exe) => match files::show_in_file_manager(exe) {
                        Shown::Yes => return,
                        Shown::NoFileManager => {
                            exe.parent().map(files::file_uri).unwrap_or_default()
                        }
                    },
                };
                let _ = qt.queue(move |o| o.located(QString::from(&name), QString::from(&folder)));
            });
        if let Err(e) = spawned {
            log::error!("finding a program's folder: {e}");
            self.located(QString::from(&name), QString::default());
        }
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        self.row(index.row())
            .map_or_else(QVariant::default, |row| row.data(role))
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::default();
        for (i, name) in ROLES.iter().enumerate() {
            roles.insert(FIRST_ROLE + int(i), QByteArray::from(*name));
        }
        roles
    }

    pub fn row_count(&self, parent: &QModelIndex) -> i32 {
        // A list: only the root has rows.
        if parent.is_valid() {
            0
        } else {
            int(self.rust().rows.len())
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn k(n: u32) -> RowKey {
        RowKey::Proc(n, 0)
    }

    #[test]
    fn a_search_shows_only_the_members_it_finds() {
        let data = tick(vec![proc(1, "sh", 1.0), proc(2, "sh", 2.0)]);
        let mut view = View::default();
        view.expanded.insert(GroupKey::Process("sh".into()));
        // Found by name, the whole group shows; by one pid, that member.
        view.search = Search::new("sh");
        assert_eq!(layout(&data, &view, &HashMap::new()).len(), 3);
        view.search = Search::new("2");
        let rows = layout(&data, &view, &HashMap::new());
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[1].key, RowKey::Proc(2, 0));
    }

    #[test]
    fn a_row_reports_only_the_roles_that_changed() {
        let row = Row {
            key: k(1),
            icon: "sh".into(),
            proc: proc(1, "sh", 1.0),
            count: 1,
            depth: 0,
            expanded: false,
        };
        // Every role has a value, and a row hasn't changed from itself,
        // its figures the machine doesn't report (NaN) included.
        assert_eq!(crate::rows::changed(&row, &row.clone()), 0);
        let mut busier = row.clone();
        busier.proc.cpu = 2.0;
        // Not shown: no role changes.
        busier.proc.parent = 7;
        let bit = |role: &str| 1 << ROLES.iter().position(|r| *r == role).unwrap();
        // The power impact is worked out from the CPU, among others.
        let mask = crate::rows::changed(&row, &busier);
        assert_eq!(mask & !bit("power"), bit("cpu"));
        // A group's pid shows as 0, so it changes with the count.
        let mut group = row.clone();
        group.count = 2;
        assert_eq!(
            crate::rows::changed(&row, &group),
            bit("pid") | bit("expandable") | bit("count")
        );
    }

    fn proc(pid: u32, name: &str, cpu: f64) -> Proc {
        Proc {
            pid,
            start_time: 0,
            name: name.into(),
            parent: 1,
            kernel: false,
            unit: None,
            container: None,
            cpu,
            memory: 0,
            gpu: None,
            net_in: None,
            net_out: None,
            disk_read: None,
            disk_write: None,
        }
    }

    fn tick(procs: Vec<Proc>) -> AppsTick {
        let mut resolver = apps::Resolver::new(apps::desktop::Index::new(Vec::new()), None);
        let groups = apps::Grouper::default()
            .group(&procs, &mut resolver)
            .to_vec();
        let keys = procs.iter().map(|p| resolver.key_of(p)).collect();
        AppsTick {
            apps: vec![None; procs.len()],
            procs,
            keys,
            groups,
            first: false,
        }
    }

    fn names(rows: &[Row]) -> Vec<String> {
        rows.iter().map(|r| r.proc.name.to_string()).collect()
    }

    #[test]
    fn groups_sort_and_open() {
        let data = tick(vec![
            proc(1, "bash", 1.0),
            proc(2, "kwin", 9.0),
            proc(3, "bash", 3.0),
        ]);
        let mut view = View::default();
        let rows = layout(&data, &view, &HashMap::new());
        // bash's two processes sum to 4%, below kwin's 9%.
        assert_eq!(names(&rows), ["kwin", "bash"]);
        assert_eq!(rows[1].count, 2);
        assert!((rows[1].proc.cpu - 4.0).abs() < 1e-9);

        view.expanded.insert(GroupKey::Process("bash".into()));
        let rows = layout(&data, &view, &HashMap::new());
        assert_eq!(names(&rows), ["kwin", "bash", "bash", "bash"]);
        assert_eq!(rows[2].key, RowKey::Proc(3, 0), "busiest member first");
        assert_eq!(rows[2].depth, 1);

        view.grouped = false;
        view.search = Search::new("bash");
        let rows = layout(&data, &view, &HashMap::new());
        assert_eq!(rows.len(), 2);
    }

    #[test]
    fn held_rows_keep_their_places() {
        let shown: HashMap<RowKey, usize> = [(k(1), 0), (k(2), 1)].into_iter().collect();
        let data = tick(vec![
            proc(1, "a", 1.0),
            proc(2, "b", 50.0),
            proc(3, "c", 99.0),
        ]);
        let view = View {
            grouped: false,
            held: true,
            ..View::default()
        };
        // Held, b doesn't jump above a, and c comes in at the end.
        assert_eq!(names(&layout(&data, &view, &shown)), ["a", "b", "c"]);
        let view = View {
            held: false,
            ..view
        };
        assert_eq!(names(&layout(&data, &view, &shown)), ["c", "b", "a"]);
    }
}
