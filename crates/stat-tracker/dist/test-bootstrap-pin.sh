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

# A stalled download must fail, not hang: the in-app update has the tracker
# stopped while this script runs. Run the helper against a stand-in curl and
# check what it was called with.
mkdir -p "$TMP/fakecurl"
cat > "$TMP/fakecurl/curl" << EOF
#!/bin/bash
printf '%s\n' "\$@" > "$TMP/curl-args"
EOF
chmod +x "$TMP/fakecurl/curl"
PATH="$TMP/fakecurl:$PATH" curl_https -o "$TMP/out" "https://example.invalid/x"
mapfile -t curl_args < "$TMP/curl-args"
has_pair() {
    local flag="$1" value="$2" i
    for ((i = 0; i + 1 < ${#curl_args[@]}; i++)); do
        [[ "${curl_args[i]}" == "$flag" && "${curl_args[i + 1]}" == "$value" ]] && return 0
    done
    return 1
}
has_pair --proto '=https' || fail "curl_https dropped --proto =https: ${curl_args[*]}"
has_pair --connect-timeout 15 || fail "curl_https has no connect timeout: ${curl_args[*]}"
has_pair --speed-limit 1024 || fail "curl_https has no stall limit: ${curl_args[*]}"
has_pair --speed-time 60 || fail "curl_https has no stall window: ${curl_args[*]}"
[[ "${curl_args[-1]}" == "https://example.invalid/x" ]] || fail "curl_https lost the URL: ${curl_args[*]}"
pass "curl_https gives up on a stalled connect or transfer"

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

# This shell sourced bootstrap.sh, so SIGPIPE is ignored here. The tag's
# script (an older one has no trap) must get the default back, the way it
# ran before main set the trap. Compared with a child started after an
# explicit reset, so a shell that began with SIGPIPE ignored (which bash
# cannot undo) does not fail the check.
if [[ -r /proc/self/status ]]; then
    sigign() { bash -c 'sed -n "s/^SigIgn:[[:space:]]*//p" /proc/self/status'; }
    ignored_here="$(sigign)"
    default_here="$( (trap - PIPE; sigign) )"
    cat > "$TMP/fetch-sig-state" << EOF
#!/bin/bash
cat > "\$2" << 'SCRIPT'
#!/bin/bash
sed -n 's/^SigIgn:[[:space:]]*//p' /proc/self/status
SCRIPT
EOF
    chmod +x "$TMP/fetch-sig-state"
    child_ign="$(
        STAT_TRACKER_TAG=stat-tracker-v0.4.7 \
        STAT_TRACKER_BOOTSTRAP_PINNED='' \
        STAT_TRACKER_BOOTSTRAP_FETCH_CMD="$TMP/fetch-sig-state" \
        maybe_reexec_tag_bootstrap
    )"
    [[ "$child_ign" == "$default_here" ]] \
        || fail "re-exec kept SIGPIPE ignored (SigIgn $child_ign, default $default_here)"
    if [[ "$ignored_here" == "$default_here" ]]; then
        pass "re-exec SIGPIPE check is vacuous: this shell started with SIGPIPE ignored"
    else
        pass "re-exec hands the tag script the default SIGPIPE"
    fi
fi

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
    # CI only. A dead mirror must fail in minutes, not hang the release job.
    apt_opts=(-o Acquire::Retries=3 -o Acquire::http::Timeout=30 -o Acquire::https::Timeout=30)
    sudo timeout 180 apt-get "${apt_opts[@]}" update -qq
    sudo timeout 180 apt-get "${apt_opts[@]}" install -y --no-install-recommends minisign
fi
command -v minisign >/dev/null 2>&1 || fail "minisign is required to test signature verify"

printf 'payload\n' > "$TMP/payload"
# -W: empty password, no prompt. The secret key stays in $TMP and is removed
# with the test directory. Do not commit it.
minisign -G -p "$TMP/minisign.pub" -s "$TMP/minisign.key" -W >/dev/null
# Distinct from ${asset}.minisig, which is where verify_release_signature
# writes the downloaded signature. Copying a file onto itself fails.
minisign -S -s "$TMP/minisign.key" -m "$TMP/payload" -x "$TMP/good.minisig" -W >/dev/null

cat > "$TMP/fetch-sig" << EOF
#!/bin/bash
cp "$TMP/good.minisig" "\$2"
EOF
chmod +x "$TMP/fetch-sig"
good="$(
    STAT_TRACKER_BOOTSTRAP_FETCH_CMD="$TMP/fetch-sig" \
    STAT_TRACKER_MINISIGN_PUB="$TMP/minisign.pub" \
    SIG_URL="https://example.invalid/payload.minisig" \
    verify_release_signature "$TMP/payload" 2>&1
)"
[[ "$good" == *"minisign ok"* ]] || fail "good signature was not accepted: $good"
pass "key plus good .minisig verifies"

printf 'tampered\n' > "$TMP/payload"
set +e
bad="$(
    STAT_TRACKER_BOOTSTRAP_FETCH_CMD="$TMP/fetch-sig" \
    STAT_TRACKER_MINISIGN_PUB="$TMP/minisign.pub" \
    SIG_URL="https://example.invalid/payload.minisig" \
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

# Every real curl invocation pins https. Comments are not invocations.
while IFS= read -r line; do
    case "$line" in
        *"curl --proto '=https'"*|*"curl_https"*) ;;
        *) fail "curl call is not https-pinned: $line" ;;
    esac
