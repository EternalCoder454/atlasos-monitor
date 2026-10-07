//! Autostart entries are written into temporary directories. The unit
//! fixtures are the dev machine's user manager (systemd 259), recorded with
//! `busctl --user --json` and cut to a few units, with rows added by hand
//! for what it didn't have: units of the user's own (`syncthing-tray`,
//! `backup.timer`, a linked `notes-sync`), a mask the user made (`obex`), one
//! the admin made in `/etc` (`mpris-proxy`), and a failed autostart unit.

use std::os::unix::fs::symlink;

use serde_json::Value as Json;
use tempfile::TempDir;

use super::*;

const FILES: &str = include_str!("../../tests/fixtures/user_unit_files_json");
const UNITS: &str = include_str!("../../tests/fixtures/user_units_json");

fn rows(text: &str) -> Vec<Json> {
    let reply: Json = serde_json::from_str(text).unwrap();
    reply["data"][0].as_array().unwrap().clone()
}

fn file_rows() -> Vec<(String, String)> {
    rows(FILES)
        .iter()
        .map(|r| (r[0].as_str().unwrap().into(), r[1].as_str().unwrap().into()))
        .collect()
}

fn live_rows() -> Vec<(String, String, String)> {
    rows(UNITS)
        .iter()
        .map(|r| {
            (
                r[0].as_str().unwrap().into(),
                r[3].as_str().unwrap().into(),
                r[4].as_str().unwrap().into(),
            )
        })
        .collect()
}

/// A session in a temporary directory: `home/.config` with system config
/// dirs `etc/xdg` and `etc/vendor` (in that order), KDE as the desktop.
struct Session {
    dir: TempDir,
    paths: Paths,
}

impl Session {
    fn new() -> Self {
        let dir = TempDir::new().unwrap();
        let root = dir.path();
        let paths = Paths {
            config_home: root.join("home/.config"),
            config_dirs: vec![root.join("etc/xdg"), root.join("etc/vendor")],
            data_home: root.join("home/.local/share"),
            runtime_dir: Some(root.join("run")),
            admin_units: root.join("etc/systemd/user"),
            unit_dirs: vec![root.join("usr/lib/systemd/user")],
            desktops: vec!["KDE".into()],
            path: vec![root.join("bin")],
            locales: vec!["pt_BR".into(), "pt".into()],
        };
        Self { dir, paths }
    }

    fn root(&self) -> &Path {
        self.dir.path()
    }

    fn system(&self, id: &str, text: &str) -> PathBuf {
        write(&self.root().join("etc/xdg/autostart").join(id), text)
    }

    fn user(&self, id: &str, text: &str) -> PathBuf {
        write(&self.paths.user_autostart().join(id), text)
    }

    fn user_file(&self, id: &str) -> Option<String> {
        fs::read_to_string(self.paths.user_autostart().join(id)).ok()
    }

    fn item(&self, id: &str) -> Item {
        list_desktop(&self.paths)
            .into_iter()
            .find(|i| i.id == id)
            .unwrap_or_else(|| panic!("no item {id}"))
    }
}

fn write(path: &Path, text: &str) -> PathBuf {
    fs::create_dir_all(path.parent().unwrap()).unwrap();
    fs::write(path, text).unwrap();
    path.to_owned()
}

const APP: &str = "[Desktop Entry]\nType=Application\nName=Syncer\nName[pt]=Sincronizador\n\
Comment=Keeps folders in step\nExec=syncer --tray\nIcon=syncer\n\n\
[Desktop Action quit]\nName=Quit\nExec=syncer --quit\nHidden=true\n";

#[test]
fn parses_the_main_group_only() {
    let d = parse_desktop(APP, &[]).unwrap();
    assert_eq!(d.name.as_deref(), Some("Syncer"));
    assert_eq!(d.comment.as_deref(), Some("Keeps folders in step"));
    assert_eq!(d.exec, "syncer --tray");
    assert_eq!(d.icon, "syncer");
    // Hidden in an action group is not the entry's.
    assert!(!d.hidden);
    assert!(parse_desktop("[Desktop Action x]\nName=A\n", &[]).is_none());
    assert!(parse_desktop("", &[]).is_none());
}

#[test]
fn prefers_the_closest_locale_and_the_last_value() {
    let locales = ["pt_BR".to_owned(), "pt".to_owned()];
    let d = parse_desktop(APP, &locales).unwrap();
    assert_eq!(d.name.as_deref(), Some("Sincronizador"));
    let text = "[Desktop Entry]\nName[pt_BR]=BR\nName[pt]=PT\nName=Plain\nComment[de]=DE\n\
Comment=One\nComment=Two\nExec=a\nExec=b\n";
    let d = parse_desktop(text, &locales).unwrap();
    assert_eq!(d.name.as_deref(), Some("BR"));
    assert_eq!(d.comment.as_deref(), Some("Two"));
    assert_eq!(d.exec, "b");
}

