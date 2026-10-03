//! The devices: the sidebar's disks and interfaces with their live rates,
//! and the Disk and Network pages' figures. Like `stats.rs`, these only take
//! what the sampling thread's sink queues; nothing here reads the system.
//! A figure the machine doesn't report is NaN, or -1 for a count or size.

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
        /// The sidebar's Disk and Network entries. The lists of each kind are
        /// parallel: kernel name (the page key), what to show, and bytes per
        /// second, read and written or received and sent together.
        #[qproperty(QStringList, disk_names, cxx_name = "diskNames")]
        #[qproperty(QStringList, disk_labels, cxx_name = "diskLabels")]
        #[qproperty(QList_f64, disk_rates, cxx_name = "diskRates")]
        #[qproperty(QStringList, net_names, cxx_name = "netNames")]
        #[qproperty(QStringList, net_labels, cxx_name = "netLabels")]
        /// NaN for an interface that is gone (unplugged) or not measured yet.
        #[qproperty(QList_f64, net_rates, cxx_name = "netRates")]
        #[namespace = "atlas_monitor"]
        type DeviceList = super::DeviceListRust;

        #[qobject]
        /// The Disk page's drive.
        #[qproperty(QString, name)]
        #[qproperty(QString, label)]
        /// Compressed swap in RAM (zram): no space or health to show.
        #[qproperty(bool, swap)]
        #[qproperty(i64, size)]
        /// Filesystems on it are mounted, so `used` and `free` mean something.
        #[qproperty(bool, mounted)]
        #[qproperty(i64, used)]
        #[qproperty(i64, free)]
        #[qproperty(f64, read_rate, cxx_name = "readRate")]
        #[qproperty(f64, write_rate, cxx_name = "writeRate")]
        #[qproperty(i64, read_total, cxx_name = "readTotal")]
        #[qproperty(i64, write_total, cxx_name = "writeTotal")]
        #[qproperty(QList_f64, read_history, cxx_name = "readHistory")]
        #[qproperty(QList_f64, write_history, cxx_name = "writeHistory")]
        /// udisks2 answered about this drive; the figures below are its.
        #[qproperty(bool, smart)]
        /// What is wrong with the drive, in a sentence; empty when nothing.
        #[qproperty(QString, warning)]
        /// Percent of its rated life left; NaN where the drive doesn't say.
        #[qproperty(f64, life)]
        #[qproperty(f64, temperature)]
        #[qproperty(i64, power_on_hours, cxx_name = "powerOnHours")]
        #[qproperty(i64, power_cycles, cxx_name = "powerCycles")]
        #[qproperty(i64, written, cxx_name = "written")]
        #[qproperty(i64, unsafe_shutdowns, cxx_name = "unsafeShutdowns")]
        #[qproperty(i64, media_errors, cxx_name = "mediaErrors")]
        #[namespace = "atlas_monitor"]
        type DiskStats = super::DiskStatsRust;

        /// Forgets the last drive's figures as its page opens for `name`,
        /// so the page shows dashes until the first reading, never the
        /// previous drive's.
        #[qinvokable]
        fn show(self: Pin<&mut DiskStats>, name: &QString, label: &QString);

        #[qobject]
        /// The Network page's interface.
        #[qproperty(QString, name)]
        #[qproperty(QString, label)]
        #[qproperty(QString, mac)]
        /// Link speed in Mbit/s; 0 where there is none (Wi-Fi, a link down).
        #[qproperty(i32, speed)]
        #[qproperty(bool, wireless)]
        /// The interface is there now (an unplugged USB one is not).
        #[qproperty(bool, present)]
        #[qproperty(f64, rx_rate, cxx_name = "rxRate")]
        #[qproperty(f64, tx_rate, cxx_name = "txRate")]
        #[qproperty(i64, rx_total, cxx_name = "rxTotal")]
        #[qproperty(i64, tx_total, cxx_name = "txTotal")]
        #[qproperty(QList_f64, rx_history, cxx_name = "rxHistory")]
        #[qproperty(QList_f64, tx_history, cxx_name = "txHistory")]
        /// Empty when it has none.
        #[qproperty(QString, ipv4)]
        #[qproperty(QString, ipv6)]
        #[namespace = "atlas_monitor"]
        type NetStats = super::NetStatsRust;

        /// As DiskStats::show, for an interface.
        #[qinvokable]
        #[cxx_name = "show"]
        fn show_interface(self: Pin<&mut NetStats>, name: &QString, label: &QString);
    }

    impl cxx_qt::Threading for DeviceList {}
    impl cxx_qt::Threading for DiskStats {}
    impl cxx_qt::Threading for NetStats {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn device_list_make_unique() -> UniquePtr<DeviceList>;
        #[cxx_name = "make_unique"]
        fn disk_stats_make_unique() -> UniquePtr<DiskStats>;
        #[cxx_name = "make_unique"]
        fn net_stats_make_unique() -> UniquePtr<NetStats>;
    }
}

