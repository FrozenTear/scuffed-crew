# Stat Tracker — Merged Deep Review (Fable + verified Grok findings)

**Date:** 2026-07-09 · **Branch:** `stat-tracker/add-neon-junction-aatlis` · **Scope:** `crates/stat-tracker` (plus the server-side sync surface)

**Provenance:** This is `fable_review.md` merged with the findings from `grok_review.md` that Fable independently verified against the source. Grok-originated findings are tagged **[Grok]**; findings both reviews reached independently are tagged **[both]**; untagged findings are Fable's. Grok claims that failed verification (or that Fable's evidence contradicts) are in the "Disagreements" section at the end — they are *not* merged into the backlog. Line numbers are 1-based.

---

## Executive summary

Both reviews, produced independently, converge on the same diagnosis — high confidence in the core findings:

1. **The sync pipeline is broken end-to-end** (C1/C2/C3): the server counts every Tab press as a match, sync wedges permanently via two independent mechanisms, and corrections/deletes don't round-trip to the cloud.
2. **The daemon select loop serializes everything** (P1): Tab-capture OCR starves the poller for a measured 45–70s — the root cause of missed outcome banners.
3. **The OCR hot path does enormous unnecessary work** (P2–P4): ungated debug-PNG dumps that re-run preprocessing, an always-on full-image fallback OCR, ~300 calibration OCRs per Tab. Grok's cost model: **~380 Tesseract calls per Tab**, ~2.3s of Tesseract time alone before preprocess and debug IO.
4. **GUI lifecycle has destructive footguns** [Grok]: most notably "Clear All Match Data" deleting the live database out from under a running daemon.
5. **`main.rs` concentrates the change-risk** (A1–A3): two ~300-line God-functions, duplicated portrait geometry that has already diverged, stringly-typed outcomes.

Grok's framing worth preserving: the session-boundary/trust-gate domain logic is unusually careful ("A−" — real production scars encoded in comments and tests), and several defensive behaviors should **not** be optimised away (see "Do not regress" below).

---

## Crate shape

`scuffed-stat-tracker` — two binaries (daemon + Dioxus desktop GUI behind the `gui` feature) sharing a lib. ~7,600 lines in `src/`, ~1,000 in `tests/`, ~650 in `examples/`. Dioxus 0.7, tokio (full), SurrealDB v3 + embedded SurrealKV (single-process), reqwest sync, leptess OCR on a capped 2–4 thread rayon pool with thread-local `LepTess` reuse (~77ms cold → ~6ms warm).

| Module | Lines | Responsibility |
|---|---|---|
| `src/main.rs` | 1077 | Daemon: CLI, PID guard, `ActiveGame` session state machine, `run_loop` (Tab / poll / command queue / signals), `handle_capture`, sync + snapshot helpers |
| `src/capture/` | 115 | libwayshot preferred, ashpd portal fallback |
| `src/detect/` | 1158 | Banner color-flood + VICTORY/DEFEAT OCR, match-start phases, `/proc` game gate (cached 5s), portrait template matching |
| `src/ocr/` | 1296 | leptess wrapper, per-cell scoreboard OCR, column calibration, preprocessing (HSV mask / Sauvola / morphology) |
| `src/parse.rs` | 754 | OCR rows → `PersonalMatch`; fuzzy hero/map matching, plausibility gates. Well tested |
| `src/storage/` | 620 | SurrealKV store, `live_snapshot.json`, `matches.jsonl`, file-based GUI→daemon command queue |
| `src/sync/` | 88 | reqwest: upload matches, fetch daemon config |
| `src/setup.rs` | 553 | Tessdata training pipeline |
| `src/gui/` | ~2,700 | status/stats/history/settings/preview views, daemon control card, tray |

Because SurrealKV is single-process, daemon↔GUI communication flows through three side channels: the JSON snapshot, the JSONL log, and a 3s-polled file command queue (see A5).

---

## Critical

