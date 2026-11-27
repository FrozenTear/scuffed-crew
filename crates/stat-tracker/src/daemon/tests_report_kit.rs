//! Tests for the report kit's diagnostics (#811), next to `debug_dumps.rs`:
//! an accepted board whose team size the blanker has no layout for still
//! leaves its reads and a warning, and a kit write that fails is a warning
//! with the session id, not a debug line the default log level drops.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use stat_tracker::report::blank::{CaptureTime, TeamLayout};
use stat_tracker::report::kit;

use crate::{main_tests::*, *};

/// What the daemon logged at one level or above, for the tests that read
/// its log back. [`Logged::warnings`] keeps WARN and above only, so a test
/// that reads a warning there fails when the line is logged below WARN.
#[derive(Clone)]
pub(super) struct Logged {
    lines: Arc<Mutex<Vec<u8>>>,
    level: tracing::Level,
}

impl std::io::Write for Logged {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        self.lines.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Logged {
    type Writer = Logged;
    fn make_writer(&'a self) -> Logged {
        self.clone()
    }
}

impl Logged {
    /// Keeps the lines at `level` and above.
    pub(super) fn at(level: tracing::Level) -> Self {
        Self {
            lines: Arc::default(),
            level,
        }
    }

    pub(super) fn warnings() -> Self {
        Self::at(tracing::Level::WARN)
    }

    pub(super) fn subscriber(&self) -> impl tracing::Subscriber + Send + Sync + 'static {
        tracing_subscriber::fmt()
            .with_writer(self.clone())
            .with_ansi(false)
            .with_max_level(self.level)
            .finish()
    }

    /// Run `f` with this log as the thread's subscriber.
    ///
    /// A no-op dispatcher is kept for the whole test run first. While only
    /// one dispatcher exists, tracing decides a new callsite's interest from
    /// the default of the thread that hits it first, so a parallel test with
    /// no subscriber can cache a callsite as off for this log too. With two
    /// or more, every callsite is decided over all of them under a lock.
    pub(super) fn during<T>(&self, f: impl FnOnce() -> T) -> T {
        static KEEP_TWO: std::sync::OnceLock<tracing::Dispatch> = std::sync::OnceLock::new();
        KEEP_TWO
            .get_or_init(|| tracing::Dispatch::new(tracing::subscriber::NoSubscriber::default()));
        tracing::subscriber::with_default(self.subscriber(), || {
            tracing::callsite::rebuild_interest_cache();
            f()
        })
    }

    /// Every line kept.
    pub(super) fn all(&self) -> Vec<String> {
        let text = String::from_utf8(self.lines.lock().unwrap().clone()).unwrap();
        text.lines().map(str::to_string).collect()
    }

    /// The WARN lines that carry `message`.
    pub(super) fn warned(&self, message: &str) -> Vec<String> {
        self.all()
            .into_iter()
            .filter(|l| l.contains(" WARN ") && l.contains(message))
            .collect()
    }
}

const REFUSED_SIZE: &str = "report kit: no layout for this team size";
const BOARD_NOT_WRITTEN: &str = "report kit: scoreboard and reads not written";
const NAMES_NOT_WRITTEN: &str = "report kit: names not written";
const RANK_NOT_WRITTEN: &str = "report kit: rank screen crops not written";

/// A flat dark 1080p frame. The blanker keeps no picture of it, so the kit
/// write is the reads alone: enough for what these tests check.
pub(super) fn frame() -> image::DynamicImage {
    image::DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
        1920,
        1080,
        image::Rgb([24, 20, 18]),
    ))
}

/// [`record_report_kit`] for one accepted board of `team_size`, run to the
/// end of its blocking write.
async fn accepted_board(data_dir: &std::path::Path, session_id: &str, team_size: usize) {
    let row = test_match(session_id, "unknown");
    record_report_kit(
        data_dir,
        session_id,
        &frame(),
        team_size,
        &row,
        &row,
        Utc::now(),
        CaptureTime::now(),
        Vec::new(),
        &BTreeMap::new(),
    )
    .await
    .expect("the kit write ran to its end");
}

pub(super) fn kit_reads(session_id: &str) -> kit::KitReads {
    let row = test_match(session_id, "unknown");
    kit::KitReads::from_capture(&row, &row, 1920, 1080, TeamLayout::Six, Utc::now())
}

/// `report-kit` is a file, so no kit folder can be made under it: every
/// kit write for `data_dir` fails, whoever runs the test.
fn block_the_kit_root(data_dir: &std::path::Path) {
    std::fs::write(kit::kit_root(data_dir), b"in the way").unwrap();
}

