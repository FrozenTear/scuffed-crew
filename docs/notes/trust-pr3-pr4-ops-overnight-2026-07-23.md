# Trust train follow-on — PR3 / PR4 + ops (overnight-ready)

**Status:** QUEUED for later today / overnight  
**Author:** grok (2026-07-23)  
**Parent plan:** `docs/notes/product-trust-plan-2026-07-23.md`  
**Baseline after train:** `origin/main` @ **`942e9b2`** (PR1 #13 + PR2 #14 MERGED)

---

## Done already (do not re-do)

| PR | Merge | Scope |
|----|--------|--------|
| **PR1 P0** #13 | `f5a304d` → main | audit `actor_name`, scrims flatten-limit, strategy `data`, games `CursorResponse`, loaders error UX |
| **PR2 P1** #14 | `942e9b2` → main | applications/moderation names, View drawer, trial actions, `format_datetime` |

Live reconfirm after deploy still useful (see §5) but code is on main.

---

## Goal for this follow-on

Finish the **product polish + content** half of the trust plan so the site matches a 2-person org (not an empty esports shell):

1. Copy / empty states / pluralization that don’t look broken.
2. Nav that doesn’t over-promise empty surfaces (tournaments).
3. Admin dashboard that points at work + health.
4. Ops: strip test fixtures, seed one real hangout + announcement if USER wants.

**Non-goals:** strategy editor features, Nostr relay production, multi-tenant, stat-tracker OCR, PR1/PR2 rewrites.

---

## PR3 — Product polish (code)

**Suggested branch:** `fix/trust-p2-empty-nav-copy`  
**Suggested worktree:** `.claude/worktrees/<agent>-trust-pr3`

### Scope

| ID | Item | Notes / likely touch |
|----|------|----------------------|
| **P3-1** | Pluralization | Home “1 players” → correct plural (`n player` / `n players`). Find team roster count render on homepage. |
| **P3-2** | Empty-state copy | Tournaments, scrims, strategy, news/comps blocks: purposeful empty text + optional CTA (not bare silence). Prefer shared empty-state component if one exists. |
| **P3-3** | Nav defaults | Demote **Tournaments** from primary → More or Hidden until used. Prefer **Settings → Navigation** defaults / seed pack over hard-coding one clan. Document recommended small-org layout: Members, Forum, Events, Stats primary; Tournaments/Scrims/Strategy secondary. |
| **P3-4** | Dashboard health | Admin dashboard: click-through KPI cards; chips when audit/relay/forum mode look unhealthy (relay Offline + forum=local → soft “optional”, not alarm). |
| **P3-5** | Public member cards | Enrich if fields already public (main role, heroes, avatar). No new privacy surface without check. |
| **P3-6** | Brand note | CLAUDE.md / brand docs say purple `#7c3aed`; live Settings mint `#46d8a4`. Either update docs to mint **or** note dual-theme; no forced pack apply without USER. |

### Acceptance (PR3)

- [ ] Browser: home grammar fixed; empty lists read intentional.
- [ ] Fresh install / settings defaults: tournaments not top-nav primary for small-org pack (or documented how to set).
- [ ] Dashboard cards navigate or filter usefully.
- [ ] `cargo check` / relevant tests green; clippy/fmt clean.
- [ ] Dual-agree before merge; author-no-merge.

### Partition hint

Single agent can own full PR3 (file surface is mostly `crates/app` + maybe settings seed). If dual: one owns nav/settings, one owns empty states/dashboard — **no shared file** without sequential handoff.

---

## PR4 — Ops content (mostly non-code)

**Not a code PR** unless seed scripts need a tiny helper. Treat as **ops checklist** with human gate on prod data.

### Checklist (production `ow.scuffedcrew.no`)

| ID | Action | Risk |
|----|--------|------|
| **O-1** | Rename/delete **test** hangout event (`test` Mon 21:00) → real hangout name or remove | Low |
| **O-2** | Replace **Test announcement** with real news or unpublish | Low |
| **O-3** | Draft/delete **Test article** | Low |
| **O-4** | Forum board named **test** → rename or archive | Low |
| **O-5** | Confirm Settings: mint accents intentional vs re-apply Scuffed purple pack | Product decision — **USER** |
| **O-6** | Optional: seed **one** real hangout + **one** real announcement | Content — **USER** |
| **O-7** | After deploy of PR1/PR2: browser reconfirm audit / scrims / strategy / applications names | Verify |

### Guardrails

- No bulk delete of members/applications without USER.
- No `rm` of production DB; prefer admin UI or documented Surreal ops.
- No force-push; no tag/release unless USER.
- Test fixtures in `stat-tracker/test-data/` stay gitignored.

---

## Overnight execution protocol

If USER says “overnight it”:

1. **Orient:** `origin/main` tip; fleet `fleet::trust-pr3` + `fleet::chat`; this doc.
2. **Claim:** one agent claims PR3 in worktree; post intent on `fleet::trust-pr3`.
3. **Implement PR3 only** unless USER also greenlights ops (PR4).
4. **Ops (PR4):** only with explicit USER OK for prod writes; prefer listing planned renames first for ACK.
5. **Review:** dual-agree; author never merges own PR.
6. **Stop:** leave open items + SHAs on fleet; no silent PR3/PR4 expansion into strategy/OCR.

### Suggested overnight order

1. P3-1 pluralization (smallest, high polish).  
2. P3-2 empty states.  
3. P3-3 nav defaults / settings.  
4. P3-4 dashboard.  
5. P3-5 / P3-6 if time.  
6. Stop; draft PR; dual-agree.  
7. Ops O-1…O-4 **only if USER pre-authorized prod ops**.

---

## Browser reconfirm pack (post-deploy + post-PR3)

Re-run against live (Helium / browser-use):

| Marker | Expect after PR1/PR2 deploy |
|--------|-----------------------------|
| `/admin/audit-log` | Rows with **names**, no deser error |
| `/scrims` | Empty board or list — **not** eternal Loading |
| `/strategy` | “No strategies found” — **not** eternal Loading |
| `/admin/applications` | Display names + game names; View works; trial has Accept/Reject |
| `/admin/moderation` | Member / Issued By names; datetime format |
| Home | After PR3: plural grammar; after ops: no “test” hangout |

---

## Fleet threads

| Thread | Use |
|--------|-----|
| `fleet::trust-pr3` | PR3 claims, REVIEW REQUEST, dual-agree, MERGED |
| `fleet::trust-pr4-ops` | Ops checklist progress (optional; or keep on trust-pr3 if tiny) |
| `fleet::chat` | Short pointers only |

Parent initiative history: `fleet::trust-plan` (PR1/PR2 closed).

---

## Open decisions for USER

1. Overnight **code-only (PR3)** vs **code + prod ops (PR4)**?  
2. Brand: keep **mint** or re-assert **purple**?  
3. Nav: apply small-org defaults to **live settings** or only to **seed/new installs**?
