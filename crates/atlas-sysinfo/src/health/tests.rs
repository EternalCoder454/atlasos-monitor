//! Each test starts from a healthy machine and spoils one thing, so a check
//! that fires on the wrong input shows up as an extra alert. Ported from Go's
//! `internal/health` tests, plus the cases the port changed.

use super::*;

const GIB_: u64 = 1 << 30;

fn memory() -> Memory {
    Memory {
        total: 32 * GIB_,
        used: 10 * GIB_,
        available: 22 * GIB_,
        ..Memory::default()
    }
}

fn disk() -> Disk {
    Disk {
        name: "nvme0n1".into(),
        model: Some("Samsung SSD 990 PRO 1TB".into()),
        size: 1000 * GIB_,
        ..Disk::default()
    }
}

fn space(used: u64, free: u64) -> Option<Space> {
    Some(Space { used, free })
}

fn drive(failing: bool, spare_low: bool, wear: Option<u8>) -> smart::Health {
    smart::Health {
        kind: smart::Kind::Nvme,
        failing,
        warnings: Vec::new(),
        wear,
        spare: None,
        spare_low,
        temperature: None,
        temperature_limit: None,
        power_on_hours: None,
        power_cycles: None,
        read_bytes: None,
        written_bytes: None,
        unsafe_shutdowns: None,
        media_errors: None,
        bad_sectors: None,
        failing_attributes: None,
        updated: 0,
    }
}

/// Runs `check` on a healthy machine with `spoil` applied to its parts.
fn run<'a>(spoil: impl FnOnce(&mut Machine<'a>)) -> Vec<Alert> {
    // Leaked so a spoil can swap in its own borrowed parts.
    let d: &'static Disk = Box::leak(Box::new(disk()));
    let disks: &'static [DiskSpace<'static>] = Box::leak(Box::new([DiskSpace {
        disk: d,
        space: space(100 * GIB_, 900 * GIB_),
    }]));
    let mut m = Machine {
        cpu_temperature: Some(45.0),
        memory: Some(memory()),
        disks,
        ..Machine::default()
    };
    spoil(&mut m);
    check(&m)
}

fn titles(alerts: &[Alert]) -> Vec<String> {
    alerts.iter().map(Alert::title).collect()
}

#[test]
fn a_healthy_machine_has_nothing_to_say() {
    assert_eq!(titles(&run(|_| {})), Vec::<String>::new());
    assert_eq!(worst(&[]), None);
    // Nothing read at all is nothing wrong.
    assert!(check(&Machine::default()).is_empty());
}

#[test]
fn each_problem_is_reported_once() {
    let full = disk();
    let full_disks = [DiskSpace {
        disk: &full,
        space: space(990 * GIB_, GIB_),
    }];
    let gpu = [Graphics {
        name: "AMD Radeon RX 7900 XTX",
        temperature: Some(90.0),
    }];
    let cases: Vec<(&str, Vec<Alert>, Level)> = vec![
        (
            "Processor is running hot",
            run(|m| m.cpu_temperature = Some(91.0)),
            Level::Critical,
        ),
        (
            "Graphics card is running hot",
            run(|m| m.graphics = &gpu),
            Level::Critical,
        ),
        (
            "Running out of memory",
            run(|m| {
                m.memory = Some(Memory {
                    available: GIB_,
                    used: 31 * GIB_,
                    ..memory()
                })
            }),
            Level::Warning,
        ),
        (
            "Running out of memory",
            // 9% available: over 2 GiB, but over 90% used.
            run(|m| {
                m.memory = Some(Memory {
                    total: 64 * GIB_,
                    available: 6 * GIB_,
                    used: 58 * GIB_,
                    ..memory()
                })
            }),
            Level::Warning,
        ),
        (
            "Swapping heavily",
            run(|m| {
                m.memory = Some(Memory {
                    available: 4 * GIB_,
                    used: 28 * GIB_,
                    swap_total: 8 * GIB_,
                    swap_used: 4 * GIB_,
                    ..memory()
                })
            }),
            Level::Warning,
        ),
        (
            "Disk nearly full: Samsung SSD 990 PRO 1TB",
            run(|m| m.disks = &full_disks),
            Level::Warning,
        ),
    ];
    for (want, got, level) in cases {
        assert_eq!(titles(&got), [want], "{want}");
        assert_eq!(got[0].level, level, "{want}");
        assert!(!got[0].detail().is_empty(), "{want}: no detail");
    }
}

