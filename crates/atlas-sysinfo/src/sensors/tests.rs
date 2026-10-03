use std::fs;
use std::os::unix::fs::symlink;

use super::names::{self, Context, Facts, Labelling};
use super::*;

/// A fake machine: `/sys/class/hwmon`, the devices the nodes link to, CPU
/// topology and cpuinfo, all in one temporary directory.
struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    fn new() -> Self {
        let t = Self {
            dir: tempfile::tempdir().unwrap(),
        };
        fs::create_dir_all(t.path("class/hwmon")).unwrap();
        fs::write(
            t.path("cpuinfo"),
            "processor\t: 0\nmodel name\t: Intel(R) Core(TM) i9-14900KF\n\n",
        )
        .unwrap();
        t
    }

    fn path(&self, rel: &str) -> PathBuf {
        self.dir.path().join(rel)
    }

    fn roots(&self) -> Roots {
        Roots {
            hwmon: self.path("class/hwmon"),
            cpu: self.path("cpu"),
            atom_cpus: self.path("cpu_atom_cpus"),
            cpuinfo: self.path("cpuinfo"),
            drm: self.path("class/drm"),
        }
    }

    /// Adds `hwmon<n>` with `files`, linked to `devices/<device>` when
    /// given; `device_files` go in the device's directory.
    fn hwmon(
        &self,
        n: u32,
        device: Option<&str>,
        files: &[(&str, &str)],
        device_files: &[(&str, &str)],
    ) {
        let dir = self.path(&format!("class/hwmon/hwmon{n}"));
        fs::create_dir_all(&dir).unwrap();
        for (name, value) in files {
            fs::write(dir.join(name), format!("{value}\n")).unwrap();
        }
        if let Some(device) = device {
            let target = self.path(&format!("devices/{device}"));
            fs::create_dir_all(&target).unwrap();
            for (name, value) in device_files {
                let p = target.join(name);
                fs::create_dir_all(p.parent().unwrap()).unwrap();
                fs::write(p, format!("{value}\n")).unwrap();
            }
            symlink(&target, dir.join("device")).unwrap();
        }
    }

    fn write(&self, rel: &str, value: &str) {
        fs::write(self.path(rel), format!("{value}\n")).unwrap();
    }

    /// A hybrid processor: cores 0 and 4 are P-cores (two threads each),
    /// cores 32 to 35 E-cores.
    fn hybrid_topology(&self) {
        let cpus = [
            (0, 0),
            (1, 0),
            (2, 4),
            (3, 4),
            (4, 32),
            (5, 33),
            (6, 34),
            (7, 35),
        ];
        for (cpu, core) in cpus {
            let t = self.path(&format!("cpu/cpu{cpu}/topology"));
            fs::create_dir_all(&t).unwrap();
            fs::write(t.join("physical_package_id"), "0\n").unwrap();
            fs::write(t.join("core_id"), format!("{core}\n")).unwrap();
        }
        self.write("cpu_atom_cpus", "4-7");
    }
}

fn find<'a>(devices: &'a [Device], name: &str) -> &'a Device {
    devices.iter().find(|d| d.name == name).unwrap_or_else(|| {
        panic!(
            "no {name:?} in {:?}",
            devices.iter().map(|d| &d.name).collect::<Vec<_>>()
        )
    })
}

fn labels(d: &Device) -> Vec<&str> {
    d.readings.iter().map(|r| r.label.as_str()).collect()
}

fn value(d: &Device, label: &str) -> Option<f64> {
    d.readings.iter().find(|r| r.label == label).unwrap().value
}

