//! Drive health (SMART) from udisks2 over the system bus: how much of the
//! drive's rated life is used, spare blocks, temperature, hours on, data
//! written, errors, and the drive's own verdict.
//!
//! Reading SMART from a block device needs root. udisks2 already runs with
//! that privilege, reads every drive's SMART log in its own housekeeping
//! (every 10 minutes, without waking a sleeping disk) and hands the cached
//! values to anyone on the bus: neither the properties nor
//! `SmartGetAttributes` asks polkit. A machine without udisks2 (neither
//! running nor activatable) gets no [`SmartReader`], and so no health
//! section. Since the values change every 10 minutes at most, the disk page
//! reads them when it opens and once a minute after.
//!
//! NVMe drives report everything directly: wear as `percent_used`, spare
//! blocks against their threshold, the critical-warning bits. ATA drives
//! give a pass/fail verdict, a temperature, hours on and bad sectors; an
//! ATA SSD's wear is the normalized value of `wear-leveling-count` (177),
//! which starts at 100 and counts down on the drives that use it (a best
//! effort: what the normalized value means is up to the vendor, so one that
//! counts from 200 or 253 gives no figure). libblockdev names an attribute only
//! when smartmontools' drive database agrees on what the ID means for that
//! model (attribute IDs are vendor-specific), so the name is trusted and the
//! ID is not.
//!
//! The parsers ([`nvme`], [`ata`]) work on the property maps udisks2 sends,
//! so the tests feed them replies recorded with `busctl --json`.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use zbus::Connection;
use zbus::zvariant::{OwnedObjectPath, OwnedValue, Value};

const DEST: &str = "org.freedesktop.UDisks2";
const BLOCK_PREFIX: &str = "/org/freedesktop/UDisks2/block_devices/";
const BLOCK_IF: &str = "org.freedesktop.UDisks2.Block";
const DRIVE_IF: &str = "org.freedesktop.UDisks2.Drive";
const NVME_IF: &str = "org.freedesktop.UDisks2.NVMe.Controller";
const ATA_IF: &str = "org.freedesktop.UDisks2.Drive.Ata";
const PROPS_IF: &str = "org.freedesktop.DBus.Properties";
const BUS: Option<&str> = Some("org.freedesktop.DBus");
const BUS_PATH: &str = "/org/freedesktop/DBus";
const KELVIN_ZERO: f64 = 273.15;
/// Longest a call may take before the reading is given up. udisks2 answers
/// from its cache; this is for a daemon that hangs, so the sampling thread
/// doesn't wait D-Bus's default 25 s.
const TIMEOUT: Duration = Duration::from_secs(3);
/// Longest a whole reading may take: a few calls, normally about 1 ms.
pub const DEADLINE: Duration = Duration::from_secs(4);
/// How long a reader whose connection broke or timed out first stays
/// quiet; it doubles with each failure in a row, up to [`RETRY_MAX`].
pub const RETRY: Duration = Duration::from_secs(30);
pub const RETRY_MAX: Duration = Duration::from_secs(300);

/// A property map as udisks2 sends it.
type Props = HashMap<String, OwnedValue>;

/// How the drive is attached, which decides what it can report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Nvme,
    Ata,
}

/// An NVMe drive's critical warnings (the SMART log's bits, as udisks2
/// names them).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Warning {
    /// Spare blocks are below the drive's threshold (`spare`).
    SpareLow,
    /// Over its warning temperature, or under its lowest (`temperature`).
    Temperature,
    /// Reliability is degraded by media errors (`degraded`).
    Degraded,
    /// The drive has made itself read-only (`readonly`).
    ReadOnly,
    /// The power-loss backup for its volatile memory failed (`volatile_mem`).
    BackupFailed,
    /// Its persistent memory region is read-only (`pmr_readonly`).
    PersistentMemoryReadOnly,
    /// A bit this version doesn't know.
    Other(String),
}

impl Warning {
    fn parse(s: &str) -> Self {
        match s {
            "spare" => Self::SpareLow,
            "temperature" => Self::Temperature,
            "degraded" => Self::Degraded,
            "readonly" => Self::ReadOnly,
            "volatile_mem" => Self::BackupFailed,
            "pmr_readonly" => Self::PersistentMemoryReadOnly,
            other => Self::Other(other.to_owned()),
        }
    }

