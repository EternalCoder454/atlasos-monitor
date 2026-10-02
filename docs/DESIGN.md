# Atlas Monitor: design

Atlas Monitor (`net.eterneon.atlas.monitor`) is the system monitor of AtlasOS,
a Fedora Kinoite 44 bootc image (`ghcr.io/eternalcoder454/atlasos`). It is a
required system app, like Atlas Updater, and replaces the Go/GTK4 Atlas
Monitor (github.com/EternalCoder454/atlas-monitor, now frozen) and Plasma
System Monitor, which the image removes.

Stack: Rust + Qt 6.11 + Kirigami 6.30 through CXX-Qt 0.10, built with CMake
and Corrosion, QML compiled ahead of time by `qt_add_qml_module`. Same stack,
build and look as [Atlas Updater](https://github.com/EternalCoder454/atlasos-updater);
its `docs/DESIGN.md` is the reference for anything this file doesn't cover.
Supports Fedora 44 Kinoite only (Qt 6.11.2, KF6 6.30, Rust 1.98).

## Layout

```
Cargo.toml                    workspace; pins atlas-core (see "Shared code")
crates/atlas-sysinfo/         the readers: /proc, /sys, hwmon, D-Bus. No Qt.
  src/<reader>.rs             one module per Go package (stats, process, gpu, ...)
  tests/fixtures/             recorded /proc and /sys files for the parsers
apps/atlas-monitor/           the app
  CMakeLists.txt              Corrosion + qt_add_qml_module; fetches Atlas.Ui
  build.rs                    cxx-qt-build: one entry per #[cxx_qt::bridge] file
  src/                        QObjects and models (CXX-Qt), settings, crash, logging
  cpp/                        main.cpp, and C++ Qt Quick items (the chart)
  qml/                        pages
  data/                       .desktop, metainfo, icon
packaging/atlas-monitor.spec  the RPM; build-rpm.sh builds it in fedora:44
scripts/dev.sh                run a command in the fedora:44 build container
```

`atlas-sysinfo` holds everything that reads the system, so it can be tested
and benchmarked without Qt. `apps/atlas-monitor/src` turns its results into
QObjects and models and owns the threads. QML only displays and calls
invokables.

## Shared code: atlas-core and Atlas.Ui

Both come from the atlasos-updater repo at **one commit**:

- atlas-core: `[workspace.dependencies] atlas-core = { git, rev }` in the
  workspace `Cargo.toml`. Cargo.lock records the same commit.
- Atlas.Ui: `apps/atlas-monitor/CMakeLists.txt` reads that `rev` from
  `Cargo.toml` and fetches `ui/` of the same commit with FetchContent
  (`SOURCE_SUBDIR ui`). `-DFETCHCONTENT_SOURCE_DIR_ATLASOS_UPDATER=<checkout>`
  builds against a local checkout instead, for trying unpushed UI work.

New shared components (`LiveChart`, `UsageBar`, `SidebarGroup`, `DataTable`,
`SearchField`, `ContextMenu`) land in atlasos-updater `ui/` first, then the pin
moves. Code that only Atlas Monitor needs stays here.

Moving the pin: push the atlasos-updater commit, change `rev`, run
`cargo update -p atlas-core`, rebuild (CMake refetches Atlas.Ui), commit
`Cargo.toml` and `Cargo.lock` together. CI fails if Cargo.lock and the `rev`
disagree.

## Process and window

- One instance per session: `KDBusService::Unique` owns
  `net.eterneon.atlas.monitor` on the session bus. A second launch (the
  launcher, Ctrl+Shift+Esc) activates the first and exits; the first raises
  its window with the activation token the launch passed
  (`KWindowSystem::updateStartupId` + `activateWindow`).
- **Nothing runs unless the window is open.** No tray, no autostart, no
  daemon. Closing the window quits the app. Automatic easing (Energy Saver)
  works only while the window is open and is undone when it closes.
- `main.cpp` is glue only: logging and the crash hook first, the Qt Quick
  backend, the single-instance service, the QML engine. All logic is Rust.

### Rendering

Qt Quick's **software backend (CPU) is the default**. Settings has "Use the
graphics card to draw the window" (off), read by `main.cpp` before
`QApplication` and applied on the next start. `QT_QUICK_BACKEND` in the
environment overrides both. The Go app measured the GPU path loading Mesa and
libLLVM (+27 MiB) for a window of charts.

Consequences:
- Charts are a C++ `QQuickPaintedItem` (QPainter), which draws on both
  backends. Custom `QSGGeometryNode`s and Qt Graphs don't render on the
  software backend.
- On the software backend, Qt repaints only dirty regions: a chart updates
  only its own rectangle, and nothing animates while idle.

## Threading rule

- **The GUI thread never blocks** on a file, D-Bus or a process. Readers run on
  a worker thread and post their results back with
  `qt_thread().queue(move |obj| ...)`, which runs the closure on the Qt thread.
- **One sampling thread** does the periodic reads (see Sampling). Occasional
  one-off reads (Atlas Monitor's own memory, process details) may use a
  short-lived `std::thread`; a request while one is running is dropped, not
  queued.
- Actions that can wait on polkit (services) run on their own thread, so a
  password prompt never stalls the samples.
- Worker threads own no Qt objects. They send plain Rust values; the closure
  on the Qt thread converts them to `QString`/models.
- A queued closure whose object is gone (`queue` returns `Err`) is dropped;
  threads end when the app quits (stop flag, joined in `Drop`).

## QObject and model API

QML sees a small set of objects, created in Rust and handed to the engine as
initial properties of `Main.qml`. Only `Backend` exists so far (Foundation);
the rest is the plan the later phases build to:

| Object | What it holds |
|---|---|
| `Backend` | Settings (`refreshInterval`, `gpuRendering`), Atlas Monitor's own memory (`ownPss`, `ownRss`). Invokables `changeRefreshInterval(ms)`, `changeGpuRendering(on)`, `refreshOwnMemory()`. |
| `Sampler` | The sampling thread. QML sets `activePage` ("overview", "cpu", "memory", "disk:nvme0n1", "network:wlp4s0", "gpu", "battery:BAT0", "sensors", "apps", ...); only that page's readers run. |
| `CpuStats`, `MemoryStats`, `GpuStats`, ... | Plain properties for the current values (`usage`, `frequency`, ...), plus one `Series` per chart. Updated in one queued closure per tick. |
| `Series` | A fixed-size ring buffer of samples (Rust), read by the C++ `LiveChart`. The exact interface is settled by the chart decision in the roadmap (measured with 10+ live charts). |
| `DeviceModel` | Disks, network interfaces and batteries, for the expanding sidebar entries (`SidebarGroup`), with a live value per row. |
| `ProcessModel` | The Apps table: a Rust `QAbstractItemModel` with row diffs (`beginInsertRows`/`dataChanged`/`beginRemoveRows`), never `beginResetModel` on a refresh, so rows hold still under the pointer. Group by App, sorting and search are done in Rust. Invokables `endTask`, `kill`, `stop`, `resume`, `details`, `openFileLocation` take a row key (pid, or the group's unit). |
| `ServiceModel`, `StartupModel`, `SensorModel` | The Services, Startup and Sensors lists, same row-diff rule. |

Names in QML are camelCase (`cxx_name`); Rust stays snake_case. Errors reach
QML as a property (`errorText`) set from the closure, never as a panic or a
blocking dialog.

## Sampling

Planned (Backend port phase):

- One thread, ticking every `refreshInterval` (500, 1000 (default), 2000 or
  5000 ms). Only the page on screen is sampled; the Overview reads the compact
  set it shows. A page opened later starts with empty charts, not history
  from when it was hidden.
- Kernel files that are read every tick are opened once and re-read with one
  `pread` each (the Go version's biggest CPU win). `/proc/<pid>` files are
  opened with `openat` against a held `/proc` fd, one read per file.
- `statfs` (disk capacity) runs every 5th tick and never while holding a lock.
- Targets are the Go version's numbers on the same machine (Fedora 44, KDE
  Wayland, ~700 processes): CPU page just opened 87 MiB RSS; idle page
  2.4 ms/s CPU and 47 read syscalls/s; Apps page 14.8 ms/s and 862 reads/s.
  The Rust version must beat all of them; measure, don't guess.

## Privilege

Atlas Monitor runs as the user and adds **no new privilege**: no setuid, no
system service of its own, no polkit actions of its own. The
atlas-system-helper keeps exactly its five methods; Atlas Monitor never calls
it.

| Feature | How | Who decides |
|---|---|---|
| Read /proc, /sys, hwmon | Plain files | File permissions. Another user's `/proc/<pid>/io`, `fd/` and `environ` are unreadable; shown as unknown, never as zero. |
| End Task, Kill, Stop, Continue | `kill(2)` / `pidfd_send_signal` on the user's own processes | The kernel. Other users' processes get "Not allowed", with no escalation. |
| Services: start, stop, restart, enable, disable | systemd's `org.freedesktop.systemd1` over the system bus (zbus) | systemd's own polkit actions (`manage-units`, `manage-unit-files`); polkit asks. |
| Drive health (SMART) | udisks2 over the system bus (`Drive.Ata` properties) | udisks2. No section when udisks2 is missing. |
| Energy Saver | `CPUWeight` on the app's unit through the **user's** systemd manager | None needed: the user's own units. Reversible; restored on exit and after a crash. |
| Startup items | XDG autostart files in `~/.config/autostart` and the user's systemd units | The user's own files. Atlas Updater's tray entry is shown but can't be switched off. |

## Settings

`~/.config/atlas-monitorrc` (KConfig INI), read and written by
`src/settings.rs` (atomic write through a temp file and rename):

```ini
[General]
RefreshInterval=1000   ; snapped to 500, 1000, 2000 or 5000
GpuRendering=false     ; applies on the next start
```

Missing or unparseable values fall back to the defaults; an interval that
isn't offered snaps to the nearest one. A failed save is logged and the
setting keeps its old value on screen.

## Crash reports and logging

- `atlas_core::crash`: `crash::install` is the first call in `main()`, and the
  Qt message handler calls `record_fatal` on `QtFatalMsg`. Reports are saved
  only when the user turned crash reports on, which happens in Atlas Updater;
  Atlas Updater is also where they are reviewed and sent. Atlas Monitor
  collects nothing else. See "Privacy and crash reports" in Atlas Updater's
  DESIGN.md.
- Logging: the Rust `log` macros write one line per message to stderr, which
  ends up in the user's journal when Plasma starts the app. Level from `ATLAS_MONITOR_LOG`
  (`error`, `warn` (default), `info`, `debug`, `trace`, `off`). Qt's messages
  go to stderr through the same handler that records fatal ones.

## System app (AtlasOS side)

- The RPM (`packaging/atlas-monitor.spec`) is built into the image under the
  read-only `/usr` by the AtlasOS repo's `build_files/build.sh`, which fails
  without it (`rpm -q atlas-monitor`). `/etc/dnf/protected.d/` lists it, like
  Atlas Updater.
- Ctrl+Shift+Esc (Plasma's System Monitor shortcut) opens Atlas Monitor.
- Updates come with the image, through Atlas Updater. No in-app updater, no
  channels.

## Testing

- `cargo fmt --all --check`, `cargo clippy --workspace --all-targets -- -D
  warnings` and `cargo test --workspace` in fedora:44 (CI does the same).
- Parsers are tested on recorded files in `tests/fixtures`; live tests assert
  invariants only. CI has no GPU, battery, kernel threads or system bus.
- Go parity: both readers run against the same fixtures and must agree.
- Everything that needs a session, polkit, systemd or real hardware is tested
  in the AtlasOS test VM (`just vm` in the AtlasOS repo), never on the host.
