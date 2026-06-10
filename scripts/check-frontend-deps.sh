#!/usr/bin/env bash
# CI guardrail: block new leptos dependencies outside approved crates.
# Approved exceptions:
#   crates/auth — dormant optional `client` feature from the Leptos era
#
# To add a new exception, append the crate path to ALLOWED_CRATES below
# and document the reason in a PR comment with CTO approval.

set -euo pipefail

REPO_ROOT="$(cd "$(dirname "$0")/.." && pwd)"

ALLOWED_CRATES=(
  "crates/auth"
)

violations=()

while IFS= read -r cargo_toml; do
  rel_path="${cargo_toml#"$REPO_ROOT/"}"
  crate_dir="$(dirname "$rel_path")"

  # Skip allowed legacy crates
  allowed=false
  for allowed_crate in "${ALLOWED_CRATES[@]}"; do
    if [[ "$crate_dir" == "$allowed_crate" ]]; then
      allowed=true
      break
    fi
  done
  if $allowed; then
    continue
  fi

  # Check for leptos dependency (direct, not in comments)
  if grep -qE '^\s*(leptos|leptos_router|leptos_meta)\s*=' "$cargo_toml"; then
    violations+=("$rel_path")
  fi
done < <(find "$REPO_ROOT/crates" -name "Cargo.toml" -not -path "*/target/*")

if [[ ${#violations[@]} -gt 0 ]]; then
  echo "ERROR: Non-standard frontend dependency (leptos) found in non-exempted crates:"
  for v in "${violations[@]}"; do
    echo "  - $v"
  done
  echo ""
  echo "The standard frontend framework is Dioxus (workspace dep)."
  echo "If an exception is needed, add the crate to ALLOWED_CRATES in this script"
  echo "and get CTO approval in the PR."
  exit 1
fi

echo "OK: No non-exempted leptos dependencies found."
