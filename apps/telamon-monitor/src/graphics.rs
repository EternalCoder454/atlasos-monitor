//! The graphics cards: the sidebar's list and the GPU page's figures. Like
//! `stats.rs`, this only takes what the sampling thread's sink queues.
//! A figure the card doesn't report is NaN, or -1 for a size or count.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_f64 = cxx_qt_lib::QList<f64>;
    }

    extern "RustQt" {
        #[qobject]
        /// Every card, the one most worth showing first, as parallel lists
        /// of DRM node (the page key) and name; then the GPU page's card.
        #[qproperty(QStringList, card_names, cxx_name = "cardNames")]
        #[qproperty(QStringList, card_labels, cxx_name = "cardLabels")]
        /// Each card's load in percent and temperature in °C, parallel to
        /// the names, for the Overview's rows; NaN where unread.
        #[qproperty(QList_f64, card_usages, cxx_name = "cardUsages")]
        #[qproperty(QList_f64, card_temperatures, cxx_name = "cardTemperatures")]
        #[qproperty(QString, name)]
        #[qproperty(QString, label)]
        #[qproperty(QString, driver)]
        /// The PCI address; empty for a card that isn't on PCI.
        #[qproperty(QString, slot)]
        /// Part of the processor, sharing system memory.
        #[qproperty(bool, integrated)]
        /// Runtime-suspended and left asleep: nothing but its load was read.
        #[qproperty(bool, asleep)]
        /// Percent busy.
        #[qproperty(f64, usage)]
        #[qproperty(QList_f64, usage_history, cxx_name = "usageHistory")]
        /// Video memory in bytes.
        #[qproperty(i64, memory_used, cxx_name = "memoryUsed")]
        #[qproperty(i64, memory_total, cxx_name = "memoryTotal")]
        /// Video memory in use, in percent.
        #[qproperty(QList_f64, memory_history, cxx_name = "memoryHistory")]
        /// System memory the card has mapped (GTT), in bytes.
        #[qproperty(i64, gtt_used, cxx_name = "gttUsed")]
        #[qproperty(i64, gtt_total, cxx_name = "gttTotal")]
        /// °C: the main sensor, the hottest point, the video memory.
        #[qproperty(f64, temperature)]
        #[qproperty(f64, hotspot)]
        #[qproperty(f64, memory_temperature, cxx_name = "memoryTemperature")]
        #[qproperty(i64, fan_rpm, cxx_name = "fanRpm")]
        /// The fan's duty cycle, for cards that can't count its turns.
        #[qproperty(f64, fan_percent, cxx_name = "fanPercent")]
        /// Watts drawn, and the limit in force.
        #[qproperty(f64, power)]
        #[qproperty(f64, power_limit, cxx_name = "powerLimit")]
        /// MHz.
        #[qproperty(f64, core_clock, cxx_name = "coreClock")]
        #[qproperty(f64, memory_clock, cxx_name = "memoryClock")]
        #[namespace = "telamon_monitor"]
        type GpuStats = super::GpuStatsRust;

        /// Forgets the last card's figures as its page opens for `name`,
        /// so the page shows dashes until the first reading.
        #[qinvokable]
        fn show(self: Pin<&mut GpuStats>, name: &QString, label: &QString);
    }

    impl cxx_qt::Threading for GpuStats {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn gpu_stats_make_unique() -> UniquePtr<GpuStats>;
    }
}

use std::pin::Pin;

use atlas_sysinfo::gpu::Card;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QString, QStringList};

use crate::sampling::GpuTick;
use crate::series::Series;

pub struct GpuStatsRust {
    card_names: QStringList,
    card_labels: QStringList,
    card_usages: QList<f64>,
    card_temperatures: QList<f64>,
    name: QString,
    label: QString,
    driver: QString,
    slot: QString,
    integrated: bool,
    asleep: bool,
    usage: f64,
    usage_history: QList<f64>,
    memory_used: i64,
    memory_total: i64,
    memory_history: QList<f64>,
    gtt_used: i64,
    gtt_total: i64,
    temperature: f64,
    hotspot: f64,
    memory_temperature: f64,
    fan_rpm: i64,
    fan_percent: f64,
    power: f64,
    power_limit: f64,
    core_clock: f64,
    memory_clock: f64,
    usages: Series,
    memory: Series,
    /// The last video memory share, for a tick without one (asleep).
    last_share: f64,
}

