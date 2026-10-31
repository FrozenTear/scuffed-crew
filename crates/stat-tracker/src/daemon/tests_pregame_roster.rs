//! Tests for the roster screen saves and the turned-down candidates
//! (`rejected_<label>.png` and their debug lines) of the pre-game saver,
//! next to `pregame_dump/roster.rs` and `pregame_dump/rejects.rs`.
//! Synthetic frames only; the real-frame checks are the opt-in
//! `SCUFFED_PREGAME_FRAMES_DIR` run in `tests/integration/pregame_frames.rs`.

use super::pregame_dump::*;
use super::tests_loading_stand_ins::*;
use super::tests_pregame_dump::names_in;
use crate::*;
use std::time::Duration;

const VOTE: Seen = Seen::Phase(PregamePhase::MapVote);
const BAN: Seen = Seen::Phase(PregamePhase::HeroBan);
const SELECT: Seen = Seen::Phase(PregamePhase::HeroSelect);
const POLL: Duration = Duration::from_millis(1_500);

/// Drives a tracker the way the poll loop does, with the checks a tick
/// turned down: a tick hands its frame only when [`keeps_checked`] says
/// so, and a check runs only when the tracker wants it. Saved files are in
/// `shots` (rejected ones too), staged ones in `staged` until promoted.
/// Every tick checks the tracker holds at most one frame.
struct Run {
    tracker: PregameTracker<Arc<u32>>,
    start: Instant,
    games: usize,
    made: Vec<Arc<u32>>,
    shots: Vec<(String, &'static str, u32)>,
    staged: Vec<(String, &'static str, u32)>,
}

impl Run {
    fn new() -> Self {
        Self {
            tracker: PregameTracker::new(POLL),
            start: Instant::now(),
            games: 0,
            made: Vec::new(),
            shots: Vec::new(),
            staged: Vec::new(),
        }
    }

    fn at(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }

    fn tick(&mut self, ms: u64, seen: Seen, frame: u32) {
        self.tick_checked(ms, seen.into(), frame);
    }

    fn tick_checked(&mut self, ms: u64, mut checked: Classified, frame: u32) {
        let now = self.at(ms);
        let want = self.tracker.want(now);
        checked.seen = match checked.seen {
            Seen::Loading if !want.loading => Seen::Other,
            Seen::SelectTitle if !want.select_title => Seen::Other,
            Seen::BanResults if !want.ban_title => Seen::Other,
            Seen::Roster if !want.roster => Seen::Other,
            seen => seen,
        };
        if !want.loading {
            checked.rejects.loading = None;
        }
        if !want.ban_title {
            checked.rejects.ban_results = None;
        }
        if !want.roster {
            checked.rejects.roster = None;
        }
        let frame = keeps_checked(want, &checked).then(|| Arc::new(frame));
        if let Some(f) = &frame {
            self.made.push(Arc::clone(f));
        }
        let games = &mut self.games;
        let ops = self.tracker.observe_checked(now, checked, frame, || {
            *games += 1;
            format!("game{games}")
        });
        self.record(ops);
    }

    fn release(&mut self, ms: u64) {
        let ops = self.tracker.release(self.at(ms));
        self.record(ops);
    }

    fn record(&mut self, ops: Vec<PregameOp<Arc<u32>>>) {
        for op in ops {
            match op {
                PregameOp::Save(s) => self.shots.push((s.folder, s.name, *s.frame)),
                PregameOp::Stage(s) => {
                    self.staged
                        .retain(|(f, n, _)| (f, *n) != (&s.folder, s.name));
                    self.staged.push((s.folder, s.name, *s.frame));
                }
                PregameOp::Promote { folder, name } => {
                    let at = self
                        .staged
                        .iter()
                        .position(|(f, n, _)| (f, *n) == (&folder, name))
                        .expect("a promote has a staged frame");
                    self.shots.push(self.staged.remove(at));
                }
                PregameOp::Discard { folder, name } => {
                    self.staged.retain(|(f, n, _)| (f, *n) != (&folder, name));
                }
            }
        }
        let held = self
            .made
            .iter()
            .filter(|f| Arc::strong_count(f) > 1)
            .count();
        assert!(held <= 1, "the tracker holds {held} frames");
    }

    /// Saved names and frames, rejected ones left out.
    fn names(&self) -> Vec<(&str, u32)> {
        self.shots
            .iter()
            .filter(|(_, n, _)| !n.starts_with("rejected_"))
            .map(|(_, n, f)| (*n, *f))
            .collect()
    }

    /// The frames written as `name`, in order.
    fn written(&self, name: &str) -> Vec<u32> {
        self.shots
            .iter()
            .filter(|(_, n, _)| *n == name)
            .map(|(_, _, f)| *f)
            .collect()
    }
}

/// A roster near miss: one part passed.
fn roster_miss() -> Classified {
    let scores = detect::roster_screen::RosterScores {
        map_logo: 0.2,
        ..Default::default()
    };
    assert!(scores.near_miss());
    Classified {
        seen: Seen::Other,
        rejects: Rejects {
            roster: Some(scores),
            ..Default::default()
        },
    }
}

/// A loading slab whose text is too dim for the glyph level (a fade-in).
fn loading_miss() -> Classified {
    Classified {
        seen: Seen::Other,
        rejects: Rejects {
            loading: Some(LoadingReject {
                text: None,
                ban_gate: false,
            }),
            ..Default::default()
        },
    }
}

#[test]
fn loading_ban_results_and_roster_are_separate_files() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(10_000, Seen::Loading, 2);
    run.tick(20_000, BAN, 3);
    run.tick(21_500, BAN, 4);
    run.tick(23_000, Seen::BanResults, 5);
    run.tick(24_500, Seen::Roster, 6);
    run.tick(26_000, Seen::Roster, 7);
    run.tick(27_500, Seen::Roster, 8);
    run.tick(29_000, SELECT, 9);
    assert_eq!(
        run.names(),
        vec![
            ("mapvote_first.png", 1),
            ("loading.png", 2),
            ("heroban_first.png", 3),
            ("heroban_last.png", 4),
            ("roster.png", 6),
            ("roster_last.png", 8),
            ("ban_results.png", 5),
            ("heroselect_first.png", 9),
        ]
    );
    assert!(run.staged.is_empty(), "{:?}", run.staged);
}

#[test]
fn a_game_with_no_loading_screen_still_saves_its_roster() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(20_000, BAN, 3);
    run.tick(23_000, Seen::Roster, 6);
    run.tick(24_500, SELECT, 9);
    let names: Vec<&str> = run.names().iter().map(|(n, _)| *n).collect();
    assert!(names.contains(&ROSTER_NAME), "{names:?}");
    assert!(!names.contains(&LOADING_NAME), "{names:?}");
    // One roster frame: no last frame.
    assert!(!names.contains(&ROSTER_LAST_NAME), "{names:?}");
}

