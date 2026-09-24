#!/usr/bin/env bash
# Discriminating checks for backup secret staging and the restore key check.
# A green run here means a blank key is refused, a mismatched key fails, and a
# matching key passes. No restic or database required.

set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/encryption-key.sh
source "${SCRIPT_DIR}/lib/encryption-key.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

TMP="$(mktemp -d)"
trap 'rm -rf "${TMP}"' EXIT

cat > "${TMP}/secrets.env" <<'EOF'
# generated fixture — not a live secret
SURREALDB_PASSWORD=secret-db
ENCRYPTION_KEY=abc123key
ENCRYPTION_KEY_VERSION=1
EOF

stage_secrets_for_backup "${TMP}/stage" "${TMP}/secrets.env"

if grep -q 'abc123key' "${TMP}/stage/encryption-key.fingerprint"; then
    fail "fingerprint file contains the raw encryption key"
fi
if grep -q 'secret-db' "${TMP}/stage/encryption-key.fingerprint"; then
    fail "fingerprint file contains the database password"
fi

perm="$(stat -c '%a' "${TMP}/stage/secrets.env")"
[[ "${perm}" == "600" ]] || fail "staged secrets.env mode is ${perm}, want 600"
perm="$(stat -c '%a' "${TMP}/stage/encryption-key.fingerprint")"
[[ "${perm}" == "600" ]] || fail "fingerprint mode is ${perm}, want 600"

check_restored_encryption_key "${TMP}/secrets.env" "${TMP}/stage/encryption-key.fingerprint" \
    >/dev/null

# Quoted key and a missing version (server default 1) must match version 1.
printf 'ENCRYPTION_KEY="abc123key"\n' > "${TMP}/quoted.env"
check_restored_encryption_key "${TMP}/quoted.env" "${TMP}/stage/encryption-key.fingerprint" \
    >/dev/null

sed 's/abc123key/different-key/' "${TMP}/secrets.env" > "${TMP}/other.env"
if check_restored_encryption_key "${TMP}/other.env" "${TMP}/stage/encryption-key.fingerprint" \
    >/dev/null 2>&1; then
    fail "mismatched ENCRYPTION_KEY was accepted"
fi

printf 'ENCRYPTION_KEY=abc123key\nENCRYPTION_KEY_VERSION=2\n' > "${TMP}/v2.env"
if check_restored_encryption_key "${TMP}/v2.env" "${TMP}/stage/encryption-key.fingerprint" \
    >/dev/null 2>&1; then
    fail "ENCRYPTION_KEY_VERSION mismatch was accepted"
fi

printf 'ENCRYPTION_KEY=abc123key\nENCRYPTION_KEY_PREVIOUS=1:oldkey\n' > "${TMP}/prev.env"
if check_restored_encryption_key "${TMP}/prev.env" "${TMP}/stage/encryption-key.fingerprint" \
    >/dev/null 2>&1; then
    fail "ENCRYPTION_KEY_PREVIOUS mismatch was accepted"
fi

printf 'ENCRYPTION_KEY=\n' > "${TMP}/blank.env"
if stage_secrets_for_backup "${TMP}/stage-blank" "${TMP}/blank.env" >/dev/null 2>&1; then
    fail "blank ENCRYPTION_KEY was staged for backup"
fi

printf 'ENCRYPTION_KEY=   \n' > "${TMP}/ws.env"
if read_encryption_material "${TMP}/ws.env"; then
    fail "whitespace ENCRYPTION_KEY counted as present"
fi

if check_restored_encryption_key "${TMP}/missing.env" "${TMP}/stage/encryption-key.fingerprint" \
    >/dev/null 2>&1; then
    fail "missing secrets file was accepted"
fi

printf 'not-a-fingerprint\n' > "${TMP}/bad.fp"
if check_restored_encryption_key "${TMP}/secrets.env" "${TMP}/bad.fp" >/dev/null 2>&1; then
    fail "malformed fingerprint was accepted"
fi

# The operator-facing wrapper must fail closed too.
if "${SCRIPT_DIR}/check-restore-key.sh" --secrets "${TMP}/other.env" \
    --fingerprint "${TMP}/stage/encryption-key.fingerprint" >/dev/null 2>&1; then
    fail "check-restore-key.sh accepted a mismatched key"
fi
"${SCRIPT_DIR}/check-restore-key.sh" --secrets "${TMP}/secrets.env" \
    --fingerprint "${TMP}/stage/encryption-key.fingerprint" >/dev/null

echo "ok: backup secret checks"
