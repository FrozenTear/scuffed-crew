//! The queue title wiring end to end on injected reads: the saver holds
//! the frames, the game is recorded, the read lands on the record. No
//! failure stops the game from being recorded.

use super::*;
use crate::daemon::queue_title::{QueueTitleFeed, ReadFn};
use image::DynamicImage;
use stat_tracker::model::{QUEUE_TITLE_WARNING, QueueTitle};
use stat_tracker::shadow::queue_title::{Field, Screen};
use std::sync::Mutex;

const POLL: Duration = Duration::from_millis(1_500);
const FOLDER: &str = "20261103_120000";

/// The screens read, in call order.
type Calls = Arc<Mutex<Vec<Screen>>>;

/// A reader that gives `value` at `confidence` on hero ban frames and
/// nothing elsewhere; panics when `value` is `"panic"`.
fn reads(value: Option<&'static str>, confidence: f32, calls: &Calls) -> ReadFn {
    let calls = Arc::clone(calls);
    Arc::new(move |_, screen| {
        calls.lock().unwrap().push(screen);
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

fn pregame() -> ShotBatch {
    let f = Arc::new(DynamicImage::new_rgb8(64, 36));
    [
        PregamePhase::MapVote.last_name(),
        PregamePhase::HeroSelect.first_name(),
        PregamePhase::HeroSelect.last_name(),
        PregamePhase::HeroBan.first_name(),
        PregamePhase::HeroBan.last_name(),
    ]
    .into_iter()
    .map(|name| {
        PregameOp::Save(PregameShot {
            folder: FOLDER.to_string(),
            name,
            frame: Arc::clone(&f),
        })
    })
    .collect()
}

struct Rig {
    _dir: tempfile::TempDir,
    dump: PregameDump,
    store: storage::LocalStore,
}

impl Rig {
    /// The saver as the daemon builds it with every other switch off.
    async fn new(read: ReadFn) -> Self {
        let dir = tempfile::tempdir().expect("tempdir");
        let mut dump = PregameDump::with_bans_picks(false, None, dir.path(), POLL);
        assert!(dump.is_none(), "no saver with everything off");
        PregameDump::add_queue_title(
            &mut dump,
            Some(QueueTitleFeed::with_read(read)),
            dir.path(),
            POLL,
        );
        let dump = dump.expect("a saver for the queue title reader");
        let store = storage::LocalStore::open(dir.path()).await.expect("store");
        Self {
            _dir: dir,
            dump,
            store,
        }
    }

    /// The pre-game (when `shots`), the recorded game, then the read.
    async fn game(&mut self, sid: &str, shots: bool) -> Option<QueueTitle> {
        if shots {
            self.dump.send(pregame());
        }
        self.store
            .insert_match(crate::main_tests::test_match(sid, "victory"))
            .await
            .unwrap();
        self.dump.attach_queue_title(&self.store, sid).await;
        let rows = self.store.get_session_snapshots(sid).await.unwrap();
        assert_eq!(rows.len(), 1, "the game is recorded whatever the read did");
        rows[0].queue_title.clone()
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
    // Hero ban last is read first and wins: one call.
    assert_eq!(*calls.lock().unwrap(), [Screen::HeroBan]);
    // A later capture of the game is not read again and carries the title.
    rig.store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    rig.dump.attach_queue_title(&rig.store, "g1").await;
    let rows = rig.store.get_session_snapshots("g1").await.unwrap();
    assert!(rows.iter().all(|r| r.queue_title.as_ref() == Some(&t)));
    assert_eq!(calls.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn a_weak_read_keeps_the_queue_and_warns_on_the_game() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("ROLE QUEUE"), 0.4, &calls)).await;
    rig.store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    let before = QueueTitle {
        value: Some("OPEN QUEUE".into()),
        confidence: 0.9,
        ..QueueTitle::default()
    };
    rig.store.attach_queue_title("g1", &before).await.unwrap();
    rig.dump.send(pregame());
    rig.dump.attach_queue_title(&rig.store, "g1").await;
    let rows = rig.store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows.len(), 1);
    let t = rows[0].queue_title.clone().expect("a queue title");
    assert_eq!(
        (t.value.as_deref(), t.confidence),
        (Some("OPEN QUEUE"), 0.9)
    );
    assert_eq!(t.warning(), Some(QUEUE_TITLE_WARNING));
    // All three policy frames were tried, the hero ban ones first.
    assert_eq!(
        *calls.lock().unwrap(),
        [Screen::HeroBan, Screen::HeroBan, Screen::HeroSelect]
    );
    // The game list sees the warning.
    let game = stat_tracker::games::model::Game::from_match(&rows[0]);
    assert_eq!(
        game.queue_title.and_then(|t| t.warning()),
        Some(QUEUE_TITLE_WARNING)
    );
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
async fn the_pending_hint_is_confirmed_dropped_or_kept() {
    let calls = Calls::default();
    let mut rig = Rig::new(reads(Some("OPEN QUEUE"), 0.93, &calls)).await;
    rig.dump.set_next_mode_hint(Some(" open "));
    let t = rig.game("g1", true).await.unwrap();
    assert_eq!(t.mode_hint.as_deref(), Some("open"));
    assert_eq!(rig.dump.queue.as_ref().unwrap().pending_hint, None);

    rig.dump.set_next_mode_hint(Some("ROLE QUEUE"));
    let t = rig.game("g2", true).await.unwrap();
    assert_eq!(t.mode_hint, None, "a disagreeing hint is dropped");
    assert_eq!(rig.dump.queue.as_ref().unwrap().pending_hint, None);

    // A weak read (no frames) leaves the hint for the next game.
    rig.dump.set_next_mode_hint(Some("open"));
    let t = rig.game("g3", false).await.unwrap();
    assert_eq!((t.mode_hint, t.unsure), (None, true));
    let pending = rig.dump.queue.as_ref().unwrap().pending_hint.clone();
    assert_eq!(pending.as_deref(), Some("open"));
    rig.dump.set_next_mode_hint(Some("  "));
    assert_eq!(rig.dump.queue.as_ref().unwrap().pending_hint, None);
}

#[tokio::test]
async fn with_the_reader_off_the_saver_and_the_game_are_unchanged() {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut none = PregameDump::with_bans_picks(false, None, dir.path(), POLL);
    PregameDump::add_queue_title(&mut none, None, dir.path(), POLL);
    assert!(none.is_none(), "off starts no saver");
    let mut on = PregameDump::with_bans_picks(true, None, dir.path(), POLL);
    PregameDump::add_queue_title(&mut on, None, dir.path(), POLL);
    let mut dump = on.expect("the saves' own saver");
    assert!(dump.queue.is_none());
    dump.set_next_mode_hint(Some("open"));
    dump.send(pregame());
    let store = storage::LocalStore::open(dir.path()).await.expect("store");
    store
        .insert_match(crate::main_tests::test_match("g1", "victory"))
        .await
        .unwrap();
    dump.attach_queue_title(&store, "g1").await;
    let rows = store.get_session_snapshots("g1").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].queue_title, None);
    assert!(dump.queue.is_none());
}
