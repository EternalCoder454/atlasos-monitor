//! Grouping, search and sorting against a desktop index made in a temporary
//! folder, so nothing depends on what this machine has installed.

use super::*;
use std::path::Path;

const FIREFOX: &str = "app-gnome-org.mozilla.firefox-4242.scope";
// One application in two units: Flatpak starts a scope per launch.
const DISCORD_A: &str = "app-flatpak-com.discordapp.Discord-3262276743.scope";
const DISCORD_B: &str = "app-com.discordapp.Discord@224f36554a1346eca11c7378b65ad44c.service";

fn write(path: &Path, text: &str) {
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

/// A resolver over three desktop files, taking icon names on trust.
fn resolver(dir: &Path) -> Resolver {
    for (id, body) in [
        (
            "org.mozilla.firefox",
            "[Desktop Entry]\nName=Firefox\nIcon=org.mozilla.firefox\n",
        ),
        (
            "com.discordapp.Discord",
            "[Desktop Entry]\nName=Discord\nIcon=com.discordapp.Discord\n",
        ),
        (
            "org.kde.konsole",
            "[Desktop Entry]\nName=Konsole\nIcon=utilities-terminal\nCategories=System;TerminalEmulator;\n",
        ),
    ] {
        write(&dir.join(format!("applications/{id}.desktop")), body);
    }
    Resolver::new(desktop::Index::new(vec![dir.into()]), None)
}

fn proc(pid: u32, name: &str, unit: &str, cpu: f64, memory: u64) -> Proc {
    Proc {
        pid,
        start_time: u64::from(pid) * 10,
        name: name.into(),
        parent: 1,
        kernel: false,
        unit: (!unit.is_empty()).then(|| unit.into()),
        cpu,
        memory,
        gpu: None,
        net_in: None,
        net_out: None,
        disk_read: None,
        disk_write: None,
    }
}

fn unit(s: &str) -> Arc<str> {
    s.into()
}

#[test]
fn groups_by_application() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    let mut procs = vec![
        proc(10, "firefox", FIREFOX, 5.0, 100),
        proc(11, "Isolated Web Co", FIREFOX, 20.0, 300),
        proc(12, "WebExtensions", FIREFOX, 1.0, 50),
        proc(20, "Discord", DISCORD_A, 2.0, 200),
        proc(21, "Discord", DISCORD_B, 3.0, 100),
        // No application: grouped by name.
        proc(30, "pipewire", "pipewire.service", 1.0, 10),
        proc(31, "pipewire", "pipewire.service", 1.0, 10),
        // Shares an application's name without being it.
        proc(40, "Discord", "", 0.5, 5),
    ];
    procs[11 - 10].gpu = Some(4.0);
    procs[12 - 10].disk_read = Some(1000.0);

    let mut g = Grouper::default();
    let groups = g.group(&procs, &mut apps).to_vec();
    assert_eq!(groups.len(), 4, "{groups:#?}");

    let ff = &groups[0];
    assert_eq!(ff.key, GroupKey::App("org.mozilla.firefox".into()));
    assert_eq!((&*ff.total.name, ff.count), ("Firefox", 3));
    assert_eq!((ff.total.cpu, ff.total.memory), (26.0, 450));
    assert_eq!(ff.total.pid, 10, "the first member's pid");
    assert_eq!(
        ff.total.gpu,
        Some(4.0),
        "one member's known GPU makes the group's known"
    );
    assert_eq!(ff.total.disk_read, Some(1000.0));
    assert_eq!(ff.total.net_in, None, "no member's network is known");
    assert_eq!(ff.app.as_ref().unwrap().name.as_ref(), "Firefox");

    let d = &groups[1];
    assert_eq!(
        (&*d.total.name, d.count, d.total.cpu, d.total.memory),
        ("Discord", 2, 5.0, 300)
    );

    let pw = &groups[2];
    assert_eq!(pw.key, GroupKey::Process("pipewire".into()));
    assert_eq!(pw.count, 2);
    assert!(pw.app.is_none());

    let other = &groups[3];
    assert_eq!(
        other.key,
        GroupKey::Process("Discord".into()),
        "an unrelated process joined Discord"
    );
    assert_eq!((other.count, other.total.pid), (1, 40));

    // Every member maps back to its row's key, and the counts agree.
    for grp in &groups {
        let members: Vec<u32> = apps.members(&grp.key, &procs).map(|p| p.pid).collect();
        assert_eq!(members.len(), grp.count as usize, "{:?}", grp.key);
        assert_eq!(members[0], grp.total.pid);
    }
}

