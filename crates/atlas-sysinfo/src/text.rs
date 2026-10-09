//! Text from outside, made fit to show.
//!
//! A process names itself (`comm`, `argv`), a cgroup directory is named by
//! whoever made it, and desktop files, unit files and container lists are
//! written by programs other than ours. None of it is trusted, and all of it
//! ends up in a table, a dialog or the clipboard. So it is cleaned where it is
//! read (docs/SECURITY.md, "Text from outside"): no control characters (a line
//! break would add a row to the Details panel, an escape sequence would reach a
//! terminal the text is pasted into), no bidirectional or other invisible
//! format characters (they make a name read as another), and a length cap, so
//! one process cannot make a megabyte string live in every row.
//!
//! This is the only place that rule is written down; the readers call it.

/// The longest name, unit or title shown, in characters.
pub const NAME_MAX: usize = 256;

/// The longest line of free text (a description, a comment, an `Exec`).
pub const LINE_MAX: usize = 1024;

/// The longest command line shown, in characters, and the most arguments.
pub const COMMAND_MAX: usize = 16 * 1024;
pub const ARGS_MAX: usize = 1024;

/// `s` as one line of at most `max` characters: control characters and
/// white space become one space between words, invisible format characters
/// (bidirectional overrides, zero-width marks, tags) are dropped, and
/// the ends are trimmed. Anything longer than `max` is cut (no ellipsis: a
/// name is a name).
pub fn plain(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max.saturating_mul(4)).min(1024));
    let mut count = 0;
    let mut space = false;
    for c in s.chars() {
        if c.is_whitespace() || c.is_control() {
            space = !out.is_empty();
        } else if crate::invisible(c) {
            continue;
        } else {
            if space {
                if count + 1 >= max {
                    break;
                }
                out.push(' ');
                count += 1;
                space = false;
            }
            if count >= max {
                break;
            }
            out.push(c);
            count += 1;
        }
    }
    out
}

/// `s` with its spaces as they are, for a path or a command line, where
/// the exact text matters: every control or invisible character becomes
/// U+FFFD, so it shows that something was there, and nothing is dropped
/// silently. Cut to `max` characters, with `…` where it was cut.
pub fn literal(s: &str, max: usize) -> String {
    let mut out = String::with_capacity(s.len().min(max.saturating_mul(4)).min(4096));
    for (count, c) in s.chars().enumerate() {
        if count >= max {
            out.push('…');
            break;
        }
        if crate::unprintable(c) {
            out.push('\u{FFFD}');
        } else {
            out.push(c);
        }
    }
    out
}

/// A command line's arguments, each cleaned by [`literal`], at most
/// [`ARGS_MAX`] of them and [`COMMAND_MAX`] characters in all; what does not
/// fit is replaced by one final `…`.
pub fn command_line<'a>(args: impl IntoIterator<Item = &'a str>) -> Vec<String> {
    let mut out = Vec::new();
    let mut budget = COMMAND_MAX;
    for arg in args {
        if out.len() == ARGS_MAX || budget == 0 {
            out.push("…".to_owned());
            break;
        }
        let a = literal(arg, budget);
        budget = budget.saturating_sub(a.chars().count() + 1);
        out.push(a);
    }
    out
}

