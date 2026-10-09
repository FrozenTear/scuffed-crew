#!/usr/bin/env bash
# The release body pins bootstrap.sh to the tag and checks em dashes and en dashes.
# bootstrap.sh on main remains the fresh-install entrypoint.
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIST/.." && pwd)"
NOTES="$DIST/release-notes.sh"
CHANGELOG="$ROOT/CHANGELOG.md"
BOOTSTRAP="$DIST/bootstrap.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }

[[ -f "$NOTES" ]] || fail "missing $NOTES"
[[ -f "$CHANGELOG" ]] || fail "missing $CHANGELOG"

PINNED_024='curl --proto '"'"'=https'"'"' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.4.24/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.4.24 bash'
MAIN_URL='https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh'

# Fresh install still curls main. An unset STAT_TRACKER_TAG installs the
# newest stable release. This file must keep that entrypoint.
grep -F "$MAIN_URL | bash" "$BOOTSTRAP" >/dev/null \
  || fail "bootstrap.sh lost the main fresh-install entrypoint"
pass "bootstrap.sh still documents the main fresh-install entrypoint"

# 0.4.24 in the changelog file itself, not only after generation.
section_024="$(awk '
  $0 == "## 0.4.24" {p=1; next}
  /^## / && p {exit}
  p
' "$CHANGELOG")"
printf '%s\n' "$section_024" | grep -F "$PINNED_024" >/dev/null \
  || fail "CHANGELOG 0.4.24 Install block is not pinned to the tag"
printf '%s\n' "$section_024" | grep -F "$MAIN_URL" >/dev/null \
  && fail "CHANGELOG 0.4.24 still fetches bootstrap.sh from main"
pass "CHANGELOG 0.4.24 Install curl pins the tag"

# Older sections stay as published. 0.4.23 still records the main curl.
section_023="$(awk '
  $0 == "## 0.4.23" {p=1; next}
  /^## / && p {exit}
  p
' "$CHANGELOG")"
printf '%s\n' "$section_023" | grep -F "$MAIN_URL" >/dev/null \
  || fail "CHANGELOG 0.4.23 was rewritten"
pass "CHANGELOG 0.4.23 Install block was left unchanged"

body="$(bash "$NOTES" --tag stat-tracker-v0.4.24 --skip-commits)"
printf '%s\n' "$body" | grep -F "$PINNED_024" >/dev/null \
  || fail "generated 0.4.24 notes missing the pinned curl"
printf '%s\n' "$body" | grep -F "$MAIN_URL" >/dev/null \
  && fail "generated 0.4.24 notes still fetch bootstrap.sh from main"
printf '%s\n' "$body" | grep -q $'\u2014' \
  && fail "generated 0.4.24 notes contain an em dash"
printf '%s\n' "$body" | grep -F 'portal), or X11' >/dev/null \
  || fail "Also required line was not rewritten"
printf '%s\n' "$body" | grep -F 'inside it. No Rust toolchain needed.' >/dev/null \
  || fail "Install footer line was not rewritten"
pass "generated 0.4.24 body pins the tag and has no em dash"

# A changelog that still has the main curl is pinned at generation time.
# An already-pinned older tag in that same line is rewritten to the tag
# being released.
TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

cat > "$TMP/from-main.md" <<'EOF'
## 9.9.9