/// A desktop like the dev machine, plus the parts it lacks.
#[test]
fn names_every_device() {
    let t = Tree::new();
    t.hybrid_topology();
    t.hwmon(
        0,
        Some("pci0000:00/0000:00:06.0/0000:04:00.0/nvme/nvme0"),
        &[
            ("name", "nvme"),
            ("temp1_label", "Composite"),
            ("temp1_input", "43850"),
            ("temp1_max", "84850"),
            ("temp1_crit", "84850"),
            ("temp2_label", "Sensor 1"),
            ("temp2_input", "39850"),
        ],
        &[("model", "Samsung SSD 970 EVO Plus 1TB            ")],
    );
    t.hwmon(
        1,
        Some("pci0000:00/0000:00:1b.0/0000:05:00.0/nvme/nvme1"),
        &[
            ("name", "nvme"),
            ("temp1_label", "Composite"),
            ("temp1_input", "40850"),
        ],
        &[("model", "Samsung SSD 970 EVO Plus 1TB")],
    );
    t.hwmon(
        2,
        Some("pci0000:00/0000:00:01.0/0000:03:00.0"),
        &[
            ("name", "amdgpu"),
            ("temp1_label", "edge"),
            ("temp1_input", "52000"),
            ("temp1_crit", "100000"),
            ("temp2_label", "junction"),
            ("temp2_input", "59000"),
            ("temp3_label", "mem"),
            ("temp3_input", "72000"),
            ("fan1_input", "546"),
            ("in0_label", "vddgfx"),
            ("in0_input", "692"),
            ("power1_label", "PPT"),
            ("power1_average", "80000000"),
        ],
        &[("power/runtime_status", "active")],
    );
    t.hwmon(
        3,
        Some("pci0000:00/0000:00:1f.4/i2c-10/10-0051"),
        &[
            ("name", "spd5118"),
            ("temp1_input", "38000"),
            ("temp1_max", "55000"),
            ("temp1_crit", "85000"),
        ],
        &[],
    );
    t.hwmon(
        4,
        Some("pci0000:00/0000:00:1f.4/i2c-10/10-0053"),
        &[("name", "spd5118"), ("temp1_input", "37750")],
        &[],
    );
    // The SMBus controller sleeps between transfers: not a reason to skip.
    fs::create_dir_all(t.path("devices/pci0000:00/0000:00:1f.4/power")).unwrap();
    t.write(
        "devices/pci0000:00/0000:00:1f.4/power/runtime_status",
        "suspended",
    );
    t.hwmon(
        5,
        Some("pci0000:00/0000:00:1c.0/0000:06:00.0/mdio_bus/r8169-0-600/r8169-0-600:00"),
        &[
            ("name", "r8169_0_600:00"),
            ("temp1_input", "45000"),
            ("temp1_max", "120000"),
        ],
        &[],
    );
    let mut coretemp: Vec<(String, String)> = [
        ("name", "coretemp"),
        ("temp1_label", "Package id 0"),
        ("temp1_input", "41000"),
        ("temp1_max", "80000"),
        ("temp1_crit", "100000"),
    ]
    .map(|(a, b)| (a.to_owned(), b.to_owned()))
    .into();
    for (i, core) in [(2, 0), (6, 4), (34, 32), (35, 33), (36, 34), (37, 35)] {
        coretemp.push((format!("temp{i}_label"), format!("Core {core}")));
        coretemp.push((format!("temp{i}_input"), format!("{}", 30000 + i * 100)));
    }
    let coretemp: Vec<(&str, &str)> = coretemp
        .iter()
        .map(|(a, b)| (a.as_str(), b.as_str()))
        .collect();
    t.hwmon(6, Some("platform/coretemp.0"), &coretemp, &[]);
    t.hwmon(
        7,
        Some("virtual/thermal/thermal_zone1"),
        &[("name", "iwlwifi_1"), ("temp1_input", "-5000")],
        &[],
    );
    t.hwmon(
        8,
        Some("platform/nct6775.656"),
        &[
            ("name", "nct6798"),
            ("temp1_label", "SYSTIN"),
            ("temp1_input", "33000"),
            ("temp2_label", "CPUTIN"),
            ("temp2_input", "40500"),
            ("temp3_label", "AUXTIN0"),
            ("temp3_input", "-128000"),
            ("temp4_label", "AUXTIN1"),
            ("temp4_input", "127000"),
            ("fan1_input", "0"),
            ("fan2_input", "1100"),
            ("fan3_input", "900"),
            ("in0_input", "1240"),
            ("in1_input", "1000"),
        ],
        &[],
    );
    t.hwmon(
        9,
        Some("platform/mystery"),
        &[
            ("name", "mystery_chip"),
            ("temp1_input", "30000"),
            ("temp2_input", "31000"),
        ],
        &[],
    );
    t.hwmon(10, None, &[("name", "empty")], &[]);
    t.hwmon(
        11,
        Some("LNXSYSTM:00/LNXSYBUS:00/PNP0C0A:00/power_supply/BAT0"),
        &[
            ("name", "BAT0"),
            ("in0_input", "12601"),
            ("curr1_input", "1520"),
        ],
        &[],
    );

    let mut s = Sensors::with_roots(t.roots());
    let devices = s.sample().to_vec();
    let names: Vec<&str> = devices.iter().map(|d| d.name.as_str()).collect();
    assert_eq!(
        names,
        [
            "Intel Core i9-14900KF",
            "Graphics Card",
            "Memory Slot 2",
            "Memory Slot 4",
            "Samsung SSD 970 EVO Plus 1TB (nvme0)",
            "Samsung SSD 970 EVO Plus 1TB (nvme1)",
            "Motherboard",
            "Ethernet Adapter",
            "Wi-Fi Adapter",
            "Battery",
            "mystery_chip",
        ]
    );
    let nodes: Vec<&str> = devices.iter().map(|d| d.node.as_str()).collect();
    assert_eq!(nodes[..3], ["hwmon6", "hwmon2", "hwmon3"]);

    let cpu = &devices[0];
    assert_eq!(cpu.driver, "coretemp");
    assert_eq!(
        labels(cpu),
        [
            "Package", "P-core 1", "P-core 2", "E-core 1", "E-core 2", "E-core 3", "E-core 4"
        ]
    );
    assert!(!cpu.readings[0].folded);
    assert!(cpu.readings[1..].iter().all(|r| r.folded), "six cores fold");
    assert_eq!(
        (cpu.readings[0].high, cpu.readings[0].critical),
        (Some(80.0), Some(100.0))
    );
    assert_eq!(cpu.readings[0].attribute, "temp1");

    let gpu = &devices[1];
    assert_eq!(
        labels(gpu),
        [
            "Edge",
            "Hotspot",
            "Memory",
            "Fan",
            "Core voltage",
            "Power draw"
        ]
    );
    let shown: Vec<String> = gpu.readings.iter().map(Reading::display).collect();
    assert_eq!(
        shown,
        ["52 °C", "59 °C", "72 °C", "546 RPM", "0.692 V", "80.0 W"]
    );
    assert_eq!(gpu.readings[0].critical, Some(100.0));
    assert!(!gpu.asleep);

    let slot2 = &devices[2];
    assert!(
        !slot2.asleep,
        "memory behind a sleeping SMBus is still read"
    );
    assert_eq!(value(slot2, "Temperature"), Some(38.0));
    assert_eq!(
        (slot2.readings[0].high, slot2.readings[0].critical),
        (Some(55.0), Some(85.0))
    );

    let drive = &devices[4];
    assert_eq!(labels(drive), ["Drive", "Sensor 1"]);
    assert_eq!(value(drive, "Drive"), Some(43.85));

    let board = &devices[6];
    // Unconnected thermistors and a stopped header left out; voltages
    // unlabelled and numbered as the chip numbers them.
    assert_eq!(
        labels(board),
        [
            "System",
            "CPU socket",
            "Fan 2",
            "Fan 3",
            "Voltage 0",
            "Voltage 1"
        ]
    );
    assert_eq!(value(board, "Voltage 0"), Some(1.24));

    let wifi = &devices[8];
    assert_eq!(value(wifi, "Temperature"), Some(-5.0), "below zero reads");

    let battery = &devices[9];
    assert_eq!(labels(battery), ["Voltage", "Current"]);
    assert_eq!(battery.readings[1].display(), "1.52 A");

    let mystery = &devices[10];
    assert_eq!(labels(mystery), ["Temperature 1", "Temperature 2"]);
    assert_eq!(mystery.category, Category::Other);
}

