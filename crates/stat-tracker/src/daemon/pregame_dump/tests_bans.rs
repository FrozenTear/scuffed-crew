//! The bans and picks wiring: which frames are read, when, and that no
//! failure stops a game from being recorded.

use super::*;
use crate::daemon::bans_picks::{
    BansPicksFeed, Engine, LoadFailure, Loader, NO_SELECT_NOTICE, missing_notice,
};
use image::{DynamicImage, RgbImage};
use stat_tracker::model::{BansPicksMissing, BansPicksStatus};
use stat_tracker::ocr_status::{publish_bans_picks_notice, read_status};
use stat_tracker::shadow::bans_picks::{
    BanResults, BanVote, BansPicksSetting, Choice, HeroSelect, Layout, Read, SlotState,
    VoteSlotRead,
};
use std::sync::Mutex;
use std::sync::atomic::{AtomicUsize, Ordering};

const POLL: Duration = Duration::from_millis(1_500);
const FOLDER: &str = "20261103_120000";

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Fine,
    Panics,
    Nothing,
}

/// The team rings the ban results were read with, in call order.
type Rings = Arc<Mutex<Vec<Option<[u8; 3]>>>>;

struct Fake {
    mode: Mode,
    calls: Arc<AtomicUsize>,
    rings: Rings,
}

fn read(hero: &str) -> Read {
    Read {
        value: Some(Choice::Hero(hero.to_string())),
        confidence: 0.9,
        score: 0.9,
        margin: 0.2,
    }
}

impl Fake {
    fn go(&self) -> bool {
        self.calls.fetch_add(1, Ordering::SeqCst);
        match self.mode {
            Mode::Panics => panic!("the reader broke"),
            Mode::Nothing => false,
            Mode::Fine => true,
        }
    }
}

impl Engine for Fake {
    fn vote(&self, _: &RgbImage) -> Option<BanVote> {
        self.go().then(|| BanVote {
            slots: Some(vec![VoteSlotRead {
                voter: 0,
                slot: 1,
                state: Some(SlotState::Hero),
                hero: Some("ana".to_string()),
                confidence: 0.9,
                score: 0.9,
                margin: 0.2,
            }]),
            score: None,
        })
    }
    fn select(&self, _: &RgbImage) -> Option<HeroSelect> {
        self.go().then(|| HeroSelect {
            picks: vec![read("mercy")],
            ring: [1, 2, 3],
        })
    }
    fn bans(&self, _: &RgbImage, rings: &[[u8; 3]]) -> Option<BanResults> {
        self.rings.lock().unwrap().push(rings.first().copied());
        self.go().then(|| BanResults {
            layout: Layout::Plain,
            bans: Vec::new(),
            lobby_ban: Some(read("genji")),
        })
    }
}

/// A loader that counts its calls and gives `result`.
fn loader(
    result: Result<Mode, LoadFailure>,
    loads: &Arc<AtomicUsize>,
    reads: &Arc<AtomicUsize>,
    rings: &Rings,
) -> Loader {
    let loads = Arc::clone(loads);
    let reads = Arc::clone(reads);
    let rings = Arc::clone(rings);
    Arc::new(move || {
        loads.fetch_add(1, Ordering::SeqCst);
        result.clone().map(|mode| {
            Arc::new(Fake {
                mode,
                calls: Arc::clone(&reads),
                rings: Arc::clone(&rings),
            }) as Arc<dyn Engine>
        })
    })
}

fn frame(w: u32, h: u32) -> Arc<DynamicImage> {
    Arc::new(DynamicImage::new_rgb8(w, h))
}

fn shot(name: &'static str, frame: &Arc<DynamicImage>) -> PregameShot<Arc<DynamicImage>> {
    PregameShot {
        folder: FOLDER.to_string(),
        name,
        frame: Arc::clone(frame),
    }
}

fn three(w: u32, h: u32) -> ShotBatch {
    let f = frame(w, h);
    vec![
        PregameOp::Save(shot(PregamePhase::HeroBan.last_name(), &f)),
        PregameOp::Save(shot(PregamePhase::HeroSelect.last_name(), &f)),
        PregameOp::Save(shot(BAN_RESULTS_NAME, &f)),
    ]
}

