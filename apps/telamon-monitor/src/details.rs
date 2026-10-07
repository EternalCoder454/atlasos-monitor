//! The Details dialog: what a process is, before somebody decides to end
//! it, or what an application is and the processes it is made of.
//!
//! The Apps page hands over what it already knows ([`Subject`]); a process's
//! own figures are read on a thread of their own, since several cost a file
//! each, and posted back. Read once, as the dialog opens.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_f64 = cxx_qt_lib::QList<f64>;
        type QList_i32 = cxx_qt_lib::QList<i32>;
    }

    extern "RustQt" {
        #[qobject]
        /// The dialog's title: the process's or the application's name.
        #[qproperty(QString, title)]
        /// An application of several processes, rather than one process.
        #[qproperty(bool, group)]
        /// A process opened from its application's list: Back returns there.
        #[qproperty(bool, nested, cxx_name = "canGoBack")]
        /// The process's figures are being read.
        #[qproperty(bool, loading)]
        /// The process had exited by the time it was read.
        #[qproperty(bool, gone)]
        //
        // One process. A figure that couldn't be read is "", NaN or -1.
        #[qproperty(i32, pid)]
        #[qproperty(QString, name)]
        /// The scheduler's word: "running", "sleeping", "stopped", ...
        #[qproperty(QString, state)]
        #[qproperty(i32, parent)]
        #[qproperty(QString, parent_name, cxx_name = "parentName")]
        #[qproperty(QString, user)]
        /// When it started, in ms since the epoch.
        #[qproperty(f64, started)]
        #[qproperty(i32, threads)]
        #[qproperty(i32, nice)]
        #[qproperty(QString, executable)]
        /// The arguments, quoted where a shell would need it.
        #[qproperty(QString, command_line, cxx_name = "commandLine")]
        /// The application it belongs to, by name.
        #[qproperty(QString, application)]
        #[qproperty(QString, unit)]
        /// Processor time, in seconds.
        #[qproperty(f64, cpu_time, cxx_name = "cpuTime")]
        /// Bytes: resident, shared pages divided among their users, its
        /// alone, and swapped out.
        #[qproperty(f64, memory)]
        #[qproperty(f64, pss)]
        #[qproperty(f64, private_memory, cxx_name = "privateMemory")]
        #[qproperty(f64, swap)]
        #[qproperty(i32, open_files, cxx_name = "openFiles")]
        //
        // An application.
        #[qproperty(QString, app_id, cxx_name = "appId")]
        #[qproperty(i32, processes)]
        /// Percent of one core, and resident bytes, summed.
        #[qproperty(f64, cpu)]
        #[qproperty(f64, group_memory, cxx_name = "groupMemory")]
        /// Its systemd units, one per line.
        #[qproperty(QString, units)]
        /// The busiest [`MOST`] members, busiest first.
        #[qproperty(QStringList, member_names, cxx_name = "memberNames")]
        #[qproperty(QList_i32, member_pids, cxx_name = "memberPids")]
        #[qproperty(QList_f64, member_cpu, cxx_name = "memberCpu")]
        #[qproperty(QList_f64, member_memory, cxx_name = "memberMemory")]
        #[namespace = "telamon_monitor"]
        type ProcessDetails = super::ProcessDetailsRust;

        /// Opens member `at` of the application shown.
        #[qinvokable]
        #[cxx_name = "openMember"]
        fn open_member(self: Pin<&mut ProcessDetails>, at: i32);

        /// From a member back to its application.
        #[qinvokable]
        fn back(self: Pin<&mut ProcessDetails>);

        /// Something new to show: the dialog opens on it.
        #[qsignal]
        fn shown(self: Pin<&mut ProcessDetails>);
    }

    impl cxx_qt::Threading for ProcessDetails {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn process_details_make_unique() -> UniquePtr<ProcessDetails>;
    }
}

use std::collections::BTreeSet;
use std::pin::Pin;
use std::time::UNIX_EPOCH;

use atlas_sysinfo::process::{self, Info};
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QList, QString, QStringList};

use crate::rows::int;

/// The most members listed for an application.
pub const MOST: usize = 100;

/// One of an application's processes, as the Apps page last sampled it.
#[derive(Debug, Clone, PartialEq)]
pub struct Member {
    pub pid: u32,
    pub start_time: u64,
    pub name: String,
    pub cpu: f64,
    pub memory: u64,
    pub unit: Option<String>,
}

/// What the Apps page asks to be shown.
#[derive(Debug, Clone, PartialEq)]
pub enum Subject {
    /// One process, by pid and start time, with its name and its
    /// application's.
    Process {
        pid: u32,
        start_time: u64,
        name: String,
        application: Option<String>,
    },
    Group {
        name: String,
        app_id: Option<String>,
        members: Vec<Member>,
    },
}

/// The application last shown, kept for Back.
#[derive(Debug, Clone, Default)]
struct Shown {
    name: String,
    app_id: String,
    members: Vec<Member>,
}

