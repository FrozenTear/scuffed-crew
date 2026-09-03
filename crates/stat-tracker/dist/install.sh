#!/usr/bin/env bash
# In-tarball installer for prebuilt Linux releases (no Rust/cargo).
#
# Expected layout (release asset root after extract):
#   bin/scuffed-stat-tracker
#   bin/stat-tracker-gui   (Iced UI from scuffed-stat-tracker-ui)
#   lib/scuffed-stat-tracker/ocr/*  (daemon OCR closure; RPATH …/ocr)
#   lib/scuffed-stat-tracker/gui/*  (GUI libxdo; RPATH …/gui)
#   (v0.4.0 used a flat lib/* on $ORIGIN/../lib — that shadowed system OpenSSL)
#   tessdata/eng.traineddata        (optional — runtime OCR model, since v0.3.0)
#   tessdata/koverwatch.traineddata (optional — CI-trained game-font model, since v0.3.0)
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
# install (default, unchanged). bootstrap.sh passes this through.
SKIP_INTEGRATION="${SKIP_INTEGRATION:-}"

# Every file this script installs is recorded here and written to
# $PREFIX/share/scuffed-stat-tracker/install-manifest.txt so uninstall.sh can
# remove exactly what we put on disk (the bundled libs land in a shared
# $PREFIX/lib/scuffed-stat-tracker/{ocr,gui} and are unidentifiable without it).
MANIFEST_ENTRIES=()
# Paths removed during a v0.4.0 → 0.4.1 upgrade (flat $PREFIX/lib leftovers).
REMOVED_ENTRIES=()

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
MANIFEST_ENTRIES+=("$BIN_DIR/scuffed-stat-tracker" "$BIN_DIR/stat-tracker-gui")
info "Installed binaries → $BIN_DIR"

if [[ -f "$PKG_ROOT/uninstall.sh" ]]; then
    install -m755 "$PKG_ROOT/uninstall.sh" "$BIN_DIR/scuffed-stat-tracker-uninstall"
    MANIFEST_ENTRIES+=("$BIN_DIR/scuffed-stat-tracker-uninstall")
    info "Installed uninstaller → $BIN_DIR/scuffed-stat-tracker-uninstall"
fi

# Bundled native libs (portable releases). Never dump into $PREFIX/lib itself:
# v0.4.0 put Ubuntu 22.04 libcrypto.so.3 there, and both binaries' RUNPATH
# ($ORIGIN/../lib) made that copy win over /usr/lib — OPENSSL_3.2.0 not found
# when the GUI loaded system libcryptsetup.
#
# v0.4.1+ layout (matches the stamped RUNPATHs):
#   $PREFIX/lib/scuffed-stat-tracker/ocr  ← daemon
#   $PREFIX/lib/scuffed-stat-tracker/gui  ← GUI (libxdo only)
is_openssl_lib() {
    local base="$1"
    [[ "$base" == libcrypto.so* || "$base" == libssl.so* ]]
}

install_bundled_tree() {
    local src="$1" dest="$2"
    local count=0
    INSTALLED_LIB_COUNT=0
    [[ -d "$src" ]] || return 0
    local f rel base dest_file
    while IFS= read -r -d '' f; do
        rel="${f#"$src"/}"
        base="$(basename "$f")"
        if is_openssl_lib "$base"; then
            warn "skipping bundled OpenSSL $rel (host libcrypto/libssl must win)"
            continue
        fi
        dest_file="$dest/$rel"
        mkdir -p "$(dirname "$dest_file")"
        install -m755 "$f" "$dest_file"
        MANIFEST_ENTRIES+=("$dest_file")
        count=$((count + 1))
    done < <(find "$src" -type f -print0)
    INSTALLED_LIB_COUNT=$count
}

BUNDLE_SRC="$PKG_ROOT/lib/scuffed-stat-tracker"
BUNDLE_DEST="$LIB_DIR/scuffed-stat-tracker"
if [[ -d "$BUNDLE_SRC" ]]; then
    install_bundled_tree "$BUNDLE_SRC" "$BUNDLE_DEST"
    info "Installed $INSTALLED_LIB_COUNT bundled libs → $BUNDLE_DEST"
    info "  daemon RUNPATH \$ORIGIN/../lib/scuffed-stat-tracker/ocr"
    info "  GUI    RUNPATH \$ORIGIN/../lib/scuffed-stat-tracker/gui"
