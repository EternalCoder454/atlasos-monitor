use super::*;
use std::os::unix::fs::symlink;

/// A fake `/sys`: `class/drm/<node>/device` linked to a PCI device folder
/// holding `files`, with the driver link that sysfs has.
struct Sys {
    root: tempfile::TempDir,
}

impl Sys {
    fn new() -> Self {
        let root = tempfile::tempdir().unwrap();
        fs::create_dir_all(root.path().join("class/drm")).unwrap();
        Self { root }
    }

    fn drm(&self) -> PathBuf {
        self.root.path().join("class/drm")
    }

    fn card(&self, node: &str, slot: &str, driver: &str, files: &[(&str, &str)]) -> PathBuf {
        let dev = self.root.path().join("devices").join(slot);
        fs::create_dir_all(&dev).unwrap();
        for (name, value) in files {
            let path = dev.join(name);
            fs::create_dir_all(path.parent().unwrap()).unwrap();
            fs::write(path, value).unwrap();
        }
        symlink(
            format!("../../../bus/pci/drivers/{driver}"),
            dev.join("driver"),
        )
        .unwrap();
        let card = self.drm().join(node);
        fs::create_dir_all(&card).unwrap();
        symlink(&dev, card.join("device")).unwrap();
        dev
    }
}

const RX_7900_XTX: &[(&str, &str)] = &[
    ("vendor", "0x1002\n"),
    ("device", "0x744c\n"),
    ("product_name", "AMD Radeon RX 7900 XTX\n"),
    ("gpu_busy_percent", "37\n"),
    ("mem_info_vram_used", "2147483648\n"),
    ("mem_info_vram_total", "25753026560\n"),
    ("mem_info_vram_vendor", "samsung\n"),
    ("mem_info_gtt_used", "104857600\n"),
    ("mem_info_gtt_total", "16777216000\n"),
    ("power/runtime_status", "active\n"),
    ("hwmon/hwmon2/temp1_label", "edge\n"),
    ("hwmon/hwmon2/temp1_input", "52000\n"),
    ("hwmon/hwmon2/power1_average", "89000000\n"),
    ("hwmon/hwmon2/power1_cap", "327000000\n"),
];

#[test]
fn lists_cards_best_first() {
    let sys = Sys::new();
    sys.card(
        "card0",
        "0000:00:02.0",
        "i915",
        &[("vendor", "0x8086\n"), ("device", "0xa780\n")],
    );
    sys.card("card1", "0000:03:00.0", "amdgpu", RX_7900_XTX);
    // An APU's small carve-out ranks below the discrete card.
    sys.card(
        "card2",
        "0000:0e:00.0",
        "amdgpu",
        &[
            ("vendor", "0x1002\n"),
            ("device", "0x164e\n"),
            ("mem_info_vram_total", "536870912\n"),
        ],
    );
    // A connector, a render node, and a display with no PCI vendor.
    fs::create_dir_all(sys.drm().join("card1-DP-1")).unwrap();
    fs::create_dir_all(sys.drm().join("renderD128")).unwrap();
    sys.card("card3", "simple-framebuffer.0", "simple-framebuffer", &[]);

    let cards = cards_in(&sys.drm());
    let nodes: Vec<&str> = cards.iter().map(|c| c.node.as_str()).collect();
    assert_eq!(nodes, ["card1", "card2", "card0"]);
    let integrated: Vec<bool> = cards.iter().map(|c| c.integrated).collect();
    assert_eq!(integrated, [false, true, true]);
    // Listed by codename ("Raphael"), named as sold.
    assert_eq!(cards[1].name, "AMD Radeon Graphics");

    let amd = &cards[0];
    assert_eq!(amd.name, "AMD Radeon RX 7900 XTX");
    assert_eq!(amd.vendor, Vendor::Amd);
    assert_eq!(amd.driver, "amdgpu");
    assert_eq!(amd.slot, "0000:03:00.0");
    assert_eq!(amd.memory_total, Some(25_753_026_560));
    assert_eq!(amd.gtt_total, Some(16_777_216_000));

    let intel = &cards[2];
    assert_eq!(intel.vendor, Vendor::Intel);
    assert_eq!(intel.driver, "i915");
    assert!(!intel.name.is_empty());
    assert_eq!(intel.memory_total, None);
}