#[test]
fn booleans_are_true_or_false_only() {
    let d = parse_desktop("[Desktop Entry]\nHidden=1\nNoDisplay=yes\n", &[]).unwrap();
    assert!(!d.hidden && !d.no_display);
    let d = parse_desktop(
        "[Desktop Entry]\nHidden=true\nNoDisplay=true\nX-systemd-skip=true\n",
        &[],
    )
    .unwrap();
    assert!(d.hidden && d.no_display && d.skip);
}

#[test]
fn decides_what_runs_like_the_generator() {
    let s = Session::new();
    let runs_of = |extra: &str| {
        let text = format!("[Desktop Entry]\nName=X\nExec=x\n{extra}");
        runs(&parse_desktop(&text, &[]).unwrap(), &s.paths)
    };
    assert_eq!(runs_of(""), Runs::Yes);
    assert_eq!(runs_of("OnlyShowIn=GNOME;Unity;"), Runs::NotThisDesktop);
    assert_eq!(runs_of("OnlyShowIn=GNOME;KDE;"), Runs::Yes);
    // Exactly, as systemd-xdg-autostart-condition compares.
    assert_eq!(runs_of("OnlyShowIn=gnome;kde;"), Runs::NotThisDesktop);
    assert_eq!(runs_of("NotShowIn=KDE;"), Runs::NotThisDesktop);
    assert_eq!(runs_of("NotShowIn=kde;"), Runs::Yes);
    assert_eq!(runs_of("X-systemd-skip=true"), Runs::ByUnit);
    assert_eq!(
        runs(
            &parse_desktop("[Desktop Entry]\nName=X\n", &[]).unwrap(),
            &s.paths
        ),
        Runs::NoCommand
    );

    assert_eq!(runs_of("TryExec=present"), Runs::MissingProgram);
    let bin = write(&s.root().join("bin/present"), "#!/bin/sh\n");
    assert_eq!(
        runs_of("TryExec=present"),
        Runs::MissingProgram,
        "not executable"
    );
    fs::set_permissions(&bin, fs::Permissions::from_mode(0o755)).unwrap();
    assert_eq!(runs_of("TryExec=present"), Runs::Yes);
    assert_eq!(
        runs_of(&format!("TryExec={}", bin.display())),
        Runs::Yes,
        "absolute"
    );
    assert_eq!(runs_of("TryExec=/nowhere/present"), Runs::MissingProgram);
}

#[test]
fn reads_kde_conditions_user_first() {
    let s = Session::new();
    let cond = "X-KDE-autostart-condition=indexrc:Basic Settings:Indexing-Enabled:true";
    let runs_of = || {
        let text = format!("[Desktop Entry]\nName=X\nExec=x\n{cond}\n");
        runs(&parse_desktop(&text, &[]).unwrap(), &s.paths)
    };
    // No file: the default.
    assert_eq!(runs_of(), Runs::Yes);
    write(
        &s.root().join("etc/xdg/indexrc"),
        "[Basic Settings]\nIndexing-Enabled[$i]=false\n",
    );
    assert_eq!(runs_of(), Runs::TurnedOff);
    write(
        &s.paths.config_home.join("indexrc"),
        "[Other]\nIndexing-Enabled=false\n[Basic Settings]\nIndexing-Enabled=true\n",
    );
    assert_eq!(runs_of(), Runs::Yes);

    assert!(!kde_condition("nofile:G:K:false", &s.paths));
    assert!(kde_condition("../escape:G:K:true", &s.paths));
    assert!(kde_condition("garbage", &s.paths));
}

#[test]
fn user_files_shadow_system_ones_and_the_first_system_dir_wins() {
    let s = Session::new();
    s.system("a.desktop", "[Desktop Entry]\nName=From xdg\nExec=a\n");
    write(
        &s.root().join("etc/vendor/autostart/a.desktop"),
        "[Desktop Entry]\nName=From vendor\nExec=a\n",
    );
    write(
        &s.root().join("etc/vendor/autostart/b.desktop"),
        "[Desktop Entry]\nName=Vendor only\nExec=b\n",
    );
    s.user("c.desktop", "[Desktop Entry]\nName=Mine\nExec=c\n");
    s.system("d.desktop", "[Desktop Entry]\nName=Shadowed\nExec=d\n");
    s.user("d.desktop", "[Desktop Entry]\nName=Mine now\nExec=d2\n");
    // Not entries.
    s.user("notes.txt", "[Desktop Entry]\nName=No\n");
    s.user(".hidden.desktop", "[Desktop Entry]\nName=No\n");
    s.user("empty.desktop", "nothing here\n");

    let items = list_desktop(&s.paths);
    let names: Vec<(&str, &str, bool)> = items
        .iter()
        .map(|i| (i.id.as_str(), i.name.as_str(), i.system))
        .collect();
    assert_eq!(
        names,
        [
            ("a.desktop", "From xdg", true),
            ("b.desktop", "Vendor only", true),
            ("c.desktop", "Mine", false),
            ("d.desktop", "Mine now", true),
        ]
    );
    let d = &items[3];
    assert_eq!(d.command, "d2");
    assert_eq!(d.file, s.paths.user_autostart().join("d.desktop"));
}

