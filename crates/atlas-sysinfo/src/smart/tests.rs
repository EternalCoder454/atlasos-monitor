//! The NVMe fixtures are a Samsung 970 EVO Plus's replies, recorded with
//! `busctl --json` (identifiers left out). No SATA drive was at hand, so the
//! ATA ones are written from udisks2 2.11's interface and libblockdev 3.5's
//! attribute names, with a Samsung 870 EVO's values.

use std::fs;

use serde_json::Value as Json;

use super::*;

const NVME_PROPS: &str = include_str!("../../tests/fixtures/udisks_nvme_props_json");
const NVME_ATTRS: &str = include_str!("../../tests/fixtures/udisks_nvme_attrs_json");
const ATA_PROPS: &str = include_str!("../../tests/fixtures/udisks_ata_props_json");
const ATA_ATTRS: &str = include_str!("../../tests/fixtures/udisks_ata_attrs_json");

/// An `a{sv}` reply as `busctl --json` writes it.
fn props(text: &str) -> Props {
    let reply: Json = serde_json::from_str(text).unwrap();
    reply["data"][0]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), value(v)))
        .collect()
}

/// One `{"type": ..., "data": ...}` variant.
fn value(v: &Json) -> OwnedValue {
    let d = &v["data"];
    let n = || d.as_i64().unwrap();
    let v = match v["type"].as_str().unwrap() {
        "y" => Value::U8(n() as u8),
        "q" => Value::U16(n() as u16),
        "u" => Value::U32(n() as u32),
        "t" => Value::U64(d.as_u64().unwrap()),
        "i" => Value::I32(n() as i32),
        "x" => Value::I64(n()),
        "b" => Value::Bool(d.as_bool().unwrap()),
        "d" => Value::F64(d.as_f64().unwrap()),
        "s" => Value::from(d.as_str().unwrap().to_owned()),
        "as" => Value::from(
            d.as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
        ),
        "aq" => Value::from(
            d.as_array()
                .unwrap()
                .iter()
                .map(|n| n.as_u64().unwrap() as u16)
                .collect::<Vec<_>>(),
        ),
        other => panic!("no fixture type {other}"),
    };
    OwnedValue::try_from(v).unwrap()
}

fn ata_attrs(text: &str) -> Vec<AtaAttribute> {
    let reply: Json = serde_json::from_str(text).unwrap();
    reply["data"][0]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| AtaAttribute {
            name: row[1].as_str().unwrap().to_owned(),
            value: row[3].as_i64().unwrap() as i32,
            pretty: row[6].as_i64().unwrap(),
            pretty_unit: row[7].as_i64().unwrap() as i32,
        })
        .collect()
}

fn set(map: &mut Props, key: &str, v: Value<'_>) {
    map.insert(key.to_owned(), OwnedValue::try_from(v).unwrap());
}

#[test]
fn nvme_recorded_reply() {
    let h = nvme(&props(NVME_PROPS), &props(NVME_ATTRS)).unwrap();
    assert_eq!(h.kind, Kind::Nvme);
    assert!(!h.failing);
    assert!(h.warnings.is_empty());
    assert_eq!(h.wear, Some(6));
    assert_eq!(h.spare, Some(100));
    assert!(!h.spare_low);
    // 315 K and a warning temperature of 358 K.
    assert!((h.temperature.unwrap() - 41.85).abs() < 1e-9);
    assert!((h.temperature_limit.unwrap() - 84.85).abs() < 1e-9);
    assert_eq!(h.power_on_hours, Some(16595));
    assert_eq!(h.power_cycles, Some(2060));
    assert_eq!(h.written_bytes, Some(90_201_113_088_000));
    assert_eq!(h.read_bytes, Some(108_720_236_544_000));
    assert_eq!(h.unsafe_shutdowns, Some(157));
    assert_eq!(h.media_errors, Some(0));
    assert_eq!(h.bad_sectors, None);
    assert_eq!(h.updated, 1_790_989_948);
}

#[test]
fn nvme_not_read_yet_is_nothing() {
    let mut p = props(NVME_PROPS);
    set(&mut p, "SmartUpdated", Value::U64(0));
    assert_eq!(nvme(&p, &props(NVME_ATTRS)), None);
}

#[test]
fn nvme_without_attributes_keeps_the_properties() {
    // SmartGetAttributes failed: the properties still say something.
    let h = nvme(&props(NVME_PROPS), &Props::new()).unwrap();
    assert_eq!(h.power_on_hours, Some(16595));
    assert!(h.temperature.is_some());
    assert_eq!(h.wear, None);
    assert_eq!(h.spare, None);
    assert!(!h.spare_low);
}

