#!/usr/bin/env bash
# The release tag must match the daemon Cargo.toml version. A mismatched
# tag fails. stat-tracker-core must carry the same version. The UI crate
# stays at 0.1.0 unless it is versioned as the release.
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
ROOT="$(cd "$DIST/../../.." && pwd)"
CHECK="$DIST/check-tag-version.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*"; }

[[ -f "$CHECK" ]] || fail "missing $CHECK"
[[ -x "$CHECK" ]] || chmod +x "$CHECK"

daemon_ver="$(
    STAT_TRACKER_CARGO_TOML="$ROOT/crates/stat-tracker/Cargo.toml" \
        awk '
            /^\[package\]/ { in_pkg = 1; next }
            /^\[/ { in_pkg = 0 }
            in_pkg && /^version[[:space:]]*=/ {
                if (match($0, /"[^"]+"/)) {
                    print substr($0, RSTART + 1, RLENGTH - 2)
                    exit
                }
            }
        ' "$ROOT/crates/stat-tracker/Cargo.toml"
)"
[[ -n "$daemon_ver" ]] || fail "could not read daemon version"
ui_ver="$(
    awk '
        /^\[package\]/ { in_pkg = 1; next }
        /^\[/ { in_pkg = 0 }
        in_pkg && /^version[[:space:]]*=/ {
            if (match($0, /"[^"]+"/)) {
                print substr($0, RSTART + 1, RLENGTH - 2)
                exit
            }
        }
    ' "$ROOT/crates/stat-tracker-ui/Cargo.toml"
)"
[[ "$ui_ver" == "0.1.0" ]] || fail "UI crate version changed from 0.1.0 (got ${ui_ver}); update this test if that is intentional"

if ! bash "$CHECK" "stat-tracker-v${daemon_ver}" >/dev/null; then
    fail "current daemon version ${daemon_ver} was rejected"
fi
pass "tag stat-tracker-v${daemon_ver} matches the daemon crate"

if bash "$CHECK" "stat-tracker-v0.0.0-mismatch" >/dev/null 2>&1; then
    fail "a mismatched tag was accepted"
fi
pass "mismatched tag is rejected"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

write_toml() {
    local dest="$1" version="$2"
    cat > "$dest" <<EOF
[package]
name = "example"
version = "${version}"
edition = "2024"

[dependencies]
serde = "1"
EOF
}

# run_pair TAG DAEMON UI [CORE]: CORE defaults to DAEMON.
run_pair() {
    local tag="$1" daemon="$2" ui="$3" core="${4:-$2}"
    write_toml "$TMP/daemon.toml" "$daemon"
    write_toml "$TMP/core.toml" "$core"
    write_toml "$TMP/ui.toml" "$ui"
    STAT_TRACKER_CARGO_TOML="$TMP/daemon.toml" \
        STAT_TRACKER_CORE_CARGO_TOML="$TMP/core.toml" \
        STAT_TRACKER_UI_CARGO_TOML="$TMP/ui.toml" \
        bash "$CHECK" "$tag"
}

if ! run_pair "stat-tracker-v1.2.3" "1.2.3" "0.1.0" >/dev/null; then
    fail "UI placeholder 0.1.0 was required to match the tag"
fi
pass "UI crate at 0.1.0 is not required to match the tag"

if ! run_pair "stat-tracker-v1.2.3" "1.2.3" "1.2.3" >/dev/null; then
    fail "UI version that matches the tag was rejected"
fi
pass "UI version that equals the tag is accepted"

if run_pair "stat-tracker-v1.2.3" "1.2.3" "9.9.9" >/dev/null 2>&1; then
    fail "UI version 9.9.9 was accepted for tag 1.2.3"
fi
pass "a separately versioned UI crate must match the tag"

if run_pair "stat-tracker-v1.2.3" "9.9.9" "0.1.0" >/dev/null 2>&1; then
    fail "daemon version 9.9.9 was accepted for tag 1.2.3"
fi
pass "daemon Cargo.toml mismatch is rejected"

if run_pair "stat-tracker-v1.2.3" "1.2.3" "0.1.0" "1.2.2" >/dev/null 2>&1; then
    fail "stat-tracker-core 1.2.2 was accepted for release 1.2.3"
fi
pass "a stat-tracker-core version behind the release is rejected"

if ! run_pair "stat-tracker-v1.2.3" "1.2.3" "0.1.0" "1.2.3" >/dev/null; then
    fail "stat-tracker-core at the release version was rejected"
fi
pass "stat-tracker-core at the release version is accepted"

if ! run_pair "stat-tracker-v0.5.0-alpha.10" "0.5.0-alpha.10" "0.1.0" >/dev/null; then
    fail "tag stat-tracker-v0.5.0-alpha.10 was rejected when Cargo.toml is 0.5.0-alpha.10"
fi
pass "tag stat-tracker-v0.5.0-alpha.10 matches Cargo.toml 0.5.0-alpha.10"

if run_pair "stat-tracker-v0.5.0-alpha.10" "0.5.0-alpha.1" "0.1.0" >/dev/null 2>&1; then
    fail "tag alpha.10 was accepted for Cargo.toml alpha.1"
fi
pass "tag alpha.10 does not match Cargo.toml alpha.1"

if run_pair "stat-tracker-v0.5.0-alpha.1" "0.5.0-alpha.10" "0.1.0" >/dev/null 2>&1; then
    fail "tag alpha.1 was accepted for Cargo.toml alpha.10"
fi
pass "tag alpha.1 does not match Cargo.toml alpha.10"

if env -u GITHUB_REF_NAME bash "$CHECK" >/dev/null 2>&1; then
    fail "missing tag was accepted"
fi
pass "missing tag is rejected"
