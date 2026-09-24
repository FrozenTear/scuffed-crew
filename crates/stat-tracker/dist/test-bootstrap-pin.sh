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
[[ "$fallback" == *"no published minisign public key"* ]] || fail "fallback reason was: $fallback"
[[ "$fallback" == *"not an independent trust root"* ]] || fail "fallback did not say sha256 is same-origin: $fallback"
pass "no public key falls back to sha256 with a log line"

set +e
missing_sig="$(
    STAT_TRACKER_MINISIGN_PUB='untrusted comment: minisign public key: test
RWQfakekeynotreal' \
    SIG_URL='' \
    verify_release_signature "$TMP/unused" 2>&1
)"
missing_sig_code=$?
set -e
[[ "$missing_sig_code" -ne 0 ]] || fail "key without .minisig was accepted"
[[ "$missing_sig" == *"no .minisig asset"* ]] || fail "missing-sig error was: $missing_sig"
[[ "$missing_sig" == *"Refusing to install"* ]] || fail "missing-sig did not refuse: $missing_sig"
[[ "$missing_sig" != *"Signature check skipped"* ]] || fail "key without .minisig fell back to sha256"
pass "configured key and missing .minisig refuses to install"

# minisign-not-installed is independent of whether the tool is on the real PATH.
mkdir -p "$TMP/nopath"
set +e
no_tool="$(
    PATH="$TMP/nopath" \
    STAT_TRACKER_MINISIGN_PUB='untrusted comment: minisign public key: test
RWQfakekeynotreal' \
    SIG_URL='file:///tmp/does-not-matter.minisig' \
    verify_release_signature "$TMP/unused" 2>&1
)"
no_tool_code=$?
set -e
[[ "$no_tool_code" -ne 0 ]] || fail "missing minisign binary was accepted"
[[ "$no_tool" == *"minisign is not installed"* ]] || fail "missing-tool error was: $no_tool"
[[ "$no_tool" != *"Signature check skipped"* ]] || fail "missing minisign fell back"
pass "key plus .minisig refuses when minisign is not installed"

if ! command -v minisign >/dev/null 2>&1; then
    sudo apt-get update -qq
    sudo apt-get install -y --no-install-recommends minisign
fi
command -v minisign >/dev/null 2>&1 || fail "minisign is required to test signature verify"

printf 'payload\n' > "$TMP/payload"
# -W: empty password, no prompt. The secret key stays in $TMP and is removed
# with the test directory. Do not commit it.
minisign -G -p "$TMP/minisign.pub" -s "$TMP/minisign.key" -W >/dev/null
minisign -S -s "$TMP/minisign.key" -m "$TMP/payload" -x "$TMP/payload.minisig" -W >/dev/null

good="$(
    STAT_TRACKER_MINISIGN_PUB="$TMP/minisign.pub" \
    SIG_URL="file://$TMP/payload.minisig" \
    verify_release_signature "$TMP/payload" 2>&1
)"
[[ "$good" == *"minisign ok"* ]] || fail "good signature was not accepted: $good"
pass "key plus good .minisig verifies"

printf 'tampered\n' > "$TMP/payload"
set +e
bad="$(
    STAT_TRACKER_MINISIGN_PUB="$TMP/minisign.pub" \
    SIG_URL="file://$TMP/payload.minisig" \
    verify_release_signature "$TMP/payload" 2>&1
)"
bad_code=$?
set -e
[[ "$bad_code" -ne 0 ]] || fail "bad signature was accepted"
[[ "$bad" == *"signature verification failed"* ]] || fail "bad-sig error was: $bad"
[[ "$bad" != *"Signature check skipped"* ]] || fail "bad signature fell back to sha256"
pass "key plus bad .minisig refuses to install"

# sha256 verify and the signature check both run before extract.
sha_line="$(grep -n 'Verifying sha256' "$BOOTSTRAP" | tail -1 | cut -d: -f1)"
sig_line="$(grep -n 'verify_release_signature "$ASSET_NAME"' "$BOOTSTRAP" | tail -1 | cut -d: -f1)"
ext_line="$(grep -n 'safe_extract "$WORKDIR/$ASSET_NAME"' "$BOOTSTRAP" | tail -1 | cut -d: -f1)"
[[ -n "$sha_line" && -n "$sig_line" && -n "$ext_line" ]] || fail "could not find verify/extract calls"
[[ "$sha_line" -lt "$sig_line" && "$sig_line" -lt "$ext_line" ]] \
    || fail "integrity checks are not before extract (sha=$sha_line sig=$sig_line extract=$ext_line)"
pass "sha256 and signature run before extract"

echo "All bootstrap pin checks passed."
