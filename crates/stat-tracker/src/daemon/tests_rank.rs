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
    // a frame with no read breaks the pair
    let r = settle(&[Some(full(40)), None, Some(full(40))]);
    assert!(r.is_empty());
    // a last frame that reads a different value (a fade misread as another
    // number) is a newer read that disagrees: the field is dropped
    let mut fading = full(40);
    fading.hero_sr = n(341, 0.6);
    let r = settle(&[Some(full(40)), Some(full(40)), Some(fading)]);
    assert_eq!(r.hero_sr, None);
    assert_eq!(r.division, n(3, 0.9));
    // a fade to no read is no value: the newest pair stands
    let r = settle(&[Some(full(40)), Some(full(40)), None]);
    assert_eq!(r, full(40));
    let mut faded = full(40);
    faded.hero_sr = None;
    let r = settle(&[Some(full(40)), Some(full(40)), Some(faded)]);
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
    // one missed tick is not the end of the screen
    assert_eq!(feed.observe(None, Some("g1"), now), None);
    assert_eq!(RANK_SETTLE_MISSES, 2);
    let s = feed.observe(None, Some("g1"), now).expect("settled");
    assert_eq!(s.session, "g1");
    assert_eq!(s.rank, full(40));
    // nothing held any more
    assert_eq!(feed.observe(None, Some("g1"), now), None);
    assert_eq!(feed.observe(None, Some("g1"), now), None);
    // a missed tick between two reads breaks the pair
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    feed.observe(None, Some("g1"), now);
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    feed.observe(None, Some("g1"), now);
    assert_eq!(feed.observe(None, Some("g1"), now), None, "no pair");
    // the open game closing (no game any more) settles at once
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    let s = feed.observe(None, None, now).expect("settled on close");
    assert_eq!(s.session, "g1");
    // as does the game process stopping
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    feed.observe(Some(Some(full(40))), Some("g1"), now);
    assert_eq!(feed.close().map(|s| s.session).as_deref(), Some("g1"));
    assert_eq!(feed.close(), None);
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
        assert!(!tick(&mut feed, Some(Some(full(40))), Some("g1"), &store, None).await);
    }
    assert!(!tick(&mut feed, None, Some("g1"), &store, None).await);
    assert!(
        tick(&mut feed, None, Some("g1"), &store, None).await,
        "stored: the caller refreshes the live snapshot"
    );
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
    for seen in [Some(None), Some(None), Some(Some(full(40))), None, None] {
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
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None, None] {
        tick(&mut feed, seen, Some("g1"), &failing, None).await;
    }
    assert!(feed.unsaved.is_some(), "kept for the next tick");
    // still failing: tried on the next ticks, then dropped; tried on
    // RANK_STORE_TRIES ticks in all, the settle tick among them
    assert_eq!(RANK_STORE_TRIES, 3);
    tick(&mut feed, None, Some("g1"), &failing, None).await;
    assert!(feed.unsaved.is_some(), "still kept after one retry");
    tick(&mut feed, None, Some("g1"), &failing, None).await;
    assert!(feed.unsaved.is_none(), "dropped after its tries");
    assert_eq!(rows_and_body(&store, "g1").await, before);
    // a write that works on a retry stores it
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None, None] {
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
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None, None] {
        tick(&mut feed, seen, Some("nope"), &store, None).await;
    }
    assert!(feed.unsaved.is_none());
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].rank_update, None);
}

