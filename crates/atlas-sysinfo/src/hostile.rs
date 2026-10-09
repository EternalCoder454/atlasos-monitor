//! Strategies for the property tests (`props` modules): hostile text. Test
//! code only.

use proptest::prelude::*;

/// Characters that have broken parsers and displays: NUL, line breaks, escape,
/// DEL and C1 controls, bidirectional overrides and isolates, zero-width marks,
/// the line and paragraph separators, a tag character, markup and shell
/// characters, a slash and dots.
const SPECIAL: &[char] = &[
    '\0',
    '\n',
    '\r',
    '\t',
    '\u{1b}',
    '\u{7f}',
    '\u{85}',
    '\u{202E}',
    '\u{202A}',
    '\u{2066}',
    '\u{2069}',
    '\u{200B}',
    '\u{200F}',
    '\u{FEFF}',
    '\u{2028}',
    '\u{2029}',
    '\u{E0041}',
    '\u{FFFD}',
    '<',
    '>',
    '&',
    '"',
    '\'',
    '\\',
    '/',
    '.',
    '=',
    '[',
    ']',
    '%',
    ';',
    ':',
    ' ',
    '-',
    '$',
    '`',
    '(',
    ')',
    '*',
    '?',
    '#',
    '~',
    '|',
    '@',
];

/// One character: mostly the hostile ones and plain letters, sometimes anything.
pub fn character() -> impl Strategy<Value = char> {
    prop_oneof![
        6 => proptest::sample::select(SPECIAL),
        3 => proptest::char::range('a', 'z'),
        1 => any::<char>(),
    ]
}

/// Hostile text of up to 120 characters (every character is a strategy of its
/// own to generate, so a longer text costs: `huge` is for big input).
pub fn string() -> impl Strategy<Value = String> {
    proptest::collection::vec(character(), 0..120).prop_map(|c| c.into_iter().collect())
}

/// Hostile text of up to `n` characters.
pub fn string_to(n: usize) -> impl Strategy<Value = String> {
    proptest::collection::vec(character(), 0..n).prop_map(|c| c.into_iter().collect())
}

/// Text that was repeated into something big (up to a few hundred KiB).
pub fn huge() -> impl Strategy<Value = String> {
    (string_to(40), 100usize..4000).prop_map(|(s, n)| s.repeat(n))
}

/// Bytes: text as the bytes of a file, with some that are not UTF-8.
pub fn bytes() -> impl Strategy<Value = Vec<u8>> {
    prop_oneof![
        3 => string().prop_map(String::into_bytes),
        1 => proptest::collection::vec(any::<u8>(), 0..400),
    ]
}

/// A config that runs `max` cases, or fewer when PROPTEST_CASES asks for
/// fewer: for the properties that touch the disk, which would take minutes at
/// the 20000 cases CI runs the others with.
pub fn cases(max: u32) -> ProptestConfig {
    let asked = std::env::var("PROPTEST_CASES")
        .ok()
        .and_then(|v| v.parse::<u32>().ok())
        .unwrap_or(256);
    ProptestConfig::with_cases(asked.min(max))
}

/// No control or invisible character and no line break; a joiner only
/// between two characters that are shown.
pub fn is_clean_line(s: &str) -> bool {
    let chars: Vec<char> = s.chars().collect();
    chars.iter().enumerate().all(|(i, &c)| {
        if crate::joiner(c) {
            let shown =
                |x: Option<&char>| x.is_some_and(|x| !x.is_whitespace() && !crate::unprintable(*x));
            shown(i.checked_sub(1).and_then(|j| chars.get(j))) && shown(chars.get(i + 1))
        } else {
            !crate::unprintable(c)
        }
    })
}
