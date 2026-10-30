//! Tests for the opt-in post-game screen saves, next to `postgame_dump.rs`.
//! Every frame is a synthetic stand-in made here: no real screenshots.

use super::postgame_dump::*;
use super::pregame_dump::*;
pub(super) use super::tests_pregame_dump::names_in;
use crate::*;
use std::time::Duration;

/// The new reader's poll interval.
pub(super) const POLL: Duration = Duration::from_millis(1_500);

/// A 160 by 90 frame of grey `level` with a bright band `band` (fractions
/// of the height) across the middle third: a VICTORY or DEFEAT style
/// banner when `band` is set, plain otherwise.
pub(super) fn picture(level: u8, band: Option<(f32, f32)>) -> image::RgbImage {
    let mut img = image::RgbImage::from_pixel(160, 90, image::Rgb([level; 3]));
    if let Some((y0, y1)) = band {
        for y in (y0 * 90.0) as u32..(y1 * 90.0) as u32 {
            for x in 40..120 {
                img.put_pixel(x, y, image::Rgb([240, 230, 200]));
            }
        }
    }
    img
}

pub(super) fn shown(img: image::RgbImage) -> Frame {
    Arc::new(image::DynamicImage::ImageRgb8(img))
}

/// The banner over a dark ground.
pub(super) fn banner() -> Frame {
    shown(picture(20, Some((0.4, 0.6))))
}

/// Gameplay stand-in number `n`: each a clearly different grey.
pub(super) fn play(n: u8) -> Frame {
    shown(picture(60 + n.wrapping_mul(37) % 150, None))
}

/// The same picture as `img` with a one-step change in a few pixels: a
/// near duplicate.
pub(super) fn noisy(img: &image::RgbImage, seed: u32) -> Frame {
    let mut out = img.clone();
    for i in 0..40u32 {
        let (x, y) = ((i * 7 + seed) % 160, (i * 13 + seed) % 90);
        let p = out.get_pixel_mut(x, y);
        p.0[0] = p.0[0].saturating_add(1);
    }
    shown(out)
}

/// A frame that changes every call: a band 7 rows further down per `n`.
pub(super) fn moving(n: u32) -> Frame {
    let mut img = picture(30, None);
    let y0 = (n * 7) % 80;
    for y in y0..y0 + 10 {
        for x in 0..160 {
            img.put_pixel(x, y, image::Rgb([200, 200, 255]));
        }
    }
    shown(img)
}

#[derive(Default)]
pub(super) struct Rec {
    pub(super) opened: Vec<f64>,
    pub(super) shots: Vec<(f64, &'static str, Frame)>,
    pub(super) ended: Vec<PostEnd>,
    /// Refuse every shot (a full writer queue).
    pub(super) refuse: bool,
}

impl PostSink for Rec {
    fn opened(&mut self, _now: Instant, confirmed_s: f64, _game: Option<Instant>) {
        self.opened.push(confirmed_s);
    }
    fn shot(&mut self, shot: PostShot) -> bool {
        if self.refuse {
            return false;
        }
        self.shots.push((shot.at_s, shot.label, shot.frame));
        true
    }
    fn relabel_last(&mut self, label: &'static str) {
        if let Some(last) = self.shots.last_mut() {
            last.1 = label;
        }
    }
    fn ended(&mut self, why: PostEnd) {
        self.ended.push(why);
    }
}

impl Rec {
    pub(super) fn times(&self) -> Vec<f64> {
        self.shots
            .iter()
            .map(|(t, _, _)| (t * 10.0).round() / 10.0)
            .collect()
    }
    pub(super) fn labels(&self) -> Vec<&'static str> {
        self.shots.iter().map(|(_, l, _)| *l).collect()
    }
}

/// Drives a tracker the way the poll loop does: a tick hands its frame
/// only when [`PostgameTracker::want`] asks for it.
pub(super) struct Run {
    pub(super) tracker: PostgameTracker,
    pub(super) start: Instant,
    pub(super) rec: Rec,
}

pub(super) const RUNNING: PostTick = PostTick {
    end: None,
    confirmed: false,
    running: true,
    finished: false,
    new_screen: false,
    game_opened: None,
};

/// After the outcome is recorded: the game is open and finished.
pub(super) const AFTER: PostTick = PostTick {
    end: None,
    confirmed: false,
    running: false,
    finished: true,
    new_screen: false,
    game_opened: None,
};

impl Run {
    pub(super) fn new() -> Self {
        Self {
            tracker: PostgameTracker::new(POLL),
            start: Instant::now(),
            rec: Rec::default(),
        }
    }

    pub(super) fn at(&self, ms: u64) -> Instant {
        self.start + Duration::from_millis(ms)
    }

