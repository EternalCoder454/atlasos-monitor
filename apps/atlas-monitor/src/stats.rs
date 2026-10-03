//! The pages' live figures: one QObject per kind, plain properties for the
//! current values and a `list<real>` per chart. The sampling thread's sink
//! (`sampler.rs`) queues each tick's parts here; nothing in this file reads
//! the system. A figure the machine doesn't report is NaN (`isNaN()` in
//! QML), never a 0 that would read as a real value.

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
        /// The processor: the Overview's row and the CPU page.
        #[qproperty(QString, model)]
        #[qproperty(i32, sockets)]
        #[qproperty(i32, cores)]
        #[qproperty(i32, threads)]
        /// MHz; NaN where the driver doesn't rate one.
        #[qproperty(f64, base_frequency, cxx_name = "baseFrequency")]
        /// Cache sizes in bytes; 0 where the kernel doesn't say.
        #[qproperty(i64, l1d)]
        #[qproperty(i64, l1i)]
        #[qproperty(i64, l2)]
        #[qproperty(i64, l3)]
        /// Load, 0..=100.
        #[qproperty(f64, usage)]
        /// The fastest core's clock in MHz; NaN without cpufreq.
        #[qproperty(f64, frequency)]
        /// Package temperature in °C; NaN without a sensor.
        #[qproperty(f64, temperature)]
        /// Load per logical processor, 0..=100.
        #[qproperty(QList_f64, core_usage, cxx_name = "coreUsage")]
        #[qproperty(QList_f64, usage_history, cxx_name = "usageHistory")]
        #[namespace = "atlas_monitor"]
        type CpuStats = super::CpuStatsRust;

        #[qobject]
        /// Memory and swap, in bytes, for the Overview and Memory page.
        #[qproperty(i64, total)]
        #[qproperty(i64, used)]
        #[qproperty(i64, available)]
        #[qproperty(i64, free)]
        #[qproperty(i64, cached)]
        #[qproperty(i64, swap_total, cxx_name = "swapTotal")]
        #[qproperty(i64, swap_used, cxx_name = "swapUsed")]
        /// Percent of memory and of swap in use, 0..=100.
        #[qproperty(QList_f64, usage_history, cxx_name = "usageHistory")]
        #[qproperty(QList_f64, swap_history, cxx_name = "swapHistory")]
        #[namespace = "atlas_monitor"]
        type MemoryStats = super::MemoryStatsRust;

        #[qobject]
        /// What is wrong with the machine, worst first, for the Overview.
        /// The four lists are parallel.
        /// 0 when all is well, 1 for a warning, 2 when something is critical.
        #[qproperty(i32, level)]
        #[qproperty(QStringList, titles)]
        #[qproperty(QStringList, details)]
        #[qproperty(QList_i32, levels)]
        /// The page that helps with each, "apps" or "services", or "".
        #[qproperty(QStringList, pages)]
        #[namespace = "atlas_monitor"]
        type HealthStatus = super::HealthStatusRust;
    }

    impl cxx_qt::Threading for CpuStats {}
    impl cxx_qt::Threading for MemoryStats {}
    impl cxx_qt::Threading for HealthStatus {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn cpu_stats_make_unique() -> UniquePtr<CpuStats>;
        #[cxx_name = "make_unique"]
        fn memory_stats_make_unique() -> UniquePtr<MemoryStats>;
        #[cxx_name = "make_unique"]
        fn health_status_make_unique() -> UniquePtr<HealthStatus>;
    }
}

use core::pin::Pin;

use atlas_sysinfo::health::{Alert, Level, Problem, worst};
use atlas_sysinfo::stats::memory::Memory;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QString, QStringList};

use crate::sampling::CpuTick;
use crate::series::Series;

fn to_i64(v: u64) -> i64 {
    i64::try_from(v).unwrap_or(i64::MAX)
}

/// A reading for a property where NaN means "not reported". The setters
/// notify whenever old != new, which NaN always is, so an unreported value
/// would wake every binding on every tick.
fn reported(v: Option<f64>) -> f64 {
    v.unwrap_or(f64::NAN)
}

fn changed(old: f64, new: f64) -> bool {
    old.to_bits() != new.to_bits() && !(old.is_nan() && new.is_nan())
}

fn count(v: usize) -> i32 {
    i32::try_from(v).unwrap_or(i32::MAX)
}

pub struct CpuStatsRust {
    model: QString,
    sockets: i32,
    cores: i32,
    threads: i32,
    base_frequency: f64,
    l1d: i64,
    l1i: i64,
    l2: i64,
    l3: i64,
    usage: f64,
    frequency: f64,
    temperature: f64,
    core_usage: QList<f64>,
    usage_history: QList<f64>,
    history: Series,
}

impl Default for CpuStatsRust {
    fn default() -> Self {
        Self {
            model: QString::default(),
            sockets: 0,
            cores: 0,
            threads: 0,
            base_frequency: f64::NAN,
            l1d: 0,
            l1i: 0,
            l2: 0,
            l3: 0,
            usage: 0.0,
            frequency: f64::NAN,
            temperature: f64::NAN,
            core_usage: QList::default(),
            usage_history: QList::default(),
            history: Series::default(),
        }
    }
}

