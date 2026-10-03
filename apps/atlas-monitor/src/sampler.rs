//! The `Sampler` QObject: QML tells it which page is on screen and how often
//! to sample; it runs the loop in `sampling.rs` and posts each tick's parts
//! to the stats objects (`stats.rs`) on the Qt thread.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
    }

    extern "RustQt" {
        #[qobject]
        #[namespace = "atlas_monitor"]
        type Sampler = super::SamplerRust;

        /// The page now on screen ("overview", "cpu", "disk:nvme0n1", ...).
        /// Only its readers run.
        #[qinvokable]
        #[cxx_name = "showPage"]
        fn show_page(self: &Sampler, page: &QString);

        /// The refresh interval in milliseconds.
        #[qinvokable]
        #[cxx_name = "changeInterval"]
        fn change_interval(self: &Sampler, ms: i32);
    }

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn sampler_make_unique() -> UniquePtr<Sampler>;
    }
}

use core::pin::Pin;
use std::time::Duration;

use cxx_qt::{CxxQtThread, CxxQtType};
use cxx_qt_lib::QString;

use crate::sampling::{Loop, Page, Tick};
use crate::stats::qobject::{CpuStats, HealthStatus, MemoryStats};

#[derive(Default)]
pub struct SamplerRust {
    /// `None` until started, and if the thread couldn't be made.
    running: Option<Loop>,
}

impl qobject::Sampler {
    pub fn show_page(&self, page: &QString) {
        if let Some(l) = &self.running {
            l.set_page(Page::parse(&page.to_string()));
        }
    }

    pub fn change_interval(&self, ms: i32) {
        if let Some(l) = &self.running {
            l.set_interval(interval(ms));
        }
    }

    /// Starts the loop, posting to `sink`'s objects. Called once, from
    /// `lib.rs`, before QML can call in.
    pub fn start(self: Pin<&mut Self>, ms: i32, sink: Sink) {
        match Loop::start(interval(ms), move |tick| sink.post(tick)) {
            Ok(l) => self.rust_mut().get_mut().running = Some(l),
            Err(e) => log::error!("starting the sampling thread: {e}"),
        }
    }
}

fn interval(ms: i32) -> Duration {
    Duration::from_millis(u64::try_from(ms).unwrap_or(0).max(100))
}

/// The stats objects' Qt-thread handles. A part whose object is gone (the
/// window closing) is dropped.
pub struct Sink {
    pub cpu: CxxQtThread<CpuStats>,
    pub memory: CxxQtThread<MemoryStats>,
    pub health: CxxQtThread<HealthStatus>,
}

impl Sink {
    fn post(&self, tick: Tick) {
        let fresh = tick.fresh;
        // Health first: the Overview waits for memory before it shows
        // anything, so the hero never shows a tick without its health.
        // SMART and failed services answer a tick or so later, so a problem
        // with a drive or a service appears a moment after the Overview opens.
        if let Some(alerts) = tick.health {
            let _ = self.health.queue(move |o| o.apply(alerts));
        }
        if let Some(cpu) = tick.cpu {
            let _ = self.cpu.queue(move |o| o.apply(cpu, fresh));
        }
        if let Some(m) = tick.memory {
            let _ = self.memory.queue(move |o| o.apply(m, fresh));
        }
    }
}