    pub(super) fn tick(&mut self, ms: u64, tick: PostTick, frame: Frame) {
        let now = self.at(ms);
        let frame = self
            .tracker
            .want(now)
            .keeps(tick.end.is_some())
            .then_some(frame);
        self.tracker.observe(now, tick, frame, &mut self.rec);
    }

    /// A game running from 0 to `ms`, one gameplay frame a poll.
    pub(super) fn play_until(&mut self, ms: u64) {
        let mut t = 0;
        let mut n = 0;
        while t < ms {
            self.tick(t, RUNNING, play(n));
            t += 1_500;
            n += 1;
        }
    }
}

pub(super) fn ended(tick: PostTick, end: EndSeen, confirmed: bool) -> PostTick {
    PostTick {
        end: Some(end),
        confirmed,
        ..tick
    }
}

#[test]
fn the_first_banner_frame_is_saved_from_the_ring_on_a_late_confirmation() {
    let mut run = Run::new();
    run.play_until(60_000);
    assert!(run.rec.shots.is_empty(), "nothing is saved mid-match");
    // The banner's first frame reads no outcome yet (the stability wait),
    // the next one reads it, the third confirms it.
    run.tick(60_000, RUNNING, banner());
    run.tick(
        61_500,
        ended(RUNNING, EndSeen::Title, false),
        noisy(&picture(20, Some((0.4, 0.6))), 1),
    );
    run.tick(
        63_000,
        ended(AFTER, EndSeen::Title, true),
        noisy(&picture(20, Some((0.4, 0.6))), 2),
    );
    assert!(run.tracker.window_open());
    assert_eq!(run.rec.opened, vec![1.5]);
    // Gameplay before it, the banner's first frame, and nothing more: the
    // banner's later frames are near duplicates of the first.
    let labels = run.rec.labels();
    assert!(labels.contains(&"before_end"));
    let banner_at = run
        .rec
        .shots
        .iter()
        .position(|(_, _, f)| Thumb::of(f) == Thumb::of(&banner()))
        .expect("the banner's first frame is saved");
    assert_eq!(run.rec.times()[banner_at], -1.5);
    assert_eq!(run.rec.shots.len(), banner_at + 1, "{:?}", run.rec.times());
    // The whole ring (5 frames, 6 s back) is young enough: two gameplay
    // frames, then the banner's first.
    assert_eq!(run.rec.times(), vec![-4.5, -3.0, -1.5]);
    assert_eq!(
        run.rec.labels(),
        vec!["before_end", "before_end", "before_end"]
    );
}

#[test]
fn time_zero_is_the_first_frame_read_as_an_end() {
    let mut run = Run::new();
    run.play_until(10_000);
    run.tick(10_500, ended(RUNNING, EndSeen::Reel, false), banner());
    run.tick(12_000, ended(AFTER, EndSeen::Title, true), moving(3));
    assert_eq!(run.rec.opened, vec![1.5]);
    let (t, label, _) = run
        .rec
        .shots
        .iter()
        .find(|(_, l, _)| *l == "end_reel")
        .expect("the reel frame");
    assert_eq!((*t, *label), (0.0, "end_reel"));
    assert_eq!(run.rec.labels().last(), Some(&"end_title"));
}

#[test]
fn the_ring_is_bounded_and_held_only_while_a_game_runs() {
    let mut run = Run::new();
    assert_eq!(
        run.tracker.want(run.at(0)),
        PostWant {
            every: false,
            on_end: true
        }
    );
    let mut made = Vec::new();
    for n in 0..40u64 {
        let f = play(n as u8);
        made.push(Arc::clone(&f));
        run.tick(n * 1_500, RUNNING, f);
        assert!(run.tracker.ring_len() <= POSTGAME_RING);
        let held = made.iter().filter(|f| Arc::strong_count(f) > 1).count();
        assert!(held <= POSTGAME_RING, "the ring holds {held} frames");
    }
    // 1.5 s polls: 5 s plus one poll back is 5 frames.
    assert_eq!(run.tracker.ring_len(), 5);
    assert_eq!(run.tracker.ring_cap(), 5);
    // 4 s polls: 2 back plus the newest; a fast poll still holds 5 at most.
    assert_eq!(PostgameTracker::new(Duration::from_secs(4)).ring_cap(), 3);
    assert_eq!(
        PostgameTracker::new(Duration::from_millis(200)).ring_cap(),
        POSTGAME_RING
    );
    // The game ends with no outcome (closed by the quiet timer): the ring
    // is dropped, nothing saved.
    run.tick(70_000, PostTick::default(), play(1));
    assert_eq!(run.tracker.ring_len(), 0);
    assert_eq!(made.iter().filter(|f| Arc::strong_count(f) > 1).count(), 0);
    assert!(run.rec.shots.is_empty());
}

