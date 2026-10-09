#!/usr/bin/env bash
# install.sh records every file it writes. uninstall.sh removes exactly
# those manifest entries. A package owner (pacman -Qo / dpkg -S) deletes
# nothing. An extra file next to an installed file must survive. Keep-data
# leaves config.toml in place.
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
    # Recorded in the manifest on purpose. Still kept unless --purge.
    printf '%s\n' "$home/.config/scuffed-stat-tracker/config.toml" \
        >> "$prefix/share/scuffed-stat-tracker/install-manifest.txt"
    # Sits next to the installed GUI binary and is not in the manifest.
    printf 'neighbor\n' > "$prefix/bin/neighbor-tool"
}

assert_canaries() {
    local home="$1" prefix="$2"
    [[ "$(cat "$home/keep-me")" == "keep" ]] || fail "canary outside HOME root was touched"
    [[ -f "$prefix/bin/other-tool" ]] || fail "unrelated binary in PREFIX/bin was removed"
    [[ "$(cat "$prefix/bin/neighbor-tool")" == "neighbor" ]] \
        || fail "extra file next to the installed binary was removed"
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
grep -q 'disable --now scuffed-stat-tracker.timer' "$LOG" \
    || fail "timer was not disabled before removal. log: $(cat "$LOG")"
if grep -q 'UNIT MISSING' "$LOG"; then
    fail "disable ran after the unit file was deleted"
fi
assert_removed_install "$HOME_DIR" "$PREFIX"
assert_data_kept "$HOME_DIR"
assert_canaries "$HOME_DIR" "$PREFIX"
pass "bootstrap.sh --uninstall keeps data and leaves the neighbor file"

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

# A file sitting next to a recorded library, not in the manifest, survives.
# The library file itself is removed. The directory stays because it is not empty.
run_install "$HOME_DIR" "$PREFIX"
printf 'neighbor-lib\n' > "$PREFIX/lib/scuffed-stat-tracker/ocr/neighbor.so"
env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --yes >/dev/null
[[ -f "$PREFIX/lib/scuffed-stat-tracker/ocr/neighbor.so" ]] \
    || fail "extra file next to a bundled library was removed"
[[ ! -e "$PREFIX/lib/scuffed-stat-tracker/ocr/liblept.so.5" ]] \
    || fail "recorded library survived"
[[ ! -e "$PREFIX/bin/stat-tracker-gui" ]] || fail "recorded GUI survived"
pass "extra file next to a recorded library survives"

# No manifest: show the fixed list, remove those paths, leave the neighbor
# and config.toml.
run_install "$HOME_DIR" "$PREFIX"
rm -f "$MANIFEST"
printf 'neighbor\n' > "$PREFIX/bin/neighbor-tool"
printf 'sync_token = "secret-token"\n' > "$HOME_DIR/.config/scuffed-stat-tracker/config.toml"
env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --yes >/dev/null 2>"$TMP/fallback.err"
grep -q 'No install manifest' "$TMP/fallback.err" \
    || fail "fallback did not say the manifest was missing: $(cat "$TMP/fallback.err")"
grep -q "$PREFIX/bin/stat-tracker-gui" "$TMP/fallback.err" \
    || fail "fallback did not show the fixed list"
[[ ! -e "$PREFIX/bin/stat-tracker-gui" ]] || fail "fallback left the GUI"
[[ "$(cat "$PREFIX/bin/neighbor-tool")" == "neighbor" ]] \
    || fail "fallback removed the neighbor file"
[[ -f "$HOME_DIR/.config/scuffed-stat-tracker/config.toml" ]] \
    || fail "fallback removed config.toml"
pass "missing manifest shows the fixed list and leaves config.toml"

# Ownership, not the path. A prefix whose path contains /usr/bin is still
# removed when no package owns the binary, and left alone when pacman does.
if grep -q 'is_system_install_path' "$UNINSTALL"; then
    fail "uninstall.sh still classifies installs by path"
fi
USR_PREFIX="$TMP/usr"
run_install "$HOME_DIR" "$USR_PREFIX"
printf 'neighbor\n' > "$USR_PREFIX/bin/neighbor-tool"
env HOME="$HOME_DIR" PREFIX="$USR_PREFIX" PATH="$TMP/bin:/usr/bin:/bin" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --yes >/dev/null
[[ ! -e "$USR_PREFIX/bin/stat-tracker-gui" ]] \
    || fail "unmanaged /usr-like prefix was not uninstalled"
[[ "$(cat "$USR_PREFIX/bin/neighbor-tool")" == "neighbor" ]] \
    || fail "neighbor next to a /usr-like install was removed"
run_install "$HOME_DIR" "$USR_PREFIX"
printf 'neighbor\n' > "$USR_PREFIX/bin/neighbor-tool"
owned="$(env HOME="$HOME_DIR" PREFIX="$USR_PREFIX" PATH="$TMP/fakepac:/usr/bin:/bin" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --purge --yes)"
[[ "$owned" == "sudo pacman -R scuffed-stat-tracker" ]] \
    || fail "package owner on a /usr-like path was '$owned'"
[[ -x "$USR_PREFIX/bin/stat-tracker-gui" ]] \
    || fail "package-owned binary under a /usr-like path was deleted"
[[ -f "$USR_PREFIX/bin/neighbor-tool" ]] \
    || fail "package-owned uninstall deleted the neighbor"
pass "package ownership is what blocks removal, not the path"

# A manifest line outside home, or one that uses .., must not be deleted.
run_install "$HOME_DIR" "$PREFIX"
OUTSIDE="$TMP/outside-secret"
ESCAPED="$TMP/escaped"
printf 'secret\n' > "$OUTSIDE"
printf 'escaped\n' > "$ESCAPED"
etc_before=0
[[ -e /etc/x ]] && etc_before=1
{
    printf '\n/etc/x\n'
    printf '%s\n' "$PREFIX/bin/../../escaped"
    printf '%s\n' "$OUTSIDE"
} >> "$MANIFEST"
env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --yes >/dev/null
[[ "$(cat "$OUTSIDE")" == "secret" ]] || fail "manifest line outside home was deleted"
[[ "$(cat "$ESCAPED")" == "escaped" ]] || fail "manifest line with .. was deleted"
etc_after=0
[[ -e /etc/x ]] && etc_after=1
[[ "$etc_before" == "$etc_after" ]] || fail "manifest line /etc/x changed that file"
[[ ! -e "$PREFIX/bin/stat-tracker-gui" ]] || fail "a safe manifest line was not removed"
pass "manifest lines outside home or with .. are not removed"

# A manifest still lists the files and waits for yes. No answer removes nothing
# and does not stop the service.
run_install "$HOME_DIR" "$PREFIX"
: > "$LOG"
cancel_out="$(printf 'n\n' | env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" 2>&1)"
printf '%s\n' "$cancel_out" | grep -q "$PREFIX/bin/stat-tracker-gui" \
    || fail "manifest uninstall did not list the files: $cancel_out"