    /// Whether the warning says the drive is failing. Low spare and heat are
    /// warnings of their own: the first is wear, the second passes.
    pub fn is_failure(&self) -> bool {
        !matches!(self, Self::SpareLow | Self::Temperature)
    }
}

/// What a drive says about itself. A figure the drive doesn't report is
/// `None`: "0 °C" and "no reading" are different things on a page.
#[derive(Debug, Clone, PartialEq)]
pub struct Health {
    pub kind: Kind,
    /// The drive's own verdict: it expects to fail. ATA's `SmartFailing`
    /// (which is also set when smartctl couldn't get the status), or an
    /// NVMe warning that [`Warning::is_failure`].
    pub failing: bool,
    /// NVMe critical warnings, in the drive's order. Empty for ATA.
    pub warnings: Vec<Warning>,
    /// Percentage of the rated life used, 0..=100 (a drive past its rating
    /// keeps counting; that is 100 here).
    pub wear: Option<u8>,
    /// Percentage of spare blocks left, 0..=100.
    pub spare: Option<u8>,
    /// Spare blocks are below the drive's threshold: the drive is near the
    /// end of its life. Not in `failing`, but as serious; the page shows it
    /// as a warning of its own.
    pub spare_low: bool,
    /// °C.
    pub temperature: Option<f64>,
    /// The drive's warning temperature in °C (NVMe `wctemp`).
    pub temperature_limit: Option<f64>,
    pub power_on_hours: Option<u64>,
    pub power_cycles: Option<u64>,
    pub read_bytes: Option<u64>,
    pub written_bytes: Option<u64>,
    /// Power lost without the drive being told (NVMe).
    pub unsafe_shutdowns: Option<u64>,
    /// Unrecovered data errors (NVMe).
    pub media_errors: Option<u64>,
    /// Reallocated and pending sectors (ATA), when the drive reports
    /// either attribute.
    pub bad_sectors: Option<u64>,
    /// Attributes at or below their threshold now (ATA).
    pub failing_attributes: Option<u32>,
    /// When udisks2 last read the drive, in seconds since the epoch.
    pub updated: u64,
}

/// One row of ATA `SmartGetAttributes`, the parts used here.
#[derive(Debug, Clone, PartialEq)]
pub struct AtaAttribute {
    /// libblockdev's name ("wear-leveling-count"), or "attribute-<id>"
    /// when the drive database doesn't vouch for the ID.
    pub name: String,
    /// The normalized value, -1 when unknown.
    pub value: i32,
    /// The value in `pretty_unit`.
    pub pretty: i64,
    /// 0 unknown, 1 a count, 2 ms, 3 sectors, 4 mK.
    pub pretty_unit: i32,
}

/// The `a(ysqiiixia{sv})` row as it comes off the bus: id, name, flags,
/// value, worst, threshold, pretty, pretty_unit, expansion.
type RawAtaAttribute = (u8, String, u16, i32, i32, i32, i64, i32, Props);

/// A connection to udisks2. Construct and use it on the sampling thread
/// (not inside a tokio runtime): it runs its own single-threaded one, driven
/// only while a read is in progress. The only other thread is the one zbus
/// opens the socket on, which exits a second after connecting.
pub struct SmartReader {
    // An Option so Drop can let go of it inside the runtime, and so a
    // broken connection can be dropped and made again.
    conn: Option<Connection>,
    /// When a reader without a connection may try again.
    retry_at: Option<Instant>,
    /// The quiet period the next failure gets.
    retry: Duration,
    rt: tokio::runtime::Runtime,
}

impl SmartReader {
    /// Connects to the system bus. `None` when there is no system bus or
    /// udisks2 is neither running nor activatable.
    pub fn new() -> Option<Self> {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .max_blocking_threads(1)
            .thread_keep_alive(Duration::from_secs(1))
            .build()
            .ok()?;
        let conn = rt.block_on(async {
            tokio::time::timeout(DEADLINE, connect())
                .await
                .ok()
                .flatten()
        })?;
        Some(Self {
            conn: Some(conn),
            retry_at: None,
            retry: RETRY,
            rt,
        })
    }

