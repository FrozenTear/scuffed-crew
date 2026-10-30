//! Pre-game saver tests for frames kept until they are known to be saved:
//! a lone ban-results frame and what is left on disk after each way the
//! ban results end (staged on disk), the loading screen held in memory and
//! saved only when the game starts (synthetic `ENTERING GAME` and
//! `APPLYING UPDATE` frames made in code), and leftovers of a stopped run.

use super::pregame_dump::*;
use super::tests_loading_stand_ins::*;
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
    // A staged loading frame is only left by an older build.
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

    // When a new game folder is made: the last game's staged frame goes
    // (and its folder, left empty), and the new game's own stays.
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
    assert_eq!(games, ["20261001_000000", "20261003_000000"]);
}

/// A game folder `name` under `root` holding `files`, each with its name
/// as its content.
fn game_with(root: &std::path::Path, name: &str, files: &[&str]) -> std::path::PathBuf {
    let dir = root.join(name);
    std::fs::create_dir_all(&dir).unwrap();
    for file in files {
        std::fs::write(dir.join(file), file.as_bytes()).unwrap();
    }
    dir
}

/// Every name in `dir`, dot names too, sorted; empty when it is gone.
fn all_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .map(|it| {
            it.map(|e| e.unwrap().file_name().into_string().unwrap())
                .collect()
        })
        .unwrap_or_default();
    names.sort();
    names
}

#[tokio::test]
async fn starting_the_saver_deletes_every_leftover_and_no_saved_screen() {
    // A run killed mid-game left staged and temporary files: in the
    // pre-game folder itself, next to saved screens, and alone in a
    // folder. Starting the saver (tracker start with the switch on)
    // deletes them all, and that folder with them; saved screens and
    // anything not ours stay.
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    let saved = game_with(
        &root,
        "20261001_000000",
        &[
            "heroban_first.png",
            ".ban_results.png.staged",
            ".loading.png.staged",
            ".heroban_last.png.tmp",
        ],
    );
    let lone = game_with(&root, "20261002_000000", &[".loading.png.staged"]);
    let other = game_with(&root, "keep-me", &[".x.png.staged"]);
    std::fs::write(root.join(".order.tmp"), b"x").unwrap();
    let dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    assert!(dump.finish(Duration::from_secs(30)).await);
    assert_eq!(all_in(&saved), ["heroban_first.png"]);
    assert!(!lone.exists(), "a folder of leftovers only is deleted");
    assert_eq!(all_in(&other), [".x.png.staged"], "not a game folder");
    assert!(!root.join(".order.tmp").exists());
}

#[test]
fn a_folder_of_leftovers_takes_no_slot_and_is_deleted() {
    // Two slots: an older saved game, then a folder holding only a staged
    // frame, then a new game. The leftover folder does not push the saved
    // game out, and it is deleted when the new game's folder is made.
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    let saved = game_with(&root, "20261001_000000", &["mapvote_first.png"]);
    let lone = game_with(&root, "20261002_000000", &[".ban_results.png.staged"]);
    let shot = PregameOp::Save(PregameShot {
        folder: "20261003_000000".to_string(),
        name: "mapvote_first.png",
        frame: frame(8),
    });
    write_pregame_ops(&root, &[shot], 2, &mut Vec::new());
    assert_eq!(all_in(&saved), ["mapvote_first.png"]);
    assert!(!lone.exists());
    assert_eq!(all_in(&root.join("20261003_000000")), ["mapvote_first.png"]);
}

#[test]
fn a_staged_file_never_makes_retention_delete_a_saved_game() {
    // Two slots, two saved games (one with a staged frame left in it),
    // and a new game whose first write is a staged frame. That staged
    // frame is no saved game, so retention deletes neither saved game;
    // only the old leftover goes, and the new game keeps its own.
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    let first = game_with(
        &root,
        "20261001_000000",
        &["heroban_first.png", ".ban_results.png.staged"],
    );
    let second = game_with(&root, "20261002_000000", &["loading.png"]);
    let stage = PregameOp::Stage(PregameShot {
        folder: "20261003_000000".to_string(),
        name: BAN_RESULTS_NAME,
        frame: frame(8),
    });
    write_pregame_ops(&root, &[stage], 2, &mut Vec::new());
    assert_eq!(all_in(&first), ["heroban_first.png"]);
    assert_eq!(all_in(&second), ["loading.png"]);
    assert_eq!(
        all_in(&root.join("20261003_000000")),
        [staged_name(BAN_RESULTS_NAME)]
    );
}

