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
  only its own rectangle, and nothing animates while idle. At fractional
  scales (1.25x, 1.5x) Qt repaints the whole window instead. Forcing partial
  updates there (`QSG_SOFTWARE_RENDERER_FORCE_PARTIAL_UPDATES`) was measured
  and saved nothing in Atlas Monitor or KWin, so it stays off.
- `main.cpp` sets `QT_NO_GUI_THREADPOOL=1` on both backends. Otherwise Qt's
  raster engine hands every fill of 96 or more spans to a thread pool and
  waits, which costs more than the fill for chart-sized shapes.

### Charts

Settled by measurement (`bench/chart/README.md`, 2026-10-02). `LiveChart` is a
C++ `QQuickPaintedItem` in Atlas.Ui; `bench/chart/livechart.cpp` is its
reference (its `series` pointer property is a benchmark shortcut, not part of
the API). It repaints its whole rectangle once per tick, and otherwise only
when it is resized or one of its properties changes.

- **Data:** `values: list<real>`, oldest sample first, at most 60, plus
  `values2` for a second series (upload beside download, write beside read).
  The Rust `Series` holds the ring buffer and publishes the list in the tick's
  queued closure. A generic list keeps Atlas.Ui free of Atlas Monitor's types,
  and it costs the same as reading the ring buffer from C++. `values2` is not
  in the benchmark; a second series adds roughly another fill and line, so
  measure it when it lands.
- **Drawing:** the line is one small anti-aliased quad per segment (extended
  half a width at both ends), not a stroked polyline. Qt's anti-aliasing
  rasterizer is slow on long, thin, nearly flat shapes, and per-segment quads
  are pixel-equivalent to a round-joined 1.5 px stroke. The quads overlap at
  the joins, so the line colour must be opaque. The fill under the
  line is drawn without anti-aliasing, since the line covers its sloped edge.
  The grid and border are 1-device-pixel rectangles snapped to device pixels
  and filled, not drawn as lines: translucent lines take Qt's per-pixel path.
  Captions are `QStaticText`, re-laid out only when their text changes.
- **Cost** (virtual KWin, 1 Hz, process CPU, measured before the grid became
  rectangles, which took a further 20% off large charts): 12 charts take
  3.8 ms/s at 1x and 10.0 ms/s at 1.5x, against 11.6 and 28.6 for a plain
  painted item. Pages show one to four charts.

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
| `Series` | A 60-sample ring buffer (Rust). Publishes `values` (`QList<f64>`, oldest first) for `LiveChart` once per tick; see Charts. |
| `DeviceModel` | Disks, network interfaces and batteries, for the expanding sidebar entries (`SidebarGroup`), with a live value per row. |
| `ProcessModel` | The Apps table: a Rust `QAbstractItemModel` with row diffs (`beginInsertRows`/`dataChanged`/`beginRemoveRows`), never `beginResetModel` on a refresh, so rows hold still under the pointer. Group by App, sorting and search are done in Rust. Invokables `endTask`, `kill`, `stop`, `resume`, `details`, `openFileLocation` (`apps::location` for a group) take a row key (pid, or the group's `apps::GroupKey`: an application ID, or a process name for processes that are no application's). |
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

The readers the loop drives (`atlas-sysinfo`):

- `sysfs::HeldFile` holds a kernel file open and re-reads it with one `pread`
  at offset zero; an attribute missing on this machine is `None`.
- `stats::{cpu, memory, disk, net}`: static facts come from plain functions
  (`cpu::info`, `disk::disks`, `net::interfaces`), read once. Changing
  figures come from a `*Sampler` that owns its held files and the previous
  counters. A sampler takes its baseline when it is made, so its first
  `sample()` is a real reading; making one when a page opens is what gives
  that page empty charts and no stale history. One tick of all four
  samplers on the development machine (32 threads) is 38 `pread`s and no
  `open`.
- `disk::space` (statvfs) and `net::addresses` (one netlink dump) are
  separate calls, for the loop to run every 5th tick.
