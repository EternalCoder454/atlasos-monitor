use std::fs;

use super::*;

/// A fake `/sys/class/power_supply` in a temporary directory.
struct Tree {
    dir: tempfile::TempDir,
}

impl Tree {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().unwrap(),
        }
    }

    fn root(&self) -> PathBuf {
        self.dir.path().to_path_buf()
    }

    /// Adds supply `name` with `files`.
    fn supply(&self, name: &str, files: &[(&str, &str)]) {
        let dir = self.dir.path().join(name);
        fs::create_dir_all(&dir).unwrap();
        for (file, value) in files {
            self.write(name, file, value);
        }
    }

    /// Rewrites one file in place, the way a held descriptor sees the
    /// kernel's new value.
    fn write(&self, name: &str, file: &str, value: &str) {
        fs::write(self.dir.path().join(name).join(file), format!("{value}\n")).unwrap();
    }

    fn remove(&self, name: &str) {
        fs::remove_dir_all(self.dir.path().join(name)).unwrap();
    }

    fn sampler(&self) -> PowerSampler {
        PowerSampler::with_root(self.root())
    }
}

/// An ACPI laptop battery in energies: 40 of 50 Wh, design 60, 10 W out.
fn energy_pack(t: &Tree, name: &str) {
    t.supply(
        name,
        &[
            ("type", "Battery"),
            ("scope", "System"),
            ("present", "1"),
            ("status", "Discharging"),
            ("capacity", "80"),
            ("energy_now", "40000000"),
            ("energy_full", "50000000"),
            ("energy_full_design", "60000000"),
            ("power_now", "10000000"),
            ("voltage_now", "11800000"),
            ("cycle_count", "123"),
            ("manufacturer", "SMP"),
            ("model_name", "5B10W13930"),
            ("technology", "Li-poly"),
        ],
    );
}

fn close(a: Option<f64>, b: f64) -> bool {
    a.is_some_and(|a| (a - b).abs() < 1e-6)
}

#[test]
fn energy_shape() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    let mut s = t.sampler();
    let r = s.sample().clone();
    assert_eq!(r.packs.len(), 1);
    let b = &r.packs[0];
    assert_eq!(b.name, "BAT0");
    assert_eq!(
        (b.vendor.as_str(), b.model.as_str(), b.technology.as_str()),
        ("SMP", "5B10W13930", "Li-poly")
    );
    assert_eq!(b.status, Status::Discharging);
    assert!(close(b.percent, 80.0));
    assert!(close(b.energy_wh, 40.0));
    assert!(close(b.full_wh, 50.0));
    assert!(close(b.design_wh, 60.0));
    assert!(close(b.watts, 10.0));
    assert!(close(b.volts, 11.8));
    assert!(close(b.health, 50.0 / 60.0 * 100.0));
    assert_eq!(b.wear(), 1);
    assert_eq!(b.cycles, Some(123));
    assert_eq!(b.time_left, Some(Duration::from_secs(4 * 3600)));
    assert_eq!(r.total.as_ref(), Some(b), "one pack is its own total");
    assert_eq!(r.on_ac(), None, "no adapter listed");
}

/// Charges in µAh turn into energy at the design voltage, so capacity and
/// health hold still as the live voltage sags under load.
#[test]
fn charge_shape_uses_the_design_voltage() {
    let t = Tree::new();
    t.supply(
        "BAT1",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("charge_now", "2000000"),
            ("charge_full", "4000000"),
            ("charge_full_design", "5000000"),
            ("current_now", "1000000"),
            ("voltage_now", "7000000"),
            ("voltage_min_design", "7600000"),
        ],
    );
    let mut s = t.sampler();
    let b = s.sample().packs[0].clone();
    assert!(close(b.energy_wh, 2.0 * 7.6));
    assert!(close(b.full_wh, 4.0 * 7.6));
    assert!(close(b.design_wh, 5.0 * 7.6));
    assert!(
        close(b.watts, 7.0),
        "the rate is live: amps at the live voltage"
    );
    assert!(close(b.percent, 50.0), "no capacity file: energy over full");
    assert!(close(b.health, 80.0));

    t.write("BAT1", "voltage_now", "6500000");
    let b = s.sample().packs[0].clone();
    assert!(close(b.full_wh, 4.0 * 7.6), "capacity moved with the load");
    assert!(close(b.watts, 6.5));
}

