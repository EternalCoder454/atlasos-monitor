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

QML sees a small set of objects. `atlas_objects_new` (`src/lib.rs`) makes
them all in Rust, wires the sampler's sink to the stats objects' Qt-thread
handles and starts the sampling thread; `main.cpp` hands them to the engine
as initial properties of `Main.qml`, owns them, and deletes the `Sampler`
first, which stops the thread that posts to the others. Built so far:
`Backend`, `Sampler`, `CpuStats`, `MemoryStats`, `HealthStatus`; the rest is
the plan the later phases build to:

| Object | What it holds |
|---|---|
| `Backend` | Settings (`refreshInterval`, `gpuRendering`), Atlas Monitor's own memory (`ownPss`, `ownRss`). Invokables `changeRefreshInterval(ms)`, `changeGpuRendering(on)`, `refreshOwnMemory()`. |
| `Sampler` | The sampling thread. QML calls `showPage(name)` when the page changes ("overview", "cpu", "memory", "disk:nvme0n1", "network:wlp4s0", "gpu", "battery:BAT0", "sensors", ...; anything else reads only the sidebar) and `changeInterval(ms)` when the setting does. |
| `CpuStats`, `MemoryStats`, `GpuStats`, ... | Plain properties for the current values (`usage`, `frequency`, ...; NaN where the machine doesn't report one), plus a `list<real>` per chart (`usageHistory`, ...). Updated in one queued closure per tick. |
| `HealthStatus` | `health::check`'s alerts for the Overview: `level` (0 fine, 1 warning, 2 critical) and the parallel lists `titles`, `details`, `levels`. Signals only when the list changes. |
| `Series` | Not a QObject: a 60-sample ring buffer (`src/series.rs`) in a stats object's Rust struct, published as that object's `list<real>` property once per tick and cleared on a page's first tick; see Charts. |
| `DeviceModel` | Disks, network interfaces and batteries, for the expanding sidebar entries (`SidebarGroup`), with a live value per row. |
| `ProcessModel` | The Apps table (`src/processes.rs`): a Rust `QAbstractListModel` changed by row diffs (`beginInsertRows`/`beginMoveRows`/`beginRemoveRows`, one `dataChanged`), never `beginResetModel`, so rows hold still under the pointer (`setHeld`) and keep the view's scroll and selection. Group by App (a group's processes are its rows when open, `toggle`), sorting (`sortBy`) and search (`setSearch`) are done in Rust from the sampling thread's `AppsTick` (processes, each one's `apps::GroupKey` and application, the groups). `act(row, action)` signals a row's every process through `process::act`; `nameAt`/`countAt` feed the confirmations. Details and Open File Location come with the Details page. |
| `ServiceModel`, `StartupModel`, `SensorModel` | The Services, Startup and Sensors lists, same row-diff rule. |

Names in QML are camelCase (`cxx_name`); Rust stays snake_case. Errors reach
QML as a property (`errorText`) set from the closure, never as a panic or a
blocking dialog.

## Sampling

`src/sampling.rs` is the loop, without Qt; `src/sampler.rs` the QObject
around it.

- One thread (`sampler`), ticking every `refreshInterval` (500, 1000
  (default), 2000 or 5000 ms). It owns every reader and takes no lock:
  commands (page, interval) arrive over a channel whose wait is also the
  sleep between ticks, and closing the channel stops it. Nothing is read
  until QML names the first page.
- Only the page on screen is read. Its readers are made when it opens (which
  takes their baseline) and dropped when it closes, so a page opened later
  starts with empty charts, not history from when it was hidden. Its first
  reading comes 500 ms after it opens, or one interval if that is shorter,
  so a 5 s interval doesn't show an empty page. The first tick is marked
  `fresh`; the stats objects clear their series on it.
- The sidebar's live values (disk and network rates, battery charge) are
  read on every page: two held files and the power supplies.
- What each page reads: Overview the processor, memory, every graphics card
  and the health inputs; CPU the processor (and `cpu::info` once); Memory
  the memory; Disk its space and SMART; Network its addresses; GPU every
  card; Sensors the hwmon readings.
- `statfs` (disk capacity) and the address dump run on a page's first tick
  and every 5th after, never under a lock. SMART is asked once a minute per
  drive; failed services every 5th tick on the Overview. Those two D-Bus
  questions run on a second thread, one at a time and never two of the same
  at once, so a hung daemon delays neither a tick nor closing the window
  (that thread is not joined). A tick uses the latest answers; a read that
  fails keeps the last one.
