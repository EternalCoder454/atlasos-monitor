//! What this computer is: its operating system, desktop, kernel, maker,
//! model and firmware, and how secure its firmware is (fwupd).
//!
//! Everything here is static for the life of a boot, so System Info reads it
//! once when the page opens. [`read`] collects the files (`os-release`,
//! Plasma's metainfo, `uname`, `/proc/cpuinfo`, the DMI strings in
//! `/sys/class/dmi/id`); each has a pure parser the tests feed with recorded
//! text. A file that is missing gives an empty field, never an error: a
//! container has no DMI, an ARM board has no `model name`. The DMI serial
//! numbers are root-only and are not read at all.
//!
//! Firmware security comes from fwupd's Host Security ID over the system bus
//! ([`security`]), the same data KDE's "Firmware Security" page shows. fwupd
//! is started by the bus on demand, so a reading can take a few seconds the
//! first time; the page calls it from a worker thread.
//!
//! The firmware, the bus and the files are all outside input: every string is
//! stripped of control characters and cut to [`MAX_TEXT`] before it is shown.

use std::collections::HashMap;
use std::fs::File;
use std::io::Read;
use std::time::Duration;

use zbus::Connection;
use zbus::zvariant::{OwnedValue, Value};

use crate::stats::memory;

const DEST: &str = "org.freedesktop.fwupd";
const FWUPD_IF: &str = "org.freedesktop.fwupd";
const PROPS_IF: &str = "org.freedesktop.DBus.Properties";
const BUS: Option<&str> = Some("org.freedesktop.DBus");
const BUS_PATH: &str = "/org/freedesktop/DBus";
const DMI: &str = "/sys/class/dmi/id";
const PLASMA_METAINFO: &str = "/usr/share/metainfo/org.kde.plasmashell.metainfo.xml";
/// Longest string kept from any file or bus reply, in characters.
const MAX_TEXT: usize = 256;
/// Most bytes read from a file. Plasma's metainfo is about 25 KB with the
/// newest release first, and the first CPU's block in `/proc/cpuinfo` is under
/// 2 KB, so neither needs more.
const MAX_FILE: u64 = 64 * 1024;
/// Most attributes taken from fwupd (it reports about 40).
const MAX_ATTRS: usize = 256;
/// Longest one fwupd call may take. The first call can start fwupd, which
/// loads its plugins before it answers; after that it answers from its own
/// cache. Under [`DEADLINE`], which bounds the whole read.
const TIMEOUT: Duration = Duration::from_secs(8);
/// Longest [`security`] may take in all, with fwupd starting up.
pub const DEADLINE: Duration = Duration::from_secs(10);

/// `FWUPD_SECURITY_ATTR_FLAG_SUCCESS`: the attribute passed.
const FLAG_SUCCESS: u64 = 1 << 0;
/// `FWUPD_SECURITY_ATTR_FLAG_OBSOLETED`: another attribute replaced this one.
const FLAG_OBSOLETED: u64 = 1 << 1;

/// The highest level fwupd's Host Security ID has.
const HSI_MAX: u8 = 5;

/// What this computer is. A field that can't be found is empty (`cpu_threads`
/// and `memory`: 0).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct About {
    /// os-release `PRETTY_NAME`, else `NAME VERSION`, else "Linux".
    pub os_name: String,
    /// os-release `LOGO`: an icon name.
    pub os_logo: String,
    /// os-release `HOME_URL`.
    pub os_url: String,
    /// KDE Plasma's version ("6.6.4"); empty when it isn't installed.
    pub plasma: String,
    /// `uname` release ("7.2.7-200.fc44.x86_64").
    pub kernel: String,
    /// `uname` machine ("x86_64").
    pub arch: String,
    /// `uname` nodename.
    pub hostname: String,
    /// The processor's name without the marketing marks and the clock tail:
    /// "Intel Core i9-14900KF".
    pub cpu: String,
    /// Logical processors that are online.
    pub cpu_threads: usize,
    /// Installed memory the kernel sees, in bytes.
    pub memory: u64,
    /// The maker, with a trailing "Inc." or "Co., Ltd." removed.
    pub vendor: String,
    /// The model. A laptop whose `product_name` is a code (Lenovo's
    /// "21AH00CNUS") gets its `product_version` instead ("ThinkPad T14 Gen 3").
    pub product: String,
    /// The motherboard's maker and name.
    pub board: String,
    /// "Vendor version (date)" of the firmware, the date as YYYY-MM-DD.
    pub firmware: String,
    /// "Desktop", "Laptop", ...; empty when the DMI says Other or Unknown.
    pub chassis: String,
}

