//! The Energy Saver page: the applications keeping the processor busy, each
//! with Ease Off (or Put Back), and the switch that eases them automatically.
//!
//! The controller (`atlas_sysinfo::ease`) runs on a thread of its own for as
//! long as the window is open, whatever page is up: automatic easing must
//! keep watching, and its weights go through the user's systemd manager,
//! which may be slow to answer. It posts the rows to this model after every
//! tick and every action. Dropping the model (the app closing) stops the
//! thread, which puts back what was eased automatically.

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
        /// The controller has answered: rows, or why there are none.
        #[qproperty(bool, loaded)]
        /// Why Energy Saver can't work here: "" when it can, else
        /// "noAppUnits", "noCpuController", "noManager" or "noPipeWire".
        #[qproperty(QString, unavailable)]
        /// Busy applications are eased off automatically.
        #[qproperty(bool, automatic)]
        /// An Ease Off or Put Back is on its way: the manager may take a
        /// few seconds.
        #[qproperty(bool, busy)]
        #[namespace = "atlas_monitor"]
        type EnergySaver = super::EnergySaverRust;
    }

    unsafe extern "RustQt" {
        #[qinvokable]
        #[cxx_name = "setAutomaticEasing"]
        fn set_automatic_easing(self: Pin<&mut EnergySaver>, on: bool);

        /// Eases the application `id` off, or puts it back.
        #[qinvokable]
        fn ease(self: Pin<&mut EnergySaver>, id: &QString);
        #[qinvokable]
        fn restore(self: Pin<&mut EnergySaver>, id: &QString);

        /// Never eases `id` off automatically (or does again).
        #[qinvokable]
        #[cxx_name = "setNever"]
        fn set_never(self: Pin<&mut EnergySaver>, id: &QString, never: bool);

        /// An Ease Off or Put Back that didn't take: "noAnswer", "gone",
        /// "notAnApp" or "refused" (with the manager's message as `detail`).
        #[qsignal]
        fn failed(self: Pin<&mut EnergySaver>, name: QString, reason: QString, detail: QString);

        #[inherit]
        #[cxx_name = "beginInsertRows"]
        fn begin_insert_rows(
            self: Pin<&mut EnergySaver>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endInsertRows"]
        fn end_insert_rows(self: Pin<&mut EnergySaver>);
        #[inherit]
        #[cxx_name = "beginRemoveRows"]
        fn begin_remove_rows(
            self: Pin<&mut EnergySaver>,
            parent: &QModelIndex,
            first: i32,
            last: i32,
        );
        #[inherit]
        #[cxx_name = "endRemoveRows"]
        fn end_remove_rows(self: Pin<&mut EnergySaver>);
        #[inherit]
        #[cxx_name = "beginMoveRows"]
        fn begin_move_rows(
            self: Pin<&mut EnergySaver>,
            source_parent: &QModelIndex,
            source_first: i32,
            source_last: i32,
            destination_parent: &QModelIndex,
            destination_child: i32,
        ) -> bool;
        #[inherit]
        #[cxx_name = "endMoveRows"]
        fn end_move_rows(self: Pin<&mut EnergySaver>);
        #[inherit]
        #[cxx_name = "beginResetModel"]
        fn begin_reset_model(self: Pin<&mut EnergySaver>);
        #[inherit]
        #[cxx_name = "endResetModel"]
        fn end_reset_model(self: Pin<&mut EnergySaver>);
        #[inherit]
        fn index(self: &EnergySaver, row: i32, column: i32, parent: &QModelIndex) -> QModelIndex;
        #[inherit]
        #[qsignal]
        #[cxx_name = "dataChanged"]
        fn data_changed(
            self: Pin<&mut EnergySaver>,
            top_left: &QModelIndex,
            bottom_right: &QModelIndex,
            roles: &QList_i32,
        );

        #[cxx_override]
        fn data(self: &EnergySaver, index: &QModelIndex, role: i32) -> QVariant;
        #[cxx_override]
        #[cxx_name = "roleNames"]
        fn role_names(self: &EnergySaver) -> QHash_i32_QByteArray;
        #[cxx_override]
        #[cxx_name = "rowCount"]
        fn row_count(self: &EnergySaver, parent: &QModelIndex) -> i32;
    }

    impl cxx_qt::Threading for EnergySaver {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn energy_saver_make_unique() -> UniquePtr<EnergySaver>;
    }
}