#[test]
fn charge_shape_without_a_design_voltage_uses_the_live_one() {
    let t = Tree::new();
    t.supply(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Charging"),
            ("charge_now", "1000000"),
            ("charge_full", "2000000"),
            ("voltage_now", "4000000"),
        ],
    );
    let b = t.sampler().sample().packs[0].clone();
    assert!(close(b.energy_wh, 4.0));
    assert!(close(b.full_wh, 8.0));
    assert_eq!(b.design_wh, None);
    assert_eq!(b.health, None);
    assert_eq!(b.watts, None, "no rate file");
    assert_eq!(b.time_left, None);
}

#[test]
fn capacity_only() {
    let t = Tree::new();
    t.supply(
        "BAT0",
        &[("type", "Battery"), ("status", "Full"), ("capacity", "100")],
    );
    let b = t.sampler().sample().packs[0].clone();
    assert_eq!(b.status, Status::Full);
    assert!(close(b.percent, 100.0));
    assert_eq!((b.energy_wh, b.full_wh, b.health), (None, None, None));
    assert_eq!(b.time_left, None);
}

/// Some drivers count discharge as a negative rate.
#[test]
fn negative_rates_are_magnitudes() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    t.write("BAT0", "power_now", "-8000000");
    assert!(close(t.sampler().sample().packs[0].watts, 8.0));

    let t = Tree::new();
    t.supply(
        "battery",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("charge_now", "1000000"),
            ("charge_full", "2000000"),
            ("current_now", "-500000"),
            ("voltage_now", "4000000"),
        ],
    );
    let b = t.sampler().sample().packs[0].clone();
    assert!(close(b.watts, 2.0));
    assert_eq!(b.time_left, Some(Duration::from_secs(2 * 3600)));
}

#[test]
fn the_drivers_own_estimate_wins() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    t.supply("BAT0", &[("time_to_empty_now", "5400")]);
    let mut s = t.sampler();
    assert_eq!(
        s.sample().packs[0].time_left,
        Some(Duration::from_secs(5400))
    );
    // A nonsense figure falls back to the estimate.
    t.write("BAT0", "time_to_empty_now", "0");
    assert_eq!(
        s.sample().packs[0].time_left,
        Some(Duration::from_secs(4 * 3600))
    );
}

#[test]
fn charging_counts_to_the_charge_limit() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    t.write("BAT0", "status", "Charging");
    t.write("BAT0", "charge_control_end_threshold", "80");
    let b = t.sampler().sample().packs[0].clone();
    assert_eq!(b.charge_limit, Some(80));
    // 40 Wh to the limit's 40 Wh of 50: already there.
    assert_eq!(b.time_left, None);

    t.write("BAT0", "energy_now", "30000000");
    let b = t.sampler().sample().packs[0].clone();
    assert_eq!(
        b.time_left,
        Some(Duration::from_secs(3600)),
        "10 Wh at 10 W"
    );

    t.write("BAT0", "charge_control_end_threshold", "100");
    let b = t.sampler().sample().packs[0].clone();
    assert_eq!(b.charge_limit, None, "100 is no limit");
    assert_eq!(b.time_left, Some(Duration::from_secs(2 * 3600)));
}

#[test]
fn implausible_rates_give_no_estimate() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    t.write("BAT0", "power_now", "1"); // 1 µW: forty million hours
    assert_eq!(t.sampler().sample().packs[0].time_left, None);
    t.write("BAT0", "power_now", "0");
    assert_eq!(t.sampler().sample().packs[0].time_left, None);
}

