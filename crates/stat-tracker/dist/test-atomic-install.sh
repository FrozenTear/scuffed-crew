#!/usr/bin/env bash
# Binary replace must be a temp file + rename. GNU install(1) truncates the
# live inode when the destination is not busy; a crash then leaves a partial
# ELF, and the GUI must not start that. This fails if install.sh goes back to
# `install -m755` onto the destination path.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
INSTALL="$DIST/install.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*" >&2; }

[[ -f "$INSTALL" ]] || fail "missing $INSTALL"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

# ── function: failed copy does not touch the live file ───────────────────────

awk '/^# >>> atomic_install$/,/^# <<< atomic_install$/' "$INSTALL" > "$TMP/atomic.sh"
# shellcheck disable=SC1091
source "$TMP/atomic.sh"
error() { echo "$*" >&2; }

printf 'OLD\n' > "$TMP/live"
inode_before="$(stat -c %i "$TMP/live")"
mkdir -p "$TMP/not-a-file"
if atomic_install "$TMP/not-a-file" "$TMP/live" 755; then
    fail "atomic_install accepted a directory"
fi
[[ "$(cat "$TMP/live")" == "OLD" ]] || fail "failed install changed live bytes"
[[ "$(stat -c %i "$TMP/live")" == "$inode_before" ]] || fail "failed install replaced the live inode"
leftover="$(find "$(dirname "$TMP/live")" -name '.live.*' | wc -l)"
[[ "$leftover" -eq 0 ]] || fail "failed install left a temp file"
pass "failed copy leaves the live file unchanged"

# ── installer: second install gets a new inode (not an in-place truncate) ───

PKG="$TMP/pkg"
PREFIX="$TMP/prefix"
HOME_DIR="$TMP/home"
mkdir -p "$PKG/bin" "$PKG/assets" \
    "$PKG/lib/scuffed-stat-tracker/ocr" \
    "$PREFIX/bin" "$HOME_DIR"
cat > "$PKG/bin/scuffed-stat-tracker" << 'EOF'
#!/bin/sh
echo OLD-DAEMON
EOF
cat > "$PKG/bin/stat-tracker-gui" << 'EOF'
#!/bin/sh
echo OLD-GUI
EOF
chmod +x "$PKG/bin/scuffed-stat-tracker" "$PKG/bin/stat-tracker-gui"
printf 'old-lib\n' > "$PKG/lib/scuffed-stat-tracker/ocr/liblept.so.5"
printf '[Desktop Entry]\nName=test\n' > "$PKG/assets/scuffed-stat-tracker.desktop"
printf '[Unit]\nDescription=test\n' > "$PKG/assets/scuffed-stat-tracker.service"
cp "$INSTALL" "$PKG/install.sh"
cp "$DIST/uninstall.sh" "$PKG/uninstall.sh"

HOME="$HOME_DIR" PREFIX="$PREFIX" SKIP_INTEGRATION=1 \
    "$PKG/install.sh" >/dev/null

daemon="$PREFIX/bin/scuffed-stat-tracker"
gui="$PREFIX/bin/stat-tracker-gui"
lib="$PREFIX/lib/scuffed-stat-tracker/ocr/liblept.so.5"
[[ -x "$daemon" && -x "$gui" && -f "$lib" ]] || fail "first install did not land binaries"
inode_daemon="$(stat -c %i "$daemon")"
inode_lib="$(stat -c %i "$lib")"
[[ "$(stat -c %a "$daemon")" == "755" ]] || fail "daemon mode is $(stat -c %a "$daemon")"

cat > "$PKG/bin/scuffed-stat-tracker" << 'EOF'
#!/bin/sh
echo NEW-DAEMON
EOF
cat > "$PKG/bin/stat-tracker-gui" << 'EOF'
#!/bin/sh
echo NEW-GUI
EOF
chmod +x "$PKG/bin/scuffed-stat-tracker" "$PKG/bin/stat-tracker-gui"
printf 'new-lib\n' > "$PKG/lib/scuffed-stat-tracker/ocr/liblept.so.5"

HOME="$HOME_DIR" PREFIX="$PREFIX" SKIP_INTEGRATION=1 \
    "$PKG/install.sh" >/dev/null

[[ "$(stat -c %i "$daemon")" != "$inode_daemon" ]] \
    || fail "daemon was overwritten in place (inode unchanged)"
[[ "$(stat -c %i "$lib")" != "$inode_lib" ]] \
    || fail "bundled lib was overwritten in place (inode unchanged)"
[[ "$("$daemon")" == "NEW-DAEMON" ]] || fail "daemon contents were not replaced"
[[ "$("$gui")" == "NEW-GUI" ]] || fail "gui contents were not replaced"
[[ "$(cat "$lib")" == "new-lib" ]] || fail "lib contents were not replaced"
find "$PREFIX" -name '.scuffed-stat-tracker.*' -o -name '.liblept.so.5.*' -o -name '.stat-tracker-gui.*' \
    | grep -q . && fail "rename left a temp file" || true
pass "reinstall replaces binaries by rename"

echo "All atomic install checks passed."
