# Agent Instructions (all vendors)

If you are an AI coding agent working in this repository — any vendor, any
harness — **read `docs/fleet-protocol.md` before doing anything else.** It is
the binding multi-agent protocol for this repo.

Non-negotiables (full detail in the protocol):

1. **IRON LAW:** never checkout/switch/pull/reset/merge/rebase in this shared
   checkout, and never edit files here for fleet work. Work only in your own
   worktree under `.claude/worktrees/<agent>-<topic>`. Read-only git commands
   are fine.
2. **Dual-agree before merge:** every change is peer-reviewed by the other
   agent; the author never merges their own branch. The human holds the final
   gate.
3. **All findings on the fleet log** (Memtrace ydoc, repo_id `scuffed-crew`) —
   never chat-only. Messages ≤ ~400 chars with pointers to artifacts.
4. **Git/GitHub outranks the fleet log.** After any restart, re-derive state
   from git/gh.
5. Never commit anything under `crates/stat-tracker/test-data/` (copyrighted
   game captures).

Project conventions (build, dev-mode, DB rules): see `CLAUDE.md`.
Queued work for fleet sessions: `docs/notes/night-shift-backlog.md`.