/// A burst of load moves the estimate a little, not by hours; plugging in
/// starts the average over.
#[test]
fn the_estimate_uses_an_averaged_rate() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    let mut s = t.sampler();
    let left = |s: &mut PowerSampler, secs: u64| {
        s.sample_at(Duration::from_secs(secs)).packs[0]
            .time_left
            .unwrap()
            .as_secs_f64()
    };
    assert_eq!(left(&mut s, 0), 4.0 * 3600.0);
    t.write("BAT0", "power_now", "40000000");
    let burst = left(&mut s, 1);
    assert!(
        burst > 3.5 * 3600.0 && burst < 4.0 * 3600.0,
        "one second at 40 W moved the estimate to {burst} s"
    );
    // Held for minutes, the estimate follows: 40 Wh at 40 W.
    let settled = left(&mut s, 600);
    assert!((settled - 3600.0).abs() < 1.0, "{settled}");

    t.write("BAT0", "status", "Charging");
    t.write("BAT0", "power_now", "20000000");
    assert_eq!(
        left(&mut s, 601),
        1800.0,
        "10 Wh to full at 20 W, unaveraged"
    );
}

#[test]
fn two_packs() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    energy_pack(&t, "BAT1");
    // The second pack is full and idle while the first drains.
    t.write("BAT1", "status", "Full");
    t.write("BAT1", "energy_now", "20000000");
    t.write("BAT1", "energy_full", "20000000");
    t.write("BAT1", "energy_full_design", "24000000");
    t.write("BAT1", "power_now", "0");
    t.write("BAT1", "capacity", "100");
    let r = t.sampler().sample().clone();
    assert_eq!(
        r.packs.iter().map(|p| p.name.as_str()).collect::<Vec<_>>(),
        ["BAT0", "BAT1"]
    );
    let total = r.total.unwrap();
    assert_eq!(total.name, "");
    assert_eq!(total.status, Status::Discharging);
    assert!(close(total.energy_wh, 60.0));
    assert!(close(total.full_wh, 70.0));
    assert!(close(total.percent, 60.0 / 70.0 * 100.0));
    assert!(close(total.health, 70.0 / 84.0 * 100.0));
    assert!(close(total.watts, 10.0));
    assert_eq!(total.cycles, None, "cycles don't add up across packs");
    assert_eq!(total.time_left, Some(Duration::from_secs(6 * 3600)));
    assert!(close(r.packs[1].percent, 100.0));
    assert_eq!(r.packs[1].time_left, None);
}

#[test]
fn an_empty_bay_is_left_out() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    energy_pack(&t, "BAT1");
    t.write("BAT1", "present", "0");
    let mut s = t.sampler();
    let r = s.sample();
    assert_eq!(r.packs.len(), 1);
    assert_eq!(r.total.as_ref().unwrap().name, "BAT0");
    t.write("BAT1", "present", "1");
    assert_eq!(s.sample().packs.len(), 2, "a pack slid into the bay");
}

#[test]
fn peripherals_and_empty_batteries_are_not_the_machines() {
    let t = Tree::new();
    t.supply(
        "hid-dc:2c:26:00:00:01-battery",
        &[("type", "Battery"), ("scope", "Device"), ("capacity", "40")],
    );
    t.supply(
        "sony_controller_battery_00:11:22",
        &[("type", "Battery"), ("scope", "device"), ("capacity", "90")],
    );
    t.supply("BAT9", &[("type", "Battery"), ("status", "Unknown")]);
    assert!(!available_in(&t.root()));
    let r = t.sampler().sample().clone();
    assert!(r.packs.is_empty());
    assert_eq!(r.total, None);

    energy_pack(&t, "BAT0");
    assert!(available_in(&t.root()));
}

#[test]
fn no_supplies_at_all() {
    let t = Tree::new();
    assert!(!available_in(&t.root()));
    assert_eq!(*t.sampler().sample(), Supplies::default());
    let missing = PowerSampler::with_root(t.root().join("absent"));
    assert!(missing.supplies().packs.is_empty());
    assert!(!available_in(&t.root().join("absent")));
}

