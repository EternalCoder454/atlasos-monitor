//! Whether an icon name will draw, without Qt.
//!
//! A desktop file can name an icon the theme doesn't have, and an app with
//! no desktop file has only guesses (its ID, its lower-case name). The Apps
//! table should show the first that draws, not a blank. `QIcon::hasThemeIcon`
//! knows, but it belongs to the GUI thread and the resolver runs on the
//! sampling thread, so this looks the way Qt does: the icon theme, the
//! themes it inherits, then hicolor; in each, the folders its `index.theme`
//! lists, under every `icons/` base directory; and a name with dashes falls
//! back to its shorter forms (`foo-bar` → `foo`).
//!
//! The themes are listed on the first question, and only a hash of each
//! name is kept: Breeze has about 7,000 names in 20,000 files, and the
//! answers are wanted for a few dozen. Most of Breeze's files are links,
//! some of them broken, and checking 12,000 links would double the walk, so
//! a name found only as a link is checked with one `stat` per folder when
//! it is asked about. A miss lists the themes again if the last listing is
//! 30 s old, so an app installed while Atlas runs gets its icon. An icon
//! found only in a `pixmaps/` folder comes back as its path, which draws
//! whatever Qt's fallback paths are.

use std::collections::{HashMap, HashSet};
use std::hash::{BuildHasher, BuildHasherDefault, DefaultHasher};
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

const EXTENSIONS: [&str; 4] = ["svg", "png", "svgz", "xpm"];

/// The biggest icon file that is drawn. Qt decodes it on the GUI thread; a
/// real icon is a few kilobytes, a big SVG a few hundred.
const ICON_MAX: u64 = 4 * 1024 * 1024;

/// The biggest `index.theme` read (Breeze's is about 60 KiB).
const INDEX_MAX: u64 = 1024 * 1024;

/// Whether `path` is an icon worth drawing: a regular file (a link is
/// followed) of a known image type and a sane size.
pub(super) fn icon_file(path: &Path) -> bool {
    let known = path
        .extension()
        .and_then(|e| e.to_str())
        .is_some_and(|e| EXTENSIONS.contains(&e.to_ascii_lowercase().as_str()));
    known && std::fs::metadata(path).is_ok_and(|m| m.is_file() && m.len() <= ICON_MAX)
}

/// How stale the listing may get before a miss lists the themes again.
const RELIST_AFTER: Duration = Duration::from_secs(30);

/// Answers whether an icon will draw. See the module documentation.
#[derive(Debug)]
pub struct IconLookup {
    theme: String,
    bases: Vec<PathBuf>,
    pixmap_dirs: Vec<PathBuf>,
    listing: Option<Listing>,
    relist_after: Duration,
}

/// The themes as last listed.
#[derive(Debug)]
struct Listing {
    at: Instant,
    /// Hashes of the names of icon files.
    files: HashSet<u64>,
    /// Hashes of the names of links, which may be broken.
    links: HashSet<u64>,
    /// Every listed folder that exists, to check a link's name in.
    dirs: Vec<PathBuf>,
    pixmaps: HashMap<String, PathBuf>,
}

/// How an icon is drawn: by name from the theme, or from a file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Icon {
    Name(String),
    Path(PathBuf),
}

impl Icon {
    /// What QML's `Kirigami.Icon.source` takes: the name, or the path.
    pub fn source(&self) -> std::borrow::Cow<'_, str> {
        match self {
            Icon::Name(n) => n.into(),
            Icon::Path(p) => p.to_string_lossy(),
        }
    }
}

impl IconLookup {
    /// A lookup in `theme` (Qt's `QIcon::themeName()`, `breeze` on Plasma)
    /// across `data_dirs` (see [`super::desktop::data_dirs`]).
    pub fn new(theme: &str, data_dirs: &[PathBuf]) -> Self {
        let mut bases = Vec::new();
        if let Some(home) = std::env::var_os("HOME").filter(|h| !h.is_empty()) {
            bases.push(Path::new(&home).join(".icons"));
        }
        bases.extend(data_dirs.iter().map(|d| d.join("icons")));
        Self {
            theme: theme.to_owned(),
            bases,
            pixmap_dirs: data_dirs.iter().map(|d| d.join("pixmaps")).collect(),
            listing: None,
            relist_after: RELIST_AFTER,
        }
    }

    pub fn theme(&self) -> &str {
        &self.theme
    }

    /// The same lookup in another theme, listed afresh.
    pub fn with_theme(&self, theme: &str) -> Self {
        Self {
            theme: theme.to_owned(),
            bases: self.bases.clone(),
            pixmap_dirs: self.pixmap_dirs.clone(),
            listing: None,
            relist_after: self.relist_after,
        }
    }