### C1. Server sync counts every Tab-press snapshot as a separate match [both]
`src/main.rs:866` inserts one `personal_match` row per Tab capture; `get_unsynced` (`src/storage/mod.rs:82-89`) returns all of them; `upload_matches` (`src/sync/mod.rs:42-86`) sends them all — and `StatsUploadEntry` (`crates/types/src/api/stats.rs:5-24`) has **no `session_id` field**, so the server cannot collapse them. Locally the GUI collapses via `latest_per_game` (`src/storage/mod.rs:486-491`); the server has no equivalent, and its dedup index `(member_id, hero, map_name, played_at)` (`crates/db/src/migrations.rs:531-532`) never matches because each snapshot gets a distinct `played_at` stamped `Utc::now()` at parse time (`src/parse.rs:117`). **A game where the user tabs 6 times = 6 wins/losses in server stats.**
**Fix:** include `session_id` in the upload with server-side per-session upsert, or upload only the final snapshot per session (on session close + shutdown).

### C2. Sync wedges permanently — two independent triggers, same terminal state
- **`unknown` outcomes violate the server ASSERT** [both]. `try_sync` runs mid-game every 5th capture (`src/main.rs:444`) while snapshots still carry `outcome = "unknown"`; the server asserts `outcome IN ['victory','defeat','draw']` (`crates/db/src/migrations.rs:519-520`) → HTTP 500 → `mark_synced` never runs. `get_unsynced` orders `played_at ASC`, so one permanently-unknown row **head-of-line-blocks all future syncs forever**.
- **The duplicate-skip guard can never fire** (Fable-only; changes the fix). `bulk_insert_personal_matches` checks `e.to_string().contains("unique")` (`crates/db/src/queries/personal_stats.rs:127`), but SurrealDB v3's unique-violation message is `` "Database index `pm_dedup_idx` already contains {value}, with record ..." `` — no substring "unique" (verified against surrealdb-core source). The server is therefore **not** insert-or-skip on duplicates: any retry after a partial upload, and any outcome back-fill (`set_session_outcome` sets `synced = false` on already-uploaded rows with an unchanged dedup key) → IndexExists → 500 → wedge. The *routine* flow (capture with unknown → back-fill → re-sync) hits this quickly.
- **Corollary — map corrections duplicate server rows** [both]: `set_session_map` (`src/storage/mod.rs:225-236`) changes `map_name`, part of the dedup key, so the corrected row inserts as a new server row while the stale old-map row remains.

**Fix:** match on the structured `IndexExists` error (not a string), filter `outcome == "unknown"` out of `get_unsynced` (or relax the server assert), and make corrections an upsert keyed by a client-generated id.

### C3. "Clear All Match Data" deletes the live database out from under a running daemon [Grok — verified]
`src/gui/settings.rs:206-212`: when the store lock can't be acquired — which is *precisely* when the daemon holds it — the fallback path calls `force_clear_data_dir` (`src/storage/mod.rs:503-511`), which `remove_dir_all`s `stats.surrealkv` while the daemon has it open, plus `clear_match_log`. No confirmation dialog. The daemon continues writing into a deleted directory tree; behavior after that is undefined (silent data loss at best).
**Fix:** disable clear while the daemon is up (or stop → clear → restart), and add a confirm step.

---

## Major — correctness

### M1. Unfinished sessions never expire → merged games (surviving sibling of the old active_game bug)
`should_start_fresh_session` (`src/main.rs:239-253`): `Some(g) if !g.finished() => false` unconditionally — no staleness bound on an *unfinished* game. If the poller misses both the outcome and the next game's start screens (likely, given P1's 45–70s starvation), or if `auto_detect.enabled = false` (the poll arm never runs), every Tab across multiple games appends to one session. Worst case: yesterday's unfinished session absorbs today's first game. The grace window only bounds *finished* games.
**Fix:** track last-capture time on `ActiveGame`; start fresh past a plausible match length (~20 min).

### M2. Map-vote *candidates* are recorded as the played map, with top priority
The MapVote arm stores every map OCR'd off the vote screen (`src/main.rs:558-571`); `extract_map_names` returns them in `MAP_NAMES` constant order — not winner order (the winner is unknowable at vote time). `handle_capture` gives `detected_maps[0]` priority over the top-bar OCR of the *actual* map (`main.rs:839-840`, verified: `parsed.map_name = detected_maps[0].clone()` unconditionally when non-empty), and accolade-map recovery plus `set_session_map` are gated on `g.maps.is_empty()`, so nothing can ever correct it. With ≥2 OCR'd candidates the recorded map is wrong ~2/3 of the time. Vote names are also UPPERCASE constants (`"KING'S ROW"`) while every other path stores display names — fracturing per-map aggregates locally and on the server. *(Grok listed the vote→panel→fuzzy priority as a strength; see Disagreements.)*
**Fix:** treat vote maps as candidates only (constrain the later top-bar/accolade read); canonicalize map strings through the `MAPS` table.

