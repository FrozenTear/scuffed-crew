//! Tests for the opt-in pre-game screen saves, next to `pregame_dump.rs`.

use super::pregame_dump::*;
use crate::*;
use std::time::Duration;

const VOTE: Seen = Seen::Phase(PregamePhase::MapVote);
const BAN: Seen = Seen::Phase(PregamePhase::HeroBan);
const SELECT: Seen = Seen::Phase(PregamePhase::HeroSelect);
/// A frame with the ban-results title.
const RESULTS: Seen = Seen::BanResults;
/// The new reader's poll interval.
const POLL: Duration = Duration::from_millis(1_500);

/// Drives a tracker the way the poll loop does: a tick hands its frame only
/// when [`keeps_frame`] says so, and a loading screen reads as one only
/// when the loading check runs ([`classify`]). Frames are numbered `Arc`s
/// so every tick can check the tracker holds at most one, or two while a
/// ban-results frame is owed a write.
struct Run {
    tracker: PregameTracker<Arc<u32>>,
    start: Instant,
    games: usize,
    made: Vec<Arc<u32>>,
    shots: Vec<(String, &'static str, u32)>,
}

impl Run {
    /// At the new reader's 1.5 s polls (ban-results window 16.5 s).
    fn new() -> Self {
        Self::polled(1_500)
    }

    /// At polls `poll_ms` apart (4 s with ocr-v1: window 19 s).
    fn polled(poll_ms: u64) -> Self {
        Self {
            tracker: PregameTracker::new(Duration::from_millis(poll_ms)),
            start: Instant::now(),
            games: 0,
            made: Vec::new(),
            shots: Vec::new(),
        }
    }

    fn at(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }

    /// One tick `ms` after the start, showing `seen` as frame `frame`.
    fn tick(&mut self, ms: u64, seen: Seen, frame: u32) {
        let now = self.at(ms);
        let want = self.tracker.want(now);
        let seen = match seen {
            Seen::Loading if !want.loading => Seen::Other,
            Seen::SelectTitle if !want.select_title => Seen::Other,
            Seen::BanResults if !want.ban_title => Seen::Other,
            seen => seen,
        };
        let frame = keeps_frame(want, seen).then(|| Arc::new(frame));
        if let Some(f) = &frame {
            self.made.push(Arc::clone(f));
        }
        let games = &mut self.games;
        let shots = self.tracker.observe(now, seen, frame, || {
            *games += 1;
            format!("game{games}")
        });
        self.record(shots);
    }

    fn release(&mut self, ms: u64) {
        let shots = self.tracker.release(self.at(ms));
        self.record(shots);
    }

    fn record(&mut self, shots: Vec<PregameShot<Arc<u32>>>) {
        self.shots
            .extend(shots.into_iter().map(|s| (s.folder, s.name, *s.frame)));
        let held = self
            .made
            .iter()
            .filter(|f| Arc::strong_count(f) > 1)
            .count();
        let bound = 1 + usize::from(self.tracker.owes_frame());
        assert!(held <= bound, "the tracker holds {held} frames");
    }

    fn names(&self) -> Vec<(&str, u32)> {
        self.shots.iter().map(|(_, n, f)| (*n, *f)).collect()
    }
}

#[test]
fn off_by_default_writes_nothing() {
    let file = config::Config::parse_file_contents("data_dir = \"/x\"\n").expect("parse");
    assert!(!file.debug_save_pregame_screens);
    let on = config::Config::parse_file_contents(
        "data_dir = \"/x\"\ndebug_save_pregame_screens = true\n",
    )
    .expect("parse");
    assert!(on.debug_save_pregame_screens);

    let dir = tempfile::tempdir().unwrap();
    assert!(PregameDump::new(file.debug_save_pregame_screens, dir.path(), POLL).is_none());
    assert!(!dir.path().join("debug").exists());
    assert_eq!(stale_pregame_dir(false, dir.path()), None);
    std::fs::create_dir_all(pregame_dir(dir.path())).unwrap();
    assert_eq!(
        stale_pregame_dir(false, dir.path()),
        Some(pregame_dir(dir.path()))
    );
    assert_eq!(stale_pregame_dir(true, dir.path()), None);
}

#[test]
fn the_poll_loop_feeds_and_releases_the_saver_and_shutdown_flushes_it() {
    let run_loop: String = crate::main_tests::fn_source("run_loop")
        .split_whitespace()
        .collect();
    // Tab-busy ticks read no phase and are not fed.
    assert!(run_loop.contains(".filter(|_|!outcome_only)"));
    assert!(run_loop.contains("dump.observe(Instant::now(),seen,frame)"));
    // The game gate going false and a failed capture both release it; an
    // expected one-poll miss does not.
    assert_eq!(run_loop.matches("dump.release(Instant::now())").count(), 2);
    assert!(run_loop.contains(
        "ifletSome(dump)=pregame_dump.as_mut()&&(failure!=PollErrorLog::Expected||capture::is_lasting_poll_miss(&e.to_string())){dump.release(Instant::now());}"
    ));
    assert!(run_loop.contains(
        "if!running{ifletSome(dump)=pregame_dump.as_mut(){dump.release(Instant::now());}clear_cadence_wakes("
    ));

    let lifecycle: String = include_str!("lifecycle.rs").split_whitespace().collect();
    let made = lifecycle
        .find("PregameDump::new(ctx.save_pregame_screens,&ctx.data_dir,ctx.auto_detect.poll_interval(ctx.use_new_reader),)")
        .expect("made from the switch");
    let looped = lifecycle.find("letresult=run_loop(").expect("run loop");
    let settle = lifecycle
        .find("finish_or_abort_sync(")
        .expect("sync settle");
    let flush = lifecycle.find("dump.finish(within).await").expect("flush");
    let close = lifecycle
        .rfind("close_store_before_exit(ctx).await")
        .expect("store close");
    assert!(made < looped && looped < settle && settle < flush && flush < close);
}

#[test]
fn each_phase_saves_its_first_and_last_frame_once() {
    let mut run = Run::new();
    run.tick(0, Seen::Other, 0);
    run.tick(1_000, VOTE, 1);
    run.tick(2_000, VOTE, 2);
    run.tick(3_000, VOTE, 3);
    run.tick(4_000, BAN, 4);
    run.tick(5_000, BAN, 5);
    // Too soon after the ban for the results to have animated in.
    run.tick(5_500, RESULTS, 6);
    run.tick(6_200, RESULTS, 7);
    // The hero select's first frame, before its read settles.
    run.tick(7_000, Seen::Other, 8);
    run.tick(8_000, SELECT, 9);
    run.tick(9_000, SELECT, 10);
    run.tick(10_000, SELECT, 11);
    run.tick(11_000, Seen::Loading, 12);
    run.tick(12_000, Seen::Loading, 13);
    run.tick(13_000, Seen::Other, 14);
    assert_eq!(
        run.names(),
        [
            ("mapvote_first.png", 1),
            ("mapvote_last.png", 3),
            ("heroban_first.png", 4),
            ("heroban_last.png", 5),
            ("ban_results.png", 7),
            ("heroselect_first.png", 9),
            ("heroselect_last.png", 11),
            ("loading.png", 12),
        ]
    );
    assert!(run.shots.iter().all(|(folder, _, _)| folder == "game1"));
}

#[test]
fn the_frame_just_before_the_select_read_is_not_the_ban_results() {
    // 1.5 s polls. The phase read lags a new screen by one poll, so the
    // one Other between the ban and the select is the select's first
    // frame.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, SELECT, 4);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("heroselect_first.png", 4),
        ]
    );
}

