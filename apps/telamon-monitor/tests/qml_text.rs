//! Guards of the QML of Telamon Monitor that draws what other programs sent
//! (docs/SECURITY.md, "Text from outside"). They read the QML source, so a new
//! `Text`, link or script that skips a rule fails here, in CI, and not in a
//! review three releases later. Adapted from atlas-framework's
//! `telamon-framework-ui/tests/qml_text.rs`, which holds Telamon.Ui to the same.
//!
//! The rules:
//!
//! - Process names, command lines, unit names, application names, container
//!   names, desktop-file text, device names and fwupd text reach the QML. They
//!   are drawn as plain text: every `Text`, `Label`, `Heading`, `TextEdit` and
//!   `TextArea` says `textFormat: Text.PlainText` (a Qt `Label` guesses, and
//!   `<img src="https://...">` in a process name would be fetched);
//!   `TelamonLabel` (plain by default) is never switched away from it, and
//!   nothing asks for rich, styled, Markdown or auto-detected text. Telamon.Ui's
//!   rows and tables are plain by default (its own `qml_text.rs`).
//! - A link is opened only by the places listed in `LINKS`, with a URL that is a
//!   constant or built in Rust.
//! - An icon named by a model (not a literal or the app's own theme names) is
//!   listed in `ICONS`, with the Rust function that vets it:
//!   `Kirigami.Icon` loads whatever URL it is given, `https:` ones from the
//!   network.
//! - QML does not run text as code (`eval`, `Function`, `Qt.include`, a
//!   non-literal `Qt.createQmlObject`) and does not fetch (`XMLHttpRequest`,
//!   `fetch`, `WebSocket`, web views, an `Image` of a non-literal source, an
//!   `http:` literal).
//! - The checker itself is tested: snippets that must pass and must fail.

use std::fs;
use std::path::{Path, PathBuf};

// ---------------------------------------------------------------- the rules

/// Files (relative to the app) that may contain rich text, and what text.
/// Each entry: (file, token, how many times, why it is safe). None: Telamon
/// Monitor draws nothing as markup.
const RICH: &[(&str, &str, usize, &str)] = &[];

/// Text types: the last segment of the type name (`QQC2.Label` is `Label`).
const TEXT_TYPES: &[&str] = &[
    "Text",
    "Label",
    "Heading",
    "SelectableLabel",
    "Abbreviation",
    "TextEdit",
    "TextArea",
];

/// Types whose instances are Telamon.Ui text: they must never be given a
/// `textFormat` that is not plain.
const PLAIN_BY_DEFAULT: &[&str] = &["TelamonLabel", "TelamonTextArea", "TelamonTextField"];

/// Who may call `Qt.openUrlExternally`, how many times, with what, and why it
/// is safe. (Telamon.Ui's `TelamonPortal.openUrl`, which checks the scheme in C++,
/// arrives with telamon-ui 2.0.9; these two take a constant and a URI built in Rust.)
const LINKS: &[(&str, usize, &str)] = &[
    (
        "qml/AboutPage.qml",
        1,
        "a constant https address of the project's repository",
    ),
    (
        "qml/AppsPage.qml",
        1,
        "the `folder` of the `located` signal: a file:// URI that atlas_sysinfo::files::file_uri \
         builds from the parent folder of /proc/<pid>/exe, every byte outside the unreserved set \
         percent-encoded; the desktop opens a folder, it does not run a file",
    ),
];

/// Icons named by something other than a literal or one of the app's own
/// theme names (`root.icons.cpu`): (file, the expression, the vetting).
const ICONS: &[(&str, &str, &str)] = &[
    (
        "qml/StartupPage.qml",
        "page.s.icons[index] || \"application-x-executable\"",
        "StartupList.icons: atlas_sysinfo::text::icon() in autostart::list_desktop: a theme name or \
         an absolute path to an image file, never a URL",
    ),
    (
        "qml/EnergyPage.qml",
        "row.icon",
        "EnergySaver rows: apps::Icon from IconLookup, a name the theme lists or an existing image \
         file of at most 4 MiB (apps::icons::icon_file)",
    ),
    (
        "qml/SystemPage.qml",
        "page.s.osLogo || \"computer\"",
        "about::parse_os_release: LOGO only if it is 128 characters of [A-Za-z0-9-_.+]",
    ),
    (
        "qml/DevicesPage.qml",
        "page.kindIcon(kind)",
        "a function of the page that returns the app's own icon names",
    ),
];

/// Files that set the text of the style's own tooltip (`QQC2.ToolTip.text`),
/// which the style draws in a format it likes. None: the app uses
/// `TelamonToolTip` (plain).
const STYLE_TOOLTIPS: &[(&str, &str)] = &[];

/// Files with a `Loader { source: ... }`: none, the app loads components.
const LOADER_SOURCES: &[(&str, &str)] = &[];

// ------------------------------------------------------------ the QML reader

struct Source {
    /// Relative to the repository, with `/`.
    path: String,
    text: String,
    /// `text` with the inside of comments and string literals blanked
    /// (same length, same line breaks), so braces and names can be found
    /// without a string or a comment fooling the search.
    masked: String,
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(".")
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
    let Ok(entries) = fs::read_dir(dir) else {
        return;
    };
    for entry in entries {
        let path = entry.unwrap().path();
        if path.is_dir() {
            walk(&path, out);
        } else if matches!(
            path.extension().and_then(|e| e.to_str()),
            Some("qml" | "js")
        ) {
            out.push(path);
        }
    }
}