#[test]
fn nvme_warnings_split_failure_from_wear_and_heat() {
    let mut p = props(NVME_PROPS);
    let warn = |p: &mut Props, w: &[&str]| {
        let w: Vec<String> = w.iter().map(|s| (*s).to_owned()).collect();
        set(p, "SmartCriticalWarning", Value::from(w));
    };

    warn(&mut p, &["spare", "temperature"]);
    let h = nvme(&p, &props(NVME_ATTRS)).unwrap();
    assert!(!h.failing, "low spare and heat are not failure");
    assert!(h.spare_low);
    assert_eq!(h.warnings, [Warning::SpareLow, Warning::Temperature]);

    for bit in [
        "degraded",
        "readonly",
        "volatile_mem",
        "pmr_readonly",
        "new_bit",
    ] {
        warn(&mut p, &[bit]);
        assert!(nvme(&p, &props(NVME_ATTRS)).unwrap().failing, "{bit}");
    }
    warn(&mut p, &["new_bit"]);
    assert_eq!(
        nvme(&p, &props(NVME_ATTRS)).unwrap().warnings,
        [Warning::Other("new_bit".into())]
    );
}

#[test]
fn nvme_spare_below_threshold_is_low_without_the_bit() {
    let mut a = props(NVME_ATTRS);
    set(&mut a, "avail_spare", Value::U8(9));
    assert!(nvme(&props(NVME_PROPS), &a).unwrap().spare_low);
    set(&mut a, "avail_spare", Value::U8(10));
    assert!(
        !nvme(&props(NVME_PROPS), &a).unwrap().spare_low,
        "at the threshold is not below it"
    );
}

#[test]
fn nvme_past_its_rating_is_100() {
    let mut a = props(NVME_ATTRS);
    set(&mut a, "percent_used", Value::U8(255));
    assert_eq!(nvme(&props(NVME_PROPS), &a).unwrap().wear, Some(100));
}

#[test]
fn nvme_unknown_or_misread_temperature_is_none() {
    let mut p = props(NVME_PROPS);
    for kelvin in [0, 0xFFFF] {
        set(&mut p, "SmartTemperature", Value::U16(kelvin));
        assert_eq!(nvme(&p, &props(NVME_ATTRS)).unwrap().temperature, None);
    }
}

#[test]
fn ata_ssd() {
    let h = ata(&props(ATA_PROPS), &ata_attrs(ATA_ATTRS), true).unwrap();
    assert_eq!(h.kind, Kind::Ata);
    assert!(!h.failing);
    assert_eq!(h.wear, Some(3), "wear-leveling-count 97");
    assert!((h.temperature.unwrap() - 34.0).abs() < 1e-9);
    assert_eq!(h.power_on_hours, Some(3500));
    assert_eq!(h.power_cycles, Some(812));
    assert_eq!(h.bad_sectors, Some(0));
    assert_eq!(h.failing_attributes, Some(0));
    assert_eq!(h.spare, None);
    assert_eq!(
        h.written_bytes, None,
        "attribute-241's units are the vendor's"
    );
}

#[test]
fn ata_hard_disk_has_no_wear() {
    let h = ata(&props(ATA_PROPS), &ata_attrs(ATA_ATTRS), false).unwrap();
    assert_eq!(h.wear, None);
}

#[test]
fn ata_wear_needs_the_trusted_name_and_a_percentage() {
    let mut attrs = ata_attrs(ATA_ATTRS);
    let row = attrs
        .iter_mut()
        .find(|a| a.name == "wear-leveling-count")
        .unwrap();
    row.value = 200; // some vendors start counting at 200
    assert_eq!(ata(&props(ATA_PROPS), &attrs, true).unwrap().wear, None);

    let row = attrs.iter_mut().find(|a| a.value == 200).unwrap();
    row.value = -1;
    assert_eq!(ata(&props(ATA_PROPS), &attrs, true).unwrap().wear, None);

    // The ID without libblockdev's name is not trusted.
    let row = attrs.iter_mut().find(|a| a.value == -1).unwrap();
    row.name = "attribute-177".into();
    row.value = 50;
    assert_eq!(ata(&props(ATA_PROPS), &attrs, true).unwrap().wear, None);
}

#[test]
fn ata_failing_and_unknowns() {
    let mut p = props(ATA_PROPS);
    set(&mut p, "SmartFailing", Value::Bool(true));
    set(&mut p, "SmartNumAttributesFailing", Value::I32(-1));
    set(&mut p, "SmartTemperature", Value::F64(0.0));
    set(&mut p, "SmartPowerOnSeconds", Value::U64(0));
    let h = ata(&p, &[], true).unwrap();
    assert!(h.failing);
    assert_eq!(h.bad_sectors, None);
    assert_eq!(h.failing_attributes, None);
    assert_eq!(h.temperature, None);
    assert_eq!(h.power_on_hours, None);
    assert_eq!(h.wear, None);
    assert_eq!(h.power_cycles, None);
}

