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

## W5a — diverging winrate poles (2026-07-20)

W5a converts winrate bars from the single `--chart-wr` accent to a two-pole
diverging encoding. Same validator, same surfaces as W2 (dark `#1f1f27`,
light `#ffffff` — the hbar track / card surface). All outputs verbatim.

### Token changes (`crates/app/src/theme/tokens.rs`)

| Token | Dark | Light | Notes |
|---|---|---|---|
| `--chart-wr-up` (new) | `#089fd7` | `#0284c7` | cool "winning" pole = the W2 `--chart-wr` blue, unchanged |
| `--chart-wr-down` (new) | `#aa5000` | `#843900` | warm "losing" pole — desaturated burnt orange, a chart pole, NOT the status red |
| `--chart-wr` | removed | removed | superseded; its only two consumers (heroes + maps bars) moved to the poles |

Down-pole derivation: OKLCH sweep (dark L 0.535 / C 0.14 / H 52°; light
L 0.44 / C 0.12 / H 50°), iterated against the validator. The dark warm
corridor is tight: gold `--chart-3` (L 0.65) above, red `--chart-4` (L 0.55)
beside, and the 3:1 surface-contrast floor below — `#aa5000` is the point that
passes the full pair run AND clears the ≥6 floor against every chart token.

### Bar color rule (`wr_bar_color` in `pages/stats/mod.rs`)

- winrate > 50% → `var(--chart-wr-up)`; < 50% → `var(--chart-wr-down)`
- **exactly 50.0%** (`wins * 2 == matches`, integer-exact — no float compare)
  → **neutral midpoint `var(--text-3)`**. Decision: a diverging scale is two
  hues + a neutral midpoint; a dead-even record has no polarity, and painting
  it blue would claim a lead that isn't there. Grays are the one legitimate
  non-chart-token fill (the diverging midpoint is definitionally neutral).
- n < MIN_GAMES → neutral fill as well, on top of the existing muted
  treatment (0.45 fill opacity + muted value text). Muting overrides
  polarity: a 1-game 0% never gets a losing-red bar, even a faint one.
- The 50% reference hairline is unchanged; `HBarChart` needed **no** change —
  `BarEntry.color` was already per-entry.

### BEFORE (naive candidate: down-pole = status `--danger`)

```
### BEFORE dark pair (naive down-pole = status --danger #f06a6a) vs #1f1f27

Palette (dark, surface #1f1f27, categorical): 2 slots
  [FAIL] Lightness band         outside band: [["#f06a6a",0.688]]
  [PASS] Chroma floor           all 2 >= 0.1
  [PASS] CVD separation         worst adjacent #f06a6a↔#089fd7 ΔE 16.1 (protan) · tritan 32.3
  [PASS] Normal-vision floor    worst adjacent #f06a6a↔#089fd7 ΔE 29.2 (normal)
  [PASS] Contrast vs surface    all 2 >= 3:1

  → FAILED — fix the marked checks  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)

### BEFORE light pair (naive down-pole = status --danger #d63031) vs #ffffff

Palette (light, surface #ffffff, categorical): 2 slots
  [PASS] Lightness band         all 2 inside L 0.43–0.77
  [PASS] Chroma floor           all 2 >= 0.1
  [PASS] CVD separation         worst adjacent #d63031↔#0284c7 ΔE 23.2 (protan) · tritan 34.4
  [PASS] Normal-vision floor    worst adjacent #d63031↔#0284c7 ΔE 32.5 (normal)
  [PASS] Contrast vs surface    all 2 >= 3:1

  → ALL CHECKS PASS  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)
```

Reusing status red fails the dark lightness band outright (0.688), collides
with `--chart-4` light (identical hex `#d63031`), and would re-blur the
chart/status separation W2 established — hence a dedicated warm pole.

### AFTER (final pair)

```
### AFTER dark pair --chart-wr-up #089fd7 / --chart-wr-down #aa5000 vs #1f1f27

Palette (dark, surface #1f1f27, categorical): 2 slots
  [PASS] Lightness band         all 2 inside L 0.48–0.67
  [PASS] Chroma floor           all 2 >= 0.1
  [PASS] CVD separation         worst adjacent #aa5000↔#089fd7 ΔE 25.0 (deutan) · tritan 31.3
  [PASS] Normal-vision floor    worst adjacent #aa5000↔#089fd7 ΔE 30.2 (normal)
  [PASS] Contrast vs surface    all 2 >= 3:1

  → ALL CHECKS PASS  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)

### AFTER light pair --chart-wr-up #0284c7 / --chart-wr-down #843900 vs #ffffff

Palette (light, surface #ffffff, categorical): 2 slots
  [PASS] Lightness band         all 2 inside L 0.43–0.77
  [PASS] Chroma floor           all 2 >= 0.1
  [PASS] CVD separation         worst adjacent #843900↔#0284c7 ΔE 25.5 (deutan) · tritan 29.6
  [PASS] Normal-vision floor    worst adjacent #843900↔#0284c7 ΔE 29.6 (normal)
  [PASS] Contrast vs surface    all 2 >= 3:1

  → ALL CHECKS PASS  (CVD in the 6–8 floor band is legal ONLY with secondary encoding: direct labels, gaps, or texture)
```

Both pairs pass every check with worst adjacent CVD ΔE ≥ 25 (target ≥ 8).

### Pole vs `--chart-1..6` collision sweep (2-color runs, verbatim summary)

