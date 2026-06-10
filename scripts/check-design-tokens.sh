#!/usr/bin/env bash
# CI guardrail: component CSS must use semantic tokens, never raw colors.
# The ONLY files allowed to contain raw hex / rgb literals are the token module
# (theme/tokens.rs) and the brand seam (theme/brand.rs).
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_SRC="$REPO_ROOT/crates/app/src"

# Allowed: the token source and the brand seam.
ALLOW_REGEX='theme/tokens.rs|theme/brand.rs'

violations=$(grep -rnE '#[0-9a-fA-F]{3,8}\b|rgba?\(' "$APP_SRC" \
  --include='*.rs' | grep -vE "$ALLOW_REGEX" || true)

if [[ -n "$violations" ]]; then
  echo "ERROR: raw color literals found outside the token module:"
  echo "$violations"
  echo ""
  echo "Use semantic CSS variables (var(--accent), var(--text-2), ...) instead."
  exit 1
fi
echo "OK: no raw color literals in component CSS."