#[test]
fn a_group_knows_a_figure_if_any_member_does() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    let mut procs = vec![
        proc(10, "firefox", FIREFOX, 0.0, 0),
        proc(11, "Web Content", FIREFOX, 0.0, 0),
    ];
    procs[1].gpu = Some(0.0);
    let mut g = Grouper::default();
    // Go kept an unknown GPU unless a member's was above zero; a member
    // read as idle is known, and says the group is idle.
    assert_eq!(g.group(&procs, &mut apps)[0].total.gpu, Some(0.0));
}

#[test]
fn a_group_rates_its_power_on_the_sum() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    // Three Firefox processes each Low; together they are Moderate.
    let procs: Vec<Proc> = (0..3)
        .map(|i| proc(10 + i, "firefox", FIREFOX, 10.0, 1))
        .collect();
    let mut g = Grouper::default();
    let groups = g.group(&procs, &mut apps);
    assert_eq!(procs[0].impact(), crate::process::Impact::Low);
    assert_eq!(groups[0].total.impact(), crate::process::Impact::Moderate);
}

#[test]
fn resolves_names_terminals_and_misses() {
    let dir = tempfile::tempdir().unwrap();
    let mut r = resolver(dir.path());
    let k = r
        .of(Some(&unit("app-org.kde.konsole@abc.service")))
        .unwrap();
    assert!(k.terminal);
    assert_eq!(k.name.as_ref(), "Konsole");
    assert_eq!(k.icon, Some(Icon::Name("utilities-terminal".into())));

    assert!(r.of(Some(&unit("pipewire.service"))).is_none());
    assert!(r.of(None).is_none());

    // No desktop file: still an application, named from its ID, with the ID
    // as its icon.
    let c = r
        .of(Some(&unit("app-org.chromium.Chromium-2270643.scope")))
        .unwrap();
    assert_eq!(c.name.as_ref(), "Chromium");
    assert_eq!(c.icon, Some(Icon::Name("org.chromium.Chromium".into())));

    // Twice goes through the cache and hands out the same application.
    let a = r.of(Some(&unit(DISCORD_A))).unwrap().clone();
    let b = r.of(Some(&unit(DISCORD_A))).unwrap();
    assert!(Arc::ptr_eq(&a, b));
}

#[test]
fn picks_the_first_icon_that_draws() {
    let dir = tempfile::tempdir().unwrap();
    let icons = dir.path().join("icons/hicolor/48x48/apps");
    std::fs::create_dir_all(&icons).unwrap();
    std::fs::write(
        dir.path().join("icons/hicolor/index.theme"),
        "[Icon Theme]\nDirectories=48x48/apps\n",
    )
    .unwrap();
    std::fs::write(icons.join("chromium.png"), "").unwrap();
    std::fs::write(icons.join("org.example.Flat.svg"), "").unwrap();
    let pixmap = dir.path().join("own-icon.png");
    std::fs::write(&pixmap, "").unwrap();
    for (id, icon) in [
        // Declares an icon the theme lacks: falls back to the ID.
        ("org.example.Flat", "not-in-the-theme"),
        // Declares a file that exists.
        ("org.example.File", pixmap.to_str().unwrap()),
        // Declares a file that is gone, and nothing else draws.
        ("org.example.Gone", "/no/such/file.png"),
    ] {
        write(
            &dir.path().join(format!("applications/{id}.desktop")),
            &format!("[Desktop Entry]\nName={id}\nIcon={icon}\n"),
        );
    }
    write(
        &dir.path().join("applications/org.example.Flat2.desktop"),
        "[Desktop Entry]\nName=Flat2\nIcon=chromium-beta\n",
    );
    std::fs::write(icons.join("org.example.Flat2.svg"), "").unwrap();
    write(
        &dir.path().join("applications/org.example.Short.desktop"),
        "[Desktop Entry]\nName=Short\nIcon=chromium-beta\n",
    );
    let dirs = vec![dir.path().to_path_buf()];
    let mut r = Resolver::new(
        desktop::Index::new(dirs.clone()),
        Some(IconLookup::new("breeze", &dirs)),
    );
    let mut icon = |u: &str| r.of(Some(&unit(u))).unwrap().icon.clone();
    assert_eq!(
        icon("app-org.example.Flat-1.scope"),
        Some(Icon::Name("org.example.Flat".into()))
    );
    assert_eq!(
        icon("app-org.example.File-1.scope"),
        Some(Icon::Path(pixmap.clone()))
    );
    assert_eq!(icon("app-org.example.Gone-1.scope"), None);
    // A declared icon found only shortened loses to the ID found as it is;
    // with nothing found as it is, the shortened one is better than none.
    assert_eq!(
        icon("app-org.example.Flat2-1.scope"),
        Some(Icon::Name("org.example.Flat2".into()))
    );
    assert_eq!(
        icon("app-org.example.Short-1.scope"),
        Some(Icon::Name("chromium-beta".into()))
    );
    // No desktop file, no icon by ID: the lower-case name.
    assert_eq!(
        icon("app-org.chromium.Chromium-1.scope"),
        Some(Icon::Name("chromium".into()))
    );
}

