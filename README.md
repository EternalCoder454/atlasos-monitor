# Telamon Monitor

The system monitor of [Telamon OS](https://github.com/EternalCoder454/AtlasOS):
your apps and what they use, the processor, memory, disks, network, graphics
card, battery and sensors, services and startup items. Its System Info and
Devices pages take the place of KDE Info Center.

`telamon-monitor --page <name>` opens a page (`system`, `devices`, `apps`,
`disk:nvme0n1`, ...), in the running window if there is one.

It comes with Telamon OS and updates with it, through Telamon Updater. It is built
for Telamon OS (Fedora Kinoite 44) only. For other distributions, see the
[original Telamon Monitor](https://github.com/EternalCoder454/telamon-monitor).

Rust + Qt 6 + Kirigami. Its look is Telamon.Ui from
[atlas-framework](https://github.com/EternalCoder454/atlas-framework), which
also gives it its start, settings file, logging and crash reports. How it is
put together: [docs/DESIGN.md](docs/DESIGN.md).

## Build

Inside a Fedora 44 container (`scripts/dev.sh` sets one up):

```sh
scripts/dev.sh cargo test --workspace
scripts/dev.sh bash -c 'cmake -S apps/telamon-monitor -B build/dev -G Ninja && cmake --build build/dev'
```

The RPM: `packaging/build-rpm.sh <out dir>`, run as root in `fedora:44`.

Both need Telamon.Ui, the `telamon-ui` RPM, which is in no repository yet. Build
atlas-framework's RPMs with its own `packaging/build-rpm.sh` and pass their
directory as `ATLAS_LOCAL_RPMS=<dir>`, to `build-rpm.sh` and to the first
`scripts/dev.sh` run (which builds the dev image; delete an older image).

## Licence

MIT
