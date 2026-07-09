# Stat Tracker — Deep Review (canonical backlog)

**Date:** 2026-07-09 · **Branch:** `stat-tracker/add-neon-junction-aatlis` · **Scope:** `crates/stat-tracker` (+ server sync surface)

**Status:** This is the **single working implementation backlog**. Do not implement from the archived source reviews.

**Lineage:**
- Independent reviews: `review-archive/fable_review.md` (Claude Fable 5, four deep-dives, Serena-verified), `review-archive/grok_review.md` (Grok, five agents)
- Intermediate merges: `review-archive/fable_grok.md`, `review-archive/grok_fable.md`
- This file is the post-reconciliation merge of both, with agreed revisions applied

**Method:** Deduplicate, re-rank, and reconcile both reviews. Prefer Fable on data-path / state-machine / sync correctness (including cases where Grok graded intentional design that is actually wrong). Prefer Grok on GUI lifecycle, ops safety, and product footguns. Shared findings are stated once at the highest justified severity.

**Revision (agreed by both reviewers):** (1) command-queue delete-then-apply and stale-JSONL-on-backfill moved from perf P12 to A5 as integrity holes; (2) `ActiveGame`-lost-on-restart promoted from m12 to **M10** and to P1; (3) refactor sequencing reconciled — P0 stays refactor-free, a thin typed-`MatchOutcome` + `FrameAnalysis` slice lands early in P1 as enablement, full `DaemonCtx`/God-function split stays P2.

---

## Executive summary

The crate has genuine production bones: clear top-level flow (capture → detect/OCR → parse → storage → sync), careful session/outcome domain logic with real scars encoded as tests and comments, unit tests on the riskiest pure logic, and a solid fixture/replay tooling set.

Problems cluster in five places:

1. **Sync is broken end-to-end** — every Tab snapshot counts as a separate server match; `unknown` and failed index handling can wedge the unsynced queue permanently; corrections/deletes do not round-trip.
2. **The daemon `select!` loop serializes everything** — Tab OCR can starve the poller for 45–70s (measured), which is the root cause of missed outcome banners that the confirm-window machinery papers over.
3. **The OCR hot path does enormous unnecessary work** — ungated debug PNGs that re-run preprocessing, always-on full-image fallback OCR, ~300 calibration OCRs per Tab with no early exit/cache.
4. **Session / map / hero residual correctness holes** — unfinished sessions never expire; map-vote candidates are recorded as the played map; Ana⊂HAVANA substring trap; pending-outcome and related leaks.
5. **GUI/daemon lifecycle footguns** — force-clear can unlink the live SurrealKV while the daemon holds it; PID checks are identity-weak; settings do not hot-reload.

**Highest leverage (merged order):**

1. Fix the sync contract (session upsert, filter `unknown`, structural `IndexExists`, timeouts, off-loop sync).
2. Never force-delete the live DB while the daemon is running; safer stop/PID handling.
3. Un-serialize the daemon loop (spawn Tab OCR; free the poller).
4. Gate debug PNGs, lazy full OCR, cache/shrink calibration.
5. Unfinished-session TTL, map-vote as candidates only, Ana word-boundary fix, portrait geometry dedup.
6. A **thin enablement slice** — typed `MatchOutcome` + a named struct for `handle_capture`’s 9-tuple — lands *before* the multi-site session/map-vote work in `main.rs` (it makes those fixes compiler-checked). The P0 items above need no refactor and should not wait on one; the full `run_loop`/`DaemonCtx` split stays P2.

---

## Crate shape

`scuffed-stat-tracker` — two binaries (daemon + optional Dioxus desktop GUI behind `gui`) sharing a lib. ~7,600 lines in `src/`, ~1,000 in `tests/`, ~650 in `examples/`. Dioxus 0.7 desktop, tokio, SurrealDB v3 / SurrealKV, reqwest sync, leptess on a capped rayon pool.

```
┌─────────────────────┐     live_snapshot.json      ┌──────────────────────┐
│  GUI (dioxus)       │◄──────────────────────────│  Daemon (main loop)    │
│  SurrealKV if free  │── commands/*.json ────────►│  owns SurrealKV       │
└─────────────────────┘                            │  Tab + poll + sync    │
                                                   └──────────┬───────────┘
                                                              │ HTTPS bearer
                                                              ▼
                                                       /api/stats/*
```

