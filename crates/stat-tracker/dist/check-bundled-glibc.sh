#!/usr/bin/env bash
# Fail when an ELF needs a glibc symbol newer than a floor (default use: 2.35,
# the Ubuntu 22.04 / Linux Mint 21 baseline).
#
# Usage:
#   check-bundled-glibc.sh <max> <file-or-dir>...
#   check-bundled-glibc.sh --expect-ok <version> <max>
#   check-bundled-glibc.sh --expect-fail <version> <max>
set -euo pipefail

version_le() {
    # $1 <= $2. Missing components count as 0. 10# avoids octal.
    local a="$1" b="$2"
    local -a as bs
    IFS=. read -r -a as <<<"${a}"
    IFS=. read -r -a bs <<<"${b}"
    local n="${#as[@]}"
    [[ "${#bs[@]}" -gt "${n}" ]] && n="${#bs[@]}"
    local i av bv
    for ((i = 0; i < n; i++)); do
        av="${as[$i]:-0}"
        bv="${bs[$i]:-0}"
        if ((10#${av} < 10#${bv})); then
            return 0
        fi
        if ((10#${av} > 10#${bv})); then
            return 1
        fi
    done
    return 0
}

if [[ "${1:-}" == "--expect-ok" || "${1:-}" == "--expect-fail" ]]; then
    mode="$1"
    ver="${2:?version}"
    max="${3:?max}"
    if version_le "${ver}" "${max}"; then
        [[ "${mode}" == "--expect-ok" ]] || {
            echo "expected ${ver} to exceed ${max}" >&2
            exit 1
        }
    else
        [[ "${mode}" == "--expect-fail" ]] || {
            echo "expected ${ver} <= ${max}" >&2
            exit 1
        }
    fi
    exit 0
fi

max="${1:?usage: check-bundled-glibc.sh <max> <file-or-dir>...}"
shift
[[ "$#" -ge 1 ]] || {
    echo "no files or directories to scan" >&2
    exit 1
}

fail=0
scan_one() {
    local f="$1" ver
    if [[ ! -e "$f" ]]; then
        return 0
    fi
    if ! readelf -h "$f" >/dev/null 2>&1; then
        return 0
    fi
    while read -r ver; do
        [[ -n "${ver}" ]] || continue
        ver="${ver#GLIBC_}"
        if ! version_le "${ver}" "${max}"; then
            echo "glibc symbol GLIBC_${ver} in ${f} is newer than ${max}" >&2
            fail=1
        fi
    done < <(readelf -V "$f" 2>/dev/null | grep -oE 'GLIBC_[0-9]+(\.[0-9]+)+' | sort -u)
}

for arg in "$@"; do
    if [[ -d "${arg}" ]]; then
        while IFS= read -r -d '' f; do
            scan_one "$f"
        done < <(find "${arg}" \( -type f -o -type l \) -print0)
    else
        scan_one "${arg}"
    fi
done

if [[ "${fail}" -ne 0 ]]; then
    exit 1
fi
echo "glibc symbols are within ${max}"
