# Night shift runbook (Grok + Claude fleet)

User offline. Grok continues **#6 leaderboards** + **security train**, keeps up with Claude on fleet.

## Standing rule (user): dual agreement + fleet-visible work

**Any fix or change must be agreed by both agents before merge.**  
Peer review alone is not enough if only one party posts a verdict: the other must **ACK/APPROVE** on the fleet channel too. Applies to **Grok and Claude symmetrically** — features, review follow-ups, hotfixes, night-shift patches.

| Who ships | Who reviews | Then |
|-----------|-------------|------|
| Grok (this agent / sub-agents) | Claude **APPROVE** on fleet + Grok **ACK** | merge + push `main` |
| Claude | Grok **APPROVE** on fleet + Claude **ACK** | merge + push `main` (or user) |

- Do **not** merge your own branch after only a self-check.
- Do **not** merge on a solo peer APPROVE when dual agreement is required — wait for the other party’s **ACK**.
- Do **not** skip re-review after addressing CHANGES REQUESTED — re-ping peer and wait for a **new** APPROVE + ACK cycle.
- **Code never** lands without dual agreement on fleet. Pure night-shift status notes (this file / state JSON) may update without review.

### Fleet channel is the team log (mandatory)

You are cooperating as a team. **Do not keep findings chat-only with the user.**

Everything material to the other agent must be posted on Memtrace `fleet::channel` (short structured notes):

| Must post on fleet | Examples |
|--------------------|----------|
| Review verdicts | APPROVE / CHANGES REQUESTED with tip SHA |
| **All review findings** | bugs, suggestions, **nits** — every one, not a summary only |
| Test evidence | what ran (`cargo check`, fixture tests, skipped + why) |
| File scope | which files you actually read / merge-stat vs triple-dot |
| Merge / CI | MERGED @ sha; CI green/red; draft release notes |
| Process notes | dual-agree holds, open questions, blockers |
| Answers to peer QUERYs | specifics on channel, not only in user chat |

User chat may mirror status for the human; it is **not** a substitute for the fleet log. If you formed a nit in review and only told the user, that is a process failure — post it on fleet.

## Memtrace / fleet ops

Host topology, MCP vs HTTP truth stack, episode advisory rules, durable
`memtrace start`, Hermes watcher crons, and restart recovery:

→ **`docs/notes/memtrace-ops.md`** (binding for this machine; pairs with
`docs/fleet-protocol.md`).

Live watcher cursor is **not** `night-shift-state.json` — see
`~/.hermes/state/fleet-watcher/` and skill `memtrace-fleet-watcher`.

## Watcher

Scheduled every ~5–10 minutes. Each tick:

1. Read `fleet::channel` (Memtrace `fleet_ydoc_read`).
2. Load `docs/notes/night-shift-state.json`; process **new** entries after `last_fleet_entry_id`.
3. Act on peer messages:

| Peer says | Action |
|-----------|--------|
| **APPROVE** on *our* branch | Post our **ACK** if we agree; only then ff-merge → `main`, push, fleet `MERGED` |
| **CHANGES REQUESTED** on *our* branch | Fix on branch, push, fleet re-ping for **re-review** (no merge until new dual cycle) |
| Claude ships / requests review | Grok reviews; post **full findings** (bugs/suggestions/**nits**) + APPROVE or CHANGES on fleet |
| We APPROVE Claude’s branch | Wait for Claude **ACK** (dual agree); then may ff-merge if night autonomy + clean history |
| Peer QUERY / challenge | Answer **on fleet** with specifics (files, tests, nits) — not user-chat only |

4. Continue implementation if no blocking review.
5. Update `last_fleet_entry_id` + queue status in state JSON.
6. Fleet messages stay **short structured** but **complete** (nits included).

## Priority order tonight

1. **#5A** — done / merged when Claude APPROVE lands (and only then).
2. **#6** (`feat/leaderboards`) → Claude review → merge only on APPROVE (re-review after any fix).
3. **Security train** (`feat/security-train`) → Claude review → merge only on APPROVE.
4. Optional: match-publish Discord hook as a **separate** branch + review.

## File partitions

| Track | Owns |
|-------|------|
| #6 | `season` schema/types, `personal_stats` agg, public leaderboards routes, `crates/app` leaderboards page |
| Security | strategy WS, scrims authz, chat relay — **avoid** leaderboards files |

## Stop conditions

- All queue items `merged` or `blocked` with reason in state.
- Or ~12h elapsed — leave status summary in state + fleet.

## Do not

- Force-push `main`
- Merge without **dual agreement on fleet** (self-check ≠ review; solo APPROVE ≠ merge)
- Merge after a fix without a **fresh** dual APPROVE+ACK cycle
- Keep review findings / nits / test gaps **only in user chat** — post them on `fleet::channel`
- Long fleet prose (but completeness beats silence on findings)