use core::pin::Pin;

use atlas_sysinfo::smart::{Health, Warning};
use atlas_sysinfo::stats::disk::Disk;
use atlas_sysinfo::stats::net::NetInterface;
use cxx_qt::CxxQtType;
use cxx_qt_lib::{QList, QString, QStringList};

use crate::sampling::{Devices, DiskTick, NetTick};
use crate::series::Series;

fn size(v: Option<u64>) -> i64 {
    v.map_or(-1, |v| i64::try_from(v).unwrap_or(i64::MAX))
}

fn rates(values: impl Iterator<Item = f64>) -> QList<f64> {
    let mut list = QList::default();
    for v in values {
        list.append(v);
    }
    list
}

fn strings<'a>(values: impl Iterator<Item = &'a str>) -> QStringList {
    values.map(QString::from).collect()
}

/// The interfaces the sidebar lists: not loopback. Two of one kind ("Wi-Fi")
/// are told apart by their kernel names.
fn listed(interfaces: &[NetInterface]) -> Vec<(String, String)> {
    let shown: Vec<&NetInterface> = interfaces.iter().filter(|i| !i.loopback).collect();
    shown
        .iter()
        .map(|i| {
            let twins = shown.iter().filter(|o| o.label() == i.label()).count();
            let label = if twins > 1 {
                format!("{} ({})", i.label(), i.name)
            } else {
                i.label().to_owned()
            };
            (i.name.clone(), label)
        })
        .collect()
}

#[derive(Default)]
pub struct DeviceListRust {
    disk_names: QStringList,
    disk_labels: QStringList,
    disk_rates: QList<f64>,
    net_names: QStringList,
    net_labels: QStringList,
    net_rates: QList<f64>,
    /// The listed interfaces' kernel names, in order, to match rates to.
    interfaces: Vec<String>,
}

impl qobject::DeviceList {
    pub fn apply(mut self: Pin<&mut Self>, d: Devices) {
        if let Some(disks) = &d.disks {
            let names = strings(disks.iter().map(|d| d.name.as_str()));
            // Labels before names: a row is made per name and reads its label.
            if names != *self.disk_names() {
                self.as_mut()
                    .set_disk_labels(strings(disks.iter().map(Disk::label)));
                self.as_mut().set_disk_names(names);
            }
        }
        if let Some(interfaces) = &d.interfaces {
            let listed = listed(interfaces);
            let names = strings(listed.iter().map(|(n, _)| n.as_str()));
            if names != *self.net_names() {
                self.as_mut().rust_mut().interfaces =
                    listed.iter().map(|(n, _)| n.clone()).collect();
                self.as_mut()
                    .set_net_labels(strings(listed.iter().map(|(_, l)| l.as_str())));
                self.as_mut().set_net_names(names);
            }
        }
        // Rates every tick: the disks' come in the list's order.
        self.as_mut().set_disk_rates(rates(
            d.disk_io.iter().map(|io| io.read_rate + io.write_rate),
        ));
        let net = rates(self.rust().interfaces.iter().map(|name| {
            d.net_io
                .iter()
                .find(|io| io.name == *name)
                .map_or(f64::NAN, |io| io.rx_rate + io.tx_rate)
        }));
        self.as_mut().set_net_rates(net);
    }
}

