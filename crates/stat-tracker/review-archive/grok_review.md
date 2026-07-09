# Stat Tracker — Deep Code Review

**Reviewer:** Grok (multi-agent deep dive)  
**Date:** 2026-07-09  
**Scope:** `crates/stat-tracker` — performance optimisations, architecture, correctness, GUI, storage/sync  
**Method:** Five parallel research agents (OCR/preprocess, capture/detect loop, storage/sync, GUI/correctness, end-to-end pipeline) plus primary synthesis against source.

---

## Executive summary

The stat tracker is a **mature Linux desktop vision pipeline**: Wayland capture → screen heuristics + Tesseract OCR → session state machine → SurrealKV → optional cloud sync, with a Dioxus companion GUI. Domain logic around **session boundaries, trust gates, and outcome confirmation** is unusually careful for a personal tooling crate — real production scars are encoded in comments and unit tests.

**Performance debt is concentrated on the Tab capture path**, not the idle poller. A single Tab can trigger on the order of **~300–400 Tesseract calls** (column calibration alone is ~300), plus unconditional multi-PNG debug dumps and a redundant full-scoreboard OCR. That work runs on the same `tokio::select!` loop as auto-detect, so capture can starve outcome detection for tens of seconds (documented 45–70s gaps).

**Data-layer / sync contract is the other weak spot:** every Tab snapshot is stored and uploaded as if it were a full match; server stats count rows, while the local GUI collapses with `latest_per_game`. Corrections (outcome/map) and deletes do not round-trip cleanly to the server.

**Highest leverage (in order):**

1. Gate/remove production debug image dumps on every Tab  
2. Cut or cache column-calibration OCR  
3. Skip full-image `recognize()` when cell OCR already succeeds  
4. Decouple poll detection from Tab OCR (so banners aren’t missed)  
5. Sync **one row per game** with a stable client id / server upsert  
6. Never force-delete the live DB while the daemon holds it open  

---

## Architecture overview

```
┌─────────────────────┐     live_snapshot.json      ┌──────────────────────┐
│  GUI (dioxus, optional) │◄──────────────────────│  Daemon (main loop)    │
│  SurrealKV if free  │── commands/*.json ────────►│  owns SurrealKV       │
└─────────────────────┘                            │  Tab + poll + sync    │
                                                   └──────────┬───────────┘
                                                              │ HTTPS bearer
                                                              ▼
                                                       /api/stats/*
```

| Module | Role | Approx. size |
|--------|------|--------------|
| `main.rs` | Daemon event loop, session FSM, Tab/poll orchestration | ~1.1k LOC |
| `ocr/` | Tesseract pool, cell OCR, calibration, preprocess | ~1.3k |
| `parse.rs` | Scoreboard → `PersonalMatch`, trust gates | ~0.75k |
| `detect/` | Game process, match start/end, portraits | ~1.1k |
| `storage/` | SurrealKV + file IPC | ~0.6k |
| `gui/` | Dashboard, history, stats, settings, tray | ~2.8k |
| `sync/` | HTTP upload client | ~0.1k |
| `capture/` | Wayshot / portal | small |

**Threading model (sound overall):**

- Single-threaded async `select!` owns the store and session state  
- Capture + vision + OCR run in `spawn_blocking`  
- OCR on a **capped Rayon pool (2–4 threads)** to avoid lagging the game  
- Thread-local `LepTess` reuse (load ~77 ms → warm call ~6 ms)  

---

## End-to-end pipeline

```
Game process gate (/proc, 5s cache)
        │
        ├─ POLL (~4s): full capture → banner → word OCR → phase (map/ban/select)
        │                 → ActiveGame open / outcome confirm / map recover
        │
        └─ TAB: debounce 3s → wait 400ms → full capture → heavy OCR → trust gates
                  → create/append session → insert snapshot → snapshot export
                  → every 5 captures: sync
```

### Session state machine (strength)

| Constant | Value | Purpose |
|----------|-------|---------|
| `POST_MATCH_GRACE` | 75s | Post-match Tab still belongs to finished game |
| `OUTCOME_CONFIRM_WINDOW` | 60s | Two agreeing word-OCR reads (non-consecutive OK) |
| `PENDING_OUTCOME_TTL` | 90s | Outcome seen with no open game |
| Map-vote debounce | 120s default | Lingering vote screen ≠ many sessions |
| Banner vs word | Banner immediate; word needs 2× | Asymmetric evidence strength |

