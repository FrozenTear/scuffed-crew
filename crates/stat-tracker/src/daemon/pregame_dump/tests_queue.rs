//! The queue title wiring end to end on injected reads: the saver holds
//! the frames, the game is recorded, the read lands on the record. No
//! failure stops the game from being recorded. Each game has its own
//! pre-game folder unless a test says otherwise.

use super::*;
use crate::daemon::queue_title::{PendingHint, QueueTitleFeed, ReadFn};
use image::DynamicImage;
use stat_tracker::model::{BansPicksStatus, QUEUE_TITLE_WARNING, QueueTitle};
use stat_tracker::shadow::queue_title::{Field, MIN_SCORE, Screen};
use std::sync::Mutex;

const POLL: Duration = Duration::from_millis(1_500);

/// The screens read and the width of each frame, in call order.
type Calls = Arc<Mutex<Vec<(Screen, u32)>>>;

/// Frame widths by slot, so a test sees which frame went where.
const BAN_LAST_W: u32 = 64;
const BAN_FIRST_W: u32 = 80;
const SELECT_LAST_W: u32 = 96;

/// A reader that gives `value` at `confidence` on hero ban frames and
/// nothing elsewhere; panics when `value` is `"panic"`.
fn reads(value: Option<&'static str>, confidence: f32, calls: &Calls) -> ReadFn {
    let calls = Arc::clone(calls);
    Arc::new(move |img, screen| {
        calls.lock().unwrap().push((screen, img.width()));
        if value == Some("panic") {
            panic!("reader failed");
        }
        if screen != Screen::HeroBan {
            return Field::none();
        }
        Field {
            value: value.map(str::to_string),
            confidence,
            bbox: None,
        }
    })
}

fn shot(folder: &str, name: &'static str, width: u32) -> PregameOp<Arc<DynamicImage>> {
    PregameOp::Save(PregameShot {
        folder: folder.to_string(),
        name,
        frame: Arc::new(DynamicImage::new_rgb8(width, 36)),
    })
}

/// A whole pre-game in `folder`, each frame its own size.
fn pregame(folder: &str) -> ShotBatch {
    vec![
        shot(folder, PregamePhase::MapVote.last_name(), 48),
        shot(folder, PregamePhase::HeroSelect.first_name(), 112),
        shot(folder, PregamePhase::HeroSelect.last_name(), SELECT_LAST_W),
        shot(folder, PregamePhase::HeroBan.first_name(), BAN_FIRST_W),
        shot(folder, PregamePhase::HeroBan.last_name(), BAN_LAST_W),
    ]
}

/// The folder of game `sid`.
fn folder_of(sid: &str) -> String {
    format!("20261103_1200_{sid}")
}

struct Rig {
    dir: tempfile::TempDir,
    dump: PregameDump,
    store: storage::LocalStore,
}

impl Rig {
    /// The saver as the daemon builds it with every other switch off.
    async fn new(read: ReadFn) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let dump = Self::saver(dir.path(), read);
        let store = storage::LocalStore::open(dir.path()).await.expect("store");
        Self { dir, dump, store }
    }

    fn saver(dir: &std::path::Path, read: ReadFn) -> PregameDump {
        assert!(
            PregameDump::with_readers(false, None, None, dir, POLL).is_none(),
            "no saver with everything off"
        );
        PregameDump::with_readers(
            false,
            None,
            Some(QueueTitleFeed::with_read(read)),
            dir,
            POLL,
        )
        .expect("a saver for the queue title reader")
    }

    async fn record(&self, sid: &str) {
        self.store
            .insert_match(crate::main_tests::test_match(sid, "victory"))
            .await
            .unwrap();
    }

    async fn title(&self, sid: &str) -> Option<QueueTitle> {
        let rows = self.store.get_session_snapshots(sid).await.unwrap();
        assert!(
            !rows.is_empty(),
            "the game is recorded whatever the read did"
        );
        let t = rows[0].queue_title.clone();
        assert!(rows.iter().all(|r| r.queue_title == t), "on every snapshot");
        t
    }

    /// The pre-game in the game's own folder (when `shots`), the recorded
    /// game, then the read.
    async fn game(&mut self, sid: &str, shots: bool) -> Option<QueueTitle> {
        if shots {
            self.dump.send(pregame(&folder_of(sid)));
        }
        self.record(sid).await;
        self.dump
            .attach_queue_title(&self.store, sid, Some(sid))
            .await;
        self.title(sid).await
    }

    fn pending(&self) -> Option<PendingHint> {
        self.dump.queue.as_ref().unwrap().pending_hint.clone()
    }
}

#[tokio::test]
async fn a_confident_read_sets_the_queue_on_the_record() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    let t = rig.game("g1", true).await.expect("a queue title");
    assert_eq!(t.value.as_deref(), Some("OPEN QUEUE"));
    assert_eq!(t.confidence, 0.93);
    assert_eq!(t.warning(), None);
    // Hero ban last is read first and wins: one call, on that frame.
    assert_eq!(*calls.lock().unwrap(), [(Screen::HeroBan, BAN_LAST_W)]);
    // A later capture of the game is not read again and carries the title.
    rig.record("g1").await;
    rig.dump
        .attach_queue_title(&rig.store, "g1", Some("g1"))
        .await;
    assert_eq!(rig.title("g1").await, Some(t));
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_read_exactly_at_the_bar_is_confident() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("ROLE QUEUE"), MIN_SCORE, &calls)).await;
    let t = rig.game("g1", true).await.expect("a queue title");
    assert_eq!((t.value.as_deref(), t.unsure), (Some("ROLE QUEUE"), false));
}

