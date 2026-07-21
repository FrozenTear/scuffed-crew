#!/usr/bin/env bash
# In-tarball installer for prebuilt Linux releases (no Rust/cargo).
#
# Expected layout (release asset root after extract):
#   bin/scuffed-stat-tracker
#   bin/stat-tracker-gui
#   lib/*          (optional — bundled OCR libs; RPATH $ORIGIN/../lib)
#   tessdata/eng.traineddata  (optional — runtime OCR model, since v0.3.0)
#   assets/scuffed-stat-tracker.desktop
#   assets/scuffed-stat-tracker.service
#   install.sh   (this file)
#   VERSION      (optional)
#
# Usage (from extracted tree):
#   ./install.sh
#   PREFIX=$HOME/.local ./install.sh
set -euo pipefail

PKG_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="${BIN_DIR:-$PREFIX/bin}"
LIB_DIR="${LIB_DIR:-$PREFIX/lib}"
DESKTOP_DIR="${DESKTOP_DIR:-$HOME/.local/share/applications}"
SYSTEMD_DIR="${SYSTEMD_DIR:-$HOME/.config/systemd/user}"
ASSETS_DIR="$PKG_ROOT/assets"
DAEMON_BIN="$PKG_ROOT/bin/scuffed-stat-tracker"
GUI_BIN="$PKG_ROOT/bin/stat-tracker-gui"
UNIT="scuffed-stat-tracker.service"
DESKTOP="scuffed-stat-tracker.desktop"

RED='\033[0;31m'
YLW='\033[1;33m'
GRN='\033[0;32m'
NC='\033[0m'

# All logging goes to stderr; stdout is reserved for any machine-parseable
# output. Mirrors the bootstrap.sh fix (3cd2c0c): a log line on stdout there
# shifted a mapfile parse and broke the release fetch. error() was already on
# stderr — info()/warn() now match it.
info()  { echo -e "${GRN}[install]${NC} $*" >&2; }
warn()  { echo -e "${YLW}[ warn ]${NC} $*" >&2; }
error() { echo -e "${RED}[error ]${NC} $*" >&2; }

# SKIP_INTEGRATION=<non-empty> installs binaries/libs only and skips the
# desktop entry + systemd user unit — so throwaway-PREFIX installs (clean-room
# tests, bootstrap smoke) don't pollute the real $HOME. Unset/empty = full
# install (default, unchanged). bootstrap.sh passes this through.
SKIP_INTEGRATION="${SKIP_INTEGRATION:-}"

# ── Layout checks ─────────────────────────────────────────────────────────────

if [[ ! -x "$DAEMON_BIN" && ! -f "$DAEMON_BIN" ]]; then
    error "missing prebuilt daemon: $DAEMON_BIN"
    error "This installer is for release tarballs. From a source checkout use crates/stat-tracker/install.sh instead."
    exit 1
fi
if [[ ! -f "$GUI_BIN" ]]; then
    error "missing prebuilt GUI: $GUI_BIN"
    exit 1
fi
if [[ ! -f "$ASSETS_DIR/$DESKTOP" ]]; then
    error "missing desktop entry: $ASSETS_DIR/$DESKTOP"
    exit 1
fi
if [[ ! -f "$ASSETS_DIR/$UNIT" ]]; then
    error "missing systemd unit: $ASSETS_DIR/$UNIT"
    exit 1
fi

if [[ -f "$PKG_ROOT/VERSION" ]]; then
    info "Installing version $(tr -d '\n' <"$PKG_ROOT/VERSION")"
fi

# ── Platform / runtime hints ──────────────────────────────────────────────────

case "$(uname -s)" in
    Linux) ;;
    *)
        error "Prebuilt releases are Linux-only (got $(uname -s))."
        exit 1
        ;;
esac

arch="$(uname -m)"
if [[ "$arch" != "x86_64" && "$arch" != "amd64" ]]; then
    warn "Release binaries are built for x86_64; this machine is $arch — they may not run."
fi

if ! groups 2>/dev/null | grep -qw input; then
    warn "You are not in the 'input' group."
    warn "The daemon reads keyboard events (Tab key) via evdev."
    warn "Add yourself and re-login:"
    warn "    sudo usermod -aG input \$USER"
    warn "Continuing anyway — you can fix this later."
    echo >&2
fi

