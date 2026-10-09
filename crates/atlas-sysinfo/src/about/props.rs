//! Property tests of what System Info reads from files and from fwupd
//! (docs/SECURITY.md, "System information"). `PROPTEST_CASES=20000 cargo test -- props`.

use proptest::prelude::*;
use zbus::zvariant::{OwnedValue, Value};

use super::*;
use crate::hostile;

proptest! {
    /// Cleaning is one line within the cap and settles in one step.
    #[test]
    fn clean_text_is_a_line_within_the_cap(s in hostile::string()) {
        let c = clean(&s);
        prop_assert!(hostile::is_clean_line(&c));
        prop_assert!(c.chars().count() <= MAX_TEXT);
        prop_assert_eq!(clean(&c), c.clone());
        prop_assert_eq!(tidy_dmi(&s).is_some_and(|t| t == c) || tidy_dmi(&s).is_none(), true);
    }

    /// os-release can say anything: the logo is an icon name or nothing, the
    /// address is http(s) or nothing, the name is a clean line.
    #[test]
    fn os_release_gives_safe_fields(text in hostile::string(), logo in hostile::string_to(40), url in hostile::string_to(40)) {
        let text = format!("{text}\nLOGO={logo}\nHOME_URL={url}\nNAME=\"x{logo}\"\nPRETTY_NAME='{url}'\n");
        let r = parse_os_release(&text);
        prop_assert!(hostile::is_clean_line(&r.name));
        prop_assert!(r.logo.is_empty() || (r.logo.len() <= 128 && r.logo.chars().all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c))));
        prop_assert!(r.url.is_empty() || r.url.starts_with("http://") || r.url.starts_with("https://"));
        prop_assert!(hostile::is_clean_line(&r.url));
    }

    /// The other file parsers: no panic, clean lines, bounded counts.
    #[test]
    fn file_parsers_are_total(text in hostile::string()) {
        let _ = parse_plasma_version(&text);
        if let Some(c) = parse_cpu_model(&text) {
            prop_assert!(hostile::is_clean_line(&c));
        }
        prop_assert!(parse_cpu_count(&text) <= 1 << 16);
        if let Some(d) = iso_date(&text) {
            prop_assert_eq!(d.len(), 10);
        }
        let _ = unquote(&text);
        let _ = short_vendor(&text);
    }

    /// A Host Security ID: a level up to 5 or none, a version of a few safe
    /// characters or none.
    #[test]
    fn hsi_ids_are_levels_and_versions(id in hostile::string_to(60), level in 0u32..400, v in hostile::string_to(40)) {
        for id in [id, format!("HSI:{level}! (v{v})"), format!("HSI:{level}(v{v}")] {
            let (l, _, version) = parse_hsi(&id);
            prop_assert!(l.is_none_or(|l| l <= HSI_MAX));
            prop_assert!(version.len() <= 32 && version.chars().all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c)));
        }
    }
}

fn attr_value() -> impl Strategy<Value = OwnedValue> {
    prop_oneof![
        hostile::string_to(200).prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<i32>().prop_map(Value::from),
        any::<u8>().prop_map(Value::from),
        any::<bool>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
    ]
    .prop_map(|v| OwnedValue::try_from(v).expect("no file descriptors"))
}

fn attr() -> impl Strategy<Value = Attr> {
    let keys = proptest::sample::select(vec![
        "HsiLevel",
        "Flags",
        "Name",
        "Summary",
        "AppstreamId",
        "Other",
    ]);
    proptest::collection::hash_map(keys.prop_map(str::to_owned), attr_value(), 0..6)
}

proptest! {
    #![proptest_config(hostile::cases(800))]

    /// fwupd's list of attributes with the wrong types and hostile text: the
    /// page gets clean, bounded titles.
    #[test]
    fn security_from_survives_any_reply(attrs in proptest::collection::vec(attr(), 0..24), id in hostile::string_to(40), level in 0u32..8) {
        for id in [id, format!("HSI:{level}")] {
            let s = security_from(&id, &attrs);
            prop_assert!(s.failing.iter().all(|t| hostile::is_clean_line(t) && t.chars().count() <= MAX_TEXT && !t.is_empty()));
            prop_assert!(s.level.is_none_or(|l| l <= HSI_MAX));
        }
    }
}

/// More rows than fwupd would send: only the first [`MAX_ATTRS`] are looked at.
#[test]
fn a_flood_of_attributes_is_cut() {
    let one = |name: String| -> Attr {
        [
            ("HsiLevel".to_owned(), OwnedValue::from(1u64)),
            ("Flags".to_owned(), OwnedValue::from(0u64)),
            (
                "Name".to_owned(),
                OwnedValue::try_from(Value::from(name)).unwrap(),
            ),
        ]
        .into_iter()
        .collect()
    };
    let attrs: Vec<Attr> = (0..MAX_ATTRS * 4)
        .map(|i| one(format!("check {i}")))
        .collect();
    let s = security_from("HSI:1", &attrs);
    assert_eq!(s.failing.len(), MAX_ATTRS);
    // Titles are cut too.
    let long = one("t".repeat(100_000));
    assert_eq!(
        security_from("HSI:1", &[long]).failing[0].chars().count(),
        MAX_TEXT
    );
}

proptest! {
    #![proptest_config(hostile::cases(200))]

    /// A megabyte of /proc/cpuinfo or os-release is read in time and cut.
    #[test]
    fn huge_files_are_cut(text in hostile::huge()) {
        prop_assert!(clean(&text).chars().count() <= MAX_TEXT);
        prop_assert!(parse_os_release(&text).name.chars().count() <= MAX_TEXT * 2 + 1);
        if let Some(c) = parse_cpu_model(&text) {
            prop_assert!(c.chars().count() <= MAX_TEXT);
        }
        let _ = parse_plasma_version(&text);
        prop_assert!(parse_cpu_count(&text) <= 1 << 16);
    }
}
