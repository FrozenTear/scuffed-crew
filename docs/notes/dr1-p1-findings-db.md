# DR-1 P1 Findings — DB lane (grok)

Repo: scuffed-crew · Author: grok · Date: 2026-07-19  
Anchor: origin/main@0ebb6c2 · Plan: docs/plans/2026-07-19-dr1-deep-review.md  
Partition (A2): DB = remainder of `crates/db` after ACCT carve-out  
**EXCLUDED (ACCT):** members.rs role/ban CAS, applications, membership_policy  
**EXCLUDED (NOSTR content):** nostr_keys/dms content crypto (shared client/rewrap noted only)

IDs: `DR1-DB-NNN` · severity CRIT/HIGH/MED/LOW/NIT  
P2: independent agent must CONFIRM/REFUTE each CRIT/HIGH (and sampled MEDs).

---

## Summary

| Sev | Count | IDs |
|-----|------:|-----|
| CRIT | 0 | — |
| HIGH | 1 | DB-001 |
| MED | 5 | DB-002, DB-003, DB-004, DB-005, DB-006 |
| LOW | 3 | DB-007, DB-008, DB-009 |
| NIT | 2 | DB-010, DB-011 |
| Clean / refuted aims | — | strategies dynamic WHERE (bound), personal_stats season filter (static), sessions/daemon hash-at-rest, poll UNIQUE index for same-option double-vote, remote PRODUCTION+ENCRYPTION_KEY gate |

**TOP:** DR1-DB-001 HIGH — `report_tournament_match` accepts arbitrary `winner_id` (ADMIN-004 handoff).  
Officer-gated, but still bracket poison + wrong advancement.

---

## Findings

### DR1-DB-001 HIGH CONFIRMED (from ADMIN-004)

- **Where:** `crates/db/src/queries/tournaments.rs:721-756` `Database::report_tournament_match`; call site `site-server/src/routes/tournaments.rs:576-600` `report_match` (OfficerUser).
- **Claim:** DB writes `winner_id` from caller with **no check** that winner ∈ {participant_a_id, participant_b_id}. Route then `set_match_participant(next, slot, winner_id)` and picks "loser" by comparing winner to a/b — if winner is neither, loser logic picks a arbitrarily and advances a **third** id into the bracket.
- **Failure scenario:** Malicious or mistaken officer posts `winner_id` of an unrelated participant (or typo id). Match completes with poison winner; next-round slot filled with that id; single-elim eliminate path may mark wrong participant eliminated.
- **Also:** No guard against re-reporting an already-`completed` match (last write wins; bracket already advanced once → double advance risk on re-report).
- **Fix:** In DB (preferred): reject unless `winner_id` equals a or b (after normalizing empty/bye); reject if status already completed (or explicit reopen path). Optionally assert scores consistent with winner. Route should still fail closed.
- **P2:** re-derive with concurrent double-report + foreign winner_id fixtures.

### DR1-DB-002 MED

- **Where:** `site-server/.../tournaments.rs:576-590` uses Path `(tournament_id, mid)` but only passes `mid` into `report_tournament_match`; DB never checks `match.tournament_id == path id`.
- **Claim:** Knowing any match id, an officer can report it under a **different** tournament path. Response still advances that match's next pointers (global match row), so path id is cosmetic for authz/audit confusion.
- **Failure:** Cross-tournament URL confuses audit/UI; soft integrity hole if clients trust path tournament for side effects.
- **Fix:** After load, require `reported.tournament_id == id` else 404/400.
- **Note:** Boundary with ADMIN route; root data check belongs in DB or shared helper.

### DR1-DB-003 MED

- **Where:** `db/.../polls.rs:123-144` `vote_poll`; `site-server/.../polls.rs:132-168`.
- **Claim:** UNIQUE index is `(poll_id, member_id, option_index)` only. DB does not enforce `poll.allow_multiple` or `is_active`. Route checks option bounds but **not** `allow_multiple` and **not** active flag before create.
- **Failure:** Single-choice poll: member votes option 0 then option 1 → two rows, both counted. Closed/`is_active=false` poll still accepts votes if id known.
- **Fix:** DB: load poll; if !is_active → err; if !allow_multiple → delete other options for member or reject if any other vote exists (transaction). Map unique violation → 409 not 500.

### DR1-DB-004 MED

- **Where:** `db/.../tournaments.rs:721-756` + route advancement block.
- **Claim:** Match report is read-modify-write **without CAS** on `status`/`updated` version. Two officers concurrent report → both read pending, both write completed, both run `set_match_participant` (second clobber / double side effects).
- **Failure:** Rare officer race corrupts next-match slots.
- **Fix:** Conditional update `WHERE status = 'pending'` (or version field); only advance on rows_updated==1.

