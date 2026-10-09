#!/usr/bin/env bash
# Print the stat-tracker GitHub Release body on stdout.
#
# Dry run (no gh, no tag push):
#   bash crates/stat-tracker/dist/release-notes.sh --tag stat-tracker-v0.4.24 --skip-commits
#
# The Install curl is pinned to the release tag, the same shape as
# pinned_install_command in crates/stat-tracker-ui/src/update.rs:
# the raw URL is the tag, and STAT_TRACKER_TAG is assigned on bash, not curl.
# bootstrap.sh on main still installs the newest stable release when that
# variable is unset. This script does not change that default.
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
TRACKER="$(cd "$DIST/.." && pwd)"
REPO="$(cd "$TRACKER/../.." && pwd)"

TAG=""
CHANGELOG=""
COMMITS_FILE=""
SKIP_COMMITS=0

while [[ $# -gt 0 ]]; do
  case "$1" in
    --tag)
      TAG="${2:?--tag needs a value}"
      shift 2
      ;;
    --changelog)
      CHANGELOG="${2:?--changelog needs a value}"
      shift 2
      ;;
    --commits-file)
      COMMITS_FILE="${2:?--commits-file needs a value}"
      shift 2
      ;;
    --skip-commits)
      SKIP_COMMITS=1
      shift
      ;;
    *)
      echo "unknown argument: $1" >&2
      exit 2
      ;;
  esac
done

[[ -n "$TAG" ]] || {
  echo "missing --tag" >&2
  exit 2
}
[[ -n "$CHANGELOG" ]] || CHANGELOG="$TRACKER/CHANGELOG.md"

NOTES_VERSION="${TAG#stat-tracker-v}"

# Stable stat-tracker-v* tags must have a changelog section. Release
# candidates (*-rc*) and other refs stay lenient: a missing section still
# prints the intro and footer.
require_section=0
if [[ "$TAG" == stat-tracker-v* && "$TAG" != *-rc* ]]; then
  require_section=1
fi

die_dash() {
  local label="$1"
  local text="$2"
  local kind="$3"
  local char="$4"
  echo "${label} contains ${kind}" >&2
  printf '%s\n' "$text" | grep -n "$char" >&2 || true
  exit 1
}

no_long_dash() {
  local label="$1"
  local text="$2"
  if [[ "$text" == *$'\u2014'* ]]; then
    die_dash "$label" "$text" "an em dash" $'\u2014'
  fi
  if [[ "$text" == *$'\u2013'* ]]; then
    die_dash "$label" "$text" "an en dash" $'\u2013'
  fi
}

# Commit subjects are historical and may contain em or en dashes (for
# example bf2bda0). The release body swaps those for a hyphen so the dash
# guard can cover the whole note, including the commit list.
flatten_subject_dashes() {
  local text="$1"
  text="${text//$'\u2014'/-}"
  text="${text//$'\u2013'/-}"
  printf '%s' "$text"
}

# Rewrite bootstrap one-liners onto this tag. Manual draft tags
# (stat-tracker-manual-*) are not git refs that host bootstrap.sh, so
# those bodies are left as written in the changelog.
pin_install_curls() {
  local tag="$1"
  local text="$2"
  if [[ "$tag" != stat-tracker-v* ]]; then
    printf '%s\n' "$text"
    return
  fi
  local url="https://raw.githubusercontent.com/FrozenTear/scuffed-crew/${tag}/crates/stat-tracker/dist/bootstrap.sh"
  local pinned="curl --proto '=https' -fsSL ${url} | STAT_TRACKER_TAG=${tag} bash"
  printf '%s\n' "$text" | sed -E \
    "s#^([[:space:]]*)curl (--proto '=https' )?-fsSL https://raw\\.githubusercontent\\.com/FrozenTear/scuffed-crew/(main|stat-tracker-v[^/[:space:]]+)/crates/stat-tracker/dist/bootstrap\\.sh \\| (STAT_TRACKER_TAG=[^[:space:]]+ )?bash[[:space:]]*\$#\\1${pinned}#"
}

CURATED=""
if [[ -f "$CHANGELOG" ]]; then
  CURATED="$(awk -v ver="${NOTES_VERSION}" '
    $0 == "## " ver {p=1; next}
    /^## / && p {exit}
    p
  ' "$CHANGELOG")"
