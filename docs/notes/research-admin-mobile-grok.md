# Research: admin UI mobile audit (grok, 2026-07-17)

**Scope (research only, no code):** `layouts/admin.rs`, `styles/admin.rs`, `components/admin_shared.rs` (DataTable / FormModal / ConfirmDialog), and representative `pages/admin/**` table-heavy screens.

**Viewport:** `crates/app/index.html` already has  
`<meta name="viewport" content="width=device-width, initial-scale=1" />` — not the problem.

**Method:** static review of layout CSS + shared table/modal components; column-count scan of admin DataTables. No device lab run (user offline / night shift).

---

## Ranked findings

### P0 — Admin shell is desktop-only (sidebar + content)

**Where:** `layouts/admin.rs` `.admin-layout` / `.admin-sidebar` / `.admin-main`

| Issue | Detail |
|--------|--------|
| Fixed sidebar width | `width: 220px` always; **no** `@media` collapse, drawer, or hamburger |
| Sticky full-height nav | `position: sticky; height: 100vh` — on a ~375px phone, ~220px is permanently consumed; content gets ~150px |
| No off-canvas pattern | Nav links (~12 officer / ~16 admin) stay in a permanent left column; cannot free horizontal space |
| Main padding | `padding: 2rem 2.5rem` — large gutters on small screens (reduces usable width further) |

**Impact:** Admin is effectively unusable on phone width without landscape + zoom. This dominates every page.

**Smallest fix (proposal):**  
1. `@media (max-width: 768px)`: hide sidebar by default; top bar with brand + menu toggle; sidebar becomes `position: fixed` drawer (slide over / full-height) with backdrop.  
2. Reduce `.admin-main` padding to `1rem` below 768px.  
3. Optional: sticky bottom primary action only where needed (settings already has sticky toolbar).

---

### P0 — DataTable has no horizontal overflow containment

**Where:** `components/admin_shared.rs` `DataTable` → bare `<table class="data-table">`; CSS in `styles/admin.rs` sets `width: 100%` but **no** wrapper with `overflow-x: auto`.

Wide tables in admin:

| Page | Header columns |
|------|----------------|
| Matches | 7 (Opponent, Score, Map, Type, Public, Date, Actions) |
| Applications | 6 |
| Moderation | 7 |
| Audit log | 5 |
| Members / Teams / Articles / … | 4–5 + Actions |

**Impact:** On narrow viewports the table forces document-wide horizontal scroll (or clips Actions), fighting the already-narrow content column after the sidebar.

**Smallest fix:** Wrap table in `.data-table-scroll { width: 100%; overflow-x: auto; -webkit-overflow-scrolling: touch; }` inside `DataTable` only (one component change fixes all admin tables). Optionally `min-width` on table so columns stay readable while scrolling.

---

### P1 — Touch targets on row actions too small

**Where:** `.row-btn` — `padding: 0.2rem 0.55rem; font-size: 0.7rem`

**Impact:** Edit/delete/approve chips hard to hit with a thumb; risk of mis-taps on danger actions.

**Smallest fix:** Under `@media (max-width: 768px)` (or always): min height ~44px / padding `0.45rem 0.75rem`; keep `.row-actions { flex-wrap: wrap }`.

---

### P1 — Dense multi-action rows wrap poorly next to many columns

**Where:** `.row-actions` + last table column

**Impact:** Even with horizontal scroll, the Actions cell stacks tiny buttons; still awkward on touch.

**Smallest fix (after scroll wrapper):** Prefer a single “⋯” / “Actions” control opening a small menu **or** full-width card layout for rows below 640px (larger change — park after P0).

---

### P2 — Form modals mostly OK; polish only

**Where:** `.form-modal` — `width: 90vw; max-width: 500px; max-height: 85vh; overflow-y: auto` (wide: 640px)

| Good | Gap |
|------|-----|
| Fluid width | Footer buttons not sticky inside tall modals (scroll to find Save) |
| Overlay full-screen | iOS keyboard can cover focused inputs near bottom (generic mobile web) |
| form-grid collapses at 480px | `.form-modal-footer` could stack full-width buttons under 400px |

**Smallest fix:** `@media (max-width: 480px)` stack footer buttons full-width; optional sticky footer on modal shell.

---

### P2 — Settings / identity packs already partial-mobile

**Where:** `styles/admin.rs` settings / pack-grid / option-tiles / copy-panel

- `auto-fill` + `minmax(...)` grids are generally OK.  
- `.copy-panel-body .form-grid` already collapses to 1 column at 640px.  
- `.color-field { min-width: 11rem }` is fine.  
- Sticky settings toolbar is good for mobile once shell is fixed.

No P0 here once shell + tables work.

---

### P3 — Typography / h1 size

**Where:** `.admin-main h1 { font-size: 1.8rem }`

**Impact:** Cosmetic only; slightly large title + padding wastes vertical space on phone.

**Smallest fix:** `1.35rem` under 640px.

---

## What is already fine

- Global viewport meta present.  
- Form modal uses `vw` + max-height + internal scroll.  
- Toolbar flex-wrap.  
- Summary cards auto-fill.  
- No hardcoded `px` page widths on admin content (problem is layout chrome + tables, not fixed page `width: 1200px`).

---

## Smallest-fix-first proposal (implementation order)

| Step | Change | Effort | Unlocks |
|------|--------|--------|---------|
| 1 | `DataTable` scroll wrapper + optional table `min-width` | ~15 min | Tables usable in landscape / after shell fix |
| 2 | Mobile admin shell: drawer sidebar + reduced main padding | ~1–2 h | Actual phone admin access |
| 3 | Larger `.row-btn` hit areas on small breakpoints | ~15 min | Safer moderation / approve taps |
| 4 | Modal footer stack + optional sticky footer | ~30 min | Long edit forms on phone |
| 5 | (Later) Card/list alternative to tables for key queues (applications, matches) | 1+ day | Best UX; not needed if 1–3 land |

**Do not block on:** redesigning every admin page; Discord vs local register (Claude’s lane). Mobile shell + table scroll are independent of account-creation work and can ship as a thin CSS/layout PR after peer review.

## Out of scope / non-findings

- Strategy editor canvas (separate product surface).  
- Public site marketing pages.  
- Prod OAuth / register — see `docs/notes/research-account-creation-claude.md`.

## Sync note for Claude

Mobile findings ready for merge into one user plan with local-register design. Recommend plan section order: (1) account creation path, (2) admin mobile P0 shell + tables so officers can process applications from a phone after signup works.