#[test]
fn thresholds_are_the_go_versions() {
    // At the line is not over it.
    assert!(run(|m| m.cpu_temperature = Some(85.0)).is_empty());
    assert_eq!(run(|m| m.cpu_temperature = Some(85.5)).len(), 1);

    // 2 GiB available, on a machine small enough that it is under 90% used.
    let mem = |total: u64, available: u64| Memory {
        total,
        available,
        used: total - available,
        ..Memory::default()
    };
    assert!(run(|m| m.memory = Some(mem(16 * GIB_, 2 * GIB_))).is_empty());
    assert_eq!(
        run(|m| m.memory = Some(mem(16 * GIB_, 2 * GIB_ - 1))).len(),
        1
    );
    // 90% used, with far more than 2 GiB available.
    assert!(run(|m| m.memory = Some(mem(100 * GIB_, 10 * GIB_))).is_empty());
    assert_eq!(
        run(|m| m.memory = Some(mem(100 * GIB_, 10 * GIB_ - 1))).len(),
        1
    );

    // Swap: over 25% full, and under 25% of memory available.
    let swapping = |swap_used: u64, available: u64| Memory {
        total: 32 * GIB_,
        available,
        used: 32 * GIB_ - available,
        swap_total: 8 * GIB_,
        swap_used,
        ..Memory::default()
    };
    assert!(run(|m| m.memory = Some(swapping(2 * GIB_, 7 * GIB_))).is_empty());
    assert!(run(|m| m.memory = Some(swapping(3 * GIB_, 8 * GIB_))).is_empty());
    assert_eq!(
        titles(&run(
            |m| m.memory = Some(swapping(2 * GIB_ + 1, 8 * GIB_ - 1))
        )),
        ["Swapping heavily"]
    );

    let card = |t: f64| {
        [Graphics {
            name: "card",
            temperature: Some(t),
        }]
    };
    let at = card(85.0);
    assert!(run(|m| m.graphics = &at).is_empty());
    let over = card(85.5);
    assert_eq!(run(|m| m.graphics = &over).len(), 1);

    // 5% free is not nearly full; just under is.
    let d = disk();
    let at = [DiskSpace {
        disk: &d,
        space: space(95 * GIB_, 5 * GIB_),
    }];
    assert!(run(|m| m.disks = &at).is_empty());
    let under = [DiskSpace {
        disk: &d,
        space: space(95 * GIB_ + 1, 5 * GIB_ - 1),
    }];
    assert_eq!(run(|m| m.disks = &under).len(), 1);
}

#[test]
fn a_sleeping_or_missing_card_is_not_hot() {
    let cards = [Graphics {
        name: "Intel Arc A380",
        temperature: None,
    }];
    assert!(run(|m| m.graphics = &cards).is_empty());
}

#[test]
fn every_hot_card_is_named() {
    let cards = [
        Graphics {
            name: "Intel UHD Graphics 770",
            temperature: Some(60.0),
        },
        Graphics {
            name: "AMD Radeon RX 7900 XTX",
            temperature: Some(96.0),
        },
    ];
    let got = run(|m| m.graphics = &cards);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].detail(), "AMD Radeon RX 7900 XTX is at 96 °C.");
}

#[test]
fn swap_is_ignored_without_swap() {
    let m = Memory {
        available: 4 * GIB_,
        used: 28 * GIB_,
        ..memory()
    };
    assert!(run(|x| x.memory = Some(m)).is_empty());
}

#[test]
fn zram_in_use_is_not_a_problem() {
    // Fedora's default: a third of zram occupied says nothing while memory
    // is plentiful (Go once told a machine with 23 GiB of 31 free it would
    // feel slow).
    let m = Memory {
        available: 23 * GIB_,
        used: 9 * GIB_,
        swap_total: 8 * GIB_,
        swap_used: 2400 << 20,
        ..memory()
    };
    assert!(run(|x| x.memory = Some(m)).is_empty());
}

#[test]
fn zram_is_not_a_disk_to_fill() {
    let d = disk();
    let zram = Disk {
        name: "zram0".into(),
        is_swap: true,
        size: 8 * GIB_,
        ..Disk::default()
    };
    let disks = [
        DiskSpace {
            disk: &d,
            space: space(100 * GIB_, 900 * GIB_),
        },
        DiskSpace {
            disk: &zram,
            space: space(8 * GIB_, 0),
        },
    ];
    assert!(run(|m| m.disks = &disks).is_empty());
}

#[test]
fn an_unmounted_disk_is_not_nearly_full() {
    // The dual-boot case: another system's disk has no space to measure.
    let d = disk();
    let windows = Disk {
        name: "nvme1n1".into(),
        size: 1000 * GIB_,
        ..Disk::default()
    };
    let mut disks = [
        DiskSpace {
            disk: &d,
            space: space(100 * GIB_, 900 * GIB_),
        },
        DiskSpace {
            disk: &windows,
            space: None,
        },
    ];
    assert!(run(|m| m.disks = &disks).is_empty());
    disks[1].space = space(0, 0);
    assert!(run(|m| m.disks = &disks).is_empty());
    // A full mounted one still reports.
    disks[1].space = space(999 * GIB_, GIB_);
    let got = run(|m| m.disks = &disks);
    assert_eq!(titles(&got), ["Disk nearly full: nvme1n1"]);
}

