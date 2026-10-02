//! Flatpak applications: where one is installed, and where a sandboxed
//! process's program is on the host.
//!
//! Most of a Flatpak needs nothing special. Each launch runs in a scope of
//! its own, `app-flatpak-<ID>-<RANDOM>.scope`, which
//! [`desktop::app_id`](super::desktop::app_id) reads like any other, and
//! its desktop file and icons are exported to its installation's
//! `exports/share`, which [`desktop::data_dirs`](super::desktop::data_dirs)
//! searches. Its files are the difference: inside the sandbox the app is
//! mounted at `/app` and its runtime at `/usr`, so `/proc/<pid>/exe` reads
//! `/app/discord/Discord`, which means nothing on the host. Flatpak writes
//! `/.flatpak-info` into every sandbox, saying where both really are.
//!
//! Everything here reads files on request, for Open File Location: run it
//! off the GUI thread (DESIGN.md, Threading rule).

use std::fs;
use std::io::Read;
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};

/// How the unit of every Flatpak launch starts.
pub const UNIT_PREFIX: &str = "app-flatpak-";

/// Far more than a real `.flatpak-info` holds (a few KiB).
const INFO_MAX: u64 = 64 * 1024;

/// A running Flatpak instance, as its `/.flatpak-info` describes it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Instance {
    /// The application ID: `com.discordapp.Discord`.
    pub id: String,
    /// The app's deployed files, mounted at `/app` inside:
    /// `<installation>/app/<ID>/<arch>/<branch>/<commit>/files`.
    pub app_path: PathBuf,
    /// The runtime's, mounted at `/usr` inside.
    pub runtime_path: Option<PathBuf>,
}

impl Instance {
    /// The instance `pid` runs in. `None` for a process outside any
    /// sandbox, another user's, or one that has gone.
    ///
    /// Flatpak writes the file, but any process of ours can make a mount
    /// namespace of its own with whatever it likes at that path: it is read
    /// with a cap, only if it is a regular file, and callers check the ID.
    pub fn of(pid: u32) -> Option<Self> {
        Self::parse(&read_info(Path::new(&format!(
            "/proc/{pid}/root/.flatpak-info"
        )))?)
    }

