//! The Devices page: what is on the PCI and USB buses, and the keyboards,
//! mice and other input devices. Read when the page opens and on Refresh,
//! on a thread of its own, and published as flat lists, one entry per
//! device.

#[cxx_qt::bridge]
pub mod qobject {
    unsafe extern "C++" {
        include!("cxx-qt-lib/qstring.h");
        type QString = cxx_qt_lib::QString;
        include!("cxx-qt-lib/qstringlist.h");
        type QStringList = cxx_qt_lib::QStringList;
        include!("cxx-qt-lib/qlist.h");
        type QList_bool = cxx_qt_lib::QList<bool>;
    }

    extern "RustQt" {
        #[qobject]
        /// Reading, the first time or again.
        #[qproperty(bool, loading)]
        /// The devices have been read at least once.
        #[qproperty(bool, loaded)]
        /// PCI devices, sorted by class: the class's name (the page's
        /// groups), the device, its maker, its driver and its slot.
        #[qproperty(QStringList, pci_classes, cxx_name = "pciClasses")]
        #[qproperty(QStringList, pci_names, cxx_name = "pciNames")]
        #[qproperty(QStringList, pci_vendors, cxx_name = "pciVendors")]
        #[qproperty(QStringList, pci_drivers, cxx_name = "pciDrivers")]
        #[qproperty(QStringList, pci_slots, cxx_name = "pciSlots")]
        /// USB devices: name, maker, "vendor:product" and speed, and
        /// whether it is a hub.
        #[qproperty(QStringList, usb_names, cxx_name = "usbNames")]
        #[qproperty(QStringList, usb_vendors, cxx_name = "usbVendors")]
        #[qproperty(QStringList, usb_ids, cxx_name = "usbIds")]
        #[qproperty(QStringList, usb_speeds, cxx_name = "usbSpeeds")]
        #[qproperty(QList_bool, usb_hubs, cxx_name = "usbHubs")]
        /// Input devices, sorted by kind: name, kind ("keyboard", "mouse",
        /// "touchpad", "touchscreen", "tablet", "joystick", "buttons",
        /// "other") and how it is connected ("USB", "Bluetooth",
        /// "Built-in", "Virtual", "").
        #[qproperty(QStringList, input_names, cxx_name = "inputNames")]
        #[qproperty(QStringList, input_kinds, cxx_name = "inputKinds")]
        #[qproperty(QStringList, input_buses, cxx_name = "inputBuses")]
        #[namespace = "atlas_monitor"]
        type HardwareList = super::HardwareListRust;

        /// Reads the devices again. Ignored while a read is under way.
        #[qinvokable]
        fn refresh(self: Pin<&mut HardwareList>);
    }

    impl cxx_qt::Threading for HardwareList {}

    #[namespace = "rust::cxxqtlib1"]
    unsafe extern "C++" {
        include!("cxx-qt-lib/common.h");

        #[cxx_name = "make_unique"]
        fn hardware_list_make_unique() -> UniquePtr<HardwareList>;
    }
}

use std::pin::Pin;

use atlas_sysinfo::hardware::{self, Hardware};
use cxx_qt::Threading;
use cxx_qt_lib::{QList, QString, QStringList};

#[derive(Default)]
pub struct HardwareListRust {
    loading: bool,
    loaded: bool,
    pci_classes: QStringList,
    pci_names: QStringList,
    pci_vendors: QStringList,
    pci_drivers: QStringList,
    pci_slots: QStringList,
    usb_names: QStringList,
    usb_vendors: QStringList,
    usb_ids: QStringList,
    usb_speeds: QStringList,
    usb_hubs: QList<bool>,
    input_names: QStringList,
    input_kinds: QStringList,
    input_buses: QStringList,
}

fn strings<'a>(items: impl Iterator<Item = &'a str>) -> QStringList {
    let mut list = QStringList::default();
    for s in items {
        list.append(QString::from(s));
    }
    list
}

impl qobject::HardwareList {
    pub fn refresh(mut self: Pin<&mut Self>) {
        if *self.loading() {
            return;
        }
        self.as_mut().set_loading(true);
        let qt = self.qt_thread();
        let spawned = std::thread::Builder::new()
            .name("devices".into())
            .spawn(move || {
                // A reader that panics leaves the lists empty rather than
                // the page loading for good.
                let found = std::panic::catch_unwind(hardware::read).unwrap_or_default();
                let _ = qt.queue(move |o| o.apply(found));
            });
        if let Err(e) = spawned {
            log::error!("reading the devices: {e}");
            self.as_mut().set_loading(false);
            self.as_mut().set_loaded(true);
        }
    }

    fn apply(mut self: Pin<&mut Self>, h: Hardware) {
        let pci = &h.pci;
        self.as_mut()
            .set_pci_names(strings(pci.iter().map(|d| d.name.as_str())));
        self.as_mut()
            .set_pci_vendors(strings(pci.iter().map(|d| d.vendor.as_str())));
        self.as_mut()
            .set_pci_drivers(strings(pci.iter().map(|d| d.driver.as_str())));
        self.as_mut()
            .set_pci_slots(strings(pci.iter().map(|d| d.slot.as_str())));
        // Last of its kind: the page groups the rows by class.
        self.as_mut()
            .set_pci_classes(strings(pci.iter().map(|d| d.class.as_str())));

        let usb = &h.usb;
        self.as_mut()
            .set_usb_names(strings(usb.iter().map(|d| d.name.as_str())));
        self.as_mut()
            .set_usb_vendors(strings(usb.iter().map(|d| d.vendor.as_str())));
        self.as_mut()
            .set_usb_ids(strings(usb.iter().map(|d| d.id.as_str())));
        self.as_mut()
            .set_usb_speeds(strings(usb.iter().map(|d| d.speed.as_str())));
        let mut hubs = QList::default();
        for d in usb {
            hubs.append(d.hub);
        }
        self.as_mut().set_usb_hubs(hubs);

        let input = &h.input;
        self.as_mut()
            .set_input_buses(strings(input.iter().map(|d| d.bus.as_str())));
        self.as_mut()
            .set_input_kinds(strings(input.iter().map(|d| d.kind.key())));
        self.as_mut()
            .set_input_names(strings(input.iter().map(|d| d.name.as_str())));

        self.as_mut().set_loading(false);
        self.as_mut().set_loaded(true);
    }
}
