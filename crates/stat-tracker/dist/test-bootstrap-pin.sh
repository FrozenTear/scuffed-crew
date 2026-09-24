#!/usr/bin/env bash
# The updater and the copy-paste command must run bootstrap.sh from the
# release tag. `main` stays the fresh-install entrypoint, and that entrypoint
# re-execs the tag's script instead of installing with whatever main has now.
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BOOTSTRAP="$DIST/bootstrap.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*" >&2; }

[[ -f "$BOOTSTRAP" ]] || fail "missing $BOOTSTRAP"

# shellcheck disable=SC1090
STAT_TRACKER_BOOTSTRAP_LIB_ONLY=1 source "$BOOTSTRAP"

url="$(bootstrap_url_for_ref stat-tracker-v0.4.7)"
[[ "$url" == "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.4.7/crates/stat-tracker/dist/bootstrap.sh" ]] \
    || fail "tag URL was $url"
[[ "$url" != *"/main/"* ]] || fail "tag URL still points at main: $url"
pass "bootstrap URL is built from the tag"

[[ -z "${STAT_TRACKER_MINISIGN_PUB_DEFAULT//[[:space:]]/}" ]] \
    || fail "a minisign public key was committed; the maintainer must embed their own"
pass "no minisign public key is shipped"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# Already-pinned process must not download again (breaks the re-exec loop).
cat > "$TMP/fetch-fail" << EOF
#!/bin/bash
echo called > "$TMP/called"
exit 1
EOF
chmod +x "$TMP/fetch-fail"
if ! STAT_TRACKER_BOOTSTRAP_PINNED=1 \
    STAT_TRACKER_BOOTSTRAP_FETCH_CMD="$TMP/fetch-fail" \
    maybe_reexec_tag_bootstrap
then
    fail "pinned bootstrap tried to re-exec"
fi
[[ ! -f "$TMP/called" ]] || fail "pinned bootstrap called the fetch hook"
pass "pinned bootstrap does not re-exec"

# Unpinned + STAT_TRACKER_TAG downloads THAT tag's script and execs it with
# the pin flag set, so the child does not fetch main.
cat > "$TMP/fetch" << EOF
#!/bin/bash
printf '%s\n' "\$1" > "$TMP/url"
cat > "\$2" << 'SCRIPT'
#!/bin/bash
printf 'pinned=%s tag=%s\n' "\${STAT_TRACKER_BOOTSTRAP_PINNED-}" "\${STAT_TRACKER_TAG-}"
SCRIPT
EOF
chmod +x "$TMP/fetch"
out="$(
    STAT_TRACKER_TAG=stat-tracker-v0.4.7 \
    STAT_TRACKER_BOOTSTRAP_PINNED= \
    STAT_TRACKER_BOOTSTRAP_FETCH_CMD="$TMP/fetch" \
    maybe_reexec_tag_bootstrap
)"
[[ "$out" == "pinned=1 tag=stat-tracker-v0.4.7" ]] || fail "re-exec env was '$out'"
got="$(cat "$TMP/url")"
[[ "$got" == "$url" ]] || fail "re-exec fetched $got"
[[ "$got" != *"/main/"* ]] || fail "re-exec fetched main"
pass "unpinned run re-execs the tag bootstrap"

set +e
unsafe="$(
    STAT_TRACKER_TAG='../evil' \
    STAT_TRACKER_BOOTSTRAP_PINNED= \
    STAT_TRACKER_BOOTSTRAP_FETCH_CMD="$TMP/fetch-fail" \
    maybe_reexec_tag_bootstrap 2>&1
)"
unsafe_code=$?
set -e
[[ "$unsafe_code" -ne 0 ]] || fail "unsafe tag was accepted"
[[ "$unsafe" == *unsafe* ]] || fail "unsafe tag error was '$unsafe'"
[[ ! -f "$TMP/called" ]] || fail "unsafe tag still fetched"
pass "unsafe tag is refused before fetch"

unset STAT_TRACKER_MINISIGN_PUB || true
SIG_URL=""
fallback="$(verify_release_signature "$TMP/unused" 2>&1)"
[[ "$fallback" == *"Signature check skipped"* ]] || fail "missing fallback log: $fallback"
[[ "$fallback" == *"not an independent trust root"* ]] || fail "fallback did not say sha256 is same-origin: $fallback"
pass "missing key or signature falls back to sha256 with a log line"

set +e
closed="$(
    STAT_TRACKER_MINISIGN_PUB='untrusted comment: minisign public key: test
RWQfakekeynotreal' \
    SIG_URL='https://example.invalid/scuffed-stat-tracker-linux-x86_64.tar.gz.minisig' \
    verify_release_signature "$TMP/unused" 2>&1
)"
closed_code=$?
set -e
[[ "$closed_code" -ne 0 ]] || fail "signature inputs were ignored"
[[ "$closed" == *minisign* ]] || fail "fail-closed message was '$closed'"
[[ "$closed" != *"Signature check skipped"* ]] || fail "both key and sig still skipped"
pass "key plus .minisig refuses to skip the check"

echo "All bootstrap pin checks passed."