#[test]
fn ban_results_shown_for_two_polls_keep_the_first_one() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, Seen::Other, 4);
    run.tick(6_000, SELECT, 5);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("heroselect_first.png", 5),
        ]
    );
}

#[test]
fn the_last_ban_results_frame_before_a_select_title_is_saved() {
    // 1.5 s polls: the ban icons and scores fill in under the title over
    // three polls, then the select title shows. Each later results frame
    // confirms the one before it, and the title confirms the last one.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    for (i, ms) in [3_000, 4_500, 6_000].into_iter().enumerate() {
        run.tick(ms, RESULTS, 3 + i as u32);
        assert!(run.tracker.holds_frame());
        assert_eq!(run.tracker.owes_frame(), i > 0);
    }
    run.tick(7_500, Seen::SelectTitle, 6);
    run.tick(9_000, SELECT, 7);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 5),
            ("heroselect_first.png", 7),
        ]
    );
}

#[test]
fn a_plain_frame_after_a_replacement_does_not_confirm_it() {
    // ocr-v1 at 4 s polls (no select title read): R1, then T (the select
    // sliding in under a results title), then the select's first frame S0
    // and its read. T confirms R1; S0 does not confirm T. R1 is saved.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, RESULTS, 3);
    run.tick(12_000, RESULTS, 4);
    run.tick(16_000, Seen::Other, 5);
    run.tick(20_000, SELECT, 6);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("heroselect_first.png", 6),
        ]
    );
    // The same at 1.5 s polls over a longer reveal: the frame before the
    // last results frame is saved.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    for (i, ms) in [3_000, 4_500, 6_000, 7_500].into_iter().enumerate() {
        run.tick(ms, RESULTS, 3 + i as u32);
    }
    run.tick(9_000, Seen::Other, 7);
    run.tick(10_500, SELECT, 8);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 5),
            ("heroselect_first.png", 8),
        ]
    );
}

#[test]
fn a_select_title_confirms_only_the_replacement_right_before_it() {
    // The title is read with no lag, so the replacement right before it
    // was still the results, with the bans filled in: written.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, RESULTS, 4);
    run.tick(6_000, RESULTS, 5);
    run.tick(7_500, Seen::SelectTitle, 6);
    run.tick(9_000, SELECT, 7);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 5),
            ("heroselect_first.png", 7),
        ]
    );
    // A plain frame between: the replacement may be the select sliding in
    // and the plain frame its first frame, so the title confirms nothing
    // and the frame before the replacement is saved.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, RESULTS, 4);
    run.tick(6_000, Seen::Other, 5);
    run.tick(7_500, Seen::SelectTitle, 6);
    run.tick(9_000, SELECT, 7);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("heroselect_first.png", 7),
        ]
    );
}

#[test]
fn a_select_title_after_a_skipped_tick_does_not_confirm_a_replacement() {
    // R1, T, then the S0 tick never reaches the tracker (Tab busy, an
    // expected capture miss), then the select title: two polls after T,
    // so not its next tick. T is not confirmed; R1 is saved.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, RESULTS, 4);
    run.tick(7_500, Seen::SelectTitle, 5);
    run.tick(9_000, SELECT, 6);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("heroselect_first.png", 6),
        ]
    );
}