# eng.traineddata locations across distros (daemon probes these at runtime too).
find_eng_traineddata() {
    local candidates=(
        "${TESSDATA_PREFIX:-}/eng.traineddata"
        "${TESSDATA_PREFIX:-}/tessdata/eng.traineddata"
        /usr/share/tessdata/eng.traineddata
        /usr/local/share/tessdata/eng.traineddata
        /usr/share/tesseract/tessdata/eng.traineddata
    )
    local d
    for d in /usr/share/tesseract-ocr/*/tessdata; do
        candidates+=("${d}/eng.traineddata")
    done
    local f
    for f in "${candidates[@]}"; do
        [[ -n "$f" && -f "$f" ]] && return 0
    done
    return 1
}

if ! find_eng_traineddata; then
    warn "eng.traineddata not found — install your distro's eng tessdata package."
    warn "  Arch:    sudo pacman -S tesseract-data-eng"
    warn "  Debian:  sudo apt install tesseract-ocr-eng"
    warn "  Fedora:  sudo dnf install tesseract-langpack-eng"
    warn "  Paths:   /usr/share/tessdata, /usr/share/tesseract-ocr/*/tessdata,"
    warn "           /usr/share/tesseract/tessdata, or TESSDATA_PREFIX"
    echo >&2
fi

# ── Install binaries ──────────────────────────────────────────────────────────

mkdir -p "$BIN_DIR"
install -m755 "$DAEMON_BIN" "$BIN_DIR/scuffed-stat-tracker"
install -m755 "$GUI_BIN"     "$BIN_DIR/stat-tracker-gui"
info "Installed binaries → $BIN_DIR"

# Bundled OCR library closure (portable releases). Daemon RPATH is
# $ORIGIN/../lib so libs must land next to bin under PREFIX.
if [[ -d "$PKG_ROOT/lib" ]] && compgen -G "$PKG_ROOT/lib/*" >/dev/null; then
    mkdir -p "$LIB_DIR"
    count=0
    for f in "$PKG_ROOT/lib"/*; do
        install -m755 "$f" "$LIB_DIR/$(basename "$f")"
        count=$((count + 1))
    done
    info "Installed $count bundled OCR libs → $LIB_DIR (RPATH \$ORIGIN/../lib)"
fi

# Bundled runtime eng model → user tessdata dir (first-priority lookup, no root,
# no distro tessdata package needed). Never clobber a user's own eng model
# (e.g. a tuned koverwatch or hand-placed eng.traineddata).
BUNDLED_ENG="$PKG_ROOT/tessdata/eng.traineddata"
USER_TESSDATA_DIR="$HOME/.local/share/scuffed-stat-tracker/tessdata"
USER_ENG="$USER_TESSDATA_DIR/eng.traineddata"
if [[ -f "$BUNDLED_ENG" ]]; then
    if [[ -f "$USER_ENG" ]]; then
        info "eng.traineddata already present at $USER_ENG — keeping yours (not overwriting)."
    else
        mkdir -p "$USER_TESSDATA_DIR"
        install -m644 "$BUNDLED_ENG" "$USER_ENG"
        info "Installed bundled eng.traineddata → $USER_ENG"
    fi
fi

if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    warn "$BIN_DIR is not in your PATH."
    warn "Add this to your shell config (~/.bashrc, ~/.zshrc, etc.):"
    warn "    export PATH=\"$BIN_DIR:\$PATH\""
fi

if [[ -n "$SKIP_INTEGRATION" ]]; then
    info "SKIP_INTEGRATION set — skipping desktop entry and systemd unit (binaries only)"
else
    # ── Desktop entry ─────────────────────────────────────────────────────────
    mkdir -p "$DESKTOP_DIR"
    install -m644 "$ASSETS_DIR/$DESKTOP" "$DESKTOP_DIR/$DESKTOP"
    if command -v update-desktop-database &>/dev/null; then
        update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
    fi
    info "Installed desktop entry → $DESKTOP_DIR"

    # ── systemd user service (installed, NOT enabled) ─────────────────────────
    mkdir -p "$SYSTEMD_DIR"
    install -m644 "$ASSETS_DIR/$UNIT" "$SYSTEMD_DIR/$UNIT"
    if command -v systemctl &>/dev/null; then
        systemctl --user daemon-reload 2>/dev/null || true
    fi
    info "Installed systemd service → $SYSTEMD_DIR (not enabled)"
fi

# ── Smoke check ───────────────────────────────────────────────────────────────

if ! "$BIN_DIR/scuffed-stat-tracker" --version >/dev/null 2>&1; then
    warn "daemon --version failed. Missing host libs (display stack/evdev) or bundled lib/ not installed beside bin?"
else
    info "daemon binary runs ($("$BIN_DIR/scuffed-stat-tracker" --version))"
fi

# ── Done ──────────────────────────────────────────────────────────────────────
# Human-facing summary to stderr too — stdout stays reserved for machine output
# (there is none), so an output-parsing caller never sees install chatter.

{
    echo
    echo -e "${GRN}Installation complete.${NC}"
    echo
    echo "  Launch the app:   stat-tracker-gui"
    echo "  Or find it in your application launcher: Scuffed Stat Tracker"
    echo
    echo "  The GUI's Status page has Start / Stop and Enable Autostart buttons."
    echo "  Autostart (systemd) starts the daemon automatically on login."
    echo
    echo "  First run: open the GUI, go to Settings, paste your server URL"
    echo "  and daemon token (from the web UI under My Stats → Daemon Tokens)."
    echo
    echo "  Source rebuilds (dev): crates/stat-tracker/install.sh (requires cargo)."
    echo
} >&2
