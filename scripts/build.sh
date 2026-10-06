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

# index.html links this stable path (`/assets/favicon.svg`). The server treats
# the unhashed file as a one-day asset, not an immutable hashed one. Keep the
# copy so that link does not fall through to the SPA shell.
if [[ ! -f dist/assets/favicon.svg ]] && [[ -f crates/app/assets/favicon.svg ]]; then
  mkdir -p dist/assets
  cp crates/app/assets/favicon.svg dist/assets/favicon.svg
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