impl Default for GpuStatsRust {
    fn default() -> Self {
        Self {
            card_names: QStringList::default(),
            card_labels: QStringList::default(),
            card_usages: QList::default(),
            card_temperatures: QList::default(),
            name: QString::default(),
            label: QString::default(),
            driver: QString::default(),
            slot: QString::default(),
            integrated: false,
            asleep: false,
            usage: f64::NAN,
            usage_history: QList::default(),
            memory_used: -1,
            memory_total: -1,
            memory_history: QList::default(),
            gtt_used: -1,
            gtt_total: -1,
            temperature: f64::NAN,
            hotspot: f64::NAN,
            memory_temperature: f64::NAN,
            fan_rpm: -1,
            fan_percent: f64::NAN,
            power: f64::NAN,
            power_limit: f64::NAN,
            core_clock: f64::NAN,
            memory_clock: f64::NAN,
            usages: Series::default(),
            memory: Series::default(),
            last_share: f64::NAN,
        }
    }
}

/// A size or count for QML: -1 when unknown.
fn size(v: Option<u64>) -> i64 {
    v.map_or(-1, |v| i64::try_from(v).unwrap_or(i64::MAX))
}

/// A per-card list's `i`th value, NaN past its end.
fn nth(list: &QList<f64>, i: usize) -> f64 {
    isize::try_from(i)
        .ok()
        .and_then(|i| list.get(i))
        .copied()
        .unwrap_or(f64::NAN)
}

impl qobject::GpuStats {
    /// The card list, from a fresh tick. Unchanged lists are left alone,
    /// so the sidebar doesn't rebuild on every page change. Changed, the
    /// Overview's figures follow their cards to the new places.
    pub fn set_cards(mut self: Pin<&mut Self>, cards: &[Card]) {
        let mut names = QStringList::default();
        let mut labels = QStringList::default();
        for c in cards {
            names.append(QString::from(&c.node));
            labels.append(QString::from(&c.name));
        }
        if names != *self.card_names() || labels != *self.card_labels() {
            let usages = self.carried(&names, |o| o.card_usages());
            let temperatures = self.carried(&names, |o| o.card_temperatures());
            self.as_mut().set_card_usages(usages);
            self.as_mut().set_card_temperatures(temperatures);
            self.as_mut().set_card_labels(labels);
            self.as_mut().set_card_names(names);
        }
    }

    /// A per-card list laid out for `names`: each card's last value, NaN
    /// for a card not listed before.
    fn carried(&self, names: &QStringList, values: impl Fn(&Self) -> &QList<f64>) -> QList<f64> {
        let old = values(self);
        let mut list = QList::default();
        for name in names.iter() {
            let at = self.card_names().iter().position(|n| n == name);
            list.append(at.map_or(f64::NAN, |i| nth(old, i)));
        }
        list
    }

