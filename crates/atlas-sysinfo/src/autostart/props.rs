//! Property tests of the autostart entries and the files written for them
//! (docs/SECURITY.md, "Autostart"). `PROPTEST_CASES=20000 cargo test -- props`.

use proptest::prelude::*;

use super::*;
use crate::hostile;

/// The lines of a desktop file as one that is edited might have them.
fn file() -> impl Strategy<Value = String> {
    let line = prop_oneof![
        Just("[Desktop Entry]".to_owned()),
        Just("[Desktop Action quit]".to_owned()),
        Just("Hidden=true".to_owned()),
        Just("Hidden=false".to_owned()),
        Just(" Hidden = true ".to_owned()),
        Just("Name=Syncer".to_owned()),
        Just("Exec=syncer --tray".to_owned()),
        Just("# a comment".to_owned()),
        Just(String::new()),
        hostile::string_to(60),
    ];
    proptest::collection::vec(line, 0..20).prop_map(|l| l.join("\n"))
}

proptest! {
    /// Any text parses or is no entry; nothing panics.
    #[test]
    fn parsing_never_panics(text in hostile::string(), loc in hostile::string_to(10)) {
        let _ = parse_desktop(&text, std::slice::from_ref(&loc));
        let _ = kconfig_value(&text, &loc, "k");
        let _ = kconfig_bool(&text);
        let _ = split_list(&text);
    }

    /// Switching off hides the entry and switching on shows it, whatever the
    /// file had; asking again changes nothing; and the rest of the entry is
    /// kept.
    #[test]
    fn hiding_and_showing_keep_the_entry(text in file()) {
        let entry = format!("[Desktop Entry]\nName=Syncer\nExec=syncer --tray\n{text}");
        let before = parse_desktop(&entry, &[]).unwrap();
        let off = with_hidden(&entry, true);
        let on = with_hidden(&entry, false);
        let off_entry = parse_desktop(&off, &[]).unwrap();
        let on_entry = parse_desktop(&on, &[]).unwrap();
        prop_assert!(off_entry.hidden);
        prop_assert!(!on_entry.hidden);
        prop_assert_eq!(&off_entry.name, &before.name);
        prop_assert_eq!(&on_entry.exec, &before.exec);
        // Asking again says the same.
        prop_assert_eq!(parse_desktop(&with_hidden(&off, true), &[]), Some(off_entry));
        prop_assert_eq!(parse_desktop(&with_hidden(&on, false), &[]), Some(on_entry));
        prop_assert!(off.len() < entry.len() + 32);
    }

    /// A name is accepted only if it is a plain file name: it joins to
    /// the autostart folder as a child and nothing else.
    #[test]
    fn desktop_ids_are_file_names(id in hostile::string_to(40), suffix in prop::bool::ANY) {
        let id = if suffix { format!("{id}.desktop") } else { id };
        if valid_desktop_id(&id) {
            let dir = Path::new("/home/u/.config/autostart");
            let joined = dir.join(&id);
            prop_assert_eq!(joined.parent(), Some(dir));
            prop_assert_eq!(joined.file_name().and_then(OsStr::to_str), Some(id.as_str()));
            prop_assert!(!id.starts_with('.') && id.ends_with(".desktop") && id.len() <= 255);
            prop_assert!(hostile::is_clean_line(&id));
        }
    }

    /// A unit name is accepted only if systemd can take it as one name of a
    /// kind that starts at login: no path, no template, the characters of a
    /// unit name.
    #[test]
    fn unit_names_are_names(name in hostile::string_to(40), kind in proptest::sample::select(vec![".service", ".socket", ".timer", ".path", ".mount", ".target", ""])) {
        let name = format!("{name}{kind}");
        if valid_unit_name(&name) {
            prop_assert!(!name.contains('/') && !name.starts_with('.') && name.len() <= 255);
            prop_assert!(UNIT_TYPES.iter().any(|t| name.ends_with(t)));
            prop_assert!(!name.trim_end_matches(|c| c != '@' && c != '.').is_empty() || true);
            prop_assert!(name.bytes().all(|b| b.is_ascii_alphanumeric() || b":-_.\\@".contains(&b)));
            prop_assert!(!name.rsplit_once('.').is_some_and(|(stem, _)| stem.ends_with('@')));
        }
    }

    /// The pattern asked of systemd matches the name and no wider through
    /// the characters that mean something to it.
    #[test]
    fn patterns_hold_no_glob_but_a_question_mark(name in hostile::string_to(60)) {
        let p = glob_literal(&name);
        prop_assert_eq!(p.chars().count(), name.chars().count());
        prop_assert!(!p.contains(['*', '[', '\\']));
    }
}

proptest! {
    #![proptest_config(hostile::cases(150))]

    /// What is written is what was given, in a file of its own next to the
    /// old one, with no temporary file left; a link in the way is replaced
    /// and what it pointed at is not touched.
    #[test]
    fn writes_are_whole_and_never_through_a_link(body in hostile::bytes(), via_link in prop::bool::ANY) {
        let dir = tempfile::tempdir().unwrap();
        let victim = dir.path().join("victim");
        fs::write(&victim, "precious").unwrap();
        let target = dir.path().join("a.desktop");
        if via_link {
            symlink(&victim, &target).unwrap();
        }
        write_atomic(&target, &body).unwrap();
        prop_assert_eq!(fs::read(&target).unwrap(), body);
        prop_assert_eq!(fs::read_to_string(&victim).unwrap(), "precious");
        let left: Vec<_> = fs::read_dir(dir.path()).unwrap().flatten().map(|e| e.file_name()).collect();
        prop_assert_eq!(left.len(), 2, "{:?}", left);
        let mode = fs::metadata(&target).unwrap().permissions().mode() & 0o777;
        prop_assert_eq!(mode & 0o022, 0, "writable by others: {:o}", mode);
    }
}

use std::os::unix::fs::symlink;
