//! The fixtures are systemd 259's replies on the dev machine, recorded with
//! `busctl --json` and cut to a few units. Changed by hand to cover what the
//! machine didn't have: `mdmonitor.service` is failed, `iscsid.service` is
//! starting with a job queued, and `fwupd-refresh.service` is masked.

use serde_json::Value as Json;

use super::*;

const UNITS: &str = include_str!("../../tests/fixtures/systemd_units_json");
const FILES: &str = include_str!("../../tests/fixtures/systemd_unit_files_json");
const UNIT_PROPS: &str = include_str!("../../tests/fixtures/systemd_unit_props_json");
const SERVICE_PROPS: &str = include_str!("../../tests/fixtures/systemd_service_props_json");

fn rows(text: &str) -> Vec<Json> {
    let reply: Json = serde_json::from_str(text).unwrap();
    reply["data"][0].as_array().unwrap().clone()
}

fn units() -> Vec<Unit> {
    rows(UNITS)
        .iter()
        .map(|r| Unit {
            name: r[0].as_str().unwrap().to_owned(),
            description: r[1].as_str().unwrap().to_owned(),
            load: r[2].as_str().unwrap().to_owned(),
            active: r[3].as_str().unwrap().to_owned(),
            sub: r[4].as_str().unwrap().to_owned(),
            job_type: r[8].as_str().unwrap().to_owned(),
        })
        .collect()
}

fn files() -> Files {
    let by_name = unit_files(
        rows(FILES)
            .iter()
            .map(|r| {
                (
                    r[0].as_str().unwrap().to_owned(),
                    r[1].as_str().unwrap().to_owned(),
                )
            })
            .collect(),
    );
    let descriptions = [
        ("fancontrol.service", "Start fan control, if configured"),
        ("dnsmasq.service", "DNS caching server."),
        ("debug-shell.service", ""),
    ]
    .into_iter()
    .map(|(n, d)| (n.to_owned(), d.to_owned()))
    .collect();
    let instances = [("getty@tty1.service".to_owned(), Some(FileState::Enabled))]
        .into_iter()
        .collect();
    Files {
        by_name,
        instances,
        descriptions,
        stale: false,
        listed_at: None,
    }
}

/// An `a{sv}` reply as `busctl --json` writes it.
fn props(text: &str) -> Props {
    let reply: Json = serde_json::from_str(text).unwrap();
    reply["data"][0]
        .as_object()
        .unwrap()
        .iter()
        .map(|(k, v)| (k.clone(), value(v)))
        .collect()
}

fn value(v: &Json) -> OwnedValue {
    let d = &v["data"];
    let v = match v["type"].as_str().unwrap() {
        "u" => Value::U32(d.as_u64().unwrap() as u32),
        "t" => Value::U64(d.as_u64().unwrap()),
        "i" => Value::I32(d.as_i64().unwrap() as i32),
        "b" => Value::Bool(d.as_bool().unwrap()),
        "s" => Value::from(d.as_str().unwrap().to_owned()),
        "as" => Value::from(
            d.as_array()
                .unwrap()
                .iter()
                .map(|s| s.as_str().unwrap().to_owned())
                .collect::<Vec<_>>(),
        ),
        other => panic!("no fixture type {other}"),
    };
    OwnedValue::try_from(v).unwrap()
}

fn set(map: &mut Props, key: &str, v: Value<'_>) {
    map.insert(key.to_owned(), OwnedValue::try_from(v).unwrap());
}

fn find<'a>(list: &'a [Service], name: &str) -> &'a Service {
    list.iter()
        .find(|s| s.name == name)
        .unwrap_or_else(|| panic!("{name} not listed"))
}

#[test]
fn unit_files_drop_templates_and_aliases() {
    let files = files();
    assert!(files.by_name.keys().all(|n| !n.ends_with("@.service")));
    assert!(!files.by_name.contains_key("dbus.service"), "an alias");
    let avahi = &files.by_name["avahi-daemon.service"];
    assert_eq!(avahi.path, "/usr/lib/systemd/system/avahi-daemon.service");
    assert_eq!(avahi.state, FileState::Enabled);
    assert_eq!(
        files.by_name["systemd-remount-fs.service"].state,
        FileState::EnabledRuntime
    );
}