#[test]
fn a_slower_poll_rings_fewer_frames() {
    let mut run = Run {
        tracker: PostgameTracker::new(Duration::from_secs(4)),
        start: Instant::now(),
        rec: Rec::default(),
    };
    for n in 0..10u64 {
        run.tick(n * 4_000, RUNNING, play(n as u8));
    }
    // 5 s at 4 s polls is 2 polls back, plus the newest.
    assert_eq!(run.tracker.ring_len(), 3);
}

/// Runs a window the way the poll loop does: a tick every `poll`, each
/// tick's work taking `work`, brought forward by [`sooner_tick`] (from the
/// end of the work, as `reset_after` does), every frame different. The
/// saved frames' times from time zero.
pub(super) fn sampled_at(
    tracker: PostgameTracker,
    poll: Duration,
    work: Duration,
) -> (Vec<f64>, Vec<PostEnd>) {
    let mut run = Run {
        tracker,
        start: Instant::now(),
        rec: Rec::default(),
    };
    run.tick(0, RUNNING, play(0));
    let mut t = poll.as_millis() as u64;
    run.tick(t, ended(AFTER, EndSeen::Title, true), moving(0));
    let mut n = 1;
    while run.tracker.window_open() && t < 200_000 {
        let done = run.at(t) + work;
        t = match run
            .tracker
            .next_due_in(done)
            .and_then(|due| sooner_tick(due, work, poll))
        {
            Some(next) => t + (work + next).as_millis() as u64,
            None => t + poll.as_millis() as u64,
        };
        run.tick(t, AFTER, moving(n));
        n += 1;
    }
    // After the window, no frames are wanted and no tick is brought forward.
    assert_eq!(run.tracker.want(run.at(t)), PostWant::default());
    assert_eq!(run.tracker.next_due_in(run.at(t)), None);
    (run.rec.times(), run.rec.ended)
}

#[test]
fn sampling_is_fast_then_slower_until_the_window_ends() {
    // The new reader's 1.5 s polls, the classic 4 s ones, and a 500 ms
    // override, with no work and with 300 ms of grab and analysis a tick:
    // 1 s apart for 30 s, then slower. A tick is brought forward only after
    // one that handed over a frame, so at 1.5 s polls the slow samples are
    // every second poll (3 s); at 4 s polls the slow step is brought
    // forward to 2.5 s, from the last fast sample on (no frameless tick at
    // 30 s, no 5 s gap).
    for work in [Duration::ZERO, Duration::from_millis(300)] {
        for (poll, first_slow, slow_gap, count) in [
            (POLL, 32.0, 3.0, 50),
            (Duration::from_secs(4), 31.5, 2.5, 54),
            (Duration::from_millis(500), 31.5, 2.5, 54),
        ] {
            let (times, ended) = sampled_at(PostgameTracker::new(poll), poll, work);
            let at = format!("{poll:?} {work:?}: {times:?}");
            assert_eq!(ended, vec![PostEnd::Time], "{at}");
            assert_eq!(times.len(), count, "{at}");
            assert!(count < POSTGAME_MAX_FRAMES, "the cap is never what ends it");
            assert_eq!(times[0], 0.0, "time zero is the confirming frame");
            let gaps: Vec<f64> = times
                .windows(2)
                .map(|w| ((w[1] - w[0]) * 10.0).round() / 10.0)
                .collect();
            for (i, gap) in gaps.iter().enumerate() {
                if times[i + 1] < 30.0 {
                    // No drift: the work time is not added to each step.
                    assert_eq!(*gap, 1.0, "{at}: fast at {}", times[i + 1]);
                } else if times[i] >= 30.0 {
                    assert_eq!(*gap, slow_gap, "{at}: slow at {}", times[i + 1]);
                }
            }
            assert_eq!((times[29], times[30]), (29.0, first_slow), "{at}");
            assert!(*times.last().unwrap() < 90.0, "{at}");
        }
    }
}

#[test]
fn the_cap_ends_the_window() {
    let (times, ended) = sampled_at(PostgameTracker::capped(POLL, 12), POLL, Duration::ZERO);
    assert_eq!(ended, vec![PostEnd::Cap]);
    assert_eq!(times.len(), 12);
    assert_eq!(postgame_name(POSTGAME_MAX_FRAMES - 1), "postgame_60.png");
}