#[test]
fn a_switch_off_file_is_described_by_the_system_entry() {
    let s = Session::new();
    s.system(
        "x.desktop",
        "[Desktop Entry]\nName=Thing\nComment=Does it\nExec=thing\nIcon=thing\nNoDisplay=true\n",
    );
    s.user("x.desktop", "[Desktop Entry]\nHidden=true\n");
    let x = s.item("x.desktop");
    assert_eq!(
        (x.name.as_str(), x.comment.as_str(), x.command.as_str()),
        ("Thing", "Does it", "thing")
    );
    assert!(!x.enabled);
    assert!(x.plumbing, "plumbing comes from the system entry");
    assert_eq!(x.runs, Runs::Yes, "what runs when switched on");

    // Not hiding it, the file shadows the entry with no command at all.
    s.user("x.desktop", "[Desktop Entry]\nIcon=other\n");
    assert_eq!(s.item("x.desktop").runs, Runs::NoCommand);
}

#[test]
fn locks_the_updater_and_skipped_entries() {
    let s = Session::new();
    s.system(
        UPDATER_TRAY,
        "[Desktop Entry]\nName=Telamon Updater (tray)\nExec=atlas-updater --tray\nNoDisplay=true\nOnlyShowIn=KDE;\n",
    );
    s.system(
        "shell.desktop",
        "[Desktop Entry]\nName=Shell\nExec=shell\nNoDisplay=true\nX-systemd-skip=true\n",
    );
    let updater = s.item(UPDATER_TRAY);
    assert_eq!(updater.lock, Some(Lock::Required));
    assert!(!updater.plumbing, "shown though NoDisplay");
    assert!(updater.enabled && !updater.can_switch());
    assert_eq!(updater.runs, Runs::Yes);

    let shell = s.item("shell.desktop");
    assert_eq!((shell.lock, shell.runs), (Some(Lock::ByUnit), Runs::ByUnit));
    assert!(!shell.can_switch());

    assert_eq!(
        set_desktop(&s.paths, UPDATER_TRAY, false),
        Err(Error::Locked)
    );
    assert_eq!(
        set_desktop(&s.paths, "shell.desktop", false),
        Err(Error::Locked)
    );
    assert_eq!(s.user_file(UPDATER_TRAY), None);

    // Switched off some other way, it can be switched back on.
    s.user(UPDATER_TRAY, "[Desktop Entry]\nHidden=true\n");
    let updater = s.item(UPDATER_TRAY);
    assert!(!updater.enabled && updater.can_switch());
    set_desktop(&s.paths, UPDATER_TRAY, true).unwrap();
    assert_eq!(s.user_file(UPDATER_TRAY), None);
    assert!(s.item(UPDATER_TRAY).enabled);
}

#[test]
fn names_the_generated_unit() {
    let s = Session::new();
    s.system(
        "org.kde.kdeconnect.daemon.desktop",
        "[Desktop Entry]\nName=KDE Connect\nExec=kdeconnectd\n",
    );
    s.system(
        "geoclue-demo-agent.desktop",
        "[Desktop Entry]\nName=Geoclue\nExec=g\n",
    );
    s.system(
        "never-generated.desktop",
        "[Desktop Entry]\nName=N\nExec=n\n",
    );
    let gen_dir = s.root().join("run/systemd/generator.late");
    write(
        &gen_dir.join("app-org.kde.kdeconnect.daemon@autostart.service"),
        &format!(
            "# Automatically generated by systemd-xdg-autostart-generator\n\n[Unit]\n\
SourcePath={}\nDescription=KDE Connect\n",
            s.root()
                .join("etc/xdg/autostart/org.kde.kdeconnect.daemon.desktop")
                .display()
        ),
    );
    write(
        &gen_dir.join("app-geoclue\\x2ddemo\\x2dagent@autostart.service"),
        "[Unit]\nSourcePath=/etc/xdg/autostart/geoclue-demo-agent.desktop\n",
    );
    write(
        &gen_dir.join("other.service"),
        "[Unit]\nSourcePath=/x/y.desktop\n",
    );

    let mut items = list_desktop(&s.paths);
    apply_live(&mut items, &live_rows());
    let get = |id: &str| items.iter().find(|i| i.id == id).unwrap();
    let kdeconnect = get("org.kde.kdeconnect.daemon.desktop");
    assert_eq!(
        kdeconnect.unit.as_deref(),
        Some("app-org.kde.kdeconnect.daemon@autostart.service")
    );
    assert_eq!(kdeconnect.status, Some(Status::Failed));
    let geoclue = get("geoclue-demo-agent.desktop");
    assert_eq!(
        geoclue.unit.as_deref(),
        Some("app-geoclue\\x2ddemo\\x2dagent@autostart.service")
    );
    assert_eq!(geoclue.status, Some(Status::Running));
    let never = get("never-generated.desktop");
    assert_eq!((&never.unit, never.status), (&None, None));
}

