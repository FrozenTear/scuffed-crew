#!/usr/bin/env bash
# The release stage copies the three files install.sh installs.
# Shadow digit PNGs are embedded in the binary and must not be packed.
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIST/.." && pwd)"
REPO="$(cd "$ROOT/../.." && pwd)"
STAGE="$DIST/stage-release-assets.sh"
WF="$REPO/.github/workflows/stat-tracker-release.yml"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }

[[ -f "$STAGE" ]] || fail "missing $STAGE"
[[ -f "$ROOT/assets/shadow-digits/digits_1440.png" ]] || fail "1440 template missing from the repo"
[[ -f "$ROOT/assets/shadow-digits/digits_1080.png" ]] || fail "1080 template missing from the repo"

if grep -n 'cp -r crates/stat-tracker/assets' "$WF"; then
  fail "release workflow still copies the whole assets tree"
fi
grep -q 'stage-release-assets.sh' "$WF" || fail "release workflow does not stage assets via stage-release-assets.sh"
pass "release workflow does not copy assets/shadow-digits"

if grep -n 'shadow-digits\|digits_1440\|digits_1080' "$ROOT/dist/install.sh" "$ROOT/install.sh"; then
  fail "install.sh references shadow digit templates"
fi
pass "install.sh does not reference shadow digit templates"

# Runtime reads: the PNGs may appear only as include_bytes! sources.
# The worker thread name is not a filesystem path.
hits="$(grep -R -n -E 'shadow-digits|digits_1440|digits_1080' "$ROOT/src" || true)"
[[ -n "$hits" ]] || fail "embedded templates are no longer referenced"
bad="$(printf '%s\n' "$hits" | grep -v 'include_bytes!' | grep -v 'name("shadow-digits"' || true)"
if [[ -n "$bad" ]]; then
  printf '%s\n' "$bad" >&2
  fail "digit templates are referenced outside include_bytes"
fi
printf '%s\n' "$hits" | grep -q 'include_bytes!' || fail "templates are not embedded with include_bytes"
pass "digit templates are include_bytes only"

if grep -R -n -E 'fs::(read|read_to_string|File::open)|std::fs::(read|read_to_string|File::open)' "$ROOT/src/shadow" | grep -E 'shadow-digits|digits_1440|digits_1080'; then
  fail "shadow code opens digit templates from disk"
fi
pass "shadow code does not open digit templates from disk"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
bash "$STAGE" "$TMP/assets"
for f in scuffed-stat-tracker.desktop scuffed-stat-tracker.service scuffed-stat-tracker-session.service; do
  [[ -f "$TMP/assets/$f" ]] || fail "missing staged $f"
done
if find "$TMP/assets" \( -name '*.png' -o -name 'shadow-digits' \) | grep -q .; then
  find "$TMP/assets" -print >&2
  fail "stage contains a digit template"
fi
# Repo copies stay. The stage script must not delete the embedded sources.
[[ -f "$ROOT/assets/shadow-digits/digits_1440.png" ]] || fail "stage script removed digits_1440.png"
[[ -f "$ROOT/assets/shadow-digits/digits_1080.png" ]] || fail "stage script removed digits_1080.png"
pass "staged assets are the three installer files"

tar -C "$TMP" -czf "$TMP/assets.tar.gz" assets
listing="$(tar -tzf "$TMP/assets.tar.gz")"
printf '%s\n' "$listing"
printf '%s\n' "$listing" | grep -q 'shadow-digits' && fail "tarball listing contains shadow-digits"
printf '%s\n' "$listing" | grep -q '\.png$' && fail "tarball listing contains a png"
pass "asset tarball listing has no digit templates"
