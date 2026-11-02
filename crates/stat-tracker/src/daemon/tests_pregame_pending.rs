//! Tests for the pending slot of the pre-game saves (`pregame_dump/pending.rs`):
//! the loading and roster frames seen before their game folder opens, the
//! two-sample roster rule, the slot's window, and its files on disk.
//! Synthetic frames only.

use super::pregame_dump::*;
use super::tests_pregame_dump::Run;
use crate::*;
use std::time::Duration;

const VOTE: Seen = Seen::Phase(PregamePhase::MapVote);
const BAN: Seen = Seen::Phase(PregamePhase::HeroBan);
const SELECT: Seen = Seen::Phase(PregamePhase::HeroSelect);

impl Run {
    /// [`PregameOp::Adopt`] on the disk model: the frame staged in the
    /// pending folder moves into `folder`, saved. Nothing staged, nothing
    /// moves.
    pub(crate) fn adopt(&mut self, folder: String, name: &'static str) {
        let pending = |(f, n, _): &(String, &str, u32)| f == PENDING_DIR && *n == name;
        if let Some(at) = self.staged.iter().position(pending) {
            let (_, _, frame) = self.staged.remove(at);
            self.shots.push((folder, name, frame));
        }
    }

    /// The saved files as (folder, name, frame), rejected ones left out.
    fn files(&self) -> Vec<(&str, &str, u32)> {
        self.shots
            .iter()
            .filter(|(_, n, _)| !n.starts_with("rejected_"))
            .map(|(f, n, frame)| (f.as_str(), *n, *frame))
            .collect()
    }

    fn staged_pending(&self) -> Vec<u32> {
        self.staged
            .iter()
            .filter(|(f, n, _)| f == PENDING_DIR && *n == ROSTER_NAME)
            .map(|(_, _, frame)| *frame)
            .collect()
    }
}

fn window_ms() -> u64 {
    PENDING_WINDOW.as_millis() as u64
}

#[test]
fn a_loading_frame_before_the_map_vote_is_saved_in_the_votes_folder() {
    let mut run = Run::new();
    run.tick(0, Seen::Loading, 1);
    run.tick(1_500, Seen::Other, 0);
    assert!(run.tracker.holds_loading());
    assert!(run.shots.is_empty());
    // The map vote after it opens the folder and takes it.
    run.tick(3_000, VOTE, 2);
    run.tick(6_000, BAN, 3);
    assert_eq!(
        run.files(),
        [
            ("game1", LOADING_NAME, 1),
            ("game1", "mapvote_first.png", 2),
            ("game1", "heroban_first.png", 3),
        ]
    );
    assert!(!run.tracker.holds_loading());
}

#[test]
fn a_pass_before_the_vote_never_shuts_out_the_games_own_loading_screen() {
    // An older pass (the last match, an update band) is saved at the vote;
    // the game's ENTERING GAME after the vote replaces it at the start.
    let mut run = Run::new();
    run.tick(0, Seen::Loading, 1);
    run.tick(3_000, VOTE, 2);
    run.tick(6_000, Seen::Loading, 3);
    run.tick(7_500, Seen::Loading, 4);
    run.tick(9_000, BAN, 5);
    assert_eq!(run.written_in("game1", LOADING_NAME), [1, 4]);
    // Saved at the start: no more passes are wanted in this game.
    run.tick(10_500, Seen::Loading, 6);
    run.tick(12_000, SELECT, 7);
    assert_eq!(run.written_in("game1", LOADING_NAME), [1, 4]);
}

#[test]
fn a_roster_sample_after_a_missed_poll_starts_the_count_again() {
    // Two roster samples with no tick fed between them for more than one
    // and a half poll intervals are not in a row.
    let mut run = Run::new();
    run.tick(0, Seen::Roster, 1);
    run.tick(3_000, Seen::Roster, 2);
    assert!(!run.tracker.holds_pending_roster());
    run.tick(4_500, Seen::Roster, 3);
    assert_eq!(run.staged_pending(), [3]);
}