#[test]
fn a_sooner_tick_counts_from_the_tick_not_from_the_end_of_its_work() {
    let poll = POLL;
    // Due in 1 s, the tick's work took 300 ms: the regular tick is 1.2 s
    // away, so the next one comes after 1 s.
    let work = Duration::from_millis(300);
    assert_eq!(sooner_tick(POSTGAME_FAST, work, poll), Some(POSTGAME_FAST));
    // Work that took 700 ms leaves the regular tick (800 ms) sooner.
    assert_eq!(
        sooner_tick(POSTGAME_FAST, Duration::from_millis(700), poll),
        None
    );
    // The slow phase at 1.5 s polls: the regular tick, then 1 s on.
    assert_eq!(sooner_tick(POSTGAME_SLOW, Duration::ZERO, poll), None);
    assert_eq!(
        sooner_tick(Duration::from_secs(1), Duration::ZERO, poll),
        Some(Duration::from_secs(1))
    );
    // Never later than the regular tick.
    assert_eq!(sooner_tick(poll, Duration::ZERO, poll), None);
    // Never at once: an overdue sample still waits the slack.
    assert_eq!(
        sooner_tick(Duration::ZERO, Duration::ZERO, poll),
        Some(POSTGAME_SLACK)
    );
    assert_eq!(
        sooner_tick(Duration::from_millis(1), work, poll),
        Some(POSTGAME_SLACK)
    );
}

#[test]
fn with_no_game_running_a_lone_end_read_is_held_for_the_confirmation() {
    // Started mid-match, or the session did not open: no game runs.
    let mut run = Run::new();
    let idle = PostTick::default();
    run.tick(0, idle, play(0));
    assert_eq!(run.tracker.ring_len(), 0, "no frame without an end read");
    run.tick(1_500, ended(idle, EndSeen::Title, false), banner());
    assert_eq!(run.tracker.ring_len(), 1);
    run.tick(3_000, ended(idle, EndSeen::Title, true), moving(1));
    assert_eq!(run.rec.opened, vec![1.5]);
    assert_eq!(run.rec.labels(), vec!["end_title", "end_title"]);
    assert_eq!(run.rec.times(), vec![0.0, 1.5]);

    // A lone read that no confirmation follows is dropped, and a finished
    // older game does not open a window for it.
    let mut run = Run::new();
    run.tick(0, ended(AFTER, EndSeen::Rank, false), banner());
    assert_eq!(run.tracker.ring_len(), 1);
    run.tick(1_500, AFTER, play(1));
    assert_eq!(run.tracker.ring_len(), 0);
    assert!(run.rec.opened.is_empty());
}

#[test]
fn near_duplicates_are_dropped_and_changes_kept_until_the_window_ends() {
    let mut run = Run::new();
    run.tick(0, RUNNING, play(0));
    let summary = picture(70, Some((0.1, 0.3)));
    run.tick(1_500, ended(AFTER, EndSeen::Title, true), banner());
    // A static summary for 40 s, with a pixel or two of noise each frame.
    let mut t = 2_500;
    while t < 42_000 {
        run.tick(
            t,
            ended(AFTER, EndSeen::Accolades, false),
            noisy(&summary, t as u32),
        );
        t += 1_000;
    }
    // Then the rank update, static until the window ends.
    let rank = picture(110, Some((0.5, 0.8)));
    while t < 100_000 {
        run.tick(
            t,
            ended(AFTER, EndSeen::Rank, false),
            noisy(&rank, t as u32),
        );
        t += 1_500;
    }
    let mut labels = run.rec.labels();
    labels.dedup();
    assert_eq!(labels, vec!["end_title", "accolades", "rank_update"]);
    // A still screen is saved again only once the last save is 5 s old,
    // until 60 s; the near duplicates in between are dropped.
    let times = run.rec.times();
    let labels = run.rec.labels();
    for i in 1..times.len() {
        if times[i] < 60.0 && labels[i] == labels[i - 1] {
            assert!(times[i] - times[i - 1] >= 5.0, "{times:?}");
        }
    }
    assert!(
        times.iter().filter(|t| **t < 60.0).count() <= 13,
        "{times:?}"
    );
    assert_eq!(run.rec.ended, vec![PostEnd::Time]);
    // The window ended at 90 s after time zero.
    assert!(!run.tracker.window_open());
}

#[test]
fn thumbnails_tell_a_near_duplicate_from_a_change() {
    let base = picture(70, Some((0.1, 0.3)));
    let a = Thumb::of(&image::DynamicImage::ImageRgb8(base.clone()));
    let b = Thumb::of(&noisy(&base, 5));
    assert!(a.diff(&b) <= THUMB_SAME);
    let c = Thumb::of(&shown(picture(70, Some((0.5, 0.7)))));
    assert!(a.diff(&c) > THUMB_SAME);
    // A frame of any size and type works: an empty one too.
    let rgba = image::DynamicImage::ImageRgba8(image::RgbaImage::new(2560, 1440));
    assert_eq!(
        Thumb::of(&rgba).diff(&Thumb::of(&shown(picture(0, None)))),
        0.0
    );
    let _ = Thumb::of(&image::DynamicImage::new_rgb8(0, 0));
}

