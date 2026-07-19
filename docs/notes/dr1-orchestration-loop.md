# DR-1 Overnight Orchestration Loop — resume contract

Claude (Fable) is the DR-1 orchestrator running an autonomous overnight loop
(started 2026-07-19, USER away, push authorized). This file is the durable
resume contract — on any wakeup, read this + git + the fleet log (git wins on
conflict per protocol).

## Every wakeup, do this in order

1. **Reconcile.** Read the newest scratchpad findings files
   (`scratchpad/dr1-p1-*-findings.md`), check which review agents have
   completed (task-notifications), and read the fleet log (`fleet::chat` +
   `fleet::dr1-*`). If the ydoc read is stale (known flakiness), trust git.
2. **Advance the phase machine** (tasks #2–#6):
   - P1: when AUTH/ACCT/ADMIN land → mirror findings to lane threads +
     `docs/notes/`, then dispatch FRONT + QUAL wave-2 Opus agents.
   - P2: for every CRIT/HIGH (and MEDs that smell CRIT), dispatch an
     independent Opus verifier that did NOT author the finding — prompt it to
     REFUTE. Record CONFIRM/REFUTE vs the DR1-<LANE>-NNN id. grok cross-checks
     survivors. Demote unverified, don't drop.
   - P3: once verification converges, write
     `docs/notes/deep-review-2026-07-19.md` (severity-ranked, deduped, each fix
     → branch plan). Commit + push.
   - P4: land ONLY CRIT/HIGH-security + correctness-MED overnight (grok A4).
     One branch per cluster, **Opus implements** (USER security ruling —
     Fable never hand-implements), CI-exact gates (fmt, clippy -D warnings,
     workspace test), dual-agree with grok, reviewer merges. Multi-file
     god-function refactors → PARK for USER morning. Tags/releases human-only.
   - P5: CI green on final main, `get_evolution` regression sweep, finalize
     report, update memory, write USER morning summary to `fleet::chat`.
3. **Escalate CRIT immediately** — the moment a CRIT is CONFIRMED, write it to
   `fleet::chat` + a dedicated `docs/notes/DR1-CRIT-<id>.md` commit so USER
   sees it first thing. Do not batch CRITs to P3.
4. **Hermes fallback:** if grok can't push (no auto mode), claude lands the
   branch quoting grok's APPROVE ULID in the merge commit body. Only after
   dual-agree closes on the log.
5. **Re-arm** the loop (ScheduleWakeup) unless all of P1–P5 are done, in which
   case stop the loop and leave the morning summary as the final state.

## Hard floor (never autonomous)
Tags, releases, force-push, data deletion, protected-path edits, policy
overrides = human-only even overnight. HS-1 hero-timeline worktree (dirty,
unpushed) is OUT of DR-1 scope — do not touch, flag for USER morning.

## Pointers
Plan: docs/plans/2026-07-19-dr1-deep-review.md @ a1f23b8.
Targets: docs/notes/dr1-p0-targets-claude.md.
Lane split: claude=AUTH/ACCT/ADMIN/FRONT/QUAL, grok=NOSTR+DB.
Memory: [[dr1-deep-review]], [[fleet-coordination]], [[cross-review-protocol]],
[[fleet-channel-is-source-of-truth]].
