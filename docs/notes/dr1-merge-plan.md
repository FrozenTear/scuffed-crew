# DR-1 Merge Plan — all remaining work (2026-07-19)

Status: **DRAFT for USER approval** · claude proposal, grok input pending.
Landed so far: NOSTR-001 @af363a8, HS-1 hero-timeline @6dad3af.
Source backlog: `docs/notes/deep-review-2026-07-19.md`.

## Principles

1. **Security/correctness before cleanup.** Refactors land last so they don't
   collide with fixes touching the same functions.
2. **One branch per file-cluster.** Findings that touch the same file merge as
   one branch — never parallel branches on the same file (conflict avoidance).
3. **Cross-review, always.** claude authors AUTH/ACCT/ADMIN/FRONT; grok authors
   NOSTR/DB/stat-tracker. The peer reviews. Author never merges own (dual-agree).
   All implementation via Opus subagents (Fable never hand-implements).
4. **Nothing lands without USER approval of this plan first**, then per-branch
   dual-agree. Tags/releases/data-deletion stay human-only.
5. **Fixtures gate the tournament + poll work** — those need failing-case tests
   written before the fix.

## Waves (recommended order)

### Wave 1 — Quick, independent, high-confidence (parallel-safe, no shared files)
| Branch | Findings | Owner→Reviewer | Notes |
|---|---|---|---|
| `fix/dr1-const-time-compares` | NOSTR-002 + CSRF + session MAC/token compares | grok→claude | `subtle::ConstantTimeEq`; codebase-wide, one sweep. Trivial. |
| `fix/dr1-db-projections` | DB-006 (password_hash projection), DB-005 (app-user pw sync footgun) | grok→claude | Explicit column lists; least-privilege. |
| `fix/dr1-audit-gaps` | ADMIN-006 + NOSTR-008 (unaudited mutations, distinct audit actions) | claude→grok | Add missing `audit()` calls + dedicated NostrKey* actions. |
| `fix/dr1-nostr-challenge-hardening` | NS-1a (`.trim()`) + NOSTR-001 created_at freshness + one-time-challenge store | grok→claude | Closes the replay path even if key leaks. Touches nostr.rs/auth.rs. |

### Wave 2 — Correctness clusters (need fixtures; distinct files, parallel-safe)
| Branch | Findings | Owner→Reviewer | Notes |
|---|---|---|---|
| `fix/dr1-tournament-integrity` | DB-001 + DB-004 + DB-002 + DB-009 | grok→claude | winner∈{a,b} + status CAS + tournament_id path check + score consistency. **Fixtures: foreign-winner, concurrent double-report.** All in tournaments.rs → one branch. |
| `fix/dr1-poll-integrity` | DB-003 | grok→claude | Enforce is_active + allow_multiple; unique-violation→409. |
| `fix/dr1-last-admin-hardening` | ACCT-002 + ACCT-003 + ACCT-004 | claude→grok | Single-statement count; gate `/setup` on monotonic bootstrap flag; CAS on change_member_role. Touches members.rs/membership_policy.rs. |
| `fix/dr1-application-ordering` | ACCT-001 | claude→grok | CAS-first-then-side-effects in apply_application_transition. applications.rs. |

### Wave 3 — Broader surface (larger diffs, review-heavy)
| Branch | Findings | Owner→Reviewer | Notes |
|---|---|---|---|
| `fix/dr1-upload-hardening` | ADMIN-001 + ADMIN-003 | claude→grok | Per-member quota + delete-on-replace + rate-limit on upload routes; dimension cap. Adds infra. |
| `fix/dr1-auth-ratelimit-trust` | AUTH-001 + AUTH-002 + AUTH-003 + AUTH-004 | claude→grok | Trusted-proxy allowlist for XFF; dummy-hash on no-user path; session-revoke on bootstrap reset; crypto gate condition. Touches auth.rs/lib.rs. |
| `fix/dr1-front-error-ux` | FRONT-001 + FRONT-002 + FRONT-003 | claude→grok | Shared list-error helper across 12 admin pages; admin-page self-guards; stop swallowing loader errors. Wide but mechanical. |

### Wave 4 — NOSTR polish + DB hygiene (lower severity)
| Branch | Findings | Owner→Reviewer | Notes |
|---|---|---|---|
| `fix/dr1-nostr-polish` | NOSTR-003 (import copy+confirm) + 005 (backup pw min) + 006 (rate-limit secret routes) + 007 (canonical conversation_key) + 011 (step-up) | grok→claude | UX + rate-limit + key-consistency. |
| `fix/dr1-db-hygiene` | DB-007 (audit immutability) + DB-008 (wiki CAS) | grok→claude | Schema permissions + optimistic CAS. |

### Wave 5 — Refactors LAST (highest conflict risk; land after fixes settle)
| Branch | Findings | Owner→Reviewer | Notes |
|---|---|---|---|
| `refactor/qual-handle-capture` | QUAL-001 (extract `analyze_frame`, cc53→~38) then QUAL-002 (full seam split →cc15) | grok→claude | stat-tracker; do AFTER HS-1 + any stat-tracker fixes settle. Behavior-preserving, before/after cc numbers required. |
| `refactor/qual-main-runloop` | QUAL-003 (`main` cc26→~9) + QUAL-004 (`run_loop` struct-ify) | grok→claude | stat-tracker. |
| `feat/hs-1a-gui-resolve` | HS-1a (GUI per-segment confirm/dismiss button) | claude→grok | Wire the already-built command layer to a GUI control. |
| `hardening/hs-1b-segment-key` | HS-1b (segment-index append-only assumption) | grok→claude | Consider timestamp-keyed resolutions if mid-session insert is real. Assess first — may be WONTFIX (captures are monotonic). |

## Open questions for grok (its lanes)
1. Tournament cluster: batch DB-001/002/004/009 as one branch, or split the CAS
   (DB-004) out? My lean: one branch — same file, same fixtures.
2. Refactor timing: land Wave 5 stat-tracker refactors before or after the
   NOSTR/DB fixes? My lean: after (fixes first, refactor a settled tree).
3. HS-1b: real fix or WONTFIX-with-comment? Captures are monotonic in practice.
4. Any Wave 1–4 branch you'd re-order or re-scope?

## Not in this plan (separate tracks)
- Anything requiring a release/tag (human-only).
- The `HS-1a` GUI work depends on GUI test infra — may slip.
