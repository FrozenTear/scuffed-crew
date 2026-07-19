# DR1 P2 — ACCT-002 family adversarial verdict

Reviewer: claude (Opus), verifying findings NOT self-authored. Date 2026-07-19. READ-ONLY.

## Bottom line
The bug that bit the team today = **ACCT-008 (same-role save → 400)**, and it is
**ALREADY-FIXED** on main (commit 0b38871, merged via PR #4/#5, ancestor of main).
ACCT-002's non-atomic count is REAL but is NOT today's bug and does not fire in
single-user / low-concurrency use. ACCT-004 (no CAS) is real but latent.

---

## ACCT-008 — ALREADY-FIXED (user-facing) · residual = LOW
Claim: same-role change returns 400 "Member already has this role" instead of no-op.
- Server logic still 400s: `membership_policy.rs:155-157` unchanged on main (verified
  `git show main:...` — line 156 still returns `BadRequest`). So the finding's server
  claim is CONFIRMED at the policy layer.
- BUT the reported user symptom ("save Change Role dialog without changing selection →
  spurious 400") is exactly what commit **0b38871** fixed, **client-side**:
  `crates/app/src/pages/admin/members.rs:181-184` now short-circuits and closes the modal
  when `role_value() == member.org_role`, so the same-role PATCH is never sent.
- Residual: server non-idempotency only reachable now via a **stale-UI edge** (client
  holds an out-of-date member row, so its equality guard compares against a stale role and
  still sends). Genuinely LOW, not the today-bug.
- **VERDICT: ALREADY-FIXED (today-bug). Do NOT re-fix.** Optional hardening only: make the
  policy treat same-role as idempotent 200 no-op instead of 400. Not required.

## ACCT-004 — CONFIRMED · MED (latent, not today's bug)
Claim: `change_member_role` is a plain UPDATE with no CAS/expected guard; compensation
writes a stale role.
- `members.rs:532-548`: `UPDATE $rid SET org_role = $role RETURN AFTER` — no WHERE guard.
  CONFIRMED, verbatim.
- Route compensation `members.rs:545`: `change_member_role(&id, target.org_role)` where
  `target` was read at handler start (`get_member_safe`, line 463) — a stale snapshot.
  CONFIRMED stale-compensation / lost-update.
- Real-world trigger requires **two concurrent role changes on the same target member** —
  effectively zero for a small-org admin panel. No persistent zero-admin corruption
  constructible (assert backstop self-heals DB-happy path). Lying-success responses are the
  real (minor) impact. **VERDICT: CONFIRMED, MED, latent.**

## ACCT-002 — CONFIRMED non-atomic · today-bug linkage REFUTED · practical MED
Claim (HIGH): `count_actionable_admins` is a two-query torn snapshot feeding every
last-admin guard + the assert backstop → spurious 403/409 and possible zero-admin lockout.
- `members.rs:573-612`: two separate `.query()` round-trips (active admins; then blocked
  moderation rows) with the subtract done in a Rust `HashSet`, no transaction/snapshot
  between them. CONFIRMED non-atomic / torn snapshot.
- **REFUTE the linkage to today's "role-change 400".** A single admin clicking Save cannot
  torn-read its own count; the false-403/409 requires a concurrent ban/lift/role mutation to
  land in the microsecond window between the two queries. The user-visible 400 they saw was
  ACCT-008 (logic), not this race. The task's own hint ("today bug was likely NOT
  concurrent") holds.
- Over-count → zero-admin lockout (which would also re-open the unauthenticated
  `/api/auth/setup` gate via `users.rs:222 has_admin_member`, ACCT-003): **SUSPECTED**, no
  precise interleave reproduced, low probability. **VERDICT: CONFIRMED non-atomic; severity
  MED in practice (HIGH only if the zero-admin interleave is ever demonstrated); today-bug
  linkage REFUTED.**

---

## Fix scoping (for what remains genuinely broken)

### ACCT-002 (recommended first) — collapse to one SurrealQL statement
Single statement, e.g. count active admins whose id is NOT IN the active-block subquery:
```
SELECT count() FROM member
WHERE is_active = true AND org_role = 'admin'
  AND id NOT IN (
    SELECT VALUE member_id FROM moderation_action
    WHERE is_active = true AND action_type IN ['suspension','ban']
      AND (expires_at IS NONE OR expires_at > time::now())
  )
GROUP ALL
```
- **Behavior-preserving**: same `u64` return, same signature — **no caller changes**.
- Blast radius (read-only consumers, all unaffected): `users.rs:222` (has_admin_member →
  setup gate), `members.rs:208` (deactivate guard), `members.rs:485` (role guard),
  `moderation.rs:82` (ban/suspend guard), `assert_has_actionable_admin` (members.rs:616).
- Test coverage exists: `api_integration.rs` asserts counts 0/1 at 2246/2359/2595/2903 —
  regression net already present. Note: `member_id` is stored as a **string** key; verify
  `id NOT IN (subquery)` matches on RecordId vs string (may need `record::id(id)` or store
  comparison) — the one thing to validate before landing.

### ACCT-004 — CAS on the role write
`UPDATE $rid SET org_role=$new WHERE org_role=$expected RETURN AFTER`; 0 rows → Conflict.
- **Signature change** (`expected` param) ripples to 3 call sites: `members.rs:531` (main),
  `members.rs:545` (compensation — expected = just-written value), `applications.rs:384`
  (accept-provisioning, where the member may have just been created/reactivated, so
  "expected" semantics need thought). Plus 2 test call sites. Higher blast radius, not
  behavior-preserving.

## Overnight vs morning
- **ACCT-002 rewrite**: low-risk and behavior-preserving, BUT it hardens a security-sensitive
  invariant (last-admin + the unauthenticated setup gate, ACCT-003). Safe to **draft
  overnight**, but land with **morning peer review** + the RecordId-vs-string subquery check
  and a concurrency test. Correctness-MED; A4-eligible only after the id-match is verified.
- **ACCT-004 CAS**: signature ripple into the accept-provisioning path → **morning**, needs
  deliberate design of `expected` on the applications.rs call site.
- **ACCT-008**: nothing to land — already fixed.
