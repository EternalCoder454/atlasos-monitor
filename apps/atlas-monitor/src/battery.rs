//! The batteries: the sidebar's list and the Battery page's figures. Read
//! with the sidebar on every tick, so this only takes what the sink queues.
//! A figure the firmware doesn't give is NaN, or -1 for a count.

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
        /// The sidebar's entries as parallel lists: page key ("total", or a
        /// pack's kernel name), what to show, and percent charged. Empty on
        /// a machine with no battery; one entry for one pack; with two or
        /// more, their sum first and then each pack.
        #[qproperty(QStringList, pack_names, cxx_name = "packNames")]
        #[qproperty(QStringList, pack_labels, cxx_name = "packLabels")]
        #[qproperty(QList_f64, pack_percents, cxx_name = "packPercents")]
        /// The Battery page's pack, by its page key.
        #[qproperty(QString, name)]
        #[qproperty(QString, label)]
        /// The pack is there (a pulled hot-swap pack's page stays open).
        #[qproperty(bool, present)]
        /// "charging", "discharging", "notCharging", "full" or "unknown".
        #[qproperty(QString, status)]
        #[qproperty(f64, percent)]
        /// Watt-hours in the pack, when full today, and when new.
        #[qproperty(f64, energy)]
        #[qproperty(f64, full)]
        #[qproperty(f64, design)]
        /// Watts in or out.
        #[qproperty(f64, watts)]
        #[qproperty(f64, volts)]
        #[qproperty(i32, cycles)]
        /// What full holds against new, in percent; and its grade, 0 to 2.
        #[qproperty(f64, health)]
        #[qproperty(i32, wear)]
        /// The firmware's charge limit in percent; -1 for none.
        #[qproperty(i32, charge_limit, cxx_name = "chargeLimit")]
        /// Seconds to empty or to full; -1 when there is no estimate.
        #[qproperty(i64, time_left, cxx_name = "timeLeft")]
        /// "Vendor · Model · Li-ion": what the pack is.
        #[qproperty(QString, identity)]
        /// The machine lists an adapter, and one is giving power.
        #[qproperty(bool, has_adapter, cxx_name = "hasAdapter")]
        #[qproperty(bool, on_ac, cxx_name = "onAc")]
        /// The adapters giving power: "USB-C Port 2 (65 W)".
        #[qproperty(QString, adapters)]
        #[qproperty(QList_f64, charge_history, cxx_name = "chargeHistory")]
        #[qproperty(QList_f64, draw_history, cxx_name = "drawHistory")]
        #[namespace = "atlas_monitor"]
        type BatteryStats = super::BatteryStatsRust;

        /// Forgets the last pack's figures as its page opens for `name`.
        #[qinvokable]
        fn show(self: Pin<&mut BatteryStats>, name: &QString, label: &QString);
    }

    impl cxx_qt::Threading for BatteryStats {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn battery_stats_make_unique() -> UniquePtr<BatteryStats>;
    }
}

use std::pin::Pin;

use atlas_sysinfo::power::{Battery, Status, Supplies};
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QString, QStringList};

use crate::series::Series;

/// The page key of every pack summed.
const TOTAL: &str = "total";

pub struct BatteryStatsRust {
    pack_names: QStringList,
    pack_labels: QStringList,
    pack_percents: QList<f64>,
    name: QString,
    label: QString,
    present: bool,
    status: QString,
    percent: f64,
    energy: f64,
    full: f64,
    design: f64,
    watts: f64,
    volts: f64,
    cycles: i32,
    health: f64,
    wear: i32,
    charge_limit: i32,
    time_left: i64,
    identity: QString,
    has_adapter: bool,
    on_ac: bool,
    adapters: QString,
    charge_history: QList<f64>,
    draw_history: QList<f64>,
    charge: Series,
    draw: Series,
    /// The last tick's supplies, to fill a page as it opens.
    last: Option<Supplies>,
}

impl Default for BatteryStatsRust {
    fn default() -> Self {
        Self {
            pack_names: QStringList::default(),
            pack_labels: QStringList::default(),
            pack_percents: QList::default(),
            name: QString::default(),
            label: QString::default(),
            present: false,
            status: QString::from("unknown"),
            percent: f64::NAN,
            energy: f64::NAN,
            full: f64::NAN,
            design: f64::NAN,
            watts: f64::NAN,
            volts: f64::NAN,
            cycles: -1,
            health: f64::NAN,
            wear: 0,
            charge_limit: -1,
            time_left: -1,
            identity: QString::default(),
            has_adapter: false,
            on_ac: false,
            adapters: QString::default(),
            charge_history: QList::default(),
            draw_history: QList::default(),
            charge: Series::default(),
            draw: Series::default(),
            last: None,
        }
    }
}