#[test]
fn a_first_candidate_right_before_a_select_title_read_is_not_saved() {
    // One results frame only, then the title: that frame never had its
    // confirmation, so nothing is written (unchanged).
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, Seen::SelectTitle, 4);
    run.tick(6_000, SELECT, 5);
    assert!(
        run.names().iter().all(|(n, _)| *n != "ban_results.png"),
        "{:?}",
        run.names()
    );
}

#[test]
fn a_release_writes_the_frame_before_the_replacement() {
    // The game closes, or capture fails, after R1 and T: T may be the
    // select sliding in, so R1 (confirmed by T) is written, not T.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, RESULTS, 4);
    run.release(5_000);
    assert!(!run.tracker.holds_frame());
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
        ]
    );
}

#[test]
fn a_replacement_right_before_the_select_read_saves_the_frame_before_it() {
    // The frame just before the select read is the select's first frame:
    // it is dropped, and the results frame it confirmed is written.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, RESULTS, 4);
    run.tick(6_000, SELECT, 5);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("heroselect_first.png", 5),
        ]
    );
    assert!(run.tracker.holds_frame(), "the select's frame");
    assert!(!run.tracker.owes_frame(), "no results frame left");
}

#[test]
fn a_replacement_keeps_the_window_of_the_first_candidate() {
    // 1.5 s polls: results from 1.5 s on, replaced every poll, and no
    // next screen. The window (16.5 s) runs from the first frame, so the
    // held one and the owed one are dropped at 19.5 s however recent.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    for (i, ms) in (1_500..=18_000).step_by(1_500).enumerate() {
        run.tick(ms, RESULTS, 2 + i as u32);
    }
    assert!(run.tracker.owes_frame());
    run.tick(19_500, RESULTS, 30);
    assert!(!run.tracker.holds_frame(), "the owed frame is dropped too");
    run.tick(21_000, Seen::Other, 31);
    run.tick(22_500, SELECT, 32);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
}

#[test]
fn a_ban_back_after_a_long_gap_drops_the_candidate() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(10_000, BAN, 2);
    // An overlay for more than PHASE_FLICKER, then the ban is back.
    for (i, ms) in (11_500..22_000).step_by(1_500).enumerate() {
        run.tick(ms, RESULTS, 10 + i as u32);
    }
    run.tick(23_000, BAN, 30);
    // Past the results window: nothing is left to write.
    run.tick(26_500, Seen::Other, 31);
    run.tick(28_000, Seen::Other, 32);
    run.tick(29_500, SELECT, 33);
    assert!(
        run.names().iter().all(|(n, _)| *n != "ban_results.png"),
        "{:?}",
        run.names()
    );
}

#[test]
fn a_one_tick_phase_writes_only_its_first_frame() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(1_000, SELECT, 2);
    run.tick(2_000, Seen::Other, 3);
    assert_eq!(
        run.names(),
        [("mapvote_first.png", 1), ("heroselect_first.png", 2)]
    );
}

#[test]
fn a_phase_back_after_a_flicker_writes_no_second_first() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(1_000, Seen::Other, 2);
    run.tick(2_000, VOTE, 3);
    run.tick(3_000, VOTE, 4);
    run.tick(4_000, Seen::Other, 5);
    assert_eq!(
        run.names(),
        [("mapvote_first.png", 1), ("mapvote_last.png", 4)]
    );
    assert!(run.shots.iter().all(|(folder, _, _)| folder == "game1"));
}

#[test]
fn hero_select_right_after_the_ban_takes_no_ban_results_frame() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_000, SELECT, 2);
    run.tick(3_000, Seen::Other, 3);
    assert_eq!(
        run.names(),
        [("heroban_first.png", 1), ("heroselect_first.png", 2)]
    );
}

#[test]
fn a_new_game_gets_a_new_folder() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(10_000, SELECT, 2);
    // A map vote after a later screen starts the next game.
    run.tick(20_000, VOTE, 3);
    // So does a ban after a long quiet stretch.
    let later = 20_000 + PREGAME_GAME_GAP.as_millis() as u64 + 1_000;
    run.tick(later, BAN, 4);
    let folders: Vec<&str> = run.shots.iter().map(|(f, _, _)| f.as_str()).collect();
    assert_eq!(folders, ["game1", "game1", "game2", "game3"]);
}

#[test]
fn no_frame_is_kept_or_checked_outside_a_pre_game() {
    let mut run = Run::new();
    assert_eq!(run.tracker.want(run.start), PregameWant::default());
    run.tick(0, Seen::Loading, 1);
    run.tick(1_000, Seen::Other, 2);
    assert!(run.shots.is_empty());
    assert!(run.made.is_empty(), "no frame handed over outside a game");
    run.tick(2_000, VOTE, 3);
    run.tick(3_000, Seen::Loading, 4);
    // Loading written: nothing more is wanted from other screens.
    assert_eq!(run.tracker.want(run.at(4_000)), PregameWant::default());
    let later = (PREGAME_GAME_GAP + Duration::from_secs(10)).as_millis() as u64;
    run.tick(later, Seen::Loading, 5);
    assert_eq!(run.names(), [("mapvote_first.png", 3), ("loading.png", 4)]);
}