#[test]
fn adapters() {
    let t = Tree::new();
    t.supply("ADP1", &[("type", "Mains"), ("online", "0")]);
    t.supply(
        "ucsi-source-psy-USBC000:001",
        &[
            ("type", "USB"),
            ("online", "1"),
            ("voltage_max", "20000000"),
            ("current_max", "3250000"),
        ],
    );
    t.supply(
        "ucsi-source-psy-USBC000:002",
        &[
            ("type", "USB"),
            ("online", "0"),
            ("voltage_max", "5000000"),
            ("current_max", "3000000"),
        ],
    );
    t.supply(
        "hidpp_battery_0",
        &[("type", "USB"), ("scope", "Device"), ("online", "1")],
    );
    let mut s = t.sampler();
    let r = s.sample().clone();
    let names: Vec<&str> = r.adapters.iter().map(|a| a.name.as_str()).collect();
    assert_eq!(names, ["AC Adapter", "USB-C Port 1", "USB-C Port 2"]);
    assert_eq!(r.adapters[0].kind, AdapterKind::Mains);
    assert!(!r.adapters[0].online);
    assert_eq!(r.adapters[0].watts, None);
    assert!(r.adapters[1].online);
    assert!(close(r.adapters[1].watts, 65.0));
    assert_eq!(r.adapters[2].watts, None, "a port with nothing in it");
    assert_eq!(r.on_ac(), Some(true));

    t.write("ucsi-source-psy-USBC000:001", "online", "0");
    let r = s.sample();
    assert_eq!(r.on_ac(), Some(false));
    assert_eq!(r.adapters[1].watts, None);
}

#[test]
fn a_programmable_source_is_online_and_a_plain_usb_charger_is_not_a_port() {
    let t = Tree::new();
    t.supply(
        "tcpm-source-psy-4-0022",
        &[
            ("type", "USB"),
            ("usb_type", "C PD [PD_PPS]"),
            ("online", "2"),
        ],
    );
    t.supply("dwc3-usb-charger", &[("type", "USB_DCP"), ("online", "0")]);
    t.supply(
        "ucsi-source-psy-USBC000:001",
        &[
            ("type", "USB"),
            ("online", "1"),
            // A non-fixed offer read as a fixed one: not believed.
            ("voltage_max", "48000000"),
            ("current_max", "100000000"),
        ],
    );
    let r = t.sampler().sample().clone();
    let names: Vec<(&str, bool)> = r
        .adapters
        .iter()
        .map(|a| (a.name.as_str(), a.online))
        .collect();
    assert_eq!(
        names,
        [
            ("USB-C Port 1", true),
            ("USB-C Port 2", true),
            ("USB Charger", false)
        ]
    );
    assert_eq!(r.adapters[1].watts, None);
    assert_eq!(r.on_ac(), Some(true));
}

#[test]
fn supplies_that_come_and_go() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    let mut s = t.sampler();
    let tick = |s: &mut PowerSampler, n: u32| s.sample_at(Duration::from_secs(n.into())).clone();
    assert_eq!(tick(&mut s, 0).packs.len(), 1);

    energy_pack(&t, "BAT1");
    t.supply("AC", &[("type", "Mains"), ("online", "1")]);
    let mut seen = None;
    for n in 1..RESCAN_TICKS {
        seen = Some(tick(&mut s, n));
    }
    let r = seen.unwrap();
    assert_eq!(r.packs.len(), 2, "found on the tenth tick");
    assert_eq!(r.on_ac(), Some(true));

    t.remove("BAT1");
    for n in RESCAN_TICKS..2 * RESCAN_TICKS {
        tick(&mut s, n);
    }
    let r = s.supplies();
    assert_eq!(r.packs.len(), 1);
    assert_eq!(r.packs[0].name, "BAT0");
}

