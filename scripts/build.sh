#!/usr/bin/env bash
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"

echo "==> Building site WASM (crates/site)"
cd "$ROOT/crates/site"
trunk build --release

echo "==> Building admin SPA (crates/admin)"
cd "$ROOT/crates/admin"
trunk build --release

echo "==> Building server (scuffed-site-server)"
cd "$ROOT"
cargo build --release -p scuffed-site-server

echo "==> Done"
echo "    dist/index.html       — public site"
echo "    dist/admin/index.html — admin SPA"
echo "    target/release/scuffed-site-server"