#[test]
fn a_ban_back_after_a_flicker_is_not_taken_as_the_results() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(2_000, BAN, 2);
    // The ban read drops for a tick and comes back: not the results (even
    // with the results title).
    run.tick(4_000, RESULTS, 3);
    run.tick(6_000, BAN, 4);
    run.tick(8_000, BAN, 5);
    run.tick(10_000, RESULTS, 6);
    run.tick(11_000, Seen::Other, 7);
    run.tick(12_000, SELECT, 8);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("heroban_last.png", 5),
            ("ban_results.png", 6),
            ("heroselect_first.png", 8),
        ]
    );
}

#[test]
fn ban_results_are_taken_when_loading_was_saved_first() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(1_000, Seen::Loading, 2);
    run.tick(2_000, BAN, 3);
    run.tick(3_000, BAN, 4);
    // The first tick after the ban is the results: its frame is kept even
    // with nothing else left to want, and the next one confirms it.
    run.tick(5_000, RESULTS, 5);
    run.tick(6_000, Seen::Other, 6);
    run.tick(7_000, SELECT, 7);
    assert_eq!(
        run.names(),
        [
            ("mapvote_first.png", 1),
            ("loading.png", 2),
            ("heroban_first.png", 3),
            ("heroban_last.png", 4),
            ("ban_results.png", 5),
            ("heroselect_first.png", 7),
        ]
    );
}

#[test]
fn a_requeue_after_a_cancelled_vote_gets_its_own_folder() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(2_000, VOTE, 2);
    run.tick(4_000, Seen::Other, 3);
    // Cancelled; the next queue pops in under a minute.
    run.tick(50_000, VOTE, 4);
    run.tick(52_000, VOTE, 5);
    run.tick(54_000, BAN, 6);
    assert_eq!(
        run.shots,
        [
            ("game1".to_string(), "mapvote_first.png", 1),
            ("game1".to_string(), "mapvote_last.png", 2),
            ("game2".to_string(), "mapvote_first.png", 4),
            ("game2".to_string(), "mapvote_last.png", 5),
            ("game2".to_string(), "heroban_first.png", 6),
        ]
    );
}

#[test]
fn the_hero_select_between_rounds_writes_nothing() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(5_000, SELECT, 2);
    run.tick(6_000, SELECT, 3);
    run.tick(8_000, Seen::Loading, 4);
    run.tick(10_000, Seen::Other, 5);
    // A short first round, then the select again, inside the game window.
    run.tick(100_000, SELECT, 6);
    run.tick(101_000, SELECT, 7);
    run.tick(102_000, Seen::Other, 8);
    // A long round: the window has closed, and a lone select opens nothing.
    run.tick(600_000, SELECT, 9);
    run.tick(601_000, Seen::Other, 10);
    assert_eq!(
        run.names(),
        [
            ("mapvote_first.png", 1),
            ("heroselect_first.png", 2),
            ("heroselect_last.png", 3),
            ("loading.png", 4),
        ]
    );
    assert_eq!(run.games, 1);
}

#[test]
fn release_writes_the_last_frame_and_holds_nothing() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(1_000, VOTE, 2);
    // The game closes mid-vote.
    run.release(2_000);
    assert!(!run.tracker.holds_frame());
    assert_eq!(
        run.names(),
        [("mapvote_first.png", 1), ("mapvote_last.png", 2)]
    );
    // An unconfirmed ban-results candidate is dropped, not written.
    run.tick(3_000, BAN, 3);
    run.tick(4_000, BAN, 4);
    run.tick(5_500, RESULTS, 5);
    run.release(6_000);
    assert!(!run.tracker.holds_frame());
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
}

#[test]
fn folder_names_are_utc_and_parsed_strictly() {
    use chrono::TimeZone;
    let at = Utc.with_ymd_and_hms(2026, 3, 29, 1, 30, 5).unwrap();
    assert_eq!(pregame_folder_name(at), "20260329_013005");
    assert!(is_game_folder("20260329_013005"));
    for near_miss in [
        "",
        "2026032_013005",
        "20260329_0130055",
        "20260329-013005",
        "20261329_013005",
        "20260230_013005",
        "20260329_246005",
        "2026032a_013005",
        "+2026032_013005",
        " 20260329_01300",
        "20260329_013005.tmp",
        "20260329_013005 ",
    ] {
        assert!(!is_game_folder(near_miss), "{near_miss:?}");
    }
}

#[test]
fn a_new_folder_never_reuses_an_existing_name() {
    use chrono::TimeZone;
    let dir = tempfile::tempdir().unwrap();
    let at = Utc.with_ymd_and_hms(2026, 10, 30, 12, 0, 0).unwrap();
    assert_eq!(unique_pregame_folder(dir.path(), at), "20261030_120000");
    std::fs::create_dir(dir.path().join("20261030_120000")).unwrap();
    std::fs::write(dir.path().join("20261030_120001"), b"").unwrap();
    assert_eq!(unique_pregame_folder(dir.path(), at), "20261030_120002");
}

fn frame(w: u32, h: u32) -> Arc<image::DynamicImage> {
    Arc::new(image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(
        w,
        h,
        |x, y| image::Rgb([x as u8, y as u8, 7]),
    )))
}

fn names_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|name| !name.starts_with('.'))
        .collect();
    names.sort();
    names
}