    pub fn show(mut self: Pin<&mut Self>, name: &QString, label: &QString) {
        let d = GpuStatsRust::default();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.usages.clear();
            rust.memory.clear();
            rust.last_share = f64::NAN;
        }
        self.as_mut().set_driver(d.driver);
        self.as_mut().set_slot(d.slot);
        self.as_mut().set_integrated(d.integrated);
        self.as_mut().set_asleep(d.asleep);
        self.as_mut().set_usage(d.usage);
        self.as_mut().set_usage_history(d.usage_history);
        self.as_mut().set_memory_used(d.memory_used);
        self.as_mut().set_memory_total(d.memory_total);
        self.as_mut().set_memory_history(d.memory_history);
        self.as_mut().set_gtt_used(d.gtt_used);
        self.as_mut().set_gtt_total(d.gtt_total);
        self.as_mut().set_temperature(d.temperature);
        self.as_mut().set_hotspot(d.hotspot);
        self.as_mut().set_memory_temperature(d.memory_temperature);
        self.as_mut().set_fan_rpm(d.fan_rpm);
        self.as_mut().set_fan_percent(d.fan_percent);
        self.as_mut().set_power(d.power);
        self.as_mut().set_power_limit(d.power_limit);
        self.as_mut().set_core_clock(d.core_clock);
        self.as_mut().set_memory_clock(d.memory_clock);
        // Last: the title follows the figures already cleared.
        self.as_mut().set_name(name.clone());
        self.as_mut().set_label(label.clone());
    }

    /// Every card's load and temperature, in the sidebar's order. A card
    /// this tick didn't read (a GPU page reads only its own) keeps its last
    /// figures, so the Overview doesn't open on dashes.
    pub fn set_loads(mut self: Pin<&mut Self>, ticks: &[GpuTick]) {
        let mut usages = QList::default();
        let mut temperatures = QList::default();
        for (i, name) in self.card_names().iter().enumerate() {
            match ticks.iter().find(|t| QString::from(&t.card.node) == *name) {
                Some(t) => {
                    let g = &t.reading;
                    usages.append(g.usage.unwrap_or(f64::NAN));
                    // Asleep, the temperature wasn't read: no figure beats
                    // an old one.
                    temperatures.append(
                        Some(g)
                            .filter(|g| !g.asleep)
                            .and_then(|g| g.temperature)
                            .unwrap_or(f64::NAN),
                    );
                }
                None => {
                    usages.append(nth(self.card_usages(), i));
                    temperatures.append(nth(self.card_temperatures(), i));
                }
            }
        }
        self.as_mut().set_card_usages(usages);
        self.as_mut().set_card_temperatures(temperatures);
    }

    /// Takes the shown card's reading out of a tick's.
    pub fn apply(mut self: Pin<&mut Self>, ticks: Vec<GpuTick>, fresh: bool) {
        let Some(t) = ticks
            .into_iter()
            .find(|t| QString::from(&t.card.node) == *self.name())
        else {
            return;
        };
        let card = &t.card;
        let g = &t.reading;
        // A total once known stays: NVIDIA's comes only while awake.
        let total = g
            .memory_total
            .or(card.memory_total)
            .map(|t| size(Some(t)))
            .unwrap_or(*self.memory_total());
        let memory_share = g
            .memory_used
            .filter(|_| total > 0)
            .map_or(f64::NAN, |u| u as f64 / total as f64 * 100.0);
        let (usages, memory) = {
            let mut rust = self.as_mut().rust_mut();
            if fresh {
                rust.usages.clear();
                rust.memory.clear();
                rust.last_share = f64::NAN;
            }
            if !memory_share.is_nan() {
                rust.last_share = memory_share;
            }
            let share = rust.last_share;
            rust.usages.push(g.usage.unwrap_or(f64::NAN));
            rust.memory.push(share);
            (rust.usages.to_qlist(), rust.memory.to_qlist())
        };
        if fresh {
            self.as_mut().set_label(QString::from(&card.name));
            self.as_mut().set_driver(QString::from(&card.driver));
            self.as_mut().set_slot(QString::from(&card.slot));
            self.as_mut().set_integrated(card.integrated);
            self.as_mut().set_gtt_total(size(card.gtt_total));
        }
        self.as_mut().set_asleep(g.asleep);
        self.as_mut().set_usage(g.usage.unwrap_or(f64::NAN));
        self.as_mut().set_memory_total(total);
        self.as_mut().set_usage_history(usages);
        self.as_mut().set_memory_history(memory);
        // Asleep, nothing else was read: the last figures stay under the
        // page's note, rather than rows coming and going as the card naps.
        if g.asleep {
            return;
        }
        self.as_mut().set_memory_used(size(g.memory_used));
        self.as_mut().set_gtt_used(size(g.gtt_used));
        self.as_mut()
            .set_temperature(g.temperature.unwrap_or(f64::NAN));
        self.as_mut().set_hotspot(g.hotspot.unwrap_or(f64::NAN));
        self.as_mut()
            .set_memory_temperature(g.memory_temperature.unwrap_or(f64::NAN));
        self.as_mut().set_fan_rpm(size(g.fan_rpm));
        self.as_mut()
            .set_fan_percent(g.fan_percent.unwrap_or(f64::NAN));
        self.as_mut().set_power(g.power.unwrap_or(f64::NAN));
        self.as_mut()
            .set_power_limit(t.power_limit.unwrap_or(f64::NAN));
        self.as_mut()
            .set_core_clock(g.core_clock.unwrap_or(f64::NAN));
        self.as_mut()
            .set_memory_clock(g.memory_clock.unwrap_or(f64::NAN));
    }
}