pub struct DiskStatsRust {
    name: QString,
    label: QString,
    swap: bool,
    size: i64,
    mounted: bool,
    used: i64,
    free: i64,
    read_rate: f64,
    write_rate: f64,
    read_total: i64,
    write_total: i64,
    read_history: QList<f64>,
    write_history: QList<f64>,
    smart: bool,
    warning: QString,
    life: f64,
    temperature: f64,
    power_on_hours: i64,
    power_cycles: i64,
    written: i64,
    unsafe_shutdowns: i64,
    media_errors: i64,
    reads: Series,
    writes: Series,
}

impl Default for DiskStatsRust {
    fn default() -> Self {
        Self {
            name: QString::default(),
            label: QString::default(),
            swap: false,
            size: -1,
            mounted: false,
            used: -1,
            free: -1,
            read_rate: f64::NAN,
            write_rate: f64::NAN,
            read_total: -1,
            write_total: -1,
            read_history: QList::default(),
            write_history: QList::default(),
            smart: false,
            warning: QString::default(),
            life: f64::NAN,
            temperature: f64::NAN,
            power_on_hours: -1,
            power_cycles: -1,
            written: -1,
            unsafe_shutdowns: -1,
            media_errors: -1,
            reads: Series::default(),
            writes: Series::default(),
        }
    }
}

fn warning_name(w: &Warning) -> String {
    match w {
        Warning::SpareLow => "spare blocks low".into(),
        Warning::Temperature => "temperature out of range".into(),
        Warning::Degraded => "reliability degraded".into(),
        Warning::ReadOnly => "read-only".into(),
        Warning::BackupFailed => "power-loss backup failed".into(),
        Warning::PersistentMemoryReadOnly => "persistent memory read-only".into(),
        Warning::Other(s) => s.clone(),
    }
}

/// What is wrong with the drive, as the Go version words it.
pub fn drive_warning(h: &Health) -> String {
    if h.failing && !h.warnings.is_empty() {
        let names: Vec<String> = h.warnings.iter().map(warning_name).collect();
        format!(
            "This drive reports that it is failing: {}. Back up anything on it that matters.",
            names.join(", ")
        )
    } else if h.failing {
        "This drive reports that it is failing. Back up anything on it that matters.".into()
    } else if h.spare_low {
        "This drive has nearly run out of spare blocks, which is how an SSD reports that it is close to the end of its life.".into()
    } else {
        String::new()
    }
}