- `process::ProcessSampler` is the Apps table, made while the Apps page is
  open. Each `/proc/<pid>` file is opened with `openat` against a held
  `/proc` fd and read once into a reused buffer. The stat line is the one
  file read for every process every tick. Everything else is skipped when it
  says nothing changed:
  - Kernel threads are dropped at their stat line (`PF_KTHREAD`), then known
    by pid and inode from the listing alone.
  - A process whose CPU time, page faults and rss haven't moved keeps its
    memory and shows no disk traffic, without reading `statm` or `io`. It is
    re-read every 5th tick, staggered by pid, and a disk rate after carried
    ticks covers all of them.
  - Sockets are counted (the network estimate) every 3rd tick per process
    while there is traffic, staggered by pid. The count is skipped while the
    descriptor count (`stat` size of `fd/`, Linux 6.2+) is unchanged; that
    skips about nine in ten links.
  - A known GPU client has only its DRM fdinfo re-read.

  On the development machine (~220 user processes, ~530 kernel threads,
  1 Hz), a tick costs ~1,250 syscalls (~360 `read`), down from ~6,070 in
  the first straight port. A scan costs 4.0 ms of CPU (mean, caches as cold
  as at 1 Hz), of which 3.8 ms is the floor: the listing and stat lines. To
  measure: `cargo run --release -p atlas-sysinfo --example processes 30 --bench`
  (per-scan CPU from schedstat), or the same without `--bench` under
  `strace -c`.
- `process::{details, act}`: the Details panel's one-off read, and End Task,
  Kill, Stop, Continue through a pidfd, after checking the start time so a
  reused pid is never signalled.
- `apps` groups the table by application. `Resolver` maps a process's unit
  to its application: the ID from systemd's `app[-launcher]-<ID>…` unit
  names, the name from that ID's `.desktop` file (folders listed once,
  re-listed at most every 30 s on a miss), and the first icon that draws
  (the file's, the ID, the lower-case name), cached per unit and forgotten
  after 120 ticks unseen. Icons are checked without Qt, which the sampling
  thread can't call, the way Qt looks: the theme chain (`QIcon::themeName()`,
  what it inherits, hicolor), the folders each `index.theme` lists, and the
  dash fallback (`foo-bar` → `foo`). The themes are listed once (again on a
  miss 30 s later) and only name hashes are kept; a name found only as a
  link (most of Breeze) is confirmed with a `stat` when asked about. An
  icon found only in `pixmaps/` comes back as a path. The app must hand
  `Resolver::icon_search_paths()` to `QIcon::setThemeSearchPaths`, so a
  Flatpak icon found here also draws when the session's `XDG_DATA_DIRS`
  lacks Flatpak's exports, and call `Resolver::set_icon_theme` when Qt's
  theme changes. `Grouper` folds a tick into one row per application (per
  name for the rest), its figures the members' sums as a `Proc`, known if
  any member's is. `Search` matches name, application name or pid (a group
  by pid only when it is one process); `sort` is stable and the columns
  never break ties, so equal rows hold still. On the development machine
  (235 processes, 113 rows): the first grouping 10 ms (desktop files and
  icon themes listed), then 15 µs a tick
  (`cargo run --release -p atlas-sysinfo --example apps -- --bench`).
