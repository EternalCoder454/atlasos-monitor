use cxx_qt_build::CxxQtBuilder;

fn main() {
    // Generates the C++ for the QObject bridges and compiles it into the Rust
    // static library. Qt is found through $QMAKE (CMake sets it).
    CxxQtBuilder::new()
        .file("src/backend.rs")
        .file("src/devices.rs")
        .file("src/graphics.rs")
        .file("src/processes.rs")
        .file("src/startup.rs")
        .file("src/services.rs")
        .file("src/details.rs")
        .file("src/battery.rs")
        .file("src/sensors.rs")
        .file("src/sampler.rs")
        .file("src/stats.rs")
        .build();
}
