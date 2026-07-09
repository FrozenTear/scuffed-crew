# Stat Tracker — Deep Review (Fable)

**Date:** 2026-07-09 · **Branch:** `stat-tracker/add-neon-junction-aatlis` · **Scope:** `crates/stat-tracker` (plus the server-side sync surface it talks to)

**Method:** four parallel deep-dive agents using Serena symbolic analysis — (1) performance of the capture/OCR/detect hot path, (2) performance of the GUI/daemon/storage/sync layers, (3) correctness & robustness of the detection/parsing state machine and data integrity, (4) architecture & code quality across the whole crate. Findings below are deduplicated and re-ranked across all four reports. All findings were verified against actual function bodies, not speculated. Line numbers are 1-based.

---

## Executive summary

The crate has genuinely good bones: clean top-level data flow (capture → detect/ocr → parse → storage → sync), unit tests on the riskiest pure logic (parse, storage, session state machine, banner color logic), a real fixture-replay workflow, and magic numbers that are at least named and commented. The old "always victory" stale-`active_game` bug class is substantially fixed, with regression tests.

The problems cluster in four places:

1. **The sync pipeline is broken end-to-end** (C1/C2): the server counts every Tab press as a separate match, and two independent failure modes wedge sync permanently. This is the highest-value fix in the codebase.
2. **The daemon select loop serializes everything** (P1): heavy Tab-capture OCR starves the poller for 45–70s (measured), which is the root cause of missed outcome banners — the very problem the confirm-window machinery exists to paper over.
3. **The OCR hot path does enormous unnecessary work** (P2–P4): ungated debug-PNG dumps that *re-run the whole preprocessing pipeline*, an always-on full-image fallback OCR, and ~300 calibration OCRs per capture with no early exit.
4. **`main.rs` concentrates the risk** (A1–A3): two ~300-line God-functions with 11 positional parameters, duplicated portrait geometry that has already diverged (latent 6v6 bug), and stringly-typed outcomes re-parsed independently in four places.

---

## Crate shape

`scuffed-stat-tracker` — two binaries (daemon + Dioxus desktop GUI behind the `gui` feature) sharing a lib. ~7,600 lines in `src/`, ~1,000 in `tests/`, ~650 in `examples/`. Dioxus 0.7 desktop, tokio (full), SurrealDB v3 with embedded SurrealKV, reqwest sync, leptess OCR on a capped rayon pool.

| Module | Lines | Responsibility |
|---|---|---|
| `src/main.rs` | 1077 | Daemon: CLI, PID guard, `ActiveGame` session state machine, `run_loop` (Tab / poll / command queue / signals), `handle_capture`, sync + snapshot helpers |
| `src/capture/` | 115 | Backend selection: libwayshot preferred, ashpd portal fallback |
| `src/detect/` | 1158 | Banner color-flood + VICTORY/DEFEAT OCR, map-vote/hero-ban/select phases, `/proc` game gate (cached), portrait template matching |
| `src/ocr/` | 1296 | leptess wrapper, per-cell scoreboard OCR, column calibration, preprocessing pipelines (HSV mask / Sauvola / morphology) |
| `src/parse.rs` | 754 | OCR rows → `PersonalMatch`; fuzzy hero/map matching, plausibility gates. Well tested |
| `src/storage/` | 620 | SurrealKV store, `live_snapshot.json` export, `matches.jsonl` log, file-based GUI→daemon command queue |
| `src/sync/` | 88 | reqwest: upload matches, fetch daemon config |
| `src/setup.rs` | 553 | Tessdata training pipeline |
| `src/gui/` | ~2,700 | Dioxus + tray: status/stats/history/settings/preview views, daemon control card |

Because SurrealKV is single-process, the daemon and GUI communicate through three side channels: the JSON snapshot, the JSONL log, and a polled file command queue (see A5).

---

## Critical — correctness

### C1. Server sync counts every Tab-press snapshot as a separate match
`src/main.rs:866` inserts one `personal_match` row per Tab capture; `get_unsynced` (`src/storage/mod.rs:82-89`) returns all of them; `upload_matches` (`src/sync/mod.rs:42-86`) sends them all — and `StatsUploadEntry` (`crates/types/src/api/stats.rs:5-24`) has **no `session_id` field**, so the server cannot collapse them. Locally the GUI collapses via `latest_per_game` (`src/storage/mod.rs:486-491`); the server has no equivalent, and its dedup index `(member_id, hero, map_name, played_at)` (`crates/db/src/migrations.rs:531-532`) never matches because each snapshot has a distinct `played_at` (stamped `Utc::now()` at parse time, `src/parse.rs:117`). **A game where the user tabs 6 times = 6 wins/losses in server stats.**
**Fix:** include `session_id` in the upload payload and dedup/upsert per session server-side, or upload only the final snapshot per session (on session close + shutdown).

