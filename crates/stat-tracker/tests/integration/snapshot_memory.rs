//! The live snapshot write must not hold the whole table.
//!
//! It compares the peak live heap of one write against the old full export
//! (every row loaded, then serialised in one buffer) on the same store. The
//! old export peaked near 98 MB on a real 19,694-row store; the compact
//! write holds one page of rows plus the one row per game it keeps.
//! Measured with live-heap counters, not RSS: RSS hides retention behind
//! allocator settings and trims, so it cannot tell the two writes apart.
//!
//! The counting allocator counts only on a thread that turned it on, so the
//! other tests in this binary do not move the numbers. The test runs on a
//! current-thread runtime: the embedded store's tasks run on that thread
//! too (blocking-pool work, such as a sort, is not counted).

use std::alloc::{GlobalAlloc, Layout, System};
use std::cell::Cell;
use std::sync::atomic::{AtomicI64, Ordering};

use stat_tracker::storage::{HeroSegment, LocalStore, MatchSession, PersonalMatch, Snapshot};

struct Counting;

thread_local! {
    static COUNTING: Cell<bool> = const { Cell::new(false) };
}

static LIVE: AtomicI64 = AtomicI64::new(0);
static PEAK: AtomicI64 = AtomicI64::new(0);
static ALLOCATED: AtomicI64 = AtomicI64::new(0);
static CALLS: AtomicI64 = AtomicI64::new(0);

fn on() -> bool {
    COUNTING.try_with(Cell::get).unwrap_or(false)
}

fn grew(by: usize) {
    if !on() {
        return;
    }
    let by = by as i64;
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
    ALLOCATED.fetch_add(by, Ordering::Relaxed);
    CALLS.fetch_add(1, Ordering::Relaxed);
}

fn shrank(by: usize) {
    if on() {
        LIVE.fetch_sub(by as i64, Ordering::Relaxed);
    }
}

unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
        let p = unsafe { System.alloc_zeroed(layout) };
        if !p.is_null() {
            grew(layout.size());
        }
        p
    }
    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        unsafe { System.dealloc(ptr, layout) };
        shrank(layout.size());
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            shrank(layout.size());
            grew(new_size);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Peak live bytes above the starting level, bytes allocated and
/// allocation calls, for `f` on this thread.
async fn measure<F: std::future::Future<Output = ()>>(f: F) -> (i64, i64, i64) {
    let (base, bytes, calls) = (
        LIVE.load(Ordering::Relaxed),
        ALLOCATED.load(Ordering::Relaxed),
        CALLS.load(Ordering::Relaxed),
    );
    PEAK.store(base, Ordering::Relaxed);
    COUNTING.with(|c| c.set(true));
    f.await;
    COUNTING.with(|c| c.set(false));
    (
        PEAK.load(Ordering::Relaxed) - base,
        ALLOCATED.load(Ordering::Relaxed) - bytes,
        CALLS.load(Ordering::Relaxed) - calls,
    )
}

const GAMES: usize = 150;
const ROWS_PER_GAME: usize = 20;

fn row(game: usize, capture: usize) -> PersonalMatch {
    let at = |s: usize| {
        let t =
            chrono::DateTime::from_timestamp(1_800_000_000 + (game * 3_600 + s) as i64, 0).unwrap();
        surrealdb_types::Datetime::from(t)
    };
    let segment = |hero: &str, from: usize, to: usize| HeroSegment {
        hero: hero.into(),
        role: "Support".into(),
        first_seen: at(from * 30),
        last_seen: at(to * 30),
        snapshots: (to - from + 1) as u32,
        confirmed: true,
        resolution: None,
    };
    PersonalMatch {
        id: None,
        hero: if capture % 7 == 6 { "Kiriko" } else { "Ana" }.into(),
        map_name: "Busan".into(),
        game_mode: "Control".into(),
        role: "Support".into(),
        outcome: "victory".into(),
        elims: capture as u32,
        deaths: 3,
        assists: 9,
        damage: 4_000,
        healing: 9_000,
        mitigation: 0,
        played_at: at(capture * 30),
        synced: true,
        sync_rev: 1,
        upload_reject: None,
        session_id: format!("game-{game:04}"),
        corrected_hero: None,
        corrected_role: None,
        corrected_map_name: None,
        corrected_outcome: None,
        corrected_elims: None,
        corrected_deaths: None,
        corrected_assists: None,
        corrected_damage: None,
        corrected_healing: None,
        corrected_mitigation: None,
        edited_fields: Vec::new(),
        edited_at: None,
        heroes_played: vec![
            segment("Ana", 0, 5),
            segment("Kiriko", 6, 6),
            segment("Ana", 7, 19),
        ],
        segment_resolutions: Vec::new(),
        recognizer: "cv-v8".into(),
        suspect_fields: Vec::new(),
        name_recognizer: Some("names-v1".into()),
    }
}

