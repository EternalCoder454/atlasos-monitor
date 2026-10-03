//! Containers: podman's, which toolbox and distrobox make too. A container's
//! processes run in its cgroup, `libpod-<ID>.scope`
//! ([`crate::process::container_from_cgroup`]), and the Apps table shows
//! them as one row under the container's name rather than as scattered
//! processes. The commands that start them (`podman exec`, `toolbox enter`)
//! and podman's `conmon` stay in the terminal or service that ran them.
//!
//! The name is podman's own record: its storage keeps a list of the
//! containers it made, `containers.json`, and of those made with `--rm`,
//! `volatile-containers.json`, side by side in `<driver>-containers/` under
//! the storage's graph root. That is the user's storage, so a rootful
//! container (whose storage only root may read) is named by its short ID.
//!
//! Reading the lists from `/proc/<pid>/root` would not do: a container sees
//! its own files, and podman writes the name into `/run/.containerenv` only
//! for `--privileged` ones.

use std::collections::HashMap;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

/// The icon of a container's row. Breeze draws a container; a theme without
/// it leaves the row without an icon.
pub const ICON: &str = "preferences-virtualization-container";

/// Far more than a real container list holds (about 1 KiB a container).
const LIST_MAX: u64 = 16 * 1024 * 1024;

/// The lists in each of a storage's `<driver>-containers/` folders.
const LISTS: [&str; 2] = ["containers.json", "volatile-containers.json"];

/// Container names by ID, from podman's lists, read again when a container
/// it doesn't know turns up, or [`refresh`](Self::refresh) is called, and a
/// list has changed since.
///
/// Owned by the sampling thread, inside [`super::Resolver`], which asks once
/// per container and refreshes now and then for renames.
#[derive(Debug, Default)]
pub struct Store {
    roots: Vec<PathBuf>,
    names: HashMap<Box<str>, Box<str>>,
    /// Each list's path, and its modification time and size when read:
    /// `None` for one that couldn't be, so it is tried again.
    read: Vec<(PathBuf, Option<Stamp>)>,
    /// How many times the lists have been read.
    reads: u64,
}

type Stamp = (SystemTime, u64);

impl Store {
    /// A store over the containers of the given graph roots.
    pub fn new(roots: Vec<PathBuf>) -> Self {
        Self {
            roots,
            ..Self::default()
        }
    }

    /// The user's own containers ([`graph_root`]).
    pub fn for_user() -> Self {
        Self::new(graph_root().into_iter().collect())
    }

    /// The name of the container `id` (its 64 digits), or `None` if podman's
    /// lists don't have it.
    pub fn name(&mut self, id: &str) -> Option<&str> {
        if !self.names.contains_key(id) {
            self.refresh();
        }
        self.names.get(id).map(|n| &**n)
    }

    /// How many times the lists have been read, by [`name`](Self::name) or
    /// [`refresh`](Self::refresh): names given out before the count moved
    /// may be out of date.
    pub fn reads(&self) -> u64 {
        self.reads
    }

    /// Reads the lists again if any has changed since they were read (new,
    /// gone, rewritten, or unreadable last time). Says whether it did.
    /// Costs a folder listing and a `stat` per list.
    pub fn refresh(&mut self) -> bool {
        let now = self.lists();
        let same = now.len() == self.read.len()
            && now
                .iter()
                .zip(&self.read)
                .all(|((p, a), (q, b))| p == q && a.is_some() && a == b);
        if !same {
            self.reload(now);
        }
        !same
    }

    /// Builds the names afresh from `lists`. A list that can't be read is
    /// marked to be tried again, and keeps the names it gave before.
    fn reload(&mut self, mut lists: Vec<(PathBuf, Option<Stamp>)>) {
        let mut names = HashMap::new();
        let mut failed = false;
        for (path, stamp) in &mut lists {
            match read_list(path) {
                Some(text) => {
                    for (id, name) in parse(&text) {
                        names.insert(id.into(), name.into());
                    }
                }
                None => {
                    *stamp = None;
                    failed = true;
                }
            }
        }
        if failed {
            for (id, name) in self.names.drain() {
                names.entry(id).or_insert(name);
            }
        }
        self.names = names;
        self.read = lists;
        self.reads += 1;
    }

    /// Every list under the roots, with its modification time and size.
    fn lists(&self) -> Vec<(PathBuf, Option<Stamp>)> {
        let mut out = Vec::new();
        for root in &self.roots {
            let mut dirs: Vec<PathBuf> = fs::read_dir(root)
                .into_iter()
                .flatten()
                .filter_map(|e| e.ok())
                .filter(|e| e.file_name().to_string_lossy().ends_with("-containers"))
                .map(|e| e.path())
                .collect();
            dirs.sort();
            for dir in dirs {
                for list in LISTS {
                    let path = dir.join(list);
                    if let Ok(m) = fs::metadata(&path) {
                        let stamp = m.modified().ok().map(|t| (t, m.len()));
                        out.push((path, stamp));
                    }
                }
            }
        }
        out
    }
}