    /// The `icons/` folders this looks in, for `QIcon::setThemeSearchPaths`:
    /// Qt takes its own from `XDG_DATA_DIRS`, which can miss Flatpak's
    /// exports, and an icon found here must draw there too.
    pub fn search_paths(&self) -> impl Iterator<Item = &Path> {
        self.bases
            .iter()
            .map(PathBuf::as_path)
            .filter(|p| p.is_dir())
    }

    /// How to draw `name`, or `None` if nothing will. An absolute path is
    /// taken as is, if the file exists. A name with dashes falls back to its
    /// shorter forms, as Qt's lookup does.
    pub fn find(&mut self, name: &str) -> Option<Icon> {
        self.lookup(name, true)
    }

    /// As [`find`](Self::find), without the dash fallback: for trying
    /// several names, where a later one found as it is should beat an
    /// earlier one found shortened.
    pub fn find_exact(&mut self, name: &str) -> Option<Icon> {
        self.lookup(name, false)
    }

    fn lookup(&mut self, name: &str, fallback: bool) -> Option<Icon> {
        if name.is_empty() {
            return None;
        }
        if name.starts_with('/') {
            return icon_file(Path::new(name)).then(|| Icon::Path(name.into()));
        }
        if self.listing.is_none() {
            self.list();
        }
        let listing = self.listing.as_ref()?;
        if let Some(icon) = listing.find(name, fallback) {
            return Some(icon);
        }
        if listing.at.elapsed() < self.relist_after {
            return None;
        }
        self.list();
        self.listing.as_ref()?.find(name, fallback)
    }

    fn list(&mut self) {
        let mut l = Listing {
            at: Instant::now(),
            files: HashSet::new(),
            links: HashSet::new(),
            dirs: Vec::new(),
            pixmaps: HashMap::new(),
        };
        for theme in theme_chain(&self.theme, &self.bases) {
            // Qt reads the first index.theme it finds and looks for its
            // folders under every base: Flatpak's exported hicolor has no
            // index.theme of its own.
            let Some(index) = read_index(&theme, &self.bases) else {
                continue;
            };
            for base in &self.bases {
                for sub in &index.dirs {
                    let dir = base.join(&theme).join(sub);
                    let Ok(rd) = std::fs::read_dir(&dir) else {
                        continue;
                    };
                    for e in rd.flatten() {
                        let file = e.file_name();
                        let Some(stem) = file.to_str().and_then(icon_stem) else {
                            continue;
                        };
                        match e.file_type() {
                            Ok(t) if t.is_file() => l.files.insert(hash(stem)),
                            Ok(t) if t.is_symlink() => l.links.insert(hash(stem)),
                            _ => false,
                        };
                    }
                    l.dirs.push(dir);
                }
            }
        }
        for dir in &self.pixmap_dirs {
            let Ok(rd) = std::fs::read_dir(dir) else {
                continue;
            };
            for e in rd.flatten() {
                let file = e.file_name();
                if let Some(stem) = file.to_str().and_then(icon_stem)
                    && icon_file(&e.path())
                {
                    l.pixmaps.entry(stem.to_owned()).or_insert_with(|| e.path());
                }
            }
        }
        self.listing = Some(l);
    }

    #[cfg(test)]
    fn relist_now(&mut self) {
        self.relist_after = Duration::ZERO;
    }
}

impl Listing {
    fn find(&self, name: &str, fallback: bool) -> Option<Icon> {
        // The name, then without its last dash-separated part, as Qt falls
        // back: "utilities-terminal-root" draws as "utilities-terminal".
        let mut candidate = name;
        loop {
            let h = hash(candidate);
            if self.files.contains(&h) || (self.links.contains(&h) && self.link_resolves(candidate))
            {
                return Some(Icon::Name(name.to_owned()));
            }
            match candidate.rsplit_once('-') {
                Some((shorter, _)) if fallback && !shorter.is_empty() => candidate = shorter,
                _ => break,
            }
        }
        self.pixmaps.get(name).cloned().map(Icon::Path)
    }

    /// Whether some link named `stem` leads to a file.
    fn link_resolves(&self, stem: &str) -> bool {
        self.dirs.iter().any(|d| {
            EXTENSIONS
                .iter()
                .any(|ext| d.join(format!("{stem}.{ext}")).is_file())
        })
    }
}

fn hash(name: &str) -> u64 {
    BuildHasherDefault::<DefaultHasher>::default().hash_one(name)
}

/// `foo.svg` → `foo`; `None` for anything that isn't an icon file.
fn icon_stem(file: &str) -> Option<&str> {
    let (stem, ext) = file.rsplit_once('.')?;
    (EXTENSIONS.contains(&ext) && !stem.is_empty()).then_some(stem)
}

/// What an `index.theme` says.
#[derive(Debug, Default, PartialEq)]
struct Index {
    inherits: Vec<String>,
    /// `Directories` and `ScaledDirectories`: the folders icons are in.
    dirs: Vec<String>,
}

