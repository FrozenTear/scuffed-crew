#!/usr/bin/env bash
# Scuffed Crew — Automated backup script
# Backs up SurrealDB data volume + logical export using restic.
#
# Prerequisites:
#   - restic installed and repo initialized (see backup-init.sh)
#   - SurrealDB running and reachable
#   - Podman with 'surrealdb-data' volume
#
# Environment (required):
#   RESTIC_REPOSITORY     Restic repo path/URL
#   RESTIC_PASSWORD       Restic repo password
#   SURREALDB_ROOT_USER   Root username for surreal export
#   SURREALDB_ROOT_PASSWORD Root password
#
# Environment (optional):
#   HEALTHCHECKS_URL      Healthchecks.io ping URL (no trailing slash)
#   SURREALDB_URL         DB URL (default: http://localhost:8000)
#   SURREALDB_NS          Namespace to export (default: scuffed_crew)
#   SURREALDB_DB          Database to export (default: main)

set -euo pipefail

SURREAL_URL="${SURREALDB_URL:-http://localhost:8000}"
SURREAL_NS="${SURREALDB_NS:-scuffed_crew}"
SURREAL_DB="${SURREALDB_DB:-main}"
ROOT_USER="${SURREALDB_ROOT_USER:?Set SURREALDB_ROOT_USER}"
ROOT_PASS="${SURREALDB_ROOT_PASSWORD:?Set SURREALDB_ROOT_PASSWORD}"
HEALTHCHECKS_URL="${HEALTHCHECKS_URL:-}"

EXPORT_DIR="$(mktemp -d)"
trap 'rm -rf "${EXPORT_DIR}"' EXIT

ping_hc() {
    if [[ -n "${HEALTHCHECKS_URL}" ]]; then
        curl -fsS --retry 3 --max-time 10 "${HEALTHCHECKS_URL}${1:-}" >/dev/null 2>&1 || true
    fi
}

# Signal start
ping_hc "/start"

echo "$(date -Iseconds) Starting backup..."

# 1. Logical export (surreal export)
EXPORT_FILE="${EXPORT_DIR}/${SURREAL_NS}_${SURREAL_DB}.surql"
echo "Exporting ${SURREAL_NS}/${SURREAL_DB}..."
surreal export \
    --conn "${SURREAL_URL}" \
    --user "${ROOT_USER}" \
    --pass "${ROOT_PASS}" \
    --ns "${SURREAL_NS}" \
    --db "${SURREAL_DB}" \
    "${EXPORT_FILE}"

echo "Export saved to ${EXPORT_FILE} ($(du -h "${EXPORT_FILE}" | cut -f1))"

# 2. Find volume mount path
VOLUME_PATH="$(podman volume inspect surrealdb-data --format '{{.Mountpoint}}' 2>/dev/null || true)"

# 3. Restic backup
echo "Running restic backup..."
BACKUP_PATHS=("${EXPORT_DIR}")
if [[ -n "${VOLUME_PATH}" && -d "${VOLUME_PATH}" ]]; then
    BACKUP_PATHS+=("${VOLUME_PATH}")
    echo "Including volume data from ${VOLUME_PATH}"
fi

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