struct Rig {
    dir: tempfile::TempDir,
    loads: Arc<AtomicUsize>,
    reads: Arc<AtomicUsize>,
    rings: Rings,
}

impl Rig {
    fn new() -> Self {
        Self {
            dir: tempfile::tempdir().expect("tempdir"),
            loads: Arc::default(),
            reads: Arc::default(),
            rings: Arc::default(),
        }
    }

    fn feed(&self, result: Result<Mode, LoadFailure>, linked: bool) -> BansPicksFeed {
        BansPicksFeed::with_loader(
            self.dir.path().to_path_buf(),
            linked,
            loader(result, &self.loads, &self.reads, &self.rings),
        )
    }

    fn dump(&self, on: bool, feed: BansPicksFeed) -> PregameDump {
        PregameDump::with_bans_picks(on, Some(feed), self.dir.path(), POLL).expect("saver")
    }

    /// What the app shows under Tracker health.
    fn health(&self) -> Option<String> {
        read_status(self.dir.path()).and_then(|s| s.health_notice())
    }
}

/// A recorded game, then the read. Returns the stored result of the game.
async fn game_then_read(
    rig: &Rig,
    dump: &mut PregameDump,
    batch: ShotBatch,
) -> (storage::LocalStore, Option<String>) {
    dump.send(batch);
    let store = storage::LocalStore::open(rig.dir.path())
        .await
        .expect("store");
    store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    dump.attach_bans_picks(&store, "g1").await;
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 1, "the game is recorded whatever the read did");
    let stored = rows[0].bans_picks.clone();
    (store, stored)
}

