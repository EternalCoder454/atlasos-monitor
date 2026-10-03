//! The Services table: the system's services from the sampling loop's
//! reader (with All Unit Types, its sockets, timers, mounts and the rest
//! too), searched, filtered to the failed ones and sorted here, and
//! changed by row diffs (`rows.rs`). Start, Stop, Restart, Enable and
//! Disable go to systemd on a thread of their own, since polkit may ask for
//! a password first: the GUI thread never waits on it.

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
        /// The list has been read at least once.
        #[qproperty(bool, loaded)]
        /// systemd answered the last read: without it the list is the last
        /// one it gave, if any.
        #[qproperty(bool, available)]
        /// Failed units of the kinds shown (services, or every kind with
        /// all unit types on), whether the search shows them or not.
        #[qproperty(i32, failed, cxx_name = "failedCount")]
        /// An action is under way; another waits for it.
        #[qproperty(bool, busy)]
        #[namespace = "atlas_monitor"]
        type ServiceModel = super::ServiceModelRust;
    }

    unsafe extern "RustQt" {
        /// Sorts by a column's role: "status" (failed first, descending),
        /// "name", "description" or "startup".
        #[qinvokable]
        #[cxx_name = "sortBy"]
        fn sort_by(self: Pin<&mut ServiceModel>, role: &QString, descending: bool);

        #[qinvokable]
        #[cxx_name = "setSearch"]
        fn set_search(self: Pin<&mut ServiceModel>, text: &QString);

        /// Only the failed services.
        #[qinvokable]
        #[cxx_name = "setProblemsOnly"]
        fn set_problems_only(self: Pin<&mut ServiceModel>, on: bool);

        /// Sockets, timers, mounts and the other kinds that can be
        /// started too, not just services.
        #[qinvokable]
        #[cxx_name = "setAllTypes"]
        fn set_all_types(self: Pin<&mut ServiceModel>, on: bool);

        /// A row's unit name: the key a menu keeps, so what it acts on is
        /// the service it was opened for, wherever its row has gone.
        #[qinvokable]
        #[cxx_name = "nameAt"]
        fn name_at(self: &ServiceModel, row: i32) -> QString;

        /// The actions that apply to a row as it stands, a bit for each
        /// (1 << Start ... 1 << Disable): what its menu offers.
        #[qinvokable]
        #[cxx_name = "availableAt"]
        fn available_at(self: &ServiceModel, row: i32) -> i32;

        /// Start (0), Stop (1), Restart (2), Enable (3) or Disable (4) for
        /// the service `name`, answered by `acted`. False when one is
        /// already under way, or for a name that isn't a service.
        #[qinvokable]
        fn act(self: Pin<&mut ServiceModel>, name: &QString, action: i32) -> bool;

        /// How an action went: "" when done, else "stillRunning" (a slow
        /// start goes on), "notEnableable", "notAllowed", "noSuchUnit",
        /// "masked", "failed", "dependency", "timeout", "canceled",
        /// "refused" (with systemd's message as `detail`) or "noAnswer".
        #[qsignal]
        fn acted(
            self: Pin<&mut ServiceModel>,
            name: QString,
            action: i32,
            result: QString,
            detail: QString,
        );

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(
            self: Pin<&mut ServiceModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut ServiceModel>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(
            self: Pin<&mut ServiceModel>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut ServiceModel>);
        #[inherit]
        #[cxx_name = "beginMoveRows"]
        fn begin_move_rows(
            self: Pin<&mut ServiceModel>,
            source_parent: &QModelIndex,
            source_first: i32,
            source_last: i32,
            destination_parent: &QModelIndex,
            destination_child: i32,
        ) -> bool;
        #[inherit]
        #[cxx_name = "endMoveRows"]
        fn end_move_rows(self: Pin<&mut ServiceModel>);
        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut ServiceModel>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut ServiceModel>);
        #[inherit]
        fn index(self: &ServiceModel, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut ServiceModel>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        #[cxx_override]
        fn data(self: &ServiceModel, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &ServiceModel) -> QHash_i32_QByteArray;
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &ServiceModel, parent: &QModelIndex) -> i32;
    }

    impl cxx_qt::Threading for ServiceModel {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn service_model_make_unique() -> UniquePtr<ServiceModel>;
    }
}

