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
4. **Dual-channel law (USER 2026-07-19):** reviews/approvals/ACKs/MERGED may land
   on the ops thread (`fleet::chat`), on the per-initiative thread
   (`fleet::<initiative>`), or both. **Poll BOTH surfaces every tick** — never
   conclude "no verdict" from one quiet channel. When **posting** a review or
   dual-agree close, put detail on the initiative **and** a short pointer on
   `fleet::chat` the same turn (branch@sha + verdict + tip). Chat-only closes are
   allowed only until an initiative thread exists. (fleet-protocol §4, §6)
5. **Git/GitHub outranks the fleet log.** After any restart, re-derive state
   from git/gh.
6. **Protocol self-learn (USER 2026-07-19):** when a session finds a durable
   process gap, unblock locally, then draft the portable fix in a worktree
   against `docs/fleet-protocol.md` (and host ops when relevant); peer dual-agree
   before the push is binding — the author never sole-merges protocol/ops, and a
   harness-local skill patch is **not** a substitute for the git protocol other
   vendors load from the repo. (fleet-protocol §8)
7. **Never hard-code a home/host path in fleet docs or state.** Resolve the repo
   root dynamically (`git rev-parse --show-toplevel`); worktrees live under
   `.claude/worktrees/<agent>-<topic>` relative to it. (DOC-HP1 host-path scrub)
8. Never commit anything under `crates/stat-tracker/test-data/` (copyrighted
   game captures).

Project conventions (build, dev-mode, DB rules): see `CLAUDE.md`.
Queued work for fleet sessions: `docs/notes/night-shift-backlog.md`.
Memtrace/fleet host ops (start, MCP attach, truth stack, watcher, recovery):
`docs/notes/memtrace-ops.md` — load on every fleet join alongside the protocol.
