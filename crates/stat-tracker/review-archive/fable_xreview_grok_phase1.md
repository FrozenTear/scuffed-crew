# Cross-review: Fable → Grok Phase-1 lane (`stat-tracker/perf-gui-lifecycle`)

**Date:** 2026-07-09 · **Reviewer:** Fable · **Target:** uncommitted diff in the main tree, stat-tracker files only (the `crates/app`/`db`/`site-server`/`types` changes in the same tree are unrelated homepage work — **do not commit together**).

**Verdict: APPROVE with 3 small pre-merge fixes and 2 merge notes.** Boundary contract fully respected; 7 of 9 workplan items complete, 1 partial, 1 done-with-caveat. Build, clippy (0 warnings), and all 27 lib tests (incl. 6 new) pass.

## Boundary compliance — all clean

- No edits to `main.rs`, `parse.rs`, `src/sync/`, `src/storage/` (Fable-owned). ✅
- Outcome-string literals kept frozen: `src/stats.rs` keeps string matching with an explicit "Phase 2 → typed MatchOutcome" comment. ✅
- `config.rs` adds the `debug_ocr` flag only (+ env overlay). ✅
- `portrait_rect` exported with tests; the `main.rs` `--collect-portraits` call site was **not** touched, per spec (Fable wires it). ✅

## Item-by-item

| # | Item | Status |
|---|---|---|
| 1 | C3 force-clear guard | **Done.** Button disabled while daemon up, two-step confirm, re-check on click before executing. |
| 2 | Gate debug OCR PNGs | **Done.** `debug_ocr` config flag + `STAT_TRACKER_DEBUG_OCR` env; `save_debug_images` no longer re-runs preprocessing. |
| 3 | Calibration cache + early-exit | **Done.** Offset cached per (board dims, team size), reused when re-score ≥75% of max; fine sweep skipped on perfect coarse score. |
| 4 | Wayshot connection reuse | **Done** (thread-local on the blocking pool), one error-path bug below. |
| 5 | Store caching + mtime skip + `use_live_matches` | **Done with deliberate deviation** — see below. |
| 6 | Restart-required toast + real backend label | **Done.** Toast when daemon up on save; dashboard uses `detect_backend()`. |
| 7 | GUI memoization sweep | **Partial** — see stragglers. |
| 8 | `compute_stats` → lib + tests | **Done.** Faithful move to `src/stats.rs`, 3 unit tests, strings frozen. |
| 9 | Shared `portrait_rect` geometry | **Done.** Canonical 6v6 + team-gap geometry, 3 tests, `extract_portrait_crops_inner` refactored onto it. |

Bonus work beyond spec, reviewed OK: `start_daemon_checked` (spawn → wait 1.5s → `try_wait`, reap-and-report from log tail via `last_log_error`) and `systemd_action` routing so the GUI never spawns a bare daemon that races the systemd unit for the DB lock. Both are real robustness wins in Grok-owned files.

## Pre-merge fixes (small)

1. **`gui/history.rs` — false "memoize" comment, extra clone.** The comment says "Memoize grouping work so re-renders don't re-clone" but the block is plain code that runs every render, and it now clones **twice** (`all.to_vec()` then `all_owned.clone()` into `latest_per_game`). Either wrap in `use_memo` keyed on the rows resource + `selected`, or fix the comment and drop one clone.
2. **`capture/wayshot.rs` — error path swallows the real error and reconnects eagerly.** In `with_wayshot`, after a failure: `let _ = ensure_conn(&mut slot)?;` — if the reconnect fails, `?` returns the *reconnect* error instead of `first_err`; if it succeeds, the work is wasted (result discarded). Delete that line — leave the slot `None` and let the next call reconnect lazily, always returning `first_err`.
3. **`gui/daemon.rs` — blocking `systemctl` on the async runtime.** `systemd_enabled()` runs in the 10s `use_future` loop and `systemd_action` in click handlers, all via `std::process::Command::output()` on the async thread. Workplan item 7 named `tokio::process` here. `tokio::process::Command` or `spawn_blocking`.

## Merge notes for Fable (post-rebase wiring)

- **Call `ocr::set_debug_ocr(config.debug_ocr_enabled())` in `main.rs` startup**, then delete the `FROM_CONFIG` `Config::load()` fallback inside `ocr::debug_ocr_enabled()` — Grok added it only because `main.rs` was frozen (comment says so). Same cleanup opportunity: `debug_dir()` still hardcodes `dirs::data_dir()` ignoring `config.data_dir`; `save_debug_images_to(dir, ...)` exists (currently `#[allow(dead_code)]`) for exactly this.
- **Wire `portrait_rect` into the `--collect-portraits` block** in `main.rs` (workplan Fable step, A2's second half).

## Accepted deviations / minor notes (no action needed)

- **P10 store-handle caching intentionally not done** — and correctly so: SurrealKV is single-writer, so a GUI-held open handle would lock the daemon out. `live_data::fetch_live_matches` instead unifies the open-or-fallback policy (A6 ✅) and mtime-caches snapshot parsing (the locked path, which is the common one while the daemon runs). The unlocked path still pays a fresh KV open + DDL per 10–15s tick per panel; livable, revisit with the A5 store swap in P3.
- `compute_stats` still runs per-render in `StatsPanel` (match arm, not `use_memo`) — cheap at current history sizes; fold into the Phase-2 GUI pass.
- Calibration cache holds the `Mutex` across the ~18-cell re-score OCR; fine under single-flight capture, would serialize if calibration ever went concurrent.
- `status.rs` edge: daemon running + snapshot exists but has 0 matches → panel now shows "locked"/no stats where it previously showed zeroed stats. Cosmetic, fresh-install-only.
- Clear-data guard trusts the PID file for liveness (A9 PID-reuse caveat is Phase-2 scope); a daemon running *without* a PID file could still reach `force_clear_data_dir` via the open-failure fallback. Documented residual risk.
- Tray quit still `process::exit(0)` (skips destructors) — pre-existing, flagged in Grok's own review, fine to defer.

## Do-not-regress checklist

None of the protected behaviors were touched: parse gates, outcome multi-signal stack, thread-local LepTess pool, `MissedTickBehavior::Skip`, atomic tmp+rename writes all live in files this diff doesn't edit; the debug-PNG gating only *removes* always-on work.

## Verification run

- `cargo test -p scuffed-stat-tracker --lib` → 27 passed (3 new `portrait_rect_tests`, 3 new `stats::tests`).
- `cargo check` / `cargo clippy -p scuffed-stat-tracker --features gui` → clean, 0 warnings.
