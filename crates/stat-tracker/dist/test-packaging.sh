#!/usr/bin/env bash
# Hermetic checks for the v0.4.1 OpenSSL-shadow packaging fix.
# Run from anywhere: bash crates/stat-tracker/dist/test-packaging.sh
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
BUNDLE="$DIST/bundle-native-libs.sh"
INSTALL="$DIST/install.sh"
UNINSTALL="$DIST/uninstall.sh"

# M19 / tracker lows: tag-pinned bootstrap, atomic binary replace, tar extract.
bash "$DIST/test-bootstrap-pin.sh"
bash "$DIST/test-atomic-install.sh"
bash "$DIST/test-safe-extract.sh"
# The tessdata check always runs; its wording follows `ocr` in config.toml.
bash "$DIST/test-ocr-off.sh"
# glibc floor comparator used by the release clean-room step.
bash "$DIST/check-bundled-glibc.sh" --expect-ok 2.35 2.35
bash "$DIST/check-bundled-glibc.sh" --expect-ok 2.34 2.35
bash "$DIST/check-bundled-glibc.sh" --expect-ok 2.2.5 2.35
bash "$DIST/check-bundled-glibc.sh" --expect-fail 2.36 2.35
bash "$DIST/check-bundled-glibc.sh" --expect-fail 2.35.1 2.35
echo "PASS  glibc floor comparator"
FAILS=0

pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*" >&2; FAILS=$((FAILS + 1)); }

# ── DENY regex: OpenSSL host-provided, OCR libs not denied ───────────────────

DENY="$("$BUNDLE" --print-deny)"
for name in libcrypto.so.3 libcrypto.so libssl.so.3 libssl.so.3.0.2; do
    if echo "$name" | grep -qE "$DENY"; then
        pass "deny matches $name"
    else
        fail "deny should match $name (DENY=$DENY)"
    fi
done
for name in liblept.so.5 libtesseract.so.4 libtesseract.so.5 libxdo.so.3 libpng16.so.16; do
    if echo "$name" | grep -qE "$DENY"; then
        fail "deny must not match OCR/GUI lib $name"
    else
        pass "deny leaves $name bundleable"
    fi
done

# ── install.sh: isolate libs, skip OpenSSL, clean v0.4.0 leftovers ───────────

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

PKG="$TMP/pkg"
PREFIX="$TMP/prefix"
HOME_DIR="$TMP/home"
mkdir -p "$PKG/bin" "$PKG/assets" \
    "$PKG/lib/scuffed-stat-tracker/ocr" \
    "$PKG/lib/scuffed-stat-tracker/gui" \
    "$PREFIX/bin" "$PREFIX/lib" \
    "$PREFIX/share/scuffed-stat-tracker" \
    "$HOME_DIR"

cat > "$PKG/bin/scuffed-stat-tracker" <<'EOF'
#!/bin/sh
echo "scuffed-stat-tracker 0.4.1"
EOF
cat > "$PKG/bin/stat-tracker-gui" <<'EOF'
#!/bin/sh
echo "stat-tracker-gui stub"
EOF
chmod +x "$PKG/bin/scuffed-stat-tracker" "$PKG/bin/stat-tracker-gui"
printf '[Desktop Entry]\nName=test\n' > "$PKG/assets/scuffed-stat-tracker.desktop"
printf '[Unit]\nDescription=test\n' > "$PKG/assets/scuffed-stat-tracker.service"
cp "$INSTALL" "$PKG/install.sh"
cp "$DIST/install-paths.sh" "$PKG/install-paths.sh"
cp "$UNINSTALL" "$PKG/uninstall.sh"

# Tarball contents: OCR + GUI libs, plus a sneaky OpenSSL that must be skipped.
printf 'ocr-lept\n' > "$PKG/lib/scuffed-stat-tracker/ocr/liblept.so.5"
printf 'ocr-tess\n' > "$PKG/lib/scuffed-stat-tracker/ocr/libtesseract.so.5"
printf 'BAD-crypto\n' > "$PKG/lib/scuffed-stat-tracker/ocr/libcrypto.so.3"
printf 'BAD-ssl\n' > "$PKG/lib/scuffed-stat-tracker/ocr/libssl.so.3"
printf 'gui-xdo\n' > "$PKG/lib/scuffed-stat-tracker/gui/libxdo.so.3"