#[tokio::test]
async fn a_weak_read_warns_on_the_game_with_no_queue() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("ROLE QUEUE"), 0.4, &calls)).await;
    let t = rig.game("g1", true).await.expect("a queue title");
    assert_eq!(t.warning(), Some(QUEUE_TITLE_WARNING));
    assert_eq!((t.value, t.unsure), (None, true));
    // All three policy frames were tried, each from its own slot, the hero
    // ban ones first.
    assert_eq!(
        *calls.lock().unwrap(),
        [
            (Screen::HeroBan, BAN_LAST_W),
            (Screen::HeroBan, BAN_FIRST_W),
            (Screen::HeroSelect, SELECT_LAST_W),
        ]
    );
    // The game list sees the warning.
    let rows = rig.store.get_session_snapshots("g1").await.unwrap();
    let game = stat_tracker::games::model::Game::from_match(&rows[0]);
    assert_eq!(
        game.queue_title.and_then(|t| t.warning()),
        Some(QUEUE_TITLE_WARNING)
    );
}

#[tokio::test]
async fn a_restart_mid_game_keeps_the_confident_title() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    let before = rig.game("g1", true).await.expect("a queue title");
    // A fresh saver and feed on the same store: the resumed game's next Tab
    // finds no frames held.
    rig.dump = Rig::saver(rig.dir.path(), reads(Some("OPEN QUEUE"), 0.93, &calls));
    rig.record("g1").await;
    rig.dump
        .attach_queue_title(&rig.store, "g1", Some("g1"))
        .await;
    let after = rig.title("g1").await.expect("a queue title");
    assert_eq!(after, before);
    assert_eq!(after.warning(), None);
    assert_eq!(calls.lock().unwrap().len(), 1, "not read again");
}

#[tokio::test]
async fn a_late_frame_of_a_read_game_is_not_read_for_the_next_game() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    rig.game("g1", true).await.expect("a queue title");
    // The hero select's last frame of g1 comes after g1 was read, and g2
    // opens no folder of its own.
    rig.dump.send(vec![
        shot(
            &folder_of("g1"),
            PregamePhase::HeroSelect.last_name(),
            SELECT_LAST_W,
        ),
        shot(
            &folder_of("g1"),
            PregamePhase::HeroBan.last_name(),
            BAN_LAST_W,
        ),
    ]);
    let t = rig.game("g2", false).await.expect("a queue title");
    assert_eq!(t.warning(), Some(QUEUE_TITLE_WARNING));
    assert_eq!((t.value, t.unsure), (None, true));
    assert_eq!(calls.lock().unwrap().len(), 1, "only g1 was read");
    // A new folder is read again.
    let t = rig.game("g3", true).await.expect("a queue title");
    assert_eq!(t.value.as_deref(), Some("OPEN QUEUE"));
    assert_eq!(calls.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn a_new_folder_drops_the_last_games_frames() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.4, &calls)).await;
    rig.dump.send(pregame(&folder_of("g0")));
    // g1's folder opens with its map vote only: g0's frames are gone.
    rig.dump.send(vec![shot(
        &folder_of("g1"),
        PregamePhase::MapVote.last_name(),
        48,
    )]);
    let t = rig.game("g1", false).await.expect("a queue title");
    assert_eq!((t.value, t.unsure), (None, true));
    assert!(
        calls.lock().unwrap().is_empty(),
        "nothing held, nothing read"
    );
}

#[tokio::test]
async fn a_game_that_does_not_fit_the_held_screens_is_recorded_and_shows_a_warning() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    rig.dump.send(pregame(&folder_of("g1")));
    let mut old = crate::main_tests::test_match("g1", "victory");
    old.played_at = (chrono::Utc::now() - chrono::Duration::minutes(10)).into();
    rig.store.insert_match(old).await.unwrap();
    rig.dump
        .attach_queue_title(&rig.store, "g1", Some("g1"))
        .await;
    let t = rig.title("g1").await.expect("a queue title");
    assert_eq!((t.value, t.unsure), (None, true));
    assert!(calls.lock().unwrap().is_empty(), "the frames are not read");
}

#[tokio::test]
async fn a_capture_of_another_session_leaves_the_frames_for_the_open_game() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    rig.dump.send(pregame(&folder_of("g2")));
    // A Tab stored under a session that is not the open game.
    rig.record("probe").await;
    rig.dump
        .attach_queue_title(&rig.store, "probe", Some("g2"))
        .await;
    assert_eq!(rig.title("probe").await, None);
    assert!(calls.lock().unwrap().is_empty());
    // The open game reads its own frames.
    rig.record("g2").await;
    rig.dump
        .attach_queue_title(&rig.store, "g2", Some("g2"))
        .await;
    let t = rig.title("g2").await.expect("a queue title");
    assert_eq!(t.value.as_deref(), Some("OPEN QUEUE"));
}