Unit tests cover pure helpers (`should_start_fresh_session`, pending TTL). This is some of the best domain code in the crate.

### Known state-machine hole

**`pending_outcome` is only consumed on the Tab path** when opening a fresh session. Auto-detect MapVote / HeroSelect always opens `ActiveGame` with `Unknown` and leaves pending sitting. Combined with the 90s TTL, a late fresh Tab could stamp an old outcome onto a different match. Prefer: apply (or explicitly clear) pending when opening a new game from the poller.

---

## Performance findings

### Critical path cost model

| Path | Dominant cost | Rough expectation |
|------|---------------|-------------------|
| Poll tick (detect only) | Banner scan + optional title OCR | Config claims ~25–35 ms; capture extra |
| Poll tick (production) | Capture + 1–3× `to_rgb8` + OCR | Often higher than the config claim |
| **Tab capture** | Calibration + cell OCR + full OCR + debug PNG | **Seconds** |
| Sync | One POST of all unsynced | Usually small vs OCR |

### High — Tab path work volume

#### H1. Column calibration ≈ hundreds of OCR calls per Tab

`calibrate_columns` / `count_valid_cells` (`src/ocr/mod.rs`): each candidate offset scores **3 probe rows × 6 cells**. Coarse + fine sweeps ≈ **~17 offsets × 18 ≈ ~300 cell OCR** before the real board OCR starts. Parallelising the offset loop improved wall time; **total CPU work is still huge**.

#### H2. Unconditional debug PNG dumps on every scoreboard OCR

```278:285:src/ocr/mod.rs
    if let Some(data_dir) = dirs::data_dir() {
        let debug_dir = data_dir.join("scuffed-stat-tracker").join("debug");
        let _ = std::fs::create_dir_all(&debug_dir);
        preprocess::save_debug_stages(&cropped, &debug_dir);
        for (idx, row_img) in &row_images {
            let _ = row_img.save(debug_dir.join(format!("row_{idx:02}.png")));
        }
    }
```

`save_debug_stages` re-runs preprocess and writes multiple full-scoreboard PNGs. Same pattern in `recognize()` → `save_debug_images`. **No env flag / feature gate.** Easiest large win: gate behind `STAT_TRACKER_DEBUG_OCR=1` or config.

#### H3. Dual OCR pipelines on every Tab

```703:704:src/main.rs
        let rows = ocr::recognize_scoreboard_cells_with_team_size(&img, Some(team_size));
        let ocr = ocr::recognize(&img);
```

Cell OCR already supplies positional stats; career hero + map are separate small crops. Full-image `recognize()` (adaptive Sauvola + optional threshold sweep + eng fallback) is largely redundant on the happy path and always pays large-crop preprocess + PNG + debug I/O.

#### H4. Poller starvation by Tab OCR

Heavy Tab work blocks the same `select!` loop. Comments document 45–70s gaps; victory banners last ~3s. Outcome confirmation window mitigates word OCR, but **missed banners remain a correctness-via-performance issue**.

### Medium — Capture / poll / preprocess

| ID | Issue | Where |
|----|--------|--------|
| M1 | Full-resolution capture every poll tick; detectors only need bands | `main.rs`, capture backends |
| M2 | Up to **3×** full-frame `to_rgb8()` in match-start cascade + 1× in banner | `match_start.rs`, `match_end.rs` |
| M3 | Dense pixel scans (banner mid-band, map-vote top ¼) with no stride | same |
| M4 | New `WayshotConnection` + output list **every** capture | `wayshot.rs` |
| M5 | Portal path: blocking decode on async task; ignores `capture_output` | `portal.rs` |
| M6 | `detect_team_size` twice per Tab; scoreboard cropped twice | `main.rs`, `hero_portrait.rs`, OCR |
| M7 | PNG encode every Tesseract call (`encode_png`) | `ocr/mod.rs` |
| M8 | Nested Rayon on 2–4 thread pool (rows × cells) | `recognize_row` + scoreboard |
| M9 | Preprocess: float HSV including unused H; `get_pixel`/`put_pixel` loops | `preprocess.rs` |
| M10 | Fixed 4s poll — no adaptive interval (idle vs post-match) | `config.rs`, `run_loop` |

### Low

- Portrait MAD @ 32×32 is cheap; Lanczos every crop is slightly overkill  
- Name OCR serial before cell parallel in `recognize_row`  
- `tessdata_lang()` filesystem probes not process-cached  
- Portrait collect path hard-codes 5v5 row spacing even in 6v6  

### What already looks good (performance)

