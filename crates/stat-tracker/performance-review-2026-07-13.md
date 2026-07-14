# Stat Tracker performance review

Date: 2026-07-13  
Scope: `crates/stat-tracker` daemon, OCR/capture pipeline, local storage/sync, and the optional desktop GUI  
Focus: CPU and I/O behavior that can interfere with Overwatch frame pacing

## Executive summary

The tracker does not appear to have an unbounded busy loop in its default configuration. The game-process gate pauses capture work when Overwatch is not running, the poll interval defaults to four seconds, debug image dumping is off by default, and only one Tab OCR capture can run at a time.

The remaining gaming risk is burst load:

1. A Tab capture can run hundreds of Tesseract calls across up to four normal-priority worker threads.
2. The four-second poller can capture and scan a full-resolution output while that Tab OCR is still running.
3. Cache misses, rejected frames, missing hero/map reads, 4K displays, the portal backend, and debug flags amplify the work substantially.
4. Snapshot/GUI work scales with the complete match history and can add periodic CPU and disk bursts over time.

On a 16-logical-CPU machine, the existing release-mode `polltick` harness measured detector work at **26.7 ms/tick for a synthetic 1920x1080 dark frame** and **106.1 ms/tick at 3840x2160**. This harness converts RGB twice whereas the daemon shares one conversion, so it is an upper bound for the detector portion; it also excludes compositor screenshot cost. At the default four-second cadence, the average CPU percentage is not alarming, but a roughly 100 ms 4K burst plus capture/compositor work can still disturb frame pacing.

A prebuilt release-mode `extract` run on a synthetic invalid 1080p frame took **0.635 seconds wall time** and emitted **3,805 lines / 62 KiB** of output, mostly Tesseract empty-image diagnostics. This is not a representative accuracy benchmark, but it confirms that rejected/blank frames can still exercise the expensive OCR path and can flood the systemd journal.

## High-impact findings

### H1. The expensive OCR pipeline runs before the scoreboard trust gate

`handle_capture` performs outcome reads, scoreboard crop, team-size detection, portrait matching, column calibration, all row/cell OCR, career/map OCR, and possibly full-board OCR before `looks_like_scoreboard` rejects the frame (`src/main.rs:1061-1145`, trust gate at `src/main.rs:1188-1201`).

This means a Tab press in a menu, transition, black frame, or partially rendered scoreboard can consume almost the same CPU as a useful capture. The game-process gate only proves Overwatch is running; it does not prove the scoreboard is visible.

Column calibration is the largest multiplier (`src/ocr/mod.rs:372-496`):

- A cache hit still OCRs 18 probe cells (three rows x six columns).
- A cache miss/degraded result evaluates roughly 16 candidate offsets x 18 cells, or about 288 probe OCR calls.
- Final 5v5 row extraction adds 70 calls (ten names plus sixty stat cells).
- Career, map, outcome, and full-board fallbacks add more calls.
- The cache is process-local and holds one geometry, so daemon restarts and resolution/team-size changes pay the cold path again.

Recommended changes, in order:

1. Add a cheap, non-OCR scoreboard preflight before calibration (expected panel/header/row geometry and ink density).
2. Skip Tesseract entirely for prepared cells with no meaningful dark-ink pixels.
3. Reduce calibration probes and candidates; trust the header offset first and recalibrate only after final-row quality fails.
4. Cache calibration by `(width, height, team_size)` instead of keeping one entry, and consider persisting it across daemon restarts.

### H2. OCR worker sizing can consume every CPU on small systems, with no gaming-friendly priority

The OCR pool uses `(logical_cpus / 2).clamp(2, 4)` (`src/ocr/mod.rs:69-85`). On one- and two-logical-CPU systems this creates two OCR workers; on a four-thread CPU it uses two. Those workers run at normal scheduling priority, so the game receives no explicit preference during a capture.

The systemd unit also has no `Nice`, `CPUWeight`, `IOSchedulingClass`, or similar protection (`assets/scuffed-stat-tracker.service:6-12`).

Recommended changes:

- Always leave at least one logical CPU unused and allow an `ocr_threads` override; one worker is safer on two-thread systems.
- Give the daemon a lower CPU and I/O scheduling priority in the user service.
- Consider a gaming mode that runs one or two OCR workers and defers nonessential storage/sync work until the scoreboard closes.

### H3. The poller continues while Tab OCR is in flight

Tab processing is correctly spawned and single-flight (`src/main.rs:647-705`), but the poll arm has no guard against an active `capture_task` (`src/main.rs:787-816`). A poll tick can therefore perform another full-output screenshot, RGB conversion, pixel scans, and title OCR while the Rayon OCR workers are already busy. Periodic sync can also overlap the capture (`src/main.rs:772-781`).

This keeps the event loop responsive, but increases contention at the exact moment the user has opened the scoreboard during a live game.

Recommended change: skip or reschedule one poll tick while a Tab capture is active, or reuse the Tab frame for outcome detection and restart the poll interval after capture completion. Keep sync single-flight, but preferably start it after the capture CPU phase has ended.