#[test]
fn a_dropped_pending_stage_empties_the_slot() {
    let mut run = Run::new();
    run.tick(0, Seen::Roster, 1);
    run.tick(1_500, Seen::Roster, 2);
    // The writer's queue was full: that stage never reached the disk.
    run.tracker.stage_lost(ROSTER_NAME);
    assert!(!run.tracker.holds_pending_roster());
    run.tick(3_000, Seen::Other, 0);
    run.tick(6_000, VOTE, 3);
    assert_eq!(run.written_in("game1", ROSTER_NAME), Vec::<u32>::new());
}

#[test]
fn a_loading_frame_survives_the_old_twenty_second_wait() {
    // In an open game: the start read 30 s after the last pass.
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(2_000, Seen::Loading, 2);
    for ms in (3_500..=30_500).step_by(1_500) {
        run.tick(ms, Seen::Other, 0);
    }
    assert!(run.tracker.holds_loading(), "held past 20 s");
    run.tick(32_000, BAN, 3);
    assert_eq!(run.written_in("game1", LOADING_NAME), [2]);
    // With no game open: the vote a minute later still takes it.
    let mut run = Run::new();
    run.tick(0, Seen::Loading, 1);
    run.tick(60_000, VOTE, 2);
    assert_eq!(run.written_in("game1", LOADING_NAME), [1]);
}

#[test]
fn a_roster_before_any_game_folder_is_attached_when_the_folder_opens() {
    let mut run = Run::new();
    run.tick(0, Seen::Roster, 1);
    assert!(
        !run.tracker.holds_pending_roster(),
        "one sample is not enough"
    );
    run.tick(1_500, Seen::Roster, 2);
    assert!(run.tracker.holds_pending_roster());
    assert_eq!(run.staged_pending(), [2]);
    // Each later roster frame replaces the staged one; nothing in memory.
    run.tick(3_000, Seen::Roster, 3);
    assert_eq!(run.staged_pending(), [3]);
    assert!(!run.tracker.holds_frame());
    run.tick(4_500, Seen::Other, 0);
    assert!(run.shots.is_empty());
    run.tick(9_000, VOTE, 4);
    assert_eq!(
        run.files(),
        [("game1", ROSTER_NAME, 3), ("game1", "mapvote_first.png", 4),]
    );
    assert!(run.staged.is_empty(), "{:?}", run.staged);
    assert!(!run.tracker.holds_pending_roster());
    // The next game folder gets nothing more from the slot.
    run.tick(
        9_000 + PREGAME_GAME_GAP.as_millis() as u64 + 10_000,
        VOTE,
        5,
    );
    assert_eq!(run.written_in("game2", ROSTER_NAME), Vec::<u32>::new());
}

#[test]
fn roster_and_loading_frames_are_held_apart() {
    // Either order: each is saved under its own name, neither replaces
    // the other.
    for roster_first in [true, false] {
        let mut run = Run::new();
        if roster_first {
            run.tick(0, Seen::Roster, 10);
            run.tick(1_500, Seen::Roster, 11);
            run.tick(4_500, Seen::Loading, 20);
        } else {
            run.tick(0, Seen::Loading, 20);
            run.tick(1_500, Seen::Roster, 10);
            run.tick(3_000, Seen::Roster, 11);
        }
        run.tick(9_000, VOTE, 30);
        let mut files = run.files();
        files.sort();
        assert_eq!(
            files,
            [
                ("game1", LOADING_NAME, 20),
                ("game1", "mapvote_first.png", 30),
                ("game1", ROSTER_NAME, 11),
            ],
            "roster first {roster_first}"
        );
    }
}

