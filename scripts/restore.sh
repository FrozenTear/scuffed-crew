#!/usr/bin/env bash
# Scuffed Crew — Interactive restore from restic backup.
#
# Usage:
#   ./scripts/restore.sh [snapshot-id]
#
# Without a snapshot ID, lists available snapshots for selection.
#
# Environment (required):
#   RESTIC_REPOSITORY   Restic repo path/URL
#   RESTIC_PASSWORD     Restic repo password

set -euo pipefail

: "${RESTIC_REPOSITORY:?Set RESTIC_REPOSITORY}"
: "${RESTIC_PASSWORD:?Set RESTIC_PASSWORD}"

SNAPSHOT="${1:-}"

if [[ -z "${SNAPSHOT}" ]]; then
    echo "Available snapshots:"
    echo
    restic snapshots --tag scuffed-crew
    echo
    read -rp "Enter snapshot ID to restore (or 'latest'): " SNAPSHOT
fi

if [[ -z "${SNAPSHOT}" ]]; then
    echo "No snapshot selected. Aborting."
    exit 1
fi

RESTORE_DIR="$(mktemp -d)"
echo "Restoring snapshot '${SNAPSHOT}' to ${RESTORE_DIR}..."

restic restore "${SNAPSHOT}" --target "${RESTORE_DIR}"

echo
echo "Restored files:"
find "${RESTORE_DIR}" -type f | head -20
echo

# Check for logical export
EXPORT_FILE=$(find "${RESTORE_DIR}" -name '*.surql' -type f | head -1)
if [[ -n "${EXPORT_FILE}" ]]; then
    echo "Found logical export: ${EXPORT_FILE}"
    echo
    read -rp "Import into SurrealDB? (y/N): " CONFIRM
    if [[ "${CONFIRM}" =~ ^[Yy]$ ]]; then
        SURREAL_URL="${SURREALDB_URL:-http://localhost:8000}"
        ROOT_USER="${SURREALDB_ROOT_USER:-root}"
        read -rsp "SurrealDB root password: " ROOT_PASS
        echo

        SURREAL_NS="${SURREALDB_NS:-scuffed_crew}"
        SURREAL_DB="${SURREALDB_DB:-main}"

        echo "Importing into ${SURREAL_NS}/${SURREAL_DB}..."
        surreal import \
            --conn "${SURREAL_URL}" \
            --user "${ROOT_USER}" \
            --pass "${ROOT_PASS}" \
            --ns "${SURREAL_NS}" \
            --db "${SURREAL_DB}" \
            "${EXPORT_FILE}"
        echo "Import complete."
    fi
fi

echo
echo "Restore directory: ${RESTORE_DIR}"
echo "Review the restored data, then clean up with: rm -rf ${RESTORE_DIR}"
