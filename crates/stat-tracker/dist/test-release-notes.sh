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

# Every stat-tracker-v* body has a terminal update pinned to that tag, so a
# player can still update when the in-app update fails. An older app shows
# the body inside its own Update offer, so the lead must point at the Update
# button first. Other tags (manual drafts) do not host bootstrap.sh and skip
# it.
term_out="$(bash "$NOTES" --tag stat-tracker-v9.9.9-rc7 --changelog "$TMP/empty.md" --skip-commits)"
printf '%s\n' "$term_out" | grep -qx '## Update from the terminal' \
  || fail "release notes are missing the Update from the terminal section"
printf '%s\n' "$term_out" | grep -qxF 'TAG=stat-tracker-v9.9.9-rc7' \
  || fail "terminal update is not pinned to the release tag"
want_term='curl --proto '"'"'=https'"'"' -fsSL "https://raw.githubusercontent.com/FrozenTear/scuffed-crew/${TAG}/crates/stat-tracker/dist/bootstrap.sh" | STAT_TRACKER_TAG="$TAG" STAT_TRACKER_PREFIX="$HOME/.local" bash'
printf '%s\n' "$term_out" | grep -qxF "$want_term" \
  || fail "terminal update curl line changed: $(printf '%s\n' "$term_out" | grep -F bootstrap.sh)"
printf '%s\n' "$term_out" | grep -qF 'About shows 9.9.9-rc7.' \
  || fail "terminal update does not name the version About shows"
printf '%s\n' "$term_out" | grep -qF 'This is for the tarball install.' \
  || fail "terminal update does not say it is for the tarball install"
printf '%s\n' "$term_out" | grep -qF 'Prefer the Update button in the app. If it fails or does not offer this release, close the app and paste this in a terminal.' \
  || fail "terminal update lead does not put the Update button first: $(printf '%s\n' "$term_out" | grep -F 'tarball install')"
printf '%s\n' "$term_out" | grep -qF 'If the Update button in the app does not offer this release' \
  && fail "terminal update still leads with the button not offering the release"
printf '%s\n' "$term_out" | grep -qF 'If you installed from the AUR, update with your AUR helper instead' \
  || fail "terminal update does not send AUR users to their AUR helper"
manual_out="$(bash "$NOTES" --tag stat-tracker-manual-9 --changelog "$TMP/empty.md" --skip-commits)"
printf '%s\n' "$manual_out" | grep -q 'Update from the terminal' \
  && fail "a manual draft tag got a terminal update"
pass "stat-tracker-v* notes carry a pinned terminal update; manual tags do not"

