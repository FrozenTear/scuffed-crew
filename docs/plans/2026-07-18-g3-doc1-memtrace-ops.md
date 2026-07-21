# G3 + DOC-1 Implementation Plan

> **For Hermes:** Use subagent-driven-development skill to implement this plan task-by-task.

**Goal:** Docs-only branch delivering (1) rewritten `night-shift-state.json` from git truth (G3) and (2) Memtrace/fleet ops documentation + protocol hooks (DOC-PLAN-1 with Claude ACK additions).

**Architecture:** Single branch `docs/g3-doc1-memtrace-ops` in worktree `.claude/worktrees/grok-g3-doc1`. Portable rules stay in `fleet-protocol.md`; host ops go in `docs/notes/memtrace-ops.md`. State JSON is git-derived scoreboard, not a live watcher cursor (cursor lives in `~/.hermes/state/fleet-watcher/`).

**Tech Stack:** Markdown + JSON only. No Rust/code changes. No `test-data/`.

**Constraints:**
- IRON LAW: only edit inside worktree
- Claude ACK DOC-PLAN-1 `01KXVKXCSQ`: include (1) Appendix A SYMMETRY (both agents implement+review; USER ruling) (2) episode-store ADVISORY-ONLY + 3 failure modes from `01KXVCNT2C`
- G1 version-integrity is OUT OF SCOPE
- Dual-agree with Claude after push; do not self-merge main

**Worktree:** `~/github/scuffed-crew/.claude/worktrees/grok-g3-doc1`  
**Branch:** `docs/g3-doc1-memtrace-ops` @ `origin/main` (`7420451`)  
**Reseed quote:** `01KXVBY7AT`

---

### Task 1: Confirm worktree + plan committed

**Objective:** Plan lives on the branch for Claude.

**Files:**
- Create: `docs/plans/2026-07-18-g3-doc1-memtrace-ops.md` (this file)

**Steps:** Ensure worktree clean aside from plan; commit plan.

```bash
cd ~/github/scuffed-crew/.claude/worktrees/grok-g3-doc1
git add docs/plans/2026-07-18-g3-doc1-memtrace-ops.md
git commit -m "docs(plan): G3 state rewrite + DOC-1 memtrace-ops"
```

---

### Task 2: Rewrite `docs/notes/night-shift-state.json` (G3)

**Objective:** Replace stale pre-night-shift state with git-true scoreboard through SE-1.

**Files:**
- Create or replace: `docs/notes/night-shift-state.json`

**Required JSON shape (top-level keys):**
```json
{
  "updated_at": "2026-07-18T22:00:00Z",
  "repo": "scuffed-crew",
  "agent_id": "fleet",
  "main": "7420451",
  "reseed_ulid": "01KXVBY7ATP4MY1E7DEX3D27Y2",
  "status": "active",
  "protocol": { ... dual-agree, fleet::chat, worktrees, never[] ... },
  "orchestration": { "orchestrator": "Fable", "implementers": "Opus", "grok": "review+dissent+watcher" },
  "symmetry": {
    "ruling": "USER 2026-07-18: both agents implement AND review; author never sole-merges own work; dual-agree unchanged",
    "note": "Appendix A binding; same standard for claude and grok"
  },
  "watcher": {
    "note": "Live cursor NOT here — see ~/.hermes/state/fleet-watcher/scuffed-crew.json",
    "skill": "memtrace-fleet-watcher",
    "crons": ["scuffed-fleet-ydoc-delta", "scuffed-fleet-heartbeat"]
  },
  "tags": {
    "stat-tracker-v0.1.0": "76b1422",
    "stat-tracker-v0.1.1": "e6785ad",
    "stat-tracker-v0.1.2": "c9b1b3e"
  },
  "queue": {
    /* each item: status merged|open|user|user, tips, notes */
  },
  "open": {
    "g1_version_integrity": "open — Cargo 0.1.0 vs tag 0.1.2",
    "doc_plan_1": "this branch",
    "ph0_defects": ["hero last-wins→majority", "cell window drift", "outcome backfill"],
    "user_repairs": ["Havana D/HLG", "Lijiang/Nepal via stat-edit GUI"]
  }
}
```

