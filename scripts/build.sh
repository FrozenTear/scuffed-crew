#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> Building Dioxus app (crates/app)"
cd "$ROOT/crates/app"
dx build --release

echo "==> Staging app bundle into dist/"
cd "$ROOT"
rm -rf dist
cp -r target/dx/scuffed-app/release/web/public dist

echo "==> Building server (scuffed-server)"
cargo build --release -p scuffed-server

echo "==> Done"
echo "    dist/index.html — Dioxus app"
echo "    target/release/scuffed-server"
