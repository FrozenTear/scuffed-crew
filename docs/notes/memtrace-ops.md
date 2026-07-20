# Memtrace / fleet ops runbook (scuffed-crew)

**Load this file on every fleet join.** Companion to the portable protocol:
`docs/fleet-protocol.md` (§1 join, §3 IRON LAW, §6 liveness). Protocol = rules;
this note = host topology, truth stack, recovery, Hermes watcher/approvals.

Do not treat chat memory or a single HTTP curl as SoT. After any restart or
suspected wipe, re-derive from git/gh first, then MCP ydoc.

---

## 1. Purpose

| Who | When |
|-----|------|
| Every agent (claude, grok/hermes, …) | Session start / fleet join |
| Watcher / night-shift ticks | Before claiming open work |
| Human operator | Daemon OOM, dual-start, bridge skew |

**Out of scope here:** product coding conventions, cargo gates detail (see
protocol Appendix A), backlog content (`night-shift-backlog.md`).

**SYMMETRY (USER ruling 2026-07-18):** both agents **implement and review**.
Author never sole-merges own work; dual-agree unchanged. Same standard for
claude and grok — not “one implements, one only reviews.”

---

## 2. Topology

```
┌─────────────────────────────────────────────────────────────┐
│  ONE owner process:  memtrace start --headless              │
│       │                                                     │
│       ├─► memcore  :50051   (durable graph / fleet backend) │
│       └─► UI/API   :3030    (dashboard + HTTP fleet APIs)   │
└─────────────────────────────────────────────────────────────┘
                    ▲ attach only
        ┌───────────┼───────────┬──────────────┐
        │           │           │              │
   memtrace mcp  memtrace mcp  memtrace mcp   …
   (Hermes)      (Claude)      (extra shell)
```

Rules:

1. **Exactly one** `memtrace start` owner for the shared data dir.
2. **Many** `memtrace mcp` stdio children may attach; never a second `start`.
3. MCP children die with the agent session; the owner must outlive them.
4. SSE `/api/fleet/events` is **known-dead** (HTTP 200, zero bytes) — do not
   use as a wake signal. Poll ydoc via MCP (primary) or HTTP (advisory).
5. Orphan `memcore-server` with a dead parent PID is half-alive — stop/clear
   before restarting the owner.

---

## 3. Host bindings (scuffed-crew)

| Binding | Value |
|---------|--------|
| `repo_id` | `scuffed-crew` |
| Shared checkout (agent READ-ONLY) | **Repo root** — `git rev-parse --show-toplevel` (or `$SC_ROOT` if set). Do not commit a personal home path here. |
| Worktrees | `.claude/worktrees/<agent>-<topic>` under shared checkout |
| Preferred memdb data dir | `MEMTRACE_DATA_DIR=~/.memdb` (single multi-repo store). **The var is `MEMTRACE_DATA_DIR`** — an earlier revision said `MEMTRACE_MEMDB_DATA_DIR`, which memtrace ignores (official docs: config/environment). |
| MCP / CLI binary | absolute path to `memtrace` on PATH or under volta (`$(command -v memtrace)` / `$HOME/.volta/bin/memtrace`) — cron/unit PATH often lacks volta |
| Owner start | `memtrace start --headless` from durable unit or `$HOME` |
| UI | `http://127.0.0.1:3030` |
| gRPC | `127.0.0.1:50051` |
| Primary ydoc thread | `fleet::chat` |
| Initiative threads | `fleet::<initiative>` |
| Legacy thread | `fleet::channel` (history only) |
| Watcher state | `~/.hermes/state/fleet-watcher/scuffed-crew.json` |
| Watcher skill | `memtrace-fleet-watcher` |
| Protected paths | `crates/stat-tracker/test-data/` — never commit |

### NEVER start the owner under alacritty/niri app cgroup

**Incident 2026-07-18:** `memtrace start` launched from a terminal/agent shell
shared the desktop cgroup (`app-niri-alacritty-….scope`). Kernel OOM-kill took
the owner with the terminal unit. Fallout: `:3030` dead, `:50051` flaky,
orphan memcore, HTTP delta `FLEET_BLIND`, presence publish errors.

**Preferred:** systemd **user** unit with `Restart=always`,
`WorkingDirectory=%h`, `ExecStart=%h/.volta/bin/memtrace start --headless`
(or any absolute `memtrace` on the unit's PATH — avoid hard-coding a
username in git).

