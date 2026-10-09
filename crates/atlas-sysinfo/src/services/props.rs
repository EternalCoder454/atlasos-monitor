//! Property tests of the services: which names an action accepts, and what
//! becomes of the properties systemd sends (docs/SECURITY.md, "Services").
//! `PROPTEST_CASES=20000 cargo test -- props`.

use std::collections::HashMap;

use proptest::prelude::*;
use zbus::zvariant::{Array, OwnedValue, Value};

use super::*;
use crate::hostile;

proptest! {
    /// A name that is accepted is one unit of a kind that can be switched:
    /// the characters of a unit name, no path, within 255 bytes.
    #[test]
    fn accepted_names_are_unit_names(name in hostile::string_to(60), kind in proptest::sample::select(vec![".service", ".socket", ".timer", ".path", ".mount", ".automount", ".swap", ".target", ".slice", ".device", ".scope", ""])) {
        let name = format!("{name}{kind}");
        if valid_name(&name) {
            prop_assert!(name.len() <= 255);
            prop_assert!(!name.contains('/') && !name.contains('\0') && !name.contains(char::is_whitespace));
            prop_assert!(KINDS.iter().any(|k| name.ends_with(&format!(".{k}"))), "{name:?}");
            let stem = name.rsplit_once('.').unwrap().0;
            prop_assert!(!stem.is_empty() && !stem.starts_with('.'));
            prop_assert!(stem.bytes().all(|b| b.is_ascii_alphanumeric() || b":-_.\\@".contains(&b)));
        }
    }

    /// Every other name is refused before a bus is asked, for every action,
    /// and a target is never started, stopped or restarted.
    #[test]
    fn other_names_are_refused_before_the_bus(name in hostile::string_to(80)) {
        for action in [Action::Start, Action::Stop, Action::Restart, Action::Enable, Action::Disable] {
            if !valid_name(&name) {
                prop_assert_eq!(act(&name, action), Err(ActionError::InvalidName));
            }
        }
        let target = format!("{}.target", name.replace(['/', '.'], ""));
        for action in [Action::Start, Action::Stop, Action::Restart] {
            prop_assert_eq!(act(&target, action), Err(ActionError::InvalidName));
        }
    }

    /// systemd's states are words, whatever it says.
    #[test]
    fn states_parse_anything(s in hostile::string()) {
        let _ = LoadState::parse(&s);
        let _ = ActiveState::parse(&s);
        let _ = FileState::parse(&s);
        let _ = Status::of(&ActiveState::parse(&s), &s);
    }

    /// A description is one clean line within the cap.
    #[test]
    fn descriptions_are_clean(text in hostile::string()) {
        let text = format!("[Unit]\nDescription={text}\n");
        if let Some(d) = parse_description(&text) {
            prop_assert!(hostile::is_clean_line(&d) && !d.is_empty() && d.chars().count() <= crate::text::LINE_MAX);
        }
    }

    /// An error reply is a clean line, whatever its message.
    #[test]
    fn error_replies_are_clean(name in hostile::string_to(40), message in hostile::string()) {
        if let ActionError::Refused(m) = error_from_reply(&name, Some(&message)) {
            prop_assert!(hostile::is_clean_line(&m));
        }
    }
}

/// A property value of any of the types a reply might hold, mostly the wrong
/// ones.
fn value() -> impl Strategy<Value = OwnedValue> {
    let any_value = prop_oneof![
        hostile::string_to(300).prop_map(Value::from),
        any::<u32>().prop_map(Value::from),
        any::<u64>().prop_map(Value::from),
        any::<i32>().prop_map(Value::from),
        any::<i64>().prop_map(Value::from),
        any::<bool>().prop_map(Value::from),
        any::<f64>().prop_map(Value::from),
        any::<u8>().prop_map(Value::from),
        proptest::collection::vec(hostile::string_to(40), 0..200)
            .prop_map(|v| Value::Array(Array::from(v))),
        proptest::collection::vec(any::<u32>(), 0..8).prop_map(|v| Value::Array(Array::from(v))),
        hostile::string_to(20).prop_map(|s| Value::Value(Box::new(Value::from(s)))),
    ];
    any_value.prop_map(|v| OwnedValue::try_from(v).expect("no file descriptors"))
}

fn props() -> impl Strategy<Value = HashMap<String, OwnedValue>> {
    let keys = proptest::sample::select(vec![
        "Id",
        "Description",
        "ActiveState",
        "SubState",
        "LoadState",
        "UnitFileState",
        "UnitFilePreset",
        "FragmentPath",
        "Documentation",
        "StateChangeTimestamp",
        "MainPID",
        "TasksCurrent",
        "MemoryCurrent",
        "CPUUsageNSec",
        "Result",
        "ExecMainCode",
        "ExecMainStatus",
        "NRestarts",
        "CanStart",
        "CanStop",
        "TriggeredBy",
        "Other",
    ]);
    proptest::collection::hash_map(keys.prop_map(str::to_owned), value(), 0..22)
}

proptest! {
    /// Properties of the wrong type, huge, or hostile give a Details with
    /// clean text and bounded lists, and never a panic.
    #[test]
    fn details_survive_any_reply(unit in props(), service in props(), name in "[a-z]{1,10}\\.service") {
        let d = parse_details(&name, &unit, &service, None);
        for text in [&d.service.name, &d.service.description, &d.service.sub] {
            prop_assert!(hostile::is_clean_line(text), "{text:?}");
        }
        for text in d.path.iter().chain(&d.preset).chain(&d.documentation).chain(&d.triggered_by) {
            prop_assert!(hostile::is_clean_line(text), "{text:?}");
        }
        prop_assert!(d.documentation.len() <= 64 && d.triggered_by.len() <= 64);
        prop_assert!(d.service.description.chars().count() <= crate::text::LINE_MAX);
    }
}