### C2. Sync wedges permanently — two independent triggers, same terminal state
- **`unknown` outcomes violate the server ASSERT.** `try_sync` runs mid-game every 5th capture (`src/main.rs:444`, `SYNC_EVERY_N_CAPTURES` at `main.rs:10`) while snapshots still carry `outcome = "unknown"`. The server schema asserts `outcome IN ['victory','defeat','draw']` (`crates/db/src/migrations.rs:519-520`) → HTTP 500 → `mark_synced` never runs. `get_unsynced` orders `played_at ASC`, so one permanently-unknown row (abandoned game, missed accolade) **head-of-line-blocks all future syncs forever**.
- **The duplicate-skip guard can never fire.** `bulk_insert_personal_matches` checks `e.to_string().contains("unique")` (`crates/db/src/queries/personal_stats.rs:127`), but SurrealDB v3's unique-violation message is `` "Database index `pm_dedup_idx` already contains {value}, with record ..." `` — no substring "unique" (verified in surrealdb-core source). Any retry after a partial upload, and any outcome back-fill (`set_session_outcome` sets `synced = false` on already-uploaded rows with an unchanged dedup key) → IndexExists → 500 → wedge. The *routine* flow (capture with unknown → back-fill → re-sync) hits this quickly.
- **Corollary:** `set_session_map` (`src/storage/mod.rs:225-236`) changes `map_name`, which is part of the dedup key — the corrected row inserts as a **new** server row while the stale old-map row remains.

**Fix:** match on the structured `IndexExists` error, filter `outcome == "unknown"` out of `get_unsynced` (or relax the server assert), and make corrections an upsert keyed by a client-generated id.

---

## Major — correctness

### M1. Unfinished sessions never expire → merged games (surviving sibling of the old active_game bug)
`should_start_fresh_session` (`src/main.rs:239-253`): `Some(g) if !g.finished() => false` unconditionally — no staleness bound on an *unfinished* game. If the poller misses both the outcome and the next game's start screens (which the 45–70s OCR starvation makes likely — see P1), or if `auto_detect.enabled = false` (the poll arm never runs at all), every Tab across multiple games appends to one session. Worst case: yesterday's unfinished session absorbs today's first game. The grace window only bounds *finished* games.
**Fix:** track last-capture time on `ActiveGame`; start fresh past a plausible match length (~20 min).

### M2. Map-vote *candidates* are recorded as the played map, with top priority
The MapVote arm stores every map OCR'd off the vote screen (`src/main.rs:558-571`); `extract_map_names` returns them in `MAP_NAMES` constant order — not winner order (the winner is unknowable at vote time). `handle_capture` then gives `detected_maps[0]` priority over the top-bar OCR of the *actual* map (`main.rs:838-842`), and both accolade-map recovery and `set_session_map` are gated on `g.maps.is_empty()`, so nothing can ever correct it. With ≥2 OCR'd candidates the recorded map is wrong ~2/3 of the time. Also: vote names are UPPERCASE constants (`"KING'S ROW"`) while every other path stores display names (`"King's Row"`) — fracturing per-map aggregates locally and on the server.
**Fix:** treat vote maps as candidates only (constrain the later top-bar/accolade read); canonicalize all map strings through the `MAPS` table.

### M3. Hero substring trap: "Ana" matches inside "HAVANA"; tie-break picks the *least*-mentioned hero
`find_hero` pass 1 (`src/parse.rs:332-343`) substring-matches hero names against joined raw text — same bug class as the fixed "king" map trap. On Havana, a bare `HAVANA` line satisfies the `num_count <= 1` branch (`parse.rs:353-364`) → hero = "Ana". The final tie-break `found.sort_by_key(|&(_, count)| count)` (`parse.rs:373-374`) sorts **ascending**, returning the hero with the *fewest* mentions (ties → alphabetical → Ana first). Only the fallback path when career-panel and portrait both miss, and `majority_hero` dilutes it — but support-Havana games can flip.
**Fix:** word-boundary matching for short names (Ana/Mei/Echo/Juno); strip the map-label line before hero search.

