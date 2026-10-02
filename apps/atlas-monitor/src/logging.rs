//! Logging for the Rust side: the `log` macros, written to stderr, which ends
//! up in the user's journal when Plasma starts the app. Qt's own messages go
//! the same way from `main.cpp`.
//!
//! The level comes from `ATLAS_MONITOR_LOG` (`error`, `warn`, `info`, `debug`,
//! `trace` or `off`); the default is `warn`, so a normal run logs nothing
//! unless something is wrong.

use std::io::Write;

use log::{LevelFilter, Log, Metadata, Record};

struct StderrLogger;

impl Log for StderrLogger {
    fn enabled(&self, metadata: &Metadata) -> bool {
        metadata.level() <= log::max_level()
    }

    fn log(&self, record: &Record) {
        if !self.enabled(record.metadata()) {
            return;
        }
        // One write per line, so lines from different threads never interleave.
        let line = format!(
            "atlas-monitor {} {}: {}\n",
            record.level().as_str().to_ascii_lowercase(),
            record.target(),
            record.args()
        );
        let _ = std::io::stderr().lock().write_all(line.as_bytes());
    }

    fn flush(&self) {
        let _ = std::io::stderr().flush();
    }
}

static LOGGER: StderrLogger = StderrLogger;

pub fn level_from(value: Option<&str>) -> LevelFilter {
    value
        .and_then(|v| v.trim().parse::<LevelFilter>().ok())
        .unwrap_or(LevelFilter::Warn)
}

/// Called first thing from `main.cpp`.
#[unsafe(no_mangle)]
pub extern "C" fn atlas_log_init() {
    if log::set_logger(&LOGGER).is_ok() {
        let env = std::env::var("ATLAS_MONITOR_LOG").ok();
        log::set_max_level(level_from(env.as_deref()));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn level_parsing() {
        assert_eq!(level_from(None), LevelFilter::Warn);
        assert_eq!(level_from(Some("debug")), LevelFilter::Debug);
        assert_eq!(level_from(Some(" INFO ")), LevelFilter::Info);
        assert_eq!(level_from(Some("off")), LevelFilter::Off);
        assert_eq!(level_from(Some("loud")), LevelFilter::Warn);
    }
}
