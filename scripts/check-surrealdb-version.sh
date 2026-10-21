#!/usr/bin/env bash
# Fail when a pinned SurrealDB server image differs from the surrealdb crate
# in Cargo.lock.
#
# The client crate and the server image are upgraded separately. Dependabot
# only sees the crate, so a bump there marks the advisories fixed while the
# production database keeps running the old server. This check makes the two
# move together: every SurrealDB image reference with a tag or digest in a
# tracked file must carry the tag `v<crate version>` (a digest may follow it,
# `:vX.Y.Z@sha256:...`). A floating tag such as `v3.0` or `latest`, or a
# digest with no tag, fails too, because it hides which server is running.
# Matched: docker.io/surrealdb/surrealdb:<tag> and the short surrealdb/surrealdb:<tag>,
# each optionally followed by @sha256:<digest>. A bare name with neither (a
# GitHub URL, `podman ps --filter ancestor=...`) is not an image pin.
#
# Changing the tag in compose.yml does not upgrade production: see
# docs/deploy.md "Upgrading the SurrealDB server" and scripts/lib/surreal-guard.sh.
#
# Usage: scripts/check-surrealdb-version.sh [repo-root]
set -euo pipefail

ROOT="${1:-$(cd "$(dirname "$0")/.." && pwd)}"
LOCK="${ROOT}/Cargo.lock"

image_re='(docker\.io/)?surrealdb/surrealdb(:[A-Za-z0-9_][A-Za-z0-9_.-]*)?(@sha256:[0-9A-Fa-f]+)?'

# Prints one result per reference in stdin lines:
#   tag <tag> | digest-only | (nothing for a bare name)
classify() {
    local ref tag
    while IFS= read -r ref; do
        tag=""
        if [[ "${ref}" == *surrealdb/surrealdb:* ]]; then
            tag="${ref#*surrealdb/surrealdb:}"
            tag="${tag%%@*}"
            # Sentence punctuation after a reference is not part of the tag.
            while [[ "${tag}" == *. || "${tag}" == *- ]]; do
                tag="${tag%?}"
            done
        fi
        if [[ -n "${tag}" ]]; then
            echo "tag ${tag}"
        elif [[ "${ref}" == *@sha256:* ]]; then
            echo "digest-only"
        fi
    done < <(grep -oE "${image_re}" || true)
}

# Self-test on fixed strings so a broken matcher cannot pass.
selftest() {
    local input="$1" want="$2" got
    got="$(printf '%s\n' "${input}" | classify | tr '\n' '|')"
    if [[ "${got}" != "${want}" ]]; then
        echo "error: image matcher self-test: '${input}' gave '${got}', want '${want}'" >&2
        exit 1
    fi
}
selftest '    image: docker.io/surrealdb/surrealdb:v3.0' 'tag v3.0|'
selftest 'run surrealdb/surrealdb:v3.0 start' 'tag v3.0|'
selftest 'image docker.io/surrealdb/surrealdb@sha256:0123abcd here' 'digest-only|'
selftest 'pin surrealdb/surrealdb:v3.2.3@sha256:0123abcd' 'tag v3.2.3|'
# shellcheck disable=SC2016  # literal backticks: a Markdown code span
selftest 'it runs `docker.io/surrealdb/surrealdb:v3.2.3`.' 'tag v3.2.3|'
selftest 'it runs docker.io/surrealdb/surrealdb:v3.2.3.' 'tag v3.2.3|'
selftest 'podman ps --filter ancestor=docker.io/surrealdb/surrealdb --format x' ''
selftest 'see https://github.com/surrealdb/surrealdb/releases' ''
selftest 's#^(    image: )docker\.io/surrealdb/surrealdb:[^[:space:]]+#x#' ''

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
    # This script names images in comments and its self-test.
    [[ "$f" -ef "${self}" ]] && continue
    grep -qE "${image_re}" "$f" 2>/dev/null || continue
    lineno=0
    while IFS= read -r line; do
        lineno=$((lineno + 1))
        [[ "${line}" == *surrealdb/surrealdb* ]] || continue
        while IFS= read -r result; do
            [[ -n "${result}" ]] || continue
            found=$((found + 1))
            case "${result}" in
                "tag ${want}") ;;
                digest-only)
                    echo "ERROR: ${f#"${ROOT}"/}:${lineno}: SurrealDB image pinned by digest only; use :${want} (optionally :${want}@sha256:...)" >&2
                    status=1
                    ;;
                *)
                    echo "ERROR: ${f#"${ROOT}"/}:${lineno}: SurrealDB image tag ${result#tag } (Cargo.lock surrealdb is ${crate_versions[0]}; pin ${want})" >&2
                    status=1
                    ;;
            esac
        done < <(printf '%s\n' "${line}" | classify)
    done <"$f"
done

if [[ "${found}" -eq 0 ]]; then
    echo "error: no tagged SurrealDB image reference found; the check would guard nothing" >&2
    exit 1
fi

if [[ "${status}" -ne 0 ]]; then
    echo "Move the server image and the crate together. Production upgrades only through docs/deploy.md 'Upgrading the SurrealDB server' (backup first)." >&2
    exit 1
fi

echo "ok: ${found} SurrealDB image reference(s) pinned to ${want} (Cargo.lock surrealdb ${crate_versions[0]})"
