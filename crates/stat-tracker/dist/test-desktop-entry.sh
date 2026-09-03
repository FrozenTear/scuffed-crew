#!/usr/bin/env bash
# Discriminating test: install.sh must write absolute Exec/TryExec.
# A bare Exec=stat-tracker-gui (v0.4.2 and earlier) fails this script.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
INSTALL="$ROOT/dist/install.sh"
TEMPLATE="$ROOT/assets/scuffed-stat-tracker.desktop"
UNIT="$ROOT/assets/scuffed-stat-tracker.service"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*" >&2; }

[[ -f "$INSTALL" ]] || fail "missing $INSTALL"
[[ -f "$TEMPLATE" ]] || fail "missing $TEMPLATE"
[[ -f "$UNIT" ]] || fail "missing $UNIT"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PKG="$TMP/pkg"
HOME_DIR="$TMP/home"
PREFIX="$TMP/prefix"
mkdir -p "$PKG/bin" "$PKG/assets" "$HOME_DIR" "$PREFIX"

# Stub binaries: install.sh requires them and runs daemon --version.
printf '%s\n' '#!/bin/sh' 'echo scuffed-stat-tracker 0.4.3' > "$PKG/bin/scuffed-stat-tracker"
printf '%s\n' '#!/bin/sh' 'echo stat-tracker-gui' > "$PKG/bin/stat-tracker-gui"
chmod +x "$PKG/bin/scuffed-stat-tracker" "$PKG/bin/stat-tracker-gui"
cp "$TEMPLATE" "$UNIT" "$PKG/assets/"
cp "$INSTALL" "$PKG/install.sh"
chmod +x "$PKG/install.sh"

HOME="$HOME_DIR" PREFIX="$PREFIX" bash "$PKG/install.sh"

DESKTOP="$HOME_DIR/.local/share/applications/scuffed-stat-tracker.desktop"
[[ -f "$DESKTOP" ]] || fail "desktop entry not installed at $DESKTOP"

want="$PREFIX/bin/stat-tracker-gui"
[[ -x "$want" ]] || fail "gui binary not installed at $want"

exec_line="$(grep -E '^Exec=' "$DESKTOP" || true)"
try_line="$(grep -E '^TryExec=' "$DESKTOP" || true)"

[[ "$exec_line" == "Exec=$want" ]] || fail "Exec not absolute. got: ${exec_line:-<missing>} want: Exec=$want"
[[ "$try_line" == "TryExec=$want" ]] || fail "TryExec not absolute. got: ${try_line:-<missing>} want: TryExec=$want"

# Bare name must not appear as the Exec/TryExec value.
if grep -Eq '^(Exec|TryExec)=stat-tracker-gui[[:space:]]*$' "$DESKTOP"; then
    fail "bare Exec/TryExec=stat-tracker-gui still present"
fi

# Theme icon (not a broken file path).
grep -q '^Icon=applications-games$' "$DESKTOP" || fail "Icon= line missing or rewritten"

pass "Exec=$want"
pass "TryExec=$want"

# Relative PREFIX must still expand to an absolute Exec (launcher PATH fix).
REL_HOME="$TMP/home-rel"
mkdir -p "$REL_HOME"
(
    cd "$TMP"
    HOME="$REL_HOME" PREFIX="rel-prefix" bash "$PKG/install.sh"
)
REL_DESKTOP="$REL_HOME/.local/share/applications/scuffed-stat-tracker.desktop"
rel_abs="$TMP/rel-prefix/bin/stat-tracker-gui"
[[ -f "$REL_DESKTOP" ]] || fail "relative-PREFIX desktop entry missing"
rel_exec="$(grep -E '^Exec=' "$REL_DESKTOP" || true)"
rel_try="$(grep -E '^TryExec=' "$REL_DESKTOP" || true)"
[[ "$rel_exec" == "Exec=$rel_abs" ]] || fail "relative PREFIX Exec not absolute. got: ${rel_exec:-<missing>}"
[[ "$rel_try" == "TryExec=$rel_abs" ]] || fail "relative PREFIX TryExec not absolute. got: ${rel_try:-<missing>}"
[[ "$rel_exec" == /* || "$rel_exec" == Exec=/* ]] || fail "Exec is not an absolute path: $rel_exec"
pass "relative PREFIX Exec=$rel_abs"

echo "$DESKTOP"