#[tokio::test(flavor = "current_thread")]
async fn the_snapshot_write_does_not_hold_the_whole_table() {
    let dir = tempfile::tempdir().unwrap();
    let store = LocalStore::open(dir.path()).await.unwrap();
    for game in 0..GAMES {
        for capture in 0..ROWS_PER_GAME {
            store.insert_match(row(game, capture)).await.unwrap();
        }
        let first = row(game, 0);
        store
            .create_session(&MatchSession {
                session_id: first.session_id.clone(),
                hero: "Ana".into(),
                map_name: "Busan".into(),
                role: "Support".into(),
                started_at: first.played_at,
                last_capture_at: row(game, ROWS_PER_GAME - 1).played_at,
                capture_count: ROWS_PER_GAME as u32,
                final_outcome: "victory".into(),
            })
            .await
            .unwrap();
    }

    // Warm both paths once (engine caches, first-use allocations).
    let old_export = || async {
        let snapshot = Snapshot {
            matches: store.get_all_matches().await.unwrap(),
            sessions: store.get_all_sessions().await.unwrap(),
            pending_deletes: store.get_pending_tombstones().await.unwrap(),
        };
        let json = serde_json::to_vec(&snapshot).unwrap();
        std::hint::black_box(json);
    };
    old_export().await;
    store.export_snapshot(dir.path()).await.unwrap();

    let (old_peak, old_bytes, old_calls) = measure(old_export()).await;
    let (new_peak, new_bytes, new_calls) = measure(async {
        store.export_snapshot(dir.path()).await.unwrap();
    })
    .await;
    let rows = GAMES * ROWS_PER_GAME;
    println!(
        "{rows} rows: old full export peak {old_peak} B, allocated {old_bytes} B in \
         {old_calls} calls; compact write peak {new_peak} B, allocated {new_bytes} B in \
         {new_calls} calls"
    );
    let written = stat_tracker::storage::read_snapshot(dir.path()).unwrap();
    assert_eq!(written.matches.len(), GAMES, "one row per game");

    // Both still read every row once (the newest row per game needs them),
    // so bytes allocated stay about the same; that churn is why memtune
    // caps the arenas. What the compact write removes is holding them all.
    assert!(
        old_calls > 0 && new_calls > 0,
        "the counter sees the store's work"
    );
    // The old export holds every row: its peak grows with the table. The
    // compact write holds a page (500 rows here, the whole 3,000-row table
    // is 6 pages) plus 150 kept rows. Expect well under half; a write that
    // loads the table again lands near 1.0 and fails.
    assert!(
        new_peak * 3 < old_peak,
        "compact write peak {new_peak} B is not under a third of the full export's {old_peak} B"
    );
}

/// The compact write's time on a copy of a real store (memfix M1). Ignored:
/// needs `MEMFIX_PARITY_STORE` = a data dir holding `stats.surrealkv`,
/// copied to a temp dir first (the given dir is never opened). Run it in a
/// release-like profile; prints the first write and the warm ones.
#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore]
async fn the_compact_write_time_on_a_real_store_copy() {
    let Some(src) = std::env::var_os("MEMFIX_PARITY_STORE") else {
        panic!("set MEMFIX_PARITY_STORE");
    };
    let dir = tempfile::tempdir().unwrap();
    let status = std::process::Command::new("cp")
        .arg("-a")
        .arg(std::path::Path::new(&src).join("stats.surrealkv"))
        .arg(dir.path())
        .status()
        .unwrap();
    assert!(status.success());
    let store = LocalStore::open(dir.path()).await.unwrap();
    let rows = store.match_count().await.unwrap();
    let mut times = Vec::new();
    for _ in 0..5 {
        let t = std::time::Instant::now();
        store.export_snapshot(dir.path()).await.unwrap();
        times.push(t.elapsed().as_millis());
    }
    let bytes = std::fs::metadata(dir.path().join("live_snapshot.json"))
        .unwrap()
        .len();
    println!("{rows} rows: compact write ms {times:?} (first, then warm); file {bytes} B");
}

/// [`measure`] for plain code on this thread.
fn measure_sync(f: impl FnOnce()) -> (i64, i64, i64) {
    let (base, bytes, calls) = (
        LIVE.load(Ordering::Relaxed),
        ALLOCATED.load(Ordering::Relaxed),
        CALLS.load(Ordering::Relaxed),
    );
    PEAK.store(base, Ordering::Relaxed);
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    (
        PEAK.load(Ordering::Relaxed) - base,
        ALLOCATED.load(Ordering::Relaxed) - bytes,
        CALLS.load(Ordering::Relaxed) - calls,
    )
}

