//! Tests for the opt-in pre-game screen saves, next to `pregame_dump.rs`.

use super::pregame_dump::*;
use crate::*;
use std::time::Duration;

const VOTE: Seen = Seen::Phase(PregamePhase::MapVote);
const BAN: Seen = Seen::Phase(PregamePhase::HeroBan);
const SELECT: Seen = Seen::Phase(PregamePhase::HeroSelect);

/// Drives a tracker the way the poll loop does: a tick hands its frame only
/// when [`keeps_frame`] says so, and a loading screen reads as one only
/// when the loading check runs ([`classify`]). Frames are numbered `Arc`s
/// so every tick can check the tracker holds at most one.
struct Run {
    tracker: PregameTracker<Arc<u32>>,
    start: Instant,
    games: usize,
    made: Vec<Arc<u32>>,
    shots: Vec<(String, &'static str, u32)>,
}

impl Run {
    fn new() -> Self {
        Self {
            tracker: PregameTracker::default(),
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
        let seen = if seen == Seen::Loading && !want.loading {
            Seen::Other
        } else {
            seen
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
        assert!(held <= 1, "the tracker holds {held} frames");
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
    assert!(PregameDump::new(file.debug_save_pregame_screens, dir.path()).is_none());
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
        "iffailure!=PollErrorLog::Expected&&letSome(dump)=pregame_dump.as_mut(){dump.release(Instant::now());}"
    ));
    assert!(run_loop.contains(
        "if!running{ifletSome(dump)=pregame_dump.as_mut(){dump.release(Instant::now());}clear_cadence_wakes("
    ));

    let lifecycle: String = include_str!("lifecycle.rs").split_whitespace().collect();
    let made = lifecycle
        .find("PregameDump::new(ctx.save_pregame_screens,&ctx.data_dir)")
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
    run.tick(5_500, Seen::Other, 6);
    run.tick(6_200, Seen::Other, 7);
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
    run.tick(3_000, Seen::Other, 3);
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
    run.tick(3_000, Seen::Other, 3);
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
fn a_ban_back_after_a_long_gap_drops_the_candidate() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(10_000, BAN, 2);
    // An overlay for more than PHASE_FLICKER, then the ban is back.
    for (i, ms) in (11_500..22_000).step_by(1_500).enumerate() {
        run.tick(ms, Seen::Other, 10 + i as u32);
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
    // The ban read drops for a tick and comes back: not the results.
    run.tick(4_000, Seen::Other, 3);
    run.tick(6_000, BAN, 4);
    run.tick(8_000, BAN, 5);
    run.tick(10_000, Seen::Other, 6);
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
    run.tick(5_000, Seen::Other, 5);
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
    run.tick(5_500, Seen::Other, 5);
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
    assert_eq!(names_in(&game), ["heroban_last.png"]);
    assert_eq!(
        image::open(game.join("heroban_last.png")).unwrap().width(),
        20
    );
}

#[tokio::test]
async fn saves_are_full_size_pngs_written_in_order() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path()).unwrap();
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

/// The loading screen shape with its black bars in a navy that the
/// map-vote gate counts (and the dark check counts as black).
fn loading_with_navy_bars() -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(640, 360, image::Rgb([10, 10, 34]));
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

#[test]
fn a_frame_passing_the_vote_gate_gets_no_loading_check() {
    let img = loading_with_navy_bars();
    assert!(detect::loading_screen::is_loading_screen(&img));
    let want = PregameWant {
        loading: true,
        other_frame: true,
    };
    let unknown = detect::GamePhase::Unknown;
    assert_eq!(classify(want, &unknown, &img), Seen::Other);
    let mut black_bars = img.clone();
    for p in black_bars.pixels_mut() {
        if p.0 == [10, 10, 34] {
            p.0 = [0, 0, 0];
        }
    }
    assert_eq!(classify(want, &unknown, &black_bars), Seen::Loading);
    let no_check = PregameWant {
        loading: false,
        other_frame: true,
    };
    assert_eq!(classify(no_check, &unknown, &black_bars), Seen::Other);
}