### M3. Hero substring trap: "Ana" matches inside "HAVANA"; tie-break picks the *least*-mentioned hero
`find_hero` pass 1 (`src/parse.rs:332-343`) substring-matches hero names against joined raw text — same bug class as the fixed "king" map trap. On Havana, a bare `HAVANA` line satisfies the `num_count <= 1` branch (`parse.rs:353-364`) → hero = "Ana". The tie-break `found.sort_by_key(|&(_, count)| count)` (`parse.rs:373-374`) sorts **ascending** — fewest mentions wins, ties resolve alphabetically (Ana first). Fallback-path only, diluted by `majority_hero`, but support-Havana games can flip.
**Fix:** word-boundary matching for short names (Ana/Mei/Echo/Juno); strip the map-label line before hero search.

### M4. No HTTP timeout on the sync client stalls the entire daemon [both]
`SyncClient::new` uses `reqwest::Client::new()` with no timeout (`src/sync/mod.rs:14-19`), and `try_sync` is awaited inline inside the `select!` arms. A hung connection blocks Tab capture, polling, GUI commands — and the SIGTERM/Ctrl-C paths, making the daemon unkillable-gracefully.
**Fix:** `Client::builder().timeout(30s)`; run sync as a spawned task off the loop.

### M5. `pending_outcome` handling leaks across game boundaries [both — complementary findings]
Two facets of the same design gap:
- **[Grok]** The poller never consumes or clears `pending_outcome` when it opens a game — verified: the only consume site is `take_fresh_pending` on the Tab path (`src/main.rs:389`); MapVote/HeroSelect open `ActiveGame` with `Unknown` and leave pending sitting for its 90s TTL.
- **[Fable]** A Tab pressed during the *next* game's opening minute (fast requeue) inherits game A's outcome onto game B (`main.rs:520-524`), and B being `finished()` then blocks recording its real outcome; the inherited outcome also restarts the 75s grace from Tab time, not detection time (`main.rs:394`).
**Fix:** apply-or-clear pending when the poller opens a game; never inherit a pending outcome into a session whose start was detected *after* the outcome was seen.

### M6. Daemon restart loses the in-memory `ActiveGame` [Grok — verified]
The session state machine lives entirely in `run_loop` locals. A daemon restart (crash, upgrade, systemd restart) mid-game splits the game: post-restart Tabs open a fresh session, and any pending outcome/map context is gone.
**Fix:** persist the open session skeleton (session_id, started_at, maps) to the store or a small state file; recover on startup within a staleness bound (dovetails with M1's last-capture timestamp).

### M7. Deletes and corrections don't round-trip to the cloud [Grok — verified]
`delete_session` (`src/storage/mod.rs:179-187`) is local-only — its own doc comment says "Already-synced snapshots remain on the server." Combined with C1/C2, junk or misdetected games cleaned up locally persist in server aggregates forever.
**Fix:** part of the same sync-contract redesign as C1/C2 — client-generated ids + server upsert/delete endpoints.

---

## Performance — daemon & hot path

Grok's Tab budget (consistent with Fable's findings): calibration ~300 + final cells ~70 + full-board 1–4 + panels 2 ≈ **~380 Tesseract calls per Tab**; at ~6ms warm each that's ~2.3s of Tesseract alone, multi-second wall clock once preprocess + debug IO are included.

### P1. Tab-capture OCR serializes the entire daemon select loop (HIGH) [both]
`src/main.rs:405`: `handle_capture` is awaited inline inside `tokio::select!`; poll ticks are skipped (`MissedTickBehavior::Skip`) — the code's own comment (`main.rs:351`) documents measured 45–70s gaps. The ~3s VICTORY/DEFEAT banner and ~20s accolade screen get missed; this is the root cause of the outcome fragility that M1/M5 compensate for. The `sleep(400ms)` at `main.rs:381` stalls the loop the same way.
**Fix:** spawn `handle_capture` (single-flight guard), report back via channel; parallelize independent OCR passes inside the closure.

