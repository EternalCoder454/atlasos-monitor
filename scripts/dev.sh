#!/bin/bash
# Run a command in the fedora:44 build container, with the repo at /src and
# the cargo and dnf caches in named podman volumes (shared with the other Telamon apps).
#   scripts/dev.sh <command...>     e.g. scripts/dev.sh cargo test --workspace
#   scripts/dev.sh                  an interactive shell
# The first run installs the build dependencies from the spec (cached after).
# They include telamon-ui, which no repository has: that run needs
# ATLAS_LOCAL_RPMS=<dir> holding atlas-framework's RPMs (telamon-ui and
# telamon-symbols-fonts): the out dir of its packaging/build-rpm.sh, one version
# only. An image made before telamon-ui was needed lacks it: delete the image.
# Set CARGO_TARGET_DIR to /src/target/<name> to keep one target dir per task.
set -euo pipefail

repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)
image=localhost/telamon-monitor-dev:44

if ! podman image exists "$image"; then
    rpms=${ATLAS_LOCAL_RPMS:?the dev image needs atlas-framework RPMs: set ATLAS_LOCAL_RPMS=<dir>}
    rpms=$(cd "$rpms" && pwd)
    # No relabelling (label=disable, as below): :z or :Z would give the
    # mounted directories this container's SELinux label.
    ctr=$(podman run -d --security-opt label=disable \
        -v "$repo/packaging":/packaging:ro \
        -v "$rpms":/atlas-rpms:ro \
        -v atlas-dnf:/var/cache/libdnf5 \
        registry.fedoraproject.org/fedora:44 sleep infinity)
    trap 'podman rm -f "$ctr" >/dev/null' EXIT
    podman exec "$ctr" bash -c '
        echo keepcache=True >>/etc/dnf/dnf.conf
        dnf -y install dnf5-plugins rpm-build clippy rustfmt xorg-x11-server-Xvfb \
            dbus-daemon qt6-qtbase-gui kf6-qqc2-desktop-style breeze-icon-theme \
            ImageMagick xdotool \
            /atlas-rpms/telamon-ui-[0-9]*.rpm /atlas-rpms/telamon-symbols-fonts-[0-9]*.rpm &&
        dnf -y builddep /packaging/telamon-monitor.spec' >&2
    podman commit "$ctr" "$image" >/dev/null
    podman rm -f "$ctr" >/dev/null
    trap - EXIT
fi

tty=()
[ -t 0 ] && tty=(-it)
# SELinux labelling is off for the container (label=disable) rather than
# relabelling the checkout with :z or :Z, which would lock other containers
# and confined tools out of it, and two containers out of each other.
# --ulimit core=0: a crash in the container leaves no core dump (and no crash
# notification on the host); no-new-privileges: nothing in it gains privilege.
exec podman run --rm "${tty[@]}" --security-opt label=disable \
    --security-opt no-new-privileges --ulimit core=0 \
    -v "$repo":/src -w /src \
    -v atlas-cargo:/root/.cargo/registry \
    -v atlas-cargo-git:/root/.cargo/git \
    -e CARGO_TARGET_DIR="${CARGO_TARGET_DIR:-/src/target/dev}" \
    "$image" "${@:-bash}"
