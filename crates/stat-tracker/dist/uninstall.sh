#!/usr/bin/env bash
# Uninstaller for prebuilt scuffed-stat-tracker installs (see install.sh).
# Installed to $PREFIX/bin/scuffed-stat-tracker-uninstall; also runnable from
# an extracted release tarball.
#
# Usage:
#   scuffed-stat-tracker-uninstall [--purge] [--yes]
#
#   --purge   also delete app data (match log, OCR models) and config
#             (server URL, daemon token). Default keeps both so a reinstall
#             picks up where it left off.
#   --yes     never prompt (non-interactive; keeps data unless --purge)
#
# Env (must match the values used at install time):
#   PREFIX        default ~/.local
#   BIN_DIR       default $PREFIX/bin
#   DESKTOP_DIR   default ~/.local/share/applications
#   SYSTEMD_DIR   default ~/.config/systemd/user
set -euo pipefail

PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="${BIN_DIR:-$PREFIX/bin}"
DESKTOP_DIR="${DESKTOP_DIR:-$HOME/.local/share/applications}"
SYSTEMD_DIR="${SYSTEMD_DIR:-$HOME/.config/systemd/user}"
# App dirs are $HOME-anchored (dirs::data_dir / dirs::config_dir), not PREFIX.
DATA_DIR="$HOME/.local/share/scuffed-stat-tracker"
CONFIG_DIR="$HOME/.config/scuffed-stat-tracker"
MANIFEST="$PREFIX/share/scuffed-stat-tracker/install-manifest.txt"
UNIT="scuffed-stat-tracker.service"
DESKTOP="scuffed-stat-tracker.desktop"

RED='\033[0;31m'
YLW='\033[1;33m'
GRN='\033[0;32m'
NC='\033[0m'
info()  { echo -e "${GRN}[uninstall]${NC} $*" >&2; }
warn()  { echo -e "${YLW}[ warn ]${NC} $*" >&2; }
error() { echo -e "${RED}[error ]${NC} $*" >&2; }

PURGE=0
ASSUME_YES=0
for arg in "$@"; do
    case "$arg" in
        --purge) PURGE=1 ;;
        --yes|-y) ASSUME_YES=1 ;;
        -h|--help)
            sed -n '2,20p' "${BASH_SOURCE[0]}" | sed 's/^# \{0,1\}//' >&2
            exit 0
            ;;
        *)
            error "unknown argument: $arg (try --help)"
            exit 1
            ;;
    esac
done

# ── Stop / disable the daemon ─────────────────────────────────────────────────

if command -v systemctl &>/dev/null; then
    systemctl --user disable --now "$UNIT" 2>/dev/null || true
fi

# ── Collect installed files ───────────────────────────────────────────────────

files=()
if [[ -f "$MANIFEST" ]]; then
    while IFS= read -r line; do
        # Manifest lines are absolute paths; ignore anything else.
        [[ "$line" == /* ]] && files+=("$line")
    done < "$MANIFEST"
else
    warn "No install manifest at $MANIFEST (install predates manifests?)."
    warn "Removing the known default paths instead."
    files=(
        "$BIN_DIR/scuffed-stat-tracker"
        "$BIN_DIR/stat-tracker-gui"
        "$BIN_DIR/scuffed-stat-tracker-uninstall"
        "$DESKTOP_DIR/$DESKTOP"
        "$SYSTEMD_DIR/$UNIT"
    )
    warn "Bundled OCR libs in $PREFIX/lib cannot be identified without a"
    warn "manifest — list them from the release tarball and remove by hand:"
    warn "    tar tzf scuffed-stat-tracker-linux-x86_64.tar.gz | grep '/lib/'"
fi

removed=0
for f in "${files[@]}"; do
    if [[ -f "$f" || -L "$f" ]]; then
        rm -f "$f"
        removed=$((removed + 1))
    fi
done
info "Removed $removed installed file(s)."

if command -v systemctl &>/dev/null; then
    systemctl --user daemon-reload 2>/dev/null || true
fi
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi

# ── App data + config ─────────────────────────────────────────────────────────

if [[ $PURGE -eq 0 && $ASSUME_YES -eq 0 && -t 0 && -t 2 ]]; then
    reply=""
    printf '%b' "${YLW}[uninstall]${NC} Also delete app data ($DATA_DIR: match log, OCR models) and config ($CONFIG_DIR: server URL, daemon token)? [y/N] " >&2
    IFS= read -r reply || reply=""
    case "$reply" in
        [yY]*) PURGE=1 ;;
    esac
fi

# Manifest is spent either way; its dir is only removed if nothing else is left
# ($PREFIX/share/scuffed-stat-tracker doubles as the data dir on default installs).
rm -f "$MANIFEST"
rmdir "$(dirname "$MANIFEST")" 2>/dev/null || true

if [[ $PURGE -eq 1 ]]; then
    rm -rf "$DATA_DIR" "$CONFIG_DIR"
    info "Deleted app data and config."
else
    info "Kept app data ($DATA_DIR) and config ($CONFIG_DIR)."
    info "Delete them later with: rm -rf $DATA_DIR $CONFIG_DIR"
fi

info "Uninstall complete."
