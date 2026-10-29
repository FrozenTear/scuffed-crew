//! The live snapshot write must not hold the whole table.
//!
//! Its own test binary, so its counting allocator sees only this test.
//! It compares the peak live heap of one write against the old full export
//! (every row loaded, then serialised in one buffer) on the same store. The
//! old export peaked near 98 MB on a real 19,694-row store; the compact
//! write holds one page of rows plus the one row per game it keeps.
//! Measured with live-heap counters, not RSS: RSS hides retention behind
//! allocator settings and trims, so it cannot tell the two writes apart.

use std::alloc::{GlobalAlloc, Layout, System};
use std::sync::atomic::{AtomicUsize, Ordering};

use stat_tracker::storage::{HeroSegment, LocalStore, MatchSession, PersonalMatch, Snapshot};

struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);
static PEAK: AtomicUsize = AtomicUsize::new(0);
static ALLOCATED: AtomicUsize = AtomicUsize::new(0);

fn grew(by: usize) {
    let now = LIVE.fetch_add(by, Ordering::Relaxed) + by;
    PEAK.fetch_max(now, Ordering::Relaxed);
    ALLOCATED.fetch_add(by, Ordering::Relaxed);
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
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
    }
    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        let p = unsafe { System.realloc(ptr, layout, new_size) };
        if !p.is_null() {
            LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
            grew(new_size);
        }
        p
    }
}

#[global_allocator]
static GLOBAL: Counting = Counting;

/// Peak live bytes above the starting level, and bytes allocated, for `f`.
async fn measure<F: std::future::Future<Output = ()>>(f: F) -> (usize, usize) {
    let base = LIVE.load(Ordering::Relaxed);
    PEAK.store(base, Ordering::Relaxed);
    let allocated = ALLOCATED.load(Ordering::Relaxed);
    f.await;
    (
        PEAK.load(Ordering::Relaxed).saturating_sub(base),
        ALLOCATED.load(Ordering::Relaxed) - allocated,
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

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
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

    let (old_peak, old_bytes) = measure(old_export()).await;
    let (new_peak, new_bytes) = measure(async {
        store.export_snapshot(dir.path()).await.unwrap();
    })
    .await;
    let rows = GAMES * ROWS_PER_GAME;
    println!(
        "{rows} rows: old full export peak {old_peak} B, allocated {old_bytes} B; \
         compact write peak {new_peak} B, allocated {new_bytes} B"
    );
    let written = stat_tracker::storage::read_snapshot(dir.path()).unwrap();
    assert_eq!(written.matches.len(), GAMES, "one row per game");

    // The old export holds every row: its peak grows with the table. The
    // compact write holds a page (500 rows here, the whole 3,000-row table
    // is 6 pages) plus 150 kept rows. Expect well under half; a write that
    // loads the table again lands near 1.0 and fails.
    assert!(
        new_peak * 3 < old_peak,
        "compact write peak {new_peak} B is not under a third of the full export's {old_peak} B"
    );
}
