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
# Removes only paths from install-paths.sh. A package-manager install
# (pacman/AUR, apt/dpkg) or a path under /usr is left untouched. The
# remove command is printed instead.
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
UNIT="scuffed-stat-tracker.service"
SESSION_UNIT="scuffed-stat-tracker-session.service"

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

# Package managers and system prefixes are never deleted.
if [[ -e "$GUI_BIN" ]]; then
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
if is_system_install_path "$GUI_BIN" || is_system_install_path "$PREFIX"; then
    error "Refusing to remove $GUI_BIN."
    error "That path is outside the bootstrap.sh user install paths."
    error "Nothing was removed."
    exit 1
fi

# Stop and disable before any file is removed, so a restart cannot
# rewrite a unit we are about to delete.
if [[ -x "$SYSTEMCTL_BIN" ]] || command -v "$SYSTEMCTL_BIN" >/dev/null 2>&1; then
    "$SYSTEMCTL_BIN" --user disable --now "$UNIT" >/dev/null 2>&1 || true
    "$SYSTEMCTL_BIN" --user stop "$SESSION_UNIT" >/dev/null 2>&1 || true
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

path_is_under() {
    local child="$1" parent="$2"
    [[ "$child" == "$parent"/* ]]
}

declare -A ALWAYS_EXACT=()
declare -a ALWAYS_FILES=()
while read -r cat when path; do
    case "$cat" in
        libdir|data|config) continue ;;
    esac
    if [[ "$when" == "always" ]]; then
        ALWAYS_EXACT["$path"]=1
        ALWAYS_FILES+=("$path")
    fi
done < <(expanded_install_paths "$HOME" "$PREFIX" always)

declare -A REMOVE=()
for f in "${ALWAYS_FILES[@]}"; do
    REMOVE["$f"]=1
done

if [[ -f "$MANIFEST" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
        [[ "$line" == /* ]] || continue
        if [[ -n "${ALWAYS_EXACT[$line]:-}" ]] || path_is_under "$line" "$LIB_DIR"; then
            REMOVE["$line"]=1
        else
            warn "not removing $line (outside the install path list)"
        fi
    done < "$MANIFEST"
fi

removed=0
for f in "${!REMOVE[@]}"; do
    if [[ ! -e "$f" && ! -L "$f" ]]; then
        continue
    fi
    if ! parents_are_real "$f"; then
        warn "not removing $f (a parent directory is a symlink)"
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
