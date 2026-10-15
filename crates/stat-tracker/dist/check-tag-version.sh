#!/usr/bin/env bash
# Fail when a pushed stat-tracker-v* tag does not match the package version.
#
# crates/stat-tracker/Cargo.toml is the release version (what --version prints
# when SST_RELEASE_VERSION is unset, and what the GUI prints).
# crates/stat-tracker-ui is versioned separately. Its Cargo.toml stays at
# 0.1.0 on purpose: the GUI does not publish that number. Any other UI
# version must equal the tag, so a half-bumped UI crate fails the job.
#
# Usage: check-tag-version.sh [stat-tracker-vX]
# Tag defaults to GITHUB_REF_NAME. Override the manifests with
# STAT_TRACKER_CARGO_TOML and STAT_TRACKER_UI_CARGO_TOML.
set -euo pipefail

fail() { echo "tag version check: $*" >&2; exit 1; }

package_version() {
    local file="$1"
    [[ -f "$file" ]] || fail "missing $file"
    awk '
        /^\[package\]/ { in_pkg = 1; next }
        /^\[/ { in_pkg = 0 }
        in_pkg && /^version[[:space:]]*=/ {
            if (match($0, /"[^"]+"/)) {
                print substr($0, RSTART + 1, RLENGTH - 2)
                found = 1
                exit
            }
        }
        END { if (!found) exit 2 }
    ' "$file" || fail "no [package] version in $file"
}

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)"
DAEMON_TOML="${STAT_TRACKER_CARGO_TOML:-$ROOT/crates/stat-tracker/Cargo.toml}"
UI_TOML="${STAT_TRACKER_UI_CARGO_TOML:-$ROOT/crates/stat-tracker-ui/Cargo.toml}"

ref="${1:-${GITHUB_REF_NAME:-}}"
[[ -n "$ref" ]] || fail "no tag (pass stat-tracker-vX or set GITHUB_REF_NAME)"
if [[ "$ref" == stat-tracker-v* ]]; then
    tag="${ref#stat-tracker-v}"
else
    tag="$ref"
fi
[[ -n "$tag" ]] || fail "empty version in $ref"

daemon="$(package_version "$DAEMON_TOML")" || exit 1
ui="$(package_version "$UI_TOML")" || exit 1
[[ -n "$daemon" ]] || fail "empty version in $DAEMON_TOML"
[[ -n "$ui" ]] || fail "empty version in $UI_TOML"

echo "tag=${tag} daemon=${daemon} ui=${ui}"

if [[ "$tag" != "$daemon" ]]; then
    fail "tag ${tag} does not match crates/stat-tracker/Cargo.toml version ${daemon}"
fi

# 0.1.0 is the UI crate's fixed version, not a release. Anything else is a
# separately versioned release and must match the tag.
if [[ "$ui" != "0.1.0" && "$ui" != "$tag" ]]; then
    fail "tag ${tag} does not match crates/stat-tracker-ui/Cargo.toml version ${ui}"
fi

echo "tag version matches Cargo.toml"
