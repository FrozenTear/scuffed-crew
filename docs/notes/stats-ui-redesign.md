# My Stats redesign — merged design plan (STATS-PLAN-1)

**Merged from:** claude's critique (scratchpad, fleet `01KXZRRNCR`) + grok's critique
(`docs/notes/stats-ui-critique-grok.md` @ a8613eb, fleet `01KXZRZHZM`) — written
independently, then unified. Convergent findings are marked **[BOTH]**; unique finds
credit their author. Dual-agree on this doc, then USER sign-off, before any impl.

**Target:** `crates/app/src/pages/stats.rs` (~1080 LOC) + shared tokens.
**North star (grok's framing, adopted):** a competitive personal performance
dashboard for a member who already tracks; setup is secondary once
`total_matches > 0`; Overview scannable in one viewport.

---

## W1 — Structural (author: claude · reviewer: grok)

1. **Progressive disclosure of setup chrome** [BOTH — each ranked it #1].
   When `total_matches > 0`: install card + daemon settings collapse to one slim
   row (`Tracker · Linux · [reinstall] [tokens]`, settings in `<details>`).
   Zero-match users keep the full onboarding cards (that's their primary UI —
   grok's "two products on one page" framing).
2. **Summary hierarchy** [BOTH]. Win Rate leads (larger tile), W–L–D record as its
   subtitle; MATCHES secondary; DRAWS demoted into the record string. Values wear
   text tokens — mint-on-everything ends; color is reserved for meaning (claude).
3. **Split `stats.rs` into per-tab modules** [BOTH]. Mechanical, zero-behavior,
   done first so every later wave ships as small reviewable PRs (grok: "hard to
   dual-agree small PRs against a monolith").
4. Empty-state triad (grok G8): first-run vs no-data-for-tab vs load-error get
   three distinct messages.

## W2 — Chart & token correctness (author: claude · reviewer: grok)

5. **End the three-color-system collision** [BOTH, identical diagnosis]:
   role identity / winrate traffic-light / mode identity all use overlapping hues
   (green = Support = winning = Control on one page).
   Merged rule set:
   - Role colors appear **only** on role surfaces (donut, role cards).
   - Mode colors appear **only** on chips/labels/section headers — never bars (both).
   - **Bars encode winrate once**: single accent fill + a **50% reference
     hairline** (claude C2; grok G4 "pick one encoding" concurs). No 3-bin
     traffic light on bars anywhere.
   - WR **text** values: at most a two-state polarity (above/below 50%), exact
     treatment decided in-wave against the validator — or plain text tokens.
     (Minor open item; the 3-bin dies either way.)
   - **DEFEAT chip → serious/red token, never gold** [BOTH]. VICTORY = good,
     draw = neutral. Status tokens used only as status.
   - Brand purple: active tab + focus, not whole panels (grok G7).
6. **CVD-validate the palette** (claude C7): run `validate_palette.js` on
   `--chart-1..6` + status tokens against BOTH light and dark surfaces; snap
   failing steps. First validation these tokens will ever get.
7. **Sample-size honesty everywhere** [BOTH]: min-games gate (default 3+) for any
   "top WR" chart/callout; `(n games)` always shown; n<3 rows muted in tables
   instead of loud green/red on 1-game 100%/0%.

## W3 — Content (author: grok · reviewer: claude)

8. **Overview insight grid** (grok G2, concrete form of claude D3): 2×2/2×3 —
   role mix (donut stays, ≤6 segments compliant) · role performance · top-heroes
   mini (gated) · mode WR chips · last-10 form strip. Claude adds: winrate-over-
   time sparkline if history is already loaded (decide vs form-strip API open Q).
9. **Heroes tab**: role filter chips (Tank/Damage/Support, client-side via
   `hero_to_role`) + sort control (WR / volume / avg) (grok G4; claude D1 concurs
   on sort). Hero icons = out of scope unless assets exist.
10. **Maps tab**: best/worst map callout chips (gated), empty modes
    collapsed/omitted (grok G5).
11. **History tab**: outcome/role/hero filters (client-side v1), optional by-day
    session grouping (grok G6). Fix the missing-map dangling separator
    ("· Tank") with "Unknown map" (claude D2 / grok G6 convergent).

## W4 — Polish (author: grok · reviewer: claude)

12. Tables: right-aligned numeric columns + `tabular-nums`; sortable headers
    (claude D1). History stat clusters aligned as columns; relative dates.
13. Hold-previous-render on refetch, no skeleton flash (claude D4).
14. Mobile pass: tile wrap, table overflow containers (both flagged, verify not
    assume).
15. Public `stats_member` parity — IF USER says yes (grok G1.4, open Q).

---

## Open questions → USER

**ANSWERED by USER 2026-07-20 (impl greenlit, USER afk, dual-agree governs):**

| # | Question | USER answer |
|---|----------|-------------|
| Q1 | Public member stat pages match this language? | **Yes** — parity in W4 |
| Q2 | Density target? | **"Why not both"** → density toggle (compact ⇄ comfortable), default dense — interpretation to be grok-ACKed |
| Q3 | Form strip data source? | Explained; defaulting to **(a) client-side reuse of history page 1** (agents' call) |
| Q4 | Donut vs bars? | **Donut stays** (tighten labels only) |
| Q5 | Wave plan + roles | **Approved** — claude+grok proceed per fleet protocol, dual-agree every merge |

## Process

Symmetric authorship 2/2 (W1+W2 claude, W3+W4 grok), author never self-merges,
dual-agree per PR, stacked small PRs over mega-PRs, CI green required (UI PRs
included). Waves land in order; W1's module split unblocks parallel W2/W3 work.

---

## W5 — post-ship USER feedback iteration (2026-07-20 evening)

USER reviewed the live page: likes the design; single-accent bars "hard to
differentiate" at ~30-bar scale (pre-attentive polarity lost with the 3-bin).

- **W5a (claude/grok):** two-pole diverging bars — `--chart-wr-up` (cool) above
  50%, `--chart-wr-down` (warm) below, **neutral at exactly 50%** and for
  n<MIN_GAMES (muting never claims a pole). Pair validator-passed both themes
  (ΔE ≥25); collision sweep vs chart-1..6 recorded in
  `stats-ui-w2-validation.md` §W5a. `--chart-wr` retired. Distinct from the
  killed 3-bin: 50% is a real midpoint → legitimate diverging encoding.
- **W5b (grok/claude):** history rows columnized — fixed E/D/A | dmg | heal,
  right-aligned tabular-nums, tighter rows; by-day grouping optional later.

Note: this file was accidentally never merged in the initial cleanup (branch
deleted unmerged); restored from e3d93f0 in W5a.
