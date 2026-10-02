//! Atlas Monitor, Rust side. `cpp/main.cpp` only starts Qt and loads the QML;
//! every QObject QML talks to is defined here. The system readers live in the
//! `atlas-sysinfo` crate, which knows nothing about Qt.

mod backend;
mod crash;
mod logging;
mod rc;
mod settings;

use std::ffi::c_void;

// cxx-qt-build's generated initializer calls into cxx-qt-lib; keep the crate
// linked even while no bridge uses one of its types.
extern crate cxx_qt_lib;

/// Called once from `main.cpp`. Returns the `Backend` QObject (no parent;
/// the caller owns it).
#[unsafe(no_mangle)]
pub extern "C" fn atlas_backend_new() -> *mut c_void {
    backend::qobject::backend_make_unique().into_raw().cast()
}