/// The sidebar's entries: (page key, label, percent).
fn entries(s: &Supplies) -> Vec<(String, String, f64)> {
    let Some(total) = &s.total else {
        return Vec::new();
    };
    let percent = |b: &Battery| b.percent.unwrap_or(f64::NAN);
    if s.packs.len() < 2 {
        return vec![(TOTAL.to_owned(), "Battery".to_owned(), percent(total))];
    }
    let mut out = vec![(TOTAL.to_owned(), "All Batteries".to_owned(), percent(total))];
    out.extend(
        s.packs
            .iter()
            .map(|p| (p.name.clone(), p.name.clone(), percent(p))),
    );
    out
}

fn strings<'a>(items: impl Iterator<Item = &'a str>) -> QStringList {
    let mut list = QStringList::default();
    for s in items {
        list.append(QString::from(s));
    }
    list
}

fn status_key(s: Status) -> &'static str {
    match s {
        Status::Charging => "charging",
        Status::Discharging => "discharging",
        Status::NotCharging => "notCharging",
        Status::Full => "full",
        Status::Unknown => "unknown",
    }
}

fn identity(b: &Battery, packs: usize) -> String {
    let mut parts: Vec<String> = [&b.vendor, &b.model, &b.technology]
        .into_iter()
        .filter(|s| !s.is_empty())
        .cloned()
        .collect();
    if b.name.is_empty() && packs > 1 {
        parts.push(format!("{packs} packs"));
    }
    match (b.name.is_empty(), parts.is_empty()) {
        (false, true) => b.name.clone(),
        (false, false) => format!("{} — {}", b.name, parts.join(" · ")),
        (true, _) => parts.join(" · "),
    }
}

fn adapters(s: &Supplies) -> String {
    s.adapters
        .iter()
        .filter(|a| a.online)
        .map(|a| match a.watts {
            Some(w) => format!("{} ({w:.0} W)", a.name),
            None => a.name.clone(),
        })
        .collect::<Vec<_>>()
        .join(", ")
}

impl qobject::BatteryStats {
    pub fn show(mut self: Pin<&mut Self>, name: &QString, label: &QString) {
        {
            let mut rust = self.as_mut().rust_mut();
            rust.charge.clear();
            rust.draw.clear();
        }
        self.as_mut().set_charge_history(QList::default());
        self.as_mut().set_draw_history(QList::default());
        self.as_mut().set_name(name.clone());
        self.as_mut().set_label(label.clone());
        // The sidebar's supplies are read every tick: fill the page from
        // the last at once, rather than wait a tick on dashes.
        let last = self.rust().last.clone();
        match last {
            Some(s) => self.fill(&s),
            None => self.clear(),
        }
    }

    /// Every tick's supplies, read with the sidebar. The page's figures
    /// move only while it is on screen.
    pub fn apply(mut self: Pin<&mut Self>, s: Supplies, on_page: bool) {
        let entries = entries(&s);
        let names = strings(entries.iter().map(|e| e.0.as_str()));
        let labels = strings(entries.iter().map(|e| e.1.as_str()));
        // Labels first: the sidebar reads them by a name's index.
        if labels != *self.pack_labels() {
            self.as_mut().set_pack_labels(labels);
        }
        if names != *self.pack_names() {
            self.as_mut().set_pack_names(names);
        }
        let changed = {
            let old = self.pack_percents();
            old.len() != entries.len() as isize
                || entries
                    .iter()
                    .zip(old.iter())
                    .any(|(e, o)| !(e.2 == *o || (e.2.is_nan() && o.is_nan())))
        };
        if changed {
            let mut percents = QList::default();
            for e in &entries {
                percents.append(e.2);
            }
            self.as_mut().set_pack_percents(percents);
        }
        if on_page {
            self.as_mut().fill(&s);
        }
        self.as_mut().rust_mut().last = Some(s);
    }