| Module | Lines (approx.) | Responsibility |
|--------|-----------------|----------------|
| `src/main.rs` | ~1.1k | Daemon CLI, PID guard, `ActiveGame` FSM, `run_loop`, `handle_capture`, sync helpers |
| `src/ocr/` | ~1.3k | leptess wrapper, cell OCR, calibration, preprocessing |
| `src/detect/` | ~1.2k | Banner/outcome, map-vote/ban/select, `/proc` gate, portraits |
| `src/parse.rs` | ~0.75k | OCR rows → `PersonalMatch`, trust gates (well tested) |
| `src/storage/` | ~0.6k | SurrealKV, snapshot, jsonl, command queue |
| `src/gui/` | ~2.7–2.8k | Status/stats/history/settings/preview/tray |
| `src/sync/` | ~0.1k | Upload matches, fetch daemon config |
| `src/capture/` | small | wayshot preferred, portal fallback |
| `src/setup.rs` | ~0.55k | Tessdata training pipeline |

**Threading model (sound overall):** single-threaded async `select!` owns store + session; capture/vision/OCR in `spawn_blocking`; OCR on capped Rayon (2–4 threads); thread-local `LepTess` reuse.

SurrealKV is single-process, so daemon↔GUI uses three side channels: JSON snapshot, JSONL log, polled file command queue.

---

## What is already good (do not casually undo)

Both reviews agree the vision kernel is product-shaped, not a prototype:

- Session constants and asymmetric evidence: `POST_MATCH_GRACE` (75s), `OUTCOME_CONFIRM_WINDOW` (60s, two agreeing word reads), `PENDING_OUTCOME_TTL` (90s), map-vote debounce, banner immediate vs word needs 2×.
- Old “always victory” stale-`active_game` class substantially fixed (idempotent outcome recording, finished games force fresh past grace, vote-debounce, “king” map trap tests). Residuals remain below.
- Parse trust: refuse unidentified player rows; `looks_like_scoreboard`; kill-column plausibility; no silent 0-defaults on unreadable cells; no regex (strsim only).
- Portrait path refuses “best teammate” fallback; hero priority career > portrait > text; majority hero across multi-capture sessions.
- OCR infra: thread-local Tess reuse, capped pool, per-cell ROI + numeric whitelists, cells skip full Sauvola, `game_rect_16_9` for scoreboard geometry, flag-gated poll-frame dumps.
- Local storage: indexes, atomic snapshot tmp+rename, command queue, append-only jsonl dual-write.
- Operator tooling: `examples/{extract,accolade,polltick,profile,probe_outcome,dumpdb}`, rejected-frame archive, fixture harnesses (`#[ignore]`).
- Narrative comments explaining *why*; no `unsafe` found in this crate; correct SurrealDatetime usage for Surreal v3.

---

## Critical — correctness

### C1. Server sync counts every Tab snapshot as a separate match

`main.rs` inserts one `personal_match` per Tab; `get_unsynced` returns all; `upload_matches` sends all. `StatsUploadEntry` has **no `session_id`**. Server dedup `(member_id, hero, map_name, played_at)` never collapses multi-Tab games because each snapshot stamps a distinct `played_at` at parse time. Local GUI collapses via `latest_per_game`; server does not.

**A game with 6 Tabs = 6 wins/losses in cloud stats.**

**Fix:** include `session_id` (or stable client id) in upload + server per-session upsert; or upload only the final snapshot on session close + shutdown.

### C2. Sync wedges permanently — multiple independent triggers

- **`unknown` outcomes violate server ASSERT** (`outcome IN ['victory','defeat','draw']`). Mid-game sync every 5th capture can upload `unknown` → HTTP 500 → `mark_synced` never runs. `get_unsynced` is `played_at ASC` → one bad head-of-line blocks **all future sync forever**.
- **Duplicate-skip guard never fires.** Server checks `e.to_string().contains("unique")`, but SurrealDB v3 IndexExists messages look like ``Database index `pm_dedup_idx` already contains ...`` — no `"unique"`. Retries after partial upload and outcome back-fill (`synced = false` on already-uploaded rows) → 500 → wedge.
- **Map correction changes dedup key** → new server row; stale old-map row remains.
- **Delete session is local-only** — junk remains in cloud stats (Grok).

**Fix:** match structured `IndexExists`; filter/hold `unknown` out of `get_unsynced` (or relax assert); upsert by client/session id; plan delete round-trip or tombstones.

