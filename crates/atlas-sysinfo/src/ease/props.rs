//! Property tests of what Energy Saver reads and writes: its state file, the
//! unit names it may set a weight on, PipeWire's dump (docs/SECURITY.md,
//! "Energy Saver"). `PROPTEST_CASES=20000 cargo test -- props`.

use proptest::prelude::*;

use super::audio::{find_program, parse_pw_dump};
use super::system::{app_unit, format_state, parse_state, parse_usage, unit_of_cgroup};
use super::{Saved, UNSET};
use crate::hostile;

fn saved() -> impl Strategy<Value = Saved> {
    (
        "app-[A-Za-z0-9:_.\\\\@-]{1,60}\\.(scope|service)",
        prop_oneof![Just(UNSET), 1u64..=10_000],
        any::<bool>(),
        any::<u64>(),
    )
        .prop_map(|(unit, prev, manual, since)| Saved {
            unit,
            prev,
            manual,
            since,
        })
}

proptest! {
    /// A state file is whatever is in the runtime directory: every line that
    /// is taken names an application unit and a weight systemd takes, and
    /// nothing else is.
    #[test]
    fn state_files_give_application_units_only(text in hostile::string()) {
        for s in parse_state(&text) {
            prop_assert!(app_unit(&s.unit));
            prop_assert!(s.prev == UNSET || (1..=10_000).contains(&s.prev));
        }
    }

    /// What is written is read back.
    #[test]
    fn state_files_round_trip(list in proptest::collection::vec(saved(), 0..12)) {
        prop_assert_eq!(parse_state(&format_state(&list)), list);
    }

    /// Lines with a unit that is not an application's are dropped, whatever
    /// else they say.
    #[test]
    fn other_units_are_never_taken(unit in hostile::string_to(40), prev in 1u64..10_000, since in any::<u64>()) {
        let parsed = parse_state(&format!("manual {prev} {since} {unit}\nauto {prev} {since} {unit}\n"));
        for s in parsed {
            prop_assert!(s.unit.starts_with("app-") && app_unit(&s.unit));
        }
    }

    /// A unit that may be given a weight is an application's scope or
    /// service in the characters of a unit name: no path, no control
    /// character, no more than 256 bytes.
    #[test]
    fn weights_are_set_on_application_units_only(unit in hostile::string_to(80), tail in proptest::sample::select(vec![".scope", ".service", ".slice", ".mount", ""])) {
        let unit = format!("app-{unit}{tail}");
        if app_unit(&unit) {
            prop_assert!(unit.len() <= 256 && unit.starts_with("app-"));
            prop_assert!(unit.ends_with(".scope") || unit.ends_with(".service"));
            prop_assert!(unit.bytes().all(|b| b.is_ascii_alphanumeric() || b":-_.\\@".contains(&b)));
        }
    }

    /// The unit found in a cgroup file is one that may be given a weight.
    #[test]
    fn cgroup_units_are_application_units(text in hostile::string(), unit in hostile::string_to(40)) {
        for t in [text, format!("0::/user.slice/app.slice/{unit}\n")] {
            if let Some(u) = unit_of_cgroup(&t) {
                prop_assert!(app_unit(&u));
            }
        }
    }

    /// PipeWire's dump is JSON from a program any client can feed: anything
    /// is parsed or refused.
    #[test]
    fn pipewire_dumps_never_panic(b in hostile::bytes(), depth in 0usize..400) {
        let _ = parse_pw_dump(&b);
        let _ = parse_usage(&b);
        let nested = format!("{}{}", "[".repeat(depth), "]".repeat(depth));
        let _ = parse_pw_dump(nested.as_bytes());
    }

    /// A program is looked up by a plain file name only.
    #[test]
    fn programs_are_found_by_file_name_only(name in hostile::string_to(30)) {
        if name.contains('/') || name.starts_with('.') || name.is_empty() {
            prop_assert_eq!(find_program(&name), None);
        }
    }
}