/// The QML and JS of the app: `qml/` next to this crate's manifest. Paths are
/// relative to the crate, with `/`.
fn sources() -> Vec<Source> {
    let root = repo();
    let mut files = Vec::new();
    walk(&root.join("qml"), &mut files);
    files.sort();
    let out: Vec<Source> = files
        .iter()
        .map(|p| {
            let text = fs::read_to_string(p).unwrap();
            Source {
                path: p
                    .strip_prefix(&root)
                    .unwrap()
                    .to_string_lossy()
                    .replace('\\', "/"),
                masked: mask(&text),
                text,
            }
        })
        .collect();
    assert!(out.len() >= 20, "found only {} QML files", out.len());
    out
}

/// `text` with comments and the inside of string, template and regular
/// expression literals replaced by spaces (newlines kept).
fn mask(text: &str) -> String {
    mask_with(text, true)
}

/// `mask`, optionally keeping the strings (only comments and regular
/// expressions blanked).
fn mask_with(text: &str, strings: bool) -> String {
    let b = text.as_bytes();
    let mut out = b.to_vec();
    let blank = |out: &mut Vec<u8>, from: usize, to: usize| {
        for byte in &mut out[from..to] {
            if *byte != b'\n' {
                *byte = b' ';
            }
        }
    };
    let mut i = 0;
    // The last byte that is not white space or part of a comment: a `/` after
    // an operator or a bracket starts a regular expression, after a name it
    // divides.
    let mut prev = b'\n';
    while i < b.len() {
        match b[i] {
            b'/' if b.get(i + 1) == Some(&b'/') => {
                let start = i;
                while i < b.len() && b[i] != b'\n' {
                    i += 1;
                }
                blank(&mut out, start, i);
                continue;
            }
            b'/' if b.get(i + 1) == Some(&b'*') => {
                let start = i;
                i += 2;
                while i < b.len() && !(b[i] == b'*' && b.get(i + 1) == Some(&b'/')) {
                    i += 1;
                }
                i = (i + 2).min(b.len());
                blank(&mut out, start, i);
                continue;
            }
            q @ (b'"' | b'\'' | b'`') => {
                let start = i + 1;
                i += 1;
                while i < b.len() && b[i] != q && !(b[i] == b'\n' && q != b'`') {
                    if b[i] == b'\\' {
                        i += 1;
                    }
                    i += 1;
                }
                let end = i.min(b.len());
                if strings {
                    blank(&mut out, start, end);
                }
                i = end + 1;
                prev = q;
                continue;
            }
            b'/' if regex_may_start(&out, i, prev) => {
                // A regular expression literal.
                let start = i + 1;
                i += 1;
                let mut class = false;
                while i < b.len() && b[i] != b'\n' && (class || b[i] != b'/') {
                    match b[i] {
                        b'\\' => i += 1,
                        b'[' => class = true,
                        b']' => class = false,
                        _ => {}
                    }
                    i += 1;
                }
                let end = i.min(b.len());
                blank(&mut out, start, end);
                i = end + 1;
                prev = b'/';
                continue;
            }
            c if !c.is_ascii_whitespace() => prev = c,
            _ => {}
        }
        i += 1;
    }
    String::from_utf8(out).unwrap()
}

/// Whether a `/` at `at` starts a regular expression: after an operator, an
/// opening bracket, a separator, `=>`, or a keyword such as `return`; after a
/// name, a number or a closing bracket it divides. `out` is the source so far
/// with comments and strings blanked.
fn regex_may_start(out: &[u8], at: usize, prev: u8) -> bool {
    if b"(,=:[!&|?{};\n+-*<>%~^".contains(&prev) {
        return true;
    }
    if !(prev.is_ascii_alphanumeric() || prev == b'_') {
        return false;
    }
    let mut end = at;
    while end > 0 && out[end - 1].is_ascii_whitespace() {
        end -= 1;
    }
    let mut start = end;
    while start > 0 && (out[start - 1].is_ascii_alphanumeric() || out[start - 1] == b'_') {
        start -= 1;
    }
    matches!(
        &out[start..end],
        b"return"
            | b"typeof"
            | b"case"
            | b"in"
            | b"of"
            | b"delete"
            | b"void"
            | b"throw"
            | b"new"
            | b"else"
            | b"do"
    )
}

fn line_of(text: &str, offset: usize) -> usize {
    text[..offset].matches('\n').count() + 1
}

fn is_name(c: u8) -> bool {
    c.is_ascii_alphanumeric() || c == b'_' || c == b'.'
}

/// One `Type { ... }`: the name as written, the line, and the byte range of
/// the braces' inside in the source.
struct Element {
    name: String,
    line: usize,
    open: usize,
    close: usize,
}

impl Element {
    /// The last segment of the name: `QQC2.Label` is `Label`.
    fn base(&self) -> &str {
        self.name.rsplit('.').next().unwrap()
    }
}

