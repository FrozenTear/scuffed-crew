# Cross-review: Grok → Fable Phase-2 contributions (`stat-tracker/phase-2`)

**Date:** 2026-07-10 · **Reviewer:** Grok · **Target:** uncommitted tree on `stat-tracker/phase-2`  
**Note on “Fable’s branch”:** There is **no separate Fable Phase-2 git branch**. Phase 2 landed as **shared uncommitted work on `stat-tracker/phase-2`** (same tree Grok wrote into). Fable’s own claims are recorded in Serena `phase-2-status.md` / Claude `stat-tracker-fable-lane.md`. This review attributes **Fable-specific additions** and spot-checks the **combined** Phase-2 tree for regressions.

**Verdict: APPROVE** Fable’s Phase-2 slice. No blocking issues. One small note on debounce first-export behaviour; one residual still open (`debug_dir` hardcodes `dirs::data_dir()`).

---

## Where to find it

| What | Where |
|------|--------|
| Branch | `stat-tracker/phase-2` (tip = local `main` + uncommitted Phase 2) |
| Separate Fable worktree for P2 | **None** (Phase-1 worktree `.claude/worktrees/sync-contract` is stale post-merge) |
| Fable self-report | `.serena/memories/phase-2-status.md` |

---

## Attribution (from Fable’s status + diff inspection)

### Fable-owned Phase-2 work (this review’s focus)

1. **Debug-OCR wiring (merge note from Phase-1 review)**  
   - `main.rs`: `ocr::set_debug_ocr(config.debug_ocr_enabled())` after config load / `create_dir_all`.  
   - `ocr::debug_ocr_enabled()`: removed `FROM_CONFIG` / `Config::load()` fallback; keeps atomic + env `STAT_TRACKER_DEBUG_OCR`.

2. **P11 completion (debounce trailing edge + shutdown)**  
   - `flush_snapshot_if_due` on empty `cmd_timer` ticks (3s).  
   - `flush_snapshot_if_dirty` on Ctrl-C and SIGTERM **after** final `try_sync`.  
   - Complements Grok’s core `SNAPSHOT_STATE` + `refresh_snapshot` / `_force`.

3. **`parse_lenient` tidy**  
   - Dead fallthrough (`other.parse().unwrap_or(Unknown)`) collapsed to `_ => Unknown` with a clear comment.

### Grok-owned Phase-2 work (context only — already self-tested)

P6 `_with_rgb` + stride, P7 pre-cropped OCR / portrait team_size, P8 title brightness gate, A8 `game_rect_16_9` on outcome crops, GUI MatchOutcome adoption, P13 encode/tessdata/column-offset, P11 core debounce.

---

## Item-by-item (Fable)

| Item | Status | Notes |
|------|--------|--------|
| Wire `set_debug_ocr` from daemon | **Done well** | Exactly the Phase-1 merge note; single startup call; env still works for GUI/examples without main. |
| Drop `FROM_CONFIG` load | **Done well** | Avoids hidden `Config::load()` on OCR hot path / wrong cwd. |
| P11 trailing-edge flush | **Done well — necessary** | Without `flush_snapshot_if_due`, a capture that only dirties inside the 2s window could wait until the *next* mutation forever. 3s cmd timer is a reasonable carrier. |
| P11 shutdown flush | **Done well — necessary** | `try_sync` only force-exports on **success**; dirty local state after a failed sync / mid-window mutations would be lost without `flush_snapshot_if_dirty`. Order (drain capture → sync → flush) is correct. |
| `parse_lenient` tidy | **Done** | Correct and clearer; behaviour = always Unknown for garbage. |

---

## Correctness notes (combined P11)

Debounce model:

```
refresh_snapshot      → dirty=true; export if force or last is None or last ≥ 2s
flush_snapshot_if_due → if dirty && (no last || last ≥ 2s) → force export
flush_snapshot_if_dirty → if dirty → force export (shutdown)
refresh_snapshot_force → force export (post-sync success)
```

- **First dirty after boot** (`last = None`): non-force export still runs (early-return needs `Some(last)`). Fine — GUI gets an initial update quickly.  
- **Burst of mutations**: collapses to ≥1 export / 2s + trailing edge on empty cmd ticks. Fine.  
- **cmd tick with commands**: applies commands, then `refresh_snapshot` (not `_force`); if still inside window, dirty stays and next empty tick flushes. Fine.  
- **Poisoned mutex**: `unwrap_or_else(|e| e.into_inner())` on the read paths is pragmatic; export path uses `if let Ok` for the clear — acceptable.

No issues found that should block merge.

---

## Residuals (not Fable regressions)

| Topic | Status |
|-------|--------|
| `debug_dir()` still hardcodes `dirs::data_dir()/scuffed-stat-tracker/debug` | Still open (Phase-1 merge note). Config `data_dir` not used for dump path. Low priority; track post-merge. |
| A1 `DaemonCtx` / A4 `anyhow` | Explicitly deferred — agree. |
| GUI `SetOutcome` uses strict `.parse()` in main | GUI sends `MatchOutcome::to_string()` (`victory`/`defeat`/`draw`) — OK. |
| P8 / A8 outcome path | Touches “do not regress” zone; **must** run `outcome_fixtures` before merge (Fable already flagged). Grok re-ran unit banner floods: still green. |

---

## Boundary

Fable stayed in Fable-owned files for *their* delta (`main.rs`, `ocr/mod.rs` switch wiring, `detect/mod.rs` parse tidy). Shared tree also contains Grok edits in detect/ocr/gui — expected for Phase 2 cohabitation; commit split by file ownership still makes sense if you want clean history.

---

## Verification (this review)

On the combined uncommitted `stat-tracker/phase-2` tree:

| Check | Result |
|-------|--------|
| `cargo test -p scuffed-stat-tracker --lib` | **30 passed** |
| `cargo test -p scuffed-stat-tracker --bin scuffed-stat-tracker` | **9 passed** |
| `cargo clippy -p scuffed-stat-tracker --features gui -- -D warnings` | **clean** |

**Not run here:** `outcome_fixtures` / `ocr_replay_benchmark` (`#[ignore]`, need local screenshots) — still the right merge gate for P8/A8.

---

## Verdict

**Approve Fable’s Phase-2 contributions.** The debug-OCR startup wire is the right fix for the Phase-1 merge note; the P11 trailing-edge + shutdown flushes are the missing half of debounced snapshots and should ship with Grok’s debounce core.

### Pre-merge checklist (whole Phase-2 tree)

1. Run `outcome_fixtures` replay if fixtures exist (P8/A8).  
2. Optional: commit split — Grok files vs Fable (`main.rs` snapshot helpers + `set_debug_ocr` + ocr switch + parse_lenient tidy).  
3. Merge `stat-tracker/phase-2` → main when ready.  
4. Later: `debug_dir` → `config.data_dir`; A1/A4.

---

*Cross-review of Fable Phase-2 slice; Grok Phase-2 bulk already verified green in-session. Prefer this file when reconciling “who fixed P11 completeness”.*
