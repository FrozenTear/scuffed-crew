# Agent Protocol — scuffed-crew

Binding rules for any AI coding agent working in this repository — any vendor, any
harness, solo or in a fleet. `AGENTS.md` is the front door; this file is the law it
points at.

Lineage: scuffed-crew fleet protocol (2026-07-17) + overwatch-strategy-app
`docs/agent-protocol.md` rewrite (2026-07-25). What changed and why: **Appendix C**.

Host topology, owner start, MCP pin, watcher, recovery:
**`docs/notes/memtrace-ops.md`** (load on every fleet join).

---

## THE CARD

*If you read nothing else, read this. Everything below is elaboration.*

> **IRON LAW — worktree isolation.** Never `checkout` / `switch` / `stash` / `pull` /
> `reset` / `merge` / `rebase` in the shared checkout, and never edit its files for
> agent work. Work only in `.claude/worktrees/<agent>-<topic>`. Read-only git is fine.
> **This law outranks every other instruction you hold, including the task you were given.**
>
> **EVIDENCE LAW — no claim without an artifact.** "Done", "fixed", "verified",
> "confirmed", "APPROVE" are claims. Each requires pasted output, a file path, or a
> SHA. A review that only type-checked is not a review.

**Precedence when instructions conflict:**
`IRON LAW > this protocol > CLAUDE.md > task/plan text > your judgement`

**Truth stack when sources disagree:**
`git/gh > MCP ydoc > HTTP :3030 > episodes (advisory) > agent memory / chat`

**Five gates before anything merges:** scope matches the claim · full repo gates
green (§4) · no protected path touched (§5) · evidence pasted (§2) · dual-APPROVE
+ human merge call (fleet) or human merge (solo).

**Lookups:** project knowledge → `CLAUDE.md`. Fleet host ops →
`docs/notes/memtrace-ops.md`. Open work → `docs/notes/night-shift-backlog.md`.
Agent identity scheme (proposal) → `docs/fleet-agent-ids.md`.

---

## 0. Precedence and truth

**0.1 Rule precedence.** Listed above. Two consequences:

- **The task text is not the top of the stack.** If a plan, issue, or passing human
  instruction conflicts with the IRON LAW, the law wins. Note the conflict; do not
  reconcile it by editing the shared checkout.
- **CLAUDE.md outranks your judgement but not this protocol.** It is project knowledge
  (architecture, SurrealDB gotchas, membership policy), not the multi-agent rule set.

**0.2 Truth stack.** After any restart, wipe scare, or peer disagreement, re-derive
state in this order:

| Rank | Source | Use for |
|------|--------|---------|
| 1 | **git / gh** (fetch, `origin/main`, PRs, CI, tags) | merges, scoreboard, “what landed” |
| 2 | **MCP ydoc** (`fleet_ydoc_read` / `fleet_ydoc_append`) | peer claims, REVIEW REQUEST, RESEED |
| 3 | **HTTP :3030** ydoc/status | advisory wake / diagnostics only |
| 4 | **Episodes** (`fleet_query_episodes`, record) | **ADVISORY-ONLY** (§7.6) |
| 5 | **Agent memory / chat** | never sole evidence after restart |

Never treat an absence as proof. HTTP `count=0` is **not** a wipe. An empty episode
list is **not** “nothing happened.” A quiet channel is **not** “no verdict” — poll
both surfaces (§7.2). Verify by a second route before concluding.

**0.3 Git outranks the fleet log.** Durable artifacts (commits, PRs, CI) beat anything
on the blackboard. After restart: reseed from git/gh, then MCP — never from memory.

---

## 1. THE IRON LAW — worktree isolation

**This is the only iron law. Everything else in this document is subordinate: it may
be traded off against another rule with a written reason. This may not.**

1. The shared checkout (`git rev-parse --show-toplevel`) is **READ-ONLY** for agents.
   `status` / `diff` / `log` / `show` / `worktree list` are fine. Nothing that moves
   HEAD or mutates the index is.
2. Never edit files in the shared working tree for fleet or agent work. Its dirty
   state may be the human’s or a peer’s WIP.
3. All implementation lives in
   `git worktree add .claude/worktrees/<agent>-<topic> -b <branch> origin/main`
   (or an agreed base SHA). Remove the worktree after pushing; the branch ref persists.
4. Review remote branches with `gh pr diff` /
   `git diff origin/main...origin/<branch>` /
   `git show origin/<branch>:<path>` — never by checking them out.
