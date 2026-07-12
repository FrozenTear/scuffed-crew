#!/usr/bin/env bash
# Pull latest git + prebuilt site-server image and recreate the stack.
# Does NOT compile on the VPS (that is what GH Actions + GHCR are for).
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS="$ROOT/data/secrets.env"
cd "$ROOT"

if [[ ! -f "$SECRETS" ]]; then
    echo "error: missing $SECRETS — run scripts/install.sh first" >&2
    exit 1
fi

# shellcheck disable=SC1090
set -a
# shellcheck source=/dev/null
source "$SECRETS"
set +a

SITE_SERVER_IMAGE="${SITE_SERVER_IMAGE:-ghcr.io/frozentear/scuffed-crew:main}"
export SITE_SERVER_IMAGE
HOST_PORT="${HOST_PORT:-3000}"

if ! command -v podman >/dev/null 2>&1; then
    echo "error: podman is required" >&2
    exit 1
fi

if podman compose version >/dev/null 2>&1; then
    COMPOSE=(podman compose)
elif command -v podman-compose >/dev/null 2>&1; then
    COMPOSE=(podman-compose)
else
    echo "error: need 'podman compose' or 'podman-compose'" >&2
    exit 1
fi

if [[ -d .git ]]; then
    echo "Git pull..."
    git pull --ff-only
fi

# Persist image pin if missing so compose and restarts stay consistent.
if ! grep -q '^SITE_SERVER_IMAGE=' "$SECRETS" 2>/dev/null; then
    echo "SITE_SERVER_IMAGE=${SITE_SERVER_IMAGE}" >> "$SECRETS"
    echo "Added SITE_SERVER_IMAGE to secrets.env"
fi

echo "Pulling ${SITE_SERVER_IMAGE} ..."
if ! podman pull "${SITE_SERVER_IMAGE}"; then
    echo >&2
    echo "error: could not pull ${SITE_SERVER_IMAGE}" >&2
    echo "  • Wait for the GitHub Action \"Publish image\" to finish on main" >&2
    echo "  • If the package is private: podman login ghcr.io" >&2
    echo "  • Fallback (slow): BUILD_FROM_SOURCE=1 ./scripts/install.sh" >&2
    exit 1
fi

# Avoid --force-recreate: with Podman + external docker-compose it often starts a
# new site-server before the old one releases 127.0.0.1:HOST_PORT (conmon still
# listening → "address already in use"). Stop + remove, wait for free port, then up.
echo "Stopping site-server..."
"${COMPOSE[@]}" --env-file "$SECRETS" stop site-server 2>/dev/null || true
echo "Removing site-server container..."
"${COMPOSE[@]}" --env-file "$SECRETS" rm -f site-server 2>/dev/null || true

# Best-effort: drop any leftover project site-server by name
podman rm -f scuffed-crew-site-server-1 2>/dev/null || true

wait_port_free() {
    local port="$1"
    local tries="${2:-40}"
    local i
    for ((i = 1; i <= tries; i++)); do
        # Match 127.0.0.1:PORT or *:PORT listen lines
        if ! ss -tln 2>/dev/null | grep -qE "[:.]${port}\\s"; then
            return 0
        fi
        sleep 0.5
    done
    return 1
}

echo "Waiting for port ${HOST_PORT} to free..."
if ! wait_port_free "${HOST_PORT}" 40; then
    echo "error: 127.0.0.1:${HOST_PORT} still in use after stopping site-server" >&2
    echo "  Run: ss -tlnp | grep ${HOST_PORT}" >&2
    echo "  Then: podman ps -a  # remove any leftover site-server container" >&2
    exit 1
fi

echo "Starting site-server (Surreal left running)..."
"${COMPOSE[@]}" --env-file "$SECRETS" up -d site-server

echo
echo "Updated. Health: curl -sS http://127.0.0.1:${HOST_PORT}/api/health"
