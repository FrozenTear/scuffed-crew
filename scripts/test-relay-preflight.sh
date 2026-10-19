#!/usr/bin/env bash
# relay-up.sh must not switch on a failed ready probe or a failed sign-in.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/relay-preflight.sh
source "${SCRIPT_DIR}/lib/relay-preflight.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

ok="$(relay_preflight_decision "cmVsYXlzZWNyZXQ=" 200 200)"
[[ "$ok" == "ok" ]] || fail "expected ok, got ${ok}"

blank="$(relay_preflight_decision "   " 200 200 || true)"
[[ "$blank" == *"missing or blank"* ]] || fail "blank password: ${blank}"

ready="$(relay_preflight_decision "cmVsYXlzZWNyZXQ=" 503 200 || true)"
[[ "$ready" == *"/api/ready returned 503"* ]] || fail "ready: ${ready}"

signin="$(relay_preflight_decision "cmVsYXlzZWNyZXQ=" 200 401 || true)"
[[ "$signin" == *"relay sign-in returned 401"* ]] || fail "signin: ${signin}"

weird="$(relay_preflight_decision "has space" 200 200 || true)"
[[ "$weird" == *"will not send"* ]] || fail "charset: ${weird}"

# The decision text must not echo a password-shaped value.
[[ "$blank" != *"cmVsYXlzZWNyZXQ="* ]] || fail "blank path printed a password"
[[ "$signin" != *"cmVsYXlzZWNyZXQ="* ]] || fail "signin path printed a password"

up="${SCRIPT_DIR}/relay-up.sh"
grep -q -- '--connect-timeout 2 --max-time 5' "$up" || fail "ready probe has no timeout"
preflight="${SCRIPT_DIR}/lib/relay-preflight.sh"
grep -q 'timeout 20 podman exec' "$preflight" || fail "sign-in exec has no timeout"
grep -q -- '--connect-timeout 2 --max-time 5' "$preflight" || fail "sign-in curl has no timeout"

echo "relay preflight checks passed"
