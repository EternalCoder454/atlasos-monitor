//! The fixtures are this machine's `/proc/bus/input/devices` and a few of its
//! udev database entries (ID properties only). The database snippets and the
//! sysfs trees are written by hand, small, with the shapes the real ones have.

use std::fs;
use std::path::Path;

use super::ids::{Wanted, resolve};
use super::input::{self, Block, classify, parse_blocks, parse_udev, udev_props};
use super::*;

const INPUT: &str = include_str!("../../tests/fixtures/hw_input_devices");
const UDEV_KEYBOARD: &str = include_str!("../../tests/fixtures/hw_udev_keyboard");
const UDEV_MOUSE: &str = include_str!("../../tests/fixtures/hw_udev_mouse");
const UDEV_POWER: &str = include_str!("../../tests/fixtures/hw_udev_powerbutton");
const UDEV_HDA: &str = include_str!("../../tests/fixtures/hw_udev_hda_jack");
const UDEV_NOT_INPUT: &str = include_str!("../../tests/fixtures/hw_udev_asrock_led");

/// A laptop touchpad and touchscreen, as the kernel prints them.
const LAPTOP: &str = "\
I: Bus=0018 Vendor=04f3 Product=3140 Version=0100
N: Name=\"ELAN1200:00 04F3:3140 Touchpad\"
H: Handlers=mouse2 event6 
B: PROP=5
B: EV=b
B: KEY=e520 10000 0 0 0 0
B: ABS=2e0800000000003

I: Bus=0018 Vendor=04f3 Product=2a1b Version=0100
N: Name=\"ELAN Touchscreen\"
H: Handlers=mouse3 event7 
B: PROP=2
B: EV=b
B: KEY=400 0 0 0 0 0
B: ABS=6618000000000003

I: Bus=0011 Vendor=0001 Product=0001 Version=ab41
N: Name=\"AT Translated Set 2 keyboard\"
H: Handlers=sysrq kbd leds event4 
B: PROP=0
B: EV=120013
B: KEY=402000000 3803078f800d001 feffffdfffefffff fffffffffffffffe

I: Bus=0005 Vendor=054c Product=09cc Version=8111
N: Name=\"Wireless Controller\"
H: Handlers=event9 js1 
B: PROP=0
B: EV=20000b
B: KEY=7cdb000000000000 0 0 0 0
B: ABS=3003f
";

fn blocks() -> Vec<Block> {
    parse_blocks(INPUT)
}

fn kind_of(text: &str, name: &str) -> InputKind {
    let d = parse_input_devices(text);
    d.iter().find(|d| d.name == name).unwrap().kind
}

// input

#[test]
fn input_blocks_parse() {
    let b = blocks();
    assert!(b.len() >= 10);
    assert!(b.iter().all(|b| !b.name.is_empty()));
    let kb = b.iter().find(|b| b.name == "Keychron Lemokey X4").unwrap();
    assert_eq!(kb.bus, 3);
    assert!(kb.handlers.contains(&"kbd".to_owned()));
    assert_eq!(kb.key.len(), 4);
}

#[test]
fn input_kinds_from_the_block_alone() {
    assert_eq!(kind_of(INPUT, "Keychron Lemokey X4"), InputKind::Keyboard);
    assert_eq!(
        kind_of(INPUT, "Logitech G502 HERO Gaming Mouse"),
        InputKind::Mouse
    );
    assert_eq!(kind_of(INPUT, "Power Button"), InputKind::Buttons);
    assert_eq!(kind_of(INPUT, "Sleep Button"), InputKind::Buttons);
    assert_eq!(kind_of(INPUT, "HDA Intel PCH Line"), InputKind::Other);
    assert_eq!(kind_of(INPUT, "PC Speaker"), InputKind::Buttons);
    assert_eq!(kind_of(INPUT, "ASRock LED Controller"), InputKind::Joystick);
    assert_eq!(
        kind_of(LAPTOP, "ELAN1200:00 04F3:3140 Touchpad"),
        InputKind::Touchpad
    );
    assert_eq!(kind_of(LAPTOP, "ELAN Touchscreen"), InputKind::Touchscreen);
    assert_eq!(
        kind_of(LAPTOP, "AT Translated Set 2 keyboard"),
        InputKind::Keyboard
    );
    assert_eq!(kind_of(LAPTOP, "Wireless Controller"), InputKind::Joystick);
}

