# Stat Tracker X11 Capture — Fleet Orchestration Plan

> **For agentic workers:** This is an ORCHESTRATION plan, not an implementation
> plan. The implementation spec is `2026-07-21-stat-tracker-x11.md` (same dir,
> review cycle R1–R5 closed, implementation-ready). This document tells the
> orchestrator (Claude) how to execute that spec with memtrace fleet
> coordination, grok as symmetric peer, and subagents — fastest wall-clock,
> highest quality. Execute phases with checkbox tracking. The binding protocol
> is `docs/fleet-protocol.md`; where this doc and the protocol disagree, the
> protocol wins.

**Goal:** Land the X11 capture backend (spec Tasks 0–7) on `main`, CI-green,
with every merge dual-agreed, in ~1 day of wall-clock instead of the spec's
2–3.5 serial days — then hand USER the Task 8 live-smoke checklist.

**Status:** Local draft — gitignored (`.gitignore:36`). Written 2026-07-21
while memtrace was offline; activates when memtrace is live and grok is
present. If handed to a session where either is unavailable, use the
**Degraded modes** section — do not improvise a partial fleet.

**Orchestrator model policy (USER ruling 2026-07-18):** Fable orchestrates and
plans; implementation runs on Opus subagents under Fable's plan; grok reviews
every plan and posts dissent on the fleet log. Dissent is expected input, not
obstruction; unresolved disagreement escalates to USER.

---

## Roles

| Actor | Role | Notes |
|-------|------|-------|
| Claude (Fable) | Orchestrator + Lane A author (via subagents) + reviewer of grok's lanes | `agent_id: "claude"` on ALL fleet calls |
| grok | Symmetric peer: Lane B + Lane D author, reviewer of Claude's lanes | SYMMETRY ruling: both agents implement and review |
| Opus subagents | Implementers inside Claude's lanes; research/spike/gate-runner/skeptic panels | Self-contained briefs per protocol §7; full repo gates before hand-back |
| USER | Final merge gate (unless scoped greenlight); Task 8 live X11 smoke; §5b hard floor | The human holds the gate — dual-approval qualifies, only USER lands |

**Merge authority for this initiative:** default is USER lands every PR. If
USER issues a scoped greenlight ("land the x11 train after dual-agree"), the
reviewer (never the author) merges after ACK-back, per §5. Record the
greenlight verbatim on the initiative thread before acting on it.

---

## Phase 0: Preflight (orchestrator, ~15 min, blocks everything)

- [ ] **Step 1: Memtrace health check** — do NOT skip; split-brain history here.

```bash
systemctl --user status memtrace.service   # must be active
# Verify MEMTRACE_DATA_DIR resolves to ~/.memdb (NOT a phantom repo/.memdb —
# known split-brain failure mode; see memory: memtrace-host-topology)
```

Confirm the scuffed-crew repo is indexed and watch/continuous indexing is on
(memtrace-index / memtrace-continuous-memory skills). If index is stale,
reindex before publishing any intent — blast-radius data from a stale graph is
worse than none.

- [ ] **Step 2: Join the fleet (protocol §1)**

`fleet_branch_context` → join message on `fleet::chat` (kind `intent`) →
presence intent. Create/adopt the initiative thread **`fleet::x11-capture`**.
All review requests / APPROVEs / ACK-backs are **dual-written**: detail on
`fleet::x11-capture`, pointer on `fleet::chat` (dual-channel law, USER
2026-07-19). SSE endpoint is dead — poll the ydoc; ULID-diff both threads
every tick.

- [ ] **Step 3: Confirm grok presence + post the charter**

Charter message on `fleet::x11-capture`: link to the spec, the lane partition
table below, consensus rule (dual-agree, author-never-merges), worktree law,
and this doc's merge order. Wait for grok's ACK before Phase 1 lane B starts;
Claude's own Phase 1 work may begin immediately after posting.

- [ ] **Step 4: Promote the spec so worktrees can see it**

The spec is gitignored → it does not exist in fresh worktrees, so grok cannot
read it. Promotion is the first real commit:

1. Worktree: `.claude/worktrees/claude-x11-plan` off `origin/main`.
2. Remove the spec and orchestration entries from `.gitignore` (the
   `…-review-archive.md` entry STAYS ignored — local review history only),
   move the two promoted files to the worktree, commit as `docs(plans):
   promote x11 capture spec + orchestration plan (review cycle R1–R5)`.
