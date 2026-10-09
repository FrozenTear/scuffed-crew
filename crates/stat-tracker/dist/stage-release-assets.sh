#!/usr/bin/env bash
# Copy the assets install.sh actually installs into a release stage directory.
#
# Digit templates under assets/shadow-digits are compiled into the daemon
# with include_bytes! (src/shadow/digits.rs) and decoded from that byte
# slice. install.sh never copies them, and nothing at runtime opens those
# PNGs from disk. They stay out of the tarball.
#
# Dry run:
#   bash crates/stat-tracker/dist/stage-release-assets.sh /tmp/stage-assets
#   find /tmp/stage-assets -print
set -euo pipefail

DEST="${1:?usage: stage-release-assets.sh DEST}"
ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
SRC="$ROOT/assets"

mkdir -p "$DEST"
cp \
  "$SRC/scuffed-stat-tracker.desktop" \
  "$SRC/scuffed-stat-tracker.service" \
  "$SRC/scuffed-stat-tracker-session.service" \
  "$DEST/"

if find "$DEST" \( -name '*.png' -o -name 'shadow-digits' \) | grep -q .; then
  echo "refusing to pack shadow digit templates into the release tarball" >&2
  find "$DEST" \( -name '*.png' -o -name 'shadow-digits' \) >&2
  exit 1
fi