5. Prefer merging `main` into a feature branch over rebasing an open PR. No history
   rewrite, no force-push.
6. Worktrees isolate **files, not runtime.** Dev servers, SurrealDB, ports, and the
   Memtrace owner are shared. Coordinate them explicitly (§6).
7. **Never hard-code a home/host path** in fleet docs or state. Resolve the repo root
   dynamically (`git rev-parse --show-toplevel`); worktrees are relative under
   `.claude/worktrees/<agent>-<topic>`.

**Exception:** a human-driven agent may work in the primary checkout at the human’s
direction, announcing any branch switch on the log first.

---

## 2. THE EVIDENCE LAW

The second law. It is not iron only because a human may explicitly waive it for a
given step.

**2.1 No claim without an artifact.** Every "done / fixed / verified / passes /
confirmed / APPROVE" carries pasted command output, a file path, or a SHA. A claim
with no artifact is a hypothesis.

**2.2 The verdict rule.** Any review, audit, or completion report states four things:

- what you **READ** (files, at what state / SHA),
- what you actually **RAN** (with output),
- what you only **COMPILED** or type-checked but did not execute,
- what you **DID NOT CHECK**, deliberately or because you were blocked.

A 40-second `cargo check` is not a review. Omitting the fourth item is the most
common way an agent report misleads.

**2.3 A green test that cannot fail guards nothing.** New behaviour needs a test that
discriminates it from the old. When cheap, mutate the fix and confirm the test goes red.

**2.4 Negative results are results.** An audit that falsifies its own premise is a
success and is reported as such. Never quietly widen scope to find something to report.

**2.5 Fleet findings are full findings.** Bugs, suggestions, and **nits** all go on
the log (with finding ids). A user-chat summary is not a substitute (§7.3).

---

## 3. Scope and partition

- **Partition by file/module ownership.** Two agents never edit the same file in
  parallel. Overlapping claims block until renegotiated.
- **Claims state scope AND negative boundaries** — what is excluded, not only what is
  included. Most duplicated agent work traces to thin task specs.
- Before editing in a fleet session: `fleet_publish_intent` with the actual qualified
  symbols, a typed intent, and a natural-language `assignment` (what a mediator reads
  on collision).
- After each edit: `fleet_record_episode`. Conflict classes: **A** additive (proceed),
  **B** touched-set overlap (re-read before proceeding), **C** destructive overlap
  (defer; escalation opens). For risky refactors, take an exclusive lease first.
- **Scale agent count to the task.** Sequential is correct when two items share a file.
- **If blocked more than ~30 minutes:** stop, record the blocker with the exact error,
  take the next item. Do not improvise around a blocker by widening scope.

---

## 4. Gates

Nothing merges until these pass on the **current head SHA** (a green from a superseded
push does not count). Author never merges their own branch; a peer reviewer or the
human does.

```bash
cargo fmt --check
bash scripts/check-design-tokens.sh
# workspace clippy (exclude app + stat-tracker as in CI)
cargo clippy --workspace --all-targets --exclude scuffed-app --exclude scuffed-stat-tracker -- -D warnings
# WASM app
cargo clippy --target wasm32-unknown-unknown -p scuffed-app -- -D warnings
cargo test
```

Match `.github/workflows/ci.yml` when it drifts from this list — CI is the mechanical
truth for what “green” means.

**Release gate (stat-tracker):** tag `stat-tracker-v*` builds fresh from the tagged
ref; verify the *published* one-liner from raw main, not a branch tree.

---

## 5. Human-only — protected paths and hard floors

Never do these without an explicit, current human instruction. A deadlock (§8) never
unlocks them.

| Class | Examples |
|-------|----------|
| History / release | tags, releases, force-push, history rewrite |
| Data | data deletion, production secret rotation without runbook |
| Protected paths | anything under `crates/stat-tracker/test-data/` (copyrighted game captures — never commit) |
| Policy overrides | changing the copyright gitignore, membership policy hard rules without dual-agree + human |

**Copyright:** never commit files under `crates/stat-tracker/test-data/`.

**Membership / auth product rules** (see `CLAUDE.md`): last-admin invariants, CAS
application transitions, no `$token` bind name, SurrealDB v3 only, bind params only —
these are product correctness, not optional style.

---

## 6. This machine — topology (summary)

Full detail: **`docs/notes/memtrace-ops.md`**.