#[tokio::test]
async fn a_read_stores_the_result_on_the_game_and_clears_the_warning() {
    let rig = Rig::new();
    publish_bans_picks_notice(rig.dir.path(), "an old warning");
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (_, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
    let stored = stored.expect("a result");
    assert!(stored.contains("cv-v1"), "{stored}");
    assert!(stored.contains("mercy") && stored.contains("genji") && stored.contains("ana"));
    assert_eq!(rig.health(), None, "the old warning is gone");
    assert_eq!(rig.reads.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_missing_pack_still_records_the_game_and_warns() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Err(LoadFailure::NotInstalled), true));
    let (_, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
    assert_eq!(stored, None);
    let warning = rig.health().expect("a warning");
    assert!(warning.contains("not installed yet"), "{warning}");
    assert_eq!(rig.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn an_install_not_linked_to_a_site_account_is_told_how_to_get_the_pack() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Err(LoadFailure::NotInstalled), false));
    let (_, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
    assert_eq!(stored, None, "the game is recorded without bans and picks");
    let warning = rig.health().expect("a warning");
    assert!(
        warning.contains("not linked to a site account"),
        "{warning}"
    );
    assert!(warning.contains("Link one"), "{warning}");
    assert_eq!(warning, missing_notice(false));
    assert_ne!(missing_notice(false), missing_notice(true));
    assert!(!missing_notice(true).contains("Link one"));
}

#[tokio::test]
async fn the_real_loader_finds_no_pack_in_a_bare_data_dir() {
    let dir = tempfile::tempdir().expect("tempdir");
    let feed = BansPicksFeed::for_config(true, BansPicksSetting::CvV1, dir.path(), false);
    let mut dump = PregameDump::with_bans_picks(false, feed, dir.path(), POLL).expect("saver");
    dump.send(three(2560, 1440));
    let store = storage::LocalStore::open(dir.path()).await.expect("store");
    store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    dump.attach_bans_picks(&store, "g1").await;
    assert_eq!(store.get_all_matches().await.unwrap().len(), 1);
    let status = read_status(dir.path()).expect("status");
    assert_eq!(status.health_notice(), Some(missing_notice(false)));
}

#[tokio::test]
async fn a_pack_that_does_not_load_still_records_the_game_and_warns() {
    let rig = Rig::new();
    let failure = LoadFailure::Failed("missing templates: new-hero".to_string());
    let mut dump = rig.dump(false, rig.feed(Err(failure), true));
    let (_, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
    assert_eq!(stored, None);
    let warning = rig.health().expect("a warning");
    assert!(warning.contains("did not load"), "{warning}");
    assert!(warning.contains("new-hero"), "{warning}");
    assert!(
        warning.contains("Settings > Capture and reading"),
        "{warning}"
    );
}

#[tokio::test]
async fn a_failed_read_still_records_the_game_and_warns() {
    for mode in [Mode::Panics, Mode::Nothing] {
        let rig = Rig::new();
        let mut dump = rig.dump(false, rig.feed(Ok(mode), true));
        let (_, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
        assert_eq!(stored, None);
        let warning = rig.health().expect("a warning");
        assert!(warning.contains("failed on this game"), "{warning}");
    }
}

#[tokio::test]
async fn a_frame_that_is_not_1440p_is_a_note_and_the_reader_is_not_called() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (_, stored) = game_then_read(&rig, &mut dump, three(1920, 1080)).await;
    assert_eq!(stored, None);
    let warning = rig.health().expect("a note");
    assert!(
        warning.contains("1920x1080") && warning.contains("2560x1440"),
        "{warning}"
    );
    assert_eq!(
        rig.reads.load(Ordering::SeqCst),
        0,
        "the reader was not called"
    );
    assert_eq!(
        rig.loads.load(Ordering::SeqCst),
        0,
        "the pack was not loaded"
    );
}

#[tokio::test]
async fn a_select_with_only_a_first_frame_gives_the_ring_and_nothing_else() {
    // A one-tick hero select (game 20261101_205808): no last frame.
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let f = frame(2560, 1440);
    let batch = vec![
        PregameOp::Save(shot(PregamePhase::HeroBan.last_name(), &f)),
        PregameOp::Save(shot(PregamePhase::HeroSelect.first_name(), &f)),
        PregameOp::Save(shot(BAN_RESULTS_NAME, &f)),
    ];
    let (_, stored) = game_then_read(&rig, &mut dump, batch).await;
    let stored = stored.expect("a result");
    assert_eq!(
        *rig.rings.lock().unwrap(),
        vec![Some([1, 2, 3])],
        "the ban results got the first frame's ring"
    );
    assert!(
        stored.contains("genji") && stored.contains("ana"),
        "{stored}"
    );
    assert!(
        !stored.contains("mercy"),
        "the first frame's picks are not stored: {stored}"
    );
    assert_eq!(rig.health(), None);
    assert_eq!(rig.reads.load(Ordering::SeqCst), 3);
}

#[tokio::test]
async fn a_last_frame_of_the_select_wins_over_its_first() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let mut batch = three(2560, 1440);
    batch.push(PregameOp::Save(shot(
        PregamePhase::HeroSelect.first_name(),
        &frame(2560, 1440),
    )));
    let (_, stored) = game_then_read(&rig, &mut dump, batch).await;
    assert!(stored.expect("a result").contains("mercy"));
    assert_eq!(
        rig.reads.load(Ordering::SeqCst),
        3,
        "the first frame is not read"
    );
    assert_eq!(*rig.rings.lock().unwrap(), vec![Some([1, 2, 3])]);
}

#[test]
fn a_first_frame_of_the_select_alone_is_nothing_to_read() {
    let rig = Rig::new();
    let mut feed = rig.feed(Ok(Mode::Fine), true);
    let f = frame(8, 8);
    feed.observe(&[PregameOp::Save(shot(
        PregamePhase::HeroSelect.first_name(),
        &f,
    ))]);
    assert!(!feed.has_frames(), "no ban results, so no ring to give");
}

#[test]
fn the_toggle_off_builds_no_feed() {
    let dir = tempfile::tempdir().expect("tempdir");
    for (config, setting, on) in [
        (true, BansPicksSetting::CvV1, true),
        (false, BansPicksSetting::CvV1, false),
        (true, BansPicksSetting::Off, false),
        (false, BansPicksSetting::Off, false),
        (true, BansPicksSetting::parse("false"), false),
        (true, BansPicksSetting::parse("NO"), false),
        (true, BansPicksSetting::parse("1"), true),
    ] {
        let feed = BansPicksFeed::for_config(config, setting, dir.path(), true);
        assert_eq!(feed.is_some(), on, "config {config}");
    }
    // Nothing on and no feed: no saver, no thread.
    assert!(PregameDump::with_bans_picks(false, None, dir.path(), POLL).is_none());
}

#[tokio::test]
async fn with_the_reader_off_nothing_is_read_and_no_warning_is_made() {
    let rig = Rig::new();
    let off = BansPicksFeed::for_config(false, BansPicksSetting::CvV1, rig.dir.path(), true);
    let mut dump = PregameDump::with_bans_picks(true, off, rig.dir.path(), POLL).expect("saver");
    dump.send(three(2560, 1440));
    let store = storage::LocalStore::open(rig.dir.path())
        .await
        .expect("store");
    store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    dump.attach_bans_picks(&store, "g1").await;
    assert!(dump.bans.is_none(), "no reader to call");
    assert_eq!(rig.health(), None);
    assert_eq!(store.get_all_matches().await.unwrap()[0].bans_picks, None);
}

#[tokio::test]
async fn the_screens_are_read_once_and_a_late_game_does_not_take_them() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (store, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
    assert!(stored.is_some());
    assert_eq!(rig.reads.load(Ordering::SeqCst), 3);
    // A later capture of the same game reads nothing more.
    dump.attach_bans_picks(&store, "g1").await;
    assert_eq!(rig.reads.load(Ordering::SeqCst), 3);
    assert_eq!(rig.loads.load(Ordering::SeqCst), 1);
}

#[test]
fn a_new_game_folder_drops_the_old_frames_and_the_window_is_30_minutes() {
    let rig = Rig::new();
    let mut feed = rig.feed(Ok(Mode::Fine), true);
    assert!(!feed.has_frames());
    assert!(
        !feed.fits(chrono::Utc::now()),
        "nothing fed, nothing to fit"
    );
    let f = frame(8, 8);
    feed.observe(&[PregameOp::Save(shot(PregamePhase::HeroBan.last_name(), &f))]);
    assert!(feed.has_frames());
    let now = chrono::Utc::now();
    assert!(feed.fits(now));
    assert!(feed.fits(now + chrono::Duration::minutes(29)));
    assert!(!feed.fits(now + chrono::Duration::minutes(31)));
    assert!(
        !feed.fits(now - chrono::Duration::minutes(1)),
        "a game before the screens"
    );
    let mut other = shot(PregamePhase::HeroSelect.last_name(), &f);
    other.folder = "20261103_130000".to_string();
    feed.observe(&[PregameOp::Save(other)]);
    assert!(feed.has_frames(), "the new game's own frame");
}

#[test]
fn staged_ban_results_count_only_once_promoted() {
    let rig = Rig::new();
    let mut feed = rig.feed(Ok(Mode::Fine), true);
    let f = frame(8, 8);
    feed.observe(&[PregameOp::Stage(shot(BAN_RESULTS_NAME, &f))]);
    assert!(!feed.has_frames(), "a staged frame is not a result");
    feed.observe(&[PregameOp::Promote {
        folder: FOLDER.to_string(),
        name: BAN_RESULTS_NAME,
    }]);
    assert!(feed.has_frames());
    let mut feed = rig.feed(Ok(Mode::Fine), true);
    feed.observe(&[
        PregameOp::Stage(shot(BAN_RESULTS_NAME, &f)),
        PregameOp::Discard {
            folder: FOLDER.to_string(),
            name: BAN_RESULTS_NAME,
        },
        PregameOp::Promote {
            folder: FOLDER.to_string(),
            name: BAN_RESULTS_NAME,
        },
    ]);
    assert!(!feed.has_frames(), "a discarded frame is gone");
}

#[test]
fn with_the_screen_saves_off_the_saver_writes_nothing_and_keeps_draining() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    for _ in 0..40 {
        // More than the queue holds: a refused send would be a lost frame.
        dump.send(three(64, 64));
        std::thread::sleep(Duration::from_millis(2));
    }
    assert!(
        !pregame_dir(rig.dir.path()).join(FOLDER).exists(),
        "nothing on disk"
    );
    assert!(dump.bans.as_ref().is_some_and(|f| f.has_frames()));
    assert!(!dump.writer.disk());
}