/// One step of a loading run.
enum Step {
    /// A frame through the loading check, and what it must classify as.
    Frame(image::RgbImage, Seen),
    Phase(PregamePhase),
    Release,
    /// Wait until this many game folders are on disk, so the next game's
    /// folder name (the time now, made unique against the disk) cannot
    /// take the last one's.
    Folders(usize),
}

/// What a game folder holds after a loading run.
#[derive(Debug)]
struct Folder {
    /// The saved loading screen, if any.
    loading: Option<image::RgbImage>,
    /// Every file in it, dot names too, sorted.
    files: Vec<String>,
}

fn shown(rgb: &image::RgbImage) -> Arc<image::DynamicImage> {
    Arc::new(image::DynamicImage::ImageRgb8(rgb.clone()))
}

/// A map vote, then `steps` at their times (ms), through the real
/// `classify_with`, `PregameDump` and writer, then shutdown. Returns each
/// game folder, oldest first.
async fn loading_run(steps: Vec<(u64, Step)>) -> Vec<Folder> {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let vote = image::RgbImage::from_pixel(64, 36, image::Rgb([20, 30, 60]));
    dump.observe(
        at(0),
        Seen::Phase(PregamePhase::MapVote),
        Some(shown(&vote)),
    );
    let no_title = |_: &image::RgbImage| false;
    for (ms, step) in steps {
        match step {
            Step::Frame(rgb, want_seen) => {
                let want = dump.want(at(ms));
                let unknown = detect::GamePhase::Unknown;
                let seen = classify_with(want, &unknown, &rgb, no_title, no_title);
                assert_eq!(seen, want_seen, "at {ms} ms");
                let frame = keeps_frame(want, seen).then(|| shown(&rgb));
                dump.observe(at(ms), seen, frame);
            }
            Step::Phase(p) => dump.observe(at(ms), Seen::Phase(p), Some(shown(&vote))),
            Step::Release => dump.release(at(ms)),
            Step::Folders(n) => {
                let root = pregame_dir(dir.path());
                let made = || {
                    std::fs::read_dir(&root)
                        .map(|it| it.flatten().filter(|e| e.path().is_dir()).count())
                        .unwrap_or(0)
                };
                for _ in 0..500 {
                    if made() >= n {
                        break;
                    }
                    tokio::time::sleep(Duration::from_millis(20)).await;
                }
                assert_eq!(made(), n, "at {ms} ms");
            }
        }
    }
    assert!(dump.finish(Duration::from_secs(30)).await);
    let root = pregame_dir(dir.path());
    let mut games: Vec<_> = std::fs::read_dir(&root)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.is_dir())
        .collect();
    games.sort();
    games
        .iter()
        .map(|game| Folder {
            loading: image::open(game.join(LOADING_NAME))
                .ok()
                .map(|i| i.to_rgb8()),
            files: all_in(game),
        })
        .collect()
}

/// A centred `APPLYING UPDATE` band with no bar: it passes the loading
/// check (no rule tells it from `ENTERING GAME` yet).
fn update_band() -> (image::RgbImage, Seen) {
    (applying_update_band(1280, 720, true, false), Seen::Loading)
}

fn entering(w: u32, h: u32) -> (image::RgbImage, Seen) {
    (entering_game_slab(w, h), Seen::Loading)
}

/// The menu, a lobby or any other screen: no phase, fails the check.
fn plain() -> (image::RgbImage, Seen) {
    (
        image::RgbImage::from_pixel(1280, 720, image::Rgb([90, 110, 80])),
        Seen::Other,
    )
}

fn frame_step(ms: u64, (rgb, seen): (image::RgbImage, Seen)) -> (u64, Step) {
    (ms, Step::Frame(rgb, seen))
}

fn phase_step(ms: u64, phase: PregamePhase) -> (u64, Step) {
    (ms, Step::Phase(phase))
}