impl qobject::DiskStats {
    pub fn show(mut self: Pin<&mut Self>, name: &QString, label: &QString) {
        let d = DiskStatsRust::default();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.reads = Series::default();
            rust.writes = Series::default();
        }
        self.as_mut().set_swap(d.swap);
        self.as_mut().set_size(d.size);
        self.as_mut().set_mounted(d.mounted);
        self.as_mut().set_used(d.used);
        self.as_mut().set_free(d.free);
        self.as_mut().set_read_rate(d.read_rate);
        self.as_mut().set_write_rate(d.write_rate);
        self.as_mut().set_read_total(d.read_total);
        self.as_mut().set_write_total(d.write_total);
        self.as_mut().set_read_history(d.read_history);
        self.as_mut().set_write_history(d.write_history);
        self.as_mut().set_health(None);
        // Last: the title follows the figures already cleared.
        self.as_mut().set_name(name.clone());
        self.as_mut().set_label(label.clone());
    }

    pub fn apply(mut self: Pin<&mut Self>, t: DiskTick, fresh: bool) {
        // A tick sampled for the drive shown before `show` is dropped,
        // fresh or not: the page has moved on.
        if QString::from(&t.name) != *self.name() {
            return;
        }
        if fresh {
            // The page opened again: nothing carries over.
            {
                let mut rust = self.as_mut().rust_mut();
                rust.reads = Series::default();
                rust.writes = Series::default();
            }
            self.as_mut().set_mounted(false);
            self.as_mut().set_used(-1);
            self.as_mut().set_free(-1);
        }
        if let Some(disk) = &t.disk {
            self.as_mut().set_label(QString::from(disk.label()));
            self.as_mut().set_swap(disk.is_swap);
            self.as_mut().set_size(size(Some(disk.size)));
        }
        if let Some(space) = t.space {
            self.as_mut().set_mounted(space.is_some());
            self.as_mut().set_used(size(space.map(|s| s.used)));
            self.as_mut().set_free(size(space.map(|s| s.free)));
        }
        let (read, write) =
            t.io.as_ref()
                .map_or((f64::NAN, f64::NAN), |io| (io.read_rate, io.write_rate));
        {
            let mut rust = self.as_mut().rust_mut();
            rust.reads.push(read);
            rust.writes.push(write);
        }
        let (reads, writes) = (self.rust().reads.to_qlist(), self.rust().writes.to_qlist());
        // Histories first, so the chart and its caption change together.
        self.as_mut().set_read_history(reads);
        self.as_mut().set_write_history(writes);
        self.as_mut().set_read_rate(read);
        self.as_mut().set_write_rate(write);
        self.as_mut()
            .set_read_total(size(t.io.as_ref().map(|io| io.read_total)));
        self.as_mut()
            .set_write_total(size(t.io.as_ref().map(|io| io.write_total)));
        if let Some(health) = t.health {
            self.as_mut().set_health(health.as_ref());
        }
    }

    fn set_health(mut self: Pin<&mut Self>, h: Option<&Health>) {
        self.as_mut()
            .set_warning(QString::from(&h.map(drive_warning).unwrap_or_default()));
        self.as_mut().set_life(
            h.and_then(|h| h.wear)
                // NVMe counts past 100% used on a drive run beyond its rating.
                .map_or(f64::NAN, |w| (100.0 - f64::from(w)).clamp(0.0, 100.0)),
        );
        self.as_mut()
            .set_temperature(h.and_then(|h| h.temperature).unwrap_or(f64::NAN));
        self.as_mut()
            .set_power_on_hours(size(h.and_then(|h| h.power_on_hours)));
        self.as_mut()
            .set_power_cycles(size(h.and_then(|h| h.power_cycles)));
        self.as_mut()
            .set_written(size(h.and_then(|h| h.written_bytes)));
        self.as_mut()
            .set_unsafe_shutdowns(size(h.and_then(|h| h.unsafe_shutdowns)));
        self.as_mut()
            .set_media_errors(size(h.and_then(|h| h.media_errors)));
        // Last: a binding woken by `smart` reads the rest.
        self.as_mut().set_smart(h.is_some());
    }
}

pub struct NetStatsRust {
    name: QString,
    label: QString,
    mac: QString,
    speed: i32,
    wireless: bool,
    present: bool,
    rx_rate: f64,
    tx_rate: f64,
    rx_total: i64,
    tx_total: i64,
    rx_history: QList<f64>,
    tx_history: QList<f64>,
    ipv4: QString,
    ipv6: QString,
    rx: Series,
    tx: Series,
}

impl Default for NetStatsRust {
    fn default() -> Self {
        Self {
            name: QString::default(),
            label: QString::default(),
            mac: QString::default(),
            speed: 0,
            wireless: false,
            present: false,
            rx_rate: f64::NAN,
            tx_rate: f64::NAN,
            rx_total: -1,
            tx_total: -1,
            rx_history: QList::default(),
            tx_history: QList::default(),
            ipv4: QString::default(),
            ipv6: QString::default(),
            rx: Series::default(),
            tx: Series::default(),
        }
    }
}

impl qobject::NetStats {
    pub fn show_interface(mut self: Pin<&mut Self>, name: &QString, label: &QString) {
        let d = NetStatsRust::default();
        {
            let mut rust = self.as_mut().rust_mut();
            rust.rx = Series::default();
            rust.tx = Series::default();
        }
        self.as_mut().set_mac(d.mac);
        self.as_mut().set_speed(d.speed);
        self.as_mut().set_wireless(d.wireless);
        self.as_mut().set_present(true);
        self.as_mut().set_rx_rate(d.rx_rate);
        self.as_mut().set_tx_rate(d.tx_rate);
        self.as_mut().set_rx_total(d.rx_total);
        self.as_mut().set_tx_total(d.tx_total);
        self.as_mut().set_rx_history(d.rx_history);
        self.as_mut().set_tx_history(d.tx_history);
        self.as_mut().set_ipv4(d.ipv4);
        self.as_mut().set_ipv6(d.ipv6);
        // Last: the title follows the figures already cleared.
        self.as_mut().set_name(name.clone());
        self.as_mut().set_label(label.clone());
    }

