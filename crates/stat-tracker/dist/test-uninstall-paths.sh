#!/usr/bin/env bash
# The install path list is the only set uninstall may touch.
# Fails if install.sh writes a path the list does not name, if a
# package-owned or /usr install deletes anything, or if keep-data
# removes the data directory.
set -euo pipefail

ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
DIST="$ROOT/dist"
INSTALL="$DIST/install.sh"
UNINSTALL="$DIST/uninstall.sh"
PATHS="$DIST/install-paths.sh"
BOOTSTRAP="$DIST/bootstrap.sh"
HELPER="$DIST/import-session-env.sh"
UNIT_LIB="$DIST/systemd-unit.sh"

fail() { echo "FAIL: $*" >&2; exit 1; }
pass() { echo "PASS: $*" >&2; }

[[ -f "$INSTALL" && -f "$UNINSTALL" && -f "$PATHS" && -f "$BOOTSTRAP" ]] \
    || fail "missing installer files"

# shellcheck source=install-paths.sh
source "$PATHS"

TMP="$(mktemp -d)"
trap 'rm -rf "$TMP"' EXIT

FAKE_CTL="$TMP/bin/systemctl"
mkdir -p "$TMP/bin" "$TMP/noproc"
LOG="$TMP/systemctl.log"
cat > "$FAKE_CTL" << EOF
#!/bin/sh
printf '%s\n' "\$*" >> "$LOG"
if [ "\$2" = "disable" ]; then
    unit="$TMP/home/.config/systemd/user/scuffed-stat-tracker.service"
    if [ ! -f "\$unit" ]; then
        echo "UNIT MISSING" >> "$LOG"
        exit 1
    fi
fi
exit 0
EOF
chmod +x "$FAKE_CTL"

stage_pkg() {
    local pkg="$1"
    mkdir -p "$pkg/bin" "$pkg/assets" "$pkg/lib/scuffed-stat-tracker/ocr"
    printf '%s\n' '#!/bin/sh' 'echo scuffed-stat-tracker 0.0.0-test' > "$pkg/bin/scuffed-stat-tracker"
    printf '%s\n' '#!/bin/sh' 'echo stat-tracker-gui' > "$pkg/bin/stat-tracker-gui"
    chmod +x "$pkg/bin/scuffed-stat-tracker" "$pkg/bin/stat-tracker-gui"
    printf 'ocr-lib\n' > "$pkg/lib/scuffed-stat-tracker/ocr/liblept.so.5"
    cp "$ROOT/assets/scuffed-stat-tracker.desktop" \
        "$ROOT/assets/scuffed-stat-tracker.service" \
        "$ROOT/assets/scuffed-stat-tracker-session.service" \
        "$pkg/assets/"
    cp "$INSTALL" "$pkg/install.sh"
    cp "$UNINSTALL" "$pkg/uninstall.sh"
    cp "$PATHS" "$pkg/install-paths.sh"
    cp "$HELPER" "$UNIT_LIB" "$pkg/"
    chmod +x "$pkg/install.sh" "$pkg/uninstall.sh" "$pkg/import-session-env.sh"
}

run_install() {
    local home="$1" prefix="$2"
    env -u WAYLAND_DISPLAY -u DISPLAY -u XDG_CURRENT_DESKTOP -u XDG_SESSION_TYPE \
        HOME="$home" \
        PREFIX="$prefix" \
        SCUFFED_PROC_ROOT="$TMP/noproc" \
        SCUFFED_SYSTEMCTL="$FAKE_CTL" \
        bash "$PKG/install.sh"
}