/// The one game folder holds no loading screen and no staged file.
fn nothing_kept(folders: &[Folder]) {
    assert_eq!(folders.len(), 1, "{folders:?}");
    assert!(folders[0].loading.is_none(), "{:?}", folders[0].files);
    assert!(
        folders[0].files.iter().all(|f| !f.starts_with('.')),
        "{:?}",
        folders[0].files
    );
}

/// The one game folder holds `want` as its loading screen and no staged
/// file.
fn kept(folders: &[Folder], want: &image::RgbImage) {
    assert_eq!(folders.len(), 1, "{folders:?}");
    assert!(folders[0].loading.as_ref() == Some(want));
    assert!(
        folders[0].files.iter().all(|f| !f.starts_with('.')),
        "{:?}",
        folders[0].files
    );
}

#[tokio::test]
async fn an_update_band_then_the_menu_past_twelve_seconds_saves_no_loading() {
    // The band is staged, then the menu for 13.5 s, then the game closes:
    // the game never started, so the band is dropped at 15.5 s.
    let mut steps = vec![frame_step(2_000, update_band())];
    for ms in (3_500..=15_500).step_by(1_500) {
        steps.push(frame_step(ms, plain()));
    }
    steps.push((17_000, Step::Release));
    nothing_kept(&loading_run(steps).await);
    // Chosen: a start read more than 12 s after the last pass, with no
    // tick between, is too late too (the loading screen is long over).
    let steps = vec![
        frame_step(2_000, update_band()),
        phase_step(14_500, PregamePhase::HeroBan),
    ];
    nothing_kept(&loading_run(steps).await);
}

#[tokio::test]
async fn an_update_band_then_the_game_closing_saves_no_loading() {
    // Released more than 12 s after the band with no tick between, and
    // within 12 s, with or without a frame that fails the check first.
    let mut runs = vec![vec![
        frame_step(2_000, update_band()),
        (16_000, Step::Release),
    ]];
    for gap in [false, true] {
        let mut steps = vec![
            frame_step(2_000, update_band()),
            frame_step(3_500, update_band()),
        ];
        if gap {
            steps.push(frame_step(5_000, plain()));
        }
        steps.push((6_000, Step::Release));
        runs.push(steps);
    }
    for steps in runs {
        nothing_kept(&loading_run(steps).await);
    }
}

#[tokio::test]
async fn a_requeue_vote_after_an_update_band_drops_it() {
    // The band 11 s after the vote, then the vote again 2 s later: a
    // requeue (back after more than 12 s), the next game. The band is
    // saved in neither folder; the next game's own loading screen is.
    let steps = vec![
        frame_step(11_000, update_band()),
        (12_000, Step::Folders(1)),
        phase_step(13_000, PregamePhase::MapVote),
        frame_step(16_000, entering(1280, 720)),
        phase_step(17_500, PregamePhase::HeroBan),
    ];
    let folders = loading_run(steps).await;
    assert_eq!(folders.len(), 2, "{folders:?}");
    assert!(folders[0].loading.is_none());
    assert!(folders[0].files.iter().all(|f| !f.starts_with('.')));
    assert!(folders[1].loading.as_ref() == Some(&entering_game_slab(1280, 720)));
    assert!(folders[1].files.iter().all(|f| !f.starts_with('.')));
    // The tracker drops the band itself at the vote, writing nothing for
    // it.
    let mut run = Run::new();
    run.tick(0, Seen::Phase(PregamePhase::MapVote), 1);
    run.tick(11_000, Seen::Loading, 2);
    assert!(run.tracker.holds_loading());
    run.tick(13_000, Seen::Phase(PregamePhase::MapVote), 3);
    assert_eq!(run.games, 2);
    assert!(!run.tracker.holds_loading());
    assert!(run.staged.is_empty(), "{:?}", run.staged);
    assert!(run.names().iter().all(|(n, _)| *n != LOADING_NAME));
}

#[tokio::test]
async fn an_update_band_then_entering_game_then_the_select_saves_entering_game() {
    // Straight after it, and with a frame between that fails the check.
    for gap in [false, true] {
        let mut steps = vec![frame_step(2_000, update_band())];
        if gap {
            steps.push(frame_step(3_500, plain()));
        }
        steps.push(frame_step(5_000, entering(1280, 720)));
        steps.push(phase_step(6_500, PregamePhase::HeroSelect));
        kept(&loading_run(steps).await, &entering_game_slab(1280, 720));
    }
}

