#!/usr/bin/env bash
# Scuffed Crew — Automated backup script
# Backs up SurrealDB data volume + uploads + logical export using restic.
#
# Prerequisites:
#   - restic installed and repo initialized (see backup-init.sh)
#   - SurrealDB running and reachable (or podman exec path)
#   - Podman with compose volumes
#
# Environment (required):
#   RESTIC_REPOSITORY     Restic repo path/URL
#   RESTIC_PASSWORD       Restic repo password
#
# Environment (DB credentials — from data/secrets.env or explicit):
#   SURREALDB_PASSWORD or SURREALDB_ROOT_PASSWORD
#   SURREALDB_USER or SURREALDB_ROOT_USER (default: root)
#
# Environment (optional):
#   HEALTHCHECKS_URL      Healthchecks.io ping URL (no trailing slash)
#   SURREALDB_URL         HTTP URL for surreal CLI (default: http://127.0.0.1:8000)
#   SURREALDB_NS          Namespace to export (default: scuffed_crew)
#   SURREALDB_DB          Database to export (default: main)
#   COMPOSE_PROJECT_NAME  Prefix for volume names (optional)

set -euo pipefail

# Load secrets if present next to the repo or under /opt
SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
for f in "${ROOT}/data/secrets.env" /opt/scuffed-crew/data/secrets.env /opt/scuffed-crew/.env; do
    if [[ -f "$f" ]]; then
        # shellcheck disable=SC1090
        set -a
        # shellcheck source=/dev/null
        source "$f"
        set +a
        break
    fi
done

SURREAL_URL="${SURREALDB_URL:-http://127.0.0.1:8000}"
# Compose uses ws:// internally — CLI needs HTTP
if [[ "${SURREAL_URL}" == ws://* ]]; then
    SURREAL_URL="http://${SURREAL_URL#ws://}"
fi
SURREAL_NS="${SURREALDB_NS:-scuffed_crew}"
SURREAL_DB="${SURREALDB_DB:-main}"
ROOT_USER="${SURREALDB_ROOT_USER:-${SURREALDB_USER:-root}}"
ROOT_PASS="${SURREALDB_ROOT_PASSWORD:-${SURREALDB_PASSWORD:?Set SURREALDB_PASSWORD or SURREALDB_ROOT_PASSWORD}}"
HEALTHCHECKS_URL="${HEALTHCHECKS_URL:-}"

EXPORT_DIR="$(mktemp -d)"
trap 'rm -rf "${EXPORT_DIR}"' EXIT

ping_hc() {
    if [[ -n "${HEALTHCHECKS_URL}" ]]; then
        curl -fsS --retry 3 --max-time 10 "${HEALTHCHECKS_URL}${1:-}" >/dev/null 2>&1 || true
    fi
}

resolve_volume() {
    local suffix="$1"
    local name path
    # Exact names first, then project-prefixed (podman compose)
    for name in "${suffix}" "scuffed-crew_${suffix}" "${COMPOSE_PROJECT_NAME:-}_${suffix}"; do
        [[ -z "${name#_}" || "${name}" == "_"* ]] && continue
        path="$(podman volume inspect "${name}" --format '{{.Mountpoint}}' 2>/dev/null || true)"
        if [[ -n "${path}" && -d "${path}" ]]; then
            echo "${path}"
            return 0
        fi
    done
    # Fallback: any volume whose name ends with the suffix
    while IFS= read -r name; do
        [[ -z "${name}" ]] && continue
        path="$(podman volume inspect "${name}" --format '{{.Mountpoint}}' 2>/dev/null || true)"
        if [[ -n "${path}" && -d "${path}" ]]; then
            echo "${path}"
            return 0
        fi
    done < <(podman volume ls --format '{{.Name}}' 2>/dev/null | grep -E "${suffix}$" || true)
    return 1
}

# Signal start
ping_hc "/start"

echo "$(date -Iseconds) Starting backup..."

# 1. Logical export (surreal export)
EXPORT_FILE="${EXPORT_DIR}/${SURREAL_NS}_${SURREAL_DB}.surql"
echo "Exporting ${SURREAL_NS}/${SURREAL_DB}..."

export_via_podman=0
if ! curl -fsS --max-time 2 "${SURREAL_URL}/health" >/dev/null 2>&1 \
    && ! curl -fsS --max-time 2 "${SURREAL_URL}/status" >/dev/null 2>&1; then
    # Surreal not published on host — try podman exec
    CID="$(podman ps --filter ancestor=surrealdb/surrealdb --format '{{.ID}}' 2>/dev/null | head -1 || true)"
    if [[ -z "${CID}" ]]; then
        CID="$(podman ps --format '{{.Names}}' 2>/dev/null | grep -i surreal | head -1 || true)"
    fi
    if [[ -n "${CID}" ]]; then
        echo "Host Surreal URL unreachable; exporting via podman exec (${CID})..."
        podman exec "${CID}" surreal export \
            --conn "http://127.0.0.1:8000" \
            --user "${ROOT_USER}" \
            --pass "${ROOT_PASS}" \
            --ns "${SURREAL_NS}" \
            --db "${SURREAL_DB}" \
            "/tmp/scuffed-export.surql"
        podman cp "${CID}:/tmp/scuffed-export.surql" "${EXPORT_FILE}"
        export_via_podman=1
    fi
fi

if [[ "${export_via_podman}" -eq 0 ]]; then
    surreal export \
        --conn "${SURREAL_URL}" \
        --user "${ROOT_USER}" \
        --pass "${ROOT_PASS}" \
        --ns "${SURREAL_NS}" \
        --db "${SURREAL_DB}" \
        "${EXPORT_FILE}"
fi

echo "Export saved to ${EXPORT_FILE} ($(du -h "${EXPORT_FILE}" | cut -f1))"

# 2. Volume mount paths
BACKUP_PATHS=("${EXPORT_DIR}")
if VOLUME_PATH="$(resolve_volume surrealdb-data)"; then
    BACKUP_PATHS+=("${VOLUME_PATH}")
    echo "Including Surreal volume data from ${VOLUME_PATH}"
fi
if UPLOADS_PATH="$(resolve_volume uploads-data)"; then
    BACKUP_PATHS+=("${UPLOADS_PATH}")
    echo "Including uploads volume from ${UPLOADS_PATH}"
fi
if STRFRY_PATH="$(resolve_volume strfry-data)"; then
    BACKUP_PATHS+=("${STRFRY_PATH}")
    echo "Including strfry volume from ${STRFRY_PATH}"
fi

# 3. Restic backup
echo "Running restic backup..."
restic backup \
    --tag scuffed-crew \
    --tag surrealdb \
    "${BACKUP_PATHS[@]}"

# 4. Prune old snapshots (7 daily, 4 weekly, 6 monthly, 1 yearly)
echo "Pruning old snapshots..."
restic forget \
    --prune \
    --tag scuffed-crew \
    --keep-daily 7 \
    --keep-weekly 4 \
    --keep-monthly 6 \
    --keep-yearly 1

echo "$(date -Iseconds) Backup complete."
ping_hc