**DEPLOYED 2026-07-20** (this exact unit is live at
`~/.config/systemd/user/memtrace.service`):

```ini
[Unit]
Description=Memtrace analysis daemon (single owner, ~/.memdb)
After=network-online.target
Wants=network-online.target
# A permanently-failing daemon must land in `failed` and stay visible,
# not restart-loop silently (same rationale as scuffed-stat-tracker.service).
StartLimitIntervalSec=300
StartLimitBurst=5

[Service]
Type=simple
# Single source of truth for the store — every attach (MCP) must match this.
Environment=MEMTRACE_DATA_DIR=%h/.memdb
# Volta shim resolves the node toolchain; give it a sane PATH.
Environment=PATH=%h/.volta/bin:/usr/local/bin:/usr/bin:/bin
# NOTE: no ExecStartPre/ExecStop with `memtrace stop` — that command is
# systemd-aware and stops the `memtrace` unit itself, canceling our own start
# job (observed 2026-07-20). Any pre-existing terminal-scoped owner must be
# stopped manually ONCE before first start; systemd's cgroup TERM handles
# shutdown thereafter.
ExecStart=%h/.volta/bin/memtrace start --headless
Restart=on-failure
RestartSec=10
# memcore holds a multi-GB store; allow a graceful close.
TimeoutStopSec=120
# If the kernel OOM-kills memcore, stop the whole unit cleanly, then
# Restart=on-failure brings it back — no half-dead daemon.
OOMPolicy=stop

[Install]
WantedBy=default.target
```

Migration procedure (one-time, when an old terminal-scoped owner exists):

```bash
systemctl --user daemon-reload
systemctl --user enable memtrace.service          # do NOT --now yet
memtrace stop                                     # stop old owner OUTSIDE the unit
# wait for :3030/:50051 to free, then:
systemctl --user start memtrace.service
```

**GOTCHA — `memtrace stop` is systemd-aware.** It stops the `memtrace` *unit*
(not just the process), so putting it in `ExecStartPre=`/`ExecStop=` cancels
the unit's own start job. Never reference it from inside the unit.

### Attach pinning — every MCP must resolve the same store

An unpinned `memtrace mcp` resolves its data dir from the **cwd chain** and
anchors a per-workspace store at the repo root (`repo/.memdb`) instead of
attaching to the running owner. Result: **split-brain** — that agent reads a
12 MB phantom while the fleet writes the real 4.2 GB `~/.memdb` (incident
2026-07-20; the "frozen ydoc" / "empty initiative threads" symptom all day).

Pin `MEMTRACE_DATA_DIR=$HOME/.memdb` in EVERY place an MCP is spawned:

| Spawner | Where the pin lives |
|---------|---------------------|
| claude (Claude Code) | `mcpServers.memtrace.env` in `~/.claude/settings.json` **and** `~/.claude.json` |
| grok (`~/.local/bin/agent`) | the launcher's env for `memtrace mcp` |
| hermes watchdog + anything else | `~/.config/environment.d/50-memtrace.conf` (whole-session; needs re-login to apply) |

Phantom diagnosis + cleanup:

```bash
# who is anchored where?
for p in $(pgrep -f 'linux-x64/bin/memtrace mcp'); do
  echo "$p cwd=$(readlink /proc/$p/cwd) env=$(tr '\0' '\n' </proc/$p/environ | grep ^MEMTRACE_DATA_DIR)"
done
# a phantom is live if repo/.memdb mtime advances or daemon.pid is held open
fuser -v <repo>/.memdb/* 2>&1 | head
# cleanup ONLY after every writer is repinned+restarted: tar backup, then rm -rf repo/.memdb
```

Avoid dual owners on different data dirs (`~/.memdb` vs `repo/.memdb` vs
`hermes-agent/.memdb`).

**Incident 2026-07-20 (host migration):** owner moved from
`app-niri-alacritty` scope into `memtrace.service` (own cgroup, restart
policy, OOMPolicy=stop). First start attempt self-canceled via the
`ExecStartPre=memtrace stop` gotcha above — zero downtime, old tree survived
until the corrected manual-stop-then-start sequence. Both MCP attach clients
survived the bounce; grok's harness respawned its MCP unpinned and re-anchored
the phantom, which is why the pin table above exists.

---

## 4. Truth stack

Highest → lowest authority for fleet coordination claims:

| Rank | Source | Use for |
|------|--------|---------|
| 1 | **git / gh** (fetch, `origin/main`, PRs, CI, tags) | merges, scoreboard, “what landed” |
| 2 | **MCP ydoc** (`fleet_ydoc_read` / `fleet_ydoc_append`) | peer claims, REVIEW REQUEST, RESEED |
| 3 | **HTTP :3030** ydoc/status | advisory wake / diagnostics only |
| 4 | **Episodes** (`fleet_query_episodes`, record) | **ADVISORY-ONLY** (see §5) |
| 5 | **Agent memory / chat** | never sole evidence after restart |

Forensics: `fleet_audit` (durable append-only provenance — intents, episodes,
leases, escalations). Survives TTL’d live intent registry.

**HTTP `count=0` is NOT wipe proof.** HTTP views lag/truncate after restart
while MCP still holds modern tips (observed 2026-07-18). Cross-check MCP
before RESEED or panic.

Reseed discipline: **quote the RESEED ULID before claiming open work.**
Example (2026-07-18 sync): `01KXVBY7ATP4MY1E7DEX3D27Y2` (short form often
cited as `01KXVBY7AT`). Main tip at G3 rewrite baseline: `7420451`.

---

## 5. Episode store = ADVISORY-ONLY

Context: reseed / ops note `01KXVCNT2C` (and fleet practice 2026-07-18).

Episodes are useful breadcrumbs. They are **not** dual-agree substrate and
**not** wipe/merge proof. Prefer git + MCP ydoc for scoreboard and consensus.

### Three failure modes

1. **Long read/append crash loop on a bridge**  
   One MCP stdio child thrashing on large episode list/read or repeated
   append can wedge or restart that bridge while peers look fine. Symptom:
   one agent “can’t see” / times out episode tools; ydoc may still work.

2. **Silent memdb rollback / ydoc loss despite write ACKs**  
   Client received success but durable state later missing or rolled back
   (undiagnosed wipe class; 07-17 wipe remains in protocol lore). Always
   **re-read** critical appends; reseed from git/gh if MCP empty *and*
   peers agree.

3. **Cross-bridge episode-view divergence**  
   Ydoc is shared and consistent across bridges; **episode views are not
   guaranteed equal**. Bridge A may show N episodes after
   `fleet_record_episode`; bridge B still shows an older subset for the same
   `repo_id`. Do **not** dual-agree or block on episode-count parity. Soft
   discrepancy once on the log, continue on RESEED + git.

Live **intent registry** is TTL’d (~120s) and empty after restart — normal,
not a wipe.

---

## 6. Join checklist

Every agent, every join (aligns with protocol §1):

1. **Owner up?** ports `:50051` + `:3030`, or `memtrace status` / user unit.
   If only MCP children exist without owner → start owner (one, durable).
2. **`fleet_status`** (MCP) — coordination alive.
3. **`fleet_branch_context`** — agent id, peer intents, escalations, recent
   peer episodes (advisory).
4. **`fleet_ydoc_read`** whole-repo / `fleet::chat` — **MCP only for truth**,
   not dashboard HTTP. Compare tip ULID to any HTTP curl for skew notes.
5. If ydoc **genuinely** empty (MCP empty + peers confirm + git history known):
   first agent posts charter + scoreboard **reseeded from git/gh**; quote
   RESEED ULID.
6. **Join append** — `fleet_ydoc_append` kind `intent`: who, model/vendor,
   claim/lane, worktree branch if any. Bodies ≤ ~400 chars; pointer to
   artifacts.
7. **Presence** — `fleet_publish_intent` on `[fleet::chat]` (TTL ~120s).
   Re-publish each watcher tick / loop. Use a **real** IntentKind (e.g.
   benign `bug_fix`); put human meaning in `assignment`, not invented kinds.
8. Load protocol + this ops note; IRON LAW: work only in
   `.claude/worktrees/<agent>-topic`.

Do not start open implementation until RESEED ULID is quoted (or charter
fresh and dual-visible).

---

## 7. Restart recovery (numbered)

After OOM, `memtrace stop/start`, host reboot, or “is the fleet wiped?”:

1. **Stop cleanly** if half-alive: `memtrace stop`; free stale pids; confirm
   `:3030` / `:50051` not held by orphans (`ss -ltnp | rg '3030|50051'`).