# Simulate a v0.4.0 install: flat $PREFIX/lib OpenSSL + OCR on the old RUNPATH.
printf 'OLD-crypto\n' > "$PREFIX/lib/libcrypto.so.3"
printf 'OLD-ssl\n' > "$PREFIX/lib/libssl.so.3"
printf 'OLD-lept\n' > "$PREFIX/lib/liblept.so.5"
# A file the user owns in $PREFIX/lib - not in our manifest, must survive.
printf 'keep-me\n' > "$PREFIX/lib/libsomething-else.so.1"
# Older OCR dir: Tesseract 4 soname next to where 5 will be installed.
mkdir -p "$PREFIX/lib/scuffed-stat-tracker/ocr"
printf 'OLD-tess4\n' > "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4"
printf 'OLD-tess4-real\n' > "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4.0.1"
{
    echo "$PREFIX/bin/scuffed-stat-tracker"
    echo "$PREFIX/lib/libcrypto.so.3"
    echo "$PREFIX/lib/libssl.so.3"
    echo "$PREFIX/lib/liblept.so.5"
    echo "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4"
    echo "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4.0.1"
} > "$PREFIX/share/scuffed-stat-tracker/install-manifest.txt"

# HOME is used for tessdata / desktop; keep it inside TMP.
# Must run the in-tarball installer so PKG_ROOT is the fake tree.
HOME="$HOME_DIR" PREFIX="$PREFIX" SKIP_INTEGRATION=1 \
    "$PKG/install.sh" >/dev/null

# OpenSSL from the tarball must not be installed anywhere under PREFIX.
if [[ -e "$PREFIX/lib/scuffed-stat-tracker/ocr/libcrypto.so.3" \
   || -e "$PREFIX/lib/scuffed-stat-tracker/ocr/libssl.so.3" \
   || -e "$PREFIX/lib/libcrypto.so.3" \
   || -e "$PREFIX/lib/libssl.so.3" ]]; then
    fail "OpenSSL landed under PREFIX"
    find "$PREFIX/lib" -name 'libcrypto.so*' -o -name 'libssl.so*' >&2 || true
else
    pass "OpenSSL not installed under PREFIX"
fi

# Isolated OCR / GUI libs installed.
if [[ -f "$PREFIX/lib/scuffed-stat-tracker/ocr/liblept.so.5" \
   && -f "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.5" \
   && ! -e "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4" \
   && ! -e "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4.0.1" ]]; then
    pass "OCR libs isolated and leftover libtesseract.so.4 removed"
else
    fail "OCR libs missing, or leftover libtesseract.so.4 still installed"
    find "$PREFIX/lib" -type f >&2 || true
fi
if [[ -f "$PREFIX/lib/scuffed-stat-tracker/gui/libxdo.so.3" ]]; then
    pass "GUI libxdo isolated under lib/scuffed-stat-tracker/gui"
else
    fail "GUI libxdo missing from private gui dir"
fi

# v0.4.0 leftover flat OCR lib removed (was in the old manifest).
if [[ -e "$PREFIX/lib/liblept.so.5" ]]; then
    fail "leftover flat liblept.so.5 still in PREFIX/lib"
else
    pass "leftover flat OCR lib removed from PREFIX/lib"
fi

# User-owned file in PREFIX/lib must survive.
if [[ -f "$PREFIX/lib/libsomething-else.so.1" ]]; then
    pass "non-manifest file in PREFIX/lib kept"
else
    fail "installer deleted a file it did not install"
fi

# Manifest: new private paths present, leftover openssl paths gone.
MANIFEST="$PREFIX/share/scuffed-stat-tracker/install-manifest.txt"
if grep -qx "$PREFIX/lib/scuffed-stat-tracker/ocr/liblept.so.5" "$MANIFEST" \
   && grep -qx "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.5" "$MANIFEST" \
   && grep -qx "$PREFIX/lib/scuffed-stat-tracker/gui/libxdo.so.3" "$MANIFEST"; then
    pass "manifest lists isolated lib paths"
else
    fail "manifest missing isolated lib paths"
    cat "$MANIFEST" >&2
