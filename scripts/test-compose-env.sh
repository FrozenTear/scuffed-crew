#!/usr/bin/env bash
# Image pin and relay credential export. No cluster, no real secrets.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/compose-env.sh
source "${SCRIPT_DIR}/lib/compose-env.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

HEX="0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef"

# Dev host default is unchanged.
unset SITE_SERVER_IMAGE SITE_SERVER_IMAGE_DIGEST
resolve_site_server_image
[[ "$SITE_SERVER_IMAGE" == "ghcr.io/frozentear/scuffed-crew:main" ]] \
    || fail "default image is ${SITE_SERVER_IMAGE}"

# Tag pin is kept.
SITE_SERVER_IMAGE="ghcr.io/frozentear/scuffed-crew:sha-abc1234"
unset SITE_SERVER_IMAGE_DIGEST
resolve_site_server_image
[[ "$SITE_SERVER_IMAGE" == "ghcr.io/frozentear/scuffed-crew:sha-abc1234" ]] \
    || fail "tag pin changed to ${SITE_SERVER_IMAGE}"

# A reference that is already a digest is kept when no digest env is set.
SITE_SERVER_IMAGE="ghcr.io/frozentear/scuffed-crew@sha256:${HEX}"
resolve_site_server_image
[[ "$SITE_SERVER_IMAGE" == "ghcr.io/frozentear/scuffed-crew@sha256:${HEX}" ]] \
    || fail "existing digest changed to ${SITE_SERVER_IMAGE}"

# Digest env wins over the tag and strips a registry port from the tag only.
SITE_SERVER_IMAGE="localhost:5000/scuffed-crew:sha-abc1234"
SITE_SERVER_IMAGE_DIGEST="sha256:${HEX^^}"
resolve_site_server_image
[[ "$SITE_SERVER_IMAGE" == "localhost:5000/scuffed-crew@sha256:${HEX}" ]] \
    || fail "digest pin is ${SITE_SERVER_IMAGE}"

# Bare hex is accepted.
SITE_SERVER_IMAGE="ghcr.io/frozentear/scuffed-crew:main"
SITE_SERVER_IMAGE_DIGEST="$HEX"
resolve_site_server_image
[[ "$SITE_SERVER_IMAGE" == "ghcr.io/frozentear/scuffed-crew@sha256:${HEX}" ]] \
    || fail "bare digest pin is ${SITE_SERVER_IMAGE}"

# A bad digest fails and does not print a secret-shaped value.
SITE_SERVER_IMAGE_DIGEST="not-a-digest"
set +e
err="$(resolve_site_server_image 2>&1)"
code=$?
set -e
[[ "$code" -ne 0 ]] || fail "bad digest was accepted"
[[ "$err" == *"64 hex"* ]] || fail "bad digest error was: ${err}"

# Relay secret set: fallback password is empty, so compose does not pass root.
SURREALDB_USER=root
SURREALDB_PASSWORD="root-password-must-not-leak"
SURREALDB_RELAY_PASSWORD="relay-password-must-not-leak"
SURREALDB_RELAY_USER=scuffed_relay
unset SITE_SERVER_IMAGE_DIGEST
SITE_SERVER_IMAGE="ghcr.io/frozentear/scuffed-crew:main"
warn_file="$(mktemp)"
export_compose_fallbacks 2>"${warn_file}"
warn="$(cat "${warn_file}")"
rm -f "${warn_file}"
[[ -z "${SURREALDB_RELAY_FALLBACK_PASSWORD}" ]] \
    || fail "fallback password was set while the relay secret exists"
[[ -z "${SURREALDB_RELAY_FALLBACK_USER}" ]] \
    || fail "fallback user was set while the relay secret exists"
[[ "$warn" != *"root-password-must-not-leak"* ]] || fail "warning printed the root password"
[[ "$warn" != *"relay-password-must-not-leak"* ]] || fail "warning printed the relay password"
[[ "$SITE_SERVER_IMAGE" == "ghcr.io/frozentear/scuffed-crew:main" ]] \
    || fail "export changed the dev image to ${SITE_SERVER_IMAGE}"

# Relay secret missing: still do not copy the root password into the relay env.
unset SURREALDB_RELAY_PASSWORD
warn_file="$(mktemp)"
export_compose_fallbacks 2>"${warn_file}"
warn="$(cat "${warn_file}")"
rm -f "${warn_file}"
[[ -z "${SURREALDB_RELAY_FALLBACK_USER}" ]] || fail "fallback user was set: ${SURREALDB_RELAY_FALLBACK_USER}"
[[ -z "${SURREALDB_RELAY_FALLBACK_PASSWORD}" ]] \
    || fail "fallback password was copied"
[[ "$warn" == *"SURREALDB_RELAY_PASSWORD is unset"* ]] || fail "missing warning: ${warn}"
[[ "$warn" != *"root-password-must-not-leak"* ]] || fail "unset warning printed the root password"

echo "compose env checks passed"