    /// Reads a `.flatpak-info`. `None` if it lacks the ID or the app's path.
    pub fn parse(text: &str) -> Option<Self> {
        let mut group = "";
        let (mut id, mut app, mut runtime) = (None, None, None);
        for line in text.lines() {
            let line = line.trim();
            if line.starts_with('[') {
                group = line;
                continue;
            }
            let Some((key, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            match (group, key.trim()) {
                ("[Application]", "name") => id = Some(value),
                ("[Instance]", "app-path") => app = Some(value),
                ("[Instance]", "runtime-path") => runtime = Some(value),
                _ => {}
            }
        }
        let absolute = |p: &&str| p.starts_with('/');
        Some(Self {
            id: id.filter(|i| !i.is_empty())?.to_owned(),
            app_path: app.filter(absolute)?.into(),
            runtime_path: runtime.filter(absolute).map(PathBuf::from),
        })
    }

    /// The app's install folder: its branch's `active/files`, which stays
    /// put when the app updates, rather than this commit's folder, which an
    /// update removes. This commit's folder when there is no `active`.
    pub fn install_folder(&self) -> PathBuf {
        let active = self
            .app_path
            .parent() // the commit
            .and_then(Path::parent) // the branch
            .map(|branch| branch.join("active/files"));
        match active {
            Some(a) if a.is_dir() => a,
            _ => self.app_path.clone(),
        }
    }

    /// Where a path inside the sandbox is on the host: `/app/…` in the
    /// app's files, `/usr/…` in the runtime's. `None` for any other path,
    /// which the sandbox shares with the host or keeps in memory.
    pub fn host_path(&self, inside: &Path) -> Option<PathBuf> {
        if let Ok(rest) = inside.strip_prefix("/app") {
            return Some(self.app_path.join(rest));
        }
        let rest = inside.strip_prefix("/usr").ok()?;
        Some(self.runtime_path.as_ref()?.join(rest))
    }
}

/// A `.flatpak-info`'s text, if `path` is a regular file. Opened without
/// blocking, which a regular file ignores and a FIFO would otherwise do in
/// `open` itself, until something writes to it.
fn read_info(path: &Path) -> Option<String> {
    let file = fs::OpenOptions::new()
        .read(true)
        .custom_flags(libc::O_NONBLOCK)
        .open(path)
        .ok()?;
    if !file.metadata().ok()?.is_file() {
        return None;
    }
    let mut text = String::new();
    file.take(INFO_MAX).read_to_string(&mut text).ok()?;
    Some(text)
}

/// Flatpak's installations in the order `flatpak` searches them: the
/// user's (`$FLATPAK_USER_DIR`, or `flatpak` in the XDG data home), the
/// system's (`$FLATPAK_SYSTEM_DIR`, or `/var/lib/flatpak`), then any that
/// the `installations.d` folders add.
pub fn installations() -> Vec<PathBuf> {
    let var = |name| std::env::var_os(name).filter(|v| !v.is_empty());
    let user = var("FLATPAK_USER_DIR").map(PathBuf::from).or_else(|| {
        var("XDG_DATA_HOME")
            .map(PathBuf::from)
            .or_else(|| var("HOME").map(|h| Path::new(&h).join(".local/share")))
            .map(|d| d.join("flatpak"))
    });
    let system = var("FLATPAK_SYSTEM_DIR").map_or_else(|| "/var/lib/flatpak".into(), PathBuf::from);

    let mut out: Vec<PathBuf> = user.into_iter().chain([system]).collect();
    for dir in INSTALLATIONS_D {
        let mut confs: Vec<PathBuf> = fs::read_dir(dir)
            .into_iter()
            .flatten()
            .filter_map(|e| Some(e.ok()?.path()))
            .filter(|p| p.extension().is_some_and(|x| x == "conf"))
            .collect();
        confs.sort();
        for conf in confs {
            if let Ok(text) = fs::read_to_string(conf) {
                out.extend(extra_installations(&text));
            }
        }
    }
    out
}

/// Where Flatpak looks for more installations, as it orders them.
const INSTALLATIONS_D: [&str; 3] = [
    "/etc/flatpak/installations.d",
    "/run/flatpak/installations.d",
    "/usr/share/flatpak/installations.d",
];

/// The `Path`s of an `installations.d` file's `[Installation "…"]` groups.
fn extra_installations(text: &str) -> Vec<PathBuf> {
    let mut inside = false;
    let mut out = Vec::new();
    for line in text.lines().map(str::trim) {
        if line.starts_with('[') {
            inside = line.starts_with("[Installation ");
        } else if inside
            && let Some((key, path)) = line.split_once('=')
            && key.trim() == "Path"
            && let path = path.trim()
            && path.starts_with('/')
        {
            out.push(path.into());
        }
    }
    out
}

/// The install folder of the Flatpak `id` in the first of `installations`
/// that has it: `<installation>/app/<ID>/current/active/files`, the branch
/// `flatpak run` picks. For an app with no instance to ask; one that is
/// running knows its own ([`Instance::install_folder`]).
pub fn install_folder(id: &str, installations: &[PathBuf]) -> Option<PathBuf> {
    // The ID comes from a unit name: never let it climb out of `app/`.
    if id.is_empty() || id.starts_with('.') || id.contains('/') {
        return None;
    }
    installations
        .iter()
        .map(|i| i.join("app").join(id).join("current/active/files"))
        .find(|f| f.is_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    const INFO: &str = include_str!("../../tests/fixtures/flatpak_info");

    fn instance(app: &Path) -> Instance {
        Instance {
            id: "com.example.App".into(),
            app_path: app.into(),
            runtime_path: Some("/rt/files".into()),
        }
    }

    #[test]
    fn parses_the_fixture() {
        let i = Instance::parse(INFO).unwrap();
        assert_eq!(i.id, "com.discordapp.Discord");
        let app = "/var/lib/flatpak/app/com.discordapp.Discord/x86_64/stable/\
                   97ae45ed73f4aba6264effd8651aeddb2cbfe76eeec3bcd8ef7248d7f6b6acdf/files";
        assert_eq!(i.app_path, Path::new(app));
        let runtime = i.runtime_path.unwrap();
        assert!(runtime.starts_with("/var/lib/flatpak/runtime/org.freedesktop.Platform"));
        assert!(runtime.ends_with("files"));
    }

    #[test]
    fn keys_count_only_in_their_group() {
        // `name` outside [Application], `app-path` outside [Instance].
        let text = "name=wrong\n[Instance]\napp-path=/a/files\n[Context]\napp-path=/b\n\
                    [Application]\nname = com.example.App\n";
        let i = Instance::parse(text).unwrap();
        assert_eq!(i.id, "com.example.App");
        assert_eq!(i.app_path, Path::new("/a/files"));
        assert_eq!(i.runtime_path, None);

        assert_eq!(Instance::parse(""), None);
        assert_eq!(Instance::parse("[Application]\nname=x\n"), None);
        assert_eq!(
            Instance::parse("[Application]\nname=x\n[Instance]\napp-path=relative\n"),
            None
        );
        assert_eq!(Instance::parse("[Instance]\napp-path=/a\n"), None);
    }

    #[test]
    fn maps_sandbox_paths_to_the_host() {
        let i = instance(Path::new(
            "/inst/app/com.example.App/x86_64/stable/abc/files",
        ));
        assert_eq!(
            i.host_path(Path::new("/app/discord/Discord")),
            Some(i.app_path.join("discord/Discord"))
        );
        assert_eq!(
            i.host_path(Path::new("/usr/bin/bash")),
            Some("/rt/files/bin/bash".into())
        );
        // Whole components only, and nothing the sandbox shares.
        assert_eq!(i.host_path(Path::new("/application/x")), None);
        assert_eq!(i.host_path(Path::new("/usrx/x")), None);
        assert_eq!(i.host_path(Path::new("/home/user/x")), None);
        let bare = Instance {
            runtime_path: None,
            ..i
        };
        assert_eq!(bare.host_path(Path::new("/usr/bin/bash")), None);
    }

    #[test]
    fn install_folder_prefers_the_active_deploy() {
        let dir = tempfile::tempdir().unwrap();
        let branch = dir.path().join("app/com.example.App/x86_64/stable");
        let commit = branch.join("abc/files");
        fs::create_dir_all(&commit).unwrap();
        let i = instance(&commit);
        // No `active` yet: the commit's own folder.
        assert_eq!(i.install_folder(), commit);
        fs::create_dir_all(branch.join("active/files")).unwrap();
        assert_eq!(i.install_folder(), branch.join("active/files"));
    }

    #[test]
    fn install_folder_by_id_searches_installations_in_order() {
        let dir = tempfile::tempdir().unwrap();
        let (user, system) = (dir.path().join("user"), dir.path().join("system"));
        let files = |inst: &Path, id: &str| inst.join("app").join(id).join("current/active/files");
        fs::create_dir_all(files(&system, "com.example.Both")).unwrap();
        fs::create_dir_all(files(&user, "com.example.Both")).unwrap();
        fs::create_dir_all(files(&system, "com.example.System")).unwrap();
        let both = [user.clone(), system.clone()];

        assert_eq!(
            install_folder("com.example.Both", &both),
            Some(files(&user, "com.example.Both"))
        );
        assert_eq!(
            install_folder("com.example.System", &both),
            Some(files(&system, "com.example.System"))
        );
        assert_eq!(install_folder("com.example.Missing", &both), None);
        for bad in ["", "..", "../app", "a/b", ".hidden"] {
            assert_eq!(install_folder(bad, &both), None, "{bad:?}");
        }
    }

    #[test]
    fn installations_start_with_the_users_and_the_systems() {
        let all = installations();
        assert!(all.iter().all(|p| p.is_absolute()), "{all:?}");
        assert!(!all.is_empty());
    }

    #[test]
    fn reads_extra_installations() {
        let text = "[Installation \"extra\"]\nPath=/opt/flatpak\nDisplayName=Extra\n\
                    [Other]\nPath=/not/this\n[Installation \"rel\"]\nPath=relative\n\
                    [Installation \"spaced\"]\nPath = /srv/flatpak\nPathX=/no\n";
        assert_eq!(
            extra_installations(text),
            [PathBuf::from("/opt/flatpak"), "/srv/flatpak".into()]
        );
    }

    #[test]
    fn reads_only_a_regular_file_and_only_so_much() {
        let dir = tempfile::tempdir().unwrap();
        let info = dir.path().join("info");
        fs::write(&info, INFO).unwrap();
        assert_eq!(read_info(&info).as_deref(), Some(INFO));

        // A FIFO with no writer: refused at once, not waited on.
        let fifo = dir.path().join("fifo");
        let made = std::process::Command::new("mkfifo").arg(&fifo).status();
        if made.is_ok_and(|s| s.success()) {
            assert_eq!(read_info(&fifo), None);
        }
        assert_eq!(read_info(dir.path()), None);
        assert_eq!(read_info(&dir.path().join("missing")), None);

        let big = dir.path().join("big");
        fs::write(&big, "x".repeat(INFO_MAX as usize * 2)).unwrap();
        assert_eq!(read_info(&big).map(|t| t.len()), Some(INFO_MAX as usize));
    }

    /// This test process is in no sandbox, whatever machine runs it.
    #[test]
    fn no_instance_outside_a_sandbox() {
        if Path::new("/.flatpak-info").exists() {
            eprintln!("running inside a Flatpak; skipping");
            return;
        }
        assert_eq!(Instance::of(std::process::id()), None);
    }
}
