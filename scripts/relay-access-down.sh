#!/usr/bin/env bash
# Remove the relay record login before rolling the site image back.
#
# An older site image runs DEFINE FIELD OVERWRITE without field permissions,
# reopens audit_log, and grants the relay SELECT on the base tables. Those
# statements make the hidden columns readable to a record user again. The
# projection events write relay_member, relay_roster, relay_moderation, and
# relay_team_channel. A write fails if those events stay and the tables are
# gone. This script removes the events, then the projection tables, then the
# access and the principal table. Root and the EDITOR app user bypass table
# permissions and keep working.
#
# The surreal image has no shell. /surreal sql reads SURREAL_USER and
# SURREAL_PASS from the container environment. This script does not pass
# the password on the command line and does not print it.
#
# Usage: ./scripts/relay-access-down.sh --yes
# See docs/deploy.md, "Rolling the site image back".
set -euo pipefail

relay_access_down_sql() {
    printf '%s\n' \
        "REMOVE EVENT IF EXISTS relay_member_write ON TABLE member;" \
        "REMOVE EVENT IF EXISTS relay_member_delete ON TABLE member;" \
        "REMOVE EVENT IF EXISTS relay_roster_write ON TABLE plays_on;" \
        "REMOVE EVENT IF EXISTS relay_roster_delete ON TABLE plays_on;" \
        "REMOVE EVENT IF EXISTS relay_moderation_write ON TABLE moderation_action;" \
        "REMOVE EVENT IF EXISTS relay_moderation_delete ON TABLE moderation_action;" \
        "REMOVE EVENT IF EXISTS relay_team_channel_write ON TABLE team_channel;" \
        "REMOVE EVENT IF EXISTS relay_team_channel_delete ON TABLE team_channel;" \
        "REMOVE TABLE IF EXISTS relay_member;" \
        "REMOVE TABLE IF EXISTS relay_roster;" \
        "REMOVE TABLE IF EXISTS relay_moderation;" \
        "REMOVE TABLE IF EXISTS relay_team_channel;" \
        "REMOVE ACCESS IF EXISTS relay ON DATABASE;" \
        "REMOVE TABLE IF EXISTS relay_db_user;"
}

# Arguments passed to the binary inside the container. No password flag.
relay_access_down_argv() {
    printf '%s\n' \
        /surreal sql \
        --endpoint http://127.0.0.1:8000 \
        --hide-welcome \
        --json
}

if [[ "${BASH_SOURCE[0]}" != "$0" ]]; then
    return 0
fi

if [[ "${1:-}" != "--yes" ]]; then
    echo "error: this removes the relay record login. Read docs/deploy.md, then pass --yes." >&2
    exit 1
fi

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
SECRETS="$ROOT/data/secrets.env"
cd "$ROOT"

if [[ ! -f "$SECRETS" ]]; then
    echo "error: missing $SECRETS. Run scripts/install.sh first." >&2
    exit 1
fi

# shellcheck disable=SC1090
source "$SECRETS"
PROJECT_NAME="${COMPOSE_PROJECT_NAME:-$(basename "$ROOT")}"

trim_value() {
    local raw="$1"
    raw="${raw#"${raw%%[![:space:]]*}"}"
    raw="${raw%"${raw##*[![:space:]]}"}"
    printf '%s' "$raw"
}

ns="$(trim_value "${SURREALDB_NS:-scuffed_crew}")"
db="$(trim_value "${SURREALDB_DB:-main}")"
if [[ ! "$ns" =~ ^[A-Za-z0-9_]+$ || ! "$db" =~ ^[A-Za-z0-9_]+$ ]]; then
    echo "error: SURREALDB_NS and SURREALDB_DB must be ASCII identifiers." >&2
    exit 1
fi

if ! command -v podman >/dev/null 2>&1; then
    echo "error: podman is required" >&2
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

canonical=""
canonical_rank=0
while IFS=$'\t' read -r cid name status; do
    [[ -n "${name}" ]] || continue
    if ! is_our_surreal "${name}"; then
        continue
    fi
    if [[ "${status}" != Up* ]]; then
        continue
    fi
    rank=1
    case "${name}" in
        "${PROJECT_NAME}-surrealdb-1") rank=3 ;;
        "${PROJECT_NAME}-surrealdb"|"${PROJECT_NAME}-surrealdb-"*) rank=2 ;;
    esac
    if [[ "${rank}" -gt "${canonical_rank}" ]]; then
        canonical="${name}"
        canonical_rank="${rank}"
    fi
done < <(podman ps -a --format '{{.ID}}\t{{.Names}}\t{{.Status}}' 2>/dev/null || true)

if [[ -z "${canonical}" ]]; then
    echo "error: no project Surreal container is Up. Not removing the login." >&2
    exit 1
fi

echo "Removing relay access on ${canonical}. Stop strfry first if it is still running."
mapfile -t surreal_argv < <(relay_access_down_argv)
# timeout bounds the CLI. It waits a few seconds after the statements finish.
if ! relay_access_down_sql | timeout 30 podman exec -i \
    -e SURREAL_NAMESPACE="$ns" \
    -e SURREAL_DATABASE="$db" \
    "$canonical" \
    "${surreal_argv[@]}"; then
    echo "error: surreal sql failed. The password was not printed." >&2
    exit 1
fi
echo "Removed relay projection events and tables, ACCESS relay, and TABLE relay_db_user on ${ns}/${db}."
echo "Next: pin the older site image and run ./scripts/update.sh."
