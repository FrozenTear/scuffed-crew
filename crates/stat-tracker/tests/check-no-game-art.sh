#!/usr/bin/env bash
# Fail if hero-portrait images are back under portraits/, or if test-data
# has an image that is not listed in test-data/image-allowlist.txt.
set -euo pipefail

root="$(cd "$(dirname "$0")/.." && pwd)"
allow="$root/test-data/image-allowlist.txt"
fail=0

if [[ ! -f "$allow" ]]; then
  echo "missing allowlist: $allow" >&2
  exit 1
fi

shopt -s nullglob
for f in "$root"/portraits/*.{png,jpg,jpeg,webp,gif,bmp,PNG,JPG,JPEG,WEBP}; do
  echo "image not allowed under portraits/: ${f#"$root"/}" >&2
  fail=1
done
if [[ -d "$root/portraits" ]]; then
  while IFS= read -r -d '' f; do
    echo "image not allowed under portraits/: ${f#"$root"/}" >&2
    fail=1
  done < <(find "$root/portraits" -type f \( \
    -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o \
    -iname '*.webp' -o -iname '*.gif' -o -iname '*.bmp' \
  \) -print0)
fi

mapfile -t listed < <(grep -vE '^[[:space:]]*(#|$)' "$allow" | sed 's|[[:space:]]*$||' | sort -u)
declare -A ok=()
for rel in "${listed[@]}"; do
  if [[ ! -f "$root/test-data/$rel" ]]; then
    echo "allowlist entry is missing: test-data/$rel" >&2
    fail=1
  fi
  ok["$rel"]=1
done

while IFS= read -r -d '' f; do
  rel="${f#"$root"/test-data/}"
  if [[ -z "${ok[$rel]+x}" ]]; then
    echo "test-data image is not on the allowlist: $rel" >&2
    fail=1
  fi
done < <(find "$root/test-data" -type f \( \
  -iname '*.png' -o -iname '*.jpg' -o -iname '*.jpeg' -o \
  -iname '*.webp' -o -iname '*.gif' -o -iname '*.bmp' \
\) -print0)

if [[ "$fail" -ne 0 ]]; then
  exit 1
fi
echo "stat-tracker image check ok (${#listed[@]} allowlisted)"
