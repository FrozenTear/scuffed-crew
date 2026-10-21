#!/usr/bin/env bash
# Fail when a pinned SurrealDB server image differs from the surrealdb crate
# in Cargo.lock.
#
# The client crate and the server image are upgraded separately. Dependabot
# only sees the crate, so a bump there marks the advisories fixed while the
# production database keeps running the old server. This check makes the two
# move together: every `docker.io/surrealdb/surrealdb:<tag>` in a tracked file
# must be exactly `v<crate version>`. A floating tag such as `v3.0` or
# `latest` fails too, because it hides which server is running.
#
# Usage: scripts/check-surrealdb-version.sh [repo-root]
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
LOCK="${ROOT}/Cargo.lock"

if [[ ! -f "${LOCK}" ]]; then
    echo "error: ${LOCK} not found" >&2
    exit 1
fi

# Every [[package]] block named exactly "surrealdb" (not surrealdb-core etc.).
mapfile -t crate_versions < <(awk '
    /^\[\[package\]\]/ { name = "" }
    /^name = "/ { name = $3; gsub(/"/, "", name) }
    /^version = "/ && name == "surrealdb" { v = $3; gsub(/"/, "", v); print v }
' "${LOCK}" | sort -u)

if [[ "${#crate_versions[@]}" -ne 1 ]]; then
    echo "error: expected one surrealdb crate version in Cargo.lock, found ${#crate_versions[@]}: ${crate_versions[*]:-none}" >&2
    exit 1
fi
want="v${crate_versions[0]}"

# Self-test the image matcher on fixed strings so a broken regex cannot pass.
image_re='docker\.io/surrealdb/surrealdb:[^[:space:]"'\''`]+'
if [[ "$(printf '%s\n' '    image: docker.io/surrealdb/surrealdb:v3.0' | grep -oE "${image_re}")" != "docker.io/surrealdb/surrealdb:v3.0" ]]; then
    echo "error: image matcher missed a compose image line" >&2
    exit 1
fi

files=()
if git -C "${ROOT}" rev-parse --is-inside-work-tree >/dev/null 2>&1; then
    while IFS= read -r -d '' f; do
        files+=("${ROOT}/${f}")
    done < <(git -C "${ROOT}" ls-files -z)
else
    while IFS= read -r -d '' f; do
        files+=("$f")
    done < <(find "${ROOT}" \( -path "${ROOT}/.git" -o -path "${ROOT}/target" \) -prune -o -type f -print0)
fi

status=0
found=0
self="$(cd "$(dirname "$0")" && pwd)/$(basename "$0")"
for f in "${files[@]}"; do
    [[ -f "$f" ]] || continue
    # This script names the image in comments and its self-test.
    [[ "$f" -ef "${self}" ]] && continue
    while IFS= read -r hit; do
        [[ -n "${hit}" ]] || continue
        found=$((found + 1))
        lineno="${hit%%:*}"
        image="${hit#*:}"
        tag="${image##*:}"
        if [[ "${tag}" != "${want}" ]]; then
            echo "ERROR: ${f#"${ROOT}"/}:${lineno}: ${image} (Cargo.lock surrealdb is ${crate_versions[0]}; pin ${want})" >&2
            status=1
        fi
    done < <(grep -noE "${image_re}" "$f" 2>/dev/null || true)
done

if [[ "${found}" -eq 0 ]]; then
    echo "error: no docker.io/surrealdb/surrealdb:<tag> reference found; the check would guard nothing" >&2
    exit 1
fi

if [[ "${status}" -ne 0 ]]; then
    echo "Upgrade the server image and the crate together. docs/deploy.md 'Upgrading the SurrealDB server' has the backup and rollback steps." >&2
    exit 1
fi

echo "ok: ${found} SurrealDB image reference(s) pinned to ${want} (Cargo.lock surrealdb ${crate_versions[0]})"