**Queue MUST include (all in_main tips verified 7420451):**
- HW-1: 4499f36 → merge 281df05
- GL-1: ef48e70 → 727ad41
- CG-1: f9b5a7f → 23a5bc9 (CG-1a OOS note)
- MM-1: 626d1f9 → 7dfd537
- RJ-1: ec44788 → 9f155a9
- IP-1: 7404bc5 → e6785ad
- LD-1: 801d7ee → c9b1b3e
- NT-1: e5dac71 → fea5185
- SE-1: a3b3997 → 7420451
- Ph1 local-reg 09494e0, Ph2 nostr 8fa99fb (ancestors)
- Prior portable items from old state can be summarized as `phase1_ship` complete @ v0.1.0

**Verify:**
```bash
python3 -c 'import json; json.load(open("docs/notes/night-shift-state.json")); print("ok")'
python3 -c 'import json;d=json.load(open("docs/notes/night-shift-state.json")); assert d["main"]=="7420451"'
```

**Commit:** `docs(g3): rewrite night-shift-state.json from git truth @7420451`

---

### Task 3: Create `docs/notes/memtrace-ops.md` (DOC-1 body)

**Objective:** Join/recovery ops runbook both agents load; ≤600 lines; link official docs.

**Files:**
- Create: `docs/notes/memtrace-ops.md`

**Required sections:**
1. Purpose + load on every fleet join
2. Topology (one `memtrace start` owner → memcore :50051 → UI :3030; many `memtrace mcp` attach)
3. Host bindings table (scuffed-crew): repo_id, MEMTRACE_MEMDB_DATA_DIR=~/.memdb preferred, absolute mcp `~/.volta/bin/memtrace`, worktrees path, NEVER start under alacritty (OOM 2026-07-18)
4. Truth stack: `git/gh > MCP ydoc > HTTP :3030 > episodes > memory`; use `fleet_audit` for forensics
5. Episode store = ADVISORY-ONLY + 3 failure modes from 01KXVCNT2C:
   - (1) long read/append crash loop on bridge
   - (2) silent memdb rollback / ydoc loss despite write ACKs
   - (3) cross-bridge episode-view divergence (ydoc shared, episodes not)
6. Join checklist (§1 protocol + MCP not HTTP)
7. Restart recovery (8 steps)
8. Watcher: skill memtrace-fleet-watcher, crons, state path ~/.hermes/state/..., HTTP delta vs MCP heartbeat, hermes gateway restart after update
9. Hermes approval: manual mode, /approve, timeout, no retry after hard deny
10. Optional worktree overlays (product feature)
11. Ydoc export durability recipe (periodic MCP dump)
12. Links: memtrace.io docs features/fleet, concepts/architecture, cli/start, config/data-directories

**Verify:** file exists, has "ADVISORY-ONLY", "7420451" or reseed ULID, "memtrace start"

**Commit:** `docs(memtrace): add memtrace-ops runbook for dual-agent fleet`

---

### Task 4: Patch `docs/fleet-protocol.md` + AGENTS.md + runbook

**Objective:** Thin protocol hooks + SYMMETRY; point to ops note.

**Files:**
- Modify: `docs/fleet-protocol.md` (§1 if needed, §6 Restart recovery, Appendix A)
- Modify: `AGENTS.md` (one line → memtrace-ops.md)
- Modify: `docs/notes/night-shift-runbook.md` (link ops note if file exists)

**§6 add:**
- Truth stack one-liner + episodes advisory + 3 modes pointer to memtrace-ops
- Intent TTL 0 agents = normal
- Link memtrace-ops.md

**Appendix A add rows:**
- Memtrace ops doc path
- Owner: one start --headless; data dir note
- Watcher skill + state path
- SYMMETRY ruling (both implement+review)
- Episodes advisory
- Known agents hermes/grok-4.5 + claude/Fable

**Verify:** rg SYMMETRY fleet-protocol; rg memtrace-ops AGENTS.md

**Commit:** `docs(fleet): §6 truth-stack, App A ops+SYMMETRY, AGENTS pointer`

---

### Task 5: Push + fleet REVIEW REQUEST

**Objective:** Branch on origin; Claude can review.

```bash
cd worktree
git push -u origin docs/g3-doc1-memtrace-ops
```

Fleet posts:
- `fleet::chat` REVIEW REQUEST DOC-1+G3 @ tip SHA
- `fleet::doc-memtrace` thread if useful

Do NOT merge main.

---

## Acceptance (Claude dual-agree)

- [ ] state.json main=7420451, night merges present, open G1 noted
- [ ] memtrace-ops covers topology, truth stack, 3 failure modes, recovery, watcher, Hermes approve
- [ ] protocol SYMMETRY + episodes advisory + ops link
- [ ] No code/Cargo changes
- [ ] Branch pushed; REVIEW REQUEST on ydoc
