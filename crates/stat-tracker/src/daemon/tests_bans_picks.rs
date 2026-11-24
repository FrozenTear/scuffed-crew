//! Tests for `daemon::bans_picks`, kept out of line so the source-text
//! helper `daemon_source()` still sees the production code after this file.

use std::sync::Mutex;

use image::Rgb;

use super::*;

/// Records the rings the ban results are read with.
struct Spy(Mutex<Vec<Vec<[u8; 3]>>>);

impl Engine for Spy {
    fn vote(&self, _: &RgbImage) -> Option<BanVote> {
        Some(BanVote {
            slots: None,
            score: None,
        })
    }
    fn select(&self, _: &RgbImage) -> Option<HeroSelect> {
        Some(HeroSelect {
            picks: Vec::new(),
            ring: [1, 2, 3],
        })
    }
    fn bans(&self, _: &RgbImage, rings: &[[u8; 3]]) -> Option<BanResults> {
        self.0.lock().unwrap().push(rings.to_vec());
        Some(BanResults {
            layout: Layout::Plain,
            bans: Vec::new(),
            lobby_ban: None,
        })
    }
}

/// A frame with a team ring of colour `c` where the game draws it.
fn ringed(w: u32, h: u32, c: [u8; 3]) -> Arc<DynamicImage> {
    let mut img = RgbImage::new(w, h);
    for y in 430..475.min(h) {
        for x in 64..70.min(w) {
            img.put_pixel(x, y, Rgb(c));
        }
    }
    Arc::new(DynamicImage::ImageRgb8(img))
}

#[test]
fn the_ban_vote_ring_is_given_to_the_ban_results_as_a_second_ring() {
    let spy = Spy(Mutex::new(Vec::new()));
    let frames = Frames {
        vote: Some(ringed(FRAME_W, FRAME_H, [128, 0, 128])),
        results: Some(ringed(FRAME_W, FRAME_H, [0, 0, 0])),
        ..Frames::default()
    };
    assert!(read_with(&spy, &frames).is_some());
    // no select frame: the vote ring is the only one
    assert_eq!(spy.0.lock().unwrap()[0], vec![[128, 0, 128]]);
    // with a select frame its ring comes first
    let frames = Frames {
        select: Some(ringed(FRAME_W, FRAME_H, [0, 0, 0])),
        ..frames
    };
    assert!(read_with(&spy, &frames).is_some());
    assert_eq!(spy.0.lock().unwrap()[1], vec![[1, 2, 3], [128, 0, 128]]);
    // a vote frame of another size gives no ring
    let frames = Frames {
        vote: Some(ringed(8, 8, [128, 0, 128])),
        ..frames
    };
    assert!(read_with(&spy, &frames).is_some());
    assert_eq!(spy.0.lock().unwrap()[2], vec![[1, 2, 3]]);
}

/// A config.toml as alpha.2.3 saved it with Read bans and picks off. The
/// switch moved to Settings, Capture and reading; its key did not.
const ALPHA_2_3_READER_OFF: &str = "\
data_dir = \"/home/member/.local/share/scuffed-stat-tracker\"
capture_route = \"auto\"
player_name = \"Member\"
setup_completed = true
bans_picks_reader = false

[auto_detect]
enabled = true
cooldown_secs = 120
";

/// The tracker keeps an alpha.2.3 file's reader off, and
/// STAT_TRACKER_BANS_PICKS_RECOGNIZER=off still wins over a saved on.
#[test]
fn the_saved_key_and_the_env_override_still_gate_the_reader() {
    let dir = tempfile::tempdir().unwrap();
    let old = stat_tracker::config::Config::parse_file_contents(ALPHA_2_3_READER_OFF).unwrap();
    assert!(!old.bans_picks_reader);
    let feed = |saved: bool, env: &str| {
        BansPicksFeed::for_config(saved, BansPicksSetting::parse(env), dir.path(), false)
    };
    assert!(
        feed(old.bans_picks_reader, "cv-v1").is_none(),
        "alpha.2.3 off stays off"
    );
    assert!(feed(true, "cv-v1").is_some());
    for off in ["off", "OFF", "0", "false", "no"] {
        assert!(feed(true, off).is_none(), "{off} wins over a saved on");
    }
    assert!(feed(false, "1").is_none(), "the variable never turns it on");
    assert_eq!(
        stat_tracker::shadow::bans_picks::ENV,
        "STAT_TRACKER_BANS_PICKS_RECOGNIZER"
    );
}

// ---- the ban results reveal gate (Scuffed Vision's rule) ----