#[tokio::test]
async fn the_banner_word_becomes_the_next_games_pending_hint() {
    let (dir, store) = store_with("g1").await;
    store
        .insert_match(crate::main_tests::test_match("g2", "victory"))
        .await
        .unwrap();
    let queue = QueueTitleFeed::with_read(Arc::new(|_, _| Field::none()));
    let mut dump = PregameDump::with_readers(
        false,
        None,
        Some(queue),
        dir.path(),
        std::time::Duration::from_millis(1_500),
    )
    .expect("a saver for the queue title reader");
    assert!(dump.has_queue_reader());
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    // the hint's time is the newest rank read's, not the settle tick's
    let t0 = Utc::now() - chrono::Duration::seconds(10);
    let seq = [
        (Some(Some(full(40))), t0),
        (Some(Some(full(40))), t0 + chrono::Duration::seconds(2)),
        (None, t0 + chrono::Duration::seconds(4)),
        (None, t0 + chrono::Duration::seconds(6)),
    ];
    for (seen, at) in seq {
        tick_at(&mut feed, seen, Some("g1"), &store, Some(&mut dump), at).await;
    }
    assert_eq!(
        dump.pending_hint(),
        Some(("OPEN QUEUE".to_string(), "g1".to_string()))
    );
    assert_eq!(
        dump.pending_hint_at(),
        Some(t0 + chrono::Duration::seconds(2))
    );
    // a later rank screen with no banner word clears the older hint: a game
    // came after the hint's source, so it no longer names the next queue
    let mut no_banner = full(40);
    no_banner.next_mode_hint = None;
    for seen in [
        Some(Some(no_banner.clone())),
        Some(Some(no_banner)),
        None,
        None,
    ] {
        tick(&mut feed, seen, Some("g2"), &store, Some(&mut dump)).await;
    }
    assert_eq!(dump.pending_hint(), None);
}

#[tokio::test]
async fn an_update_waiting_for_the_store_is_tried_before_a_newer_one() {
    let (_dir, store) = store_with("g1").await;
    store
        .insert_match(crate::main_tests::test_match("g2", "victory"))
        .await
        .unwrap();
    let failing = Failing {
        store: &store,
        fail: Mutex::new(true),
    };
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    tick(&mut feed, Some(Some(full(40))), Some("g1"), &failing, None).await;
    tick(&mut feed, Some(Some(full(40))), Some("g1"), &failing, None).await;
    // g2's screen settles g1's at once; the store refuses it
    tick(&mut feed, Some(Some(full(50))), Some("g2"), &failing, None).await;
    assert_eq!(
        feed.unsaved.as_ref().map(|(s, _)| s.session.as_str()),
        Some("g1")
    );
    // keep the store failing through g2's second read so g1 still waits;
    // the two updates have not met on one tick yet
    tick(&mut feed, Some(Some(full(50))), Some("g2"), &failing, None).await;
    assert_eq!(
        feed.unsaved.as_ref().map(|(s, _)| s.session.as_str()),
        Some("g1")
    );
    *failing.fail.lock().unwrap() = false;
    // g1 is tried first, then g2 settles (the game closed) on the same
    // tick: both are stored and nothing waits (were g1 tried only on a tick
    // with nothing settling, it would still wait here)
    assert!(tick(&mut feed, None, None, &failing, None).await);
    assert!(feed.unsaved.is_none());
    let g1 = store.get_session_snapshots("g1").await.unwrap();
    let g2 = store.get_session_snapshots("g2").await.unwrap();
    assert_eq!(g1[0].rank_update, Some(full(40)));
    assert_eq!(g2[0].rank_update, Some(full(50)));
}

#[tokio::test]
async fn the_game_process_stopping_stores_the_held_screen() {
    let (_dir, store) = store_with("g1").await;
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    tick(&mut feed, Some(Some(full(40))), Some("g1"), &store, None).await;
    tick(&mut feed, Some(Some(full(40))), Some("g1"), &store, None).await;
    assert!(close(&mut feed, &store, None).await);
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].rank_update, Some(full(40)));
    assert!(!close(&mut feed, &store, None).await, "nothing held");
}

