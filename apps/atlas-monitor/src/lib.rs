//! Atlas Monitor, Rust side. `cpp/main.cpp` only starts Qt and loads the QML;
//! every QObject QML talks to is defined here. The system readers live in the
//! `atlas-sysinfo` crate, which knows nothing about Qt.

mod backend;
mod crash;
mod devices;
mod logging;
mod rc;
mod sampler;
mod sampling;
mod series;
mod settings;
mod stats;

use std::ffi::c_void;

use cxx_qt::Threading;

// cxx-qt-build's generated initializer calls into cxx-qt-lib; keep the crate
// linked even while no bridge uses one of its types.
extern crate cxx_qt_lib;

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
}

/// Called once from `main.cpp`. Makes every QObject and starts the sampling
/// thread, which posts to the stats objects. Delete `sampler` first: that
/// stops the thread.
#[unsafe(no_mangle)]
pub extern "C" fn atlas_objects_new() -> AtlasObjects {
    let backend = backend::qobject::backend_make_unique();
    let mut sampler = sampler::qobject::sampler_make_unique();
    let mut cpu = stats::qobject::cpu_stats_make_unique();
    let mut memory = stats::qobject::memory_stats_make_unique();
    let mut health = stats::qobject::health_status_make_unique();
    let mut devices = devices::qobject::device_list_make_unique();
    let mut disk = devices::qobject::disk_stats_make_unique();
    let mut net = devices::qobject::net_stats_make_unique();

    let sink = sampler::Sink {
        cpu: cpu.pin_mut().qt_thread(),
        memory: memory.pin_mut().qt_thread(),
        health: health.pin_mut().qt_thread(),
        devices: devices.pin_mut().qt_thread(),
        disk: disk.pin_mut().qt_thread(),
        net: net.pin_mut().qt_thread(),
    };
    sampler.pin_mut().start(*backend.refresh_interval(), sink);

    AtlasObjects {
        backend: backend.into_raw().cast(),
        sampler: sampler.into_raw().cast(),
        cpu: cpu.into_raw().cast(),
        memory: memory.into_raw().cast(),
        health: health.into_raw().cast(),
        devices: devices.into_raw().cast(),
        disk: disk.into_raw().cast(),
        net: net.into_raw().cast(),
    }
}