# Internal merge-gate markers in PR titles must not reach members. A leading
# "HOLD-MERGE:" or "[HOLD-MERGE]" and a "(HOLD-MERGE)" anywhere are dropped;
# the rest of the subject, and a HOLD-MERGE that is not a marker, stay.
{
  printf -- '- HOLD-MERGE: stat-tracker 0.5.0-rc.5 (service restart) (#415)\n'
  printf -- '- [HOLD-MERGE] app: stats tracker notice (#366)\n'
  printf -- '- stat-tracker: bundle Tesseract 5 in release builds (HOLD-MERGE) (#360)\n'
  printf -- '- HOLD-MERGE:no space after the colon\n'
  printf -- '- docs: explain the HOLD-MERGE: convention\n'
  printf -- '- plain subject (#1)\n'
} > "$TMP/hold-commits.txt"
hold_out="$(bash "$NOTES" --tag stat-tracker-v7.7.7 --changelog "$TMP/commits-changelog.md" --commits-file "$TMP/hold-commits.txt")"
hold_list="$(printf '%s\n' "$hold_out" | awk '/^## Commits$/ {p=1; next} /^## / && p {exit} p && NF')"
want_hold="$(cat <<'WANT'
- stat-tracker 0.5.0-rc.5 (service restart) (#415)
- app: stats tracker notice (#366)
- stat-tracker: bundle Tesseract 5 in release builds (#360)
- no space after the colon
- docs: explain the HOLD-MERGE: convention
- plain subject (#1)
WANT
)"
[[ "$hold_list" == "$want_hold" ]] \
  || fail "HOLD-MERGE markers were not stripped from the commit list:
$hold_list"
pass "commit list drops HOLD-MERGE markers and keeps the subject"

# The default path reads subjects from git log since the previous tag. Run
# the script from a copy inside a scratch repo so that path is covered too.
GITREPO="$TMP/repo"
mkdir -p "$GITREPO/crates/stat-tracker/dist"
cp "$NOTES" "$GITREPO/crates/stat-tracker/dist/release-notes.sh"
cp "$TMP/commits-changelog.md" "$GITREPO/crates/stat-tracker/CHANGELOG.md"
git_q() {
  git -C "$GITREPO" -c user.name=test -c user.email=test@example.invalid \
    -c commit.gpgsign=false -c tag.gpgsign=false "$@" >/dev/null
}
git_q init -q
git_q add -A
git_q commit -q -m 'stat-tracker: base'
git_q tag stat-tracker-v7.7.6
echo a > "$GITREPO/crates/stat-tracker/a.txt"
git_q add -A
git_q commit -q -m 'HOLD-MERGE: stat-tracker 7.7.7 (#2)'
echo b > "$GITREPO/crates/stat-tracker/b.txt"
git_q add -A
git_q commit -q -m 'stat-tracker: fix a thing (HOLD-MERGE) (#3)'
# A commit that touches only a crate split out of the tracker lib, one that
# touches only the GUI, and one outside the tracker.
mkdir -p "$GITREPO/crates/stat-tracker-core/src" "$GITREPO/crates/stat-tracker-ui/src" \
  "$GITREPO/crates/site-server/src"
echo c > "$GITREPO/crates/stat-tracker-core/src/c.rs"
git_q add -A
git_q commit -q -m 'stat-tracker-core: core-only change (#4)'
echo u > "$GITREPO/crates/stat-tracker-ui/src/u.rs"
git_q add -A
git_q commit -q -m 'stat-tracker-ui: gui-only change (#5)'
echo s > "$GITREPO/crates/site-server/src/s.rs"
git_q add -A
git_q commit -q -m 'site-server: not a tracker change (#6)'
git_out="$(bash "$GITREPO/crates/stat-tracker/dist/release-notes.sh" --tag stat-tracker-v7.7.7)"
printf '%s\n' "$git_out" | grep -qxF -- '- stat-tracker-core: core-only change (#4)' \
  || fail "a commit touching only stat-tracker-core is missing: $git_out"
printf '%s\n' "$git_out" | grep -qxF -- '- stat-tracker-ui: gui-only change (#5)' \
  || fail "a commit touching only the GUI is missing: $git_out"
printf '%s\n' "$git_out" | grep -qF 'site-server: not a tracker change' \
  && fail "a commit outside the tracker crates is listed: $git_out"
pass "commits since the previous tag cover every crates/stat-tracker* crate, and only those"
printf '%s\n' "$git_out" | grep -qx '## Commits since v7.7.6' \
  || fail "git log path did not print a commit list: $git_out"
printf '%s\n' "$git_out" | grep -qxF -- '- stat-tracker 7.7.7 (#2)' \
  || fail "git log path kept the HOLD-MERGE: prefix: $git_out"
printf '%s\n' "$git_out" | grep -qxF -- '- stat-tracker: fix a thing (#3)' \
  || fail "git log path kept the (HOLD-MERGE) marker: $git_out"
printf '%s\n' "$git_out" | grep -q 'HOLD-MERGE' \
  && fail "git log path still contains HOLD-MERGE: $git_out"
pass "commits since the previous tag drop HOLD-MERGE markers"

# A stable tag lists the commits since the previous stable tag, not since
# the rc tags cut between the two stables. An rc tag still lists the
# commits since the newest tag of any kind. Own repo, with fixed dates, so
# the tag order by creatordate does not depend on the clock.
STABLE_REPO="$TMP/stable-repo"
mkdir -p "$STABLE_REPO/crates/stat-tracker/dist"
cp "$NOTES" "$STABLE_REPO/crates/stat-tracker/dist/release-notes.sh"
printf '%s\n' '## 7.9.0' '' 'Stable summary.' '' '## 7.9.0-rc.2' '' 'Rc summary.' \
  > "$STABLE_REPO/crates/stat-tracker/CHANGELOG.md"
sgit() {
  git -C "$STABLE_REPO" -c user.name=test -c user.email=test@example.invalid \
    -c commit.gpgsign=false -c tag.gpgsign=false "$@" >/dev/null
}
dated_commit() {
  local when="$1" name="$2" msg="$3"
  echo "$name" > "$STABLE_REPO/crates/stat-tracker/$name.txt"
  sgit add -A
  GIT_AUTHOR_DATE="$when" GIT_COMMITTER_DATE="$when" sgit commit -q -m "$msg"
}
dated_tag() {
  GIT_COMMITTER_DATE="$1" sgit tag -a -m "$2" "$2"
}
sgit init -q
dated_commit 2026-01-01T00:00:00Z base 'stat-tracker: base (#9)'
dated_tag 2026-01-01T00:00:01Z stat-tracker-v7.8.0
dated_commit 2026-01-02T00:00:00Z c1 'stat-tracker: before rc.1 (#10)'
dated_tag 2026-01-02T00:00:01Z stat-tracker-v7.9.0-rc.1
dated_commit 2026-01-03T00:00:00Z c2 'stat-tracker: before rc.2 (#11)'
dated_tag 2026-01-03T00:00:01Z stat-tracker-v7.9.0-rc.2
dated_commit 2026-01-04T00:00:00Z c3 'stat-tracker: after rc.2 (#12)'
dated_tag 2026-01-04T00:00:01Z stat-tracker-v7.9.0
stable_out="$(bash "$STABLE_REPO/crates/stat-tracker/dist/release-notes.sh" --tag stat-tracker-v7.9.0)"
printf '%s\n' "$stable_out" | grep -qx '## Commits since v7.8.0' \
  || fail "stable notes did not list commits since the previous stable: $stable_out"
for subject in 'before rc.1 (#10)' 'before rc.2 (#11)' 'after rc.2 (#12)'; do
  printf '%s\n' "$stable_out" | grep -qxF -- "- stat-tracker: $subject" \
    || fail "stable notes lost '$subject': $stable_out"
done
printf '%s\n' "$stable_out" | grep -qF -- '- stat-tracker: base (#9)' \
  && fail "stable notes listed a commit from before the previous stable: $stable_out"
printf '%s\n' "$stable_out" | grep -q 'Commits since v7.9.0-rc' \
  && fail "stable notes listed commits since an rc: $stable_out"
rc_out="$(bash "$STABLE_REPO/crates/stat-tracker/dist/release-notes.sh" --tag stat-tracker-v7.9.0-rc.2)"
printf '%s\n' "$rc_out" | grep -qx '## Commits since v7.9.0' \
  || fail "rc notes did not list commits since the newest tag: $rc_out"
pass "a stable tag lists commits since the previous stable, past the rc tags"
