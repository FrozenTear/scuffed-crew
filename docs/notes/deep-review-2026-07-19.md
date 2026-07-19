# Deep Review DR-1 — Admin Backend & User Accounts (2026-07-19)

Orchestrator: claude (Fable) · NOSTR/DB lanes: grok · Implementers: Opus subagents
Plan: `docs/plans/2026-07-19-dr1-deep-review.md` · Anchor: main@db18605

All findings passed adversarial P2 verification (independent agent re-derives every
CRIT/HIGH, prompted to refute). Severities below are POST-verification. grok's
concurrence on the four demotions of its own HIGHs is pending (§5b severity window).

## Headline

**No anonymous-attacker critical exists.** One real HIGH survives verification
(`NOSTR-001`), and it is bounded to a MITM-class capture requirement. Every other
HIGH from the first pass demoted under verification. The "stupid bug" that bit the
team today (role-change 400) was already fixed upstream. The rest is a tidy backlog
of latent-concurrency correctness MEDs, defense-in-depth MEDs, one systemic
frontend error-UX gap, and a QUAL refactor list.

## Severity roll-up (verified)

| Sev | Count | IDs |
|-----|------:|-----|
| CRIT | 0 | — |
| HIGH | 1 | NOSTR-001 |
| MED  | ~15 | AUTH-001/003/004, ACCT-001/002/003/004, ADMIN-001/003/006, NOSTR-003/005/006/007/011, DB-002/003/004/005/006, FRONT-001/002/003 |
| LOW/NIT | many | see per-lane files |
| Refuted / already-fixed | — | ACCT-008 (fixed @0b38871), NOSTR-002→LOW, recon "rate-limit absent" + "Secure cookie off" + "compute_stats forked" all false |

Per-lane detail: `docs/notes/dr1-p1-*-findings.md` (claude) and, from grok's branch
`origin/docs/dr1-p0-grok`, `dr1-p1-findings-nostr.md` + `dr1-p1-findings-db.md`.
P2 verdicts: `docs/notes/dr1-p2-*-verdict.md`.

## The one HIGH — NOSTR-001

Deterministic Nostr challenge-token MAC key when `NOSTR_CHALLENGE_SECRET` is unset;
`install.sh` never provisions it; production fails open. Direct member_id forgery is
blocked by the caller-binding guard (`nostr.rs:266`); residual risk is login-event
replay (stateless challenge verify + no `created_at` freshness in nostr 0.44.2), but
the login event is never relayed so capture is MITM-class → HIGH not CRIT. Full
writeup + operator mitigation: `docs/notes/DR1-CRIT-NOSTR-001-ESCALATION.md`.
**Fix branch `fix/dr1-nostr-challenge-secret` in preparation; grok dual-agree required.**

## Fix backlog → branch plan

### Overnight (A4: HIGH-security + correctness, dual-agree, reviewer merges)
- **NOSTR-001** → `fix/dr1-nostr-challenge-secret` (fail-closed + provision secret).
  **LANDED main@af363a8** (merge of e559413; grok APPROVE 01KXW00M7S, claude landed
  via Hermes-fallback). CI verified on merged head. Follow-up NS-1a (`.trim()`
  whitespace hardening) parked to morning below.

### Morning (USER review — larger blast radius or needs fixtures)
- **Tournament cluster** DB-001 + DB-004 (+ DB-002 path check, DB-009 score check)
  → one branch: winner∈{a,b} guard + `WHERE status='pending'` CAS. Needs foreign-winner
  + concurrent-double-report fixtures.
- **Poll integrity** DB-003 → enforce `is_active` + `allow_multiple`, map unique-violation to 409.
- **Last-admin hardening** ACCT-002 (single-statement count) + ACCT-003 (gate `/setup`
  on a monotonic bootstrap flag, not the live count) + ACCT-004 (CAS on change_member_role).
- **Application transition ordering** ACCT-001 → CAS-first-then-side-effects.
- **Upload hardening** ADMIN-001 (per-member quota + delete-on-replace + rate-limit)
  + ADMIN-003 + audit gaps ADMIN-006 / NOSTR-008.
- **Rate-limit key trust** AUTH-001 (trusted-proxy allowlist) + AUTH-002 (dummy-hash)
  + AUTH-003 (session revoke on bootstrap reset) + AUTH-004 (crypto gate condition).
- **Constant-time compares** NOSTR-002 + CSRF + session (codebase-wide, one batch).
- **Frontend error-UX** FRONT-001 (shared list-error helper across 12 admin pages)
  + FRONT-002 (admin-page self-guards) + FRONT-003 (stop swallowing loader errors).
- **NOSTR polish** NOSTR-003 (import-flow copy + confirm), 005 (backup pw min), 006
  (per-member rate limit on secret routes), 007 (canonical conversation_key), 011 (step-up).
- **NS-1a** (grok NS-1 follow-up): tighten the challenge-secret guard to
  `.trim().is_empty()` so a whitespace-only secret can't boot as a weak key
  (matches the ENCRYPTION_KEY check). One-line, needs a re-review since it
  touches the boot path.
- **NOSTR-001 defense-in-depth** (verifier bonus, grok-agreed follow-up): add an
  event `created_at` freshness window + one-time-challenge store — closes the
  login-replay path even if the MAC key ever leaks. The secret fix is necessary
  but not sufficient long-term.
- **DB hygiene** DB-005 (app-user password sync footgun), DB-006 (password_hash projection),
  DB-007 (audit-log immutability), DB-008 (wiki CAS).

### QUAL refactor backlog (metrics-only until scheduled; A4 = mostly morning)
- Overnight-safe (single-file stat-tracker, A3-isolated): QUAL-001 (`handle_capture`
  extract `analyze_frame`, cc53→~38), QUAL-003 (`main` cc26→~9), QUAL-005, QUAL-007.
- Morning: QUAL-002 (full `handle_capture` seam split →cc15), QUAL-004 (`run_loop`
  struct-ify), god-file splits.
- **P0 false positives — do NOT chase:** `compute_stats` is NOT forked (gui imports the
  shared fn); `detect/*/open()` repetition doesn't exist; `find_hero`/`fuzzy_match` ×4-6
  is an AST artifact.

## Relay-policy (future work, not deployed)
- NOSTR-004 (group-membership enforcement half-wired) → fix before any relay deploy;
  blocked on a group→member DB source that doesn't exist yet. Not production-reachable.

## Process notes
- Recon first-pass over-called three items (crypto downgrade, rate-limit absence,
  compute_stats fork); verification corrected all three. Kept in the record as a
  reminder that first-pass severity is a hypothesis, not a finding.
- Fleet-log ydoc read went stale mid-session (known flakiness); all coordination
  anchored to git commits as source-of-truth per protocol.
