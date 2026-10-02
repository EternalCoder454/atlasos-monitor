#!/bin/bash
# Runs chartbench in the fedora:44 dev container against a private virtual
# KWin on the host (Wayland, as AtlasOS runs it), never the real desktop.
#   bench/chart/run-kwin.sh <scale> <chartbench args...>
# Prints chartbench's JSON line on stdout and KWin's CPU (kwin_cpu_ms) on
# stderr; KWin's and the app's logs go to out/chartbench/. Build first (see
# README.md). Extra podman arguments go in CHART_ENV (e.g. "-e QT_SCALE_FACTOR=1.5").
set -euo pipefail
repo=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd)
scale=$1
shift
sock=wl-chartbench-$$
rt=${XDG_RUNTIME_DIR:-/run/user/$(id -u)}
mkdir -p "$repo/out/chartbench"

# KWin runs on a private bus that cannot start services, so nothing it starts
# outlives the run. podman itself (not the app) is given the real user bus,
# which it needs to create the container's scope in the user's systemd; the
# app in the container gets no bus at all. The inner script is single-quoted
# on purpose; its arguments come in as $1..$n.
# shellcheck disable=SC2016
dbus-run-session --config-file="${PRIVATE_BUS_CONF:-$HOME/.claude/headless/private-bus.conf}" -- bash -c '
    set -uo pipefail
    kwin_wayland --virtual --no-lockscreen --width 2880 --height 1800 --scale "$1" --socket "$2" >"$4/out/chartbench/kwin.log" 2>&1 &
    kwin=$!
    trap "kill $kwin 2>/dev/null; wait $kwin 2>/dev/null" EXIT INT TERM
    for _ in $(seq 50); do [ -S "$3/$2" ] && break; sleep 0.1; done
    if [ ! -S "$3/$2" ]; then
        echo "kwin did not start; see out/chartbench/kwin.log" >&2
        exit 1
    fi
    # CHART_ENV is a list of podman arguments, split on purpose.
    out=$(DBUS_SESSION_BUS_ADDRESS=unix:path=$3/bus podman run --rm --security-opt label=disable \
        -v "$4":/src -w /src \
        -v "$3/$2":/run/wl/wayland-0 \
        -e XDG_RUNTIME_DIR=/run/wl -e WAYLAND_DISPLAY=wayland-0 -e QT_QPA_PLATFORM=wayland \
        ${CHART_ENV:-} \
        localhost/atlas-monitor-dev:44 build/chartbench/chartbench "${@:5}" 2>"$4/out/chartbench/app.log")
    line=$(grep "^{" <<<"$out" || true)
    if [ -z "$line" ]; then
        echo "chartbench printed no result; see out/chartbench/app.log" >&2
        exit 1
    fi
    echo "$line"
    # KWin'"'"'s own CPU over the run (utime+stime, ms, startup included)
    read -r -a st < /proc/$kwin/stat
    echo "kwin_cpu_ms $(( (st[13] + st[14]) * 1000 / $(getconf CLK_TCK) ))" >&2
' _ "$scale" "$sock" "$rt" "$repo" "$@"