#[test]
fn switching_a_system_entry_writes_a_copy_and_hands_it_back() {
    let s = Session::new();
    let text = "# vendor\n[Desktop Entry]\nName=Thing\nExec=thing\n\n[Desktop Action a]\nExec=b\n";
    let sys = s.system("x.desktop", text);

    set_desktop(&s.paths, "x.desktop", false).unwrap();
    let copy = s.user_file("x.desktop").unwrap();
    assert_eq!(
        copy,
        "# vendor\n[Desktop Entry]\nName=Thing\nExec=thing\nHidden=true\n\n[Desktop Action a]\nExec=b\n"
    );
    assert_eq!(
        fs::read_to_string(&sys).unwrap(),
        text,
        "system file untouched"
    );
    let x = s.item("x.desktop");
    assert!(!x.enabled && x.system);

    // Off again changes nothing.
    set_desktop(&s.paths, "x.desktop", false).unwrap();
    assert_eq!(s.user_file("x.desktop").unwrap(), copy);

    set_desktop(&s.paths, "x.desktop", true).unwrap();
    assert_eq!(s.user_file("x.desktop"), None, "the copy is removed");
    assert!(s.item("x.desktop").enabled);
    // On again: nothing to write.
    set_desktop(&s.paths, "x.desktop", true).unwrap();
    assert_eq!(s.user_file("x.desktop"), None);
}

#[test]
fn a_marker_file_is_removed_or_made_whole() {
    let s = Session::new();
    s.system("x.desktop", "[Desktop Entry]\nName=Thing\nExec=thing\n");
    s.user("x.desktop", "[Desktop Entry]\nHidden=true\n");
    set_desktop(&s.paths, "x.desktop", true).unwrap();
    assert_eq!(s.user_file("x.desktop"), None);

    // A system entry hidden by its package: on needs a copy without Hidden.
    let s = Session::new();
    s.system(
        "y.desktop",
        "[Desktop Entry]\nName=Y\nExec=y\nHidden=true\n",
    );
    assert!(!s.item("y.desktop").enabled);
    set_desktop(&s.paths, "y.desktop", true).unwrap();
    assert_eq!(
        s.user_file("y.desktop").unwrap(),
        "[Desktop Entry]\nName=Y\nExec=y\n"
    );
    assert!(s.item("y.desktop").enabled);
    set_desktop(&s.paths, "y.desktop", false).unwrap();
    assert_eq!(s.user_file("y.desktop"), None, "back to the package");
}

#[test]
fn a_customised_override_keeps_its_changes() {
    let s = Session::new();
    s.system("x.desktop", "[Desktop Entry]\nName=Thing\nExec=thing\n");
    s.user(
        "x.desktop",
        "[Desktop Entry]\nName=Thing\nExec=thing --quiet\n",
    );
    set_desktop(&s.paths, "x.desktop", false).unwrap();
    assert_eq!(
        s.user_file("x.desktop").unwrap(),
        "[Desktop Entry]\nName=Thing\nExec=thing --quiet\nHidden=true\n"
    );
    set_desktop(&s.paths, "x.desktop", true).unwrap();
    assert_eq!(
        s.user_file("x.desktop").unwrap(),
        "[Desktop Entry]\nName=Thing\nExec=thing --quiet\n"
    );
}

