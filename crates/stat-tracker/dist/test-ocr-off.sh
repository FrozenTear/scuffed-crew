#!/usr/bin/env bash
# The installers and the OCR on/off switch (`ocr` in config.toml).
#
# With classic OCR off the tracker never starts Tesseract, so the installers
# must not check for (or warn about) tessdata. With it on they check as before.
# Run from anywhere: bash crates/stat-tracker/dist/test-ocr-off.sh
set -euo pipefail

DIST="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
# shellcheck source=install-paths.sh
source "$DIST/install-paths.sh"
FAILS=0

pass() { echo "PASS  $*"; }
fail() { echo "FAIL  $*" >&2; FAILS=$((FAILS + 1)); }

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT
unset STAT_TRACKER_OCR XDG_CONFIG_HOME

# ── config_ocr_is_off: what counts as off ────────────────────────────────────

# expect <off|on> <label> <config text, or "-" for no file>
N=0
expect() {
    local want="$1" label="$2" text="$3" home
    N=$((N + 1))
    home="$TMP/home-$N"
    mkdir -p "$home/.config/scuffed-stat-tracker"
    if [[ "$text" != "-" ]]; then
        printf '%s\n' "$text" > "$home/.config/scuffed-stat-tracker/config.toml"
    fi
    local got=on
    if config_ocr_is_off "$home"; then
        got=off
    fi
    if [[ "$got" == "$want" ]]; then
        pass "config: $label is $want"
    else
        fail "config: $label should be $want, got $got"
    fi
}

expect on  "no config.toml"                 "-"
expect on  "an empty config"                ""
expect on  "a config without the key"       'data_dir = "/x"'
expect off 'ocr = "off"'                    'ocr = "off"'
expect off "single quotes"                  "ocr = 'off'"
expect off "no spaces"                      'ocr="off"'
expect off "indented, with a comment"       '  ocr = "off"   # no Tesseract'
expect off "after other root keys"          $'data_dir = "/x"\nreader = "new"\nocr = "off"\n\n[auto_detect]\nenabled = true'
expect on  'ocr = "on"'                     'ocr = "on"'
expect on  "a commented-out key"            '# ocr = "off"'
expect on  "ocr_threads is another key"     'ocr_threads = 2'
expect on  "a value that only starts off"   'ocr = "offline"'
expect on  "the key inside a table"         $'[auto_detect]\nocr = "off"'
expect on  "the key after a table"          $'data_dir = "/x"\n[sync]\ntoken = "t"\nocr = "off"'

# Environment override, the same variable the tracker reads.
home="$TMP/env-home"
mkdir -p "$home/.config/scuffed-stat-tracker"
if STAT_TRACKER_OCR=off config_ocr_is_off "$home"; then
    pass "env: STAT_TRACKER_OCR=off is off without a config"
else
    fail "env: STAT_TRACKER_OCR=off should be off"
fi
if STAT_TRACKER_OCR=OFF config_ocr_is_off "$home"; then
    pass "env: the value is not case sensitive"
else
    fail "env: STAT_TRACKER_OCR=OFF should be off"
fi
printf 'ocr = "off"\n' > "$home/.config/scuffed-stat-tracker/config.toml"
if STAT_TRACKER_OCR=on config_ocr_is_off "$home"; then
    fail "env: STAT_TRACKER_OCR=on should win over the file"
else
    pass "env: STAT_TRACKER_OCR=on wins over ocr = \"off\" in the file"
fi
if STAT_TRACKER_OCR=maybe config_ocr_is_off "$home"; then
    pass "env: another value leaves the file's answer"
else
    fail "env: an unknown STAT_TRACKER_OCR value should fall back to the file"
fi

# XDG_CONFIG_HOME is where the tracker looks first.
xdg="$TMP/xdg"
mkdir -p "$xdg/scuffed-stat-tracker"
printf 'ocr = "off"\n' > "$xdg/scuffed-stat-tracker/config.toml"
if XDG_CONFIG_HOME="$xdg" config_ocr_is_off "$TMP/home-1"; then
    pass "config: XDG_CONFIG_HOME is honoured"
else
    fail "config: XDG_CONFIG_HOME/scuffed-stat-tracker/config.toml was not read"
fi

# ── dist/install.sh: the tessdata check runs only with OCR on ────────────────