/// An icon name or the path of an icon file, as a desktop file writes it, or
/// "" if it is neither. An icon is looked up in the theme by `Kirigami.Icon`,
/// which also loads whatever URL it is given (`https://...` from the network,
/// `file:`, `qrc:`), so the string is not passed on unless it is a plain name
/// (letters, digits and `-_.+`, not a URL) or an absolute path to a file with an
/// image extension, with no control characters and no `..`.
pub fn icon(s: &str) -> String {
    const EXTENSIONS: [&str; 4] = ["svg", "png", "svgz", "xpm"];
    let s = s.trim();
    if s.is_empty() || s.len() > 4096 {
        return String::new();
    }
    let name = s.len() <= 128
        && !s.starts_with('.')
        && s.bytes()
            .all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b));
    if name {
        return s.to_owned();
    }
    let path = s.starts_with('/')
        && !s.starts_with("//")
        && !s.chars().any(crate::unprintable)
        && !s.contains(['?', '#', '%'])
        && !s.split('/').any(|p| p == "..")
        && s.rsplit_once('.')
            .is_some_and(|(_, ext)| EXTENSIONS.contains(&ext.to_ascii_lowercase().as_str()));
    if path { s.to_owned() } else { String::new() }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn plain_is_one_clean_line() {
        assert_eq!(plain("  a\tb\nc\u{7f} ", 64), "a b c");
        assert_eq!(plain("Evil\u{202E}draobyeK", 64), "EvildraobyeK");
        assert_eq!(plain("a\u{200B}b\u{2066}c\u{E0041}", 64), "abc");
        assert_eq!(plain("\u{1b}[31mred\u{1b}[0m", 64), "[31mred [0m");
        assert_eq!(plain("", 64), "");
        assert_eq!(plain("\n\n", 64), "");
        assert_eq!(plain("<b>x</b>", 64), "<b>x</b>");
        assert_eq!(plain(&"é".repeat(300), 10).chars().count(), 10);
        assert_eq!(plain("ab cd", 4), "ab c");
        assert_eq!(plain("ab cd", 3), "ab");
    }

    #[test]
    fn literal_keeps_the_text_and_marks_the_rest() {
        assert_eq!(literal("a  b", 64), "a  b");
        assert_eq!(
            literal("a\nb\u{1b}c\u{202E}d\0", 64),
            "a\u{FFFD}b\u{FFFD}c\u{FFFD}d\u{FFFD}"
        );
        assert_eq!(literal("abcdef", 3), "abc…");
        assert_eq!(literal("abc", 3), "abc");
    }

    #[test]
    fn command_lines_are_bounded() {
        let args = command_line(["a", "b\nc"]);
        assert_eq!(args, ["a", "b\u{FFFD}c"]);
        let many = vec!["x"; ARGS_MAX + 50];
        let cut = command_line(many.iter().copied());
        assert_eq!(cut.len(), ARGS_MAX + 1);
        assert_eq!(cut.last().unwrap(), "…");
        let long = "y".repeat(COMMAND_MAX * 3);
        let cut = command_line([long.as_str(), "z"]);
        assert!(cut.iter().map(|a| a.chars().count()).sum::<usize>() <= COMMAND_MAX + 8);
    }

    #[test]
    fn icons_are_names_or_image_files_only() {
        for ok in [
            "firefox",
            "org.kde.konsole",
            "utilities-terminal",
            "/usr/share/pixmaps/x.png",
            "/opt/My App/icon.SVG",
        ] {
            assert_eq!(icon(ok), ok, "{ok}");
        }
        for bad in [
            "",
            "https://example.org/x.png",
            "http://127.0.0.1/x",
            "file:///etc/passwd",
            "qrc:/x",
            "image://theme/x",
            "//host/share/x.png",
            "../x",
            ".hidden",
            "/usr/../etc/x.png",
            "/etc/passwd",
            "/tmp/x\n.png",
            "/tmp/x\u{202E}.png",
            "/tmp/x.png?y",
            "a b",
            "a/b",
        ] {
            assert_eq!(icon(bad), "", "{bad:?}");
        }
        assert_eq!(icon(&"a".repeat(129)), "");
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use crate::hostile;
    use proptest::prelude::*;

    proptest! {
        /// Whatever the text, `plain` is one clean line within the cap, with
        /// no leading, trailing or doubled space, and cleaning twice changes
        /// nothing.
        #[test]
        fn plain_is_one_bounded_clean_line(s in hostile::string(), max in 0usize..400) {
            let p = plain(&s, max);
            prop_assert!(p.chars().count() <= max);
            prop_assert!(hostile::is_clean_line(&p), "{p:?}");
            prop_assert!(!p.chars().any(char::is_whitespace) || !p.contains(['\t', '\n', '\r']));
            prop_assert!(!p.starts_with(' ') && !p.ends_with(' ') && !p.contains("  "), "{p:?}");
            prop_assert_eq!(plain(&p, max), p);
        }

        /// A text without anything to clean passes unchanged (up to the cap).
        #[test]
        fn plain_keeps_ordinary_text(s in "[A-Za-z0-9._-]{1,60}( [A-Za-z0-9._-]{1,20}){0,3}") {
            prop_assert_eq!(plain(&s, text_max()), s);
        }

        /// `literal` keeps every character that is fine, marks the others, and
        /// never passes a control or invisible one.
        #[test]
        fn literal_marks_and_caps(s in hostile::string(), max in 0usize..400) {
            let l = literal(&s, max);
            prop_assert!(l.chars().count() <= max + 1);
            prop_assert!(hostile::is_clean_line(&l), "{l:?}");
            if s.chars().count() <= max {
                prop_assert_eq!(l.chars().count(), s.chars().count());
            }
        }

        /// An icon is a name or a safe path, whatever it was given, and it
        /// never looks like a URL.
        #[test]
        fn icons_are_safe_names_or_paths(s in hostile::string()) {
            let i = icon(&s);
            if !i.is_empty() {
                let name = i.bytes().all(|b| b.is_ascii_alphanumeric() || b"-_.+".contains(&b)) && !i.starts_with('.');
                let path = i.starts_with('/') && !i.starts_with("//") && hostile::is_clean_line(&i)
                    && !i.split('/').any(|p| p == "..") && !i.contains(['?', '#', '%']);
                prop_assert!(name || path, "{i:?}");
                prop_assert!(!i.contains("://") && !i.starts_with("file:") && !i.starts_with("qrc:"), "{i:?}");
            }
            prop_assert_eq!(icon(&i), i);
        }

        /// A URL is never an icon, whatever follows the scheme.
        #[test]
        fn urls_are_never_icons(scheme in "(https?|ftp|file|qrc|image|data|ws)", rest in hostile::string_to(60)) {
            prop_assert_eq!(icon(&format!("{scheme}://{rest}")), "");
            prop_assert_eq!(icon(&format!("{scheme}:{rest}")), "");
        }
    }

    proptest! {
        #![proptest_config(hostile::cases(2500))]

        /// Arguments come out marked and bounded in number and size.
        #[test]
        fn command_lines_stay_bounded(args in proptest::collection::vec(hostile::string(), 0..16)) {
            let out = command_line(args.iter().map(String::as_str));
            prop_assert!(out.len() <= ARGS_MAX + 1);
            prop_assert!(out.iter().all(|a| hostile::is_clean_line(a)));
            prop_assert!(out.iter().map(|a| a.chars().count()).sum::<usize>() <= COMMAND_MAX + out.len() + 8);
        }
    }

    proptest! {
        #![proptest_config(hostile::cases(300))]

        /// Megabytes in, a short line out, and it does not take long.
        #[test]
        fn plain_and_literal_cap_huge_input(s in hostile::huge()) {
            prop_assert!(plain(&s, NAME_MAX).chars().count() <= NAME_MAX);
            prop_assert!(literal(&s, NAME_MAX).chars().count() <= NAME_MAX + 1);
        }
    }

    fn text_max() -> usize {
        NAME_MAX
    }
}