use crate::daemon::pregame_dump::PregameShot;
use stat_tracker::shadow::bans_picks::Choice;
use std::sync::atomic::{AtomicUsize, Ordering};

const FOLDER: &str = "20261113_232437";

fn ban(hero: &str) -> BanRead {
    BanRead {
        read: Read {
            value: Some(Choice::Hero(hero.to_string())),
            confidence: 0.9,
            score: 0.9,
            margin: 0.2,
        },
        team: None,
    }
}

/// A slot that reads no hero (an empty slot before the reveal).
fn no_ban() -> BanRead {
    BanRead {
        read: Read {
            value: None,
            confidence: 0.0,
            score: 0.3,
            margin: 0.01,
        },
        team: None,
    }
}

const HEROES: [&str; 4] = ["mauga", "kiriko", "genji", "sojourn"];

/// Ban results with the first `set` of the four bans set; the lobby layout
/// carries a lobby ban read.
fn results(set: usize, lobby: bool) -> BanResults {
    BanResults {
        layout: if lobby { Layout::Lobby } else { Layout::Plain },
        bans: (0..4)
            .map(|i| if i < set { ban(HEROES[i]) } else { no_ban() })
            .collect(),
        lobby_ban: lobby.then(|| ban("ana").read),
    }
}

/// Reads the ban results from a mark in the frame's top-left pixel: red is
/// how many bans are set, green 1 the lobby layout. Counts its ban reads.
#[derive(Default)]
struct Marked(AtomicUsize);

impl Engine for Marked {
    fn vote(&self, _: &RgbImage) -> Option<BanVote> {
        Some(BanVote {
            slots: None,
            score: None,
        })
    }
    fn select(&self, _: &RgbImage) -> Option<HeroSelect> {
        Some(HeroSelect {
            picks: Vec::new(),
            ring: [1, 2, 3],
        })
    }
    fn bans(&self, img: &RgbImage, _: &[[u8; 3]]) -> Option<BanResults> {
        self.0.fetch_add(1, Ordering::SeqCst);
        let [set, lobby, _] = img.get_pixel(0, 0).0;
        Some(results(set as usize, lobby == 1))
    }
}

fn marked(set: u8, lobby: bool) -> Arc<DynamicImage> {
    let mut img = RgbImage::new(FRAME_W, FRAME_H);
    img.put_pixel(0, 0, Rgb([set, u8::from(lobby), 0]));
    Arc::new(DynamicImage::ImageRgb8(img))
}

fn shot(name: &'static str, frame: &Arc<DynamicImage>) -> PregameShot<Arc<DynamicImage>> {
    PregameShot {
        folder: FOLDER.to_string(),
        name,
        frame: Arc::clone(frame),
    }
}

fn rejected(frame: &Arc<DynamicImage>) -> PregameOp<Arc<DynamicImage>> {
    PregameOp::Save(shot(RejectLabel::BanResults.file_name(), frame))
}

/// A feed reading with `engine`, and its data dir.
fn marked_feed(engine: &Arc<Marked>) -> (tempfile::TempDir, BansPicksFeed) {
    let dir = tempfile::tempdir().unwrap();
    let engine = Arc::clone(engine);
    let loader: Loader = Arc::new(move || Ok(Arc::clone(&engine) as Arc<dyn Engine>));
    let feed = BansPicksFeed::with_loader(dir.path().to_path_buf(), true, loader);
    (dir, feed)
}

/// The stored result as JSON, and the status.
async fn read_bans(feed: &mut BansPicksFeed) -> (serde_json::Value, BansPicksStatus) {
    let outcome = feed.read_now().await;
    let stored = outcome.stored.expect("a result");
    (serde_json::from_str(&stored).unwrap(), outcome.status)
}

fn set_count(stored: &serde_json::Value) -> usize {
    stored["bans"]
        .as_array()
        .expect("bans")
        .iter()
        .filter(|b| !b["value"].is_null())
        .count()
}

fn pick(candidates: Vec<(BanSource, BanResults)>) -> Option<(BanSource, usize)> {
    choose_ban_results(candidates).map(|(s, r)| (s, r.bans_set()))
}

#[test]
fn an_unrevealed_frame_is_told_by_its_four_bans_alone() {
    assert!(results(0, false).unrevealed());
    // the lobby layout with its lobby ban read is unrevealed all the same:
    // the layout is real and the lobby ban is not looked at
    let lobby = results(0, true);
    assert!(lobby.lobby_ban.as_ref().is_some_and(|r| r.value.is_some()));
    assert!(lobby.unrevealed());
    assert!(!results(1, true).unrevealed());
    assert_eq!(results(3, false).bans_set(), 3);
}

