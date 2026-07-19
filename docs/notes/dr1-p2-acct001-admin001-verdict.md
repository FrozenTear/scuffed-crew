# DR1 P2 — Adversarial verification (READ-ONLY)
Reviewer: claude (Opus) · Date: 2026-07-19 · Did NOT author findings · Default = demote unless a concrete reachable failure traces.

---

## DR1-ACCT-001 — VERDICT: CONFIRMED-LOWER (mechanism real & reachable; severity MED, not HIGH)

### (a) Ordering: side-effect-then-CAS — CONFIRMED
`apply_application_transition` (crates/site-server/src/routes/applications.rs:242-331):
- L254-301: side effects run FIRST.
  - ensure-member branch (L254-256) for Trial/Accepted, OR
  - deactivate branch (L257-301): `get_member_by_user` (read, L258-262) → guard `member.is_active && member.org_role == Recruit` (L264-265) → `update_member(is_active=false)` (L267-284) → `delete_sessions_for_user` (L285) → audit.
- L303-310: the atomic CAS `update_application_status(existing.id, existing.status, to, …)` runs AFTER, and on `DbError::Conflict` returns `conflict(&msg)` (409) with NO compensation.
Confirmed: `application_status_deactivates_member` = {Rejected, Withdrawn} and `_ensures_member` = {Trial, Accepted} (membership_policy.rs:80-93).

### (b) CAS-Conflict returns without compensating — CONFIRMED
L307-309: `Conflict => conflict(&msg)`. No reactivation, no session re-issue, no undo of the `is_active=false`. The `?` propagates the 409 immediately.

### (c) Concrete reachable interleave — CONFIRMED (one valid path; another is REFUTED by the role guard)

Each HTTP request is a separate tokio task; nothing serializes two transitions on the same application (no lock/mutex/DB transaction spanning side-effect+CAS). There IS an await boundary between the guard-read (get_member_by_user) and the write (update_member) in the deactivate branch, so tasks interleave there.

WORKING lockout path — **direct accept (Pending→Accepted) racing self-withdraw (Pending→Withdrawn)**:
1. Accept task: `ensure_member` creates member as **Recruit**, is_active=true (role_on_application_accept(Pending)=Recruit, membership_policy.rs:75; create at applications.rs:406-413).
2. Withdraw task: deactivate branch reads member = {active, Recruit} → guard passes (await-suspends before the update).
3. Accept task: CAS Pending→Accepted → SUCCEEDS.
4. Withdraw task: `update_member(is_active=false)` commits + `delete_sessions_for_user` runs.
5. Withdraw task: CAS Pending→Withdrawn → expected=Pending, actual=Accepted → Conflict → 409, no rollback.
Final state: **application=Accepted, member=Recruit is_active=false, all sessions revoked** → accepted member locked out.

A Trial→Accepted vs Trial→Withdrawn race is largely SELF-REFUTED: accept promotes Recruit→Member (role_on_application_accept(Trial)=Member, applications.rs:377-387), and the withdraw deactivate only fires for `org_role==Recruit`. For the is_active=false to be the final write it must occur after accept's ensure_member, but by then the role is Member and the guard skips — so most Trial interleavings do NOT lock out (a narrow read-before/write-after-promotion window can still hit it, but the Pending path above is the clean one).

Also note the finding's *literal* cited scenario (pure Pending applicant with no member yet) does NOT trigger: `get_member_by_user` returns None for a Pending applicant with no member row, so the deactivate branch is skipped. The lockout requires a member to EXIST as an active Recruit at the read — achieved by the concurrent direct-accept creating it (path above) or a prior Trial provision.

### Severity argument (HIGH → MED)
Real, reachable, no attacker needed — but: (1) accidental race requiring two conflicting transitions (officer-accept vs applicant-withdraw, or two officers) to interleave within a sub-millisecond await window on the SAME application; (2) blast radius = ONE just-accepted member; (3) fully admin-recoverable (re-activate member, user re-logs in); (4) no privilege escalation, no data loss, no attacker leverage or control over timing. The side-effect-before-CAS-without-compensation is a genuine correctness bug worth fixing (do CAS first, then side effects; or compensate on Conflict), but the impact profile is MED, not HIGH.

---

## DR1-ADMIN-001 — VERDICT: CONFIRMED-LOWER (all sub-claims true; severity MED, not HIGH)

### (a) No rate limit / quota on upload routes — CONFIRMED
lib.rs: `GovernorLayer` is built once (L51-58) and `.layer()`-attached ONLY to the `auth_routes` sub-router (L76), which is then `.merge()`d (L94). A tower layer on a sub-router wraps only that sub-router's services. Upload routes are added directly to the MAIN router — `/api/upload/avatar` (L451) and `/api/upload/image` (L452) — and the only bottom-level layers on the main router are `DefaultBodyLimit::max(6MB)` (L500), `TraceLayer` (L501), `cors` (L502). No governor, no per-user quota anywhere. Uploads are unthrottled. CONFIRMED (this is also ADMIN-012).

### (b) No cleanup of replaced files — CONFIRMED
`save_upload` (uploads.rs:70-101) generates `format!("{}.{ext}", Uuid::new_v4())` (L95) and `fs::write`s a NEW file every call (L98). Nothing reads/deletes a prior avatar; the handlers (routes/uploads.rs:23-140) only call save_upload and return the URL. No reaper exists. Files grow unbounded even in normal avatar-change use. CONFIRMED.

### (c) Size cap & attacker pool — CONFIRMED with important mitigation
- Per-request caps: avatar `AVATAR_MAX_BYTES = 2MB` (routes/uploads.rs:14), image `5MB` (L15), enforced at save_upload L77-79; global body limit 6MB (lib.rs:500). So each avatar POST writes ≤2MB.
- Attacker pool is NOT anonymous. `upload_avatar` requires `OrgMember` (routes/uploads.rs:25); the extractor (extractors.rs:35-87) requires an existing member row via `get_member_auth_by_user` (403 "Not an org member" if none, L59-66), plus `is_active` (L68) and not-suspended (L77). A member row is created only when an officer moves an application to Trial/Accepted (applications.rs ensure_member) — **registration alone does NOT grant OrgMember**. So the attacker must be an officer-approved recruit+ insider. `upload_image` is stricter (OfficerUser).

### Severity argument (HIGH → MED)
Every technical sub-claim is TRUE (no throttle, no cleanup, no quota, unbounded growth), and looping 2MB writes to fill disk/inodes on a shared host (DB+SPA) is a real outage vector. Downgrade rationale: the actor must be a vetted, officer-approved member (not anonymous, small pool), every request is bound to a session→user so the abuse is fully ATTRIBUTABLE, and remediation is immediate (deactivate/suspend the member → OrgMember extractor then 403s them). This is an authenticated-insider resource-abuse gap, not an unauthenticated internet-facing DoS. The no-GC storage leak is worth fixing on its own (delete-on-replace + quota + a member-scoped rate limit). Impact profile = MED.

---

### One-line summary
- DR1-ACCT-001: CONFIRMED-LOWER (MED) — side-effect-before-CAS with no Conflict compensation is real and a concrete accept/withdraw interleave locks out one just-accepted member, but it's a rare accidental single-user admin-recoverable race, not HIGH.
- DR1-ADMIN-001: CONFIRMED-LOWER (MED) — no rate limit + no delete-on-replace + no quota all verified, but exploitation requires an officer-approved, attributable, easily-revocable member insider, so MED not HIGH.
