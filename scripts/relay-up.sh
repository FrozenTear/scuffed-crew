#!/usr/bin/env bash
# Build and start the optional strfry relay. Never creates or recreates SurrealDB.
#
# podman-compose names containers with underscores (scuffed-crew_surrealdb_1).
# Compose v2 uses hyphens (scuffed-crew-surrealdb-1). Both attach the same
# volume (${PROJECT_NAME}_surrealdb-data). `compose up strfry` without
# --no-deps follows depends_on and can start that underscore twin beside the
# live hyphen database. RocksDB's LOCK is not a safe guard. This script
# always passes --no-deps, and it refuses to run unless a project Surreal
# container is already Up.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS="$ROOT/data/secrets.env"
cd "$ROOT"

if [[ ! -f "$SECRETS" ]]; then
    echo "error: missing $SECRETS. Run scripts/install.sh first." >&2
    exit 1
fi

# shellcheck disable=SC1090
set -a
# shellcheck source=/dev/null
source "$SECRETS"
set +a
# shellcheck source=lib/compose-env.sh
source "$ROOT/scripts/lib/compose-env.sh"
export_compose_fallbacks

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

# Prefer hyphen, and …-surrealdb-1 over any other hyphen, when several are Up.
# Rank: 3 exact hyphen -1, 2 other hyphen, 1 underscore.
surreal_rank() {
    case "$1" in
        "${PROJECT_NAME}-surrealdb-1") echo 3 ;;
        "${PROJECT_NAME}-surrealdb"|"${PROJECT_NAME}-surrealdb-"*) echo 2 ;;
        *) echo 1 ;;
    esac
}

canonical=""
canonical_rank=0
found=0
while IFS=$'\t' read -r cid name status; do
    [[ -n "${name}" ]] || continue
    if ! is_our_surreal "${name}"; then
        continue
    fi
    echo "Surreal ${name}  ${status}  ${cid}"
    found=1
    if [[ "${status}" == Up* ]]; then
        rank="$(surreal_rank "${name}")"
        if [[ "${rank}" -gt "${canonical_rank}" ]]; then
            canonical="${name}"
            canonical_rank="${rank}"
        fi
    fi
done < <(podman ps -a --format '{{.ID}}\t{{.Names}}\t{{.Status}}' 2>/dev/null || true)

if [[ -z "${canonical}" ]]; then
    echo "error: no project Surreal container is Up." >&2
    echo "  Looked for ${PROJECT_NAME}-surrealdb-1 and ${PROJECT_NAME}_surrealdb_1." >&2
    if [[ "${found}" != "1" ]]; then
        echo "  None of those containers exist." >&2
    fi
    echo "  Start the existing stack with ./scripts/update.sh." >&2
    echo "  This script will not create a database. A compose up that starts" >&2
    echo "  surrealdb (site-server without --no-deps, or up with no service name)" >&2
    echo "  can start ${PROJECT_NAME}_surrealdb_1 on the same volume as" >&2
    echo "  ${PROJECT_NAME}-surrealdb-1. Do not volume rm ${PROJECT_NAME}_surrealdb-data." >&2
    exit 1
fi

echo "Building strfry with --no-deps (leaving ${canonical} running; not creating Surreal)..."
# Exit on failure. A second up without COMPOSE_PROJECT_NAME only builds again,
# under whatever project name the shell already has.
if ! COMPOSE_PROJECT_NAME="${PROJECT_NAME}" "${COMPOSE[@]}" --env-file "$SECRETS" --profile relay up --build -d --no-deps strfry; then
    echo "error: strfry up failed for COMPOSE_PROJECT_NAME=${PROJECT_NAME}." >&2
    echo "  Command: ${COMPOSE[*]} --env-file data/secrets.env --profile relay up --build -d --no-deps strfry" >&2
    exit 1
fi