PKG="$TMP/pkg"
mkdir -p "$PKG/bin" "$PKG/assets"
cat > "$PKG/bin/scuffed-stat-tracker" <<'EOF'
#!/bin/sh
echo "scuffed-stat-tracker 0.0.0"
EOF
cat > "$PKG/bin/stat-tracker-gui" <<'EOF'
#!/bin/sh
echo "stat-tracker-gui stub"
EOF
chmod +x "$PKG/bin/scuffed-stat-tracker" "$PKG/bin/stat-tracker-gui"
printf '[Desktop Entry]\nName=test\n' > "$PKG/assets/scuffed-stat-tracker.desktop"
printf '[Unit]\nDescription=test\n' > "$PKG/assets/scuffed-stat-tracker.service"
cp "$DIST/install.sh" "$PKG/install.sh"
cp "$DIST/install-paths.sh" "$PKG/install-paths.sh"
cp "$DIST/uninstall.sh" "$PKG/uninstall.sh"

# run_install <name> <config text, or "-">: stderr of one install.
run_install() {
    local name="$1" text="$2"
    local home="$TMP/inst-$name/home" prefix="$TMP/inst-$name/prefix"
    mkdir -p "$home/.config/scuffed-stat-tracker" "$prefix"
    if [[ "$text" != "-" ]]; then
        printf '%s\n' "$text" > "$home/.config/scuffed-stat-tracker/config.toml"
    fi
    # An empty TESSDATA_PREFIX and no bundled model: nothing in the fake tree
    # supplies eng.traineddata, so only a system package can.
    # stderr only: that is where the installer logs.
    {
        HOME="$home" PREFIX="$prefix" SKIP_INTEGRATION=1 TESSDATA_PREFIX="" \
            "$PKG/install.sh" >/dev/null
    } 2>&1
}

SKIP_LINE='not checking for eng.traineddata'
WARN_LINE='eng.traineddata not found'

off_out="$(run_install off 'ocr = "off"')"
if grep -q "$SKIP_LINE" <<<"$off_out"; then
    pass "install: OCR off skips the eng.traineddata check"
else
    fail "install: OCR off did not skip the eng.traineddata check"
    echo "$off_out" >&2
fi
if grep -q "$WARN_LINE" <<<"$off_out"; then
    fail "install: OCR off still warns about eng.traineddata"
else
    pass "install: OCR off prints no eng.traineddata warning"
fi

for case_name in on none; do
    case "$case_name" in
        on)   text='ocr = "on"' ;;
        none) text='-' ;;
    esac
    on_out="$(run_install "$case_name" "$text")"
    if grep -q "$SKIP_LINE" <<<"$on_out"; then
        fail "install: OCR on ($case_name) skipped the eng.traineddata check"
    else
        pass "install: OCR on ($case_name) still checks for eng.traineddata"
    fi
done

# The check itself, where this machine has no system model to find.
system_eng=0
for f in /usr/share/tessdata/eng.traineddata /usr/local/share/tessdata/eng.traineddata \
         /usr/share/tesseract/tessdata/eng.traineddata \
         /usr/share/tesseract-ocr/*/tessdata/eng.traineddata; do
    [[ -f "$f" ]] && system_eng=1
done
if [[ "$system_eng" -eq 0 ]]; then
    if grep -q "$WARN_LINE" <<<"$on_out"; then
        pass "install: OCR on warns when eng.traineddata is missing"
    else
        fail "install: OCR on did not warn about the missing eng.traineddata"
    fi
else
    echo "SKIP  install: this machine has a system eng.traineddata, so the OCR-on warning cannot fire"
fi

# OCR off must not stop the bundled models from being installed: turning OCR
# back on later has to work without another install.
mkdir -p "$PKG/tessdata"
printf 'eng-model\n' > "$PKG/tessdata/eng.traineddata"
run_install bundled 'ocr = "off"' >/dev/null
if [[ -f "$TMP/inst-bundled/home/.local/share/scuffed-stat-tracker/tessdata/eng.traineddata" ]]; then
    pass "install: OCR off still installs the bundled eng.traineddata"
else
    fail "install: OCR off dropped the bundled eng.traineddata"
fi
rm -rf "$PKG/tessdata"

# ── install.sh (source build): the same guard, by shape ──────────────────────
# That script builds with cargo, so it is not run here.

SRC_INSTALL="$DIST/../install.sh"
if grep -q 'source "\$DIST/install-paths.sh"' "$SRC_INSTALL" \
   && grep -A3 '^if config_ocr_is_off "\$HOME"; then$' "$SRC_INSTALL" \
        | grep -q '^elif ! command -v tesseract'; then
    pass "source install: the tesseract and tessdata checks sit behind config_ocr_is_off"
else
    fail "source install: the tessdata checks are not guarded by config_ocr_is_off"
fi
if grep -B1 'OCR will use eng tessdata' "$SRC_INSTALL" | grep -q 'OCR_OFF" -eq 0'; then
    pass "source install: the tessdata note prints only with OCR on"
else
    fail "source install: the tessdata note is not guarded"
fi

if [[ "$FAILS" -gt 0 ]]; then
    echo "$FAILS check(s) failed" >&2
    exit 1
fi
echo "all OCR-off installer checks passed"