#[test]
fn a_stable_frame_with_three_or_more_bans_is_kept() {
    use BanSource::*;
    assert_eq!(BANS_KEPT_MIN, 3);
    assert_eq!(
        pick(vec![
            (Saved, results(4, false)),
            (Rejected, results(4, false))
        ]),
        Some((Saved, 4))
    );
    assert_eq!(
        pick(vec![
            (Saved, results(3, true)),
            (Rejected, results(4, false))
        ]),
        Some((Saved, 3))
    );
    // the newest stable frame with three or more, even over a fuller one
    assert_eq!(
        pick(vec![
            (Saved, results(2, false)),
            (Earlier, results(3, false)),
            (Earlier, results(4, false)),
            (Rejected, results(4, false)),
        ]),
        Some((Earlier, 3))
    );
}

#[test]
fn an_unrevealed_saved_frame_takes_the_turned_down_revealed_one() {
    use BanSource::*;
    // the field case: ban_results.png empty, the good frame turned down
    assert_eq!(
        pick(vec![
            (Saved, results(0, false)),
            (Rejected, results(4, false))
        ]),
        Some((Rejected, 4))
    );
    // the lobby layout passes the same way
    assert_eq!(
        pick(vec![
            (Saved, results(0, true)),
            (Rejected, results(3, true))
        ]),
        Some((Rejected, 3))
    );
}

#[test]
fn with_no_stable_frame_of_three_the_most_bans_set_wins() {
    use BanSource::*;
    assert_eq!(
        pick(vec![
            (Saved, results(2, false)),
            (Rejected, results(4, false))
        ]),
        Some((Rejected, 4))
    );
    assert_eq!(
        pick(vec![
            (Saved, results(1, false)),
            (Rejected, results(1, false))
        ]),
        Some((Saved, 1)),
        "a tie keeps the first: the stable frame"
    );
    assert_eq!(
        pick(vec![
            (Saved, results(2, false)),
            (Rejected, results(1, false))
        ]),
        Some((Saved, 2))
    );
}

#[test]
fn only_unrevealed_frames_store_no_bans() {
    use BanSource::*;
    assert_eq!(
        pick(vec![
            (Saved, results(0, false)),
            (Earlier, results(0, true)),
            (Rejected, results(0, false)),
        ]),
        None
    );
    assert_eq!(pick(Vec::new()), None);
}

#[test]
fn the_same_hero_twice_is_no_reason_to_drop_a_frame() {
    // no duplicate-hero guard: the frame is kept as read
    let mut twice = results(4, false);
    twice.bans[1] = twice.bans[0].clone();
    assert_eq!(
        pick(vec![(BanSource::Saved, twice)]),
        Some((BanSource::Saved, 4))
    );
}

#[tokio::test]
async fn the_field_case_stores_the_turned_down_frame_not_four_empty_bans() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    // two results frames: the pre-reveal one is staged when the revealed
    // one replaces it; the hero select read does not confirm the newer
    // one, so the staged one is saved and the newer one turned down
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(0, false)))]);
    feed.observe(&[
        PregameOp::Promote {
            folder: FOLDER.to_string(),
            name: BAN_RESULTS_NAME,
        },
        rejected(&marked(4, false)),
    ]);
    let (stored, status) = read_bans(&mut feed).await;
    assert_eq!(set_count(&stored), 4, "{stored}");
    assert_eq!(
        status,
        BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::BanVote, BansPicksMissing::HeroSelect]
        },
        "the ban results are there"
    );
}

#[tokio::test]
async fn an_unrevealed_frame_alone_stores_no_bans_and_counts_as_missing() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    feed.observe(&[PregameOp::Save(shot(BAN_RESULTS_NAME, &marked(0, true)))]);
    let (stored, status) = read_bans(&mut feed).await;
    assert_eq!(stored["bans"], serde_json::json!([]), "{stored}");
    assert!(
        stored["layout"].is_null() && stored["lobby_ban"].is_null(),
        "{stored}"
    );
    assert_eq!(
        status,
        BansPicksStatus::Partial {
            missing: vec![
                BansPicksMissing::BanVote,
                BansPicksMissing::HeroSelect,
                BansPicksMissing::BanResults,
            ]
        }
    );
}

#[tokio::test]
async fn an_earlier_stable_frame_with_three_bans_wins_over_a_partial_saved_one() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    // stages of 3, then 1 bans set (the 3 kept as an earlier frame); then
    // the held frame of 2 is confirmed and saved, the staged 1 deleted
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(3, false)))]);
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(1, false)))]);
    feed.observe(&[
        PregameOp::Save(shot(BAN_RESULTS_NAME, &marked(2, false))),
        PregameOp::Discard {
            folder: FOLDER.to_string(),
            name: BAN_RESULTS_NAME,
        },
    ]);
    let (stored, _) = read_bans(&mut feed).await;
    assert_eq!(set_count(&stored), 3, "{stored}");
}

