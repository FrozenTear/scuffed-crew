#!/usr/bin/env bash
# A present-but-blank generated key is replaced, mode goes back to 0600,
# and the value is not printed.
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=lib/secrets-file.sh
source "${SCRIPT_DIR}/lib/secrets-file.sh"

fail() {
    echo "FAIL: $*" >&2
    exit 1
}

tmp="$(mktemp)"
cleanup() { rm -f "$tmp"; }
trap cleanup EXIT

printf 'SURREALDB_RELAY_PASSWORD=\n\nSURREALDB_PASSWORD=keep-me\nENCRYPTION_KEY=keep-key\n' > "$tmp"
chmod 644 "$tmp"
value="$(openssl rand -base64 32 | tr -d '\n')"
out="$(upsert_generated_secret "$tmp" SURREALDB_RELAY_PASSWORD "$value")"
[[ "$out" == "Replaced blank SURREALDB_RELAY_PASSWORD" ]] || fail "replace text: ${out}"
[[ "$out" != *"$value"* ]] || fail "stdout contained the secret"
mode="$(stat -c %a "$tmp")"
[[ "$mode" == "600" ]] || fail "mode after replace is ${mode}"
grep -q "^SURREALDB_RELAY_PASSWORD=${value}$" "$tmp" || fail "value was not written"
grep -q "^SURREALDB_PASSWORD=keep-me$" "$tmp" || fail "root password changed"
grep -q "^ENCRYPTION_KEY=keep-key$" "$tmp" || fail "encryption key changed"
# The blank line collapse leaves a single blank line at most between keys.
blank_run="$(awk 'BEGIN{n=0;max=0} /^[[:space:]]*$/{n++; if(n>max) max=n; next} {n=0} END{print max+0}' "$tmp")"
[[ "$blank_run" -le 1 ]] || fail "duplicate blank lines remain (${blank_run})"

out2="$(upsert_generated_secret "$tmp" SURREALDB_RELAY_PASSWORD "other-value")"
[[ -z "$out2" ]] || fail "second call printed: ${out2}"
grep -q "^SURREALDB_RELAY_PASSWORD=${value}$" "$tmp" || fail "existing value was replaced"

printf 'SURREALDB_APP_PASSWORD=   \n' > "$tmp"
chmod 644 "$tmp"
out3="$(upsert_generated_secret "$tmp" SURREALDB_APP_PASSWORD "app-secret")"
[[ "$out3" == "Replaced blank SURREALDB_APP_PASSWORD" ]] || fail "whitespace replace: ${out3}"
[[ "$out3" != *app-secret* ]] || fail "stdout contained app secret"
grep -q '^SURREALDB_APP_PASSWORD=app-secret$' "$tmp" || fail "whitespace key was not replaced"
mode="$(stat -c %a "$tmp")"
[[ "$mode" == "600" ]] || fail "mode after whitespace replace is ${mode}"

printf 'NOSTR_CHALLENGE_SECRET=already\n' > "$tmp"
chmod 644 "$tmp"
out4="$(upsert_generated_secret "$tmp" SURREALDB_RELAY_USER "scuffed_relay")"
[[ "$out4" == "Appended SURREALDB_RELAY_USER" ]] || fail "append text: ${out4}"
mode="$(stat -c %a "$tmp")"
[[ "$mode" == "600" ]] || fail "mode after append is ${mode}"
grep -q '^NOSTR_CHALLENGE_SECRET=already$' "$tmp" || fail "append clobbered a neighbor"

echo "secrets file checks passed"
