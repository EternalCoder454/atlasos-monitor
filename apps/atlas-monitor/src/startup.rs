//! The Startup page: what starts at login, each with its switch. Reading
//! the list and switching an item both ask the user's systemd manager over
//! D-Bus, so they run on a thread of their own and post back; the GUI
//! thread never waits on the manager.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_bool = cxx_qt_lib::QList<bool>;
    }

    extern "RustQt" {
        #[qobject]
        /// Reading the list, the first time or again.
        #[qproperty(bool, loading)]
        /// The list has been read at least once.
        #[qproperty(bool, loaded)]
        /// The user's systemd manager answered: without it there are no
        /// units, and entries have no state.
        #[qproperty(bool, units)]
        /// One entry per item, sorted by name: its id (the page's key), name,
        /// what it is or runs, and icon (a name or a path; "" for none).
        #[qproperty(QStringList, ids)]
        #[qproperty(QStringList, names)]
        #[qproperty(QStringList, subtitles)]
        #[qproperty(QStringList, icons)]
        /// Switched on, and whether its switch can move from there.
        #[qproperty(QList_bool, enabled)]
        #[qproperty(QList_bool, switchable)]
        /// Desktop plumbing, behind "Show System Entries".
        #[qproperty(QList_bool, plumbing)]
        /// Why a switched-on item doesn't start: "", "notThisDesktop",
        /// "missingProgram", "noCommand", "turnedOff" or "byUnit".
        #[qproperty(QStringList, notes)]
        /// Why its switch is held: "", "required" (part of AtlasOS),
        /// "session" (the session needs it) or "byUnit".
        #[qproperty(QStringList, locks)]
        /// Its unit's state: "", "running", "active", "starting",
        /// "stopping", "stopped" or "failed".
        #[qproperty(QStringList, states)]
        /// Why the last switch failed, by key ("locked", "invalid",
        /// "notFound", "notEnableable", "io", "noAnswer", "refused"), and
        /// the item's name; "" when it didn't.
        #[qproperty(QString, error)]
        #[qproperty(QString, error_name, cxx_name = "errorName")]
        #[namespace = "atlas_monitor"]
        type StartupList = super::StartupListRust;

        /// Reads the list again.
        #[qinvokable]
        fn refresh(self: Pin<&mut StartupList>);

        /// Switches the item `id` on or off.
        #[qinvokable]
        #[cxx_name = "setEnabled"]
        fn set_item_enabled(self: Pin<&mut StartupList>, id: &QString, on: bool);
    }

    impl cxx_qt::Threading for StartupList {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn startup_list_make_unique() -> UniquePtr<StartupList>;
    }
}

use std::pin::Pin;

use atlas_sysinfo::autostart::{self, Error, Item, List, Lock, Runs};
use atlas_sysinfo::services::Status;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString, QStringList};

#[derive(Default)]
pub struct StartupListRust {
    loading: bool,
    loaded: bool,
    units: bool,
    ids: QStringList,
    names: QStringList,
    subtitles: QStringList,
    icons: QStringList,
    enabled: QList<bool>,
    switchable: QList<bool>,
    plumbing: QList<bool>,
    notes: QStringList,
    locks: QStringList,
    states: QStringList,
    error: QString,
    error_name: QString,
    items: Vec<Item>,
    /// Bumped by every read asked for: an answer from an older one is
    /// dropped, so a slow read can't undo a newer switch.
    reading: u64,
}

fn strings<'a>(items: impl Iterator<Item = &'a str>) -> QStringList {
    let mut list = QStringList::default();
    for s in items {
        list.append(QString::from(s));
    }
    list
}

fn flags(items: impl Iterator<Item = bool>) -> QList<bool> {
    let mut list = QList::default();
    for v in items {
        list.append(v);
    }
    list
}

fn note(runs: Runs, lock: Option<Lock>) -> &'static str {
    match (runs, lock) {
        (_, Some(Lock::ByUnit)) | (Runs::ByUnit, _) => "byUnit",
        (Runs::Yes, _) => "",
        (Runs::NotThisDesktop, _) => "notThisDesktop",
        (Runs::MissingProgram, _) => "missingProgram",
        (Runs::NoCommand, _) => "noCommand",
        (Runs::TurnedOff, _) => "turnedOff",
    }
}

fn lock(l: Option<Lock>) -> &'static str {
    match l {
        None => "",
        Some(Lock::Required) => "required",
        Some(Lock::Session) => "session",
        Some(Lock::ByUnit) => "byUnit",
    }
}

fn state(s: Option<Status>) -> &'static str {
    match s {
        None => "",
        Some(Status::Running) => "running",
        Some(Status::Active) => "active",
        Some(Status::Starting) => "starting",
        Some(Status::Stopping) => "stopping",
        Some(Status::Stopped) => "stopped",
        Some(Status::Failed) => "failed",
    }
}