#[test]
fn forgets_units_no_longer_seen() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    let mut g = Grouper::default();
    let old = [proc(10, "Discord", DISCORD_A, 0.0, 0)];
    g.group(&old, &mut apps);
    let current = [proc(11, "Discord", DISCORD_B, 0.0, 0)];
    for _ in 0..FORGET_AFTER * 2 {
        g.group(&current, &mut apps);
    }
    assert_eq!(apps.units_known(), 1, "the old unit is still cached");
    assert!(apps.of(Some(&unit(DISCORD_B))).is_some());
}

#[test]
fn forgetting_survives_the_tick_wrapping() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    apps.tick = u32::MAX - 3;
    let mut g = Grouper::default();
    let old = [proc(10, "Discord", DISCORD_A, 0.0, 0)];
    g.group(&old, &mut apps);
    let current = [proc(11, "Discord", DISCORD_B, 0.0, 0)];
    for _ in 0..FORGET_AFTER / 2 {
        g.group(&current, &mut apps);
    }
    assert_eq!(
        apps.units_known(),
        2,
        "a unit was forgotten early across the wrap"
    );
    for _ in 0..FORGET_AFTER * 2 {
        g.group(&current, &mut apps);
    }
    assert_eq!(apps.units_known(), 1);
}

#[test]
fn a_new_icon_theme_picks_icons_again() {
    let dir = tempfile::tempdir().unwrap();
    for (theme, icon) in [("light", "own-light"), ("dark", "org.example.T")] {
        let d = dir.path().join("icons").join(theme);
        std::fs::create_dir_all(d.join("apps")).unwrap();
        std::fs::write(d.join("index.theme"), "[Icon Theme]\nDirectories=apps\n").unwrap();
        std::fs::write(d.join("apps").join(format!("{icon}.svg")), "").unwrap();
    }
    write(
        &dir.path().join("applications/org.example.T.desktop"),
        "[Desktop Entry]\nName=T\nIcon=own-light\n",
    );
    let dirs = vec![dir.path().to_path_buf()];
    let mut r = Resolver::new(
        desktop::Index::new(dirs.clone()),
        Some(IconLookup::new("light", &dirs)),
    );
    let u = unit("app-org.example.T-1.scope");
    assert_eq!(
        r.of(Some(&u)).unwrap().icon,
        Some(Icon::Name("own-light".into()))
    );
    // The dark theme lacks the declared icon, so the ID is next.
    r.set_icon_theme("dark");
    assert_eq!(
        r.of(Some(&u)).unwrap().icon,
        Some(Icon::Name("org.example.T".into()))
    );
    // The same theme again keeps what it knows.
    let before = r.of(Some(&u)).unwrap().clone();
    r.set_icon_theme("dark");
    assert!(Arc::ptr_eq(&before, r.of(Some(&u)).unwrap()));
}

#[test]
fn search_finds_an_applications_processes() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    let helper = proc(11, "Isolated Web Co", FIREFOX, 0.0, 0);
    let stranger = proc(99, "Isolated Web Co", "other.service", 0.0, 0);
    let s = Search::new(" FireFox ");
    let app = apps.of(helper.unit.as_ref()).cloned();
    assert!(
        s.matches(&helper, app.as_deref()),
        "a Firefox process wasn't found by searching for Firefox"
    );
    let none = apps.of(stranger.unit.as_ref()).cloned();
    assert!(!s.matches(&stranger, none.as_deref()));

    assert!(
        Search::new("web co").matches(&stranger, None),
        "by its own name, any case"
    );
    assert!(Search::new("9").matches(&stranger, None), "by pid");
    assert!(!Search::new("100").matches(&stranger, None));
    assert!(Search::new("").matches(&stranger, None));
    assert!(Search::new("").is_empty());
}

#[test]
fn search_matches_a_group_by_pid_only_when_it_shows_one() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    let procs = [
        proc(4242, "firefox", FIREFOX, 0.0, 0),
        proc(4243, "Web Content", FIREFOX, 0.0, 0),
        proc(777, "bash", "", 0.0, 0),
    ];
    let mut g = Grouper::default();
    let groups = g.group(&procs, &mut apps);
    assert!(
        !Search::new("4242").matches_group(&groups[0]),
        "a group matched by a pid it doesn't show"
    );
    assert!(Search::new("fire").matches_group(&groups[0]));
    assert!(Search::new("777").matches_group(&groups[1]));
}