#[test]
fn merge_lists_failed_first_then_by_name() {
    let list = merge(units(), &files());
    assert_eq!(list[0].name, "mdmonitor.service");
    assert_eq!(list[0].status(), Status::Failed);
    let rest = &list[1..];
    assert!(rest.iter().all(|s| s.active != ActiveState::Failed));
    for pair in rest.windows(2) {
        assert_eq!(
            by_name(&pair[0].name, &pair[1].name),
            std::cmp::Ordering::Less,
            "{} before {}",
            pair[0].name,
            pair[1].name
        );
    }
    let names: HashSet<&str> = list.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), list.len(), "a unit listed twice");
    assert!(list.iter().all(|s| valid_name(&s.name)));
}

#[test]
fn merge_leaves_out_what_cannot_be_used() {
    let list = merge(units(), &files());
    for name in [
        // not-found and inactive: a name something refers to.
        "ipset.service",
        "power-profiles-daemon.service",
        // An alias of dbus-broker.service.
        "dbus.service",
        // Templates.
        "getty@.service",
        "drkonqi-coredump-processor@.service",
        // Not loaded, and can't be enabled.
        "systemd-reboot.service",
        "fwupd-refresh.service",
    ] {
        assert!(list.iter().all(|s| s.name != name), "{name} listed");
    }
}

#[test]
fn merge_adds_units_that_could_be_enabled() {
    let list = merge(units(), &files());
    let fan = find(&list, "fancontrol.service");
    assert_eq!(fan.load, LoadState::NotLoaded);
    assert_eq!(fan.file_state, Some(FileState::Disabled));
    assert_eq!(fan.description, "Start fan control, if configured");
    assert_eq!(fan.status(), Status::Stopped);
    assert_eq!(fan.job, None);
    // No description read: the name stands in.
    assert_eq!(
        find(&list, "debug-shell.service").description,
        "debug-shell.service"
    );
    assert_eq!(
        find(&list, "cups-browsed.service").description,
        "cups-browsed.service"
    );
}

#[test]
fn merge_states() {
    let list = merge(units(), &files());
    let chronyd = find(&list, "chronyd.service");
    assert_eq!(chronyd.status(), Status::Running);
    assert_eq!(chronyd.file_state, Some(FileState::Enabled));
    assert_eq!(chronyd.description, "NTP client/server");
    // Socket-activated: running, and not enabled itself.
    assert_eq!(
        find(&list, "cups.service").file_state,
        Some(FileState::Disabled)
    );
    let seed = find(&list, "systemd-random-seed.service");
    assert_eq!(seed.status(), Status::Active);
    assert_eq!(seed.file_state, Some(FileState::Static));
    assert_eq!(
        find(&list, "rpmdb-rebuild.service").status(),
        Status::Stopped
    );
    let iscsid = find(&list, "iscsid.service");
    assert_eq!(iscsid.status(), Status::Starting);
    assert_eq!(iscsid.job.as_deref(), Some("start"));
    // Instances: asked for one by one; one not asked yet has none.
    assert_eq!(
        find(&list, "getty@tty1.service").file_state,
        Some(FileState::Enabled)
    );
    assert_eq!(find(&list, "user@1000.service").file_state, None);
    assert_eq!(
        find(&list, "dbus-:1.3-org.freedesktop.problems@0.service").file_state,
        Some(FileState::Transient)
    );
}

#[test]
fn merge_keeps_a_running_unit_whose_file_is_gone() {
    let mut units = units();
    let gone = units
        .iter_mut()
        .find(|u| u.name == "ipset.service")
        .unwrap();
    gone.active = "active".to_owned();
    gone.sub = "running".to_owned();
    let list = merge(units, &files());
    let ipset = find(&list, "ipset.service");
    assert_eq!(ipset.load, LoadState::NotFound);
    assert_eq!(ipset.status(), Status::Running);
    assert_eq!(ipset.file_state, None);
}

#[test]
fn status_from_active_state() {
    let mut s = merge(units(), &files())[0].clone();
    let cases = [
        ("active", "running", Status::Running),
        ("active", "exited", Status::Active),
        ("reloading", "reload", Status::Running),
        ("activating", "start-pre", Status::Starting),
        ("deactivating", "stop-sigterm", Status::Stopping),
        ("inactive", "dead", Status::Stopped),
        ("maintenance", "cleaning", Status::Stopped),
        ("failed", "failed", Status::Failed),
        ("something-new", "dead", Status::Stopped),
    ];
    for (active, sub, want) in cases {
        s.active = ActiveState::parse(active);
        s.sub = sub.to_owned();
        assert_eq!(s.status(), want, "{active}/{sub}");
    }
}