2. **Start one owner** under durable unit (or `cd "$HOME" && memtrace start
   --headless` with `MEMTRACE_MEMDB_DATA_DIR=~/.memdb`). **Not** under
   alacritty/niri scope if avoidable.
3. **Verify ports + owner**, not only an MCP child process.
4. **MCP first:** `fleet_status`, `fleet_ydoc_read` (`fleet::chat` + key
   initiative threads). If MCP modern and HTTP ancient → **skew, not wipe**.
5. **git/gh reseed:**
   ```bash
   SC_ROOT="${SC_ROOT:-$(git rev-parse --show-toplevel)}"
   git -C "$SC_ROOT" fetch origin
   git -C "$SC_ROOT" rev-parse --short origin/main
   gh pr list --state open --limit 20   # run from clone or pass --repo
   ```
6. **Intents are volatile** — empty registry is normal; re-publish presence.
7. **JOIN/REJOIN** on ydoc; quote RESEED ULID before open work; reset
   watcher quiet/backoff (see §8).
8. **Re-arm watchers** (delta + heartbeat). After **`hermes update`**, run
   **`hermes gateway restart`** — stale gateway can break tools
   (`build_tool_label` / missing memtrace PATH).
9. Both agents independently reseeding from git/gh and matching is itself a
   correctness check. Do not mass re-record episodes to “heal” bridges.

Quick owner bounce:

```bash
memtrace stop
ss -ltnp | rg '3030|50051'   # expect free
cd "$HOME" && MEMTRACE_MEMDB_DATA_DIR="$HOME/.memdb" \
  "$(command -v memtrace || echo "$HOME/.volta/bin/memtrace")" start --headless
curl -fsS -m 5 http://127.0.0.1:3030/api/fleet/status
# then MCP fleet_ydoc_read — not only HTTP tip
```

---

## 8. Watcher

| Piece | Detail |
|-------|--------|
| Skill | `memtrace-fleet-watcher` |
| State dir | `~/.hermes/state/fleet-watcher/` |
| State file | `…/scuffed-crew.json` (live **cursor** lives here — not
`docs/notes/night-shift-state.json`) |
| Delta cron | HTTP tip poll (`ydoc-delta` / `scuffed-fleet-ydoc-delta`) — **advisory wake only** |
| Heartbeat cron | MCP agent tick (`scuffed-fleet-heartbeat`) — real act/presence |
| Tick shape | load state → health → git reseed → MCP ydoc ULID diff (**chat + watch_threads**) → act\|backoff → presence → persist → report |
| Dual-channel | Poll initiative **and** `fleet::chat` every tick; dual-write reviews (USER 2026-07-19). Skill `memtrace-fleet-watcher` v1.3.6+ |
| Protocol self-learn | Durable process fixes → worktree docs draft → claude dual-agree before binding push (protocol §8) |
| Backoff | 3 min base → 5 → 10; activity resets; pin base when blind |
| Heartbeat schedule | prefer **every 10m** (ticks often wall ~2.5–3.5 min); delta may stay every 3m |

Iron rules for ticks:

- One tick per invocation — no sleep-loop inside the model.
- Cursor advances **after** process, never on dual-fail blind.
- Never rewind cursor to HTTP tip when MCP is ahead.
- SSE is dead; do not wait on it.
- Shared checkout remains READ-ONLY unless USER §3 exception is on the log.
- After hermes update: **`hermes gateway restart`** (stale `build_tool_label`).
- Visual act feedback (scuffed-crew hermes): non-silent banner + desktop notify on act; idle may `[SILENT]`.

Deploy sketch (operator):

```bash
# A) delta — no LLM
hermes cron create 'every 3m' \
  --name 'scuffed-fleet-ydoc-delta' \
  --script fleet-ydoc-delta.sh \
  --no-agent \
  --deliver local

# B) heartbeat — agent
SC_ROOT="${SC_ROOT:-$(git rev-parse --show-toplevel)}"
PROMPT=$(cat ~/.hermes/skills/memtrace-fleet-watcher/templates/cron-heartbeat-prompt.md)
hermes cron create 'every 10m' "$PROMPT" \
  --name 'scuffed-fleet-heartbeat' \
  --workdir "$SC_ROOT" \
  --skill memtrace-fleet-watcher \
  --skill memtrace-fleet-first \
  --deliver local
```

`last_status: ok` on cron ≠ body `tick ok` — read Response / state file.

---

## 9. Hermes command approval

When Hermes runs with command approvals:

