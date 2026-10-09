//! The app-wide QObject: settings and Telamon Monitor's own state. Page data
//! (samples, the process table) gets its own objects and models, see
//! docs/DESIGN.md. Slow work runs on a worker thread and posts back through
//! `qt_thread()`, so the GUI thread never blocks.

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
        /// How often the pages on screen are sampled, in milliseconds.
        #[qproperty(i32, refresh_interval, cxx_name = "refreshInterval")]
        /// The saved "draw with the graphics card" setting. It takes effect
        /// the next time Telamon Monitor starts (`main.cpp` reads it).
        #[qproperty(bool, gpu_rendering, cxx_name = "gpuRendering")]
        /// Telamon Monitor's own memory, in bytes (0 until first read).
        #[qproperty(i64, own_pss, cxx_name = "ownPss")]
        #[qproperty(i64, own_rss, cxx_name = "ownRss")]
        /// The window as it was left, read once at start: its size (0 for
        /// the default), maximized or not, and its page ("" for Overview).
        #[qproperty(i32, window_width, cxx_name = "windowWidth")]
        #[qproperty(i32, window_height, cxx_name = "windowHeight")]
        #[qproperty(bool, window_maximized, cxx_name = "windowMaximized")]
        #[qproperty(QString, last_page, cxx_name = "lastPage")]
        /// The ids of the sections folded shut (`cpu.cores`), kept between
        /// runs.
        #[qproperty(QStringList, folded_sections, cxx_name = "foldedSections")]
        #[namespace = "telamon_monitor"]
        type Backend = super::BackendRust;

        /// Saves the refresh interval (snapped to one Settings offers).
        #[qinvokable]
        #[cxx_name = "changeRefreshInterval"]
        fn change_refresh_interval(self: Pin<&mut Backend>, ms: i32);

        /// Saves the rendering setting; applies on next start.
        #[qinvokable]
        #[cxx_name = "changeGpuRendering"]
        fn change_gpu_rendering(self: Pin<&mut Backend>, on: bool);

        /// Reads `ownPss`/`ownRss` on a worker thread.
        #[qinvokable]
        #[cxx_name = "refreshOwnMemory"]
        fn refresh_own_memory(self: Pin<&mut Backend>);

        /// Hands memory freed and kept for reuse back to the system, then
        /// reads `ownPss` again.
        #[qinvokable]
        #[cxx_name = "releaseIdleMemory"]
        fn release_idle_memory(self: Pin<&mut Backend>);

        /// Hands memory freed and kept for reuse back to the system, as a
        /// page closes.
        #[qinvokable]
        #[cxx_name = "trimMemory"]
        fn trim_memory(self: &Backend);

        /// Starts Telamon Updater, which keeps the crash report setting (a
        /// running one comes to the front). False when it isn't installed.
        #[qinvokable]
        #[cxx_name = "openUpdater"]
        fn open_updater(self: &Backend) -> bool;

        /// Saves the window's size, when it closes.
        #[qinvokable]
        #[cxx_name = "saveWindowSize"]
        fn save_window_size(self: &Backend, width: i32, height: i32, maximized: bool);

        /// Saves the page on screen, to open on next time.
        #[qinvokable]
        #[cxx_name = "savePage"]
        fn save_page(self: &Backend, page: &QString);

        /// Folds the section `id` shut, or opens it, and saves that.
        #[qinvokable]
        #[cxx_name = "setFolded"]
        fn set_folded(self: Pin<&mut Backend>, id: &QString, fold: bool);
    }

    impl cxx_qt::Threading for Backend {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn backend_make_unique() -> UniquePtr<Backend>;
    }
}

use core::pin::Pin;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use atlas_sysinfo::sysmem;
use cxx_qt::{CxxQtType, Threading};
use cxx_qt_lib::{QString, QStringList};

use crate::processes::string_list;
use crate::settings::{self, Settings, WindowState};

pub struct BackendRust {
    refresh_interval: i32,
    gpu_rendering: bool,
    own_pss: i64,
    own_rss: i64,
    window_width: i32,
    window_height: i32,
    window_maximized: bool,
    last_page: QString,
    folded_sections: QStringList,
    folded: Vec<String>,
    /// A memory read is running; a second request waits for it.
    reading_memory: Arc<AtomicBool>,
    /// Something changed while a read was under way: read again after it.
    read_again: bool,
}

impl Default for BackendRust {
    fn default() -> Self {
        let s = Settings::load();
        let w = WindowState::load();
        let folded = WindowState::load_folded();
        Self {
            folded_sections: string_list(&folded),
            folded,
            window_width: w.width,
            window_height: w.height,
            window_maximized: w.maximized,
            last_page: QString::from(&w.page),
            refresh_interval: s.refresh_interval_ms,
            gpu_rendering: s.gpu_rendering,
            own_pss: 0,
            own_rss: 0,
            reading_memory: Arc::new(AtomicBool::new(false)),
            read_again: false,
        }
    }
}

