# Atlas Monitor

The system monitor of [AtlasOS](https://github.com/EternalCoder454/AtlasOS):
your apps and what they use, the processor, memory, disks, network, graphics
card, battery and sensors, services and startup items.

It comes with AtlasOS and updates with it, through Atlas Updater. It is built
for AtlasOS (Fedora Kinoite 44) only. For other distributions, see the
[original Atlas Monitor](https://github.com/EternalCoder454/atlas-monitor).

Rust + Qt 6 + Kirigami. Its look is Atlas.Ui from
[atlas-framework](https://github.com/EternalCoder454/atlas-framework), and its
core library is shared with [Atlas Updater](https://github.com/EternalCoder454/atlasos-updater). How it is
put together: [docs/DESIGN.md](docs/DESIGN.md).

## Build

Inside a Fedora 44 container (`scripts/dev.sh` sets one up):

```sh
scripts/dev.sh cargo test --workspace
scripts/dev.sh bash -c 'cmake -S apps/atlas-monitor -B build/dev -G Ninja && cmake --build build/dev'
```

The RPM: `packaging/build-rpm.sh <out dir>`, run as root in `fedora:44`.

Both need Atlas.Ui, the `atlas-ui` RPM, which is in no repository yet. Build
atlas-framework's RPMs with its own `packaging/build-rpm.sh` and pass their
directory as `ATLAS_LOCAL_RPMS=<dir>`, to `build-rpm.sh` and to the first
`scripts/dev.sh` run (which builds the dev image; delete an older image).

## Licence

MIT