/// The parts of os-release that System Info shows.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct OsRelease {
    /// `PRETTY_NAME`, else `NAME VERSION`, else `NAME`; empty if none.
    pub name: String,
    /// `LOGO`, an icon name.
    pub logo: String,
    /// `HOME_URL`, an http(s) address.
    pub url: String,
}

/// Reads everything. Never fails: what can't be read stays empty.
pub fn read() -> About {
    let os = ["/etc/os-release", "/usr/lib/os-release"]
        .iter()
        .filter_map(|p| read_text(p, MAX_FILE))
        .map(|t| parse_os_release(&t))
        .find(|o| !o.name.is_empty())
        .unwrap_or_default();
    let (kernel, arch, hostname) = uname();
    let dmi = |name: &str| {
        read_text(&format!("{DMI}/{name}"), 512)
            .as_deref()
            .and_then(tidy_dmi)
    };
    let vendor = dmi("sys_vendor");
    let product = product_name(dmi("product_name"), dmi("product_version"));
    let board = board_name(dmi("board_vendor"), dmi("board_name"));
    let firmware = firmware_text(
        dmi("bios_vendor"),
        dmi("bios_version"),
        dmi("bios_date").as_deref(),
    );
    let chassis = dmi("chassis_type")
        .and_then(|c| c.parse::<u32>().ok())
        .map(chassis_name)
        .unwrap_or_default();
    About {
        os_name: if os.name.is_empty() {
            "Linux".to_owned()
        } else {
            os.name
        },
        os_logo: os.logo,
        os_url: os.url,
        plasma: read_text(PLASMA_METAINFO, MAX_FILE)
            .as_deref()
            .and_then(parse_plasma_version)
            .unwrap_or_default(),
        kernel,
        arch,
        hostname,
        cpu: read_text("/proc/cpuinfo", MAX_FILE)
            .as_deref()
            .and_then(parse_cpu_model)
            .unwrap_or_default(),
        cpu_threads: read_text("/sys/devices/system/cpu/online", 4096)
            .map_or(0, |t| parse_cpu_count(&t)),
        memory: std::fs::read("/proc/meminfo")
            .ok()
            .and_then(|d| memory::parse_meminfo(&d))
            .map_or(0, |m| m.total),
        vendor: vendor.map(|v| short_vendor(&v)).unwrap_or_default(),
        product,
        board,
        firmware,
        chassis: chassis.to_owned(),
    }
}

/// Reads up to `max` bytes of a text file; `None` if it can't be opened.
/// A file cut off in the middle of a character loses only that character.
fn read_text(path: &str, max: u64) -> Option<String> {
    let mut buf = Vec::new();
    File::open(path)
        .ok()?
        .take(max)
        .read_to_end(&mut buf)
        .ok()?;
    Some(String::from_utf8_lossy(&buf).into_owned())
}

/// Kernel release, machine and node name.
fn uname() -> (String, String, String) {
    let u = rustix::system::uname();
    let text = |s: &std::ffi::CStr| clean(&s.to_string_lossy());
    (text(u.release()), text(u.machine()), text(u.nodename()))
}