### C3. Force-clear live SurrealKV while daemon holds it open (Grok)

“Clear All Match Data” on store lock failure can `force_clear_data_dir` and unlink `stats.surrealkv` while the daemon still has the store open. No confirm step. Unsafe data-loss / corruption class.

**Fix:** disable clear when daemon is up, or stop → clear → restart; always require confirm.

---

## Major — correctness

### M1. Unfinished sessions never expire → merged games (Fable)

`should_start_fresh_session`: unfinished games return `false` with **no staleness bound**. If the poller misses outcome and next start screens (likely under Tab starvation), or `auto_detect.enabled = false`, every Tab across multiple games appends to one session. Worst case: yesterday’s unfinished session absorbs today’s first game. Grace only bounds *finished* games.

**Fix:** track last-capture (or last-activity) on `ActiveGame`; start fresh past a plausible match length (~20 min). Prefer wall-clock-aware time if suspend is a real concern (see m5).

### M2. Map-vote *candidates* recorded as the played map (Fable; Grok misread as strength)

MapVote arm stores every OCR’d vote name; `extract_map_names` returns constant order, not winner order (unknowable at vote time). Capture path then prioritizes `detected_maps[0]` over top-bar OCR of the *actual* map; accolade recovery / `set_session_map` gated on empty maps → **uncorrectable**. ≥2 candidates → wrong map ~2/3 of the time. Vote names are UPPERCASE constants while other paths use display names → fractured aggregates.

**Fix:** treat vote maps as candidates only (constrain later reads); canonicalize all map strings through the MAPS table.

### M3. Hero substring trap: "Ana" in "HAVANA"; ascending tie-break (Fable)

`find_hero` pass 1 substring-matches short names against joined text — same class as fixed “king” map trap. Bare `HAVANA` can yield Ana; tie-break sorts ascending by mention count (least-mentioned wins). Fallback path only, but support-on-Havana can flip.

**Fix:** word-boundary matching for short names (Ana/Mei/Echo/Juno); strip map-label line before hero search.

### M4. No HTTP timeout; sync on the event loop stalls the daemon

`reqwest::Client::new()` with no timeout; `try_sync` awaited inline in `select!` arms. Hung connection blocks Tab, poll, commands, and graceful SIGTERM/Ctrl-C.

**Fix:** builder timeout (e.g. 30s); run sync on a spawned/off-loop task.

### M5. Pending-outcome / confirmation leaks (both; Fable more exhaustive)

- Pending outcome only consumed on Tab path when opening a fresh session; poller MapVote/HeroSelect can open `Unknown` and leave pending sitting → late Tab stamps old outcome onto a new match (Grok).
- Confirmed outcome with no game open inherited by next Tab within 90s; Tab during next game’s opening minute can stamp game A’s result onto B and block B’s real outcome (Fable m3).
- `word_outcome_streak` reset only when poller opens a game, not Tab-opened sessions (Fable m4).
- Mid-game Tab `detect_banner` can false-positive on heavy red vignette ≥35% and split the real game (Fable m6).

**Fix:** apply or explicitly clear pending when poller opens a game; reset streaks on any new session; restrict Tab banner application or raise bar mid-game.

### M6. Session create / insert half-failures (Fable)

`create_session` succeeds → `insert_match` fails → session not marked created → next Tab re-creates session; no unique index on `session_id`. Conversely `create_session` failure is warn-only yet marked created → later updates match nothing.

### M7. `mark_synced` by count/order, not identity

Correct only under single-writer / full success assumptions. Mark specific fetched record IDs; chunk uploads.

### M8. Settings do not hot-reload (Grok)

Daemon reads config once at start. Changing player name, token, output, auto-detect is a silent no-op until restart.

**Fix:** toast “restart required” and/or SIGHUP / restart path.

### M9. PID liveness ≠ daemon identity (Grok; Fable m11)

Status/stop check `/proc/{pid}` exists; PID reuse risk; pid file removed before exit confirmed; `PidGuard::drop` unconditionally deletes whatever pid file exists (including a newly started daemon’s). GUI 10s store-open polling can hold the KV lock exactly when daemon starts.

**Fix:** verify pid is this binary/cmdline before kill; remove pid only after exit confirmed; coordinate lock/start races.

### M10. `ActiveGame` is memory-only — daemon restart splits or merges games (Grok; promoted from minor)