#[tokio::test]
async fn a_shutdown_on_the_rank_screen_stores_the_held_screen() {
    let (_dir, store) = store_with("g1").await;
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    // the tracker stops while the rank screen still shows: no miss, no
    // game close, nothing settled yet
    tick(&mut feed, Some(Some(full(40))), Some("g1"), &store, None).await;
    tick(&mut feed, Some(Some(full(40))), Some("g1"), &store, None).await;
    assert_eq!(
        store.get_session_snapshots("g1").await.unwrap()[0].rank_update,
        None
    );
    assert!(finish_on_shutdown(&mut feed, &store, None).await);
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].rank_update, Some(full(40)));
    assert!(!rows[0].synced, "queued for the next upload");
    assert!(
        feed.held.is_none(),
        "nothing left for the drop to throw away"
    );
    assert!(!finish_on_shutdown(&mut feed, &store, None).await);
}

#[tokio::test]
async fn a_shutdown_gives_an_update_the_store_refused_one_last_try() {
    let (_dir, store) = store_with("g1").await;
    let failing = Failing {
        store: &store,
        fail: Mutex::new(true),
    };
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None, None] {
        tick(&mut feed, seen, Some("g1"), &failing, None).await;
    }
    assert!(feed.unsaved.is_some());
    *failing.fail.lock().unwrap() = false;
    assert!(finish_on_shutdown(&mut feed, &failing, None).await);
    assert!(feed.unsaved.is_none());
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].rank_update, Some(full(40)));
}

#[test]
fn the_caller_builds_the_feed_and_settles_it_after_the_loop() {
    let lifecycle: String = include_str!("lifecycle.rs").split_whitespace().collect();
    let at = |needle: &str| {
        assert_eq!(lifecycle.matches(needle).count(), 1, "{needle} once");
        lifecycle.find(needle).expect("counted above")
    };
    let made = at("letmutrank_feed=daemon::rank::RankFeed::from_env(&ctx.data_dir);");
    let looped = at("letresult=run_loop(");
    let handed = at("&mutpregame_dump,&mutrank_feed,)");
    let settle = at("finish_or_abort_sync(");
    let flush = at("daemon::rank::finish_on_shutdown(feed,&ctx.store,pregame_dump.as_mut())");
    let pregame = at("dump.finish(within).await");
    let close = lifecycle
        .rfind("close_store_before_exit(ctx).await")
        .expect("store close");
    // built before the loop and handed to it; after the loop, settled and
    // stored before the pre-game flush takes the saver, and before the
    // store closes
    assert!(made < looped && looped < handed && handed < settle);
    assert!(settle < flush && flush < pregame && pregame < close);
    // bounded by the shutdown deadline
    assert!(lifecycle[flush..pregame].contains("tokio::time::timeout(within,flush)"));
}

// ---- the live feed and the event loop ----

#[test]
fn a_feed_with_a_pack_on_disk_is_on_and_reads_frames() {
    use stat_tracker::shadow::rank::templates::ClassTemplate;
    use stat_tracker::shadow::rank::{
        Family, GATE_CLASS, RECOGNIZER_ID, TEMPLATE_DIR, TemplateSet,
    };
    // the smallest pack that loads: the gate template alone
    let (w, h, _) = Family::Gate.shape();
    let mut set = TemplateSet {
        recognizer: RECOGNIZER_ID.to_string(),
        ..TemplateSet::default()
    };
    set.families.insert(
        Family::Gate,
        vec![ClassTemplate {
            class: GATE_CLASS.to_string(),
            aspect: 1.0,
            n: 1,
            vector: vec![0.5; w * h],
        }],
    );
    let dir = tempfile::tempdir().expect("tempdir");
    set.save_dir(&dir.path().join(TEMPLATE_DIR)).unwrap();
    let feed = RankFeed::for_setting(RankSetting::RankV1, dir.path()).expect("the feed is on");
    // a frame flagged as the rank screen goes through the pack's reader:
    // a blank frame is no rank screen, so no read, and no panic
    let read = feed.reader();
    let seen = read_rank_screen(Some(&read), Some(OutcomeSource::RankScreen), &frame());
    assert_eq!(seen, Some(None));
    // another size is no read either (logged once per run)
    let small = RgbImage::new(1920, 1080);
    assert_eq!(read(&small), None);
    assert_eq!(read(&small), None);
    // switched off with the same pack: no feed
    assert!(RankFeed::for_setting(RankSetting::Off, dir.path()).is_none());
}