/// The status stored on the one game of the test.
async fn status_of(store: &storage::LocalStore) -> Option<BansPicksStatus> {
    store.get_all_matches().await.unwrap()[0]
        .bans_picks_status
        .clone()
}

const NO_SCREENS: &str =
    "Bans and picks were not read: no pre-game screens were seen for this game.";

#[tokio::test]
async fn bans_without_a_hero_select_store_empty_picks_and_show_a_warning() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let f = frame(2560, 1440);
    let batch = vec![
        PregameOp::Save(shot(PregamePhase::HeroBan.last_name(), &f)),
        PregameOp::Save(shot(BAN_RESULTS_NAME, &f)),
    ];
    let (_, stored) = game_then_read(&rig, &mut dump, batch).await;
    let stored = stored.expect("a result");
    assert!(stored.contains("genji"), "{stored}");
    assert!(stored.contains("\"picks\":[]"), "{stored}");
    assert_eq!(rig.health().as_deref(), Some(NO_SELECT_NOTICE));
}

#[tokio::test]
async fn the_samoa_game_is_partial_with_the_hero_select_missing() {
    // Bans read, hero select never seen: the ban vote and results are there.
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let f = frame(2560, 1440);
    let batch = vec![
        PregameOp::Save(shot(PregamePhase::HeroBan.last_name(), &f)),
        PregameOp::Save(shot(BAN_RESULTS_NAME, &f)),
    ];
    let (store, stored) = game_then_read(&rig, &mut dump, batch).await;
    assert!(stored.is_some());
    assert_eq!(
        status_of(&store).await,
        Some(BansPicksStatus::Partial {
            missing: vec![BansPicksMissing::HeroSelect]
        })
    );
}

