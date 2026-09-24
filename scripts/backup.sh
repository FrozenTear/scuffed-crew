#!/usr/bin/env bash
# Scuffed Crew — Automated backup script
# Backs up SurrealDB data volume + uploads + logical export + data/secrets.env
# (including ENCRYPTION_KEY) using restic.
#
# secrets.env goes in the same restic repository. The repository must live OFF
# this host. RESTIC_PASSWORD has to be ON the host for the systemd timer: point
# RESTIC_PASSWORD_FILE at a root-owned mode-600 file outside the repo and
# outside every path this script snapshots. Do not put RESTIC_PASSWORD in
# data/secrets.env (that file is copied into the snapshot). Keep a copy of
# RESTIC_PASSWORD and ENCRYPTION_KEY in a password manager so a lost host is
# recoverable from the off-host backup plus the password manager.
# restore.sh checks the encryption key before the app starts.
#
# Prerequisites:
#   - restic installed and repo initialized (see backup-init.sh)
#   - SurrealDB running and reachable (or podman exec path)
#   - Podman with compose volumes
#
# Environment (required):
#   RESTIC_REPOSITORY       Off-host restic repo (sftp:, s3:, rest:, …).
#                           A local path is refused unless BACKUP_ALLOW_LOCAL_REPO=1.
#   RESTIC_PASSWORD         Restic repo password (interactive runs)
#     or RESTIC_PASSWORD_FILE
#                           Root-owned mode-600 file for the timer. Not in the repo.
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
# shellcheck source=lib/encryption-key.sh
source "${SCRIPT_DIR}/lib/encryption-key.sh"
# shellcheck source=lib/restic-access.sh
source "${SCRIPT_DIR}/lib/restic-access.sh"
SECRETS_FILE=""
for f in "${ROOT}/data/secrets.env" /opt/scuffed-crew/data/secrets.env /opt/scuffed-crew/.env; do
    if [[ -f "$f" ]]; then
        SECRETS_FILE="$f"
        break
    fi
done

# data/secrets.env is copied into the snapshot. A restic password stored there
# would ship inside the repository it unlocks.
if [[ -n "${SECRETS_FILE}" ]] && secrets_file_defines_restic_password "${SECRETS_FILE}"; then
    echo "error: RESTIC_PASSWORD must not be set in ${SECRETS_FILE}." >&2
    echo "  That file is included in the restic snapshot. Use RESTIC_PASSWORD_FILE" >&2
    echo "  (root-owned, mode 600, outside the repo) for the timer, and keep a copy" >&2
    echo "  of the password in a password manager." >&2
    exit 1
fi

# The timer exports RESTIC_* before this script runs. Do not let secrets.env
# overwrite an off-host repository or password-file path.
_repo_set=${RESTIC_REPOSITORY+y}
_repo=${RESTIC_REPOSITORY-}
_pw_set=${RESTIC_PASSWORD+y}
_pw=${RESTIC_PASSWORD-}
_pwfile_set=${RESTIC_PASSWORD_FILE+y}
_pwfile=${RESTIC_PASSWORD_FILE-}
_allow_set=${BACKUP_ALLOW_LOCAL_REPO+y}
_allow=${BACKUP_ALLOW_LOCAL_REPO-}
if [[ -n "${SECRETS_FILE}" ]]; then
    # shellcheck disable=SC1090
    set -a
    # shellcheck source=/dev/null
    source "${SECRETS_FILE}"
    set +a
fi
[[ -n "${_repo_set}" ]] && RESTIC_REPOSITORY="${_repo}"
[[ -n "${_pw_set}" ]] && RESTIC_PASSWORD="${_pw}"
[[ -n "${_pwfile_set}" ]] && RESTIC_PASSWORD_FILE="${_pwfile}"
[[ -n "${_allow_set}" ]] && BACKUP_ALLOW_LOCAL_REPO="${_allow}"
unset _repo_set _repo _pw_set _pw _pwfile_set _pwfile _allow_set _allow

if ! load_restic_access; then
    exit 1
fi

# Fail before any export. A snapshot without ENCRYPTION_KEY cannot decrypt
# Nostr keys, OAuth ids, or DMs after the host is lost.
if [[ -z "${SECRETS_FILE}" ]] || ! read_encryption_material "${SECRETS_FILE}"; then
    echo "error: ENCRYPTION_KEY is missing or blank (${SECRETS_FILE:-no secrets file found})" >&2
    echo "  Refusing to back up data that cannot be decrypted after restore." >&2
    echo "  data/secrets.env must contain the ENCRYPTION_KEY that sealed the database." >&2
    exit 1
fi

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
SECRETS_STAGE="$(mktemp -d)"
chmod 700 "${SECRETS_STAGE}"
trap 'rm -rf "${EXPORT_DIR}" "${SECRETS_STAGE}"' EXIT

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

# 2b. Secrets required to decrypt restored rows. Staged as mode 600, not logged.
echo "Including secrets.env (encryption key) in the restic snapshot"
stage_secrets_for_backup "${SECRETS_STAGE}" "${SECRETS_FILE}"
BACKUP_PATHS+=("${SECRETS_STAGE}")

for _backup_path in "${BACKUP_PATHS[@]}"; do
    restic_password_file_not_inside "${_backup_path}"
done
unset _backup_path

# 3. Restic backup
echo "Running restic backup..."
restic backup \
    --tag scuffed-crew \
    --tag surrealdb \
    --tag secrets \
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