### P2. Ungated debug-image pipeline re-runs full preprocessing twice per Tab (HIGH) [both]
`src/ocr/mod.rs:278-285` + `save_debug_images` (`mod.rs:486-495`): every capture calls `save_debug_stages` (`src/ocr/preprocess.rs:766-787`), which **recomputes the entire pipeline** (HSV mask + Sauvola + morphological close on the ~1664×1008 scoreboard) purely to save stage PNGs — ~15–17 PNG encodes + disk writes per Tab, no flag gate (unlike `--dump-poll-frames`, which is gated). Also hardcodes `dirs::data_dir()`, ignoring `config.data_dir`.
**Fix:** gate behind a config/env flag (e.g. `STAT_TRACKER_DEBUG_OCR=1`); save already-computed intermediates; pass the debug dir in.

### P3. Full-image fallback OCR always runs even when the per-cell path succeeds (HIGH) [both]
`src/main.rs:704`: `ocr::recognize(&img)` re-crops, runs the full adaptive pipeline, a whole-board Tesseract pass, and up to three more threshold sweeps when confidence < 65. Output only feeds hero/map lookup and the name-anchored row fallback.
**Fix:** run lazily — only when the row-by-name/brightness path or the career/map panel reads failed.

### P4. Column calibration ≈ 300 cell OCRs per Tab, no early exit, no caching (HIGH) [both]
`src/ocr/mod.rs:292-369`: coarse ~8 + fine 9 offsets × (3 probe rows × 6 cells) each, every one a crop + HSV mask + PNG encode + Tesseract, before the real 60–70 cell reads. The comment says calibration "dominated capture latency".
**Fix:** early-exit at max score; don't re-score coarse offsets in the fine sweep; cache the winning offset per session/resolution, re-sweep only when validity drops. [Grok adds:] consider scoring candidate offsets *without* Tesseract (cheap ink/column-alignment heuristics) — the offset only needs relative ranking, not text.

### P5. New Wayland connection per screenshot (MEDIUM) [both]
`src/capture/wayshot.rs:14-48`: every 4s tick and every Tab creates a fresh `WayshotConnection` (handshake, output enumeration, shm/dmabuf setup), then tears it down.
**Fix:** create once, reuse, recreate on error; resolve the output name once.

### P6. Up to 4× full-frame RGBA→RGB conversions + dense unstrided pixel scans per poll tick (MEDIUM) [both]
`detect_map_vote`/`detect_hero_ban`/`detect_hero_select` each do their own `img.to_rgb8()` (`src/detect/match_start.rs:19, 68-69, 121-122`) plus `detect_banner`'s (`src/detect/match_end.rs:98`) — ~11MB and 3.7M pixels each, and on normal in-game frames none early-return. Scans use per-pixel `get_pixel` with no stride.
**Fix:** convert once per tick, pass `&RgbImage` down; iterate `as_raw()` chunks; the ratio tests tolerate 1-in-2/1-in-4 subsampling.

### P7. Redundant work per Tab (MEDIUM) [both]
`detect_team_size` runs twice (`src/main.rs:696` + inside `match_player_hero`, `src/detect/hero_portrait.rs:102`), each a full scoreboard `to_rgb8` + saturation scan; the scoreboard crop is computed 2–3× (`main.rs:695`, `ocr/mod.rs:252, 402`), ~6.7MB per copy.
**Fix:** compute once, pass down.

### P8. 1–2 unconditional Tesseract calls per idle poll tick (MEDIUM-LOW)
`src/detect/match_end.rs:38-45`: `read_result_word` always runs; `read_rank_screen_result` whenever the first returns Unknown (virtually every tick). A cheap crop-local brightness-histogram probe would skip both on the overwhelming majority of ticks.

