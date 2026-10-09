#!/usr/bin/env bash
# Removes the tracker from this computer.
# Installed to $PREFIX/bin/scuffed-stat-tracker-uninstall. Also runnable
# from an extracted release, and via `bootstrap.sh --uninstall`.
#
# Usage:
#   scuffed-stat-tracker-uninstall [--purge] [--yes]
#
#   --purge   also delete saved games and settings
#   --yes     do not ask (keeps saved games unless --purge)
#
# Removes the files the installer recorded. It does not guess by name.
# A recorded path with `..`, or one that does not resolve under your home
# folder or the install folder, is left alone. An older install with no
# record shows the original files and asks first. Settings stay unless
# --purge. The file list is shown, and removal waits for a yes, unless
# --yes is set.
#
# If a system package owns this copy, the remove command is printed and
# nothing is deleted.
#
# Env (must match install time):
#   PREFIX        default ~/.local
#   HOME
#   SCUFFED_SYSTEMCTL   stand-in for systemctl
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
            sed -n '2,25p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
            exit 0
            ;;
        *)
            error "unknown argument: $arg (try --help)"
            exit 1
            ;;
    esac
done

# A system package owns this program: print the command, delete nothing.
# The folder is not consulted.
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

# A missing unit, or no user service manager, is not a running tracker.
# Anything else from disable is a real failure: stop and delete nothing.
stop_failure_is_harmless() {
    local text
    text="$(printf '%s' "$1" | tr '[:upper:]' '[:lower:]')"
    case "$text" in
        *"not found"*|*"does not exist"*|*"not loaded"*|*"no such file"*|\
        *"failed to connect to bus"*|*"not been booted with systemd"*|\
        *"no medium found"*|*"failed to connect to user scope"*)
            return 0
            ;;
    esac
    return 1
}

# Stop and disable the user service, its timer, and the session unit
# before any file is removed. A restart must not rewrite a unit we delete.
disable_user_units() {
    if [[ ! -x "$SYSTEMCTL_BIN" ]] && ! command -v "$SYSTEMCTL_BIN" >/dev/null 2>&1; then
        error "Could not stop the tracker. Nothing was removed."
        exit 1
    fi
    local unit err state
    while read -r unit; do
        [[ -n "$unit" ]] || continue
        err=""
        if ! err="$("$SYSTEMCTL_BIN" --user disable --now "$unit" 2>&1)"; then
            if ! stop_failure_is_harmless "$err"; then
                error "Could not stop the tracker. Nothing was removed."
                exit 1
            fi
        fi
        if [[ "$unit" == "scuffed-stat-tracker.service" ]]; then
            state="$("$SYSTEMCTL_BIN" --user is-active "$unit" 2>/dev/null || true)"
            state="${state%%$'\n'*}"
            case "$state" in
                active|activating)
                    error "The tracker is still running. Nothing was removed."
                    exit 1
                    ;;
            esac
        fi
    done < <(systemd_units_to_disable)
}

