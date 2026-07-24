# Product Trust Plan — ow.scuffedcrew.no (2026-07-23)

**Author:** grok (grok-build)  
**Status:** PR1+PR2 MERGED on main @ `942e9b2` (2026-07-23). PR3/PR4 queued → `docs/notes/trust-pr3-pr4-ops-overnight-2026-07-23.md`  
**Baseline (original):** origin/main @ `bf6b2e8` (local checkout when plan written)  
**Evidence:** browser-use reconfirm 2026-07-23 (Helium CDP) — 12/12 markers PASS  
**Memtrace:** index present (`scuffed-crew`, ~4.5k nodes, last full index older); symbol hits for `list_scrims`; `find_code` timed out this session — plan grounded via graph symbol + tree grep + live API probes.

---

## 0. Goal

Make the live site and admin **trustworthy for daily use** by a small org:

1. Stop lying UI (infinite “Loading…”, broken audit, raw IDs).
2. Make recruiting / membership ops usable without reading Surreal keys.
3. Align public content + nav with real org size (2 members, no tournaments).
4. Keep architecture decisions intact (membership CAS, audit append-only, mint/settings packs).

Non-goals (this plan): multi-tenant launch polish, Nostr relay production, strategy editor feature work, stat-tracker OCR.

---

## 1. Browser reconfirm (live)

| ID | Surface | Confirmed | Marker |
|----|---------|-----------|--------|
| B1 | `/admin/audit-log` | YES | `Deserialization error: missing field actor_name` |
| B2 | `GET /api/audit-log` | YES | keys: `id,actor_id,action,target_type,target_id,details,created_at` — **no `actor_name`** |
| B3 | `/admin/applications` | YES | Applicant = UUID; Games = game id; Actions empty for accepted/rejected |
| B4 | `/scrims` | YES | stuck `Loading scrims...` |
| B5 | `/strategy` | YES | stuck `Loading strategies...` |
| B6 | `/` | YES | empty fixtures/comps + “Test announcement”; “1 players” grammar |
| B7 | `/members` | YES | letter avatars only (frozen/fucku) |
| B8 | `/tournaments` | YES | “No tournaments yet.” |
| B9 | `/admin/relay` | YES | Offline / inactive while forum backend = local |
| B10 | `/admin/members` | YES | ISO joined timestamps |
| B11 | `/admin/moderation` | YES | member column = `npt5034…` raw ids |
| B12 | `/apply` (admin session) | YES | shows Rejected status |

### Live API probes (root causes)

| Endpoint | Result | Implication |
|----------|--------|-------------|
| `GET /api/audit-log` | 200, no `actor_name` | Frontend `AuditLogEntry.actor_name: String` required → hard fail |
| `GET /api/strategy/strategies` | 200 `{"data":[],"total":0}` | Frontend expects `{strategies,total}` → deserialize fail → forever Loading |
| `GET /api/scrims?limit=100` | **400** `invalid type: string "100", expected u32` | `use_api_list` always appends `limit=100` → list never loads |
| `GET /api/games?limit=100` | 200 **bare array** `[{…}]` | Not `CursorPage{data}` — games list via `use_api_list` fails |
| `GET /api/teams?limit=100` | 200 `{"data":[…]}` | OK for cursor page |

---

## 2. Root-cause map (code)

### P0-A — Audit log schema drift
- **UI:** `crates/app/src/pages/admin/audit_log.rs` requires `actor_name: String`
- **API/types:** `crates/types/src/org/audit.rs` `AuditLogEntry` has only `actor_id`
- **DB:** `crates/db/src/queries/audit_log.rs` `db_to_entry` does not join member names
- **Fix direction:** Prefer **server-enriched** `actor_name` (and optional `target_label`) on list response; keep append-only table schema (no `actor_name` column required). Frontend field becomes present. Fallback display `actor_id` if join misses.

### P0-B — Scrims infinite loading
- **UI:** `crates/app/src/pages/scrims.rs` — `None => "Loading scrims..."` (no error branch)
- **Hook:** `use_api_list` → always `?limit=100` (`crates/app/src/hooks/api.rs`)
- **API:** `GET /api/scrims` rejects `limit` query deserialize (live 400)
- **Also:** `/api/games` is bare array; scrims page also loads games via `use_api_list`
- **Fix direction:**
  1. Fix `PaginationParams` / scrims query to accept limit (or align all list APIs to `CursorResponse`).
  2. Make `use_api_list` resilient: show **error state**, not perpetual loading.
  3. Align `/api/games` with cursor envelope **or** special-case bare arrays in client (prefer server consistency).

### P0-C — Strategy infinite loading
- **UI:** `ListResponse { strategies, total }` in `browse.rs`
- **API:** `StrategyListResponse { data, total }` from `crates/server/src/routes/strategy.rs`
- **Fix direction:** One shape. Prefer rename frontend to `data` **or** serde `#[serde(alias = "data")]` on `strategies`, plus empty-state already exists when parse works.

### P1-A — Applications unreadable
- **UI:** `crates/app/src/pages/admin/applications.rs` comment: *“no joined display name yet”*; renders `user_id` + raw `preferred_games` ids
- Actions only when `status == "pending"` (empty Actions for closed is intentional but harsh)
- **Fix direction:** API DTO with `applicant_name`, `preferred_game_names[]`; View drawer for closed; keep Accept/Reject for pending/trial only (membership CAS rules unchanged).

