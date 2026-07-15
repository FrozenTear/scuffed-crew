#!/usr/bin/env bash
# Build and (re)install the stat tracker: daemon + GUI binaries into
# ~/.local/bin, the user systemd unit into ~/.config/systemd/user, then
# restart the service if it is running. Run from anywhere.
#
# A stale installed binary is invisible — the 2026-07-14 session-merge ran
# for days on a build that predated committed fixes. Re-run this after
# every stat-tracker change you want live.
set -euo pipefail

CRATE_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
REPO_ROOT="$(cd "$CRATE_DIR/../.." && pwd)"
BIN_DIR="${HOME}/.local/bin"
UNIT_DIR="${HOME}/.config/systemd/user"
UNIT="scuffed-stat-tracker.service"

echo "==> building release binaries (daemon + gui)"
cargo build --release -p scuffed-stat-tracker --features gui \
    --manifest-path "$REPO_ROOT/Cargo.toml"

echo "==> installing binaries to $BIN_DIR"
install -Dm755 "$REPO_ROOT/target/release/scuffed-stat-tracker" "$BIN_DIR/scuffed-stat-tracker"
install -Dm755 "$REPO_ROOT/target/release/stat-tracker-gui" "$BIN_DIR/stat-tracker-gui"

echo "==> installing systemd user unit"
install -Dm644 "$CRATE_DIR/assets/$UNIT" "$UNIT_DIR/$UNIT"
systemctl --user daemon-reload

if systemctl --user is-active --quiet "$UNIT"; then
    echo "==> restarting running daemon"
    systemctl --user restart "$UNIT"
else
    echo "==> daemon not running — start it with: systemctl --user start $UNIT"
fi

echo "==> installed:"
systemctl --user show "$UNIT" -p Nice -p CPUWeight -p IOSchedulingClass 2>/dev/null || true
systemctl --user status "$UNIT" --no-pager | head -4 || true
