#!/usr/bin/env bash
# Restic repository and password rules for backup.sh, backup-init.sh, and restore.sh.
#
# The scheduled backup runs on the host, so RESTIC_PASSWORD has to be on the
# host too. Put it in RESTIC_PASSWORD_FILE (mode 600, outside the repo and
# outside every path restic snapshots). The repository itself must be off the
# host. A same-host path is refused unless BACKUP_ALLOW_LOCAL_REPO=1.
#
# shellcheck shell=bash

# True when the repository specifier is a local filesystem path.
restic_repo_is_local() {
    local repo="$1"
    case "${repo}" in
        local:* | file:* | /* | ./* | ../* | . | ..)
            return 0
            ;;
    esac
    # No scheme (sftp:, s3:, rest:, b2:, …) means a relative local path.
    if [[ "${repo}" != *:* ]]; then
        return 0
    fi
    return 1
}

# True when secrets.env assigns RESTIC_PASSWORD (that file is snapshotted).
secrets_file_defines_restic_password() {
    local file="$1"
    [[ -f "${file}" ]] || return 1
    grep -qE '^[[:space:]]*(export[[:space:]]+)?RESTIC_PASSWORD=' "${file}"
}

# Drop restic credentials before the secrets file is copied into a snapshot.
filter_secrets_for_snapshot() {
    local src="$1"
    local dest="$2"
    umask 077
    awk '
        /^[[:space:]]*(export[[:space:]]+)?RESTIC_PASSWORD=/ { next }
        /^[[:space:]]*(export[[:space:]]+)?RESTIC_PASSWORD_FILE=/ { next }
        /^[[:space:]]*(export[[:space:]]+)?RESTIC_REPOSITORY=/ { next }
        /^[[:space:]]*(export[[:space:]]+)?BACKUP_ALLOW_LOCAL_REPO=/ { next }
        { print }
    ' "${src}" > "${dest}"
    chmod 600 "${dest}"
}

# Validate RESTIC_PASSWORD_FILE.
# Args: file, euid, repo_root
# euid 0 requires owner root. Any other euid requires the current owner.
validate_restic_password_file() {
    local file="$1"
    local euid="$2"
    local repo_root="$3"
    local mode owner oct resolved root_resolved opt

    if [[ ! -f "${file}" ]]; then
        echo "error: RESTIC_PASSWORD_FILE not found: ${file}" >&2
        return 1
    fi
    mode="$(stat -c '%a' "${file}")"
    owner="$(stat -c '%u' "${file}")"
    oct=$((8#${mode}))
    if (( (oct & 077) != 0 )); then
        echo "error: RESTIC_PASSWORD_FILE must be mode 600 (no group or world access): ${file} is ${mode}" >&2
        return 1
    fi
    if (( (oct & 0400) == 0 )); then
        echo "error: RESTIC_PASSWORD_FILE is not readable by its owner: ${file} is ${mode}" >&2
        return 1
    fi
    if [[ "${euid}" == "0" ]]; then
        if [[ "${owner}" != "0" ]]; then
            echo "error: RESTIC_PASSWORD_FILE must be owned by root when the backup runs as root: ${file}" >&2
            return 1
        fi
    elif [[ "${owner}" != "${euid}" ]]; then
        echo "error: RESTIC_PASSWORD_FILE must be owned by the user running the backup: ${file}" >&2
        return 1
    fi

    resolved="$(realpath -e "${file}")"
    root_resolved="$(realpath -e "${repo_root}")"
    case "${resolved}" in
        "${root_resolved}" | "${root_resolved}"/*)
            echo "error: RESTIC_PASSWORD_FILE must live outside the repo (${root_resolved}): ${resolved}" >&2
            echo "  The repo tree and data/secrets.env are what restic snapshots. A password stored there is not an off-snapshot secret." >&2
            return 1
            ;;
    esac
    if [[ -d /opt/scuffed-crew ]]; then
        opt="$(realpath -e /opt/scuffed-crew)"
        case "${resolved}" in
            "${opt}" | "${opt}"/*)
                echo "error: RESTIC_PASSWORD_FILE must live outside the deploy tree (${opt}): ${resolved}" >&2
                return 1
                ;;
        esac
    fi

    local pw
    pw="$(<"${file}")"
    pw="${pw%$'\n'}"
    pw="${pw%$'\r'}"
    if [[ -z "${pw//[[:space:]]/}" ]]; then
        echo "error: RESTIC_PASSWORD_FILE is empty: ${file}" >&2
        return 1
    fi
    unset pw
    return 0
}

# Refuse a password file that sits inside a directory restic is about to read.
restic_password_file_not_inside() {
    local dir="$1"
    local file dir_resolved
    [[ -n "${RESTIC_PASSWORD_FILE:-}" ]] || return 0
    [[ -n "${dir}" && -d "${dir}" ]] || return 0
    file="$(realpath -e "${RESTIC_PASSWORD_FILE}")"
    dir_resolved="$(realpath -e "${dir}")"
    case "${file}" in
        "${dir_resolved}" | "${dir_resolved}"/*)
            echo "error: RESTIC_PASSWORD_FILE is inside a path this backup stores: ${file}" >&2
            echo "  Move it outside the repo, the data volumes, and the export directory." >&2
            return 1
            ;;
    esac
    return 0
}

# Check RESTIC_REPOSITORY and exactly one of RESTIC_PASSWORD / RESTIC_PASSWORD_FILE.
# Requires ROOT (repo root) for the password-file location check.
load_restic_access() {
    local repo="${RESTIC_REPOSITORY:-}"
    repo="${repo#"${repo%%[![:space:]]*}"}"
    repo="${repo%"${repo##*[![:space:]]}"}"
    if [[ -z "${repo}" ]]; then
        echo "error: RESTIC_REPOSITORY is unset or blank." >&2
        echo "  Set it to an off-host restic repository (sftp:, s3:, rest:, or another backend)." >&2
        return 1
    fi
    export RESTIC_REPOSITORY="${repo}"

    if restic_repo_is_local "${repo}"; then
        if [[ "${BACKUP_ALLOW_LOCAL_REPO:-}" != "1" ]]; then
            echo "error: RESTIC_REPOSITORY is a local path on this host: ${repo}" >&2
            echo "  Store the repository off the host (sftp, S3-compatible, rest-server, or another restic backend)." >&2
            echo "  Set BACKUP_ALLOW_LOCAL_REPO=1 only for a deliberate same-host repository." >&2
            return 1
        fi
    fi

    local has_pw=0 has_file=0
    [[ -n "${RESTIC_PASSWORD:-}" ]] && has_pw=1
    [[ -n "${RESTIC_PASSWORD_FILE:-}" ]] && has_file=1
    if (( has_pw == 1 && has_file == 1 )); then
        echo "error: set only one of RESTIC_PASSWORD or RESTIC_PASSWORD_FILE." >&2
        return 1
    fi
    if (( has_file == 1 )); then
        validate_restic_password_file "${RESTIC_PASSWORD_FILE}" "$(id -u)" "${ROOT}"
        return $?
    fi
    if (( has_pw == 0 )) || [[ -z "${RESTIC_PASSWORD//[[:space:]]/}" ]]; then
        echo "error: set RESTIC_PASSWORD or RESTIC_PASSWORD_FILE." >&2
        echo "  A scheduled backup needs the password on the host: root-owned mode 600, outside the repo." >&2
        echo "  Also keep a copy of RESTIC_PASSWORD and ENCRYPTION_KEY in a password manager." >&2
        return 1
    fi
    return 0
}
