#!/usr/bin/env bash
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BIN_DIR="$HOME/.local/bin"
DESKTOP_DIR="$HOME/.local/share/applications"
SYSTEMD_DIR="$HOME/.config/systemd/user"
ASSETS="$REPO_ROOT/crates/stat-tracker/assets"

RED='\033[0;31m'
YLW='\033[1;33m'
GRN='\033[0;32m'
NC='\033[0m'

info()  { echo -e "${GRN}[install]${NC} $*"; }
warn()  { echo -e "${YLW}[ warn ]${NC} $*"; }
error() { echo -e "${RED}[error ]${NC} $*" >&2; }

# ── Prerequisites ─────────────────────────────────────────────────────────────

if ! command -v cargo &>/dev/null; then
    error "cargo not found — install Rust from https://rustup.rs"
    exit 1
fi

# Check input group (needed for evdev keyboard monitoring)
if ! groups | grep -qw input; then
    warn "You are not in the 'input' group."
    warn "The daemon reads keyboard events (Tab key) via evdev."
    warn "Add yourself and re-login:"
    warn "    sudo usermod -aG input \$USER"
    warn "Continuing anyway — you can fix this later."
    echo
fi

# ── OCR dependencies ──────────────────────────────────────────────────────────

if ! command -v tesseract &>/dev/null; then
    warn "tesseract not found — OCR will not work for source builds that link system OCR."
    warn "Install:  sudo pacman -S tesseract tesseract-data-eng"
    echo
elif ! { ls /usr/share/tessdata/eng.traineddata \
             /usr/local/share/tessdata/eng.traineddata \
             /usr/share/tesseract/tessdata/eng.traineddata \
             /usr/share/tesseract-ocr/*/tessdata/eng.traineddata \
             "${TESSDATA_PREFIX:-}/eng.traineddata" \
             "${TESSDATA_PREFIX:-}/tessdata/eng.traineddata" 2>/dev/null | grep -q .; }; then
    warn "eng.traineddata not found — install your distro's eng tessdata package."
    warn "  Arch: tesseract-data-eng · Debian: tesseract-ocr-eng · Fedora: tesseract-langpack-eng"
    echo
fi

# ── Build ─────────────────────────────────────────────────────────────────────

info "Building daemon (release)..."
cargo build --release -p scuffed-stat-tracker \
    --bin scuffed-stat-tracker \
    2>&1

info "Building GUI (release, --features gui)..."
cargo build --release -p scuffed-stat-tracker \
    --bin stat-tracker-gui \
    --features gui \
    2>&1

# ── Install binaries ──────────────────────────────────────────────────────────

mkdir -p "$BIN_DIR"
install -m755 "$REPO_ROOT/target/release/scuffed-stat-tracker" "$BIN_DIR/scuffed-stat-tracker"
install -m755 "$REPO_ROOT/target/release/stat-tracker-gui"     "$BIN_DIR/stat-tracker-gui"
info "Installed binaries → $BIN_DIR"

# ── Generate koverwatch tessdata ───────────────────────────────────────────────
# Run after install so we use the freshly built binary (which has --generate-tessdata).
# Also avoids the PID-conflict problem if a daemon is already running.

info "OCR will use eng tessdata (default). Koverwatch tessdata can be generated later from the GUI → Settings → Install Koverwatch Tessdata"

# Ensure ~/.local/bin is on PATH
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    warn "$BIN_DIR is not in your PATH."
    warn "Add this to your shell config (~/.bashrc, ~/.zshrc, etc.):"
    warn "    export PATH=\"\$HOME/.local/bin:\$PATH\""
fi

# ── Desktop entry ─────────────────────────────────────────────────────────────

mkdir -p "$DESKTOP_DIR"
install -m644 "$ASSETS/scuffed-stat-tracker.desktop" \
    "$DESKTOP_DIR/scuffed-stat-tracker.desktop"

# Refresh the desktop database so the launcher picks it up immediately
if command -v update-desktop-database &>/dev/null; then
    update-desktop-database "$DESKTOP_DIR" 2>/dev/null || true
fi
info "Installed desktop entry → $DESKTOP_DIR"

# ── systemd user service (installed, NOT enabled) ─────────────────────────────

mkdir -p "$SYSTEMD_DIR"
install -m644 "$ASSETS/scuffed-stat-tracker.service" \
    "$SYSTEMD_DIR/scuffed-stat-tracker.service"

if command -v systemctl &>/dev/null; then
    systemctl --user daemon-reload 2>/dev/null || true
fi
info "Installed systemd service → $SYSTEMD_DIR (not enabled)"

# ── Done ──────────────────────────────────────────────────────────────────────

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