use std::cmp::Ordering;
use std::collections::HashSet;
use std::panic::AssertUnwindSafe;
use std::pin::Pin;

use atlas_sysinfo::services::{
    self, Action, ActionError, ActiveState, FileState, JobResult, LoadState, Outcome, Service,
    Status,
};
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};

use crate::rows::int;
use crate::sampler::qobject::Sampler;

#[derive(Debug, Clone, PartialEq)]
struct Row {
    /// The unit name.
    key: String,
    service: Service,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Column {
    Status,
    Name,
    Description,
    Startup,
}

#[derive(Debug, Clone)]
struct View {
    column: Column,
    descending: bool,
    /// Lowercased.
    search: String,
    problems_only: bool,
    all_types: bool,
}

impl Default for View {
    fn default() -> Self {
        Self {
            column: Column::Status,
            descending: true,
            search: String::new(),
            problems_only: false,
            all_types: false,
        }
    }
}

#[derive(Default)]
pub struct ServiceModelRust {
    count: i32,
    loaded: bool,
    available: bool,
    failed: i32,
    busy: bool,
    rows: Vec<Row>,
    services: Vec<Service>,
    view: View,
    /// Told when an action finishes, so the list reads the unit files again
    /// even if the page has closed in the meantime.
    pub sampler: Option<Box<CxxQtThread<Sampler>>>,
}

const ROLES: [&str; 5] = ["name", "description", "status", "startup", "job"];
/// Qt::UserRole: roles below it are Qt's own.
const FIRST_ROLE: i32 = 0x0100;

fn status_key(s: Status) -> &'static str {
    match s {
        Status::Running => "running",
        Status::Active => "active",
        Status::Starting => "starting",
        Status::Stopping => "stopping",
        Status::Stopped => "stopped",
        Status::Failed => "failed",
    }
}

/// How much a state asks for attention: the descending status sort.
fn rank(s: Status) -> u8 {
    match s {
        Status::Failed => 5,
        Status::Starting => 4,
        Status::Stopping => 3,
        Status::Running => 2,
        Status::Active => 1,
        Status::Stopped => 0,
    }
}

fn startup_key(f: Option<&FileState>) -> &str {
    match f {
        None => "",
        Some(FileState::Enabled) => "enabled",
        Some(FileState::EnabledRuntime) => "enabled-runtime",
        Some(FileState::Linked) => "linked",
        Some(FileState::LinkedRuntime) => "linked-runtime",
        Some(FileState::Alias) => "alias",
        Some(FileState::Masked) => "masked",
        Some(FileState::MaskedRuntime) => "masked-runtime",
        Some(FileState::Static) => "static",
        Some(FileState::Disabled) => "disabled",
        Some(FileState::Indirect) => "indirect",
        Some(FileState::Generated) => "generated",
        Some(FileState::Transient) => "transient",
        Some(FileState::Bad) => "bad",
        Some(FileState::Other(s)) => s,
    }
}

/// What the menu offers for a service as it stands.
struct Can {
    start: bool,
    stop: bool,
    restart: bool,
    enable: bool,
    disable: bool,
}

