#!/usr/bin/env bash
# Pull latest git + prebuilt site-server image and recreate this project's site-server.
# Does NOT compile on the VPS (that is what GH Actions + GHCR are for).
#
# Multi-pod hosts: only stop/remove containers from *this* compose project.
# Never kill arbitrary processes or remove other stacks that share the host.
#
# Also hardens data/secrets.env (idempotent appends) so older installs pick up
# new production env contracts after image upgrades without a crash-loop.
#
# Before the running site-server is removed, its image is tagged
# <repo>:rollback. If the new container is not healthy in time, or the update
# fails anywhere else after that removal (port still busy, the database
# container does not start), the previous image is started again and the
# script exits non-zero.
#
#   --backup   Take a SurrealDB export into backups-manual/ first (HTTP
#              /export, mode 600). A failed or empty export, or a body
#              that is not a SurrealQL export, stops the update with the
#              site still running.
#   --no-deps  Accepted and ignored. The script already starts site-server
#              with --no-deps when a project Surreal is Up.
set -euo pipefail

# "$@" is left untouched: the re-exec after git pull passes it on.
DO_BACKUP=0
for _arg in "$@"; do
    case "${_arg}" in
        --backup) DO_BACKUP=1 ;;
        --no-deps) ;;
        *)
            echo "error: unknown argument: ${_arg}" >&2
            echo "usage: $0 [--backup]" >&2
            exit 2
            ;;
    esac
done

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
# alone in the file. compose-env.sh fills a blank from REDIRECT_BASE_URL
# in the shell before compose runs (podman-compose 1.5 cannot nest defaults).
ensure_secret_key ALLOWED_ORIGINS "${REDIRECT_BASE_URL:-http://127.0.0.1:${HOST_PORT}}"
# shellcheck source=lib/compose-env.sh
source "$ROOT/scripts/lib/compose-env.sh"
export_compose_fallbacks

# Compose project name → container prefix (e.g. scuffed-crew-site-server-1).
# Override with COMPOSE_PROJECT_NAME in secrets.env if your project is renamed.
PROJECT_NAME="${COMPOSE_PROJECT_NAME:-$(basename "$ROOT")}"

if ! command -v podman >/dev/null 2>&1; then
    echo "error: podman is required" >&2
    exit 1
fi

# Seconds one podman or compose stop, rm or start may take (default 60).
# timeout(1) then sends SIGTERM, and SIGKILL PODMAN_KILL_SECS later, so a
# hung podman cannot keep update.sh waiting forever. That matters most in
# the rollback, which ignores INT/TERM/HUP. A call that times out fails.
PODMAN_SECS="${SCUFFED_UPDATE_PODMAN_SECS:-60}"
if [[ ! "${PODMAN_SECS}" =~ ^[1-9][0-9]*$ ]]; then
    echo "error: SCUFFED_UPDATE_PODMAN_SECS must be a whole number of seconds above 0, got '${PODMAN_SECS}'." >&2
    exit 1
