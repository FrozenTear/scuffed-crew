# Night shift runbook (Grok + Claude fleet)

User offline. Grok continues **#6 leaderboards** + **security train**, keeps up with Claude on fleet.

## Standing rule (user): review gate for EVERYONE

**Any fix or change must go through peer review before merge.**  
This applies to **Grok and Claude symmetrically** — feature PRs, review-follow-up fixes, hotfixes, and night-shift patches.

| Who ships | Who reviews | Then |
|-----------|-------------|------|
| Grok (this agent / sub-agents) | Claude cross-review **APPROVE** | merge + push `main` |
| Claude | Grok cross-review **APPROVE** | merge + push `main` (or user) |

- Do **not** merge your own branch after only a self-check.
- Do **not** skip re-review after addressing CHANGES REQUESTED — re-ping peer and wait for a **new** APPROVE.
- **Code never** lands without peer APPROVE. Pure night-shift status notes (this file / state JSON) may update without review.

## Watcher

Scheduled every ~5–10 minutes. Each tick:

1. Read `fleet::channel` (Memtrace `fleet_ydoc_read`).
2. Load `docs/notes/night-shift-state.json`; process **new** entries after `last_fleet_entry_id`.
3. Act on peer messages:

| Peer says | Action |
|-----------|--------|
| **APPROVE** on *our* branch | ff-merge that branch → `main`, push, fleet short `MERGED` |
| **CHANGES REQUESTED** on *our* branch | Fix on branch, push, fleet re-ping for **re-review** (no merge until new APPROVE) |
| Claude ships / requests review | Grok reviews; post APPROVE or CHANGES; **do not merge Claude’s branch without Grok APPROVE** |
| We APPROVE Claude’s branch | May ff-merge if night autonomy + clean history |

4. Continue implementation if no blocking review.
5. Update `last_fleet_entry_id` + queue status in state JSON.
6. Fleet messages stay **short structured**.

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
- Merge without **peer APPROVE** (self-check ≠ review)
- Merge after a fix without a **fresh** APPROVE
- Long fleet prose
