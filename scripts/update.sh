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
# MAC key for Nostr login challenge tokens. Server refuses to boot without it
# (no public dev-key fallback in production). Safe to generate on older installs:
# a fresh value only invalidates in-flight, short-TTL challenge tokens.
if ! grep -q '^NOSTR_CHALLENGE_SECRET=' "$SECRETS" 2>/dev/null; then
    if ! command -v openssl >/dev/null 2>&1; then
        echo "error: openssl required to generate NOSTR_CHALLENGE_SECRET" >&2
        exit 1
    fi
    ensure_secret_key NOSTR_CHALLENGE_SECRET "$(openssl rand -base64 32 | tr -d '\n')"
fi

# shellcheck disable=SC1090
set -a
# shellcheck source=/dev/null
source "$SECRETS"
set +a

SITE_SERVER_IMAGE="${SITE_SERVER_IMAGE:-ghcr.io/frozentear/scuffed-crew:main}"
export SITE_SERVER_IMAGE
HOST_PORT="${HOST_PORT:-3000}"
# Older installs may lack ALLOWED_ORIGINS. Blank existing values are left
# alone — the server treats blank as unset and falls back to REDIRECT_BASE_URL.
ensure_secret_key ALLOWED_ORIGINS "${REDIRECT_BASE_URL:-http://127.0.0.1:${HOST_PORT}}"

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

# Contabo / podman-compose name-skew: Compose-v2 hyphen
#   ${PROJECT_NAME}-surrealdb-1
# vs classic underscore
#   ${PROJECT_NAME}_surrealdb_1
# Both attach the SAME volume (${PROJECT_NAME}_surrealdb-data). A day-2
# `up -d site-server` that honors depends_on creates the underscore twin
# (Exit 1); site-server can sit in Created → public 502.
# Never volume rm the Surreal data volume for this. Never pin container_name.
is_our_surreal() {
    local name="$1"
    case "${name}" in
        "${PROJECT_NAME}"-surrealdb|"${PROJECT_NAME}"-surrealdb-*|\
        "${PROJECT_NAME}"_surrealdb|"${PROJECT_NAME}"_surrealdb_*)
            return 0
            ;;
    esac
    return 1
}

# Prints: cid<TAB>name<TAB>status  for this project's Surreal containers.
list_project_surreal() {
    local cid name status
    while IFS=$'\t' read -r cid name status; do
        [[ -n "${cid}" && -n "${name}" ]] || continue
        if is_our_surreal "${name}"; then
            printf '%s\t%s\t%s\n' "${cid}" "${name}" "${status}"
        fi
    done < <(podman ps -a --format '{{.ID}}\t{{.Names}}\t{{.Status}}' 2>/dev/null || true)
}

surreal_is_up() {
    local status="$1"
    [[ "${status}" == Up* ]]
}

# Sets CANONICAL_SURREAL_CID / CANONICAL_SURREAL_NAME from an Up container.
# Prefer hyphen (especially …-surrealdb-1) when both hyphen and underscore exist.
pick_canonical_up_surreal() {
    CANONICAL_SURREAL_CID=""
    CANONICAL_SURREAL_NAME=""
    local cid name status
    local h1_cid="" h1_name=""
    local h_cid="" h_name=""
    local u_cid="" u_name=""

    while IFS=$'\t' read -r cid name status; do
        [[ -n "${cid}" ]] || continue
        if ! surreal_is_up "${status}"; then
            continue
        fi
        case "${name}" in
            "${PROJECT_NAME}"-surrealdb-1)
                h1_cid="${cid}"
                h1_name="${name}"
                ;;
            "${PROJECT_NAME}"-surrealdb|"${PROJECT_NAME}"-surrealdb-*)
                if [[ -z "${h_name}" ]]; then
                    h_cid="${cid}"
                    h_name="${name}"
                fi
                ;;
            *)
                if [[ -z "${u_name}" ]]; then
                    u_cid="${cid}"
                    u_name="${name}"
                fi
                ;;
        esac
    done < <(list_project_surreal)

    if [[ -n "${h1_name}" ]]; then
        CANONICAL_SURREAL_CID="${h1_cid}"
        CANONICAL_SURREAL_NAME="${h1_name}"
        return 0
    fi
    if [[ -n "${h_name}" ]]; then
        CANONICAL_SURREAL_CID="${h_cid}"
        CANONICAL_SURREAL_NAME="${h_name}"
        return 0
    fi
    if [[ -n "${u_name}" ]]; then
        CANONICAL_SURREAL_CID="${u_cid}"
        CANONICAL_SURREAL_NAME="${u_name}"
        return 0
    fi
    return 1
}

# If hyphen and underscore are both Up, stop extras (prefer keeping hyphen)
# before recreating site-server so compose cannot attach the twin.
stop_noncanonical_up_surreal() {
    local canonical="$1"
    local cid name status
    while IFS=$'\t' read -r cid name status; do
        [[ -n "${cid}" ]] || continue
        [[ "${name}" == "${canonical}" ]] && continue
        if surreal_is_up "${status}"; then
            echo "Stopping duplicate Surreal ${name} (${cid}) before site-server recreate (name-skew; same volume)..."
            podman stop "${cid}" 2>/dev/null || true
        fi
    done < <(list_project_surreal)
}