#[test]
fn touchscreens_and_pens_from_the_block() {
    let touch = "\
I: Bus=0003 Vendor=1 Product=2 Version=3
N: Name=\"Touch\"
H: Handlers=event2 
B: PROP=2
B: EV=b
B: ABS=260800000000003
";
    // PROP=2 is INPUT_PROP_DIRECT.
    assert_eq!(kind_of(touch, "Touch"), InputKind::Touchscreen);
    let pen = touch
        .replace("B: ABS", "B: KEY=1 0 0 0 0 0\nB: ABS")
        .replace("Touch", "Pen");
    // BTN_TOOL_PEN, 0x140 = 320, is bit 0 of the sixth word from the right.
    assert_eq!(kind_of(&pen, "Pen"), InputKind::Tablet);
}

#[test]
fn input_kinds_from_udev() {
    let props = |db| parse_udev(db);
    let b = Block::default();
    assert_eq!(
        classify(&b, Some(&props(UDEV_KEYBOARD))),
        InputKind::Keyboard
    );
    assert_eq!(classify(&b, Some(&props(UDEV_MOUSE))), InputKind::Mouse);
    assert_eq!(classify(&b, Some(&props(UDEV_POWER))), InputKind::Buttons);
    assert_eq!(classify(&b, Some(&props(UDEV_HDA))), InputKind::Other);
    // Looked at by udev and not an input device, whatever the block says.
    assert_eq!(classify(&b, Some(&props(UDEV_NOT_INPUT))), InputKind::Other);
    // Several tags: the most specific wins.
    let tags = |t: &[&str]| t.iter().map(|s| s.to_string()).collect::<Vec<_>>();
    assert_eq!(
        classify(
            &b,
            Some(&tags(&[
                "ID_INPUT=1",
                "ID_INPUT_KEY=1",
                "ID_INPUT_TOUCHPAD=1",
                "ID_INPUT_MOUSE=1"
            ]))
        ),
        InputKind::Touchpad
    );
    assert_eq!(
        classify(
            &b,
            Some(&tags(&["ID_INPUT_MOUSE=0", "ID_INPUT_KEYBOARD=1"]))
        ),
        InputKind::Keyboard
    );
}

#[test]
fn udev_joystick_needs_a_js_handler() {
    let tags = vec![
        "ID_INPUT=1".to_owned(),
        "ID_INPUT_JOYSTICK=1".to_owned(),
        "ID_INPUT_KEY=1".to_owned(),
    ];
    let mut b = Block::default();
    assert_eq!(classify(&b, Some(&tags)), InputKind::Buttons);
    b.handlers = vec!["event5".into(), "js0".into()];
    assert_eq!(classify(&b, Some(&tags)), InputKind::Joystick);
}

#[test]
fn udev_beats_the_block() {
    // The Keychron "Keyboard" block says keyboard; udev says mouse.
    let out = input::devices(INPUT, |_| Some(parse_udev(UDEV_MOUSE)));
    assert!(out.iter().all(|d| d.kind == InputKind::Mouse));
}

#[test]
fn input_list_is_sorted_and_deduplicated() {
    let d = parse_input_devices(INPUT);
    let keys: Vec<_> = d
        .iter()
        .map(|d| (d.kind, d.name.clone(), d.bus.clone()))
        .collect();
    let mut sorted = keys.clone();
    sorted.sort();
    sorted.dedup();
    assert_eq!(keys, sorted);
    // The two "Power Button"s are one entry.
    assert_eq!(d.iter().filter(|d| d.name == "Power Button").count(), 1);
    let doubled = format!("{LAPTOP}\n{LAPTOP}");
    assert_eq!(
        parse_input_devices(&doubled).len(),
        parse_input_devices(LAPTOP).len()
    );
}