/// An AMD laptop processor with an Intel Arc beside it: the discrete card
/// comes first whatever its maker.
#[test]
fn discrete_before_integrated() {
    let sys = Sys::new();
    sys.card(
        "card0",
        "0000:c4:00.0",
        "amdgpu",
        &[
            ("vendor", "0x1002\n"),
            ("device", "0x15bf\n"),
            ("mem_info_vram_total", "2147483648\n"),
        ],
    );
    sys.card(
        "card1",
        "0000:03:00.0",
        "xe",
        &[
            ("vendor", "0x8086\n"),
            ("device", "0xe20b\n"),
            ("tile0/physical_vram_size_bytes", "12884901888\n"),
        ],
    );
    let cards = cards_in(&sys.drm());
    let nodes: Vec<&str> = cards.iter().map(|c| c.node.as_str()).collect();
    assert_eq!(nodes, ["card1", "card0"]);
    assert!(!cards[0].integrated);
    assert_eq!(cards[0].memory_total, Some(12_884_901_888));
    assert!(cards[1].integrated);
}

#[test]
fn no_cards() {
    let dir = tempfile::tempdir().unwrap();
    assert!(cards_in(dir.path()).is_empty());
    assert!(cards_in(&dir.path().join("absent")).is_empty());
}

#[test]
fn samples_amdgpu() {
    let sys = Sys::new();
    let dev = sys.card("card1", "0000:03:00.0", "amdgpu", RX_7900_XTX);
    let cards = cards_in(&sys.drm());
    let mut s = GpuSampler::new(&cards[0], &cards);
    assert!(s.ready);
    assert!(matches!(s.load, Load::Busy(_)));
    assert_eq!(s.power_limit(), Some(327.0));

    let g = s.sample();
    assert_eq!(g.usage, Some(37.0));
    assert_eq!(g.memory_used, Some(2_147_483_648));
    assert_eq!(g.memory_total, Some(25_753_026_560));
    assert_eq!(g.gtt_used, Some(104_857_600));
    assert_eq!(g.temperature, Some(52.0));
    assert_eq!(g.power, Some(89.0));
    assert!(!g.asleep);

    // A suspended card is left alone.
    fs::write(dev.join("power/runtime_status"), "suspended\n").unwrap();
    let g = s.sample();
    assert!(g.asleep);
    assert_eq!(g.usage, Some(0.0));
    assert_eq!(g.memory_used, None);
    assert_eq!(g.temperature, None);
    assert_eq!(g.memory_total, Some(25_753_026_560));

    fs::write(dev.join("power/runtime_status"), "active\n").unwrap();
    assert!(!s.sample().asleep);
    // A busy file out of range is capped.
    fs::write(dev.join("gpu_busy_percent"), "250\n").unwrap();
    assert_eq!(s.sample().usage, Some(100.0));
}

/// A card asleep when the sampler is made is left asleep: nothing but its
/// runtime status is opened until it wakes.
#[test]
fn waits_for_a_sleeping_card() {
    let sys = Sys::new();
    let dev = sys.card("card1", "0000:03:00.0", "amdgpu", RX_7900_XTX);
    fs::write(dev.join("power/runtime_status"), "suspended\n").unwrap();
    let cards = cards_in(&sys.drm());
    let mut s = GpuSampler::new(&cards[0], &cards);
    assert!(!s.ready);
    assert!(matches!(s.load, Load::None));
    assert_eq!(s.power_limit(), None);
    assert!(s.sample().asleep);
    assert!(!s.ready);

    fs::write(dev.join("power/runtime_status"), "active\n").unwrap();
    // The first awake tick takes the baselines and shows no figures.
    let g = s.sample();
    assert!(s.ready);
    assert!(!g.asleep);
    assert_eq!(g.usage, None);
    assert_eq!(s.power_limit(), Some(327.0));
    let g = s.sample();
    assert_eq!(g.usage, Some(37.0));
    assert_eq!(g.power, Some(89.0));
    assert_eq!(s.power_limit(), Some(327.0));
}

/// A card with no busy file falls back to its clients' counters, and a
/// driver clock file stands in for hwmon's.
#[test]
fn samples_other_drivers() {
    let sys = Sys::new();
    let dev = sys.card(
        "card0",
        "0000:00:02.0",
        "xe",
        &[
            ("vendor", "0x8086\n"),
            ("tile0/gt0/freq0/act_freq", "1550\n"),
        ],
    );
    let cards = cards_in(&sys.drm());
    let mut s = GpuSampler::new(&cards[0], &cards);
    assert!(matches!(s.load, Load::Engines(_)));
    let g = s.sample();
    assert_eq!(g.core_clock, Some(1550.0));
    assert_eq!(g.memory_used, None);
    assert_eq!(g.temperature, None);
    // No runtime PM file: never asleep.
    assert!(!g.asleep);
    // The fake card's slot matches no real client, so there is no load.
    assert_eq!(g.usage, None);
    drop(dev);
}

