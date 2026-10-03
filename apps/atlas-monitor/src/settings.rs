//! The user's settings, in `~/.config/atlas-monitorrc`:
//!
//! ```ini
//! [General]
//! RefreshInterval=1000
//! GpuRendering=false
//!
//! [EnergySaver]
//! Automatic=false
//! Never=org.kde.kdenlive,com.obsproject.Studio
//!
//! [Apps]
//! GroupByApp=true
//! KernelThreads=false
//! HiddenColumns=diskRead,diskWrite
//!
//! [Window]
//! Width=1100
//! Height=800
//! Maximized=false
//! Page=cpu
//! FoldedSections=cpu.cores
//! ```
//!
//! Missing or unparseable values fall back to the defaults; an interval that
//! isn't one Settings offers snaps to the nearest one.

use std::io;

use crate::rc;

const GROUP: &str = "General";
const KEY_INTERVAL: &str = "RefreshInterval";
const KEY_GPU: &str = "GpuRendering";
const ENERGY: &str = "EnergySaver";
const KEY_AUTOMATIC: &str = "Automatic";
const KEY_NEVER: &str = "Never";
const APPS: &str = "Apps";
const KEY_GROUPED: &str = "GroupByApp";
const KEY_KERNEL: &str = "KernelThreads";
const KEY_HIDDEN: &str = "HiddenColumns";
const WINDOW: &str = "Window";
const KEY_WIDTH: &str = "Width";
const KEY_HEIGHT: &str = "Height";
const KEY_MAXIMIZED: &str = "Maximized";
const KEY_PAGE: &str = "Page";
const KEY_FOLDED: &str = "FoldedSections";

/// The Apps columns hidden until the user shows them, by role.
pub const DEFAULT_HIDDEN: [&str; 2] = ["diskRead", "diskWrite"];

/// The refresh intervals Settings offers, in milliseconds.
pub const INTERVALS_MS: [i32; 6] = [500, 1000, 2000, 3000, 5000, 10000];
pub const DEFAULT_INTERVAL_MS: i32 = 1000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Settings {
    /// How often the pages on screen are sampled.
    pub refresh_interval_ms: i32,
    /// Draw with the graphics card (Qt Quick's RHI backend) instead of the CPU
    /// (the software backend, the default). Read once, before Qt starts.
    pub gpu_rendering: bool,
}

impl Default for Settings {
    fn default() -> Self {
        Self {
            refresh_interval_ms: DEFAULT_INTERVAL_MS,
            gpu_rendering: false,
        }
    }
}

impl Settings {
    pub fn load() -> Self {
        Self::from_values(rc::get(GROUP, KEY_INTERVAL), rc::get(GROUP, KEY_GPU))
    }

    fn from_values(interval: Option<String>, gpu: Option<String>) -> Self {
        let d = Self::default();
        Self {
            refresh_interval_ms: interval
                .and_then(|v| v.parse::<i32>().ok())
                .map_or(d.refresh_interval_ms, nearest_interval),
            gpu_rendering: gpu.map_or(d.gpu_rendering, |v| parse_bool(&v)),
        }
    }

    /// Saves an interval (callers snap it with [`nearest_interval`] first).
    pub fn save_refresh_interval(ms: i32) -> io::Result<()> {
        rc::set(GROUP, KEY_INTERVAL, Some(&ms.to_string()))
    }

    pub fn save_gpu_rendering(on: bool) -> io::Result<()> {
        rc::set(GROUP, KEY_GPU, Some(if on { "true" } else { "false" }))
    }
}

/// Energy Saver's choices.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Energy {
    /// Ease busy applications off by itself. Off until the user turns it on.
    pub automatic: bool,
    /// Applications, by ID, never eased automatically.
    pub never: Vec<String>,
}

impl Energy {
    pub fn load() -> Self {
        Self::from_values(rc::get(ENERGY, KEY_AUTOMATIC), rc::get(ENERGY, KEY_NEVER))
    }

    fn from_values(automatic: Option<String>, never: Option<String>) -> Self {
        Self {
            automatic: automatic.is_some_and(|v| parse_bool(&v)),
            never: list(&never.unwrap_or_default()),
        }
    }

    pub fn save_automatic(on: bool) -> io::Result<()> {
        rc::set(
            ENERGY,
            KEY_AUTOMATIC,
            Some(if on { "true" } else { "false" }),
        )
    }

    /// Desktop IDs hold no commas, so a plain KConfig list needs no escapes.
    pub fn save_never(ids: &[String]) -> io::Result<()> {
        let value = ids.join(",");
        rc::set(
            ENERGY,
            KEY_NEVER,
            (!value.is_empty()).then_some(value.as_str()),
        )
    }
}