1. Thread-local LepTess reuse + PSM/whitelist reset each call  
2. Dedicated OCR pool capped at half cores (2–4)  
3. Per-cell ROI + numeric whitelists (right accuracy/latency trade)  
4. Cells skip Sauvola (fixed threshold after HSV mask)  
5. `spawn_blocking` for heavy work; `MissedTickBehavior::Skip`  
6. Game process gate with 5s cache  
7. Banner before word OCR; small title crops + Otsu for outcomes  
8. Parallel calibration offsets (vs fully serial)  
9. 16:9 `game_rect_16_9` for ultrawide/16:10 geometry  
10. Bounded debug rings for poll dump / rejected frames  

### Ranked performance recommendations

| Rank | Change | Impact |
|------|--------|--------|
| 1 | Gate/remove production debug PNG pipeline | High wall-time + disk I/O |
| 2 | Cache column offset per resolution/session; OCR-score only when validity drops; or score crops without Tesseract | Cut ~300 OCR/Tab |
| 3 | Skip full `recognize()` when cells + panels suffice | Large redundant work |
| 4 | Run poll detection on a task that is not blocked by Tab OCR | Outcome reliability |
| 5 | Single RGB conversion + subsampled scans per poll frame | Poll CPU |
| 6 | Reuse Wayshot connection + resolved output | Capture overhead |
| 7 | Flat par over all cells; pass pre-cropped scoreboard + team_size | Less nesting/redundancy |
| 8 | Raw/grayscale feed to Tesseract if API allows (skip PNG) | × hundreds of cells |
| 9 | Slim HSV mask (S/V only, GrayImage out, bulk buffers) | Preprocess CPU |
| 10 | Adaptive poll interval; early-out phase when mid-game stable | Idle CPU |

**Rough Tab OCR budget (5v5, warm cache):** calibration ~300 + final ~70 + full board 1–4 + panels 2 ≈ **~380 calls**. At ~6 ms serial → ~2.3 s Tess alone; wall clock better with 4 threads but still multi-second once preprocess + debug I/O are included.

---

## Correctness & algorithm quality

### Strengths (do not “optimise away” carelessly)

1. **`parse_scoreboard_cells` refuses unidentified player rows** — no “first plausible row” teammate theft  
2. **`looks_like_scoreboard`** weak shape gate before record  
3. Kill-column plausibility (elims/assists ≤ 99, deaths ≤ 50)  
4. Outcome multi-signal stack with asymmetric trust + TTLs  
5. Portrait path **refuses** “best teammate portrait” fallback  
6. Hero priority: career panel > portrait > OCR text; **majority hero** across multi-capture sessions  
7. Map priority: vote → panel → fuzzy; King’s Row / Wrecking Ball trap fixed with tests  
8. Rejected-frame archive for “it didn’t record” diagnosis  

### Residual false positive / negative risks

| Risk | Type | Residual |
|------|------|----------|
| Tab outside scoreboard | FP | Weak shape gate (3 rows); process gate helps |
| Wrong teammate stats | FP | Largely eliminated by positive ID only |
| Weak highlight + no `player_name` | FN | Drop capture (good) but silent for users |
| Wrong team size (5↔6) | both | Near pitch threshold; no hysteresis |
| Double word-OCR hallucination | FP | Two agreeing wrong words still confirm |
| Missed short banner | FN | Poll interval + Tab starvation |
| Session split on daemon restart | FN/split | `ActiveGame` is memory-only |
| Post-match Tab after 75s grace | duplicate | Edge AFK case |

### Algorithm notes

- **Portrait MAD** is lighting/theme sensitive; career panel mitigates confusable heroes  
- **Team size** via saturation dips is clever and empty-row resilient  
- **Match start** colour + OCR confirms; 120s debounce limits FP sessions  
- **Hand-rolled CV** on `image` (no OpenCV) — fine; cost is loops and full-frame conversions  

---

## Storage, sync & data layer

### Local design (pragmatic and well documented)