#[test]
fn a_roster_needs_two_samples_in_a_row() {
    // With no game: a gap after each single sample starts the count again.
    let mut run = Run::new();
    for k in 0..4u64 {
        run.tick(3_000 * k, Seen::Roster, 1);
        run.tick(3_000 * k + 1_500, Seen::Other, 0);
    }
    assert!(!run.tracker.holds_pending_roster());
    assert!(run.staged.is_empty());
    // In a game: the same, then two in a row are accepted.
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(3_000, BAN, 2);
    run.tick(6_000, Seen::Roster, 3);
    run.tick(7_500, Seen::BanResults, 0);
    run.tick(9_000, Seen::Roster, 4);
    run.tick(10_500, Seen::Other, 0);
    assert_eq!(run.written_in("game1", ROSTER_NAME), Vec::<u32>::new());
    run.tick(12_000, Seen::Roster, 5);
    run.tick(13_500, Seen::Roster, 6);
    run.tick(15_000, SELECT, 7);
    assert_eq!(run.written_in("game1", ROSTER_NAME), [6]);
}

#[test]
fn the_pending_slot_drops_its_frames_after_its_window() {
    let mut run = Run::new();
    run.tick(0, Seen::Roster, 1);
    run.tick(1_500, Seen::Roster, 2);
    run.tick(3_000, Seen::Loading, 3);
    // Inside the window both are still held.
    run.tick(window_ms(), Seen::Other, 0);
    assert!(run.tracker.holds_pending_roster() && run.tracker.holds_loading());
    // Past it: the loading frame is let go, the staged roster deleted.
    run.tick(3_000 + window_ms() + 1_500, Seen::Other, 0);
    assert!(!run.tracker.holds_pending_roster());
    assert!(!run.tracker.holds_loading());
    assert!(run.staged.is_empty(), "{:?}", run.staged);
    // A game folder after that gets neither.
    run.tick(3_000 + window_ms() + 3_000, VOTE, 4);
    assert_eq!(run.files(), [("game1", "mapvote_first.png", 4)]);
}

#[test]
fn a_release_drops_the_pending_slot() {
    let mut run = Run::new();
    run.tick(0, Seen::Roster, 1);
    run.tick(1_500, Seen::Roster, 2);
    run.tick(3_000, Seen::Loading, 3);
    run.release(4_500);
    assert!(!run.tracker.holds_pending_roster());
    assert!(!run.tracker.holds_frame());
    assert!(run.staged.is_empty(), "{:?}", run.staged);
    // The roster count starts again after a release.
    run.tick(6_000, Seen::Roster, 4);
    assert!(!run.tracker.holds_pending_roster());
    run.tick(10_000, VOTE, 5);
    assert_eq!(run.files(), [("game1", "mapvote_first.png", 5)]);
}

#[test]
fn a_lone_hero_select_with_no_game_does_not_take_the_slot() {
    // The select between rounds opens no folder: the slot waits for one.
    let mut run = Run::new();
    run.tick(0, Seen::Loading, 1);
    run.tick(3_000, SELECT, 2);
    assert!(run.tracker.holds_loading());
    assert!(run.shots.is_empty());
    run.tick(6_000, BAN, 3);
    assert_eq!(run.written_in("game1", LOADING_NAME), [1]);
}

impl Run {
    /// The frames saved as `name` in `folder`, in order.
    fn written_in(&self, folder: &str, name: &str) -> Vec<u32> {
        self.shots
            .iter()
            .filter(|(f, n, _)| f == folder && *n == name)
            .map(|(_, _, frame)| *frame)
            .collect()
    }
}

// The pending files on disk, through the real write ops.

fn frame(w: u32) -> Arc<image::DynamicImage> {
    Arc::new(image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        w,
        4,
        image::Rgb([40, 60, 200]),
    )))
}

fn stage_pending(w: u32) -> PregameOp<Arc<image::DynamicImage>> {
    PregameOp::Stage(PregameShot {
        folder: PENDING_DIR.to_string(),
        name: ROSTER_NAME,
        frame: frame(w),
    })
}

fn adopt(folder: &str) -> PregameOp<Arc<image::DynamicImage>> {
    PregameOp::Adopt {
        folder: folder.to_string(),
        name: ROSTER_NAME,
    }
}

fn pending_file(root: &std::path::Path) -> std::path::PathBuf {
    root.join(PENDING_DIR).join(staged_name(ROSTER_NAME))
}

