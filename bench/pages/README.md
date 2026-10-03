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
medians of 3 runs of 30 s (2026-10-03). Memory in MiB.

| Page | CPU ms/s | reads/s | RSS just opened | RSS | PSS |
|---|---|---|---|---|---|
| Overview | 4.7 | 68 | 127 | 128 | 119 |
| Processor | 5.7 | 57 | 131 | 131 | 122 |
| Memory | 6.3 | 23 | 129 | 129 | 120 |
| Disk | 4.7 | 23 | 133 | 133 | 124 |
| Network | 4.7 | 23 | 130 | 131 | 122 |
| Graphics | 6.0 | 33 | 133 | 134 | 125 |
| Sensors | 5.3 | 65 | 135 | 135 | 126 |
| Apps | 22.7 | 405 | 155 | 155 | 146 |
| Services | 7.7 | 22 | 142 | 142 | 133 |
| Energy Saver | 3.0 | 23 | 127 | 127 | 118 |
| Startup | 3.0 | 22 | 127 | 128 | 119 |
| Settings | 3.3 | 22 | 129 | 129 | 120 |

What moved the figures:

- Forcing partial updates at a fractional scale (`main.cpp`): every page
  was 9-13 ms/s while the renderer repainted the whole window each second
  for the sidebar's figures.
- Apps, 32 → 29: `qsTr` looks up its file in Qt's resources on every call,
  and the column formatters called it for every cell each second (a fifth
  of the CPU). The strings are translated once now.
- Apps, 29 → 22.7: a changed label dirties only its glyphs, so a busy table
  left the software renderer a region of about 1,000 slivers, which it
  carries through every node twice a frame (`QRegion` was a third of the
  CPU). Atlas.Ui's `RepaintArea` over each row makes it one rectangle per
  row.

Where it stands against Go: reads are lower everywhere (Apps 405 against
862). CPU is still above Go: Apps 22.7 ms/s against 14.8, idle pages 3-6
against 2.4. Memory is the widest gap: 131 MiB against Go's 87 on the
Processor page. Of it, about 22 MiB is the Wayland buffers (two
window-sized shm buffers, a third while partial updates are on), about 30
the Qt libraries, unshared in the container (they would be shared with
Plasma on a desktop, which PSS would show), and about 18 heap and code.

With "Use the graphics card" (`PAGES_ENV="-e QT_QUICK_BACKEND=rhi --device
/dev/dri"`, RX 7900 XTX, same day): Overview 5.3 ms/s and 174 MiB RSS,
Processor 6.7 and 177, Apps 18.0 and 207. Mesa and its shader compiler
cost about 45-50 MiB on every page, and the idle pages no CPU less, which
is why drawing on the CPU stays the default.