### M4. No HTTP timeout on the sync client stalls the entire daemon
`SyncClient::new` uses `reqwest::Client::new()` with no timeout (`src/sync/mod.rs:14-19`), and `try_sync` is awaited inline inside the `select!` arms. A hung connection blocks Tab capture, polling, GUI commands — and the SIGTERM/Ctrl-C paths, making the daemon unkillable-gracefully.
**Fix:** `Client::builder().timeout(Duration::from_secs(30))`.

---

## Performance — daemon & hot path

### P1. Tab-capture OCR serializes the entire daemon select loop (HIGH)
`src/main.rs:405`: `handle_capture` is awaited inline inside `tokio::select!`; its `spawn_blocking` closure runs the whole OCR pipeline serially and the loop blocks on the join. Poll ticks are skipped (`MissedTickBehavior::Skip`) — the code's own comment (`main.rs:351`) documents measured 45–70s tick gaps. The ~3s VICTORY/DEFEAT banner and ~20s accolade screen get missed entirely; this is the root cause of the outcome-detection fragility that M1/m3/m4 compensate for. The `sleep(400ms)` at `main.rs:381` stalls the loop the same way.
**Fix:** run `handle_capture` as a spawned task (single-flight guard against double-Tab) reporting back via a channel; parallelize the independent OCR passes inside the closure (rayon is already a dep).

### P2. Ungated debug-image pipeline re-runs full preprocessing twice per Tab capture (HIGH)
`src/ocr/mod.rs:278-285` and again via `save_debug_images` (`mod.rs:486-495`): every capture calls `save_debug_stages` (`src/ocr/preprocess.rs:766-787`), which **recomputes the entire pipeline** — HSV white mask + Sauvola + morphological close on the full ~1664×1008 scoreboard — purely to save stage PNGs, plus ~15–17 PNG encodes and disk writes. Not gated by any flag (unlike `--dump-poll-frames`, which is). This burns CPU/IO exactly while the user is in-game. It also hardcodes `dirs::data_dir()/"scuffed-stat-tracker"`, ignoring `config.data_dir` — a layering leak.
**Fix:** gate behind a config/env debug flag; when enabled, save already-computed intermediates; pass the debug dir in from the caller.

### P3. Full-image fallback OCR always runs per Tab even when the per-cell path succeeds (HIGH)
`src/main.rs:704`: `ocr::recognize(&img)` re-crops the scoreboard, runs the full adaptive pipeline, a whole-board Tesseract pass, and up to **three more** threshold sweeps (incl. per-pixel median filter) when confidence < 65. Its output is only used for hero/map lookup and the name-anchored row fallback.
**Fix:** run lazily — only when the row-by-name/brightness path failed or the career/map panel reads came up empty.

### P4. Column calibration performs ~300 cell OCRs per Tab, no early exit, no caching (HIGH)
`src/ocr/mod.rs:292-369` (`calibrate_columns`): coarse ~8 offsets + fine 9 offsets × 18 OCRs each (crop + HSV mask + PNG encode + Tesseract) before the real 60–70 cell reads start. The comment itself says calibration "dominated capture latency".
**Fix:** early-exit at max score; don't re-score coarse offsets in the fine sweep; cache the winning offset per session/resolution and re-sweep only on score degradation.

### P5. New Wayland connection per screenshot (MEDIUM)
`src/capture/wayshot.rs:14-48`: every 4s poll tick and every Tab creates a fresh `WayshotConnection` — compositor handshake, output enumeration, shm/dmabuf setup — then tears it down.
**Fix:** create once (thread-local on a dedicated blocking thread, or `Mutex<WayshotConnection>`), reuse, recreate on error; resolve the output name once.

### P6. Four separate full-frame RGBA→RGB conversions per poll tick (MEDIUM)
`detect_map_vote`/`detect_hero_ban`/`detect_hero_select` each start with their own `img.to_rgb8()` (`src/detect/match_start.rs:19, 68-69, 121-122`), plus `detect_banner`'s (`src/detect/match_end.rs:98`). Each is ~11MB allocated + 3.7M pixels converted; on normal in-game frames none early-return, so all four run every 4s, followed by per-pixel `get_pixel` scans.
**Fix:** convert once per tick, pass `&RgbImage` to all detectors; iterate `as_raw()` chunks; the ratio tests tolerate 1-in-2/1-in-4 subsampling.