#[test]
fn switches_the_users_own_entry_in_place() {
    let s = Session::new();
    s.user(
        "mine.desktop",
        "[Desktop Entry]\nName=Mine\nExec=mine\nHidden=false\nHidden=false\n",
    );
    set_desktop(&s.paths, "mine.desktop", false).unwrap();
    assert_eq!(
        s.user_file("mine.desktop").unwrap(),
        "[Desktop Entry]\nName=Mine\nExec=mine\nHidden=true\n"
    );
    let mine = s.item("mine.desktop");
    assert!(!mine.enabled && !mine.system && !mine.plumbing);
    set_desktop(&s.paths, "mine.desktop", true).unwrap();
    assert_eq!(
        s.user_file("mine.desktop").unwrap(),
        "[Desktop Entry]\nName=Mine\nExec=mine\n"
    );
    // No temporary file left behind.
    let names: Vec<_> = fs::read_dir(s.paths.user_autostart())
        .unwrap()
        .map(|e| e.unwrap().file_name())
        .collect();
    assert_eq!(names, ["mine.desktop"]);
}

#[test]
fn replaces_a_link_rather_than_writing_through_it() {
    let s = Session::new();
    let text = "[Desktop Entry]\nName=Linked\nExec=linked\n";
    let target = write(
        &s.root().join("usr/share/applications/linked.desktop"),
        text,
    );
    fs::create_dir_all(s.paths.user_autostart()).unwrap();
    let link = s.paths.user_autostart().join("linked.desktop");
    symlink(&target, &link).unwrap();
    assert!(s.item("linked.desktop").enabled);

    set_desktop(&s.paths, "linked.desktop", false).unwrap();
    assert_eq!(fs::read_to_string(&target).unwrap(), text);
    assert!(!fs::symlink_metadata(&link).unwrap().is_symlink());
    assert!(!s.item("linked.desktop").enabled);
}

#[test]
fn refuses_bad_names_and_missing_entries() {
    let s = Session::new();
    for id in [
        "../x.desktop",
        "a/b.desktop",
        ".desktop",
        ".x.desktop",
        "x.service",
        "",
    ] {
        assert_eq!(
            set_desktop(&s.paths, id, false),
            Err(Error::InvalidName),
            "{id}"
        );
    }
    assert_eq!(
        set_desktop(&s.paths, "gone.desktop", false),
        Err(Error::NotFound)
    );
    assert!(!s.paths.user_autostart().exists(), "nothing created");
}

#[test]
fn skips_a_fifo() {
    let s = Session::new();
    s.system("ok.desktop", "[Desktop Entry]\nName=Ok\nExec=ok\n");
    let fifo = s.root().join("etc/xdg/autostart/pipe.desktop");
    let c = std::ffi::CString::new(fifo.as_os_str().as_encoded_bytes()).unwrap();
    assert_eq!(unsafe { libc::mkfifo(c.as_ptr(), 0o644) }, 0);
    let ids: Vec<String> = list_desktop(&s.paths).into_iter().map(|i| i.id).collect();
    assert_eq!(ids, ["ok.desktop"]);
}

#[test]
fn with_hidden_edits_only_the_main_group() {
    assert_eq!(
        with_hidden("[Desktop Entry]\nName=A\n", true),
        "[Desktop Entry]\nName=A\nHidden=true\n"
    );
    assert_eq!(
        with_hidden(
            "[Desktop Entry]\nHidden = true\nName=A\n\n\n[X]\nHidden=true\n",
            false
        ),
        "[Desktop Entry]\nName=A\n\n\n[X]\nHidden=true\n"
    );
    assert_eq!(
        with_hidden("[Desktop Entry]\nName=A\n\n[X]\nB=c", true),
        "[Desktop Entry]\nName=A\nHidden=true\n\n[X]\nB=c\n"
    );
    assert_eq!(
        with_hidden("# no group\n", true),
        "# no group\n[Desktop Entry]\nHidden=true\n"
    );
    // Off then on gives back the same text, so a copy is recognised.
    let text = "[Desktop Entry]\nName=A\nExec=a\n";
    assert_eq!(with_hidden(&with_hidden(text, true), false), text);
}

/// Paths for the unit fixture: its home is `/home/atlas`.
fn unit_paths() -> Paths {
    Paths {
        config_home: "/home/atlas/.config".into(),
        data_home: "/home/atlas/.local/share".into(),
        unit_dirs: vec!["/nonexistent".into()],
        ..Paths::default()
    }
}

fn scan_of(user_wants: &[&str], login_wants: &[&str]) -> Scan {
    let set = |names: &[&str]| names.iter().map(|n| n.to_string()).collect();
    Scan {
        candidates: Vec::new(),
        user_wants: set(user_wants),
        login_wants: set(login_wants),
    }
}