| Rule | Detail |
|------|--------|
| Mode | `approvals.mode` **manual** (default safe for fleet machines) |
| Allow | Human **Allow** within the approval **timeout** |
| Slash | `/approve` for pending commands when interactive |
| Hard deny | **Do not retry** the same command hoping for a different outcome |
| YOLO | `/yolo` (or equivalent auto-approve) **only if the USER chooses** — agents must not enable it unilaterally |

Cron ticks that need shell writes: prefer agent `write_file` + atomic rename
over blocked python heredocs. If worktree add is denied, **stop and ask** —
do not thrash alternate git mutations.

---

## 10. Optional worktree overlays (`index_directory`)

Product feature: Memtrace can index a worktree path and expose overlays on
search (`find_code` / `worktree=` / `include_overlays`).

Ops guidance:

- Canonical graph stays on main/shared `repo_id` `scuffed-crew`.
- Optional: `index_directory` on
  `$SC_ROOT/.claude/worktrees/<agent>-<topic>` (repo-relative worktree path)
  for peer-WIP-aware search.
- Overlays are **not** a substitute for git push + dual-agree.
- Sweep stale overlays: `cleanup_worktrees` / TTL; do not leave deleted
  worktree paths as permanent graph noise.
- Prefer incremental index; avoid `clear_existing` on the shared repo during
  a live fleet session unless the human ordered a rebuild.

---

## 11. Periodic ydoc export durability

memdb has survived restarts in practice, but silent loss has also happened.
Treat ydoc as **convenience**, not sole archive.

Recipe (periodic, human or assigned agent):

1. MCP `fleet_ydoc_read` for `fleet::chat` + active `fleet::<initiative>`
   threads (full enough tail for the shift).
2. Dump to a dated file under the worktree, e.g.
   `docs/notes/fleet-ydoc-export-YYYY-MM-DD.md` (or `.json` raw dump).
3. Open PR on a docs branch; dual-agree; human merge.
4. Pointer on `fleet::chat` (≤400 chars) with SHA/path — not the full dump.

Do not spam full ydoc replay into the live thread. Export is for git
durability when the shift mattered (release day, multi-PR night).

---

## 12. Official Memtrace docs

- Fleet concepts: https://memtrace.io/docs/features/fleet  
- Architecture: https://memtrace.io/docs/concepts/architecture  
- CLI start: https://memtrace.io/docs/cli/start  
- Data directories: https://memtrace.io/docs/config/data-directories  

In-product: MCP `search_docs` / `ask_docs` / `read_doc` when online.

---

## Quick reference card

```
Truth:     git/gh > MCP ydoc > HTTP :3030 > episodes(advisory) > memory
Owner:     one memtrace start --headless → :50051 + :3030
Attach:    many memtrace mcp; never second start
MCP bin:   $(command -v memtrace) or $HOME/.volta/bin/memtrace
Data:      MEMTRACE_MEMDB_DATA_DIR=~/.memdb
Cgroup:    NEVER owner under alacritty/niri (OOM 2026-07-18)
Worktree:  .claude/worktrees/<agent>-topic  |  shared checkout R/O ($SC_ROOT)
Join:      status → branch_context → ydoc_read(MCP) → append → presence 120s
Poll:      fleet::chat AND initiatives every tick; dual-write review/ACK
Self-learn: process gap → worktree protocol/ops draft → peer dual-agree → land
Wipe?:     HTTP count=0 ≠ wipe; MCP + peers + git
Episodes:  ADVISORY-ONLY; 3 modes (bridge crash, silent rollback, cross-bridge)
Watcher:   skill memtrace-fleet-watcher; state ~/.hermes/state/fleet-watcher/
Post-upd:  hermes gateway restart
Approve:   manual; Allow in time; /approve; hard deny = stop; /yolo = USER only
SYMMETRY:  both agents implement + review; dual-agree; no self-merge
Reseed eg: 01KXVBY7AT … / main@7420451 (quote before claim)
```

---

## Related local docs

| Doc | Role |
|-----|------|
| `docs/fleet-protocol.md` | Portable multi-agent protocol + Appendix A bindings |
| `docs/notes/night-shift-runbook.md` | Night-shift act table / ops loop |
| `docs/notes/night-shift-backlog.md` | Queued work |
| `docs/notes/night-shift-state.json` | Git-derived scoreboard snapshot (not live cursor) |
| `AGENTS.md` | Entry pointer for all harnesses |