#[test]
fn a_game_with_no_roster_still_saves_its_loading_screen() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(10_000, Seen::Loading, 2);
    run.tick(20_000, BAN, 3);
    run.tick(30_000, SELECT, 9);
    assert_eq!(run.written(LOADING_NAME), vec![2]);
    assert!(run.written(ROSTER_NAME).is_empty());
}

#[test]
fn the_roster_is_saved_once_per_game_and_not_looked_for_after_the_select() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(3_000, BAN, 2);
    run.tick(6_000, Seen::Roster, 3);
    // A flicker of the layout check inside ROSTER_GONE: the screen goes on.
    run.tick(7_500, Seen::Other, 0);
    run.tick(9_000, Seen::Roster, 4);
    run.tick(10_500, SELECT, 5);
    assert_eq!(run.written(ROSTER_NAME), vec![3]);
    assert_eq!(run.written(ROSTER_LAST_NAME), vec![4]);
    assert!(!run.tracker.want(run.at(12_000)).roster);
    run.tick(12_000, Seen::Roster, 6);
    assert_eq!(run.written(ROSTER_NAME), vec![3]);
}

#[test]
fn the_roster_ends_after_a_gap_with_no_roster_frame() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(3_000, BAN, 2);
    run.tick(6_000, Seen::Roster, 3);
    run.tick(7_500, Seen::Roster, 4);
    run.tick(9_000, Seen::Other, 0);
    assert!(
        run.written(ROSTER_LAST_NAME).is_empty(),
        "still within the gap"
    );
    run.tick(12_000, Seen::Other, 0);
    assert_eq!(run.written(ROSTER_LAST_NAME), vec![4]);
    assert!(!run.tracker.want(run.at(13_500)).roster);
}