```
DARK (surface #1f1f27): charts = 1:#8f73ff 2:#15ac7d 3:#b98a02 4:#ca474c 5:#089fd7 6:#984ab2
  up(#089fd7) vs chart-1(#8f73ff): CVD ΔE 8.3 (deutan) · tritan 9.3 · normal ΔE 16.7
  up(#089fd7) vs chart-2(#15ac7d): CVD ΔE 14.4 (deutan) · tritan 2.8 · normal ΔE 15.2
  up(#089fd7) vs chart-3(#b98a02): CVD ΔE 23.3 (protan) · tritan 21.5 · normal ΔE 26.0
  up(#089fd7) vs chart-4(#ca474c): CVD ΔE 19.9 (deutan) · tritan 34.1 · normal ΔE 30.1
  up(#089fd7) vs chart-5(#089fd7): SAME COLOR (deliberate alias, see notes)
  up(#089fd7) vs chart-6(#984ab2): CVD ΔE 10.6 (deutan) · tritan 23.8 · normal ΔE 23.5
  down(#aa5000) vs chart-1(#8f73ff): CVD ΔE 31.4 (deutan) · tritan 23.5 · normal ΔE 32.0
  down(#aa5000) vs chart-2(#15ac7d): CVD ΔE 14.3 (deutan) · tritan 30.8 · normal ΔE 26.2
  down(#aa5000) vs chart-3(#b98a02): CVD ΔE 12.5 (deutan) · tritan 14.1 · normal ΔE 14.8
  down(#aa5000) vs chart-4(#ca474c): CVD ΔE 6.0 (deutan) · tritan 6.6 · normal ΔE 9.3
  down(#aa5000) vs chart-5(#089fd7): CVD ΔE 25.0 (deutan) · tritan 31.3 · normal ΔE 30.2
  down(#aa5000) vs chart-6(#984ab2): CVD ΔE 22.2 (deutan) · tritan 9.9 · normal ΔE 22.9

LIGHT (surface #ffffff): charts = 1:#6d4aff 2:#0ea66e 3:#c2830a 4:#d63031 5:#0284c7 6:#7405c3
  up(#0284c7) vs chart-1(#6d4aff): CVD ΔE 10.3 (deutan) · tritan 7.8 · normal ΔE 17.6
  up(#0284c7) vs chart-2(#0ea66e): CVD ΔE 18.2 (protan) · tritan 5.3 · normal ΔE 19.2
  up(#0284c7) vs chart-3(#c2830a): CVD ΔE 23.7 (protan) · tritan 23.3 · normal ΔE 28.3
  up(#0284c7) vs chart-4(#d63031): CVD ΔE 23.2 (protan) · tritan 34.4 · normal ΔE 32.5
  up(#0284c7) vs chart-5(#0284c7): SAME COLOR (deliberate alias, see notes)
  up(#0284c7) vs chart-6(#7405c3): CVD ΔE 12.7 (deutan) · tritan 21.3 · normal ΔE 24.5
  down(#843900) vs chart-1(#6d4aff): CVD ΔE 34.4 (deutan) · tritan 25.9 · normal ΔE 35.5
  down(#843900) vs chart-2(#0ea66e): CVD ΔE 19.9 (deutan) · tritan 32.7 · normal ΔE 29.6
  down(#843900) vs chart-3(#c2830a): CVD ΔE 22.3 (deutan) · tritan 22.0 · normal ΔE 22.7
  down(#843900) vs chart-4(#d63031): CVD ΔE 8.2 (protan) · tritan 18.0 · normal ΔE 17.0
  down(#843900) vs chart-5(#0284c7): CVD ΔE 25.5 (deutan) · tritan 29.6 · normal ΔE 29.6
  down(#843900) vs chart-6(#7405c3): CVD ΔE 28.1 (deutan) · tritan 13.5 · normal ΔE 29.7
```

No pole/chart pair is below the ≥6 protan/deutan floor. Notes for review:

1. **up-pole ≡ `--chart-5` (ΔE 0), both themes.** Deliberate continuation of
   W2's "`--chart-wr` aliases the chart-5 blue" decision, kept per the W5a
   brief (blue stays the up-pole). The only co-appearance is the maps tab,
   where chart-5 is the *Escort* section **header text** — a word, not a
   mark; poles and `--chart-1..6` never co-appear as marks anywhere in the
   stats page (heroes/maps charts are pole-only; role/mode colors live on
   text and borders).
2. **down-pole vs `--chart-4` dark ΔE 6.0 (deutan)** — exactly at the ≥6
   floor, the tightest pair. Same text-vs-mark situation (chart-4 dark is the
   *Clash* header text). Going higher is not reachable inside the dark band:
   +L loses the deutan margin, −L breaks 3:1 surface contrast, +C leaves
   sRGB gamut. Every bar row also carries a direct visible label
   ("`54.2% (13 games)`"), which is the validator's prescribed secondary
   encoding, and the full data table sits below each chart.
3. **Low tritan values on up-pole vs chart-2** (2.8 dark / 5.3 light) are
   informational — the validator PASSes them (its floor is protan/deutan) —
   and pre-date W5a: `--chart-wr` has been this same blue since W2, and
   chart-2/chart-5 are non-adjacent slots the W2 adjacent-pairs run never
   tested. No W5a regression.

### Out-of-scope (unchanged, per the brief)

- Role colors, mode chip/label colors, WR **text** (stays plain text tokens
  per the W2 decision), history tab, `pages/stats_member.rs` (no bars).
