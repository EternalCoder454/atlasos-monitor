//! Opt-in crash reports through atlas-core. Off unless the user turned them on
//! in Atlas Updater, which is also where reports are reviewed and sent; this
//! app only saves them.

use std::ffi::{CStr, c_char};

use atlas_core::crash::{self, AppInfo};

pub const APP_ID: &str = "net.eterneon.atlas.monitor";
pub const REPO: &str = "atlasos-monitor";

pub fn app_info() -> AppInfo {
    AppInfo {
        name: "Atlas Monitor".into(),
        id: APP_ID.into(),
        version: env!("CARGO_PKG_VERSION").into(),
        repo: REPO.into(),
    }
}

/// Called first thing from `main.cpp`: panics save a report, but only when
/// the user enabled crash reports (atlas-core checks the setting).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_crash_install() {
    crash::install(app_info());
}

/// Called from the C++ Qt message handler on `QtFatalMsg`, before abort.
///
/// # Safety
/// `msg` must be null or a valid NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atlas_crash_fatal(msg: *const c_char) {
    let text = if msg.is_null() {
        String::from("Qt fatal message")
    } else {
        unsafe { CStr::from_ptr(msg) }
            .to_string_lossy()
            .into_owned()
    };
    let _ = crash::record_fatal(&text);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn app_info_names_this_app() {
        let a = app_info();
        assert_eq!(a.id, "net.eterneon.atlas.monitor");
        assert_eq!(a.repo, "atlasos-monitor");
        assert!(!a.version.is_empty());
    }
}