fn to_i64(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

/// Telamon Updater's program, where Telamon OS installs it, and the name it
/// had before it was renamed, which an image that has not moved the Updater
/// yet still has (this release only). Absolute: not looked up in the `PATH`,
/// where a program in `~/.local/bin` could stand in for it.
const UPDATER: &str = "/usr/bin/telamon-updater";
const LEGACY_UPDATER: &str = "/usr/bin/atlas-updater";

fn spawn_updater() -> std::io::Result<std::process::Child> {
    let start = |program: &str| {
        std::process::Command::new(program)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn()
    };
    match start(UPDATER) {
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => start(LEGACY_UPDATER),
        other => other,
    }
}

impl qobject::Backend {
    pub fn change_refresh_interval(self: Pin<&mut Self>, ms: i32) {
        let ms = settings::nearest_interval(ms);
        if ms == *self.refresh_interval() {
            return;
        }
        // Only show what was saved: on failure the radio row's binding puts
        // the old choice back.
        match Settings::save_refresh_interval(ms) {
            Ok(()) => self.set_refresh_interval(ms),
            Err(e) => log::warn!("saving the refresh interval: {e}"),
        }
    }

    pub fn change_gpu_rendering(self: Pin<&mut Self>, on: bool) {
        if on == *self.gpu_rendering() {
            return;
        }
        match Settings::save_gpu_rendering(on) {
            Ok(()) => self.set_gpu_rendering(on),
            Err(e) => log::warn!("saving the rendering setting: {e}"),
        }
    }

    pub fn save_window_size(&self, width: i32, height: i32, maximized: bool) {
        WindowState::save_size(width, height, maximized);
    }

    pub fn save_page(&self, page: &QString) {
        WindowState::save_page(&page.to_string());
    }

    pub fn set_folded(mut self: Pin<&mut Self>, id: &QString, fold: bool) {
        let id = id.to_string();
        // An id goes in a comma-separated list.
        if id.is_empty() || id.contains(',') {
            return;
        }
        let folded = settings::with_folded(&self.rust().folded, &id, fold);
        if folded == self.rust().folded {
            return;
        }
        // Saved in the background; a save that fails still folds it for
        // this run.
        WindowState::save_folded(&folded);
        self.as_mut().set_folded_sections(string_list(&folded));
        self.as_mut().rust_mut().folded = folded;
    }

    pub fn release_idle_memory(self: Pin<&mut Self>) {
        let qt = self.qt_thread();
        trim(move || {
            let _ = qt.queue(|o| o.refresh_own_memory());
        });
    }

    pub fn trim_memory(&self) {
        trim(|| {});
    }

    pub fn refresh_own_memory(mut self: Pin<&mut Self>) {
        let flag = &self.rust().reading_memory;
        if flag.swap(true, Ordering::AcqRel) {
            self.as_mut().rust_mut().read_again = true;
            return;
        }
        // Clears the flag when the last holder drops it: after the result is
        // applied, or wherever the read ends early (queue refused, panic).
        let busy = Busy(Arc::clone(flag));
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new()
            .name("own-memory".into())
            .spawn(move || {
                let read = sysmem::read();
                if let Err(e) = &read {
                    log::warn!("reading own memory: {e}");
                }
                let _ = qt.queue(move |mut obj| {
                    if let Ok(m) = read {
                        obj.as_mut().set_own_pss(to_i64(m.pss));
                        obj.as_mut().set_own_rss(to_i64(m.rss));
                    }
                    drop(busy);
                    if std::mem::take(&mut obj.as_mut().rust_mut().read_again) {
                        obj.refresh_own_memory();
                    }
                });
            });
        if let Err(e) = spawned {
            log::warn!("starting the memory read: {e}");
        }
    }

    pub fn open_updater(&self) -> bool {
        match spawn_updater() {
            Ok(mut child) => {
                // Reaped when it exits; a running Updater answers the new
                // one at once, which then exits.
                let reaped = std::thread::Builder::new()
                    .name("updater-wait".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                if let Err(e) = reaped {
                    log::warn!("waiting for Telamon Updater: {e}");
                }
                true
            }
            Err(e) => {
                log::warn!("starting {UPDATER}: {e}");
                false
            }
        }
    }
}

/// Holds a "work is running" flag; dropping it clears the flag.
struct Busy(Arc<AtomicBool>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}

/// Trims the heap, then calls `then`. A grown heap takes a while to walk,
/// and other threads' mallocs wait on it: not on the GUI thread.
fn trim(then: impl FnOnce() + Send + 'static) {
    let spawned = std::thread::Builder::new()
        .name("trim".into())
        .spawn(move || {
            sysmem::trim();
            then();
        });
    if let Err(e) = spawned {
        log::warn!("starting the memory trim: {e}");
    }
}