#[test]
fn a_release_saves_the_staged_roster_frame() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(3_000, Seen::Roster, 3);
    run.tick(4_500, Seen::Roster, 4);
    run.release(5_000);
    assert_eq!(run.written(ROSTER_LAST_NAME), vec![4]);
    assert!(!run.tracker.holds_frame());
}

#[test]
fn no_roster_is_looked_for_outside_a_pre_game() {
    let run = Run::new();
    let want = run.tracker.want(run.at(0));
    assert!(!want.roster && want.rejected == 0);
}

#[test]
fn turned_down_frames_are_written_rate_limited_per_label() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    // A long stretch of near misses every poll for a minute.
    let mut ms = 1_500;
    let mut frame = 100;
    while ms <= 61_500 {
        run.tick_checked(ms, roster_miss(), frame);
        ms += 1_500;
        frame += 1;
    }
    let written = run.written(RejectLabel::Roster.file_name());
    let most = (60_000 / REJECTED_FRAME_EVERY.as_millis() as usize) + 1;
    assert!(
        !written.is_empty() && written.len() <= most,
        "{} writes: {written:?}",
        written.len()
    );
    // Each label is limited on its own: a loading miss now still writes.
    run.tick_checked(ms, loading_miss(), 999);
    assert_eq!(run.written(RejectLabel::Loading.file_name()), vec![999]);
    // Nothing turned down is saved under a real name.
    assert_eq!(run.names(), vec![("mapvote_first.png", 1)]);
}

#[test]
fn no_turned_down_frame_is_kept_outside_a_pre_game() {
    let mut run = Run::new();
    run.tick_checked(0, roster_miss(), 1);
    run.tick_checked(1_500, loading_miss(), 2);
    assert!(run.shots.is_empty(), "{:?}", run.shots);
}

#[test]
fn a_held_loading_frame_dropped_for_no_game_start_is_kept_as_rejected() {
    let mut run = Run::new();
    run.tick(0, VOTE, 1);
    run.tick(3_000, Seen::Loading, 2);
    // No hero ban or select within LOADING_START_WAIT.
    run.tick(
        3_000 + LOADING_START_WAIT.as_millis() as u64 + 1_500,
        Seen::Other,
        0,
    );
    assert!(!run.tracker.holds_loading());
    assert!(run.written(LOADING_NAME).is_empty());
    assert_eq!(run.written(RejectLabel::Loading.file_name()), vec![2]);
}

#[test]
fn a_dropped_ban_results_candidate_is_kept_as_rejected() {
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, Seen::BanResults, 2);
    // The ban comes back: the candidate was a frame inside it.
    run.tick(3_000, BAN, 3);
    assert!(run.written(BAN_RESULTS_NAME).is_empty());
    assert_eq!(run.written(RejectLabel::BanResults.file_name()), vec![2]);
}

#[test]
fn the_limiter_logs_changes_once_a_second_and_repeats_after_a_while() {
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let mut limiter = RejectLimiter::default();
    let label = RejectLabel::Roster;
    assert!(limiter.log_due(label, at(0), "a"));
    // Too soon, even with new numbers.
    assert!(!limiter.log_due(label, at(500), "b"));
    // New numbers a second on: logged.
    assert!(limiter.log_due(label, at(1_000), "b"));
    // The same numbers: only after REJECT_LOG_SAME.
    assert!(!limiter.log_due(label, at(5_000), "b"));
    let later = 1_000 + REJECT_LOG_SAME.as_millis() as u64;
    assert!(limiter.log_due(label, at(later), "b"));
    // Labels are limited apart.
    assert!(limiter.log_due(RejectLabel::Loading, at(later), "b"));
    // Frames: one per REJECTED_FRAME_EVERY.
    assert!(limiter.frame_due(label, at(0)));
    limiter.frame_written(label, at(0));
    assert!(!limiter.frame_due(label, at(9_999)));
    assert!(limiter.frame_due(label, at(REJECTED_FRAME_EVERY.as_millis() as u64)));
}

#[test]
fn the_debug_line_carries_the_loading_rule_numbers() {
    let t = detect::loading_screen::text_box(&two_word_band(1280, 720, (0.19, 0.74), Some(false)))
        .expect("a text box");
    let rejects = Rejects {
        loading: Some(LoadingReject {
            text: Some(t),
            ban_gate: false,
        }),
        ..Default::default()
    };
    let details = rejects.details();
    let (label, detail) = &details[0];
    assert_eq!(*label, RejectLabel::Loading);
    for part in ["A samples", "B width", "C word ratio", "spinner"] {
        assert!(detail.contains(part), "{detail}");
    }
}