The session state machine lives entirely in `run_loop` locals. Restarts are routine (crash, upgrade, systemd restart), and this amplifies M1: restart mid-game with no unfinished-session TTL is exactly the recipe for split *or* merged sessions, and any pending outcome/map context is gone.

**Fix:** persist an open-session skeleton (session_id, started_at, maps) to the store or a small state file; recover on startup within a staleness bound (dovetails with M1’s last-activity timestamp).

---

## Performance — daemon & hot path

### P1. Tab-capture OCR serializes the entire `select!` loop (HIGH)

`handle_capture` awaited inline; `spawn_blocking` runs the full OCR pipeline; loop joins it. Poll ticks use `MissedTickBehavior::Skip` — comments document 45–70s gaps. ~3s VICTORY/DEFEAT banner and ~20s accolade screen get missed. The 400ms sleep after Tab stalls the loop the same way.

**Fix:** spawn `handle_capture` as a task (single-flight against double-Tab) reporting via channel; parallelize independent OCR passes inside the closure where safe.

### P2. Ungated debug-image pipeline re-runs full preprocessing (HIGH)

Every capture calls `save_debug_stages` (and again via `save_debug_images` on full recognize): recomputes HSV/Sauvola/morphology on full scoreboard purely to write PNGs; ~15–17 encodes + disk writes. Not gated (unlike `--dump-poll-frames`). Hardcodes `dirs::data_dir()/"scuffed-stat-tracker"`, ignoring `config.data_dir`.

**Fix:** gate behind config/env; when enabled, save already-computed intermediates; pass debug dir from caller.

### P3. Full-image `recognize()` always runs per Tab (HIGH)

Cell OCR already supplies positional stats; career/map are small crops. Full-board adaptive pipeline + up to three threshold sweeps when confidence < 65 is largely redundant on the happy path.

**Fix:** run lazily — only when row-by-name/brightness path failed or career/map reads empty.

### P4. Column calibration ~300 cell OCRs, no early exit, no cache (HIGH)

Coarse + fine offsets × ~18 OCRs each before real 60–70 cell reads. Comment admits calibration “dominated capture latency”.

**Fix:** early-exit at max score; don’t re-score coarse in fine sweep; cache winning offset per session/resolution; re-sweep only on score degradation. Longer-term: score crops without Tesseract if possible.

### P5. New Wayland connection per screenshot (MEDIUM)

Every poll tick and Tab creates a fresh `WayshotConnection` (handshake, outputs, shm/dmabuf) and tears it down.

**Fix:** create once, reuse, recreate on error; resolve output name once.

### P6. Multiple full-frame RGBA→RGB conversions per poll (MEDIUM)

`detect_map_vote` / `detect_hero_ban` / `detect_hero_select` / `detect_banner` each convert. ~11MB + 3.7M pixels each; on normal frames all run every 4s.

**Fix:** convert once per tick; pass `&RgbImage`; subsample scans (ratio tests tolerate 1-in-2 / 1-in-4).

### P7. Redundant Tab work (MEDIUM)

- `detect_team_size` twice (main + `match_player_hero`).
- Scoreboard crop 2–3× (~6.7MB each).
- Nested Rayon on already-capped 2–4 pool (Grok).
- PNG encode every Tesseract call (Grok); raw/grayscale feed if API allows.

**Fix:** compute team size + crop once; flat par over cells; pass pre-cropped board.

### P8. 1–2 unconditional title OCR calls per idle poll (MEDIUM-LOW)

`read_result_word` always; rank-screen path when first is Unknown (virtually every tick). Cheap crop-local brightness probe would skip both most ticks.

### P9. Full-resolution capture every poll; fixed 4s interval (MEDIUM — Grok)

Detectors only need bands; no adaptive interval (idle vs post-match).

---

## Performance — GUI / storage / sync

### P10. GUI panels reopen SurrealKV (+ DDL) on 10–15s timers (HIGH)

History/status/stats each `LocalStore::open` → lock + 6 `DEFINE TABLE/INDEX` + full-table select, then drop. When daemon holds lock, failed open still costs the attempt before re-reading/re-parsing entire `live_snapshot.json`. Cost grows with history forever while the window is open.

**Fix:** cache one open `LocalStore` in context; skip re-parse when snapshot mtime unchanged; lift DDL out of `open` into once-init.

### P11. `export_snapshot` full rewrite after every mutation (MEDIUM)