use std::pin::Pin;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, Sender};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use atlas_sysinfo::apps::Resolver;
use atlas_sysinfo::ease::system::{self as ease_system, Unavailable};
use atlas_sysinfo::ease::{self, Controller, Error, Status, kwin};
use cxx_qt::{CxxQtThread, CxxQtType, Threading};
use cxx_qt_lib::{QByteArray, QHash, QHashPair_i32_QByteArray, QModelIndex, QString, QVariant};

use crate::rows::{FIRST_ROLE, Roles, Value, int};
use crate::settings::Energy;

/// The busiest applications listed; an eased one is listed whatever it
/// does, so its Put Back never scrolls away.
const LISTED: usize = 8;
/// The first tick has nothing to measure against: the second comes soon.
const FIRST_TICK: Duration = Duration::from_secs(2);

/// For an application with no icon of its own.
const PROGRAM_ICON: &str = "application-x-executable";

#[derive(Debug, Clone, PartialEq)]
struct Row {
    key: String,
    name: String,
    icon: String,
    cpu: f64,
    status: Status,
    never: bool,
}

/// What the page asks the controller's thread.
enum Command {
    Automatic(bool),
    Never(Vec<String>),
    Ease(String),
    Restore(String),
}

/// The controller's thread. Dropping it closes the channel, the thread's
/// signal to put back what it eased and end, and waits for that, up to
/// [`SHUTDOWN_WAIT`].
struct Runner {
    commands: Option<Sender<Command>>,
    /// Disconnects when the thread ends, however it ends.
    done: Receiver<()>,
    thread: Option<JoinHandle<()>>,
}

/// How long closing waits for the eases to be put back. A manager that
/// doesn't answer could hold it for seconds per app; what is left eased is
/// in the state file, which the next start puts back.
const SHUTDOWN_WAIT: Duration = Duration::from_secs(3);

impl Drop for Runner {
    fn drop(&mut self) {
        self.commands = None;
        match self.done.recv_timeout(SHUTDOWN_WAIT) {
            Err(RecvTimeoutError::Timeout) => {
                log::warn!("Energy Saver didn't finish putting apps back; leaving it");
            }
            _ => {
                if let Some(t) = self.thread.take() {
                    let _ = t.join();
                }
            }
        }
    }
}

#[derive(Default)]
pub struct EnergySaverRust {
    count: i32,
    loaded: bool,
    unavailable: QString,
    automatic: bool,
    busy: bool,
    /// Ease Offs and Put Backs sent and not yet answered.
    pending: u32,
    rows: Vec<Row>,
    never: Vec<String>,
    runner: Option<Runner>,
}

const ROLES: [&str; 6] = ["appId", "name", "icon", "cpu", "status", "never"];

fn status_key(s: Status) -> &'static str {
    match s {
        Status::Normal => "normal",
        Status::Busy => "busy",
        Status::EasedAuto => "easedAuto",
        Status::EasedManual => "easedManual",
        Status::KeptSound => "keptSound",
        Status::KeptTerminal => "keptTerminal",
        Status::KeptNever => "keptNever",
        Status::KeptByUser => "keptByUser",
        Status::KeptInUse => "keptInUse",
        Status::KeptOther => "keptOther",
    }
}

fn unavailable_key(u: Unavailable) -> &'static str {
    match u {
        Unavailable::NoAppUnits => "noAppUnits",
        Unavailable::NoCpuController => "noCpuController",
        Unavailable::NoManager => "noManager",
        Unavailable::NoPipeWire => "noPipeWire",
    }
}

fn error_of(e: &Error) -> (&'static str, String) {
    match e {
        Error::NoAnswer => ("noAnswer", String::new()),
        Error::Gone => ("gone", String::new()),
        Error::NotAnApp => ("notAnApp", String::new()),
        Error::Refused(m) => ("refused", m.clone()),
    }
}

/// The rows to show: the busiest few, and every eased one.
fn listed(c: &Controller, resolver: &mut Resolver) -> Vec<Row> {
    c.rows()
        .into_iter()
        .enumerate()
        .filter(|(i, r)| *i < LISTED || r.status.is_eased())
        .map(|(_, r)| Row {
            key: r.id.to_string(),
            name: r.name.to_string(),
            icon: resolver
                .of(Some(&r.unit))
                .and_then(|a| a.icon.as_ref())
                .map_or_else(|| PROGRAM_ICON.to_owned(), |i| i.source().into_owned()),
            cpu: r.cpu,
            status: r.status,
            never: false,
        })
        .collect()
}

/// Clears the page's `busy` when the controller's thread ends. On a
/// normal close the object is already gone and the queue refuses it.
struct Ended(CxxQtThread<qobject::EnergySaver>);