3. Review request to grok (docs-only, light review). Dual-agree → land.
4. Everything downstream branches from a `main` that contains the spec.

---

## Lane partition (publish as intents BEFORE editing)

Conflict-avoidance by construction: each lane owns disjoint files; the one
deliberate overlap is a one-line `pub mod x11;` (noted below). Publish each
lane's file/symbol set via `fleet_publish_intent` with a natural-language
assignment; re-publish on TTL (~120 s) while actively editing; record every
completed edit burst with `fleet_record_episode` and act on its conflict class.

| Lane | Owner | Spec tasks | Files owned | Branch / worktree |
|------|-------|-----------|-------------|-------------------|
| **A: prep refactor** | claude (Opus subagent implements) | Task 1 | `capture/mod.rs` (list_outputs fn only), `capture/wayshot.rs`, `main.rs` (info flags, `log_selected_output`), `gui/main.rs`, `gui/status.rs`, `gui/settings.rs`, `gui/preview.rs` | `x11-prep` / `.claude/worktrees/claude-x11-prep` |
| **B: x11 module** | grok | Tasks 0(confirm)+2+4 | `capture/x11.rs` (new), `Cargo.toml`, `pub mod x11;` line in `capture/mod.rs` | `x11-module` / grok's worktree |
| **C: wiring** | claude (Opus subagent) | Tasks 3+5 | `capture/mod.rs` (enum, `detect_backend`, dispatch, policy tests), `capture/wayshot.rs` (`probe()` [R5]), `gui/status.rs` (label), `main.rs` (verify log) | `x11-wire` / `.claude/worktrees/claude-x11-wire` |
| **D: docs + packaging** | grok | Task 6 | `README.md`, `dist/install.sh`, `dist/bundle-native-libs.sh` (check), `.github/workflows/stat-tracker*.yml` (check), `main.rs` help string | `x11-docs` / grok's worktree |

Notes:
- Lane B's `pub mod x11;` in `capture/mod.rs` is the ONLY sanctioned overlap
  with lanes A/C — a one-line rebase either direction. Declare it in both
  intents so memtrace classifies it Class A/B, not C.
- Lane D's help-string edit in `main.rs` lands AFTER lane A merges (same file);
  sequence it, don't parallel-edit.
- Anything outside a lane's file set = publish a new intent first. No
  spontaneous scope creep; the spec's Non-goals are binding.

---

## Phase 1: Parallel launch (t≈0, three tracks at once)

**Track 1 — Task 0 spike (Claude, subagents, ~1–2 h).** Fan out in parallel:

- [ ] Research subagent 1: verify `x11rb 0.14` APIs — `RustConnection`, RandR
  enumeration, core `GetImage`, byte-order/visual masks, `image` feature
  (`PixelLayout`) usefulness, `shm` feature surface. Deliverable: API map +
  recommended feature set.
- [ ] Research subagent 2: `xcap` current-release Linux dependency audit
  (PipeWire/Wayland/zbus/xcb closure; any X11-only feature). Deliverable:
  accept/reject with evidence. (Expected: reject per spec R1-4.)
- [ ] Build subagent (worktree `.claude/worktrees/claude-x11-spike`): add the
  chosen dep, run spec Task 0 Step 3 verbatim (`cargo tree/check/build`,
  `ldd`), confirm no new libX11/libxcb/PipeWire/DBus in closure. Discard the
  worktree; keep the evidence.
- [ ] **Decision post:** orchestrator writes the Task 0 record (crate, pin,
  features, closure evidence, reasoning) to `fleet::x11-capture`. Grok ACKs or
  dissents. This post is Lane B's start gun and gets pasted into PR-2's body.

**Track 2 — Lane A starts immediately** (no Task 0 dependency): Opus subagent
brief = spec Task 1 verbatim (unified `list_outputs`, root-owned GUI backend
per R3-1, async info flags, R2 portal UX line, "detecting…" pending state per
R4 note). Brief includes worktree setup, IRON LAW, file-ownership list, and
the full gate matrix (below) to run before hand-back.

**Track 3 — grok reads the promoted spec** and preps Lane B (branch, intent,
module skeleton) so implementation starts the moment the Task 0 decision posts.