fi
if grep -E 'libtesseract\.so\.4' "$MANIFEST"; then
    fail "manifest still lists libtesseract.so.4"
else
    pass "manifest has no libtesseract.so.4"
fi
if grep -E 'libcrypto\.so|libssl\.so' "$MANIFEST"; then
    fail "manifest still lists OpenSSL paths"
    grep -E 'libcrypto\.so|libssl\.so' "$MANIFEST" >&2
else
    pass "manifest has no OpenSSL paths"
fi

# ── flat pre-0.4.1 tarball fallback: skip OpenSSL, isolate into ocr/ ─────────

PKG2="$TMP/pkg-flat"
PREFIX2="$TMP/prefix-flat"
mkdir -p "$PKG2/bin" "$PKG2/assets" "$PKG2/lib" "$PREFIX2"
cp "$PKG/bin/"* "$PKG2/bin/"
cp "$PKG/assets/"* "$PKG2/assets/"
cp "$INSTALL" "$PKG2/install.sh"
cp "$DIST/install-paths.sh" "$PKG2/install-paths.sh"
cp "$UNINSTALL" "$PKG2/uninstall.sh"
printf 'flat-lept\n' > "$PKG2/lib/liblept.so.5"
printf 'flat-crypto\n' > "$PKG2/lib/libcrypto.so.3"

HOME="$HOME_DIR" PREFIX="$PREFIX2" SKIP_INTEGRATION=1 \
    "$PKG2/install.sh" >/dev/null

if [[ -f "$PREFIX2/lib/scuffed-stat-tracker/ocr/liblept.so.5" \
   && ! -e "$PREFIX2/lib/libcrypto.so.3" \
   && ! -e "$PREFIX2/lib/scuffed-stat-tracker/ocr/libcrypto.so.3" ]]; then
    pass "flat tarball fallback isolates OCR and skips OpenSSL"
else
    fail "flat tarball fallback did not isolate/skip correctly"
    find "$PREFIX2/lib" -type f >&2 || true
fi

# ── uninstall removes isolated tree ──────────────────────────────────────────

PREFIX="$PREFIX2" HOME="$HOME_DIR" \
    "$PREFIX2/bin/scuffed-stat-tracker-uninstall" --yes >/dev/null
if [[ -e "$PREFIX2/lib/scuffed-stat-tracker" ]]; then
    fail "uninstall left lib/scuffed-stat-tracker behind"
else
    pass "uninstall removed isolated lib tree"
fi

# A tarball that still ships libtesseract.so.4 keeps that file.
PKG4="$TMP/pkg-so4"
PREFIX4="$TMP/prefix-so4"
rm -rf "$PKG4"
cp -a "$PKG" "$PKG4"
rm -f "$PKG4/lib/scuffed-stat-tracker/ocr/libtesseract.so.5"
printf 'shipped4\n' > "$PKG4/lib/scuffed-stat-tracker/ocr/libtesseract.so.4"
mkdir -p "$PREFIX4/lib/scuffed-stat-tracker/ocr"
printf 'preexisting4\n' > "$PREFIX4/lib/scuffed-stat-tracker/ocr/libtesseract.so.4"
HOME="$HOME_DIR" PREFIX="$PREFIX4" SKIP_INTEGRATION=1 \
    "$PKG4/install.sh" >/dev/null
if grep -qx 'shipped4' "$PREFIX4/lib/scuffed-stat-tracker/ocr/libtesseract.so.4"; then
    pass "tarball that ships libtesseract.so.4 keeps it"
else
    fail "installer removed libtesseract.so.4 that the tarball shipped"
fi

# Update already deleted .so.4. Uninstall of that prefix removes .so.5 too.
PREFIX="$PREFIX" HOME="$HOME_DIR" \
    "$PREFIX/bin/scuffed-stat-tracker-uninstall" --yes >/dev/null
if [[ -e "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4" \
   || -e "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.4.0.1" \
   || -e "$PREFIX/lib/scuffed-stat-tracker/ocr/libtesseract.so.5" ]]; then
    fail "uninstall left a tesseract library behind"
else
    pass "uninstall removed bundled libtesseract.so.5"
fi

if [[ $FAILS -ne 0 ]]; then
    echo "$FAILS check(s) failed" >&2
    exit 1
fi
echo "All packaging checks passed."