### P7. Redundant work per Tab (MEDIUM)
- `detect_team_size` runs twice — `src/main.rs:696` and again inside `match_player_hero` (`src/detect/hero_portrait.rs:102`); each does a full scoreboard `to_rgb8` + per-pixel saturation scan.
- Scoreboard crop computed 2–3× (`main.rs:695`, `ocr/mod.rs:252`, `mod.rs:402`), each copying ~6.7MB.
**Fix:** compute once, pass down (`match_player_hero_with_team_size`, crop param).

### P8. 1–2 unconditional Tesseract calls per idle poll tick (MEDIUM-LOW)
`src/detect/match_end.rs:38-45`: `read_result_word` always runs; `read_rank_screen_result` runs whenever the first returns Unknown (i.e. virtually every tick). Each includes Lanczos upscale + Otsu + PNG encode + OCR. A cheap crop-local brightness-histogram probe would skip both on the overwhelming majority of ticks.

---

## Performance — GUI / storage / sync

### P9. GUI panels reopen the SurrealKV database (with DDL) on a 10–15s poll timer (HIGH)
`src/gui/history.rs:18-42`, `gui/status.rs:14-63`, `gui/stats.rs:328-358`: every tick, each panel calls `LocalStore::open` — fresh KV store open + lock acquisition + 6 `DEFINE TABLE/INDEX` statements (`src/storage/mod.rs:47-71`) + full-table `SELECT * ORDER BY played_at DESC` — then drops the handle. When the daemon holds the lock, the failed open still costs the attempt before falling back to re-reading and re-parsing the entire `live_snapshot.json`. Cost grows with history size, forever, while the window is open.
**Fix:** cache one open `LocalStore` in context; in the snapshot path, skip re-parse when the file mtime is unchanged; lift DDL out of `open` into a once-init.

### P10. `export_snapshot` rewrites the full history after every mutation, with blocking IO on the runtime (MEDIUM)
`src/storage/mod.rs:302-312`, called via `refresh_snapshot` after every capture, back-fill, command batch, and sync: two full-table scans + full serialize + full file rewrite, O(total history) per event, using synchronous `std::fs::write`/`rename` directly on the tokio runtime — inside the already-contended select loop (P1). A single capture can trigger it 2–3×.
**Fix:** debounce (dirty flag, one export per few seconds); move serialize+write into `spawn_blocking`.

### P11. Smaller GUI/daemon costs (LOW)
- `MatchesPanel` re-clones and re-derives full history on every render — `src/gui/history.rs:69-101` (`all.to_vec()` + dedup + day-grouping + chrono formatting per render, including renders from clicking a row). Wrap in `use_memo`.
- DaemonCard's 10s loop spawns `systemctl` synchronously on an async thread and sets signals unconditionally (re-rendering every tick) — `src/gui/daemon.rs:215-221`, `139-145`. Use `tokio::process` and set-on-change.
- `stats_from_rows` takes matches by value forcing a full clone (`src/gui/status.rs:231-233`); `compute_stats` re-runs inside `rsx!` per render (`gui/stats.rs:379`). Take slices; memoize.
- Blocking file IO in async paths: `take_commands` (sync `read_dir`+unlink in the 3s cmd arm — 10s cadence would also do), `append_match_log`, `queue_command` (`src/storage/mod.rs:365-443`).
- Tray quit poller wakes 1 Hz forever, draining one event per wake (`src/gui/main.rs:19-27`). Use blocking `MenuEvent::receiver().recv()`.
- Synchronous Wayland roundtrip in signal initializers on first render (`src/gui/status.rs:40`, `gui/settings.rs:8`). Use `use_resource` + `spawn_blocking`.

### P12. Micro / build (LOW)
- `hsv_white_mask` computes float HSV per pixel but only uses S and V (integer-derivable); per-cell chain allocates ~7 images × ~70 cells (+~300 calibration cells) per capture (`src/ocr/preprocess.rs:488-564`). Fuse mask+grayscale+threshold over `as_raw()` slices.
- `tessdata_lang()` stats the filesystem on every call, ~12+×/Tab (`src/ocr/mod.rs:118-128`). `OnceLock`.
- Needless full-image clones before PNG encode (`src/ocr/mod.rs:146, 407`). Encode from `&GrayImage` via `PngEncoder::write_image`.
- `detect_column_offset` converts the whole scoreboard to RGB to scan the top 2.5% (`src/ocr/preprocess.rs:248-265`). Crop first.
- No `[profile.release]` tuning in the workspace root — `lto = "thin"` would help pixel loops; `[profile.dev.package.image]` opt override would speed the dev loop.