/// A sleeping graphics card is not read, and its limits wait until it wakes.
#[test]
fn sleeping_card_is_left_asleep() {
    let t = Tree::new();
    let card = "pci0000:00/0000:00:01.0/0000:01:00.0";
    t.hwmon(
        0,
        Some(card),
        &[
            ("name", "amdgpu"),
            ("temp1_label", "edge"),
            ("temp1_input", "45000"),
            ("temp1_crit", "100000"),
            ("energy1_input", "1000000"),
        ],
        &[("power/runtime_status", "suspended")],
    );
    let mut s = Sensors::with_roots(t.roots());
    let d = s.sample()[0].clone();
    assert!(d.asleep);
    assert!(d.readings.iter().all(|r| r.value.is_none()));
    assert_eq!(d.readings[0].critical, None, "limits not read while asleep");

    t.write(&format!("devices/{card}/power/runtime_status"), "active");
    let d = s.sample()[0].clone();
    assert!(!d.asleep);
    assert_eq!(d.readings[0].value, Some(45.0));
    assert_eq!(d.readings[0].critical, Some(100.0));
    // The energy counter's first awake reading is its baseline.
    assert_eq!(d.readings[1].kind, Kind::Power);
    assert_eq!(d.readings[1].value, None);
    std::thread::sleep(std::time::Duration::from_millis(20));
    t.write("class/hwmon/hwmon0/energy1_input", "2000000");
    let w = s.sample()[0].readings[1].value.unwrap();
    assert!(w > 0.0 && w <= 50.0, "{w}");
}

