#!/usr/bin/env bash
# The installers and the OCR on/off switch (`ocr` in config.toml).
#
# The installers always check for Tesseract's model: the tracker keeps classic
# OCR on when it has to (a reader pack is missing), whatever config.toml asks
# for. With `ocr = "off"` a missing model is a note, not a warning. With OCR
# on the warning is as before.
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
expect off "any case, as the tracker reads it"  'ocr = "Off"'
expect off "spaces inside the quotes"       'ocr = " off "'
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
rm "$home/.config/scuffed-stat-tracker/config.toml"
if STAT_TRACKER_OCR=' off ' config_ocr_is_off "$home"; then
    pass "env: spaces around the value are ignored"
else
    fail "env: STAT_TRACKER_OCR=' off ' should be off"
fi
if STAT_TRACKER_OCR='o f f' config_ocr_is_off "$home"; then
    fail "env: 'o f f' is not off (the tracker does not read it as off)"
else
    pass "env: spaces inside the value make it another word"
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

# ── dist/install.sh: the model check always runs; its wording follows `ocr` ──

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

# A stand-in for "/" with no model, and one with a model, so the check does
# not depend on what this machine has installed.
NO_MODEL="$TMP/sysroot-empty"
WITH_MODEL="$TMP/sysroot-model"
mkdir -p "$NO_MODEL" "$WITH_MODEL/usr/share/tessdata"
printf 'eng-model\n' > "$WITH_MODEL/usr/share/tessdata/eng.traineddata"

# run_install <name> <system root> <config text, or "-">: stderr of one install.
run_install() {
    local name="$1" sysroot="$2" text="$3"
    local home="$TMP/inst-$name/home" prefix="$TMP/inst-$name/prefix"
    mkdir -p "$home/.config/scuffed-stat-tracker" "$prefix"
    if [[ "$text" != "-" ]]; then
        printf '%s\n' "$text" > "$home/.config/scuffed-stat-tracker/config.toml"
    fi
    # stderr only: that is where the installer logs.
    {
        HOME="$home" PREFIX="$prefix" SKIP_INTEGRATION=1 TESSDATA_PREFIX="" \
            TESSDATA_SYSTEM_ROOT="$sysroot" "$PKG/install.sh" >/dev/null
    } 2>&1
}

WARN_LINE="install your distro's eng tessdata package"
NOTE_LINE='eng.traineddata not found. config.toml asks for classic OCR off, so the tracker may not need it.'

# check_out <yes|no> <output> <line> <label>: the line is (or is not) in the output.
check_out() {
    local want="$1" out="$2" line="$3" label="$4"
    local got=no
    if grep -qF -- "$line" <<<"$out"; then
        got=yes
    fi
    if [[ "$got" == "$want" ]]; then
        pass "install: $label"
    else
        fail "install: $label"
        echo "$out" >&2
    fi
}

off_out="$(run_install off "$NO_MODEL" 'ocr = "off"')"
check_out yes "$off_out" "$NOTE_LINE" "OCR off and no model: a note says when it is needed"
check_out no  "$off_out" "$WARN_LINE" "OCR off and no model: no warning"
check_out no  "$off_out" "does not need it" "OCR off and no model: the note does not say the model is not needed"
check_out yes "$off_out" "a tracker version that cannot turn it off yet" "OCR off and no model: the note names a refusing version"
check_out yes "$off_out" "a missing reader pack" "OCR off and no model: the note names a missing pack"
check_out yes "$off_out" "sudo pacman -S tesseract-data-eng" "OCR off and no model: the package to install is named (Arch)"
check_out yes "$off_out" "sudo apt install tesseract-ocr-eng" "OCR off and no model: the package to install is named (Debian)"
check_out yes "$off_out" "sudo dnf install tesseract-langpack-eng" "OCR off and no model: the package to install is named (Fedora)"
if grep -F -- "warn ]" <<<"$off_out" | grep -q "traineddata"; then
    fail "install: OCR off and no model: the model is still mentioned at warn level"
else
    pass "install: OCR off and no model: no warn line mentions the model"
fi

for case_name in on none; do
    case "$case_name" in
        on)   text='ocr = "on"' ;;
        none) text='-' ;;
    esac
    on_out="$(run_install "$case_name" "$NO_MODEL" "$text")"
    check_out yes "$on_out" "$WARN_LINE" "OCR on ($case_name) and no model: the warning, as before"
    check_out no  "$on_out" "$NOTE_LINE" "OCR on ($case_name) and no model: not the OCR-off note"
done

# The check runs with OCR off too: a model that is there is found, and
# nothing is said.
for text in 'ocr = "off"' '-'; do
    found_out="$(run_install "found-${text:0:1}" "$WITH_MODEL" "$text")"
    check_out no "$found_out" "eng.traineddata not found" "a model that is there is found ($text)"
done

# OCR off must not stop the bundled models from being installed: turning OCR
# back on later has to work without another install.
mkdir -p "$PKG/tessdata"
printf 'eng-model\n' > "$PKG/tessdata/eng.traineddata"
run_install bundled "$NO_MODEL" 'ocr = "off"' >/dev/null
if [[ -f "$TMP/inst-bundled/home/.local/share/scuffed-stat-tracker/tessdata/eng.traineddata" ]]; then
    pass "install: OCR off still installs the bundled eng.traineddata"
else
    fail "install: OCR off dropped the bundled eng.traineddata"
fi
rm -rf "$PKG/tessdata"

# ── install.sh (source build), by shape ──────────────────────────────────────
# That script builds with cargo, so it is not run here.

SRC_INSTALL="$DIST/../install.sh"
if grep -q 'source "\$DIST/install-paths.sh"' "$SRC_INSTALL" \
   && grep -q '^if ! command -v tesseract &>/dev/null; then$' "$SRC_INSTALL"; then
    pass "source install: the tesseract check runs whatever ocr says"
else
    fail "source install: the tesseract check is behind a condition"
fi
if grep -A4 'if \[\[ "\$OCR_OFF" -eq 1 \]\]; then' "$SRC_INSTALL" | grep -qF "$NOTE_LINE" \
   && grep -A4 'if \[\[ "\$OCR_OFF" -eq 1 \]\]; then' "$SRC_INSTALL" | grep -qF "tesseract-data-eng" \
   && grep -qF "$WARN_LINE" "$SRC_INSTALL"; then
    pass "source install: a missing model is a note with OCR off and a warning with it on"
else
    fail "source install: the missing-model wording does not follow ocr"
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