#[tokio::test]
async fn entering_game_saves_its_last_pass_when_the_game_starts() {
    // The hero select or the hero ban read after it (the ban when loading
    // comes after the vote).
    for start in [PregamePhase::HeroSelect, PregamePhase::HeroBan] {
        let steps = vec![
            frame_step(2_000, entering(1280, 720)),
            frame_step(3_500, entering(1920, 1080)),
            frame_step(5_000, plain()),
            phase_step(6_500, start),
        ];
        kept(&loading_run(steps).await, &entering_game_slab(1920, 1080));
    }
    // Not yet started when the game closes: nothing is saved.
    let steps = vec![
        frame_step(2_000, entering(1280, 720)),
        frame_step(3_500, plain()),
        (8_000, Step::Release),
    ];
    nothing_kept(&loading_run(steps).await);
}

#[tokio::test]
async fn entering_game_after_a_dropped_update_band_is_still_saved() {
    // The band is dropped at 15.5 s (no start in 12 s); the loading screen
    // is still looked for, so ENTERING GAME after it is held and saved.
    let mut steps = vec![frame_step(2_000, update_band())];
    for ms in (3_500..=15_500).step_by(1_500) {
        steps.push(frame_step(ms, plain()));
    }
    steps.push(frame_step(17_000, entering(1280, 720)));
    steps.push(phase_step(18_500, PregamePhase::HeroBan));
    kept(&loading_run(steps).await, &entering_game_slab(1280, 720));
}

#[tokio::test]
async fn only_the_latest_loading_frame_is_held() {
    // Each pass replaces the one before in memory: after every tick the
    // saver (tracker, game, queue) holds exactly one loading frame, the
    // newest. Also with a ban-results frame held just before.
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let vote = image::RgbImage::from_pixel(64, 36, image::Rgb([20, 30, 60]));
    dump.observe(
        at(0),
        Seen::Phase(PregamePhase::MapVote),
        Some(shown(&vote)),
    );
    dump.observe(at(1_500), BAN, Some(shown(&vote)));
    dump.observe(at(3_000), RESULTS, Some(shown(&vote)));
    let mut handed: Vec<Arc<image::DynamicImage>> = Vec::new();
    for k in 0..6u32 {
        let frame = shown(&entering_game_slab(320 + 32 * k, 180));
        handed.push(Arc::clone(&frame));
        dump.observe(at(4_500 + 1_500 * u64::from(k)), Seen::Loading, Some(frame));
        let held: Vec<usize> = (0..handed.len())
            .filter(|&i| Arc::strong_count(&handed[i]) > 1)
            .collect();
        assert_eq!(held, [k as usize], "pass {k}: the newest, and only it");
    }
    dump.observe(at(13_500), SELECT, Some(shown(&vote)));
    assert!(dump.finish(Duration::from_secs(30)).await);
    assert!(handed.iter().all(|f| Arc::strong_count(f) == 1));
}

/// A writer that queues batches nobody writes, with the queue empty, and
/// the saver on it.
fn counted_dump(dir: &std::path::Path) -> (PregameDump, std::sync::mpsc::Receiver<ShotBatch>) {
    let (writer, queue) = PregameWriter::stalled();
    (
        PregameDump::with_writer(pregame_dir(dir), POLL, writer),
        queue,
    )
}

/// The kind of every op naming the loading screen.
fn loading_ops(ops: &[PregameOp<Arc<image::DynamicImage>>]) -> Vec<&'static str> {
    ops.iter()
        .filter_map(|op| match op {
            PregameOp::Save(s) if s.name == LOADING_NAME => Some("Save"),
            PregameOp::Stage(s) if s.name == LOADING_NAME => Some("Stage"),
            PregameOp::Promote { name, .. } if *name == LOADING_NAME => Some("Promote"),
            PregameOp::Discard { name, .. } if *name == LOADING_NAME => Some("Discard"),
            _ => None,
        })
        .collect()
}

