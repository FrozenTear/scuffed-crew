# shellcheck shell=bash
# Keep compose from creating or recreating this project's SurrealDB container
# as a side effect of starting site-server.
#
# `compose up site-server` without --no-deps follows depends_on. When the
# surrealdb service's image in compose.yml differs from the running
# container's image, or when no surrealdb container is Up, compose creates
# the database container on the compose.yml image over the live volume. A
# newer SurrealDB rewrites the datastore so the older one cannot open it
# (3.0.5 after 3.2.3: "Invalid revision `2` for type `Node`"). That is a
# database upgrade with no backup and no rollback, so it must only happen by
# following docs/deploy.md "Upgrading the SurrealDB server".
#
# Callers set PROJECT_NAME and ROOT, then:
#   surreal_guard_plan   decide; no side effects; returns 1 when it refuses
#   surreal_guard_apply  start a stopped DB container if the plan says so
# and then start site-server with --no-deps unless SURREAL_GUARD_ACTION is
# "with-deps" (no DB container and no DB volume: a fresh install).
#
# Overrides (environment):
#   SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1  the existing DB container runs another
#       image than compose.yml names; leave it as it is and continue. The
#       database is not changed either way.
#   SCUFFED_ALLOW_DB_CREATE=1  no DB container exists but the data volume
#       does; let compose create the container on the compose.yml image over
#       that volume. Only after a backup, or when that image wrote the data.

SURREAL_GUARD_RUNBOOK='docs/deploy.md, "Upgrading the SurrealDB server"'

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

# Rank for picking one container: Up first, then hyphen -1, other hyphen,
# underscore (same preference as update.sh and relay-up.sh).
_surreal_guard_rank() {
    local name="$1" status="$2" r=0
    case "${name}" in
        "${PROJECT_NAME}-surrealdb-1") r=3 ;;
        "${PROJECT_NAME}-surrealdb"|"${PROJECT_NAME}-surrealdb-"*) r=2 ;;
        *) r=1 ;;
    esac
    if surreal_is_up "${status}"; then
        r=$((r + 10))
    fi
    echo "${r}"
}

