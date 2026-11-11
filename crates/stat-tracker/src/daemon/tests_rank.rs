//! The rank update wiring on injected reads: off by default, which frames
//! are read, how the reads of one rank screen settle, what lands on the
//! game record and in the upload, and that no failure costs the game.

use super::*;
use crate::daemon::queue_title::QueueTitleFeed;
use stat_tracker::shadow::queue_title::Field;
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

fn n(value: i64, confidence: f32) -> Option<RankNumber> {
    Some(RankNumber { value, confidence })
}

fn t(value: &str, confidence: f32) -> Option<RankText> {
    Some(RankText {
        value: value.to_string(),
        confidence,
    })
}

/// A full read of one rank screen.
fn full(pct: i64) -> RankUpdate {
    RankUpdate {
        role: t("Support", 0.9),
        role_rank: t("Diamond", 0.8),
        division: n(3, 0.9),
        progress_pct: n(pct, 0.9),
        progress_change: n(-15, 0.8),
        change_tags: Some(RankTags {
            value: vec!["Calibration Up".to_string()],
            confidence: 0.7,
        }),
        higher_than_pct: n(88, 0.9),
        hero_sr: n(3412, 0.85),
        hero_sr_change: n(21, 0.8),
        next_mode_hint: t("OPEN QUEUE", 0.75),
    }
}

/// A reader that counts its calls and gives `out`.
fn counting(out: Option<RankUpdate>, calls: &Arc<AtomicUsize>) -> RankReadFn {
    let calls = Arc::clone(calls);
    Arc::new(move |_| {
        calls.fetch_add(1, Ordering::SeqCst);
        out.clone()
    })
}

fn frame() -> RgbImage {
    RgbImage::new(2560, 1440)
}

/// The store, with its rank writes failing while `fail` is set.
struct Failing<'a> {
    store: &'a crate::storage::LocalStore,
    fail: Mutex<bool>,
}

impl RankStore for Failing<'_> {
    async fn attach_rank_update(&self, session_id: &str, rank: &RankUpdate) -> StoreResult<bool> {
        if *self.fail.lock().unwrap() {
            return Err("rank write refused (test)".into());
        }
        self.store.attach_rank_update(session_id, rank).await
    }
}

async fn store_with(sid: &str) -> (tempfile::TempDir, crate::storage::LocalStore) {
    let dir = tempfile::tempdir().expect("tempdir");
    let store = crate::storage::LocalStore::open(dir.path())
        .await
        .expect("store");
    store
        .insert_match(crate::main_tests::test_match(sid, "victory"))
        .await
        .unwrap();
    (dir, store)
}

/// The rows of `sid` and the upload body for them.
async fn rows_and_body(
    store: &crate::storage::LocalStore,
    sid: &str,
) -> (Vec<stat_tracker::model::PersonalMatch>, String) {
    let rows = store.get_session_snapshots(sid).await.unwrap();
    assert!(!rows.is_empty(), "the game is recorded");
    let body = serde_json::to_string(&stat_tracker::sync::upload_request(&rows, &[])).unwrap();
    (rows, body)
}

// ---- off by default ----

#[test]
fn the_feed_is_off_unless_switched_on_and_the_pack_loads() {
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(RankFeed::for_setting(RankSetting::default(), dir.path()).is_none());
    assert!(RankFeed::for_setting(RankSetting::Off, dir.path()).is_none());
    // on, but no pack: off as well
    assert!(RankFeed::for_setting(RankSetting::RankV1, dir.path()).is_none());
}

#[test]
fn with_the_reader_off_no_frame_is_read() {
    let img = frame();
    for source in [
        Some(OutcomeSource::RankScreen),
        Some(OutcomeSource::EndTitle),
        None,
    ] {
        assert_eq!(read_rank_screen(None, source, &img), None);
    }
}

