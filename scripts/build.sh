#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> Building Dioxus app (crates/app)"
# Dioxus.toml lives at monorepo root (title, assets config). Build from there
# with app path so the shell title/meta are not the CLI defaults.
cd "$ROOT/crates/app"
dx build --release

echo "==> Staging app bundle into dist/"
cd "$ROOT"
rm -rf dist
cp -r target/dx/scuffed-app/release/web/public dist

# index.html links this stable path. dx copies it because the app references
# the unhashed asset. Do not paper over a missing file; the image would 404.
if [[ ! -f dist/assets/favicon.svg ]]; then
  echo "error: dist/assets/favicon.svg missing after dx build" >&2
  exit 1
fi

# Safety: never ship the default Dioxus CLI title
if grep -q 'dioxus |' dist/index.html 2>/dev/null || grep -q 'Dioxus | An elegant' dist/index.html 2>/dev/null; then
  echo "WARN: dist/index.html still has default Dioxus title — check Dioxus.toml is present at repo root"
fi

echo "==> Building server (scuffed-server)"
cargo build --release -p scuffed-server

echo "==> Done"
echo "    dist/index.html — Dioxus app"
echo "    target/release/scuffed-server"
