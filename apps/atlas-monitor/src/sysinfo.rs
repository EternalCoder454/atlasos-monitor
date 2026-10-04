//! The System Info page: what this computer is (its system, desktop, kernel,
//! maker and firmware) and how secure its firmware is, by fwupd. Read when
//! the page first opens and on Refresh, on a thread of its own: fwupd may
//! have to be started first, and the GUI thread never waits on it.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
    }

    extern "RustQt" {
        #[qobject]
        /// Reading, the first time or again.
        #[qproperty(bool, loading)]
        /// The machine has been read at least once.
        #[qproperty(bool, loaded)]
        /// The operating system's name and its logo (an icon name).
        #[qproperty(QString, os_name, cxx_name = "osName")]
        #[qproperty(QString, os_logo, cxx_name = "osLogo")]
        /// KDE Plasma's version, "" without Plasma.
        #[qproperty(QString, plasma)]
        #[qproperty(QString, kernel)]
        #[qproperty(QString, arch)]
        #[qproperty(QString, hostname)]
        /// The processor's model and how many threads it runs (0: unknown).
        #[qproperty(QString, cpu)]
        #[qproperty(i32, cpu_threads, cxx_name = "cpuThreads")]
        /// Memory the system can use, in bytes.
        #[qproperty(f64, memory)]
        /// From the firmware's tables; "" where it only has a placeholder.
        #[qproperty(QString, vendor)]
        #[qproperty(QString, product)]
        #[qproperty(QString, board)]
        #[qproperty(QString, firmware)]
        #[qproperty(QString, chassis)]
        /// fwupd has answered (or been found missing) at least once.
        #[qproperty(bool, security_loaded, cxx_name = "securityLoaded")]
        /// fwupd is on the system and answered.
        #[qproperty(bool, security_available, cxx_name = "securityAvailable")]
        /// The Host Security ID level, 0 to 5; -1 when fwupd gave none.
        #[qproperty(i32, security_level, cxx_name = "securityLevel")]
        /// A problem found while running (the "!" after the level).
        #[qproperty(bool, security_runtime_issue, cxx_name = "securityRuntimeIssue")]
        /// fwupd's version, "" if it didn't say.
        #[qproperty(QString, fwupd_version, cxx_name = "fwupdVersion")]
        /// The checks that didn't pass, by fwupd's titles.
        #[qproperty(QStringList, security_failing, cxx_name = "securityFailing")]
        #[namespace = "atlas_monitor"]
        type SystemInfo = super::SystemInfoRust;

        /// Reads the machine and its firmware security again. Ignored while
        /// a read is under way.
        #[qinvokable]
        fn refresh(self: Pin<&mut SystemInfo>);
    }

    impl cxx_qt::Threading for SystemInfo {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn system_info_make_unique() -> UniquePtr<SystemInfo>;
    }
}

use std::pin::Pin;

use atlas_sysinfo::about::{self, About, Security};
use cxx_qt::Threading;
use cxx_qt_lib::{QString, QStringList};

pub struct SystemInfoRust {
    loading: bool,
    loaded: bool,
    os_name: QString,
    os_logo: QString,
    plasma: QString,
    kernel: QString,
    arch: QString,
    hostname: QString,
    cpu: QString,
    cpu_threads: i32,
    memory: f64,
    vendor: QString,
    product: QString,
    board: QString,
    firmware: QString,
    chassis: QString,
    security_loaded: bool,
    security_available: bool,
    security_level: i32,
    security_runtime_issue: bool,
    fwupd_version: QString,
    security_failing: QStringList,
}

impl Default for SystemInfoRust {
    fn default() -> Self {
        Self {
            loading: false,
            loaded: false,
            os_name: QString::default(),
            os_logo: QString::default(),
            plasma: QString::default(),
            kernel: QString::default(),
            arch: QString::default(),
            hostname: QString::default(),
            cpu: QString::default(),
            cpu_threads: 0,
            memory: 0.0,
            vendor: QString::default(),
            product: QString::default(),
            board: QString::default(),
            firmware: QString::default(),
            chassis: QString::default(),
            security_loaded: false,
            security_available: false,
            security_level: -1,
            security_runtime_issue: false,
            fwupd_version: QString::default(),
            security_failing: QStringList::default(),
        }
    }
}

fn strings(items: &[String]) -> QStringList {
    let mut list = QStringList::default();
    for s in items {
        list.append(QString::from(s));
    }
    list
}

impl qobject::SystemInfo {
    pub fn refresh(mut self: Pin<&mut Self>) {
        // One read at a time, so every answer is the latest one's.
        if *self.loading() {
            return;
        }
        self.as_mut().set_loading(true);
        let qt = self.qt_thread();
        // The machine first: it is read in milliseconds, while fwupd may
        // take seconds to start.
        let spawned = std::thread::Builder::new()
            .name("system-info".into())
            .spawn(move || {
                // A reader that panics leaves its part empty rather than
                // the page loading for good.
                let machine = std::panic::catch_unwind(about::read).unwrap_or_default();
                if qt.queue(move |o| o.apply(machine)).is_err() {
                    return;
                }
                let security = std::panic::catch_unwind(about::security).unwrap_or_default();
                let _ = qt.queue(move |o| o.apply_security(security));
            });
        if let Err(e) = spawned {
            log::error!("reading the system information: {e}");
            self.as_mut().set_loading(false);
            self.as_mut().set_loaded(true);
            self.as_mut().set_security_loaded(true);
        }
    }

    fn apply(mut self: Pin<&mut Self>, a: About) {
        self.as_mut().set_os_name(QString::from(&a.os_name));
        self.as_mut().set_os_logo(QString::from(&a.os_logo));
        self.as_mut().set_plasma(QString::from(&a.plasma));
        self.as_mut().set_kernel(QString::from(&a.kernel));
        self.as_mut().set_arch(QString::from(&a.arch));
        self.as_mut().set_hostname(QString::from(&a.hostname));
        self.as_mut().set_cpu(QString::from(&a.cpu));
        self.as_mut()
            .set_cpu_threads(i32::try_from(a.cpu_threads).unwrap_or(i32::MAX));
        // Exact to 2^53 bytes, 8 PiB.
        #[allow(clippy::cast_precision_loss)]
        self.as_mut().set_memory(a.memory as f64);
        self.as_mut().set_vendor(QString::from(&a.vendor));
        self.as_mut().set_product(QString::from(&a.product));
        self.as_mut().set_board(QString::from(&a.board));
        self.as_mut().set_firmware(QString::from(&a.firmware));
        self.as_mut().set_chassis(QString::from(&a.chassis));
        self.as_mut().set_loaded(true);
    }

    fn apply_security(mut self: Pin<&mut Self>, s: Security) {
        self.as_mut().set_security_available(s.available);
        self.as_mut()
            .set_security_level(s.level.map_or(-1, i32::from));
        self.as_mut().set_security_runtime_issue(s.runtime_issue);
        self.as_mut().set_fwupd_version(QString::from(&s.version));
        self.as_mut().set_security_failing(strings(&s.failing));
        self.as_mut().set_security_loaded(true);
        self.as_mut().set_loading(false);
    }
}
