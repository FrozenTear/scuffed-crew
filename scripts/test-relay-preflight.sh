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

echo "relay preflight checks passed"