#[test]
fn retention_keeps_the_newest_twenty_games() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    for i in 0..22 {
        std::fs::create_dir_all(root.join(format!("20260101_0000{i:02}"))).unwrap();
    }
    // Not a game folder: left alone.
    std::fs::create_dir_all(root.join("keep-me")).unwrap();
    let shot = PregameShot {
        folder: "20261030_120000".to_string(),
        name: "mapvote_first.png",
        frame: frame(64, 36),
    };
    write_pregame_shots(&root, &[shot], PREGAME_KEEP_GAMES, &mut Vec::new());
    let left = names_in(&root);
    assert_eq!(left.len(), PREGAME_KEEP_GAMES + 1);
    for i in 0..3 {
        assert!(!left.contains(&format!("20260101_0000{i:02}")), "{left:?}");
    }
    assert!(left.contains(&"20260101_000003".to_string()));
    assert!(left.contains(&"20261030_120000".to_string()));
    assert!(left.contains(&"keep-me".to_string()));
}

#[cfg(unix)]
#[test]
fn retention_never_deletes_files_symlinks_near_misses_or_the_root() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    std::fs::create_dir_all(&root).unwrap();
    // A game-named symlink to a folder outside, with a canary in it.
    let outside = dir.path().join("outside");
    std::fs::create_dir_all(&outside).unwrap();
    std::fs::write(outside.join("canary"), b"keep").unwrap();
    std::os::unix::fs::symlink(&outside, root.join("20250101_000000")).unwrap();
    // A game-named regular file.
    std::fs::write(root.join("20250101_000001"), b"file").unwrap();
    // Near-miss folder names.
    for near in [
        "20250101_00000",
        "20251301_000000",
        "20250101-000000",
        "x20250101_000000",
    ] {
        std::fs::create_dir_all(root.join(near)).unwrap();
    }
    std::fs::create_dir_all(root.join("20250101_000002")).unwrap();
    // Keep none: only the one real game folder other than the new one goes.
    prune_pregame_games(&root, 0, &["20250101_000002".to_string()]);
    assert!(root.is_dir(), "the root survives");
    assert!(
        outside.join("canary").exists(),
        "the symlink target survives"
    );
    let left = names_in(&root);
    assert!(left.contains(&"20250101_000000".to_string()), "{left:?}");
    assert!(left.contains(&"20250101_000001".to_string()), "{left:?}");
    assert!(
        left.contains(&"20250101_000002".to_string()),
        "current kept"
    );
    assert_eq!(left.len(), 7, "{left:?}");

    std::fs::create_dir_all(root.join("20250101_000003")).unwrap();
    prune_pregame_games(
        &root,
        0,
        &["20250101_000002".to_string(), "20250101_000003".to_string()],
    );
    let left = names_in(&root);
    assert!(!left.contains(&"20250101_000002".to_string()), "{left:?}");
    assert!(left.contains(&"20250101_000003".to_string()), "{left:?}");
    assert!(outside.join("canary").exists());
}

#[test]
fn a_clock_set_back_does_not_delete_the_new_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    for i in 0..PREGAME_KEEP_GAMES {
        std::fs::create_dir_all(root.join(format!("20261001_0000{i:02}"))).unwrap();
    }
    // The new game's name sorts oldest.
    let shot = PregameShot {
        folder: "20200101_000000".to_string(),
        name: "mapvote_first.png",
        frame: frame(8, 8),
    };
    write_pregame_shots(&root, &[shot], PREGAME_KEEP_GAMES, &mut Vec::new());
    assert!(
        root.join("20200101_000000")
            .join("mapvote_first.png")
            .exists()
    );
    assert_eq!(names_in(&root).len(), PREGAME_KEEP_GAMES);
}

#[test]
fn the_folder_order_survives_a_restart_with_the_clock_behind() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    for i in 0..PREGAME_KEEP_GAMES - 1 {
        std::fs::create_dir_all(root.join(format!("20261001_0000{i:02}"))).unwrap();
    }
    let shot = |name: &str| PregameShot {
        folder: name.to_string(),
        name: "mapvote_first.png",
        frame: frame(8, 8),
    };
    // Run 1, clock behind: one game, 20 folders in all.
    let mut made = load_pregame_order(&root);
    write_pregame_shots(
        &root,
        &[shot("20200101_000000")],
        PREGAME_KEEP_GAMES,
        &mut made,
    );
    // Run 2 reads the order back; its first game must not prune run 1's.
    let mut made = load_pregame_order(&root);
    assert_eq!(made, ["20200101_000000"]);
    write_pregame_shots(
        &root,
        &[shot("20200102_000000")],
        PREGAME_KEEP_GAMES,
        &mut made,
    );
    let left = names_in(&root);
    assert!(left.contains(&"20200101_000000".to_string()), "{left:?}");
    assert!(left.contains(&"20200102_000000".to_string()), "{left:?}");
    assert!(!left.contains(&"20261001_000000".to_string()), "{left:?}");
    assert_eq!(
        load_pregame_order(&root),
        ["20200101_000000", "20200102_000000"]
    );
}

#[test]
fn a_repeated_name_in_the_order_file_counts_once() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    for i in 0..PREGAME_KEEP_GAMES - 1 {
        std::fs::create_dir_all(root.join(format!("20261001_0000{i:02}"))).unwrap();
    }
    let repeated = "20261001_000005\n".repeat(25);
    std::fs::write(root.join(".order"), format!("{repeated}not-a-game\n")).unwrap();
    let mut made = load_pregame_order(&root);
    assert_eq!(made, ["20261001_000005"]);
    // Prune itself also counts a repeat once.
    made.push("20261001_000005".to_string());
    let shot = PregameShot {
        folder: "20261030_120000".to_string(),
        name: "mapvote_first.png",
        frame: frame(8, 8),
    };
    // 20 folders with the new one: nothing goes.
    write_pregame_shots(&root, &[shot], PREGAME_KEEP_GAMES, &mut made);
    assert_eq!(names_in(&root).len(), PREGAME_KEEP_GAMES);
}

