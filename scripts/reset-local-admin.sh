#!/usr/bin/env bash
# Emergency reset of a local admin password (after first-boot setup).
#
# Usage:
#   BOOTSTRAP_ADMIN_USERNAME=admin \
#   BOOTSTRAP_ADMIN_PASSWORD='your-new-long-password' \
#   ./scripts/reset-local-admin.sh
#
# Requires an existing local user. Recreates the site-server container with
# BOOTSTRAP_ADMIN_RESET=1 once, then you should remove those env vars.

set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS="$ROOT/data/secrets.env"
cd "$ROOT"

: "${BOOTSTRAP_ADMIN_PASSWORD:?Set BOOTSTRAP_ADMIN_PASSWORD to the new password}"
export BOOTSTRAP_ADMIN_USERNAME="${BOOTSTRAP_ADMIN_USERNAME:-admin}"
export BOOTSTRAP_ADMIN_RESET=1

if [[ ! -f "$SECRETS" ]]; then
    echo "error: missing $SECRETS — run scripts/install.sh first" >&2
    exit 1
fi

if podman compose version >/dev/null 2>&1; then
    COMPOSE=(podman compose)
elif command -v podman-compose >/dev/null 2>&1; then
    COMPOSE=(podman-compose)
else
    echo "error: need podman compose" >&2
    exit 1
fi

echo "Recreating site-server with BOOTSTRAP_ADMIN_RESET=1 for user ${BOOTSTRAP_ADMIN_USERNAME}..."
"${COMPOSE[@]}" --env-file "$SECRETS" up -d --force-recreate site-server

echo
echo "If the server log shows the reset applied, remove BOOTSTRAP_ADMIN_RESET from the environment"
echo "and recreate site-server again without it (or re-run compose up without those vars)."
echo "Then sign in at /login with the new password."
