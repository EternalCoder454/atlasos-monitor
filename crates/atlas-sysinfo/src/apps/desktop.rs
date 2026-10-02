//! Which application a systemd unit is, and what its desktop file says.
//!
//! The unit naming is systemd's convention for desktop environments
//! (<https://systemd.io/DESKTOP_ENVIRONMENTS/>):
//!
//! ```text
//! app[-<launcher>]-<ApplicationID>[@<RANDOM>].service
//! app[-<launcher>]-<ApplicationID>-<RANDOM>.scope
//! ```
//!
//! A dash inside an ID is escaped as `\x2d`, so the literal dashes are the
//! separators. The ID is also the desktop file's name, which holds the name
//! people know the application by and its icon.

use std::borrow::Cow;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

/// The application ID encoded in a systemd unit name, or `None` when the
/// unit is not an application's: services, the session, and the D-Bus
/// activated helpers that share `app.slice` are not.
pub fn app_id(unit: &str) -> Option<Cow<'_, str>> {
    let rest = unit.strip_prefix("app-")?;
    let (rest, scope) = if let Some(r) = rest.strip_suffix(".scope") {
        (r, true)
    } else {
        // Neither: a slice, or something else that holds no processes.
        let r = rest.strip_suffix(".service")?;
        // The instance: a random string, or "autostart".
        (r.split_once('@').map_or(r, |(id, _)| id), false)
    };
    let mut parts = rest.rsplit('-');
    let mut id = parts.next()?;
    if scope && let Some(before) = parts.next() {
        id = before; // the last part was the random suffix
    }
    // What is left in front of the ID is the launcher, if there is one.
    let id = unescape(id);
    (!id.is_empty()).then_some(id)
}

/// Reverses systemd's `\xNN` escaping of unit names.
pub fn unescape(s: &str) -> Cow<'_, str> {
    if !s.contains("\\x") {
        return Cow::Borrowed(s);
    }
    let b = s.as_bytes();
    let mut out = Vec::with_capacity(b.len());
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\'
            && b.get(i + 1) == Some(&b'x')
            && let Some(hex) = b.get(i + 2..i + 4)
            && hex.iter().all(u8::is_ascii_hexdigit)
            && let Some(v) = std::str::from_utf8(hex)
                .ok()
                .and_then(|h| u8::from_str_radix(h, 16).ok())
        {
            out.push(v);
            i += 4;
            continue;
        }
        out.push(b[i]);
        i += 1;
    }
    Cow::Owned(String::from_utf8_lossy(&out).into_owned())
}

/// A readable name for an ID with no desktop file: the last component of a
/// reverse-DNS ID (`com.discordapp.Discord` → `Discord`), or the ID itself.
pub fn fallback_name(id: &str) -> &str {
    match id.rsplit_once('.') {
        Some((_, last)) if !last.is_empty() && id.matches('.').count() >= 2 => last,
        _ => id,
    }
}

/// What a desktop file says about an application.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entry {
    pub name: String,
    /// An icon name to look up in the theme, or an absolute path. Empty for
    /// none.
    pub icon: String,
    /// A terminal emulator (`Categories=TerminalEmulator`). A terminal busy
    /// with a build is a job someone is waiting on, so Energy Saver leaves
    /// terminals be.
    pub terminal: bool,
}

/// Parses the `[Desktop Entry]` group of a desktop file. `locales` are the
/// `Name[...]` keys to prefer, best first (see [`locale_keys`]). `None` when
/// the file says the application is hidden: the specification's way of
/// deleting an entry.
pub fn parse_entry(text: &str, id: &str, locales: &[String]) -> Option<Entry> {
    let mut e = Entry {
        name: String::new(),
        icon: String::new(),
        terminal: false,
    };
    let mut best = locales.len(); // rank of the Name found so far; lower wins
    let mut inside = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if line.starts_with('[') {
            if inside {
                break; // past the main group: actions don't matter
            }
            inside = line == "[Desktop Entry]";
            continue;
        }
        if !inside {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        let (key, val) = (key.trim(), val.trim());
        if key == "Name" {
            if best == locales.len() {
                e.name = val.to_owned();
            }
        } else if let Some(loc) = key.strip_prefix("Name[").and_then(|k| k.strip_suffix(']')) {
            if let Some(rank) = locales.iter().position(|l| l == loc)
                && rank < best
            {
                e.name = val.to_owned();
                best = rank;
            }
        } else if key == "Icon" {
            e.icon = val.to_owned();
        } else if key == "Categories" {
            e.terminal = val.split(';').any(|c| c == "TerminalEmulator");
        } else if key == "Hidden" && val == "true" {
            return None;
        }
    }
    if e.name.is_empty() {
        e.name = fallback_name(id).to_owned();
    }
    Some(e)
}

