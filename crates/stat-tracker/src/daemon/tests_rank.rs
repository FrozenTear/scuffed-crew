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
        assert_eq!(read_rank_screen(None, source, false, &img), None);
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
        assert_eq!(read_rank_screen(Some(&read), source, false, &img), None);
    }
    assert_eq!(calls.load(Ordering::SeqCst), 0);
    let seen = read_rank_screen(Some(&read), Some(OutcomeSource::RankScreen), false, &img);
    assert_eq!(seen, Some(Some(full(40))));
    assert_eq!(calls.load(Ordering::SeqCst), 1);
}

#[test]
fn a_reader_that_panics_is_a_frame_with_no_read() {
    let read: RankReadFn = Arc::new(|_| panic!("reader failed (test)"));
    let seen = read_rank_screen(
        Some(&read),
        Some(OutcomeSource::RankScreen),
        false,
        &frame(),
    );
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
    let seen = read_rank_screen(
        Some(&read),
        Some(OutcomeSource::RankScreen),
        false,
        &frame(),
    );
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
    // built once from config `rank_reader` (env override inside); off is `None`
    let feed = at("daemon::rank::RankFeed::for_config(rank_reader, data_dir)");
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
    // read in the analysis thread, after the poll analysis perf sample
    let perf = at("stat_tracker::perf::Stage::PollAnalyze");
    let read = at("daemon::rank::read_rank_screen(");
    assert!(
        perf < read,
        "the rank read is not in the poll analysis time"
    );
    // the held-back flag is this tick's: forgotten before the outcome
    // detector runs, asked after it, and passed to the read
    let holds = at("let rank_holds = rank_feed.as_ref().is_some_and(");
    let forget = at("stability.forget_first_seen();");
    let detect = at("detect::match_end::detect_outcome_signal_polled(");
    let asked = at("stability.first_seen(detect::RANK_SCREEN_CONTEXT)");
    assert!(holds < forget && forget < detect && detect < asked && asked < read);
    let call = &run_loop[read..];
    let call = &call[..call.find(");").expect("the read call ends")];
    assert!(call.contains("rank_held_back"), "{call}");
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

// ---- the switch: config `rank_reader` under the env overrides ----

#[test]
fn rank_reader_drives_both_readers_and_the_env_overrides_it_both_ways() {
    use stat_tracker::shadow::queue_title::QueueTitleSetting;
    use stat_tracker::shadow::rank::override_of;
    for config in [false, true] {
        assert_eq!(RankSetting::for_config(config, None).is_on(), config);
        assert_eq!(QueueTitleSetting::for_config(config, None).is_on(), config);
        let on = Some(RankSetting::parse("on"));
        let off = Some(RankSetting::parse("off"));
        assert!(RankSetting::for_config(config, on).is_on());
        assert!(!RankSetting::for_config(config, off).is_on());
        let on = Some(QueueTitleSetting::parse("1"));
        let off = Some(QueueTitleSetting::parse("0"));
        assert!(QueueTitleSetting::for_config(config, on).is_on());
        assert!(!QueueTitleSetting::for_config(config, off).is_on());
    }
    // unset or blank leaves the config in charge
    assert_eq!(override_of(None), None);
    assert_eq!(override_of(Some("")), None);
    assert_eq!(override_of(Some("  ")), None);
    assert_eq!(override_of(Some("off")), Some("off"));
    // the default stays off: a file with no key reads no rank
    assert!(!stat_tracker::config::Config::default().rank_reader);
}

/// Fail safe: a reader that is on with its pack missing or broken is no
/// feed (the poll runs as with the reader off, so games record as before),
/// and the app's warning names the pack.
#[test]
fn a_reader_on_with_no_usable_pack_records_games_and_warns() {
    use stat_tracker::shadow::queue_title;
    use stat_tracker::shadow::rank::{TEMPLATE_DIR, pack_warning};
    let dir = tempfile::tempdir().expect("tempdir");
    assert!(RankFeed::for_setting(RankSetting::RankV1, dir.path()).is_none());
    let queue = queue_title::QueueTitleSetting::On;
    assert!(queue_title::Reader::for_setting(queue, dir.path()).is_none());
    // no feed reads no frame, held back or not
    for held_back in [false, true] {
        let seen = read_rank_screen(None, Some(OutcomeSource::RankScreen), held_back, &frame());
        assert_eq!(seen, None);
    }
    assert_eq!(pack_warning(dir.path(), false, false), None, "both off");
    let missing = pack_warning(dir.path(), true, true).expect("a warning");
    assert_eq!(
        missing,
        "Read my rank is on, but the pack in templates/rank-v1 is not installed and the pack \
         in templates/queue-title-v1 is not installed. Games still record, without what that \
         pack reads. Install the pack or turn off Read my rank after Comp games."
    );
    assert!(!missing.contains('\u{2014}'), "no em dash");
    let only_queue = pack_warning(dir.path(), false, true).expect("a warning");
    assert!(only_queue.contains("queue-title-v1") && !only_queue.contains("rank-v1 is"));
    // a pack file that does not parse: did not load
    let rank_dir = dir.path().join(TEMPLATE_DIR);
    std::fs::create_dir_all(&rank_dir).unwrap();
    std::fs::write(rank_dir.join("templates.json"), "not a pack").unwrap();
    assert!(RankFeed::for_setting(RankSetting::RankV1, dir.path()).is_none());
    let broken = pack_warning(dir.path(), true, false).expect("a warning");
    assert!(
        broken.starts_with("Read my rank is on, but the pack in templates/rank-v1 did not load ("),
        "{broken}"
    );
}

// ---- Numbani 2026-11-13: a focus pause must not cost the rank ----

/// A synthetic frame: pixel (0, 0) says what it shows.
fn shows(what: u8) -> RgbImage {
    let mut img = frame();
    img.put_pixel(0, 0, image::Rgb([what, 0, 0]));
    img
}
const PROGRESSION: u8 = 1;
const RANK: u8 = 2;
const MENU: u8 = 3;

/// rank-v1 on the synthetic frames: only the rank screen reads.
fn rank_v1(calls: &Arc<AtomicUsize>) -> RankReadFn {
    let calls = Arc::clone(calls);
    Arc::new(move |img| {
        calls.fetch_add(1, Ordering::SeqCst);
        (img.get_pixel(0, 0).0[0] == RANK).then(|| full(40))
    })
}

/// One analyzed poll tick as the loop runs it: the held-back flag counts
/// only while a screen is held. `fixed` false replays the old wiring,
/// which never read a held-back frame.
fn poll(
    feed: &mut RankFeed,
    source: Option<OutcomeSource>,
    held_back: bool,
    img: &RgbImage,
    fixed: bool,
) -> Option<Settled> {
    let read = feed.reader();
    let held_back = fixed && held_back && feed.holds_screen();
    let seen = read_rank_screen(Some(&read), source, held_back, img);
    feed.observe(seen, Some("g1"), Utc::now())
}

/// The replay: a SUMMARY or progression frame the end detector took for
/// the rank screen (rank-v1 rejects it), a tick after it that missed,
/// the focus pause (no tick reaches the feed), the first tick after it
/// held back by the stability gate on a real rank screen, one more read of
/// it, then the screen is gone.
fn numbani(fixed: bool) -> (Vec<Option<Settled>>, usize) {
    let calls = Arc::new(AtomicUsize::new(0));
    let mut feed = RankFeed::with_read(rank_v1(&calls));
    let rank_screen = Some(OutcomeSource::RankScreen);
    let progression = shows(PROGRESSION);
    // one entry held (the log's "1 entry"), then one miss before the pause
    let mut out = vec![
        poll(&mut feed, rank_screen, false, &progression, fixed),
        poll(&mut feed, None, false, &progression, fixed),
    ];
    // focus pause: `poll_skip` continues before any capture or rank tick
    out.push(poll(&mut feed, None, true, &shows(RANK), fixed));
    out.push(poll(&mut feed, rank_screen, false, &shows(RANK), fixed));
    out.push(poll(&mut feed, None, false, &shows(MENU), fixed));
    out.push(poll(&mut feed, None, false, &shows(MENU), fixed));
    (out, calls.load(Ordering::SeqCst))
}

#[test]
fn a_held_back_tick_after_a_focus_pause_keeps_the_rank() {
    let (settled, calls) = numbani(true);
    let kept: Vec<_> = settled.into_iter().flatten().collect();
    assert_eq!(kept.len(), 1, "{kept:?}");
    assert_eq!(kept[0].session, "g1");
    assert_eq!(kept[0].rank, full(40));
    // the false fire, the held-back frame and the real read
    assert_eq!(calls, 3);
    // the old wiring counted the held-back tick as the second miss: it
    // settled the false fire with no rank, then held the one real read
    // alone, which keeps nothing
    let (settled, calls) = numbani(false);
    assert!(settled.iter().all(Option::is_none), "{settled:?}");
    assert_eq!(calls, 2);
}

#[test]
fn a_held_back_frame_counts_only_as_rank_v1_reads_it() {
    let calls = Arc::new(AtomicUsize::new(0));
    let read = rank_v1(&calls);
    // the rank screen under a held-back crop is a read
    let seen = read_rank_screen(Some(&read), None, true, &shows(RANK));
    assert_eq!(seen, Some(Some(full(40))));
    // a frame rank-v1 rejects was looked at and is no rank screen: a miss
    assert_eq!(
        read_rank_screen(Some(&read), None, true, &shows(PROGRESSION)),
        None
    );
    // a classified frame rank-v1 rejects stays a frame with no read
    let seen = read_rank_screen(
        Some(&read),
        Some(OutcomeSource::RankScreen),
        true,
        &shows(MENU),
    );
    assert_eq!(seen, Some(None));
    // not held back: not read
    assert_eq!(
        read_rank_screen(Some(&read), None, false, &shows(RANK)),
        None
    );
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    // a reader that panics on a held-back frame is a miss
    let panics: RankReadFn = Arc::new(|_| panic!("reader failed (test)"));
    assert_eq!(read_rank_screen(Some(&panics), None, true, &frame()), None);
    // with no screen held a held-back frame is not read at all
    let mut feed = RankFeed::with_read(rank_v1(&calls));
    assert!(!feed.holds_screen());
    assert_eq!(poll(&mut feed, None, true, &shows(RANK), true), None);
    assert!(!feed.holds_screen());
    assert_eq!(calls.load(Ordering::SeqCst), 3);
    // progression frames held back while a screen is held settle it as
    // misses (looked at, not a rank screen)
    let rank_screen = Some(OutcomeSource::RankScreen);
    poll(&mut feed, rank_screen, false, &shows(PROGRESSION), true);
    assert!(feed.holds_screen());
    poll(&mut feed, None, true, &shows(PROGRESSION), true);
    poll(&mut feed, None, true, &shows(PROGRESSION), true);
    assert!(!feed.holds_screen(), "two looked-at misses settle it");
}

/// The real gate: the first tick after the focus pause (which drops the
/// stability history) holds a lit rank-screen word crop back, and names it.
#[test]
fn the_first_tick_after_a_pause_holds_the_rank_word_back() {
    use stat_tracker::detect::match_end::{
        FinalScoreSource, ResultWords, Titles, detect_outcome_signal_polled,
    };
    use stat_tracker::detect::{RANK_SCREEN_CONTEXT, stability::FrameStability};
    let detect = |rgb: &RgbImage, stability: &mut FrameStability| {
        stability.forget_first_seen();
        detect_outcome_signal_polled(
            &image::DynamicImage::ImageRgb8(rgb.clone()),
            rgb,
            stability,
            FinalScoreSource::for_reader(false),
            &Titles::for_rgb(false, rgb),
            &ResultWords::for_reader(false),
        )
    };
    // white bars inside the rank word crop (x 1% to 25%, y 14.5% to
    // 22.5% of the 16:9 frame), the rest black
    let mut rgb = frame();
    for y in (1440 * 160 / 1000)..(1440 * 200 / 1000) {
        for x in (2560 * 20 / 1000)..(2560 * 200 / 1000) {
            if (x / 24) % 2 == 0 {
                rgb.put_pixel(x, y, image::Rgb([255, 255, 255]));
            }
        }
    }
    let mut stability = FrameStability::default();
    // as `focus::poll_pause` does on the pause edge
    stability.reset();
    assert_eq!(detect(&rgb, &mut stability), None, "deferred: no source");
    assert!(stability.first_seen(RANK_SCREEN_CONTEXT));
    // a dark frame is never checked, so never a first sighting
    let mut stability = FrameStability::default();
    assert_eq!(detect(&frame(), &mut stability), None);
    assert!(!stability.first_seen(RANK_SCREEN_CONTEXT));
}
