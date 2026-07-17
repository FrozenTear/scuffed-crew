#!/usr/bin/env bash
# Pull latest git + prebuilt site-server image and recreate this project's site-server.
# Does NOT compile on the VPS (that is what GH Actions + GHCR are for).
#
# Multi-pod hosts: only stop/remove containers from *this* compose project.
# Never kill arbitrary processes or remove other stacks that share the host.
#
# Also hardens data/secrets.env (idempotent appends) so older installs pick up
# new production env contracts after image upgrades without a crash-loop.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS="$ROOT/data/secrets.env"
cd "$ROOT"

if [[ ! -f "$SECRETS" ]]; then
    echo "error: missing $SECRETS — run scripts/install.sh first" >&2
    exit 1
fi

# --- git pull first, then re-exec if this script changed (so new harden logic runs) ---
if [[ -d .git ]]; then
    script_path="$(readlink -f "$0" 2>/dev/null || realpath "$0" 2>/dev/null || echo "$0")"
    before_mtime="$(stat -c %Y "$script_path" 2>/dev/null || stat -f %m "$script_path" 2>/dev/null || echo 0)"
    echo "Git pull..."
    git pull --ff-only
    after_mtime="$(stat -c %Y "$script_path" 2>/dev/null || stat -f %m "$script_path" 2>/dev/null || echo 0)"
    if [[ "$before_mtime" != "$after_mtime" && "${SCUFFED_UPDATE_REEXEC:-}" != "1" ]]; then
        echo "update.sh changed on pull — re-executing with the new script..."
        export SCUFFED_UPDATE_REEXEC=1
        exec bash "$script_path" "$@"
    fi
fi

# Idempotent appends — same contract as scripts/install.sh for existing secrets.
# Never overwrites keys; never regenerates ENCRYPTION_KEY (would brick sealed data).
ensure_secret_key() {
    local key="$1" val="$2"
    if ! grep -q "^${key}=" "$SECRETS" 2>/dev/null; then
        echo "${key}=${val}" >> "$SECRETS"
        echo "Appended ${key} to secrets.env"
    fi
}

ensure_secret_key PRODUCTION 1
ensure_secret_key SURREALDB_AUTH_MODE scoped
ensure_secret_key SURREALDB_APP_USER scuffed_app
if ! grep -q '^SURREALDB_APP_PASSWORD=' "$SECRETS" 2>/dev/null; then
    # Must be distinct from root password in production (do not reuse SURREALDB_PASSWORD).
    if ! command -v openssl >/dev/null 2>&1; then
        echo "error: openssl required to generate SURREALDB_APP_PASSWORD" >&2
        exit 1
    fi
    ensure_secret_key SURREALDB_APP_PASSWORD "$(openssl rand -base64 32 | tr -d '\n')"
fi

# shellcheck disable=SC1090
set -a
# shellcheck source=/dev/null
source "$SECRETS"
set +a

SITE_SERVER_IMAGE="${SITE_SERVER_IMAGE:-ghcr.io/frozentear/scuffed-crew:main}"
export SITE_SERVER_IMAGE
HOST_PORT="${HOST_PORT:-3000}"

# Compose project name → container prefix (e.g. scuffed-crew-site-server-1).
# Override with COMPOSE_PROJECT_NAME in secrets.env if your project is renamed.
PROJECT_NAME="${COMPOSE_PROJECT_NAME:-$(basename "$ROOT")}"

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

# ENCRYPTION_KEY cannot be invented here — sealed OAuth/Nostr/DM data depends on it.
if ! grep -q '^ENCRYPTION_KEY=' "$SECRETS" 2>/dev/null \
    || [[ -z "${ENCRYPTION_KEY:-}" ]]; then
    echo "error: ENCRYPTION_KEY is missing or empty in $SECRETS" >&2
    echo "  Remote production requires a stable ENCRYPTION_KEY (install.sh generates it)." >&2
    echo "  Do not invent a new key if the DB already has encrypted rows — restore from backup." >&2
    exit 1
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