    /// What the drive behind a kernel device name (`nvme0n1`, `sda`; a
    /// partition gives its drive) says about itself. `None` for a device
    /// with no drive behind it (a loop, zram or device-mapper device), a
    /// drive without SMART (most USB sticks), one udisks2 hasn't read yet,
    /// and when udisks2 doesn't answer.
    ///
    /// A read that times out or loses the bus drops the connection, and
    /// every read in the next [`RETRY`] is `None` at once; the one after
    /// connects again, and each failure in a row doubles the wait, up to
    /// [`RETRY_MAX`]. So a hung udisks2 costs the sampling thread
    /// [`DEADLINE`] every few minutes, not every call, and a restarted
    /// system bus (or a udisks2 slow to start) is picked up again.
    pub fn read(&mut self, device: &str) -> Option<Health> {
        if self.conn.is_none() {
            if self.retry_at.is_some_and(|t| Instant::now() < t) {
                return None;
            }
            self.conn = self.rt.block_on(async {
                tokio::time::timeout(DEADLINE, connect())
                    .await
                    .ok()
                    .flatten()
            });
            if self.conn.is_none() {
                self.back_off();
                return None;
            }
        }
        let conn = self.conn.as_ref()?;
        let reading = self
            .rt
            .block_on(async { tokio::time::timeout(DEADLINE, read_drive(conn, device)).await });
        match reading {
            Ok(Ok(health)) => {
                self.retry = RETRY;
                health
            }
            // Timed out, or the bus failed rather than udisks2 answering.
            _ => {
                let _guard = self.rt.enter();
                self.conn = None;
                self.back_off();
                None
            }
        }
    }

    fn back_off(&mut self) {
        self.retry_at = Some(Instant::now() + self.retry);
        self.retry = (self.retry * 2).min(RETRY_MAX);
    }
}