printf '%s\n' "$cancel_out" | grep -q 'Uninstall cancelled' \
    || fail "declining did not cancel: $cancel_out"
[[ -x "$PREFIX/bin/stat-tracker-gui" ]] || fail "declining removed the GUI"
if grep -q 'disable --now' "$LOG"; then
    fail "declining stopped the service. log: $(cat "$LOG")"
fi
printf 'y\n' | env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" >/dev/null
[[ ! -e "$PREFIX/bin/stat-tracker-gui" ]] || fail "confirming left the GUI"
pass "manifest uninstall lists files and asks before removing them"

# If the service is still running, remove nothing.
run_install "$HOME_DIR" "$PREFIX"
ACTIVE_CTL="$TMP/bin/systemctl-active"
cat > "$ACTIVE_CTL" << 'EOF'
#!/bin/sh
echo active
exit 0
EOF
chmod +x "$ACTIVE_CTL"
set +e
env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$ACTIVE_CTL" \
    bash "$UNINSTALL" --yes >/dev/null 2>"$TMP/active.err"
active_code=$?
set -e
[[ "$active_code" -ne 0 ]] || fail "a still-running tracker exited 0"
grep -q 'still running' "$TMP/active.err" \
    || fail "still-running message missing: $(cat "$TMP/active.err")"
[[ -x "$PREFIX/bin/stat-tracker-gui" ]] || fail "a still-running tracker was uninstalled"
pass "a tracker that is still running is not uninstalled"

# A parent that is itself a symlink is not followed. The file stays, and
# the script does not say the uninstall finished.
run_install "$HOME_DIR" "$PREFIX"
mv "$PREFIX/bin" "$PREFIX/real-bin"
ln -s "$PREFIX/real-bin" "$PREFIX/bin"
set +e
env HOME="$HOME_DIR" PREFIX="$PREFIX" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --yes >/dev/null 2>"$TMP/symlink.err"
link_code=$?
set -e
[[ "$link_code" -ne 0 ]] || fail "a symlinked bin directory exited 0"
grep -q 'Still there' "$TMP/symlink.err" \
    || fail "symlinked bin was not reported: $(cat "$TMP/symlink.err")"
