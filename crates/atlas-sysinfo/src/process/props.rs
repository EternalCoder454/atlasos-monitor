//! Property tests of the `/proc` parsers and of what a process can make of its
//! own name (docs/SECURITY.md, "Text from outside" and "Process actions").
//! `PROPTEST_CASES=20000 cargo test -- props` runs them with CI's count.

use proptest::prelude::*;

use super::parse::{
    COMM_MAX, container_from_cgroup, full_name, parse_io, parse_stat, parse_statm_resident,
    unit_from_cgroup,
};
use super::{Action, ActionError, act, reuse};
use crate::hostile;

/// A `/proc/<pid>/stat` line for the fields the parser reads, `comm` as given.
fn stat_line(comm: &[u8], state: u8, fields: &[u64; 22], nice: i32) -> Vec<u8> {
    let mut line = b"4242 (".to_vec();
    line.extend_from_slice(comm);
    line.extend_from_slice(b") ");
    line.push(state);
    for (i, f) in fields.iter().enumerate().skip(1) {
        line.push(b' ');
        // Field 19 (index 16) is the nice value.
        if i == 16 {
            line.extend_from_slice(nice.to_string().as_bytes());
        } else {
            line.extend_from_slice(f.to_string().as_bytes());
        }
    }
    line.push(b'\n');
    line
}

fn comm() -> impl Strategy<Value = Vec<u8>> {
    // Any byte but NUL, brackets and spaces and newlines included.
    proptest::collection::vec(1u8..=255, 0..=COMM_MAX)
}

proptest! {
    /// The line is split at the *last* bracket: a name with brackets,
    /// spaces, newlines or bytes that are not UTF-8 comes back whole, and the
    /// numbers after it are read from the right places.
    #[test]
    fn stat_round_trips_whatever_the_name(
        comm in comm(),
        state in proptest::sample::select(b"RSDZTtIXx".to_vec()),
        mut fields in proptest::array::uniform22(0u64..u64::MAX / 4),
        nice in -20i32..20,
        ppid in 0u32..u32::MAX,
        kernel in any::<bool>(),
    ) {
        fields[1] = u64::from(ppid);
        fields[6] = if kernel { 0x0020_0000 } else { 0 };
        let line = stat_line(&comm, state, &fields, nice);
        let s = parse_stat(&line).expect("a well-formed line");
        prop_assert_eq!(s.name, &comm[..]);
        prop_assert_eq!(s.state, state);
        prop_assert_eq!(s.ppid, ppid);
        prop_assert_eq!(s.kernel, kernel);
        prop_assert_eq!(s.jiffies, fields[11] + fields[12]);
        prop_assert_eq!(s.faults, fields[7] + fields[9]);
        prop_assert_eq!(s.nice, nice);
        prop_assert_eq!(s.start_time, fields[19]);
        prop_assert_eq!(s.rss, fields[21]);
        // Cut anywhere, it parses or it is refused: it does not panic.
        for end in (0..line.len()).step_by(3) {
            let _ = parse_stat(&line[..end]);
        }
    }

    /// Any bytes at all: no panic, and what is read is within the input.
    #[test]
    fn stat_of_anything_never_panics(b in hostile::bytes()) {
        if let Some(s) = parse_stat(&b) {
            prop_assert!(s.name.len() <= b.len());
        }
        let _ = parse_io(&b);
        let _ = parse_statm_resident(&b);
    }

    /// Numbers that do not fit are a broken line, not a wrapped value: a
    /// field the parser reads that overflows refuses the whole line; one it
    /// does not read is none of its business.
    #[test]
    fn stat_numbers_that_overflow_are_refused(digits in 20usize..60, at in 1usize..22) {
        const READ: [usize; 9] = [1, 6, 7, 9, 11, 12, 16, 19, 21];
        let line = stat_line(b"x", b'S', &[1; 22], 0);
        let text = String::from_utf8(line).unwrap();
        let (head, tail) = text.split_once(") ").unwrap();
        let mut parts: Vec<String> = tail.trim_end().split(' ').map(str::to_owned).collect();
        parts[at] = "9".repeat(digits);
        let line = format!("{head}) {}\n", parts.join(" "));
        prop_assert_eq!(parse_stat(line.as_bytes()).is_some(), !READ.contains(&at), "field {}", at);
    }

    /// The unit is a path component that ends .scope or .service, and a
    /// container is 64 lower-case hex digits; both lie inside the input.
    #[test]
    fn cgroup_files_give_units_and_containers_or_nothing(b in hostile::bytes()) {
        if let Some(u) = unit_from_cgroup(&b) {
            prop_assert!(u.ends_with(b".scope") || u.ends_with(b".service"));
            prop_assert!(!u.contains(&b'/'));
        }
        if let Some(c) = container_from_cgroup(&b) {
            prop_assert_eq!(c.len(), 64);
            prop_assert!(c.iter().all(|c| matches!(c, b'0'..=b'9' | b'a'..=b'f')));
        }
    }

    /// A cut name is completed from the command line, or left: the answer is
    /// a longer name that starts with the cut one, from the file name of the
    /// first argument.
    #[test]
    fn full_names_continue_the_comm(comm in proptest::collection::vec(1u8..=255, 0..24), cmd in hostile::bytes()) {
        if let Some(n) = full_name(&comm, &cmd) {
            prop_assert!(comm.len() >= COMM_MAX);
            prop_assert!(n.len() > comm.len() && n.starts_with(&comm));
        }
    }

    /// Whatever a process calls itself, the name that is kept is one clean
    /// line within the cap, and the same name keeps the same allocation.
    #[test]
    fn names_are_cleaned_where_they_are_read(raw in hostile::bytes()) {
        let name = reuse(None, &raw);
        prop_assert!(hostile::is_clean_line(&name), "{name:?}");
        prop_assert!(name.chars().count() <= crate::text::NAME_MAX);
        let again = reuse(Some(&name), &raw);
        prop_assert!(std::sync::Arc::ptr_eq(&name, &again));
    }

    /// A pid that would reach a group or everyone (0, a negative one as a
    /// `u32`) or PID 1 is refused before any signal is sent, and a pid that
    /// is not the process the start time names is left alone. The start time
    /// `u64::MAX` is no process's, and `Continue` is the harmless signal, so
    /// this cannot hurt a real process if it were wrong.
    #[test]
    fn no_pid_is_signalled_without_its_start_time(pid in prop_oneof![any::<u32>(), 0u32..3, (i32::MAX as u32 - 2)..=u32::MAX, 0u32..100_000]) {
        let r = act(pid, u64::MAX, Action::Continue);
        prop_assert!(r.is_err());
        if pid <= 1 || pid > i32::MAX as u32 {
            prop_assert!(
                matches!(r, Err(ActionError::Gone) | Err(ActionError::NotAllowed)),
                "{pid}"
            );
        }
    }
}