/// Strips control characters, folds runs of white space into one space and
/// cuts the text to [`MAX_TEXT`] characters.
fn clean(s: &str) -> String {
    let mut out = String::new();
    for word in s.split(|c: char| c.is_whitespace() || c.is_control() || crate::invisible(c)) {
        if word.is_empty() {
            continue;
        }
        if !out.is_empty() {
            out.push(' ');
        }
        out.push_str(word);
        if out.chars().count() >= MAX_TEXT {
            break;
        }
    }
    out.chars().take(MAX_TEXT).collect()
}

/// The os-release fields System Info shows, from the file's text (os-release(5):
/// `KEY=value` lines, the value bare, 'single quoted' or "double quoted"; a
/// later assignment replaces an earlier one).
pub fn parse_os_release(text: &str) -> OsRelease {
    let (mut name, mut version, mut pretty) = (String::new(), String::new(), String::new());
    let (mut logo, mut url) = (String::new(), String::new());
    for line in text.lines() {
        let line = line.trim_start();
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let slot = match key {
            "NAME" => &mut name,
            "VERSION" => &mut version,
            "PRETTY_NAME" => &mut pretty,
            "LOGO" => &mut logo,
            "HOME_URL" => &mut url,
            _ => continue,
        };
        *slot = clean(&unquote(value));
    }
    // Without PRETTY_NAME: NAME and its VERSION. os-release says NAME
    // defaults to "Linux", so a VERSION alone isn't left bare.
    let name = if !pretty.is_empty() {
        pretty
    } else if name.is_empty() && !version.is_empty() {
        clean(&format!("Linux {version}"))
    } else {
        clean(&format!("{name} {version}"))
    };
    let icon_ok = !logo.is_empty()
        && logo.len() <= 128
        && logo
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "-_.+".contains(c));
    let url_ok = url.starts_with("https://") || url.starts_with("http://");
    OsRelease {
        name,
        logo: if icon_ok { logo } else { String::new() },
        url: if url_ok { url } else { String::new() },
    }
}

/// A shell-style value: bare (ends at white space, so a `#` after it starts
/// a comment), 'single quoted'
/// (literal) or "double quoted" (a backslash escapes `$`, `` ` ``, `"` and `\`).
/// Anything after the closing quote is ignored.
fn unquote(raw: &str) -> String {
    let raw = raw.trim_start();
    let mut chars = raw.chars();
    let mut out = String::new();
    match chars.next() {
        Some('\'') => out.extend(chars.take_while(|&c| c != '\'')),
        Some('"') => {
            while let Some(c) = chars.next() {
                match c {
                    '"' => break,
                    '\\' => match chars.next() {
                        Some(e @ ('$' | '`' | '"' | '\\')) => out.push(e),
                        Some(e) => {
                            out.push('\\');
                            out.push(e);
                        }
                        None => out.push('\\'),
                    },
                    c => out.push(c),
                }
            }
        }
        _ => {
            let mut chars = raw.chars();
            while let Some(c) = chars.next() {
                match c {
                    '\\' => out.extend(chars.next()),
                    c if c.is_whitespace() => break,
                    c => out.push(c),
                }
            }
        }
    }
    out
}

/// The newest Plasma release from `org.kde.plasmashell.metainfo.xml`: the
/// `version` of the first `<release>`. `None` unless it is digits and dots.
pub fn parse_plasma_version(metainfo: &str) -> Option<String> {
    let mut rest = metainfo;
    while let Some(at) = rest.find("<release") {
        rest = &rest[at + "<release".len()..];
        // "<releases>" is the list, not a release.
        if !rest.starts_with(|c: char| c.is_ascii_whitespace()) {
            continue;
        }
        let tag = &rest[..rest.find('>')?];
        let value = tag.find("version=").map(|i| &tag[i + "version=".len()..])?;
        let quote = value.chars().next().filter(|&c| c == '"' || c == '\'')?;
        let value = &value[1..];
        let version = &value[..value.find(quote)?];
        let ok = version.len() <= 32
            && version.starts_with(|c: char| c.is_ascii_digit())
            && !version.ends_with('.')
            && !version.contains("..")
            && version.chars().all(|c| c.is_ascii_digit() || c == '.');
        return ok.then(|| version.to_owned());
    }
    None
}

