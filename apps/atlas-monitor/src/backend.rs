//! The app-wide QObject: settings and Atlas Monitor's own state. Page data
//! (samples, the process table) gets its own objects and models, see
//! docs/DESIGN.md. Slow work runs on a worker thread and posts back through
//! `qt_thread()`, so the GUI thread never blocks.

#[cxx_qt::bridge]
pub mod qobject {
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

use crate::settings::{self, Settings};

pub struct BackendRust {
    refresh_interval: i32,
    gpu_rendering: bool,
    own_pss: i64,
    own_rss: i64,
    /// A memory read is running; a second request is dropped.
    reading_memory: Arc<AtomicBool>,
}

impl Default for BackendRust {
    fn default() -> Self {
        let s = Settings::load();
        Self {
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
}

/// Holds a "work is running" flag; dropping it clears the flag.
struct Busy(Arc<AtomicBool>);

impl Drop for Busy {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Release);
    }
}