- The sink splits a tick and queues each part to its object with
  `qt_thread().queue`; a part whose object is gone is dropped.
- Kernel files that are read every tick are opened once and re-read with one
  `pread` each (the Go version's biggest CPU win). `/proc/<pid>` files are
  opened with `openat` against a held `/proc` fd, one read per file.
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
- `sensors`: `Sensors` finds every hwmon device when made, holds each
  reading's file open and samples them all: one `pread` per reading, 42 a
  tick on the development machine. A device is named for what it is:
  the processor's model, the graphics card's name as the GPU page has it,
  "Memory Slot 2" from the SPD address, a drive's model, "Motherboard",
  "Wi-Fi Adapter". Readings get the chip's labels made readable ("junction"
  is "Hotspot", "Tccd1" is "Chiplet 1"). On a hybrid Intel processor,
  coretemp's "Core 32" is "E-core 1" (core types from `cpu_atom/cpus`).
  More than four per-core temperatures are `folded` behind the package.
  Temperatures carry the hardware's `_max` and `_crit` as `high` and
  `critical`, and `warmth()` grades them. Two devices with the same name get
  what tells them apart ("(nvme1)") or a number. A motherboard chip's
  unconnected headers (0 RPM, -128 °C) are left out and checked again every
  30 ticks. `/sys/class/hwmon` is listed again every 10 ticks, so a device
  that comes or goes appears or disappears. A graphics card or network adapter
  that is runtime-suspended is not read and shows `asleep`, the GPU rule.
  Other devices are read even when their bus sleeps: the SMBus controller
  the memory sensors sit behind suspends between transfers. To see it:
  `--example sensors`.
- `power`: `PowerSampler` finds the batteries and adapters in
  `/sys/class/power_supply` and holds their files open: a few `pread`s per
  supply a tick, and the directory listed again every 10 ticks for a pack
  pulled from its bay or a dock, or at once when a held pack stops reading
  (ACPI removes a pulled pack's supply). The cycle count and charge limit
  are read every 10 ticks: on a ThinkPad each read runs an ACPI method. A battery's charge in µAh is turned into
  watt-hours at its design voltage (`voltage_min_design`), so capacity and
  health don't move with the load, and the time estimate takes the rate
  at that voltage too (charge over current). The rate is read as a magnitude, since
  some drivers count discharge as negative. The percentage is the
  firmware's `capacity` (what Plasma's applet shows) before energy over
  full. Time left is the driver's `time_to_empty_now`/`time_to_full_now`
  where it has them, else the energy left at a rate averaged over about
  30 s on the boot clock, reset when the status changes or after a
  suspend. Charging, it counts to the charge limit
  (`charge_control_end_threshold`) when one is set, and then the driver's
  time to full (which counts to 100%) is not used. `Supplies::total`
  sums the packs; `packs` keeps each, since two packs drain one after the
  other. An idle pack with no rate adds 0 W to the sum; a charging or
  discharging one without a rate leaves it unknown. A peripheral's battery (`scope=Device`: a mouse, a controller) is
  left out, and so is an empty bay (`present=0`). Adapters are named "AC
  Adapter", "USB-C Port", "USB Charger" and "Wireless Charger", and are
  online at `online` 1 or 2 (a programmable PPS source). A USB-C port
  online gives the charger's highest offer (`voltage_max` ×
  `current_max`, up to 240 W). To see it:
  `--example power`.
