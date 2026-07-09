# Cross-review: Grok → Fable Phase-1 lane (`stat-tracker/fix-sync-contract`)

**Date:** 2026-07-09 · **Reviewer:** Grok · **Target:** worktree  
`/home/soot/github/scuffed-crew/.claude/worktrees/sync-contract`  
branch `stat-tracker/fix-sync-contract` (5 commits ahead of `stat-tracker/add-neon-junction-aatlis`).

**Verdict: APPROVE with 2 pre-merge fixes and a short merge checklist.**  
Boundary mostly clean; workplan Fable steps 1–5 are substantially complete; tests for the sync contract and session FSM are strong. No correctness red flags that should block merge after the two small fixes.

---

## Where this lives in the workspace

Yes — Fable’s lane is in-repo as a **git worktree**, not only a remote branch:

| | |
|--|--|
| Path | `.claude/worktrees/sync-contract` |
| Branch | `stat-tracker/fix-sync-contract` |
| Tips | `12c78cc` sync → `564cc41` enablement → `fbbc481` session → `ef25858` map/hero → `5f30856` un-serialize loop |

Main tree currently sits on `stat-tracker/perf-gui-lifecycle` (Grok). That is why the Fable diff is not mixed into the working tree unless you `git worktree` / checkout that branch.

---

## Boundary compliance

| Area | Expected owner | Status |
|------|----------------|--------|
| `main.rs`, `parse.rs`, `sync/`, `storage/` | Fable | ✅ Heavy, appropriate |
| `detect/mod.rs` (`MatchOutcome`) | Fable | ✅ Typed Display/FromStr/serde |
| `crates/types` stats API, `crates/db`, site-server stats | Fable | ✅ |
| `src/gui/**`, `ocr/**`, `hero_portrait.rs` geometry | Grok | ✅ Untouched |
| `src/capture/**` | Grok | ⚠️ **Tiny leak:** `CaptureBackend` gained `Clone + Copy + PartialEq + Eq` only — needed for spawn/single-flight. Acceptable; note for rebase. |

Outcome strings in GUI left alone (Phase 2) — Fable did not “fix” GUI literals. ✅

---

## Item-by-item vs workplan / `review.md`

| Workplan step | Status | Notes |
|---------------|--------|--------|
| **1. Sync contract (C1/C2/M4/M7)** | **Done well** | `session_id` on upload; server `upsert_personal_matches`; drop `pm_dedup_idx`; filter `unknown` client-side (`get_unsynced`) **and** server-side (skip unstorable outcomes); `mark_synced` by record ids; HTTP 30s timeout; sync as single-flight spawned task; client collapses to `latest_per_game` before POST. In-memory DB tests cover collapse, correction in place, legacy content dedup, member scoping. |
| **2. Thin enablement (MatchOutcome + FrameAnalysis)** | **Done** | Canonical string path via Display/FromStr; `FrameAnalysis` replaces 9-tuple; still many args on `handle_capture` / `run_loop` (full `DaemonCtx` correctly deferred). |
| **3. Session boundaries (M1/M5/M6/M10 + m1)** | **Done well** | `UNFINISHED_SESSION_IDLE` 20m; `active_game.json` persist/recover with wall-clock + idle bound; pending cleared on poller open; streak reset on new game; Tab banner gated by `MIN_BANNER_SESSION_AGE` / fresh Tab session; `create_session` UPSERT (half-failure dup mitigated). Zero-output panic: capture path uses `.first()` style via Fable’s spawn path — confirm outputs empty path if not already. |
| **4. Map-vote + Ana/HAVANA (M2/M3)** | **Done well** | Vote → candidates only + `canonical_map`; `resolve_map` never picks `candidates[0]`; hero word-boundary for short names; tie-break most-mentioned; unit tests. |
| **5. Un-serialize loop + lazy full OCR (P1/P3)** | **Done well** | Tab capture single-flight spawn + channel; 400ms sleep inside task; capture completion arm updates only matching `session_id`; full-board `recognize()` only when cells/career/map incomplete. |

---

## Strengths (do not undo)

- Sync redesign matches the review’s “snapshot ≠ game” diagnosis: one server row per session, corrections update in place, unknown cannot HOL-block.
- Session FSM comments and tests encode the real scars (grace, idle, pending TTL, vote candidates).
- Poller clears `pending_outcome` + `word_outcome_streak` when opening games — closes the leak Grok flagged.
- Mid-game Tab banner guard is the right fix for red-vignette FPs without killing post-match recovery.
- Server filters bad outcomes instead of 500ing the batch — defense in depth if an old client still uploads `unknown`.
- Tests: 5 `scuffed-db` personal_stats upserts; 9 daemon FSM tests in `main.rs`; parse hero/map tests. All re-run green in this review.

