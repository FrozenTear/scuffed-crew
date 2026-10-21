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

if [[ ! -f "$SECRETS" ]]; then
    echo "error: missing $SECRETS — run scripts/install.sh first" >&2
    exit 1
fi

# shellcheck disable=SC1090
set -a
# shellcheck source=/dev/null
source "$SECRETS"
set +a
# Re-assert after source so a blank line in secrets.env cannot clear the reset.
export BOOTSTRAP_ADMIN_USERNAME="${BOOTSTRAP_ADMIN_USERNAME:-admin}"
export BOOTSTRAP_ADMIN_RESET=1
# shellcheck source=lib/compose-env.sh
source "$ROOT/scripts/lib/compose-env.sh"
export_compose_fallbacks

if podman compose version >/dev/null 2>&1; then
    COMPOSE=(podman compose)
elif command -v podman-compose >/dev/null 2>&1; then
    COMPOSE=(podman-compose)
else
    echo "error: need podman compose" >&2
    exit 1
fi

# Never let this recreate the database: without --no-deps, compose follows
# depends_on and recreates surrealdb when compose.yml names another image
# than the running one (an upgrade with no backup). See lib/surreal-guard.sh.
PROJECT_NAME="${COMPOSE_PROJECT_NAME:-$(basename "$ROOT")}"
# shellcheck source=lib/surreal-guard.sh
source "$ROOT/scripts/lib/surreal-guard.sh"
surreal_guard_plan
if [[ "${SURREAL_GUARD_ACTION}" == "with-deps" ]]; then
    echo "error: no ${PROJECT_NAME} SurrealDB container exists. There is no admin to reset." >&2
    echo "  Run ./scripts/install.sh or ./scripts/update.sh first." >&2
    exit 1
fi
surreal_guard_apply

echo "Recreating site-server with BOOTSTRAP_ADMIN_RESET=1 for user ${BOOTSTRAP_ADMIN_USERNAME}..."
"${COMPOSE[@]}" --env-file "$SECRETS" up -d --force-recreate --no-deps site-server

echo
echo "If the server log shows the reset applied, remove BOOTSTRAP_ADMIN_RESET and"
echo "BOOTSTRAP_ADMIN_PASSWORD from your shell and from data/secrets.env, then run"
echo "./scripts/update.sh to recreate site-server without them."
echo "Then sign in at /login with the new password."