#[test]
fn details_from_properties() {
    let d = parse_details(
        "chronyd.service",
        &props(UNIT_PROPS),
        &props(SERVICE_PROPS),
        None,
    );
    assert_eq!(d.service.name, "chronyd.service");
    assert_eq!(d.service.description, "NTP client/server");
    assert_eq!(d.service.load, LoadState::Loaded);
    assert_eq!(d.service.status(), Status::Running);
    assert_eq!(d.service.file_state, Some(FileState::Enabled));
    assert_eq!(d.preset.as_deref(), Some("enabled"));
    assert_eq!(
        d.path.as_deref(),
        Some("/usr/lib/systemd/system/chronyd.service")
    );
    assert_eq!(d.documentation, ["man:chronyd(8)", "man:chrony.conf(5)"]);
    assert_eq!(
        d.since,
        Some(SystemTime::UNIX_EPOCH + Duration::from_micros(1_790_908_347_562_250))
    );
    assert_eq!(d.main_pid, Some(1173));
    assert_eq!(d.tasks, Some(1));
    assert_eq!(d.memory, Some(1_687_552));
    assert_eq!(d.cpu_time, Some(Duration::from_nanos(253_974_000)));
    assert_eq!(d.result, None, "success is no result to show");
    assert_eq!(d.exit_status, None);
    assert_eq!(d.restarts, Some(0));
    assert!(d.can_start && d.can_stop);
    assert!(d.triggered_by.is_empty());
}

#[test]
fn details_prefer_the_file_list_and_drop_unknowns() {
    let unit = props(UNIT_PROPS);
    let mut service = props(SERVICE_PROPS);
    // Not counted: accounting off.
    set(&mut service, "MemoryCurrent", Value::U64(u64::MAX));
    set(&mut service, "TasksCurrent", Value::U64(u64::MAX));
    set(&mut service, "MainPID", Value::U32(0));
    let d = parse_details(
        "chronyd.service",
        &unit,
        &service,
        Some(FileState::Disabled),
    );
    assert_eq!(d.service.file_state, Some(FileState::Disabled));
    assert_eq!(d.memory, None);
    assert_eq!(d.tasks, None);
    assert_eq!(d.main_pid, None);
}

#[test]
fn details_exit_status_only_for_an_exit() {
    let unit = props(UNIT_PROPS);
    let mut service = props(SERVICE_PROPS);
    set(&mut service, "Result", Value::from("exit-code"));
    set(&mut service, "ExecMainCode", Value::I32(1));
    set(&mut service, "ExecMainStatus", Value::I32(3));
    let d = parse_details("chronyd.service", &unit, &service, None);
    assert_eq!(d.result.as_deref(), Some("exit-code"));
    assert_eq!(d.exit_status, Some(3));
    // Killed by a signal: the status is the signal's number.
    set(&mut service, "Result", Value::from("signal"));
    set(&mut service, "ExecMainCode", Value::I32(2));
    set(&mut service, "ExecMainStatus", Value::I32(9));
    let d = parse_details("chronyd.service", &unit, &service, None);
    assert_eq!(d.result.as_deref(), Some("signal"));
    assert_eq!(d.exit_status, None);
}

#[test]
fn details_without_service_properties() {
    // A unit whose Service interface didn't answer still has its state.
    let d = parse_details("chronyd.service", &props(UNIT_PROPS), &Props::new(), None);
    assert_eq!(d.service.status(), Status::Running);
    assert_eq!(d.main_pid, None);
    assert_eq!(d.cpu_time, None);
    assert_eq!(d.restarts, None);
}

#[test]
fn description_from_unit_file() {
    let text = "\
# A comment
[Unit]
Description=First
Documentation=man:x(8)
Description = Second, as systemd takes the last

[Service]
Description=Not this one
ExecStart=/usr/bin/x
";
    assert_eq!(
        parse_description(text).as_deref(),
        Some("Second, as systemd takes the last")
    );
    assert_eq!(parse_description("[Unit]\nDescription=\n"), None);
    assert_eq!(parse_description("[Service]\nDescription=x\n"), None);
    assert_eq!(parse_description(""), None);
}

#[test]
fn valid_names() {
    for name in [
        "NetworkManager.service",
        "getty@tty1.service",
        "getty@.service",
        r"systemd-fsck@dev-disk-by\x2duuid-8186\x2dCBAC.service",
        "dbus-:1.3-org.freedesktop.problems@0.service",
        "sshd.socket",
        "fstrim.timer",
        "-.mount",
        r"dev-disk-by\x2duuid-1234.swap",
        "proc-sys-fs-binfmt_misc.automount",
        "cups.path",
        "multi-user.target",
    ] {
        assert!(valid_name(name), "{name}");
    }
    for name in [
        "",
        ".service",
        ".socket",
        "x.device",
        "x.slice",
        "x.scope",
        "x.servicex",
        "service",
        "x.service.d",
        "/etc/systemd/system/x.service",
        "../x.service",
        ".hidden.service",
        "a b.service",
        "x.service\n",
        "é.service",
    ] {
        assert!(!valid_name(name), "{name:?}");
    }
    let long = format!("{}.service", "a".repeat(247));
    assert_eq!(long.len(), 255);
    assert!(valid_name(&long));
    assert!(!valid_name(&format!("a{long}")));
}