---

## Pre-merge fixes (Fable should apply)

### 1. Wire `portrait_rect` into `--collect-portraits` (workplan A2 half)

`handle_capture` still hard-codes 5v5-only geometry:

```1146:1154:.claude/worktrees/sync-contract/crates/stat-tracker/src/main.rs
            let portrait_w = sw * 6 / 100;
            ...
            let row_height = sh * 7 / 100;
            let start_y = sh * 12 / 100;
            let row_y = start_y + player_row_idx.unwrap_or(0) as u32 * row_height;
```

Grok exported `detect::hero_portrait::portrait_rect` for exactly this. After rebase onto Grok (or if already merged Grok→base), replace with:

```rust
if let Some(r) = detect::hero_portrait::portrait_rect((sw, sh), player_row_idx.unwrap_or(0), team_size) {
    let crop = scoreboard_img.crop_imm(r.x, r.y, r.w, r.h);
    ...
}
```

Leaving this unmerged re-poisons 6v6 / team-2 auto-collected templates.

### 2. Prefer failing closed on HTTP client builder (nit, but real)

```rust
.unwrap_or_else(|e| {
    tracing::warn!(...);
    reqwest::Client::new() // no timeout
});
```

If the builder ever fails, you silently lose the M4 timeout that the whole design depends on. Prefer `expect` / return error from `SyncClient::new`, or reuse a once-built timed client only.

---

## Notes / residuals (not merge blockers)

| Topic | Assessment |
|-------|------------|
| **Delete still local-only** | Still true (`delete_session` doc). Review P0 was insert/upsert/wedge; cloud delete is later. OK. |
| **Legacy `win`/`loss` FromStr** | `MatchOutcome` is strict; GUI still has legacy strings until Phase 2. Local store writes `to_string()` → victory/defeat. Fine. |
| **Surreal `UPSERT … WHERE`** | Unusual vs record-id upsert; **but** Fable’s five in-memory DB tests prove collapse + correction + multi-member. Accept. |
| **`mark_synced` empty ids** | Relies on SELECT populating `PersonalMatch.id`. If a row ever comes back without id, it never marks synced and re-uploads forever (harmless under upsert, noisy). Worth a debug assert/log if `ids.len() != unsynced.len()`. |
| **CaptureBackend Clone** | Boundary micro-leak; keep when Grok rebases. |
| **Full DaemonCtx** | Correctly deferred (P2). |
| **set_debug_ocr / data_dir for OCR dumps** | Fable’s own merge note from Grok review — still open on this branch; can land when wiring debug or in Phase 2. |
| **Suspend-blind Instant** | `active_game.json` uses wall clock for recovery; in-process windows still Instant. Residual m5. |

---

## Verification run (this review)

From `.claude/worktrees/sync-contract`:

- `cargo test -p scuffed-db --lib personal_stats` → **5 passed**
- `cargo test -p scuffed-stat-tracker --lib` + bin tests → **24 lib + 9 main passed** (observed)

---

## Merge protocol reminder (workplan)

1. ✅ Fable → Grok Phase-1 review (done; 3 fixes applied on Grok branch)  
2. ✅ Grok → Fable Phase-1 review (this document)  
3. **Merge Fable first** (`fix-sync-contract`)  
4. **Rebase Grok** (`perf-gui-lifecycle`) — expect trivial conflicts on `lib.rs` / none on `main.rs`; keep Fable’s `CaptureBackend` derives if Grok’s wayshot still needs them  
5. Fable applies pre-merge #1 (`portrait_rect`) on the merged tree (or Grok after rebase if Fable prefers — geometry call site is `main.rs` so **Fable**)  
6. Checkpoint full suite + fixture replays  
7. Phase 2  

---

## Bottom line

Fable’s lane delivers the **correctness-critical** half of Phase 1 at high quality: sync contract, session FSM, map-vote/Ana fixes, and the un-serialized capture loop are the right design and are tested.  

**Approve to merge** after:

1. Wiring `portrait_rect` for portrait collection, and  
2. (Optional but cheap) hard-fail timed HTTP client construction.

Then rebase Grok (already carrying Fable’s three requested GUI/OCR fixes) and only then start Phase 2.
