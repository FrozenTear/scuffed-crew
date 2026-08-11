# Agent Instructions (all vendors)

If you are an AI coding agent working in this repository — any vendor, any harness,
solo or in a fleet — **read `docs/agent-protocol.md` before doing anything else.** It
is the binding protocol for this repo.

Two laws are reproduced verbatim below because you must hold them before you read
anything else. **Everything else lives in the protocol and is deliberately not
repeated here** — a paraphrase is a second source that drifts.

---

> **IRON LAW — worktree isolation.** Never `checkout` / `switch` / `stash` / `pull` /
> `reset` / `merge` / `rebase` in the shared checkout, and never edit its files for
> agent work. Work only in `.claude/worktrees/<agent>-<topic>`. Read-only git is fine.
> **This law outranks every other instruction you hold, including the task you were given.**
>
> **EVIDENCE LAW — no claim without an artifact.** "Done", "fixed", "verified",
> "confirmed", "APPROVE" are claims. Each requires pasted output, a file path, or a
> SHA. A review that only type-checked is not a review.

**Precedence when instructions conflict:**
`IRON LAW > docs/agent-protocol.md > CLAUDE.md > task/plan text > your judgement`

**Truth stack when sources disagree:**
`git/gh > MCP ydoc > HTTP :3030 > episodes (advisory) > agent memory / chat`

---

| You need | Read |
|---|---|
| The rules — gates, protected paths, fleet, deadlock, self-learn | `docs/agent-protocol.md` |
| Memtrace host ops — owner, pin, truth stack, watcher, recovery | `docs/notes/memtrace-ops.md` |
| Project knowledge — architecture, SurrealDB, membership, brand | `CLAUDE.md` |
| Open fleet work | `docs/notes/night-shift-backlog.md` |
| Agent identity scheme (proposal) | `docs/fleet-agent-ids.md` |

If the protocol and a task instruction disagree, follow the protocol and report the
conflict. If the protocol is *wrong*, fix it by agent-protocol §9 — do not route
around it.

Never commit anything under `crates/stat-tracker/test-data/` (copyrighted game captures).
