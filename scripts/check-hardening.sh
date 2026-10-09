#!/bin/bash
# Checks the hardening of the program Telamon Monitor ships (docs/SECURITY.md,
# "Build hardening"): what Fedora's build flags (%{build_cflags}, %{build_cxxflags},
# %{build_ldflags} and %{build_rustflags}) are meant to give them, read back
# from the finished ELF files with readelf, so a change of flags, a macro that
# isn't expanded or a build that bypasses them fails the package build.
#
#   scripts/check-hardening.sh [--cxx] [--require-cet] <elf>...
#
# Every file must be
#   - a position-independent executable (ET_DYN with an entry point), so ASLR
#     moves all of it;
#   - fully RELRO (a GNU_RELRO segment and BIND_NOW), so the GOT is read-only
#     once the program starts;
#   - non-executable stack (GNU_STACK without E);
#   - free of RPATH and RUNPATH (no library search path from outside the
#     system's);
#   - free of text relocations (TEXTREL).
# Reported, not required: on x86_64, whether the program is marked for Intel CET
# (IBT and SHSTK in the GNU property note). The C++ is built with
# -fcf-protection, but rustc has no stable switch for IBT, and the linker marks
# a program only when every object in it is marked, so a program with Rust code
# in it has SHSTK (the shadow stack: Rust code is compatible) and no IBT.
# `--require-cet` makes a missing mark an error (for a program without Rust).
# With --cxx (a program with C++ in it, as Telamon Monitor's is: Qt on top of a
# Rust library) also
#   - built with stack protectors (it calls __stack_chk_fail).
set -euo pipefail

cxx=0
require_cet=0
while [ $# -gt 0 ]; do
    case $1 in
        --cxx) cxx=1; shift ;;
        --require-cet) require_cet=1; shift ;;
        --) shift; break ;;
        -*) echo "usage: $0 [--cxx] [--require-cet] <elf>..." >&2; exit 2 ;;
        *) break ;;
    esac
done
if [ $# -eq 0 ]; then
    echo "usage: $0 [--cxx] [--require-cet] <elf>..." >&2
    exit 2
fi
for tool in readelf grep; do
    command -v "$tool" >/dev/null || { echo "check-hardening: $tool is needed" >&2; exit 2; }
done

failed=0
bad() {
    echo "check-hardening: $1: $2" >&2
    failed=1
}

for f in "$@"; do
    [ -f "$f" ] || { bad "$f" "not a file"; continue; }
    header=$(readelf -hW "$f" 2>/dev/null) || { bad "$f" "not an ELF file"; continue; }
    segments=$(readelf -lW "$f")
    dynamic=$(readelf -dW "$f" 2>/dev/null || true)

    grep -Eq '^ *Type: +DYN' <<<"$header" || bad "$f" "not a position-independent executable (Type is not DYN)"
    # A DYN file with no interpreter is a shared library, not an executable.
    grep -q 'Requesting program interpreter' <<<"$segments" || bad "$f" "no program interpreter (not a PIE executable)"
    grep -q 'GNU_RELRO' <<<"$segments" || bad "$f" "no GNU_RELRO segment (partial or no RELRO)"
    if ! grep -Eq '\(BIND_NOW\)|FLAGS.*BIND_NOW|FLAGS_1.*NOW' <<<"$dynamic"; then
        bad "$f" "lazy binding (no BIND_NOW): the GOT stays writable"
    fi
    stack=$(grep 'GNU_STACK' <<<"$segments" || true)
    if [ -z "$stack" ]; then
        bad "$f" "no GNU_STACK segment (the stack may be executable)"
    elif grep -Eq 'GNU_STACK.* RWE |GNU_STACK.* R E ' <<<"$stack"; then
        bad "$f" "executable stack"
    fi
    if grep -Eq '\((RPATH|RUNPATH)\)' <<<"$dynamic"; then
        bad "$f" "has an RPATH or RUNPATH"
    fi
    if grep -Eq '\(TEXTREL\)|FLAGS.*TEXTREL' <<<"$dynamic"; then
        bad "$f" "has text relocations"
    fi

    machine=$(awk -F: '/Machine:/ { gsub(/^ +| +$/, "", $2); print $2 }' <<<"$header")
    case "$machine" in
        *X86-64*)
            notes=$(readelf -nW "$f" 2>/dev/null || true)
            missing=""
            grep -q 'x86 feature:.*IBT' <<<"$notes" || missing="IBT"
            grep -q 'x86 feature:.*SHSTK' <<<"$notes" || missing="${missing:+$missing and }SHSTK"
            if [ -n "$missing" ]; then
                if [ "$require_cet" = 1 ]; then
                    bad "$f" "not marked for Intel CET: $missing (-fcf-protection)"
                else
                    echo "check-hardening: note: $f is not marked for Intel CET: $missing (the Rust code in it is not built with -fcf-protection)" >&2
                fi
            fi
            ;;
    esac

    if [ "$cxx" = 1 ]; then
        symbols=$(readelf -sW --dyn-syms "$f" 2>/dev/null; readelf -sW "$f" 2>/dev/null || true)
        grep -q '__stack_chk_fail' <<<"$symbols" || bad "$f" "no stack protector (__stack_chk_fail is not used)"
    fi
done

if [ "$failed" = 0 ]; then
    echo "check-hardening: $# program(s) ok"
fi
exit "$failed"
