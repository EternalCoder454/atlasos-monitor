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

        /// Keeps the rows in their places while the pointer is over them.
        #[qinvokable]
        #[cxx_name = "setHeld"]
        fn set_held(self: Pin<&mut ProcessModel>, held: bool);

        /// Opens or closes an application's row.
        #[qinvokable]
        fn toggle(self: Pin<&mut ProcessModel>, row: i32);

        /// End Task (0), Kill (1), Stop (2) or Continue (3) for a row: every
        /// process of an application's. "" when done, else "gone" (it had
        /// exited), "denied" (another user's) or "failed".
        #[qinvokable]
        fn act(self: &ProcessModel, row: i32, action: i32) -> QString;

        /// A row's name and how many processes it stands for, for the
        /// confirmations.
        #[qinvokable]
        #[cxx_name = "nameAt"]
        fn name_at(self: &ProcessModel, row: i32) -> QString;
        #[qinvokable]
        #[cxx_name = "countAt"]
        fn count_at(self: &ProcessModel, row: i32) -> i32;

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
use atlas_sysinfo::process::{self, Action, ActionError, Proc};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{
    QByteArray, QHash, QHashPair_i32_QByteArray, QList, QModelIndex, QString, QVariant,
};

use crate::sampling::AppsTick;

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
    rows: Vec<Row>,
    data: AppsTick,
    view: View,
}