#[tokio::test]
async fn a_read_of_all_three_screens_is_ok() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (store, _) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
    assert_eq!(status_of(&store).await, Some(BansPicksStatus::Ok));
}

#[tokio::test]
async fn a_frame_of_the_wrong_size_is_a_frame_size_status() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (store, _) = game_then_read(&rig, &mut dump, three(1920, 1080)).await;
    assert_eq!(
        status_of(&store).await,
        Some(BansPicksStatus::FrameSize {
            width: 1920,
            height: 1080
        })
    );
}

#[tokio::test]
async fn a_pack_that_is_missing_or_will_not_load_or_a_reader_that_fails_has_its_status() {
    for (result, linked, want) in [
        (
            Err(LoadFailure::NotInstalled),
            true,
            BansPicksStatus::PackMissing { linked: true },
        ),
        (
            Err(LoadFailure::NotInstalled),
            false,
            BansPicksStatus::PackMissing { linked: false },
        ),
        (
            Err(LoadFailure::Failed("bad".to_string())),
            true,
            BansPicksStatus::PackFailed,
        ),
        (Ok(Mode::Panics), true, BansPicksStatus::ReaderFailed),
        (Ok(Mode::Nothing), true, BansPicksStatus::ReaderFailed),
    ] {
        let rig = Rig::new();
        let mut dump = rig.dump(false, rig.feed(result, linked));
        let (store, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
        assert_eq!(stored, None);
        assert_eq!(status_of(&store).await, Some(want));
    }
}

#[tokio::test]
async fn a_game_with_no_pregame_screens_is_recorded_and_shows_a_warning() {
    let rig = Rig::new();
    publish_bans_picks_notice(rig.dir.path(), "");
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (store, stored) = game_then_read(&rig, &mut dump, Vec::new()).await;
    assert_eq!(stored, None);
    assert_eq!(rig.health().as_deref(), Some(NO_SCREENS));
    assert_eq!(rig.reads.load(Ordering::SeqCst), 0);
    assert_eq!(status_of(&store).await, Some(BansPicksStatus::NoFrames));
}

#[tokio::test]
async fn a_game_that_does_not_fit_the_held_screens_is_recorded_and_shows_a_warning() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    dump.send(three(2560, 1440));
    let store = storage::LocalStore::open(rig.dir.path())
        .await
        .expect("store");
    let mut old = crate::main_tests::test_match("g1", "victory");
    old.played_at = (chrono::Utc::now() - chrono::Duration::minutes(10)).into();
    store.insert_match(old).await.unwrap();
    dump.attach_bans_picks(&store, "g1").await;
    let rows = store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 1, "the game is recorded");
    assert_eq!(rows[0].bans_picks, None);
    assert_eq!(rows[0].bans_picks_status, Some(BansPicksStatus::NoFrames));
    assert_eq!(rig.health().as_deref(), Some(NO_SCREENS));
    assert_eq!(rig.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn a_later_capture_of_a_read_game_keeps_its_result_and_adds_no_warning() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (store, stored) = game_then_read(&rig, &mut dump, three(2560, 1440)).await;
    assert!(stored.is_some());
    dump.attach_bans_picks(&store, "g1").await;
    assert_eq!(rig.health(), None);
    // The next game has no screens of its own: that one is told.
    store
        .insert_match(crate::main_tests::test_match("g2", "defeat"))
        .await
        .unwrap();
    dump.attach_bans_picks(&store, "g2").await;
    assert_eq!(rig.health().as_deref(), Some(NO_SCREENS));
}