---

## Architecture & code quality

### A1. `run_loop` and `handle_capture` are God-functions with 11 positional parameters each (HIGH)
`src/main.rs:306-645` (~340 lines) and `main.rs:648-915` (~270 lines), both `#[allow(clippy::too_many_arguments)]`; `handle_capture` returns a **9-tuple** from its blocking closure. Adjacent `Option<&str>` params (`player_name`, `capture_output`) are swap-prone with no compiler protection. This is the crate's core logic and its hardest place to change safely.
**Fix:** a `DaemonCtx` struct passed by reference; a named `FrameAnalysis` struct for the tuple; split the `select!` arms into `on_tab()` / `on_poll_tick()` / `on_command_tick()`.

### A2. Portrait-row geometry duplicated — and the copies have already diverged (latent 6v6 bug) (HIGH)
The `--collect-portraits` block in `handle_capture` (`src/main.rs:~820`) recomputes row geometry inline (`row_height = sh*7/100`, linear `row_y`, no team gap), while the canonical `extract_portrait_crops_inner` (`src/detect/hero_portrait.rs:223-253`) uses `h*58/1000` for 6v6 **and** a one-row team gap. In 6v6 games — and for any team-2 player — auto-collected portrait references are cropped from the wrong rows, silently poisoning the template library that `match_player_hero` depends on. (Flagged independently by two agents.)
**Fix:** one `portrait_rect(scoreboard_dims, row_idx, team_size)` in `hero_portrait.rs`, called from both sites.

### A3. Match outcome is stringly-typed across every layer (HIGH)
`MatchOutcome` lives in `src/detect/mod.rs:12`, but `outcome_str`/`outcome_from_str` are free functions in the daemon binary (`src/main.rs:289-303`), storage persists raw strings, and the GUI re-parses literals independently in three places with legacy variants (`src/gui/stats.rs:108-118` accepts `"victory"|"win"`; `gui/history.rs:136-144, 186-205`; `gui/status.rs:~90`). A typo'd literal silently misclassifies games.
**Fix:** `impl Display + FromStr` (+ serde) on `MatchOutcome` in the lib; delete the four scattered translations.

### A4. Error handling: `Box<dyn Error>` + `format!` strings everywhere; `thiserror` declared but never used (HIGH)
All 12 modules return `Box<dyn std::error::Error>` — sometimes `+ Send + Sync`, sometimes not — forcing awkward `map_err(... .into())` conversions (`src/main.rs:~875`) and losing source chains. `Cargo.toml:19` pulls `thiserror = "2"` with zero uses.
**Fix:** `anyhow::Result` with `.context()` in both binaries; drop `thiserror` or introduce real typed errors.

### A5. The daemon↔GUI IPC exists to work around the storage engine choice (MEDIUM)
SurrealKV is single-process, so the crate grew three side channels: the full-rewrite JSON snapshot (P10), `matches.jsonl`, and a 3s-polled file command queue. Nothing here needs SurrealQL; a multi-reader embedded store (SQLite/redb) would delete the snapshot/command machinery, the GUI's "database locked" states, the pid/lock races (m11), and cut compile time. Worth weighing before more features stack on the IPC.

### A6. GUI data-loading copy-pasted across views, policies already drifting (MEDIUM)
The `Config::load` + refresh timer + "open store, else snapshot, else locked" pattern appears in `status.rs:10-62`, `stats.rs:324-360`, `history.rs:11-66` (+ store-opens in `settings.rs:206`, `daemon.rs:209`). Status sets `db_locked`; stats/history silently fall back.
**Fix:** one shared `use_live_matches()` hook.

### A7. ~200 lines of untested stats aggregation live in the GUI (MEDIUM)
`compute_stats` (`src/gui/stats.rs:120-321`) — win rates, per-hero/role/map aggregates, rolling win rate — compiled only under the `gui` feature, zero tests. This is exactly the "my win rate looks wrong" bug class.
**Fix:** move to the lib next to `PersonalMatch`; unit test; view keeps rendering only.