done < <(grep -E '^[[:space:]]*curl ' "$BOOTSTRAP" || true)
grep -q "curl --proto '=https' -fsSL" "$BOOTSTRAP" \
    || fail "curl helper is missing --proto '=https' -fsSL"
grep -q 'curl_https -o "$ASSET_NAME"' "$BOOTSTRAP" \
    || fail "tarball download does not go through the https curl helper"
if grep -q -- '-fL --progress-bar' "$BOOTSTRAP"; then
    fail "tarball fetch still uses -fL --progress-bar without -fsSL"
fi
pass "curl calls are https-only and the tarball fetch is -fsSL"

python3 - "$BOOTSTRAP" <<'PY'
import sys
text = open(sys.argv[1], encoding="utf-8").read()
bad = []
for number, line in enumerate(text.splitlines(), 1):
    if "\u2014" in line or "\u2013" in line:
        bad.append(f"{number}:{line}")
if bad:
    raise SystemExit("em or en dash in bootstrap.sh\n" + "\n".join(bad))
PY
pass "bootstrap.sh has no em or en dash"

unset STAT_TRACKER_MINISIGN_PUB || true
SHA_URL=""
set +e
skip_sum="$(verify_release_checksum "$TMP/payload" 2>&1)"
skip_sum_code=$?
set -e
[[ "$skip_sum_code" -ne 0 ]] || fail "missing .sha256 was accepted: $skip_sum"
[[ "$skip_sum" == *"no .sha256 asset"* ]] || fail "missing-sha256 error was: $skip_sum"
[[ "$skip_sum" == *"Refusing to install"* ]] || fail "missing-sha256 did not refuse: $skip_sum"
pass "missing .sha256 refuses to install"

set +e
miss_sum="$(
    STAT_TRACKER_MINISIGN_PUB='untrusted comment: minisign public key: test
RWQfakekeynotreal' \
    SHA_URL='' \
    verify_release_checksum "$TMP/payload" 2>&1
)"
miss_sum_code=$?
set -e
[[ "$miss_sum_code" -ne 0 ]] || fail "key without .sha256 was accepted"
[[ "$miss_sum" == *"no .sha256 asset"* ]] || fail "missing-sha256 error was: $miss_sum"
[[ "$miss_sum" == *"Refusing to install"* ]] || fail "missing-sha256 did not refuse: $miss_sum"
pass "configured key and missing .sha256 refuses to install"