pub struct ProcessDetailsRust {
    title: QString,
    group: bool,
    nested: bool,
    loading: bool,
    gone: bool,
    pid: i32,
    name: QString,
    state: QString,
    parent: i32,
    parent_name: QString,
    user: QString,
    started: f64,
    threads: i32,
    nice: i32,
    executable: QString,
    command_line: QString,
    application: QString,
    unit: QString,
    cpu_time: f64,
    memory: f64,
    pss: f64,
    private_memory: f64,
    swap: f64,
    open_files: i32,
    app_id: QString,
    processes: i32,
    cpu: f64,
    group_memory: f64,
    units: QString,
    member_names: QStringList,
    member_pids: QList<i32>,
    member_cpu: QList<f64>,
    member_memory: QList<f64>,
    shown: Shown,
    /// Bumped by every read: an answer for a process no longer asked for
    /// is dropped.
    reading: u64,
}

impl Default for ProcessDetailsRust {
    fn default() -> Self {
        Self {
            title: QString::default(),
            group: false,
            nested: false,
            loading: false,
            gone: false,
            pid: -1,
            name: QString::default(),
            state: QString::default(),
            parent: -1,
            parent_name: QString::default(),
            user: QString::default(),
            started: f64::NAN,
            threads: -1,
            nice: 0,
            executable: QString::default(),
            command_line: QString::default(),
            application: QString::default(),
            unit: QString::default(),
            cpu_time: f64::NAN,
            memory: f64::NAN,
            pss: f64::NAN,
            private_memory: f64::NAN,
            swap: f64::NAN,
            open_files: -1,
            app_id: QString::default(),
            processes: 0,
            cpu: 0.0,
            group_memory: 0.0,
            units: QString::default(),
            member_names: QStringList::default(),
            member_pids: QList::default(),
            member_cpu: QList::default(),
            member_memory: QList::default(),
            shown: Shown::default(),
            reading: 0,
        }
    }
}

/// Joins a command line back into something that could be pasted into a
/// shell: an argument with a space, a quote or a shell character in it is
/// single-quoted. A lone argument is shown as it is: a program that writes
/// its whole command line over its arguments (Electron, Chromium) leaves
/// one, spaces and all, and quoting it would hide that it was several.
pub fn quote_args(args: &[String]) -> String {
    if let [one] = args {
        return one.clone();
    }
    let mut out = String::new();
    for (i, a) in args.iter().enumerate() {
        if i > 0 {
            out.push(' ');
        }
        if a.is_empty() || a.contains([' ', '\t', '\n', '\'', '"', '\\', '$', '`']) {
            out.push('\'');
            out.push_str(&a.replace('\'', r"'\''"));
            out.push('\'');
        } else {
            out.push_str(a);
        }
    }
    out
}

fn bytes(v: Option<u64>) -> f64 {
    v.map_or(f64::NAN, |b| b as f64)
}

fn qstr(s: Option<&str>) -> QString {
    QString::from(s.unwrap_or_default())
}

impl qobject::ProcessDetails {
    /// From the Apps page: shows `subject` and says so.
    pub fn show(mut self: Pin<&mut Self>, subject: Subject) {
        match subject {
            Subject::Process {
                pid,
                start_time,
                name,
                application,
            } => {
                self.as_mut().set_nested(false);
                self.as_mut().read(pid, start_time, &name, application);
            }
            Subject::Group {
                name,
                app_id,
                members,
            } => {
                self.as_mut().rust_mut().shown = Shown {
                    name,
                    app_id: app_id.unwrap_or_default(),
                    members,
                };
                self.as_mut().show_group();
            }
        }
        self.shown();
    }