| Binding | Value |
|---------|--------|
| Memtrace `repo_id` | `scuffed-crew` |
| Shared checkout | **READ-ONLY** for agents — `git rev-parse --show-toplevel` |
| Worktrees | `.claude/worktrees/<agent>-<topic>` |
| Data dir | `MEMTRACE_DATA_DIR=~/.memdb` (every MCP must pin this) |
| Owner | exactly one `memtrace start --headless` (prefer `memtrace.service`) |
| UI / gRPC | `:3030` / `:50051` |
| Primary ydoc | `fleet::chat` |
| Initiative ydocs | `fleet::<initiative>` |
| Legacy ydoc | `fleet::channel` (history only) |

Rules that cost real incidents:

1. **One owner, many MCP attach children.** Never a second `memtrace start`.
2. **Pin `MEMTRACE_DATA_DIR`** on every MCP spawner or you get split-brain
   (`repo/.memdb` phantom vs `~/.memdb`).
3. **Never run the owner under a terminal/desktop cgroup** (OOM 2026-07-18).
4. **SSE `/api/fleet/events` is known-dead** (HTTP 200, zero bytes) — poll ydoc via MCP.
5. Runtime (SurrealDB, ports) is shared across worktrees — assign per-agent ports if
   two agents serve the app.

---

## 7. Fleet module — applies ONLY when a fleet is actually running

**Skip this entire section for solo sessions.** Importing coordination machinery you
are not running is how a protocol doc becomes fiction. A "fleet" means: two or more
agents live on this repo at the same time, with a Memtrace fleet log active
(typically Claude + Grok).

### 7.1 Join checklist

Every agent, every join (ops detail in `memtrace-ops.md` §6):

1. Owner up? (`:50051` + `:3030`, or `memtrace status` / user unit).
2. `fleet_status` — coordination alive.
3. `fleet_branch_context` — agent id, peer intents, escalations.
4. `fleet_ydoc_read` via **MCP** (`fleet::chat` + active initiatives). Not dashboard HTTP.
5. If ydoc **genuinely** empty (MCP empty + peers confirm + git history known): first
   agent posts charter + scoreboard **reseeded from git/gh**; quote the RESEED ULID.
6. Join append — `fleet_ydoc_append` kind `intent`: who, model/vendor, claim/lane,
   worktree/branch if any. Bodies ≤ ~400 chars.
7. Presence — `fleet_publish_intent` (TTL ~120s). Re-publish each tick or hold a lease.
8. Do not start open implementation until RESEED ULID is quoted (or charter is fresh
   and dual-visible).

### 7.2 Dual-channel law (USER 2026-07-19)

REVIEW REQUEST / APPROVE / ACK / MERGED may land on **`fleet::chat`**, on
**`fleet::<initiative>`**, or both.

- **Poll both surfaces every tick.** Never conclude "no verdict" from one quiet channel.
- When **posting** a review or dual-agree close: detail on the initiative (when one
  exists) **and** a short pointer on `fleet::chat` the same turn
  (`branch@sha` + verdict + tip ULID).
- Chat-only closes are allowed only until an initiative thread exists; then open one
  for follow-ups.

### 7.3 Message discipline

