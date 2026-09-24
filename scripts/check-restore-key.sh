#!/usr/bin/env bash
# Fail if the secrets file's ENCRYPTION_KEY is missing or does not match a
# backup fingerprint. Run after restoring secrets and before starting the app.
#
# Usage:
#   scripts/check-restore-key.sh --secrets data/secrets.env \
#       --fingerprint path/to/encryption-key.fingerprint

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/encryption-key.sh
source "${SCRIPT_DIR}/lib/encryption-key.sh"

secrets=""
fingerprint=""
while [[ $# -gt 0 ]]; do
    case "$1" in
        --secrets)
            secrets="${2:-}"
            shift 2
            ;;
        --fingerprint)
            fingerprint="${2:-}"
            shift 2
            ;;
        *)
            echo "error: unknown argument: $1" >&2
            echo "usage: check-restore-key.sh --secrets FILE --fingerprint FILE" >&2
            exit 2
            ;;
    esac
done

if [[ -z "${secrets}" || -z "${fingerprint}" ]]; then
    echo "usage: check-restore-key.sh --secrets FILE --fingerprint FILE" >&2
    exit 2
fi

check_restored_encryption_key "${secrets}" "${fingerprint}"