---

## Phase 2: Implementation lanes (parallel)

- [ ] **Lane A (claude):** Task 1 complete → gates green in worktree → push →
  PR-1 → REVIEW REQUEST on initiative thread (+chat pointer). **Reviewer:
  grok.**
- [ ] **Lane B (grok):** Tasks 2+4 — full `x11.rs`: probe, list, TrueColor-only
  capture policy, pure selection helper + unit tests, no connection cache
  (R3-2), runtime-stable XID fallback naming (R3-5). Compiles and unit-tests
  green with no enum wiring. PR-2. **Reviewer: claude.**
- [ ] **Claude's review of PR-2 is adversarial, not a read-through** (this is
  the code nobody can run against a real X server yet — spec's residual risk):
  - Skeptic panel: 3 parallel Opus subagents, each briefed to REFUTE one lens —
    (1) pixel-format math: masks/byte-order/scanline-pad vs the x11rb API map
    from Track 1; (2) policy conformance: rejects PseudoColor/DirectColor/16-bpp
    et al. with the mandated diagnostic fields, no silent channel swaps;
    (3) selection/order: pure-helper edge cases (missing name, no primary,
    dup/empty names, stable order) actually covered by the tests.
  - ≥1 sustained refutation → findings (≤400 chars + pointers) on
    `fleet::x11-capture`; grok fixes; re-verify only the refuted lens.
  - Panel silence ≠ approval: claude still does the normal protocol review and
    posts the APPROVE personally. Subagents inform the verdict; they don't cast it.
- [ ] Grok reviews PR-1 symmetrically (his method is his own; findings on the
  log either way).
- [ ] **Merge order: PR-1 (prep) first, then PR-2 (module).** After PR-1 lands,
  grok rebases `x11-module` (expected conflict: none or the `pub mod` line).
  Both merges: dual-APPROVE on log → USER (or scoped-greenlight reviewer)
  lands.
- [ ] **Force-push/review race rule** (memory: stats-ui): once a REVIEW REQUEST
  is posted, the author does not force-push that branch; fixups go on top.

---

## Phase 3: Wiring + docs (parallel)

- [ ] **Lane C (claude):** branch from post-PR-2 `main`. Tasks 3+5: enum arm,
  probed `detect_backend` (including `wayshot::probe()` per R5), dispatch (keep
  `&CaptureBackend` — R4 convention is final), backend-aware `list_outputs` X11
  arm, GUI label, fail-closed `STAT_TRACKER_CAPTURE`, pure policy tests (no
  env mutation). PR-3. **Reviewer: grok.**
- [ ] **Lane D (grok), concurrent:** Task 6 — README experimental tier wording
  (R3-6 table is canonical), config table, help string (after PR-1; `main.rs`
  is A-then-D sequenced), packaging/CI check against the Task 0 closure
  evidence. PR-4. **Reviewer: claude** (+ one skeptic subagent cross-checking
  claimed `ldd` closure vs Track 1 evidence).
- [ ] Claude runs a policy-matrix skeptic on PR-3: every row of spec Task 5
  Step 2's selection table exercised by an actual test; refutations to the log.
- [ ] Merge order: PR-3, then PR-4 (docs describe merged behavior). Dual-agree
  + human gate as above.

---

## Phase 4: Verification train (Task 7, on merged main)

- [ ] **Gate-runner subagent** on a fresh `origin/main` worktree — CI-exact
  repo gate (protocol Appendix A) plus the spec's stat-tracker matrix:

```bash
(cd crates/stat-tracker && cargo fmt --check)
cargo build -p scuffed-stat-tracker
cargo build -p scuffed-stat-tracker --features gui
cargo test -p scuffed-stat-tracker
cargo clippy -p scuffed-stat-tracker -- -D warnings
cargo clippy -p scuffed-stat-tracker --features gui --all-targets -- -D warnings
bash scripts/check-design-tokens.sh
# + remaining repo gate per docs/fleet-protocol.md Appendix A
```

- [ ] **Runtime regression (orchestrator, real Wayland session — not
  delegable to a subagent):** spec Task 7 Steps 2–3: `--list-outputs` →
  Wayshot + unchanged outputs; `STAT_TRACKER_CAPTURE=x11` → honored, fail-closed
  non-zero (or XWayland success, documented as not-the-smoke). Daemon Tab
  capture unchanged.