fi
# At most 3600s (1 hour). Longer is no real bound, and a huge value would
# wrap in bash arithmetic, so the length check runs before any comparison.
if ((${#PODMAN_SECS} > 4)) || ((PODMAN_SECS > 3600)); then
    echo "error: SCUFFED_UPDATE_PODMAN_SECS must be at most 3600 seconds (1 hour), got '${PODMAN_SECS}'." >&2
    exit 1
fi
PODMAN_KILL_SECS=5
if ((PODMAN_SECS < PODMAN_KILL_SECS)); then
    PODMAN_KILL_SECS="${PODMAN_SECS}"
fi
# Container stop grace handed to compose stop -t (podman's default 10s),
# kept under the outer bound so podman kills a slow container first.
PODMAN_STOP_SECS=10
if ((PODMAN_SECS <= 2 * PODMAN_STOP_SECS)); then
    PODMAN_STOP_SECS=$((PODMAN_SECS / 2))
fi
if ! command -v timeout >/dev/null 2>&1; then
    echo "error: timeout (coreutils) is required to bound podman calls; install coreutils." >&2
    exit 1
fi

# Set by rollback_on_exit.
ROLLING_BACK=0

# podman_bounded [--quiet] [--rollback-only] <label> <command...>
# Runs the command under timeout(1). --quiet sends the command's stderr to
# /dev/null; the timeout line always goes to stderr. --rollback-only bounds
# the call only once the rollback has started and runs it as it is before
# that. Returns the command's status, or 124 (SIGTERM) / 137 (SIGKILL) when
# it timed out.
# Before the rollback, timeout runs with --foreground: podman stays in the
# terminal's process group, so Ctrl-C still stops it at once and the INT
# trap starts the rollback. In the rollback, which ignores INT/TERM/HUP,
# timeout puts podman in a process group of its own, out of reach of
# Ctrl-C, and kills that whole group when the bound runs out.
podman_bounded() {
    local quiet=0 rollback_only=0 label rc=0
    local bound=(timeout --kill-after="${PODMAN_KILL_SECS}" "${PODMAN_SECS}")
    if [[ "${ROLLING_BACK}" != "1" ]]; then
        bound=(timeout --foreground --kill-after="${PODMAN_KILL_SECS}" "${PODMAN_SECS}")
    fi
    while [[ "${1:-}" == --* ]]; do
        case "$1" in
            --quiet) quiet=1 ;;
            --rollback-only) rollback_only=1 ;;
        esac
        shift
    done
    label="$1"
    shift
    if [[ "${rollback_only}" == "1" && "${ROLLING_BACK}" != "1" ]]; then
        bound=()
    fi
    if [[ "${quiet}" == "1" ]]; then
        "${bound[@]}" "$@" 2>/dev/null || rc=$?
    else
        "${bound[@]}" "$@" || rc=$?
    fi
    if [[ "${#bound[@]}" -gt 0 ]] && is_timeout_rc "${rc}"; then
        echo "error: ${label} timed out after ${PODMAN_SECS}s (SCUFFED_UPDATE_PODMAN_SECS), treated as failed." >&2
    fi
    return "${rc}"
}

is_timeout_rc() {
    [[ "$1" == "124" || "$1" == "137" ]]
}

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
    # Each call is bounded. A stop that times out still goes on to rm -f and
    # the leftover sweep below, so the container is removed either way.
    # stop -t works for both podman compose (Compose v2) and podman-compose.
    # Scope compose to this directory + project name when supported
    if podman_bounded --quiet "${COMPOSE[*]} version" "${COMPOSE[@]}" version | grep -qi project; then
        COMPOSE_PROJECT_NAME="${PROJECT_NAME}" podman_bounded --quiet "${COMPOSE[*]} stop site-server" \
            "${COMPOSE[@]}" --env-file "$SECRETS" stop -t "${PODMAN_STOP_SECS}" site-server || true
        COMPOSE_PROJECT_NAME="${PROJECT_NAME}" podman_bounded --quiet "${COMPOSE[*]} rm -f site-server" \
            "${COMPOSE[@]}" --env-file "$SECRETS" rm -f site-server || true
    else
        podman_bounded --quiet "${COMPOSE[*]} stop site-server" \
            "${COMPOSE[@]}" --env-file "$SECRETS" stop -t "${PODMAN_STOP_SECS}" site-server || true
        podman_bounded --quiet "${COMPOSE[*]} rm -f site-server" \
            "${COMPOSE[@]}" --env-file "$SECRETS" rm -f site-server || true
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
                podman_bounded --quiet "podman rm -f ${cid}" podman rm -f "${cid}" || true
                ;;
        esac
    done < <(podman_bounded --quiet "podman ps -a" podman ps -a --format '{{.ID}} {{.Names}}' || true)
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
# Never volume rm the Surreal data volume for this. Never pin container_name
# or the top-level compose project name. A hyphen container_name collides
# with the Contabo hyphen DB (podman create fails, no second writer) but
# on a host whose only DB is the underscore name it creates a new hyphen
# container on the same volume. x-podman.name_separator_compat aligns
# container names and also renames networks (scuffed-crew-internal vs the
# existing scuffed-crew_internal), so strfry would miss the running DB.
# Volume names already match across both tools; that shared name is why
# the twin is dangerous. Relay bring-up is scripts/relay-up.sh (--no-deps).
# strfry has no depends_on, so `up strfry` does not create this service.
# is_our_surreal, list_project_surreal, surreal_is_up and the guard that
# keeps this script from creating or recreating the database container.
# shellcheck source=lib/surreal-guard.sh
source "$ROOT/scripts/lib/surreal-guard.sh"

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
# Bounded in the rollback only. On the update path a fresh install may pull
# the database image here, which can take longer than the bound.
start_site_server() {
    local deps=() label rc=0
    if [[ "${1:-}" == "--no-deps" ]]; then
        deps=(--no-deps)
        echo "Starting site-server for project ${PROJECT_NAME} (--no-deps; existing Surreal left running)..."
    else
        echo "Starting site-server for project ${PROJECT_NAME} (other pods left alone)..."
    fi
    label="${COMPOSE[*]} up -d ${deps[*]:+--no-deps }site-server"
    COMPOSE_PROJECT_NAME="${PROJECT_NAME}" podman_bounded --quiet --rollback-only "${label}" \
        "${COMPOSE[@]}" --env-file "$SECRETS" up -d "${deps[@]}" site-server || rc=$?
    [[ "${rc}" != "0" ]] || return 0
    # A bounded call that timed out is not tried again: that doubles the wait.
    if [[ "${ROLLING_BACK}" == "1" ]] && is_timeout_rc "${rc}"; then
        return "${rc}"
    fi
    podman_bounded --rollback-only "${label}" \
        "${COMPOSE[@]}" --env-file "$SECRETS" up -d "${deps[@]}" site-server
}

# Bounded in the rollback only. Before the removal an empty answer means
# "no container, nothing to roll back to", so a slow podman must not get
# cut short into that answer.
our_container_name() {
    podman_bounded --quiet --rollback-only "podman ps -a" podman ps -a --format '{{.Names}}' \
        | grep -E "^${PROJECT_NAME}[-_]site-server([-_][0-9]+)?$" | head -n1
}

# Day-2 --no-deps: Contabo / podman-compose may leave site-server Created
# (container exists, not started). Start it before the health loop. A start
# that fails here is left to the health loop to report.
start_created_site_server() {
    local _ss _st rc=0
    _ss="$(our_container_name || true)"
    [[ -n "${_ss}" ]] || return 0
    _st="$(podman_bounded --quiet "podman inspect ${_ss}" podman inspect -f '{{.State.Status}}' "${_ss}" || true)"
    [[ "${_st}" == "created" ]] || return 0
    echo "site-server ${_ss} is Created after --no-deps, starting it..."
    podman_bounded "podman start ${_ss}" podman start "${_ss}" || rc=$?
    # A start that timed out is not tried again through compose: that would
    # double the wait, and the health loop reports the outcome either way.
    if [[ "${rc}" == "0" ]] || is_timeout_rc "${rc}"; then
        return 0
    fi
    if ! COMPOSE_PROJECT_NAME="${PROJECT_NAME}" podman_bounded "${COMPOSE[*]} start site-server" \
        "${COMPOSE[@]}" --env-file "$SECRETS" start site-server; then
        podman_bounded "${COMPOSE[*]} start site-server" \
            "${COMPOSE[@]}" --env-file "$SECRETS" start site-server || true
    fi
}

HEALTH_URL="http://127.0.0.1:${HOST_PORT}/api/health"
# Health wait in seconds (default 60). A site that is not healthy by then is
# rolled back, so raise it for a first boot with a long startup migration.
HEALTH_SECS="${SCUFFED_UPDATE_HEALTH_SECS:-60}"

wait_site_health() {
    local _
    for _ in $(seq 1 $((HEALTH_SECS / 2))); do
        if curl -sf --max-time 2 "${HEALTH_URL}" >/dev/null 2>&1; then
            return 0
        fi
        sleep 2
    done
    return 1
}

# git_sha from /api/health, or "unknown" (site down, no sha in the body).
live_git_sha() {
    local body sha
    body="$(curl -s --max-time 5 "${HEALTH_URL}" 2>/dev/null || true)"
    sha="$(sed -n 's/.*"git_sha"[[:space:]]*:[[:space:]]*"\([^"]*\)".*/\1/p' <<<"${body}" | head -n1)"
    echo "${sha:-unknown}"
}

# Tag the image of this project's running (or last) site-server container as
# <repo>:rollback before that container is removed. Sets ROLLBACK_TAG,
# ROLLBACK_IMAGE_ID and PREV_GIT_SHA. No container (fresh install) leaves
# ROLLBACK_TAG empty: nothing to roll back to.
ROLLBACK_TAG=""
ROLLBACK_IMAGE_ID=""
PREV_GIT_SHA="unknown"
record_rollback_target() {
    local cname img_name repo kept_id
    cname="$(our_container_name || true)"
    if [[ -z "${cname}" ]]; then
        echo "No existing site-server container: no rollback target recorded."
        return 0
    fi
    PREV_GIT_SHA="$(live_git_sha)"
    ROLLBACK_IMAGE_ID="$(podman inspect -f '{{.Image}}' "${cname}" 2>/dev/null || true)"
    if [[ -z "${ROLLBACK_IMAGE_ID}" ]]; then
        echo "warning: could not read the image of ${cname}: no rollback target recorded."
        return 0
    fi
    img_name="$(podman inspect -f '{{.ImageName}}' "${cname}" 2>/dev/null || true)"
    # A container created from a bare image ID has no repo name.
    if [[ -z "${img_name}" || "${img_name}" == sha256:* || "${img_name}" != */* ]]; then
        img_name="${SITE_SERVER_IMAGE}"
    fi
    repo="${img_name%%@*}"
    if [[ "${repo##*/}" == *:* ]]; then
        repo="${repo%:*}"
    fi
    ROLLBACK_TAG="${repo}:rollback"
    # A site that does not answer health may be the leftover of an earlier
    # failed update. Do not let it replace a rollback image that is already
    # tagged. (A healthy build without a sha also reports "unknown".)
    if ! curl -sf --max-time 5 "${HEALTH_URL}" >/dev/null 2>&1; then
        kept_id="$(podman image inspect "${ROLLBACK_TAG}" --format '{{.Id}}' 2>/dev/null || true)"
        if [[ -n "${kept_id}" ]]; then
            ROLLBACK_IMAGE_ID="${kept_id}"
            echo "Live site does not answer health. Keeping the existing rollback image:"
            echo "  ${ROLLBACK_TAG} (image ${ROLLBACK_IMAGE_ID})"
            return 0
        fi
    fi
    if ! podman tag "${ROLLBACK_IMAGE_ID}" "${ROLLBACK_TAG}"; then
        echo "error: could not tag ${ROLLBACK_IMAGE_ID} as ${ROLLBACK_TAG}." >&2
        echo "update.sh stopped before removing the running site-server." >&2
        exit 1
    fi
    echo "Live git_sha before update: ${PREV_GIT_SHA}"
    echo "Rollback image: ${ROLLBACK_TAG} (image ${ROLLBACK_IMAGE_ID})"
}

# What data/secrets.env has to say for the rolled-back build to stay.
rollback_pin_note() {
    local repo="${ROLLBACK_TAG%:rollback}"
    echo "  data/secrets.env still names the failed image: SITE_SERVER_IMAGE=${SITE_SERVER_IMAGE}" >&2
    echo "  The next compose up of site-server, or ./scripts/update.sh, starts that pin again." >&2
    echo "  To keep the previous build (git_sha ${PREV_GIT_SHA}, image ${ROLLBACK_IMAGE_ID}), set in data/secrets.env:" >&2
    echo "    SITE_SERVER_IMAGE=${ROLLBACK_TAG}" >&2
    echo "  ${ROLLBACK_TAG} is a local tag that update.sh cannot pull. Before the next" >&2
    if [[ "${PREV_GIT_SHA}" == "unknown" ]]; then
        echo "  ./scripts/update.sh set a pullable pin: the fixed image, or the ${repo}:sha-<short> tag of the previous build." >&2
    else
        echo "  ./scripts/update.sh set a pullable pin: the fixed image, or ${repo}:sha-${PREV_GIT_SHA:0:7} (if that commit was published)." >&2
    fi
}

# EXIT trap, installed once the rollback target is recorded, before this
# project's site-server is removed, and cleared when the new one is
# healthy. Every failing exit in between (an interrupt during the removal, port
# still busy, Surreal not started, compose up, health, an interrupt, a
# command failing under set -e) lands here, so none of them leaves the site removed: the
# previous image is started again. --no-deps unless this run was allowed to
# create the database (with-deps) and none is Up, so Surreal is never
# recreated here either. Always exits non-zero.
rollback_on_exit() {
    local rc=$?
    # Off before the rollback itself: a failure in here must not recurse.
    trap - EXIT
    # A second interrupt must not stop the restore halfway.
    trap '' INT TERM HUP
    set +e
    [[ "${rc}" != "0" ]] || return 0
    # Every podman stop, rm and start from here on is bounded (podman_bounded).
    ROLLING_BACK=1

    if [[ -z "${ROLLBACK_TAG}" ]]; then
        echo "No rollback target was recorded (no previous site-server): nothing to roll back to." >&2
        exit 1
    fi

    echo >&2
    echo "Rolling back to ${ROLLBACK_TAG} (image ${ROLLBACK_IMAGE_ID}, previous git_sha ${PREV_GIT_SHA})..." >&2
    local rollback_ok=0 rollback_missing=0 rollback_deps="--no-deps" inspect_rc=0
    podman_bounded --quiet "podman image inspect ${ROLLBACK_TAG}" \
        podman image inspect "${ROLLBACK_TAG}" --format '{{.Id}}' >/dev/null || inspect_rc=$?
    if is_timeout_rc "${inspect_rc}"; then
        echo "error: podman did not answer, the previous image was not started." >&2
    elif [[ "${inspect_rc}" != "0" ]]; then
        echo "error: rollback image ${ROLLBACK_TAG} is missing." >&2
        rollback_missing=1
    else
        remove_our_site_server >&2
        wait_port_free "${HOST_PORT}" 40 || true
        if [[ "${SURREAL_GUARD_ACTION:-}" == "with-deps" ]] && ! pick_canonical_up_surreal; then
            rollback_deps=""
        fi
        # Shell value wins over --env-file, so compose starts the previous image.
        if SITE_SERVER_IMAGE="${ROLLBACK_TAG}" start_site_server ${rollback_deps:+"${rollback_deps}"} >&2; then
            if [[ -n "${rollback_deps}" ]]; then
                start_created_site_server >&2
            fi
            echo "Waiting for the previous site-server to answer health (up to ${HEALTH_SECS}s)..." >&2
            if wait_site_health; then
                rollback_ok=1
            fi
        else
            echo "error: compose could not start ${ROLLBACK_TAG}." >&2
        fi
    fi

    echo >&2
    if [[ "${rollback_ok}" == "1" ]]; then
        echo "error: update FAILED. Rolled back: the previous image ${ROLLBACK_TAG} is running and healthy." >&2
        echo "  Previous git_sha: ${PREV_GIT_SHA}. Live git_sha now: $(live_git_sha)." >&2
        rollback_pin_note
        echo "  The rollback is the image only. Database changes made by the failed boot (migrations) are not undone." >&2
    else
        echo "error: update FAILED and the rollback FAILED. The site is DOWN." >&2
        echo "  Previous image: ${ROLLBACK_TAG} (image ${ROLLBACK_IMAGE_ID}, git_sha ${PREV_GIT_SHA})." >&2
        if [[ "${rollback_missing}" != "1" ]]; then
            echo "  Manual: SITE_SERVER_IMAGE=${ROLLBACK_TAG} ${COMPOSE[*]} --env-file data/secrets.env up -d --no-deps site-server" >&2
            rollback_pin_note
        fi
    fi
    exit 1
}

# --backup: logical export over HTTP from the running project Surreal. The
# `surreal export` CLI fails on 3.2.x, the image has no curl or shell, and
# compose does not publish 8000, so the request goes to the container IP on
# the compose network. Never recreates or restarts Surreal. Separate from
# scripts/backup.sh (restic). Any failure exits before a container is touched.
# An export starts with a comment or a statement and has OPTION IMPORT, a
# DEFINE or an INSERT at the start of a line. An HTML or JSON error body
# has neither.
looks_like_surql_export() {
    local first
    first="$(grep -a -m1 -v '^[[:space:]]*$' "$1" 2>/dev/null || true)"
    [[ "${first}" == --* || "${first}" == OPTION* || "${first}" == DEFINE* ]] || return 1
    grep -a -qE '^(OPTION IMPORT|DEFINE |INSERT )' "$1"
}

backup_surreal_export() {
    local ip dir file size user pw http
    if ! pick_canonical_up_surreal; then
        echo "error: --backup needs a running project Surreal, none is Up." >&2
        return 1
    fi
    ip="$(podman inspect -f '{{range .NetworkSettings.Networks}}{{.IPAddress}} {{end}}' "${CANONICAL_SURREAL_CID}" 2>/dev/null || true)"
    read -r ip _ <<<"${ip}" || true
    if [[ -z "${ip}" ]]; then
        echo "error: --backup could not read the IP of ${CANONICAL_SURREAL_NAME}." >&2
        return 1
    fi
    dir="$ROOT/backups-manual"
    mkdir -p "${dir}"
    chmod 700 "${dir}"
    file="${dir}/surreal-export-$(date -u +%Y%m%dT%H%M%SZ).surql"
    user="${SURREALDB_USER:-root}"
    # Credentials go to curl on stdin, not argv (ps) and not the terminal.
    pw="${SURREALDB_PASSWORD:-}"
    pw="${pw//\\/\\\\}"
    pw="${pw//\"/\\\"}"
    echo "Backup: exporting ${SURREALDB_NS:-scuffed_crew}/${SURREALDB_DB:-main} from ${CANONICAL_SURREAL_NAME} (${ip}) ..."
    # -f fails on HTTP 400 and up. The status is checked as well, so any
    # answer that is not 2xx fails even when a body was written.
    if ! http="$(umask 077 && printf 'user = "%s:%s"\n' "${user}" "${pw}" \
        | curl -sS -f -K - --max-time "${SCUFFED_BACKUP_MAX_SECS:-600}" \
            -H "surreal-ns: ${SURREALDB_NS:-scuffed_crew}" \
            -H "surreal-db: ${SURREALDB_DB:-main}" \
            -H "Accept: application/octet-stream" \
            -w '%{http_code}' \
            -o "${file}" "http://${ip}:8000/export")" \
        || [[ "${http}" != 2[0-9][0-9] ]]; then
        rm -f "${file}"
        echo "error: --backup export request failed (http://${ip}:8000/export, HTTP status ${http:-none})." >&2
        echo "  The container IP is only reachable from the host with rootful podman." >&2
        return 1
    fi
    if [[ ! -s "${file}" ]]; then
        rm -f "${file}"
        echo "error: --backup export is missing or empty." >&2
        return 1
    fi
    if ! looks_like_surql_export "${file}"; then
        size="$(wc -c <"${file}" | tr -d '[:space:]')"
        rm -f "${file}"
        echo "error: --backup export (${size} bytes) is not a SurrealQL export (an HTML or JSON error body?)." >&2
        return 1
    fi
    chmod 600 "${file}"
    size="$(wc -c <"${file}" | tr -d '[:space:]')"
    echo "Backup: ${file} (${size} bytes, mode 600)"
}

# Discover the compose-network gateway while site-server still exists, and
# write TRUSTED_PROXIES if it is missing. Re-source so an empty shell export
# does not override the value compose reads from --env-file.
echo "Checking TRUSTED_PROXIES (compose-network gateway)..."
bash "$ROOT/scripts/ensure-trusted-proxies.sh"
set -a
# shellcheck disable=SC1090
source "$SECRETS"
set +a
# Re-source can clear a blank ALLOWED_ORIGINS / app password. Fill again
# before compose. Shell values win over --env-file in podman-compose 1.5.
export_compose_fallbacks

# Decide how to start site-server before taking the old one down, so a
# refusal leaves the site running. This script never creates or recreates
# the SurrealDB container while one exists or its volume holds data, and it
# never changes the database image. See scripts/lib/surreal-guard.sh.
if ! surreal_guard_plan; then
    echo "update.sh stopped before touching any container." >&2
    exit 1
fi

if [[ "${DO_BACKUP}" == "1" ]]; then
    if ! backup_surreal_export; then
        echo "update.sh stopped before touching any container. The site is still running." >&2
        exit 1
    fi
else
    echo "warning: no --backup, this update takes no database export."
    echo "  If the new build migrates the database and then fails to boot, the rollback"
    echo "  leaves the old image on an already-migrated database."
fi

record_rollback_target
# From here on a failing exit starts the previous image again. An interrupt
# becomes a failing exit, so it does too. Armed before the removal (an
# interrupt during it must not leave the site down) and not before the
# rollback tag exists.
trap rollback_on_exit EXIT
trap 'exit 130' INT TERM HUP

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
start_failed=0
# A stopped project DB container is started as it is (same image), never
# recreated through depends_on. If it does not start, the new site-server
# is not started either.
if ! surreal_guard_apply; then
    echo "error: ${SURREAL_GUARD_NAME} did not start. Not starting the new site-server." >&2
    exit 1
fi
if pick_canonical_up_surreal; then
    echo "Existing Surreal is Up: ${CANONICAL_SURREAL_NAME} (canonical; hyphen preferred when both exist)."
    stop_noncanonical_up_surreal "${CANONICAL_SURREAL_NAME}"
    start_site_server --no-deps || start_failed=1
    used_no_deps=1
elif [[ "${SURREAL_GUARD_ACTION}" == "with-deps" ]]; then
    if surreal_db_volume_exists; then
        echo "SCUFFED_ALLOW_DB_CREATE=1: no project Surreal container; compose creates it on $(compose_surreal_image) over the existing ${PROJECT_NAME}_surrealdb-data volume."
    else
        echo "No project Surreal container or volume — starting site-server with dependencies (fresh install)."
    fi
    start_site_server || start_failed=1
else
    echo "error: no project Surreal container is Up after starting ${SURREAL_GUARD_NAME:-it}." >&2
    echo "  Not starting site-server with dependencies: that would recreate the database." >&2
    echo "  Check: podman ps -a | grep surrealdb; podman logs <name>" >&2
    exit 1
fi

# Verify the new container actually serves before declaring success. A
# crash-looping boot (bad env contract, failed DB bootstrap) otherwise looks
# identical to a good deploy from this script's output.
healthy=0
if [[ "${start_failed}" == "1" ]]; then
    echo "error: compose could not start site-server." >&2
else
    if [[ "${used_no_deps}" == "1" ]]; then
        start_created_site_server
    fi
    echo "Waiting for site-server health on 127.0.0.1:${HOST_PORT} (up to ${HEALTH_SECS}s)..."
    if wait_site_health; then
        healthy=1
    fi
fi

if [[ "${healthy}" != "1" ]]; then
    cname="$(our_container_name || true)"
    if [[ "${start_failed}" != "1" ]]; then
        echo "error: site-server is not healthy after ${HEALTH_SECS}s, deploy FAILED." >&2
    fi
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

    # rollback_on_exit starts the previous image.
    exit 1
fi
trap - EXIT INT TERM HUP

# Fresh install may have just created Surreal; re-pick if we had none at start.
if [[ -z "${CANONICAL_SURREAL_NAME}" ]]; then
    pick_canonical_up_surreal || true
fi
if [[ -n "${CANONICAL_SURREAL_NAME}" ]]; then
    cleanup_extra_surreal "${CANONICAL_SURREAL_NAME}"
fi

echo
echo "Updated & healthy: http://127.0.0.1:${HOST_PORT}/api/health OK"
echo "Live git_sha: $(live_git_sha)"
if [[ -n "${ROLLBACK_TAG}" ]]; then
    echo "Rollback image: ${ROLLBACK_TAG} (previous git_sha ${PREV_GIT_SHA})"
fi
