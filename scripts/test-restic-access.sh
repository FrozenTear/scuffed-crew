#!/usr/bin/env bash
# Repository location and RESTIC_PASSWORD_FILE rules. No restic binary required.
# A local repository must be refused unless BACKUP_ALLOW_LOCAL_REPO=1, and a
# password file must be mode 600 and outside the repo.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/restic-access.sh
source "${SCRIPT_DIR}/lib/restic-access.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT
mkdir -p "${TMP}/repo"
EUID_NOW="$(id -u)"

expect_fail() {
    local label="$1"
    shift
    if "$@" >/dev/null 2>&1; then
        fail "${label} was accepted"
    fi
}

expect_ok() {
    local label="$1"
    shift
    if ! "$@" >/dev/null 2>&1; then
        fail "${label} was refused"
    fi
}

# Local paths are refused. Remote backends are not.
for repo in /var/backups/scuffed local:/var/backups/scuffed ./repo ../repo repo; do
    if ! restic_repo_is_local "${repo}"; then
        fail "expected local repository: ${repo}"
    fi
done
for repo in \
    'sftp:backup@backup.example:/srv/restic/scuffed-crew' \
    's3:https://s3.example.com/bucket/scuffed-crew' \
    'rest:https://backup.example:8000/scuffed-crew' \
    'b2:bucket:scuffed-crew'; do
    if restic_repo_is_local "${repo}"; then
        fail "expected off-host repository: ${repo}"
    fi
done

(
    export ROOT="${TMP}/repo"
    export RESTIC_REPOSITORY=/var/backups/scuffed
    export RESTIC_PASSWORD=secret
    unset RESTIC_PASSWORD_FILE BACKUP_ALLOW_LOCAL_REPO
    expect_fail "local repository" load_restic_access
)

(
    export ROOT="${TMP}/repo"
    export RESTIC_REPOSITORY=/var/backups/scuffed
    export RESTIC_PASSWORD=secret
    export BACKUP_ALLOW_LOCAL_REPO=yes
    unset RESTIC_PASSWORD_FILE
    expect_fail "BACKUP_ALLOW_LOCAL_REPO=yes" load_restic_access
)

(
    export ROOT="${TMP}/repo"
    export RESTIC_REPOSITORY=/var/backups/scuffed
    export RESTIC_PASSWORD=secret
    export BACKUP_ALLOW_LOCAL_REPO=1
    unset RESTIC_PASSWORD_FILE
    expect_ok "BACKUP_ALLOW_LOCAL_REPO=1" load_restic_access
)

(
    export ROOT="${TMP}/repo"
    export RESTIC_REPOSITORY='sftp:backup@backup.example:/srv/restic/scuffed-crew'
    export RESTIC_PASSWORD=secret
    unset RESTIC_PASSWORD_FILE BACKUP_ALLOW_LOCAL_REPO
    expect_ok "sftp repository" load_restic_access
)

(
    export ROOT="${TMP}/repo"
    unset RESTIC_PASSWORD RESTIC_PASSWORD_FILE BACKUP_ALLOW_LOCAL_REPO
    export RESTIC_REPOSITORY='rest:https://backup.example:8000/scuffed-crew'
    expect_fail "missing password" load_restic_access
)

PW="${TMP}/restic-password"
printf '%s\n' 'correct horse' > "${PW}"
chmod 600 "${PW}"

(
    export ROOT="${TMP}/repo"
    export RESTIC_REPOSITORY='s3:https://s3.example.com/bucket/scuffed-crew'
    export RESTIC_PASSWORD_FILE="${PW}"
    unset RESTIC_PASSWORD BACKUP_ALLOW_LOCAL_REPO
    expect_ok "password file" load_restic_access
)

(
    export ROOT="${TMP}/repo"
    export RESTIC_REPOSITORY='s3:https://s3.example.com/bucket/scuffed-crew'
    export RESTIC_PASSWORD_FILE="${PW}"
    export RESTIC_PASSWORD=also-set
    unset BACKUP_ALLOW_LOCAL_REPO
    expect_fail "both password and password file" load_restic_access
)

chmod 640 "${PW}"
expect_fail "mode 640 password file" \
    validate_restic_password_file "${PW}" "${EUID_NOW}" "${TMP}/repo"
chmod 600 "${PW}"

if [[ "${EUID_NOW}" == "0" ]]; then
    expect_ok "root-owned password file" \
        validate_restic_password_file "${PW}" 0 "${TMP}/repo"
else
    expect_fail "non-root password file when euid is root" \
        validate_restic_password_file "${PW}" 0 "${TMP}/repo"
fi
expect_ok "password file owned by the current user" \
    validate_restic_password_file "${PW}" "${EUID_NOW}" "${TMP}/repo"

INSIDE="${TMP}/repo/restic-password"
printf '%s\n' 'correct horse' > "${INSIDE}"
chmod 600 "${INSIDE}"
expect_fail "password file inside the repo" \
    validate_restic_password_file "${INSIDE}" "${EUID_NOW}" "${TMP}/repo"

printf '\n' > "${PW}"
expect_fail "empty password file" \
    validate_restic_password_file "${PW}" "${EUID_NOW}" "${TMP}/repo"
printf '%s\n' 'correct horse' > "${PW}"
chmod 600 "${PW}"

printf 'RESTIC_PASSWORD=nope\n' > "${TMP}/repo/secrets.env"
if ! secrets_file_defines_restic_password "${TMP}/repo/secrets.env"; then
    fail "did not detect RESTIC_PASSWORD in secrets.env"
fi
printf 'ENCRYPTION_KEY=abc\n' > "${TMP}/repo/secrets.env"
if secrets_file_defines_restic_password "${TMP}/repo/secrets.env"; then
    fail "ENCRYPTION_KEY looked like RESTIC_PASSWORD"
fi

# A password file must not sit inside a directory that will be snapshotted.
(
    export RESTIC_PASSWORD_FILE="${PW}"
    mkdir -p "${TMP}/vol"
    expect_ok "password file outside the volume" restic_password_file_not_inside "${TMP}/vol"
    cp "${PW}" "${TMP}/vol/restic-password"
    chmod 600 "${TMP}/vol/restic-password"
    export RESTIC_PASSWORD_FILE="${TMP}/vol/restic-password"
    expect_fail "password file inside a backup path" restic_password_file_not_inside "${TMP}/vol"
)

echo "ok: restic access checks"