#[test]
fn lists_the_units_that_start_and_the_users_own() {
    let scan = scan_of(
        &["wireplumber.service"],
        &["drkonqi-sentry-postman.path", "web.service"],
    );
    let items = merge_units(&unit_paths(), &file_rows(), &scan);
    let got: Vec<(&str, bool, bool, bool, Option<Lock>)> = items
        .iter()
        .map(|i| (i.id.as_str(), i.enabled, i.system, i.plumbing, i.lock))
        .collect();
    let session = Some(Lock::Session);
    let mut want = vec![
        ("systemd-tmpfiles-setup.service", true, true, true, session),
        ("grub-boot-success.timer", true, true, true, None),
        ("xdg-user-dirs.service", true, true, true, None),
        ("pipewire.socket", true, true, true, None),
        ("dbus-broker.service", true, true, true, session),
        ("wireplumber.service", true, true, false, None),
        ("dbus.socket", true, true, true, session),
        // Disabled, but a login target wants it from /usr/lib.
        ("drkonqi-sentry-postman.path", true, true, true, None),
        // Generated and wanted: a Quadlet.
        ("web.service", true, true, true, None),
        ("syncthing-tray.service", true, false, false, None),
        ("backup.timer", false, false, false, None),
        ("notes-sync.service", false, false, false, None),
        ("obex.service", false, true, true, None),
    ];
    let mut got_sorted = got.clone();
    got_sorted.sort_by_key(|g| g.0);
    want.sort_by_key(|w| w.0);
    assert_eq!(got_sorted, want);
    for i in &items {
        assert_eq!(i.kind, Kind::Unit);
        assert_eq!(i.unit.as_deref(), Some(i.id.as_str()));
        // Descriptions come from files this machine doesn't have.
        assert!(!i.name.is_empty());
        assert_eq!(i.own, !i.system, "{}", i.id);
        assert_eq!(
            i.wanted,
            ["drkonqi-sentry-postman.path", "web.service"].contains(&i.id.as_str())
        );
    }
}

#[test]
fn a_masked_unit_is_described_from_its_installed_file() {
    let dir = TempDir::new().unwrap();
    let mut paths = unit_paths();
    paths.unit_dirs = vec![dir.path().to_owned()];
    write(
        &dir.path().join("obex.service"),
        "[Unit]\nDescription=Bluetooth OBEX service\n",
    );
    let items = merge_units(&paths, &file_rows(), &Scan::default());
    let obex = items.iter().find(|i| i.id == "obex.service").unwrap();
    assert_eq!(obex.name, "Bluetooth OBEX service");
    assert_eq!(obex.file, dir.path().join("obex.service"));
    assert!(obex.can_switch());
}

#[test]
fn a_quadlet_is_the_users_when_its_source_is() {
    let dir = TempDir::new().unwrap();
    let mut paths = unit_paths();
    paths.config_home = dir.path().join("home/.config");
    let unit = write(
        &dir.path().join("run/systemd/generator/web.service"),
        &format!(
            "# Automatically generated by podman-user-generator\n[Unit]\nDescription=Web app\n\
SourcePath={}\n",
            paths
                .config_home
                .join("containers/systemd/web.container")
                .display()
        ),
    );
    let files = vec![(unit.to_string_lossy().into_owned(), "generated".to_owned())];
    let items = merge_units(&paths, &files, &scan_of(&[], &["web.service"]));
    let web = &items[0];
    assert_eq!(web.name, "Web app");
    assert_eq!(
        web.file,
        paths.config_home.join("containers/systemd/web.container")
    );
    assert!(web.enabled && !web.plumbing && !web.system && web.wanted && !web.own);
    // Not wanted by a login target, a generated unit doesn't start.
    assert!(merge_units(&paths, &files, &Scan::default()).is_empty());
}

