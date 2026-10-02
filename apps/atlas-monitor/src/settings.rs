//! The user's settings, in `~/.config/atlas-monitorrc`:
//!
//! ```ini
//! [General]
//! RefreshInterval=1000
//! GpuRendering=false
//! ```
//!
//! Missing or unparseable values fall back to the defaults; an interval that
//! isn't one Settings offers snaps to the nearest one.

use std::io;

use crate::rc;

const GROUP: &str = "General";
const KEY_INTERVAL: &str = "RefreshInterval";
const KEY_GPU: &str = "GpuRendering";

/// The refresh intervals Settings offers, in milliseconds.
pub const INTERVALS_MS: [i32; 4] = [500, 1000, 2000, 5000];
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
    fn intervals_are_clamped_to_the_offered_ones() {
        assert_eq!(nearest_interval(1), 500);
        assert_eq!(nearest_interval(-5), 500);
        assert_eq!(nearest_interval(1400), 1000);
        assert_eq!(nearest_interval(1500), 1000);
        assert_eq!(nearest_interval(1600), 2000);
        assert_eq!(nearest_interval(i32::MAX), 5000);
        assert_eq!(nearest_interval(i32::MIN), 500);
        for i in INTERVALS_MS {
            assert_eq!(nearest_interval(i), i);
        }
    }
}
