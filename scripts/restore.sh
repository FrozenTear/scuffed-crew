#!/usr/bin/env bash
# Scuffed Crew — Interactive restore from restic backup.
#
# Restores the snapshot, installs data/secrets.env (including ENCRYPTION_KEY)
# when the host key is missing or different, and checks the key against the
# snapshot fingerprint BEFORE the app may start. The check exits non-zero if
# ENCRYPTION_KEY is missing or does not match.
#
# Usage:
#   ./scripts/restore.sh [snapshot-id]
#
# Environment (required):
#   RESTIC_REPOSITORY       Off-host restic repo (local paths need BACKUP_ALLOW_LOCAL_REPO=1)
#   RESTIC_PASSWORD         or RESTIC_PASSWORD_FILE (mode 600, outside the repo)
#
# The timer keeps RESTIC_PASSWORD on the host. Recovery after the host is gone
# uses the off-host repository plus the password-manager copies of
# RESTIC_PASSWORD and ENCRYPTION_KEY.
#
# Environment (optional):
#   SCUFFED_SECRETS_FILE
#       Destination secrets path (default: <repo>/data/secrets.env)
#   SCUFFED_RESTORE_INSTALL_SECRETS=1
#       Non-interactive: install the snapshot's secrets.env over the
#       destination when the key is missing or does not match, then re-check.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "${SCRIPT_DIR}/.." && pwd)"
# shellcheck source=lib/encryption-key.sh
source "${SCRIPT_DIR}/lib/encryption-key.sh"
# shellcheck source=lib/restic-access.sh
source "${SCRIPT_DIR}/lib/restic-access.sh"
load_restic_access

SECRETS_DEST="${SCUFFED_SECRETS_FILE:-${ROOT}/data/secrets.env}"
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
find "${RESTORE_DIR}" -type f | head -20 || true
echo

# Prefer the secrets.env that sits beside the fingerprint written by backup.sh.
FP_FILE=""
while IFS= read -r candidate; do
    FP_FILE="${candidate}"
    break
done < <(find "${RESTORE_DIR}" -name 'encryption-key.fingerprint' -type f)

SNAP_SECRETS=""
if [[ -n "${FP_FILE}" && -f "$(dirname "${FP_FILE}")/secrets.env" ]]; then
    SNAP_SECRETS="$(dirname "${FP_FILE}")/secrets.env"
fi
if [[ -z "${SNAP_SECRETS}" ]]; then
    while IFS= read -r candidate; do
        SNAP_SECRETS="${candidate}"
        break
    done < <(find "${RESTORE_DIR}" -name 'secrets.env' -type f)
fi

install_snapshot_secrets() {
    if [[ -z "${SNAP_SECRETS}" || ! -f "${SNAP_SECRETS}" ]]; then
        echo "error: snapshot has no secrets.env to install" >&2
        return 1
    fi
    mkdir -p "$(dirname "${SECRETS_DEST}")"
    install -m 600 "${SNAP_SECRETS}" "${SECRETS_DEST}"
    echo "Installed backup secrets to ${SECRETS_DEST} (mode 600)."
    echo "Do not start the app until the encryption-key check succeeds."
}

if [[ -z "${FP_FILE}" || -z "${SNAP_SECRETS}" ]]; then
    echo "error: this snapshot has no secrets.env / encryption-key fingerprint." >&2
    echo "  Backups taken before secrets were included cannot prove which ENCRYPTION_KEY sealed the data." >&2
    echo "  Put the original data/secrets.env in place, then run scripts/check-restore-key.sh." >&2
    echo "  Do not start the app until that check succeeds." >&2
    EXPORT_FILE="$(find "${RESTORE_DIR}" -name '*.surql' -type f | head -1 || true)"
    if [[ -n "${EXPORT_FILE}" ]]; then
        echo "  Logical export left unrestored: ${EXPORT_FILE}" >&2
    fi
    echo "  Restored files remain in ${RESTORE_DIR}" >&2
    exit 1
fi

echo "Checking ENCRYPTION_KEY in ${SECRETS_DEST} against this snapshot..."
echo "Stop the app. Secrets must match before it starts."
if ! check_restored_encryption_key "${SECRETS_DEST}" "${FP_FILE}"; then
    echo
    echo "The app must not start until ${SECRETS_DEST} holds the ENCRYPTION_KEY from this backup."
    install_now=0
    if [[ "${SCUFFED_RESTORE_INSTALL_SECRETS:-}" == "1" ]]; then
        install_now=1
    elif [[ -t 0 ]]; then
        if [[ -f "${SECRETS_DEST}" ]]; then
            read -rp "Replace ${SECRETS_DEST} with the backup's secrets.env? (y/N): " CONFIRM
            [[ "${CONFIRM}" =~ ^[Yy]$ ]] && install_now=1
        else
            read -rp "Install the backup's secrets.env to ${SECRETS_DEST} before starting the app? (Y/n): " CONFIRM
            [[ ! "${CONFIRM}" =~ ^[Nn]$ ]] && install_now=1
        fi
    else
        echo "error: not a terminal. Re-run with SCUFFED_RESTORE_INSTALL_SECRETS=1 to install the backup secrets, or copy them yourself and re-run scripts/check-restore-key.sh." >&2
        echo "  Restored files remain in ${RESTORE_DIR}" >&2
        exit 1
    fi
    if [[ "${install_now}" -ne 1 ]]; then
        echo "error: encryption key was not installed. Do not start the app." >&2
        echo "  Restored files remain in ${RESTORE_DIR}" >&2
        exit 1
    fi
    install_snapshot_secrets
    echo
    if ! check_restored_encryption_key "${SECRETS_DEST}" "${FP_FILE}"; then
        echo "error: encryption key still does not match after installing backup secrets." >&2
        echo "  Do not start the app. Restored files remain in ${RESTORE_DIR}" >&2
        exit 1
    fi
fi

echo
echo "Encryption key matches. Secrets are in ${SECRETS_DEST}."
echo "Import the database next if you need to. Start the app only after that, using this secrets file."

# Check for logical export. Skip the prompt when stdin is not a terminal so a
# non-interactive restore can exit 0 after the key check.
EXPORT_FILE=$(find "${RESTORE_DIR}" -name '*.surql' -type f | head -1 || true)
if [[ -n "${EXPORT_FILE}" ]]; then
    echo "Found logical export: ${EXPORT_FILE}"
    echo
    if [[ ! -t 0 ]]; then
        echo "Not a terminal — skipping SurrealDB import."
        echo "Import ${EXPORT_FILE} yourself before starting the app."
    else
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
fi

echo
echo "Restore directory: ${RESTORE_DIR}"
echo "Review the restored data, then clean up with: rm -rf ${RESTORE_DIR}"
echo "Do not start the app unless the encryption-key check above succeeded."