/// The processor's name from `/proc/cpuinfo`, tidied: "Intel(R) Core(TM)
/// i7-8700 CPU @ 3.20GHz" becomes "Intel Core i7-8700" and "AMD Ryzen 9 7950X
/// 16-Core Processor" "AMD Ryzen 9 7950X". Boards without a `model name` (ARM,
/// POWER) give their `Hardware`, `Processor`, `cpu` or `Model` line instead.
pub fn parse_cpu_model(cpuinfo: &str) -> Option<String> {
    const FALLBACKS: [&str; 4] = ["Hardware", "Processor", "cpu", "Model"];
    let mut fallback: [Option<&str>; 4] = [None; 4];
    for line in cpuinfo.lines() {
        let Some((key, value)) = line.split_once(':') else {
            continue;
        };
        let (key, value) = (key.trim(), value.trim());
        if value.is_empty() {
            continue;
        }
        if key == "model name" {
            if let Some(name) = tidy_cpu(value) {
                return Some(name);
            }
        } else if let Some(i) = FALLBACKS.iter().position(|&k| k == key) {
            fallback[i].get_or_insert(value);
        }
    }
    fallback.into_iter().flatten().find_map(tidy_cpu)
}

fn tidy_cpu(raw: &str) -> Option<String> {
    let text = clean(raw)
        .replace("(R)", "")
        .replace("(r)", "")
        .replace("(TM)", "")
        .replace("(tm)", "");
    let mut words: Vec<&str> = text.split_whitespace().collect();
    // "... CPU @ 3.20GHz": the clock is shown elsewhere and changes with load
    // on some chips.
    if let Some(at) = words.iter().position(|w| w.starts_with('@')) {
        words.truncate(at);
    }
    if let [.., cores, "Processor"] = words[..] {
        let n = cores.strip_suffix("-Core").or(cores.strip_suffix("-core"));
        if n.is_some_and(|n| !n.is_empty() && n.chars().all(|c| c.is_ascii_digit())) {
            words.truncate(words.len() - 2);
        }
    }
    if words.last() == Some(&"CPU") {
        words.pop();
    }
    let name = words.join(" ");
    (!name.is_empty()).then_some(name)
}

/// How many processors a CPU list like `0-7,9,12-15` (the kernel's
/// `online` file) names. 0 for text that isn't one.
pub fn parse_cpu_count(list: &str) -> usize {
    // Larger than any kernel's NR_CPUS (8192), so garbage can't claim more.
    const MAX_CPU: usize = 1 << 16;
    let mut count = 0usize;
    for part in list.trim().split(',') {
        let range = match part.split_once('-') {
            Some((a, b)) => a
                .trim()
                .parse::<usize>()
                .ok()
                .zip(b.trim().parse::<usize>().ok()),
            None => part.trim().parse::<usize>().ok().map(|n| (n, n)),
        };
        match range {
            Some((a, b)) if a <= b && b < MAX_CPU => count += b - a + 1,
            _ => return 0,
        }
    }
    count.min(MAX_CPU)
}

/// A DMI string cleaned up, or `None` when it is empty or one of the
/// placeholders makers leave in when they never filled the field in.
pub fn tidy_dmi(raw: &str) -> Option<String> {
    // Compared without case and without trailing dots, so "To Be Filled By
    // O.E.M" matches too.
    const PLACEHOLDERS: [&str; 22] = [
        "to be filled by o.e.m",
        "default string",
        "default company name",
        "system product name",
        "system manufacturer",
        "system version",
        "system serial number",
        "not applicable",
        "not specified",
        "not available",
        "not defined",
        "unknown",
        "undefined",
        "none",
        "default",
        "oem",
        "o.e.m",
        "0123456789",
        "x.x",
        "type1productconfigid",
        "n/a",
        "",
    ];
    let text = clean(raw);
    let key = text.trim_end_matches(['.', ' ']).to_lowercase();
    let filler = key
        .chars()
        .all(|c| matches!(c, '*' | '-' | '_' | '.' | ' '));
    (!filler && !PLACEHOLDERS.contains(&key.as_str())).then_some(text)
}