#[tokio::test]
async fn a_revealed_saved_frame_is_the_only_ban_read() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    feed.observe(&[
        PregameOp::Save(shot(BAN_RESULTS_NAME, &marked(3, false))),
        rejected(&marked(4, false)),
    ]);
    let (stored, _) = read_bans(&mut feed).await;
    assert_eq!(set_count(&stored), 3, "the saved frame as read: {stored}");
    assert_eq!(engine.0.load(Ordering::SeqCst), 1, "no fallback read");
}

#[tokio::test]
async fn the_fallback_holds_only_the_newest_turned_down_frames() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    // the only revealed turned-down frame is the oldest of three: dropped
    feed.observe(&[rejected(&marked(4, false))]);
    for _ in 0..BAN_FALLBACK_FRAMES {
        feed.observe(&[rejected(&marked(0, false))]);
    }
    feed.observe(&[PregameOp::Save(shot(BAN_RESULTS_NAME, &marked(0, false)))]);
    let (stored, _) = read_bans(&mut feed).await;
    assert_eq!(stored["bans"], serde_json::json!([]), "{stored}");
    assert_eq!(engine.0.load(Ordering::SeqCst), 1 + BAN_FALLBACK_FRAMES);
}

#[tokio::test]
async fn the_fallback_holds_only_the_newest_earlier_frames() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    // the only revealed stable frame is the oldest stage: each newer stage
    // pushes the one it replaces into the earlier list, which keeps the
    // newest two, so the revealed one is dropped
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(4, false)))]);
    for _ in 0..=BAN_FALLBACK_FRAMES {
        feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(0, false)))]);
    }
    assert_eq!(feed.frames.earlier.len(), BAN_FALLBACK_FRAMES);
    feed.observe(&[PregameOp::Save(shot(BAN_RESULTS_NAME, &marked(0, false)))]);
    let (stored, _) = read_bans(&mut feed).await;
    assert_eq!(stored["bans"], serde_json::json!([]), "{stored}");
    assert_eq!(engine.0.load(Ordering::SeqCst), 1 + BAN_FALLBACK_FRAMES);
}

#[tokio::test]
async fn a_game_with_only_turned_down_ban_results_is_read() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    feed.observe(&[rejected(&marked(4, false))]);
    assert!(feed.has_frames(), "not a game without frames");
    let (stored, _) = read_bans(&mut feed).await;
    assert_eq!(set_count(&stored), 4, "{stored}");
}

#[tokio::test]
async fn a_game_with_only_earlier_ban_results_is_read() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    // a stage replaced by a newer one, which the saver then deleted: both
    // are earlier stable frames and nothing was saved
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(3, false)))]);
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(1, false)))]);
    feed.observe(&[PregameOp::Discard {
        folder: FOLDER.to_string(),
        name: BAN_RESULTS_NAME,
    }]);
    assert!(feed.frames.results.is_none() && feed.frames.staged.is_none());
    assert!(feed.has_frames(), "not a game without frames");
    let (stored, _) = read_bans(&mut feed).await;
    assert_eq!(set_count(&stored), 3, "{stored}");
}

#[test]
fn a_new_game_folder_drops_the_held_fallback_frames() {
    let engine = Arc::new(Marked::default());
    let (_dir, mut feed) = marked_feed(&engine);
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(3, false)))]);
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &marked(1, false)))]);
    feed.observe(&[rejected(&marked(4, false))]);
    assert_eq!(feed.frames.earlier.len(), 1);
    assert_eq!(feed.frames.rejected.len(), 1);
    assert!(feed.frames.staged.is_some());
    // the next game's first save; the old game was never read
    feed.observe(&[PregameOp::Save(PregameShot {
        folder: "20261113_235901".to_string(),
        name: PregamePhase::HeroBan.last_name(),
        frame: marked(0, false),
    })]);
    assert_eq!(feed.folder.as_deref(), Some("20261113_235901"));
    assert!(feed.frames.earlier.is_empty(), "earlier frames dropped");
    assert!(
        feed.frames.rejected.is_empty(),
        "turned-down frames dropped"
    );
    assert!(feed.frames.staged.is_none(), "the stage dropped");
    assert!(feed.frames.vote.is_some(), "the new game's frame is held");
    assert_eq!(engine.0.load(Ordering::SeqCst), 0, "nothing was read");
}