# Every file install.sh records must be an expanded list path, or a
# file inside the listed library directory.
assert_manifest_matches_list() {
    local home="$1" prefix="$2" manifest="$3" line cat when path covered
    [[ -f "$manifest" ]] || fail "no manifest at $manifest"
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ -n "$line" ]] || continue
        covered=0
        while read -r cat when path; do
            if [[ "$line" == "$path" ]]; then
                covered=1
                break
            fi
            if [[ "$cat" == "libdir" && "$line" == "$path"/* ]]; then
                covered=1
                break
            fi
        done < <(expanded_install_paths "$home" "$prefix")
        [[ "$covered" -eq 1 ]] || fail "manifest path is not in the install list: $line"
    done < "$manifest"
}

plant_canaries() {
    local home="$1" prefix="$2"
    printf 'keep\n' > "$home/keep-me"
    printf 'other-bin\n' > "$prefix/bin/other-tool"
    mkdir -p "$prefix/lib" "$home/.config/other-app" \
        "$home/.local/share/applications" \
        "$home/.config/systemd/user" \
        "$home/.local/share/scuffed-stat-tracker/stats.surrealkv" \
        "$home/.local/share/scuffed-stat-tracker/debug" \
        "$home/.local/share/scuffed-stat-tracker/shadow" \
        "$home/.config/scuffed-stat-tracker"
    printf 'other-lib\n' > "$prefix/lib/libother.so"
    printf 'other-app\n' > "$home/.config/other-app/file"
    printf 'mime\n' > "$home/.local/share/applications/mimeinfo.cache"
    printf '[Unit]\nDescription=not ours\n' > "$home/.config/systemd/user/some-other.service"
    printf 'games\n' > "$home/.local/share/scuffed-stat-tracker/stats.surrealkv/db"
    printf 'crop\n' > "$home/.local/share/scuffed-stat-tracker/debug/crop.png"
    printf 'shadow\n' > "$home/.local/share/scuffed-stat-tracker/shadow/digits.jsonl"
    printf 'sync_token = "secret-token"\n' > "$home/.config/scuffed-stat-tracker/config.toml"
    # A manifest line outside the list must survive.
    printf '%s\n' "$home/keep-me" >> "$prefix/share/scuffed-stat-tracker/install-manifest.txt"
}

assert_canaries() {
    local home="$1" prefix="$2"
    [[ "$(cat "$home/keep-me")" == "keep" ]] || fail "canary outside HOME root was touched"
    [[ -f "$prefix/bin/other-tool" ]] || fail "unrelated binary in PREFIX/bin was removed"
    [[ -f "$prefix/lib/libother.so" ]] || fail "unrelated library in PREFIX/lib was removed"
    [[ -f "$home/.config/other-app/file" ]] || fail "unrelated config was removed"
    [[ -f "$home/.local/share/applications/mimeinfo.cache" ]] || fail "desktop cache outside the list was removed"
    [[ -f "$home/.config/systemd/user/some-other.service" ]] || fail "unrelated user unit was removed"
}

assert_removed_install() {
    local home="$1" prefix="$2"
    [[ ! -e "$prefix/bin/scuffed-stat-tracker" ]] || fail "daemon binary survived"
    [[ ! -e "$prefix/bin/stat-tracker-gui" ]] || fail "GUI binary survived"
    [[ ! -e "$prefix/bin/scuffed-stat-tracker-uninstall" ]] || fail "uninstaller survived"
    [[ ! -e "$prefix/lib/scuffed-stat-tracker" ]] || fail "library directory survived"
    [[ ! -e "$home/.local/share/applications/scuffed-stat-tracker.desktop" ]] || fail "desktop entry survived"
    [[ ! -e "$home/.config/systemd/user/scuffed-stat-tracker.service" ]] || fail "systemd unit survived"
    [[ ! -e "$home/.config/systemd/user/scuffed-stat-tracker-session.service" ]] || fail "session unit survived"
}

assert_data_kept() {
    local home="$1"
    [[ -d "$home/.local/share/scuffed-stat-tracker" ]] || fail "data dir was removed in keep-data mode"
    [[ -f "$home/.local/share/scuffed-stat-tracker/stats.surrealkv/db" ]] || fail "games database was removed"
    [[ -f "$home/.local/share/scuffed-stat-tracker/debug/crop.png" ]] || fail "debug crop was removed"
    [[ -f "$home/.local/share/scuffed-stat-tracker/shadow/digits.jsonl" ]] || fail "shadow log was removed"
    [[ -f "$home/.config/scuffed-stat-tracker/config.toml" ]] || fail "config was removed"
    grep -q 'secret-token' "$home/.config/scuffed-stat-tracker/config.toml" \
        || fail "sync token was removed"
}

PKG="$TMP/pkg"
HOME_DIR="$TMP/home"
PREFIX="$HOME_DIR/.local"
stage_pkg "$PKG"
mkdir -p "$HOME_DIR"
run_install "$HOME_DIR" "$PREFIX"

MANIFEST="$PREFIX/share/scuffed-stat-tracker/install-manifest.txt"
assert_manifest_matches_list "$HOME_DIR" "$PREFIX" "$MANIFEST"

# Full install creates every always path except the enable symlink and
# the optional data_dir drop-in.
while read -r cat when path; do
    case "$cat" in
        autostart|dropin) continue ;;
    esac
    if [[ "$when" == "always" ]]; then
        if [[ "$cat" == "libdir" ]]; then
            [[ -d "$path" ]] || fail "install did not create library dir $path"
        else
            [[ -e "$path" ]] || fail "install did not create $cat $path"
        fi
    fi
done < <(expanded_install_paths "$HOME_DIR" "$PREFIX" always)
pass "install paths match the shared list"

plant_canaries "$HOME_DIR" "$PREFIX"
: > "$LOG"
env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    STAT_TRACKER_PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$BOOTSTRAP" --uninstall --yes >/dev/null

grep -q 'disable --now scuffed-stat-tracker.service' "$LOG" \
    || fail "systemd unit was not disabled before removal. log: $(cat "$LOG")"
if grep -q 'UNIT MISSING' "$LOG"; then
    fail "disable ran after the unit file was deleted"
fi
assert_removed_install "$HOME_DIR" "$PREFIX"
assert_data_kept "$HOME_DIR"
assert_canaries "$HOME_DIR" "$PREFIX"
[[ -f "$HOME_DIR/keep-me" ]] || fail "path listed only in the manifest was removed"
pass "bootstrap.sh --uninstall keeps data and leaves everything else"

# Purge removes the data dir and config, still nothing outside the list.
run_install "$HOME_DIR" "$PREFIX"
plant_canaries "$HOME_DIR" "$PREFIX"
env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --purge --yes >/dev/null
[[ ! -e "$HOME_DIR/.local/share/scuffed-stat-tracker" ]] || fail "purge left the data dir"
[[ ! -e "$HOME_DIR/.config/scuffed-stat-tracker" ]] || fail "purge left the config dir"
assert_canaries "$HOME_DIR" "$PREFIX"
pass "purge removes data and config only"

# Package-owned install: print the command, delete nothing.
run_install "$HOME_DIR" "$PREFIX"
plant_canaries "$HOME_DIR" "$PREFIX"
mkdir -p "$TMP/fakepac"
cat > "$TMP/fakepac/pacman" << 'EOF'
#!/bin/sh
if [ "$1" = "-Qo" ]; then
    echo "$2 is owned by scuffed-stat-tracker 1.0-1"
    exit 0
fi
exit 1
EOF
chmod +x "$TMP/fakepac/pacman"
pac_out="$(env HOME="$HOME_DIR" PREFIX="$PREFIX" PATH="$TMP/fakepac:$PATH" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --purge --yes)"
[[ "$pac_out" == "sudo pacman -R scuffed-stat-tracker" ]] \
    || fail "pacman command was '$pac_out'"
[[ -x "$PREFIX/bin/stat-tracker-gui" ]] || fail "package-owned GUI was deleted"
[[ -f "$HOME_DIR/.local/share/scuffed-stat-tracker/stats.surrealkv/db" ]] \
    || fail "package-owned uninstall deleted the games database"
assert_canaries "$HOME_DIR" "$PREFIX"
pass "pacman install prints sudo pacman -R and deletes nothing"

# apt/dpkg install, including a .deb. Same rule.
mkdir -p "$TMP/fakedeb"
cat > "$TMP/fakedeb/dpkg" << 'EOF'
#!/bin/sh
if [ "$1" = "-S" ]; then
    echo "scuffed-stat-tracker: $2"
    exit 0
fi
exit 1
EOF
chmod +x "$TMP/fakedeb/dpkg"
apt_out="$(env HOME="$HOME_DIR" PREFIX="$PREFIX" PATH="$TMP/fakedeb:$PATH" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --purge --yes)"
[[ "$apt_out" == "sudo apt remove scuffed-stat-tracker" ]] \
    || fail "apt command was '$apt_out'"
[[ -x "$PREFIX/bin/scuffed-stat-tracker" ]] || fail "dpkg-owned daemon was deleted"
pass "dpkg install prints sudo apt remove and deletes nothing"

# /usr is outside the bootstrap user paths. Do not create files there.
# The refusal happens before any removal.
set +e
env HOME="$HOME_DIR" PREFIX=/usr \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --purge --yes >/dev/null 2>"$TMP/usr.err"
usr_status=$?
set -e
[[ "$usr_status" -ne 0 ]] || fail "PREFIX=/usr uninstall exited 0"
grep -q 'outside the bootstrap.sh user install paths' "$TMP/usr.err" \
    || fail "PREFIX=/usr did not explain the refusal: $(cat "$TMP/usr.err")"
[[ -f "$HOME_DIR/keep-me" ]] || fail "PREFIX=/usr uninstall touched HOME"
pass "paths under /usr are not removed"

echo "all uninstall path checks passed"
