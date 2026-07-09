# Cross-review: Fable → Grok Phase-2 work

**Date:** 2026-07-10 · **Reviewer:** Fable · **Target:** uncommitted Phase-2 diff in the working tree on `stat-tracker/phase-2` (the `stat-tracker/phase2-deferred` branch carries no commits beyond main — the diff itself is the review target). Fable's own Phase-2 contributions (debug-OCR wiring, P11 trailing/shutdown flushes, `parse_lenient` tidy) are excluded.

**Verdict: APPROVE after 1 confirmed-regression fix (applied by Fable, see below).** Everything else verified clean: compiles with `--features gui`, clippy 0 warnings, 30 lib + 9 bin tests green.

## Confirmed regression — P8 brightness pre-gate breaks accolade defeat detection

`outcome_fixture_replay` **fails** on this diff: `defeat_accolade_01.png: expected Defeat, got Unknown`.

Root cause, measured on the fixture set (sampled every 4th pixel, per the gate):

| Frame | Crop | ratio r+g+b>480 | ratio max-ch>200 |
|---|---|---|---|
| defeat_accolade_01 (REAL DEFEAT) | accolade | **0.006** | 0.233 |
| none_ingame_01 | accolade | 0.148 | 0.049 |
| none_transition_01 | accolade | 0.215 | 0.188 |
| defeat_rank_01/02 (REAL) | rank | 0.140 | 0.135 |
| none_ingame_01 / none_transition_01 | rank | 0.000 | 0.000 |

Two conclusions:
1. **The near-white test (`r+g+b > 480`) is color-scheme-dependent** — the user's magenta UI theme renders DEFEAT with a low channel *sum* but a high *max channel*. `read_result_word`'s own doc comment says it was validated on that magenta theme, and the Otsu pipeline it guards was chosen precisely for color independence. The gate silently undid that.
2. **Brightness cannot separate "title present" from "bright game world" at any threshold** — the negative in-game/transition frames light up 15–22%, *more* than the real defeat frame's 0.6%. The gate's premise ("most idle ticks are near-black in this crop") is false for the accolade crop. What IS reliably separable: near-black crops (the rank crop is exactly 0.000 on in-game/transition frames vs ≥0.135 on real rank screens).

**Fix (applied by Fable in `detect/match_end.rs`):** re-scope the gate to "skip only near-black crops" using max-channel > 200 at a 1% ratio — 13–23× margin below every real title, still skips the rank-screen OCR on ordinary gameplay/transition ticks (~1 of the 2 idle Tesseract calls). The accolade-crop OCR keeps running on bright frames; that cost cannot be gated on brightness.

## Verified clean (the rest of the diff)

- **P6 single-RGB-per-poll** — `detect_phase_with_rgb` / `detect_outcome_signal_with_rgb`, one `to_rgb8` per poll tick in the spawn_blocking closure; stride-2 subsampling with numerator and denominator sampled identically (ratios unbiased). Old signatures kept as wrappers — `examples/polltick.rs` and the fixture tests still compile. `step_x/step_y` `.max(1)` guards divide-by-zero on small frames.
- **P7 pass-crop/team-size-once** — `recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size)` + `match_player_hero_with_team_size`; `handle_capture` now crops once and detects team size once. Old entry points preserved for examples/tests/replay-benchmark.
- **A8 letterbox routing** — outcome word, accolade map, banner band, and `detect_outcome_text` now crop via `game_rect_16_9`. On 16:9 this is identity (fixtures unaffected); on ultrawide/16:10 it fixes the silent degradation flagged in the original review. Flood tests still pass.
- **GUI MatchOutcome adoption** — `parse_lenient` + `row_class`/`text_class` helpers; history/status/stats.rs literals gone; `MatchOutcome` is `Copy + PartialEq` so the by-value GUI use compiles; buttons emit `MatchOutcome::X.to_string()` so only canonical spellings are ever written.
- **P11 core** — debounced `refresh_snapshot` (2s) + forced export after sync. The two gaps (trailing dirty never flushed; shutdown could drop the final state) were Fable-side completions, not defects in the committed intent. Residual: a mutation landing between an in-flight export's data read and its dirty-clear can leave a stale snapshot until the next mutation/sync — self-heals within seconds, accepted.
- **P13 micro** — `encode_png` by-ref (all 4 callers updated), `tessdata_lang` OnceLock, `detect_column_offset` converts only the header strip. Note: OnceLock means tessdata generated mid-process (GUI button) isn't picked up until restart — cosmetic, GUI preview only.

## Verification run

- `cargo test -p scuffed-stat-tracker --features gui` → 30 lib + 9 bin green; clippy 0 warnings.
- `outcome_fixture_replay` → **fails before the P8 fix, passes after** (see regression section).