### P9. Micro / structural OCR costs (LOW)
- PNG encode on every Tesseract call (`encode_png`, `src/ocr/mod.rs:146, 407` — with needless full-image clones); [Grok adds:] feeding raw/grayscale buffers to Tesseract, if leptess's API allows, would save an encode × ~380 calls.
- Nested rayon (rows × cells) on a 2–4 thread pool [Grok] — flatten to one par-iter over all cells.
- `hsv_white_mask` computes float HSV incl. unused H via `get_pixel`/`put_pixel` (`src/ocr/preprocess.rs:488-564`); ~7 image allocations × ~70 cells + ~300 calibration cells per Tab. Integer S/V test over `as_raw()`, fused into one pass.
- `tessdata_lang()` stats the filesystem ~12+×/Tab (`src/ocr/mod.rs:118-128`). `OnceLock`.
- `detect_column_offset` converts the whole scoreboard to RGB to scan the top 2.5% (`src/ocr/preprocess.rs:248-265`).
- Fixed 4s poll with no adaptive interval [Grok] — idle vs in-game vs post-match could poll at different rates.
- No `[profile.release]` tuning (`lto = "thin"`); no `[profile.dev.package.image]` opt override for the dev loop.

---

## Performance — GUI / storage / sync

### P10. GUI panels reopen the SurrealKV database (with DDL) on a 10–15s poll timer (HIGH)
`src/gui/history.rs:18-42`, `gui/status.rs:14-63`, `gui/stats.rs:328-358`: every tick, each panel calls `LocalStore::open` — fresh KV open + lock acquisition + 6 `DEFINE TABLE/INDEX` statements (`src/storage/mod.rs:47-71`) + full-table `SELECT * ORDER BY played_at DESC` — then drops the handle. When the daemon holds the lock, the failed attempt still costs, then falls back to re-reading and re-parsing the entire `live_snapshot.json`. Grows with history size, forever, while the window is open. This polling also races the daemon's startup lock acquisition (m11).
**Fix:** cache one open `LocalStore` in context; skip snapshot re-parse when file mtime unchanged; lift DDL into a once-init.

### P11. `export_snapshot` rewrites the full history after every mutation, blocking the runtime (MEDIUM) [both]
`src/storage/mod.rs:302-312`, called after every capture, back-fill, command batch, and sync: two full-table scans + full serialize + full rewrite, O(total history), synchronous `std::fs::write`/`rename` on the tokio runtime inside the already-contended loop. A single capture triggers it 2–3×. Related [Grok]: snapshot grows unboundedly (all Tab rows forever — no retention of final-only rows after a session closes).
**Fix:** debounce; `spawn_blocking` the serialize+write; consider dropping intermediate snapshots after session finalize.

### P12. Smaller GUI/daemon costs (LOW)
- `MatchesPanel` re-clones and re-derives full history every render (`src/gui/history.rs:69-101`) — `use_memo`.
- DaemonCard's 10s loop spawns `systemctl` synchronously on an async thread, sets signals unconditionally (`src/gui/daemon.rs:215-221, 139-145`) — `tokio::process`, set-on-change.
- `stats_from_rows` takes matches by value (`src/gui/status.rs:231-233`); `compute_stats` re-runs inside `rsx!` per render (`gui/stats.rs:379`) — slices + memoize.
- Blocking file IO in async paths: `take_commands` (3s cadence is also aggressive; 10s would do), `append_match_log`, `queue_command` (`src/storage/mod.rs:365-443`).
- Tray quit poller wakes 1 Hz forever, one event per wake (`src/gui/main.rs:19-27`) — blocking `MenuEvent::receiver().recv()`. Also [Grok]: tray quit uses `process::exit(0)`, skipping cleanup, and the tray-icon `expect` can panic the GUI.
- Synchronous Wayland roundtrip in signal initializers on first render (`src/gui/status.rs:40`, `gui/settings.rs:8`) — `use_resource` + `spawn_blocking`.

---

## Architecture & code quality

### A1. `run_loop` and `handle_capture` are God-functions with 11 positional parameters each (HIGH) [both]
`src/main.rs:306-645` (~340 lines) and `main.rs:648-915` (~270 lines); `handle_capture` returns a **9-tuple**. Adjacent `Option<&str>` params are swap-prone with no compiler protection.
**Fix:** `DaemonCtx` struct; named `FrameAnalysis` for the tuple; split `select!` arms into methods.

### A2. Portrait-row geometry duplicated — copies have already diverged (latent 6v6 bug) (HIGH) [both]
The `--collect-portraits` block (`src/main.rs:~820`) recomputes geometry inline (linear `row_y`, `sh*7/100`, no team gap) while canonical `extract_portrait_crops_inner` (`src/detect/hero_portrait.rs:223-253`) uses `h*58/1000` for 6v6 **and** a one-row team gap. 6v6 games and team-2 players get mis-cropped references, silently poisoning the template library.
**Fix:** one `portrait_rect(scoreboard_dims, row_idx, team_size)` called from both sites.