if grep -q 'Uninstall complete' "$TMP/symlink.err"; then
    fail "symlinked bin said uninstall was complete"
fi
[[ -f "$PREFIX/real-bin/stat-tracker-gui" ]] \
    || fail "file behind a symlinked bin directory was removed"
rm -rf "$PREFIX/bin" "$PREFIX/real-bin"
pass "a symlinked parent is reported as still there"

# /home as a symlink (Silverblue) still removes the real files.
LINK_BASE="$TMP/silverblue"
mkdir -p "$LINK_BASE/var/home/user"
ln -s "$LINK_BASE/var/home" "$LINK_BASE/home"
LINK_HOME="$LINK_BASE/home/user"
LINK_PREFIX="$LINK_HOME/.local"
run_install "$LINK_HOME" "$LINK_PREFIX"
env HOME="$LINK_HOME" PREFIX="$LINK_PREFIX" \
    SCUFFED_SYSTEMCTL=/bin/true \
    bash "$UNINSTALL" --yes >/dev/null
[[ ! -e "$LINK_PREFIX/bin/stat-tracker-gui" ]] \
    || fail "home symlink left the GUI in place"
[[ ! -e "$LINK_BASE/var/home/user/.local/bin/stat-tracker-gui" ]] \
    || fail "home symlink left the real GUI in place"
pass "a symlinked home folder still removes the tracker files"

# The package command is printed once.
run_install "$HOME_DIR" "$PREFIX"
pac_both="$(env HOME="$HOME_DIR" PREFIX="$PREFIX" PATH="$TMP/fakepac:$PATH" \
    SCUFFED_SYSTEMCTL="$FAKE_CTL" \
    bash "$UNINSTALL" --yes 2>&1)"
pac_count="$(printf '%s\n' "$pac_both" | grep -c 'sudo pacman -R scuffed-stat-tracker' || true)"
[[ "$pac_count" -eq 1 ]] || fail "package command was printed $pac_count times: $pac_both"
pass "package remove command is printed once"

# --purge follows data-dir.conf, and only tracker files inside a broad folder.
write_dropin() {
    local home="$1" data="$2"
    local drop="$home/.config/systemd/user/scuffed-stat-tracker.service.d/data-dir.conf"
    mkdir -p "$(dirname "$drop")"
    cat > "$drop" << EOF
# scuffed-stat-tracker data_dir drop-in
[Service]
ReadWritePaths=-$data
EOF
}
for broad in ".config" "Documents" "Games"; do
    run_install "$HOME_DIR" "$PREFIX"
    DATA="$HOME_DIR/$broad"
    mkdir -p "$DATA/stats.surrealkv" "$DATA/other-app" "$HOME_DIR/.local/share/scuffed-stat-tracker/stats.surrealkv"
    printf 'games\n' > "$DATA/stats.surrealkv/db"
    printf 'stay\n' > "$DATA/keep-me.txt"
    printf 'other\n' > "$DATA/other-app/file"
    printf 'default\n' > "$HOME_DIR/.local/share/scuffed-stat-tracker/stats.surrealkv/db"
    write_dropin "$HOME_DIR" "$DATA"
    env HOME="$HOME_DIR" PREFIX="$PREFIX" \
        SCUFFED_SYSTEMCTL="$FAKE_CTL" \
        bash "$UNINSTALL" --purge --yes >/dev/null
    [[ -d "$DATA" ]] || fail "$broad folder was removed"
    [[ "$(cat "$DATA/keep-me.txt")" == "stay" ]] || fail "$broad neighbor was removed"
    [[ "$(cat "$DATA/other-app/file")" == "other" ]] || fail "$broad other app was removed"
    [[ ! -e "$DATA/stats.surrealkv" ]] || fail "$broad tracker database was left"
    [[ "$(cat "$HOME_DIR/.local/share/scuffed-stat-tracker/stats.surrealkv/db")" == "default" ]] \
        || fail "default saved games were removed while purging $broad"
    [[ -d "$PREFIX" ]] || fail "install folder was removed while purging $broad"
done
pass "purge uses data-dir.conf and leaves other files in broad folders"

echo "all uninstall path checks passed"