### DR1-DB-005 MED

- **Where:** `db/src/client.rs:305` `ensure_database_app_user` — `DEFINE USER OVERWRITE {username} ON DATABASE PASSWORD '{pass}'` after `assert_safe_sql_ident(username)` + `escape_surreal_string(password)`.
- **Claim:** Password is still SQL-string-interpolated (Surreal limitation noted in comments). Escape only handles `\` and `'`. Env-controlled, not HTTP-user — residual risk if password contains unescaped control chars or Surreal parser quirks; also **OVERWRITE resets password every boot** to env (ops footgun if two replicas disagree on env briefly).
- **Failure:** Mis-set `SURREALDB_APP_PASSWORD` with exotic chars → DEFINE fail or unexpected parse; multi-instance password thrash.
- **Fix:** Prefer Surreal API that binds secrets if/when available; document forbidden charset; consider DEFINE only when user missing unless `FORCE_APP_PASSWORD_SYNC=1`.

### DR1-DB-006 MED

- **Where:** `db/.../users.rs:80-100` `get_user_by_provider` / local username lookup — `SELECT * FROM user`.
- **Claim:** Full user row including `password_hash` and encrypted provider blobs loaded for every provider lookup. Not a remote leak by itself (stays in process) but expands memory/log footguns and violates least-privilege projection rule from P0 focus.
- **Failure:** Future debug `{:?}` or error path dumps hash; larger attack surface on memory disclosure.
- **Fix:** Explicit column lists; never SELECT password_hash except verify_password path.

### DR1-DB-007 LOW

- **Where:** `db/.../audit_log.rs` — only `create` + list/count. Migrations: no permissions denying UPDATE/DELETE for EDITOR app user.
- **Claim:** App code is append-only, but **schema does not enforce** immutability. Compromised app credentials can `DELETE/UPDATE audit_log`.
- **Fix:** Surreal permissions or separate audit role; app deny-list tests.

### DR1-DB-008 LOW

- **Where:** `db/.../wiki.rs` content update — `UPDATE ... SET content_markdown` last-write-wins; revisions recorded but no compare-and-swap on `updated_at`/revision id.
- **Claim:** Concurrent editors silently drop one write (revision history keeps both bodies only if both UPDATEs ran after separate revision inserts — race still loses one page tip).
- **Fix:** Optional `IF updated_at = $prev` CAS; 409 on conflict.

### DR1-DB-009 LOW

- **Where:** `db/.../tournaments.rs` `report_tournament_match` does not validate `score_a`/`score_b` vs winner (e.g. winner a with score_a < score_b).
- **Claim:** Bracket stands on winner_id only; inconsistent scores confuse standings/UI.
- **Fix:** Require strict inequality matching winner slot (ties policy explicit).

### DR1-DB-010 NIT

- **Where:** `strategies.rs:286-340` dynamic `where_clause` via `format!`.
- **Claim:** **Not injection** — conditions are static fragments; user values only via `$game_mode` / `$search` binds. Matches CLAUDE.md intent.
- **Note:** Keep pattern; do not "simplify" by interpolating search text.

### DR1-DB-011 NIT

- **Where:** `personal_stats.rs:337` season filter string is compile-time static `""` or fixed AND clause + binds.
- **Claim:** Clean. Same for sessions token hash, daemon_token hash, poll unique same-option, `assert_remote_production_policy`, load_crypto PRODUCTION gate.

---

## Cross-lane handoffs

| ID | To | Note |
|----|-----|------|
| DB-001/002/004 | ADMIN / P4 | Tournament report cluster — one fix branch |
| DB-003 | ADMIN polls route + DB | allow_multiple / is_active |
| DB-006 | AUTH | password_hash projection |
| ACCT-002/004 | ACCT owns | non-atomic admin count / change_member_role no CAS — not re-litigated here |
| NOSTR CRIT-001 | P2 + P4 | already escalated; claude wt `fix/dr1-nostr-challenge-secret` locked |

---

## P2 candidates (priority)

1. DB-001 (HIGH) — foreign winner + re-report + concurrent  
2. DB-003 — single-choice multi-option votes  
3. DB-004 — concurrent report CAS  
4. Sample DB-002 tournament_id mismatch  

## Clean notes (brief)

- Inject sweep: user-influenced values generally `$bind`; strategies/personal_stats format! only static SQL shape.  
- Daemon/session secrets stored hashed (BLAKE3 via `hash_session_token`).  
- Rewrap covers users provider_id_encrypted, members nostr secret, dm messages.  
- Poll same-option double-vote blocked by UNIQUE index (error mapping still 500 — NIT under DB-003 fix).