# After the app is healthy: stop + try rm -f extras. If rm is blocked by a
# compose dependency, leave the extra Exited and say so. Never volume rm.
cleanup_extra_surreal() {
    local canonical="$1"
    [[ -n "${canonical}" ]] || return 0
    local cid name status
    while IFS=$'\t' read -r cid name status; do
        [[ -n "${cid}" ]] || continue
        [[ "${name}" == "${canonical}" ]] && continue
        echo "Stopping extra Surreal ${name} (${cid}); canonical is ${canonical}..."
        podman stop "${cid}" 2>/dev/null || true
        if podman rm -f "${cid}" 2>/dev/null; then
            echo "Removed extra Surreal ${name}."
        else
            echo "warning: could not remove extra Surreal ${name} (compose may hold a dependency)."
            echo "  Left Exited. Do NOT volume rm ${PROJECT_NAME}_surrealdb-data for this symptom."
            echo "  Manual: podman stop ${name} && podman rm -f ${name}"
        fi
    done < <(list_project_surreal)
}

# $1 = --no-deps  → do not recreate Surreal (day-2).
# empty           → normal up (fresh install: depends_on may start surrealdb).
start_site_server() {
    if [[ "${1:-}" == "--no-deps" ]]; then
        echo "Starting site-server for project ${PROJECT_NAME} (--no-deps; existing Surreal left running)..."
        if COMPOSE_PROJECT_NAME="${PROJECT_NAME}" "${COMPOSE[@]}" --env-file "$SECRETS" up -d --no-deps site-server 2>/dev/null; then
            return 0
        fi
        "${COMPOSE[@]}" --env-file "$SECRETS" up -d --no-deps site-server
        return
    fi
    echo "Starting site-server for project ${PROJECT_NAME} (other pods left alone)..."
    if COMPOSE_PROJECT_NAME="${PROJECT_NAME}" "${COMPOSE[@]}" --env-file "$SECRETS" up -d site-server 2>/dev/null; then
        return 0
    fi
    "${COMPOSE[@]}" --env-file "$SECRETS" up -d site-server
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

echo "Project Surreal containers:"
surreal_found=0
while IFS=$'\t' read -r _s_cid _s_name _s_status; do
    [[ -n "${_s_name}" ]] || continue
    echo "  ${_s_name}  ${_s_status}  ${_s_cid}"
    surreal_found=1
done < <(list_project_surreal)
if [[ "${surreal_found}" != "1" ]]; then
    echo "  (none)"
fi

CANONICAL_SURREAL_CID=""
CANONICAL_SURREAL_NAME=""
used_no_deps=0
if pick_canonical_up_surreal; then
    echo "Existing Surreal is Up: ${CANONICAL_SURREAL_NAME} (canonical; hyphen preferred when both exist)."
    stop_noncanonical_up_surreal "${CANONICAL_SURREAL_NAME}"
    start_site_server --no-deps
    used_no_deps=1
else
    echo "No project Surreal is Up — starting site-server with dependencies (fresh install)."
    start_site_server
fi

# Verify the new container actually serves before declaring success — a
# crash-looping boot (bad env contract, failed DB bootstrap) otherwise looks
# identical to a good deploy from this script's output.
our_container_name() {
    podman ps -a --format '{{.Names}}' 2>/dev/null \
        | grep -E "^${PROJECT_NAME}[-_]site-server([-_][0-9]+)?$" | head -n1
}

# Day-2 --no-deps: Contabo / podman-compose may leave site-server Created
# (container exists, not started). Start it before the health loop.
if [[ "${used_no_deps}" == "1" ]]; then
    _ss="$(our_container_name)"
    if [[ -n "${_ss}" ]]; then
        _st="$(podman inspect -f '{{.State.Status}}' "${_ss}" 2>/dev/null || true)"
        if [[ "${_st}" == "created" ]]; then
            echo "site-server ${_ss} is Created after --no-deps — starting it..."
            if ! podman start "${_ss}"; then
                if ! COMPOSE_PROJECT_NAME="${PROJECT_NAME}" "${COMPOSE[@]}" --env-file "$SECRETS" start site-server; then
                    "${COMPOSE[@]}" --env-file "$SECRETS" start site-server
                fi
            fi
        fi
    fi
fi

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
    echo "Project Surreal containers (name-skew: hyphen vs underscore twins share one volume):" >&2
    while IFS=$'\t' read -r _s_cid _s_name _s_status; do
        [[ -n "${_s_name}" ]] || continue
        echo "  ${_s_name}  ${_s_status}  ${_s_cid}" >&2
    done < <(list_project_surreal)
    if [[ -n "${cname}" ]]; then
        echo "--- last 40 log lines (${cname}) ---" >&2
        podman logs --tail 40 "${cname}" >&2 2>&1 || true
    fi
    echo "If site-server is Created / public 502 and two Surreal names exist, see docs/deploy.md" >&2
    echo "(duplicate Surreal _ vs -). Do NOT volume rm ${PROJECT_NAME}_surrealdb-data." >&2
    exit 1
fi

# Fresh install may have just created Surreal; re-pick if we had none at start.
if [[ -z "${CANONICAL_SURREAL_NAME}" ]]; then
    pick_canonical_up_surreal || true
fi
if [[ -n "${CANONICAL_SURREAL_NAME}" ]]; then
    cleanup_extra_surreal "${CANONICAL_SURREAL_NAME}"
fi

echo
echo "Updated & healthy: http://127.0.0.1:${HOST_PORT}/api/health OK"
