#!/usr/bin/env bash
# /api/ready 404 is an old image. The deploy check falls back to /api/health.
# 503 and connection failure keep waiting.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/deploy-probe.sh
source "${SCRIPT_DIR}/lib/deploy-probe.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

[[ "$(deploy_probe_choice 200)" == "ready" ]] || fail "200"
[[ "$(deploy_probe_choice 404)" == "health" ]] || fail "404"
[[ "$(deploy_probe_choice 503)" == "wait" ]] || fail "503 must keep waiting"
[[ "$(deploy_probe_choice 000)" == "wait" ]] || fail "connection failure"
[[ "$(deploy_probe_choice 500)" == "wait" ]] || fail "500"

update="${SCRIPT_DIR}/update.sh"
grep -q 'deploy_probe_choice' "$update" || fail "update.sh does not use the probe choice"
grep -q 'Falling back to /api/health' "$update" || fail "update.sh does not log the fallback"
grep -q '/api/health' "$update" || fail "update.sh never requests /api/health"
# curl -sf hides the status code, so a 404 would look like every other failure.
if grep -n 'curl -sf' "$update" | grep -q '/api/ready'; then
    fail "update.sh still uses curl -sf for /api/ready"
fi

echo "deploy probe checks passed"