impl Drop for Ended {
    fn drop(&mut self) {
        let _ = self.0.queue(|mut o| {
            o.as_mut().rust_mut().pending = 0;
            o.set_busy(false);
        });
    }
}

/// The controller's thread: a tick every [`ease::TICK_EVERY`], the page's
/// commands between, and the rows posted after each.
fn run(
    commands: Receiver<Command>,
    qt: CxxQtThread<qobject::EnergySaver>,
    choices: Energy,
    theme: String,
    done: Sender<()>,
) {
    // Dropped on every way out, a panic included: Runner stops waiting,
    // and the page stops waiting for answers that won't come.
    let _done = done;
    let _ended = Ended(qt.clone());
    let mut c = match ease_system::open(ease_system::state_file()) {
        Ok(c) => c,
        Err(u) => {
            log::info!("Energy Saver is unavailable: {u}");
            let _ = qt.queue(move |o| o.set_unavailable_reason(u));
            // Nothing to put back; wait for the app to close.
            while commands.recv().is_ok() {}
            return;
        }
    };
    c.set_never(choices.never);
    c.set_automatic(choices.automatic);
    let mut resolver = Resolver::for_session(&theme);
    // The window with focus, on Plasma: the app in use is left alone. Its
    // script is unloaded when this ends, the window closed.
    let mut focus = ease_system::runtime_dir().map(kwin::Watch::start);
    let mut next = Instant::now();
    let mut first = true;
    loop {
        if Instant::now() >= next {
            let pid = focus.as_ref().map_or(0, kwin::Watch::pid);
            c.set_focused((pid != 0).then(|| ease_system::unit_of_pid(pid)).flatten());
            c.tick(&mut resolver);
            next = Instant::now() + if first { FIRST_TICK } else { ease::TICK_EVERY };
            first = false;
            let rows = listed(&c, &mut resolver);
            let _ = qt.queue(move |o| o.apply(rows));
        }
        let command = match commands.recv_timeout(next.saturating_duration_since(Instant::now())) {
            Ok(command) => command,
            Err(RecvTimeoutError::Timeout) => continue,
            Err(RecvTimeoutError::Disconnected) => break,
        };
        let acted = matches!(command, Command::Ease(_) | Command::Restore(_));
        let failed = match command {
            Command::Automatic(on) => {
                c.set_automatic(on);
                None
            }
            Command::Never(ids) => {
                c.set_never(ids);
                None
            }
            Command::Ease(id) => c.ease(&id).err().map(|e| (id, e)),
            Command::Restore(id) => c.restore(&id).err().map(|e| (id, e)),
        };
        let rows = listed(&c, &mut resolver);
        let _ = qt.queue(move |mut o| {
            if let Some((id, e)) = failed {
                log::warn!("Energy Saver, {id}: {e}");
                let name = o
                    .rust()
                    .rows
                    .iter()
                    .find(|r| r.key == id)
                    .map_or(id.clone(), |r| r.name.clone());
                let (reason, detail) = error_of(&e);
                o.as_mut().failed(
                    QString::from(&name),
                    QString::from(reason),
                    QString::from(&detail),
                );
            }
            o.as_mut().apply(rows);
            if acted {
                o.answered();
            }
        });
    }
    // The window closed: what was eased automatically goes back, since
    // nothing watches it any more. The user's own eases stay. KWin unloads
    // the focus script meanwhile; `focus` waits for that as it drops.
    if let Some(f) = &mut focus {
        f.stop();
    }
    c.shutdown();
}

impl Roles for Row {
    const NAMES: &'static [&'static str] = &ROLES;

    fn value(&self, role: usize) -> Value<'_> {
        match ROLES[role] {
            "appId" => Value::Text(&self.key),
            "name" => Value::Text(&self.name),
            "icon" => Value::Text(&self.icon),
            "cpu" => Value::Real(self.cpu),
            "status" => Value::Text(status_key(self.status)),
            "never" => Value::Bool(self.never),
            _ => unreachable!("a role in ROLES without a value"),
        }
    }
}

crate::rows::row_model!(qobject::EnergySaver, Row, "Energy Saver");

