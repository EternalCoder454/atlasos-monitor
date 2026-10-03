#!/bin/bash
# The container half of run.sh: opens the app on each page in turn, with a
# fresh config and session bus each time, and measures it.
#   measure.sh <runs> <seconds> <page...>
set -uo pipefail
runs=$1 seconds=$2
shift 2
warmup=10
tck=$(getconf CLK_TCK)

cpu_ms() { # utime+stime of $1, in ms
    local st
    read -r -a st <"/proc/$1/stat"
    echo $(((st[13] + st[14]) * 1000 / tck))
}
reads() { awk '/^syscr:/ {print $2}' "/proc/$1/io"; }
rollup() { awk -v k="$2:" '$1 == k {printf "%.1f", $2 / 1024}' "/proc/$1/smaps_rollup"; }
per_s() { awk -v a="$1" -v b="$2" -v s="$3" 'BEGIN {printf "%.2f", (b - a) / s}'; }
export tck
export -f cpu_ms reads rollup per_s

for run in $(seq "$runs"); do
    for page in "$@"; do
        home=$(mktemp -d)
        mkdir -p "$home/config" "$home/data" "$home/cache" "$home/runtime"
        chmod 700 "$home/runtime"
        printf '[Window]\nPage=%s\nWidth=1100\nHeight=1150\n' "$page" >"$home/config/atlas-monitorrc"
        # The inner script is single-quoted on purpose; its arguments come in as $1..$4.
        # shellcheck disable=SC2016
        HOME=$home XDG_CONFIG_HOME=$home/config XDG_DATA_HOME=$home/data \
            XDG_CACHE_HOME=$home/cache XDG_RUNTIME_DIR=$home/runtime \
            dbus-run-session -- bash -c '
                /src/build/release/atlas-monitor &
                app=$!
                sleep "$1"
                if ! kill -0 $app 2>/dev/null; then
                    echo "{\"page\":\"$2\",\"run\":$3,\"error\":\"exited\"}"
                    exit
                fi
                c0=$(cpu_ms $app) r0=$(reads $app) open=$(rollup $app Rss)
                sleep "$4"
                c1=$(cpu_ms $app) r1=$(reads $app)
                printf "{\"page\":\"%s\",\"run\":%d,\"cpu_ms_s\":%s,\"reads_s\":%s,\"rss_open\":%s,\"rss\":%s,\"pss\":%s}\n" \
                    "$2" "$3" "$(per_s $c0 $c1 $4)" "$(per_s $r0 $r1 $4)" \
                    "$open" "$(rollup $app Rss)" "$(rollup $app Pss)"
                kill $app
                wait $app 2>/dev/null
            ' _ "$warmup" "$page" "$run" "$seconds"
        rm -rf "$home"
    done
done