- [ ] **Xephyr nested smoke (optional but recommended):** spec Task 7 Step 4.
  Zero-output Xephyr = probe policy working, not a bug (R2). Post latency
  numbers (median / worst-of-20) to the log — this is the SHM go/no-go data.
- [ ] Post a Task-7 verdict message; record episodes for all landed lanes.

---

## Phase 5: Handoff to USER (orchestration ends here)

- [ ] Post completion summary on `fleet::x11-capture` + `fleet::chat`: what
  landed (PR list), gate results, latency numbers, residual risk (no pure-X11
  validation yet).
- [ ] Hand USER the spec's **Task 8 checklist A–F** verbatim, with: exact
  binary/branch to run, `STAT_TRACKER_CAPTURE=x11` usage, what to record
  (DE, GPU, monitors, latency, X11 errors).
- [ ] Prepared-but-unlanded: the README promotion commit (experimental →
  validated/supported per R3 tier table) drafted and parked; lands only after
  USER reports A–D / E–F results. **Task 8 and the §5b hard floor are
  human-only. Do not simulate, Xephyr-substitute, or claim them.**

---

## Quality harness (summary)

| Layer | Mechanism |
|-------|-----------|
| Spec fidelity | Implementation briefs quote spec tasks verbatim; deviations need an [R6+] plan amendment posted to the log first |
| Per-lane gates | Full CI-exact matrix in the lane worktree BEFORE review request — reviewers review working code, not intentions |
| Peer review | Symmetric dual-agree, author never merges own branch, dual-channel messages, ≤400 chars + pointers |
| Adversarial depth | Skeptic subagent panels on the two nobody-can-run-it surfaces: pixel conversion (PR-2) and selection policy (PR-3); refute-oriented briefs |
| Merge safety | Strict merge order (PR-1→2→3→4); no force-push after review request; episodes recorded on land |
| Memtrace | Intents before edits, episodes after, Class C → mediation flow, preflight/impact checks on `capture/mod.rs` edits (hottest shared file) |
| Human gate | USER lands merges (or scoped greenlight, recorded verbatim); Task 8 human-only |

## Timeline target

| Phase | Wall clock | Parallelism |
|-------|-----------|-------------|
| 0 preflight + promote | ~0.5 h | — |
| 1–2 spike + lanes A/B + reviews | ~4–6 h | 3 tracks, then 2 lanes + cross-review |
| 3 lanes C/D + reviews | ~2–4 h | 2 lanes |
| 4 verification | ~1–2 h | gates ∥ runtime checks |
| **Total to Task-7-green** | **~1 working day** | vs 2–3.5 days serial |

Timeline is a target, not a gate — never trade a review round for the clock.
If a lane stalls >2 h with no fleet message, orchestrator pings the thread;
protocol §6 backoff/liveness rules apply.

## Degraded modes

| Condition | Mode |
|-----------|------|
| Memtrace down at start | Do not start the fleet flow. Either wait, or run SOLO: same branches/merge order, reviews via `gh` PR comments, USER approves each merge (this session's earlier solo pattern). Cross-review protocol still binds — grok reviews via gh if reachable. |
| Memtrace dies mid-flight | git/gh outranks the fleet log (memory rule). Freeze intents, finish open PRs via gh review flow, note the outage on the log once it returns. |
| grok absent | Claude may author all lanes, but SYMMETRY's spirit holds: every PR still gets an independent adversarial subagent review + USER approval. No self-merge, ever. §5b hard floor unchanged. |
| Class C conflict | Stop both lanes' edits on the contested symbols, run the mediation flow (fleet-resolve), escalate to USER if verdicts split. |
| Skeptic panel and grok disagree with each other | Orchestrator does NOT tiebreak silently: post both positions to the log; USER decides. |

## Success criteria

1. Spec Tasks 0–7 all checked; every land dual-agreed; zero IRON-LAW
   violations; zero chat-only verdicts.
2. `main` CI-green; Wayland behavior regression-free; forced-X11 fail-closed
   verified.
3. README says **experimental** (tier 1) — nothing stronger until USER's
   Task 8 results.
4. Fleet log tells the whole story: a reader can reconstruct every decision,
   review, and land from `fleet::x11-capture` + git alone.
