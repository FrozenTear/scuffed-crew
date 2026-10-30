//! Tests for the opt-in pre-game screen saves, next to `pregame_dump.rs`.

use super::pregame_dump::*;
use crate::*;
use std::time::Duration;

const VOTE: Seen = Seen::Phase(PregamePhase::MapVote);
const BAN: Seen = Seen::Phase(PregamePhase::HeroBan);
const SELECT: Seen = Seen::Phase(PregamePhase::HeroSelect);

/// Drives a tracker the way the poll loop does: a tick hands its frame only
/// when [`keeps_frame`] says so. Frames are plain numbers.
struct Run {
    tracker: PregameTracker<u32>,
    start: Instant,
    games: usize,
    shots: Vec<(String, &'static str, u32)>,
}

impl Run {
    fn new() -> Self {
        Self {
            tracker: PregameTracker::default(),
            start: Instant::now(),
            games: 0,
            shots: Vec::new(),
        }
    }

    /// One tick `ms` after the start, showing `seen` as frame `frame`.
    fn tick(&mut self, ms: u64, seen: Seen, frame: u32) {
        let now = self.start + Duration::from_millis(ms);
        let want = self.tracker.want(now);
        let frame = keeps_frame(want, seen).then_some(frame);
        let games = &mut self.games;
        let shots = self.tracker.observe(now, seen, frame, || {
            *games += 1;
            format!("game{games}")
        });
        self.shots
            .extend(shots.into_iter().map(|s| (s.folder, s.name, s.frame)));
    }

    fn names(&self) -> Vec<(&str, u32)> {
        self.shots.iter().map(|(_, n, f)| (*n, *f)).collect()
    }
}

#[test]
fn off_by_default_writes_nothing() {
    assert!(!config::Config::default().debug_save_pregame_screens);
    let dir = tempfile::tempdir().unwrap();
    assert!(daemon::pregame_dump::PregameDump::new(false, dir.path()).is_none());
    assert!(spawn_pregame_saves(pregame_dir(dir.path()), Vec::new()).is_none());
    assert!(!dir.path().join("debug").exists());
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
    run.tick(2_000, VOTE, 3);
    run.tick(3_000, Seen::Loading, 4);
    // Loading written: nothing more is wanted from other screens.
    let after = run.start + Duration::from_millis(4_000);
    assert_eq!(run.tracker.want(after), PregameWant::default());
    let later = run.start + PREGAME_GAME_GAP + Duration::from_secs(10);
    run.tick(
        later.duration_since(run.start).as_millis() as u64,
        Seen::Loading,
        5,
    );
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
    run.tick(12_000, SELECT, 7);
    assert_eq!(
        run.names(),
        [
            ("heroban_first.png", 1),
            ("heroban_last.png", 2),
            ("heroban_last.png", 5),
            ("ban_results.png", 6),
            ("heroselect_first.png", 7),
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
    // with nothing else left to want.
    run.tick(5_000, Seen::Other, 5);
    run.tick(7_000, SELECT, 6);
    assert_eq!(
        run.names(),
        [
            ("mapvote_first.png", 1),
            ("loading.png", 2),
            ("heroban_first.png", 3),
            ("heroban_last.png", 4),
            ("ban_results.png", 5),
            ("heroselect_first.png", 6),
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

fn frame(w: u32, h: u32) -> Arc<image::DynamicImage> {
    Arc::new(image::DynamicImage::ImageRgb8(image::RgbImage::from_fn(
        w,
        h,
        |x, y| image::Rgb([x as u8, y as u8, 7]),
    )))
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
    write_pregame_shots(&root, &[shot], PREGAME_KEEP_GAMES);
    let mut left: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left.len(), PREGAME_KEEP_GAMES + 1);
    for i in 0..3 {
        assert!(!left.contains(&format!("20260101_0000{i:02}")), "{left:?}");
    }
    assert!(left.contains(&"20260101_000003".to_string()));
    assert!(left.contains(&"20261030_120000".to_string()));
    assert!(left.contains(&"keep-me".to_string()));
}

#[tokio::test]
async fn saves_are_full_size_pngs_in_one_game_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = daemon::pregame_dump::PregameDump::new(true, dir.path()).unwrap();
    let start = Instant::now();
    let first = dump.observe(start, VOTE, Some(frame(2560, 1440)));
    first.expect("first frame written").await.unwrap();
    let still = dump.observe(
        start + Duration::from_secs(1),
        VOTE,
        Some(frame(2560, 1440)),
    );
    assert!(still.is_none(), "a frame inside the phase writes nothing");
    let end = dump.observe(start + Duration::from_secs(2), Seen::Other, None);
    end.expect("last frame written").await.unwrap();

    let games: Vec<_> = std::fs::read_dir(pregame_dir(dir.path()))
        .unwrap()
        .map(|e| e.unwrap().path())
        .collect();
    assert_eq!(games.len(), 1);
    for name in ["mapvote_first.png", "mapvote_last.png"] {
        let img = image::open(games[0].join(name)).unwrap();
        assert_eq!((img.width(), img.height()), (2560, 1440));
    }
}