#[test]
fn ata_failing_stands_without_a_failing_attribute() {
    // A missed failure is worse than a false alarm: the verdict is kept,
    // and failing_attributes says no attribute agrees.
    let mut p = props(ATA_PROPS);
    set(&mut p, "SmartFailing", Value::Bool(true));
    let h = ata(&p, &ata_attrs(ATA_ATTRS), true).unwrap();
    assert!(h.failing);
    assert_eq!(h.failing_attributes, Some(0));
}

#[test]
fn ata_bad_sectors_come_from_the_attributes() {
    let mut attrs = ata_attrs(ATA_ATTRS);
    attrs.push(AtaAttribute {
        name: "current-pending-sector".into(),
        value: 100,
        pretty: 8,
        pretty_unit: 3,
    });
    attrs
        .iter_mut()
        .find(|a| a.name == "reallocated-sector-count")
        .unwrap()
        .pretty = 16;
    assert_eq!(
        ata(&props(ATA_PROPS), &attrs, true).unwrap().bad_sectors,
        Some(24)
    );

    // udisks2 says 0 for a drive with neither attribute; that is unknown.
    attrs.retain(|a| {
        !["reallocated-sector-count", "current-pending-sector"].contains(&a.name.as_str())
    });
    assert_eq!(
        ata(&props(ATA_PROPS), &attrs, true).unwrap().bad_sectors,
        None
    );
}

#[test]
fn ata_without_smart_is_nothing() {
    for (key, v) in [
        ("SmartSupported", Value::Bool(false)),
        ("SmartEnabled", Value::Bool(false)),
        ("SmartUpdated", Value::U64(0)),
    ] {
        let mut p = props(ATA_PROPS);
        set(&mut p, key, v);
        assert_eq!(ata(&p, &ata_attrs(ATA_ATTRS), true), None, "{key}");
    }
    assert_eq!(ata(&Props::new(), &[], true), None);
}

#[test]
fn integers_of_any_width() {
    for v in [
        Value::U8(7),
        Value::U16(7),
        Value::U32(7),
        Value::U64(7),
        Value::I16(7),
        Value::I32(7),
        Value::I64(7),
        Value::Value(Box::new(Value::U16(7))),
    ] {
        let v = OwnedValue::try_from(v).unwrap();
        assert_eq!(uint(&v), Some(7));
        assert_eq!(int(&v), Some(7));
    }
    let minus = OwnedValue::try_from(Value::I64(-1)).unwrap();
    assert_eq!(uint(&minus), None);
    assert_eq!(int(&minus), Some(-1));
    let text = OwnedValue::try_from(Value::from("seven")).unwrap();
    assert_eq!(uint(&text), None);
}

#[test]
fn block_paths_escape_like_udisks2() {
    let p = |d| block_path(d).unwrap();
    assert_eq!(
        p("nvme0n1"),
        "/org/freedesktop/UDisks2/block_devices/nvme0n1"
    );
    assert_eq!(p("/dev/sda"), "/org/freedesktop/UDisks2/block_devices/sda");
    assert_eq!(p("dm-0"), "/org/freedesktop/UDisks2/block_devices/dm_2d0");
    assert_eq!(p("md_x"), "/org/freedesktop/UDisks2/block_devices/md_x");
    assert_eq!(
        p("cciss!c0d0"),
        "/org/freedesktop/UDisks2/block_devices/c0d0"
    );
    assert_eq!(block_path(""), None);
    assert_eq!(block_path("sdé"), None);
    assert_eq!(block_path("../x"), None);
}

/// Whatever drives this machine has, through the real udisks2. Skips where
/// there is no system bus or udisks2, as in CI and the build container.
#[test]
fn live_drives_are_plausible() {
    let Some(mut reader) = SmartReader::new() else {
        eprintln!("skipped: no udisks2 on this bus");
        return;
    };
    let Ok(devices) = fs::read_dir("/sys/block") else {
        return;
    };
    for d in devices.flatten() {
        let name = d.file_name().to_string_lossy().into_owned();
        let Some(h) = reader.read(&name) else {
            continue;
        };
        assert!(h.updated > 0, "{name}");
        assert!(h.wear.is_none_or(|w| w <= 100), "{name}");
        assert!(h.spare.is_none_or(|s| s <= 100), "{name}");
        if let Some(t) = h.temperature {
            assert!((-40.0..=150.0).contains(&t), "{name}: {t} °C");
        }
        if h.kind == Kind::Nvme {
            assert_eq!(
                h.failing,
                h.warnings.iter().any(Warning::is_failure),
                "{name}"
            );
        }
    }
}