### A3. Match outcome is stringly-typed across every layer (HIGH)
`MatchOutcome` lives in `src/detect/mod.rs:12`; `outcome_str`/`outcome_from_str` are free functions in the binary (`src/main.rs:289-303`); the GUI re-parses literals in three places with legacy variants (`src/gui/stats.rs:108-118`, `gui/history.rs:136-144, 186-205`, `gui/status.rs:~90`).
**Fix:** `impl Display + FromStr` (+ serde) on `MatchOutcome`; delete the scattered translations.

### A4. Error handling: `Box<dyn Error>` everywhere; `thiserror` declared but never used (HIGH) [both]
Inconsistent `+ Send + Sync` bounds force awkward `map_err(... .into())` (`src/main.rs:~875`); string errors lose source chains.
**Fix:** `anyhow::Result` + `.context()` in the binaries; drop or actually use `thiserror`.

### A5. The daemon↔GUI IPC exists to work around the storage engine choice (MEDIUM) [both]
SurrealKV is single-process → three side channels (snapshot, JSONL, polled command queue). Nothing needs SurrealQL; SQLite/redb would delete the snapshot/command machinery, the "database locked" states, the pid/lock races, and cut compile time. [Grok adds two concrete integrity holes in the current design:]
- **Command queue is delete-then-apply** — `take_commands` (`src/storage/mod.rs:419-443`) unlinks command files before the daemon applies them; a crash between unlink and apply silently loses GUI edits. Two-phase (apply, then delete) fixes it cheaply.
- **`matches.jsonl` is never updated on back-fill** — verified: `append_match_log` is only called at insert (`src/main.rs:876`); `set_session_outcome`/`set_session_map` don't touch it, so the JSONL fallback permanently shows `unknown` outcomes for back-filled games.

### A6. GUI data-loading copy-pasted across views, policies drifting (MEDIUM) [both]
`status.rs:10-62`, `stats.rs:324-360`, `history.rs:11-66` (+ store-opens in `settings.rs:206`, `daemon.rs:209`); status sets `db_locked`, stats/history silently fall back.
**Fix:** one shared `use_live_matches()` hook.

### A7. ~200 lines of untested stats aggregation live in the GUI (MEDIUM)
`compute_stats` (`src/gui/stats.rs:120-321`) — the most user-visible arithmetic in the app, compiled only under the `gui` feature, zero tests.
**Fix:** move to the lib; unit test; view renders only.

### A8. Outcome/map OCR regions ignore the letterbox correction the scoreboard path uses (MEDIUM)
Scoreboard/career/map crops map through `game_rect_16_9` (`src/ocr/preprocess.rs:346-368`), but `ocr_outcome_word` (`src/detect/match_end.rs:190-206`), `read_accolade_map` (`match_end.rs:168-180`), and the banner/vote sampling bands crop from the raw frame. On ultrawide/16:10, scoreboard capture works while win/loss detection quietly degrades. *(Grok listed `game_rect_16_9` as a strength without noticing the bypass; see Disagreements.)*
**Fix:** route these crops through `game_rect_16_9` too.

### A9. Daemon lifecycle & GUI robustness [Grok — verified]
- **Settings don't hot-reload:** the daemon reads config once at startup (`src/main.rs:86` area); changing player name, token, output, or auto-detect in the GUI is a silent no-op until restart. Fix: "restart required" toast + optional restart/SIGHUP.
- **PID liveness ≠ daemon identity:** status/stop only check `/proc/{pid}` existence; PID reuse risks signalling the wrong process. Verify the process name/exe before kill. Related (Fable m11): `stop_daemon` removes the pid file right after SIGTERM while the daemon may still be final-syncing, and `PidGuard::drop` unconditionally deletes whatever pid file exists — including a newly started daemon's.
- **Dashboard hardcodes the capture backend** as `"libwayshot (Wayland)"` (`src/gui/status.rs:145`) regardless of the actual backend.
- **Portal backend ignores `capture_output`** (verified: `portal.rs` never references an output) and its `is_available` only checks `XDG_CURRENT_DESKTOP`.
- **`config.toml` (bearing the sync token) gets default permissions** — no `set_permissions` anywhere in the crate; chmod 600 on save.