/// Drops the legal ending from a maker's name: "Micro-Star International Co.,
/// Ltd." is "Micro-Star International".
fn short_vendor(vendor: &str) -> String {
    const ENDINGS: [&str; 10] = [
        ", inc.",
        " inc.",
        " inc",
        " corporation",
        " corp.",
        " corp",
        " co., ltd.",
        " co.,ltd.",
        " ltd.",
        " ltd",
    ];
    for ending in ENDINGS {
        // The endings are ASCII, so compare the last bytes as they are: a
        // name's other characters can change length when lowercased.
        let Some(cut) = vendor.len().checked_sub(ending.len()).filter(|&n| n > 0) else {
            continue;
        };
        if vendor.as_bytes()[cut..].eq_ignore_ascii_case(ending.as_bytes()) {
            // "Foo, Inc" and "Bar Co., Ltd" leave a comma behind.
            if let Some(head) = vendor.get(..cut) {
                return head.trim_end_matches([',', ' ']).to_owned();
            }
        }
    }
    vendor.to_owned()
}

/// The model: `product_name`, or the friendlier `product_version` when the
/// name is a code, or both when they say different things.
fn product_name(name: Option<String>, version: Option<String>) -> String {
    let letters = |s: &str| s.chars().filter(|c| c.is_alphabetic()).count();
    let version = version.filter(|v| {
        // "1.0", "v2", "Rev 1" and the like are revisions, not names.
        let lower = v.to_lowercase();
        letters(v) >= 2 && !lower.starts_with("rev") && !lower.starts_with("version")
    });
    match (name, version) {
        (Some(n), Some(v)) => {
            let (nl, vl) = (n.to_lowercase(), v.to_lowercase());
            if nl.contains(&vl) {
                n
            } else if vl.contains(&nl) || is_model_code(&n) {
                v
            } else {
                format!("{n} {v}")
            }
        }
        (Some(n), None) => n,
        (None, Some(v)) => v,
        (None, None) => String::new(),
    }
}

/// A machine-type code like "21AH00CNUS": one word of capitals and digits
/// with at least one digit.
fn is_model_code(s: &str) -> bool {
    s.len() >= 6
        && s.chars().any(|c| c.is_ascii_digit())
        && s.chars()
            .all(|c| c.is_ascii_uppercase() || c.is_ascii_digit())
}

fn board_name(vendor: Option<String>, name: Option<String>) -> String {
    let vendor = vendor.map(|v| short_vendor(&v));
    match (vendor, name) {
        // The name already starts with the maker's: "ASUSTeK Z690".
        (Some(v), Some(n))
            if v.split_whitespace()
                .next()
                .is_some_and(|w| n.to_lowercase().starts_with(&w.to_lowercase())) =>
        {
            n
        }
        (Some(v), Some(n)) => format!("{v} {n}"),
        (Some(v), None) => v,
        (None, Some(n)) => n,
        (None, None) => String::new(),
    }
}

/// "Vendor version (YYYY-MM-DD)", leaving out what is missing.
fn firmware_text(vendor: Option<String>, version: Option<String>, date: Option<&str>) -> String {
    let mut text = vendor.unwrap_or_default();
    for part in [
        version,
        date.map(|d| iso_date(d).map_or_else(|| d.to_owned(), |iso| format!("({iso})"))),
    ]
    .into_iter()
    .flatten()
    {
        if !text.is_empty() {
            text.push(' ');
        }
        text.push_str(&part);
    }
    text
}