set +e
no_sum_tool="$(
    PATH="$TMP/nopath" \
    STAT_TRACKER_MINISIGN_PUB='untrusted comment: minisign public key: test
RWQfakekeynotreal' \
    SHA_URL='https://example.invalid/asset.sha256' \
    verify_release_checksum "$TMP/payload" 2>&1
)"
no_sum_tool_code=$?
set -e
[[ "$no_sum_tool_code" -ne 0 ]] || fail "key without sha256sum was accepted"
[[ "$no_sum_tool" == *"sha256sum is not installed"* ]] || fail "missing-sha256sum error was: $no_sum_tool"
pass "configured key and missing sha256sum refuses to install"

set +e
warn_tool="$(
    PATH="$TMP/nopath" \
    STAT_TRACKER_MINISIGN_PUB='' \
    SHA_URL='https://example.invalid/asset.sha256' \
    verify_release_checksum "$TMP/payload" 2>&1
)"
warn_tool_code=$?
set -e
[[ "$warn_tool_code" -ne 0 ]] || fail "no-key missing sha256sum was accepted: $warn_tool"
[[ "$warn_tool" == *"sha256sum is not installed"* ]] || fail "no-key missing sha256sum error was: $warn_tool"
[[ "$warn_tool" == *"Refusing to install"* ]] || fail "no-key missing sha256sum did not refuse: $warn_tool"
[[ "$warn_tool" != *skipping* ]] || fail "missing sha256sum still skips the check: $warn_tool"
pass "missing sha256sum refuses to install, with or without a key"

