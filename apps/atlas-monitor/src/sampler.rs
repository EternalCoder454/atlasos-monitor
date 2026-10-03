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

        /// Kernel threads as rows of the Apps table.
        #[qinvokable]
        #[cxx_name = "showKernelThreads"]
        fn show_kernel_threads(self: &Sampler, on: bool);

        /// The window can't be seen (minimized, hidden, or suspended by
        /// the compositor): nothing is read until it can again.
        #[qinvokable]
        #[cxx_name = "setPaused"]
        fn set_paused(self: &Sampler, paused: bool);
    }

    impl cxx_qt::Threading for Sampler {}

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

use crate::battery::qobject::BatteryStats;
use crate::devices::qobject::{DeviceList, DiskStats, NetStats};
use crate::graphics::qobject::GpuStats;
use crate::processes::qobject::ProcessModel;
use crate::sampling::{Loop, Page, Tick};
use crate::sensors::qobject::SensorList;
use crate::services::qobject::ServiceModel;
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

    pub fn show_kernel_threads(&self, on: bool) {
        if let Some(l) = &self.running {
            l.set_kernel_threads(on);
        }
    }

    pub fn set_paused(&self, paused: bool) {
        if let Some(l) = &self.running {
            l.set_paused(paused);
        }
    }

    pub fn services_changed(&self) {
        if let Some(l) = &self.running {
            l.services_changed();
        }
    }

    /// Starts the loop, posting to `sink`'s objects. Called once, from
    /// `lib.rs`, before QML can call in.
    pub fn start(self: Pin<&mut Self>, ms: i32, icon_theme: String, sink: Sink) {
        match Loop::start(interval(ms), move |tick| sink.post(tick)) {
            Ok(l) => {
                l.set_icon_theme(icon_theme);
                self.rust_mut().get_mut().running = Some(l);
            }
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
    pub devices: CxxQtThread<DeviceList>,
    pub disk: CxxQtThread<DiskStats>,
    pub net: CxxQtThread<NetStats>,
    pub gpu: CxxQtThread<GpuStats>,
    pub battery: CxxQtThread<BatteryStats>,
    pub sensors: CxxQtThread<SensorList>,
    pub apps: CxxQtThread<ProcessModel>,
    pub services: CxxQtThread<ServiceModel>,
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
        if let Some(d) = tick.disk {
            let _ = self.disk.queue(move |o| o.apply(d, fresh));
        }
        if let Some(n) = tick.net {
            let _ = self.net.queue(move |o| o.apply(n, fresh));
        }
        if let Some(s) = tick.services {
            let _ = self.services.queue(move |o| o.apply(s));
        }
        if let Some(a) = tick.apps {
            let _ = self.apps.queue(move |o| o.apply(a));
        }
        if let Some(s) = tick.sensors {
            let _ = self.sensors.queue(move |o| o.apply(s));
        }
        let mut devices = tick.devices;
        if let Some(cards) = devices.cards.take() {
            let _ = self.gpu.queue(move |o| o.set_cards(&cards));
        }
        // Every reading feeds the Overview's rows; only the GPU page's
        // card feeds the page.
        if let Some(g) = tick.gpus {
            let on_page = matches!(tick.page, Page::Gpu(_));
            let _ = self.gpu.queue(move |mut o| {
                o.as_mut().set_loads(&g);
                if on_page {
                    o.apply(g, fresh);
                }
            });
        }
        if let Some(p) = devices.power.take() {
            let on_page = matches!(tick.page, Page::Battery(_));
            let _ = self.battery.queue(move |o| o.apply(p, on_page));
        }
        let _ = self.devices.queue(move |o| o.apply(devices));
    }
}