/// Live: whatever cards this machine has, the readings hang together.
#[test]
fn live_invariants() {
    let cards = cards();
    for card in &cards {
        assert!(!card.name.is_empty());
        assert!(card.node.starts_with("card"));
        let mut s = GpuSampler::new(card, &cards);
        let g = s.sample();
        if let Some(u) = g.usage {
            assert!((0.0..=100.0).contains(&u), "{} usage {u}", card.name);
        }
        if let (Some(used), Some(total)) = (g.memory_used, g.memory_total) {
            assert!(used <= total, "{} memory {used} of {total}", card.name);
        }
        if let Some(t) = g.temperature {
            assert!((-40.0..150.0).contains(&t), "{} at {t} °C", card.name);
        }
        if let Some(p) = g.fan_percent {
            assert!((0.0..=100.0).contains(&p));
        }
    }
}

/// Intel: the load is the time out of RC6, which counts the compositor.
#[test]
fn samples_idle_residency() {
    let sys = Sys::new();
    sys.card(
        "card0",
        "0000:00:02.0",
        "i915",
        &[("vendor", "0x8086\n"), ("device", "0xa780\n")],
    );
    let rc6 = sys.drm().join("card0/gt/gt0/rc6_residency_ms");
    fs::create_dir_all(rc6.parent().unwrap()).unwrap();
    fs::write(&rc6, "1000\n").unwrap();
    let cards = cards_in(&sys.drm());
    let mut s = GpuSampler::new(&cards[0], &cards);
    assert!(matches!(s.load, Load::Idle(..)));
    // Idle the whole time (and then some: the clamp).
    std::thread::sleep(std::time::Duration::from_millis(20));
    fs::write(&rc6, "100000\n").unwrap();
    assert_eq!(s.sample().usage, Some(0.0));
    // Never idle.
    std::thread::sleep(std::time::Duration::from_millis(20));
    assert_eq!(s.sample().usage, Some(100.0));
    // A counter that went back is no reading.
    fs::write(&rc6, "5\n").unwrap();
    assert_eq!(s.sample().usage, None);
    // The client-counting variant replaces it.
    let s = GpuSampler::counting_clients(&cards[0], &cards);
    assert!(matches!(s.load, Load::Engines(_)));
}

#[test]
fn busy_from_idle_time() {
    assert_eq!(busy_from_idle(250, 1.0), Some(75.0));
    assert_eq!(busy_from_idle(0, 2.0), Some(100.0));
    assert_eq!(busy_from_idle(5000, 1.0), Some(0.0));
    assert_eq!(busy_from_idle(0, 0.0), None);
}

/// A card showing a screen, or with runtime power management off, is read
/// for the sidebar; a laptop's spare discrete GPU is left to sleep, and a
/// sleeping card's connectors aren't looked at.
#[test]
fn stays_awake_with_a_screen_or_without_runtime_pm() {
    let sys = Sys::new();
    let ids = |control: &'static str, status: &'static str| {
        let mut f = vec![("vendor", "0x1002\n"), ("device", "0x744c\n")];
        if !control.is_empty() {
            f.push(("power/control", control));
        }
        if !status.is_empty() {
            f.push(("power/runtime_status", status));
        }
        f
    };
    sys.card(
        "card0",
        "0000:03:00.0",
        "amdgpu",
        &ids("auto\n", "active\n"),
    );
    sys.card(
        "card1",
        "0000:04:00.0",
        "amdgpu",
        &ids("auto\n", "active\n"),
    );
    sys.card("card2", "0000:05:00.0", "amdgpu", &ids("on\n", "active\n"));
    sys.card("card3", "0000:06:00.0", "amdgpu", &ids("", ""));
    sys.card(
        "card4",
        "0000:07:00.0",
        "amdgpu",
        &ids("auto\n", "suspended\n"),
    );
    sys.card(
        "card5",
        "0000:08:00.0",
        "virtio-pci",
        &ids("auto\n", "unsupported\n"),
    );
    let connector = |card: &str, name: &str, enabled: &str| {
        let dir = sys.drm().join(card).join(name);
        fs::create_dir_all(&dir).unwrap();
        fs::write(dir.join("enabled"), enabled).unwrap();
    };
    connector("card0", "card0-DP-1", "disabled\n");
    connector("card0", "card0-eDP-1", "enabled\n");
    connector("card1", "card1-HDMI-A-1", "disabled\n");
    // Looked at, this would count; asleep, the card's connectors aren't.
    connector("card4", "card4-eDP-1", "enabled\n");
    // No runtime PM in the driver (a VM's card): it never sleeps.
    connector("card5", "card5-Virtual-1", "enabled\n");
    // A status that says connected doesn't count: reading it can probe.
    fs::write(sys.drm().join("card1/card1-HDMI-A-1/status"), "connected\n").unwrap();

    let cards = cards_in(&sys.drm());
    let awake = |node: &str| cards.iter().find(|c| c.node == node).unwrap().stays_awake();
    assert!(awake("card0"));
    assert!(!awake("card1"));
    assert!(awake("card2"));
    assert!(!awake("card3"));
    assert!(!awake("card4"));
    assert!(awake("card5"));
}
