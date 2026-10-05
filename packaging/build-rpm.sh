#!/bin/bash
# Build the Atlas Monitor RPM inside a fedora:44 container, as root.
#   packaging/build-rpm.sh <out dir> [rpmbuild options]
# The binary RPM (no source, no debuginfo) is copied to <out dir>.
# Cargo needs network access.
# ATLAS_LOCAL_RPMS=<dir> installs the RPMs in <dir> first: atlas-framework's
# (atlas-ui), which the app builds against and no repository has.
set -euo pipefail

# A fresh checkout gives every file a new modification time, so cargo and
# ninja would rebuild everything the cache holds. A file whose content is
# what the cached build was made from gets that build's time for it back;
# every other file (changed, new, or no record of the last build) gets the
# current time, so it is rebuilt whatever time the checkout gave it.
# <file>: "<sha256> <mtime> <path>" per line, path last.
restore_times() {
    local stage=$1 file=$2 now hash time path
    local -A was=()
    now=$(date +%s)
    # Everything starts at now; only a file matched below gets an old time.
    find "$stage" -type f -exec touch -m -d "@$now" -- {} +
    if [ -f "$file" ]; then
        while read -r hash time path; do
            [[ $time =~ ^[0-9]+(\.[0-9]+)?$ ]] && was["$hash $path"]=$time
        done <"$file"
    fi
    (cd "$stage" && find . -type f -print0 | xargs -0 -r sha256sum) >"$stage.sums"
    while read -r hash path; do
        path=${path#\*}  # sha256sum's "binary" marker
        time=${was["$hash $path"]:-}
        # A name sha256sum had to escape (or read had to trim) matches nothing
        # and keeps now.
        if [ -n "$time" ] && [ -f "$stage/$path" ]; then
            touch -m -d "@$time" -- "$stage/$path"
        fi
    done <"$stage.sums"
    rm -f "$stage.sums"
}

record_times() {
    local stage=$1 file=$2 hash path time
    local -A at=()
    while IFS= read -r -d '' time && IFS= read -r -d '' path; do
        at["$path"]=$time
    done < <(cd "$stage" && find . -type f -printf '%T@\0%p\0')
    while read -r hash path; do
        path=${path#\*}
        time=${at["$path"]:-}
        [ -n "$time" ] && printf '%s %s %s\n' "$hash" "$time" "$path"
    done < <(cd "$stage" && find . -type f -print0 | xargs -0 -r sha256sum) >"$file.tmp"
    mv "$file.tmp" "$file"
}

main() {
    out=${1:?usage: build-rpm.sh <out dir> [rpmbuild options]}
    shift

    here=$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)
    src=$(dirname "$here")
    spec=$here/atlas-monitor.spec
    version=$(awk '/^Version:/ {print $2; exit}' "$spec")

    # ATLAS_LOCAL_RPMS goes to it in the environment.
    bash "$here/install-builddeps.sh"

    # ATLAS_BUILD_CACHE=<dir> keeps the CMake build (Corrosion's cargo target
    # in it) in <dir>, at a fixed path, so a rebuild only compiles what
    # changed (CI caches it). It starts over when the version or the
    # toolchain changes. Without it, every build is a clean one in a temp dir.
    rpmopts=()
    cache=${ATLAS_BUILD_CACHE:-}
    if [ -n "$cache" ]; then
        mkdir -p "$cache"
        cache=$(cd "$cache" && pwd -P)
        case $cache/ in
            "$(cd "$src" && pwd -P)"/*) echo "ATLAS_BUILD_CACHE must be outside the source tree" >&2; exit 1 ;;
        esac
        # It goes into the build flags (split on spaces) and the spec's shell.
        if [[ ! $cache =~ ^[A-Za-z0-9_./-]+$ ]]; then
            echo "ATLAS_BUILD_CACHE may hold only letters, digits and _ . / -" >&2; exit 1
        fi
        # Only a directory this script made (or an empty one): it deletes
        # rpmbuild/ and cmake/ in it.
        if [ ! -e "$cache/.atlas-monitor-build-cache" ] && [ -n "$(ls -A "$cache")" ]; then
            echo "ATLAS_BUILD_CACHE ($cache) is not empty and is not an Atlas Monitor build cache" >&2; exit 1
        fi
        touch "$cache/.atlas-monitor-build-cache"
        top=$cache/rpmbuild
        rm -rf "$top"
        toolchain=$(rpm -q rust cargo corrosion gcc-c++ cmake qt6-qtbase-devel qt6-qtdeclarative-devel \
            kf6-kirigami-devel atlas-ui || true)
        toolchain="atlas-monitor-$version
$toolchain"
        if [ "$(cat "$cache/toolchain" 2>/dev/null)" != "$toolchain" ]; then
            rm -rf "$cache/cmake"
            printf '%s\n' "$toolchain" >"$cache/toolchain"
        fi
        rpmopts+=(--define "_atlas_build_cache $cache")
    else
        top=$(mktemp -d)
    fi
    trap 'rm -rf "$top"' EXIT
    mkdir -p "$top"/{SOURCES,BUILD,RPMS,SRPMS,SPECS}
    # The sources go through a copy, so a cached build can give its unchanged
    # files their old times back (the checkout itself is not touched).
    stage=$top/source
    mkdir -p "$stage"
    tar -C "$src" --exclude=./.git --exclude=./target --exclude=./out --exclude=./build -cf - . |
        tar -C "$stage" -xf -
    if [ -n "$cache" ]; then
        restore_times "$stage" "$cache/source-times"
        # Until this build succeeds, its outputs may be newer than sources
        # they were not built from: no times to restore next time.
        rm -f "$cache/source-times"
    fi
    tar -C "$stage" --transform "s,^\./,atlas-monitor-$version/," \
        -czf "$top/SOURCES/atlas-monitor-$version.tar.gz" .

    rpmbuild -bb "${rpmopts[@]}" "$@" --define "_topdir $top" "$spec"
    if [ -n "$cache" ]; then
        # Without a record the next build is a full one; the RPM still counts.
        record_times "$stage" "$cache/source-times" ||
            echo "warning: could not record the source times; the next build starts over" >&2
    fi

    mkdir -p "$out"
    find "$top/RPMS" -name '*.rpm' ! -name '*.src.rpm' ! -name '*debuginfo*' ! -name '*debugsource*' \
        -exec cp -v {} "$out"/ \;
}

main "$@"
exit $?
