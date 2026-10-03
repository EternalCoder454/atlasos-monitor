//! The app-wide QObject: settings and Atlas Monitor's own state. Page data
//! (samples, the process table) gets its own objects and models, see
//! docs/DESIGN.md. Slow work runs on a worker thread and posts back through
//! `qt_thread()`, so the GUI thread never blocks.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        /// How often the pages on screen are sampled, in milliseconds.
        #[qproperty(i32, refresh_interval, cxx_name = "refreshInterval")]
        /// The saved "draw with the graphics card" setting. It takes effect
        /// the next time Atlas Monitor starts (`main.cpp` reads it).
        #[qproperty(bool, gpu_rendering, cxx_name = "gpuRendering")]
        /// Atlas Monitor's own memory, in bytes (0 until first read).
        #[qproperty(i64, own_pss, cxx_name = "ownPss")]
        #[qproperty(i64, own_rss, cxx_name = "ownRss")]
        /// The window as it was left, read once at start: its size (0 for
        /// the default), maximized or not, and its page ("" for Overview).
        #[qproperty(i32, window_width, cxx_name = "windowWidth")]
        #[qproperty(i32, window_height, cxx_name = "windowHeight")]
        #[qproperty(bool, window_maximized, cxx_name = "windowMaximized")]
        #[qproperty(QString, last_page, cxx_name = "lastPage")]
        #[namespace = "atlas_monitor"]
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

        /// Starts Atlas Updater, which keeps the crash report setting (a
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
use cxx_qt_lib::QString;

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
    /// A memory read is running; a second request is dropped.
    reading_memory: Arc<AtomicBool>,
}

impl Default for BackendRust {
    fn default() -> Self {
        let s = Settings::load();
        let w = WindowState::load();
        Self {
            window_width: w.width,
            window_height: w.height,
            window_maximized: w.maximized,
            last_page: QString::from(&w.page),
            refresh_interval: s.refresh_interval_ms,
            gpu_rendering: s.gpu_rendering,
            own_pss: 0,
            own_rss: 0,
            reading_memory: Arc::new(AtomicBool::new(false)),
        }
    }
}

fn to_i64(bytes: u64) -> i64 {
    i64::try_from(bytes).unwrap_or(i64::MAX)
}

/// Atlas Updater's program, on the `PATH` wherever AtlasOS installs it.
const UPDATER: &str = "atlas-updater";

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
        if let Err(e) = WindowState::save_size(width, height, maximized) {
            log::warn!("saving the window's size: {e}");
        }
    }

    pub fn save_page(&self, page: &QString) {
        if let Err(e) = WindowState::save_page(&page.to_string()) {
            log::warn!("saving the page on screen: {e}");
        }
    }

    pub fn refresh_own_memory(self: Pin<&mut Self>) {
        let flag = &self.rust().reading_memory;
        if flag.swap(true, Ordering::AcqRel) {
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
                });
            });
        if let Err(e) = spawned {
            log::warn!("starting the memory read: {e}");
        }
    }

    pub fn open_updater(&self) -> bool {
        let child = std::process::Command::new(UPDATER)
            .stdin(std::process::Stdio::null())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .spawn();
        match child {
            Ok(mut child) => {
                // Reaped when it exits; a running Updater answers the new
                // one at once, which then exits.
                let reaped = std::thread::Builder::new()
                    .name("updater-wait".into())
                    .spawn(move || {
                        let _ = child.wait();
                    });
                if let Err(e) = reaped {
                    log::warn!("waiting for Atlas Updater: {e}");
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