#[test]
fn malformed_values_are_missing_not_wrong() {
    let t = Tree::new();
    t.supply(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Exploding"),
            ("capacity", "lots"),
            ("energy_now", ""),
            ("energy_full", "-5"),
            ("power_now", "fast"),
            ("voltage_now", "0"),
            ("cycle_count", "0"),
            ("technology", "Unknown"),
            ("manufacturer", "  "),
        ],
    );
    let b = t.sampler().sample().packs[0].clone();
    assert_eq!(b.status, Status::Unknown);
    assert_eq!(b.percent, None);
    assert_eq!((b.energy_wh, b.full_wh, b.health), (None, None, None));
    assert_eq!((b.watts, b.volts, b.cycles), (None, None, None));
    assert_eq!((b.technology.as_str(), b.vendor.as_str()), ("", ""));
    assert_eq!(b.time_left, None);
}

#[test]
fn readings_follow_the_files() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    let mut s = t.sampler();
    assert!(close(s.sample().packs[0].percent, 80.0));
    t.write("BAT0", "capacity", "79");
    t.write("BAT0", "energy_now", "39500000");
    let b = &s.sample().packs[0];
    assert!(close(b.percent, 79.0));
    assert!(close(b.energy_wh, 39.5));
}

#[test]
fn health_is_clamped_and_a_capacity_past_100_is_not_believed() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    t.write("BAT0", "capacity", "255");
    t.write("BAT0", "energy_full", "61000000");
    let b = t.sampler().sample().packs[0].clone();
    assert!(
        close(b.percent, 40.0 / 61.0 * 100.0),
        "energy over full instead"
    );
    assert!(close(b.health, 100.0), "a new pack holding over its design");
    assert_eq!(b.wear(), 0);
}

/// ACPI unregisters a pulled pack's supply: its held files stop reading.
/// It must not linger as an "Unknown" pack with nothing in it.
#[test]
fn a_supply_that_stops_reading_is_gone_at_once() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    energy_pack(&t, "BAT1");
    let mut s = t.sampler();
    assert_eq!(s.sample_at(Duration::from_secs(1)).packs.len(), 2);
    // What a held descriptor on a removed sysfs file reads: nothing.
    for f in ["status", "energy_now", "capacity", "present"] {
        fs::write(t.root().join("BAT1").join(f), "").unwrap();
    }
    let r = s.sample_at(Duration::from_secs(2)).clone();
    assert_eq!(r.packs.len(), 1);
    let total = r.total.unwrap();
    assert_eq!(total.name, "BAT0", "the total is the pack that is left");
    assert!(total.time_left.is_some());
    assert!(s.rescan, "the directory is listed again on the next tick");
    // Still unreadable on the next tick: no second forced listing.
    s.rescan = false;
    s.sample_at(Duration::from_secs(3));
    assert!(!s.rescan, "a pack that stays unreadable lists nothing more");

    t.remove("BAT1");
    s.rescan = true;
    let r = s.sample_at(Duration::from_secs(4)).clone();
    assert_eq!(r.packs.len(), 1);
    assert_eq!(s.packs.len(), 1, "BAT1's files were let go");
    assert!(!s.rescan);
}

/// Time left on a charge-shaped pack is charge over current, whatever the
/// live voltage does to the watts.
#[test]
fn a_charge_packs_estimate_ignores_the_voltage() {
    let t = Tree::new();
    t.supply(
        "BAT0",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("charge_now", "2000000"),
            ("charge_full", "4000000"),
            ("current_now", "1000000"),
            ("voltage_now", "8200000"),
            ("voltage_min_design", "7600000"),
            ("voltage_max_design", "8800000"),
        ],
    );
    let b = t.sampler().sample().packs[0].clone();
    assert!(close(b.watts, 8.2));
    assert!(close(b.energy_wh, 2.0 * 7.6), "at the nominal, not the max");
    let left = b.time_left.unwrap().as_secs_f64();
    assert!((left - 2.0 * 3600.0).abs() < 0.01, "{left} s, not 2 h");

    // Two of them summed: the total's estimate holds too.
    t.supply(
        "BAT1",
        &[
            ("type", "Battery"),
            ("status", "Discharging"),
            ("charge_now", "1000000"),
            ("charge_full", "4000000"),
            ("current_now", "1000000"),
            ("voltage_now", "7000000"),
            ("voltage_min_design", "7600000"),
        ],
    );
    let total = t.sampler().sample().total.clone().unwrap();
    let left = total.time_left.unwrap().as_secs_f64();
    assert!((left - 1.5 * 3600.0).abs() < 0.01, "3 Ah at 2 A: {left} s");
}