/// SMBIOS writes the BIOS date as MM/DD/YYYY.
fn iso_date(date: &str) -> Option<String> {
    let mut parts = date.split('/');
    let (m, d, y) = (parts.next()?, parts.next()?, parts.next()?);
    let digits = |s: &str, n: usize| s.len() == n && s.bytes().all(|b| b.is_ascii_digit());
    let ok = parts.next().is_none()
        && digits(m, 2)
        && digits(d, 2)
        && digits(y, 4)
        && (1..=12).contains(&m.parse::<u8>().ok()?)
        && (1..=31).contains(&d.parse::<u8>().ok()?);
    ok.then(|| format!("{y}-{m}-{d}"))
}

/// The word for an SMBIOS chassis type; empty for "Other", "Unknown" and the
/// types that say nothing about the machine (expansion chassis, RAID).
pub fn chassis_name(code: u32) -> &'static str {
    match code {
        3 | 4 | 6 | 7 | 15 | 16 | 24 => "Desktop",
        5 | 17 | 23 | 28 | 29 => "Server",
        8..=10 | 14 => "Laptop",
        11 => "Handheld",
        12 => "Docking Station",
        13 => "All-in-One",
        30 => "Tablet",
        31 => "Convertible",
        32 => "Detachable",
        33 => "IoT Gateway",
        34 => "Embedded PC",
        35 => "Mini PC",
        36 => "Stick PC",
        _ => "",
    }
}

/// How secure the machine's firmware is, from fwupd's Host Security ID.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Security {
    /// False when fwupd isn't on the bus (neither running nor startable) or
    /// didn't answer in time.
    pub available: bool,
    /// The HSI level, 0..=5.
    pub level: Option<u8>,
    /// The id ends in "!": something changed after boot (a runtime issue).
    pub runtime_issue: bool,
    /// fwupd's version, from the id.
    pub version: String,
    /// What stops the next level: the attributes at or below it that did not
    /// pass, by name, in fwupd's order, once each.
    pub failing: Vec<String>,
}

/// Asks fwupd for the Host Security ID and its attributes. Blocks for up to
/// [`DEADLINE`], so call it from a worker thread, not the UI's.
pub fn security() -> Security {
    // The runtime lives for this call only: nothing outlives it.
    let Ok(rt) = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .max_blocking_threads(1)
        .thread_keep_alive(Duration::from_secs(1))
        .build()
    else {
        return Security::default();
    };
    // The connection is dropped inside the future, in the runtime, where
    // zbus's socket reader task can be cancelled.
    let reply = rt.block_on(async { tokio::time::timeout(DEADLINE, fetch()).await });
    // Not waiting for the thread zbus opens the socket on.
    rt.shutdown_background();
    match reply {
        Ok(Some((id, attrs))) => security_from(&id, &attrs),
        _ => Security::default(),
    }
}

/// fwupd's id and attribute list. `None` when fwupd can't be reached or has
/// no id to give.
async fn fetch() -> Option<(String, Vec<Attr>)> {
    let conn = connect().await?;
    let reply = conn
        .call_method(
            Some(DEST),
            "/",
            Some(PROPS_IF),
            "Get",
            &(FWUPD_IF, "HostSecurityId"),
        )
        .await
        .ok()?;
    let id: OwnedValue = reply.body().deserialize().ok()?;
    let Value::Str(id) = &*id else {
        return None;
    };
    let id = id.as_str().to_owned();
    // Without the list the page still has the level.
    let attrs = match conn
        .call_method(Some(DEST), "/", Some(FWUPD_IF), "GetHostSecurityAttrs", &())
        .await
    {
        Ok(m) => m.body().deserialize::<Vec<Attr>>().unwrap_or_default(),
        Err(_) => Vec::new(),
    };
    Some((id, attrs))
}

