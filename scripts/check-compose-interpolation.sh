#!/usr/bin/env bash
# Reject nested ${...${...}} in Compose files.
# podman-compose 1.5 renders a nested default as the outer value plus a
# stray "}" when the outer name is set, and as literal text when it is not.
set -euo pipefail

ROOT="$(cd "$(dirname "$0")/.." && pwd)"
nested_re='\$\{[^}]*\$\{'

if ! printf '%s\n' '${A:-${B}}' | grep -qE "$nested_re"; then
    echo "error: nested-default detector missed a nested substitution" >&2
    exit 1
fi
if printf '%s\n' '${A:-plain}' | grep -qE "$nested_re"; then
    echo "error: nested-default detector flagged a single-level substitution" >&2
    exit 1
fi

status=0
while IFS= read -r -d '' f; do
    hits="$(grep -nE "$nested_re" "$f" || true)"
    if [[ -n "$hits" ]]; then
        echo "ERROR: nested compose interpolation in ${f#"$ROOT"/}:" >&2
        printf '%s\n' "$hits" >&2
        echo "Use a single-level \${VAR:-default}, or export the fallback from scripts/lib/compose-env.sh before compose runs." >&2
        status=1
    fi
done < <(find "$ROOT" \
    \( -path "$ROOT/.git" -o -path "$ROOT/.claude" \) -prune -o \
    -type f \( \
        -name 'compose.yml' -o -name 'compose.yaml' -o \
        -name 'compose.*.yml' -o -name 'compose.*.yaml' -o \
        -name 'docker-compose.yml' -o -name 'docker-compose.yaml' -o \
        -name 'docker-compose.*.yml' -o -name 'docker-compose.*.yaml' -o \
        -name 'podman-compose.yml' -o -name 'podman-compose.yaml' \
    \) -print0)

exit "$status"