/// An energy counter is used only without a power reading of its number.
#[test]
fn energy_only_without_power() {
    let t = Tree::new();
    t.hwmon(
        0,
        None,
        &[
            ("name", "xe"),
            ("power1_input", "15000000"),
            ("energy1_input", "5"),
            ("energy2_label", "pkg"),
            ("energy2_input", "5"),
        ],
        &[],
    );
    let s = Sensors::with_roots(t.roots());
    let d = &s.devices()[0];
    let attrs: Vec<&str> = d.readings.iter().map(|r| r.attribute.as_str()).collect();
    assert_eq!(attrs, ["power1", "energy2"]);
    assert_eq!(labels(d), ["Power", "Package"]);
}

/// The first sample after discovery is an energy counter's baseline, not a
/// figure over microseconds.
#[test]
fn energy_baseline_on_first_sample() {
    let t = Tree::new();
    t.hwmon(
        0,
        None,
        &[("name", "xe"), ("energy1_input", "1000000")],
        &[],
    );
    let mut s = Sensors::with_roots(t.roots());
    assert_eq!(s.sample()[0].readings[0].value, None);
    std::thread::sleep(std::time::Duration::from_millis(20));
    t.write("class/hwmon/hwmon0/energy1_input", "2000000");
    assert!(s.sample()[0].readings[0].value.is_some());
}

/// A node that went and came back under the same number is found again,
/// and devices of one name are listed by what tells them apart.
#[test]
fn same_node_replaced() {
    let t = Tree::new();
    let drive = |n: u32, ctrl: &str| {
        t.hwmon(
            n,
            Some(&format!("pci0000:00/0000:0{n}:00.0/nvme/{ctrl}")),
            &[("name", "nvme"), ("temp1_input", "40000")],
            &[("model", "Same Drive")],
        );
    };
    drive(1, "nvme1");
    drive(2, "nvme0");
    let mut s = Sensors::with_roots(t.roots());
    let names: Vec<String> = s.sample().iter().map(|d| d.name.clone()).collect();
    assert_eq!(names, ["Same Drive (nvme0)", "Same Drive (nvme1)"]);

    fs::remove_dir_all(t.path("class/hwmon/hwmon1")).unwrap();
    t.hwmon(
        1,
        None,
        &[("name", "spd5118"), ("temp1_input", "36000")],
        &[],
    );
    for _ in 1..RESCAN_TICKS {
        s.sample();
    }
    assert!(s.devices().iter().any(|d| d.driver == "spd5118"));
}

/// A device that comes, and a header that starts turning, show up within
/// their recheck intervals.
#[test]
fn rescans() {
    let t = Tree::new();
    t.hwmon(
        0,
        None,
        &[
            ("name", "nct6798"),
            ("fan1_input", "0"),
            ("fan2_input", "800"),
        ],
        &[],
    );
    let mut s = Sensors::with_roots(t.roots());
    assert_eq!(labels(&s.sample()[0]), ["Fan 2"]);

    t.hwmon(
        1,
        None,
        &[("name", "spd5118"), ("temp1_input", "36000")],
        &[],
    );
    for _ in 1..RESCAN_TICKS - 1 {
        assert_eq!(s.sample().len(), 1);
    }
    assert_eq!(
        s.sample().len(),
        2,
        "the new device after {RESCAN_TICKS} ticks"
    );

    t.write("class/hwmon/hwmon0/fan1_input", "650");
    let mut ticks = 0;
    while labels(find(s.sample(), "Motherboard")) != ["Fan 1", "Fan 2"] {
        ticks += 1;
        assert!(ticks <= RECHECK_TICKS, "the fan never came back");
    }
    assert_eq!(
        value(find(s.devices(), "Motherboard"), "Fan 1"),
        Some(650.0)
    );
}