fn can(s: &Service) -> Can {
    let status = s.status();
    let startable = !matches!(s.load, LoadState::Masked | LoadState::NotFound)
        && !matches!(
            s.file_state,
            Some(FileState::Masked | FileState::MaskedRuntime)
        );
    let up = matches!(status, Status::Running | Status::Active | Status::Starting)
        || s.active == ActiveState::Reloading;
    // Not a target (see `services::runs_by_hand`). A file system or swap
    // only stops, behind a question: restarting one unmounts it unasked.
    let runs = services::runs_by_hand(&s.name);
    let storage = matches!(
        services::kind(&s.name),
        Some("mount" | "automount" | "swap")
    );
    Can {
        start: runs && startable && matches!(status, Status::Stopped | Status::Failed),
        stop: runs && up,
        restart: runs
            && !storage
            && startable
            && matches!(status, Status::Running | Status::Active | Status::Failed),
        enable: s.file_state.as_ref().is_some_and(FileState::can_enable),
        disable: s.file_state.as_ref().is_some_and(FileState::can_disable),
    }
}

fn fold_cmp(a: &str, b: &str) -> Ordering {
    a.chars()
        .flat_map(char::to_lowercase)
        .cmp(b.chars().flat_map(char::to_lowercase))
}

/// Whether the unit is of a kind shown: services, or every kind.
fn kind_shown(s: &Service, view: &View) -> bool {
    view.all_types || services::kind(&s.name) == Some("service")
}

fn matches(s: &Service, view: &View) -> bool {
    if !kind_shown(s, view) || view.problems_only && s.status() != Status::Failed {
        return false;
    }
    view.search.is_empty()
        || s.name.to_lowercase().contains(&view.search)
        || s.description.to_lowercase().contains(&view.search)
}

/// The failed units of the kinds shown, by name, as the rows are.
fn failed_count(services: &[Service], view: &View) -> i32 {
    int(services
        .iter()
        .filter(|s| kind_shown(s, view) && s.status() == Status::Failed)
        .map(|s| &s.name)
        .collect::<HashSet<_>>()
        .len())
}

/// The rows to show: the matching services, sorted, ties by name.
fn layout(services: &[Service], view: &View) -> Vec<Row> {
    let mut shown: Vec<&Service> = services.iter().filter(|s| matches(s, view)).collect();
    shown.sort_by(|a, b| {
        let by = match view.column {
            Column::Status => rank(a.status()).cmp(&rank(b.status())),
            Column::Name => fold_cmp(&a.name, &b.name),
            Column::Description => fold_cmp(&a.description, &b.description),
            Column::Startup => {
                startup_key(a.file_state.as_ref()).cmp(startup_key(b.file_state.as_ref()))
            }
        };
        let by = if view.descending { by.reverse() } else { by };
        by.then_with(|| fold_cmp(&a.name, &b.name))
    });
    let mut seen = std::collections::HashSet::new();
    shown
        .into_iter()
        .filter(|s| seen.insert(&s.name))
        .map(|s| Row {
            key: s.name.clone(),
            service: s.clone(),
        })
        .collect()
}

fn action_of(n: i32) -> Option<Action> {
    Some(match n {
        0 => Action::Start,
        1 => Action::Stop,
        2 => Action::Restart,
        3 => Action::Enable,
        4 => Action::Disable,
        _ => return None,
    })
}

/// An action's answer for `acted`: its key and systemd's message.
fn result_of(r: Result<Outcome, ActionError>) -> (&'static str, String) {
    match r {
        Ok(Outcome::Done) => ("", String::new()),
        Ok(Outcome::StillRunning) => ("stillRunning", String::new()),
        Ok(Outcome::NotEnableable) => ("notEnableable", String::new()),
        Err(ActionError::InvalidName | ActionError::NoSuchUnit) => ("noSuchUnit", String::new()),
        Err(ActionError::NotAllowed) => ("notAllowed", String::new()),
        Err(ActionError::Masked) => ("masked", String::new()),
        Err(ActionError::Job(JobResult::Failed | JobResult::Other(_))) => ("failed", String::new()),
        Err(ActionError::Job(JobResult::Dependency)) => ("dependency", String::new()),
        Err(ActionError::Job(JobResult::Timeout)) => ("timeout", String::new()),
        Err(ActionError::Job(JobResult::Canceled)) => ("canceled", String::new()),
        Err(ActionError::Refused(m)) => ("refused", m),
        Err(ActionError::NoAnswer) => ("noAnswer", String::new()),
    }
}