async fn connect() -> Option<Connection> {
    let conn = zbus::connection::Builder::system()
        .ok()?
        .method_timeout(TIMEOUT)
        .build()
        .await
        .ok()?;
    // Asked first rather than found out from a failed call: a call to a
    // name nobody owns or can start waits for the timeout.
    let running = conn
        .call_method(BUS, BUS_PATH, BUS, "NameHasOwner", &(DEST,))
        .await
        .ok()
        .and_then(|m| m.body().deserialize::<bool>().ok())
        == Some(true);
    if running {
        return Some(conn);
    }
    let activatable = conn
        .call_method(BUS, BUS_PATH, BUS, "ListActivatableNames", &())
        .await
        .ok()
        .and_then(|m| m.body().deserialize::<Vec<String>>().ok())
        .is_some_and(|names| names.iter().any(|n| n == DEST));
    activatable.then_some(conn)
}

/// One security attribute as fwupd sends it (an `a{sv}` row).
pub type Attr = HashMap<String, OwnedValue>;

/// The [`Security`] for fwupd's id and attribute list.
pub fn security_from(id: &str, attrs: &[Attr]) -> Security {
    let (level, runtime_issue, version) = parse_hsi(id);
    Security {
        available: true,
        level,
        runtime_issue,
        version,
        failing: level.map_or_else(Vec::new, |l| failing_attrs(l, attrs)),
    }
}

/// Splits a Host Security ID: "HSI:2! (v2.1.8)" is level 2, a runtime issue,
/// fwupd 2.1.8. A level above 5 or text that isn't an id gives no level.
pub fn parse_hsi(id: &str) -> (Option<u8>, bool, String) {
    let id = id.trim();
    let Some(rest) = id.strip_prefix("HSI:") else {
        return (None, false, String::new());
    };
    let digits = rest.bytes().take_while(u8::is_ascii_digit).count();
    let level = rest[..digits].parse::<u8>().ok().filter(|&l| l <= HSI_MAX);
    let Some(level) = level else {
        return (None, false, String::new());
    };
    let rest = &rest[digits..];
    let runtime_issue = rest.starts_with('!');
    let version = rest
        .split_once("(v")
        .and_then(|(_, v)| v.split_once(')'))
        .map(|(v, _)| v)
        .filter(|v| {
            v.len() <= 32
                && v.chars()
                    .all(|c| c.is_ascii_alphanumeric() || ".-+".contains(c))
        })
        .unwrap_or_default();
    (Some(level), runtime_issue, version.to_owned())
}

/// The titles of the attributes that keep the machine from level + 1: those
/// at or below it that did not pass and that nothing replaced. An attribute
/// with no `HsiLevel` is informational. A title is `Name`, else `Summary`,
/// else the `AppstreamId`.
fn failing_attrs(level: u8, attrs: &[Attr]) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for attr in attrs.iter().take(MAX_ATTRS) {
        let Some(attr_level) = number(attr, "HsiLevel") else {
            continue;
        };
        let flags = number(attr, "Flags").unwrap_or(0);
        if attr_level > u64::from(level) + 1 || flags & (FLAG_SUCCESS | FLAG_OBSOLETED) != 0 {
            continue;
        }
        let title = ["Name", "Summary", "AppstreamId"]
            .iter()
            .filter_map(|k| text(attr, k))
            .find(|t| !t.is_empty());
        if let Some(title) = title
            && !out.contains(&title)
        {
            out.push(title);
        }
    }
    out
}

fn text(attr: &Attr, key: &str) -> Option<String> {
    match &**attr.get(key)? {
        Value::Str(s) => Some(clean(s.as_str())),
        _ => None,
    }
}

/// An unsigned integer of any width; `None` if absent or negative.
fn number(attr: &Attr, key: &str) -> Option<u64> {
    match &**attr.get(key)? {
        Value::U8(n) => Some(u64::from(*n)),
        Value::U16(n) => Some(u64::from(*n)),
        Value::U32(n) => Some(u64::from(*n)),
        Value::U64(n) => Some(*n),
        Value::I32(n) => u64::try_from(*n).ok(),
        Value::I64(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

#[cfg(test)]
mod tests;