/// The first `index.theme` for `theme` among `bases`.
fn read_index(theme: &str, bases: &[PathBuf]) -> Option<Index> {
    bases
        .iter()
        .find_map(|b| crate::files::read_text_capped(&b.join(theme).join("index.theme"), INDEX_MAX))
        .map(|text| parse_index(&text))
}

/// The `[Icon Theme]` group of an `index.theme`.
fn parse_index(text: &str) -> Index {
    let list = |v: &str| -> Vec<String> {
        v.split(',')
            .map(str::trim)
            // A folder outside the theme is no folder of it.
            .filter(|t| !t.is_empty() && !t.starts_with('/') && !t.split('/').any(|c| c == ".."))
            .map(str::to_owned)
            .collect()
    };
    let mut index = Index::default();
    let mut inside = false;
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line == "[Icon Theme]";
            continue;
        }
        if !inside {
            continue;
        }
        let Some((key, val)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "Inherits" => index.inherits = list(val),
            "Directories" | "ScaledDirectories" => {
                for d in list(val) {
                    if !index.dirs.contains(&d) {
                        index.dirs.push(d);
                    }
                }
            }
            _ => {}
        }
    }
    index
}

/// `theme`, the themes it inherits (depth first, each once), and hicolor
/// last, as the specification says.
fn theme_chain(theme: &str, bases: &[PathBuf]) -> Vec<String> {
    let mut chain: Vec<String> = Vec::new();
    let mut queue = vec![theme.to_owned()];
    while let Some(t) = queue.pop() {
        if t.is_empty()
            || t == "hicolor"
            || t.contains('/')
            || t == "."
            || t == ".."
            || chain.contains(&t)
            || chain.len() >= 8
        {
            continue;
        }
        let inherits = read_index(&t, bases)
            .map(|i| i.inherits)
            .unwrap_or_default();
        chain.push(t);
        queue.extend(inherits.into_iter().rev());
    }
    chain.push("hicolor".into());
    chain
}

#[cfg(test)]
mod tests {
    use super::*;