elif [[ "$require_section" -eq 1 ]]; then
  echo "changelog not found: ${CHANGELOG} (required for ${TAG})" >&2
  exit 1
fi

if [[ -n "$(printf '%s' "${CURATED}" | tr -d '[:space:]')" ]]; then
  CURATED="$(pin_install_curls "$TAG" "$CURATED")"
  no_long_dash "changelog section ${NOTES_VERSION}" "$CURATED"
elif [[ "$require_section" -eq 1 ]]; then
  echo "changelog has no ## ${NOTES_VERSION} section (required for ${TAG})" >&2
  exit 1
fi

FOOTER="$(cat <<'EOF'
## Requirements & install

**Daemon:** runs on any current systemd distro (glibc ≥ 2.35); OCR libraries are bundled.

**GUI:** Iced 0.14 desktop app (`stat-tracker-gui` from `scuffed-stat-tracker-ui`). Needs GTK 3, a Vulkan-capable GPU/compositor (or software fallback), and glibc ≥ 2.35. Reinstall to replace an old Dioxus binary; the desktop-entry and PATH name stay `stat-tracker-gui`.

**Also required:** Linux + Wayland (wlr-screencopy compositor or XDG portal), or X11 (experimental, auto-detected; force with `STAT_TRACKER_CAPTURE=x11`), plus membership in the `input` group. OCR models (`eng` + game-font `koverwatch`) are bundled.

**Install:** extract the tarball and run `./install.sh` inside it. No Rust toolchain needed. Existing Dioxus installs: run the same installer (or `crates/stat-tracker/install.sh` from a source checkout) to replace `stat-tracker-gui`.
EOF
)"
no_long_dash "requirements footer" "$FOOTER"

INTRO="Prebuilt Linux x86_64 build of the Overwatch 2 stat tracker (daemon + Iced GUI)."
no_long_dash "release intro" "$INTRO"

MAIN_BOOTSTRAP_URL="https://raw.githubusercontent.com/FrozenTear/scuffed-crew/main/crates/stat-tracker/dist/bootstrap.sh"

COMMITS_BLOCK=""
if [[ "$SKIP_COMMITS" -eq 0 ]]; then
  if [[ -n "$COMMITS_FILE" ]]; then
    if [[ -s "$COMMITS_FILE" ]]; then
      subjects="$(flatten_subject_dashes "$(cat "$COMMITS_FILE")")"
      COMMITS_BLOCK="$(printf '## Commits\n\n%s' "$subjects")"
    fi
  else
    # Exclude the tag being released. On a tag push that tag is already local.
    PREV="$(git -C "$REPO" tag --list 'stat-tracker-v*' --sort=-creatordate \
      | grep -vx "${TAG}" | head -1 || true)"
    if [[ -n "${PREV}" ]]; then
      subjects="$(flatten_subject_dashes "$(git -C "$REPO" log "${PREV}..HEAD" --no-merges --pretty='- %s' -- \
        crates/stat-tracker crates/stat-tracker-ui \
        .github/workflows/stat-tracker-release.yml)")"
      COMMITS_BLOCK="$(printf '## Commits since %s\n\n%s' "${PREV#stat-tracker-}" "$subjects")"
    fi
  fi
fi

body="$(
  printf '%s\n\n' "$INTRO"
  if [[ -n "$(printf '%s' "${CURATED}" | tr -d '[:space:]')" ]]; then
    printf '%s\n\n' "${CURATED}"
  fi
  if [[ -n "$COMMITS_BLOCK" ]]; then
    printf '%s\n\n' "$COMMITS_BLOCK"
  fi
  printf '%s\n' "$FOOTER"
)"

# Covers the intro, changelog section, commit list, and footer.
no_long_dash "release notes" "$body"

# A stat-tracker-v* body that still names main's bootstrap.sh means the
# curl rewrite above did not match that line.
if [[ "$TAG" == stat-tracker-v* && "$body" == *"$MAIN_BOOTSTRAP_URL"* ]]; then
  echo "release notes still contain the unpinned main bootstrap.sh URL; Install curl was not pinned to ${TAG}" >&2
  exit 1
fi

printf '%s\n' "$body"