#[test]
fn a_pending_roster_is_staged_apart_and_moved_into_the_game_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut made = Vec::new();
    write_pregame_ops(root, &[stage_pending(8), stage_pending(9)], 20, &mut made);
    assert!(pending_file(root).is_file());
    // The pending folder is no game folder: not in the made order.
    assert!(made.is_empty(), "{made:?}");
    let game = "20261102_120000";
    write_pregame_ops(root, &[adopt(game)], 20, &mut made);
    assert!(!pending_file(root).exists());
    let saved = image::open(root.join(game).join(ROSTER_NAME)).unwrap();
    assert_eq!(saved.width(), 9, "the latest staged frame");
    assert_eq!(made, [game]);
}

#[test]
fn an_adopt_with_nothing_staged_makes_no_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut made = Vec::new();
    write_pregame_ops(dir.path(), &[adopt("20261102_120000")], 20, &mut made);
    assert!(!dir.path().join("20261102_120000").exists());
    assert!(made.is_empty());
}

#[test]
fn a_dropped_pending_roster_leaves_no_file() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut made = Vec::new();
    let discard = PregameOp::Discard {
        folder: PENDING_DIR.to_string(),
        name: ROSTER_NAME,
    };
    write_pregame_ops(root, &[stage_pending(8), discard], 20, &mut made);
    assert!(!pending_file(root).exists());
    // A run that ended with one staged: the sweep at start deletes it, a
    // new game folder's sweep does not (the slot may still attach it).
    write_pregame_ops(root, &[stage_pending(8)], 20, &mut made);
    sweep_staged(root, Some("20261102_120000"));
    assert!(pending_file(root).is_file());
    sweep_staged(root, None);
    assert!(!pending_file(root).exists());
    assert!(!root.join(PENDING_DIR).exists(), "emptied, then removed");
}

#[test]
fn an_attached_pending_roster_counts_in_the_game_cap() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let keep = PREGAME_KEEP_GAMES;
    let mut made: Vec<String> = (0..keep).map(|i| format!("20260101_0000{i:02}")).collect();
    for name in &made {
        std::fs::create_dir_all(root.join(name)).unwrap();
        std::fs::write(root.join(name).join("mapvote_first.png"), b"x").unwrap();
    }
    let game = "20261102_120000";
    write_pregame_ops(root, &[stage_pending(8), adopt(game)], keep, &mut made);
    let mut games: Vec<String> = std::fs::read_dir(root)
        .unwrap()
        .flatten()
        .filter(|e| e.path().is_dir())
        .filter_map(|e| e.file_name().into_string().ok())
        .filter(|n| is_game_folder(n))
        .collect();
    games.sort();
    assert_eq!(games.len(), keep, "{games:?}");
    assert!(
        !games.contains(&"20260101_000000".to_string()),
        "oldest gone"
    );
    assert_eq!(games.last().map(String::as_str), Some(game));
    assert!(root.join(game).join(ROSTER_NAME).is_file());
}

#[test]
fn the_roster_is_checked_before_the_other_screens() {
    // A frame every check passes: the roster comes out, and with the
    // roster not wanted the select title does.
    let roster = roster_frame(1920, 1080);
    assert!(detect::roster_screen::is_roster_screen(&roster));
    let want = PregameWant {
        loading: true,
        other_frame: true,
        select_title: true,
        ban_title: true,
        roster: true,
        rejected: 0,
    };
    let unknown = detect::GamePhase::Unknown;
    let yes = |_: &image::RgbImage| true;
    assert_eq!(
        classify_with(want, &unknown, &roster, yes, yes),
        Seen::Roster
    );
    let no_roster = PregameWant {
        roster: false,
        ..want
    };
    assert_eq!(
        classify_with(no_roster, &unknown, &roster, yes, yes),
        Seen::SelectTitle
    );
    // A phase read still comes first.
    assert_eq!(
        classify_with(want, &detect::GamePhase::HeroSelect, &roster, yes, yes),
        SELECT
    );
}