#[test]
fn search_folds_non_ascii_application_names() {
    let dir = tempfile::tempdir().unwrap();
    write(
        &dir.path().join("applications/org.example.E.desktop"),
        "[Desktop Entry]\nName=Éditeur\n",
    );
    let mut r = Resolver::new(desktop::Index::new(vec![dir.path().into()]), None);
    let p = proc(5, "editor", "app-org.example.E-1.scope", 0.0, 0);
    let app = r.of(p.unit.as_ref()).cloned();
    assert!(Search::new("ÉDIT").matches(&p, app.as_deref()));
}

#[test]
fn digits_of_pids() {
    let mut buf = [0u8; 10];
    assert_eq!(digits(0, &mut buf), b"0");
    assert_eq!(digits(4_194_304, &mut buf), b"4194304");
    assert_eq!(digits(u32::MAX, &mut buf), b"4294967295");
}

fn by_cpu(cpus: &[f64]) -> Vec<Proc> {
    cpus.iter()
        .enumerate()
        .map(|(i, &c)| proc(i as u32 + 1, "p", "", c, 0))
        .collect()
}

#[test]
fn sort_keeps_ties_where_they_were() {
    let rows = by_cpu(&[5.0, 1.0, 5.0, 0.0, 1.0, 5.0]);
    // Shown last tick in this order:
    let mut order = vec![5, 3, 4, 2, 1, 0];
    sort(&mut order, &rows, Column::Cpu, false);
    assert_eq!(order, [3, 4, 1, 5, 2, 0]);
    // Descending keeps ties in their order too, rather than flipping them.
    let mut order = vec![5, 3, 4, 2, 1, 0];
    sort(&mut order, &rows, Column::Cpu, true);
    assert_eq!(order, [5, 2, 0, 4, 1, 3]);
}

#[test]
fn sort_is_ordered_and_repeatable() {
    let cpus: Vec<f64> = (0..500).map(|i| ((i * 7919) % 13) as f64).collect();
    let rows = by_cpu(&cpus);
    let mut order: Vec<usize> = (0..rows.len()).rev().collect();
    sort(&mut order, &rows, Column::Cpu, true);
    for w in order.windows(2) {
        let (a, b) = (&rows[w[0]], &rows[w[1]]);
        assert!(a.cpu >= b.cpu);
        if a.cpu == b.cpu {
            assert!(
                w[0] > w[1],
                "a tie moved: the old order was descending index"
            );
        }
    }
    let again = {
        let mut o = order.clone();
        sort(&mut o, &rows, Column::Cpu, true);
        o
    };
    assert_eq!(order, again, "re-sorting a sorted table moved rows");
}

#[test]
fn columns_order_their_values() {
    let mut a = proc(2, "bash", "", 0.0, 0);
    let mut b = proc(10, "Firefox", "", 0.0, 0);
    assert_eq!(
        Column::Name.compare(&a, &b),
        Ordering::Less,
        "names ignore case"
    );
    assert_eq!(Column::Name.compare(&a, &a), Ordering::Equal);
    assert_eq!(Column::Pid.compare(&a, &b), Ordering::Less);
    // Unknown sorts below zero.
    b.gpu = Some(0.0);
    assert_eq!(Column::Gpu.compare(&a, &b), Ordering::Less);
    a.gpu = Some(3.0);
    assert_eq!(Column::Gpu.compare(&a, &b), Ordering::Greater);
    // Power by the score: Very Low < High, not alphabetical.
    a.cpu = 90.0;
    assert_eq!(Column::Power.compare(&a, &b), Ordering::Greater);
    b.disk_write = Some(1.0);
    assert_eq!(Column::DiskWrite.compare(&a, &b), Ordering::Less);
}

#[test]
fn groups_sort_like_processes() {
    let dir = tempfile::tempdir().unwrap();
    let mut apps = resolver(dir.path());
    let procs = [
        proc(1, "a", "", 1.0, 0),
        proc(2, "firefox", FIREFOX, 1.0, 0),
        proc(3, "Web Content", FIREFOX, 1.0, 0),
    ];
    let mut g = Grouper::default();
    let groups = g.group(&procs, &mut apps);
    let mut order = vec![0, 1];
    sort(&mut order, groups, Column::Cpu, true);
    assert_eq!(order, [1, 0], "Firefox's two processes outweigh one");
}