Two full-table scans + serialize + `std::fs::write`/`rename` on the tokio runtime inside the contended select loop. One capture can trigger 2–3×.

**Fix:** debounce (dirty flag); `spawn_blocking` for serialize+write; longer-term incremental / final-only retention.

### P12. Smaller GUI/daemon costs (LOW)

- `MatchesPanel` re-clones/re-derives history every render → `use_memo`.
- DaemonCard 10s loop: sync `systemctl` + unconditional signal sets → `tokio::process` + set-on-change.
- `stats_from_rows` by-value clone; `compute_stats` inside `rsx!` per render → slices + memoize.
- Blocking file IO in async paths (`take_commands`, `append_match_log`, `queue_command`).
- Tray quit poller 1 Hz forever → blocking `MenuEvent` recv.
- Sync Wayland roundtrips in signal initializers on first render → `use_resource` + `spawn_blocking`.

### P13. Micro / build (LOW)

- `hsv_white_mask` float HSV but only S/V; per-cell ~7 image allocs × ~70 cells (+ calibration).
- `tessdata_lang()` stats FS every call (~12+/Tab) → `OnceLock`.
- Needless full-image clones before PNG encode.
- `detect_column_offset` converts whole board to RGB for top 2.5% — crop first.
- No `[profile.release]` thin LTO / dev package opt for `image`.
- Slim `tokio` features (not `full`); slim `image` default features.

**Rough Tab OCR budget (5v5, warm):** calibration ~300 + final ~70 + full board 1–4 + panels 2 ≈ **~380 Tess calls**. At ~6 ms serial ≈ ~2.3 s Tess alone; wall multi-second with preprocess + debug I/O.

---

## Architecture & code quality

### A1. `run_loop` / `handle_capture` God-functions (HIGH)

~300+ lines each, 11 positional params, `#[allow(clippy::too_many_arguments)]`; blocking closure returns a 9-tuple; adjacent `Option<&str>` swap-prone.

**Fix:** `DaemonCtx` by reference; named `FrameAnalysis`; split `on_tab` / `on_poll_tick` / `on_command_tick`.

### A2. Portrait-row geometry duplicated and diverged (HIGH)

`--collect-portraits` recomputes geometry (`row_height = sh*7/100`, linear `row_y`, no team gap) while canonical `extract_portrait_crops_inner` uses `h*58/1000` for 6v6 **and** a team gap. 6v6 and team-2 players: auto-collected refs crop wrong rows → silently poison template library.

**Fix:** single `portrait_rect(scoreboard_dims, row_idx, team_size)` used by both sites.

### A3. Match outcome stringly-typed across every layer (HIGH — Fable)

`MatchOutcome` exists in detect, but string free functions in daemon; storage raw strings; GUI re-parses in three places with legacy `"win"` variants. Typo → silent misclassification.

**Fix:** `Display + FromStr + serde` on `MatchOutcome` in the lib; delete scattered translations.

### A4. Error handling: `Box<dyn Error>` everywhere; `thiserror` unused (HIGH)

Inconsistent `Send + Sync` bounds; lost source chains. `thiserror` in Cargo.toml with zero uses.

**Fix:** `anyhow` + `.context()` in binaries; drop or use typed errors intentionally.

### A5. Daemon↔GUI IPC exists to work around SurrealKV single-writer (MEDIUM)

Snapshot + jsonl + command queue are pragmatic. Nothing here needs SurrealQL; multi-reader embedded store (SQLite/redb) would delete much IPC, “database locked” states, and pid/lock races. Weigh before stacking more features on the side channels. (Local design remains documented and reasonable if you stay on SurrealKV.)

Two concrete **integrity holes** in the current IPC (moved here from the perf section — these are data-loss bugs, not render costs):

- **Command queue is delete-then-apply** — `take_commands` unlinks command files *before* the daemon applies them; a crash between unlink and apply silently loses the user’s manual edits (SetOutcome / DeleteSession). Two-phase (apply, then delete) fixes it cheaply.
- **`matches.jsonl` is never updated on back-fill** — `append_match_log` only runs at insert; `set_session_outcome`/`set_session_map` don’t touch it, so the JSONL fallback permanently shows `unknown` outcomes / stale maps for back-filled games.

### A6. GUI data-loading copy-pasted; policies drift (MEDIUM)

