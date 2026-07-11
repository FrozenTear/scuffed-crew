# Website fix list — ow.scuffedcrew.no

**Source:** live + code review (2026-07-12)  
**Deploy:** https://ow.scuffedcrew.no  
**Status:** P0/P1 code done (2026-07-12); P0-3 content + P1-6 Discord still open

Priorities: **P0** ship this week · **P1** next product pass · **P2** design-system debt

---

## P0 — First impression / ship blockers

### P0-1. Fix document title — **done (code)**
**Problem:** Live shell serves `<title>dioxus | ⛺</title>` instead of the org name.  
**Root cause:** `Containerfile` did not `COPY Dioxus.toml`, so `dx build` used CLI defaults.  
**Fix:**
- [x] `Containerfile` copies `Dioxus.toml`
- [x] `document::Title` in `main.rs` after WASM boot
- [x] Custom `crates/app/index.html` with correct title
- [x] `scripts/build.sh` warns if default title ships

**Done when:** Tab title on ow.scuffedcrew.no is “The Scuffed Crew” after redeploy.

---

### P0-2. Add description, favicon, basic meta — **done (code)**
**Problem:** Shell has only charset/viewport. No description, OG tags, or favicon.  
**Fix:**
- [x] Meta description + `og:title` / `og:description` / `theme-color` in shell + runtime
- [x] `crates/app/assets/favicon.svg` (SC mark) + document link
- [x] First-paint loading skeleton (see P1-8)

**Done when:** View-source after redeploy shows description + favicon; Discord unfurl is non-empty.

---

### P0-3. Replace or remove staging content — **open (admin)**
**Problem:** Live data undermines the brand (“no ghost members / intentional rosters”).  
**Work (admin / content, not code):**
- [ ] Delete or rewrite announcements titled **“test”**
- [ ] Delete or rename event **“test”** (or set a real play night)
- [ ] Fill **Scuffed Crew** roster (code now labels empty as Forming — still better to fill)
- [ ] Align **seeking tags** with actual games (tags mention OW2 + D2; DB currently has Overwatch only)

**Done when:** A cold visitor sees no “test” strings and the squad/roster story is honest.

---

### P0-4. Remove Dev login from public UI — **done (code)**
**Problem:** Apply page exposes “Dev login” → `/api/dev/login`.  
**Fix:**
- [x] Gate behind `cfg!(debug_assertions)` on Apply, Login, Admin access-denied
- [x] Server still only registers route when `SURREALDB_URL` unset

**Done when:** Production release build has no Dev login affordance.

---

### P0-5. Honest empty / forming squad state — **done (code)**
**Problem:** Home shows zero-roster team as a normal active squad.  
**Fix:**
- [x] Label **Forming** + roster column **Open** when `roster_count == 0`
- [x] Metrics still skip empty squads for the “active squads” count

**Done when:** Empty roster is clearly forming, not a full competitive team.

---

## P1 — Product / UX polish

### P1-1. Unify brand accent (drop legacy `#7c3aed`) — **done (code)**
**Fix:** Home CSS uses `var(--accent)` / `var(--warn)` / `var(--danger)` + `color-mix`; no `#7c3aed`.

---

### P1-2. Single footer source of truth — **done (code)**
**Fix:** `PublicLayout` footer = `© {org_name} · {site_description}` from settings. Home only shows optional `footer_note`.

---

### P1-3. Org name in page titles / apply copy — **done (code)**
**Fix:** Apply H1/success and Community hero/banner/publish use `settings.org_name`. NIP-05 domain externalization remains P2-4.

---

### P1-4. Mobile nav: theme toggle + account actions — **done (code)**
**Fix:** Theme toggle beside hamburger on mobile + in overlay; Apply CTA styled in overlay.

---

### P1-5. Verify Forum public path end-to-end — **done (verified)**
**Note:** App correctly uses `GET /api/forum/tree` (200 live). Earlier 405 was wrong path (`/api/forum/boards` is POST-only). Improved fail copy.

---

### P1-6. Discord OAuth when recruiting opens up — **open (ops)**
**Work:**
- [ ] Configure Discord OAuth in production secrets
- [ ] Confirm Login shows “Sign in with Discord”
- [ ] Apply → login → back to apply still works

**Done when:** Applicants can sign in with Discord without a local password.

---

### P1-7. 404 page polish — **done (code)**
**Fix:** Tokenized 404 with path hint + home CTA.