#[test]
fn kinds_and_templates() {
    assert_eq!(kind("sshd.service"), Some("service"));
    assert_eq!(kind("dbus-org.freedesktop.socket"), Some("socket"));
    assert_eq!(kind("-.mount"), Some("mount"));
    assert_eq!(kind("dev-sda.device"), None);
    assert_eq!(kind("service"), None);
    assert!(is_template("getty@.service"));
    assert!(is_template("sshd@.socket"));
    assert!(!is_template("getty@tty1.service"));
    assert!(is_instance("sshd@0-1.2.3.4:22.socket"));
    assert!(!is_instance("sshd.socket"));
    assert_eq!(
        kind_interface("sshd.socket").as_deref(),
        Some("org.freedesktop.systemd1.Socket")
    );
    assert_eq!(
        kind_interface("proc-sys-fs-binfmt_misc.automount").as_deref(),
        Some("org.freedesktop.systemd1.Automount")
    );
    assert_eq!(kind_interface("multi-user.target"), None);
    assert!(runs_by_hand("fstrim.timer"));
    assert!(!runs_by_hand("reboot.target"));
    assert!(!runs_by_hand("x.device"));
}

#[test]
fn names_sort_without_case_and_totally() {
    use std::cmp::Ordering::*;
    assert_eq!(by_name("avahi.service", "NetworkManager.service"), Less);
    assert_eq!(by_name("NetworkManager.service", "nfs.service"), Less);
    assert_eq!(by_name("A.service", "a.service"), Less);
    assert_eq!(by_name("a.service", "a.service"), Equal);
}

#[test]
fn job_results() {
    assert_eq!(job_outcome("done"), Ok(Outcome::Done));
    assert_eq!(job_outcome("skipped"), Ok(Outcome::Done));
    assert_eq!(
        job_outcome("failed"),
        Err(ActionError::Job(JobResult::Failed))
    );
    assert_eq!(
        job_outcome("dependency"),
        Err(ActionError::Job(JobResult::Dependency))
    );
    assert_eq!(
        job_outcome("timeout"),
        Err(ActionError::Job(JobResult::Timeout))
    );
    assert_eq!(
        job_outcome("canceled"),
        Err(ActionError::Job(JobResult::Canceled))
    );
    assert_eq!(
        job_outcome("assert"),
        Err(ActionError::Job(JobResult::Other("assert".to_owned())))
    );
    assert_eq!(job_outcome(""), Err(ActionError::NoAnswer));
}

#[test]
fn error_replies() {
    assert_eq!(
        error_from_reply(
            "org.freedesktop.DBus.Error.InteractiveAuthorizationRequired",
            None
        ),
        ActionError::NotAllowed
    );
    assert_eq!(
        error_from_reply(
            "org.freedesktop.DBus.Error.AccessDenied",
            Some("Access denied")
        ),
        ActionError::NotAllowed
    );
    assert_eq!(
        error_from_reply(
            "org.freedesktop.systemd1.NoSuchUnit",
            Some("Unit x.service not found.")
        ),
        ActionError::NoSuchUnit
    );
    assert_eq!(
        error_from_reply("org.freedesktop.systemd1.UnitMasked", None),
        ActionError::Masked
    );
    let refused = "Operation refused, unit x.service may be requested by dependency only.";
    assert_eq!(
        error_from_reply("org.freedesktop.systemd1.OnlyByDependency", Some(refused)),
        ActionError::Refused(refused.to_owned())
    );
}

fn signal<B>(member: &str, body: &B) -> Message
where
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    Message::signal(PATH, MANAGER_IF, member)
        .unwrap()
        .sender(":1.1")
        .unwrap()
        .build(body)
        .unwrap()
}