- SurrealKV single-writer → **live_snapshot.json** + **commands/*.json** for GUI  
- Indexes on `synced`, `played_at`, `session_id`, `last_capture_at`  
- Atomic snapshot via tmp+rename  
- Command queue for SetOutcome / DeleteSession  
- Dual write: Surreal + append-only `matches.jsonl` (jsonl not updated on backfill — fallback can be stale)

### Critical / high sync issues

| ID | Severity | Finding |
|----|----------|---------|
| C1 | **Critical** | Every Tab snapshot is uploaded; server treats each row as a match. Local GUI uses `latest_per_game`. Multi-Tab games inflate cloud W/L and aggregates. |
| C2 | **Critical** | `reqwest::Client` has **no timeouts**; `try_sync` is awaited on the capture `select!` loop — hung network freezes Tab/poll/commands. |
| H1 | High | Outcome re-queue sets `synced = false`, but server dedup is `(member_id, hero, map_name, played_at)` **insert-or-skip** — corrections never update the server. |
| H2 | High | Map correction changes unique key → **duplicate** server rows; old map remains. |
| H3 | High | Local `outcome = "unknown"` can fail server ASSERT and **block the whole unsynced queue** (ORDER BY played_at ASC keeps failing head). |
| H4 | High | `mark_synced(count)` is count-based, not identity-based — fragile if concurrency or partial success appears. |
| H5 | High | Delete session is local-only; junk remains in cloud stats. |

### Medium storage issues

- Unbounded snapshot growth (all Tab rows forever; no retention of “final only”)  
- Full-table `export_snapshot` after many mutations  
- Command queue: delete-then-apply (crash can lose GUI commands)  
- Dead APIs: `find_active_session`, `session_window_secs`, `update_session`, `get_multi_capture_sessions`  
- Daemon restart loses in-memory `ActiveGame`  

### Recommendations (data)

**P0:** Sync one final row per session (or send `session_id` + server upsert); filter/hold `unknown`; HTTP timeouts + off-loop sync task.  
**P1:** Mark synced by record IDs; chunk uploads; retries/backoff; two-phase command apply; persist open session.  
**P2:** Retention / drop intermediate snapshots after finalize; incremental snapshot; chmod 600 on `config.toml`.

---

## GUI, lifecycle & robustness

### Overall

Capable companion UI (dashboard / matches / stats / settings / tray), brand-consistent dark theme, thoughtful systemd-aware start path. Gaps are lifecycle and footguns more than polish.

### Critical / high

1. **“Clear All Match Data” while daemon is running**  
   On store lock failure, `force_clear_data_dir` unlinks `stats.surrealkv` while the daemon still holds it open — unsafe. No confirm step. Disable clear when daemon is up, or stop → clear → restart.

2. **PID liveness ≠ daemon identity**  
   Status/stop only check `/proc/{pid}` exists; risk of PID reuse killing wrong process; pid file removed before exit confirmed.

3. **Settings do not hot-reload**  
   Daemon reads config once at start. Changing player name, token, output, auto-detect is a silent no-op until restart. Need toast + optional restart / SIGHUP.

4. **Dashboard hardcodes capture backend** as `"libwayshot (Wayland)"` regardless of Portal/None.

### Medium / low

- Tray quit uses `process::exit(0)` (skips cleanup)  
- Tray icon `expect` can panic the GUI  
- Portal “available” = `XDG_CURRENT_DESKTOP` set (weak)  
- Keyboard total loss exits daemon (systemd restarts if unit used)  
- Accessibility: match rows are mouse-centric click divs  
- No GUI tests; config load uses `unwrap_or_default` silently  

### Platform

Linux-only by design: Wayland/wayshot, evdev + `input` group, `/proc` game gate, systemd user units. Fine product scope; missing crate-level README stating it.

---

## Testing & tooling assessment

| Area | Coverage |
|------|----------|
| Session pure helpers | Good (`main.rs` tests) |
| Parse trust gates, maps, heroes | Good |
| Banner colour floods | Synthetic unit tests |
| Game process gate | Good (`/proc/self`) |
| Storage helpers (queue, majority, latest_per_game) | Good |
| OCR replay / outcome fixtures | `#[ignore]`, local fixtures |
| GUI / capture / sync | **None** |
| Poller FSM with fake frames | **None** beyond pure helpers |

**Operator tooling is excellent:** `examples/{extract,accolade,polltick,profile,probe_outcome,dumpdb}`, `--dump-poll-frames`, rejected rings. That partially compensates for thin CI E2E.

**Gap:** no committed CI wall-clock microbench of Tab path; no un-ignored smoke with synthetic (non-copyright) scoreboard-like images if possible.

---

## Code quality & maintainability

### Strengths

- Narrative comments explaining *why* (especially `main.rs`, `match_end`, storage IPC)  
- Clear module split; optional `gui` feature  
- Sparse `unsafe` (**none** found in this crate)  
- Production panics mostly limited to OCR pool / tray edge cases  
- Correct SurrealDatetime usage for Surreal v3  

### Friction

- God-loop in `run_loop` / large `handle_capture`  
- GUI thrice reimplements “open store or snapshot”  
- `style.rs` large CSS string with layered redefinitions  
- Dead config/API surface invites misuse (especially legacy `parse_scoreboard` with unsafe row fallback — unused by production path)  
- `Box<dyn Error>` everywhere; `thiserror` unused in storage/sync  
- `PersonalMatch` means “capture” locally and “game” on the server — overloaded naming  
- `tokio` with `features = ["full"]`; `tempfile` in normal deps  

---

## Consolidated priority backlog

### P0 — Correctness / safety / performance critical path

1. Gate debug OCR image dumps in production  
2. Never force-delete SurrealKV while daemon is running (+ confirm dialog)  
3. Sync one match per session (or stable id + upsert); don’t upload every snapshot as a game  
4. HTTP client timeouts; run sync off the capture event loop  
5. Apply or clear `pending_outcome` when poller opens a new game  
6. Isolate poll detection from multi-second Tab OCR  

### P1 — Large latency / robustness wins

7. Cache / shrink column calibration  
8. Skip full-image `recognize()` on happy path  
9. `mark_synced` by IDs; hold/filter `unknown` so it cannot block the queue  
10. Settings “restart required” (or reload); verify PID is this binary before kill  
11. Shared RGB + subsampled banner/phase scans; reuse Wayshot connection  
12. Persist or recover active session across daemon restart  

### P2 — Quality of life / hygiene

13. Real capture-backend status on dashboard  
14. Remove or gate dead APIs (`find_active_session`, `session_window_secs`, unused `parse_scoreboard`)  
15. Chunked uploads + retries; command queue two-phase apply  
16. Crate README (Linux/Wayland, input group, tessdata, clear-data caveats)  
17. Team-size hysteresis; portrait collect uses real team_size geometry  
18. Shrink tokio features; typed errors; shared GUI data load helper  

### P3 — Longer term

19. Incremental OCR (player row only after first full board in a session)  
20. Stronger portrait metric if career panel missing  
21. CI smoke + documented latency bands from `polltick` / timed `extract`  
22. A11y (keyboard match rows, real confirm modals)  

---

## Bottom line

| Dimension | Grade | Comment |
|-----------|-------|---------|
| Domain / session correctness | **A−** | Grace windows, trust gates, majority repair — battle-tested design |
| OCR accuracy architecture | **B+** | Cell OCR + calibration + multi-source hero/map is solid |
| OCR / Tab performance | **C** | Work volume (calibration, dual OCR, debug I/O) dominates |
| Poll path efficiency | **B−** | Acceptable defaults; repeated full-frame RGB and fixed interval |
| Local storage / IPC | **B+** | Snapshot + commands is the right single-writer design |
| Cloud sync semantics | **D+** | Snapshot≠game, weak identity, corrections/deletes don’t round-trip |
| GUI quality | **B** | Usable and coherent; lifecycle footguns |
| Tests | **B−** | Core pure logic good; E2E/GUI thin |
| Maintainability | **B** | Great comments; large loops and dead surface area |

This is a **product-shaped crate with a strong vision kernel**, not a prototype. Invest next in **cutting Tab OCR work volume**, **decoupling the poller**, and **fixing the upload/session contract with the server** — those three areas buy both speed and correctness. Do not relax “refuse bad player rows” or outcome confirmation without a strong replacement; that philosophy is why local aggregates stay trustworthy.

---

## Evidence map (primary files)

| Concern | Path |
|---------|------|
| Daemon loop / session FSM | `src/main.rs` |
| OCR + calibration + debug dumps | `src/ocr/mod.rs` |
| Preprocess / geometry | `src/ocr/preprocess.rs` |
| Parse / trust | `src/parse.rs` |
| Outcome | `src/detect/match_end.rs` |
| Match start | `src/detect/match_start.rs` |
| Portraits / team size | `src/detect/hero_portrait.rs` |
| Process gate | `src/detect/game_running.rs` |
| Capture | `src/capture/{mod,wayshot,portal}.rs` |
| Storage / IPC | `src/storage/mod.rs` |
| Sync client | `src/sync/mod.rs` |
| GUI lifecycle | `src/gui/{daemon,settings,status,main}.rs` |
| Poll latency tool | `examples/polltick.rs` |
| Full extract mirror | `examples/extract.rs` |

---

*End of review. Produced for comparison with a second independent review agent.*