#[test]
fn fullness_is_measured_against_the_filesystems_not_the_disk() {
    // A 500 GiB Linux partition with 40 GiB free on a 2 TiB disk whose rest
    // is unmounted: 8% free, not 2%.
    let d = Disk {
        size: 2048 * GIB_,
        ..disk()
    };
    let roomy = [DiskSpace {
        disk: &d,
        space: space(460 * GIB_, 40 * GIB_),
    }];
    assert!(run(|m| m.disks = &roomy).is_empty());
    // And a full partition on a mostly empty disk is still full.
    let full = [DiskSpace {
        disk: &d,
        space: space(495 * GIB_, 5 * GIB_),
    }];
    let got = run(|m| m.disks = &full);
    assert_eq!(got.len(), 1);
    assert_eq!(got[0].detail(), "5.00 GiB free of 500.00 GiB.");
}

#[test]
fn drive_health_is_reported() {
    let name = "Kingston KC3000";
    let one = |h: &smart::Health| {
        let drives = [Drive { name, health: h }];
        run(|m| m.drives = &drives)
    };
    assert!(one(&drive(false, false, Some(1))).is_empty());
    // Ordinary ageing is not news.
    assert!(one(&drive(false, false, Some(40))).is_empty());
    assert!(one(&drive(false, false, None)).is_empty());

    let failing = one(&drive(true, false, Some(1)));
    assert_eq!(titles(&failing), ["Drive is failing: Kingston KC3000"]);
    assert_eq!(failing[0].level, Level::Critical);

    // Worn is from 90% up.
    assert!(one(&drive(false, false, Some(89))).is_empty());
    let worn = one(&drive(false, false, Some(90)));
    assert_eq!(titles(&worn), ["Drive is wearing out: Kingston KC3000"]);
    assert_eq!(worn[0].level, Level::Warning);
    assert_eq!(worn[0].detail(), "90% of its rated life is used.");

    // Out of spares is worth saying without a wear figure.
    let spares = one(&drive(false, true, None));
    assert_eq!(
        titles(&spares),
        ["Drive is nearly worn out: Kingston KC3000"]
    );

    // One alert per drive, the worst it has.
    assert_eq!(one(&drive(true, true, Some(100))).len(), 1);
}

#[test]
fn failed_services_are_summarised() {
    let one = ["nginx.service".to_owned()];
    let got = run(|m| m.failed_services = &one);
    assert_eq!(titles(&got), ["A background service has failed"]);
    assert_eq!(got[0].detail(), "nginx.service");

    let three: Vec<String> = ["a", "b", "c"].map(|s| format!("{s}.service")).into();
    let got = run(|m| m.failed_services = &three);
    assert_eq!(got[0].detail(), "a.service, b.service, c.service");

    let four: Vec<String> = ["a", "b", "c", "d"].map(|s| format!("{s}.service")).into();
    let got = run(|m| m.failed_services = &four);
    assert_eq!(
        got[0].detail(),
        "a.service, b.service, c.service and 1 more"
    );

    let five: Vec<String> = ["a", "b", "c", "d", "e"]
        .map(|s| format!("{s}.service"))
        .into();
    let got = run(|m| m.failed_services = &five);
    assert_eq!(titles(&got), ["5 background services have failed"]);
    assert_eq!(
        got[0].detail(),
        "a.service, b.service, c.service and 2 more"
    );
}

#[test]
fn critical_comes_first() {
    let d = disk();
    let full = [DiskSpace {
        disk: &d,
        space: space(990 * GIB_, GIB_),
    }];
    let h = drive(true, false, None);
    let drives = [Drive {
        name: "Kingston KC3000",
        health: &h,
    }];
    let failed = ["nginx.service".to_owned()];
    let got = run(|m| {
        m.disks = &full;
        m.failed_services = &failed;
        m.drives = &drives;
        m.cpu_temperature = Some(95.0);
    });
    assert_eq!(
        titles(&got),
        [
            "Processor is running hot",
            "Drive is failing: Kingston KC3000",
            "Disk nearly full: Samsung SSD 990 PRO 1TB",
            "A background service has failed",
        ]
    );
    assert_eq!(worst(&got), Some(Level::Critical));
    assert_eq!(worst(&got[2..]), Some(Level::Warning));
}

#[test]
fn figures_read_as_go_wrote_them() {
    assert_eq!(gib(10_995_116_278), "10.24 GiB");
    assert_eq!(bytes(0), "0 B");
    assert_eq!(bytes(1023), "1023 B");
    assert_eq!(bytes(1536), "2 KiB");
    assert_eq!(bytes(512 << 20), "512.0 MiB");
    assert_eq!(bytes(2_000_398_934_016), "1.82 TiB");
    let m = Memory {
        available: GIB_,
        used: 31 * GIB_,
        ..memory()
    };
    let got = run(|x| x.memory = Some(m));
    assert_eq!(
        got[0].detail(),
        "1.00 GiB free of 32.00 GiB. Programs may start closing."
    );
}

#[test]
fn readings_that_are_not_numbers_check_nothing() {
    let cards = [Graphics {
        name: "card",
        temperature: Some(f64::NAN),
    }];
    assert!(
        run(|m| {
            m.cpu_temperature = Some(f64::NAN);
            m.graphics = &cards;
        })
        .is_empty()
    );
    // No memory total: nothing to compare against.
    assert!(run(|m| m.memory = Some(Memory::default())).is_empty());
}