#[test]
fn input_buses() {
    let d = parse_input_devices(INPUT);
    let bus = |n: &str| d.iter().find(|d| d.name == n).unwrap().bus.clone();
    assert_eq!(bus("Keychron Lemokey X4"), "USB");
    assert_eq!(bus("Power Button"), "Built-in");
    assert_eq!(bus("PC Speaker"), "Built-in");
    assert_eq!(bus("HDA Intel PCH Line"), "");
    let l = parse_input_devices(LAPTOP);
    let bus = |n: &str| l.iter().find(|d| d.name == n).unwrap().bus.clone();
    assert_eq!(bus("Wireless Controller"), "Bluetooth");
    assert_eq!(bus("ELAN Touchscreen"), "Built-in");
    assert_eq!(input::bus_name(6), "Virtual");
}

#[test]
fn input_kind_keys() {
    let all = [
        InputKind::Keyboard,
        InputKind::Mouse,
        InputKind::Touchpad,
        InputKind::Touchscreen,
        InputKind::Tablet,
        InputKind::Joystick,
        InputKind::Buttons,
        InputKind::Other,
    ];
    let keys: Vec<_> = all.iter().map(|k| k.key()).collect();
    assert_eq!(
        keys,
        [
            "keyboard",
            "mouse",
            "touchpad",
            "touchscreen",
            "tablet",
            "joystick",
            "buttons",
            "other"
        ]
    );
    assert_eq!(InputKind::default(), InputKind::Other);
}

#[test]
fn input_survives_garbage() {
    for text in [
        "",
        "\n\n\n",
        "N: Name=\"x\"",
        "N: Name=\"\u{1b}[31mred\u{7}\"\nH: Handlers=kbd\nB: KEY=zz\nB: PROP=\n",
        "I: Bus=zzzz\nN: Name=\nB: KEY=ffffffffffffffff ffffffffffffffff\n\nH: nonsense",
        "B: ABS=1 2 3 4 5 6 7 8 9 a b c d e f 10 11 12 13 14 15 16 17 18 19 1a 1b 1c 1d 1e 1f 20 21 22 23 24 25 26 27 28 29 2a 2b 2c 2d 2e 2f 30 31 32 33 34 35 36 37 38 39 3a 3b 3c 3d 3e 3f 40 41 42",
    ] {
        let _ = parse_input_devices(text);
    }
    let d = parse_input_devices("I: Bus=0003\nN: Name=\"\u{1b}[31mred\u{7}\"\nH: Handlers=kbd\n");
    assert_eq!(d[0].name, "[31mred");
}

#[test]
fn udev_props_follow_the_event_node() {
    let dir = tempfile::tempdir().unwrap();
    let class = dir.path().join("class");
    let udev = dir.path().join("udev");
    fs::create_dir_all(class.join("event3")).unwrap();
    fs::create_dir_all(&udev).unwrap();
    fs::write(class.join("event3/dev"), "13:67\n").unwrap();
    let b = blocks()
        .into_iter()
        .find(|b| b.name == "Keychron Lemokey X4")
        .unwrap();
    // No database entry yet.
    assert_eq!(udev_props(&b, &class, &udev), None);
    fs::write(udev.join("c13:67"), UDEV_KEYBOARD).unwrap();
    let props = udev_props(&b, &class, &udev).unwrap();
    assert_eq!(classify(&b, Some(&props)), InputKind::Keyboard);
    // An entry without properties is not an answer.
    fs::write(udev.join("c13:67"), "I:123\nS:input/by-id/x\n").unwrap();
    assert_eq!(udev_props(&b, &class, &udev), None);
    // A hostile dev file never leaves the udev directory.
    fs::write(class.join("event3/dev"), "13:../../x\n").unwrap();
    assert_eq!(udev_props(&b, &class, &udev), None);
}

// ids

const DB: &str = "\
# comment
1002  Advanced Micro Devices, Inc. [AMD/ATI]
\t744c  Navi 31 [Radeon RX 7900 XT/7900 XTX/7900M]
\t\t1eae 7901  RX-79XMERCB9
\t7480  Navi 33
10de  NVIDIA Corporation
\t2684  AD102 [GeForce RTX 4090]
8086  Intel Corporation
\ta780  Raptor Lake-S GT1 [UHD Graphics 770]
AT 0001  Not a vendor
HID 00  Nor this

