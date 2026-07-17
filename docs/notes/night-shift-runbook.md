# Night shift runbook (Grok + Claude fleet)

User offline. Grok continues **#6 leaderboards** + **security train**, keeps up with Claude on fleet.

## Watcher

Scheduled every ~5–10 minutes. Each tick:

1. Read `fleet::channel` (Memtrace `fleet_ydoc_read`).
2. Load `docs/notes/night-shift-state.json`; process **new** entries after `last_fleet_entry_id`.
3. Act on Claude messages:

| Claude says | Grok does |
|-------------|-----------|
| `#5A` **APPROVE** | `checkout main`, `merge --ff-only feat/discord-webhooks`, `push origin main`, fleet: `#5A MERGED` |
| `#5A` **CHANGES REQUESTED** | Fix on branch, push, fleet re-ping |
| `#6` **APPROVE** | Same merge/push pattern for `feat/leaderboards` |
| `#6` **CHANGES** | Fix, re-ping |
| Security **APPROVE** | Merge `feat/security-train` |
| Security **CHANGES** | Fix, re-ping |

4. Continue implementation if no blocking review.
5. Update `last_fleet_entry_id` + queue status in state JSON.
6. Fleet messages stay **short structured** (Claude protocol).

## Priority order tonight

1. Land **#5A** when Claude re-approves (already fixed `allowed_mentions`).
2. Build **#6** (`feat/leaderboards`) → request Claude review → merge on approve.
3. Build **security train** (`feat/security-train`) for remaining HIGH/MED items → review → merge.
4. Optional: match-publish Discord hook after #5A on main (small follow-up).

## File partitions

| Track | Owns |
|-------|------|
| #6 | `season` schema/types, `personal_stats` agg, public leaderboards routes, `crates/app` leaderboards page |
| Security | strategy WS, scrims authz, chat relay — **avoid** leaderboards files |

## Stop conditions

- All three queue items `merged` or `blocked` with reason written to state.
- Or 12h elapsed — leave status summary in state + fleet.

## Do not

- Force-push `main`
- Merge without Claude **APPROVE** (or explicit user pre-auth — user said finish without them, so APPROVE = go)
- Long fleet prose