#[test]
fn games_written_while_the_clock_is_behind_all_count_as_newest() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    for i in 0..PREGAME_KEEP_GAMES {
        std::fs::create_dir_all(root.join(format!("20261001_0000{i:02}"))).unwrap();
    }
    let mut made = Vec::new();
    for name in ["20200101_000000", "20200101_000500"] {
        let shot = PregameShot {
            folder: name.to_string(),
            name: "mapvote_first.png",
            frame: frame(8, 8),
        };
        write_pregame_shots(&root, &[shot], PREGAME_KEEP_GAMES, &mut made);
    }
    let left = names_in(&root);
    assert_eq!(left.len(), PREGAME_KEEP_GAMES);
    assert!(left.contains(&"20200101_000000".to_string()), "{left:?}");
    assert!(left.contains(&"20200101_000500".to_string()), "{left:?}");
    // The two oldest names from before this run went instead.
    assert!(!left.contains(&"20261001_000000".to_string()), "{left:?}");
    assert!(!left.contains(&"20261001_000001".to_string()), "{left:?}");
}

#[test]
fn a_save_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    let shot = |w| PregameShot {
        folder: "20261030_120000".to_string(),
        name: "heroban_last.png",
        frame: frame(w, 9),
    };
    write_pregame_shots(
        &root,
        &[shot(10), shot(20)],
        PREGAME_KEEP_GAMES,
        &mut Vec::new(),
    );
    let game = root.join("20261030_120000");
    // Every name, dot names too: a leftover `.heroban_last.png.tmp` fails.
    let mut all: Vec<String> = std::fs::read_dir(&game)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    all.sort();
    assert_eq!(all, ["heroban_last.png"]);
    assert_eq!(
        image::open(game.join("heroban_last.png")).unwrap().width(),
        20
    );
}

#[tokio::test]
async fn saves_are_full_size_pngs_written_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    dump.observe(start, VOTE, Some(frame(64, 36)));
    dump.observe(start + Duration::from_secs(1), VOTE, Some(frame(48, 36)));
    dump.observe(start + Duration::from_secs(2), Seen::Other, None);
    dump.observe(start + Duration::from_secs(3), VOTE, Some(frame(32, 36)));
    // Still on screen at shutdown: the last frame is written then.
    assert!(dump.finish(Duration::from_secs(30)).await);

    let root = pregame_dir(dir.path());
    let games: Vec<_> = names_in(&root).iter().map(|n| root.join(n)).collect();
    assert_eq!(games.len(), 1);
    assert_eq!(load_pregame_order(&root), names_in(&root));
    assert_eq!(
        names_in(&games[0]),
        ["mapvote_first.png", "mapvote_last.png"]
    );
    let first = image::open(games[0].join("mapvote_first.png")).unwrap();
    assert_eq!((first.width(), first.height()), (64, 36));
    // The flicker's last frame (48) is replaced by the later one (32).
    let last = image::open(games[0].join("mapvote_last.png")).unwrap();
    assert_eq!((last.width(), last.height()), (32, 36));
}

/// The loading screen shape: black bars, a navy strip with glyphs.
fn loading_frame() -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(640, 360, image::Rgb([0, 0, 0]));
    for y in 136..224 {
        for x in 0..640 {
            img.put_pixel(x, y, image::Rgb([24, 32, 52]));
        }
    }
    for y in 168..192 {
        for x in (200..440).filter(|x| x % 7 < 3) {
            img.put_pixel(x, y, image::Rgb([240, 240, 240]));
        }
    }
    img
}

const LOADING_WANTED: PregameWant = PregameWant {
    loading: true,
    other_frame: true,
    select_title: false,
    ban_title: false,
};

fn no_title(_: &image::RgbImage) -> bool {
    false
}

#[test]
fn the_loading_check_skips_a_ban_screen_but_not_a_navy_tinted_black() {
    let unknown = detect::GamePhase::Unknown;
    assert_eq!(
        classify_with(
            LOADING_WANTED,
            &unknown,
            &loading_frame(),
            no_title,
            no_title
        ),
        Seen::Loading
    );
    // Black tinted navy (blue 34 counts as dark): still the loading screen.
    let mut tinted = loading_frame();
    for p in tinted.pixels_mut() {
        if p.0 == [0, 0, 0] {
            p.0 = [10, 10, 34];
        }
    }
    assert!(detect::match_start::gate_metrics(&tinted).navy_ratio >= 0.40);
    assert_eq!(
        classify_with(LOADING_WANTED, &unknown, &tinted, no_title, no_title),
        Seen::Loading
    );
    // A bright red accent in the top band: the hero-ban gate passes, so it
    // is a ban screen settling, not the loading screen.
    let mut ban = loading_frame();
    for y in 6..16 {
        for x in 300..340 {
            ban.put_pixel(x, y, image::Rgb([220, 30, 30]));
        }
    }
    assert!(detect::loading_screen::is_loading_screen(&ban));
    assert!(detect::match_start::passes_ban_gate(&ban));
    assert_eq!(
        classify_with(LOADING_WANTED, &unknown, &ban, no_title, no_title),
        Seen::Other
    );
    let no_check = PregameWant {
        loading: false,
        ..LOADING_WANTED
    };
    assert_eq!(
        classify_with(no_check, &unknown, &loading_frame(), no_title, no_title),
        Seen::Other
    );
}