/// The Apps page's View menu.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppsView {
    /// One row per application, its processes under it.
    pub grouped: bool,
    pub kernel_threads: bool,
    /// Column roles not shown. Saved even when empty: a missing key is
    /// the default, which hides some.
    pub hidden: Vec<String>,
}

impl Default for AppsView {
    fn default() -> Self {
        Self {
            grouped: true,
            kernel_threads: false,
            hidden: DEFAULT_HIDDEN.map(str::to_owned).to_vec(),
        }
    }
}

impl AppsView {
    pub fn load() -> Self {
        Self::from_values(
            rc::get(APPS, KEY_GROUPED),
            rc::get(APPS, KEY_KERNEL),
            rc::get(APPS, KEY_HIDDEN),
        )
    }

    fn from_values(
        grouped: Option<String>,
        kernel: Option<String>,
        hidden: Option<String>,
    ) -> Self {
        let d = Self::default();
        Self {
            grouped: grouped.map_or(d.grouped, |v| parse_bool(&v)),
            kernel_threads: kernel.map_or(d.kernel_threads, |v| parse_bool(&v)),
            hidden: hidden.map_or(d.hidden, |v| list(&v)),
        }
    }

    pub fn save_grouped(on: bool) -> io::Result<()> {
        rc::set(APPS, KEY_GROUPED, Some(if on { "true" } else { "false" }))
    }

    pub fn save_kernel_threads(on: bool) -> io::Result<()> {
        rc::set(APPS, KEY_KERNEL, Some(if on { "true" } else { "false" }))
    }

    pub fn save_hidden(roles: &[String]) -> io::Result<()> {
        rc::set(APPS, KEY_HIDDEN, Some(&roles.join(",")))
    }
}

/// The window as it was left: its size (logical pixels; 0 until saved),
/// maximized or not, and the page it showed.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WindowState {
    pub width: i32,
    pub height: i32,
    pub maximized: bool,
    pub page: String,
}

impl WindowState {
    pub fn load() -> Self {
        Self::from_values(
            rc::get(WINDOW, KEY_WIDTH),
            rc::get(WINDOW, KEY_HEIGHT),
            rc::get(WINDOW, KEY_MAXIMIZED),
            rc::get(WINDOW, KEY_PAGE),
        )
    }

    fn from_values(
        width: Option<String>,
        height: Option<String>,
        maximized: Option<String>,
        page: Option<String>,
    ) -> Self {
        // A size from a hand-edited file is kept sane; QML still applies
        // the window's minimum.
        let size = |v: Option<String>| {
            v.and_then(|v| v.trim().parse::<i32>().ok())
                .filter(|n| (1..=32768).contains(n))
                .unwrap_or(0)
        };
        Self {
            width: size(width),
            height: size(height),
            maximized: maximized.is_some_and(|v| parse_bool(&v)),
            page: page.unwrap_or_default().trim().to_owned(),
        }
    }

    /// A maximized window keeps the size it had before, to come back to.
    pub fn save_size(width: i32, height: i32, maximized: bool) -> io::Result<()> {
        if !maximized {
            rc::set(WINDOW, KEY_WIDTH, Some(&width.to_string()))?;
            rc::set(WINDOW, KEY_HEIGHT, Some(&height.to_string()))?;
        }
        rc::set(
            WINDOW,
            KEY_MAXIMIZED,
            Some(if maximized { "true" } else { "false" }),
        )
    }

    pub fn save_page(page: &str) -> io::Result<()> {
        rc::set(WINDOW, KEY_PAGE, Some(page))
    }

    /// The sections folded shut, by their ids (`cpu.cores`).
    pub fn load_folded() -> Vec<String> {
        rc::get(WINDOW, KEY_FOLDED).map_or_else(Vec::new, |v| list(&v))
    }

    pub fn save_folded(ids: &[String]) -> io::Result<()> {
        rc::set(WINDOW, KEY_FOLDED, Some(&ids.join(",")))
    }
}

/// `folded` with `id` folded or not, as a sorted list without repeats.
pub fn with_folded(folded: &[String], id: &str, fold: bool) -> Vec<String> {
    let mut out: Vec<String> = folded.iter().filter(|f| *f != id).cloned().collect();
    if fold {
        out.push(id.to_owned());
    }
    out.sort();
    out
}

/// A comma-separated list, trimmed, sorted, without repeats or blanks.
fn list(v: &str) -> Vec<String> {
    let mut out: Vec<String> = v
        .split(',')
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .map(str::to_owned)
        .collect();
    out.sort();
    out.dedup();
    out
}