#[test]
fn the_rank_screen_belongs_to_the_open_game_while_it_is_recent() {
    use crate::ActiveGame;
    use std::time::{Duration, Instant};
    let open = |idle: Duration| {
        ActiveGame::open_at(
            "g1".to_string(),
            stat_tracker::detect::MatchOutcome::Unknown,
            Vec::new(),
            Instant::now().checked_sub(idle).expect("a past instant"),
        )
    };
    assert_eq!(rank_game(None), None);
    let recent = open(Duration::from_secs(5));
    assert_eq!(rank_game(Some(&recent)), Some("g1"));
    let quiet = open(crate::daemon::RANK_OUTCOME_MAX_IDLE + Duration::from_secs(60));
    assert_eq!(rank_game(Some(&quiet)), None);
}

#[test]
fn the_poll_loop_wires_the_feed_in_order() {
    let run_loop = crate::main_tests::source_text::fn_source("run_loop");
    let at = |needle: &str| {
        run_loop
            .find(needle)
            .unwrap_or_else(|| panic!("run_loop must contain {needle:?}"))
    };
    // built once from the env switch, by the caller (off is `None`), and
    // handed in; the loop never builds its own
    assert!(!run_loop.contains("RankFeed::from_env("));
    let feed = at("if rank_feed.is_some()");
    // the game process stopping settles a held screen at once (the poll
    // arm, not the keyboard arm's earlier `if !running`)
    let poll = at("poll_timer.tick()");
    let stopped = poll
        + run_loop[poll..]
            .find("if !running {")
            .unwrap_or_else(|| panic!("the poll arm must contain if !running {{"));
    let close = at("daemon::rank::close(feed, store, pregame_dump.as_mut())");
    let pregame = at("daemon::focus::note_pregame(");
    assert!(feed < poll && poll < stopped && stopped < close && close < pregame);
    let branch = &run_loop[stopped..pregame];
    let close_at = branch
        .find("daemon::rank::close(feed, store, pregame_dump.as_mut())")
        .expect("close sits in the poll-arm process-stop branch");
    assert!(
        !branch[..close_at].contains("continue;"),
        "close sits before the process-stop branch's continue, not after the branch"
    );
    let refresh_at = branch
        .find("refresh_snapshot(store, data_dir)")
        .expect("the close's snapshot refresh sits in that branch");
    assert!(
        close_at < refresh_at,
        "refresh_snapshot follows close in the process-stop branch"
    );
    // and before the branch's own `continue;`: a refresh after it would sit
    // outside the branch and run on every poll tick, not when close stored
    // a rank
    let continue_at = close_at
        + branch[close_at..]
            .find("continue;")
            .expect("the process-stop branch ends in continue");
    assert!(
        refresh_at < continue_at,
        "refresh_snapshot sits before the process-stop branch's continue"
    );
    // read in the analysis thread, after the poll analysis perf sample
    let perf = at("stat_tracker::perf::Stage::PollAnalyze");
    let read = at("daemon::rank::read_rank_screen(");
    assert!(
        perf < read,
        "the rank read is not in the poll analysis time"
    );
    // a failed analysis is skipped, and the tick runs before the session
    // machine moves the open game on
    let ok = at("let analysis_ok = matches!(&analyzed, Ok(Ok(_)));");
    let tick = at("daemon::rank::tick(");
    let apply = at("apply_poll_tick(");
    assert!(read < ok && ok < tick && tick < apply);
    assert!(run_loop[ok..tick].contains("if analysis_ok"));
    assert!(run_loop[tick..apply].contains("refresh_snapshot(store, data_dir)"));
    let game = at("daemon::rank::rank_game(st.active_game.as_ref())");
    assert!(ok < game && game < apply);
}