### A10. Layering nits (MEDIUM-LOW)
- `detect` ↔ `ocr` bidirectional dependency (`src/ocr/mod.rs:12` vs detect modules calling `crate::ocr`) — extract a `layout` module.
- `parse_scoreboard_cells` (`src/parse.rs:73-120`) fabricates a full `storage::PersonalMatch`, stamping `Utc::now()` and a SurrealDB datetime — return a plain parsed struct; caller assembles. [Grok adds:] `PersonalMatch` means "capture" locally but "game" on the server — overloaded naming that invites exactly the C1 class of bug.
- Config path/save duplicated (`src/config.rs:69-118` vs `gui/settings.rs:251-260`); `Config::load` parses `std::env::args()` inline — precedence untestable, zero tests.
- Dead/misleading API surface [both]: `detect_stat_columns` (`src/ocr/preprocess.rs:227-230`) ignores its argument and returns the fallback constant; [Grok] `find_active_session`, `session_window_secs`, `update_session`, `get_multi_capture_sessions`, and the legacy `parse_scoreboard` (with an unsafe row fallback the production path correctly avoids) are unused — gate or remove per the repo's pending-wiring convention.

### A11. Low-priority hygiene [both]
`tokio features=["full"]`; `image` default features (every codec for PNG+JPEG use); `thiserror` unused; [Grok] `tempfile` in normal deps; seven `prepare_*` preprocessing variants (worth a variant→call-site table comment); `rand_id` deserves a "not cryptographic" comment; [Grok] no crate README stating the Linux/Wayland/evdev/`input`-group/tessdata requirements.

---

## Minor correctness findings

- **m1. Startup panic on zero Wayland outputs** [both] — `src/main.rs:~128`: `unwrap_or(&outputs[0])` evaluates eagerly; empty output list panics even with `capture_output` configured. `.first()`.
- **m2. Duplicate `match_session` rows after a failed insert** — `create_session` succeeds → `insert_match` fails → `session_created` never set → next Tab re-creates; no unique index on `session_id` (`src/storage/mod.rs:63`). Conversely a `create_session` failure is warn-only yet marked created (`src/main.rs:857-869`).
- **m3. `word_outcome_streak` reset only when the poller opens a game** (`src/main.rs:571, 581`), not on Tab-opened sessions — a <60s-old rank-screen read can pair with one stray read and confirm the previous game's outcome onto the new session.
- **m4. Suspend-blind timing** — all windows use `Instant` (stops during suspend). Suspend on the post-match screen, resume tomorrow: first Tab lands "within grace" and appends to yesterday's session with yesterday's outcome.
- **m5. Mid-game banner false positive via the Tab path** — `handle_capture` runs `detect_banner` when outcome is unknown (`src/main.rs:684-692`); a ≥35% red flood (heavy red damage vignette) marks the game finished with a wrong outcome; later Tabs split the real game.
- **m6. `mark_synced` marks by count/order, not identity** [both] (`src/storage/mod.rs:91-97`) — mark the specific fetched record ids.
- **m7. Row-index compaction** — `filter_map` re-packs rows on mid-row crop failure (`src/ocr/mod.rs:263-276`), desyncing `rows[i]` from on-screen indices that portrait/brightness logic assumes (`src/main.rs:697-701`). Low likelihood.
- **m8. Config fragility** — `AutoDetectConfig` has no per-field serde defaults (`src/config.rs:34-39`); a partial `[auto_detect]` table fails to parse and the daemon won't start.
- **m9. [Grok] Team-size detection has no hysteresis** near the row-pitch threshold — a 5↔6 flap between captures in one session mislabels rows both ways.
- **m10. [Grok] Two agreeing word-OCR hallucinations still confirm an outcome** — the confirm window requires agreement, not plausibility; the same misread twice passes.
- **m11. [Grok] Total keyboard loss exits the daemon** — acceptable under a systemd restart unit, silent otherwise.

---

## Do not regress (verified strengths — both reviews)

Grok's framing, Fable-verified: these defensive behaviors are why local aggregates stay trustworthy. Don't trade them away while optimising.

