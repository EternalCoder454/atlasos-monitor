//! The Sensors page: every hwmon device and its readings. The page's rows
//! are built from the structure (devices, labels), which is published only
//! when it changes; the values are published every tick, so the rows (and a
//! device's unfolded cores) don't rebuild once a second.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_i32 = cxx_qt_lib::QList<i32>;
        type QList_bool = cxx_qt_lib::QList<bool>;
    }

    extern "RustQt" {
        #[qobject]
        /// The machine has a sensor at all: whether the sidebar lists the page.
        #[qproperty(bool, available)]
        /// One entry per device: its name, the kernel's name for it, where
        /// its readings start in the flat lists below and how many there are,
        /// and whether it was left asleep.
        #[qproperty(QStringList, names)]
        #[qproperty(QStringList, drivers)]
        #[qproperty(QList_i32, first)]
        #[qproperty(QList_i32, counts)]
        #[qproperty(QList_bool, asleep)]
        /// The hottest folded per-core temperature per device, "" for none.
        #[qproperty(QStringList, hottest)]
        /// Every reading, device after device: what it measures, and whether
        /// it is a per-core temperature folded behind a "Cores" row.
        #[qproperty(QStringList, labels)]
        #[qproperty(QList_bool, folded)]
        /// Every reading's value with its unit, and its grade against the
        /// hardware's limits (0, 1 past high, 2 past critical).
        #[qproperty(QStringList, values)]
        #[qproperty(QList_i32, warmths)]
        #[namespace = "atlas_monitor"]
        type SensorList = super::SensorListRust;
    }

    impl cxx_qt::Threading for SensorList {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn sensor_list_make_unique() -> UniquePtr<SensorList>;
    }
}

use std::pin::Pin;

use atlas_sysinfo::sensors::{Device, Kind};
use cxx_qt_lib::{QList, QString, QStringList};

#[derive(Default)]
pub struct SensorListRust {
    available: bool,
    names: QStringList,
    drivers: QStringList,
    first: QList<i32>,
    counts: QList<i32>,
    asleep: QList<bool>,
    hottest: QStringList,
    labels: QStringList,
    folded: QList<bool>,
    values: QStringList,
    warmths: QList<i32>,
}

/// The hottest folded core, "" when the device folds none or none read.
fn hottest(d: &Device) -> String {
    d.readings
        .iter()
        .filter(|r| r.folded && r.kind == Kind::Temperature)
        .filter_map(|r| r.value)
        .reduce(f64::max)
        .map(|v| format!("{v:.0} °C"))
        .unwrap_or_default()
}

fn strings<'a>(items: impl Iterator<Item = &'a str>) -> QStringList {
    let mut list = QStringList::default();
    for s in items {
        list.append(QString::from(s));
    }
    list
}

fn list<T>(items: impl Iterator<Item = T>) -> QList<T>
where
    T: cxx_qt_lib::QListElement + cxx::ExternType<Kind = cxx::kind::Trivial>,
{
    let mut list = QList::default();
    for v in items {
        list.append(v);
    }
    list
}

impl qobject::SensorList {
    pub fn apply(mut self: Pin<&mut Self>, devices: Vec<Device>) {
        let all = || devices.iter().flat_map(|d| &d.readings);
        let values: Vec<String> = all().map(|r| r.display()).collect();
        let values = strings(values.iter().map(String::as_str));
        let warmths = list(all().map(|r| i32::from(r.warmth())));
        // The structure: every list the page's rows are laid out from. A
        // change in any (two devices trading a reading) republishes it all.
        let labels = strings(all().map(|r| r.label.as_str()));
        let names = strings(devices.iter().map(|d| d.name.as_str()));
        let drivers = strings(devices.iter().map(|d| d.driver.as_str()));
        let folded = list(all().map(|r| r.folded));
        let counts = list(
            devices
                .iter()
                .map(|d| i32::try_from(d.readings.len()).unwrap_or(i32::MAX)),
        );
        let mut start = 0;
        let first = list(devices.iter().map(|d| {
            let at = start;
            start += d.readings.len();
            i32::try_from(at).unwrap_or(i32::MAX)
        }));
        let changed = labels != *self.labels()
            || names != *self.names()
            || drivers != *self.drivers()
            || folded != *self.folded()
            || counts != *self.counts()
            || first != *self.first();
        // Values before the structure, so new rows find theirs.
        self.as_mut().set_values(values);
        self.as_mut().set_warmths(warmths);
        if changed {
            self.as_mut().set_folded(folded);
            self.as_mut().set_labels(labels);
            self.as_mut().set_drivers(drivers);
            self.as_mut().set_first(first);
            self.as_mut().set_counts(counts);
            // Last: the page's devices are built from the name list.
            self.as_mut().set_names(names);
        }
        self.as_mut()
            .set_asleep(list(devices.iter().map(|d| d.asleep)));
        let hot: Vec<String> = devices.iter().map(hottest).collect();
        self.as_mut()
            .set_hottest(strings(hot.iter().map(String::as_str)));
    }
}