/// A game with a stored board, active now.
fn recorded_game(session_id: &str) -> ActiveGame {
    let mut game = ActiveGame::open_now(
        session_id.to_string(),
        detect::MatchOutcome::Defeat,
        Vec::new(),
    );
    game.session_created = true;
    game
}

pub(super) fn rank_pngs() -> kit::RankScreenPngs {
    kit::RankScreenPngs {
        outcome: b"outcome".to_vec(),
        rank: Some(b"rank".to_vec()),
        rank_banner: Some(b"banner".to_vec()),
    }
}

/// #811: `record_report_kit` returned with no log and no file when the
/// team size was not 5 or 6. The board was accepted and stored, so its
/// reads are kept (no picture, unknown layout) and the size is logged.
#[tokio::test]
async fn an_accepted_board_with_a_refused_team_size_leaves_its_reads_and_a_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Logged::warnings();
    // The warning is logged on this thread, before the blocking write.
    let _sub = tracing::subscriber::set_default(log.subscriber());
    tracing::callsite::rebuild_interest_cache();

    accepted_board(tmp.path(), "seven", 7).await;

    let saved = kit::load(tmp.path(), "seven");
    let reads = saved
        .reads
        .expect("the reads of an accepted board are saved whatever its team size");
    assert_eq!(reads.team_size, None, "a refused size is an unknown layout");
    assert_eq!(reads.reads["e"].value.as_deref(), Some("10"));
    assert!(
        saved.scoreboard_png.is_none(),
        "no picture without a layout"
    );
    assert!(kit::has_reads(tmp.path(), "seven"));

    let warned = log.warned(REFUSED_SIZE);
    assert_eq!(
        warned.len(),
        1,
        "one warning for the refused size: {warned:?}"
    );
    assert!(
        warned[0].contains("team_size=7") && warned[0].contains("session_id=\"seven\""),
        "the warning names the size and the session: {}",
        warned[0]
    );
}

/// The sizes the blanker knows are written as before, with no warning.
#[tokio::test]
async fn an_accepted_5v5_or_6v6_board_leaves_its_reads_and_no_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Logged::warnings();
    let _sub = tracing::subscriber::set_default(log.subscriber());
    tracing::callsite::rebuild_interest_cache();

    for (session, size) in [("five", 5), ("six", 6)] {
        accepted_board(tmp.path(), session, size).await;
        let reads = kit::load(tmp.path(), session).reads.expect("reads saved");
        assert_eq!(reads.team_size, Some(size as u32), "{session}");
    }
    assert!(log.all().is_empty(), "no warning: {:?}", log.all());
}

/// #811: a failed write of the board and its reads was a debug line.
#[test]
fn a_failed_board_write_is_a_warning_with_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    block_the_kit_root(tmp.path());
    let log = Logged::warnings();
    log.during(|| {
        write_report_kit(
            tmp.path(),
            "blocked",
            &frame(),
            Some(6),
            &kit_reads("blocked"),
            CaptureTime::now(),
            &[],
        );
    });
    assert!(
        !kit::has_reads(tmp.path(), "blocked"),
        "nothing was written"
    );
    let warned = log.warned(BOARD_NOT_WRITTEN);
    assert_eq!(
        warned.len(),
        1,
        "one warning for the failed write: {warned:?}"
    );
    assert!(
        warned[0].contains("session_id=\"blocked\"") && warned[0].contains("error="),
        "the warning names the session and the error: {}",
        warned[0]
    );
}

/// #811: a failed write of the board's names was a debug line too. The
/// reads of the same board are written, so only the names line is logged.
#[test]
fn a_failed_names_write_is_a_warning_with_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    // `names.json` is a folder: the names file cannot be renamed over it.
    let dir = kit::session_dir(tmp.path(), "names").unwrap();
    std::fs::create_dir_all(dir.join(kit::NAMES_JSON)).unwrap();
    let log = Logged::warnings();
    log.during(|| {
        write_report_kit(
            tmp.path(),
            "names",
            &frame(),
            Some(6),
            &kit_reads("names"),
            CaptureTime::now(),
            &["Somebody".to_string()],
        );
    });
    assert!(
        kit::has_reads(tmp.path(), "names"),
        "the reads were written"
    );
    assert!(kit::names(tmp.path(), "names").is_empty());
    let warned = log.warned(NAMES_NOT_WRITTEN);
    assert_eq!(
        warned.len(),
        1,
        "one warning for the failed write: {warned:?}"
    );
    assert!(
        warned[0].contains("session_id=\"names\"") && warned[0].contains("error="),
        "the warning names the session and the error: {}",
        warned[0]
    );
    assert_eq!(log.all().len(), 1, "and nothing else: {:?}", log.all());
}