/// KConfig writes `true`/`false`; accept what a person might type by hand too.
fn parse_bool(v: &str) -> bool {
    matches!(
        v.trim().to_ascii_lowercase().as_str(),
        "true" | "1" | "yes" | "on"
    )
}

/// The offered interval closest to `ms` (ties go to the shorter one).
pub fn nearest_interval(ms: i32) -> i32 {
    INTERVALS_MS
        .into_iter()
        .min_by_key(|&i| (i64::from(i) - i64::from(ms)).abs())
        .unwrap_or(DEFAULT_INTERVAL_MS)
}

/// Called from `main.cpp` before `QApplication` exists, to pick the Qt Quick
/// backend.
#[unsafe(no_mangle)]
pub extern "C" fn atlas_settings_gpu_rendering() -> bool {
    Settings::load().gpu_rendering
}

#[cfg(test)]
mod tests {
    use super::*;

    fn s(v: &str) -> Option<String> {
        Some(v.to_string())
    }

    #[test]
    fn defaults_when_missing_or_broken() {
        assert_eq!(Settings::from_values(None, None), Settings::default());
        assert_eq!(
            Settings::from_values(s("fast"), s("maybe")),
            Settings::default()
        );
    }

    #[test]
    fn reads_values() {
        let got = Settings::from_values(s("2000"), s("true"));
        assert_eq!(got.refresh_interval_ms, 2000);
        assert!(got.gpu_rendering);
        assert!(Settings::from_values(None, s(" Yes ")).gpu_rendering);
        assert!(!Settings::from_values(None, s("false")).gpu_rendering);
    }

    #[test]
    fn energy_lists_read_back() {
        assert_eq!(Energy::from_values(None, None), Energy::default());
        let got = Energy::from_values(s("true"), s(" b.App, a.App,,b.App "));
        assert!(got.automatic);
        assert_eq!(got.never, ["a.App", "b.App"]);
    }

    #[test]
    fn apps_view_reads_back() {
        assert_eq!(AppsView::from_values(None, None, None), AppsView::default());
        assert_eq!(AppsView::default().hidden, ["diskRead", "diskWrite"]);
        let got = AppsView::from_values(s("false"), s("true"), s("pid, gpu"));
        assert!(!got.grouped);
        assert!(got.kernel_threads);
        assert_eq!(got.hidden, ["gpu", "pid"]);
        // Saved empty: every column shown, not the default.
        assert!(AppsView::from_values(None, None, s("")).hidden.is_empty());
        let text = crate::rc::set_in("", APPS, KEY_HIDDEN, Some(""));
        assert_eq!(
            crate::rc::get_in(&text, APPS, KEY_HIDDEN).as_deref(),
            Some("")
        );
    }

    #[test]
    fn the_window_reads_back() {
        assert_eq!(
            WindowState::from_values(None, None, None, None),
            WindowState::default()
        );
        let got = WindowState::from_values(s("1200"), s(" 900 "), s("true"), s("disk:sda"));
        assert_eq!((got.width, got.height, got.maximized), (1200, 900, true));
        assert_eq!(got.page, "disk:sda");
        let bad = WindowState::from_values(s("-4"), s("huge"), s("no"), None);
        assert_eq!((bad.width, bad.height, bad.maximized), (0, 0, false));
    }

    #[test]
    fn folding_adds_and_takes_out_once() {
        let folded = with_folded(&[], "cpu.cores", true);
        assert_eq!(folded, ["cpu.cores"]);
        let folded = with_folded(&folded, "cpu.cores", true);
        assert_eq!(folded, ["cpu.cores"]);
        let folded = with_folded(&folded, "a.b", true);
        assert_eq!(folded, ["a.b", "cpu.cores"]);
        assert_eq!(with_folded(&folded, "cpu.cores", false), ["a.b"]);
        assert_eq!(list(" cpu.cores ,,a.b"), ["a.b", "cpu.cores"]);
    }

    #[test]
    fn intervals_are_clamped_to_the_offered_ones() {
        assert_eq!(nearest_interval(1), 500);
        assert_eq!(nearest_interval(-5), 500);
        assert_eq!(nearest_interval(1400), 1000);
        assert_eq!(nearest_interval(1500), 1000);
        assert_eq!(nearest_interval(1600), 2000);
        assert_eq!(nearest_interval(4000), 3000);
        assert_eq!(nearest_interval(8000), 10000);
        assert_eq!(nearest_interval(i32::MAX), 10000);
        assert_eq!(nearest_interval(i32::MIN), 500);
        for i in INTERVALS_MS {
            assert_eq!(nearest_interval(i), i);
        }
    }
}