/// Every `Name {` of the masked source whose last segment starts with a
/// capital: an object of that type (or an `enum`, which no rule asks about).
fn elements(src: &Source) -> Vec<Element> {
    let b = src.masked.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < b.len() {
        if !(b[i].is_ascii_alphabetic() || b[i] == b'_') || (i > 0 && is_name(b[i - 1])) {
            i += 1;
            continue;
        }
        let start = i;
        while i < b.len() && is_name(b[i]) {
            i += 1;
        }
        let name = &src.masked[start..i];
        let mut j = i;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        let last = name.rsplit('.').next().unwrap();
        if b.get(j) == Some(&b'{') && last.as_bytes()[0].is_ascii_uppercase() {
            let mut depth = 0i32;
            let mut k = j;
            let mut close = b.len();
            while k < b.len() {
                match b[k] {
                    b'{' => depth += 1,
                    b'}' => {
                        depth -= 1;
                        if depth == 0 {
                            close = k;
                            break;
                        }
                    }
                    _ => {}
                }
                k += 1;
            }
            out.push(Element {
                name: name.to_string(),
                line: line_of(&src.text, start),
                open: j + 1,
                close,
            });
        }
    }
    out
}

/// The values of `prop:` (or `prop =`) that belong to the element itself, not
/// to an element or a function inside it.
fn own_values<'a>(src: &'a Source, e: &Element, prop: &str) -> Vec<&'a str> {
    own_spans(src, e, prop)
        .into_iter()
        .map(|(from, to)| src.masked[from..to].trim())
        .collect()
}

/// The byte ranges of the values of `prop:` that belong to the element itself.
fn own_spans(src: &Source, e: &Element, prop: &str) -> Vec<(usize, usize)> {
    let b = src.masked.as_bytes();
    let mut out = Vec::new();
    let mut depth = 0i32;
    let mut i = e.open;
    while i < e.close {
        match b[i] {
            b'{' => depth += 1,
            b'}' => depth -= 1,
            c if depth == 0
                && (c.is_ascii_alphabetic() || c == b'_')
                && (i == 0 || !is_name(b[i - 1])) =>
            {
                let start = i;
                while i < e.close && is_name(b[i]) {
                    i += 1;
                }
                if &src.masked[start..i] == prop {
                    let mut j = i;
                    while j < e.close && b[j].is_ascii_whitespace() {
                        j += 1;
                    }
                    if j < e.close && b[j] == b':' {
                        j += 1;
                        let from = j;
                        while j < e.close && !matches!(b[j], b'\n' | b';' | b'}') {
                            j += 1;
                        }
                        out.push((from, j));
                    }
                }
                continue;
            }
            _ => {}
        }
        i += 1;
    }
    out
}

/// The names of the types of the app that are text: the files whose root is a
/// text type, and the inline `component Name: Label {` ones; an instance of
/// one follows the same rule as the `Text` itself.
fn derived_text_types() -> &'static Vec<String> {
    static TYPES: std::sync::OnceLock<Vec<String>> = std::sync::OnceLock::new();
    TYPES.get_or_init(|| {
        let all = sources();
        let mut found: Vec<String> = Vec::new();
        loop {
            let before = found.len();
            for src in &all {
                let stem = src.path["qml/".len()..src.path.len() - ".qml".len()].to_string();
                if !found.contains(&stem)
                    && let Some(root) = elements(src).first()
                    && (TEXT_TYPES.contains(&root.base()) || found.iter().any(|f| f == root.base()))
                {
                    found.push(stem);
                }
                // `component Name: Type {`
                for part in src.masked.split("component ").skip(1) {
                    let Some((name, rest)) = part.split_once(':') else {
                        continue;
                    };
                    let ty: String = rest
                        .trim_start()
                        .chars()
                        .take_while(|c| is_name(*c as u8))
                        .collect();
                    let base = ty.rsplit('.').next().unwrap_or("");
                    let name = name.trim().to_string();
                    if name.chars().all(|c| c.is_ascii_alphanumeric() || c == '_')
                        && !found.contains(&name)
                        && (TEXT_TYPES.contains(&base) || found.iter().any(|f| f == base))
                    {
                        found.push(name);
                    }
                }
            }
            if found.len() == before {
                return found;
            }
        }
    })
}

// ---------------------------------------------------------------- the checks

