#!/usr/bin/env bash
# PRODUCTION=1 with only the relay password set. The relay env must not
# receive the root password.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
# shellcheck source=lib/compose-env.sh
source "${ROOT}/scripts/lib/compose-env.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

export PRODUCTION=1
export SURREALDB_URL=ws://surrealdb:8000
export SURREALDB_USER=root
export SURREALDB_PASSWORD=root-password-must-not-leak
export SURREALDB_RELAY_PASSWORD=only-relay
unset SURREALDB_RELAY_FALLBACK_USER SURREALDB_RELAY_FALLBACK_PASSWORD SURREALDB_PASS || true

warn_file="$(mktemp)"
export_compose_fallbacks 2>"${warn_file}"
warn="$(cat "${warn_file}")"
rm -f "${warn_file}"

[[ -z "${SURREALDB_RELAY_FALLBACK_USER}" ]] || fail "fallback user leaked: ${SURREALDB_RELAY_FALLBACK_USER}"
[[ -z "${SURREALDB_RELAY_FALLBACK_PASSWORD}" ]] || fail "fallback password leaked"
[[ "$warn" != *"root-password-must-not-leak"* ]] || fail "warning printed the root password"
[[ "$warn" != *"only-relay"* ]] || fail "warning printed the relay password"

surreal_image='docker.io/surrealdb/surrealdb@sha256:f703abfda71faa20572c9b0e99a1e2cb89494525ecc7a88345306c0fd3333845'
grep -q "$surreal_image" "${ROOT}/compose.yml" || fail "surreal image is not pinned"
grep -q 'docker.io/library/rust:1.92-alpine@sha256:f6c22e0a256c05d44fca23bf530120b5d4a6249a393734884281ca80782329bc' \
    "${ROOT}/relay/Containerfile" || fail "relay builder image is not pinned"
grep -q 'FORCE_RELAY_PASSWORD_SYNC:' "${ROOT}/compose.yml" || fail "site-server does not receive FORCE_RELAY_PASSWORD_SYNC"
if grep -q 'surrealdb:v3.0' "${ROOT}/relay/ci-chat-flow.sh"; then
    fail "relay CI still uses the floating surrealdb:v3.0 tag"
fi
grep -q 'surreal_image_from_compose' "${ROOT}/relay/ci-chat-flow.sh" || fail "relay CI does not read the compose digest"
grep -q 'relay-chat-flow -- relay-login' "${ROOT}/relay/ci-chat-flow.sh" || fail "relay CI does not run the record login"

# The helper refuses anything that is not the compose digest pin.
# shellcheck source=../relay/ci-chat-flow.sh
# Sourcing the CI script would start docker. Re-read the digest the same way.
ci_image="$(awk '
    $0 ~ /^  surrealdb:/ { in_svc = 1; next }
    in_svc && $0 ~ /^  [^[:space:]]/ { in_svc = 0 }
    in_svc && $1 == "image:" { print $2; exit }
' "${ROOT}/compose.yml")"
[[ "$ci_image" == "$surreal_image" ]] || fail "compose digest drifted from the expected pin"

echo "relay production env checks passed"