/// An idle, full second pack often has no rate: the total keeps the
/// draining pack's.
#[test]
fn a_total_with_an_idle_pack_that_has_no_rate() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    energy_pack(&t, "BAT1");
    t.write("BAT1", "status", "Full");
    t.write("BAT1", "power_now", "");
    let total = t.sampler().sample().total.clone().unwrap();
    assert!(close(total.watts, 10.0));
    assert_eq!(total.time_left, Some(Duration::from_secs(8 * 3600)));

    // A moving pack without a rate leaves the sum unknown, not low.
    t.write("BAT1", "status", "Discharging");
    let total = t.sampler().sample().total.clone().unwrap();
    assert_eq!((total.watts, total.time_left), (None, None));
}

/// Without energies, the packs' percentages are weighted by what each
/// holds: 100% of 20 Wh and 50% of 50 Wh is 64%, not 75%.
#[test]
fn a_total_without_energies_weights_the_packs() {
    let t = Tree::new();
    for (name, capacity, full) in [("BAT0", "100", "20000000"), ("BAT1", "50", "50000000")] {
        t.supply(
            name,
            &[
                ("type", "Battery"),
                ("status", "Discharging"),
                ("capacity", capacity),
                ("energy_full", full),
            ],
        );
    }
    let total = t.sampler().sample().total.clone().unwrap();
    assert!(close(total.percent, 45.0 / 70.0 * 100.0));
}

#[test]
fn a_total_charging_one_pack_while_the_other_drains() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    energy_pack(&t, "BAT1");
    t.write("BAT1", "status", "Charging");
    let total = t.sampler().sample().total.clone().unwrap();
    assert_eq!(total.status, Status::Charging);
}

#[test]
fn a_totals_estimate_counts_to_a_shared_limit_and_restarts_on_a_change() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    energy_pack(&t, "BAT1");
    for name in ["BAT0", "BAT1"] {
        t.write(name, "status", "Charging");
        t.write(name, "energy_now", "30000000");
        t.write(name, "charge_control_end_threshold", "80");
    }
    let mut s = t.sampler();
    let total = s.sample_at(Duration::from_secs(1)).total.clone().unwrap();
    assert_eq!(total.charge_limit, Some(80));
    // 20 Wh to the limit's 80 of 100, at 20 W.
    assert_eq!(total.time_left, Some(Duration::from_secs(3600)));

    for name in ["BAT0", "BAT1"] {
        t.write(name, "status", "Discharging");
        t.write(name, "power_now", "30000000");
    }
    let total = s.sample_at(Duration::from_secs(2)).total.clone().unwrap();
    // 60 Wh at 60 W, unaveraged: the charging rate is forgotten.
    assert_eq!(total.time_left, Some(Duration::from_secs(3600)));
}

#[test]
fn the_drivers_time_to_full_counts_to_100() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    t.write("BAT0", "status", "Charging");
    t.write("BAT0", "energy_now", "30000000");
    t.supply("BAT0", &[("time_to_full_now", "4000")]);
    assert_eq!(
        t.sampler().sample().packs[0].time_left,
        Some(Duration::from_secs(4000))
    );
    // With a limit, the driver's figure overshoots: the estimate to the
    // limit is used, 10 Wh at 10 W.
    t.write("BAT0", "charge_control_end_threshold", "80");
    assert_eq!(
        t.sampler().sample().packs[0].time_left,
        Some(Duration::from_secs(3600))
    );
}