#[test]
fn a_new_game_screen_ends_the_window_and_rearms() {
    let mut run = Run::new();
    run.tick(0, RUNNING, play(0));
    run.tick(1_500, ended(AFTER, EndSeen::Title, true), banner());
    run.tick(3_000, AFTER, moving(1));
    let saved = run.rec.shots.len();
    let vote = PostTick {
        new_screen: true,
        ..PostTick::default()
    };
    run.tick(4_000, vote, moving(2));
    assert_eq!(run.rec.ended, vec![PostEnd::NewScreen]);
    assert_eq!(run.rec.shots.len(), saved, "the vote frame is not saved");
    // The next game's ticks arm the ring again (the first tick arms it,
    // the next one fills it), and its end opens another window.
    run.tick(5_500, RUNNING, play(3));
    run.tick(7_000, RUNNING, play(4));
    assert_eq!(run.tracker.ring_len(), 1);
    run.tick(8_500, ended(AFTER, EndSeen::Title, true), banner());
    assert_eq!(run.rec.opened.len(), 2);
}

#[test]
fn after_a_window_a_repeat_confirmation_opens_none_until_a_rearm() {
    let mut run = Run::new();
    run.tick(
        0,
        ended(PostTick::default(), EndSeen::Title, true),
        banner(),
    );
    // No game open (started mid-match): the confirming frame alone.
    assert_eq!(run.rec.labels(), vec!["end_title"]);
    let mut t = 1_000;
    while t <= 91_000 {
        run.tick(t, PostTick::default(), banner());
        t += 1_500;
    }
    assert_eq!(run.rec.ended, vec![PostEnd::Time]);
    run.tick(
        92_000,
        ended(PostTick::default(), EndSeen::Rank, true),
        moving(1),
    );
    assert_eq!(run.rec.opened.len(), 1);
    // A start screen rearms; so does the rearm time.
    run.tick(
        93_000 + POSTGAME_REARM.as_millis() as u64,
        ended(PostTick::default(), EndSeen::Rank, true),
        moving(2),
    );
    assert_eq!(run.rec.opened.len(), 2);
}

#[test]
fn release_drops_the_ring_and_ends_a_window() {
    let mut run = Run::new();
    run.tick(0, RUNNING, play(0));
    run.tick(1_500, RUNNING, play(1));
    let now = run.at(2_000);
    run.tracker.release(now, PostEnd::GameClosed, &mut run.rec);
    assert_eq!(run.tracker.ring_len(), 0);
    assert!(run.rec.ended.is_empty());
    run.tick(3_000, RUNNING, play(2));
    run.tick(4_500, ended(AFTER, EndSeen::Title, true), banner());
    let now = run.at(5_000);
    run.tracker.release(now, PostEnd::GameClosed, &mut run.rec);
    assert_eq!(run.rec.ended, vec![PostEnd::GameClosed]);
    assert!(!run.tracker.window_open());
}

#[test]
fn a_refused_frame_is_not_counted_and_the_next_change_is_tried() {
    let mut run = Run::new();
    run.rec.refuse = true;
    run.tick(
        0,
        ended(PostTick::default(), EndSeen::Title, true),
        banner(),
    );
    run.tick(
        1_500,
        PostTick::default(),
        noisy(&picture(20, Some((0.4, 0.6))), 3),
    );
    run.rec.refuse = false;
    // The writer had no room for the banner; its near duplicate is still
    // new to the saves.
    run.tick(
        3_000,
        PostTick::default(),
        noisy(&picture(20, Some((0.4, 0.6))), 4),
    );
    assert_eq!(run.rec.shots.len(), 1);
}

#[test]
fn the_folder_is_the_games_pre_game_folder_when_recent_and_unused() {
    let now = Instant::now();
    let later = now + Duration::from_secs(20 * 60);
    let new = || "new".to_string();
    assert_eq!(postgame_folder(Some(("g1", now)), &[], later, new), "g1");
    assert_eq!(
        postgame_folder(Some(("g1", now)), &["g1".into()], later, new),
        "new"
    );
    // Game A wrote into its folder, game B (no pre-game read) got a new
    // one, and game C (no pre-game read either) must not go back to A's.
    let used = ["g1".to_string(), "20261030_121500".to_string()];
    assert_eq!(postgame_folder(Some(("g1", now)), &used, later, new), "new");
    let too_late = now + POSTGAME_FOLDER_GAP + Duration::from_secs(1);
    assert_eq!(
        postgame_folder(Some(("g1", now)), &[], too_late, new),
        "new"
    );
    assert_eq!(postgame_folder(None, &[], later, new), "new");
}

