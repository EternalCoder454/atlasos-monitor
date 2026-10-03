# Page benchmark

What each page of the app costs while it is open: CPU per second, read
syscalls per second, and memory. The roadmap's targets are the Go Atlas
Monitor's figures (Fedora 44, KDE Wayland, about 700 processes):

| | Go |
|---|---|
| CPU page just opened, RSS | 87 MiB |
| An idle page | 2.4 ms/s CPU, 47 reads/s |
| Apps | 14.8 ms/s CPU, 862 reads/s |

Not part of the app build or CI. Build the Release app in the dev container:

```sh
scripts/dev.sh bash -c 'cmake -S apps/atlas-monitor -B build/release -G Ninja -DCMAKE_BUILD_TYPE=Release && cmake --build build/release'
```

(With a local Atlas Updater checkout, add
`-DFETCHCONTENT_SOURCE_DIR_ATLASOS_UPDATER=...` and mount it, as the pages
build does.)

## Running

`bench/pages/run.sh [runs] [seconds] [page...]` starts a private virtual KWin
on the host (Wayland, its own D-Bus, never the real desktop) and opens the
app in the container against it at 1.5x, once per page and run, each time
with a fresh config and session bus. After a 10 s warm-up it measures for
`seconds` and prints one JSON line per page and run:

- `cpu_ms_s`: the app's CPU (user + system) per second
- `reads_s`: read syscalls per second (`syscr` in `/proc/PID/io`), the figure
  the Go numbers use
- `rss_open`: RSS in MiB after the warm-up (the page "just opened")
- `rss`, `pss`: RSS and PSS in MiB at the end

The defaults are 3 runs of 30 s over the pages without a device. Device pages
take their name: `disk:nvme0n1`, `network:wlp7s0`, `gpu:card1`,
`battery:BAT0`. The app sees the host's processes (`--pid=host`) and the
host's system bus (read-only: failed services, the Services list, SMART),
so Apps and Services read what they would on the desktop. It gets no
session services of the host's: Energy Saver finds no user manager and has
nothing to ease. `PAGES_ENV` passes extra podman arguments (for example
`"-e QT_QUICK_BACKEND=rhi --device /dev/dri"` for GPU rendering).

This machine is noisy (the same case can differ by half between batches):
compare only within one batch, by the median of 3 runs.

## Results

Release build, software backend, virtual KWin at 1.5x, about 700 processes;
medians of 3 runs of 30 s (2026-10-03, after the changes listed below).
Memory in MiB.

| Page | CPU ms/s | reads/s | RSS just opened | RSS | PSS |
|---|---|---|---|---|---|
| Overview | 4.3 | 32 | 125 | 126 | 117 |
| Processor | 6.0 | 55 | 129 | 129 | 120 |
| Memory | 6.3 | 23 | 128 | 128 | 119 |
| Disk | 2.7 | 23 | 131 | 131 | 122 |
| Network | 2.0 | 23 | 130 | 130 | 121 |
| Graphics | 3.0 | 31 | 132 | 133 | 123 |
| Sensors | 5.3 | 65 | 133 | 133 | 124 |
| Apps | 18.3 | 344 | 153 | 154 | 145 |
| Services | 7.7 | 22 | 141 | 141 | 132 |
| Energy Saver | 3.0 | 23 | 126 | 127 | 118 |
| Startup | 2.7 | 22 | 126 | 126 | 118 |
| Settings | 2.7 | 22 | 127 | 128 | 119 |

What moved the figures:

- Forcing partial updates at a fractional scale (`main.cpp`): every page
  was 9-13 ms/s while the renderer repainted the whole window each second
  for the sidebar's figures.
- Apps, 32 → 29: `qsTr` looks up its file in Qt's resources on every call,
  and the column formatters called it for every cell each second (a fifth
  of the CPU). The strings are translated once now; the pages' unit
  formats (`Format.qml`) too.
- Apps, 29 → 22.7: a changed label dirties only its glyphs, so a busy table
  left the software renderer a region of about 1,000 slivers, which it
  carries through every node twice a frame (`QRegion` was a third of the
  CPU). Atlas.Ui's `RepaintArea` over each row makes it one rectangle per
  row. One per cell instead measured no better.
- Apps, 22.7 → 18.3: the process model tells the table which roles of a
  row changed rather than all of them, and the scan keeps each process's
  `stat`, `statm` and `io` open between ticks and reads them again with
  `pread` (no open and close per file; a held file pins its process, so a
  reused pid can't be misread). The sampler went from 7.7 to 6.5 ms/s.
- Overview, 68 → 32 reads/s: it reads no clock speeds, and the sidebar
  takes the page's CPU load and GPU busy figure instead of reading them
  again (`gpu_busy_percent` on amdgpu is a firmware query, 250-400 µs).
  Failed services are asked of systemd once rather than every tick.

Where it stands against Go: reads are lower everywhere (Apps 344 against
862, idle pages 22-32 against 47). CPU is level or below on the device and
quiet pages (Network 2.0, Disk 2.7, Startup and Settings 2.7 against 2.4,
within this machine's noise) and still above on the charts pages
(Overview 4.3, Processor 6.0, Memory 6.3) and Apps (18.3 against 14.8).
Most of the GUI thread's time is the software renderer (40-70% in `perf`),
much of it walking the scene graph's nodes.

Memory is the widest gap: 129 MiB against Go's 87 on the Processor page.
Of it, 33 MiB is three window-sized Wayland buffers (11 MiB each at 1.5x):
the virtual KWin keeps a replaced buffer until the next commit, so Qt
needs a third. About 30 MiB is the Qt libraries, unshared in the container
(shared with Plasma on a desktop, which PSS would show), 12 the heap and
about 9 other anonymous memory, the QML engine's mostly. `MALLOC_ARENA_MAX=2` changed nothing.

Tried and left out: each tick paints twice, because Qt's software render
loop asks for another frame when a layout moves an item during polish
(the threaded GL loop has a guard for it, this one doesn't). The second
frame has nothing to draw, and dropping it measured no difference.

With "Use the graphics card" (`PAGES_ENV="-e QT_QUICK_BACKEND=rhi --device
/dev/dri"`, RX 7900 XTX, earlier the same day): Overview 5.3 ms/s and 174
MiB RSS, Processor 6.7 and 177, Apps 18.0 and 207. Mesa and its shader
compiler cost about 45-50 MiB on every page, and the idle pages no CPU
less, which is why drawing on the CPU stays the default.