/// A plain 5v5 roster layout: the two white header bars and the coloured
/// strips on the columns' inner edges under them, on a dim backdrop.
fn roster_frame(w: u32, h: u32) -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(w, h, image::Rgb([60, 70, 90]));
    let mut fill = |b: [f32; 4], c: [u8; 3]| {
        let (x0, y0) = ((b[0] * w as f32) as u32, (b[1] * h as f32) as u32);
        let (x1, y1) = ((b[2] * w as f32) as u32, (b[3] * h as f32) as u32);
        for y in y0..y1.min(h) {
            for x in x0..x1.min(w) {
                img.put_pixel(x, y, image::Rgb(c));
            }
        }
    };
    fill([0.0676, 0.236, 0.4059, 0.278], [247, 246, 246]);
    fill([0.5938, 0.236, 0.9324, 0.278], [247, 246, 246]);
    fill([0.3801, 0.284, 0.4059, 0.75], [107, 22, 145]);
    fill([0.5938, 0.284, 0.6195, 0.75], [255, 210, 4]);
    img
}

#[test]
fn the_roster_needs_two_samples_only_at_fast_polls() {
    let ms = Duration::from_millis;
    assert_eq!(roster_samples(ms(1_500)), 2, "the new reader");
    assert_eq!(roster_samples(ROSTER_ONE_SAMPLE_POLL - ms(1)), 2);
    assert_eq!(roster_samples(ROSTER_ONE_SAMPLE_POLL), 1, "the boundary");
    assert_eq!(roster_samples(ms(4_000)), 1, "ocr-v1");
    // Through the tracker: one sample is held at 3 s and 4 s polls, not
    // just under 3 s.
    for (poll, held) in [(2_999, false), (3_000, true), (4_000, true)] {
        let mut run = Run::polled(poll);
        run.tick(0, Seen::Roster, 1);
        assert_eq!(run.tracker.holds_pending_roster(), held, "{poll} ms");
        run.tick(poll, Seen::Roster, 2);
        assert!(run.tracker.holds_pending_roster(), "{poll} ms, two");
    }
    // In a game at 4 s polls: one roster sample is saved.
    let mut run = Run::polled(4_000);
    run.tick(0, VOTE, 1);
    run.tick(4_000, BAN, 2);
    run.tick(8_000, Seen::Roster, 3);
    run.tick(12_000, SELECT, 4);
    assert_eq!(run.written_in("game1", ROSTER_NAME), [3]);
}

/// Feed `rgb` to `tracker` at `now` the way the poll loop does (its want,
/// the real classify, the frame kept only when wanted); returns what the
/// tick classified and the ops it queued.
fn feed(
    tracker: &mut PregameTracker<Arc<u32>>,
    now: Instant,
    rgb: &image::RgbImage,
) -> (Seen, Vec<PregameOp<Arc<u32>>>) {
    let want = tracker.want(now);
    let no = |_: &image::RgbImage| false;
    let unknown = detect::GamePhase::Unknown;
    let checked = classify_checked_with(want, &unknown, rgb, no, |_| Default::default());
    let frame = keeps_checked(want, &checked).then(|| Arc::new(1));
    let seen = checked.seen;
    (seen, tracker.observe_checked(now, checked, frame, || "game1".into()))
}

fn staged_pending_roster(ops: &[PregameOp<Arc<u32>>]) -> bool {
    ops.iter().any(|op| {
        matches!(op, PregameOp::Stage(s) if s.folder == PENDING_DIR && s.name == ROSTER_NAME)
    })
}

fn board_game(read_at: Instant) -> crate::ActiveGame {
    let mut game = crate::ActiveGame::open_at(
        "s1".into(),
        detect::MatchOutcome::Unknown,
        Vec::new(),
        read_at,
    );
    game.session_created = true;
    game
}

