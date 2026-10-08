//! The fwupd fixture is `GetHostSecurityAttrs` recorded with `busctl --json` on
//! a Fedora 44 desktop (fwupd 2.1.8, "HSI:0!"), with the per-attribute GUIDs,
//! descriptions and metadata taken out. The other fixtures are an unmodified
//! os-release and the first lines of the files they are named for.

use serde_json::Value as Json;

use super::*;

const OS_RELEASE: &str = include_str!("../../tests/fixtures/about_os_release");
const OS_RELEASE_QUOTED: &str = include_str!("../../tests/fixtures/about_os_release_quoted");
const METAINFO: &str = include_str!("../../tests/fixtures/about_plasma_metainfo.xml");
const CPUINFO_INTEL: &str = include_str!("../../tests/fixtures/cpuinfo");
const CPUINFO_INTEL_OLD: &str = include_str!("../../tests/fixtures/about_cpuinfo_intel_old");
const CPUINFO_AMD: &str = include_str!("../../tests/fixtures/about_cpuinfo_amd");
const CPUINFO_ARM: &str = include_str!("../../tests/fixtures/about_cpuinfo_arm");
const FWUPD_ATTRS: &str = include_str!("../../tests/fixtures/about_fwupd_attrs_json");

/// One `{"type": ..., "data": ...}` variant, for the types fwupd uses.
fn value(v: &Json) -> OwnedValue {
    let d = &v["data"];
    let v = match v["type"].as_str().unwrap() {
        "u" => Value::U32(d.as_u64().unwrap() as u32),
        "t" => Value::U64(d.as_u64().unwrap()),
        "s" => Value::from(d.as_str().unwrap().to_owned()),
        "as" => Value::from(
            d.as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
        ),
        other => panic!("no fixture type {other}"),
    };
    OwnedValue::try_from(v).unwrap()
}

fn attrs() -> Vec<Attr> {
    let reply: Json = serde_json::from_str(FWUPD_ATTRS).unwrap();
    reply["data"][0]
        .as_array()
        .unwrap()
        .iter()
        .map(|row| {
            row.as_object()
                .unwrap()
                .iter()
                .map(|(k, v)| (k.clone(), value(v)))
                .collect()
        })
        .collect()
}