    pub fn apply(mut self: Pin<&mut Self>, t: NetTick, fresh: bool) {
        // A tick sampled for the interface shown before `show` is dropped,
        // fresh or not: the page has moved on.
        if QString::from(&t.name) != *self.name() {
            return;
        }
        if fresh {
            {
                let mut rust = self.as_mut().rust_mut();
                rust.rx = Series::default();
                rust.tx = Series::default();
            }
            self.as_mut().set_ipv4(QString::default());
            self.as_mut().set_ipv6(QString::default());
        }
        if let Some(i) = &t.interface {
            self.as_mut().set_label(QString::from(i.label()));
            self.as_mut()
                .set_mac(QString::from(i.mac.as_deref().unwrap_or_default()));
            self.as_mut().set_speed(
                i.speed_mbit
                    .map_or(0, |s| i32::try_from(s).unwrap_or(i32::MAX)),
            );
            self.as_mut().set_wireless(i.wireless);
        }
        if let Some(a) = &t.addresses {
            self.as_mut().set_ipv4(QString::from(
                &a.ipv4.map(|ip| ip.to_string()).unwrap_or_default(),
            ));
            self.as_mut().set_ipv6(QString::from(
                &a.ipv6.map(|ip| ip.to_string()).unwrap_or_default(),
            ));
        }
        let (rx, tx) =
            t.io.as_ref()
                .map_or((f64::NAN, f64::NAN), |io| (io.rx_rate, io.tx_rate));
        {
            let mut rust = self.as_mut().rust_mut();
            rust.rx.push(rx);
            rust.tx.push(tx);
        }
        let (rxh, txh) = (self.rust().rx.to_qlist(), self.rust().tx.to_qlist());
        self.as_mut().set_present(t.io.is_some());
        // Histories first, so the chart and its caption change together.
        self.as_mut().set_rx_history(rxh);
        self.as_mut().set_tx_history(txh);
        self.as_mut().set_rx_rate(rx);
        self.as_mut().set_tx_rate(tx);
        self.as_mut()
            .set_rx_total(size(t.io.as_ref().map(|io| io.rx_total)));
        self.as_mut()
            .set_tx_total(size(t.io.as_ref().map(|io| io.tx_total)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn iface(name: &str, display: &str, loopback: bool) -> NetInterface {
        NetInterface {
            name: name.into(),
            display: display.into(),
            loopback,
            ..NetInterface::default()
        }
    }

    #[test]
    fn interfaces_of_one_kind_are_told_apart() {
        let list = listed(&[
            iface("enp6s0", "Ethernet", false),
            iface("enp7s0", "Ethernet", false),
            iface("wlp4s0", "Wi-Fi", false),
            iface("lo", "Loopback", true),
        ]);
        let labels: Vec<&str> = list.iter().map(|(_, l)| l.as_str()).collect();
        assert_eq!(labels, ["Ethernet (enp6s0)", "Ethernet (enp7s0)", "Wi-Fi"]);
    }

    #[test]
    fn a_failing_drive_says_so_and_names_its_warnings() {
        let mut h = Health {
            kind: atlas_sysinfo::smart::Kind::Nvme,
            failing: false,
            warnings: Vec::new(),
            wear: None,
            spare: None,
            spare_low: false,
            temperature: None,
            temperature_limit: None,
            power_on_hours: None,
            power_cycles: None,
            read_bytes: None,
            written_bytes: None,
            unsafe_shutdowns: None,
            media_errors: None,
            bad_sectors: None,
            failing_attributes: None,
            updated: 0,
        };
        assert_eq!(drive_warning(&h), "");
        h.spare_low = true;
        assert!(drive_warning(&h).contains("spare blocks"));
        h.failing = true;
        assert!(drive_warning(&h).ends_with("Back up anything on it that matters."));
        h.warnings = vec![Warning::ReadOnly, Warning::Other("odd".into())];
        assert!(drive_warning(&h).contains("failing: read-only, odd."));
    }
}