#[test]
fn loading_passes_write_nothing_until_one_save_at_the_game_start() {
    // A map vote, then seven loading passes (an update band, then ENTERING
    // GAME): the writer gets no batch at all for them. The hero select
    // read then queues one batch with exactly one op for the loading
    // screen: a save of the last pass.
    let dir = tempfile::tempdir().unwrap();
    let (mut dump, queue) = counted_dump(dir.path());
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    dump.observe(at(0), Seen::Phase(PregamePhase::MapVote), Some(frame(8)));
    assert_eq!(queue.try_iter().count(), 1, "the vote's first frame");
    for k in 0..7u32 {
        dump.observe(
            at(1_500 + 1_500 * u64::from(k)),
            Seen::Loading,
            Some(frame(20 + k)),
        );
    }
    let during: Vec<ShotBatch> = queue.try_iter().collect();
    assert!(during.is_empty(), "{during:?}");
    dump.observe(at(12_000), SELECT, Some(frame(9)));
    let at_start: Vec<_> = queue.try_iter().flatten().collect();
    assert_eq!(loading_ops(&at_start), ["Save"], "{at_start:?}");
    let saved = at_start.iter().find_map(|op| match op {
        PregameOp::Save(s) if s.name == LOADING_NAME => Some(s.frame.width()),
        _ => None,
    });
    assert_eq!(saved, Some(26), "the last pass");
    // Saved: nothing more is held or looked for in this game.
    assert!(!dump_holds_loading(&dump));
    dump.observe(at(13_500), Seen::Loading, Some(frame(40)));
    let after: Vec<_> = queue.try_iter().flatten().collect();
    assert!(loading_ops(&after).is_empty(), "{after:?}");
}

fn dump_holds_loading(dump: &PregameDump) -> bool {
    dump.tracker().holds_loading()
}

#[test]
fn a_dropped_loading_frame_writes_nothing() {
    // Each way a held loading frame ends without a game start queues no op
    // for it at all, and the saver holds no loading frame after it.
    enum End {
        Timeout,
        Release,
        Requeue,
        LateStart,
    }
    for end in [End::Timeout, End::Release, End::Requeue, End::LateStart] {
        let dir = tempfile::tempdir().unwrap();
        let (mut dump, queue) = counted_dump(dir.path());
        let start = Instant::now();
        let at = |ms: u64| start + Duration::from_millis(ms);
        dump.observe(at(0), Seen::Phase(PregamePhase::MapVote), Some(frame(8)));
        dump.observe(at(1_500), Seen::Loading, Some(frame(9)));
        dump.observe(at(3_000), Seen::Loading, Some(frame(10)));
        assert!(dump_holds_loading(&dump));
        match end {
            End::Timeout => {
                for ms in (4_500..=16_500).step_by(1_500) {
                    dump.observe(at(ms), Seen::Other, None);
                }
            }
            End::Release => dump.release(at(4_500)),
            End::Requeue => dump.observe(
                at(16_000),
                Seen::Phase(PregamePhase::MapVote),
                Some(frame(11)),
            ),
            End::LateStart => dump.observe(at(15_500), BAN, Some(frame(11))),
        }
        assert!(!dump_holds_loading(&dump));
        let ops: Vec<_> = queue.try_iter().flatten().collect();
        assert!(loading_ops(&ops).is_empty(), "{ops:?}");
    }
}