#[test]
fn within_the_skip_window_after_a_board_read_no_check_runs() {
    // A match in progress: its scoreboard read at the start, then roster
    // and loading frames with no folder, all within MATCH_SKIP_IDLE.
    let start = Instant::now();
    let game = board_game(start);
    let mut tracker = PregameTracker::new(Duration::from_millis(1_500));
    let frames = [
        roster_frame(1920, 1080),
        roster_frame(1920, 1080),
        super::tests_loading_stand_ins::entering_game_slab(1280, 720),
    ];
    for (k, rgb) in frames.iter().enumerate() {
        let now = start + Duration::from_millis(1_500 * k as u64);
        tracker.set_match_read(match_board_read(Some(&game), now));
        assert_eq!(tracker.want(now), PregameWant::default(), "frame {k}");
        let (seen, ops) = feed(&mut tracker, now, rgb);
        assert_eq!(seen, Seen::Other, "frame {k}");
        assert!(ops.is_empty(), "frame {k}: {ops:?}");
    }
    // Just inside the window: still off.
    let edge = start + MATCH_SKIP_IDLE;
    tracker.set_match_read(match_board_read(Some(&game), edge));
    assert_eq!(tracker.want(edge), PregameWant::default());
    assert!(!tracker.holds_pending_roster() && !tracker.holds_frame());
}

#[test]
fn a_missed_result_holds_the_checks_off_only_for_the_window() {
    // Scoreboard reads and no result ever read: 3 minutes and more after
    // the last read, a roster with no folder is checked and staged.
    let start = Instant::now();
    let game = board_game(start);
    let mut tracker = PregameTracker::new(Duration::from_millis(1_500));
    let late = start + MATCH_SKIP_IDLE + Duration::from_secs(1);
    let mut staged = false;
    for k in 0..2u64 {
        let now = late + Duration::from_millis(1_500 * k);
        tracker.set_match_read(match_board_read(Some(&game), now));
        assert!(tracker.want(now).roster, "tick {k}");
        let (seen, ops) = feed(&mut tracker, now, &roster_frame(1920, 1080));
        assert_eq!(seen, Seen::Roster, "tick {k}");
        staged |= staged_pending_roster(&ops);
    }
    assert!(staged && tracker.holds_pending_roster());
}

#[test]
fn an_end_signal_or_a_phase_read_ends_the_skip_at_once() {
    let start = Instant::now();
    let read = Some(start);
    // An end signal (an outcome, end title or end-reel read) right after a
    // board read: the checks run again at once.
    let mut tracker: PregameTracker<Arc<u32>> = PregameTracker::new(Duration::from_millis(1_500));
    tracker.set_match_read(read);
    let t1 = start + Duration::from_secs(5);
    assert_eq!(tracker.want(t1), PregameWant::default());
    tracker.note_match_end(t1);
    assert!(tracker.want(t1).roster && tracker.want(t1).loading);
    // Until the next scoreboard read.
    let t2 = t1 + Duration::from_secs(5);
    tracker.set_match_read(Some(t2));
    assert_eq!(tracker.want(t2), PregameWant::default());
    // A pre-game phase read (a lone hero select, which opens no folder)
    // ends it too.
    let t3 = t2 + Duration::from_secs(5);
    let ops = tracker.observe(t3, Seen::Phase(PregamePhase::HeroSelect), Some(Arc::new(2)), || {
        "game1".into()
    });
    assert!(ops.is_empty(), "{ops:?}");
    let t4 = t3 + Duration::from_millis(1_500);
    let want = tracker.want(t4);
    assert!(want.roster && want.loading, "{want:?}");
    let (seen, _) = feed(&mut tracker, t4, &roster_frame(1920, 1080));
    assert_eq!(seen, Seen::Roster);
}

#[test]
fn a_board_read_counts_only_for_an_unfinished_match_in_the_window() {
    let now = Instant::now();
    assert_eq!(match_board_read(None, now), None);
    let mut game = crate::ActiveGame::open_at(
        "s1".into(),
        detect::MatchOutcome::Unknown,
        Vec::new(),
        now,
    );
    // Opened at its start screens, no scoreboard read yet: the pre-game.
    assert_eq!(match_board_read(Some(&game), now), None);
    game.session_created = true;
    assert_eq!(match_board_read(Some(&game), now), Some(now));
    assert_eq!(match_board_read(Some(&game), now + MATCH_SKIP_IDLE), Some(now));
    let past = now + MATCH_SKIP_IDLE + Duration::from_secs(1);
    assert_eq!(match_board_read(Some(&game), past), None);
    // The result is in: the match is over.
    game.record_outcome_at(detect::MatchOutcome::Victory, now);
    assert_eq!(match_board_read(Some(&game), now), None);
}

