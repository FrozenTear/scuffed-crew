# Fleet Protocol — Multi-Agent Collaboration over Memtrace

A portable protocol for running two or more independent AI coding agents
(different vendors welcome — e.g. Claude + Grok) against one repository,
coordinated through a Memtrace fleet, with a human holding the final gate.

The core protocol (§1–§7) is project-agnostic. Project-specific bindings live in
**Appendix A**. Origin: authored from the overwatch-strategy-app "Great Review"
session (2026-07-17, 10 PRs, two daemon restarts survived) and battle-tested the
same day on scuffed-crew (v0.1.0 ship). Canonical shared copy:
claude.ai artifact `3f9c0a32-ecb7-49dc-824e-afd8d283d911`; this file is the
operational copy for this repo — if they diverge, this file wins here.

---

## 0. Principles

1. **No single agent decides.** Every finding needs the other agent's CONFIRM or
   REFUTE; every merge needs every agent's APPROVE. Cross-model agreement is a
   real quality signal — agreement between different vendors validates,
   disagreement escalates.
2. **The human holds the final gate.** Dual-approval qualifies a change; only the
   human lands it (or delegates landing with an explicit, scoped greenlight).
   Judge the *diff and end state*, not whether the agent followed a prescribed
   sequence.
3. **Git/GitHub is the source of truth. The fleet log is a convenience.** Durable
   artifacts (commits, PRs, CI runs) outrank anything in the coordination log.
   After any infra restart, re-derive state from git/gh, never from memory.
4. **Agents never touch the shared checkout.** All work happens in per-agent
   worktrees. This is the IRON LAW (§3).
5. **Coordinate through the blackboard, not DMs.** Agents write to the shared
   ydoc threads; nobody depends on point-to-point messages.

## 1. Session start — joining the fleet

Every agent, on joining:

1. `fleet_branch_context` — agent id, live peer intents, pending escalations,
   recent peer episodes for your branch.
2. `fleet_status` — confirm coordination is active.
3. `fleet_ydoc_read` (whole repo thread) — read the existing charter/scoreboard.
   **Read via MCP, not the dashboard HTTP API** — HTTP views reset after daemon
   restarts and can wedge into empty replies while MCP stays truthful. An HTTP
   `count=0` is NOT proof of a wipe; verify via MCP before reseeding.
4. Post a join message (`fleet_ydoc_append`, kind `intent`): who you are,
   model/vendor, what you're claiming.
5. Publish a presence intent (`fleet_publish_intent`). Intents TTL at ~120 s —
   re-publish each loop tick, or hold `fleet_acquire_lease`/`fleet_renew_lease`
   for long operations.

If the ydoc is genuinely empty (fresh fleet or verified wipe), the first agent
posts a **charter**: thread names, partition, consensus rule, worktree law,
message-size rule, and a scoreboard reseeded from git/gh.

## 2. Work partition and claims

- **Partition by file/module ownership.** Two agents must never edit the same
  file in parallel. Overlapping claims block until renegotiated.
- **Claims carry scope AND negative boundaries** — say what is *excluded* as
  well as included. Most duplicated agent work traces to thin task specs.
- Before editing, `fleet_publish_intent` with the actual qualified symbols and a
  typed intent plus a natural-language `assignment` — the assignment is what a
  mediator reads if two agents collide.
- After each edit, `fleet_record_episode`. Conflict classes: **A** additive
  (proceed), **B** touched-set overlap (re-read before proceeding), **C**
  destructive overlap (defer; escalation opens). For risky refactors, take an
  exclusive lease first.
- **Scale agent count to the task** — sequential is correct when two work items
  share a file.

## 3. The IRON LAW — worktree isolation

Agents on a shared machine work ONLY in their own git worktrees. Real work has
been lost to shared-checkout branch switches (twice nearly, 2026-07-17).

1. NEVER `git checkout`/`switch`/`stash`/`pull`/`reset`/`merge`/`rebase` in the
   shared main checkout. It is READ-ONLY for agents (`status`/`diff`/`log`/
   `show` are fine).
2. NEVER edit files in the shared working tree for fleet work — its dirty state
   may be the human's or a peer's WIP.
3. All implementation lives in
   `git worktree add .claude/worktrees/<agent>-<topic> -b <branch> origin/main`.
   Remove the worktree after pushing; the branch ref persists.
4. Review remote branches with `gh pr diff` / `git diff origin/main...origin/<br>`
   / `git show origin/<br>:<path>` — never by checking them out.
5. Prefer merging main into a feature branch over rebase when updating an open
   PR — no history rewrite, no force-push.
6. Worktrees isolate *files*, not *runtime* — dev servers, databases, ports are
   shared. If multiple agents run the app, assign per-agent ports/instances.

Exception: the *human-driven* agent may operate in the primary checkout at the
human's direction, announcing any branch switch on the log FIRST.

## 4. Communication — ydoc conventions

- **Threads:** one chat/ops thread (`fleet::chat`) + one findings/decisions
  thread per initiative (`fleet::<initiative>`). `kind`: `intent` for
  claims/charters, `edit` for findings/status, `resolution` for locked
  decisions.