/// The `Name[...]` keys to prefer, best first, from the message locale:
/// `pt_BR.UTF-8` gives `pt_BR`, then `pt`.
pub fn locale_keys() -> Vec<String> {
    let loc = ["LC_ALL", "LC_MESSAGES", "LANG"]
        .iter()
        .filter_map(|v| std::env::var(v).ok())
        .find(|v| !v.is_empty())
        .unwrap_or_default();
    locale_keys_of(&loc)
}

fn locale_keys_of(loc: &str) -> Vec<String> {
    let loc = loc.split(['.', '@']).next().unwrap_or("");
    if loc.is_empty() || loc == "C" || loc == "POSIX" {
        return Vec::new();
    }
    let mut keys = vec![loc.to_owned()];
    if let Some((lang, _)) = loc.split_once('_') {
        keys.push(lang.to_owned());
    }
    keys
}

/// The XDG data directories, most important first, with Flatpak's exports
/// added when the session didn't include them. A session that never
/// sourced Flatpak's profile script would otherwise leave every Flatpak app
/// nameless, and most apps on Kinoite are Flatpaks.
pub fn data_dirs() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME").filter(|h| !h.is_empty());
    let data_home = std::env::var_os("XDG_DATA_HOME")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.map(|h| Path::new(&h).join(".local/share")));
    let sys = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".into());

    let mut dirs: Vec<PathBuf> = Vec::new();
    dirs.extend(data_home.clone());
    dirs.extend(sys.split(':').filter(|d| !d.is_empty()).map(PathBuf::from));
    dirs.extend(data_home.map(|d| d.join("flatpak/exports/share")));
    dirs.push("/var/lib/flatpak/exports/share".into());

    let mut out: Vec<PathBuf> = Vec::with_capacity(dirs.len());
    for d in dirs {
        let d: PathBuf = d.components().collect(); // drops a trailing slash
        if !out.contains(&d) {
            out.push(d);
        }
    }
    out
}

/// How stale the listing may get before a miss makes it look again:
/// something installed since Atlas started turns up without a restart, but
/// a process with no desktop file doesn't cost a directory walk every tick.
const RESCAN_AFTER: Duration = Duration::from_secs(30);

/// Finds desktop entries by ID. It lists the `applications/` folders once
/// and parses a file only when its ID is first asked about.
#[derive(Debug)]
pub struct Index {
    dirs: Vec<PathBuf>,
    locales: Vec<String>,
    /// ID → file, from the last scan. `None` until the first lookup.
    paths: Option<HashMap<String, PathBuf>>,
    scanned: Option<Instant>,
    /// Parsed entries; `None` for a file that is hidden or unreadable.
    entries: HashMap<String, Option<Entry>>,
    rescan_after: Duration,
}

impl Index {
    /// An index over `dirs`, each a data directory with an `applications/`
    /// folder in it. Earlier directories win, as XDG specifies.
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        Self {
            dirs,
            locales: locale_keys(),
            paths: None,
            scanned: None,
            entries: HashMap::new(),
            rescan_after: RESCAN_AFTER,
        }
    }

    /// The entry for `id`, or `None` when there is no desktop file for it or
    /// the file says the application is hidden.
    pub fn lookup(&mut self, id: &str) -> Option<&Entry> {
        if !self.entries.contains_key(id) {
            let stale = self
                .scanned
                .is_none_or(|t| t.elapsed() >= self.rescan_after);
            let known = self.paths.as_ref().is_some_and(|p| p.contains_key(id));
            if !known && stale {
                self.scan();
            }
            // A miss isn't remembered: it may be installed later.
            let path = self.paths.as_ref()?.get(id)?;
            // An unreadable file (a broken link) isn't remembered either:
            // it may be mended.
            let text = std::fs::read_to_string(path).ok()?;
            let entry = parse_entry(&text, id, &self.locales);
            self.entries.insert(id.to_owned(), entry);
        }
        self.entries.get(id)?.as_ref()
    }

    /// Lists every desktop file under the data directories. A file in a
    /// subdirectory has an ID with the separator turned into a dash, as the
    /// specification says: `kde4/konsole.desktop` is `kde4-konsole`.
    fn scan(&mut self) {
        let mut paths = HashMap::new();
        for d in &self.dirs {
            walk(&d.join("applications"), "", &mut paths, 0);
        }
        self.paths = Some(paths);
        self.scanned = Some(Instant::now());
    }
}

