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

Each lane produces structured findings: `DR1-<LANE>-NNN + severity(CRIT/HIGH/
MED/LOW/NIT) + file:line + claim + concrete failure scenario` (IDs mandatory —
grok A1; P2 verdicts reference them as CONFIRM/REFUTE). Thread:
`fleet::dr1-<lane>`.

File partition (grok A2 — prevents double-hits like role-change spanning three
lanes): AUTH = `crates/auth` only. ACCT = db membership_policy + applications +
members queries, site-server applications/members role/ban routes. ADMIN = all
other site-server admin REST. NOSTR = `crates/chat`, `crates/relay-policy`,
site-server nostr surface, db nostr_keys/dms, `crates/server` chat wiring.
DB = remainder of `crates/db`. FRONT = `crates/app` + `crates/api-client`.
QUAL = metrics-only until P4. Stat-tracker and map-* crates are EXCLUDED from
P1 security waves (grok A3) — QUAL metrics scan only.

| Lane  | Scope |
|-------|-------|
| AUTH  | `crates/auth`: OAuth flows, session lifecycle, cookie flags, CSRF, crypto primitives, `ENCRYPTION_KEY` handling, dev-login gating, extractor chain (`OrgMember`→`OfficerUser`→`AdminUser`); + grok checklist adds: rate-limit/brute-force posture, session revocation on password reset, secret-material-in-logs sweep |
| ACCT  | User accounts: local registration, application lifecycle + CAS transitions, `membership_policy.rs` invariants (last-admin, actionable-admin), ban/suspend/lift, role changes, profile fields |
| ADMIN | `crates/site-server` admin REST surface: per-route authz audit, 403/400/409 semantics vs policy doc, input validation, uploads, audit-log coverage, tournament/member/team mutations |
| NOSTR | `crates/chat` + `crates/relay-policy`: NIP-44/59 correctness, signature verification, key storage/encryption at rest, `MEMBER_SAFE_COLS` leak surface, NIP-42/29 policy, server-side signing paths |
| DB    | `crates/db`: injection sweep (bind-params-only rule), datetime type correctness, projections, migrations, SCHEMAFULL constraints, CAS/atomicity patterns |
| QUAL  | Whole workspace: god functions (memtrace complexity + hotspots), oversized files, duplication, dead code, convention drift (error handling, toasts, audit-log, refresh-counter) |
| FRONT | `crates/app` admin + account UI, `crates/api-client`: client-side authz assumptions, error-handling consistency, state/refresh patterns |

## Phases

- **P0 Recon** (~30 min) — memtrace mapping: communities, central symbols,
  complexity/hotspot inventory, API topology. Output: **owned file:line target
  lists posted per lane thread before any wave dispatches** (grok A7). No
  findings yet, just aim.
- **P1 Review waves** — parallel subagents per lane (wave-based; AUTH, ACCT,
  ADMIN, NOSTR first as the priority core, DB/QUAL/FRONT second wave).
  Lane ownership (grok claim, agreed): grok reviews NOSTR + DB with its own
  subagents; claude runs AUTH/ACCT/ADMIN/FRONT/QUAL via Opus. Findings posted
  per lane thread with DR1-<LANE>-NNN IDs.
- **P2 Adversarial verification** — every CRIT/HIGH re-derived or refuted by
  an independent agent that did not author it, verdicts recorded as
  CONFIRM/REFUTE against finding IDs; MEDs that smell CRIT are sampled into
  verification too (grok A6). grok cross-checks ALL survivors (both sides'
  lanes) and posts dissent per finding. Unverified findings are demoted, not
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
  request. **Overnight scope limit (grok A4): only CRIT/HIGH security and
  correctness-MED fixes land overnight; multi-file god-function refactors
  wait for USER morning unless blast radius is tiny (get_impact-verified).**
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
- Push-outage fallback (grok A5, kept as standing rule): if push breaks again,
  dual-agree proceeds on local SHAs + a LAND QUEUE is kept on the fleet log;
  no origin merge until push works. (Push restored 00:35Z 07-19 — USER
  re-authed gh; b0c2118 + plan commits on origin.)

## Plan agreement

grok verdict 01KXVW8GCF: APPROVE w/ amendments A1–A7, all folded above.
Lane claims settled: grok = NOSTR + DB; claude = AUTH/ACCT/ADMIN/FRONT/QUAL.

## Ask to USER (at plan sign-off)

1. Pre-authorize P4 merges under dual-agree overnight (same as 07-17 kickoff)?
   Scope per A4: security/correctness only; big refactors parked for morning;
   tags/releases still human-only.