#[test]
fn warmth_grades_temperatures() {
    let mut r = Reading {
        label: String::new(),
        kind: Kind::Temperature,
        value: None,
        high: Some(80.0),
        critical: Some(100.0),
        folded: false,
        attribute: "temp1".into(),
        index: 1,
    };
    for (v, want) in [(50.0, 0), (80.0, 1), (99.9, 1), (100.0, 2)] {
        r.value = Some(v);
        assert_eq!(r.warmth(), want, "{v}");
    }
    r.value = None;
    assert_eq!(r.warmth(), 0);
    assert_eq!(r.display(), "—");
    r.value = Some(120.0);
    r.kind = Kind::Fan;
    assert_eq!(r.warmth(), 0, "a fan is not graded");
    r.kind = Kind::Temperature;
    (r.high, r.critical) = (None, None);
    assert_eq!(r.warmth(), 0, "no limits, no grade");
}

#[test]
fn processor_names() {
    for (model, want) in [
        ("Intel(R) Core(TM) i9-14900KF", "Intel Core i9-14900KF"),
        (
            "Intel(R) Core(TM) i7-8700 CPU @ 3.20GHz",
            "Intel Core i7-8700",
        ),
        (
            "13th Gen Intel(R) Core(TM) i7-1360P",
            "13th Gen Intel Core i7-1360P",
        ),
        ("AMD Ryzen 9 7950X 16-Core Processor", "AMD Ryzen 9 7950X"),
        (
            "AMD Ryzen 7 7840U w/ Radeon  780M Graphics",
            "AMD Ryzen 7 7840U",
        ),
        (
            "AMD Ryzen 5 5600G with Radeon Graphics",
            "AMD Ryzen 5 5600G",
        ),
        (
            "Intel(R) Xeon(R) Gold 6338 CPU @ 2.00GHz",
            "Intel Xeon Gold 6338",
        ),
    ] {
        assert_eq!(names::processor_name(model), want, "{model}");
    }
}

#[test]
fn reading_labels() {
    let none = Labelling::default();
    let die = Labelling {
        has_die: true,
        ..Labelling::default()
    };
    let t = Kind::Temperature;
    for (given, l, want) in [
        ("Tctl", &none, "Package"),
        ("Tctl", &die, "Control"),
        ("Tdie", &die, "Package"),
        ("Tccd3", &none, "Chiplet 3"),
        ("AUXTIN2", &none, "Auxiliary 2"),
        ("PCH_CHIP_TEMP", &none, "Chipset"),
        ("Package id 1", &none, "Package"),
        ("Core 12", &none, "Core 12"),
        ("vram", &none, "Memory"),
        ("card", &none, "Card"),
        ("PECI Agent 0", &none, "PECI Agent 0"),
        ("Sensor 2", &none, "Sensor 2"),
    ] {
        assert_eq!(names::label(given, t, 1, false, l), want, "{given}");
    }
    assert_eq!(names::label("", Kind::Fan, 3, true, &none), "Fan 3");
    assert_eq!(names::label("", Kind::Fan, 3, false, &none), "Fan");
}

#[test]
fn core_names_only_on_hybrid() {
    assert!(names::core_names(&[(0, false), (1, false)]).is_empty());
    let n = names::core_names(&[(8, false), (32, true), (0, false), (0, false), (40, true)]);
    assert_eq!(n[&0], "P-core 1");
    assert_eq!(n[&8], "P-core 2");
    assert_eq!(n[&32], "E-core 1");
    assert_eq!(n[&40], "E-core 2");
    assert_eq!(n.len(), 4);
}