#[test]
fn the_select_title_is_read_only_while_the_ban_results_wait() {
    let unknown = detect::GamePhase::Unknown;
    let img = image::RgbImage::new(64, 36);
    let waiting = PregameWant {
        loading: false,
        other_frame: true,
        select_title: true,
        ban_title: true,
    };
    assert_eq!(
        classify_with(waiting, &unknown, &img, |_| true, no_title),
        Seen::SelectTitle
    );
    assert_eq!(
        classify_with(waiting, &unknown, &img, no_title, no_title),
        Seen::Other
    );
    let idle = PregameWant {
        select_title: false,
        ban_title: false,
        ..waiting
    };
    let mut asked = false;
    let seen = classify_with(
        idle,
        &unknown,
        &img,
        |_| {
            asked = true;
            true
        },
        no_title,
    );
    assert_eq!(seen, Seen::Other);
    assert!(!asked, "no title read outside the wait");
    // A phase read wins over the title.
    let select = detect::GamePhase::HeroSelect;
    assert_eq!(
        classify_with(waiting, &select, &img, |_| true, |_| true),
        SELECT
    );
    assert!(!keeps_frame(waiting, Seen::SelectTitle));
    // The ban-results title comes after the select title, and both after
    // a phase read.
    assert_eq!(
        classify_with(waiting, &unknown, &img, no_title, |_| true),
        Seen::BanResults
    );
    assert_eq!(
        classify_with(waiting, &unknown, &img, |_| true, |_| true),
        Seen::SelectTitle
    );
    assert!(keeps_frame(waiting, Seen::BanResults));
    assert!(!keeps_frame(waiting, Seen::Other));
}

#[test]
fn a_sure_or_suspect_select_title_read_counts() {
    use stat_tracker::shadow::result::phrase::PhraseRead;
    assert!(shows_select_title(Some(PhraseRead::Sure(0.9))));
    assert!(shows_select_title(Some(PhraseRead::Suspect(0.5))));
    assert!(!shows_select_title(Some(PhraseRead::Absent(0.1))));
    assert!(!shows_select_title(None));
}

#[test]
fn a_select_read_two_polls_late_is_not_the_ban_results() {
    // 1.5 s polls: the select's first frame reads as nothing, its second
    // shows the title but its phase read waits one more poll.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, Seen::SelectTitle, 4);
    run.tick(6_000, SELECT, 5);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("heroselect_first.png", 5),
        ]
    );
}

#[test]
fn a_missed_results_screen_then_the_roster_saves_no_results() {
    // A 1 s results screen falls between two 1.5 s polls; the roster
    // (no results title) follows for two polls.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, Seen::Other, 3);
    run.tick(4_500, Seen::Other, 4);
    run.tick(6_000, SELECT, 5);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
    // The same at 4 s polls.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, Seen::Other, 3);
    run.tick(12_000, Seen::Other, 4);
    run.tick(16_000, SELECT, 5);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
}

#[test]
fn a_dropped_candidate_is_never_saved_later() {
    // No results title after the ban: nothing becomes a candidate, and
    // neither the game closing nor shutdown writes one.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, Seen::Other, 3);
    run.tick(4_500, Seen::Other, 4);
    run.release(6_000);
    run.tick(7_500, Seen::Loading, 5);
    run.tick(9_000, SELECT, 6);
    run.release(10_500);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
    // A confirmed candidate past the window is dropped too, on every path:
    // here the game closing (`release`) settles it 18.5 s after its frame.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, RESULTS, 2);
    run.tick(3_000, Seen::Other, 3);
    run.release(20_000);
    assert!(!run.tracker.holds_frame());
    run.tick(21_000, Seen::Loading, 4);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
}

#[tokio::test]
async fn the_shutdown_flush_writes_no_unmatched_candidate() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    dump.observe(at(0), BAN, Some(frame(64, 36)));
    dump.observe(at(1_500), BAN, Some(frame(64, 36)));
    // No examples are installed: the shape rule decides, and the roster (no
    // bright title) fails it. Each frame goes through the real classify and
    // is handed over, so a tracker that took any frame would write one.
    let roster = image::RgbImage::from_pixel(640, 360, image::Rgb([30, 34, 44]));
    for ms in [3_000, 4_500] {
        let want = dump.want(at(ms));
        assert!(want.ban_title);
        let seen = classify(want, &detect::GamePhase::Unknown, &roster, &dump.examples());
        assert_eq!(seen, Seen::Other);
        let shown = Arc::new(image::DynamicImage::ImageRgb8(roster.clone()));
        dump.observe(at(ms), seen, Some(shown));
    }
    assert!(dump.finish(Duration::from_secs(30)).await);
    let root = pregame_dir(dir.path());
    let games = names_in(&root);
    assert_eq!(games.len(), 1);
    assert_eq!(
        names_in(&root.join(&games[0])),
        ["heroban_first.png", "heroban_last.png"]
    );
}

#[test]
fn results_then_a_late_select_read_keep_the_results() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, Seen::Other, 4);
    run.tick(6_000, Seen::SelectTitle, 5);
    run.tick(7_500, SELECT, 6);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("heroselect_first.png", 6),
        ]
    );
}