Summary without a long dash.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh | bash
```
EOF

from_main="$(bash "$NOTES" --tag stat-tracker-v9.9.9 --changelog "$TMP/from-main.md" --skip-commits)"
want_999="curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v9.9.9/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v9.9.9 bash"
printf '%s\n' "$from_main" | grep -F "$want_999" >/dev/null \
  || fail "generator left the main curl in place: $from_main"
printf '%s\n' "$from_main" | grep -F "$MAIN_URL" >/dev/null \
  && fail "generator output still contains the main URL"
pass "generator rewrites a main Install curl onto the tag"

cat > "$TMP/from-old-tag.md" <<'EOF'
## 9.9.9

Summary.

### Install

```sh
curl -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v9.9.8/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v9.9.8 bash
```
EOF

from_old="$(bash "$NOTES" --tag stat-tracker-v9.9.9 --changelog "$TMP/from-old-tag.md" --skip-commits)"
printf '%s\n' "$from_old" | grep -F "$want_999" >/dev/null \
  || fail "generator kept an older tag in the Install curl"
printf '%s\n' "$from_old" | grep -F 'stat-tracker-v9.9.8' >/dev/null \
  && fail "older tag survived generation"
pass "generator rewrites an older pinned curl onto the tag being released"

# Rendering 0.4.23 from the unchanged file still pins that tag in the body.
body_023="$(bash "$NOTES" --tag stat-tracker-v0.4.23 --skip-commits)"
want_023="curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.4.23/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.4.23 bash"
printf '%s\n' "$body_023" | grep -F "$want_023" >/dev/null \
  || fail "0.4.23 generated notes were not pinned"
printf '%s\n' "$body_023" | grep -F "$MAIN_URL" >/dev/null \
  && fail "0.4.23 generated notes still use main"
pass "generated 0.4.23 body pins its own tag without editing that section"

# A manual draft tag is not a git ref. Do not invent a raw URL for it.
cat > "$TMP/manual.md" <<EOF
## not-a-release

### Install

\`\`\`sh
curl --proto '=https' -fsSL ${MAIN_URL} | bash
\`\`\`
EOF
manual="$(bash "$NOTES" --tag not-a-release --changelog "$TMP/manual.md" --skip-commits)"
printf '%s\n' "$manual" | grep -F "$MAIN_URL" >/dev/null \
  || fail "non-release tag rewrote the changelog curl"
pass "non-release tags do not rewrite the Install curl"

{
  printf '%s\n' '## 1.2.3' ''
  printf 'Hello %sworld.\n' $'\u2014'
} > "$TMP/em.md"
: > "$TMP/empty.md"
set +e
bash "$NOTES" --tag stat-tracker-v1.2.3 --changelog "$TMP/em.md" --skip-commits >"$TMP/em.out" 2>"$TMP/em.err"
em_code=$?
set -e
[[ "$em_code" -ne 0 ]] || fail "em dash in the changelog section was accepted"
grep -q 'em dash' "$TMP/em.err" || fail "em dash failure did not say why"
pass "em dash in the curated section fails the notes build"

{
  printf '%s\n' '## 1.2.4' ''
  printf 'Hello %sworld.\n' $'\u2013'
} > "$TMP/en.md"
set +e
bash "$NOTES" --tag stat-tracker-v1.2.4 --changelog "$TMP/en.md" --skip-commits >"$TMP/en.out" 2>"$TMP/en.err"
en_code=$?
set -e
[[ "$en_code" -ne 0 ]] || fail "en dash in the changelog section was accepted"
grep -q 'en dash' "$TMP/en.err" || fail "en dash failure did not say why"
pass "en dash in the curated section fails the notes build"

# A stable tag with no matching section must not publish intro+footer only.
set +e
missing_out="$(bash "$NOTES" --tag stat-tracker-v9.9.9 --changelog "$TMP/empty.md" --skip-commits 2>"$TMP/missing.err")"
missing_code=$?
set -e
[[ "$missing_code" -ne 0 ]] || fail "stable tag with no changelog section exited 0"
[[ -z "$missing_out" ]] || fail "stable tag with no section still printed notes"
grep -q 'no ## 9.9.9 section' "$TMP/missing.err" \
  || fail "missing-section error was: $(cat "$TMP/missing.err")"
pass "stable tag with no changelog section exits 1"

set +e
nofile_out="$(bash "$NOTES" --tag stat-tracker-v9.9.9 --changelog "$TMP/does-not-exist.md" --skip-commits 2>"$TMP/nofile.err")"
nofile_code=$?
set -e
[[ "$nofile_code" -ne 0 ]] || fail "stable tag with a missing changelog file exited 0"
[[ -z "$nofile_out" ]] || fail "missing changelog file still printed notes"
grep -q 'changelog not found' "$TMP/nofile.err" \
  || fail "missing-file error was: $(cat "$TMP/nofile.err")"
pass "stable tag with a missing changelog file exits 1"

# Release candidates stay lenient when the section is absent.
rc="$(bash "$NOTES" --tag stat-tracker-v9.9.9-rc1 --changelog "$TMP/empty.md" --skip-commits)"
printf '%s\n' "$rc" | grep -F '## Requirements & install' >/dev/null \
  || fail "rc tag did not print the footer"
pass "rc tag with no changelog section still prints the footer"

# Alpha is not an rc. A missing section still fails the notes build.
set +e
alpha_missing="$(bash "$NOTES" --tag stat-tracker-v9.9.9-alpha.1 --changelog "$TMP/empty.md" --skip-commits 2>"$TMP/alpha-missing.err")"
alpha_missing_code=$?
set -e
[[ "$alpha_missing_code" -ne 0 ]] || fail "alpha tag with no changelog section exited 0"
[[ -z "$alpha_missing" ]] || fail "alpha tag with no section still printed notes"
grep -q 'no ## 9.9.9-alpha.1 section' "$TMP/alpha-missing.err" \
  || fail "alpha missing-section error was: $(cat "$TMP/alpha-missing.err")"
pass "alpha tag with no changelog section exits 1"

PINNED_ALPHA='curl --proto '"'"'=https'"'"' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v0.5.0-alpha.1/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v0.5.0-alpha.1 bash'
alpha_body="$(bash "$NOTES" --tag stat-tracker-v0.5.0-alpha.1 --skip-commits)"
printf '%s\n' "$alpha_body" | grep -F "$PINNED_ALPHA" >/dev/null \
  || fail "generated 0.5.0-alpha.1 notes missing the pinned curl"
printf '%s\n' "$alpha_body" | grep -F "$MAIN_URL" >/dev/null \
  && fail "generated 0.5.0-alpha.1 notes fetch bootstrap.sh from main"
printf '%s\n' "$alpha_body" | grep -q $'\u2014' \
  && fail "generated 0.5.0-alpha.1 notes contain an em dash"
printf '%s\n' "$alpha_body" | grep -q $'\u2013' \
  && fail "generated 0.5.0-alpha.1 notes contain an en dash"
pass "generated 0.5.0-alpha.1 body pins the tag"

# The footer itself has neither dash. An rc tag is the lenient path that
# still renders that footer when the changelog section is absent.
printf '%s\n' "$rc" | grep -q $'\u2014' && fail "footer contains an em dash"
printf '%s\n' "$rc" | grep -q $'\u2013' && fail "footer contains an en dash"
pass "footer has no em dash or en dash"

# A curl the rewriter does not recognize must not ship the main URL.
cat > "$TMP/unpinned.md" <<EOF
## 8.8.8

### Install

curl -fL ${MAIN_URL} | bash
EOF
set +e
unpinned_out="$(bash "$NOTES" --tag stat-tracker-v8.8.8 --changelog "$TMP/unpinned.md" --skip-commits 2>"$TMP/unpinned.err")"
unpinned_code=$?
set -e
[[ "$unpinned_code" -ne 0 ]] || fail "unpinned main bootstrap URL was accepted"
[[ -z "$unpinned_out" ]] || fail "unpinned URL was written to stdout"
grep -q 'unpinned main bootstrap.sh URL' "$TMP/unpinned.err" \
  || fail "unpinned URL error was: $(cat "$TMP/unpinned.err")"
pass "unpinned main bootstrap URL fails the notes build"

# Commit subjects may contain em or en dashes. The generated list swaps
# them for a hyphen, and the dash guard then accepts the whole note.
cat > "$TMP/commits-changelog.md" <<'EOF'
## 7.7.7

Summary.

### Install

```sh
curl --proto '=https' -fsSL https://raw.githubusercontent.com/FrozenTear/scuffed-crew/stat-tracker-v7.7.7/crates/stat-tracker/dist/bootstrap.sh | STAT_TRACKER_TAG=stat-tracker-v7.7.7 bash
```
EOF
{
  printf -- '- fix(stat-tracker-ui): P1 review %s win bars, wrap filters, session groups\n' $'\u2014'
  printf -- '- range %s end\n' $'\u2013'
} > "$TMP/commits.txt"
commits_out="$(bash "$NOTES" --tag stat-tracker-v7.7.7 --changelog "$TMP/commits-changelog.md" --commits-file "$TMP/commits.txt")"
printf '%s\n' "$commits_out" | grep -q $'\u2014' && fail "commit list kept an em dash"
printf '%s\n' "$commits_out" | grep -q $'\u2013' && fail "commit list kept an en dash"
printf '%s\n' "$commits_out" | grep -F -- '- fix(stat-tracker-ui): P1 review - win bars, wrap filters, session groups' >/dev/null \
  || fail "em dash in a commit subject was not replaced with a hyphen"
printf '%s\n' "$commits_out" | grep -F -- '- range - end' >/dev/null \
  || fail "en dash in a commit subject was not replaced with a hyphen"
pass "commit subjects replace em dashes and en dashes with a hyphen"

# An en dash in the footer is not a commit subject, so it is not rewritten.
# Plant it after the footer's own check. Only the whole-notes guard
# (no_long_dash "release notes") can reject it. Removing that guard makes
# this test fail.
grep -F 'no_long_dash "release notes" "$body"' "$NOTES" >/dev/null \
  || fail "whole-notes dash guard is missing"
cp "$NOTES" "$TMP/guard.sh"
python3 - "$TMP/guard.sh" <<'PY'
import pathlib, sys
path = pathlib.Path(sys.argv[1])
text = path.read_text()
needle = 'no_long_dash "requirements footer" "$FOOTER"\n'
extra = "FOOTER=\"${FOOTER}\"$'" + "\\u2013'\n"
if needle not in text:
    raise SystemExit("footer check anchor missing")
path.write_text(text.replace(needle, needle + extra, 1))
PY
set +e
guard_out="$(bash "$TMP/guard.sh" --tag stat-tracker-v9.9.9-rc1 --changelog "$TMP/empty.md" --skip-commits 2>"$TMP/guard.err")"
guard_code=$?
set -e
[[ "$guard_code" -ne 0 ]] || fail "en dash in the footer was accepted"
[[ -z "$guard_out" ]] || fail "footer en dash was written to stdout"
grep -q 'release notes contains an en dash' "$TMP/guard.err" \
  || fail "whole-notes guard did not report the en dash: $(cat "$TMP/guard.err")"
grep -q 'requirements footer contains an en dash' "$TMP/guard.err" \
  && fail "the footer check ran instead of the whole-notes guard"
pass "en dash in the footer exits 1 through the whole-notes guard"