/// The cycle count and charge limit are read when found and every tenth
/// tick, not every tick.
#[test]
fn slow_figures_are_read_every_tenth_tick() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    let mut s = t.sampler();
    assert_eq!(s.sample().packs[0].cycles, Some(123));
    t.write("BAT0", "cycle_count", "124");
    for _ in 2..RESCAN_TICKS {
        assert_eq!(s.sample().packs[0].cycles, Some(123));
    }
    assert_eq!(s.sample().packs[0].cycles, Some(124));
}

/// A rescan that finds the same pack keeps its averaged rate.
#[test]
fn a_rescan_keeps_a_packs_rate() {
    let t = Tree::new();
    energy_pack(&t, "BAT0");
    let mut s = t.sampler();
    let left = |s: &mut PowerSampler, n: u64| {
        s.sample_at(Duration::from_secs(n)).packs[0]
            .time_left
            .unwrap()
    };
    left(&mut s, 1);
    t.write("BAT0", "power_now", "40000000");
    for n in 2..RESCAN_TICKS {
        left(&mut s, n.into());
    }
    let before = left(&mut s, 9);
    t.supply("AC", &[("type", "Mains"), ("online", "0")]); // forces a discover
    let after = left(&mut s, 10);
    assert!(
        after > Duration::from_secs(3600) && after < before,
        "the average went on: {before:?} then {after:?}"
    );
}

#[test]
fn names_order_by_their_numbers() {
    let t = Tree::new();
    for n in ["BAT10", "BAT2", "BAT1"] {
        energy_pack(&t, n);
    }
    let names: Vec<String> = t
        .sampler()
        .sample()
        .packs
        .iter()
        .map(|p| p.name.clone())
        .collect();
    assert_eq!(names, ["BAT1", "BAT2", "BAT10"]);
    assert_eq!(
        natural("a01", "a1"),
        Ordering::Less,
        "a tie broken by bytes"
    );
    assert_eq!(natural("a9b", "a10a"), Ordering::Less);
    assert_eq!(natural("", "a"), Ordering::Less);
}

#[test]
fn status_labels() {
    for (text, status) in [
        ("Charging", Status::Charging),
        ("Discharging\n", Status::Discharging),
        ("Not charging", Status::NotCharging),
        ("Full", Status::Full),
        ("", Status::Unknown),
    ] {
        assert_eq!(Status::parse(text.as_bytes()), status);
    }
    assert_eq!(Status::NotCharging.label(), "Not charging");
}

/// This machine, whatever it has: CI has no battery, a laptop does.
#[test]
fn live_invariants() {
    let mut s = PowerSampler::new();
    let r = s.sample().clone();
    // An empty bay is available but not in the reading.
    assert!(r.packs.is_empty() || available());
    assert_eq!(r.total.is_some(), !r.packs.is_empty());
    for b in r.packs.iter().chain(&r.total) {
        for p in [b.percent, b.health].into_iter().flatten() {
            assert!((0.0..=100.0).contains(&p), "{b:?}");
        }
        for v in [b.energy_wh, b.full_wh, b.design_wh, b.watts, b.volts]
            .into_iter()
            .flatten()
        {
            assert!(v >= 0.0 && v.is_finite(), "{b:?}");
        }
        if let (Some(e), Some(f)) = (b.energy_wh, b.full_wh) {
            // Firmware can overshoot full a little; not by half again.
            assert!(e <= f * 1.5, "{b:?}");
        }
        assert!(
            b.time_left
                .is_none_or(|d| d.as_secs_f64() <= MAX_HOURS * 3600.0)
        );
    }
    for a in &r.adapters {
        assert!(!a.name.is_empty());
        assert!(a.online || a.watts.is_none());
    }
}