elif [[ -d "$PKG_ROOT/lib" ]] && compgen -G "$PKG_ROOT/lib/*" >/dev/null; then
    # Older tarball with a flat lib/. Isolate into the private OCR dir and
    # skip OpenSSL so a mixed upgrade cannot re-shadow system libcrypto.
    warn "flat lib/ layout (pre-0.4.1 tarball) — installing into $BUNDLE_DEST/ocr, skipping OpenSSL"
    mkdir -p "$BUNDLE_DEST/ocr"
    count=0
    for f in "$PKG_ROOT/lib"/*; do
        [[ -f "$f" ]] || continue
        base="$(basename "$f")"
        if is_openssl_lib "$base"; then
            warn "skipping bundled OpenSSL $base (host libcrypto/libssl must win)"
            continue
        fi
        install -m755 "$f" "$BUNDLE_DEST/ocr/$base"
        MANIFEST_ENTRIES+=("$BUNDLE_DEST/ocr/$base")
        count=$((count + 1))
    done
    info "Installed $count bundled libs → $BUNDLE_DEST/ocr"
fi

# Upgrade cleanup: v0.4.0 wrote sonames into $PREFIX/lib (the GUI RUNPATH).
# Delete those leftover files if our previous manifest listed them, especially
# libcrypto/libssl which break hosts with a newer system OpenSSL.
MANIFEST_DIR="$PREFIX/share/scuffed-stat-tracker"
MANIFEST="$MANIFEST_DIR/install-manifest.txt"
if [[ -f "$MANIFEST" ]]; then
    while IFS= read -r line; do
        [[ "$line" == /* ]] || continue
        rel="${line#"$LIB_DIR"/}"
        # Only flat files we previously dumped into $PREFIX/lib — not the
        # new private tree ($LIB_DIR/scuffed-stat-tracker/…).
        if [[ "$rel" == "$line" || "$rel" == */* ]]; then
            continue
        fi
        if [[ -f "$line" || -L "$line" ]]; then
            rm -f "$line"
            REMOVED_ENTRIES+=("$line")
        fi
    done < "$MANIFEST"
    if [[ ${#REMOVED_ENTRIES[@]} -gt 0 ]]; then
        info "Removed ${#REMOVED_ENTRIES[@]} leftover v0.4.0 lib(s) from $LIB_DIR"
    fi
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

# Bundled game-font model (koverwatch). Unlike eng, the release bundle is the
# canonical source for this file — most machines cannot regenerate it locally
# (text2image hangs/segfaults on pango >= 1.56) — so a differing existing copy
# is replaced, with a .bak kept for anyone who genuinely self-trained one.
BUNDLED_KOV="$PKG_ROOT/tessdata/koverwatch.traineddata"
USER_KOV="$USER_TESSDATA_DIR/koverwatch.traineddata"
if [[ -f "$BUNDLED_KOV" ]]; then
    if [[ -f "$USER_KOV" ]] && cmp -s "$BUNDLED_KOV" "$USER_KOV"; then
        info "koverwatch.traineddata already up to date at $USER_KOV"
    else
        mkdir -p "$USER_TESSDATA_DIR"
        if [[ -f "$USER_KOV" ]]; then
            cp "$USER_KOV" "$USER_KOV.bak"
            info "Existing koverwatch.traineddata backed up → $USER_KOV.bak"
        fi
        install -m644 "$BUNDLED_KOV" "$USER_KOV"
        info "Installed bundled koverwatch.traineddata → $USER_KOV"
    fi
fi

if [[ ":$PATH:" != *":$BIN_DIR:"* ]]; then
    warn "$BIN_DIR is not in your shell PATH."
    warn "Add this to your shell config (~/.bashrc, ~/.zshrc, etc.):"
    warn "    export PATH=\"$BIN_DIR:\$PATH\""
    warn "App launchers (AerynOS / Cosmic / GNOME) often omit this dir even"
    warn "when a terminal has it — the .desktop Exec is an absolute path so"
    warn "the menu does not depend on session PATH."
fi

if [[ -n "$SKIP_INTEGRATION" ]]; then
    info "SKIP_INTEGRATION set — skipping desktop entry and systemd unit (binaries only)"
else
    # ── Desktop entry ─────────────────────────────────────────────────────────
    write_desktop_entry "$ASSETS_DIR/$DESKTOP" "$DESKTOP_DIR/$DESKTOP" "$(absolute_gui_bin)"
    MANIFEST_ENTRIES+=("$DESKTOP_DIR/$DESKTOP")
    refresh_desktop_database "$DESKTOP_DIR"
    info "Installed desktop entry → $DESKTOP_DIR (Exec=$(absolute_gui_bin))"

    # ── systemd user service (installed, NOT enabled) ─────────────────────────
    mkdir -p "$SYSTEMD_DIR"
    install -m644 "$ASSETS_DIR/$UNIT" "$SYSTEMD_DIR/$UNIT"
    MANIFEST_ENTRIES+=("$SYSTEMD_DIR/$UNIT")
    if command -v systemctl &>/dev/null; then
        systemctl --user daemon-reload 2>/dev/null || true
    fi
    info "Installed systemd service → $SYSTEMD_DIR (not enabled)"
fi

# ── Install manifest ──────────────────────────────────────────────────────────
# Union with any previous manifest so an upgrade that drops a file still
# leaves the old copy removable by the uninstaller.

MANIFEST_DIR="$PREFIX/share/scuffed-stat-tracker"
MANIFEST="$MANIFEST_DIR/install-manifest.txt"
mkdir -p "$MANIFEST_DIR"
{
    if [[ -f "$MANIFEST" ]]; then
        if [[ ${#REMOVED_ENTRIES[@]} -gt 0 ]]; then
            grep -Fvx -f <(printf '%s\n' "${REMOVED_ENTRIES[@]}") "$MANIFEST" || true
        else
            cat "$MANIFEST"
        fi
    fi
    printf '%s\n' "${MANIFEST_ENTRIES[@]}"
} | sort -u > "$MANIFEST.tmp"
mv "$MANIFEST.tmp" "$MANIFEST"
info "Wrote install manifest → $MANIFEST"

# ── Smoke check ───────────────────────────────────────────────────────────────

if ! "$BIN_DIR/scuffed-stat-tracker" --version >/dev/null 2>&1; then
    warn "daemon --version failed. Missing host libs (display stack/evdev) or bundled OCR libs not at \$ORIGIN/../lib/scuffed-stat-tracker/ocr?"
else
    info "daemon binary runs ($("$BIN_DIR/scuffed-stat-tracker" --version))"
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
    echo "  Uninstall:        scuffed-stat-tracker-uninstall   (--purge removes data/config too)"
    echo
    echo "  Source rebuilds (dev): crates/stat-tracker/install.sh (requires cargo)."
    echo
} >&2