/// A list's text; `None` if it can't be read whole.
fn read_list(path: &Path) -> Option<String> {
    let mut text = String::new();
    fs::File::open(path)
        .ok()?
        .take(LIST_MAX + 1)
        .read_to_string(&mut text)
        .ok()?;
    (text.len() as u64 <= LIST_MAX).then_some(text)
}

/// Each container's ID and first name in a podman container list. An entry
/// that isn't shaped as podman writes them is skipped, not the whole list.
pub fn parse(text: &str) -> Vec<(String, String)> {
    let Ok(serde_json::Value::Array(entries)) = serde_json::from_str(text) else {
        return Vec::new();
    };
    entries
        .iter()
        .filter_map(|e| {
            let id = e.get("id")?.as_str().filter(|i| !i.is_empty())?;
            let name = e
                .get("names")?
                .as_array()?
                .iter()
                .filter_map(|n| n.as_str())
                .find(|n| !n.is_empty())?;
            Some((id.to_owned(), name.to_owned()))
        })
        .collect()
}

/// The first 12 digits of a container's ID, as podman shows it.
pub fn short_id(id: &str) -> &str {
    id.get(..12).unwrap_or(id)
}

/// The user's podman storage, as podman picks it for a user: the
/// `graphroot` of the user's `storage.conf` (`$CONTAINERS_STORAGE_CONF`, or
/// `containers/storage.conf` in the XDG config home) when there is one, else
/// the system's `rootless_storage_path` (`/etc/containers/storage.conf`, or
/// `/usr/share/containers/`), else `containers/storage` in the XDG data home.
pub fn graph_root() -> Option<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    let home = var("HOME").map(PathBuf::from);
    let user_conf = var("CONTAINERS_STORAGE_CONF")
        .map(PathBuf::from)
        .or_else(|| {
            var("XDG_CONFIG_HOME")
                .map(PathBuf::from)
                .or_else(|| home.as_ref().map(|h| h.join(".config")))
                .map(|c| c.join("containers/storage.conf"))
        });
    let configured = match user_conf.and_then(|c| fs::read_to_string(c).ok()) {
        Some(text) => setting(&text, "graphroot"),
        None => [
            "/etc/containers/storage.conf",
            "/usr/share/containers/storage.conf",
        ]
        .iter()
        .find_map(|c| fs::read_to_string(c).ok())
        .and_then(|text| setting(&text, "rootless_storage_path")),
    };
    let expanded = configured.and_then(|v| {
        let user = var("USER").map(|u| u.to_string_lossy().into_owned());
        expand(
            &v,
            home.as_deref(),
            user.as_deref(),
            rustix::process::getuid().as_raw(),
        )
    });
    if let Some(root) = expanded {
        return Some(root);
    }
    var("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| home.map(|h| h.join(".local/share")))
        .map(|d| d.join("containers/storage"))
}

/// The quoted value of `key` in a `storage.conf`'s `[storage]` table.
fn setting(text: &str, key: &str) -> Option<String> {
    let mut table = "";
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            table = line.split('#').next().unwrap_or("").trim();
            continue;
        }
        if table != "[storage]" {
            continue;
        }
        let Some((k, value)) = line.split_once('=') else {
            continue;
        };
        if k.trim() != key {
            continue;
        }
        let value = value.trim();
        return value
            .strip_prefix('"')
            .and_then(|v| v.split_once('"'))
            .or_else(|| value.strip_prefix('\'').and_then(|v| v.split_once('\'')))
            .map(|(v, _)| v.to_owned());
    }
    None
}