crate::rows::row_model!(qobject::ServiceModel, Row, "Services");

impl qobject::ServiceModel {
    /// A read from the sampling loop: the services, or `None` when systemd
    /// didn't answer, which keeps the last list.
    pub fn apply(mut self: Pin<&mut Self>, list: Option<Vec<Service>>) {
        self.as_mut().set_available(list.is_some());
        let Some(list) = list else {
            self.as_mut().set_loaded(true);
            return;
        };
        self.as_mut().rust_mut().services = list;
        self.as_mut().relayout();
        self.as_mut().set_loaded(true);
    }

    fn relayout(mut self: Pin<&mut Self>) {
        let (new, failed) = {
            let r = self.rust();
            (
                layout(&r.services, &r.view),
                failed_count(&r.services, &r.view),
            )
        };
        self.as_mut().replace_rows(new);
        if failed != *self.failed() {
            self.as_mut().set_failed(failed);
        }
    }

    pub fn sort_by(mut self: Pin<&mut Self>, role: &QString, descending: bool) {
        let column = match role.to_string().as_str() {
            "status" => Column::Status,
            "name" => Column::Name,
            "description" => Column::Description,
            "startup" => Column::Startup,
            _ => return,
        };
        {
            let mut r = self.as_mut().rust_mut();
            r.view.column = column;
            r.view.descending = descending;
        }
        self.relayout();
    }

    pub fn set_search(mut self: Pin<&mut Self>, text: &QString) {
        self.as_mut().rust_mut().view.search = text.to_string().trim().to_lowercase();
        self.relayout();
    }

    pub fn set_problems_only(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().view.problems_only = on;
        self.relayout();
    }

    pub fn set_all_types(mut self: Pin<&mut Self>, on: bool) {
        self.as_mut().rust_mut().view.all_types = on;
        self.relayout();
    }

    fn row(&self, row: i32) -> Option<&Row> {
        self.rust().rows.get(usize::try_from(row).ok()?)
    }

    pub fn name_at(&self, row: i32) -> QString {
        self.row(row)
            .map(|r| QString::from(&r.key))
            .unwrap_or_default()
    }

    pub fn available_at(&self, row: i32) -> i32 {
        self.row(row).map_or(0, |r| {
            let c = can(&r.service);
            [c.start, c.stop, c.restart, c.enable, c.disable]
                .iter()
                .enumerate()
                .filter(|(_, on)| **on)
                .fold(0, |bits, (i, _)| bits | 1 << i)
        })
    }