#[test]
fn names_run_from_01_to_the_cap() {
    assert_eq!(postgame_name(0), "postgame_01.png");
    assert_eq!(postgame_name(59), "postgame_60.png");
    assert_eq!(POSTGAME_MAX_FRAMES, 60);
}

#[test]
fn the_sidecar_lists_each_written_frame_with_its_time_and_label() {
    let zero = chrono::DateTime::parse_from_rfc3339("2026-10-30T12:00:01.250Z")
        .unwrap()
        .with_timezone(&Utc);
    let doc = PostSidecar {
        confirmed_s: 1.5,
        zero_utc: Some(zero),
        frames: vec![
            PostEntry {
                file: "postgame_01.png",
                t_s: -1.5,
                label: "before_end",
            },
            PostEntry {
                file: "postgame_02.png",
                t_s: 0.0,
                label: "end_title",
            },
            PostEntry {
                file: "postgame_03.png",
                t_s: 12.004,
                label: "rank_update",
            },
        ],
        ended: Some(PostEnd::Time),
    };
    let json: serde_json::Value = serde_json::from_str(&doc.render(|_| true)).unwrap();
    assert_eq!(json["version"], 1);
    assert_eq!(json["confirmed_s"], 1.5);
    assert_eq!(json["time_zero_utc"], "2026-10-30T12:00:01.250Z");
    assert_eq!(json["ended"], "window_time");
    assert_eq!(json["frames"][0]["file"], "postgame_01.png");
    assert_eq!(json["frames"][0]["t_s"], -1.5);
    assert_eq!(json["frames"][2]["t_s"], 12.0);
    assert_eq!(json["frames"][2]["label"], "rank_update");
    // A frame whose write failed is not listed.
    let json: serde_json::Value =
        serde_json::from_str(&doc.render(|f| f != "postgame_02.png")).unwrap();
    let files: Vec<&str> = json["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["file"].as_str().unwrap())
        .collect();
    assert_eq!(files, ["postgame_01.png", "postgame_03.png"]);
    let open = PostSidecar {
        ended: None,
        zero_utc: None,
        ..doc
    };
    let json: serde_json::Value = serde_json::from_str(&open.render(|_| true)).unwrap();
    assert!(json["ended"].is_null());
    assert!(json["time_zero_utc"].is_null());
}

#[tokio::test]
async fn the_saver_writes_the_frames_and_sidecar_into_the_games_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).expect("on");
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    // A pre-game makes the game's folder.
    dump.observe(at(0), Seen::Phase(PregamePhase::MapVote), Some(play(9)));
    let want = dump.post_want(at(1_500));
    assert_eq!(
        want,
        PostWant {
            every: false,
            on_end: true
        }
    );
    // The game opened at its map vote: the folder is its own.
    let running = PostTick {
        running: true,
        game_opened: Some(at(0)),
        ..PostTick::default()
    };
    dump.observe_post(at(1_500), running, None);
    dump.post_want(at(3_000));
    dump.observe_post(at(3_000), running, Some(play(1)));
    let read = PostTick {
        end: Some(EndSeen::Title),
        ..running
    };
    dump.post_want(at(4_500));
    dump.observe_post(at(4_500), read, Some(banner()));
    let confirm = PostTick {
        end: Some(EndSeen::Title),
        confirmed: true,
        finished: true,
        game_opened: Some(at(0)),
        ..PostTick::default()
    };
    dump.post_want(at(6_000));
    dump.observe_post(
        at(6_000),
        confirm,
        Some(noisy(&picture(20, Some((0.4, 0.6))), 1)),
    );
    assert_eq!(dump.post_sooner_tick(at(6_000), POLL), Some(POSTGAME_FAST));
    // A failed capture releases the pre-game saves only: the window goes on.
    dump.release(at(6_500));
    dump.post_want(at(7_000));
    dump.observe_post(
        at(7_000),
        PostTick {
            finished: true,
            game_opened: Some(at(0)),
            ..PostTick::default()
        },
        Some(moving(5)),
    );
    assert!(dump.finish(Duration::from_secs(30)).await);

    let root = pregame_dir(dir.path());
    let folders = names_in(&root);
    assert_eq!(folders.len(), 1, "{folders:?}");
    let game = root.join(&folders[0]);
    assert_eq!(
        names_in(&game),
        vec![
            "mapvote_first.png",
            "postgame.json",
            "postgame_01.png",
            "postgame_02.png",
            "postgame_03.png"
        ]
    );
    let saved = image::open(game.join("postgame_02.png")).unwrap();
    assert_eq!(
        Thumb::of(&saved),
        Thumb::of(&banner()),
        "full frame, as polled"
    );
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(game.join("postgame.json")).unwrap())
            .unwrap();
    assert_eq!(doc["ended"], "shutdown");
    let frames = doc["frames"].as_array().unwrap();
    let got: Vec<(&str, f64, &str)> = frames
        .iter()
        .map(|f| {
            (
                f["file"].as_str().unwrap(),
                f["t_s"].as_f64().unwrap(),
                f["label"].as_str().unwrap(),
            )
        })
        .collect();
    assert_eq!(
        got,
        vec![
            ("postgame_01.png", -1.5, "before_end"),
            ("postgame_02.png", 0.0, "end_title"),
            ("postgame_03.png", 2.5, "after_end"),
        ]
    );
    assert_eq!(doc["confirmed_s"], 1.5);
}