# True when $1 is a child of $2, not $2 itself.
path_strictly_inside() {
    local path="$1" root="$2"
    [[ -n "$root" && "$path" == "$root"/* ]]
}

# Reject `..` and patterns. Resolve the parent and require the file to sit
# under the home folder or the install folder. Prints that resolved path.
confine_file() {
    local path="$1"
    [[ "$path" == /* ]] || return 1
    case "$path" in
        *'*'*|*'?'*|*'['*) return 1 ;;
        ..|../*|*/..|*/../*) return 1 ;;
    esac
    local parent name canon_parent canon_home canon_prefix resolved
    parent="$(dirname -- "$path")"
    name="$(basename -- "$path")"
    [[ -n "$name" && "$name" != "." && "$name" != ".." ]] || return 1
    [[ -d "$parent" && ! -L "$parent" ]] || return 1
    canon_parent="$(cd -P -- "$parent" && pwd)" || return 1
    resolved="$canon_parent/$name"
    if [[ -d "$HOME" ]]; then
        canon_home="$(cd -P -- "$HOME" && pwd)" || return 1
    else
        canon_home="$HOME"
    fi
    if [[ -d "$PREFIX" ]]; then
        canon_prefix="$(cd -P -- "$PREFIX" && pwd)" || return 1
    else
        canon_prefix="$PREFIX"
    fi
    if path_strictly_inside "$resolved" "$canon_home" \
        || path_strictly_inside "$resolved" "$canon_prefix"; then
        printf '%s\n' "$resolved"
        return 0
    fi
    return 1
}

# Exact absolute paths only. A glob character is not expanded.
# Each path must resolve under the home folder or the install folder.
declare -a REMOVE_FILES=()
if [[ -f "$MANIFEST" ]]; then
    while IFS= read -r line || [[ -n "$line" ]]; do
        line="${line#"${line%%[![:space:]]*}"}"
        line="${line%"${line##*[![:space:]]}"}"
        [[ -n "$line" ]] || continue
        [[ "$line" == /* ]] || continue
        resolved=""
        if ! resolved="$(confine_file "$line")"; then
            warn "not removing $line (outside your home folder and the install folder)"
            continue
        fi
        REMOVE_FILES+=("$resolved")
    done < "$MANIFEST"
else
    while read -r cat when path; do
        case "$cat" in
            libdir|data|config) continue ;;
        esac
        [[ "$when" == "always" ]] || continue
        resolved=""
        if resolved="$(confine_file "$path")"; then
            REMOVE_FILES+=("$resolved")
        fi
    done < <(expanded_install_paths "$HOME" "$PREFIX" always)
    info "No install manifest at $MANIFEST."
    info "These are the files from the original install."
fi

if [[ ${#REMOVE_FILES[@]} -eq 0 ]]; then
    info "No installed files to remove."
else
    info "These files will be removed:"
    for f in "${REMOVE_FILES[@]}"; do
        printf '  %s\n' "$f" >&2
    done
    if [[ $ASSUME_YES -eq 0 ]]; then
        reply=""
        printf '%b' "${YLW}[uninstall]${NC} Remove these files? [y/N] " >&2
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
declare -a REMOVED_PATHS=()
for idx in "${!REMOVE_FILES[@]}"; do
    f="${REMOVE_FILES[$idx]}"
    if [[ ! -e "$f" && ! -L "$f" ]]; then
        continue
    fi
    if ! parents_are_real "$f"; then
        warn "not removing $f (a parent directory is a symlink)"
        continue
    fi
    # Settings stay unless --purge removes the settings folder later.
    if [[ "$(basename "$f")" == "config.toml" ]]; then
        continue
    fi
    if [[ "$f" == "$DATA_DIR" || "$f" == "$CONFIG_DIR" || "$f" == "/" || "$f" == "$HOME" ]]; then
        warn "not removing $f (saved games stay unless --purge)"
        continue
    fi
    if [[ -d "$f" && ! -L "$f" ]]; then
        warn "not removing directory $f (only listed files are unlinked here)"
        continue
    fi
    if ! rm -f -- "$f"; then
        left_text=""
        for rest in "${REMOVE_FILES[@]:$idx}"; do
            if [[ -n "$left_text" ]]; then
                left_text="$left_text, $rest"
            else
                left_text="$rest"
            fi
        done
        removed_text=""
        for done_path in "${REMOVED_PATHS[@]}"; do
            if [[ -n "$removed_text" ]]; then
                removed_text="$removed_text, $done_path"
            else
                removed_text="$done_path"
            fi
        done
        if [[ -z "$removed_text" ]]; then
            error "Uninstall stopped. Nothing was removed. Still there: $left_text."
        else
            error "Uninstall stopped. Removed $removed_text. Still there: $left_text."
        fi
        exit 1
    fi
    REMOVED_PATHS+=("$f")
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
        resolved=""
        if ! resolved="$(confine_file "$d")"; then
            warn "not deleting $d (outside your home folder and the install folder)"
            continue
        fi
        d="$resolved"
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
