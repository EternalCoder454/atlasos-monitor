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