impl qobject::EnergySaver {
    /// Starts the controller's thread with the saved choices. Called once,
    /// from `lib.rs`.
    pub fn start(mut self: Pin<&mut Self>, icon_theme: String) {
        let choices = Energy::load();
        self.as_mut().set_automatic(choices.automatic);
        self.as_mut().rust_mut().never = choices.never.clone();
        let (tx, rx) = mpsc::channel();
        let (done_tx, done) = mpsc::channel();
        let qt = self.qt_thread();
        match std::thread::Builder::new()
            .name("energy-saver".into())
            .spawn(move || run(rx, qt, choices, icon_theme, done_tx))
        {
            Ok(thread) => {
                self.as_mut().rust_mut().runner = Some(Runner {
                    commands: Some(tx),
                    done,
                    thread: Some(thread),
                });
            }
            Err(e) => {
                log::error!("starting Energy Saver: {e}");
                self.as_mut().set_unavailable(QString::from("noManager"));
                self.set_loaded(true);
            }
        }
    }

    /// False when the controller's thread is gone (it panicked).
    fn send(&self, command: Command) -> bool {
        self.rust()
            .runner
            .as_ref()
            .and_then(|r| r.commands.as_ref())
            .is_some_and(|tx| tx.send(command).is_ok())
    }

    fn act(mut self: Pin<&mut Self>, id: String, command: Command) {
        if !self.unavailable().is_empty() {
            return;
        }
        self.as_mut().rust_mut().pending += 1;
        self.as_mut().set_busy(true);
        if !self.send(command) {
            // The thread is gone, and with it every answer still owed,
            // this one's and any it died before giving.
            self.as_mut().rust_mut().pending = 0;
            self.as_mut().set_busy(false);
            let name = self
                .rust()
                .rows
                .iter()
                .find(|r| r.key == id)
                .map_or(id.clone(), |r| r.name.clone());
            self.failed(
                QString::from(&name),
                QString::from("noAnswer"),
                QString::default(),
            );
        }
    }

    fn answered(mut self: Pin<&mut Self>) {
        let left = {
            let mut r = self.as_mut().rust_mut();
            r.pending = r.pending.saturating_sub(1);
            r.pending
        };
        if left == 0 {
            self.set_busy(false);
        }
    }

    fn set_unavailable_reason(mut self: Pin<&mut Self>, u: Unavailable) {
        self.as_mut()
            .set_unavailable(QString::from(unavailable_key(u)));
        self.set_loaded(true);
    }

    fn apply(mut self: Pin<&mut Self>, mut rows: Vec<Row>) {
        let never = &self.rust().never;
        for r in &mut rows {
            r.never = never.contains(&r.key);
        }
        self.as_mut().replace_rows(rows);
        self.as_mut().update_count();
        self.set_loaded(true);
    }

    pub fn set_automatic_easing(mut self: Pin<&mut Self>, on: bool) {
        if *self.automatic() == on {
            return;
        }
        self.as_mut().set_automatic(on);
        if let Err(e) = Energy::save_automatic(on) {
            log::warn!("saving Energy Saver's switch: {e}");
        }
        self.send(Command::Automatic(on));
    }

    pub fn ease(self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        self.act(id.clone(), Command::Ease(id));
    }

    pub fn restore(self: Pin<&mut Self>, id: &QString) {
        let id = id.to_string();
        self.act(id.clone(), Command::Restore(id));
    }

    pub fn set_never(mut self: Pin<&mut Self>, id: &QString, never: bool) {
        let id = id.to_string();
        let list = {
            let mut r = self.as_mut().rust_mut();
            r.never.retain(|n| *n != id);
            if never {
                r.never.push(id);
                r.never.sort();
            }
            r.never.clone()
        };
        if let Err(e) = Energy::save_never(&list) {
            log::warn!("saving Energy Saver's never list: {e}");
        }
        // The row shows it at once; the controller's next post agrees.
        let rows = self.rust().rows.clone();
        self.as_mut().apply(rows);
        self.send(Command::Never(list));
    }

    pub fn data(&self, index: &QModelIndex, role: i32) -> QVariant {
        let Some(row) = usize::try_from(index.row())
            .ok()
            .and_then(|i| self.rust().rows.get(i))
        else {
            return QVariant::default();
        };
        row.data(role)
    }

    pub fn role_names(&self) -> QHash<QHashPair_i32_QByteArray> {
        let mut roles = QHash::default();
        for (i, name) in ROLES.iter().enumerate() {
            roles.insert(FIRST_ROLE + int(i), QByteArray::from(*name));
        }
        roles
    }

    pub fn row_count(&self, parent: &QModelIndex) -> i32 {
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

    #[test]
    fn every_role_has_a_value() {
        let row = Row {
            key: "org.kde.dolphin".into(),
            name: "Dolphin".into(),
            icon: "system-file-manager".into(),
            cpu: f64::NAN,
            status: Status::Normal,
            never: false,
        };
        assert_eq!(crate::rows::changed(&row, &row.clone()), 0);
    }
}