fn error_key(e: &Error) -> &'static str {
    match e {
        Error::Locked => "locked",
        Error::InvalidName => "invalid",
        Error::NotFound => "notFound",
        Error::NotEnableable => "notEnableable",
        Error::Io(_) => "io",
        Error::NoAnswer => "noAnswer",
        Error::Refused(_) => "refused",
    }
}

impl qobject::StartupList {
    /// Ignored while a read or a switch is under way: its answer is on its
    /// way, and a newer read would drop it.
    pub fn refresh(mut self: Pin<&mut Self>) {
        if *self.loading() {
            return;
        }
        self.as_mut().set_error(QString::default());
        self.read(None);
    }

    /// Shows item `at` switched `on`, before or without the manager's say.
    fn show_enabled(mut self: Pin<&mut Self>, at: usize, on: bool) {
        let mut enabled = self.enabled().clone();
        if let Ok(i) = isize::try_from(at) {
            enabled.remove(i);
            enabled.insert(i, on);
        }
        self.as_mut().set_enabled(enabled);
        if let Some(item) = self.as_mut().rust_mut().items.get_mut(at) {
            item.enabled = on;
        }
    }

    /// Reads the list on a thread, after switching `switch` if given, and
    /// posts the result back.
    fn read(mut self: Pin<&mut Self>, switch: Option<(Item, bool)>) {
        let generation = {
            let mut r = self.as_mut().rust_mut();
            r.reading += 1;
            r.reading
        };
        self.as_mut().set_loading(true);
        let undo = switch.as_ref().and_then(|(item, on)| {
            Some((self.rust().items.iter().position(|i| i.id == item.id)?, !on))
        });
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new()
            .name("startup".into())
            .spawn(move || {
                let failed = switch.and_then(|(item, on)| {
                    autostart::set_enabled(&item, on)
                        .err()
                        .map(|e| (item.name, e))
                });
                let list = autostart::list();
                let _ = qt.queue(move |o| o.apply(generation, list, failed));
            });
        if let Err(e) = spawned {
            log::error!("reading the startup list: {e}");
            // Nothing was switched: the switch goes back.
            if let Some((at, was)) = undo {
                self.as_mut().show_enabled(at, was);
                self.as_mut().set_error(QString::from("io"));
            }
            self.as_mut().set_loading(false);
        }
    }

    fn apply(
        mut self: Pin<&mut Self>,
        generation: u64,
        list: List,
        failed: Option<(String, Error)>,
    ) {
        if let Some((name, e)) = &failed {
            log::warn!("switching {name}: {e:?}");
            self.as_mut().set_error_name(QString::from(name));
            self.as_mut().set_error(QString::from(error_key(e)));
        }
        if generation != self.rust().reading {
            return;
        }
        let items = &list.items;
        self.as_mut().set_units(list.units);
        self.as_mut().set_subtitles(strings(items.iter().map(|i| {
            if i.comment.is_empty() {
                i.command.as_str()
            } else {
                i.comment.as_str()
            }
        })));
        self.as_mut()
            .set_icons(strings(items.iter().map(|i| i.icon.as_str())));
        self.as_mut()
            .set_enabled(flags(items.iter().map(|i| i.enabled)));
        self.as_mut()
            .set_switchable(flags(items.iter().map(Item::can_switch)));
        self.as_mut()
            .set_plumbing(flags(items.iter().map(|i| i.plumbing)));
        self.as_mut()
            .set_notes(strings(items.iter().map(|i| note(i.runs, i.lock))));
        self.as_mut()
            .set_locks(strings(items.iter().map(|i| lock(i.lock))));
        self.as_mut()
            .set_states(strings(items.iter().map(|i| state(i.status))));
        self.as_mut()
            .set_names(strings(items.iter().map(|i| i.name.as_str())));
        // Last: the page's rows are made per id.
        let ids = strings(items.iter().map(|i| i.id.as_str()));
        if ids != *self.ids() {
            self.as_mut().set_ids(ids);
        }
        self.as_mut().rust_mut().items = list.items;
        self.as_mut().set_loading(false);
        self.as_mut().set_loaded(true);
    }

    /// Ignored while a read or another switch is under way (the page
    /// disables the switches then): two switches at once could reach the
    /// manager out of order.
    pub fn set_item_enabled(mut self: Pin<&mut Self>, id: &QString, on: bool) {
        if *self.loading() {
            return;
        }
        let id = id.to_string();
        let Some(at) = self.rust().items.iter().position(|i| i.id == id) else {
            return;
        };
        let item = self.rust().items[at].clone();
        if item.enabled == on {
            return;
        }
        // The switch moves now; the read after the change puts it back if
        // it didn't take.
        self.as_mut().show_enabled(at, on);
        self.as_mut().set_error(QString::default());
        self.as_mut().set_error_name(QString::from(&item.name));
        self.read(Some((item, on)));
    }
}