# Remove only containers that belong to this compose project + site-server service.
# Never: kill random PIDs, rm containers that only "publish HOST_PORT", or touch other pods.
remove_our_site_server() {
    echo "Stopping this project's site-server (${PROJECT_NAME})..."
    # Scope compose to this directory + project name when supported
    if "${COMPOSE[@]}" version 2>/dev/null | grep -qi project; then
        COMPOSE_PROJECT_NAME="${PROJECT_NAME}" "${COMPOSE[@]}" --env-file "$SECRETS" stop site-server 2>/dev/null || true
        COMPOSE_PROJECT_NAME="${PROJECT_NAME}" "${COMPOSE[@]}" --env-file "$SECRETS" rm -f site-server 2>/dev/null || true
    else
        "${COMPOSE[@]}" --env-file "$SECRETS" stop site-server 2>/dev/null || true
        "${COMPOSE[@]}" --env-file "$SECRETS" rm -f site-server 2>/dev/null || true
    fi

    # Name match for this project only. Compose naming varies:
    #   hyphen:  scuffed-crew-site-server-1   (compose v2 / podman)
    #   underscore: scuffed-crew_site-server_1 (classic docker-compose)
    local cid name
    while read -r cid name; do
        [[ -n "${cid}" ]] || continue
        case "${name}" in
            "${PROJECT_NAME}"-site-server|"${PROJECT_NAME}"-site-server-*|\
            "${PROJECT_NAME}"_site-server|"${PROJECT_NAME}"_site-server_*)
                echo "Removing project container ${name} (${cid})..."
                podman rm -f "${cid}" 2>/dev/null || true
                ;;
        esac
    done < <(podman ps -a --format '{{.ID}} {{.Names}}' 2>/dev/null || true)
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

describe_port_holders() {
    local port="$1"
    echo "Port ${port} listeners:" >&2
    ss -tlnp 2>/dev/null | grep -E "[:.]${port}[[:space:]]" >&2 || true
    echo "Containers that might be related:" >&2
    podman ps -a --format 'table {{.ID}}\t{{.Names}}\t{{.Status}}\t{{.Ports}}' 2>/dev/null | grep -E "site-server|${PROJECT_NAME}|:${port}" >&2 || true
    echo "All running containers (for multi-pod diagnosis):" >&2
    podman ps --format 'table {{.ID}}\t{{.Names}}\t{{.Ports}}' 2>/dev/null >&2 || true
}

# Avoid --force-recreate: races with the previous container still holding HOST_PORT.
remove_our_site_server

echo "Waiting for port ${HOST_PORT} to free (this project only)..."
if ! wait_port_free "${HOST_PORT}" 40; then
    echo "error: 127.0.0.1:${HOST_PORT} still in use after removing *this* project's site-server." >&2
    echo "This host runs multiple pods — the script will NOT kill other services." >&2
    echo >&2
    describe_port_holders "${HOST_PORT}"
    echo >&2
    echo "Pick one:" >&2
    echo "  1) If the listener is an orphan leftover of THIS stack only:" >&2
    echo "       podman ps -a | grep ${PROJECT_NAME}" >&2
    echo "       podman rm -f <that-container-id>" >&2
    echo "       # If only orphan conmon remains and you are sure it is this stack:" >&2
    echo "       #   kill <conmon-pid>   # from: ss -tlnp | grep ${HOST_PORT}" >&2
    echo "  2) If another app owns ${HOST_PORT}, give scuffed-crew a free port:" >&2
    echo "       # edit data/secrets.env → HOST_PORT=<free>  (and Caddy/proxy)" >&2
    echo "       ./scripts/update.sh" >&2
    exit 1
fi

echo "Starting site-server for project ${PROJECT_NAME} (other pods left alone)..."
if COMPOSE_PROJECT_NAME="${PROJECT_NAME}" "${COMPOSE[@]}" --env-file "$SECRETS" up -d site-server 2>/dev/null; then
    :
else
    "${COMPOSE[@]}" --env-file "$SECRETS" up -d site-server
fi

# Verify the new container actually serves before declaring success — a
# crash-looping boot (bad env contract, failed DB bootstrap) otherwise looks
# identical to a good deploy from this script's output.
our_container_name() {
    podman ps -a --format '{{.Names}}' 2>/dev/null \
        | grep -E "^${PROJECT_NAME}[-_]site-server([-_][0-9]+)?$" | head -n1
}

echo "Waiting for site-server health on 127.0.0.1:${HOST_PORT} (up to 60s)..."
healthy=0
for _ in $(seq 1 30); do
    if curl -sf --max-time 2 "http://127.0.0.1:${HOST_PORT}/api/health" >/dev/null 2>&1; then
        healthy=1
        break
    fi
    sleep 2
done

if [[ "${healthy}" != "1" ]]; then
    cname="$(our_container_name)"
    echo "error: site-server is not healthy after 60s — deploy FAILED." >&2
    echo "Container status:" >&2
    podman ps -a --filter "name=${PROJECT_NAME}" --format 'table {{.Names}}\t{{.Status}}' >&2 || true
    if [[ -n "${cname}" ]]; then
        echo "--- last 40 log lines (${cname}) ---" >&2
        podman logs --tail 40 "${cname}" >&2 2>&1 || true
    fi
    exit 1
fi

echo
echo "Updated & healthy: http://127.0.0.1:${HOST_PORT}/api/health OK"