- `apps::flatpak`: a Flatpak groups like any app (its launches'
  `app-flatpak-<ID>-*.scope` units, its exported desktop file), and `App`
  says it is one. Its processes see the app at `/app` and the runtime at
  `/usr`, so `process::executable` reads the sandbox's `/.flatpak-info`
  (through `/proc/<pid>/root`) and returns the host path of the app's or
  runtime's deployed file. `apps::location` is Open File Location for a
  grouped row: a Flatpak's install folder (`<installation>/app/<ID>/<arch>/
  <branch>/active/files`, from a running member's sandbox, else the first
  installation that has it), otherwise the first member's program. Both
  read files on request: call them off the GUI thread. To see them live:
  `--example apps -- --locations`.
- `apps::container`: a podman container's processes (toolbox and distrobox
  are podman too) are one row under the container's name. The ID comes
  from the cgroup, read with the unit at no extra syscall: `libpod-<ID>.scope`,
  `libpod-conmon-<ID>.scope`, or a Quadlet's `libpod-payload-<ID>`, the
  outermost winning, so systemd inside a container stays its row. The name
  comes from podman's own lists in the user's storage (`containers.json`
  and `volatile-containers.json` in each `<driver>-containers/` under
  the user's `graphroot`, the system's `rootless_storage_path`, or
  `~/.local/share/containers/storage`), read again when an unknown
  container turns up and a list has changed, and checked every 30 ticks
  while containers are shown, for renames. A container they lack (a
  rootful one) is "Container <short ID>". `podman exec` and
  `toolbox enter` stay in the terminal or service that ran them, and so
  does `conmon` unless podman gave it a `libpod-conmon` scope. A rootless
  container's files are mounted only in podman's namespace, so a container
  row has no Open File Location, and
  `process::executable` keeps a path only when the host's file there is
  the running program (same device and inode), which also covers a
  Flatpak's mapped paths.
- `gpu`: `gpu::cards()` lists every card, read once: discrete before
  integrated, then NVIDIA, AMD, Intel and others, then more video memory
  first. An Intel GPU on the processor's root bus (`0000:00`) is
  integrated, and so is an AMD one whose VRAM has no maker
  (`mem_info_vram_vendor`), since a carve-out has no memory chips. An
  integrated AMD GPU listed by codename alone ("Raphael") is named "AMD
  Radeon Graphics".
  It is named "AMD Radeon RX 7900 XTX": the driver's `product_name`, else
  the PCI ID database's model. Where a chip is sold as several models, the
  board's subsystem entry picks one. The GPU page and the Overview's GPU row
  show the first card. `GpuSampler` reads the card the best way its driver
  allows:
  - amdgpu: `gpu_busy_percent`, VRAM and GTT in use, and the card's hwmon
    (edge, junction and memory temperatures by label, fan, power, clocks).
    About 11 `pread`s a tick on the development machine, and no `open`.
  - NVIDIA's driver: NVML, `dlopen`ed only for a card bound to `nvidia`.
    AtlasOS ships no NVIDIA driver, so this is for someone who layered it.
    Not checked on hardware yet.
  - Intel (i915, xe): load from the time out of RC6 (idle residency).
    Clocks and VRAM size come from the driver's files. A discrete card's power
    comes from its hwmon energy counter.
  - Other drivers: load from the clients' drm-usage-stats counters. `/proc`
    is listed every 5 ticks and only new processes are walked. A process
    with no GPU handle is walked again on its own tick once every 30, if
    its descriptor count changed. Known clients have only their fdinfo
    re-read. Deltas are taken per client, and the busiest engine is the
    load. On this machine that is ~74 syscalls a tick, after a first walk
    of ~7,900.

  These counters are a last resort because they can't see the compositor.
  `kwin_wayland` has `cap_sys_nice`, which makes it non-dumpable, so its
  descriptors are closed to the user. For the same reason the Apps table
  shows no GPU use for KWin. `gpu::fdinfo` is the one fdinfo parser, for
  this and for the Apps table. It reads time counters (`drm-engine-*`),
  cycle counters (`drm-cycles-*`, xe) and engine capacities.

  A runtime-suspended card (a laptop's sleeping dGPU) is checked through
  `power/runtime_status` and otherwise left alone: on many kernels, reading
  its sysfs or asking NVML would wake it. The reading says `asleep`.
  `cards()` reads only identity files and sizes the kernel keeps in memory.
  A card asleep when its sampler is made gets its files, NVML and
  baselines on its first awake tick, which shows nothing else. Reading an awake card every second
  can restart its autosuspend timer, so a dGPU that would have dozed off
  may stay up while the GPU page is open (not yet measured on a laptop).
  To see
  the readings live: `--example gpu -- --clients`. `--clients` adds the
  counter path beside the driver's figure. Here amdgpu says 15% at idle
  clocks and the counters say 4%, KWin left out.
- History is not kept here: the app's `Series` holds it.

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