/// A configured path with podman's `$HOME`, `$USER` and `$UID` filled in;
/// `None` unless that makes it absolute.
fn expand(value: &str, home: Option<&Path>, user: Option<&str>, uid: u32) -> Option<PathBuf> {
    let mut out = value.to_owned();
    for (name, with) in [
        ("$HOME", home.and_then(Path::to_str)),
        ("$USER", user),
        ("$UID", Some(&*uid.to_string())),
    ] {
        if out.contains(name) {
            out = out.replace(name, with?);
        }
    }
    out.starts_with('/').then(|| out.into())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    const LIST: &str = include_str!("../../tests/fixtures/containers_json");
    const TOOLBOX: &str = "2414f7b322e3a727aae64037b633eefd3f14b985dff8ddcd38915e66ffd4e539";
    const VOLATILE: &str = "cefc74515359953ba0e49a9c4e8ab07c48b821d8d4e1f9cd254464d0c3c5b91c";

    #[test]
    fn lists() {
        let got = parse(LIST);
        assert_eq!(got.len(), 2, "the entry without a name is left out");
        assert_eq!(got[0], (TOOLBOX.into(), "fedora-toolbox-44".into()));
        assert_eq!(got[1].1, "ubuntu");
        assert!(parse("").is_empty());
        assert!(parse("{\"id\": \"x\"}").is_empty());
        assert!(parse("[{\"names\": [\"no-id\"]}]").is_empty());
    }

    #[test]
    fn short_ids() {
        assert_eq!(short_id(TOOLBOX), "2414f7b322e3");
        assert_eq!(short_id("abc"), "abc");
    }

    fn write(path: &Path, text: &str, modified: SystemTime) {
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, text).unwrap();
        let f = fs::File::options().write(true).open(path).unwrap();
        f.set_modified(modified).unwrap();
    }

    /// A container made after the lists were read is found once they change,
    /// and an unknown one doesn't read them again while they haven't.
    #[test]
    fn store_follows_the_lists() {
        let dir = tempfile::tempdir().unwrap();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let lists = dir.path().join("overlay-containers");
        write(&lists.join("containers.json"), LIST, t0);
        let mut store = Store::new(vec![dir.path().into()]);
        assert_eq!(store.name(TOOLBOX), Some("fedora-toolbox-44"));
        assert_eq!(store.name(VOLATILE), None);
        assert!(!store.refresh());

        let volatile = format!("[{{\"id\": \"{VOLATILE}\", \"names\": [\"atlas-ctest\"]}}]");
        write(&lists.join("volatile-containers.json"), &volatile, t0);
        assert_eq!(store.name(VOLATILE), Some("atlas-ctest"));
        assert_eq!(store.name(TOOLBOX), Some("fedora-toolbox-44"));

        // Renamed in place, same size, a second later.
        let renamed = volatile.replace("atlas-ctest", "atlas-ctes2");
        let t1 = t0 + Duration::from_secs(1);
        write(&lists.join("volatile-containers.json"), &renamed, t1);
        assert_eq!(
            store.name(VOLATILE),
            Some("atlas-ctest"),
            "known: not re-read"
        );
        assert_eq!(store.name("0".repeat(64).as_str()), None);
        assert_eq!(store.name(VOLATILE), Some("atlas-ctes2"));
    }

    #[test]
    fn no_storage() {
        let dir = tempfile::tempdir().unwrap();
        let mut store = Store::new(vec![dir.path().join("missing")]);
        assert_eq!(store.name(TOOLBOX), None);
        let mut store = Store::default();
        assert_eq!(store.name(TOOLBOX), None);
    }

    #[test]
    fn storage_conf() {
        for (text, want) in [
            (
                "[storage]\ngraphroot = \"/srv/podman\"\n",
                Some("/srv/podman"),
            ),
            (
                "[storage] # mine\ndriver = \"overlay\"\n graphroot='/srv/p' # mine\n",
                Some("/srv/p"),
            ),
            ("[storage.options]\ngraphroot = \"/srv/podman\"\n", None),
            ("[storage]\nrunroot = \"/run/x\"\n", None),
            ("", None),
        ] {
            assert_eq!(setting(text, "graphroot").as_deref(), want, "{text}");
        }
        let home = Some(Path::new("/home/ada"));
        for (value, want) in [
            ("/srv/podman", Some("/srv/podman")),
            ("$HOME/.containers", Some("/home/ada/.containers")),
            (
                "/var/tmp/$USER-$UID/storage",
                Some("/var/tmp/ada-1000/storage"),
            ),
            ("relative", None),
        ] {
            assert_eq!(
                expand(value, home, Some("ada"), 1000),
                want.map(PathBuf::from),
                "{value}"
            );
        }
        assert_eq!(
            expand("$HOME/x", None, None, 1000),
            None,
            "no home to fill in"
        );
    }

    /// A list podman is halfway through rewriting, or can't be read, keeps
    /// the names it gave, and is read again once it can be.
    #[test]
    fn a_bad_read_loses_nothing() {
        let dir = tempfile::tempdir().unwrap();
        let t0 = SystemTime::UNIX_EPOCH + Duration::from_secs(1_000_000);
        let list = dir.path().join("overlay-containers/containers.json");
        write(&list, LIST, t0);
        let mut store = Store::new(vec![dir.path().into()]);
        assert_eq!(store.name(TOOLBOX), Some("fedora-toolbox-44"));

        // Unreadable: a folder where the file was.
        fs::remove_file(&list).unwrap();
        fs::create_dir(&list).unwrap();
        assert!(store.refresh());
        assert_eq!(store.name(TOOLBOX), Some("fedora-toolbox-44"));
        assert!(store.refresh(), "tried again while it can't be read");

        fs::remove_dir(&list).unwrap();
        let volatile = format!("[{{\"id\": \"{VOLATILE}\", \"names\": [\"atlas-ctest\"]}}]");
        write(&list, &volatile, t0);
        assert!(store.refresh());
        assert_eq!(store.name(VOLATILE), Some("atlas-ctest"));
        assert_eq!(store.name(TOOLBOX), None, "gone from the list, gone");
        assert!(!store.refresh());
    }

    #[test]
    fn odd_entries_are_skipped_alone() {
        let text = format!(
            "[{{\"id\": \"{TOOLBOX}\", \"names\": null}}, {{\"id\": 7, \"names\": [\"x\"]}},\
             {{\"id\": \"{VOLATILE}\", \"names\": [3, \"\", \"good\"]}}]"
        );
        assert_eq!(parse(&text), [(VOLATILE.to_owned(), "good".to_owned())]);
    }
}