#[test]
fn only_rank_screen_frames_are_read() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read = counting(Some(full(40)), &calls);
    let img = frame();
    for source in [
        None,
        Some(OutcomeSource::EndTitle),
        Some(OutcomeSource::ResultWord),
    ] {
        assert_eq!(read_rank_screen(Some(&read), source, &img), None);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let seen = read_rank_screen(Some(&read), Some(OutcomeSource::RankScreen), &img);
    assert_eq!(seen, Some(Some(full(40))));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_reader_that_panics_is_a_frame_with_no_read() {
    let read: RankReadFn = Arc::new(|_| panic!("reader failed (test)"));
    let seen = read_rank_screen(Some(&read), Some(OutcomeSource::RankScreen), &frame());
    assert_eq!(seen, Some(None));
}

// ---- settling ----

#[test]
fn a_field_is_kept_only_where_two_reads_in_a_row_agree() {
    // one read alone keeps nothing
    assert!(settle(&[Some(full(40))]).is_empty());
    assert!(settle(&[]).is_empty());
    // the progress counts up, the rest holds still
    let reads = [
        Some(full(10)),
        Some(full(25)),
        Some(full(40)),
        Some(full(40)),
    ];
    let r = settle(&reads);
    assert_eq!(r.progress_pct, n(40, 0.9));
    assert_eq!(r.hero_sr, n(3412, 0.85));
    // still counting on the last two reads: no progress, the rest kept
    let counting_up = [Some(full(10)), Some(full(25)), Some(full(40))];
    let r = settle(&counting_up);
    assert_eq!(r.progress_pct, None);
    assert_eq!(r.division, n(3, 0.9));
    // a frame with no read breaks the pair; a fading last frame does not
    // undo the agreed pair before it
    let r = settle(&[Some(full(40)), None, Some(full(40))]);
    assert!(r.is_empty());
    let mut fading = full(40);
    fading.hero_sr = n(341, 0.6);
    let r = settle(&[Some(full(40)), Some(full(40)), Some(fading)]);
    assert_eq!(r.hero_sr, n(3412, 0.85));
}

#[test]
fn an_agreed_field_has_the_lower_confidence() {
    let mut a = full(40);
    a.division = n(3, 0.6);
    let r = settle(&[Some(a), Some(full(40))]);
    assert_eq!(r.division, n(3, 0.6));
}

#[test]
fn reads_are_held_for_their_game_and_settle_when_the_screen_is_gone() {
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    let now = Utc::now();
    assert_eq!(feed.observe(Some(Some(full(40))), Some("g1"), now), None);
    assert_eq!(feed.observe(Some(Some(full(40))), Some("g1"), now), None);
    let s = feed.observe(None, Some("g1"), now).expect("settled");
    assert_eq!(s.session, "g1");
    assert_eq!(s.rank, full(40));
    // nothing held any more
    assert_eq!(feed.observe(None, Some("g1"), now), None);
    // another game's screen settles the first game's reads
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    let s = feed.observe(Some(Some(full(50))), Some("g2"), now);
    assert_eq!(s.map(|s| s.session).as_deref(), Some("g1"));
    // a rank screen with no game to put it on is not held
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    feed.observe(Some(Some(full(40))), None, now);
    feed.observe(Some(Some(full(40))), None, now);
    assert_eq!(feed.observe(None, None, now), None);
}

// ---- the game record ----

#[tokio::test]
async fn a_settled_read_is_stored_on_its_game_and_uploaded() {
    let (_dir, store) = store_with("g1").await;
    let (before_rows, before) = rows_and_body(&store, "g1").await;
    assert!(!before.contains("\"rank\""), "{before}");
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    for _ in 0..2 {
        tick(&mut feed, Some(Some(full(40))), Some("g1"), &store, None).await;
    }
    tick(&mut feed, None, Some("g1"), &store, None).await;
    let (rows, body) = rows_and_body(&store, "g1").await;
    assert!(rows.iter().all(|r| r.rank_update == Some(full(40))));
    assert!(!rows[0].synced, "queued for upload");
    let json: serde_json::Value = serde_json::from_str(&body).unwrap();
    let rank = &json["matches"][0]["rank"];
    assert_eq!(rank["hero_sr"]["value"], 3412, "{body}");
    assert_eq!(rank["rank_recognizer"], "rank-v1");
    // every other field of the game is as it was
    let mut same = rows[0].clone();
    same.rank_update = None;
    same.sync_rev = before_rows[0].sync_rev;
    same.synced = before_rows[0].synced;
    same.upload_reject = before_rows[0].upload_reject.clone();
    assert_eq!(same, before_rows[0]);
}

#[tokio::test]
async fn an_unreadable_rank_screen_leaves_the_game_and_its_upload_as_they_were() {
    let (_dir, store) = store_with("g1").await;
    let before = rows_and_body(&store, "g1").await;
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    // frames read as the rank screen that the reader could not read (wrong
    // size, a reader error or panic), then a single read alone
    for seen in [Some(None), Some(None), Some(Some(full(40))), None] {
        tick(&mut feed, seen, Some("g1"), &store, None).await;
    }
    assert_eq!(rows_and_body(&store, "g1").await, before);
    assert!(feed.unsaved.is_none());
}

#[tokio::test]
async fn a_store_error_is_retried_then_dropped_and_never_costs_the_game() {
    let (_dir, store) = store_with("g1").await;
    let before = rows_and_body(&store, "g1").await;
    let failing = Failing {
        store: &store,
        fail: Mutex::new(true),
    };
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None] {
        tick(&mut feed, seen, Some("g1"), &failing, None).await;
    }
    assert!(feed.unsaved.is_some(), "kept for the next tick");
    // still failing: tried on the next ticks, then dropped
    for _ in 1..RANK_STORE_TRIES {
        tick(&mut feed, None, Some("g1"), &failing, None).await;
    }
    assert!(feed.unsaved.is_none(), "dropped after its tries");
    assert_eq!(rows_and_body(&store, "g1").await, before);
    // a write that works on a retry stores it
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None] {
        tick(&mut feed, seen, Some("g1"), &failing, None).await;
    }
    *failing.fail.lock().unwrap() = false;
    tick(&mut feed, None, Some("g1"), &failing, None).await;
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].rank_update, Some(full(40)));
}