- `smart`: `SmartReader` asks udisks2 over the system bus what a drive
  says about itself, by kernel name (a partition gives its drive). It
  exists only when udisks2 is running or activatable; otherwise there is
  no health section. udisks2 reads every drive's SMART log in its own
  10-minute housekeeping and serves the cached values without polkit, so
  the disk page reads them when it opens and once a minute after: a few
  calls, about 10 ms for every drive here. zbus runs on a current-thread
  tokio runtime the reader owns, driven only during a read. A read has a
  4 s deadline (3 s a call), and so does connecting; one that times out or
  loses the bus drops the connection, reads are `None` at once for 30 s,
  doubling with each failure in a row up to 5 minutes, then it connects
  again, so a hung udisks2 can't stall the sampling thread every minute and
  a restarted bus is picked up. An error reply (no drive, no such
  interface) is an answer, not a failure: only that moves on from NVMe to
  ATA. NVMe:
  wear (`percent_used`, held to 100), spare against its threshold, the
  critical warnings (low spare and heat are warnings, the rest are
  failure), temperature and its warning limit, hours, cycles, data read
  and written, unsafe shutdowns, media errors. ATA: the drive's
  `SmartFailing` verdict as udisks2 gives it (also set when smartctl
  couldn't get the status: a missed failure is worse than a false alarm,
  and the failing-attribute count shows whether an attribute agrees),
  temperature, hours, failing attributes, bad sectors summed from
  `reallocated-sector-count` and `current-pending-sector` (udisks2's own
  count is 0, not unknown, for a drive without them), and on a
  solid-state drive (`RotationRate` 0) wear from
  `wear-leveling-count`'s normalized value, a best effort since vendors
  scale it differently (over 100 gives none); libblockdev names an
  attribute only when smartmontools' drive database vouches for its ID
  on that model, so the name is matched, never the ID. udisks2's 0 K,
  0 s and -1 are "unknown", and so is a temperature outside -40..150 °C. To see it: `--example smart`.
- `services`: `ServiceReader` reads systemd over the system bus, on a
  current-thread runtime of its own with SMART's deadline and back-off
  (3 s a call, 4 s a read; 30 s quiet after a failure, doubling to 5
  minutes). `list` gives the loaded services and the installed ones that
  could be enabled or disabled, failed first, then by name without case;
  left out are names with no unit file that nothing runs under
  (`not-found`), templates, aliases, and unloaded units that can't be
  enabled. `failed` gives the failed ones' names for `health`; `details`
  one unit's state, unit file, preset, docs, main PID, tasks, memory, CPU
  time, last result and restarts. Reading costs PID 1, not us: listing
  unit files takes it about 240 ms (it walks every unit directory per
  file) and listing units by pattern about 20 ms whatever matches (it goes
  through all 576 loaded units here), while listing 231 services by name
  takes under 1 ms. So the reader subscribes and keeps state: unit files
  are listed once, again after `UnitFilesChanged` or `Reloading`, and
  every 10 minutes in case a signal was lost; every unit is listed once a
  minute and after a reload, and in between the loaded services by name,
  the names kept by `UnitNew` and `UnitRemoved` (only loaded names: asking
  by name loads a unit). Signals that arrive during a full list are
  applied again over its answer, since some are newer. A `Peer.Ping`
  first takes in the signals sent before it; systemd sends `UnitNew` and
  `UnitRemoved` a little after the change, so the list is eventually
  right, a read later at most. An error reply to a list call is a failed
  read, not an empty list. The signal queues are emptied while each call
  waits, since zbus stops reading the socket when one is full. A
  connection that breaks (the bus drops a client whose signals pile up,
  as an unread reader's do) is made again at once, once, before the
  back-off. After the first, a list costs PID 1 about
  0.3 ms and takes 2 ms. An instance's state (`getty@tty1`) is asked for
  once per file listing; an unloaded unit's description comes from its
  file. `act` does Start, Stop, Restart, Enable or Disable with the
  call flagged to allow interactive authorization, so systemd asks polkit
  and polkit's agent shows the password dialog (without the flag systemd
  refuses at once). It blocks for the dialog (up to 5 minutes) and then
  for a start, stop or restart's job (`JobRemoved`, up to 60 s, then
  "still running"), so it runs on a thread of its own, with its own
  connection. Names are checked first: `EnableUnitFiles` would link a
  path from anywhere. Enable doesn't daemon-reload after (a second
  password, and it only matters for dependencies before the next boot).
  A simple service that fails after starting reports Done; the list shows
  the failure. To see it: `--example services` (`--details`, `--bench`,
  `--watch`, and `--act` for the test VM).