/// A board with no names writes no names file and logs nothing.
#[test]
fn a_written_kit_logs_no_warning() {
    let tmp = tempfile::tempdir().unwrap();
    let log = Logged::warnings();
    log.during(|| {
        write_report_kit(
            tmp.path(),
            "fine",
            &frame(),
            Some(6),
            &kit_reads("fine"),
            CaptureTime::now(),
            &["Somebody".to_string()],
        );
    });
    assert!(kit::has_reads(tmp.path(), "fine"));
    assert_eq!(kit::names(tmp.path(), "fine"), vec!["Somebody"]);
    assert!(log.all().is_empty(), "no warning: {:?}", log.all());
}

/// #811: a failed write of the rank screen crops was a debug line.
#[test]
fn a_failed_rank_crop_write_is_a_warning_with_the_session() {
    let tmp = tempfile::tempdir().unwrap();
    block_the_kit_root(tmp.path());
    let game = recorded_game("ranked");
    let log = Logged::warnings();
    log.during(|| record_rank_crops(tmp.path(), Some(&game), Some(&rank_pngs())));
    let warned = log.warned(RANK_NOT_WRITTEN);
    assert_eq!(
        warned.len(),
        1,
        "one warning for the failed write: {warned:?}"
    );
    assert!(
        warned[0].contains("session_id=ranked") && warned[0].contains("error="),
        "the warning names the session and the error: {}",
        warned[0]
    );
}

/// The crops go to the open game while it is recent, as before the write
/// moved out of the poll loop: no tick without crops, no open game and no
/// game quiet past [`RANK_OUTCOME_MAX_IDLE`] writes anything.
#[test]
fn rank_crops_go_to_the_open_game_only_while_it_is_recent() {
    let tmp = tempfile::tempdir().unwrap();
    let files = |session: &str| {
        let dir = kit::session_dir(tmp.path(), session).unwrap();
        [kit::OUTCOME_PNG, kit::RANK_PNG, kit::RANK_BANNER_PNG].map(|name| dir.join(name).is_file())
    };
    let log = Logged::warnings();
    log.during(|| {
        let game = recorded_game("recent");
        record_rank_crops(tmp.path(), Some(&game), None);
        assert_eq!(files("recent"), [false; 3], "no crops this tick");
        record_rank_crops(tmp.path(), None, Some(&rank_pngs()));
        assert!(!kit::kit_root(tmp.path()).exists(), "no open game");

        let mut quiet = recorded_game("quiet");
        quiet.last_activity = Instant::now()
            .checked_sub(RANK_OUTCOME_MAX_IDLE + Duration::from_secs(60))
            .expect("a past instant");
        record_rank_crops(tmp.path(), Some(&quiet), Some(&rank_pngs()));
        assert_eq!(files("quiet"), [false; 3], "quiet past the idle limit");

        record_rank_crops(tmp.path(), Some(&game), Some(&rank_pngs()));
        assert_eq!(files("recent"), [true; 3]);
    });
    assert!(log.all().is_empty(), "no warning: {:?}", log.all());
}

/// The daemon's delete path (#811): a game deleted in the app reaches the
/// daemon as a queued `DeleteSession`. Its kit goes, and the record and
/// the INFO line give the reason "game deleted", not "game report sent"
/// (the app's reason for a kit it removed after a sent report). The INFO
/// line itself is read in `report::kit::store`'s removal tests.
#[tokio::test]
async fn a_queued_delete_removes_the_kit_and_records_a_deleted_game() {
    let dir = tempfile::tempdir().unwrap();
    write_report_kit(
        dir.path(),
        "delete-me",
        &frame(),
        Some(6),
        &kit_reads("delete-me"),
        CaptureTime::now(),
        &[],
    );
    assert!(kit::has_reads(dir.path(), "delete-me"));
    storage::queue_command(
        dir.path(),
        &storage::StoreCommand::DeleteSession {
            session_id: "delete-me".into(),
        },
    )
    .unwrap();

    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut st = session(None, test_now());
    let backoff = Arc::new(Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = None;
    apply_queued_commands(&mut st, &store, dir.path(), &mut sync_task, &backoff, None).await;

    assert!(
        !kit::has_reads(dir.path(), "delete-me"),
        "the kit is deleted"
    );
    let record = dir.path().join("debug").join(kit::REMOVED_KITS_JSONL);
    let lines: Vec<serde_json::Value> = std::fs::read_to_string(&record)
        .unwrap()
        .lines()
        .map(|line| serde_json::from_str(line).unwrap())
        .collect();
    assert_eq!(lines.len(), 1, "{lines:?}");
    assert_eq!(lines[0]["session_id"], "delete-me");
    assert_eq!(lines[0]["why"], "game deleted");
    assert_eq!(lines[0]["had_kit"], true);
}