impl qobject::CpuStats {
    pub fn apply(mut self: Pin<&mut Self>, tick: CpuTick, fresh: bool) {
        if let Some(info) = tick.info {
            let c = info.caches;
            let mut s = self.as_mut();
            s.as_mut().set_model(QString::from(&info.model));
            s.as_mut().set_sockets(count(info.sockets));
            s.as_mut().set_cores(count(info.physical_cores));
            s.as_mut().set_threads(count(info.logical));
            let base = reported(info.base_mhz);
            if changed(*s.base_frequency(), base) {
                s.as_mut().set_base_frequency(base);
            }
            s.as_mut().set_l1d(to_i64(c.l1d.unwrap_or(0)));
            s.as_mut().set_l1i(to_i64(c.l1i.unwrap_or(0)));
            s.as_mut().set_l2(to_i64(c.l2.unwrap_or(0)));
            s.as_mut().set_l3(to_i64(c.l3.unwrap_or(0)));
        }
        let sample = tick.sample;
        {
            let mut rust = self.as_mut().rust_mut();
            if fresh {
                rust.history.clear();
            }
            rust.history.push(sample.usage);
        }
        let history = self.rust().history.to_qlist();
        let mut cores = QList::default();
        cores.reserve(sample.cores.len() as isize);
        for &c in &sample.cores {
            cores.append(c);
        }
        self.as_mut().set_usage(sample.usage);
        let frequency = reported(sample.frequency_mhz);
        if changed(*self.frequency(), frequency) {
            self.as_mut().set_frequency(frequency);
        }
        let temperature = reported(sample.temperature);
        if changed(*self.temperature(), temperature) {
            self.as_mut().set_temperature(temperature);
        }
        self.as_mut().set_core_usage(cores);
        self.as_mut().set_usage_history(history);
    }
}

#[derive(Default)]
pub struct MemoryStatsRust {
    total: i64,
    used: i64,
    available: i64,
    free: i64,
    cached: i64,
    swap_total: i64,
    swap_used: i64,
    usage_history: QList<f64>,
    swap_history: QList<f64>,
    usage: Series,
    swap: Series,
}

impl qobject::MemoryStats {
    pub fn apply(mut self: Pin<&mut Self>, m: Memory, fresh: bool) {
        {
            let mut rust = self.as_mut().rust_mut();
            if fresh {
                rust.usage.clear();
                rust.swap.clear();
            }
            rust.usage.push(m.usage_percent());
            rust.swap.push(m.swap_percent());
        }
        let (usage, swap) = (self.rust().usage.to_qlist(), self.rust().swap.to_qlist());
        self.as_mut().set_total(to_i64(m.total));
        self.as_mut().set_used(to_i64(m.used));
        self.as_mut().set_available(to_i64(m.available));
        self.as_mut().set_free(to_i64(m.free));
        self.as_mut().set_cached(to_i64(m.cached));
        self.as_mut().set_swap_total(to_i64(m.swap_total));
        self.as_mut().set_swap_used(to_i64(m.swap_used));
        self.as_mut().set_usage_history(usage);
        self.as_mut().set_swap_history(swap);
    }
}

#[derive(Default)]
pub struct HealthStatusRust {
    level: i32,
    titles: QStringList,
    details: QStringList,
    levels: QList<i32>,
    pages: QStringList,
    /// What was last published, so an unchanged list sends no signals.
    shown: Vec<Alert>,
}

fn level_number(level: Option<Level>) -> i32 {
    match level {
        None => 0,
        Some(Level::Warning) => 1,
        Some(Level::Critical) => 2,
    }
}

/// Where the Overview's button takes the user for a problem: what is
/// heating the processor or filling memory is on Apps, failed services on
/// Services. The disk and drive alerts name a drive as people know it,
/// not the device a page is keyed by, so they have none yet.
fn page_for(p: &Problem) -> &'static str {
    match p {
        Problem::HotProcessor { .. }
        | Problem::HotGraphics { .. }
        | Problem::LowMemory { .. }
        | Problem::HeavySwap { .. } => "apps",
        Problem::ServicesFailed { .. } => "services",
        Problem::DiskNearlyFull { .. }
        | Problem::DriveFailing { .. }
        | Problem::DriveOutOfSpares { .. }
        | Problem::DriveWorn { .. } => "",
    }
}

impl qobject::HealthStatus {
    pub fn apply(mut self: Pin<&mut Self>, alerts: Vec<Alert>) {
        if alerts == self.rust().shown {
            return;
        }
        let titles: QStringList = alerts.iter().map(|a| QString::from(&a.title())).collect();
        let details: QStringList = alerts.iter().map(|a| QString::from(&a.detail())).collect();
        let pages: QStringList = alerts
            .iter()
            .map(|a| QString::from(page_for(&a.problem)))
            .collect();
        let mut levels = QList::default();
        for a in &alerts {
            levels.append(level_number(Some(a.level)));
        }
        let level = level_number(worst(&alerts));
        self.as_mut().rust_mut().shown = alerts;
        // The lists first: a binding woken by `level` reads them.
        self.as_mut().set_titles(titles);
        self.as_mut().set_details(details);
        self.as_mut().set_levels(levels);
        self.as_mut().set_pages(pages);
        self.as_mut().set_level(level);
    }
}
