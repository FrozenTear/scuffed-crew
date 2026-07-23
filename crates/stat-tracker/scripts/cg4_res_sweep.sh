#!/usr/bin/env bash
# CG-4 multi-resolution merge gate (USER requirement, fleet plan 01KY6V153B).
#
# Runs the scoreboard cell-OCR pipeline over a pixel-ground-truthed fixture
# frame at 1440p native, 0.75x (1080p), 1.5x (4K) and 0.5x (720p) via the
# resprobe example, then asserts the known row-0 values survive at every
# BLOCKING scale (native / 0.75x / 1.5x — 0.5x is reported, non-blocking).
#
# Ground truth (pixel-verified 2026-07-23, King's Row 6v6, player row):
#   E 22, A 9, D 12, DMG 16,456, HLG 2,644, MIT 7,795
# The MIT value is the regression pin: the pre-CG-4 geometry decapitated it
# to "1795" (comma OCR'd as a leading 1) and, on drifted frames, annexed its
# lead digit into HLG (the 2026-07-22 Numbani HLG-22994 corruption).
#
# Fixtures are local-only (gitignored). Default frame:
#   crates/stat-tracker/test-data/cg4-20260723/accepted_20260723_004531.png
# Usage: crates/stat-tracker/scripts/cg4_res_sweep.sh [frame.png]

set -euo pipefail
cd "$(dirname "$0")/../../.."

FRAME=${1:-crates/stat-tracker/test-data/cg4-20260723/accepted_20260723_004531.png}
if [[ ! -f "$FRAME" ]]; then
    echo "SKIP: fixture $FRAME not present (local-only test data)" >&2
    exit 0
fi

OUT=$(cargo run -q -p scuffed-stat-tracker --example resprobe -- "$FRAME" 0.75 1.5 0.5 2>/dev/null \
    | grep -vE "Warning|Estimating" || true)
echo "$OUT"
echo

fail=0
for scale in native x0.75 x1.5; do
    row0=$(echo "$OUT" | awk -v s="--- $scale " 'index($0, s) {f=1} f && /row  0/ {print; exit}')
    if [[ -z "$row0" ]]; then
        echo "FAIL [$scale]: no row 0 output"
        fail=1
        continue
    fi
    for want in "22" "16,456" "2,644" "7,795"; do
        if ! grep -qF "$want" <<<"$row0"; then
            echo "FAIL [$scale]: row 0 missing '$want': $row0"
            fail=1
        fi
    done
done

row0_half=$(echo "$OUT" | awk 'index($0, "--- x0.5 ") {f=1} f && /row  0/ {print; exit}')
echo "INFO [x0.5, non-blocking]: $row0_half"

if [[ $fail -ne 0 ]]; then
    echo "CG-4 res sweep: FAIL"
    exit 1
fi
echo "CG-4 res sweep: PASS (native, 1080p, 4K)"
