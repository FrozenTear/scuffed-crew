//! Pre-game saver tests for frames staged on disk until they are known to
//! be saved: a lone ban-results frame, and what is left on disk after each
//! way the ban results end.

use super::pregame_dump::*;
use super::tests_pregame_dump::Run;
use crate::*;
use std::time::Duration;

const BAN: Seen = Seen::Phase(PregamePhase::HeroBan);
const SELECT: Seen = Seen::Phase(PregamePhase::HeroSelect);
const RESULTS: Seen = Seen::BanResults;
const POLL: Duration = Duration::from_millis(1_500);

fn no_ban_results(run: &Run) -> bool {
    run.names().iter().all(|(n, _)| *n != BAN_RESULTS_NAME) && run.staged.is_empty()
}

#[test]
fn a_lone_results_frame_then_the_select_read_is_not_saved() {
    // Chosen: dropped. The phase read lags a poll, so a lone frame right
    // before it cannot be told from the select sliding in under the
    // results title (the PR 579 lag case), and nothing confirmed it.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.tick(4_500, SELECT, 4);
    assert!(no_ban_results(&run), "{:?}", run.names());
}

#[test]
fn a_lone_results_frame_then_the_game_closing_is_not_saved() {
    // Chosen: dropped. A release confirms nothing (finding 3), and the
    // tick after the frame never came, so it may be the select's first.
    let mut run = Run::new();
    run.tick(0, BAN, 1);
    run.tick(1_500, BAN, 2);
    run.tick(3_000, RESULTS, 3);
    run.release(4_000);
    assert!(!run.tracker.holds_frame());
    assert!(no_ban_results(&run), "{:?}", run.names());
}

/// A plain frame `w` pixels wide, so a saved file tells which it was.
fn frame(w: u32) -> Arc<image::DynamicImage> {
    Arc::new(image::DynamicImage::ImageRgb8(image::RgbImage::new(w, 4)))
}

/// How the ban results end, after the ban and `results` frames (widths
/// 10, 11, ...) at 1.5 s polls from 3 s on.
#[derive(Debug, Clone, Copy)]
enum End {
    SelectTitle,
    SelectRead,
    Release,
    BanBack,
    /// No next screen inside the window.
    WindowOut,
}

/// Every file in the one game folder, dot names too, and the width of the
/// saved ban results, if any.
async fn on_disk(results: u32, end: End) -> (Vec<String>, Option<u32>) {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    dump.observe(at(0), BAN, Some(frame(64)));
    dump.observe(at(1_500), BAN, Some(frame(64)));
    let mut ms = 3_000;
    for k in 0..results {
        dump.observe(at(ms), RESULTS, Some(frame(10 + k)));
        ms += 1_500;
    }
    match end {
        End::SelectTitle => dump.observe(at(ms), Seen::SelectTitle, None),
        End::SelectRead => dump.observe(at(ms), SELECT, Some(frame(70))),
        End::Release => dump.release(at(ms)),
        End::BanBack => dump.observe(at(ms), BAN, Some(frame(64))),
        End::WindowOut => dump.observe(at(30_000), Seen::Other, None),
    }
    assert!(dump.finish(Duration::from_secs(30)).await);
    let root = pregame_dir(dir.path());
    let games: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    assert_eq!(games.len(), 1);
    let mut files: Vec<String> = std::fs::read_dir(&games[0])
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    files.sort();
    let saved = image::open(games[0].join(BAN_RESULTS_NAME))
        .ok()
        .map(|i| i.width());
    (files, saved)
}

#[tokio::test]
async fn no_staged_frame_is_left_after_any_end_of_the_ban_results() {
    use End::*;
    // (results frames, end, saved width): the held frame when the select
    // title confirms it, the staged one before it on a select read or a
    // release, nothing when the ban comes back or the window runs out, and
    // a lone frame only with the select title.
    let cases = [
        (2, SelectTitle, Some(11)),
        (2, SelectRead, Some(10)),
        (2, Release, Some(10)),
        (2, BanBack, None),
        (2, WindowOut, None),
        (1, SelectTitle, Some(10)),
        (1, SelectRead, None),
        (1, Release, None),
    ];
    for (results, end, want) in cases {
        let (files, saved) = on_disk(results, end).await;
        assert_eq!(saved, want, "{results} results, {end:?}");
        assert!(
            files.iter().all(|f| !f.starts_with('.')),
            "{results} results, {end:?}: {files:?}"
        );
    }
}

#[tokio::test]
async fn stale_staged_frames_are_swept_and_never_count_as_games() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    let old = root.join("20261001_000000");
    std::fs::create_dir_all(&old).unwrap();
    let stale = [
        staged_name(BAN_RESULTS_NAME),
        staged_name(LOADING_NAME),
        format!(".{}.tmp", staged_name(BAN_RESULTS_NAME)),
    ];
    for name in &stale {
        std::fs::write(old.join(name), b"x").unwrap();
    }
    std::fs::write(old.join("notes.txt"), b"mine").unwrap();
    // At start: a run that ended mid-game left them.
    let dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    assert!(dump.finish(Duration::from_secs(30)).await);
    let mut left: Vec<String> = std::fs::read_dir(&old)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    left.sort();
    assert_eq!(left, ["notes.txt"]);

    // When a new game folder is made: the last game's staged frame goes,
    // the new game's own stays, and only folders count toward the 20.
    let stage = |folder: &str| {
        PregameOp::Stage(PregameShot {
            folder: folder.to_string(),
            name: BAN_RESULTS_NAME,
            frame: frame(8),
        })
    };
    let mut made = Vec::new();
    write_pregame_ops(&root, &[stage("20261002_000000")], 2, &mut made);
    let staged = root
        .join("20261002_000000")
        .join(staged_name(BAN_RESULTS_NAME));
    assert!(staged.is_file());
    write_pregame_ops(&root, &[stage("20261003_000000")], 2, &mut made);
    assert!(!staged.exists());
    assert!(
        root.join("20261003_000000")
            .join(staged_name(BAN_RESULTS_NAME))
            .is_file()
    );
    let mut games: Vec<String> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .filter(|n| is_game_folder(n))
        .collect();
    games.sort();
    assert_eq!(games, ["20261002_000000", "20261003_000000"]);
}
