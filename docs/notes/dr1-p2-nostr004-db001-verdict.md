# DR1 P2 Adversarial Verification — NOSTR-004 & DB-001

Verifier: claude (independent re-derive of grok findings). Read-only. Repo anchor: main HEAD.

---

## DR1-NOSTR-004 — VERDICT: CONFIRMED, severity LOWERED to MED-latent (grok claimed HIGH)

### (a) Default + dead-code wiring gap — CONFIRMED
- `crates/relay-policy/src/policy.rs:79` — `PolicyConfig::default()` sets `enforce_group_membership: false`. CONFIRMED verbatim.
- `crates/relay-policy/src/policy.rs:131-137` — `update_group_members` carries `#[allow(dead_code)]` with doc comment: "the binary does not yet load group membership from the DB (NIP-29 group enforcement is not wired into `main`)." CONFIRMED.
- `crates/relay-policy/src/main.rs:129-137` — config built from env: `enforce_group_membership: env_or("RELAY_POLICY_ENFORCE_GROUPS","false")=="true"`. Main loads ONLY the pubkey allowlist (`update_allowlist`, main.rs:157 + refresh loop 168-186). `update_group_members` is NEVER called anywhere outside tests. CONFIRMED.
- Consequence: the feature is half-wired, not merely off. With `RELAY_POLICY_ENFORCE_GROUPS=false` (default) → any allowlisted pubkey may write any `h`-tag group event (policy.rs:167-181 skips the block). With `RELAY_POLICY_ENFORCE_GROUPS=true` → `group_members` map is empty, so EVERY group event hits the "unknown group" reject (policy.rs:176-178) → group chat entirely broken. So there is no working configuration for group authz. CONFIRMED and slightly worse than grok stated.

### (b) CRUCIAL CONTEXT — is relay-policy deployed/reachable? — LATENT, not live
- relay-policy IS built and wired, contrary to a naive "dead code" read: `relay/Containerfile` compiles `-p relay-policy` and `relay/strfry.conf:34-38` chains it as a strfry write-policy plugin: `plugin = "/usr/local/bin/strfry29 /usr/local/bin/relay-policy"`.
- BUT the entire relay stack is opt-in: `compose.yml` `strfry` service carries `profiles: ["relay"]` (line ~31). It does NOT start on a default `podman compose up` — only with `--profile relay`. Matches CLAUDE.md ("standalone, not yet in deploy (future work)") and deploy specs ("Relay (strfry): Optional Compose profile; not required for core site").
- Additional upstream mitigation: `strfry29` (fiatjaf/relay29) runs FIRST in the plugin chain and is a purpose-built NIP-29 relay that does its own group management/authz before relay-policy sees the event. So even under the relay profile, group membership is not solely relay-policy's responsibility.
- Therefore this is NOT a live production-reachable HIGH. It is a real but latent gap in an opt-in, not-yet-shipped component → **MED-latent**.

### (c) Group-write authz gap on its own terms — CONFIRMED
- Independent of deploy: relay-policy's own group-write authorization is non-functional (off by default; unusable when on). The design intent (per-group membership enforcement) is not achievable with current wiring. CONFIRMED on its own terms.

### Fix scope (do as part of relay-deploy hardening, NOT overnight)
1. Wire `update_group_members` from DB in `main` initial load + refresh loop (needs a DB query mapping group_id → member pubkeys; does not exist yet).
2. Only after (1): make enforcement the production default (or keep env gate but document that enabling it without the DB load bricks group chat — better to couple them).
3. Unknown-group reject path (policy.rs:176-178) already correct.
Blocked on there being a group→member source in DB; this is genuinely future work, aligned with A12 in docs/security-quality-review-fix-list.md ("relay-policy gift-wrap allowlist — Before relay deploy"). No overnight action.

---

## DR1-DB-001 — VERDICT: CONFIRMED, severity LOWERED to MED (grok/ADMIN-004 claimed HIGH)

### (a) No winner-membership validation — CONFIRMED
- `crates/db/src/queries/tournaments.rs:721-756` `report_tournament_match`: selects the row, then blindly `db.winner_id = Some(winner_id.to_string())` (line 738) and `db.status = "completed"` (739). No check that `winner_id ∈ {participant_a_id, participant_b_id}`. CONFIRMED.
- `crates/site-server/src/routes/tournaments.rs:576-594` `report_match` (OfficerUser-gated): passes `&body.winner_id` straight through from the request body. No validation. CONFIRMED.

### (b) Bracket advance mis-behaves with foreign winner — CONFIRMED
- Winner advance: routes/tournaments.rs:597-603 — `set_match_participant(next_id, next_slot, &body.winner_id)` writes the foreign id directly into the next match slot. A third, unrelated participant id is injected into the next round. CONFIRMED.
- Loser pick (double-elim 610-614 and single-elim 626-630): `if reported.participant_a_id == Some(winner_id) { loser = b } else { loser = a }`. If winner is NEITHER a nor b, the `else` branch fires and picks participant_a as the "loser" → wrong participant is advanced to the losers bracket / marked Eliminated (single-elim, 632-645). CONFIRMED — both the winner and loser paths corrupt.

### (c) Re-report / double-advance — CONFIRMED
- `report_tournament_match` (721-756) is unconditional read-modify-write: no `WHERE status = 'pending'` guard, sets status="completed" every call. Re-invoking on an already-completed match re-runs the entire advance block in the route → the next-round slots get overwritten again and single-elim elimination re-applied. No idempotency / no CAS. CONFIRMED. (This is also grok DB-004's concurrency variant — same missing CAS.)

### (d) Severity weigh — HIGH → MED
- Gated behind `OfficerUser` (officer+), a trusted role. No auth bypass, no secret exposure, no cross-tenant leak. Blast radius is confined to tournament bracket integrity (a non-critical feature), and in normal UI flow winner_id comes from a dropdown of the two participants, so accidental trigger is unlikely.
- Real exposure requires a malicious/compromised officer or a hand-crafted request from one. That is a meaningful but bounded threat. Honest severity: **MED** (integrity/correctness, trusted-role-gated), down from HIGH. The bug is genuine and the fix is cheap.

### Fix scope (small, contained)
In `report_tournament_match` (DB layer, preferred single choke point):
1. Guard: after loading the row, reject unless `winner_id == participant_a_id || winner_id == participant_b_id` (normalize empty/bye first) → return a DbError (route maps to 400).
2. Status CAS: reject if `status == "completed"` already (or gate behind an explicit reopen path); ideally do the update as `UPDATE ... WHERE status = 'pending'` and only advance when `rows_updated == 1` — closes both re-report (c) and the concurrent-officer race (DB-004) in one move.
3. Optional: assert score/winner consistency (DB-009).
Route (`report_match`) should still fail closed on the DB error rather than advancing.

### Overnight vs morning
**Morning.** Officer-gated, tournaments are not a live hot path, no secret/auth surface. Deserves proper fixtures (foreign winner_id + concurrent double-report) and peer review rather than a rushed overnight patch. Bundle DB-001 + DB-004 (CAS) + optionally DB-002/DB-009 into one tournament-report fix branch.