---

### P1-8. Reduce blank first paint — **done (code)**
**Fix:** Boot skeleton in `crates/app/index.html` (SC mark + Loading), auto-hides when `#main` mounts.

---

## P2 — Design system / platform debt

### P2-1. Finish migrating public pages onto `components/ui`
**Problem:** Library exists (`Button`, `Card`, `EmptyState`, `PageShell`, …) but most pages still inject one-off `const PAGE_CSS` (~71 blocks). Home is the worst (~1150 lines).  
**Batches (from design revamp plan):**
- [ ] Content/public: home, members, profile, news, blog, community, apply, events, polls, wiki, forum, tournaments, scrims, stats, dm, identity
- [ ] Admin pages
- [ ] Strategy section last (has its own accent)

**Done when:** Primary public flows use `PageShell` + shared buttons/cards; page CSS is layout-only.

---

### P2-2. Raw hex / rgba guard
**Problem:** Spec required CI to ban raw colors outside `theme/`; ~39 hits remain in pages/layouts/components.  
**Work:**
- [ ] Confirm/extend guard script (e.g. next to `scripts/check-frontend-deps.sh`)
- [ ] Clean remaining violations, starting with public chrome + home

**Done when:** CI fails on new raw hex outside `theme/` (and canvas exceptions if needed).

---

### P2-3. Brand settings (design SP2)
**Problem:** `BrandConfig` is hardcoded; white-label/settings-driven accent not wired.  
**Work (future sub-project):**
- [ ] Admin brand: accent, logo, fonts density
- [ ] Wire `theme::brand::current()` (or async brand) from `/api/settings`

**Done when:** Accent/logo changeable without code change.

---

### P2-4. Externalize domain strings
**Problem:** `scuffed.gg` / NIP-05 domain still hardcoded in places (identity, community copy, server routes).  
**Work:** Settings field for primary domain; use for NIP-05 display and docs links.

**Done when:** No hardcoded production domain in user-facing strings for multi-host deploys (`ow.scuffedcrew.no` vs future apex).

---

### P2-5. Deduplicate security headers
**Problem:** `referrer-policy`, `x-frame-options`, `x-content-type-options` appear twice (app + Caddy). Harmless but noisy.  
**Work:** Set headers in one place (prefer Caddy edge **or** Axum, not both).

**Done when:** Response has a single copy of each security header.

---

## Ops / content checklist (no code)

Use after deploy or content edits:

| Check | How |
|-------|-----|
| Title | Open site → browser tab text |
| Meta | View-source description + favicon |
| No “test” content | Home news + schedule + `/news` + `/events` |
| Roster honesty | Home squads section |
| Apply funnel | Logged out → Apply → Login → Apply |
| No Dev login | Apply + Login HTML |
| Theme | Toggle light/dark on home + apply |
| Mobile | ≤820px: nav, Apply, theme |
| Forum | Logged out `/forum` |
| Share | Paste URL in Discord |

---

## Suggested order of work

```
Week 1 (P0)
  P0-3 content cleanup          (admin, no PR)
  P0-1 title fix                (build/shell)
  P0-2 meta + favicon           (shell/assets)
  P0-4 hide Dev login           (app)
  P0-5 empty roster UX          (app, small)

Week 2 (P1)
  P1-1 home token colors
  P1-2 footer single source
  P1-3 org_name on Apply/Community
  P1-4 mobile nav
  P1-5 forum verification/fix
  P1-6 Discord when ready

Later (P2)
  P2-1 page migration
  P2-2 hex CI guard
  P2-3 brand settings
  P2-4 domain externalization
  P2-5 header dedupe
```

---

## Tracking

| ID | Owner | Status |
|----|-------|--------|
| P0-1 | | done (needs redeploy) |
| P0-2 | | done (needs redeploy) |
| P0-3 | | open (admin content) |
| P0-4 | | done |
| P0-5 | | done |
| P1-1 | | done |
| P1-2 | | done |
| P1-3 | | done |
| P1-4 | | done |
| P1-5 | | done (verified) |
| P1-6 | | open (ops / Discord secrets) |
| P1-7 | | done |
| P1-8 | | done |
| P2-1 | | open |
| P2-2 | | open |
| P2-3 | | open |
| P2-4 | | open |
| P2-5 | | open |

Mark status `in_progress` / `done` as work lands; tick boxes under each item when verifying.
