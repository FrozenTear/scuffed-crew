# My Stats UI — independent critique (Grok)

**Lane:** `fleet::stats-ui`  
**Author:** grok (independent of claude’s critique until merge)  
**Code baseline:** `crates/app/src/pages/stats.rs` (~1080 LOC), `stats_tokens.rs`  
**Screens reviewed:** Overview / Heroes / Maps / History (USER 2026-07-20, ~199 matches sample)

Brand constraints: `#7c3aed`, dark-first, 16+, direct tone.

---

## North star

Competitive **personal performance dashboard** for an org member who already tracks.  
Setup/onboarding is secondary once `total_matches > 0`. Analytics should be scannable in one viewport of Overview.

---

## G1 — Structural / IA

### G1.1 Setup chrome steals the hero (highest impact)
Always-visible **Get the Stat Tracker** (curl, Linux badge, release link) + **Daemon Settings** sit above summary cards on every tab. For a user with hundreds of matches this is the wrong primary content.

**Proposal:** Progressive disclosure once tracking is live (e.g. `total_matches > 0` or last successful upload known):

- Collapse install block to one line: `Tracker · Linux · [reinstall] · [tokens]`
- Daemon settings stay in `<details>` (already partial)
- Optional status chip in header: `Tracker ●` / last upload if API allows later

### G1.2 Two products on one page
Onboarding + analytics fight for attention. Split mentally even if same route:

| Mode | Primary UI |
|------|------------|
| Empty / zero matches | Install CTA + first-run copy |
| Active tracker | Summary + tabs; install collapsed |

### G1.3 `stats.rs` monolith
~1080 LOC: models, CSS, role/map aggregation, all four tabs. Hard to dual-agree small PRs.

**Proposal:** Wave-1 split into tab modules (or shared `stats/` dir) without behavior change where possible; CSS co-located or shared tokens only.

### G1.4 Public parity
`stats_member.rs` exists. Design should state whether public member pages track the same visual language (at least summary + heroes table).

---

## G2 — Overview thin for available data

Current: role donut + role WR cards only. APIs already load heroes + maps + summary.

**Missing at a glance:**

| Insight | Source (likely no new API) |
|---------|----------------------------|
| Form strip (last 10 W/L) | History page / matches feed |
| Main hero + main map | Max matches from heroes/maps |
| Role WR already present | Keep, tighten |
| Weakest mode | Map mode aggregates |

**Proposal:** Overview 2×2 (or 2×3) insight grid:

1. Role mix (donut)  
2. Role performance cards  
3. Top heroes mini (3–5, min-games gate)  
4. Mode WR summary (Escort/Hybrid/Control/Push chips)  
5. Optional: last-10 form strip when history is cheap

---

## G3 — Summary strip hierarchy

Five equal tiles (Matches / Wins / Losses / Draws / Win Rate). No emphasis.

**Proposal:** Lead with **Win Rate** (larger or accent border); secondary counts; draws demoted if always 0 for this game mode mix.

---

## G4 — Heroes

**Works:** WR bars (3+ matches) + full table.

**Gaps:**

| Issue | Fix |
|-------|-----|
| No role filter | Tank / Damage / Support chips (client-side via `hero_to_role`) |
| No sort control | WR vs volume vs elims |
| Low-sample noise | Default min 3 games for chart; table can show all with muted WR |
| Text-only identity | Optional later: hero icons (OOS unless assets exist) |
| Double-encoding risk | Bar length = WR; avoid also painting bar color as WR bins if table already has WR class — pick one encoding (align with claude if they flagged same) |

Hero-filter API landed on **roster/leaderboards**; personal heroes list may remain full client filter for v1.

---

## G5 — Maps

**Works:** Mode groups (Escort / Hybrid / Control / Push) with WR bars.

**Gaps:**

| Issue | Fix |
|-------|-----|
| Mode color on bar is decorative | Use mode color for **label/chip only**; bar = WR single scale or neutral fill |
| Sample size honesty | Show `(n games)` always; de-emphasize n&lt;3 (opacity or “low n”) |
| Best/worst callouts | Chips above groups: best map / worst map (min-games gate) |
| Flashpoint/Clash empty | Still list empty modes collapsed or omit until data |

---

## G6 — History

**Works:** Card list with outcome · hero · map/role · E/D/A · date.

**Gaps:**

| Issue | Fix |
|-------|-----|
| No filters | Outcome / role / hero (client on loaded page; server later if needed) |
| Flat feed | Optional session group by day |
| Scan cost | Align columns denser; ensure map never empty-separator glitch |
| No detail route | OOS unless match detail exists |

---

## G7 — Visual / token systems

Observed collisions (from screens + code patterns):

1. **Role identity colors** (donut / role cards)  
2. **Winrate traffic light** (high/mid/low → ok/warn/danger)  
3. **Mode identity colors** on map bars  

**Proposal (merge with claude CVD notes):**

- Role = categorical palette only on role surfaces  
- WR = traffic light **only** on WR text/chips, not on categorical bars  
- Mode = chip/label color only  
- DEFEAT/VICTORY chips: defeat must use `--danger` (or serious red token), never gold/warn  
- Brand purple: active tab + focus only, not whole panels  

---

## G8 — Content rules

1. **Min-games gate** default 3+ for any “top WR” chart/callout (already used for hero bars — apply everywhere).  
2. **Don’t invent stats** the API doesn’t serve; form strip needs matches page or a small last-N endpoint if we refuse to load full history.  
3. Linux-only tracker copy stays accurate.  
4. Empty states: first-run vs no hero data vs load error — three different messages.

---

## Proposed waves (impl partition)

| Wave | Focus | Notes |
|------|--------|------|
| **W1** | Structural | Progressive disclosure setup; summary hierarchy; optional split `stats.rs` modules; dual-agree layout only |
| **W2** | Chart correctness | Single WR encoding; mode bars; defeat chip; token collision cleanup |
| **W3** | Content | Overview insight grid; heroes filters/sort; maps callouts; history filters |
| **W4** | Polish | Mobile, table alignment, split remaining debt, public `stats_member` parity if agreed |

**Symmetry:** each wave has author + peer review; author never sole-merges. Prefer stacked small PRs over one mega PR.

---

## Open questions (USER / dual-agree)

1. Public `stats_member` must match personal My Stats?  
2. Density target: competitive-dense vs airier marketing?  
3. Form strip: require new API or load matches only on Overview?  
4. Merge policy AFK: reviewer-merge docs-only OK? CI green required for UI PRs?

---

## Next step after both critiques posted

1. MERGE session: unify G\* + claude S\*/C\* into one plan doc (`docs/notes/stats-ui-redesign.md`).  
2. Dual-agree plan.  
3. Partition W1 authors and open worktrees.

---

## Pointers

- Code: `crates/app/src/pages/stats.rs`  
- Thread: `fleet::stats-ui`  
- This doc branch: `docs/stats-ui-critique-grok` (push after commit)