    fn touch(path: &Path) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, "").unwrap();
    }

    fn theme(icons: &Path, name: &str, index: &str) {
        std::fs::create_dir_all(icons.join(name)).unwrap();
        std::fs::write(icons.join(name).join("index.theme"), index).unwrap();
    }

    #[test]
    fn stems() {
        assert_eq!(icon_stem("firefox.svg"), Some("firefox"));
        assert_eq!(icon_stem("org.kde.konsole.png"), Some("org.kde.konsole"));
        assert_eq!(icon_stem("index.theme"), None);
        assert_eq!(icon_stem("icon-theme.cache"), None);
        assert_eq!(icon_stem(".svg"), None);
        assert_eq!(icon_stem("48x48"), None);
    }

    #[test]
    fn parses_index_theme() {
        let text = "[Icon Theme]\nName=Breeze Dark\nInherits=breeze, hicolor\n\
                    Directories=apps/48,actions/16\nScaledDirectories=apps/48@2x,apps/48,/abs,../out\n\n\
                    [apps/48]\nInherits=nope\nDirectories=nope\n";
        let i = parse_index(text);
        assert_eq!(i.inherits, ["breeze", "hicolor"]);
        assert_eq!(i.dirs, ["apps/48", "actions/16", "apps/48@2x"]);
        assert_eq!(parse_index("[Other]\nInherits=x\n"), Index::default());
    }

    #[test]
    fn finds_icons_the_way_qt_does() {
        let data = tempfile::tempdir().unwrap();
        let flatpak = tempfile::tempdir().unwrap();
        let icons = data.path().join("icons");
        theme(
            &icons,
            "child",
            "[Icon Theme]\nInherits=parent\nDirectories=apps/48\n",
        );
        theme(
            &icons,
            "parent",
            "[Icon Theme]\nInherits=hicolor\nDirectories=preferences/32\n",
        );
        theme(
            &icons,
            "hicolor",
            "[Icon Theme]\nDirectories=48x48/apps,scalable/apps\n",
        );
        theme(&icons, "unrelated", "[Icon Theme]\nDirectories=apps/48\n");
        touch(&icons.join("child/apps/48/own.svg"));
        touch(&icons.join("child/apps/48/utilities-terminal.svg"));
        touch(&icons.join("parent/preferences/32/inherited.svg"));
        touch(&icons.join("hicolor/48x48/apps/fallback.png"));
        touch(&icons.join("unrelated/apps/48/elsewhere.svg"));
        // In the theme, but in no folder it lists, or in its root.
        touch(&icons.join("child/unlisted/48/stray.svg"));
        touch(&icons.join("child/rootfile.svg"));
        // Flatpak's hicolor has no index.theme; hicolor's own is used.
        touch(
            &flatpak
                .path()
                .join("icons/hicolor/scalable/apps/com.discordapp.Discord.svg"),
        );
        touch(&data.path().join("pixmaps/oldapp.xpm"));
        // Links, one good and one broken.
        let apps = icons.join("child/apps/48");
        std::os::unix::fs::symlink("own.svg", apps.join("linked.svg")).unwrap();
        std::os::unix::fs::symlink("gone.svg", apps.join("broken.svg")).unwrap();

        let mut x = IconLookup::new("child", &[data.path().into(), flatpak.path().into()]);
        for name in [
            "own",
            "inherited",
            "fallback",
            "com.discordapp.Discord",
            "linked",
            "utilities-terminal-root",
        ] {
            assert_eq!(x.find(name), Some(Icon::Name(name.into())), "{name}");
        }
        for name in [
            "elsewhere",
            "stray",
            "rootfile",
            "broken",
            "missing",
            "",
            "-",
        ] {
            assert_eq!(x.find(name), None, "{name}");
        }
        assert_eq!(x.find_exact("utilities-terminal-root"), None);
        assert_eq!(x.find_exact("own"), Some(Icon::Name("own".into())));
        let pixmap = data.path().join("pixmaps/oldapp.xpm");
        assert_eq!(x.find("oldapp"), Some(Icon::Path(pixmap.clone())));
        assert_eq!(x.find(pixmap.to_str().unwrap()), Some(Icon::Path(pixmap)));
        assert_eq!(x.find("/no/such/icon.png"), None);
        assert!(x.search_paths().any(|p| p == flatpak.path().join("icons")));
    }

    #[test]
    fn finds_an_icon_installed_later() {
        let data = tempfile::tempdir().unwrap();
        let icons = data.path().join("icons");
        theme(
            &icons,
            "hicolor",
            "[Icon Theme]\nDirectories=scalable/apps\n",
        );
        let mut x = IconLookup::new("breeze", &[data.path().into()]);
        assert_eq!(x.find("org.example.New"), None);
        touch(&icons.join("hicolor/scalable/apps/org.example.New.svg"));
        assert_eq!(
            x.find("org.example.New"),
            None,
            "a miss listed the themes again at once"
        );
        x.relist_now();
        assert_eq!(
            x.find("org.example.New"),
            Some(Icon::Name("org.example.New".into()))
        );
    }

    #[test]
    fn a_theme_that_inherits_itself_ends() {
        let data = tempfile::tempdir().unwrap();
        let icons = data.path().join("icons");
        theme(&icons, "a", "[Icon Theme]\nInherits=b\n");
        theme(&icons, "b", "[Icon Theme]\nInherits=a,b,../x,..,.\n");
        assert_eq!(
            theme_chain("a", std::slice::from_ref(&icons)),
            ["a", "b", "hicolor"]
        );
        assert_eq!(theme_chain("..", &[icons]), ["hicolor"]);
    }
}

#[cfg(test)]
mod props {
    use super::*;
    use crate::hostile;
    use proptest::prelude::*;

    proptest! {
        /// An `index.theme` can list folders and parents as it likes: none of
        /// the folders climbs out of the theme or is absolute, and nothing
        /// panics.
        #[test]
        fn index_themes_cannot_leave_their_folder(text in hostile::string_to(400), dirs in proptest::collection::vec(hostile::string_to(30), 0..8)) {
            let text = format!("[Icon Theme]\nInherits={}\nDirectories={}\n{text}", dirs.join(","), dirs.join(","));
            let index = parse_index(&text);
            for d in index.dirs.iter().chain(&index.inherits) {
                if index.dirs.contains(d) {
                    prop_assert!(!d.starts_with('/') && !d.split('/').any(|c| c == ".."), "{d:?}");
                }
            }
            let _ = parse_index(&format!("{text}\n{text}"));
        }

        /// A stem is a file name without its image extension.
        #[test]
        fn stems_are_image_files(file in hostile::string_to(60)) {
            if let Some(stem) = icon_stem(&file) {
                prop_assert!(!stem.is_empty());
                prop_assert!(file.starts_with(stem));
                let ext = file.rsplit_once('.').map(|(_, e)| e).unwrap_or_default();
                prop_assert!(EXTENSIONS.contains(&ext));
            }
        }

        /// A theme name from a file cannot name a folder elsewhere.
        #[test]
        fn theme_chains_name_folders_of_the_bases(theme in hostile::string_to(40)) {
            let chain = theme_chain(&theme, &[]);
            prop_assert_eq!(chain.last().map(String::as_str), Some("hicolor"));
            prop_assert!(chain.iter().all(|t| !t.contains('/') && t != ".." && t != "."));
        }
    }
}