#[tokio::test]
async fn with_no_pre_game_the_window_makes_its_own_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).expect("on");
    let t0 = Instant::now();
    let confirm = PostTick {
        end: Some(EndSeen::Rank),
        confirmed: true,
        ..PostTick::default()
    };
    dump.observe_post(t0, confirm, Some(banner()));
    // The game closes: the window ends, the sidecar says so.
    dump.game_closed(t0 + Duration::from_secs(1));
    assert!(dump.finish(Duration::from_secs(30)).await);
    let root = pregame_dir(dir.path());
    let folders = names_in(&root);
    assert_eq!(folders.len(), 1);
    assert!(is_game_folder(&folders[0]));
    let game = root.join(&folders[0]);
    assert_eq!(names_in(&game), vec!["postgame.json", "postgame_01.png"]);
    let doc: serde_json::Value =
        serde_json::from_str(&std::fs::read_to_string(game.join("postgame.json")).unwrap())
            .unwrap();
    assert_eq!(doc["ended"], "game_closed");
    assert_eq!(doc["frames"][0]["label"], "rank_update");
    // The new folder is in the order file, so it counts for the retention.
    assert_eq!(load_pregame_order(&root), folders);
}

#[test]
fn the_retention_counts_post_game_folders_and_files() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut made = Vec::new();
    // 25 games, each only post-game files: a frame and its sidecar.
    for g in 0..25 {
        let folder = format!("20261030_12{g:02}00");
        let shot = PregameOp::Save(PregameShot {
            folder: folder.clone(),
            name: postgame_name(0),
            frame: shown(picture(10, None)),
        });
        write_pregame_ops(root, &[shot], PREGAME_KEEP_GAMES, &mut made);
        let note = NoteFile {
            folder: folder.clone(),
            name: POSTGAME_SIDECAR,
            doc: PostSidecar::default(),
        };
        write_note(root, &note, PREGAME_KEEP_GAMES, &mut made);
    }
    let folders = names_in(root);
    assert_eq!(folders.len(), PREGAME_KEEP_GAMES);
    assert_eq!(folders[0], "20261030_120500", "the oldest 5 went");
    assert_eq!(
        names_in(&root.join(&folders[0])),
        vec!["postgame.json", "postgame_01.png"]
    );
    // A sidecar alone in a new folder makes a game too.
    let note = NoteFile {
        folder: "20261030_130000".into(),
        name: POSTGAME_SIDECAR,
        doc: PostSidecar::default(),
    };
    write_note(root, &note, PREGAME_KEEP_GAMES, &mut made);
    assert_eq!(names_in(root).len(), PREGAME_KEEP_GAMES);
    assert!(!root.join("20261030_120500").exists());
    assert!(names_in(&root.join("20261030_130000")).contains(&"postgame.json".to_string()));
}

#[test]
fn the_staged_pre_game_sweep_leaves_post_game_files_alone() {
    // A game folder with post-game files and staged pre-game frames left
    // by a stopped run: a post-game sidecar in a new folder sweeps only the
    // staged frames, and the post-game names never look staged.
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let old = root.join("20261030_120000");
    std::fs::create_dir_all(&old).unwrap();
    for name in [postgame_name(0), POSTGAME_SIDECAR] {
        std::fs::write(old.join(name), b"x").unwrap();
    }
    for name in [BAN_RESULTS_NAME, LOADING_NAME] {
        std::fs::write(old.join(staged_name(name)), b"x").unwrap();
    }
    let note = NoteFile {
        folder: "20261030_130000".into(),
        name: POSTGAME_SIDECAR,
        doc: PostSidecar::default(),
    };
    write_note(root, &note, PREGAME_KEEP_GAMES, &mut Vec::new());
    // Every name, dot names too.
    let mut left: Vec<String> = std::fs::read_dir(&old)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["postgame.json", "postgame_01.png"]);
    for index in 0..POSTGAME_MAX_FRAMES {
        assert!(!postgame_name(index).starts_with('.'));
    }
}