#[test]
fn scans_the_folders_for_what_may_start() {
    let s = Session::new();
    let root = s.root();
    let user = s.paths.user_units();
    let admin = root.join("etc/systemd/user");
    let vendor = root.join("usr/lib/systemd/user");
    let mut paths = s.paths.clone();
    paths.admin_units = admin.clone();
    paths.unit_dirs = vec![admin.clone(), vendor.clone()];
    let link = |target: &str, at: &Path| {
        fs::create_dir_all(at.parent().unwrap()).unwrap();
        symlink(target, at).unwrap();
    };
    // The user's own unit, enabled; a mask; an installed unit enabled by
    // hand.
    write(&user.join("own.service"), "[Unit]\n");
    link(
        &user.join("own.service").to_string_lossy(),
        &user.join("default.target.wants/own.service"),
    );
    link("/dev/null", &user.join("masked.service"));
    link(
        &vendor.join("tray.service").to_string_lossy(),
        &user.join("graphical-session.target.wants/tray.service"),
    );
    write(
        &s.paths.data_home.join("systemd/user/data.timer"),
        "[Unit]\n",
    );
    // The admin's alias and enablement.
    link(
        &vendor.join("dbus-broker.service").to_string_lossy(),
        &admin.join("dbus.service"),
    );
    link(
        &vendor.join("pipewire.socket").to_string_lossy(),
        &admin.join("sockets.target.wants/pipewire.socket"),
    );
    // Installed: everything at the top is left alone, and only login
    // targets' wants count.
    write(&vendor.join("idle.service"), "[Unit]\n");
    link(
        "../push.service",
        &vendor.join("graphical-session.target.wants/push.service"),
    );
    link(
        "../ibus.service",
        &vendor.join("gnome-session.target.wants/ibus.service"),
    );
    link(
        "../pickup.service",
        &vendor.join("plasma-core.target.wants/pickup.service"),
    );
    // A Quadlet, and the generator's autostart units the entries cover.
    link(
        "../web.service",
        &root.join("run/systemd/generator/default.target.wants/web.service"),
    );
    link(
        "../app-x@autostart.service",
        &root.join("run/systemd/generator/default.target.wants/app-x@autostart.service"),
    );
    write(&user.join("notes.txt"), "");

    let scan = scan_units(&paths);
    assert_eq!(
        scan.candidates,
        [
            "data.timer",
            "dbus-broker.service",
            "dbus.service",
            "masked.service",
            "own.service",
            "pickup.service",
            "pipewire.socket",
            "push.service",
            "tray.service",
            "web.service",
        ]
    );
    assert_eq!(
        scan.user_wants,
        ["own.service", "tray.service"]
            .map(String::from)
            .into_iter()
            .collect()
    );
    assert_eq!(
        scan.login_wants,
        ["pickup.service", "push.service", "web.service"]
            .map(String::from)
            .into_iter()
            .collect()
    );
}

#[test]
fn unit_state_comes_from_the_live_list() {
    let paths = unit_paths();
    let mut items = merge_units(&paths, &file_rows(), &Scan::default());
    apply_live(&mut items, &live_rows());
    let status = |id: &str| items.iter().find(|i| i.id == id).unwrap().status;
    assert_eq!(status("syncthing-tray.service"), Some(Status::Running));
    assert_eq!(status("wireplumber.service"), Some(Status::Running));
    assert_eq!(status("backup.timer"), None, "not loaded");
}

#[test]
fn whole_list_is_sorted_and_says_whether_units_answered() {
    let s = Session::new();
    s.system("zz.desktop", "[Desktop Entry]\nName=alpha\nExec=a\n");
    s.user("aa.desktop", "[Desktop Entry]\nName=Zulu\nExec=z\n");
    let without = finish(list_desktop(&s.paths), None);
    assert!(!without.units);
    let names: Vec<&str> = without.items.iter().map(|i| i.name.as_str()).collect();
    assert_eq!(names, ["alpha", "Zulu"]);

    let replies = UnitReplies {
        items: merge_units(&unit_paths(), &file_rows(), &Scan::default()),
        live: live_rows(),
    };
    let with = finish(list_desktop(&s.paths), Some(replies));
    assert!(with.units);
    assert!(with.items.len() > 2);
    for pair in with.items.windows(2) {
        assert!(pair[0].name.to_lowercase() <= pair[1].name.to_lowercase());
    }
    let ids: HashSet<&str> = with.items.iter().map(|i| i.id.as_str()).collect();
    assert_eq!(ids.len(), with.items.len(), "ids are unique");
    let sync = with
        .items
        .iter()
        .find(|i| i.id == "syncthing-tray.service")
        .unwrap();
    assert_eq!(sync.status, Some(Status::Running));
}

#[test]
fn unit_names_and_patterns() {
    for ok in [
        "a.service",
        "a.socket",
        "a.timer",
        "a.path",
        "getty@tty1.service",
        "a\\x2db.service",
    ] {
        assert!(valid_unit_name(ok), "{ok}");
    }
    for bad in [
        "a.target",
        "a@.service",
        ".service",
        ".a.service",
        "/etc/a.service",
        "../a.service",
        "a b.service",
    ] {
        assert!(!valid_unit_name(bad), "{bad}");
    }
    assert_eq!(glob_literal("a\\x2d[b]*?.service"), "a?x2d?b]??.service");
    assert!(login_target("default.target") && login_target("plasma-core.target"));
    assert!(!login_target("gnome-session.target") && !login_target("plasma-x.service"));
}

#[test]
fn units_refuse_before_the_bus() {
    let paths = unit_paths();
    assert_eq!(
        set_unit(&paths, "../x.service", true, false, true),
        Err(Error::InvalidName)
    );
    assert_eq!(
        set_unit(&paths, "dbus.socket", false, false, false),
        Err(Error::Locked)
    );
    let locked = Item {
        id: "dbus-broker.service".into(),
        kind: Kind::Unit,
        name: String::new(),
        comment: String::new(),
        icon: String::new(),
        command: String::new(),
        file: PathBuf::new(),
        system: true,
        plumbing: true,
        enabled: true,
        runs: Runs::Yes,
        lock: Some(Lock::Session),
        unit: None,
        status: None,
        own: false,
        wanted: false,
    };
    assert_eq!(set_enabled(&locked, false), Err(Error::Locked));
}