# The image compose.yml names for the surrealdb service.
compose_surreal_image() {
    awk '
        /^[^[:space:]#]/ { in_services = ($0 ~ /^services:/); svc = ""; next }
        in_services && /^  [A-Za-z0-9_.-]+:/ { svc = $1; sub(/:$/, "", svc); next }
        svc == "surrealdb" && /^    image:/ { print $2; exit }
    ' "${ROOT}/compose.yml"
}

# docker.io/x/y:tag -> x/y:tag. With a digest the tag is dropped
# (x/y:tag@sha256:d -> x/y@sha256:d): podman reports a container created from
# a tag+digest reference by repository and digest only.
_surreal_guard_norm_image() {
    local i="$1" repo digest
    i="${i#docker.io/}"
    i="${i#library/}"
    if [[ "${i}" == *@* ]]; then
        repo="${i%%@*}"
        digest="${i#*@}"
        if [[ "${repo##*/}" == *:* ]]; then
            repo="${repo%:*}"
        fi
        i="${repo}@${digest}"
    fi
    printf '%s' "${i}"
}

# Same image? Compare image IDs when the compose image is present locally
# (stricter: a re-pushed tag counts as different, as it does for compose).
# Before it is pulled, compare the normalized names.
_surreal_guard_same_image() {
    local cid="$1" want="$2" have_name="$3" have_id want_id
    have_id="$(podman inspect -f '{{.Image}}' "${cid}" 2>/dev/null || true)"
    want_id="$(podman image inspect "${want}" --format '{{.Id}}' 2>/dev/null || true)"
    if [[ -n "${have_id}" && -n "${want_id}" ]]; then
        SURREAL_GUARD_COMPARED="image ID ${have_id:0:12} vs ${want_id:0:12}"
        [[ "${have_id}" == "${want_id}" ]]
        return
    fi
    SURREAL_GUARD_COMPARED="image name (${want} is not pulled yet)"
    [[ "$(_surreal_guard_norm_image "${have_name}")" == "$(_surreal_guard_norm_image "${want}")" ]]
}

surreal_db_volume_exists() {
    podman volume ls --format '{{.Name}}' 2>/dev/null \
        | grep -qxF "${PROJECT_NAME}_surrealdb-data"
}

# Sets SURREAL_GUARD_ACTION to one of:
#   no-deps     a project DB container is Up; start site-server with --no-deps
#   start       one exists but is stopped; surreal_guard_apply starts it,
#               then --no-deps
#   with-deps   no DB container and no DB volume (fresh install)
# and SURREAL_GUARD_CID / SURREAL_GUARD_NAME / SURREAL_GUARD_IMAGE for the
# chosen container. Returns 1 (and prints why) when it refuses.
surreal_guard_plan() {
    SURREAL_GUARD_ACTION=""
    SURREAL_GUARD_CID=""
    SURREAL_GUARD_NAME=""
    SURREAL_GUARD_IMAGE=""
    local want best_rank=0 cid name status rank best_status=""
    want="$(compose_surreal_image)"
    if [[ -z "${want}" ]]; then
        echo "error: could not read the surrealdb image from ${ROOT}/compose.yml" >&2
        return 1
    fi

    while IFS=$'\t' read -r cid name status; do
        [[ -n "${cid}" ]] || continue
        rank="$(_surreal_guard_rank "${name}" "${status}")"
        if [[ "${rank}" -gt "${best_rank}" ]]; then
            best_rank="${rank}"
            SURREAL_GUARD_CID="${cid}"
            SURREAL_GUARD_NAME="${name}"
            best_status="${status}"
        fi
    done < <(list_project_surreal)

    if [[ -z "${SURREAL_GUARD_CID}" ]]; then
        if surreal_db_volume_exists && [[ "${SCUFFED_ALLOW_DB_CREATE:-}" != "1" ]]; then
            echo "error: no ${PROJECT_NAME} SurrealDB container exists, but the volume" >&2
            echo "  ${PROJECT_NAME}_surrealdb-data does. Starting site-server would let compose" >&2
            echo "  create the database on ${want} over that data. If an older" >&2
            echo "  SurrealDB wrote it, that is an upgrade with no backup and no rollback." >&2
            echo "  Upgrade path: ${SURREAL_GUARD_RUNBOOK} (backup first)." >&2
            echo "  If ${want} wrote this data, or you have a backup and mean to" >&2
            echo "  upgrade now, rerun with SCUFFED_ALLOW_DB_CREATE=1." >&2
            return 1
        fi
        SURREAL_GUARD_ACTION="with-deps"
        return 0
    fi

    SURREAL_GUARD_IMAGE="$(podman inspect -f '{{.ImageName}}' "${SURREAL_GUARD_CID}" 2>/dev/null || true)"
    if ! _surreal_guard_same_image "${SURREAL_GUARD_CID}" "${want}" "${SURREAL_GUARD_IMAGE}"; then
        if [[ "${SCUFFED_ALLOW_DB_IMAGE_MISMATCH:-}" != "1" ]]; then
            echo "error: ${SURREAL_GUARD_NAME} runs ${SURREAL_GUARD_IMAGE:-an unknown image}, but compose.yml names ${want}" >&2
            echo "  (compared by ${SURREAL_GUARD_COMPARED})." >&2
            echo "  This script never changes the database image. To upgrade the database," >&2
            echo "  follow ${SURREAL_GUARD_RUNBOOK} (backup first)." >&2
            echo "  To update only the app and leave the database on its current image," >&2
            echo "  rerun with SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1." >&2
            return 1
        fi
        echo "warning: ${SURREAL_GUARD_NAME} stays on ${SURREAL_GUARD_IMAGE:-an unknown image}; compose.yml names ${want}." >&2
        echo "  SCUFFED_ALLOW_DB_IMAGE_MISMATCH=1: the database is left as it is." >&2
    fi

    if surreal_is_up "${best_status}"; then
        SURREAL_GUARD_ACTION="no-deps"
    else
        SURREAL_GUARD_ACTION="start"
    fi
    return 0
}

# Start the chosen container when the plan is "start", and wait until it runs.
surreal_guard_apply() {
    [[ "${SURREAL_GUARD_ACTION}" == "start" ]] || return 0
    echo "Starting existing SurrealDB container ${SURREAL_GUARD_NAME} (not recreating it)..."
    if ! podman start "${SURREAL_GUARD_CID}" >/dev/null; then
        echo "error: podman start ${SURREAL_GUARD_NAME} failed. Check: podman logs ${SURREAL_GUARD_NAME}" >&2
        return 1
    fi
    local i running
    for ((i = 1; i <= ${SURREAL_GUARD_WAIT_TRIES:-30}; i++)); do
        running="$(podman inspect -f '{{.State.Running}}' "${SURREAL_GUARD_CID}" 2>/dev/null || true)"
        if [[ "${running}" == "true" ]]; then
            SURREAL_GUARD_ACTION="no-deps"
            return 0
        fi
        sleep "${SURREAL_GUARD_WAIT_SECS:-1}"
    done
    echo "error: ${SURREAL_GUARD_NAME} did not stay running. Check: podman logs ${SURREAL_GUARD_NAME}" >&2
    return 1
}