#[tokio::test]
async fn with_the_reader_off_a_game_without_screens_stays_silent() {
    let rig = Rig::new();
    let off = BansPicksFeed::for_config(false, BansPicksSetting::CvV1, rig.dir.path(), true);
    let mut dump = PregameDump::with_bans_picks(true, off, rig.dir.path(), POLL).expect("saver");
    let store = storage::LocalStore::open(rig.dir.path())
        .await
        .expect("store");
    store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    dump.attach_bans_picks(&store, "g1").await;
    assert_eq!(store.get_all_matches().await.unwrap().len(), 1);
    assert_eq!(rig.health(), None);
    assert_eq!(status_of(&store).await, Some(BansPicksStatus::ReaderOff));
}

// A game whose screens were read never shows the no-screens notice: see
// a_read_stores_the_result_on_the_game_and_clears_the_warning and
// a_later_capture_of_a_read_game_keeps_its_result_and_adds_no_warning.
#[tokio::test]
async fn the_no_pregame_screens_notice_is_shown_once_per_game() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    let (store, stored) = game_then_read(&rig, &mut dump, Vec::new()).await;
    assert_eq!(stored, None);
    assert_eq!(rig.health().as_deref(), Some(NO_SCREENS));
    // The notice is dismissed; a later capture of the same game does not
    // bring it back.
    publish_bans_picks_notice(rig.dir.path(), "");
    dump.attach_bans_picks(&store, "g1").await;
    assert_eq!(rig.health(), None, "not told again for the same game");
    // The next game without screens is told.
    store
        .insert_match(crate::main_tests::test_match("g2", "defeat"))
        .await
        .unwrap();
    dump.attach_bans_picks(&store, "g2").await;
    assert_eq!(rig.health().as_deref(), Some(NO_SCREENS));
    assert_eq!(rig.reads.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn the_does_not_fit_notice_is_shown_once_per_game() {
    let rig = Rig::new();
    let mut dump = rig.dump(false, rig.feed(Ok(Mode::Fine), true));
    dump.send(three(2560, 1440));
    let store = storage::LocalStore::open(rig.dir.path())
        .await
        .expect("store");
    let mut old = crate::main_tests::test_match("g1", "victory");
    old.played_at = (chrono::Utc::now() - chrono::Duration::minutes(10)).into();
    store.insert_match(old).await.unwrap();
    dump.attach_bans_picks(&store, "g1").await;
    assert_eq!(rig.health().as_deref(), Some(NO_SCREENS));
    assert_eq!(status_of(&store).await, Some(BansPicksStatus::NoFrames));
    // The notice is dismissed; a later capture of the same game does not
    // bring it back.
    publish_bans_picks_notice(rig.dir.path(), "");
    dump.attach_bans_picks(&store, "g1").await;
    assert_eq!(rig.health(), None, "not told again for the same game");
    // The next game that also does not fit the held screens is told.
    let mut next = crate::main_tests::test_match("g2", "defeat");
    next.played_at = (chrono::Utc::now() - chrono::Duration::minutes(10)).into();
    store.insert_match(next).await.unwrap();
    dump.attach_bans_picks(&store, "g2").await;
    assert_eq!(rig.health().as_deref(), Some(NO_SCREENS));
    assert_eq!(rig.reads.load(Ordering::SeqCst), 0);
}