fn walk(dir: &Path, prefix: &str, paths: &mut HashMap<String, PathBuf>, depth: usize) {
    let Ok(rd) = std::fs::read_dir(dir) else {
        return;
    };
    // In name order, so when `a-b.desktop` and `a/b.desktop` both claim the
    // ID `a-b` the same one wins every time.
    let mut entries: Vec<_> = rd.flatten().collect();
    entries.sort_by_key(|e| e.file_name());
    for e in entries {
        let name = e.file_name();
        let Some(name) = name.to_str() else { continue };
        // file_type doesn't follow links: a linked folder isn't walked, and
        // a linked desktop file (Flatpak exports them so) counts without a
        // stat. A broken one fails when it is read.
        let Ok(kind) = e.file_type() else { continue };
        if kind.is_dir() {
            if depth < 4 {
                walk(&e.path(), &format!("{prefix}{name}-"), paths, depth + 1);
            }
        } else if let Some(stem) = name.strip_suffix(".desktop") {
            paths
                .entry(format!("{prefix}{stem}"))
                .or_insert_with(|| e.path());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_ids_from_units() {
        let cases = [
            (
                "app-gnome-org.mozilla.firefox-4242.scope",
                Some("org.mozilla.firefox"),
            ),
            (
                "app-flatpak-com.discordapp.Discord-2370014976.scope",
                Some("com.discordapp.Discord"),
            ),
            (
                "app-com.anthropic.Claude-5502.scope",
                Some("com.anthropic.Claude"),
            ),
            (
                "app-com.discordapp.Discord@224f36554a1346eca11c7378b65ad44c.service",
                Some("com.discordapp.Discord"),
            ),
            ("app-org.kde.konsole@abc.service", Some("org.kde.konsole")),
            (
                "app-org.kde.discover.notifier@autostart.service",
                Some("org.kde.discover.notifier"),
            ),
            ("app-org.kde.dolphin.service", Some("org.kde.dolphin")),
            // Escaped dashes are part of the ID, not separators.
            (
                r"app-brave\x2dorigin@8be0aeef71bd4ac99a1bdf73cde0de98.service",
                Some("brave-origin"),
            ),
            (
                r"app-youtube\x2dmusic\x2ddesktop\x2dapp-4042.scope",
                Some("youtube-music-desktop-app"),
            ),
            (
                r"app-kde\x2dlauncher-org.kde.kate-77.scope",
                Some("org.kde.kate"),
            ),
            // A scope's last part is always the random suffix, even when
            // a launcher-less unit leaves out the suffix systemd asks for.
            ("app-gnome-firefox.scope", Some("gnome")),
            // An instance may hold dashes; it's cut at the '@' first.
            ("app-org.kde.kate@a-b-c.service", Some("org.kde.kate")),
            // Not applications.
            ("app.slice", None),
            ("app-flatpak.slice", None),
            ("pipewire.service", None),
            ("session-2.scope", None),
            ("plasma-plasmashell.service", None),
            ("app-.service", None),
            ("app-@x.service", None),
            ("", None),
        ];
        for (unit, want) in cases {
            assert_eq!(app_id(unit).as_deref(), want, "{unit}");
        }
    }

    #[test]
    fn unescapes() {
        assert_eq!(unescape("plain"), "plain");
        assert_eq!(unescape(r"a\x2db\x2dc"), "a-b-c");
        assert_eq!(unescape(r"bad\xzz"), r"bad\xzz");
        assert_eq!(unescape(r"short\x2"), r"short\x2");
        assert_eq!(unescape(r"end\x"), r"end\x");
        assert_eq!(
            unescape(r"plus\x+f"),
            r"plus\x+f",
            "from_str_radix takes a sign"
        );
    }

    #[test]
    fn fallback_names() {
        assert_eq!(fallback_name("com.discordapp.Discord"), "Discord");
        assert_eq!(fallback_name("brave-origin"), "brave-origin");
        assert_eq!(fallback_name("org.kde."), "org.kde.");
        // One dot is a file-ish name, not reverse DNS.
        assert_eq!(fallback_name("kate.bin"), "kate.bin");
    }

    #[test]
    fn locale_keys_from_the_locale() {
        assert_eq!(locale_keys_of("pt_BR.UTF-8"), ["pt_BR", "pt"]);
        assert_eq!(locale_keys_of("de_DE@euro"), ["de_DE", "de"]);
        assert_eq!(locale_keys_of("fr"), ["fr"]);
        assert!(locale_keys_of("C.UTF-8").is_empty());
        assert!(locale_keys_of("POSIX").is_empty());
        assert!(locale_keys_of("").is_empty());
    }

    const KONSOLE: &str = "\
# a comment
[Desktop Entry]
Type=Application
Name=Konsole
Name[de]=Konsole DE
Name[pt_BR]=Konsole BR
Name[pt]=Konsole PT
Icon=utilities-terminal
Categories=Qt;KDE;System;TerminalEmulator;

[Desktop Action NewWindow]
Name=New Window
Icon=window-new
Categories=Other;
";

    #[test]
    fn parses_an_entry() {
        let e = parse_entry(KONSOLE, "org.kde.konsole", &[]).unwrap();
        assert_eq!(e.name, "Konsole");
        assert_eq!(
            e.icon, "utilities-terminal",
            "an action's icon replaced the app's"
        );
        assert!(e.terminal, "an action's categories replaced the app's");
    }

    #[test]
    fn prefers_the_closest_locale_whatever_the_order() {
        let br = ["pt_BR".to_owned(), "pt".to_owned()];
        assert_eq!(parse_entry(KONSOLE, "k", &br).unwrap().name, "Konsole BR");
        let pt = ["pt_PT".to_owned(), "pt".to_owned()];
        assert_eq!(parse_entry(KONSOLE, "k", &pt).unwrap().name, "Konsole PT");
        // Name after Name[..] must not undo the localised one.
        let late = "[Desktop Entry]\nName[pt]=Olá\nName=Hello\n";
        assert_eq!(parse_entry(late, "k", &pt).unwrap().name, "Olá");
    }

    #[test]
    fn hidden_and_nameless_entries() {
        assert!(parse_entry("[Desktop Entry]\nName=X\nHidden=true\n", "x", &[]).is_none());
        let e = parse_entry("[Desktop Entry]\nIcon=x\n", "com.example.Thing", &[]).unwrap();
        assert_eq!(e.name, "Thing");
        // Keys before the main group belong to nothing.
        let e = parse_entry("Name=Wrong\n[Desktop Entry]\nName=Right\n", "x", &[]).unwrap();
        assert_eq!(e.name, "Right");
    }

    fn write(path: &Path, text: &str) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    #[test]
    fn index_finds_entries_in_order_and_subfolders() {
        let a = tempfile::tempdir().unwrap();
        let b = tempfile::tempdir().unwrap();
        write(
            &a.path().join("applications/org.example.App.desktop"),
            "[Desktop Entry]\nName=From A\n",
        );
        write(
            &b.path().join("applications/org.example.App.desktop"),
            "[Desktop Entry]\nName=From B\n",
        );
        write(
            &b.path().join("applications/kde4/konsole.desktop"),
            "[Desktop Entry]\nName=Old Konsole\n",
        );
        write(
            &b.path().join("applications/gone.desktop"),
            "[Desktop Entry]\nName=Gone\nHidden=true\n",
        );
        let mut x = Index::new(vec![a.path().into(), b.path().into()]);
        assert_eq!(x.lookup("org.example.App").unwrap().name, "From A");
        assert_eq!(x.lookup("kde4-konsole").unwrap().name, "Old Konsole");
        assert!(x.lookup("gone").is_none());
        assert!(x.lookup("missing").is_none());
    }

    #[test]
    fn index_reads_localised_names_and_follows_links() {
        let d = tempfile::tempdir().unwrap();
        let apps = d.path().join("applications");
        write(
            &apps.join("real/org.example.L.desktop"),
            "[Desktop Entry]\nName=Plain\nName[fr]=Français\n",
        );
        std::os::unix::fs::symlink(
            apps.join("real/org.example.L.desktop"),
            apps.join("org.example.L.desktop"),
        )
        .unwrap();
        std::os::unix::fs::symlink("/no/such.desktop", apps.join("org.example.Broken.desktop"))
            .unwrap();
        let mut x = Index::new(vec![d.path().into()]);
        x.locales = vec!["fr_FR".into(), "fr".into()];
        assert_eq!(x.lookup("org.example.L").unwrap().name, "Français");
        assert_eq!(x.lookup("real-org.example.L").unwrap().name, "Français");
        assert!(x.lookup("org.example.Broken").is_none());
    }

    #[test]
    fn index_finds_an_app_installed_later() {
        let d = tempfile::tempdir().unwrap();
        let mut x = Index::new(vec![d.path().into()]);
        x.rescan_after = Duration::ZERO;
        assert!(x.lookup("org.example.New").is_none());
        write(
            &d.path().join("applications/org.example.New.desktop"),
            "[Desktop Entry]\nName=New\n",
        );
        assert_eq!(x.lookup("org.example.New").unwrap().name, "New");
    }

    #[test]
    fn index_does_not_rescan_on_every_miss() {
        let d = tempfile::tempdir().unwrap();
        let mut x = Index::new(vec![d.path().into()]);
        assert!(x.lookup("org.example.New").is_none());
        write(
            &d.path().join("applications/org.example.New.desktop"),
            "[Desktop Entry]\nName=New\n",
        );
        assert!(
            x.lookup("org.example.New").is_none(),
            "a miss walked the folders again at once"
        );
    }

    #[test]
    fn data_dirs_include_flatpak_once() {
        let dirs = data_dirs();
        assert!(
            dirs.iter()
                .any(|d| d == Path::new("/var/lib/flatpak/exports/share"))
        );
        for (i, d) in dirs.iter().enumerate() {
            assert!(!dirs[..i].contains(d), "{d:?} listed twice");
        }
    }
}