    /// Puts the shown pack's figures from `s` on the page, and a sample on
    /// its charts.
    fn fill(mut self: Pin<&mut Self>, s: &Supplies) {
        let on_ac = s.on_ac();
        self.as_mut().set_has_adapter(on_ac.is_some());
        self.as_mut().set_on_ac(on_ac == Some(true));
        self.as_mut().set_adapters(QString::from(&adapters(s)));

        let shown = self.name().to_string();
        let pack = if shown == TOTAL {
            s.total.as_ref()
        } else {
            s.packs.iter().find(|p| p.name == shown)
        };
        let Some(b) = pack else {
            // Pulled, or the page key is stale: no figures, not the last ones.
            self.clear();
            return;
        };
        // The title follows a pack docked or pulled beside this one
        // ("Battery" becomes "All Batteries").
        if let Some((_, label, _)) = entries(s).into_iter().find(|e| e.0 == shown) {
            self.as_mut().set_label(QString::from(&label));
        }
        let (charge, draw) = {
            let mut rust = self.as_mut().rust_mut();
            // An unknown charge is a gap, not a fall to 0%.
            if let Some(p) = b.percent {
                rust.charge.push(p);
            }
            rust.draw.push(b.watts.unwrap_or(f64::NAN));
            (rust.charge.to_qlist(), rust.draw.to_qlist())
        };
        self.as_mut().set_charge_history(charge);
        self.as_mut().set_draw_history(draw);
        self.as_mut()
            .set_status(QString::from(status_key(b.status)));
        self.as_mut().set_percent(b.percent.unwrap_or(f64::NAN));
        self.as_mut().set_energy(b.energy_wh.unwrap_or(f64::NAN));
        self.as_mut().set_full(b.full_wh.unwrap_or(f64::NAN));
        self.as_mut().set_design(b.design_wh.unwrap_or(f64::NAN));
        self.as_mut().set_watts(b.watts.unwrap_or(f64::NAN));
        self.as_mut().set_volts(b.volts.unwrap_or(f64::NAN));
        self.as_mut().set_cycles(
            b.cycles
                .map_or(-1, |c| i32::try_from(c).unwrap_or(i32::MAX)),
        );
        self.as_mut().set_health(b.health.unwrap_or(f64::NAN));
        self.as_mut().set_wear(i32::from(b.wear()));
        self.as_mut()
            .set_charge_limit(b.charge_limit.map_or(-1, i32::from));
        self.as_mut().set_time_left(
            b.time_left
                .map_or(-1, |t| i64::try_from(t.as_secs()).unwrap_or(i64::MAX)),
        );
        self.as_mut()
            .set_identity(QString::from(&identity(b, s.packs.len())));
        // Last: the page's sections show once the figures are in.
        self.as_mut().set_present(true);
    }

    /// Every figure to its unknown value; the charts keep their line.
    fn clear(mut self: Pin<&mut Self>) {
        let d = BatteryStatsRust::default();
        self.as_mut().set_present(d.present);
        self.as_mut().set_status(d.status);
        self.as_mut().set_percent(d.percent);
        self.as_mut().set_energy(d.energy);
        self.as_mut().set_full(d.full);
        self.as_mut().set_design(d.design);
        self.as_mut().set_watts(d.watts);
        self.as_mut().set_volts(d.volts);
        self.as_mut().set_cycles(d.cycles);
        self.as_mut().set_health(d.health);
        self.as_mut().set_wear(d.wear);
        self.as_mut().set_charge_limit(d.charge_limit);
        self.as_mut().set_time_left(d.time_left);
        self.as_mut().set_identity(d.identity);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pack(name: &str, percent: f64) -> Battery {
        Battery {
            name: name.to_owned(),
            percent: Some(percent),
            ..Battery::default()
        }
    }

    #[test]
    fn one_pack_is_one_entry_and_two_add_their_sum() {
        assert!(entries(&Supplies::default()).is_empty());

        let one = Supplies {
            total: Some(pack("BAT0", 80.0)),
            packs: vec![pack("BAT0", 80.0)],
            ..Supplies::default()
        };
        let e = entries(&one);
        assert_eq!(e.len(), 1);
        assert_eq!(e[0].0, TOTAL);

        let two = Supplies {
            total: Some(pack("", 60.0)),
            packs: vec![pack("BAT0", 80.0), pack("BAT1", 40.0)],
            ..Supplies::default()
        };
        let keys: Vec<String> = entries(&two).into_iter().map(|e| e.0).collect();
        assert_eq!(keys, [TOTAL, "BAT0", "BAT1"]);
    }
}