#[tokio::test]
async fn loading_png_is_never_on_disk_before_the_game_starts() {
    // Through the real writer: the folder is watched after every pass,
    // once the writer is done with every pass before the newest, and no
    // loading file (saved, staged or temporary) is ever in it. The game
    // start then saves the last pass, and nothing else is left.
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).unwrap();
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let vote = image::RgbImage::from_pixel(64, 36, image::Rgb([20, 30, 60]));
    dump.observe(
        at(0),
        Seen::Phase(PregamePhase::MapVote),
        Some(shown(&vote)),
    );
    dump.observe(
        at(1_500),
        Seen::Phase(PregamePhase::MapVote),
        Some(shown(&vote)),
    );
    let root = pregame_dir(dir.path());
    let listed = || -> Vec<String> {
        std::fs::read_dir(&root)
            .into_iter()
            .flatten()
            .flatten()
            .filter(|e| e.path().is_dir())
            .flat_map(|e| all_in(&e.path()))
            .collect()
    };
    let mut handed: Vec<Arc<image::DynamicImage>> = Vec::new();
    for k in 0..6u32 {
        let frame = shown(&entering_game_slab(320 + 32 * k, 180));
        handed.push(Arc::clone(&frame));
        dump.observe(at(3_000 + 1_500 * u64::from(k)), Seen::Loading, Some(frame));
        for _ in 0..500 {
            let busy = handed[..handed.len() - 1]
                .iter()
                .any(|f| Arc::strong_count(f) > 1);
            if !busy && listed().iter().any(|n| n == "mapvote_last.png") {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        let files = listed();
        assert!(files.iter().any(|n| n == "mapvote_last.png"), "{files:?}");
        assert!(
            files.iter().all(|n| !n.contains("loading")),
            "pass {k}: {files:?}"
        );
    }
    dump.observe(at(12_000), SELECT, Some(shown(&vote)));
    assert!(dump.finish(Duration::from_secs(30)).await);
    let files = listed();
    assert_eq!(
        files
            .iter()
            .filter(|n| n.contains("loading"))
            .collect::<Vec<_>>(),
        [LOADING_NAME]
    );
    assert!(files.iter().all(|f| !f.starts_with('.')), "{files:?}");
    let game = std::fs::read_dir(&root)
        .unwrap()
        .flatten()
        .map(|e| e.path())
        .find(|p| p.is_dir())
        .unwrap();
    let saved = image::open(game.join(LOADING_NAME)).unwrap();
    assert_eq!(saved.width(), 320 + 32 * 5, "the last pass");
}

#[test]
fn a_dropped_ban_results_stage_keeps_the_confirmed_frame_on_disk_owed() {
    // R1 is staged (confirmed by R2); R2's own stage is dropped with a
    // full queue, so R1 is still the file on disk. It stays owed, and the
    // select read after a plain frame saves it, as with no drop.
    let mut tracker: PregameTracker<u32> = PregameTracker::new(POLL);
    let start = Instant::now();
    let at = |ms: u64| start + Duration::from_millis(ms);
    let game = || "game1".to_string();
    tracker.observe(at(0), BAN, Some(0), game);
    tracker.observe(at(1_500), RESULTS, Some(1), game);
    tracker.observe(at(3_000), RESULTS, Some(2), game);
    let ops = tracker.observe(at(4_500), RESULTS, Some(3), game);
    assert!(matches!(&ops[..], [PregameOp::Stage(s)] if s.frame == 2));
    tracker.stage_lost(BAN_RESULTS_NAME);
    assert!(tracker.owes_frame());
    tracker.observe(at(6_000), Seen::Other, None, game);
    let ops = tracker.observe(at(7_500), SELECT, Some(4), game);
    assert!(
        ops.iter()
            .any(|op| matches!(op, PregameOp::Promote { name, .. } if *name == BAN_RESULTS_NAME)),
        "{ops:?}"
    );
    // A first stage dropped: nothing of this candidate is on disk, so
    // nothing is owed.
    let mut tracker: PregameTracker<u32> = PregameTracker::new(POLL);
    tracker.observe(at(0), BAN, Some(0), game);
    tracker.observe(at(1_500), RESULTS, Some(1), game);
    tracker.observe(at(3_000), RESULTS, Some(2), game);
    tracker.stage_lost(BAN_RESULTS_NAME);
    assert!(!tracker.owes_frame());
}

#[test]
fn a_stage_that_fails_to_write_leaves_no_older_frame_to_save() {
    // R1 is staged; R2's write fails (a folder in the way of its temporary
    // file). The promote must not save R1 in its place.
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    let folder = "20261001_000000";
    let stage = |w| {
        PregameOp::Stage(PregameShot {
            folder: folder.to_string(),
            name: BAN_RESULTS_NAME,
            frame: frame(w),
        })
    };
    let mut made = Vec::new();
    write_pregame_ops(&root, &[stage(8)], 2, &mut made);
    let game = root.join(folder);
    assert!(game.join(staged_name(BAN_RESULTS_NAME)).is_file());
    std::fs::create_dir(game.join(format!(".{}.tmp", staged_name(BAN_RESULTS_NAME)))).unwrap();
    let promote = PregameOp::Promote {
        folder: folder.to_string(),
        name: BAN_RESULTS_NAME,
    };
    write_pregame_ops(&root, &[stage(9), promote], 2, &mut made);
    assert!(!game.join(BAN_RESULTS_NAME).exists());
    assert!(!game.join(staged_name(BAN_RESULTS_NAME)).exists());
}
