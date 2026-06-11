#!/usr/bin/env bash
# CI guardrail: component CSS must use the design-system semantic tokens.
# Two checks:
#   1. No raw hex / rgb() color literals (use var(--accent), var(--text-2), ...).
#   2. No legacy/undefined CSS vars or legacy font literals (they predate the
#      token system; the file that defined them was removed, so they render wrong).
# The ONLY files allowed to contain raw color literals are the token module
# (theme/tokens.rs) and the brand seam (theme/brand.rs).
set -euo pipefail
REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"
APP_SRC="$REPO_ROOT/crates/app/src"

# Allowed to hold raw color literals: the token source and the brand seam.
ALLOW_REGEX='theme/tokens.rs|theme/brand.rs'

status=0

# --- Check 1: raw color literals ---
raw=$(grep -rnE '#[0-9a-fA-F]{3,8}\b|rgba?\(' "$APP_SRC" \
  --include='*.rs' | grep -vE "$ALLOW_REGEX" || true)
if [[ -n "$raw" ]]; then
  echo "ERROR: raw color literals found outside the token module:"
  echo "$raw"
  echo ""
  echo "Use semantic CSS variables (var(--accent), var(--text-2), ...) instead."
  echo "Canvas 2D colors that must be concrete strings belong in theme/tokens.rs as pub consts."
  status=1
fi

# --- Check 2: legacy / undefined CSS vars and legacy font literals ---
legacy=$(grep -rnE \
  '(^|[^a-z-])(--bg-void|--bg-surface|--bg-card|--bg-elevated|--bg-card-alt|--text-bright|--text-primary|--text-secondary|--text-muted|--border-light|--accent-bright|--accent-glow|--success|--warning|--font-display|--font-heading)\b|Bebas Neue|Rajdhani|Source Sans|DM Mono' \
  "$APP_SRC" --include='*.rs' || true)
if [[ -n "$legacy" ]]; then
  echo "ERROR: legacy/undefined design tokens found (the old theme module is gone):"
  echo "$legacy"
  echo ""
  echo "Map to current tokens: --bg-card->--surface, --text-bright->--text,"
  echo "--text-secondary->--text-2, --text-muted->--text-3, fonts->var(--font-head|body|mono), etc."
  status=1
fi

if [[ "$status" -eq 0 ]]; then
  echo "OK: component CSS uses design-system tokens (no raw colors, no legacy vars)."
fi
exit "$status"