/// A stream frame as the daemon gets it: RGB (`framebuf::to_dynamic`),
/// 2560x1440, a dark game scene with a lighter band (no scoreboard, no
/// title), the frame a poll tick sees most of the time.
fn stream_frame() -> image::DynamicImage {
    let (w, h) = (2560u32, 1440u32);
    image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(w, h, |x, y| {
        let v = ((x / 40 + y / 40) % 7) as u8 * 6 + if (500..700).contains(&y) { 60 } else { 10 };
        image::Rgb([v, v / 2 + 5, v / 3 + 8])
    }))
}

const FRAME_BYTES: i64 = 2560 * 1440 * 3;

/// One poll-tick outcome check and one report-kit blanking on a stream
/// frame: what memfix PR 3 cut. Before it, each made a full RGB copy of
/// the frame (11 MB) through `DynamicImage::to_rgb8`; now they read the
/// frame in place. Fails if a full-frame copy comes back on either path.
#[test]
fn the_poll_check_and_the_report_blanking_do_not_copy_the_frame() {
    let frame = stream_frame();
    let _ = stat_tracker::detect::match_end::detect_outcome_signal(&frame); // warm
    let (_, poll_bytes, poll_calls) = measure_sync(|| {
        std::hint::black_box(stat_tracker::detect::match_end::detect_outcome_signal(
            &frame,
        ));
    });
    let (_, blank_bytes, blank_calls) = measure_sync(|| {
        std::hint::black_box(stat_tracker::report::blank::blanked_scoreboard(
            &frame, None,
        ));
    });
    println!(
        "frame {FRAME_BYTES} B; outcome check allocated {poll_bytes} B in {poll_calls} calls; \
         report blanking allocated {blank_bytes} B in {blank_calls} calls"
    );
    assert!(
        poll_bytes < FRAME_BYTES,
        "the outcome check copied the frame ({poll_bytes} B)"
    );
    // The blanking keeps its scoreboard crop (smaller than the frame).
    assert!(
        blank_bytes < FRAME_BYTES,
        "the blanking copied the frame ({blank_bytes} B)"
    );
}

/// Step 0 of memfix PR 3, as far as it runs without game frames: the poll
/// check and the report blanking on stream-size frames at 4 per second for
/// `SOAK_SECS` (default 1800), printing RSS and live heap every 30 s and a
/// linear fit. Ignored: run by hand (`--ignored --nocapture`).
#[test]
#[ignore]
fn frame_path_soak() {
    let secs: u64 = std::env::var("SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800);
    let rss_mb = || {
        let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
        statm
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<f64>()
            .unwrap()
            * 4096.0
            / 1e6
    };
    let frame_seed = stream_frame();
    let start = std::time::Instant::now();
    let mut samples: Vec<(f64, f64)> = Vec::new();
    let mut ticks = 0u64;
    let mut next = std::time::Duration::ZERO;
    let mut total_bytes = 0i64;
    while start.elapsed().as_secs() < secs {
        // A fresh frame per tick, as each grab makes one.
        let frame = frame_seed.clone();
        let (_, bytes, _) = measure_sync(|| {
            std::hint::black_box(stat_tracker::detect::match_end::detect_outcome_signal(
                &frame,
            ));
            if ticks % 20 == 0 {
                std::hint::black_box(stat_tracker::report::blank::blanked_scoreboard(
                    &frame, None,
                ));
            }
        });
        total_bytes += bytes;
        drop(frame);
        ticks += 1;
        if start.elapsed() >= next {
            let t = start.elapsed().as_secs_f64() / 3600.0;
            samples.push((t, rss_mb()));
            println!(
                "t={:>5.0}s rss={:.1} MB ticks={ticks} alloc_per_tick={:.2} MB",
                start.elapsed().as_secs_f64(),
                rss_mb(),
                total_bytes as f64 / ticks as f64 / 1e6
            );
            next += std::time::Duration::from_secs(30);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    let n = samples.len() as f64;
    let (sx, sy) = samples
        .iter()
        .fold((0.0, 0.0), |a, s| (a.0 + s.0, a.1 + s.1));
    let (mx, my) = (sx / n, sy / n);
    let slope = samples.iter().map(|s| (s.0 - mx) * (s.1 - my)).sum::<f64>()
        / samples
            .iter()
            .map(|s| (s.0 - mx).powi(2))
            .sum::<f64>()
            .max(1e-12);
    let max = samples.iter().map(|s| s.1).fold(0.0, f64::max);
    println!(
        "soak {secs}s: rss start {:.1} MB, max {max:.1} MB, end {:.1} MB, fit {slope:.2} MB/h, ticks {ticks}, alloc per tick {:.2} MB",
        samples.first().unwrap().1,
        samples.last().unwrap().1,
        total_bytes as f64 / ticks as f64 / 1e6
    );
}
