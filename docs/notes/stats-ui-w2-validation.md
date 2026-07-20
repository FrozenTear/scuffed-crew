# Stats UI W2 — palette validation record (plan items 5–7)

Validator: `dataviz/scripts/validate_palette.js` (bundled skill, Machado 2009
CVD sim, OKLab ΔE×100). Surfaces used: dark `--surface #1f1f27`, light
`--surface #ffffff` (the hbar track / card surface the marks actually sit on).
All outputs below are verbatim.

## Summary of token changes (`crates/app/src/theme/tokens.rs`)

| Token | Dark before | Dark after | Light before | Light after |
|---|---|---|---|---|
| `--chart-1` | `#8f73ff` | unchanged | `#6d4aff` | unchanged |
| `--chart-2` | `#46d8a4` | `#15ac7d` | `#0ea66e` | unchanged |
| `--chart-3` | `#fbbf24` | `#b98a02` | `#c2830a` | unchanged |
| `--chart-4` | `#f06a6a` | `#ca474c` | `#d63031` | unchanged |
| `--chart-5` | `#38bdf8` | `#089fd7` | `#0284c7` | unchanged |
| `--chart-6` | `#c084fc` | `#984ab2` | `#9333ea` | `#7405c3` |
| `--chart-wr` (new) | — | `#089fd7` | — | `#0284c7` |
| `--ok` / `--warn` / `--danger` | unchanged | unchanged | unchanged | unchanged |

Snapping method: convert to OKLCH, hold hue (±0–12° only where CVD required
it), lower L into the dark band (0.48–0.67), gamut-clip chroma, iterate against
the validator. Dark `--chart-6` also moved hue 305°→317° (toward magenta) and
dropped to L 0.55 to fix the deutan ΔE 3.7 FAIL against `--chart-5`. Light
`--chart-6` darkened (L 0.558→0.46) to lift the 7.8 WARN-band deutan pair
against `--chart-5` above the 8.0 target.

`--chart-wr` is the new single winrate-bar accent (aliases the chart-5 blue
values); bars never use status or mode colors again.

## BEFORE

```
### BEFORE dark chart-1..6 vs #1f1f27

Palette (dark, surface #1f1f27, categorical): 6 slots
  [FAIL] Lightness band         outside band: [["#46d8a4",0.792],["#fbbf24",0.837],["#f06a6a",0.688],["#38bdf8",0.754],["#c084fc",0.722]]
  [PASS] Chroma floor           all 6 >= 0.1
  [FAIL] CVD separation         worst adjacent #c084fc↔#38bdf8 ΔE 3.7 (deutan) · tritan 17.6
  [PASS] Normal-vision floor    worst adjacent #c084fc↔#38bdf8 ΔE 19.3 (normal)
  [PASS] Contrast vs surface    all 6 >= 3:1

  → FAILED — fix the marked checks  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)

### BEFORE light chart-1..6 vs #ffffff

Palette (light, surface #ffffff, categorical): 6 slots
  [PASS] Lightness band         all 6 inside L 0.43–0.77
  [PASS] Chroma floor           all 6 >= 0.1
  [WARN] CVD separation         worst adjacent #9333ea↔#0284c7 ΔE 7.8 (deutan) · tritan 13.2
  [PASS] Normal-vision floor    worst adjacent #d63031↔#c2830a ΔE 17.3 (normal)
  [PASS] Contrast vs surface    all 6 >= 3:1

  → ALL CHECKS PASS  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)

### BEFORE dark status ok,warn,danger vs #1f1f27

Palette (dark, surface #1f1f27, categorical): 3 slots
  [FAIL] Lightness band         outside band: [["#46d8a4",0.792],["#fbbf24",0.837],["#f06a6a",0.688]]
  [PASS] Chroma floor           all 3 >= 0.1
  [PASS] CVD separation         worst adjacent #fbbf24↔#46d8a4 ΔE 11.7 (protan) · tritan 17.6
  [PASS] Normal-vision floor    worst adjacent #fbbf24↔#46d8a4 ΔE 20.6 (normal)
  [PASS] Contrast vs surface    all 3 >= 3:1

  → FAILED — fix the marked checks  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)

### BEFORE light status ok,warn,danger vs #ffffff

Palette (light, surface #ffffff, categorical): 3 slots
  [PASS] Lightness band         all 3 inside L 0.43–0.77
  [PASS] Chroma floor           all 3 >= 0.1
  [PASS] CVD separation         worst adjacent #d63031↔#c2830a ΔE 8.3 (deutan) · tritan 13.5
  [PASS] Normal-vision floor    worst adjacent #d63031↔#c2830a ΔE 17.3 (normal)
  [PASS] Contrast vs surface    all 3 >= 3:1

  → ALL CHECKS PASS  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)
```