fn attr(pairs: &[(&str, Value<'static>)]) -> Attr {
    pairs
        .iter()
        .map(|(k, v)| ((*k).to_owned(), OwnedValue::try_from(v.clone()).unwrap()))
        .collect()
}

#[test]
fn os_release_of_fedora() {
    let os = parse_os_release(OS_RELEASE);
    assert_eq!(os.name, "Fedora Linux 44 (KDE Plasma Desktop Edition)");
    assert_eq!(os.logo, "fedora-logo-icon");
    assert_eq!(os.url, "https://fedoraproject.org/");
}

#[test]
fn os_release_quoting() {
    let os = parse_os_release(OS_RELEASE_QUOTED);
    assert_eq!(os.name, "Unquoted Name");
    assert_eq!(os.logo, "odd-logo");
    // The later HOME_URL wins.
    assert_eq!(os.url, "https://example.org/override");

    let os = parse_os_release("NAME='Odd \"Linux\"'\nVERSION=\"1.0 \\\"Dusk\\\" \\\\ \\$x\"\n");
    assert_eq!(os.name, "Odd \"Linux\" 1.0 \"Dusk\" \\ $x");
}

#[test]
fn os_release_falls_back_and_validates() {
    assert_eq!(parse_os_release("NAME=Arch Linux\n").name, "Arch");
    assert_eq!(parse_os_release("NAME=\"Arch Linux\"\n").name, "Arch Linux");
    assert_eq!(parse_os_release("").name, "");
    // NAME defaults to "Linux"; a bare value's '#' starts a comment only
    // after white space.
    assert_eq!(parse_os_release("VERSION=1.0\n").name, "Linux 1.0");
    assert_eq!(parse_os_release("NAME=Arch#1\n").name, "Arch#1");
    assert_eq!(parse_os_release("NAME=Arch # the best\n").name, "Arch");
    assert_eq!(parse_os_release("garbage\n\0\n=\n"), OsRelease::default());
    let os = parse_os_release("LOGO=\"a b/../c\"\nHOME_URL=javascript:alert(1)\n");
    assert_eq!((os.logo.as_str(), os.url.as_str()), ("", ""));
    let long = format!("PRETTY_NAME={}\n", "x".repeat(10_000));
    assert_eq!(parse_os_release(&long).name.chars().count(), MAX_TEXT);
}

#[test]
fn plasma_version() {
    assert_eq!(parse_plasma_version(METAINFO).as_deref(), Some("6.6.4"));
    assert_eq!(
        parse_plasma_version("<release date='x' version='5.27.1'/>").as_deref(),
        Some("5.27.1")
    );
    for bad in [
        "",
        "<releases></releases>",
        "<release version=\"\"/>",
        "<release version=\"6.x\"/>",
        "<release version=\"6..4\"/>",
        "<release version=\"6.4.\"/>",
        "<release version=\"6.4",
        "<release version=6.4/>",
        "<release",
    ] {
        assert_eq!(parse_plasma_version(bad), None, "{bad:?}");
    }
}

#[test]
fn cpu_models() {
    assert_eq!(
        parse_cpu_model(CPUINFO_INTEL).as_deref(),
        Some("Intel Core i9-14900KF")
    );
    assert_eq!(
        parse_cpu_model(CPUINFO_INTEL_OLD).as_deref(),
        Some("Intel Core i7-8700")
    );
    assert_eq!(
        parse_cpu_model(CPUINFO_AMD).as_deref(),
        Some("AMD Ryzen 9 7950X")
    );
    // No "model name" on ARM: the SoC.
    assert_eq!(parse_cpu_model(CPUINFO_ARM).as_deref(), Some("BCM2835"));
    assert_eq!(
        parse_cpu_model("Model\t: Board X\n").as_deref(),
        Some("Board X")
    );
    for none in [
        "",
        "processor : 0\n",
        "model name\t:\n",
        "model name : @ 3GHz\n",
    ] {
        assert_eq!(parse_cpu_model(none), None, "{none:?}");
    }
}

#[test]
fn cpu_counts() {
    for (list, want) in [
        ("0-31\n", 32),
        ("0\n", 1),
        ("0-7,9,12-15", 13),
        ("0-3, 5", 5),
        ("", 0),
        ("\n", 0),
        ("3-1", 0),
        ("a-b", 0),
        ("0-99999999999999999999", 0),
        ("0-70000", 0),
        ("0-3,", 0),
    ] {
        assert_eq!(parse_cpu_count(list), want, "{list:?}");
    }
}

#[test]
fn dmi_placeholders_are_none() {
    for p in [
        "To Be Filled By O.E.M.",
        "To be filled by O.E.M.",
        "Default string",
        "System Product Name",
        "System manufacturer",
        "System Version",
        "Not Applicable",
        "Not Specified",
        "None",
        "Default",
        "OEM",
        "O.E.M.",
        "0123456789",
        "x.x",
        "Type1ProductConfigId",
        "",
        "  \t\n",
        "\0",
        "DEFAULT STRING\n",
        "To Be Filled By O.E.M",
        "To be filled by O.E.M. ",
        "Default Company Name",
        "System Serial Number",
        "Not Available",
        "Not Defined",
        "Unknown",
        "undefined",
        "*",
        "-",
        "---",
        "...",
    ] {
        assert_eq!(tidy_dmi(p), None, "{p:?}");
    }
    assert_eq!(tidy_dmi(" ASRock \n").as_deref(), Some("ASRock"));
    assert_eq!(
        tidy_dmi("Z790   Lightning\tWiFi").as_deref(),
        Some("Z790 Lightning WiFi")
    );
    assert_eq!(
        tidy_dmi(&"y".repeat(1000)).unwrap().chars().count(),
        MAX_TEXT
    );
}

#[test]
fn chassis_words() {
    for (code, want) in [
        (3, "Desktop"),
        (7, "Desktop"),
        (9, "Laptop"),
        (10, "Laptop"),
        (13, "All-in-One"),
        (17, "Server"),
        (23, "Server"),
        (30, "Tablet"),
        (31, "Convertible"),
        (35, "Mini PC"),
        (1, ""),
        (2, ""),
        (0, ""),
        (999, ""),
        (u32::MAX, ""),
    ] {
        assert_eq!(chassis_name(code), want, "{code}");
    }
}

#[test]
fn vendors_lose_their_legal_ending() {
    for (raw, want) in [
        (
            "Micro-Star International Co., Ltd.",
            "Micro-Star International",
        ),
        ("ASUSTeK COMPUTER INC.", "ASUSTeK COMPUTER"),
        ("Dell Inc.", "Dell"),
        ("Intel Corporation", "Intel"),
        ("LENOVO", "LENOVO"),
        ("ASRock", "ASRock"),
        ("Inc.", "Inc."),
        // Lowercasing "İ" makes it longer; the ending is still found.
        ("İstanbul Bilgisayar Ltd.", "İstanbul Bilgisayar"),
        ("Ünïcode Gerät Corp", "Ünïcode Gerät"),
        ("Foo, Inc", "Foo"),
        ("Bar Co., Ltd", "Bar Co."),
    ] {
        assert_eq!(short_vendor(raw), want, "{raw}");
    }
}

#[test]
fn product_names() {
    let some = |s: &str| Some(s.to_owned());
    for (name, version, want) in [
        (
            some("21AH00CNUS"),
            some("ThinkPad T14 Gen 3"),
            "ThinkPad T14 Gen 3",
        ),
        (some("XPS 15 9520"), None, "XPS 15 9520"),
        (some("ROG Zephyrus G14"), some("1.0"), "ROG Zephyrus G14"),
        (
            some("Laptop 13"),
            some("Laptop 13 (12th Gen Intel)"),
            "Laptop 13 (12th Gen Intel)",
        ),
        (
            some("ThinkPad T14 Gen 3"),
            some("ThinkPad"),
            "ThinkPad T14 Gen 3",
        ),
        (
            some("Precision"),
            some("Mobile Workstation"),
            "Precision Mobile Workstation",
        ),
        (some("Z790"), some("Rev 1.02"), "Z790"),
        (None, None, ""),
    ] {
        assert_eq!(
            product_name(name.clone(), version.clone()),
            want,
            "{name:?} {version:?}"
        );
    }
}

#[test]
fn boards_and_firmware() {
    let some = |s: &str| Some(s.to_owned());
    assert_eq!(
        board_name(some("ASRock"), some("Z790 Lightning WiFi")),
        "ASRock Z790 Lightning WiFi"
    );
    assert_eq!(
        board_name(some("ASUSTeK COMPUTER INC."), some("ASUSTeK Z690")),
        "ASUSTeK Z690"
    );
    assert_eq!(board_name(None, some("Board")), "Board");
    assert_eq!(board_name(None, None), "");
    // The long maker stays whole and apart: the page shows it on a line of
    // its own, so the version and date are never what gets cut off.
    assert_eq!(
        firmware_parts(
            some("American Megatrends International, LLC."),
            some("11.02"),
            Some("05/05/2025")
        ),
        Firmware {
            vendor: "American Megatrends International, LLC.".to_owned(),
            version: "11.02".to_owned(),
            date: "2025-05-05".to_owned(),
        }
    );
    assert_eq!(
        firmware_parts(None, some("1.2"), None),
        Firmware {
            version: "1.2".to_owned(),
            ..Firmware::default()
        }
    );
    assert_eq!(firmware_parts(None, None, None), Firmware::default());
    assert_eq!(iso_date("13/01/2025"), None);
    assert_eq!(iso_date("3/4/25"), None);
    assert_eq!(iso_date("03/14/2025/9"), None);
    assert_eq!(iso_date("03/14/2025").as_deref(), Some("2025-03-14"));
    // A date that isn't one is kept as written.
    assert_eq!(firmware_parts(None, None, Some("soon")).date, "soon");
}

#[test]
fn hsi_ids() {
    for (id, want) in [
        ("HSI:0! (v2.1.8)", (Some(0), true, "2.1.8")),
        ("HSI:3", (Some(3), false, "")),
        ("HSI:5 (v2.0.1)", (Some(5), false, "2.0.1")),
        ("HSI:6", (None, false, "")),
        ("HSI:", (None, false, "")),
        ("HSI:999999999999", (None, false, "")),
        ("HSI:-1", (None, false, "")),
        ("garbage", (None, false, "")),
        ("", (None, false, "")),
        ("HSI:2 (v", (Some(2), false, "")),
        ("HSI:2 (v<b>)", (Some(2), false, "")),
    ] {
        let (level, issue, version) = parse_hsi(id);
        assert_eq!((level, issue, version.as_str()), want, "{id:?}");
    }
}

#[test]
fn fixture_failing_attributes() {
    let attrs = attrs();
    assert!(attrs.len() > 20, "fixture has {} rows", attrs.len());
    let sec = security_from("HSI:0! (v2.1.8)", &attrs);
    assert!(sec.available);
    assert_eq!(sec.level, Some(0));
    assert!(sec.runtime_issue);
    assert_eq!(sec.version, "2.1.8");
    // Level 1 is next: its attributes that did not pass, in fwupd's order.
    assert_eq!(
        sec.failing,
        ["SPI lock", "SPI BIOS Descriptor", "SPI BIOS region"]
    );

    // One level up adds level 2's, once each ("Platform debugging" is at
    // levels 1 and 2 under one title).
    let sec = security_from("HSI:1", &attrs);
    assert!(sec.failing.len() > 3);
    let mut seen = sec.failing.clone();
    seen.sort();
    seen.dedup();
    assert_eq!(seen.len(), sec.failing.len(), "{:?}", sec.failing);
    assert!(sec.failing.iter().all(|t| !t.is_empty()));
    // Nothing that passed, and nothing above the next level, is listed.
    assert!(!sec.failing.contains(&"TPM v2.0".to_owned()));
    assert!(!sec.failing.contains(&"SMAP".to_owned()));
}

#[test]
fn failing_rules() {
    let row = |level: u32, flags: u64| {
        attr(&[
            ("HsiLevel", Value::U32(level)),
            ("Flags", Value::U64(flags)),
            (
                "Name",
                Value::from("n".to_owned() + &level.to_string() + &flags.to_string()),
            ),
        ])
    };
    let attrs = [
        row(1, 0),
        row(1, 1), // passed
        row(1, 2), // obsoleted
        row(2, 0),
        row(3, 0),                                         // two levels up
        attr(&[("Name", Value::from("info".to_owned()))]), // no level
        attr(&[
            ("HsiLevel", Value::U32(1)),
            ("Summary", Value::from("only a summary".to_owned())),
        ]),
        attr(&[
            ("HsiLevel", Value::U32(1)),
            ("AppstreamId", Value::from("org.fwupd.hsi.X".to_owned())),
            ("Name", Value::from("  ".to_owned())),
        ]),
        attr(&[("HsiLevel", Value::from("one".to_owned()))]),
    ];
    let sec = security_from("HSI:1", &attrs);
    assert_eq!(
        sec.failing,
        ["n10", "n20", "only a summary", "org.fwupd.hsi.X"]
    );
    // No level, no list; no rows, no list.
    assert!(security_from("nonsense", &attrs).failing.is_empty());
    assert!(security_from("HSI:1", &[]).failing.is_empty());
}

#[test]
fn many_attributes_are_capped() {
    let attrs: Vec<Attr> = (0..10_000)
        .map(|i| {
            attr(&[
                ("HsiLevel", Value::U32(1)),
                ("Name", Value::from(format!("attr {i}"))),
            ])
        })
        .collect();
    assert_eq!(security_from("HSI:0", &attrs).failing.len(), MAX_ATTRS);
}

/// The live readers on whatever machine this runs on: a container has no
/// DMI, no Plasma and no fwupd, and every one of those is just empty.
#[test]
fn live_read_never_fails() {
    let about = read();
    assert!(!about.os_name.is_empty());
    if std::path::Path::new("/proc/meminfo").exists() {
        assert!(about.memory > 0);
    }
    if std::path::Path::new("/sys/devices/system/cpu/online").exists() {
        assert!(about.cpu_threads > 0);
    }
    assert!(!about.kernel.is_empty());
    assert!(about.plasma.is_empty() || about.plasma.starts_with(|c: char| c.is_ascii_digit()));
    println!("{about:#?}");

    let sec = security();
    assert!(sec.level.is_none_or(|l| l <= 5));
    if !sec.available {
        assert_eq!(sec, Security::default());
    }
    println!("{sec:#?}");
}
