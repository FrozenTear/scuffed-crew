#!/usr/bin/env bash
# Uninstaller for bootstrap.sh / install.sh user installs.
# Installed to $PREFIX/bin/scuffed-stat-tracker-uninstall. Also runnable
# from an extracted release tarball, and via `bootstrap.sh --uninstall`.
#
# Usage:
#   scuffed-stat-tracker-uninstall [--purge] [--yes]
#
#   --purge   also delete local data and config
#   --yes     never prompt (keeps data unless --purge)
#
# Removes exactly the files install.sh recorded in the install manifest.
# No globs and no "everything under this directory". An older install
# with no manifest falls back to the fixed list in install-paths.sh and
# shows that list before deleting. config.toml stays unless --purge.
#
# A package install is detected by who owns the GUI binary
# (`pacman -Qo`, `dpkg -S`), not by its path. An AppImage is not a
# package. The remove command is printed and nothing is deleted.
#
# Env (must match install time):
#   PREFIX        default ~/.local
#   HOME
#   SCUFFED_SYSTEMCTL   systemctl stand-in for tests
set -euo pipefail

HERE="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PATHS_FILE=""
if [[ -f "$HERE/install-paths.sh" ]]; then
    PATHS_FILE="$HERE/install-paths.sh"
elif [[ -f "$HERE/../lib/scuffed-stat-tracker/install-paths.sh" ]]; then
    PATHS_FILE="$HERE/../lib/scuffed-stat-tracker/install-paths.sh"
fi
if [[ -z "$PATHS_FILE" ]]; then
    echo "missing install-paths.sh (expected next to uninstall.sh or under lib/scuffed-stat-tracker)" >&2
    exit 1
fi
# shellcheck source=install-paths.sh
source "$PATHS_FILE"

PREFIX="${PREFIX:-${STAT_TRACKER_PREFIX:-$HOME/.local}}"
SYSTEMCTL_BIN="${SCUFFED_SYSTEMCTL:-systemctl}"

RED='\033[0;31m'
YLW='\033[1;33m'
GRN='\033[0;32m'
NC='\033[0m'
info()  { echo -e "${GRN}[uninstall]${NC} $*" >&2; }
warn()  { echo -e "${YLW}[ warn ]${NC} $*" >&2; }
error() { echo -e "${RED}[error ]${NC} $*" >&2; }

require_path() {
    local label="$1"
    local path
    path="$(install_path_named "$HOME" "$PREFIX" "$2" "$3")" || {
        error "install path list has no $label"
        exit 1
    }
    printf '%s\n' "$path"
}

GUI_BIN="$(require_path "GUI binary" bin stat-tracker-gui)"
LIB_DIR="$(require_path "library directory" libdir scuffed-stat-tracker)"
DATA_DIR="$(require_path "data directory" data scuffed-stat-tracker)"
CONFIG_DIR="$(require_path "config directory" config scuffed-stat-tracker)"
MANIFEST="$(require_path "install manifest" manifest install-manifest.txt)"

PURGE=0
ASSUME_YES=0
for arg in "$@"; do
    case "$arg" in
        --purge) PURGE=1 ;;
        --yes|-y) ASSUME_YES=1 ;;
        -h|--help)
            sed -n '2,22p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
            exit 0
            ;;
        *)
            error "unknown argument: $arg (try --help)"
            exit 1
            ;;
    esac
done

# Package manager owns the GUI binary: print the command, delete nothing.
# Path is not consulted. An AppImage is not owned, so it stays a script install.
if [[ -e "$GUI_BIN" || -L "$GUI_BIN" ]]; then
    owner=""
    if owner="$(package_owner_of "$GUI_BIN")"; then
        kind="${owner%% *}"
        pkg="${owner#* }"
        package_remove_command "$kind" "$pkg"
        info "Package manager owns $GUI_BIN. Nothing was removed."
        info "Run: $(package_remove_command "$kind" "$pkg")"
        exit 0
    fi
fi

# A parent symlink would make a listed path point somewhere else.
parents_are_real() {
    local dir
    dir="$(dirname "$1")"
    while [[ "$dir" != "/" && -n "$dir" && "$dir" != "." ]]; do
        if [[ -L "$dir" ]]; then
            return 1
        fi
        dir="$(dirname "$dir")"
    done
    return 0
}

# Stop and disable the user service, its timer, and the session unit
# before any file is removed. A restart must not rewrite a unit we delete.
disable_user_units() {
    if [[ -x "$SYSTEMCTL_BIN" ]] || command -v "$SYSTEMCTL_BIN" >/dev/null 2>&1; then
        local unit
        while read -r unit; do
            [[ -n "$unit" ]] || continue
            "$SYSTEMCTL_BIN" --user disable --now "$unit" >/dev/null 2>&1 || true
        done < <(systemd_units_to_disable)
    fi
}