## AFTER

```
### AFTER dark chart-1..6 vs #1f1f27

Palette (dark, surface #1f1f27, categorical): 6 slots
  [PASS] Lightness band         all 6 inside L 0.48–0.67
  [PASS] Chroma floor           all 6 >= 0.1
  [PASS] CVD separation         worst adjacent #ca474c↔#b98a02 ΔE 9.8 (deutan) · tritan 10.3
  [PASS] Normal-vision floor    worst adjacent #b98a02↔#15ac7d ΔE 17.5 (normal)
  [PASS] Contrast vs surface    all 6 >= 3:1

  → ALL CHECKS PASS  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)

### AFTER light chart-1..6 vs #ffffff

Palette (light, surface #ffffff, categorical): 6 slots
  [PASS] Lightness band         all 6 inside L 0.43–0.77
  [PASS] Chroma floor           all 6 >= 0.1
  [PASS] CVD separation         worst adjacent #d63031↔#c2830a ΔE 8.3 (deutan) · tritan 13.2
  [PASS] Normal-vision floor    worst adjacent #d63031↔#c2830a ΔE 17.3 (normal)
  [PASS] Contrast vs surface    all 6 >= 3:1

  → ALL CHECKS PASS  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)
```

Both palettes now pass every check with worst adjacent CVD ΔE ≥ 8.0 (no
WARN-band pairs, so no mandatory-secondary-encoding debt).

## Status tokens (`--ok` / `--warn` / `--danger`): kept unchanged — rationale

- The only FAIL on status tokens was the dark **lightness band** — a criterion
  for *chart marks* glowing on dark surfaces. After W2, status colors are never
  chart marks: bars use `--chart-wr`, so status tokens survive only as
  status **text/chips** (outcome chips, badges, toasts). The validator's own
  scope note says lone status/text colors are governed by WCAG text contrast,
  not the categorical band.
- Snapping them into the 0.48–0.67 band would *reduce* their WCAG text
  contrast on dark surfaces — trading a real accessibility property for an
  inapplicable one.
- The checks that do apply, pass: CVD separation (dark worst pair 11.7, light
  8.3) and contrast vs surface (all ≥ 3:1), both themes. Status chips always
  carry a text label (VICTORY / DEFEAT / DRAW), never color alone.

## WR-text decision (plan item 5, open sub-item)

**Chosen: plain text tokens** (`--text`, muted `--text-3` under the min-games
gate) — not the two-state polarity. Reasons:

1. Polarity is already encoded once, geometrically: every winrate bar is drawn
   on an absolute 0–100 scale against the 50% reference hairline. A colored
   text state would re-introduce a second color system for the same fact —
   exactly the collision item 5 removes.
2. The dataviz rule "text wears text tokens, never the series/status color"
   survives review with zero validator debt; a good/serious text pair would
   put small colored text back in every table row and re-open contrast
   obligations per theme.
3. The remaining text distinction is sample-size honesty (n < 3 muted), which
   carries real information instead of restating the number's own sign.

Implementation: `wr_text_class(matches)` in `pages/stats/mod.rs`
(`.stats-wr` / `.stats-wr.muted`), `MIN_GAMES = 3`, caption
`min 3 games — smaller samples muted` on the heroes table and maps tab.

## Out-of-scope notes for the reviewer

- `pages/stats_member.rs` (public member stats) still has its own 3-bin
  `winrate_class` + CSS copy — untouched here; W4 handles public-page parity
  per the plan.
- The W1 hero tile (`.stat-tile-hero .stat-tile-value`) uses `--accent`;
  brand-purple usage was NOT expanded in W2 but this W1 choice was left as-is
  (flag if it should count against "active tab + focus only").
- Strategy pages use `--chart-5` as the Tank role color; the dark chart-5 snap
  (`#38bdf8`→`#089fd7`) shifts them slightly. Same hue, validator-passing.