#[test]
fn leaves_a_user_file_it_cannot_read_whole() {
    let s = Session::new();
    s.system("x.desktop", "[Desktop Entry]\nName=X\nExec=x\n");
    let user = s.paths.user_autostart().join("x.desktop");
    fs::create_dir_all(user.parent().unwrap()).unwrap();

    let latin1 = b"[Desktop Entry]\nName=Caf\xe9\nExec=x --mine\n";
    fs::write(&user, latin1).unwrap();
    assert_eq!(
        set_desktop(&s.paths, "x.desktop", false),
        Err(Error::Io(io::ErrorKind::InvalidData))
    );
    assert_eq!(fs::read(&user).unwrap(), latin1);

    let long = format!(
        "[Desktop Entry]\nName=X\nExec=x\n#{}\n",
        "a".repeat(FILE_MAX as usize)
    );
    fs::write(&user, &long).unwrap();
    assert_eq!(
        set_desktop(&s.paths, "x.desktop", false),
        Err(Error::Io(io::ErrorKind::FileTooLarge))
    );
    assert_eq!(fs::read_to_string(&user).unwrap(), long);

    // A link to nothing is nothing: replaced.
    fs::remove_file(&user).unwrap();
    symlink(s.root().join("gone"), &user).unwrap();
    assert_eq!(set_desktop(&s.paths, "x.desktop", false), Ok(()));
    assert!(s.user_file("x.desktop").unwrap().contains("Hidden=true"));
}

#[test]
fn copies_the_system_entry_the_list_shows() {
    let s = Session::new();
    // The first folder's file has no entry group: the list skips it.
    s.system("x.desktop", "[Something Else]\nName=Wrong\n");
    write(
        &s.root().join("etc/vendor/autostart/x.desktop"),
        "[Desktop Entry]\nName=Right\nExec=x\n",
    );
    assert_eq!(s.item("x.desktop").name, "Right");
    assert_eq!(set_desktop(&s.paths, "x.desktop", false), Ok(()));
    let copy = s.user_file("x.desktop").unwrap();
    assert!(
        copy.contains("Name=Right") && copy.contains("Hidden=true"),
        "{copy}"
    );
}

#[test]
fn nothing_is_read_or_written_without_a_home() {
    let s = Session::new();
    let paths = Paths {
        config_home: PathBuf::from(".config"),
        data_home: PathBuf::from(".local/share"),
        ..s.paths.clone()
    };
    s.system("x.desktop", "[Desktop Entry]\nName=X\nExec=x\n");
    assert_eq!(
        set_desktop(&paths, "x.desktop", false),
        Err(Error::Io(io::ErrorKind::NotFound))
    );
    assert!(read_autostart_dir(&paths.user_autostart(), &[]).is_empty());
    assert!(unit_entries(&paths.user_units()).is_empty());
}

#[test]
fn keeps_only_the_rows_asked_for() {
    let rows = vec![
        ("/u/a\\x2db.service".to_owned(), "enabled".to_owned()),
        ("/u/a?x2db.service".to_owned(), "enabled".to_owned()),
        ("/u/other.service".to_owned(), "enabled".to_owned()),
    ];
    let got = asked_for(rows, &["a\\x2db.service".to_owned()]);
    assert_eq!(
        got,
        [("/u/a\\x2db.service".to_owned(), "enabled".to_owned())]
    );
}

#[test]
fn finds_a_linked_unit_and_not_a_mask() {
    let s = Session::new();
    let units = s.paths.user_units();
    fs::create_dir_all(&units).unwrap();
    symlink(
        "/opt/notes/notes-sync.service",
        units.join("notes-sync.service"),
    )
    .unwrap();
    symlink("../../elsewhere/rel.service", units.join("rel.service")).unwrap();
    symlink("/dev/null", units.join("obex.service")).unwrap();
    write(&units.join("plain.service"), "[Service]\nExecStart=x\n");
    assert_eq!(
        s.paths.linked_unit("notes-sync.service"),
        Some((
            units.join("notes-sync.service"),
            PathBuf::from("/opt/notes/notes-sync.service")
        ))
    );
    assert_eq!(
        s.paths.linked_unit("rel.service").map(|(_, t)| t),
        Some(units.join("../../elsewhere/rel.service"))
    );
    assert_eq!(s.paths.linked_unit("obex.service"), None);
    assert_eq!(s.paths.linked_unit("plain.service"), None);
}
