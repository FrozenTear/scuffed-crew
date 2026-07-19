# DR1-ACCT-003 adversarial verification

## VERDICT: CONFIRMED-LOWER (CRIT framing REFUTED)

The endpoint is genuinely unauthenticated and coupled to a racy count — both true — but
it is NOT an unauthenticated-attacker-driven privilege escalation. The attacker has no
lever to reach the required precondition (zero actionable admins). CRIT is refuted;
true severity LOW–MED (defense-in-depth design coupling).

## What must be true for setup to create an admin

- Route registered unconditionally in prod: `crates/site-server/src/lib.rs:62`
  `.route("/api/auth/setup", post(routes::auth::setup))` — no dev-mode guard.
- Handler `setup` (`auth.rs:454`) takes NO auth extractor. Its only gate:
  `auth.rs:459` `match state.db.has_admin_member().await { Ok(true) => 403, Ok(false) => proceed }`.
- `has_admin_member` (`users.rs:221-223`) = `count_actionable_admins().await? > 0`.
  Gated on the LIVE actionable-admin count, NOT on zero-member-rows and NOT on a
  monotonic bootstrap flag. This part of the finding is CONFIRMED.

## Is the count racy / can an attacker drive it to transient zero?

- Count IS two non-transactional round-trips: `members.rs:579-583` (active admins, set A)
  then `members.rs:592-600` (blocked member_ids, set B); subtract in Rust
  (`members.rs:604-608`). Non-atomic — CONFIRMED (this is ACCT-002).
- BUT a torn snapshot only produces +/-1 skew vs a concurrent ban/lift. For the count to
  read 0 it needs set A empty (early return `members.rs:584-586`) OR every admin in A
  covered by blocked set B. q2 reads only committed block rows — a torn read cannot
  INVENT blocks. So the count can read 0 only when the org is truly at/near zero
  actionable admins; it can NOT spuriously read 0 while >=2 healthy admins exist.
- The attacker cannot move the count toward zero at all. Every count-reducing op is
  behind an Officer/Admin extractor AND a last-admin guard:
  - demote: `can_change_role` blocks last admin (`membership_policy.rs:164`)
  - deactivate: `can_set_is_active` blocks last admin + blocks self-flip
    (`membership_policy.rs:189, 209-218`)
  - ban/suspend: `can_suspend_or_ban_admin` + post-write `assert_has_actionable_admin`
    with compensation/reactivation (`moderation.rs:94-107, 133-160, 186-199`).
  The task's "time it against an admin self-demotion" premise fails twice: self active-flip
  is forbidden, and the attacker cannot trigger any admin's destructive action.

## Reachability in production

The 0-actionable-admin precondition is reachable only via:
(a) INTENTIONAL full lockout — all admins banned/suspended/deactivated by authorized
    actors. Reopening setup here is BY DESIGN (lockout recovery), documented at
    `users.rs:219-220`. Self-inflicted, not attacker-induced.
(b) The separate ACCT-002 over-count race letting a guard wrongly permit removing the
    last admin — requires concurrent destructive ops by real admins/officers AND the
    ACCT-002 bug firing past the assert+compensate backstops (`moderation.rs:133`,
    `members.rs:304, 543`). Still needs privileged actors, not the attacker.

## True severity

LOW–MED defense-in-depth issue: setup should be gated on a monotonic "an admin has ever
existed" flag / zero-member-rows rather than the recoverable live actionable count, so a
genuinely locked-out org's recovery window can't be hijacked by an unauthenticated party.
There is NO call sequence by which an unauthenticated attacker escalates against an org
that has a healthy admin. The CRIT (attacker-driven prod escalation) is refuted; the
residual risk is opportunistic seizure of an already-self-locked-out org.