### H4. Full-resolution polling scales directly with display resolution

Every enabled tick captures the selected full output, converts the whole frame to RGB, then runs several large pixel scans (`src/main.rs:793-815`, `src/detect/match_end.rs:103-158`, `src/detect/match_start.rs:31-219`). The stride-two scans help, but 4K still processes about four times as many pixels as 1080p.

The result-word pre-gate only rejects near-black crops. Its own comments note that bright gameplay frames pass the gate (`src/detect/match_end.rs:258-287`), so normal gameplay can still invoke one small Tesseract title read every four seconds.

In addition to CPU, full-output Wayland screenshots may cause compositor/GPU readback and memory-bandwidth pressure that the detector-only benchmark does not measure.

Recommended changes:

- Build a downscaled detector frame once per tick and express pixel thresholds against it.
- Scan only the required bands/regions instead of converting the entire frame.
- Use an adaptive cadence: slower during an active mid-game session, faster only near start/end evidence.
- Expose a gaming-safe poll interval preset (for example 6-8 seconds), documenting the outcome-detection tradeoff.

### H5. Thread-local Tesseract caching is fragmented across arbitrary Tokio blocking workers

Tesseract engines are cached per OS thread and language (`src/ocr/mod.rs:88-127`). The comment documents an approximately 23 MiB model and about 77 ms construction cost. However, the poll detector and the outer Tab analysis run on `tokio::task::spawn_blocking` workers (`src/main.rs:796`, `src/main.rs:1061`). Tokio may schedule later calls on different blocking threads, causing repeated model loads and multiple resident engines before the worker set warms up.

Wayshot has the same affinity issue: its connection is thread-local, but every capture is dispatched to the general blocking pool (`src/capture/wayshot.rs:18-23`, `src/capture/wayshot.rs:61-98`). Reuse is therefore opportunistic rather than guaranteed.

Recommended change: route all OCR through the fixed OCR pool (including region/title/full-board OCR), and use a dedicated capture worker/actor that owns one Wayshot connection and cached output selection.

### H6. The lazy full-board fallback still runs unnecessarily after a portrait match

`need_full_ocr` becomes true whenever `career_hero` is absent (`src/main.rs:1113-1131`), even if `portrait_match` already identified the hero. This is common on replay/post-match layouts without the career panel. The fallback then processes a board covering about 65% x 70% of the 16:9 game area, runs adaptive contrast/Sauvola/morphology, PNG encoding, Tesseract, and up to three legacy threshold retries (`src/ocr/mod.rs:525-584`).

Recommended change: require full hero-text OCR only when both career and portrait sources failed. Split hero/map/raw-name fallbacks so one missing field does not force a full-board pass, and accept the already-cropped scoreboard to avoid cropping/copying it again.

## Medium-impact findings

### M1. Every OCR call allocates and PNG-compresses an image

Cell and region OCR preprocess into new buffers and then encode PNG before `LepTess::set_image_from_mem` (`src/ocr/mod.rs:130-140`, `src/ocr/mod.rs:198-248`). This repeats for every calibration probe and every final cell.

The cell mask also converts to RGB, calculates floating-point HSV even though hue is discarded, converts back to grayscale, thresholds into another image, then allocates a bordered image (`src/ocr/preprocess.rs:104-149`, `src/ocr/preprocess.rs:488-564`).

Recommended changes: use a raw grayscale/Pix input API if `leptess` permits it, reuse scratch buffers, calculate saturation/value with integer max/min directly, and combine mask/threshold/border steps where practical.

### M2. Blank cells can trigger heavy OCR and journal noise

There is no ink-density/empty-cell check before `recognize_cell` or `recognize_name`. Calibration always sends every probe crop to Tesseract (`src/ocr/mod.rs:499-511`). On the synthetic blank-frame run, Tesseract produced thousands of diagnostic lines. The service sends stderr to journald, turning a CPU problem into disk and logging work too.

Recommended change: reject near-empty prepared crops before Tesseract, and verify whether Tesseract debug parameters can suppress the empty-page diagnostics. Rate-limit or redirect unavoidable native-library noise.

### M3. Live snapshots rewrite the complete history after effectively every Tab capture

`export_snapshot` queries all matches and sessions, serializes the complete snapshot, then performs synchronous file writes (`src/storage/mod.rs:344-362`). The nominal snapshot debounce is two seconds (`src/main.rs:1409-1469`), but Tab presses are debounced for three seconds, so normal accepted captures occur outside the snapshot window and still trigger a full rewrite. A successful periodic sync forces another full export (`src/main.rs:1531-1542`).

As history grows, this produces increasing database, allocation, serialization, and disk-write bursts during gaming.

Recommended changes: export only one latest row per session plus recent detail rows, move serialization/file I/O to a blocking worker, coalesce capture and sync mutations, and consider flushing at session close or after a longer quiet period.

### M4. GUI refreshes still scale with full history and repeatedly probe the locked database