impl Drop for SmartReader {
    fn drop(&mut self) {
        // zbus's socket reader is a task on this runtime.
        let _guard = self.rt.enter();
        self.conn.take();
    }
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

/// The drive's health. `Ok(None)` when udisks2 answered that there is none
/// (an error reply: no such device, no such interface); `Err` when it didn't
/// answer, which the caller takes as a broken connection.
async fn read_drive(conn: &Connection, device: &str) -> zbus::Result<Option<Health>> {
    let Some(block) = block_path(device) else {
        return Ok(None);
    };
    let Some(drive) = get(conn, &block, BLOCK_IF, "Drive").await? else {
        return Ok(None);
    };
    let Ok(drive) = OwnedObjectPath::try_from(drive) else {
        return Ok(None);
    };
    let drive = drive.as_str();
    if drive == "/" {
        return Ok(None);
    }
    // An error reply to the NVMe interface is udisks2 saying the drive has
    // none, and only then is it asked for ATA.
    if let Some(props) = get_all(conn, drive, NVME_IF).await? {
        let attrs = smart_attributes::<Props>(conn, drive, NVME_IF)
            .await?
            .unwrap_or_default();
        return Ok(nvme(&props, &attrs));
    }
    let Some(props) = get_all(conn, drive, ATA_IF).await? else {
        return Ok(None);
    };
    if !ata_ready(&props) {
        return Ok(None);
    }
    let attrs: Vec<AtaAttribute> = smart_attributes::<Vec<RawAtaAttribute>>(conn, drive, ATA_IF)
        .await?
        .unwrap_or_default()
        .into_iter()
        .map(
            |(_, name, _, value, _, _, pretty, pretty_unit, _)| AtaAttribute {
                name,
                value,
                pretty,
                pretty_unit,
            },
        )
        .collect();
    // 0 is a solid-state drive; -1 (unknown) is not taken for one.
    let solid_state = get(conn, drive, DRIVE_IF, "RotationRate")
        .await?
        .and_then(|v| int(&v))
        == Some(0);
    Ok(ata(&props, &attrs, solid_state))
}

/// Calls a udisks2 method. An error reply is `Ok(None)`, and so is a reply
/// that isn't the expected type; a failure to get any reply is `Err`.
async fn call<T, B>(
    conn: &Connection,
    path: &str,
    iface: &str,
    method: &str,
    body: &B,
) -> zbus::Result<Option<T>>
where
    T: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type,
    B: serde::Serialize + zbus::zvariant::DynamicType,
{
    match conn
        .call_method(Some(DEST), path, Some(iface), method, body)
        .await
    {
        Ok(msg) => Ok(msg.body().deserialize::<T>().ok()),
        Err(zbus::Error::MethodError(..)) => Ok(None),
        Err(e) => Err(e),
    }
}

async fn get(
    conn: &Connection,
    path: &str,
    iface: &str,
    name: &str,
) -> zbus::Result<Option<OwnedValue>> {
    call(conn, path, PROPS_IF, "Get", &(iface, name)).await
}

async fn get_all(conn: &Connection, path: &str, iface: &str) -> zbus::Result<Option<Props>> {
    call(conn, path, PROPS_IF, "GetAll", &(iface,)).await
}

async fn smart_attributes<T>(conn: &Connection, path: &str, iface: &str) -> zbus::Result<Option<T>>
where
    T: for<'de> serde::Deserialize<'de> + zbus::zvariant::Type,
{
    let options: HashMap<&str, Value> = HashMap::new();
    call(conn, path, iface, "SmartGetAttributes", &(options,)).await
}

/// udisks2's object path for a block device: the basename of its `/dev`
/// node with every byte outside `[A-Za-z0-9_]` written `_xx` (`dm-0` is
/// `dm_2d0`). sysfs writes a `/` in the node's path as `!`
/// (`cciss!c0d0` is `/dev/cciss/c0d0`).
fn block_path(device: &str) -> Option<String> {
    let name = device.strip_prefix("/dev/").unwrap_or(device);
    let name = name.rsplit('!').next().unwrap_or(name);
    // Kernel names are ASCII; udisks2 would write a high byte sign-extended.
    if name.is_empty() || name.contains('/') || !name.is_ascii() {
        return None;
    }
    let mut path = String::from(BLOCK_PREFIX);
    for b in name.bytes() {
        if b.is_ascii_alphanumeric() || b == b'_' {
            path.push(char::from(b));
        } else {
            path.push_str(&format!("_{b:02x}"));
        }
    }
    Some(path)
}

/// An NVMe drive's health from its `NVMe.Controller` properties and
/// `SmartGetAttributes`. `None` until udisks2 has read the SMART log.
fn nvme(props: &Props, attrs: &Props) -> Option<Health> {
    let updated = uint(props.get("SmartUpdated")?)?;
    if updated == 0 {
        return None;
    }
    let warnings: Vec<Warning> = strings(props.get("SmartCriticalWarning"))
        .iter()
        .map(|s| Warning::parse(s))
        .collect();
    let field = |key: &str| attrs.get(key).and_then(uint);
    let spare = field("avail_spare").map(percent);
    let below_threshold = matches!(
        (spare, field("spare_thresh")),
        (Some(s), Some(t)) if u64::from(s) < t
    );
    Some(Health {
        kind: Kind::Nvme,
        failing: warnings.iter().any(Warning::is_failure),
        spare_low: below_threshold || warnings.contains(&Warning::SpareLow),
        warnings,
        wear: field("percent_used").map(percent),
        spare,
        temperature: celsius(
            props
                .get("SmartTemperature")
                .and_then(uint)
                .map(|k| k as f64),
        ),
        temperature_limit: celsius(field("wctemp").map(|k| k as f64)),
        power_on_hours: props.get("SmartPowerOnHours").and_then(uint),
        power_cycles: field("power_cycles"),
        read_bytes: field("total_data_read"),
        written_bytes: field("total_data_written"),
        unsafe_shutdowns: field("unsafe_shutdowns"),
        media_errors: field("media_errors"),
        bad_sectors: None,
        failing_attributes: None,
        updated,
    })
}

/// Whether an ATA drive has SMART on and udisks2 has read it.
fn ata_ready(props: &Props) -> bool {
    let flag = |key: &str| props.get(key).and_then(boolean) == Some(true);
    flag("SmartSupported")
        && flag("SmartEnabled")
        && props
            .get("SmartUpdated")
            .and_then(uint)
            .is_some_and(|t| t > 0)
}

/// An ATA drive's health from its `Drive.Ata` properties and
/// `SmartGetAttributes`. Wear is read only from a solid-state drive.
fn ata(props: &Props, attrs: &[AtaAttribute], solid_state: bool) -> Option<Health> {
    if !ata_ready(props) {
        return None;
    }
    let attr = |name: &str| attrs.iter().find(|a| a.name == name);
    let wear = attr("wear-leveling-count")
        .filter(|_| solid_state)
        .and_then(|a| u8::try_from(a.value).ok())
        .filter(|&v| v <= 100)
        .map(|v| 100 - v);
    // A count (pretty_unit 1) or nothing: the raw value of an attribute
    // libblockdev couldn't interpret is vendor-encoded.
    let power_cycles = attr("power-cycle-count")
        .filter(|a| a.pretty_unit == 1)
        .and_then(|a| u64::try_from(a.pretty).ok());
    // udisks2's SmartNumBadSectors is 0 for a drive without these
    // attributes, so they are summed here: none of them is "unknown".
    let sectors: Vec<u64> = ["reallocated-sector-count", "current-pending-sector"]
        .into_iter()
        .filter_map(attr)
        .filter_map(|a| u64::try_from(a.pretty).ok())
        .collect();
    let bad_sectors = (!sectors.is_empty()).then(|| sectors.iter().sum());
    let failing_attributes = props
        .get("SmartNumAttributesFailing")
        .and_then(int)
        .and_then(|n| u32::try_from(n).ok());
    // SmartFailing is the drive's SMART status, or that smartctl didn't
    // report one (a status command that failed), which udisks2 can't tell
    // apart. Taken as it is: a missed failure is worse than a false alarm,
    // and `failing_attributes` says whether an attribute agrees.
    let failing = props.get("SmartFailing").and_then(boolean) == Some(true);
    Some(Health {
        kind: Kind::Ata,
        failing,
        warnings: Vec::new(),
        wear,
        spare: None,
        spare_low: false,
        temperature: celsius(props.get("SmartTemperature").and_then(float)),
        temperature_limit: None,
        // 0 is udisks2's "unknown".
        power_on_hours: props
            .get("SmartPowerOnSeconds")
            .and_then(uint)
            .filter(|&s| s > 0)
            .map(|s| s / 3600),
        power_cycles,
        read_bytes: None,
        written_bytes: None,
        unsafe_shutdowns: None,
        media_errors: None,
        bad_sectors,
        failing_attributes,
        updated: props.get("SmartUpdated").and_then(uint)?,
    })
}

/// A percentage figure, held to 100.
fn percent(v: u64) -> u8 {
    v.min(100) as u8
}

/// Kelvin to °C; 0 K is udisks2's "unknown", and a reading no drive could
/// survive (an NVMe drive's 0xFFFF K) is a misread.
fn celsius(kelvin: Option<f64>) -> Option<f64> {
    kelvin
        .filter(|&k| k > 0.0)
        .map(|k| k - KELVIN_ZERO)
        .filter(|c| (-40.0..=150.0).contains(c))
}

/// The value inside any variants it is wrapped in.
fn inner<'a>(v: &'a Value<'a>) -> &'a Value<'a> {
    match v {
        Value::Value(v) => inner(v),
        v => v,
    }
}

