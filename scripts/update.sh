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

port_in_use() {
    local port="$1"
    ss -tln 2>/dev/null | grep -qE "[:.]${port}[[:space:]]"
}

# Free HOST_PORT: compose stop/rm, any container publishing the port, orphan conmon.
free_host_port() {
    local port="$1"

    echo "Stopping site-server..."
    "${COMPOSE[@]}" --env-file "$SECRETS" stop site-server 2>/dev/null || true
    echo "Removing site-server container..."
    "${COMPOSE[@]}" --env-file "$SECRETS" rm -f site-server 2>/dev/null || true

    # Named leftovers (compose project prefixes vary)
    podman rm -f scuffed-crew-site-server-1 2>/dev/null || true
    podman ps -aq --filter "name=site-server" 2>/dev/null | while read -r cid; do
        [[ -n "${cid}" ]] || continue
        echo "Removing leftover container ${cid}..."
        podman rm -f "${cid}" 2>/dev/null || true
    done

    # Containers that still publish this host port
    podman ps -aq --filter "publish=${port}" 2>/dev/null | while read -r cid; do
        [[ -n "${cid}" ]] || continue
        echo "Removing container publishing :${port} (${cid})..."
        podman rm -f "${cid}" 2>/dev/null || true
    done

    # Orphan conmon can hold the bind after the container is gone (Podman + docker-compose).
    if port_in_use "${port}" && command -v ss >/dev/null 2>&1; then
        local pids pid comm
        pids="$(ss -tlnp 2>/dev/null | grep -E "[:.]${port}[[:space:]]" | grep -oE 'pid=[0-9]+' | cut -d= -f2 | sort -u || true)"
        for pid in ${pids}; do
            comm="$(ps -p "${pid}" -o comm= 2>/dev/null || true)"
            if [[ "${comm}" == "conmon" ]]; then
                echo "Killing orphan conmon pid=${pid} still bound to :${port}..."
                kill "${pid}" 2>/dev/null || true
                sleep 0.3
                kill -9 "${pid}" 2>/dev/null || true
            fi
        done
    fi
}

wait_port_free() {
    local port="$1"
    local tries="${2:-40}"
    local i
    for ((i = 1; i <= tries; i++)); do
        if ! port_in_use "${port}"; then
            return 0
        fi
        sleep 0.5
    done
    return 1
}

# Avoid --force-recreate: Podman often leaves conmon on HOST_PORT → bind race.
free_host_port "${HOST_PORT}"

echo "Waiting for port ${HOST_PORT} to free..."
if ! wait_port_free "${HOST_PORT}" 40; then
    # One more aggressive pass
    free_host_port "${HOST_PORT}"
    if ! wait_port_free "${HOST_PORT}" 20; then
        echo "error: 127.0.0.1:${HOST_PORT} still in use after cleanup" >&2
        echo "  ss -tlnp | grep ${HOST_PORT}" >&2
        ss -tlnp 2>/dev/null | grep -E "[:.]${HOST_PORT}[[:space:]]" >&2 || true
        echo "  podman ps -a" >&2
        podman ps -a >&2 || true
        exit 1
    fi
fi

echo "Starting site-server (Surreal left running)..."
"${COMPOSE[@]}" --env-file "$SECRETS" up -d site-server

echo
echo "Updated. Health: curl -sS http://127.0.0.1:${HOST_PORT}/api/health"