#[tokio::test]
async fn a_panicking_reader_or_no_frames_still_records_the_game_with_the_warning() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("panic"), 0.9, &calls)).await;
    let t = rig.game("g1", true).await.expect("a queue title");
    assert_eq!((t.value, t.unsure), (None, true));
    // The next game had no pre-game screens at all.
    let t = rig.game("g2", false).await.expect("a queue title");
    assert_eq!((t.value, t.unsure), (None, true));
    assert_eq!(calls.lock().unwrap().len(), 1, "no frame, no read");
}

#[tokio::test]
async fn the_next_game_confirms_drops_or_expires_the_hint() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    rig.dump.set_next_mode_hint(Some(" open "), "g0");
    let t = rig.game("g1", true).await.unwrap();
    assert_eq!(t.mode_hint.as_deref(), Some("open"));
    assert_eq!(rig.pending(), None);

    rig.dump.set_next_mode_hint(Some("ROLE QUEUE"), "g1");
    let t = rig.game("g2", true).await.unwrap();
    assert_eq!(t.mode_hint, None, "a disagreeing hint is dropped");
    assert_eq!(rig.pending(), None);

    // Changed on purpose: a weak read (no frames) used to leave the hint
    // for a later game. It is the next game's hint only, so it expires.
    rig.dump.set_next_mode_hint(Some("open"), "g2");
    let t = rig.game("g3", false).await.unwrap();
    assert_eq!((t.mode_hint, t.unsure), (None, true));
    assert_eq!(rig.pending(), None);
    let t = rig.game("g4", true).await.unwrap();
    assert_eq!(t.mode_hint, None, "the expired hint is not g4's");

    // A new hint replaces a pending one; a blank one clears it.
    rig.dump.set_next_mode_hint(Some("open"), "g4");
    rig.dump.set_next_mode_hint(Some("role"), "g4");
    assert_eq!(rig.pending().map(|h| h.text).as_deref(), Some("role"));
    rig.dump.set_next_mode_hint(Some("  "), "g4");
    assert_eq!(rig.pending(), None);
}

#[tokio::test]
async fn the_source_game_never_settles_its_own_hint() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    // g1's rank screen is read before g1 is first recorded.
    rig.dump.send(pregame(&folder_of("g1")));
    rig.dump.set_next_mode_hint(Some("open"), "g1");
    rig.record("g1").await;
    rig.dump
        .attach_queue_title(&rig.store, "g1", Some("g1"))
        .await;
    let t = rig.title("g1").await.expect("a queue title");
    assert_eq!(
        (t.value.as_deref(), t.mode_hint),
        (Some("OPEN QUEUE"), None)
    );
    let pending = rig.pending().expect("still pending");
    assert_eq!(
        (pending.text.as_str(), pending.source.as_str()),
        ("open", "g1")
    );
    // The game after it settles it.
    let t = rig.game("g2", true).await.unwrap();
    assert_eq!(t.mode_hint.as_deref(), Some("open"));
}

#[tokio::test]
async fn with_the_reader_off_the_saver_and_the_game_are_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let none = PregameDump::with_readers(false, None, None, dir.path(), POLL);
    assert!(none.is_none(), "off starts no saver");
    let mut dump =
        PregameDump::with_readers(true, None, None, dir.path(), POLL).expect("the saves' saver");
    assert!(dump.queue.is_none());
    assert!(!dump.queue_only);
    dump.set_next_mode_hint(Some("open"), "g0");
    dump.send(pregame(&folder_of("g1")));
    let store = storage::LocalStore::open(dir.path()).await.expect("store");
    store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    dump.attach_queue_title(&store, "g1", Some("g1")).await;
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].queue_title, None);
    assert!(dump.queue.is_none());
}

#[tokio::test]
async fn a_saver_for_the_queue_reader_alone_writes_nothing_and_wants_no_checks() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    assert!(rig.dump.queue_only);
    assert!(!rig.dump.writer.disk());
    assert_eq!(rig.dump.want(Instant::now()), PregameWant::default());
    // The writer keeps frames in memory only (`start(true, ..)` would write
    // them): the disk check above is what pins that; this one is a backstop.
    rig.dump.send(pregame(&folder_of("g1")));
    assert!(
        !pregame_dir(rig.dir.path()).join(folder_of("g1")).exists(),
        "nothing on disk"
    );
    rig.record("g1").await;
    rig.dump.attach_bans_picks(&rig.store, "g1").await;
    rig.dump
        .attach_queue_title(&rig.store, "g1", Some("g1"))
        .await;
    let rows = rig.store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows[0].bans_picks_status, Some(BansPicksStatus::ReaderOff));
    assert!(
        !rig.dir.path().join(trail::SUMMARY_FILE).exists(),
        "no pre-game summary for the bans and picks reader that is off"
    );
    let t = rig.title("g1").await.expect("a queue title");
    assert_eq!(t.value.as_deref(), Some("OPEN QUEUE"));
}