`Config::load` + timer + open-store-else-snapshot pattern in status/stats/history (+ store opens in settings/daemon). Status sets `db_locked`; stats/history silently fall back.

**Fix:** shared `use_live_matches()` (or equivalent) hook.

### A7. ~200 lines untested stats aggregation in the GUI (MEDIUM — Fable)

`compute_stats` — win rates, per-hero/role/map, rolling rates — `gui` feature only, zero tests. Classic “win rate looks wrong” class.

**Fix:** move next to `PersonalMatch` in lib; unit test; view renders only.

### A8. Outcome/map OCR regions ignore letterbox correction (MEDIUM — Fable)

Scoreboard/career/map use `game_rect_16_9`; outcome word / accolade map / banner sampling often crop raw frame despite “1/1000ths of 16:9” docs. Ultrawide/16:10: scoreboard works, W/L degrades.

**Fix:** route those crops through `game_rect_16_9`.

### A9. Layering nits (MEDIUM-LOW)

- `detect` ↔ `ocr` bidirectional dependency; extract `layout` for scoreboard geometry.
- `parse_scoreboard_cells` fabricates full `PersonalMatch` with `Utc::now()` — parsing shouldn’t own clock/persistence type.
- Config path/save duplicated; `Config::load` parses `std::env::args()` inline (untestable precedence).
- `PersonalMatch` means “capture” locally and “game” on the server — overloaded naming (Grok).
- Dead APIs: `find_active_session`, `session_window_secs`, `update_session`, `get_multi_capture_sessions`; unused `parse_scoreboard` with unsafe row fallback (Grok).
- `detect_stat_columns` ignores argument and returns constant — name misleads; real path is `calibrate_columns`.
- Dashboard hardcodes capture backend as libwayshot regardless of Portal/None (Grok).
- Portal path: blocking decode on async task; ignores `capture_output` (Grok).

### A10. Low priority / product polish

- Seven `prepare_*` preprocess variants — document variant→call-site table while tuning.
- `rand_id` time-xor-pid — fine at human cadence; note non-cryptographic.
- Tray quit `process::exit(0)` skips cleanup; tray icon `expect` can panic GUI.
- Keyboard total loss exits daemon (systemd restarts if unit used).
- Accessibility: match rows mouse-centric.
- Missing crate-level README (Linux/Wayland, input group, tessdata, clear-data caveats).
- `chmod 600` on config with token.
- `tempfile` in normal deps if only needed by tests/examples.

---

## Minor correctness findings

| ID | Finding |
|----|---------|
| m1 | Startup panic on zero Wayland outputs: `unwrap_or(&outputs[0])` evaluates eagerly — use `.first()`. |
| m2 | Duplicate `match_session` rows after failed insert / false “created” (see M6). |
| m3 | Pending-outcome leak onto next game (see M5). |
| m4 | `word_outcome_streak` not reset on Tab-opened sessions (see M5). |
| m5 | Suspend-blind `Instant` windows — resume can treat old session as still in grace. |
| m6 | Mid-game Tab banner false-positive (see M5). |
| m7 | Portrait collect wrong rows for team-2 / 6v6 (= A2). |
| m8 | `mark_synced` by count (= M7). |
| m9 | Row-index compaction on mid-row crop failure desyncs `rows[i]` from on-screen indices (low likelihood). |
| m10 | `AutoDetectConfig` lacks per-field serde defaults — partial table fails parse, daemon won’t start. |
| m11 | PID + lock races (see M9). |
| m12 | `ActiveGame` memory-only — **promoted to M10**. |
| m13 | Unbounded local history of intermediate Tab rows; no “final only” retention (Grok). |
| m14 | Team-size near pitch threshold — no hysteresis 5↔6 (Grok). |
| m15 | Double word-OCR hallucination can still confirm (two agreeing wrong words) (Grok). |

---

## Residual detection risks (keep in mind)

| Risk | Type | Notes |
|------|------|--------|
| Tab outside scoreboard | FP | Weak shape gate; process gate helps |
| Wrong teammate stats | FP | Largely eliminated by positive ID only |
| Weak highlight + no `player_name` | FN | Drop capture (good) but silent for users |
| Missed short banner | FN | Poll interval + Tab starvation (P1) |
| Post-match Tab after 75s grace | split/dup | AFK edge |
| Portrait MAD | theme-sensitive | Career panel mitigates |

---

## Verified non-issues (for the record)