# List of known device classes
C 00  Unclassified device
\t00  Non-VGA unclassified device
C 03  Display controller
\t00  VGA compatible controller
\t\t00  VGA controller
\t80  Display controller
C 0c  Serial bus controller
\t03  USB controller
";

fn wanted(
    vendors: &[u16],
    devices: &[(u16, u16)],
    classes: &[u8],
    subclasses: &[(u8, u8)],
) -> Wanted {
    Wanted {
        vendors: vendors.iter().copied().collect(),
        devices: devices.iter().copied().collect(),
        classes: classes.iter().copied().collect(),
        subclasses: subclasses.iter().copied().collect(),
    }
}

#[test]
fn one_pass_resolves_everything() {
    let w = wanted(
        &[0x1002, 0x8086, 0xdead],
        &[(0x1002, 0x744c), (0x8086, 0xa780), (0x8086, 0xffff)],
        &[0x03, 0x0c, 0x77],
        &[(0x03, 0x00), (0x0c, 0x03), (0x03, 0x99)],
    );
    let n = resolve(DB.as_bytes(), &w);
    assert_eq!(n.vendors.len(), 2);
    assert_eq!(n.vendors[&0x1002], "Advanced Micro Devices, Inc. [AMD/ATI]");
    assert_eq!(n.vendors[&0x8086], "Intel Corporation");
    assert_eq!(n.devices.len(), 2);
    assert_eq!(
        n.devices[&(0x1002, 0x744c)],
        "Navi 31 [Radeon RX 7900 XT/7900 XTX/7900M]"
    );
    assert_eq!(
        n.devices[&(0x8086, 0xa780)],
        "Raptor Lake-S GT1 [UHD Graphics 770]"
    );
    assert_eq!(n.classes[&0x03], "Display controller");
    assert_eq!(n.classes[&0x0c], "Serial bus controller");
    assert_eq!(n.classes.len(), 2);
    assert_eq!(n.subclasses[&(0x03, 0x00)], "VGA compatible controller");
    assert_eq!(n.subclasses[&(0x0c, 0x03)], "USB controller");
    assert_eq!(n.subclasses.len(), 2);
}

#[test]
fn ids_are_not_confused() {
    // A subsystem line is not a device, a class code is not a vendor, a
    // device id under another vendor does not count.
    let w = wanted(
        &[0x0c, 0x1eae],
        &[(0x10de, 0x744c), (0x1002, 0x7901)],
        &[],
        &[],
    );
    let n = resolve(DB.as_bytes(), &w);
    assert_eq!(n, Default::default());
    // "AT 0001" and "HID 00" are not vendors.
    let n = resolve(DB.as_bytes(), &wanted(&[0xa7], &[], &[], &[]));
    assert!(n.vendors.is_empty());
}

#[test]
fn empty_and_broken_databases() {
    let w = wanted(&[0x1002], &[(0x1002, 0x744c)], &[3], &[(3, 0)]);
    assert_eq!(resolve(&b""[..], &w), Default::default());
    assert_eq!(
        resolve(&b"\xff\xfe\x00\n\t\t\n\t"[..], &w),
        Default::default()
    );
    assert_eq!(
        resolve(DB.as_bytes(), &Wanted::default()),
        Default::default()
    );
    // Truncated mid-line.
    let n = resolve(&DB.as_bytes()[..80], &w);
    assert!(n.vendors.contains_key(&0x1002));
    // No database file at all.
    assert_eq!(
        ids::resolve_files(&["/nonexistent/pci.ids"], &w),
        Default::default()
    );
}

// PCI

fn pci_node(
    root: &Path,
    slot: &str,
    vendor: &str,
    device: &str,
    class: &str,
    driver: Option<&str>,
) {
    let d = root.join(slot);
    fs::create_dir_all(&d).unwrap();
    fs::write(d.join("vendor"), format!("{vendor}\n")).unwrap();
    fs::write(d.join("device"), format!("{device}\n")).unwrap();
    fs::write(d.join("class"), format!("{class}\n")).unwrap();
    if let Some(drv) = driver {
        let target = root.join("drivers").join(drv);
        fs::create_dir_all(&target).unwrap();
        std::os::unix::fs::symlink(&target, d.join("driver")).unwrap();
    }
}