#[test]
fn signals_keep_the_state() {
    let mut state = State {
        listed_at: Some(Instant::now()),
        ..State::default()
    };
    let path = OwnedObjectPath::try_from("/org/freedesktop/systemd1/unit/x_2eservice").unwrap();
    state.seen(&signal("UnitNew", &("x.service", &path)));
    state.seen(&signal("UnitNew", &("dev-sda.device", &path)));
    assert_eq!(state.loaded, HashSet::from(["x.service".to_owned()]));
    assert!(!state.files.stale);
    state.seen(&signal("UnitRemoved", &("x.service", &path)));
    assert!(state.loaded.is_empty());
    // A body of the wrong shape is ignored.
    state.seen(&signal("UnitNew", &("y.service",)));
    assert!(state.loaded.is_empty());
    state.seen(&signal("UnitFilesChanged", &()));
    assert!(state.files.stale);
    assert!(state.listed_at.is_some(), "files changing leaves the units");
    state.files.stale = false;
    state.seen(&signal("Reloading", &(false,)));
    assert!(state.files.stale);
    assert!(state.listed_at.is_none(), "a reload lists every unit again");
    assert_eq!(state.reloads, 1);
}

#[test]
fn signals_during_a_full_list_are_kept_in_order() {
    let mut state = State {
        during: Some(Vec::new()),
        ..State::default()
    };
    let path = OwnedObjectPath::try_from("/org/freedesktop/systemd1/unit/x_2eservice").unwrap();
    state.seen(&signal("UnitNew", &("x.service", &path)));
    state.seen(&signal("UnitRemoved", &("y.service", &path)));
    state.seen(&signal("UnitNew", &("dev-sda.device", &path)));
    assert_eq!(
        state.during,
        Some(vec![
            (true, "x.service".to_owned()),
            (false, "y.service".to_owned()),
        ])
    );
}

#[test]
fn a_lost_signal_reads_everything_again() {
    let mut state = State {
        listed_at: Some(Instant::now()),
        ..State::default()
    };
    state.take(Err(zbus::Error::Failure("lost".into())));
    assert!(state.files.stale);
    assert!(state.listed_at.is_none());
}

#[test]
fn act_refuses_a_path_before_the_bus() {
    assert_eq!(
        act("/tmp/evil.service", Action::Enable),
        Err(ActionError::InvalidName)
    );
    assert_eq!(
        act("x.device", Action::Start),
        Err(ActionError::InvalidName)
    );
    for action in [Action::Start, Action::Stop, Action::Restart] {
        assert_eq!(
            act("poweroff.target", action),
            Err(ActionError::InvalidName)
        );
    }
}

/// Live: only where there is a system bus with systemd (not in CI).
#[test]
fn live_list_agrees_with_failed() {
    let Some(mut reader) = ServiceReader::new() else {
        eprintln!("no systemd on the system bus");
        return;
    };
    let list = reader.list().expect("systemd answered the connect");
    assert!(!list.is_empty());
    assert!(!reader.state.files.stale, "the file list was read");
    assert!(reader.state.files.listed_at.is_some());
    assert!(reader.state.listed_at.is_some(), "every unit was listed");
    assert!(
        list.iter()
            .all(|s| valid_name(&s.name) && !is_template(&s.name))
    );
    let names: HashSet<&str> = list.iter().map(|s| s.name.as_str()).collect();
    assert_eq!(names.len(), list.len(), "a unit listed twice");
    let failed_in_list: HashSet<&str> = list
        .iter()
        .filter(|s| s.active == ActiveState::Failed)
        .map(|s| s.name.as_str())
        .collect();
    // Failed first.
    assert!(
        list[..failed_in_list.len()]
            .iter()
            .all(|s| s.active == ActiveState::Failed)
    );
    let failed = reader.failed().expect("systemd answered");
    for name in &failed {
        assert!(failed_in_list.contains(name.as_str()), "{name}");
    }
    // The second list asks for the loaded services by name, and finds the
    // same ones (give or take what started or stopped meanwhile).
    let listed_at = reader.state.listed_at;
    let again = reader.list().expect("systemd answered");
    assert_eq!(reader.state.listed_at, listed_at, "listed by name");
    let names_again: HashSet<&str> = again.iter().map(|s| s.name.as_str()).collect();
    assert!(names.symmetric_difference(&names_again).count() < 5);
    let loaded = list
        .iter()
        .find(|s| s.load == LoadState::Loaded)
        .expect("a loaded service");
    let details = reader
        .details(&loaded.name)
        .expect("details of a loaded unit");
    assert_eq!(details.service.name, loaded.name);
    assert_eq!(details.service.load, LoadState::Loaded);
    assert!(reader.details("no-such-unit-atlas-test.service").is_none());
    assert!(reader.details("/etc/passwd").is_none());
}
