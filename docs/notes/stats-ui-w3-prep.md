# Stats UI W3 prep (design detail only — no impl until W1 lands)

**Author:** grok  
**Depends on:** W1 module split (`feat/stats-ui-w1-structural`) + dual-agree  
**Plan:** `docs/notes/stats-ui-redesign.md` @ `e3d93f0` (STATS-PLAN-1 + USER Q1–Q5)

---

## ACK block (for fleet)

| Item | Verdict |
|------|---------|
| STATS-PLAN-1 merge decisions (a)(b)(c) | Already APPROVED (`01KXZSA785`) |
| USER Q1 public parity W4 | ACK |
| USER Q2 “why not both” → **density toggle** compact⇄comfortable, **default dense** | **ACK** (claude interpretation) |
| USER Q3 form strip = client history page 1 | ACK |
| USER Q4 donut stays | ACK |
| USER Q5 AFK dual-agree exec | ACK |
| W1 claude-author / grok-review | Standing by REVIEW REQUEST on push |

---

## Density toggle (Q2) — mechanics for W3/W4

| Mode | Class / attr | Behavior |
|------|----------------|----------|
| **compact** (default) | `data-density="compact"` on `.stats-page` | Tighter gaps (0.5–0.75rem), smaller type on sublabels, denser match cards |
| **comfortable** | `data-density="comfortable"` | Larger gaps (1–1.25rem), airier cards, more padding on tables |

- Control: small toggle in stats header (next to tokens), label “Density” or icons only  
- Persist: `localStorage` key `stats-ui-density` (client-only; no API)  
- Scope: personal My Stats first; public `stats_member` can inherit default dense without toggle in W4 unless cheap  

W1 should **not** block on this — leave a stable root class hook if easy; full toggle in W3/W4.

---

## Overview insight grid (W3 item 8)

After W1 hierarchy, Overview content becomes:

```
┌─────────────────────┬─────────────────────┐
│ Role mix (donut)    │ Role performance    │
│ ≤6 segments, labels │ cards (WR + n)      │
│ tightened           │                     │
├─────────────────────┼─────────────────────┤
│ Top heroes mini     │ Mode WR chips       │
│ 3–5, min 3 games    │ Escort/Hybrid/…     │
├─────────────────────┴─────────────────────┤
│ Form strip: last 10 outcomes (from hist)  │
│ [optional sparkline if same load cheap]   │
└───────────────────────────────────────────┘
```

**Data:**

| Widget | Source after W1 modules |
|--------|-------------------------|
| Role mix / cards | existing heroes → `aggregate_roles` |
| Top heroes | heroes list, sort WR, filter matches≥3 |
| Mode chips | maps → `map_game_mode` aggregate |
| Form strip | `use_api_with` matches `limit=10` (or first page already loaded if History visited — prefer dedicated small fetch on Overview so first paint works) |

**Donut:** keep; tighten labels (short role name + % only; no legend bloat).

**No new backend** for form strip v1 (USER Q3).

---

## Heroes tab (W3 item 9)

- Chips: All | Tank | Damage | Support (`hero_to_role`)  
- Sort: WR | Matches | Avg elims (client)  
- Chart: only matches≥3; table all rows with n&lt;3 muted  
- Bars: single accent + 50% hairline (W2 may land first or same rules in W3 if W2 delayed)

## Maps tab (W3 item 10)

- Best / worst map chips (min 3 games) above groups  
- Collapse empty modes  
- Mode color on section header only  

## History tab (W3 item 11)

- Filters: outcome, role, hero (client on loaded pages — note: pagination means filter is “this page” unless we load more; v1 document “filters apply to loaded rows”, or fetch larger limit for filter UX)  
- “Unknown map” if map empty  
- Optional group-by-day  

**Pagination + filter tension:** prefer `limit=50` when any filter active, or document page-local filter. Call out in REVIEW for claude.

---

## Do not implement until

1. W1 merged (or at least on origin and dual-agreed) so module paths are stable  
2. Prefer W2 tokens before visual polish on bars/chips if claude ships W2 next  

---

## Next actions for grok

1. Post fleet ACK (this prep)  
2. On W1 REVIEW REQUEST: full review, dual-agree merge  
3. Then implement W3 in `feat/stats-ui-w3-content` worktree off post-W1 main  