#[test]
fn pci_devices_are_named_and_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("devices");
    pci_node(
        &root,
        "0000:0c:00.0",
        "0x8086",
        "0xa780",
        "0x0c0330",
        Some("xhci_hcd"),
    );
    pci_node(
        &root,
        "0000:03:00.1",
        "0x1002",
        "0x744c",
        "0x030000",
        Some("amdgpu"),
    );
    pci_node(
        &root,
        "0000:03:00.0",
        "0x1002",
        "0x744c",
        "0x030000",
        Some("amdgpu"),
    );
    // Unknown vendor and device, known subclass; unknown class entirely; no driver.
    pci_node(&root, "0000:00:02.0", "0xbeef", "0x0001", "0x038000", None);
    pci_node(&root, "0000:00:03.0", "0xbeef", "0x0002", "0xee0000", None);
    // Unnamed too, and below the display class: still with the other
    // unnamed one, at the end, so "Other" is one group.
    pci_node(&root, "0000:00:04.0", "0xbeef", "0x0003", "0x010000", None);
    // Not devices.
    fs::create_dir_all(root.join("not-a-slot")).unwrap();
    pci_node(&root, "0000:00:09.0", "garbage", "0x1", "0x0", None);
    let db = dir.path().join("pci.ids");
    fs::write(&db, DB).unwrap();

    let out = read_pci(&root, &[db.to_str().unwrap()]);
    let slots: Vec<_> = out.iter().map(|d| d.slot.as_str()).collect();
    assert_eq!(
        slots,
        [
            "0000:03:00.0",
            "0000:03:00.1",
            "0000:00:02.0",
            "0000:0c:00.0",
            "0000:00:04.0",
            "0000:00:03.0"
        ]
    );
    let gpu = &out[0];
    assert_eq!(gpu.class, "Display controller");
    assert_eq!(gpu.class_code, 0x030000);
    assert_eq!(gpu.vendor, "Advanced Micro Devices, Inc. [AMD/ATI]");
    assert_eq!(gpu.name, "Navi 31 [Radeon RX 7900 XT/7900 XTX/7900M]");
    assert_eq!(gpu.driver, "amdgpu");
    let odd = &out[2];
    assert_eq!(odd.vendor, "Vendor beef");
    assert_eq!(odd.name, "Display controller"); // the subclass 0x80's name
    assert_eq!(odd.driver, "");
    let unknown = &out[5];
    assert_eq!(unknown.class, "Other");
    assert_eq!(unknown.name, "Device 0002");
    assert_eq!(out[4].class, "Other");
}

#[test]
fn pci_without_a_database_or_bus() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("devices");
    pci_node(&root, "0000:03:00.0", "0x1002", "0x744c", "0x030000", None);
    let out = read_pci(&root, &["/nonexistent/pci.ids"]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].vendor, "Vendor 1002");
    assert_eq!(out[0].name, "Device 744c");
    assert_eq!(out[0].class, "Other");
    assert!(read_pci(&dir.path().join("absent"), &[]).is_empty());
}

#[test]
fn slots() {
    for ok in ["0000:03:00.0", "0000:00:1f.6", "10000:e0:06.0"] {
        assert!(valid_slot(ok), "{ok}");
    }
    for bad in [
        "",
        "drivers",
        "0000:03:00",
        "0000:03:00.00",
        "0000:03:00.0/..",
        "00:00.0",
    ] {
        assert!(!valid_slot(bad), "{bad}");
    }
}

// USB

fn usb_node(root: &Path, name: &str, attrs: &[(&str, &[u8])]) {
    let d = root.join(name);
    fs::create_dir_all(&d).unwrap();
    for (k, v) in attrs {
        fs::write(d.join(k), v).unwrap();
    }
}

const USB_DB: &str = "\
046d  Logitech, Inc.
\tc08b  G502 HERO Gaming Mouse
1d6b  Linux Foundation
\t0002  2.0 root hub
AT 0001  Not a vendor

C 09  Hub
\t00  Unused
";