- Stale always-victory class substantially fixed; residual leaks are M1/M5/m5 etc.
- `parse_cell_number` / `looks_like_scoreboard` solid against OCR garbage.
- Tess instances cached per (thread, lang); OCR pool capped; Sauvola O(n) integrals; `/proc` gate cached 5s.
- Sync batched (one POST for all unsynced) — problem is *semantics*, not N+1 requests.
- Local writes atomic where it matters (snapshot, queue); poll-frame dump correctly flag-gated (unlike OCR debug saves).
- No unbounded channels of full screenshots.

---

## Testability

| Area | Coverage |
|------|----------|
| Session pure helpers | Good |
| Parse trust gates, maps, heroes | Good |
| Banner colour floods | Synthetic unit tests |
| Game process gate | Good |
| Storage helpers (queue, majority, latest_per_game) | Good |
| OCR replay / outcome fixtures | `#[ignore]`, local fixtures |
| `compute_stats` | **None** (highest-value gap) |
| Team size + portrait matching | Ideal fixture candidates, thin today |
| Match-start detection | **None** (threshold-heavy) |
| GUI / capture / sync / poller FSM | **None** beyond pure helpers |

**Gaps by value:** (1) `compute_stats`, (2) `detect_team_size` + portraits, (3) match_start, (4) config precedence, (5) CI wall-clock microbench of Tab path / synthetic smoke if possible.

Operator tooling partially compensates for thin CI E2E — keep it.

---

## Consolidated priority backlog

### P0 — Correctness, safety, critical path

1. **Sync contract (C1 + C2 + M4):** `session_id` / stable client id + server upsert; filter/hold `unknown`; structural `IndexExists`; HTTP timeout; sync off the capture loop.
2. **Never force-delete SurrealKV while daemon is running (C3)** + confirm dialog.
3. **Un-serialize daemon loop (P1)** — spawn Tab OCR so banners/accolades can be seen again.
4. **Gate debug OCR PNGs (P2)** — easiest wall-time + disk win on every Tab.

### P1 — Large latency / robustness / wrong-data

5. Cache/shrink column calibration (P4); skip full `recognize()` on happy path (P3).
6. **Thin enablement slice** (half-day, before items 7–10): typed `MatchOutcome` (A3) + named `FrameAnalysis` struct replacing `handle_capture`’s 9-tuple — the session/map/outcome fixes below all edit `main.rs`, and this makes them compiler-checked. Full God-function split stays P2.
7. Unfinished-session staleness bound (M1); persist/recover `ActiveGame` across restart (M10).
8. Map-vote as candidates only + map name canonicalization (M2).
9. Ana/HAVANA word-boundary + hero tie-break (M3).
10. Pending-outcome apply/clear on poller open; streak resets; safer Tab banner mid-game (M5).
11. Shared portrait geometry (A2); letterbox-correct outcome regions (A8).
12. `mark_synced` by IDs; safer PID stop (M7, M9).
13. Settings “restart required” or reload (M8).
14. Single RGB per poll + Wayshot reuse + pass team_size/crop once (P5–P7).

### P2 — Quality of life / hygiene

15. GUI store-handle caching + snapshot mtime skip (P10); snapshot debounce (P11).
16. Shared `use_live_matches()` (A6); extract + test `compute_stats` (A7).
17. Full `run_loop`/`handle_capture` split with `DaemonCtx` (A1) — the typed-outcome + `FrameAnalysis` slice already landed in P1 item 6.
18. Real capture-backend status; delete/tombstone cloud round-trip; two-phase command apply + jsonl back-fill updates (A5 integrity holes).
19. Crate README; config `chmod 600`; dead API cleanup; slim deps.
20. Team-size hysteresis.
21. `anyhow`/typed errors; layout module; parse returns plain struct without clock.

### P3 — Longer term

22. Incremental OCR (player row only after first full board in a session).
23. Stronger portrait metric if career panel missing.
24. Adaptive poll interval; early-out when mid-game stable.
25. CI smoke + documented latency bands from `polltick` / timed `extract`.
26. Accessibility (keyboard rows, real confirm modals).
27. Revisit SurrealKV vs multi-reader store if IPC cost keeps growing (A5).

---

## Dimension grades (merged view)