- `autostart`: `list` gives what starts at login, sorted by name without
  case, and `set_enabled` switches one; no reader to keep, since the page
  reads it on open and after a switch (about 10 ms). XDG autostart entries
  (`~/.config/autostart` over each `$XDG_CONFIG_DIRS/autostart`, the first
  listed winning) are read the way `systemd-xdg-autostart-generator` reads
  them, since Plasma 6 starts in systemd mode: `Hidden`,
  `OnlyShowIn`/`NotShowIn`, `TryExec`, `X-KDE-autostart-condition` (the
  KConfig key), and `X-systemd-skip`, whose entry a unit of the desktop's
  own starts instead, so its switch is locked; `X-GNOME-Autostart-enabled`
  is ignored, as the generator does. Each entry's unit
  (`app-…@autostart.service`) comes from `SourcePath=` in the generator's
  output, its state from the user's manager. Off writes `Hidden=true` in
  the user's file, or in a copy of the system's there (written beside and
  renamed; a link there is replaced, never written through); on removes a
  copy that only switched it off, so the entry follows the package again.
  Units: the user's own (Enable/Disable), and the installed ones that
  start, enabled from `/etc` or wanted by a login target from `/usr/lib`
  or a generator (Quadlets), switched by Mask/Unmask in the user's own
  folder, since the user can't remove `/etc`'s links. Candidates come
  from the link folders and only they are asked about: the full unit file
  list costs the user's manager about 40 ms, a dozen names about 10.
  Locked: Atlas Updater's tray, and D-Bus, `systemd-*`, `plasma-*` and
  portal units (off only). `NoDisplay` entries, skipped entries and
  installed units the user didn't enable are `plumbing`. Tested against a
  real user manager in a systemd container: each switch, both locks, and
  the generator dropping a hidden entry's unit. To see it:
  `--example autostart` (`--bench`, and `--set <id> on|off` for the test
  VM).
- `ease`: Energy Saver. `ease::open(state_file())` gives a `Controller`
  (or `Unavailable`, the reason the page shows instead of a switch), owned
  by the sampling thread, which calls `tick(&mut resolver)` every
  `TICK_EVERY` (5 s) while the window is open, whatever page is showing,
  and drops it when the window closes. `rows()` is the page's list,
  `ease`/`restore` its per-app actions, `set_automatic`/`set_never` the
  settings. An eased app's units get `CPUWeight=10` as a runtime property
  through the user's manager (`app-*` units only). Automatic: 30 s above
  50% of a core eases it, 60 s below 15% puts it back. Never eased:
  anything playing or recording (pw-dump, checked before every ease; no
  answer means no easing), terminals, Atlas Monitor, apps listed as never,
  an app the user put back (while it runs), and any unit whose weight
  someone else set. uresourced, which AtlasOS runs, raises the focused app
  and apps playing sound to 300; a weight changed after Atlas set it is let
  go and never restored over. Closing the window or turning automatic off
  puts back the automatic eases; manual ones stay. Every ease is listed in
  `$XDG_RUNTIME_DIR/net.eterneon.atlas.monitor/eased`, so after a crash
  `open` puts back the automatic ones and takes up the manual ones again,
  where the weight is still 10. A tick reads two held files per unit
  (86 µs for 33 units); a sound check takes about 12 ms and runs only
  while something is busy or eased. To see it: `--example ease` (watches,
  changes nothing), `--bench`, and `--trial`, which eases throwaway
  `app-atlastest*` scopes only, simulates a crash and recovers.
- `health`: `check` turns what the other readers last gave into the short
  list of what is wrong, critical first: processor or a graphics card over
  85 °C (critical), memory short (under 2 GiB available or over 90% used),
  swap over 25% full while under 25% of memory is available, a disk's
  mounted filesystems under 5% free, a drive that expects to fail
  (critical), is out of spare blocks or has used 90% of its rated life,
  and failed services (three named, the rest counted). The thresholds are
  the Go version's. It reads nothing itself: the loop passes this tick's
  temperatures and memory, the last `space` and the slower SMART and
  `failed` answers. zram and disks with nothing mounted are never full.
  Each alert has a `title` and a `detail` in English. To see it:
  `--example health`.
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
| Services: start, stop, restart, enable, disable | systemd's `org.freedesktop.systemd1` over the system bus (zbus), the call flagged to allow interactive authorization | systemd's own polkit actions (`manage-units`, `manage-unit-files`); polkit's agent asks for the password. Listing and details need nothing. |
| Drive health (SMART) | udisks2 over the system bus (`NVMe.Controller` and `Drive.Ata` properties and `SmartGetAttributes`, its cached values) | udisks2, which asks polkit for none of these. No section when udisks2 is missing. |
| Energy Saver | `CPUWeight` on the app's unit through the **user's** systemd manager | None needed: the user's own units. Reversible; restored on exit and after a crash. |
| Startup items | XDG autostart files in `~/.config/autostart`; the user's systemd units through the **user's** systemd manager (Enable, Disable, Mask, Unmask) | None needed: the user's own files and manager. Atlas Updater's tray entry and the session's own units are shown but can't be switched off. |

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