    pub fn act(mut self: Pin<&mut Self>, name: &QString, action: i32) -> bool {
        let name = name.to_string();
        let Some(what) = action_of(action) else {
            return false;
        };
        if *self.busy() || !services::valid_name(&name) {
            return false;
        }
        self.as_mut().set_busy(true);
        let qt = self.qt_thread();
        let sampler = self.rust().sampler.as_deref().cloned();
        let unit = name.clone();
        let spawned = std::thread::Builder::new()
            .name("service-action".into())
            .spawn(move || {
                // A panic in the bus code still answers: `busy` is cleared
                // only by the answer.
                let (result, detail) = std::panic::catch_unwind(AssertUnwindSafe(|| {
                    result_of(services::act(&unit, what))
                }))
                .unwrap_or_else(|_| ("noAnswer", String::new()));
                if let Some(s) = sampler {
                    let _ = s.queue(|s| s.services_changed());
                }
                let _ = qt.queue(move |mut o| {
                    o.as_mut().set_busy(false);
                    o.acted(
                        QString::from(&unit),
                        action,
                        QString::from(result),
                        QString::from(&detail),
                    );
                });
            });
        if let Err(e) = spawned {
            log::error!("starting a service action: {e}");
            self.as_mut().set_busy(false);
            return false;
        }
        true
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = self.row(index.row()) else {
            return QVariant::default();
        };
        let s = &row.service;
        let Some(&name) = usize::try_from(role - FIRST_ROLE)
            .ok()
            .and_then(|i| ROLES.get(i))
        else {
            return QVariant::default();
        };
        let text = |t: &str| QVariant::from(&QString::from(t));
        match name {
            "name" => text(&s.name),
            "description" => text(&s.description),
            "status" => text(status_key(s.status())),
            "startup" => text(startup_key(s.file_state.as_ref())),
            "job" => text(s.job.as_deref().unwrap_or_default()),
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

    fn service(name: &str, description: &str, active: ActiveState, sub: &str) -> Service {
        Service {
            name: name.into(),
            description: description.into(),
            load: LoadState::Loaded,
            active,
            sub: sub.into(),
            file_state: Some(FileState::Enabled),
            job: None,
        }
    }

    fn names(rows: &[Row]) -> Vec<&str> {
        rows.iter().map(|r| r.key.as_str()).collect()
    }

    #[test]
    fn failed_first_then_by_name_and_filters_stack() {
        let list = vec![
            service("b.service", "Bee", ActiveState::Active, "running"),
            service("A.service", "Ay", ActiveState::Inactive, "dead"),
            service("z.service", "Zed web server", ActiveState::Failed, "failed"),
        ];
        let mut view = View::default();
        assert_eq!(
            names(&layout(&list, &view)),
            ["z.service", "b.service", "A.service"]
        );
        view.column = Column::Name;
        view.descending = false;
        assert_eq!(
            names(&layout(&list, &view)),
            ["A.service", "b.service", "z.service"]
        );
        view.search = "web".into();
        assert_eq!(names(&layout(&list, &view)), ["z.service"]);
        view.search = "bee".into();
        view.problems_only = true;
        assert!(layout(&list, &view).is_empty());
    }

    #[test]
    fn other_unit_types_only_when_asked() {
        let list = vec![
            service("a.service", "", ActiveState::Active, "running"),
            service("b.socket", "", ActiveState::Failed, "failed"),
            service("c.timer", "", ActiveState::Active, "waiting"),
            service("d.service", "", ActiveState::Failed, "failed"),
        ];
        let mut view = View {
            column: Column::Name,
            descending: false,
            ..View::default()
        };
        assert_eq!(names(&layout(&list, &view)), ["a.service", "d.service"]);
        assert_eq!(failed_count(&list, &view), 1);
        view.all_types = true;
        assert_eq!(
            names(&layout(&list, &view)),
            ["a.service", "b.socket", "c.timer", "d.service"]
        );
        assert_eq!(failed_count(&list, &view), 2);
    }

    #[test]
    fn the_menu_offers_what_applies() {
        let running = service("a.service", "", ActiveState::Active, "running");
        let c = can(&running);
        assert!(!c.start && c.stop && c.restart && !c.enable && c.disable);
        let mut masked = service("b.service", "", ActiveState::Inactive, "dead");
        masked.file_state = Some(FileState::Masked);
        let c = can(&masked);
        assert!(!c.start && !c.stop && !c.restart && !c.enable && !c.disable);
        let failed = service("c.service", "", ActiveState::Failed, "failed");
        let c = can(&failed);
        assert!(c.start && !c.stop && c.restart);
        // A target is only switched on or off at boot.
        let target = service("reboot.target", "", ActiveState::Inactive, "dead");
        let c = can(&target);
        assert!(!c.start && !c.stop && !c.restart && !c.enable && c.disable);
        let mount = service("home.mount", "", ActiveState::Active, "mounted");
        let c = can(&mount);
        assert!(!c.start && c.stop && !c.restart);
    }
}