### A8. Outcome/map OCR regions ignore the letterbox correction the scoreboard path uses (MEDIUM)
Scoreboard/career/map crops map through `game_rect_16_9` (`src/ocr/preprocess.rs:346-368`), but `ocr_outcome_word` (`src/detect/match_end.rs:190-206`), `read_accolade_map` (`match_end.rs:168-180`), and the `detect_banner`/`detect_map_vote` sampling bands crop from the raw frame — despite doc comments saying "1/1000ths of the 16:9 frame". On ultrawide/16:10 monitors, scoreboard capture works but win/loss detection quietly degrades — a nasty asymmetry to diagnose.
**Fix:** route these crops through `game_rect_16_9` too.

### A9. Layering nits (MEDIUM-LOW)
- `detect` ↔ `ocr` bidirectional dependency (`src/ocr/mod.rs:12` imports `detect::hero_portrait::detect_team_size`; detect modules call back into `crate::ocr`). Team-size/row-cropping are scoreboard-*layout* concerns — extract a `layout` module to make the chain one-directional.
- `parse_scoreboard_cells` (`src/parse.rs:73-120`) fabricates a full `storage::PersonalMatch`, stamping `Utc::now()` and a SurrealDB datetime — parsing shouldn't read the clock or know the persistence type. Return a plain parsed struct; caller assembles the record.
- Config path/save duplicated between `src/config.rs:69-118` and `gui/settings.rs:251-260`; `Config::load` parses `std::env::args()` inline, making precedence untestable (it has zero tests).

### A10. Low-priority
- **Deps:** `tokio features=["full"]` (rt-multi-thread/macros/signal/time/sync suffice); `image` default features pull every codec for PNG+JPEG use; `thiserror` unused.
- `detect_stat_columns` (`src/ocr/preprocess.rs:227-230`) ignores its argument and returns the fallback constant — the name actively misleads; real calibration is `calibrate_columns`.
- Seven `prepare_*` preprocessing variants, at least one documented as legacy — fine while tuning is active, but worth a variant→call-site table comment.
- `rand_id` (`src/main.rs:958-965`) is time-xor-pid; fine at human cadence, deserves a "not cryptographic" comment.

---

## Minor correctness findings

- **m1. Startup panic on zero Wayland outputs** — `src/main.rs:~128`: `unwrap_or(&outputs[0])` evaluates `&outputs[0]` eagerly, so an empty output list panics even when `capture_output` is configured. Use `.first()`. (Flagged independently by two agents.)
- **m2. Duplicate `match_session` rows after a failed insert** — `create_session` succeeds → `insert_match` fails → `session_created` never set → next Tab re-creates the session; no unique index on `session_id` (`src/storage/mod.rs:63`). Conversely a `create_session` failure is warn-only yet marked created, so later `UPDATE … WHERE session_id` silently matches nothing (`src/main.rs:857-869`).
- **m3. Pending-outcome leak onto the next game** — a confirmed outcome seen with no game open (`src/main.rs:520-524`) is inherited by the next Tab-created session within 90s; a Tab during the next game's opening minute (fast requeue) stamps game B with game A's result, and B being `finished()` blocks recording its real outcome. The inherited outcome also restarts the 75s grace from Tab time, not detection time (`main.rs:394`).
- **m4. `word_outcome_streak` reset only when the poller opens a game** (`src/main.rs:571, 581`), not on Tab-opened sessions — a <60s-old rank-screen read can pair with one stray read and confirm the previous game's outcome onto the new session.
- **m5. Suspend-blind timing** — all windows use `Instant` (stops during suspend). Suspend on the post-match screen, resume tomorrow: the first Tab lands "within the grace window" and appends to yesterday's session with yesterday's outcome.
- **m6. Mid-game banner false-positive via the Tab path** — `handle_capture` runs `detect_banner` when outcome is unknown (`src/main.rs:684-692`); a ≥35% red flood in the mid band (heavy red damage vignette) marks the game finished with a wrong outcome; later Tabs split the real game into a second session.
- **m7.** (= A2) `--collect-portraits` crops the wrong row for team-2 players — mislabeled references silently poison portrait matching.
- **m8. `mark_synced` marks by count/order, not identity** (`src/storage/mod.rs:91-97`) — correct only under single-writer; mark the specific fetched record ids.
- **m9. Row-index compaction** — `filter_map` re-packs rows on mid-row crop failure (`src/ocr/mod.rs:263-276`), desyncing `rows[i]` from on-screen row i that portrait/brightness indices assume (`src/main.rs:697-701`). Low likelihood (only trailing rows fail in practice).
- **m10. Config fragility** — `AutoDetectConfig` has no per-field serde defaults (`src/config.rs:34-39`); a partial `[auto_detect]` table fails to parse and the daemon won't start.
- **m11. GUI/daemon pid + lock races** — `stop_daemon` (`src/gui/daemon.rs:193-205`) removes the pid file right after SIGTERM while the daemon may still be final-syncing; `PidGuard::drop` (`src/main.rs:31-35`) unconditionally deletes whatever pid file exists, including a newly started daemon's. The GUI's 10s `LocalStore::open` polling can transiently hold the KV lock exactly when the daemon starts.