impl Default for ProcessModelRust {
    fn default() -> Self {
        Self {
            count: 0,
            grouped: true,
            rows: Vec::new(),
            data: AppsTick::default(),
            view: View::default(),
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
/// Qt::UserRole: roles below it are Qt's own.
const FIRST_ROLE: i32 = 0x0100;

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
        return ordered(procs, idx, proc_key, shown, view)
            .into_iter()
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
    for g in ordered(groups, idx, group_key, shown, view) {
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
            for i in ordered(procs, members[g].clone(), proc_key, shown, view) {
                rows.push(proc_row(i, icon.clone(), 1));
            }
        }
    }
    rows
}

/// One step from the rows shown to the rows wanted.
#[derive(Debug, Clone, PartialEq)]
enum Op {
    /// Rows `first..=last` go.
    Remove(usize, usize),
    /// The row at `from` moves up to `to`.
    Move(usize, usize),
    /// `n` new rows from `at`.
    Insert(usize, usize),
}

/// The steps that turn `old` into `new`: removals from the bottom up, then
/// each place filled in order by a row moved up or a new one.
fn plan(old: &[RowKey], new: &[RowKey]) -> Vec<Op> {
    let wanted: HashSet<&RowKey> = new.iter().collect();
    let mut ops = Vec::new();
    let mut cur: Vec<&RowKey> = old.iter().collect();
    let mut i = cur.len();
    while i > 0 {
        i -= 1;
        if !wanted.contains(cur[i]) {
            let last = i;
            while i > 0 && !wanted.contains(cur[i - 1]) {
                i -= 1;
            }
            ops.push(Op::Remove(i, last));
            cur.drain(i..=last);
        }
    }
    for (i, k) in new.iter().enumerate() {
        if cur.get(i) == Some(&k) {
            continue;
        }
        if let Some(j) = cur[i..].iter().position(|c| *c == k).map(|j| j + i) {
            ops.push(Op::Move(j, i));
            let moved = cur.remove(j);
            cur.insert(i, moved);
        } else {
            match ops.last_mut() {
                Some(Op::Insert(at, n)) if *at + *n == i => *n += 1,
                _ => ops.push(Op::Insert(i, 1)),
            }
            cur.insert(i, k);
        }
    }
    ops
}

fn int(v: usize) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

fn opt(v: Option<f64>) -> f64 {
    v.unwrap_or(f64::NAN)
}

impl qobject::ProcessModel {
    /// A tick from the sampling thread.
    pub fn apply(mut self: Pin<&mut Self>, data: AppsTick) {
        self.as_mut().rust_mut().data = data;
        self.relayout();
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
        let old: Vec<RowKey> = self.rust().rows.iter().map(|r| r.key.clone()).collect();
        let keys: Vec<RowKey> = new.iter().map(|r| r.key.clone()).collect();
        let root = QModelIndex::default();
        for op in plan(&old, &keys) {
            match op {
                Op::Remove(first, last) => {
                    self.as_mut()
                        .begin_remove_rows(&root, int(first), int(last));
                    self.as_mut().rust_mut().rows.drain(first..=last);
                    self.as_mut().end_remove_rows();
                }
                Op::Move(from, to) => {
                    // `to` is above `from`, so Qt's destination is `to`.
                    if self
                        .as_mut()
                        .begin_move_rows(&root, int(from), int(from), &root, int(to))
                    {
                        let rows = &mut self.as_mut().rust_mut().rows;
                        let row = rows.remove(from);
                        rows.insert(to, row);
                        self.as_mut().end_move_rows();
                    }
                }
                Op::Insert(at, n) => {
                    self.as_mut()
                        .begin_insert_rows(&root, int(at), int(at + n - 1));
                    self.as_mut()
                        .rust_mut()
                        .rows
                        .splice(at..at, new[at..at + n].iter().cloned());
                    self.as_mut().end_insert_rows();
                }
            }
        }
        // The figures: one report covering every row that changed.
        let mut changed: Option<(usize, usize)> = None;
        {
            let mut r = self.as_mut().rust_mut();
            for (i, (row, want)) in r.rows.iter_mut().zip(new).enumerate() {
                if *row != want {
                    *row = want;
                    changed = Some(changed.map_or((i, i), |(a, _)| (a, i)));
                }
            }
        }
        if let Some((a, b)) = changed {
            let top = self.index(int(a), 0, &root);
            let bottom = self.index(int(b), 0, &root);
            self.as_mut().data_changed(&top, &bottom, &QList::default());
        }
        let count = int(self.rust().rows.len());
        if count != *self.count() {
            self.as_mut().set_count(count);
        }
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
        self.as_mut().rust_mut().view.grouped = on;
        self.as_mut().set_grouped(on);
        self.relayout_unheld();
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

    pub fn name_at(&self, row: i32) -> QString {
        self.row(row)
            .map(|r| QString::from(&*r.proc.name))
            .unwrap_or_default()
    }

    pub fn count_at(&self, row: i32) -> i32 {
        self.row(row).map_or(0, |r| int(r.count as usize))
    }

    pub fn act(&self, row: i32, action: i32) -> QString {
        let action = match action {
            0 => Action::End,
            1 => Action::Kill,
            2 => Action::Stop,
            3 => Action::Continue,
            _ => return QString::from("failed"),
        };
        let Some(row) = self.row(row) else {
            return QString::from("gone");
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
        // Every member is tried; the worst answer is the one reported. A
        // member that exited in the meantime was what was wanted anyway.
        let mut done = 0;
        let mut worst = "";
        for (pid, start) in targets {
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

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = self.row(index.row()) else {
            return QVariant::default();
        };
        let p = &row.proc;
        let Some(&name) = usize::try_from(role - FIRST_ROLE)
            .ok()
            .and_then(|i| ROLES.get(i))
        else {
            return QVariant::default();
        };
        match name {
            "name" => QVariant::from(&QString::from(&*p.name)),
            "icon" => QVariant::from(&QString::from(&row.icon)),
            // A row of several processes has no one pid to show.
            "pid" => QVariant::from(&if row.count == 1 {
                int(p.pid as usize)
            } else {
                0
            }),
            "cpu" => QVariant::from(&p.cpu),
            "memory" => QVariant::from(&(p.memory as f64)),
            "gpu" => QVariant::from(&opt(p.gpu)),
            "power" => QVariant::from(&(p.impact() as i32)),
            "netIn" => QVariant::from(&opt(p.net_in)),
            "netOut" => QVariant::from(&opt(p.net_out)),
            "diskRead" => QVariant::from(&opt(p.disk_read)),
            "diskWrite" => QVariant::from(&opt(p.disk_write)),
            "depth" => QVariant::from(&row.depth),
            "expandable" => QVariant::from(&(row.count > 1)),
            "expanded" => QVariant::from(&row.expanded),
            "count" => QVariant::from(&int(row.count as usize)),
            _ => QVariant::default(),
        }
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

    /// Plays a plan on `old`, as the model does.
    fn play(old: &[RowKey], new: &[RowKey]) -> Vec<RowKey> {
        let mut cur = old.to_vec();
        for op in plan(old, new) {
            match op {
                Op::Remove(a, b) => {
                    cur.drain(a..=b);
                }
                Op::Move(from, to) => {
                    assert!(to < from, "rows only move up");
                    let r = cur.remove(from);
                    cur.insert(to, r);
                }
                Op::Insert(at, n) => {
                    for (j, key) in new[at..at + n].iter().enumerate() {
                        cur.insert(at + j, key.clone());
                    }
                }
            }
        }
        cur
    }

    #[test]
    fn plans_reach_the_new_rows() {
        let cases: [(&[u32], &[u32]); 7] = [
            (&[], &[1, 2, 3]),
            (&[1, 2, 3], &[]),
            (&[1, 2, 3], &[1, 2, 3]),
            (&[1, 2, 3], &[3, 2, 1]),
            (&[1, 2, 3, 4, 5], &[2, 6, 4, 7, 8]),
            (&[1, 2, 3, 4], &[1, 3]),
            (&[5, 1, 2], &[1, 2, 5, 9]),
        ];
        for (old, new) in cases {
            let old: Vec<_> = old.iter().map(|&n| k(n)).collect();
            let new: Vec<_> = new.iter().map(|&n| k(n)).collect();
            assert_eq!(play(&old, &new), new, "{old:?} -> {new:?}");
        }
    }

    #[test]
    fn an_unchanged_order_is_no_step_and_new_rows_come_together() {
        let old = [k(1), k(2)];
        assert!(plan(&old, &old).is_empty());
        assert_eq!(
            plan(&old, &[k(1), k(2), k(3), k(4)]),
            vec![Op::Insert(2, 2)]
        );
        assert_eq!(plan(&[k(1), k(2), k(3)], &[k(3)]), vec![Op::Remove(0, 1)]);
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