### P1-B — Moderation raw IDs
- Same class: resolve member id → display name in list DTO.

### P1-C — Human timestamps
- Shared formatter for admin tables (members joined, mod dates, forum threads public).

### P2 — Product / content / nav
- Empty-state copy quality; hide or demote Tournaments in primary nav (Settings already has nav editor).
- Grammar: “1 players” → pluralization helper.
- Dashboard: clickable KPI cards + health chips (audit fail, relay offline if forum=nostr).
- Members public cards: main role / heroes when present.
- Test-data hygiene (ops, not code): rename `test` event/board/announcement.
- Brand: Settings currently mint `#46d8a4` — update CLAUDE.md brand note **or** re-apply purple pack deliberately (product decision).

### P2 — Loading/error UX contract
- Anywhere `use_api*` returns `None` with `error.is_some()`: show toast or inline error + Retry (pattern from audit log). Ban infinite Loading as the only failure mode.

---

## 3. Phased PR plan

### PR1 — Trust fixes (P0) — **ship first**
**Branch:** `fix/trust-p0-audit-scrims-strategy`  
**Scope:**
1. Audit log: enrich list API with `actor_name`; frontend already expects it (or soft-optional + fallback).
2. Strategy list JSON: align `data` ↔ `strategies` (+ regression test).
3. Scrims list query: fix limit deserialization / pagination.
4. Games list envelope consistency (if required for scrims page).
5. Client: `use_api_list` / browse pages surface errors instead of eternal Loading.

**Acceptance (browser + tests):**
- `/admin/audit-log` renders rows (no red error).
- `/scrims` shows empty board (not Loading) when `data: []`.
- `/strategy` shows “No strategies found” (not Loading).
- Unit/integration: audit list JSON includes `actor_name`; scrims `?limit=100` returns 200; strategy list deserializes.

**Out of scope:** nav redesign, content seeding, relay.

### PR2 — Ops readability (P1)
**Branch:** `fix/trust-p1-admin-labels`
1. Applications list DTO: names + game names; detail/view for non-pending.
2. Moderation list DTO: member display names.
3. Shared admin datetime formatting.
4. Optional: target labels on audit rows.

**Acceptance:** No UUID-only primary columns on applications/moderation for known members.

### PR3 — Product polish (P2)
**Branch:** `fix/trust-p2-empty-nav-copy`
1. Pluralization / empty-state copy.
2. Default nav recommendation (Tournaments → More/Hidden until used) via settings defaults or docs for ops.
3. Dashboard health + click-through.
4. Public member card enrichment if fields exist.
5. Doc brand color note vs live mint.

### PR4 — Ops content (non-code checklist)
- Rename/delete test fixtures on production DB (event, announcement, article, forum board).
- Confirm Settings → Scuffed pack vs mint accents intentional.
- Optional: seed one real hangout + one real announcement.

---

## 4. Partition for fleet (if dual-agent)

| Agent | Owns |
|-------|------|
| **Implementer (either)** | PR1 full; author does not merge |
| **Reviewer** | Dual-agree on PR1; optional implement PR2 |
| **Excluded during PR1** | stat-tracker, map-pipeline, fleet-protocol edits, test-data frames |

IRON LAW: worktrees under `.claude/worktrees/<agent>-trust-p0` only.

---

## 5. Verification matrix

| Check | How |
|-------|-----|
| Unit | `cargo test -p scuffed-db -p scuffed-types` relevant modules |
| API | `cargo test -p scuffed-site-server --test api_integration` filtered scrims/audit/strategy if present |
| Browser | Helium + browser-use: re-run the 12-check script; all P0 markers must flip green |
| Fleet | Post REVIEW REQUEST + APPROVE on `fleet::trust-plan` + pointer on `fleet::chat` |

---

## 6. Risks / constraints

- **Membership policy:** do not change CAS / last-admin / application transition order (CLAUDE.md).
- **Audit append-only:** enrich on **read**, do not rewrite historical rows.
- **`$token` Surreal reserved:** never bind `$token`.
- **Index staleness:** re-index `scuffed-crew` after PR1 if using Memtrace impact for review.
- **Relay offline:** not a bug if forum backend is local — improve copy only in P2/P3.

---

## 7. Ask Claude

**Please APPROVE or DISSENT this plan** on `fleet::trust-plan` (detail) and a short pointer on `fleet::chat`.

Particularly review:
1. PR1 root-cause fixes (audit `actor_name`, strategy `data` vs `strategies`, scrims `limit` 400).
2. Phasing (P0 code before content/nav).
3. Whether games bare-array should become `CursorResponse` (server) vs client dual-decode.

Dissent on correctness blocks; approach disagreements use §5b.

---

## 8. Session evidence pointers

- Browser confirm summary: 12/12 checks PASS (2026-07-23 session)
- API: audit keys missing `actor_name`; strategy body uses `data`; scrims `?limit=100` → 400
- Code: `audit_log.rs` (app), `audit.rs` (types), `audit_log.rs` (db), `scrims.rs` (app+site-server), `strategy/browse.rs` + `server/routes/strategy.rs`, `hooks/api.rs` `use_api_list`