| Dimension | Grade | Comment |
|-----------|-------|---------|
| Domain / session correctness | **A−** | Grace windows, trust gates, majority repair — battle-tested; residual M1/M5/M10 holes |
| OCR accuracy architecture | **B+** | Cell OCR + multi-source hero/map solid; map-vote priority and short-name traps bite |
| OCR / Tab performance | **C** | Calibration, dual OCR, debug I/O dominate |
| Poll path efficiency | **B−** | Acceptable defaults; full-frame RGB, fixed interval, title OCR on idle |
| Local storage / IPC | **B+** | Right single-writer design; snapshot cost and clear-data footgun |
| Cloud sync semantics | **D+** | Snapshot≠game, weak identity, corrections/deletes don’t round-trip, wedge modes |
| GUI quality | **B** | Usable and coherent; lifecycle/PID/settings footguns |
| Tests | **B−** | Core pure logic good; stats/E2E/GUI thin |
| Maintainability | **B** | Great comments; God-loops, stringly outcomes, dead surface |

---

## How the source reviews were reconciled

| Topic | Fable | Grok | Merged stance |
|-------|-------|------|----------------|
| Sync every Tab as match | C1 Critical | C1 Critical | **Critical** |
| Sync wedge / IndexExists / unknown | C2 deep (incl. dead `"unique"` match) | H3/H1 high | **Critical** (Fable’s IndexExists detail wins) |
| Force-clear live DB | thin (m11 races only) | Critical safety | **Critical** (Grok) |
| Unfinished session merge | M1 Major | edge-case table | **Major** (Fable) |
| Map-vote as played map | M2 Major | listed as *strength* | **Major bug** (Fable; Grok wrong here) |
| Ana ⊂ HAVANA | M3 | not named | **Major** (Fable) |
| Priority order | Sync → loop → OCR cost | Debug PNG → calibration → sync | **Fable overall order**, with Grok force-clear as P0 safety |
| Session FSM praise | residual focus | strong “don’t undo” | Keep both: praise + residuals |
| GUI lifecycle / settings / PID | lighter | stronger | Keep Grok items at M8/M9/C3 |
| Grades / product framing | severity lists | dimension grades | Both retained |
| Command queue / JSONL integrity | under IPC design (A5) | under P12 low GUI cost (misfiled) | **A5 integrity holes** (Fable quibble; both agreed) |
| `ActiveGame` lost on restart | M6 Major | m12 minor | **M10 Major + P1** (both agreed) |
| Refactor as enablement | full DaemonCtx earlier in spirit | exec vs backlog inconsistent | **Thin typed outcome + `FrameAnalysis` in P1; full `DaemonCtx` in P2** (both agreed) |

---

## Evidence map

| Concern | Path |
|---------|------|
| Daemon loop / session FSM | `src/main.rs` |
| OCR + calibration + debug dumps | `src/ocr/mod.rs` |
| Preprocess / geometry | `src/ocr/preprocess.rs` |
| Parse / trust / heroes | `src/parse.rs` |
| Outcome / banner / accolade | `src/detect/match_end.rs` |
| Match start | `src/detect/match_start.rs` |
| Portraits / team size | `src/detect/hero_portrait.rs` |
| Process gate | `src/detect/game_running.rs` |
| Capture | `src/capture/{mod,wayshot,portal}.rs` |
| Storage / IPC | `src/storage/mod.rs` |
| Sync client | `src/sync/mod.rs` |
| Upload types | `crates/types/src/api/stats.rs` |
| Server stats / dedup / assert | `crates/db/src/migrations.rs`, `crates/db/src/queries/personal_stats.rs` |
| GUI lifecycle | `src/gui/{daemon,settings,status,history,stats,main}.rs` |
| Poll latency tool | `examples/polltick.rs` |
| Full extract mirror | `examples/extract.rs` |

---

## Bottom line

This is a **product-shaped vision crate with a strong kernel**. Invest next in:

1. **Upload/session contract with the server** (truth of W/L),
2. **Safe daemon/GUI lifecycle** (clear data, PID, settings),
3. **Cutting Tab OCR work and decoupling the poller** (speed *and* outcome reliability),
4. **Closing residual wrong-data bugs** (unfinished sessions, restart recovery, map-vote, Ana/Havana, portrait geometry).

Do **not** relax “refuse bad player rows” or outcome confirmation without a strong replacement — that philosophy is why local aggregates stay trustworthy.

---

*Canonical backlog. Source and intermediate reviews live under `review-archive/` for history only — implement from this file.*