- `parse_scoreboard_cells` refuses unidentified player rows — no "first plausible row" teammate theft; `parse_cell_number` has no silent 0-defaults (unreadable cells reject the row); `looks_like_scoreboard` + kill-column plausibility gates.
- The outcome multi-signal stack: banner immediate, word-OCR needs 2 agreeing reads, asymmetric trust, TTLs; idempotent outcome recording (`ActiveGame::finished` guard); the "king" map trap fixed with regression tests.
- Portrait path refuses "best teammate portrait" fallback; hero priority career-panel > portrait > text with majority-vote across captures.
- Thread-local LepTess reuse; capped OCR pool; per-cell ROI + numeric whitelists; Sauvola integral images; 5s-cached `/proc` gate that short-circuits the tick; `MissedTickBehavior::Skip`; bounded flag-gated debug rings (poll frames, rejected frames); atomic tmp+rename writes; batched single-POST sync; indexed queries; no unbounded channels.
- The operator tooling: six `examples/` dev tools + `#[ignore]`d fixture-replay harnesses form a real diagnosis workflow.

---

## Disagreements with the Grok review

Recorded so the two reviews can be reconciled; these Grok claims are **not** merged above.

1. **"Map priority: vote → panel → fuzzy" listed as a strength.** Verified wrong: `main.rs:839-840` unconditionally overrides with `detected_maps[0]`, vote candidates come back in constant order (winner unknowable at vote time), and `maps.is_empty()` gates block correction. It's a wrong-data bug (M2), not a strength.
2. **Server dedup characterized as "insert-or-skip".** The skip guard `contains("unique")` never matches SurrealDB v3's error text, so duplicates produce a 500 that wedges the whole queue (C2) — materially worse than described, and the fix differs (structured `IndexExists` matching, not just upsert).
3. **`game_rect_16_9` listed as a strength for ultrawide/16:10.** True for scoreboard/career/map-panel crops, but the outcome word, accolade map, and banner/vote bands bypass it (A8) — precisely the paths that degrade on non-16:9 displays.

---

## Consolidated priority backlog

**P0 — data correctness, safety, and the critical path**
1. Fix the sync contract (C1 + C2 + M7): `session_id` in the upload + server per-session upsert; filter `unknown` from `get_unsynced`; match `IndexExists` structurally; client HTTP timeout (M4); sync off the select loop.
2. Never delete the live DB under a running daemon (C3): disable clear while daemon is up + confirm dialog.
3. Un-serialize the daemon loop (P1): spawn `handle_capture` — recovers missed banners/accolades and shrinks the M1/M5 windows.
4. Gate the debug-PNG pipeline (P2).

**P1 — large latency and robustness wins**
5. Cache/shrink column calibration (P4); lazy full-image `recognize()` (P3).
6. Session-boundary hardening: staleness bound on unfinished sessions (M1), pending-outcome apply-or-clear (M5), persist/recover `ActiveGame` across restart (M6).
7. Map-vote priority inversion (M2) + Ana/Havana trap (M3) — wrong-data bugs, small fixes.
8. Shared portrait geometry (A2) before more 6v6 games poison the template library.
9. `mark_synced` by ids (m6); two-phase command queue (A5); settings restart-required + PID identity check (A9).
10. Single RGB conversion + subsampled scans per poll tick (P6); reuse the Wayshot connection (P5).

**P2 — hygiene and quality of life**
11. GUI store-handle caching (P10) + snapshot debounce/retention (P11) + shared `use_live_matches()` (A6).
12. Typed `MatchOutcome` (A3); `DaemonCtx` refactor of `main.rs` (A1); anyhow error strategy (A4).
13. `compute_stats` to lib + tests (A7); letterbox-correct outcome regions (A8); JSONL back-fill updates (A5).
14. Real backend label on dashboard, portal output handling, chmod 600 config, crate README, dead-API gating (A9/A10); team-size hysteresis (m9); dep trimming + release profile (A11).

**P3 — longer term**
15. Multi-reader embedded store to delete the IPC machinery (A5); incremental OCR (player row only after first full board); adaptive poll interval; CI smoke with synthetic scoreboard images + latency bands from `polltick`; keyboard-accessible match rows.

---

*Merged by Claude Fable 5 from `fable_review.md` (four deep-dive agents, findings verified via Serena symbolic analysis) and `grok_review.md` (five agents); every Grok-originated finding above was re-verified against source before inclusion, and unverifiable or contradicted claims are quarantined in the Disagreements section.*
