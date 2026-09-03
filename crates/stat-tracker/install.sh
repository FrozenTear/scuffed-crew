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

# All logging goes to stderr; stdout is reserved for any machine-parseable
# output. Mirrors the bootstrap.sh fix (3cd2c0c): a log line on stdout there
# shifted a mapfile parse and broke the release fetch. error() was already on
# stderr — info()/warn() now match it.
info()  { echo -e "${GRN}[install]${NC} $*" >&2; }
warn()  { echo -e "${YLW}[ warn ]${NC} $*" >&2; }
error() { echo -e "${RED}[error ]${NC} $*" >&2; }

# Quote a path for Desktop Entry Exec/TryExec (Freedesktop reserved chars).
desktop_exec_value() {
    local p="$1"
    case "$p" in
        *[[:space:]\"\'\\\<\>~\|\&\;\$\*\?\#\(\)\`]*)
            p="${p//\\/\\\\}"
            p="${p//\"/\\\"}"
            printf '"%s"' "$p"
            ;;
        *)
            printf '%s' "$p"
            ;;
    esac
}

# Resolve BIN_DIR/stat-tracker-gui to an absolute path. App launchers
# (AerynOS / Cosmic / GNOME) often omit ~/.local/bin from session PATH,
# so a bare Exec=stat-tracker-gui works from a terminal but not the menu.
absolute_gui_bin() {
    local gui="$BIN_DIR/stat-tracker-gui"
    if [[ "$gui" != /* ]]; then
        gui="$(cd "$(dirname "$gui")" && pwd)/stat-tracker-gui"
    fi
    printf '%s' "$gui"
}

write_desktop_entry() {
    local src="$1" dest="$2" gui_bin="$3"
    local exec_val
    exec_val="$(desktop_exec_value "$gui_bin")"
    mkdir -p "$(dirname "$dest")"
    {
        while IFS= read -r line || [[ -n "$line" ]]; do
            case "$line" in
                Exec=*)    printf 'Exec=%s\n' "$exec_val" ;;
                TryExec=*) printf 'TryExec=%s\n' "$exec_val" ;;
                *)         printf '%s\n' "$line" ;;
            esac
        done < "$src"
    } > "$dest"
    chmod 644 "$dest"
}

refresh_desktop_database() {
    local dir="$1"
    if command -v update-desktop-database &>/dev/null; then
        update-desktop-database "$dir" 2>/dev/null || true
        info "Refreshed desktop database → $dir"
    else
        warn "update-desktop-database not found (desktop-file-utils)."
        warn "If the app launcher does not show Scuffed Stat Tracker, run:"
        warn "    update-desktop-database $dir"
        warn "then log out/in. gtk-update-icon-cache is not needed (theme Icon)."
    fi
}

# SKIP_INTEGRATION=<non-empty> installs binaries/libs only and skips the
# desktop entry + systemd user unit — so throwaway-PREFIX installs (clean-room
# tests, bootstrap smoke) don't pollute the real $HOME. Unset/empty = full
# install (default, unchanged).
SKIP_INTEGRATION="${SKIP_INTEGRATION:-}"

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
    echo >&2
fi

# Optional tray (Hide-to-tray). Missing is fine — the Iced window still starts.
appindicator_found=0
if ldconfig -p 2>/dev/null | grep -qE 'libayatana-appindicator3|libappindicator3'; then
    appindicator_found=1
else
    for p in /usr/lib/libayatana-appindicator3.so.1 \
             /usr/lib64/libayatana-appindicator3.so.1 \
             /usr/lib/x86_64-linux-gnu/libayatana-appindicator3.so.1 \
             /usr/lib/libappindicator3.so.1 \
             /usr/lib64/libappindicator3.so.1 \
             /usr/lib/x86_64-linux-gnu/libappindicator3.so.1; do
        if [[ -e "$p" ]]; then
            appindicator_found=1
            break
        fi
    done
fi
if [[ "$appindicator_found" -eq 0 ]]; then
    warn "Ayatana AppIndicator not found — stat-tracker-gui will start without a tray."
    warn "Hide-to-tray needs libayatana-appindicator3 (or libappindicator3)."
    warn "  Debian/Ubuntu: sudo apt install libayatana-appindicator3-1"
    warn "  Fedora:        sudo dnf install libayatana-appindicator-gtk3"
    warn "  Arch:          sudo pacman -S libayatana-appindicator"
    warn "AerynOS may not ship this package; the window still works."
    echo >&2
fi

# ── OCR dependencies ──────────────────────────────────────────────────────────

if ! command -v tesseract &>/dev/null; then
    warn "tesseract not found — OCR will not work for source builds that link system OCR."
    warn "Install:  sudo pacman -S tesseract tesseract-data-eng"
    echo >&2
elif ! { ls /usr/share/tessdata/eng.traineddata \
             /usr/local/share/tessdata/eng.traineddata \
             /usr/share/tesseract/tessdata/eng.traineddata \
             /usr/share/tesseract-ocr/*/tessdata/eng.traineddata \
             "${TESSDATA_PREFIX:-}/eng.traineddata" \
             "${TESSDATA_PREFIX:-}/tessdata/eng.traineddata" 2>/dev/null | grep -q .; }; then
    warn "eng.traineddata not found — install your distro's eng tessdata package."
    warn "  Arch: tesseract-data-eng · Debian: tesseract-ocr-eng · Fedora: tesseract-langpack-eng"
    echo >&2
fi

# ── Build ─────────────────────────────────────────────────────────────────────

info "Building daemon (release)..."
cargo build --release -p scuffed-stat-tracker \
    --bin scuffed-stat-tracker \
    2>&1

info "Building GUI (release, scuffed-stat-tracker-ui / Iced)..."
cargo build --release -p scuffed-stat-tracker-ui \
    --bin stat-tracker-gui \
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

# Ensure ~/.local/bin is on PATH (terminals). Launchers often omit it.
if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    warn "$BIN_DIR is not in your shell PATH."
    warn "Add this to your shell config (~/.bashrc, ~/.zshrc, etc.):"
    warn "    export PATH=\"\$HOME/.local/bin:\$PATH\""
    warn "App launchers (AerynOS / Cosmic / GNOME) often omit this dir even"
    warn "when a terminal has it — the .desktop Exec is an absolute path so"
    warn "the menu does not depend on session PATH."
fi

if [[ -n "$SKIP_INTEGRATION" ]]; then
    info "SKIP_INTEGRATION set — skipping desktop entry and systemd unit (binaries only)"
else
    # ── Desktop entry ─────────────────────────────────────────────────────────
    write_desktop_entry "$ASSETS/scuffed-stat-tracker.desktop" \
        "$DESKTOP_DIR/scuffed-stat-tracker.desktop" "$(absolute_gui_bin)"
    refresh_desktop_database "$DESKTOP_DIR"
    info "Installed desktop entry → $DESKTOP_DIR (Exec=$(absolute_gui_bin))"

    # ── systemd user service (installed, NOT enabled) ─────────────────────────
    mkdir -p "$SYSTEMD_DIR"
    install -m644 "$ASSETS/scuffed-stat-tracker.service" \
        "$SYSTEMD_DIR/scuffed-stat-tracker.service"

    if command -v systemctl &>/dev/null; then
        systemctl --user daemon-reload 2>/dev/null || true
    fi
    info "Installed systemd service → $SYSTEMD_DIR (not enabled)"
fi

# ── Done ──────────────────────────────────────────────────────────────────────
# Human-facing summary to stderr too — stdout stays reserved for machine output
# (there is none), so an output-parsing caller never sees install chatter.

{
    echo
    echo -e "${GRN}Installation complete.${NC}"
    echo
    echo "  Launch the app:   $(absolute_gui_bin)"
    echo "  Or find it in your application launcher: Scuffed Stat Tracker"
    echo "  If the launcher is stale: update-desktop-database $DESKTOP_DIR"
    echo
    echo "  The GUI Settings page has Start / Stop and Start on login."
    echo "  Autostart (systemd) starts the daemon automatically on login."
    echo
    echo "  First run: open the GUI, go to Settings, paste your server URL"
    echo "  and daemon token (from the web UI under My Stats → Daemon Tokens)."
    echo
} >&2