Each active GUI route refreshes every 10-15 seconds. `fetch_live_matches` first calls `LocalStore::open`, whose open path executes table/index DDL, even though the daemon normally owns the single-process SurrealKV lock (`src/gui/live_data.rs:59-80`, `src/storage/mod.rs:56-85`). The snapshot cache avoids reparsing an unchanged file but clones the entire match vector on every hit (`src/gui/live_data.rs:33-56`). Stats/history then collapse, clone, group, or recompute across that history (`src/gui/stats.rs:14-31`, `src/gui/stats.rs:45-53`, `src/gui/history.rs:63-99`).

Only one route is mounted at once, which limits the damage, but leaving the GUI open can still add periodic CPU spikes as data grows. The dashboard also spawns a `systemctl is-enabled` process every ten seconds (`src/gui/daemon.rs:226-242`).

Recommended changes: prefer the snapshot immediately when the daemon PID is valid, share immutable cached data without full-vector cloning, memoize computed stats, suspend refresh while hidden/minimized, and stop polling static systemd enablement state.

### M5. Portal capture is a costly conditional fallback

The portal backend requests a screenshot, waits for a file URI, decodes the image, and deletes the temporary file (`src/capture/portal.rs:7-34`). If this backend is used with the four-second poller, the desktop portal/compositor may PNG-encode and write a full-resolution screenshot every tick. That work happens partly outside the tracker process, so process-only CPU measurements can miss it.

Recommended change: warn that auto-detect polling is not gaming-safe on the portal backend, increase its minimum interval substantially, or disable continuous polling unless a persistent screencast/capture API is available.

## Low/conditional findings

- `--dump-poll-frames` PNG-encodes a full-resolution frame every four seconds and scans/sorts up to 150 directory entries each time (`src/main.rs:795-799`, `src/main.rs:1349-1377`). This option can absolutely affect gaming and should remain diagnostics-only.
- `debug_ocr` writes preprocessing stages and every row crop on each Tab capture (`src/ocr/mod.rs:357-364`, `src/ocr/preprocess.rs:763-787`). It is correctly off by default, but the settings/help text should explicitly warn about frame-time and disk impact.
- Wayshot enumerates all outputs on every screenshot even after the connection is cached (`src/capture/wayshot.rs:65-95`). Cache the selected output and refresh it only after failure/hotplug.
- Sync sends the full unsynced backlog every fifth accepted capture and forces a snapshot on success (`src/main.rs:1475-1542`). It is off the main loop and HTTP-bounded now, but a large offline backlog can still contend for CPU, database, network, and disk while gaming; batch it and schedule after the capture phase.
- Team-size detection converts and scans roughly half the scoreboard width for every row of pixels using floating-point saturation, then performs slice sums for smoothing (`src/detect/hero_portrait.rs:315-375`). It is smaller than Tesseract but is another resolution-scaled pass that could be downsampled.

## Existing protections worth keeping

- Default `game_process_names = ["Overwatch.exe"]` and a five-second `/proc` cache prevent desktop-wide polling when the game is closed (`src/config.rs:18-28`, `src/detect/game_running.rs:9-65`).
- Tab captures are single-flight and three-second debounced (`src/main.rs:641-653`).
- OCR is capped at four Rayon workers and Tesseract instances are reused per thread (`src/ocr/mod.rs:69-127`).
- Poll pixel scans use stride two and share a single RGB frame in the daemon (`src/detect/match_start.rs:13-29`, `src/main.rs:800-815`).
- OCR debug dumping is off by default and uses the configured data directory (`src/config.rs:24-28`, `src/ocr/mod.rs:25-67`).
- Sync is spawned, single-flight, and protected by a 30-second HTTP timeout (`src/main.rs:607-612`, `src/sync/mod.rs:14-23`).

## Recommended implementation order

1. Add the cheap pre-OCR scoreboard/empty-cell gates (H1, M2).
2. Skip poll work during Tab OCR and prevent sync from starting during the CPU phase (H3).
3. Fix full-board fallback gating to honor portrait success (H6).
4. Reserve CPU for the game and lower service scheduling priority (H2).
5. Put OCR and Wayshot on stable dedicated workers (H5).
6. Downscale/region-limit and cadence-limit polling, especially at 4K and on portal capture (H4, M5).
7. Remove PNG as the per-cell transport and simplify preprocessing allocations (M1).
8. Bound/defer full-history snapshots and GUI aggregation (M3, M4).

## Validation plan

Before and after changes, measure on real 1080p, 1440p, and 4K frames:

- Poll tick wall time and CPU time, including screenshot acquisition.
- Tab capture wall/CPU time for cold calibration, warm cache, cache degradation, valid scoreboard, and invalid frame.
- Per-thread CPU utilization and game 1%/0.1% low FPS while pressing Tab.
- Number of Tesseract invocations, model constructions, allocated bytes, and native stderr lines per capture.
- Snapshot/export time at 100, 1,000, and 10,000 stored snapshots.
- Wayshot versus portal behavior separately.

The current `examples/polltick.rs` benchmark should be updated to exercise the daemon's shared-RGB functions so it no longer double-counts conversion. A dedicated capture benchmark should also time screenshot acquisition independently from detection.