#[test]
fn usb_devices_are_named_and_sorted() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("devices");
    usb_node(
        &root,
        "1-3",
        &[
            ("idVendor", b"362d\n"),
            ("idProduct", b"0240\n"),
            ("manufacturer", b"Keychron\n"),
            ("product", b"Lemokey X4\n"),
            ("speed", b"12\n"),
            ("bDeviceClass", b"00\n"),
        ],
    );
    // Names only from the database.
    usb_node(
        &root,
        "1-10",
        &[
            ("idVendor", b"046d\n"),
            ("idProduct", b"c08b\n"),
            ("speed", b"480\n"),
            ("bDeviceClass", b"00\n"),
        ],
    );
    usb_node(
        &root,
        "1-2",
        &[
            ("idVendor", b"1d6b\n"),
            ("idProduct", b"0002\n"),
            ("speed", b"5000\n"),
            ("bDeviceClass", b"09\n"),
        ],
    );
    // Unknown to everything, hostile text, no speed.
    usb_node(
        &root,
        "2-9.1",
        &[
            ("idVendor", b"beef\n"),
            ("idProduct", b"0001\n"),
            ("manufacturer", b"\x1b[31mEvil\x07\n\n"),
            ("product", &[b'x'; 5000]),
        ],
    );
    usb_node(
        &root,
        "2-9",
        &[
            ("idVendor", b"0001\n"),
            ("idProduct", b"0002\n"),
            ("manufacturer", b"\xff\xfe bad \xc3"),
            ("product", b"   \n"),
        ],
    );
    // Skipped: root hubs, interfaces, device with no ID, junk.
    usb_node(
        &root,
        "usb1",
        &[("idVendor", b"1d6b\n"), ("idProduct", b"0002\n")],
    );
    usb_node(
        &root,
        "1-3:1.0",
        &[("idVendor", b"362d\n"), ("idProduct", b"0240\n")],
    );
    usb_node(&root, "1-5", &[]);
    usb_node(&root, "-", &[("idVendor", b"1\n"), ("idProduct", b"1\n")]);
    let db = dir.path().join("usb.ids");
    fs::write(&db, USB_DB).unwrap();

    let out = read_usb(&root, &[db.to_str().unwrap()]);
    let paths: Vec<_> = out.iter().map(|d| d.path.as_str()).collect();
    assert_eq!(paths, ["1-2", "1-3", "1-10", "2-9", "2-9.1"]);
    assert_eq!(out[0].name, "2.0 root hub");
    assert_eq!(out[0].vendor, "Linux Foundation");
    assert!(out[0].hub);
    assert_eq!(out[0].speed, "USB 3 (5 Gbit/s)");
    assert_eq!(out[1].vendor, "Keychron");
    assert_eq!(out[1].name, "Lemokey X4");
    assert_eq!(out[1].id, "362d:0240");
    assert_eq!(out[1].speed, "USB 1 (12 Mbit/s)");
    assert!(!out[1].hub);
    assert_eq!(out[2].vendor, "Logitech, Inc.");
    assert_eq!(out[2].name, "G502 HERO Gaming Mouse");
    assert_eq!(out[2].speed, "USB 2 (480 Mbit/s)");
    // Device-controlled text: lossy, no blank names, nothing control, capped.
    assert_eq!(out[3].vendor, "\u{fffd}\u{fffd} bad \u{fffd}");
    assert_eq!(out[3].name, "Device 0002");
    assert_eq!(out[4].vendor, "[31mEvil");
    assert_eq!(out[4].name.chars().count(), 128);
    assert_eq!(out[4].speed, "");
    assert_eq!(out[4].vendor.chars().filter(|c| c.is_control()).count(), 0);
}

#[test]
fn usb_without_a_database_or_bus() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path().join("devices");
    usb_node(
        &root,
        "1-1",
        &[("idVendor", b"362d"), ("idProduct", b"0240")],
    );
    let out = read_usb(&root, &["/nonexistent/usb.ids"]);
    assert_eq!(out.len(), 1);
    assert_eq!(out[0].vendor, "Vendor 362d");
    assert_eq!(out[0].name, "Device 0240");
    assert_eq!(out[0].id, "362d:0240");
    assert!(read_usb(&dir.path().join("absent"), &[]).is_empty());
}