# Stable channel keeps 0.4.24 when a newer alpha is published. The alpha is
# the prerelease line, including when GitHub left the prerelease flag off.
ASSET="scuffed-stat-tracker-linux-x86_64.tar.gz"
cat > "$TMP/releases.json" <<'JSON'
[
  {"tag_name":"stat-tracker-v0.5.0-alpha.1","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.4.24","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]}
]
JSON
mapfile -t picked < <(choose_release_tags "$ASSET" < "$TMP/releases.json")
[[ "${picked[0]}" == "stat-tracker-v0.4.24" ]] || fail "stable tag was ${picked[0]}"
[[ "${picked[1]}" == "stat-tracker-v0.5.0-alpha.1" ]] || fail "prerelease tag was ${picked[1]}"
pass "0.4.24 stays the stable pick beside 0.5.0-alpha.1"

cat > "$TMP/releases.json" <<'JSON'
[
  {"tag_name":"stat-tracker-v0.4.24","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-alpha.1","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]}
]
JSON
mapfile -t picked < <(choose_release_tags "$ASSET" < "$TMP/releases.json")
[[ "${picked[0]}" == "stat-tracker-v0.4.24" ]] || fail "unflagged alpha became stable: ${picked[0]}"
[[ "${picked[1]}" == "stat-tracker-v0.5.0-alpha.1" ]] || fail "unflagged alpha was not the prerelease: ${picked[1]}"
pass "an unflagged alpha suffix is not the stable release"

cat > "$TMP/releases.json" <<'JSON'
[
  {"tag_name":"stat-tracker-v0.5.0-alpha.2","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-alpha.10","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.4.24","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]}
]
JSON
mapfile -t picked < <(choose_release_tags "$ASSET" < "$TMP/releases.json")
[[ "${picked[0]}" == "stat-tracker-v0.5.0" ]] || fail "final stable tag was ${picked[0]}"
[[ -z "${picked[1]}" ]] || fail "alpha was still newer than 0.5.0: ${picked[1]}"
pass "0.5.0 is newer than 0.5.0-alpha.10"

cat > "$TMP/releases.json" <<'JSON'
[
  {"tag_name":"stat-tracker-v0.5.0-alpha.2","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-alpha.10","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.4.24","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]}
]
JSON
mapfile -t picked < <(choose_release_tags "$ASSET" < "$TMP/releases.json")
[[ "${picked[0]}" == "stat-tracker-v0.4.24" ]] || fail "stable beside alphas was ${picked[0]}"
[[ "${picked[1]}" == "stat-tracker-v0.5.0-alpha.10" ]] || fail "newer alpha was ${picked[1]}"
pass "alpha.10 sorts after alpha.2"

# A 4-part point release sorts between its alpha and the next one.
cat > "$TMP/releases.json" <<'JSON'
[
  {"tag_name":"stat-tracker-v0.5.0-alpha.5","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-alpha.5.1","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.4.24","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]}
]
JSON
mapfile -t picked < <(choose_release_tags "$ASSET" < "$TMP/releases.json")
[[ "${picked[1]}" == "stat-tracker-v0.5.0-alpha.5.1" ]] || fail "alpha.5.1 was not newer than alpha.5: ${picked[1]}"
cat > "$TMP/releases.json" <<'JSON'
[
  {"tag_name":"stat-tracker-v0.5.0-alpha.6","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-alpha.5.1","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.4.24","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]}
]
JSON
mapfile -t picked < <(choose_release_tags "$ASSET" < "$TMP/releases.json")
[[ "${picked[1]}" == "stat-tracker-v0.5.0-alpha.6" ]] || fail "alpha.6 was not newer than alpha.5.1: ${picked[1]}"
pass "alpha.5 < alpha.5.1 < alpha.6"

# A test build (-test.N) is never picked, flagged or not, on any channel.
cat > "$TMP/releases.json" <<'JSON'
[
  {"tag_name":"stat-tracker-v0.5.0-test.1","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-test.2","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-alpha.10","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.5.0-rc1","draft":false,"prerelease":true,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]},
  {"tag_name":"stat-tracker-v0.4.24","draft":false,"prerelease":false,"assets":[{"name":"scuffed-stat-tracker-linux-x86_64.tar.gz"}]}
]
JSON
mapfile -t picked < <(choose_release_tags "$ASSET" < "$TMP/releases.json")
[[ "${picked[0]}" == "stat-tracker-v0.4.24" ]] || fail "stable beside a test build was ${picked[0]}"
[[ "${picked[1]}" == "stat-tracker-v0.5.0-rc1" ]] || fail "prerelease beside a test build was ${picked[1]}"
pass "a -test.N build is never picked; alpha and rc still are"

stable_pick="$(pick_release_tag stat-tracker-v0.4.24 stat-tracker-v0.5.0-alpha.1 "")"
[[ "$stable_pick" == "stat-tracker-v0.4.24" ]] || fail "default channel picked $stable_pick"
stable_pick="$(pick_release_tag stat-tracker-v0.4.24 stat-tracker-v0.5.0-alpha.1 stable)"
[[ "$stable_pick" == "stat-tracker-v0.4.24" ]] || fail "stable channel picked $stable_pick"
pre_pick="$(pick_release_tag stat-tracker-v0.4.24 stat-tracker-v0.5.0-alpha.1 prerelease)"
[[ "$pre_pick" == "stat-tracker-v0.5.0-alpha.1" ]] || fail "prerelease channel picked $pre_pick"
grep -F 'Install [s]table' "$BOOTSTRAP" >/dev/null && fail "bootstrap still prompts for a prerelease"
pass "stable skips the alpha unless the channel is prerelease"

# The release list is paged. site-* releases share it, so the only stable
# tracker release can sit past item 20 (0.4.24 was item 18 of 20 at rc.7).
# fetch_release_page is swapped for a reader of fixture pages; the pages
# come newest first, the way GitHub lists releases.
PAGES="$TMP/pages"
mkdir -p "$PAGES"
# make_page FILE SPEC...: SPEC is tag:kind with kind stable, pre or site
# (site ships no tracker asset).
make_page() {
    python3 - "$@" <<'PY'
import json, sys, zlib
out = sys.argv[1]
asset = "scuffed-stat-tracker-linux-x86_64.tar.gz"
page = []
for spec in sys.argv[2:]:
    tag, kind = spec.rsplit(":", 1)
    assets = [] if kind == "site" else [
        {"name": asset, "browser_download_url": "https://example.invalid/%s/%s" % (tag, asset)},
        {"name": asset + ".sha256", "browser_download_url": "https://example.invalid/%s/%s.sha256" % (tag, asset)},
    ]
    page.append({"id": zlib.crc32(tag.encode()), "tag_name": tag, "draft": False,
                 "prerelease": kind == "pre", "assets": assets})
with open(out, "w") as fh:
    json.dump(page, fh, indent=2)
PY
}
# fill N PREFIX KIND: N specs PREFIX1..PREFIXN of one kind.
fill() {
    local n="$1" prefix="$2" kind="$3" i
    for ((i = 1; i <= n; i++)); do
        printf '%s%s:%s\n' "$prefix" "$i" "$kind"
    done
}
fetch_release_page() {
    printf '%s\n' "$1" >> "$PAGES/fetched"
    [[ -f "$PAGES/page-$1.json" ]] || return 22
    cat "$PAGES/page-$1.json"
}
reset_pages() {
    rm -f "$PAGES"/page-*.json "$PAGES/fetched"
}
fetched_pages() {
    tr '\n' ' ' < "$PAGES/fetched" | sed 's/ $//'
}

# Stable at item 31 of a full first page: one request, stable found.
reset_pages
mapfile -t specs < <(
    fill 12 site-2026-10- site
    fill 18 stat-tracker-v0.5.0-rc. pre
    echo stat-tracker-v0.4.24:stable
    fill 69 site-2026-09- site
)
[[ "${#specs[@]}" -eq 100 ]] || fail "fixture page 1 has ${#specs[@]} releases"
make_page "$PAGES/page-1.json" "${specs[@]}"
make_page "$PAGES/page-2.json" stat-tracker-v0.4.23:stable
listing="$(fetch_release_listing)"
[[ "$(fetched_pages)" == "1" ]] || fail "stable on page 1 still read pages: $(fetched_pages)"
mapfile -t picked < <(choose_release_tags "$ASSET" <<<"$listing")
[[ "${picked[0]}" == "stat-tracker-v0.4.24" ]] || fail "stable at item 31 was ${picked[0]}"
[[ "${picked[1]}" == "stat-tracker-v0.5.0-rc.18" ]] || fail "prerelease beside it was ${picked[1]}"
pass "stable past item 20 is found on a 100-release page"

# A full page with no stable: page 2 is read, and paging stops there.
reset_pages
mapfile -t specs < <(fill 60 site-a- site; fill 40 stat-tracker-v0.5.0-rc. pre)
make_page "$PAGES/page-1.json" "${specs[@]}"
mapfile -t specs < <(fill 4 site-b- site; echo stat-tracker-v0.4.24:stable; fill 95 site-c- site)
make_page "$PAGES/page-2.json" "${specs[@]}"
mapfile -t specs < <(fill 100 site-d- site)
make_page "$PAGES/page-3.json" "${specs[@]}"
listing="$(fetch_release_listing)"
[[ "$(fetched_pages)" == "1 2" ]] || fail "stable on page 2 read pages: $(fetched_pages)"
mapfile -t picked < <(choose_release_tags "$ASSET" <<<"$listing")
[[ "${picked[0]}" == "stat-tracker-v0.4.24" ]] || fail "stable on page 2 was ${picked[0]}"
[[ "${picked[1]}" == "stat-tracker-v0.5.0-rc.40" ]] || fail "prerelease from page 1 was ${picked[1]}"
pass "a stable on page 2 is found and page 3 is not read"

# resolve_release end to end on those pages, stable channel: tag, asset,
# and sha256 of the page-2 stable.
rm -f "$PAGES/fetched"
mapfile -t resolved < <(STAT_TRACKER_TAG='' STAT_TRACKER_CHANNEL=stable resolve_release 2>/dev/null)
[[ "${resolved[0]:-}" == "stat-tracker-v0.4.24" ]] || fail "resolve_release tag was ${resolved[0]:-}"
[[ "${resolved[1]:-}" == "https://example.invalid/stat-tracker-v0.4.24/$ASSET" ]] \
    || fail "resolve_release url was ${resolved[1]:-}"
[[ "${resolved[2]:-}" == "https://example.invalid/stat-tracker-v0.4.24/$ASSET.sha256" ]] \
    || fail "resolve_release sha256 url was ${resolved[2]:-}"
pass "resolve_release on the stable channel picks the page-2 stable"

# A short page ends the list even with no stable in it.
reset_pages
mapfile -t specs < <(fill 30 site-a- site; fill 3 stat-tracker-v0.5.0-rc. pre)
make_page "$PAGES/page-1.json" "${specs[@]}"
listing="$(fetch_release_listing)"
[[ "$(fetched_pages)" == "1" ]] || fail "short page still read on: $(fetched_pages)"
mapfile -t picked < <(choose_release_tags "$ASSET" <<<"$listing")
[[ -z "${picked[0]}" && "${picked[1]}" == "stat-tracker-v0.5.0-rc.3" ]] \
    || fail "short page picks were '${picked[0]}' '${picked[1]}'"
pass "a short page ends the list"

# Never a stable: at most RELEASE_MAX_PAGES pages are read.
reset_pages
for n in 1 2 3 4 5 6; do
    mapfile -t specs < <(fill 100 "site-p$n-" site)
    make_page "$PAGES/page-$n.json" "${specs[@]}"
done
listing="$(fetch_release_listing)"
[[ "$(fetched_pages)" == "1 2 3 4 5" ]] || fail "page cap read: $(fetched_pages)"
count="$(release_page_len <<<"$listing")"
[[ "$count" -eq 500 ]] || fail "five pages merged to $count releases"
pass "paging stops after $RELEASE_MAX_PAGES pages"

# The list moved between reads: a release seen on two pages is kept once.
reset_pages
mapfile -t specs < <(fill 100 site-a- site)
make_page "$PAGES/page-1.json" "${specs[@]}"
make_page "$PAGES/page-2.json" site-a-100:site stat-tracker-v0.4.24:stable
listing="$(fetch_release_listing)"
count="$(release_page_len <<<"$listing")"
[[ "$count" -eq 101 ]] || fail "a repeated release was kept twice: $count releases"
pass "a release repeated across pages is kept once"

# A failed page read, or an API error object, fails the listing.
reset_pages
mapfile -t specs < <(fill 100 site-a- site)
make_page "$PAGES/page-1.json" "${specs[@]}"
if fetch_release_listing >/dev/null 2>&1; then
    fail "a failed page 2 read still returned a listing"
fi
printf '{"message":"API rate limit exceeded"}' > "$PAGES/page-1.json"
set +e
not_list="$(fetch_release_listing 2>&1)"
not_list_code=$?
set -e
[[ "$not_list_code" -ne 0 ]] || fail "an API error object was taken as a release list"
[[ "$not_list" == *"rate limit"* ]] || fail "API error was not reported: $not_list"
pass "a failed or non-list page read fails the listing"

# Logging must survive a closed stderr: the app that ran the update quit.
# The reader (true) exits at once; the writes after the sleep hit a pipe
# with no reader.
rm -f "$TMP/closed-ok"
set +e
bash -c '
    set -euo pipefail
    STAT_TRACKER_BOOTSTRAP_LIB_ONLY=1 source "$1"
    sleep 0.3
    info "to a closed pipe"
    warn "to a closed pipe"
    error "to a closed pipe"
    echo survived > "$2"
' _ "$BOOTSTRAP" "$TMP/closed-ok" 2>&1 | true
set -e
[[ -f "$TMP/closed-ok" ]] || fail "bootstrap logging died on a closed stderr"
pass "bootstrap logging survives a closed stderr"

echo "All bootstrap pin checks passed."