    fn show_group(mut self: Pin<&mut Self>) {
        // A read still on its way is for a process no longer shown.
        self.as_mut().rust_mut().reading += 1;
        let shown = self.rust().shown.clone();
        let mut members = shown.members;
        members.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(a.pid.cmp(&b.pid)));
        let units: BTreeSet<&str> = members.iter().filter_map(|m| m.unit.as_deref()).collect();
        self.as_mut().set_units(QString::from(
            &units.into_iter().collect::<Vec<_>>().join("\n"),
        ));
        self.as_mut().set_cpu(members.iter().map(|m| m.cpu).sum());
        self.as_mut()
            .set_group_memory(members.iter().map(|m| m.memory as f64).sum());
        self.as_mut().set_processes(int(members.len()));
        let listed = &members[..members.len().min(MOST)];
        let mut names = QStringList::default();
        let mut pids = QList::default();
        let mut cpu = QList::default();
        let mut memory = QList::default();
        for m in listed {
            names.append(QString::from(&m.name));
            pids.append(i32::try_from(m.pid).unwrap_or(-1));
            cpu.append(m.cpu);
            memory.append(m.memory as f64);
        }
        // Names last: the list's rows are made per name, and read the
        // others by index.
        self.as_mut().set_member_pids(pids);
        self.as_mut().set_member_cpu(cpu);
        self.as_mut().set_member_memory(memory);
        self.as_mut().set_member_names(names);
        self.as_mut().set_app_id(QString::from(&shown.app_id));
        self.as_mut().set_title(QString::from(&shown.name));
        self.as_mut().set_nested(false);
        self.as_mut().set_loading(false);
        self.as_mut().set_gone(false);
        self.as_mut().set_group(true);
    }

    pub fn open_member(mut self: Pin<&mut Self>, at: i32) {
        if !*self.group() {
            return;
        }
        let mut members = self.rust().shown.members.clone();
        members.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(a.pid.cmp(&b.pid)));
        let Some(m) = usize::try_from(at).ok().and_then(|i| members.get(i)) else {
            return;
        };
        let (pid, start, name) = (m.pid, m.start_time, m.name.clone());
        let application = Some(self.rust().shown.name.clone());
        self.as_mut().set_nested(true);
        self.read(pid, start, &name, application);
    }

    pub fn back(self: Pin<&mut Self>) {
        if *self.nested() {
            self.show_group();
        }
    }

    /// Reads process `pid` on a thread, and fills the dialog from it if it
    /// is still the process that started at `start_time`.
    fn read(
        mut self: Pin<&mut Self>,
        pid: u32,
        start_time: u64,
        name: &str,
        application: Option<String>,
    ) {
        let generation = {
            let mut r = self.as_mut().rust_mut();
            r.reading += 1;
            r.reading
        };
        self.as_mut().set_group(false);
        self.as_mut().set_gone(false);
        self.as_mut().set_loading(true);
        self.as_mut().set_title(QString::from(name));
        self.as_mut().set_name(QString::from(name));
        self.as_mut().set_pid(i32::try_from(pid).unwrap_or(-1));
        self.as_mut().set_application(qstr(application.as_deref()));
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new()
            .name("details".into())
            .spawn(move || {
                let info = process::details(pid).filter(|i| i.start_time == start_time);
                let _ = qt.queue(move |o| o.apply(generation, info));
            });
        if let Err(e) = spawned {
            log::error!("reading process {pid}: {e}");
            self.apply(generation, None);
        }
    }

    fn apply(mut self: Pin<&mut Self>, generation: u64, info: Option<Info>) {
        if generation != self.rust().reading {
            return;
        }
        let Some(info) = info else {
            self.as_mut().set_gone(true);
            self.as_mut().set_loading(false);
            return;
        };
        self.as_mut().set_title(QString::from(&info.name));
        self.as_mut().set_name(QString::from(&info.name));
        self.as_mut().set_state(QString::from(&info.state));
        self.as_mut()
            .set_parent(i32::try_from(info.parent).unwrap_or(-1));
        self.as_mut()
            .set_parent_name(qstr(info.parent_name.as_deref()));
        self.as_mut().set_user(qstr(info.user.as_deref()));
        self.as_mut().set_started(
            info.started
                .and_then(|t| t.duration_since(UNIX_EPOCH).ok())
                .map_or(f64::NAN, |d| d.as_secs_f64() * 1000.0),
        );
        self.as_mut().set_threads(
            info.threads
                .and_then(|t| i32::try_from(t).ok())
                .unwrap_or(-1),
        );
        self.as_mut().set_nice(info.nice);
        self.as_mut().set_executable(QString::from(
            &info
                .executable
                .as_ref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_default(),
        ));
        self.as_mut()
            .set_command_line(QString::from(&quote_args(&info.command_line)));
        self.as_mut().set_unit(qstr(info.unit.as_deref()));
        self.as_mut().set_cpu_time(info.cpu_time.as_secs_f64());
        self.as_mut().set_memory(bytes(info.memory));
        self.as_mut().set_pss(bytes(info.pss));
        self.as_mut().set_private_memory(bytes(info.private));
        self.as_mut().set_swap(bytes(info.swap));
        self.as_mut().set_open_files(
            info.open_files
                .and_then(|n| i32::try_from(n).ok())
                .unwrap_or(-1),
        );
        self.as_mut().set_loading(false);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn args(a: &[&str]) -> Vec<String> {
        a.iter().map(|s| (*s).to_owned()).collect()
    }

    #[test]
    fn command_lines_paste_back_into_a_shell() {
        assert_eq!(
            quote_args(&args(&["/usr/bin/bash", "-l"])),
            "/usr/bin/bash -l"
        );
        assert_eq!(
            quote_args(&args(&["python3", "my script.py", ""])),
            "python3 'my script.py' ''"
        );
        assert_eq!(
            quote_args(&args(&["echo", "it's $HOME"])),
            r"echo 'it'\''s $HOME'"
        );
        assert_eq!(quote_args(&[]), "");
        assert_eq!(
            quote_args(&args(&["/opt/app --type=gpu"])),
            "/opt/app --type=gpu"
        );
    }
}