#[test]
fn usb_path_order_is_natural() {
    let mut keys: Vec<_> = ["1-10", "1-2", "2-1", "1-2.1", "1-2.10", "1-2.2", "10-1"]
        .iter()
        .map(|p| (usb_path_key(p).unwrap(), *p))
        .collect();
    keys.sort();
    let order: Vec<_> = keys.iter().map(|k| k.1).collect();
    assert_eq!(
        order,
        ["1-2", "1-2.1", "1-2.2", "1-2.10", "1-10", "2-1", "10-1"]
    );
    for bad in [
        "usb1",
        "1-3:1.0",
        "",
        "-",
        "1-",
        "a-b",
        "1--2",
        "99999999999-1",
    ] {
        assert_eq!(usb_path_key(bad), None, "{bad}");
    }
}

#[test]
fn usb_speeds() {
    for (mbps, want) in [
        ("1.5", "USB 1 (1.5 Mbit/s)"),
        ("12", "USB 1 (12 Mbit/s)"),
        ("12\n", "USB 1 (12 Mbit/s)"),
        ("480", "USB 2 (480 Mbit/s)"),
        ("5000", "USB 3 (5 Gbit/s)"),
        ("10000", "USB 3 (10 Gbit/s)"),
        ("20000", "USB 3 (20 Gbit/s)"),
        ("40000", "USB4 (40 Gbit/s)"),
        ("80000", "USB4 (80 Gbit/s)"),
        ("2500", "USB 3 (2.5 Gbit/s)"),
        ("", ""),
        ("unknown", ""),
        ("-5", ""),
        ("0", ""),
        ("NaN", ""),
        ("inf", ""),
        ("1e300", ""),
    ] {
        assert_eq!(usb_speed(mbps), want, "{mbps:?}");
    }
}

#[test]
fn clean_text() {
    assert_eq!(clean("  a\tb\nc\u{7f} "), "a b c");
    assert_eq!(clean(&"é".repeat(300)).chars().count(), 128);
    assert_eq!(clean(&format!("{} z", "a".repeat(127))), "a".repeat(127));
    assert_eq!(clean(""), "");
}

// live

/// On the real system: nothing panics, and whatever is listed is well formed.
#[test]
fn live_read_invariants() {
    let hw = read();
    for d in &hw.pci {
        assert!(valid_slot(&d.slot), "{}", d.slot);
        assert!(!d.class.is_empty() && !d.vendor.is_empty() && !d.name.is_empty());
        assert!(d.class_code <= 0xff_ffff);
    }
    assert!(
        hw.pci
            .windows(2)
            .all(|w| (w[0].class_code, &w[0].slot) <= (w[1].class_code, &w[1].slot))
    );
    for d in &hw.usb {
        assert_eq!(d.id.len(), 9, "{}", d.id);
        assert_eq!(d.id.as_bytes()[4], b':');
        assert!(usb_path_key(&d.path).is_some(), "{}", d.path);
        assert!(!d.vendor.is_empty() && !d.name.is_empty());
        assert!(d.speed.is_empty() || d.speed.starts_with("USB"));
    }
    assert!(
        hw.usb
            .windows(2)
            .all(|w| usb_path_key(&w[0].path) <= usb_path_key(&w[1].path))
    );
    for d in &hw.input {
        assert!(!d.name.is_empty());
        assert!(!d.name.chars().any(char::is_control));
    }
    assert!(
        hw.input
            .windows(2)
            .all(|w| (w[0].kind, &w[0].name) <= (w[1].kind, &w[1].name))
    );
}

/// Run with `--release --nocapture --ignored` to see the time and a sample.
#[test]
#[ignore = "timing and sample output"]
fn live_timing() {
    let first = std::time::Instant::now();
    let hw = read();
    println!("cold-ish read: {:?}", first.elapsed());
    let t = std::time::Instant::now();
    for _ in 0..20 {
        std::hint::black_box(read());
    }
    println!("warm read: {:?} each", t.elapsed() / 20);
    println!(
        "pci {} usb {} input {}",
        hw.pci.len(),
        hw.usb.len(),
        hw.input.len()
    );
    for d in hw.pci.iter().take(4) {
        println!("{d:?}");
    }
    for d in hw.usb.iter().take(4) {
        println!("{d:?}");
    }
    for d in hw.input.iter().take(6) {
        println!("{d:?}");
    }
}

