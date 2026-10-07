//! Atlas Monitor, Rust side. `cpp/main.cpp` only starts Qt and loads the QML;
//! every QObject QML talks to is defined here. The system readers live in the
//! `atlas-sysinfo` crate, which knows nothing about Qt.

mod backend;
mod battery;
mod details;
mod devices;
mod energy;
mod graphics;
mod hardware;
mod processes;
mod rows;
mod sampler;
mod sampling;
mod sensors;
mod series;
mod services;
mod settings;
mod startup;
mod stats;
mod sysinfo;

use std::ffi::{CStr, CString, c_char, c_void};
use std::sync::OnceLock;

use cxx_qt::{CxxQtType, Threading};

// cxx-qt-build's generated initializer calls into cxx-qt-lib; keep the crate
// linked even while no bridge uses one of its types.
extern crate cxx_qt_lib;

// Who this app is, for atlas-framework: `main.cpp`'s `telamon_app_init` and
// `telamon_app_ready` take the names, the logger and the crash hooks from it.
// The ID is the desktop file, the icon and the single-instance D-Bus name.
telamon_framework_ui::app! {
    name: "Atlas Monitor",
    id: "net.eterneon.atlas.monitor",
    repo: "atlasos-monitor",
    // TelamonCard, TelamonStat, TelamonDetailGrid, TelamonSparkline, TelamonDialog and
    // TelamonSidebar; the spec's Requires says the same.
    ui: "2.0.0",
}

/// The QObjects QML sees, handed to the engine as `Main.qml`'s initial
/// properties. The caller owns them all; see `atlas_objects_new`.
#[repr(C)]
pub struct AtlasObjects {
    pub backend: *mut c_void,
    pub sampler: *mut c_void,
    pub cpu: *mut c_void,
    pub memory: *mut c_void,
    pub health: *mut c_void,
    pub devices: *mut c_void,
    pub disk: *mut c_void,
    pub net: *mut c_void,
    pub gpu: *mut c_void,
    pub battery: *mut c_void,
    pub sensors: *mut c_void,
    pub apps: *mut c_void,
    pub startup: *mut c_void,
    pub services: *mut c_void,
    pub details: *mut c_void,
    pub energy: *mut c_void,
    pub system: *mut c_void,
    pub hardware: *mut c_void,
}

/// Called once from `main.cpp`. Makes every QObject and starts the sampling
/// thread, which posts to the stats objects. Delete `sampler` first: that
/// stops the thread. `icon_theme` is Qt's (`QIcon::themeName()`), UTF-8;
/// the Apps table looks its icons up there.
///
/// # Safety
///
/// `icon_theme` is null or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atlas_objects_new(icon_theme: *const c_char) -> AtlasObjects {
    // SAFETY: the caller passes null or a NUL-terminated string.
    let icon_theme = unsafe { text(icon_theme) };
    let backend = backend::qobject::backend_make_unique();
    let mut sampler = sampler::qobject::sampler_make_unique();
    let mut cpu = stats::qobject::cpu_stats_make_unique();
    let mut memory = stats::qobject::memory_stats_make_unique();
    let mut health = stats::qobject::health_status_make_unique();
    let mut devices = devices::qobject::device_list_make_unique();
    let mut disk = devices::qobject::disk_stats_make_unique();
    let mut net = devices::qobject::net_stats_make_unique();
    let mut gpu = graphics::qobject::gpu_stats_make_unique();
    let mut battery = battery::qobject::battery_stats_make_unique();
    let mut sensors = sensors::qobject::sensor_list_make_unique();
    let mut apps = processes::qobject::process_model_make_unique();
    let startup = startup::qobject::startup_list_make_unique();
    let mut services = services::qobject::service_model_make_unique();
    let mut details = details::qobject::process_details_make_unique();
    let mut energy = energy::qobject::energy_saver_make_unique();
    // Read when their pages first open.
    let system = sysinfo::qobject::system_info_make_unique();
    let hardware = hardware::qobject::hardware_list_make_unique();
    sensors
        .pin_mut()
        .set_available(atlas_sysinfo::sensors::available());

    services.pin_mut().rust_mut().sampler = Some(Box::new(sampler.pin_mut().qt_thread()));
    apps.pin_mut().rust_mut().details = Some(Box::new(details.pin_mut().qt_thread()));
    apps.pin_mut().rust_mut().sampler = Some(Box::new(sampler.pin_mut().qt_thread()));

    let sink = sampler::Sink {
        cpu: cpu.pin_mut().qt_thread(),
        memory: memory.pin_mut().qt_thread(),
        health: health.pin_mut().qt_thread(),
        devices: devices.pin_mut().qt_thread(),
        disk: disk.pin_mut().qt_thread(),
        net: net.pin_mut().qt_thread(),
        gpu: gpu.pin_mut().qt_thread(),
        battery: battery.pin_mut().qt_thread(),
        sensors: sensors.pin_mut().qt_thread(),
        apps: apps.pin_mut().qt_thread(),
        services: services.pin_mut().qt_thread(),
    };
    energy.pin_mut().start(icon_theme.clone());
    sampler
        .pin_mut()
        .start(*backend.refresh_interval(), icon_theme, sink);
    // The saved choice; a first tick already under way lists without it.
    sampler.show_kernel_threads(*apps.kernel_threads());

    AtlasObjects {
        backend: backend.into_raw().cast(),
        sampler: sampler.into_raw().cast(),
        cpu: cpu.into_raw().cast(),
        memory: memory.into_raw().cast(),
        health: health.into_raw().cast(),
        devices: devices.into_raw().cast(),
        disk: disk.into_raw().cast(),
        net: net.into_raw().cast(),
        gpu: gpu.into_raw().cast(),
        battery: battery.into_raw().cast(),
        sensors: sensors.into_raw().cast(),
        apps: apps.into_raw().cast(),
        startup: startup.into_raw().cast(),
        services: services.into_raw().cast(),
        details: details.into_raw().cast(),
        energy: energy.into_raw().cast(),
        system: system.into_raw().cast(),
        hardware: hardware.into_raw().cast(),
    }
}

/// A C string as Rust's, "" for null.
///
/// # Safety
///
/// `s` is null or a NUL-terminated string.
unsafe fn text(s: *const c_char) -> String {
    if s.is_null() {
        return String::new();
    }
    // SAFETY: as the caller promises.
    unsafe { CStr::from_ptr(s) }.to_string_lossy().into_owned()
}

/// The `icons/` folders the Apps table finds icons in, one per line, for
/// `QIcon::setThemeSearchPaths`: Qt's own can miss Flatpak's exports. The
/// string lives as long as the program.
///
/// # Safety
///
/// `icon_theme` is null or a NUL-terminated string.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn atlas_icon_search_paths(icon_theme: *const c_char) -> *const c_char {
    static PATHS: OnceLock<CString> = OnceLock::new();
    // SAFETY: the caller passes null or a NUL-terminated string.
    let theme = unsafe { text(icon_theme) };
    PATHS
        .get_or_init(|| {
            // The lookup alone: it reads nothing until asked for an icon,
            // so the window isn't kept waiting on a scan.
            let dirs = atlas_sysinfo::apps::desktop::data_dirs();
            let paths: Vec<String> = atlas_sysinfo::apps::IconLookup::new(&theme, &dirs)
                .search_paths()
                .map(|p| p.to_string_lossy().into_owned())
                .collect();
            CString::new(paths.join("\n")).unwrap_or_default()
        })
        .as_ptr()
}