// ---- review regressions: the newest pair only, and a missed tick ----

#[test]
fn a_newer_read_that_disagrees_keeps_no_older_pair() {
    // [A, A, B]: the screen moved on from A, so A is not kept
    let r = settle(&[Some(full(72)), Some(full(72)), Some(full(10))]);
    assert_eq!(r.progress_pct, None);
    // the fields the newest two reads agree on are kept
    assert_eq!(r.hero_sr, n(3412, 0.85));
    assert_eq!(r.division, n(3, 0.9));
}

#[test]
fn a_rank_up_seen_once_before_the_screen_goes_keeps_no_old_division() {
    // division 3 at 95%, then the rank-up to division 2 shows on the last
    // read before the screen goes: neither division is kept
    let mut up = full(5);
    up.division = n(2, 0.9);
    let r = settle(&[Some(full(95)), Some(full(95)), Some(up)]);
    assert_eq!(r.division, None);
    assert_eq!(r.progress_pct, None);
    assert_eq!(r.role_rank, t("Diamond", 0.8));
}

#[tokio::test]
async fn one_missed_tick_does_not_settle_the_screen_and_the_final_reads_are_stored() {
    let (_dir, store) = store_with("g1").await;
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    // [10, 10, gap, 40, 40, gone]: the gap is one tick that missed the screen
    for seen in [
        Some(Some(full(10))),
        Some(Some(full(10))),
        None,
        Some(Some(full(40))),
        Some(Some(full(40))),
        None,
        None,
    ] {
        tick(&mut feed, seen, Some("g1"), &store, None).await;
    }
    let rows = store.get_session_snapshots("g1").await.unwrap();
    let pct = rows[0]
        .rank_update
        .as_ref()
        .and_then(|r| r.progress_pct.clone());
    assert_eq!(pct, n(40, 0.9), "the final reads, not the first ones");
}

#[tokio::test]
async fn a_second_settle_of_the_same_game_keeps_the_hint_its_first_settle_handed_on() {
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
    // [A, A, miss, miss]: the first part agrees on the banner and settles
    for seen in [Some(Some(full(40))), Some(Some(full(40))), None, None] {
        tick(&mut feed, seen, Some("g1"), &store, Some(&mut dump)).await;
    }
    assert_eq!(
        dump.pending_hint(),
        Some(("OPEN QUEUE".to_string(), "g1".to_string()))
    );
    // the screen comes back after two misses with no agreed banner and
    // settles again: the store refuses the second rank, and the hint the
    // first settle handed on stays (no later game came)
    let mut no_banner = full(40);
    no_banner.next_mode_hint = None;
    for seen in [
        Some(Some(no_banner.clone())),
        Some(Some(no_banner)),
        None,
        None,
    ] {
        tick(&mut feed, seen, Some("g1"), &store, Some(&mut dump)).await;
    }
    assert_eq!(
        dump.pending_hint(),
        Some(("OPEN QUEUE".to_string(), "g1".to_string()))
    );
}

