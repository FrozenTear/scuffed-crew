#!/usr/bin/env bash
# ENCRYPTION_KEY parse, fingerprint, and restore check.
#
# Sourced by backup.sh, restore.sh, and check-restore-key.sh. Not executed directly.
#
# Values are taken literally (quotes stripped, no shell expansion), matching
# Podman/Compose env-file substitution into the server process — not `source`.
#
# The fingerprint is sha256 of the material the server will load: the current
# key, the numeric version (default 1), and ENCRYPTION_KEY_PREVIOUS. It is not
# the key. Restore compares data/secrets.env to the fingerprint stored in the
# restic snapshot and fails if the key is missing or different.

# shellcheck shell=bash

_ek_trim() {
    local s="$1"
    s="${s#"${s%%[![:space:]]*}"}"
    s="${s%"${s##*[![:space:]]}"}"
    printf '%s' "$s"
}

_ek_unquote() {
    local s="$1"
    local len=${#s}
    if (( len >= 2 )); then
        local first="${s:0:1}"
        local last="${s:len-1:1}"
        if [[ "$first" == "$last" && ( "$first" == '"' || "$first" == "'" ) ]]; then
            s="${s:1:len-2}"
        fi
    fi
    printf '%s' "$s"
}

# Sets EK_KEY, EK_VERSION (u32, default 1), EK_PREVIOUS (may be empty).
# Returns 1 when the file is missing or ENCRYPTION_KEY is missing or blank.
read_encryption_material() {
    local file="$1"
    local line key="" version="" previous="" saw_version=0
    EK_KEY=""
    EK_VERSION="1"
    EK_PREVIOUS=""
    if [[ ! -f "$file" ]]; then
        return 1
    fi
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line%$'\r'}"
        [[ "$line" =~ ^[[:space:]]*# ]] && continue
        [[ -z "${line//[[:space:]]/}" ]] && continue
        if [[ "$line" =~ ^[[:space:]]*ENCRYPTION_KEY=(.*)$ ]]; then
            key="$(_ek_unquote "$(_ek_trim "${BASH_REMATCH[1]}")")"
        elif [[ "$line" =~ ^[[:space:]]*ENCRYPTION_KEY_VERSION=(.*)$ ]]; then
            version="$(_ek_unquote "$(_ek_trim "${BASH_REMATCH[1]}")")"
            saw_version=1
        elif [[ "$line" =~ ^[[:space:]]*ENCRYPTION_KEY_PREVIOUS=(.*)$ ]]; then
            previous="$(_ek_unquote "$(_ek_trim "${BASH_REMATCH[1]}")")"
        fi
    done < "$file"

    if [[ -z "$key" ]]; then
        return 1
    fi
    EK_KEY="$key"
    # Match CryptoService::from_env: unparseable versions become 1.
    if (( saw_version == 1 )) \
        && [[ "$version" =~ ^[0-9]+$ ]] \
        && (( ${#version} <= 10 )) \
        && (( 10#$version <= 4294967295 )); then
        EK_VERSION="$((10#$version))"
    else
        EK_VERSION="1"
    fi
    EK_PREVIOUS="$previous"
    return 0
}

encryption_key_fingerprint_hex() {
    printf '%s\n%s\n%s' "$EK_KEY" "$EK_VERSION" "$EK_PREVIOUS" | sha256sum | awk '{print $1}'
}

write_encryption_key_fingerprint() {
    local dest="$1"
    local hex
    hex="$(encryption_key_fingerprint_hex)"
    umask 077
    printf '%s\n' "$hex" > "$dest"
    chmod 600 "$dest"
}

# Copy secrets.env and write encryption-key.fingerprint into dest_dir.
# Refuses to stage a backup that has no usable ENCRYPTION_KEY.
stage_secrets_for_backup() {
    local dest_dir="$1"
    local secrets_file="$2"
    if ! read_encryption_material "$secrets_file"; then
        echo "error: ENCRYPTION_KEY is missing or blank in ${secrets_file}" >&2
        echo "  Refusing to back up data that cannot be decrypted after restore." >&2
        return 1
    fi
    mkdir -p "$dest_dir"
    chmod 700 "$dest_dir"
    # Never copy restic credentials into the snapshot. ENCRYPTION_KEY stays.
    if ! declare -F filter_secrets_for_snapshot >/dev/null 2>&1; then
        # shellcheck source=restic-access.sh
        source "$(dirname "${BASH_SOURCE[0]}")/restic-access.sh"
    fi
    filter_secrets_for_snapshot "$secrets_file" "${dest_dir}/secrets.env"
    write_encryption_key_fingerprint "${dest_dir}/encryption-key.fingerprint"
}

# Compare the secrets file the app will load against a backup fingerprint.
# Prints a loud error and returns 1 if the key is missing or does not match.
check_restored_encryption_key() {
    local secrets="$1"
    local fp_file="$2"
    local expected actual

    if [[ ! -f "$secrets" ]]; then
        echo "error: encryption key check failed: secrets file not found: ${secrets}" >&2
        echo "  Restore data/secrets.env from the backup before starting the app." >&2
        echo "  Do not start the app without the ENCRYPTION_KEY that sealed the database." >&2
        return 1
    fi
    if ! read_encryption_material "$secrets"; then
        echo "error: encryption key check failed: ENCRYPTION_KEY is missing or blank in ${secrets}" >&2
        echo "  Refusing this restore. Nostr keys, OAuth ids, and DMs cannot be decrypted." >&2
        echo "  Do not start the app." >&2
        return 1
    fi
    if [[ ! -f "$fp_file" ]]; then
        echo "error: encryption key check failed: fingerprint file not found: ${fp_file}" >&2
        echo "  This snapshot does not record which ENCRYPTION_KEY sealed the data." >&2
        echo "  Do not start the app until the original key is restored and checked." >&2
        return 1
    fi
    expected="$(awk 'NF && $1 !~ /^#/ { print $1; exit }' "$fp_file" | tr -d '[:space:]')"
    if [[ ! "$expected" =~ ^[0-9a-f]{64}$ ]]; then
        echo "error: encryption key check failed: fingerprint file is malformed: ${fp_file}" >&2
        echo "  Do not start the app." >&2
        return 1
    fi
    actual="$(encryption_key_fingerprint_hex)"
    if [[ "$actual" != "$expected" ]]; then
        echo "error: encryption key check failed: ENCRYPTION_KEY does not match this backup." >&2
        echo "  secrets: ${secrets}" >&2
        echo "  expected fingerprint: ${expected}" >&2
        echo "  actual fingerprint:   ${actual}" >&2
        echo "  Do not start the app. A mismatched key leaves Nostr keys and DMs unreadable." >&2
        return 1
    fi
    echo "encryption key matches backup fingerprint ${actual}"
    return 0
}