#[test]
fn entering_pregame_is_replaced_by_entering_game_before_the_start() {
    // A game shows two loading screens: ENTERING PREGAME before the vote
    // (frame 1 here) and ENTERING GAME before the roster (frame 3). The
    // first is saved at the vote; the second replaces it at the start, so
    // the folder's loading.png is the later frame, with a hero ban or a
    // hero select as the start.
    for start in [BAN, SELECT] {
        let mut run = Run::new();
        run.tick(0, Seen::Loading, 1);
        run.tick(3_000, VOTE, 2);
        run.tick(15_000, Seen::Other, 0);
        run.tick(16_500, Seen::Loading, 3);
        run.tick(18_000, start, 4);
        let loading = run.written_in("game1", LOADING_NAME);
        assert_eq!(loading, [1, 3], "{start:?}");
        assert_eq!(loading.last(), Some(&3), "the later frame is on disk");
        assert!(!run.tracker.holds_loading());
    }
}

/// An ENTERING PREGAME-like band, synthetic: on the navy loading band, two
/// words of solid letters with small letter gaps and a wider word gap,
/// the second word about 0.97 as wide as the first (8 and 7 letters),
/// and with `spinner` a blob 0.012 of the width wide, 0.011 after the
/// second word. The whole line is centred.
fn pregame_band(w: u32, h: u32, spinner: bool) -> image::RgbImage {
    use super::tests_loading_stand_ins::{GLYPH, navy_band};
    let mut img = navy_band(w, h);
    let px = |share: f32| (share * w as f32).round() as u32;
    let (letter_gap, word_gap) = (px(0.0018), px(0.0055));
    let (first, second) = (px(0.0645), px(0.0625));
    let (spin_gap, spin) = (px(0.011), px(0.012));
    let spin_part = if spinner { spin_gap + spin } else { 0 };
    let line = first + word_gap + second + spin_part;
    let (y0, y1) = (168 * h / 360, 192 * h / 360);
    let mut fill = |x0: u32, width: u32| {
        for y in y0..y1 {
            for x in x0..x0 + width {
                img.put_pixel(x, y, image::Rgb(GLYPH));
            }
        }
    };
    let mut x = (w - line) / 2;
    for (word, letters) in [(first, 8u32), (second, 7)] {
        let letter = (word - letter_gap * (letters - 1)) / letters;
        for k in 0..letters {
            fill(x + k * (letter + letter_gap), letter);
        }
        x += word + word_gap;
    }
    if spinner {
        fill(x - word_gap + spin_gap, spin);
    }
    img
}

#[test]
fn an_entering_pregame_like_band_is_turned_down_by_the_word_rule() {
    // Two words of near equal width, as ENTERING PREGAME before the map
    // vote: the word rule turns it down like APPLYING UPDATE, so it never
    // fills the loading slot. A spinner after the words is left out.
    use detect::loading_screen::{is_entering_game, is_loading_screen, text_box};
    let want = PregameWant {
        loading: true,
        other_frame: true,
        ..PregameWant::default()
    };
    let no = |_: &image::RgbImage| false;
    for (w, h) in [(1920, 1080), (2560, 1440)] {
        for spinner in [false, true] {
            let band = pregame_band(w, h, spinner);
            let at = format!("{w}x{h} spinner {spinner}");
            // The band itself is the loading slab; only the text fails.
            assert!(is_loading_screen(&band), "{at}");
            let t = text_box(&band).expect("text");
            assert_eq!(t.words, 2, "{at}: {t:?}");
            let ratio = t.word_ratio.expect("two words");
            assert!((ratio - 0.97).abs() < 0.03, "{at}: {t:?}");
            assert!(!is_entering_game(&band), "{at}: {t:?}");
            let seen = classify_with(want, &detect::GamePhase::Unknown, &band, no, no);
            assert_eq!(seen, Seen::Other, "{at}");
        }
    }
}