# Exact absolute paths only. A glob character is not expanded.
declare -a REMOVE_FILES=()
if [[ -f "$MANIFEST" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line#"${line%%[![:space:]]*}"}"
        line="${line%"${line##*[![:space:]]}"}"
        [[ "$line" == /* ]] || continue
        case "$line" in
            *'*'*|*'?'*|*'['*)
                warn "not removing $line (refusing a pattern)"
                continue
                ;;
        esac
        REMOVE_FILES+=("$line")
    done < "$MANIFEST"
else
    while read -r cat when path; do
        case "$cat" in
            libdir|data|config) continue ;;
        esac
        [[ "$when" == "always" ]] || continue
        REMOVE_FILES+=("$path")
    done < <(expanded_install_paths "$HOME" "$PREFIX" always)
    info "No install manifest at $MANIFEST."
    info "These fixed paths would be removed:"
    for f in "${REMOVE_FILES[@]}"; do
        printf '  %s\n' "$f" >&2
    done
    if [[ $ASSUME_YES -eq 0 ]]; then
        reply=""
        printf '%b' "${YLW}[uninstall]${NC} Remove these paths? [y/N] " >&2
        IFS= read -r reply || reply=""
        case "$reply" in
            [yY]*) ;;
            *)
                info "Uninstall cancelled. Nothing was removed."
                exit 0
                ;;
        esac
    fi
fi

disable_user_units

removed=0
for f in "${REMOVE_FILES[@]}"; do
    if [[ ! -e "$f" && ! -L "$f" ]]; then
        continue
    fi
    if ! parents_are_real "$f"; then
        warn "not removing $f (a parent directory is a symlink)"
        continue
    fi
    # config.toml is local data. The data checkbox (--purge) removes the
    # config directory later. A manifest line must not delete it early.
    if [[ "$(basename "$f")" == "config.toml" ]]; then
        continue
    fi
    if [[ "$f" == "$DATA_DIR" || "$f" == "$CONFIG_DIR" || "$f" == "/" || "$f" == "$HOME" ]]; then
        warn "not removing $f (local data stays unless --purge)"
        continue
    fi
    if [[ -d "$f" && ! -L "$f" ]]; then
        warn "not removing directory $f (only listed files are unlinked here)"
        continue
    fi
    rm -f -- "$f"
    removed=$((removed + 1))
done
info "Removed $removed installed file(s)."

if [[ -d "$LIB_DIR" && ! -L "$LIB_DIR" ]] && parents_are_real "$LIB_DIR"; then
    find -P "$LIB_DIR" -depth -type d -exec rmdir {} \; 2>/dev/null || true
fi

if [[ -x "$SYSTEMCTL_BIN" ]] || command -v "$SYSTEMCTL_BIN" >/dev/null 2>&1; then
    "$SYSTEMCTL_BIN" --user daemon-reload >/dev/null 2>&1 || true
fi
DESKTOP_DIR="$(dirname "$(require_path "desktop entry" desktop scuffed-stat-tracker.desktop)")"
if command -v update-desktop-database >/dev/null 2>&1; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi

if [[ $PURGE -eq 0 && $ASSUME_YES -eq 0 && -t 0 && -t 2 ]]; then
    reply=""
    printf '%b' "${YLW}[uninstall]${NC} Also delete local data ($DATA_DIR: games database, debug crops, shadow logs) and config ($CONFIG_DIR, including the sync token)? [y/N] " >&2
    IFS= read -r reply || reply=""
    case "$reply" in
        [yY]*) PURGE=1 ;;
    esac
fi

if [[ $PURGE -eq 1 ]]; then
    for d in "$DATA_DIR" "$CONFIG_DIR"; do
        case "$d" in
            /|""|"$HOME")
                error "refusing to delete $d"
                exit 1
                ;;
        esac
        if [[ -L "$d" ]]; then
            warn "not deleting $d (symlink)"
            continue
        fi
        if ! parents_are_real "$d"; then
            warn "not deleting $d (a parent directory is a symlink)"
            continue
        fi
        if [[ -d "$d" ]]; then
            rm -rf -- "$d"
        elif [[ -e "$d" ]]; then
            rm -f -- "$d"
        fi
    done
    info "Deleted local data and config."
else
    info "Kept local data ($DATA_DIR) and config ($CONFIG_DIR)."
fi

info "Uninstall complete."