#[test]
fn switch_off_has_no_saver_and_the_poll_loop_checks_nothing_new() {
    // Switch off: no saver, so no tracker, no writer thread, no folder.
    let dir = tempfile::tempdir().unwrap();
    let mut off = PregameDump::new(false, dir.path(), POLL);
    assert!(off.is_none());
    assert!(!dir.path().join("debug").exists());
    // The poll's want comes from the saver: none, so the tick keeps no
    // frame, even one read as an end, and makes no `Arc` for it.
    let t0 = Instant::now();
    assert_eq!(PregameDump::post_want_of(off.as_mut(), t0), None);
    let img = Arc::new(());
    let (pregame, postgame) = kept_frames(Arc::clone(&img), None, None, true);
    assert!(pregame.is_none() && postgame.is_none());
    assert_eq!(Arc::strong_count(&img), 1, "the frame is dropped, not held");
    // After the decision: nothing fed, no clock, game or cadence read, no
    // faster tick, and the frame is not held.
    let calls = std::cell::Cell::new(0);
    let count = || calls.set(calls.get() + 1);
    let frame = banner();
    let sooner = PregameDump::post_after_decision(
        off.as_mut(),
        || {
            count();
            t0
        },
        || {
            count();
            PostTick::default()
        },
        Some(Arc::clone(&frame)),
        POLL,
        || {
            count();
            true
        },
    );
    assert_eq!(sooner, None, "the regular interval stays");
    assert_eq!(calls.get(), 0, "nothing is checked");
    assert_eq!(
        Arc::strong_count(&frame),
        1,
        "the frame is dropped, not held"
    );

    // The same steps with the switch on do keep, feed and bring the tick
    // forward, so the checks above are not empty.
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let confirm = PostTick {
        end: Some(EndSeen::Title),
        confirmed: true,
        ..PostTick::default()
    };
    let every = PostWant {
        every: true,
        on_end: false,
    };
    let (pregame, postgame) = kept_frames(Arc::clone(&img), None, Some(every), false);
    assert!(pregame.is_none() && postgame.is_some());
    let pre = Some((Seen::BanResults, true));
    let (pregame, postgame) = kept_frames(Arc::clone(&img), pre, None, false);
    assert!(pregame.is_some_and(|(_, frame)| frame.is_some()) && postgame.is_none());
    for every_tick_polls in [true, false] {
        let on_dir = tempfile::tempdir().unwrap();
        let mut on = PregameDump::new(true, on_dir.path(), POLL);
        assert!(PregameDump::post_want_of(on.as_mut(), at(0)).is_some());
        calls.set(0);
        let sooner = PregameDump::post_after_decision(
            on.as_mut(),
            || {
                count();
                at(300)
            },
            || {
                count();
                confirm
            },
            Some(banner()),
            POLL,
            || {
                count();
                every_tick_polls
            },
        );
        assert!(on.as_ref().is_some_and(|dump| dump.post_window_open()));
        // Two clock reads, the tick, the cadence check.
        assert_eq!(calls.get(), 4);
        // Only where every tick polls.
        let want = every_tick_polls.then_some(Duration::from_millis(700));
        assert_eq!(sooner, want);
    }

    // The wiring stays a source check: `run_loop` needs a live capture
    // route, the game process gate, signal handlers and a store, so no unit
    // test can run one of its ticks. Only call names and their order are
    // matched, never layout, so a reformat does not break it.
    let run_loop = crate::main_tests::fn_source("run_loop");
    let find = |name: &str| run_loop.find(name).unwrap_or_else(|| panic!("{name}"));
    let want = find("PregameDump::post_want_of(");
    let split = find("postgame_dump::kept_frames(");
    let decided = find("apply_poll_tick(");
    let fed = find("PregameDump::post_after_decision(");
    let faster = find("poll_timer.reset_after(");
    assert!(want < split && split < decided && decided < fed && fed < faster);
    // The cadence check is the one handed to the post-game step, and it
    // asks that every tick polls (divisor 1).
    let cadence = &run_loop[fed..faster];
    let divisor = cadence.find("poll_divisor(").expect("cadence check");
    let rest: String = cadence[divisor..].split_whitespace().collect();
    assert!(rest.contains(")==1"), "only where every tick polls");
    // No faster tick elsewhere, and nothing reaches the post-game saves
    // around these steps.
    assert_eq!(run_loop.matches("poll_timer.reset_after(").count(), 1);
    for direct in [".post_want(", ".observe_post(", ".post_sooner_tick("] {
        assert!(!run_loop.contains(direct), "{direct}");
    }
}
