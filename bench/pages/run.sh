#!/bin/bash
# What each page costs: the Release build under a private virtual KWin on the
# host (Wayland at 1.5x, as AtlasOS runs it), never the real desktop.
#   bench/pages/run.sh [runs] [seconds] [page...]
# Build build/release first (see README.md). One JSON line per page and run
# on stdout:
#   cpu_ms_s    the app's CPU (user+system) per second over the window
#   reads_s     read syscalls per second (/proc/PID/io syscr)
#   rss_open    RSS in MiB after the warm-up, the page "just opened"
#   rss, pss    RSS and PSS in MiB at the end of the window
# The app sees the host's processes (--pid=host) and the host's system bus
# (read-only questions: failed services, the Services list, SMART), but no
# session services: its session bus is a private one in the container.
# Extra podman arguments go in PAGES_ENV (e.g. "-e QT_QUICK_BACKEND=rhi").
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
runs=${1:-3}
seconds=${2:-30}
shift 2 || shift $#
pages=("$@")
if [ ${#pages[@]} -eq 0 ]; then
    pages=(overview cpu memory apps services sensors energy startup settings)
fi
sock=wl-pages-$$
rt=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
mkdir -p "$repo/out/pages"

# As in bench/chart/run-kwin.sh: KWin on a private bus that cannot start
# services; podman itself gets the user bus for its systemd scope.
# shellcheck disable=SC2016
dbus-run-session --config-file="${PRIVATE_BUS_CONF:-$HOME/.claude/headless/private-bus.conf}" -- bash -c '
    set -uo pipefail
    sock=$1 rt=$2 repo=$3 runs=$4 seconds=$5
    shift 5
    kwin_wayland --virtual --no-lockscreen --width 2880 --height 1800 --socket "$sock" >"$repo/out/pages/kwin.log" 2>&1 &
    kwin=$!
    trap "kill $kwin 2>/dev/null; wait $kwin 2>/dev/null" EXIT INT TERM
    for _ in $(seq 50); do [ -S "$rt/$sock" ] && break; sleep 0.1; done
    if [ ! -S "$rt/$sock" ]; then
        echo "kwin did not start; see out/pages/kwin.log" >&2
        exit 1
    fi
    # PAGES_ENV is a list of podman arguments, split on purpose.
    DBUS_SESSION_BUS_ADDRESS=unix:path=$rt/bus podman run --rm --security-opt label=disable \
        --userns=keep-id --pid=host \
        -v "$repo":/src:ro -w /src \
        -v "$rt/$sock":/run/wl/wayland-0 \
        -v /run/dbus/system_bus_socket:/run/dbus/system_bus_socket \
        -e WAYLAND_DISPLAY=/run/wl/wayland-0 -e QT_QPA_PLATFORM=wayland -e QT_SCALE_FACTOR=1.5 \
        ${PAGES_ENV:-} \
        localhost/atlas-monitor-dev:44 bash /src/bench/pages/measure.sh "$runs" "$seconds" "$@" \
        2>"$repo/out/pages/app.log"
' _ "$sock" "$rt" "$repo" "$runs" "$seconds" "${pages[@]}"
