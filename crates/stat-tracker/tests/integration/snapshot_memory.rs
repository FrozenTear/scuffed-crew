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
/// The largest single allocation since [`measure_sync`] started: a frame
/// copy is one frame-sized buffer, while working buffers are smaller.
static LARGEST: AtomicI64 = AtomicI64::new(0);

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
    LARGEST.fetch_max(by, Ordering::Relaxed);
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

/// The counters are process-wide: one measurement at a time, across the
/// tests of this binary that measure.
static MEASURING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Peak live bytes above the starting level, bytes allocated and
/// allocation calls, for `f` on this thread.
// The guard is held across `f.await` on purpose: the one caller runs on a
// current-thread runtime, whose tasks never take MEASURING, and another
// test waiting for it blocks its own thread only.
#[allow(clippy::await_holding_lock)]
async fn measure<F: std::future::Future<Output = ()>>(f: F) -> (i64, i64, i64) {
    let _one = MEASURING.lock().unwrap_or_else(|e| e.into_inner());
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

/// [`measure`] for plain code on this thread, with the largest single
/// allocation as a fourth number.
fn measure_sync(f: impl FnOnce()) -> (i64, i64, i64, i64) {
    let _one = MEASURING.lock().unwrap_or_else(|e| e.into_inner());
    let (base, bytes, calls) = (
        LIVE.load(Ordering::Relaxed),
        ALLOCATED.load(Ordering::Relaxed),
        CALLS.load(Ordering::Relaxed),
    );
    PEAK.store(base, Ordering::Relaxed);
    LARGEST.store(0, Ordering::Relaxed);
    COUNTING.with(|c| c.set(true));
    f();
    COUNTING.with(|c| c.set(false));
    (
        PEAK.load(Ordering::Relaxed) - base,
        ALLOCATED.load(Ordering::Relaxed) - bytes,
        CALLS.load(Ordering::Relaxed) - calls,
        LARGEST.load(Ordering::Relaxed),
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

/// The RGB view the poll tick hands its detectors (daemon/event_loop.rs
/// run_loop: `stat_tracker::frame::rgb_view(&img)`).
fn tick_rgb(img: &image::DynamicImage) -> std::borrow::Cow<'_, image::RgbImage> {
    stat_tracker::frame::rgb_view(img)
}

/// The poll tick's frame work, as run_loop calls it: the RGB view, the
/// polled outcome check, the polled phase check and the end-reel check.
/// `stability` carries over between ticks as `st.ocr_stability` does, so a
/// still frame's second tick runs the reads gated on a repeat sighting.
fn poll_tick(
    img: &image::DynamicImage,
    new_reader: bool,
    stability: &mut stat_tracker::frame::stability::FrameStability,
) {
    use stat_tracker::detect::{match_end, match_start};
    let rgb = tick_rgb(img);
    let titles = match_end::Titles::for_rgb(new_reader, &rgb);
    let words = match_end::ResultWords::for_reader(new_reader);
    std::hint::black_box(match_end::detect_outcome_signal_polled(
        img,
        &rgb,
        stability,
        match_end::FinalScoreSource::for_reader(new_reader),
        &titles,
        &words,
    ));
    std::hint::black_box(match_start::detect_phase_polled(
        img,
        &rgb,
        stability,
        match_start::PhaseSource::for_reader(new_reader),
    ));
    std::hint::black_box(match_end::detect_end_reel_with(img, &rgb, &titles));
}

/// One poll tick and one report-kit blanking on a stream frame: what memfix
/// PR 3 cut. Before it, the tick made a full RGB copy of the frame (11 MB)
/// through `DynamicImage::to_rgb8`, and so did the blanking on top of its
/// scoreboard crop; now both read the frame in place. Fails if a full-frame
/// copy comes back on either path.
///
/// This process loads no words pack (no `SCUFFED_TEMPLATE_DIR`), so both
/// reader settings read through OCR here. The words-pack tick is measured
/// in a process of its own
/// ([`the_poll_tick_on_a_words_pack_does_not_copy_the_frame`]).
#[test]
fn the_poll_tick_and_the_report_blanking_do_not_copy_the_frame() {
    let frame = stream_frame();
    for new_reader in [true, false] {
        let mut stability = stat_tracker::frame::stability::FrameStability::default();
        // Warm: packs and tables load once, and the next tick is a repeat.
        poll_tick(&frame, new_reader, &mut stability);
        let (_, bytes, calls, largest) =
            measure_sync(|| poll_tick(&frame, new_reader, &mut stability));
        println!(
            "frame {FRAME_BYTES} B; poll tick (new reader {new_reader}) allocated {bytes} B in {calls} calls, largest {largest} B"
        );
        assert!(
            bytes < FRAME_BYTES / 2 && largest < FRAME_BYTES / 2,
            "the poll tick copied the frame ({bytes} B, largest {largest} B, new reader {new_reader})"
        );
    }
    let _ = stat_tracker::report::blank::blanked_scoreboard(&frame, None); // warm
    let (_, blank_bytes, blank_calls, _) = measure_sync(|| {
        std::hint::black_box(stat_tracker::report::blank::blanked_scoreboard(
            &frame, None,
        ));
    });
    println!("report blanking allocated {blank_bytes} B in {blank_calls} calls");
    // It keeps its scoreboard crop (about 0.9 of the frame); a full copy
    // on top of it would be about 1.9 frames.
    assert!(
        blank_bytes < FRAME_BYTES + FRAME_BYTES / 2,
        "the blanking copied the frame ({blank_bytes} B)"
    );
}

/// Step 0 of memfix PR 3, as far as it runs without game frames: the poll
/// tick (4 per second) and a report blanking every 20th tick on stream-size
/// frames, on a pool of worker threads like the daemon's blocking pool, for
/// `SOAK_SECS` (default 1800). Prints RSS every 30 s, a linear fit and the
/// allocation per tick. Run it once with the daemon's allocator limits in
/// `GLIBC_TUNABLES` and once without. Ignored: run by hand.
#[test]
#[ignore]
fn frame_path_soak() {
    const WORKERS: usize = 8;
    let secs: u64 = std::env::var("SOAK_SECS")
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(1800)
        .max(1);
    // SAFETY: sysconf has no preconditions.
    let page = unsafe { libc::sysconf(libc::_SC_PAGESIZE) }.max(1) as f64;
    let rss_mb = || {
        let statm = std::fs::read_to_string("/proc/self/statm").unwrap();
        statm
            .split_whitespace()
            .nth(1)
            .unwrap()
            .parse::<f64>()
            .unwrap()
            * page
            / 1e6
    };
    println!(
        "soak setup: {secs}s, {WORKERS} worker threads, GLIBC_TUNABLES={:?}",
        std::env::var("GLIBC_TUNABLES").ok()
    );
    let seed = std::sync::Arc::new(stream_frame());
    let (work_tx, work_rx) = std::sync::mpsc::channel::<(u64, std::sync::mpsc::Sender<i64>)>();
    let work_rx = std::sync::Arc::new(std::sync::Mutex::new(work_rx));
    let workers: Vec<_> = (0..WORKERS)
        .map(|_| {
            let (rx, seed) = (work_rx.clone(), seed.clone());
            std::thread::spawn(move || {
                let mut stability = stat_tracker::frame::stability::FrameStability::default();
                loop {
                    let job = rx.lock().unwrap().recv();
                    let Ok((tick, done)) = job else { return };
                    // A fresh frame per tick, as each grab makes one.
                    let frame = (*seed).clone();
                    let (_, bytes, _, _) = measure_sync(|| {
                        poll_tick(&frame, true, &mut stability);
                        if tick % 20 == 0 {
                            std::hint::black_box(stat_tracker::report::blank::blanked_scoreboard(
                                &frame, None,
                            ));
                        }
                    });
                    drop(frame);
                    let _ = done.send(bytes);
                }
            })
        })
        .collect();
    let start = std::time::Instant::now();
    let mut samples: Vec<(f64, f64)> = Vec::new();
    let (mut ticks, mut total) = (0u64, 0i64);
    let mut next = std::time::Duration::ZERO;
    while start.elapsed().as_secs() < secs {
        let (done_tx, done_rx) = std::sync::mpsc::channel();
        work_tx.send((ticks, done_tx)).unwrap();
        total += done_rx.recv().unwrap();
        ticks += 1;
        if start.elapsed() >= next {
            samples.push((start.elapsed().as_secs_f64() / 3600.0, rss_mb()));
            println!(
                "t={:>5.0}s rss={:.1} MB ticks={ticks} alloc_per_tick={:.2} MB",
                start.elapsed().as_secs_f64(),
                rss_mb(),
                total as f64 / ticks as f64 / 1e6
            );
            next += std::time::Duration::from_secs(30);
        }
        std::thread::sleep(std::time::Duration::from_millis(250));
    }
    drop(work_tx);
    for w in workers {
        let _ = w.join();
    }
    if samples.len() < 2 {
        println!("soak {secs}s: too short for a fit");
        return;
    }
    let n = samples.len() as f64;
    let (mx, my) = samples
        .iter()
        .fold((0.0, 0.0), |a, s| (a.0 + s.0 / n, a.1 + s.1 / n));
    let slope = samples.iter().map(|s| (s.0 - mx) * (s.1 - my)).sum::<f64>()
        / samples
            .iter()
            .map(|s| (s.0 - mx).powi(2))
            .sum::<f64>()
            .max(1e-12);
    let max = samples.iter().map(|s| s.1).fold(0.0, f64::max);
    println!(
        "soak {secs}s: rss start {:.1} MB, max {max:.1} MB, end {:.1} MB, fit {slope:.2} MB/h, ticks {ticks}, alloc per tick {:.2} MB",
        samples[0].1,
        samples[samples.len() - 1].1,
        total as f64 / ticks as f64 / 1e6
    );
}

/// The poll tick under `reader = "new"` with a words pack loaded (synthetic
/// end-title, accolade and FINAL SCORE templates, as `end_title_wiring` and
/// `final_score_wiring` write them): the words reads take the frame in
/// place too. They build working buffers per region every tick (several MB
/// in all), so the check is on the largest single allocation: a frame copy
/// is one frame-sized buffer. One test per process, so the reader is set
/// up here.
#[test]
fn the_poll_tick_on_a_words_pack_does_not_copy_the_frame() {
    use stat_tracker::detect::match_end::Titles;
    use stat_tracker::shadow::result::WORDS_SUBDIR;
    if !crate::common::own_process(
        module_path!(),
        "the_poll_tick_on_a_words_pack_does_not_copy_the_frame",
    ) {
        return;
    }
    let bars = |seed: u32, w: u32, h: u32| {
        image::GrayImage::from_fn(w, h, |x, y| {
            let on = (2..h - 2).contains(&y)
                && ((x / (2 + seed)).is_multiple_of(2) || (y + x * seed).is_multiple_of(11));
            image::Luma([if on { 229 } else { 0 }])
        })
    };
    let data = tempfile::tempdir().unwrap();
    let words = data.path().join(WORDS_SUBDIR);
    std::fs::create_dir_all(&words).unwrap();
    for (name, img) in [
        ("accolade_victory.png", bars(1, 70, 30)),
        ("accolade_defeat.png", bars(3, 60, 30)),
        ("end_title_victory.png", bars(2, 90, 34)),
        ("end_title_defeat.png", bars(4, 80, 34)),
        ("p4_final_score.png", bars(5, 120, 24)),
    ] {
        img.save(words.join(name)).unwrap();
    }
    // SAFETY: this test runs alone in a process of its own
    // (`common::own_process`); nothing else reads the environment
    // concurrently.
    unsafe { std::env::set_var(stat_tracker::shadow::reader::TEMPLATE_DIR_ENV, data.path()) };
    let frame = stream_frame();
    let titles = Titles::for_rgb(true, &tick_rgb(&frame));
    assert!(
        matches!(titles, Titles::Words(_)),
        "no words read: {titles:?}"
    );
    let mut stability = stat_tracker::frame::stability::FrameStability::default();
    poll_tick(&frame, true, &mut stability);
    let (_, bytes, calls, largest) = measure_sync(|| poll_tick(&frame, true, &mut stability));
    println!(
        "frame {FRAME_BYTES} B; poll tick (words pack) allocated {bytes} B in {calls} calls, largest {largest} B"
    );
    assert!(
        largest < FRAME_BYTES / 2,
        "the poll tick copied the frame (largest {largest} B, words pack)"
    );
}