#[tokio::test]
async fn an_unknown_game_gets_nothing_and_the_feed_goes_on() {
    let (_dir, store) = store_with("g1").await;
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None] {
        tick(&mut feed, seen, Some("nope"), &store, None).await;
    }
    assert!(feed.unsaved.is_none());
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].rank_update, None);
}

#[tokio::test]
async fn the_banner_word_becomes_the_next_games_pending_hint() {
    let (dir, store) = store_with("g1").await;
    let queue = QueueTitleFeed::with_read(Arc::new(|_, _| Field::none()));
    let mut dump = PregameDump::with_readers(
        false,
        None,
        Some(queue),
        dir.path(),
        std::time::Duration::from_millis(1_500),
    )
    .expect("a saver for the queue title reader");
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None] {
        tick(&mut feed, seen, Some("g1"), &store, Some(&mut dump)).await;
    }
    assert_eq!(
        dump.pending_hint(),
        Some(("OPEN QUEUE".to_string(), "g1".to_string()))
    );
    // a settled read with no banner word leaves no hint
    let (dir, store) = store_with("g2").await;
    let queue = QueueTitleFeed::with_read(Arc::new(|_, _| Field::none()));
    let mut dump = PregameDump::with_readers(
        false,
        None,
        Some(queue),
        dir.path(),
        std::time::Duration::from_millis(1_500),
    )
    .expect("a saver");
    let mut no_banner = full(40);
    no_banner.next_mode_hint = None;
    for seen in [Some(Some(no_banner.clone())), Some(Some(no_banner)), None] {
        tick(&mut feed, seen, Some("g2"), &store, Some(&mut dump)).await;
    }
    assert_eq!(dump.pending_hint(), None);
}