- **Bodies ≤ ~400 chars / ~6 lines.** Long content goes in files, PRs, or
  commits — the log entry carries the pointer (SHA, PR #, path). A log message
  is a *checkpoint with a pointer to the artifact*, not the artifact.
- **Structured findings:** `id / file:line / severity / one-line summary /
  concrete failure scenario / confidence`. Findings get ids (F<pr>-N, FST-N…)
  so CONFIRM/REFUTE replies are unambiguous.
- **Verify your appends landed** (re-read) — appends can silently fail.
- Never rewrite history — post corrections as new entries.
- A verdict must state what was READ, what was RUN vs merely compiled, every nit
  including non-blocking, and what was deliberately NOT checked. (A 40-second
  cargo-check APPROVE is not a review.)

## 5. Consensus pipeline

```
claim (intent) → implement (worktree) → push/PR → peer review (other agent)
→ findings dual-CONFIRMed / nits resolved → CI green on current head
→ dual-APPROVE on the log → HUMAN merge call → land → record episode
```

- **Reviewer rubric:** scope matches the agreed plan exactly; no protected paths
  touched; tests discriminate the new behavior from the old (mutation-test when
  cheap — a green test that cannot fail guards nothing); CI green on the
  **current head SHA**.
- **Cap review rounds at ~2**, then escalate to the human.
- **Class C conflicts:** both agents `fleet_submit_verdict`; human resolves via
  the escalation queue if agents don't safely converge.
- Disagreement between agents is not failure — it's the mechanism. Both
  directions of correction have happened in every session to date.
- The author never merges their own branch; the reviewer merges after ACK-back,
  or the human does.

## 6. Liveness — watchers, presence, backoff

**Push watchers (primary wake signal):** peer-message poll (dedupe by entry
ULID, pre-seed the seen-set), CI poll to terminal state (always emit on failure
— silence must never look like success), long-job watch by run id.

**Heartbeat loop (fallback + presence):** each tick re-publish presence, verify
the watcher's data source is healthy (a wedged endpoint makes a push watcher
silently blind — the 07-17 SSE endpoint returned HTTP 200 and zero bytes for 40
minutes), process missed items, act.

- **Backoff ladder:** base 3 min → 3 quiet ticks → 5 min → 5 more → 10 min.
  ANY activity resets to base. Add jitter (±20–30 s). Pin to base while any
  watcher is blind.
- Carry loop state in the loop prompt itself so a context reset doesn't lose
  the ladder position.

**Restart recovery:** ydoc entries may survive (memdb: 2/2 on 07-17), live
intents and HTTP views do not. On restart: rejoin (§1), verify via MCP, reseed
from git/gh, reset cadence, re-arm watchers. Both agents independently reseeding
from git/gh and matching is itself a correctness check.

## 7. Delegation inside one agent (orchestrator + subagents)

- Subagent briefs are self-contained: repo, worktree setup, the IRON LAW
  verbatim, exact finding(s) with file:line, test expectations, return format.
- Implementers work in a named worktree, run the full repo gates (not just
  compile), push, clean up. Reviewers are read-only.
- Verify subagent claims that matter before relaying them to the fleet as fact.
- Evaluate by end state, but require deviations to be REPORTED.

---

## Appendix A — Project bindings (scuffed-crew)

| Binding | Value |
|---|---|
| Memtrace repo_id | `scuffed-crew` |
| Shared checkout (READ-ONLY for agents; human-driven agent exception §3) | `/home/soot/github/scuffed-crew` |
| Worktree root | `.claude/worktrees/<agent>-<topic>` |
| Ydoc threads | legacy monolith `fleet::channel` (history); NEW work: `fleet::chat` + `fleet::<initiative>` |
| SSE `/api/fleet/events` | **known-dead** (HTTP 200, zero bytes) — poll ydoc via MCP |
| Ydoc durability | memdb survived 2/2 daemon restarts 07-17; the 13:32Z 07-17 wipe remains undiagnosed — reseed from git/gh regardless |
| Review gate = CI exactly | `cargo fmt --check` + `bash scripts/check-design-tokens.sh` + workspace clippy (excl. app/tracker) + `cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings` + `cargo test` |
| Release gate | tag `stat-tracker-v*` builds fresh from tagged ref; verify the *published* one-liner from raw main, not a branch tree |
| Protected paths | `crates/stat-tracker/test-data/` (copyrighted frames — never commit, see its .gitignore) |
| Known agents | `claude` (Fable orchestrator, Opus subagents), `grok` (grok-build) |
| Backlog | `docs/notes/night-shift-backlog.md` |

## Appendix B — Provenance

Distilled from two 2026-07-17 sessions: overwatch-strategy-app "Great Review"
(10 PRs #78–#87, protocol authored) and scuffed-crew v0.1.0 ship day (protocol
lessons re-derived independently under fire: worktree hazard, dead watcher,
stale-read corrections, vacuous-test catch, release-gate bootstrap bug). Full
source list with citations lives in the canonical artifact.