#[test]
fn classify_reports_turned_down_loading_and_roster_checks() {
    let unknown = detect::GamePhase::Unknown;
    let want = PregameWant {
        loading: true,
        other_frame: true,
        select_title: false,
        ban_title: false,
        roster: true,
        rejected: 0,
    };
    let no_title = |_: &image::RgbImage| false;
    let no_ban = |_: &image::RgbImage| super::pregame_ban_title::BanCheck::default();
    // A navy slab with no text yet: a loading candidate turned down by A.
    let checked = classify_checked_with(want, &unknown, &navy_band(1280, 720), no_title, no_ban);
    assert_eq!(checked.seen, Seen::Other);
    assert_eq!(
        checked.rejects.loading,
        Some(LoadingReject {
            text: None,
            ban_gate: false
        })
    );
    // The APPLYING UPDATE band: turned down by C, with its numbers.
    let update = two_word_band(1280, 720, (0.19, 0.74), Some(false));
    let checked = classify_checked_with(want, &unknown, &update, no_title, no_ban);
    assert_eq!(checked.seen, Seen::Other);
    let ratio = checked
        .rejects
        .loading
        .and_then(|l| l.text)
        .and_then(|t| t.word_ratio);
    assert!(ratio.is_some_and(|r| r >= detect::loading_screen::WORD_RATIO_REJECT));
    // A plain frame: no candidate of any label.
    let plain = image::RgbImage::from_pixel(1280, 720, image::Rgb([90, 90, 90]));
    let checked = classify_checked_with(want, &unknown, &plain, no_title, no_ban);
    assert_eq!(checked, Seen::Other.into());
    // Not wanted: not checked.
    let off = PregameWant {
        loading: false,
        roster: false,
        ..want
    };
    assert_eq!(
        classify_checked_with(off, &unknown, &navy_band(1280, 720), no_title, no_ban),
        Seen::Other.into()
    );
}

/// A small file at `dir/name`.
fn put(dir: &std::path::Path, name: &str) {
    std::fs::create_dir_all(dir).unwrap();
    std::fs::write(dir.join(name), b"png").unwrap();
}

#[test]
fn rejected_files_count_toward_the_twenty_games_and_go_with_their_folder() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    // 21 old games holding only a rejected frame each.
    for i in 0..21 {
        put(
            &root.join(format!("20260101_0000{i:02}")),
            RejectLabel::Roster.file_name(),
        );
    }
    let shot = PregameShot {
        folder: "20261030_120000".to_string(),
        name: RejectLabel::Loading.file_name(),
        frame: Arc::new(image::DynamicImage::ImageRgb8(image::RgbImage::new(64, 36))),
    };
    write_pregame_ops(
        &root,
        &[PregameOp::Save(shot)],
        PREGAME_KEEP_GAMES,
        &mut Vec::new(),
    );
    let left = names_in(&root);
    assert_eq!(left.len(), PREGAME_KEEP_GAMES, "{left:?}");
    for i in 0..2 {
        assert!(!left.contains(&format!("20260101_0000{i:02}")), "{left:?}");
    }
    assert_eq!(
        names_in(&root.join("20261030_120000")),
        vec![RejectLabel::Loading.file_name().to_string()]
    );
    // The sweep of staged leftovers keeps them: they are saved files.
    sweep_staged(&root, None);
    assert_eq!(
        names_in(&root.join("20260101_000002")),
        vec![RejectLabel::Roster.file_name().to_string()]
    );
}

#[test]
fn a_rejected_frame_is_replaced_by_a_newer_one_of_its_label() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    let save = |w: u32| {
        PregameOp::Save(PregameShot {
            folder: "20261030_120000".to_string(),
            name: RejectLabel::Roster.file_name(),
            frame: Arc::new(image::DynamicImage::ImageRgb8(image::RgbImage::new(w, 36))),
        })
    };
    let mut made = Vec::new();
    write_pregame_ops(&root, &[save(64)], PREGAME_KEEP_GAMES, &mut made);
    write_pregame_ops(&root, &[save(32)], PREGAME_KEEP_GAMES, &mut made);
    let path = root
        .join("20261030_120000")
        .join(RejectLabel::Roster.file_name());
    assert_eq!(image::image_dimensions(&path).unwrap(), (32, 36));
    assert_eq!(names_in(&root.join("20261030_120000")).len(), 1);
}