---

## Verified non-issues (for the record)

- The old "always victory" stale-`active_game` class is substantially fixed: idempotent outcome recording (`ActiveGame::finished` guard), finished games force fresh sessions past the grace window, vote-debounce prevents cross-game confirmation, and the "king" map trap has regression tests. Residual leaks are m3/m4/m5 and the M1 merge hole.
- `parse_cell_number` has no silent 0-defaults — unreadable cells reject the row; `looks_like_scoreboard` plausibility gates are solid against OCR garbage. `parse.rs` has no regex at all (strsim only) — no recompilation issue.
- Tesseract instances are cached per (thread, lang); OCR runs on a capped 2–4 thread rayon pool; Sauvola uses O(n) integral images; the `/proc` game gate is cached 5s and short-circuits the whole tick.
- Sync is batched (one POST for all unsynced rows), no per-item requests, no retry-without-backoff loop.
- Local file writes are atomic where it matters (`export_snapshot`, `queue_command` use tmp+rename; `matches.jsonl` is append-only). Storage queries are indexed. No unbounded channels; no full screenshots pass through channels.
- Poll-frame dumping (`save_frame_ring`) is correctly flag-gated — unlike the OCR debug saves in P2.

---

## Testability

Good bones: `parse.rs`, `storage`, `game_running`, `match_end` color logic, and the session state machine all have unit tests; the `#[ignore]`d fixture harnesses (`tests/ocr_replay_benchmark.rs`, `tests/outcome_fixtures.rs`) plus six `examples/` dev tools form a real replay workflow (fixtures local-only by design — personal screenshots). Gaps, in order of value:

1. `compute_stats` (A7) — most user-visible arithmetic, zero tests
2. `detect_team_size` + portrait matching — pure functions over images, ideal fixture tests
3. `match_start` detection — zero tests, threshold-heavy
4. Config precedence (A9)

Detection thresholds (banner 0.35, navy 0.40, team-pitch 0.079) are all named/commented near their use — better than average — but not centralized or regression-pinned by committed fixtures.

---

## Prioritized recommendations

1. **Fix the sync pipeline (C1 + C2 + M4).** Add `session_id` to `StatsUploadEntry` with server-side per-session upsert; filter `unknown` from `get_unsynced`; match `IndexExists` structurally; add a client timeout. Until this lands, server stats are wrong and sync will wedge for every user.
2. **Un-serialize the daemon loop (P1).** Spawn `handle_capture`; this alone should recover most of the missed-banner/accolade detections and shrinks the window for M1/m3/m4.
3. **Gate the debug-PNG pipeline (P2) and lazy-load the fallback OCR (P3), cache calibration (P4).** Together these remove the bulk of per-capture latency — which also feeds back into fix 2.
4. **Extract shared portrait geometry (A2/m7)** before more 6v6 games poison the template library, and **add a staleness bound to unfinished sessions (M1)**.
5. **Fix the map-vote priority inversion (M2) and the Ana/Havana trap (M3)** — both are wrong-data-recorded bugs with small fixes.
6. **Refactor `main.rs` (A1) with a `DaemonCtx` + typed `MatchOutcome` (A3)** — this is prep that makes every fix above safer, and `main.rs` is where fixes 1, 2, 4, 5 all land.
7. Then the medium tail: GUI store-handle caching (P9), snapshot debounce (P10), shared `use_live_matches()` hook (A6), `compute_stats` extraction + tests (A7), letterbox-correct outcome regions (A8).

---

*Review by Claude Fable 5 — four parallel deep-dive agents (capture/OCR perf, GUI/daemon perf, correctness/state, architecture) over ~440k agent tokens, findings verified against function bodies via Serena symbolic analysis.*