#[test]
fn the_banner_classes_map_to_the_queue_title_spelling() {
    use crate::daemon::queue_title::hint_agrees;
    for (banner, hint, title, other) in [
        (
            "COMPETITIVE: 6V6 OPEN QUEUE",
            "OPEN QUEUE",
            "OPEN QUEUE",
            "ROLE QUEUE",
        ),
        (
            "COMPETITIVE-6V6-OPEN-QUEUE",
            "OPEN-QUEUE",
            "OPEN QUEUE",
            "ROLE QUEUE",
        ),
        (
            "COMPETITIVE_6V6_OPEN_QUEUE",
            "OPEN_QUEUE",
            "OPEN QUEUE",
            "ROLE QUEUE",
        ),
        (
            "competitive:6v6_open_queue",
            "open_queue",
            "OPEN QUEUE",
            "ROLE QUEUE",
        ),
        (
            "COMPETITIVE: 5V5 ROLE QUEUE",
            "ROLE QUEUE",
            "ROLE QUEUE",
            "OPEN QUEUE",
        ),
        (
            "COMPETITIVE: OPEN QUEUE",
            "OPEN QUEUE",
            "OPEN QUEUE",
            "ROLE QUEUE",
        ),
        (
            "COMPETITIVE: ROLE QUEUE",
            "ROLE QUEUE",
            "ROLE QUEUE",
            "OPEN QUEUE",
        ),
        (
            " competitive:role queue ",
            "role queue",
            "ROLE QUEUE",
            "OPEN QUEUE",
        ),
        ("ROLE QUEUE", "ROLE QUEUE", "ROLE QUEUE", "OPEN QUEUE"),
    ] {
        assert_eq!(queue_hint(banner), hint, "{banner:?}");
        assert!(hint_agrees(queue_hint(banner), title), "{banner:?}");
        assert!(!hint_agrees(queue_hint(banner), other), "{banner:?}");
    }
    // Anything else is kept as read, trimmed. A team size word is dropped
    // only right after a dropped COMPETITIVE.
    for (banner, kept) in [
        ("OPEN QUEUE", "OPEN QUEUE"),
        (" ROLE QUEUE ", "ROLE QUEUE"),
        ("6V6 OPEN QUEUE", "6V6 OPEN QUEUE"),
        ("COMPETITIVE", "COMPETITIVE"),
        ("COMPETITIVE:", "COMPETITIVE:"),
        ("COMPETITIVE: 6V6", "COMPETITIVE: 6V6"),
        ("COMPETITIVEROLE QUEUE", "COMPETITIVEROLE QUEUE"),
        ("QUICK PLAY: ROLE QUEUE", "QUICK PLAY: ROLE QUEUE"),
    ] {
        assert_eq!(queue_hint(banner), kept, "{banner:?}");
    }
    // QUICK PLAY stays fail-closed: it never agrees with a queue title.
    assert!(!hint_agrees(
        queue_hint("QUICK PLAY: ROLE QUEUE"),
        "ROLE QUEUE"
    ));
    assert!(!hint_agrees(
        queue_hint("QUICK PLAY: OPEN QUEUE"),
        "OPEN QUEUE"
    ));
}

#[tokio::test]
async fn an_open_queue_banner_is_handed_on_without_its_team_size() {
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
    let mut read = full(40);
    read.next_mode_hint = t("COMPETITIVE: 6V6 OPEN QUEUE", 0.75);
    for seen in [
        Some(Some(read.clone())),
        Some(Some(read.clone())),
        None,
        None,
    ] {
        tick(&mut feed, seen, Some("g1"), &store, Some(&mut dump)).await;
    }
    assert_eq!(
        dump.pending_hint(),
        Some(("OPEN QUEUE".to_string(), "g1".to_string()))
    );
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(
        rows[0].rank_update,
        Some(read),
        "the game record keeps the banner word as read"
    );
}

#[tokio::test]
async fn a_competitive_banner_is_handed_on_as_the_queue_title_spelling() {
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
    let mut read = full(40);
    read.next_mode_hint = t("COMPETITIVE: ROLE QUEUE", 0.75);
    for seen in [
        Some(Some(read.clone())),
        Some(Some(read.clone())),
        None,
        None,
    ] {
        tick(&mut feed, seen, Some("g1"), &store, Some(&mut dump)).await;
    }
    assert_eq!(
        dump.pending_hint(),
        Some(("ROLE QUEUE".to_string(), "g1".to_string()))
    );
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(
        rows[0].rank_update,
        Some(read),
        "the game record keeps the banner word as read"
    );
}
