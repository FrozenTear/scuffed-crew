# DR-1: Deep Review — Admin Backend, User Accounts, and Codebase Health

Date: 2026-07-19 · Orchestrator: claude (Fable) · Plan reviewer: grok · Sponsor: USER

## Mandate (USER, verbatim intent)

Big, hours-scale review covering most of the codebase, centered on the **admin
backend and user accounts** — design, security, cryptography, nostr. Recent
"stupid bugs" (e.g. role-change 400) suggest correctness gaps. Consistency
issues, big files, and god functions are in scope **to be fixed, not just
listed**. Grok is included in the review process. Implementation is done by
subagents only; on the Fable side implementers must be **Opus** (USER security
ruling 2026-07-19, extends orchestration ruling in fleet-protocol Appendix A).

## Roles

- **claude (Fable)** — orchestrates, plans, synthesizes, dispatches. Does not
  hand-implement. All claude-side implementation and heavy file-reading runs
  through Opus subagents.
- **grok** — reviews this plan (dissent is expected input), cross-checks
  verified findings in P2, reviews + merges every claude-authored fix branch
  (dual-agree, author never merges own). May claim review lanes (suggest:
  NOSTR and DB) if capacity allows — claim on the lane thread.
- **USER** — plan sign-off tonight; CRIT findings escalated immediately;
  tags/releases/data-deletion stay human-only (§5b hard floor).

## Lanes

Each lane produces structured findings: `severity(CRIT/HIGH/MED/LOW/NIT) +
file:line + claim + concrete failure scenario`. Thread: `fleet::dr1-<lane>`.

| Lane  | Scope |
|-------|-------|
| AUTH  | `crates/auth`: OAuth flows, session lifecycle, cookie flags, CSRF, crypto primitives, `ENCRYPTION_KEY` handling, dev-login gating, extractor chain (`OrgMember`→`OfficerUser`→`AdminUser`) |
| ACCT  | User accounts: local registration, application lifecycle + CAS transitions, `membership_policy.rs` invariants (last-admin, actionable-admin), ban/suspend/lift, role changes, profile fields |
| ADMIN | `crates/site-server` admin REST surface: per-route authz audit, 403/400/409 semantics vs policy doc, input validation, uploads, audit-log coverage, tournament/member/team mutations |
| NOSTR | `crates/chat` + `crates/relay-policy`: NIP-44/59 correctness, signature verification, key storage/encryption at rest, `MEMBER_SAFE_COLS` leak surface, NIP-42/29 policy, server-side signing paths |
| DB    | `crates/db`: injection sweep (bind-params-only rule), datetime type correctness, projections, migrations, SCHEMAFULL constraints, CAS/atomicity patterns |
| QUAL  | Whole workspace: god functions (memtrace complexity + hotspots), oversized files, duplication, dead code, convention drift (error handling, toasts, audit-log, refresh-counter) |
| FRONT | `crates/app` admin + account UI, `crates/api-client`: client-side authz assumptions, error-handling consistency, state/refresh patterns |

## Phases

- **P0 Recon** (~30 min) — memtrace mapping: communities, central symbols,
  complexity/hotspot inventory, API topology. Output: per-lane target lists
  posted to lane threads. No findings yet, just aim.
- **P1 Review waves** — parallel Opus subagents per lane (wave-based; AUTH,
  ACCT, ADMIN, NOSTR first as the priority core, DB/QUAL/FRONT second wave).
  Findings posted per lane thread.
- **P2 Adversarial verification** — every CRIT/HIGH re-derived or refuted by
  an independent agent that did not author it; grok cross-checks the surviving
  set and posts dissent per finding. Unverified findings are demoted, not
  silently dropped.
- **P3 Consolidated report** — `docs/notes/deep-review-2026-07-19.md`:
  severity-ranked, deduplicated, each fix mapped to a branch plan. USER-facing
  summary posted to `fleet::chat`.
- **P4 Fixes** — priority order: CRIT/HIGH security → correctness MED →
  god-function/consistency refactors (QUAL fixes land as focused refactor
  branches with before/after complexity numbers). One branch per fix cluster,
  Opus implements, gates = CI-exact (fmt, clippy -D warnings, workspace
  tests). Every branch dual-agree; reviewer merges. Refactors must be
  behavior-preserving; any behavior change is called out in the review
  request.
- **P5 Close-out** — full CI green on final main, `get_evolution` regression
  sweep, report finalized, memory + backlog updated, USER morning summary.

## Protocol bindings

- Fleet log is source of truth for findings/verdicts; git outranks after
  restarts. Threads: `fleet::dr1-plan`, `fleet::dr1-<lane>`, heartbeats on
  `fleet::chat` at each phase transition.
- §5b deadlock rules apply to P4 disagreements: correctness objections block;
  approach disputes → measure with fixtures/tests → Fable provisional +
  dissent recorded → USER reviews provisionals first.
- CRIT security findings: escalate to USER immediately on discovery, do not
  batch to P3.
- Push auth is currently broken on this box (gh token invalid). Until USER
  re-auths, branches stay local + all coordination goes through the fleet
  log with explicit "local-only" markers. No merge happens from a cached
  remote view.

## Ask to USER (at plan sign-off)

1. Pre-authorize P4 merges under dual-agree overnight (same as 07-17 kickoff),
   tags/releases still human-only?
2. Re-auth `gh` so branches/report can reach origin.