#[test]
fn identifies_by_driver() {
    let ctx = Context {
        processor: None,
        cards: vec![("0000:03:00.0".into(), "AMD Radeon RX 7900 XTX".into())],
    };
    let id = |driver: &str, address: &str, pci: Option<&str>| {
        let f = Facts {
            driver: driver.into(),
            address: address.into(),
            pci: pci.map(Into::into),
            ..Facts::default()
        };
        let (name, category, _) = names::identify(&f, &ctx);
        (name, category)
    };
    let c = |s: &str, cat| (s.to_owned(), cat);
    assert_eq!(
        id("amdgpu", "0000:03:00.0", Some("0000:03:00.0")),
        c("AMD Radeon RX 7900 XTX", Category::Graphics)
    );
    assert_eq!(
        id("k10temp", "0000:00:18.3", Some("0000:00:18.3")),
        c("Processor", Category::Processor)
    );
    assert_eq!(
        id("jc42", "0-001a", None),
        c("Memory Slot 3", Category::Memory)
    );
    assert_eq!(
        id("ee1004", "garbage", None),
        c("Memory Module", Category::Memory)
    );
    assert_eq!(
        id("drivetemp", "0:0:0:0", None),
        c("Drive", Category::Storage)
    );
    assert_eq!(
        id("mt7921_phy0", "", None),
        c("Wi-Fi Adapter", Category::Network)
    );
    assert_eq!(
        id("acpitz", "LNXTHERM:00", None),
        c("Thermal Zone", Category::Motherboard)
    );
    assert_eq!(
        id("thinkpad", "thinkpad_hwmon", None),
        c("Embedded Controller", Category::Motherboard)
    );
    assert_eq!(id("BAT1", "", None), c("Battery", Category::PowerSupply));
    assert_eq!(id("bat_thing", "", None), c("bat_thing", Category::Other));
    assert_eq!(
        id("ADP1", "", None),
        c("Power Adapter", Category::PowerSupply)
    );
    assert_eq!(
        id("ucsi_source_psy_USBC000:001", "", None),
        c("USB-C Port", Category::PowerSupply)
    );
}

#[test]
fn attributes_and_cpu_lists() {
    assert_eq!(
        parse_attribute("temp12_input"),
        Some((Kind::Temperature, 12, "temp", Suffix::Input))
    );
    assert_eq!(
        parse_attribute("power1_average"),
        Some((Kind::Power, 1, "power", Suffix::Average))
    );
    assert_eq!(
        parse_attribute("in0_input"),
        Some((Kind::Voltage, 0, "in", Suffix::Input))
    );
    assert_eq!(
        parse_attribute("energy1_input"),
        Some((Kind::Power, 1, "energy", Suffix::Input))
    );
    for bad in [
        "energy1_average",
        "temp1_max",
        "temp_input",
        "freq1_input",
        "name",
        "pwm1",
    ] {
        assert_eq!(parse_attribute(bad), None, "{bad}");
    }
    assert_eq!(
        parse_cpu_list("0-3,8,10-11\n"),
        Some(vec![0, 1, 2, 3, 8, 10, 11])
    );
    assert_eq!(parse_cpu_list(""), Some(vec![]));
    assert_eq!(parse_cpu_list("3-1"), None);
    assert_eq!(parse_cpu_list("x"), None);
}

#[test]
fn no_hwmon() {
    let t = Tree::new();
    fs::remove_dir(t.path("class/hwmon")).unwrap();
    assert!(!available_in(&t.path("class/hwmon")));
    let mut s = Sensors::with_roots(t.roots());
    assert!(s.sample().is_empty());
}

/// On the live system: invariants only, since CI's container has whatever
/// the host lends it, often nothing.
#[test]
fn live_invariants() {
    let mut s = Sensors::new();
    let devices = s.sample().to_vec();
    if !devices.is_empty() {
        assert!(available());
    }
    let mut nodes = std::collections::HashSet::new();
    for pair in devices.windows(2) {
        assert!(
            pair[0].category <= pair[1].category,
            "{} before {}",
            pair[0].name,
            pair[1].name
        );
    }
    for d in &devices {
        assert!(nodes.insert(&d.node), "{} twice", d.node);
        assert!(!d.name.is_empty() && !d.readings.is_empty(), "{d:?}");
        let mut attrs = std::collections::HashSet::new();
        for r in &d.readings {
            assert!(
                attrs.insert(&r.attribute),
                "{} {} twice",
                d.node,
                r.attribute
            );
            assert!(!r.label.is_empty());
            // No bounds: a drive may report its "not set" 255 °C.
            assert!(r.value.is_none_or(f64::is_finite));
            if let (Some(h), Some(c)) = (r.high, r.critical) {
                assert!(h > 0.0 && c > 0.0);
            }
        }
    }
}