/// A non-negative integer of whichever width the property uses: one SMART
/// log mixes bytes, `u16`s and `u64`s.
fn uint(v: &OwnedValue) -> Option<u64> {
    match inner(v) {
        Value::U8(n) => Some(u64::from(*n)),
        Value::U16(n) => Some(u64::from(*n)),
        Value::U32(n) => Some(u64::from(*n)),
        Value::U64(n) => Some(*n),
        Value::I16(n) => u64::try_from(*n).ok(),
        Value::I32(n) => u64::try_from(*n).ok(),
        Value::I64(n) => u64::try_from(*n).ok(),
        _ => None,
    }
}

/// A signed integer of any width (udisks2 gives -1 for unknown).
fn int(v: &OwnedValue) -> Option<i64> {
    match inner(v) {
        Value::I16(n) => Some(i64::from(*n)),
        Value::I32(n) => Some(i64::from(*n)),
        Value::I64(n) => Some(*n),
        _ => uint(v).and_then(|n| i64::try_from(n).ok()),
    }
}

fn float(v: &OwnedValue) -> Option<f64> {
    match inner(v) {
        Value::F64(f) => Some(*f),
        _ => None,
    }
}

fn boolean(v: &OwnedValue) -> Option<bool> {
    match inner(v) {
        Value::Bool(b) => Some(*b),
        _ => None,
    }
}

fn strings(v: Option<&OwnedValue>) -> Vec<String> {
    match v.map(|v| inner(v)) {
        Some(Value::Array(a)) => a
            .iter()
            .filter_map(|v| match inner(v) {
                Value::Str(s) => Some(s.as_str().to_owned()),
                _ => None,
            })
            .collect(),
        _ => Vec::new(),
    }
}

#[cfg(test)]
mod tests;