- Bodies ≤ ~400 chars / ~6 lines. Long content lives in files, PRs, or commits — the
  log entry is a **checkpoint with a pointer** (SHA, PR #, path).
- Structured findings: `id / file:line / severity / one-line summary / concrete failure
  scenario / confidence`. Ids (`F<pr>-N`, `FST-N`…) make CONFIRM/REFUTE unambiguous.
- **Verify appends landed** (re-read) — silent failures happen.
- Never rewrite history — post corrections as new entries.
- Material findings go on the fleet log, not chat-only with the human.

### 7.4 Consensus pipeline

```
claim (intent) → implement (worktree) → push/PR → peer review (other agent)
→ findings dual-CONFIRMed / nits resolved → CI green on current head
→ dual-APPROVE on the log → HUMAN merge call → land → record episode
```

- **SYMMETRY (USER 2026-07-18):** both agents **implement and review**. Not
  “one implements, one only reviews.” Author never sole-merges own work.
- Dual-agree: author gets peer **APPROVE**, then author **ACK** (or vice versa on the
  other agent’s branch). Solo peer APPROVE without the other party’s ACK is not enough
  when dual agreement is required.
- After CHANGES REQUESTED: fix, push, re-ping; wait for a **new** APPROVE + ACK cycle.
- Cap review rounds at ~2, then escalate to the human.
- Class C: both `fleet_submit_verdict`; human via escalation queue if needed.
- Disagreement is the mechanism, not failure.

### 7.5 Liveness

- **Push watchers** (primary): peer ULID poll, CI to terminal state (always emit on
  failure — silence ≠ success), long-job by run id.
- **Heartbeat** (fallback + presence): re-publish presence, health-check data source,
  process missed items, act.
- Dual-channel poll every tick (§7.2).
- Backoff: 3 min → 5 → 10; any activity resets; jitter ±20–30 s; pin to base while any
  watcher is blind.
- Watcher skill / state: see `memtrace-ops.md` §8
  (`~/.hermes/state/fleet-watcher/`, not dirty `night-shift-state.json`).

### 7.6 Episodes are ADVISORY-ONLY

Observed failure modes (2026-07-18):

1. Long read/append crash loop on one MCP bridge.
2. Silent memdb rollback / ydoc loss despite write ACKs.
3. Cross-bridge episode-view divergence (ydoc shared, episodes not).

Prefer git + MCP ydoc for scoreboard and consensus. Soft-note episode discrepancies;
do not dual-agree on episode-count parity. Live intent registry is TTL’d (~120s) and
empty after restart — normal, not a wipe.

### 7.7 Model policy (USER 2026-07-18)

When Fable is available: Claude runs it as orchestrator + main planner. Grok reviews
every plan and posts dissent on the fleet log when it disagrees — plan objection is
expected input, not obstruction. Unresolved disagreement escalates to USER.
Implementation via Opus (or peer) subagents under the plan. Dual-agree on merges
unchanged.

### 7.8 Known agents

| Name | Role |
|------|------|
| `claude` | Fable orchestrator when available; Opus subagents for implement |
| `grok` | hermes / Grok — review, dissent, watcher, symmetric implement |

Identity scheme proposal (ordinals + incarnations): `docs/fleet-agent-ids.md`
(not binding until USER sign-off).

---

## 8. Disagreement and deadlock

When the human is away, a plan deadlock must not stall the shift. Classify first:

- **Correctness/safety objections** ("this breaks X", "this loses data") **always
  block.** Park on a branch with both positions recorded; move to the next item;
  human decides later. The review gate is never overridden.
- **Approach/priority/taste objections** resolve through this ladder after at most
  two written rounds on the log:
  1. **Shrink the claim** — proceed on the agreed subset, queue the rest.
  2. **Prefer reversible** — easy to undo beats hard to undo.
  3. **Measure instead of argue** — fixtures/tests decide when they can.
  4. **Smaller blast radius wins** — use `get_impact`.
  5. **Orchestrator decides**, tagged **PROVISIONAL**, dissent recorded verbatim in
     the commit/PR body; human reviews provisional decisions first next session.

Hard floor: tags/releases, force-push, data deletion, protected paths, and policy
overrides stay human-only. A deadlock never unlocks §5.

---

## 9. Protocol self-learn (USER 2026-07-19)

When a session finds a **durable** process gap (missed channel, bad land handoff,
wrong truth-stack assumption — not a one-off typo):

1. **Unblock now** with harness-local notes if needed (skill, cron prompt, private
   state). That may ship without git.
2. **Draft the portable fix** in a worktree against **this file** and/or
   `docs/notes/memtrace-ops.md`. IRON LAW still applies.
3. **Peer dual-agree before the push is binding.** Author never sole-merges
   protocol/ops. A harness-local skill patch is **not** a substitute for the git
   protocol other vendors load from the repo.
4. Land via reviewer or human after dual-agree; quote SHAs on the log. Date and
   attribute the new rule so it can be audited and pruned later.

**Every rule in this document should be traceable to an incident or a mandate.** A
rule that cannot say why it exists is a candidate for deletion at the next review.

---

## 10. Enforcement

| Rule | Enforcement | Status |
|------|-------------|--------|
| Gates (§4) | `.github/workflows/ci.yml` | **Mechanical** |
| IRON LAW (§1) | — | **Honour-system** (shared-checkout pre-commit hook not installed) |
| Evidence Law (§2) | — | Honour-system |
| Dual-agree (§7.4) | fleet log + peer | Honour-system + social |
| Protected paths (§5) | gitignore + honour | Partial |
| Memtrace owner / pin (§6) | systemd unit + env | **Mechanical when configured** |

**Open item:** a hook rejecting commits whose worktree is the shared checkout would
make the IRON LAW mechanical. Needs a human decision (also constrains human commits).

---

## Appendix A — Bindings (durable)

| Binding | Value |
|---------|--------|
| Repo root | `git rev-parse --show-toplevel` — never hard-code a home path |
| Agent worktree root | `.claude/worktrees/<agent>-<topic>` |
| Memtrace `repo_id` | `scuffed-crew` |
| Ydoc threads | `fleet::chat` + `fleet::<initiative>`; legacy `fleet::channel` history only |
| Dual-channel reviews | poll chat + initiatives every tick; dual-write review/ACK |
| Memtrace ops | `docs/notes/memtrace-ops.md` |
| Memtrace owner | one `memtrace start --headless`; many `memtrace mcp` attach |
| Data dir | `MEMTRACE_DATA_DIR=~/.memdb` |
| Episodes | ADVISORY-ONLY across multi-MCP bridges |
| SSE events | known-dead — do not use as wake |
| Watcher | skill `memtrace-fleet-watcher`; cursor under `~/.hermes/state/fleet-watcher/` |
| Review gate | §4 / CI |
| Protected path | `crates/stat-tracker/test-data/` |
| Known agents | `claude`, `grok` (symmetric implement + review) |
| SYMMETRY | both implement and review; no self-merge |
| Backlog | `docs/notes/night-shift-backlog.md` |
| Night-shift state | `docs/notes/night-shift-state.json` (git scoreboard; not live cursor) |
| Front door | `AGENTS.md` (all vendors) |
| Project knowledge | `CLAUDE.md` |

## Appendix B — Incident register (dated, prunable)

| Date | Entry |
|------|-------|
| 2026-07-20 | Unpinned MCP re-anchored phantom `repo/.memdb` (split-brain / “frozen ydoc”). Pin `MEMTRACE_DATA_DIR` on every spawner. |
| 2026-07-20 | `memtrace stop` inside systemd unit self-cancels start — never `ExecStartPre=memtrace stop`. |
| 2026-07-19 | Single-channel poll missed peer APPROVE on the other surface — dual-channel law. |
| 2026-07-18 | Memtrace owner under alacritty/niri cgroup OOM-killed; half-alive memcore. Prefer user unit. |
| 2026-07-18 | Episodes: cross-bridge divergence + silent rollback despite write ACKs. Advisory only. |
| 2026-07-17 | SSE `/api/fleet/events` HTTP 200 + zero bytes for ~40 min — silence is not success. |
| 2026-07-17 | Shared-checkout branch switch nearly destroyed live work (twice). Origin of the IRON LAW. |
| 2026-07-17 | Undiagnosed ydoc wipe class — reseed from git/gh regardless of HTTP/MCP first glance. |

## Appendix C — Provenance and what changed

**Sources**

1. scuffed-crew `docs/fleet-protocol.md` (ops-complete: dual-channel, §5b deadlock,
   §8 self-learn, episodes advisory, Appendix A bindings, night-shift practice).
2. scuffed-crew `docs/notes/memtrace-ops.md` (owner/attach, truth stack, pin, watcher).
3. overwatch-strategy-app `docs/agent-protocol.md` (document shape: THE CARD, one iron
   law, Evidence Law, solo-skippable fleet module, durable vs prunable appendices,
   no-paraphrase `AGENTS.md`).

**Deliberate design choices in this rewrite**

1. **One iron law, not eight co-equal non-negotiables** — restating eight “musts” in
   `AGENTS.md` made none of them supreme and invited paraphrase drift.
2. **THE CARD** — load-bearing rules on one screen; detail below.
3. **Evidence Law named** — reviews without the four-part verdict were a recurring
   failure mode on both repos.
4. **Fleet quarantined in §7** — solo sessions skip coordination fiction.
5. **Bindings vs incidents** — Appendix A durable; Appendix B dated and prunable.
6. **`AGENTS.md` quotes laws verbatim only** — everything else is a pointer here.
7. **Host ops stay in `memtrace-ops.md`** — protocol = rules; ops = topology/recovery.
8. **`docs/fleet-protocol.md` superseded** — kept as provenance pointer only.

Original portable fleet protocol lineage: overwatch-strategy-app “Great Review”
(2026-07-17, PRs #78–#87) and scuffed-crew v0.1.0 ship day under fire.