/// Every violation of the plain-text rules in `src`; `rich` says how many
/// times each markup token is allowed in this file.
fn text_findings(src: &Source, rich: &[(&str, &str, usize, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for e in elements(src) {
        if TEXT_TYPES.contains(&e.base()) {
            let formats = own_values(src, &e, "textFormat");
            if !formats
                .iter()
                .any(|f| (f.ends_with(".PlainText") && !f.contains('?')) || rich_ok(src, rich, f))
            {
                out.push(format!(
                    "{}:{}: `{}` without `textFormat: Text.PlainText` draws <b>, <a href> and <img> of \
                     its text as markup",
                    src.path, e.line, e.name
                ));
            }
        }
        if PLAIN_BY_DEFAULT.contains(&e.base())
            || derived_text_types().iter().any(|t| t == e.base())
        {
            for f in own_values(src, &e, "textFormat") {
                if !f.ends_with(".PlainText") {
                    out.push(format!(
                        "{}:{}: `{}` is plain text unless told otherwise; this one is `{f}`",
                        src.path, e.line, e.name
                    ));
                }
            }
        }
    }
    // Any other `textFormat` that is not plain, in any form (a binding, an
    // assignment in a handler), and any markup name.
    let m = &src.masked;
    let mut at = 0;
    while let Some(p) = m[at..].find("textFormat") {
        let p = at + p;
        at = p + "textFormat".len();
        let before_ok = p == 0 || !is_name(m.as_bytes()[p - 1]) || m.as_bytes()[p - 1] == b'.';
        if !before_ok {
            continue;
        }
        let rest = m[at..].trim_start();
        if rest.starts_with(':') || (rest.starts_with('=') && !rest.starts_with("==")) {
            let value: String = rest[1..]
                .lines()
                .next()
                .unwrap_or("")
                .split([';', '}'])
                .next()
                .unwrap_or("")
                .trim()
                .to_string();
            if (!value.ends_with(".PlainText") || value.contains('?'))
                && !rich_ok(src, rich, &value)
            {
                out.push(format!(
                    "{}:{}: textFormat is `{value}`: text from outside is plain",
                    src.path,
                    line_of(&src.text, p)
                ));
            }
        }
    }
    if let Some(p) = m.find("ToolTip.text")
        && !STYLE_TOOLTIPS.iter().any(|(f, _)| *f == src.path)
    {
        out.push(format!(
            "{}:{}: ToolTip.text: the style draws that tooltip in a format of its own; use \
             TelamonToolTip (plain), or list the file in STYLE_TOOLTIPS with the reason",
            src.path,
            line_of(&src.text, p)
        ));
    }
    for token in [
        "RichText",
        "StyledText",
        "MarkdownText",
        "AutoText",
        "Text.Markdown",
        "TextEdit.Markdown",
    ] {
        let count = m.matches(token).count();
        let allowed = rich
            .iter()
            .filter(|(f, t, _, _)| *f == src.path && *t == token)
            .map(|(_, _, n, _)| *n)
            .sum::<usize>();
        if count != allowed {
            out.push(format!(
                "{}: {count} x {token}, {allowed} allowed (RICH lists the components that draw \
                 markup, each with the reason it is safe)",
                src.path
            ));
        }
    }
    out
}

/// Whether `value` (a `textFormat`) is a markup this file is listed for.
fn rich_ok(src: &Source, rich: &[(&str, &str, usize, &str)], value: &str) -> bool {
    rich.iter()
        .any(|(f, token, _, _)| *f == src.path && value.ends_with(token))
}

/// Where the name `word` is called in the masked source: `word`, white space,
/// `(`, and not the tail of a longer name (`evaluate(` is another name; a `.`
/// before it is a method call of the same name and counts). Offsets of the
/// word and of the first byte after the `(`.
fn calls(masked: &str, word: &str) -> Vec<(usize, usize)> {
    let b = masked.as_bytes();
    let mut out = Vec::new();
    let mut at = 0;
    while let Some(p) = masked[at..].find(word) {
        let p = at + p;
        at = p + word.len();
        let before_ok = p == 0 || !(b[p - 1].is_ascii_alphanumeric() || b[p - 1] == b'_');
        let mut j = at;
        while j < b.len() && b[j].is_ascii_whitespace() {
            j += 1;
        }
        if before_ok && j < b.len() && b[j] == b'(' {
            out.push((p, j + 1));
        }
    }
    out
}

/// Violations of the link rules in `src`: `Qt.openUrlExternally` is called
/// only where `LINKS` lists it, as often as it says; `Qt.openUrl` and the like
/// are not used; `linkActivated` is not handled (nothing here draws links).
fn link_findings(src: &Source, links: &[(&str, usize, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    let m = &src.masked;
    // openUrlExternally in any spelling (also Qt["openUrlExternally"], which
    // the masking blanks): anywhere in the code, comments aside.
    let code = mask_with(&src.text, false);
    let count = code.matches("openUrlExternally").count();
    let allowed = links
        .iter()
        .filter(|(f, _, _)| *f == src.path)
        .map(|(_, n, _)| *n)
        .sum::<usize>();
    if count != allowed {
        out.push(format!(
            "{}: {count} x openUrlExternally, {allowed} allowed (LINKS lists the places that open \
             a link, with the reason the address is safe)",
            src.path
        ));
    }
    for (p, _) in calls(m, "openUrl") {
        out.push(format!(
            "{}:{}: openUrl: open a link with the listed Qt.openUrlExternally calls only",
            src.path,
            line_of(&src.text, p)
        ));
    }
    for hook in ["linkActivated", "LinkActivated", "onLinkHovered"] {
        if let Some(p) = m.find(hook) {
            out.push(format!(
                "{}:{}: {hook}: nothing in this app draws a link",
                src.path,
                line_of(&src.text, p)
            ));
        }
    }
    out
}

/// Violations of the icon rule: the value of every `iconName`, `source`,
/// `icon.name` and `icon.source` of an element is a literal, a choice between
/// literals, the app's own theme names (`<id>.icons.<name>`), or listed in
/// `ICONS`.
fn icon_findings(src: &Source, icons: &[(&str, &str, &str)]) -> Vec<String> {
    let mut out = Vec::new();
    for e in elements(src) {
        for prop in [
            "iconName",
            "source",
            "icon.name",
            "icon.source",
            "iconSource",
        ] {
            for (from, to) in own_spans(src, &e, prop) {
                let value = src.text[from..to].trim();
                let masked = src.masked[from..to].trim();
                if icon_value_ok(masked, value) {
                    continue;
                }
                if icons.iter().any(|(f, v, _)| *f == src.path && *v == value) {
                    continue;
                }
                out.push(format!(
                    "{}:{}: `{prop}: {value}`: an icon named by data is loaded by Kirigami.Icon, \
                     which fetches URLs; list it in ICONS with the function that vets it",
                    src.path, e.line
                ));
            }
        }
    }
    out
}

/// Whether every value the expression can have is a literal or one of the
/// app's own theme names (`root.icons.cpu`): the conditions of `?:` do not
/// matter, the branches and the sides of `||` and `??` do. `masked` is the
/// expression with the inside of its strings blanked.
fn icon_value_ok(masked: &str, _raw: &str) -> bool {
    icon_leaves(masked).iter().all(|leaf| {
        let leaf = leaf.trim().trim_matches(|c| c == '(' || c == ')').trim();
        let b = leaf.as_bytes();
        let literal = b.len() >= 2
            && (b[0] == b'"' || b[0] == b'\'')
            && b[b.len() - 1] == b[0]
            && leaf[1..leaf.len() - 1].bytes().all(|c| c == b' ');
        let theme = leaf.split_once(".icons.").is_some_and(|(id, name)| {
            !id.is_empty()
                && id
                    .bytes()
                    .all(|c| c.is_ascii_alphanumeric() || c == b'_' || c == b'.')
                && !name.is_empty()
                && name.bytes().all(|c| c.is_ascii_alphanumeric() || c == b'_')
        });
        literal || theme
    })
}

/// The values an expression can have: the branches of a `?:`, the sides of
/// `||` and `??`, and otherwise the expression.
fn icon_leaves(e: &str) -> Vec<&str> {
    let e = e.trim();
    if let Some(q) = top_level(e, "?") {
        let rest = &e[q + 1..];
        return match top_level(rest, ":") {
            Some(c) => [icon_leaves(&rest[..c]), icon_leaves(&rest[c + 1..])].concat(),
            None => icon_leaves(rest),
        };
    }
    for op in ["||", "??"] {
        if let Some(o) = top_level(e, op) {
            return [icon_leaves(&e[..o]), icon_leaves(&e[o + 2..])].concat();
        }
    }
    vec![e]
}

/// The first place `op` stands outside brackets and strings (whose inside is
/// blanked, their quotes kept).
fn top_level(e: &str, op: &str) -> Option<usize> {
    let b = e.as_bytes();
    let (mut depth, mut quote) = (0i32, None::<u8>);
    let mut i = 0;
    while i < b.len() {
        match (quote, b[i]) {
            (Some(q), c) if c == q => quote = None,
            (Some(_), _) => {}
            (None, c @ (b'"' | b'\'')) => quote = Some(c),
            (None, b'(' | b'[' | b'{') => depth += 1,
            (None, b')' | b']' | b'}') => depth -= 1,
            // `?.` is optional chaining and `??` another operator, not a `?:`.
            (None, _)
                if depth == 0
                    && e[i..].starts_with(op)
                    && !(op == "?" && matches!(b.get(i + 1), Some(b'.' | b'?'))) =>
            {
                return Some(i);
            }
            _ => {}
        }
        i += 1;
    }
    None
}

/// The text of a call's first argument, from just after the `(`.
fn text_of_call(text: &str, masked: &str, from: usize) -> String {
    let b = masked.as_bytes();
    let mut depth = 0;
    let mut i = from;
    while i < b.len() {
        match b[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' | b']' | b'}' if depth == 0 => break,
            b')' | b']' | b'}' => depth -= 1,
            b',' if depth == 0 => break,
            _ => {}
        }
        i += 1;
    }
    text[from..i].to_string()
}

/// Violations of the code and network rules in `src`.
fn code_findings(src: &Source) -> Vec<String> {
    let mut out = Vec::new();
    let m = &src.masked;
    for bad in [
        "new Function",
        "Qt.include",
        "XMLHttpRequest",
        "WebSocket",
        "WebView",
        "WebEngine",
        "Qt.callLater(eval",
        "importScripts",
        "setSource",
    ] {
        let mut at = 0;
        while let Some(p) = m[at..].find(bad) {
            let p = at + p;
            at = p + bad.len();
            note(
                &mut out,
                src,
                p,
                &format!("{bad}: QML here neither runs text as code nor fetches"),
            );
        }
    }
    // Calls of these, `eval (s)` and `window.fetch(u)` too.
    for word in ["eval", "Function", "fetch"] {
        for (p, _) in calls(m, word) {
            note(
                &mut out,
                src,
                p,
                &format!("{word}(): QML here neither runs text as code nor fetches"),
            );
        }
    }
    // A template literal hides what is in its ${ } from this scan.
    if let Some(p) = m.find('`') {
        note(
            &mut out,
            src,
            p,
            "a template literal: use string concatenation, so the lint reads all the code",
        );
    }
    // A `source` set by a statement or a Binding is not read by the image rule.
    for (n, line) in src.text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('*') {
            continue;
        }
        let squeezed: String = line.split_whitespace().collect();
        if squeezed.contains("property:\"source\"") || squeezed.contains("property:'source'") {
            out.push(format!(
                "{}:{}: a Binding or PropertyChanges on `source`: set it as a property of the item, where the image rule reads it",
                src.path,
                n + 1
            ));
        }
    }
    {
        let b = m.as_bytes();
        let mut at = 0;
        while let Some(p) = m[at..].find(".source") {
            let p = at + p;
            at = p + ".source".len();
            let mut j = at;
            while j < b.len() && b[j].is_ascii_whitespace() {
                j += 1;
            }
            if j < b.len() && b[j] == b'=' && !matches!(b.get(j + 1), Some(b'=') | Some(b'>')) {
                note(
                    &mut out,
                    src,
                    p,
                    ".source = …: set the source as a property of the item, where the image rule reads it",
                );
            }
        }
    }
    // Qt.createQmlObject / Qt.createComponent: a literal argument only.
    for call in ["Qt.createQmlObject(", "Qt.createComponent("] {
        let mut at = 0;
        while let Some(p) = m[at..].find(call) {
            let p = at + p;
            at = p + call.len();
            let arg = text_of_call(&src.text, m, at);
            let arg = arg.trim();
            let literal = arg.len() >= 2
                && (arg.starts_with('"') || arg.starts_with('\''))
                && arg.ends_with(arg.chars().next().unwrap())
                && !arg[1..arg.len() - 1].contains(arg.chars().next().unwrap())
                && !arg.contains('+');
            if !literal {
                note(
                    &mut out,
                    src,
                    p,
                    &format!(
                        "{call}…): the argument is not a string literal, so text from outside could become code"
                    ),
                );
            }
        }
    }
    // A literal http: URL (cleartext) anywhere in a string.
    for (n, line) in src.text.lines().enumerate() {
        let t = line.trim_start();
        if t.starts_with("//") || t.starts_with('*') || t.starts_with("/*") {
            continue;
        }
        if line.contains("\"http://") || line.contains("'http://") {
            out.push(format!(
                "{}:{}: a cleartext http:// URL in a string",
                src.path,
                n + 1
            ));
        }
    }
    // A Loader whose `source` is a string loads whatever it names as QML.
    for e in elements(src) {
        if e.base() == "Loader"
            && !own_values(src, &e, "source").is_empty()
            && !LOADER_SOURCES.iter().any(|(f, _)| *f == src.path)
        {
            out.push(format!(
                "{}:{}: Loader with `source`: use sourceComponent, a string could name any QML",
                src.path, e.line
            ));
        }
    }
    out
}

fn note(out: &mut Vec<String>, src: &Source, p: usize, what: &str) {
    out.push(format!("{}:{}: {what}", src.path, line_of(&src.text, p)));
}

fn assert_clean(findings: Vec<String>) {
    assert!(findings.is_empty(), "\n{}\n", findings.join("\n"));
}

// ------------------------------------------------------------ the real files

#[test]
fn drawn_text_is_plain() {
    let mut findings = Vec::new();
    for src in sources() {
        let rich: Vec<_> = RICH
            .iter()
            .copied()
            .filter(|(f, ..)| *f == src.path)
            .collect();
        findings.extend(text_findings(&src, &rich));
    }
    assert_clean(findings);
}

#[test]
fn links_are_opened_in_known_places_only() {
    let mut findings = Vec::new();
    for src in sources() {
        findings.extend(link_findings(&src, LINKS));
    }
    assert_clean(findings);
    let all = sources();
    for (file, n, why) in LINKS {
        let src = all
            .iter()
            .find(|s| s.path == *file)
            .unwrap_or_else(|| panic!("{file} on LINKS"));
        assert_eq!(
            mask_with(&src.text, false)
                .matches("openUrlExternally")
                .count(),
            *n,
            "{file}"
        );
        assert!(why.len() > 30, "{file}: say why it is safe");
    }
}

#[test]
fn icons_named_by_data_are_vetted() {
    let mut findings = Vec::new();
    let all = sources();
    for src in &all {
        findings.extend(icon_findings(src, ICONS));
    }
    assert_clean(findings);
    for (file, expr, why) in ICONS {
        let src = all
            .iter()
            .find(|s| s.path == *file)
            .unwrap_or_else(|| panic!("{file} on ICONS"));
        assert!(
            src.text.contains(expr),
            "{file}: `{expr}` is gone from the file; take it off ICONS"
        );
        assert!(why.len() > 30, "{file}: say what vets it");
    }
}

#[test]
fn no_qml_runs_text_as_code_or_fetches() {
    let mut findings = Vec::new();
    for src in sources() {
        findings.extend(code_findings(&src));
    }
    assert_clean(findings);
}

#[test]
fn images_are_not_made_in_qml() {
    // The app draws icons through Kirigami.Icon (the ICONS rule) and charts in
    // C++: an Image of a model's text would load it.
    for src in sources() {
        for e in elements(&src) {
            assert!(
                !matches!(
                    e.base(),
                    "Image" | "AnimatedImage" | "BorderImage" | "AnimatedSprite"
                ),
                "{}:{}: {}: an Image is not used here; add a rule for its source first",
                src.path,
                e.line,
                e.name
            );
        }
    }
}

#[test]
fn the_details_dialog_shows_text_it_can_select_as_plain_text() {
    // The one TextEdit: a path or a command line a person copies.
    let all = sources();
    let d = all
        .iter()
        .find(|s| s.path == "qml/DetailsDialog.qml")
        .unwrap();
    let edits: Vec<_> = elements(d)
        .into_iter()
        .filter(|e| e.base() == "TextEdit")
        .collect();
    assert_eq!(edits.len(), 1);
    assert_eq!(
        own_values(d, &edits[0], "textFormat"),
        ["TextEdit.PlainText"]
    );
    assert_eq!(own_values(d, &edits[0], "readOnly"), ["true"]);
}

#[test]
fn copy_details_puts_only_the_pages_text_on_the_clipboard() {
    // Platform.copy is the one way to the clipboard; the page hands it the
    // text it shows (rows of cleaned strings) and nothing from /proc.
    let all = sources();
    let uses: Vec<&str> = all
        .iter()
        .filter(|s| s.masked.contains(".copy("))
        .map(|s| s.path.as_str())
        .collect();
    assert_eq!(uses, ["qml/SystemPage.qml"]);
}

// ------------------------------------------------- the checker checks itself

fn snippet(path: &str, text: &str) -> Source {
    Source {
        path: path.to_string(),
        masked: mask(text),
        text: text.to_string(),
    }
}

fn text_of(path: &str, text: &str) -> Vec<String> {
    text_findings(&snippet(path, text), &[])
}

#[test]
fn checker_accepts_plain_text() {
    for ok in [
        "Item { Text { text: a; textFormat: Text.PlainText } }",
        "Item {\n  QQC2.Label {\n    text: x\n    textFormat: Text.PlainText\n  }\n}",
        "Item {\n  contentItem: Text {\n    textFormat: Text.PlainText\n  }\n}",
        "Item { Kirigami.Heading { text: a; textFormat: Text.PlainText } }",
        "Item { TextEdit { textFormat: TextEdit.PlainText } }",
        // A comment and a string that mention markup are not markup.
        "Item { // RichText is not used\n Text { text: \"RichText\"; textFormat: Text.PlainText } }",
        "Item { TelamonLabel { text: a } }",
        // The nested element's format does not count for its parent, but its own does.
        "Item { Text { textFormat: Text.PlainText; Item { Text { textFormat: Text.PlainText } } } }",
    ] {
        assert_eq!(text_of("qml/X.qml", ok), Vec::<String>::new(), "{ok}");
    }
}

#[test]
fn checker_rejects_markup_text() {
    for (bad, what) in [
        ("Item { Text { text: a } }", "no textFormat"),
        ("Item { QQC2.Label { text: a } }", "no textFormat Label"),
        (
            "Item { Text { text: a; textFormat: Text.RichText } }",
            "RichText",
        ),
        (
            "Item { Text { text: a; textFormat: Text.StyledText } }",
            "StyledText",
        ),
        (
            "Item { Text { text: a; textFormat: Text.AutoText } }",
            "AutoText",
        ),
        (
            "Item { Text { text: a; textFormat: Text.MarkdownText } }",
            "MarkdownText",
        ),
        ("Item { Text { text: a; textFormat: fmt } }", "a variable"),
        (
            "Item { Text { textFormat: Text.PlainText; Item { Text { text: a } } } }",
            "the child has none",
        ),
        (
            "Item { Text { textFormat: Text.PlainText; Component.onCompleted: textFormat = Text.RichText } }",
            "assigned in a handler",
        ),
        (
            "Item { TelamonLabel { text: a; textFormat: Text.RichText } }",
            "TelamonLabel switched away",
        ),
        ("Item { TextEdit { text: a } }", "TextEdit without a format"),
        (
            "Item { Kirigami.Heading { text: a } }",
            "Heading without a format",
        ),
    ] {
        assert!(
            !text_of("qml/X.qml", bad).is_empty(),
            "must fail ({what}): {bad}"
        );
    }
}

#[test]
fn checker_allow_list_is_exact() {
    let rich = [("qml/R.qml", "RichText", 1usize, "why")];
    let one = snippet("qml/R.qml", "Text { textFormat: Text.RichText }");
    assert!(text_findings(&one, &rich).is_empty());
    // A second use in the same file is not listed.
    let two = snippet(
        "qml/R.qml",
        "Item { Text { textFormat: Text.RichText } Text { textFormat: Text.RichText } }",
    );
    assert!(!text_findings(&two, &rich).is_empty());
    // Another file is not allowed.
    let other = snippet("qml/Other.qml", "Text { textFormat: Text.RichText }");
    assert!(!text_findings(&other, &rich).is_empty());
    // Another token in the allowed file is not allowed.
    let styled = snippet("qml/R.qml", "Text { textFormat: Text.StyledText }");
    assert!(!text_findings(&styled, &rich).is_empty());
}

#[test]
fn checker_masks_strings_comments_and_regexes() {
    let m = mask("a // Text {\n/* Text { */ \"Text {\" '}' x.replace(/\"/g, \"y\") z");
    assert!(!m.contains("Text"), "{m}");
    assert_eq!(
        m.len(),
        "a // Text {\n/* Text { */ \"Text {\" '}' x.replace(/\"/g, \"y\") z".len()
    );
    // A regular expression with a quote must not swallow the code after it.
    let src = snippet(
        "qml/X.qml",
        "Item { function f(s) { return s.replace(/\"/g, \"\"); } Text { text: a } }",
    );
    assert_eq!(text_findings(&src, &[]).len(), 1);
    // Division is not a regular expression.
    let src = snippet(
        "qml/X.qml",
        "Item { width: a / 2; Text { text: \"x\"; textFormat: Text.PlainText } }",
    );
    assert!(text_findings(&src, &[]).is_empty());
}

#[test]
fn checker_code_and_network() {
    for bad in [
        "Item { Component.onCompleted: eval(s) }",
        "Item { Component.onCompleted: new Function(s)() }",
        "Item { Component.onCompleted: Qt.include(s) }",
        "Item { Component.onCompleted: Qt.createQmlObject(s, this) }",
        "Item { Component.onCompleted: Qt.createQmlObject(\"Item {\" + s + \"}\", this) }",
        "Item { Component.onCompleted: Qt.createComponent(url) }",
        "Item { Component.onCompleted: { var x = new XMLHttpRequest(); } }",
        "Item { Component.onCompleted: fetch(u) }",
        "Item { WebSocket { } }",
        "Item { property string u: \"http://example.com/a.png\" }",
        "Item { Loader { source: page } }",
    ] {
        assert!(
            !code_findings(&snippet("qml/X.qml", bad)).is_empty(),
            "{bad}"
        );
    }
    for ok in [
        "Item { Component.onCompleted: Qt.createQmlObject(\"import QtQuick\\nItem {}\", this) }",
        "Item { Component.onCompleted: Qt.createComponent(\"Foo.qml\") }",
        "Item { // eval(x) is not used\n property string s: \"fetch(\" }",
        "Item { function retrieval() {} property var v: evaluate(1) }",
        "Item { property string u: \"https://example.com\"; Loader { sourceComponent: c } }",
    ] {
        assert_eq!(
            code_findings(&snippet("qml/X.qml", ok)),
            Vec::<String>::new(),
            "{ok}"
        );
    }
}

#[test]
fn checker_masks_a_regex_after_return_and_arrow() {
    // `return /"/` and `=> /'/` are regular expressions: the quote in them must not
    // start a string that swallows the Text after it.
    for js in [
        "Item { function f(s) { return /\"/.test(s); } Text { text: a } }",
        "Item { property var g: s => /'/.test(s); Text { text: a } }",
        "Item { function f(s) { if (a) return /{/.test(s); } Text { text: a } }",
    ] {
        assert_eq!(text_of("qml/X.qml", js).len(), 1, "{js}");
    }
}

fn links_of(path: &str, text: &str) -> Vec<String> {
    link_findings(&snippet(path, text), LINKS)
}

fn icons_of(path: &str, text: &str) -> Vec<String> {
    icon_findings(&snippet(path, text), ICONS)
}

#[test]
fn checker_links() {
    assert_eq!(
        links_of("qml/X.qml", "Item { onClicked: Qt.openUrlExternally(u) }").len(),
        1
    );
    assert_eq!(
        links_of(
            "qml/X.qml",
            "Item { onClicked: Qt[\"openUrlExternally\"](u) }"
        )
        .len(),
        1
    );
    assert_eq!(
        links_of("qml/X.qml", "Item { onClicked: Qt.openUrl(u) }").len(),
        1
    );
    assert_eq!(
        links_of("qml/X.qml", "Text { onLinkActivated: l => go(l) }").len(),
        1
    );
    // The listed file, the listed number.
    assert!(
        links_of(
            "qml/AboutPage.qml",
            "Item { onClicked: Qt.openUrlExternally(\"https://example.org\") }"
        )
        .is_empty()
    );
    // The listed file, one more.
    assert_eq!(
        links_of(
            "qml/AboutPage.qml",
            "Item { onClicked: { Qt.openUrlExternally(a); Qt.openUrlExternally(b); } }"
        )
        .len(),
        1
    );
    // A comment is not a call.
    assert!(links_of("qml/X.qml", "Item { // Qt.openUrlExternally(u)\n }").is_empty());
}

#[test]
fn checker_icons() {
    // Literals, choices between literals and the app's own names are fine.
    for ok in [
        "Item { MonitorIcon { source: \"computer\" } }",
        "Item { SectionRow { iconName: a ? \"dialog-error\" : \"checkmark\" } }",
        "Item { SectionRow { iconName: page.icons.cpu } }",
        "Item { SectionRow { iconName: root.devices.routeWireless ? root.icons.wireless : root.icons.wired } }",
        "Item { ToolButton { icon.name: shown ? \"checkmark\" : \"\" } }",
    ] {
        assert_eq!(icons_of("qml/X.qml", ok), Vec::<String>::new(), "{ok}");
    }
    // Data is not, unless listed for that file with that expression.
    for bad in [
        "Item { SectionRow { iconName: page.s.icons[index] } }",
        "Item { SectionRow { iconName: modelData.icon } }",
        "Item { MonitorIcon { source: row.icon } }",
        "Item { MonitorIcon { source: someUrl } }",
        "Item { MonitorIcon { source: \"file://\" + name } }",
        "Item { SectionRow { iconName: page.s.icons[index] || \"application-x-executable\" } }",
    ] {
        assert!(!icons_of("qml/X.qml", bad).is_empty(), "{bad}");
    }
    assert!(
        icons_of(
            "qml/EnergyPage.qml",
            "Item { SectionRow { iconName: row.icon } }"
        )
        .is_empty()
    );
    assert!(
        !icons_of(
            "qml/EnergyPage.qml",
            "Item { SectionRow { iconName: row.icon2 } }"
        )
        .is_empty()
    );
}

#[test]
fn checker_derived_types() {
    let derived = derived_text_types();
    assert!(derived.iter().any(|d| d == "NavHeading"), "{derived:?}");
    assert!(
        !text_of(
            "qml/X.qml",
            "Item { NavHeading { textFormat: Text.RichText } }"
        )
        .is_empty()
            || !derived.is_empty()
    );
}

#[test]
fn checker_finds_the_real_files() {
    // If the walk found no Text at all, every rule above would pass for
    // nothing.
    let all = sources();
    let texts: usize = all
        .iter()
        .map(|s| {
            elements(s)
                .iter()
                .filter(|e| TEXT_TYPES.contains(&e.base()))
                .count()
        })
        .sum();
    assert!(texts >= 20, "found only {texts} Text elements");
    let opens: usize = all
        .iter()
        .map(|s| s.masked.matches("openUrlExternally(").count())
        .sum();
    assert_eq!(opens, 2);
    assert!(all.iter().any(|s| s.path == "qml/Main.qml"));
}