#[test]
fn ban_results_at_a_four_second_poll() {
    // ocr-v1 polls every 4 s: results up for two polls are kept.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, RESULTS, 3);
    run.tick(12_000, Seen::Other, 4);
    run.tick(16_000, SELECT, 5);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("heroselect_first.png", 5),
        ]
    );
    // One poll between the ban and the select read: it is the select.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, RESULTS, 3);
    run.tick(12_000, SELECT, 4);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
    // The window runs from the results frame: ban 0 s, results 4 s, roster
    // 8 s, the select's first frame 12 s, the select read 16 s.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, RESULTS, 2);
    run.tick(8_000, Seen::Other, 3);
    run.tick(12_000, Seen::Other, 4);
    run.tick(16_000, SELECT, 5);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("ban_results.png", 2),
            ("heroselect_first.png", 5),
        ]
    );
    // Two late polls, the second showing the select title: still the select.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, RESULTS, 3);
    run.tick(12_000, Seen::SelectTitle, 4);
    run.tick(16_000, SELECT, 5);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
}

#[test]
fn results_then_the_roster_then_loading_inside_the_window_are_saved() {
    // 4 s polls. Ban 2 s, results 6 s, roster 10 s and 14 s, ENTERING GAME
    // 18 s: past 15 s from the ban, inside the window from the results.
    let mut run = Run::polled(4_000);
    run.tick(0, VOTE, 1);
    run.tick(2_000, BAN, 2);
    run.tick(6_000, RESULTS, 3);
    run.tick(10_000, Seen::Other, 4);
    run.tick(14_000, Seen::Other, 5);
    run.tick(18_000, Seen::Loading, 6);
    assert_eq!(
        run.names(),
        [
            ("mapvote_first.png", 1),
            ("heroban_first.png", 2),
            ("ban_results.png", 3),
            ("loading.png", 6),
        ]
    );
    // The same with the loading screen past the window (15 s plus the 4 s
    // poll) from the results frame.
    let mut run = Run::polled(4_000);
    run.tick(0, VOTE, 1);
    run.tick(2_000, BAN, 2);
    run.tick(6_000, RESULTS, 3);
    run.tick(10_000, Seen::Other, 4);
    run.tick(14_000, Seen::Other, 5);
    run.tick(25_500, Seen::Loading, 6);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
}

#[test]
fn results_then_the_roster_then_nothing_drop_the_candidate() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, RESULTS, 2);
    // Past 16.5 s (15 s plus the 1.5 s poll) after the results frame.
    for ms in (3_000..=19_500).step_by(1_500) {
        run.tick(ms, Seen::Other, 3);
    }
    assert!(!run.tracker.holds_frame(), "dropped past its window");
    run.release(30_000);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
}

#[test]
fn the_window_adds_one_poll_to_fifteen_seconds() {
    // 4 s polls: ban 0 s and 4 s, results 8 s, roster 12, 16 and 20 s,
    // ENTERING GAME 24 s, 16 s after the results frame: past 15 s, inside
    // 15 s plus one poll.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, RESULTS, 3);
    for (i, ms) in [12_000, 16_000, 20_000].into_iter().enumerate() {
        run.tick(ms, Seen::Other, 4 + i as u32);
    }
    run.tick(24_000, Seen::Loading, 7);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("ban_results.png", 3),
            ("loading.png", 7),
        ]
    );
    // One more roster poll: 20 s after the results, past 19 s.
    let mut run = Run::polled(4_000);
    run.tick(0, BAN, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, RESULTS, 3);
    for (i, ms) in [12_000, 16_000, 20_000, 24_000].into_iter().enumerate() {
        run.tick(ms, Seen::Other, 4 + i as u32);
    }
    run.tick(28_000, Seen::Loading, 8);
    assert!(run.names().iter().all(|(n, _)| *n != "ban_results.png"));
    assert_eq!(BAN_RESULTS_WINDOW, Duration::from_secs(15));
}

#[test]
fn a_ban_read_lost_past_the_window_saves_no_results() {
    // BAN at 0 s and 10 s, other screens every 1.5 s from 11.5 s, the ban
    // back at 31 s.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(10_000, BAN, 2);
    for (i, ms) in (11_500..31_000).step_by(1_500).enumerate() {
        run.tick(ms, RESULTS, 10 + i as u32);
    }
    // The candidate from 11.5 s ran out of its window (16.5 s) at 28 s.
    assert!(!run.tracker.holds_frame());
    run.tick(31_000, BAN, 40);
    run.tick(32_500, Seen::Other, 41);
    run.tick(34_000, Seen::Other, 42);
    run.tick(35_500, SELECT, 43);
    assert!(
        run.names().iter().all(|(n, _)| *n != "ban_results.png"),
        "{:?}",
        run.names()
    );
}

#[test]
fn a_dead_writer_is_logged_once_and_told_apart_from_a_busy_one() {
    let mut writer = PregameWriter::dead();
    let shot = || {
        vec![PregameShot {
            folder: "20261030_120000".to_string(),
            name: "mapvote_first.png",
            frame: frame(4, 4),
        }]
    };
    assert!(writer.send(Vec::new()), "nothing to send is no failure");
    assert!(!writer.dead_logged());
    assert!(!writer.send(shot()));
    assert!(writer.dead_logged());
    assert!(!writer.send(shot()));
    assert!(writer.dead_logged());
    // A busy writer (queue full, thread alive) is not logged as dead.
    let (mut busy, _queue) = PregameWriter::stalled();
    for _ in 0..PREGAME_QUEUE {
        assert!(busy.send(shot()));
    }
    assert!(!busy.send(shot()), "a full queue drops the batch");
    assert!(!busy.dead_logged());
}
