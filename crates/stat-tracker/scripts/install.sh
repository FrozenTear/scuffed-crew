#!/usr/bin/env bash
# Build and (re)install the stat tracker: daemon + Iced GUI binaries into
# $PREFIX/bin (default ~/.local/bin) and the user systemd unit into
# ~/.config/systemd/user, then restart the service if it is running.
# Run from anywhere.
#
# A stale installed binary is invisible — the 2026-07-14 session-merge ran
# for days on a build that predated committed fixes. Re-run this after
# every stat-tracker change you want live.
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$CRATE_DIR/../.." && pwd)"
# shellcheck source=../dist/systemd-unit.sh
source "$CRATE_DIR/dist/systemd-unit.sh"
PREFIX="${PREFIX:-$HOME/.local}"
BIN_DIR="${BIN_DIR:-$PREFIX/bin}"
LIB_DIR="${LIB_DIR:-$PREFIX/lib}"
UNIT_DIR="${HOME}/.config/systemd/user"
UNIT="scuffed-stat-tracker.service"

echo "==> building release binaries (daemon + iced GUI)"
cargo build --release \
    -p scuffed-stat-tracker --bin scuffed-stat-tracker \
    -p scuffed-stat-tracker-ui --bin stat-tracker-gui \
    --manifest-path "$REPO_ROOT/Cargo.toml"

echo "==> installing binaries to $BIN_DIR"
install -Dm755 "$REPO_ROOT/target/release/scuffed-stat-tracker" "$BIN_DIR/scuffed-stat-tracker"
install -Dm755 "$REPO_ROOT/target/release/stat-tracker-gui" "$BIN_DIR/stat-tracker-gui"

echo "==> installing systemd user unit (ExecStart → $BIN_DIR/scuffed-stat-tracker)"
DAEMON_EXEC="$(absolute_install_path "$BIN_DIR/scuffed-stat-tracker")"
HELPER_DEST="$(absolute_install_path "$LIB_DIR/scuffed-stat-tracker/import-session-env.sh")"
install_user_units "$CRATE_DIR/assets" "$UNIT_DIR" "$DAEMON_EXEC" \
    "$CRATE_DIR/dist/import-session-env.sh" "$HELPER_DEST"
SYSTEMCTL_BIN="${SCUFFED_SYSTEMCTL:-systemctl}"
"$SYSTEMCTL_BIN" --user daemon-reload
# Import display vars for this login and write session.env. The oneshot
# repeats this before every start; a failure here must not hide the install.
"$HELPER_DEST" || echo "warning: session environment was not imported"

if "$SYSTEMCTL_BIN" --user is-active --quiet "$UNIT"; then
    echo "==> restarting running daemon"
    "$SYSTEMCTL_BIN" --user restart "$UNIT"
else
    echo "==> daemon not running — start it with: systemctl --user start $UNIT"
fi

echo "==> installed:"
"$SYSTEMCTL_BIN" --user show "$UNIT" -p Nice -p CPUWeight -p IOSchedulingClass 2>/dev/null || true
"$SYSTEMCTL_BIN" --user status "$UNIT" --no-pager | head -4 || true
