# Atlas Monitor (AtlasOS)

Rust + Qt 6.11 + Kirigami (CXX-Qt) system monitor for AtlasOS, a Fedora Kinoite
44 bootc image (repo `~/Documents/AtlasOS`). Read `docs/DESIGN.md` first: it
fixes the layout, the QObject/model API, the threading rule and what may need
privilege. Change it only together with the code that implements the change.
The roadmap is the Atlas Notes note "AtlasOS/Atlas Monitor/Roadmap".

The stack, build and look are Atlas Updater's (`~/Documents/Atlas Updater`,
github.com/EternalCoder454/atlasos-updater). When in doubt, do what it does,
except for what atlas-framework provides (startup, settings file, logging,
crash reports), which the Monitor takes from there.
The Go/GTK4 Atlas Monitor (`~/Documents/Atlas Monitor`) is the reference for
behaviour and numbers; it is frozen, so don't change it from here.

## Hard rules

- **Build and test inside a `registry.fedoraproject.org/fedora:44` container**,
  never on the host: the host lacks the Qt/KF6 -devel packages and sudo.
  `scripts/dev.sh <command>` runs one in a dev image with the build
  dependencies, the repo at `/src` and the cargo caches in named podman
  volumes. Use a separate target dir per agent or task
  (`CARGO_TARGET_DIR=/src/target/<name> scripts/dev.sh ...`).
- **Never run the GUI on the user's display.** For smoke tests inside the
  container, use `QT_QPA_PLATFORM=offscreen`, or `xvfb-run -a -s "-screen 0 1920x1080x24"`
  inside `dbus-run-session`. Real end-to-end tests happen in the AtlasOS test
  VM, which the lead runs.
- **No new privilege** (DESIGN.md, Privilege). Never call the
  atlas-system-helper and never add a method to it. Services go through
  systemd's own polkit actions, SMART through udisks2, Energy Saver through
  the user's systemd manager.
- **Atlas.Ui is the installed `atlas-ui` package** from atlas-framework
  (`~/Documents/Atlas Framework`, github.com/EternalCoder454/atlas-framework),
  not part of this build. Never fork or copy Atlas.Ui components into this
  repo: shared UI goes into atlas-framework `ui/` first, under its
  compatibility rules (DESIGN.md, Shared code). The Rust side (startup,
  settings file, logging, crash reports) is the `atlas-framework-ui` crate,
  pinned by release `tag` in the workspace `Cargo.toml`.
- **The GUI thread never blocks.** Readers live in `crates/atlas-sysinfo` (no
  Qt) and run on a worker thread; results come back with `qt_thread().queue`.
- **Rendering defaults to the CPU** (Qt Quick software backend). Charts are
  `QQuickPaintedItem`s; no custom scene-graph nodes or Qt Graphs.
- Tests assert invariants, never this machine's hardware: CI has no GPU,
  battery, kernel threads or system bus. Parsers get recorded files in
  `crates/atlas-sysinfo/tests/fixtures`.
- Commit only the paths you own (`git commit -- <paths>`). Other agents may be
  committing in this repo at the same time; retry if `index.lock` exists.
  Don't push unless the lead asked.
- Licence: MIT. App ID `net.eterneon.atlas.monitor`. Wording follows KDE:
  Title Case buttons and titles, US spelling.

## Commands

| Task | Command (from the repo root on the host) |
|---|---|
| Format | `scripts/dev.sh cargo fmt --all --check` |
| Lint | `scripts/dev.sh cargo clippy --workspace --all-targets --locked -- -D warnings` |
| Tests | `scripts/dev.sh cargo test --workspace --locked` |
| App build | `scripts/dev.sh bash -c 'cmake -S apps/atlas-monitor -B build/dev -G Ninja && cmake --build build/dev'` |
| Smoke run | `scripts/dev.sh dbus-run-session -- env QT_QPA_PLATFORM=offscreen build/dev/atlas-monitor` |
| RPM | `podman run --rm -v "$PWD":/src:Z -v <framework rpms>:/atlas-rpms:ro,z -e ATLAS_LOCAL_RPMS=/atlas-rpms -v atlas-cargo:/root/.cargo/registry -v atlas-cargo-git:/root/.cargo/git -e CARGO_HOME=/root/.cargo registry.fedoraproject.org/fedora:44 /src/packaging/build-rpm.sh /src/out` |

`<framework rpms>` is the out dir of atlas-framework's `packaging/build-rpm.sh`
(run the same way from its checkout): no repository has atlas-ui.

The app needs a session bus (single instance), hence `dbus-run-session`.
`scripts/dev.sh` builds `localhost/atlas-monitor-dev:44` on first use, which
needs `ATLAS_LOCAL_RPMS=<dir>` holding atlas-framework's RPMs; delete that
image after changing the spec's BuildRequires or to take a newer atlas-ui.
Cold builds compile CXX-Qt and Qt bindings for a few minutes.

## Moving the atlas-framework pin

The crates are pinned to a release tag (`tag = "vX.Y.Z"`). Each
atlas-framework release also opens a pull request here that moves it.
1. Change `tag` in `Cargo.toml`, then
   `scripts/dev.sh cargo update -p atlas-framework-ui`.
2. Move `.github/workflows/ci.yml`'s app-checks job to that release:
   `uses: ...app-checks.yml@<the tag's commit> # vX.Y.Z` and
   `framework-ref: vX.Y.Z` (`git ls-remote` the tag; for an annotated one the
   `^{}` line). CI's framework RPM job reads the tag from `Cargo.toml` and
   fails, naming the line it wants, when they disagree. When the app uses something new in Atlas.Ui,
   `ui:` in `src/lib.rs` and `atlas-ui >=` in the spec (Requires and
   BuildRequires) to that version.
3. Rebuild the dev image against that release's RPMs (delete it, then
   `ATLAS_LOCAL_RPMS=<dir> scripts/dev.sh ...`).
4. Commit `Cargo.toml` and `Cargo.lock` together.