#[test]
fn clean_keeps_one_line_without_invisible_characters() {
    // A right-to-left override would show "Keyboard" as "draobyeK".
    assert_eq!(clean("Evil\u{202E}draobyeK"), "EvildraobyeK");
    assert_eq!(clean("\u{200B}Zero\u{FEFF} width\u{2066}"), "Zero width");
    assert_eq!(clean("Two\u{2028}lines\nhere\t\x07"), "Two lines here");
    assert_eq!(clean("  \u{200B}  "), "");
    let long = clean(&"word ".repeat(100));
    assert!(long.chars().count() <= MAX_TEXT);
    assert!(!long.ends_with(' '));
}

#[test]
fn a_name_with_line_breaks_cant_add_a_device() {
    // A name the kernel would print as two blocks in /proc/bus/input/devices
    // is one device, on one line, in sysfs.
    let dir = tempfile::tempdir().unwrap();
    let dev = dir.path().join("input7");
    fs::create_dir_all(dev.join("capabilities")).unwrap();
    fs::create_dir_all(dev.join("id")).unwrap();
    fs::create_dir_all(dev.join("event9")).unwrap();
    fs::write(
        dev.join("name"),
        "Gadget\nH: Handlers=kbd event4\n\nI: Bus=0003\nN: Name=\"Trusted Keyboard\n",
    )
    .unwrap();
    fs::write(dev.join("id/bustype"), "0003\n").unwrap();
    fs::write(dev.join("capabilities/key"), "ffff\n").unwrap();
    fs::write(dev.join("properties"), "0\n").unwrap();
    // Not devices: no number, or not an input.
    fs::create_dir_all(dir.path().join("inputx")).unwrap();
    fs::create_dir_all(dir.path().join("event9")).unwrap();
    let blocks = input::sysfs_blocks(dir.path());
    assert_eq!(blocks.len(), 1);
    let b = &blocks[0];
    assert!(
        b.name
            .starts_with("Gadget H: Handlers=kbd event4 I: Bus=0003")
    );
    assert!(!b.name.contains('\n'));
    assert_eq!(b.bus, 3);
    assert_eq!(b.handlers, ["event9"]);

    // In /proc's text, a block must start with I: and have each line once.
    let text = "P: Phys=x\nN: Name=\"No I line\"\nH: Handlers=kbd event1\n";
    assert!(parse_blocks(text).is_empty());
    let text = "I: Bus=0003\nN: Name=\"Twice\"\nH: Handlers=kbd\nH: Handlers=js0\n";
    assert!(parse_blocks(text).is_empty());
}

#[test]
fn sysfs_keyboards_without_udev() {
    // sysfs has no node for the kbd handler, so without udev's database a
    // keyboard is known by its keys alone.
    let dir = tempfile::tempdir().unwrap();
    let device = |n: u32, name: &str, key: &str, handler: &str| {
        let dev = dir.path().join(format!("input{n}"));
        fs::create_dir_all(dev.join("capabilities")).unwrap();
        fs::create_dir_all(dev.join("id")).unwrap();
        fs::create_dir_all(dev.join(handler)).unwrap();
        fs::write(dev.join("name"), format!("{name}\n")).unwrap();
        fs::write(dev.join("id/bustype"), "0019\n").unwrap();
        fs::write(dev.join("capabilities/key"), format!("{key}\n")).unwrap();
        fs::write(dev.join("capabilities/abs"), "0\n").unwrap();
        fs::write(dev.join("properties"), "0\n").unwrap();
    };
    // Q (16) to P (25), and the power key (116) alone.
    device(1, "AT Keyboard", "3ff0000", "event1");
    device(2, "Power Button", "10000000000000 0", "event2");
    device(3, "No Keys", "0", "event3");
    let found = input::sysfs_devices(dir.path(), |_| None);
    let kind = |name: &str| found.iter().find(|d| d.name == name).map(|d| d.kind);
    assert_eq!(kind("AT Keyboard"), Some(InputKind::Keyboard));
    assert_eq!(kind("Power Button"), Some(InputKind::Buttons));
    assert_eq!(kind("No Keys"), Some(InputKind::Other));
}
