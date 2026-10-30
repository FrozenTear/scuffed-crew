//! Tests for the post-game saves' poll pacing, ring age filter, dedup
//! relaxation, folder owner, final sidecar note, dead writer and crash
//! leftovers (the post-game temp files, cleared by the shared pre-game
//! sweep and retention rules). Synthetic stand-ins only (see
//! `tests_postgame_dump.rs`).

use super::postgame_dump::*;
use super::pregame_dump::*;
use super::tests_postgame_dump::*;
use crate::*;
use std::time::Duration;

#[test]
fn a_stale_end_frame_in_the_ring_is_neither_saved_nor_time_zero() {
    // A tracker for 1.5 s polls (ring span 5 s plus one poll) fed every
    // 3 s (every other tick lost to Tab captures): its 5 frames reach 10 s
    // back. The end-reel frame 10 s back is stale; the fresh end title read
    // 1 s before the confirmation is time zero.
    let mut run = Run::new();
    run.tick(0, ended(RUNNING, EndSeen::Reel, false), banner());
    run.tick(3_000, RUNNING, play(1));
    run.tick(6_000, RUNNING, play(2));
    run.tick(9_000, ended(RUNNING, EndSeen::Title, false), moving(1));
    assert_eq!(run.tracker.ring_len(), 4);
    run.tick(10_000, ended(AFTER, EndSeen::Title, true), moving(2));
    assert_eq!(
        run.rec.opened,
        vec![1.0],
        "time zero is the fresh end frame"
    );
    // 3 s (7 s back) is stale too; 6 s (4 s back) is young enough.
    assert_eq!(run.rec.times(), vec![-3.0, 0.0, 1.0]);
    assert_eq!(
        run.rec.labels(),
        vec!["before_end", "end_title", "end_title"]
    );
}

/// A window opened at 1.5 s, with its first sample on the confirming tick.
fn open_window() -> Run {
    let mut run = Run::new();
    run.tick(0, RUNNING, play(0));
    run.tick(1_500, ended(AFTER, EndSeen::Title, true), banner());
    assert!(run.tracker.window_open());
    run
}

#[test]
fn the_next_tick_is_brought_forward_only_after_a_handed_over_frame_and_never_to_zero() {
    let mut run = open_window();
    // The confirming tick handed over its frame: next sample in 1 s.
    let due = run.tracker.next_due_in(run.at(1_500)).expect("handed");
    assert_eq!(due, POSTGAME_FAST);
    // Late: the sample is overdue, yet the next tick waits the slack.
    let late = run.tracker.next_due_in(run.at(4_000)).expect("handed");
    assert_eq!(late, POSTGAME_SLACK);
    // A tick whose analysis panicked hands over no frame: the regular
    // tick comes next, never one at once.
    let now = run.at(2_500);
    run.tracker.observe(now, AFTER, None, &mut run.rec);
    assert!(run.tracker.window_open());
    assert_eq!(run.tracker.next_due_in(now), None);
    assert_eq!(run.tracker.next_due_in(run.at(9_000)), None);
    // Every panicking tick keeps it so: the window polls at the regular
    // interval, as with the switch off.
    for ms in [4_000, 5_500, 7_000] {
        let now = run.at(ms);
        run.tracker.observe(now, AFTER, None, &mut run.rec);
        assert_eq!(run.tracker.next_due_in(now), None);
    }
    // A frame again: brought forward again, at least the slack.
    run.tick(8_500, AFTER, moving(3));
    let due = run.tracker.next_due_in(run.at(8_500)).expect("handed");
    assert!(due >= POSTGAME_SLACK);
    // Outside a window nothing is brought forward, frame or not.
    let mut idle = Run::new();
    idle.tick(0, RUNNING, play(0));
    idle.tick(1_500, RUNNING, play(1));
    assert_eq!(idle.tracker.next_due_in(idle.at(1_500)), None);
}

#[test]
fn the_poll_loop_side_backs_off_after_a_panicked_tick() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).expect("on");
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let confirm = PostTick {
        end: Some(EndSeen::Title),
        confirmed: true,
        ..PostTick::default()
    };
    // Polled at 0, fed after 300 ms of grab and analysis: the sample
    // counts from the poll, so the next tick is 700 ms from now.
    dump.post_want(at(0));
    dump.observe_post(at(300), confirm, Some(banner()));
    assert_eq!(
        dump.post_sooner_tick(at(300), POLL),
        Some(Duration::from_millis(700))
    );
    // The analysis panicked: no frame, so the regular tick.
    assert!(dump.post_want(at(1_000)).every);
    dump.observe_post(at(1_000), PostTick::default(), None);
    assert_eq!(dump.post_sooner_tick(at(1_100), POLL), None);
    assert!(dump.post_window_open());
}

#[test]
fn a_still_screen_is_saved_again_every_few_seconds_and_always_late_in_the_window() {
    let mut run = open_window();
    // The same accolade screen for the whole window, a pixel or two of
    // noise a frame: the counters on it may still change.
    let still = picture(70, Some((0.1, 0.3)));
    let mut t = 2_500;
    while run.tracker.window_open() {
        run.tick(t, AFTER, noisy(&still, t as u32));
        t += 500;
    }
    let times = run.rec.times();
    // The banner, then the still screen at 1 s and again on the first
    // sample 5 s after the last save, until 60 s.
    let early: Vec<f64> = times.iter().copied().filter(|t| *t < 60.0).collect();
    assert_eq!(
        early,
        vec![
            0.0, 1.0, 6.0, 11.0, 16.0, 21.0, 26.0, 31.5, 36.5, 41.5, 46.5, 51.5, 56.5
        ],
        "{times:?}"
    );
    // From 60 s every sample, near duplicate or not (2.5 s steps).
    let late: Vec<f64> = times.iter().copied().filter(|t| *t >= 60.0).collect();
    assert_eq!(
        late,
        vec![
            61.5, 64.0, 66.5, 69.0, 71.5, 74.0, 76.5, 79.0, 81.5, 84.0, 86.5, 89.0
        ],
        "{times:?}"
    );
    assert_eq!(run.rec.ended, vec![PostEnd::Time]);
}

#[tokio::test]
async fn a_game_with_no_pre_game_read_does_not_take_an_older_games_folder() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).expect("on");
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let min = 60_000;
    // Game A: its map vote makes its folder; it opened then, and ends with
    // no window (the result was never read).
    dump.observe(at(0), Seen::Phase(PregamePhase::MapVote), Some(play(9)));
    let a = PostTick {
        running: true,
        game_opened: Some(at(0)),
        ..PostTick::default()
    };
    dump.post_want(at(1_500));
    dump.observe_post(at(1_500), a, None);
    dump.game_closed(at(10 * min));
    // Game B, 20 minutes later: opened by its first Tab, no map vote or
    // ban folder. Its end gets a folder of its own.
    let b = PostTick {
        running: true,
        game_opened: Some(at(20 * min)),
        ..PostTick::default()
    };
    dump.post_want(at(20 * min));
    dump.observe_post(at(20 * min), b, None);
    let confirm = PostTick {
        end: Some(EndSeen::Title),
        confirmed: true,
        finished: true,
        ..b
    };
    // Game A's folder is on disk (the writer runs on its own thread).
    let root = pregame_dir(dir.path());
    for _ in 0..500 {
        if root.is_dir() && names_in(&root).len() == 1 {
            break;
        }
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    dump.post_want(at(30 * min));
    dump.observe_post(at(30 * min), confirm, Some(banner()));
    assert!(dump.finish(Duration::from_secs(30)).await);

    let folders = names_in(&root);
    assert_eq!(folders.len(), 2, "{folders:?}");
    let with = |name: &str| {
        folders
            .iter()
            .filter(|f| names_in(&root.join(f)).iter().any(|n| n == name))
            .count()
    };
    assert_eq!(with("mapvote_first.png"), 1);
    assert_eq!(with("postgame_01.png"), 1);
    let a_folder = folders
        .iter()
        .find(|f| names_in(&root.join(f)).contains(&"mapvote_first.png".to_string()))
        .unwrap();
    assert_eq!(names_in(&root.join(a_folder)), vec!["mapvote_first.png"]);
}

#[test]
fn a_folder_belongs_to_the_first_game_opened_near_it() {
    let t0 = Instant::now();
    let min = Duration::from_secs(60);
    let mut owner = FolderOwner::default();
    // Seen while an older game (opened 30 minutes before) is still open:
    // not that game's.
    owner.note(t0 + 30 * min, Some("g1"), Some(t0));
    assert!(!owner.owns("g1", Some(t0)));
    // The game opened by its first Tab after its own pre-game: its folder.
    owner.note(t0 + 40 * min, Some("g1"), Some(t0 + 40 * min));
    assert!(owner.owns("g1", Some(t0 + 40 * min)));
    // Bound once: a later game does not take it.
    owner.note(t0 + 60 * min, Some("g1"), Some(t0 + 60 * min));
    assert!(!owner.owns("g1", Some(t0 + 60 * min)));
    assert!(!owner.owns("g1", None));
    // A new folder starts over.
    owner.note(t0 + 61 * min, Some("g2"), Some(t0 + 60 * min));
    assert!(owner.owns("g2", Some(t0 + 60 * min)));
    assert!(!owner.owns("g1", Some(t0 + 40 * min)));
}

#[test]
fn a_pre_game_folder_passed_over_is_taken_by_no_later_game() {
    let dir = tempfile::tempdir().unwrap();
    let mut writer = PregameWriter::dead();
    let mut log = PostLog::default();
    let t0 = Instant::now();
    let min = Duration::from_secs(60);
    let mut owner = FolderOwner::default();
    // Game A's folder, first seen while an older unfinished session Z (the
    // vote did not open a new one) was open: nobody owns it, and A's
    // window, under Z, goes to a new folder.
    let z = Some(t0);
    owner.note(t0 + 30 * min, Some("20261030_120000"), z);
    let mut sink = DumpSink {
        root: dir.path(),
        latest: Some(("20261030_120000", t0 + 32 * min)),
        owner: &owner,
        log: &mut log,
        writer: &mut writer,
    };
    sink.opened(t0 + 50 * min, 1.0, z);
    let a_folder = log.folder.clone().unwrap();
    assert_ne!(a_folder, "20261030_120000");
    // Game B, no pre-game of its own, opens later and would bind A's
    // folder; it is passed over, so B gets a new one too.
    let b = Some(t0 + 55 * min);
    owner.note(t0 + 55 * min, Some("20261030_120000"), b);
    assert!(owner.owns("20261030_120000", b));
    let mut sink = DumpSink {
        root: dir.path(),
        latest: Some(("20261030_120000", t0 + 32 * min)),
        owner: &owner,
        log: &mut log,
        writer: &mut writer,
    };
    sink.opened(t0 + 70 * min, 1.0, b);
    assert_ne!(log.folder.as_deref(), Some("20261030_120000"));
    assert!(log.used.iter().any(|u| u == "20261030_120000"));
}

#[test]
fn the_final_note_is_kept_when_the_queue_is_full_and_sent_when_it_has_room() {
    let dir = tempfile::tempdir().unwrap();
    let (mut writer, queue) = PregameWriter::stalled();
    let mut log = PostLog::default();
    let owner = FolderOwner::default();
    let mut sink = DumpSink {
        root: dir.path(),
        latest: None,
        owner: &owner,
        log: &mut log,
        writer: &mut writer,
    };
    // Time zero 2 s before this tick's work ended, on the poll's clock.
    let now = Instant::now() - Duration::from_secs(2);
    sink.opened(now, 1.5, None);
    for n in 0..PREGAME_QUEUE {
        assert!(sink.shot(PostShot {
            at_s: n as f64,
            label: "after_end",
            frame: moving(n as u32),
        }));
    }
    assert!(!sink.shot(PostShot {
        at_s: 9.0,
        label: "after_end",
        frame: moving(9),
    }));
    sink.ended(PostEnd::Time);
    // Time zero on the UTC clock: 2 s back, not the 1.5 s from the
    // confirmation (the work after the poll is not added).
    let utc = log.doc.zero_utc.expect("time zero");
    let back = (Utc::now() - utc).num_milliseconds();
    assert!((2_000..2_900).contains(&back), "{back} ms");
    let pending = log.pending.as_ref().expect("kept for a retry");
    assert_eq!(pending.doc.ended, Some(PostEnd::Time));
    assert_eq!(pending.doc.frames.len(), PREGAME_QUEUE);
    assert!(log.doc.zero_utc.is_some());
    // Still full: kept.
    retry_pending(&mut log, &mut writer);
    assert!(log.pending.is_some());
    // Room for one: sent.
    queue.recv().unwrap();
    retry_pending(&mut log, &mut writer);
    assert!(log.pending.is_none());
    let sent: Vec<WriterJob> = queue.try_iter().collect();
    let last = sent.last().and_then(|j| j.note.as_ref()).expect("the note");
    assert_eq!(last.doc.ended, Some(PostEnd::Time));
    // A dead writer drops it: nothing could write it.
    let mut dead = PregameWriter::dead();
    log.pending = Some(last.clone());
    retry_pending(&mut log, &mut dead);
    assert!(log.pending.is_none());
}

#[tokio::test]
async fn shutdown_waits_briefly_for_room_for_the_final_note() {
    let (mut writer, queue) = PregameWriter::stalled();
    for _ in 0..PREGAME_QUEUE {
        assert!(writer.send(vec![PregameOp::Discard {
            folder: "20261030_120000".into(),
            name: BAN_RESULTS_NAME,
        }]));
    }
    let drain = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(100));
        queue.iter().count()
    });
    let note = NoteFile {
        folder: "20261030_120000".into(),
        name: POSTGAME_SIDECAR,
        doc: PostSidecar::default(),
    };
    let job = WriterJob {
        shots: Vec::new(),
        note: Some(note),
    };
    assert!(writer.send_wait(job, Duration::from_secs(10)).await);
    drop(writer);
    assert_eq!(drain.join().unwrap(), PREGAME_QUEUE + 1);
    // With no room before the deadline it gives up, without hanging.
    let (mut full, _queue) = PregameWriter::stalled();
    for _ in 0..PREGAME_QUEUE {
        assert!(full.send(vec![PregameOp::Discard {
            folder: "20261030_120000".into(),
            name: BAN_RESULTS_NAME,
        }]));
    }
    let job = WriterJob {
        shots: Vec::new(),
        note: None,
    };
    assert!(!full.send_wait(job, Duration::from_millis(50)).await);
}

#[tokio::test]
async fn with_the_writer_gone_no_ring_is_armed_and_no_window_opens() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).expect("on");
    dump.kill_writer();
    let t0 = Instant::now();
    let running = PostTick {
        running: true,
        ..PostTick::default()
    };
    assert_eq!(dump.post_want(t0), PostWant::default());
    dump.observe_post(t0, running, Some(play(1)));
    let confirm = PostTick {
        end: Some(EndSeen::Title),
        confirmed: true,
        ..PostTick::default()
    };
    let at = t0 + Duration::from_millis(1_500);
    assert_eq!(dump.post_want(at), PostWant::default());
    dump.observe_post(at, confirm, Some(banner()));
    assert!(!dump.post_window_open());
    assert_eq!(dump.post_sooner_tick(at, POLL), None);
    let alive = PregameWriter::stalled().0;
    assert!(alive.alive());
    assert!(!PregameWriter::dead().alive());
    dump.finish(Duration::from_secs(5)).await;
    assert!(!pregame_dir(dir.path()).join(".order").exists());
}

/// Every name in `dir`, dot names too, sorted.
fn all_in(dir: &std::path::Path) -> Vec<String> {
    let mut names: Vec<String> = std::fs::read_dir(dir)
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    names.sort();
    names
}

fn write(dir: &std::path::Path, names: &[&str]) {
    std::fs::create_dir_all(dir).unwrap();
    for name in names {
        std::fs::write(dir.join(name), b"x").unwrap();
    }
}

#[tokio::test]
async fn post_game_crash_leftovers_are_cleared_at_start() {
    let dir = tempfile::tempdir().unwrap();
    let root = pregame_dir(dir.path());
    // A game with saved frames and the temp files of a write a crash cut.
    write(
        &root.join("20261030_120000"),
        &[
            "postgame_01.png",
            ".postgame_02.png.tmp",
            ".postgame.json.tmp",
        ],
    );
    // Folders holding post-game leftovers only: no games.
    write(&root.join("20261030_121000"), &[".postgame_01.png.tmp"]);
    write(
        &root.join("20261030_122000"),
        &[".postgame.json.tmp", ".postgame_02.png.tmp"],
    );
    // Not ours: a folder of another name, and a user file in a game folder.
    write(&root.join("keep"), &[".postgame_01.png.tmp"]);
    write(&root.join("20261030_124000"), &["notes.txt"]);
    let writer = PregameWriter::start(root.clone()).unwrap();
    assert!(writer.finish(Duration::from_secs(10)).await);
    assert_eq!(all_in(&root.join("20261030_120000")), ["postgame_01.png"]);
    for gone in ["20261030_121000", "20261030_122000"] {
        assert!(!root.join(gone).exists(), "{gone}");
    }
    assert_eq!(all_in(&root.join("keep")), [".postgame_01.png.tmp"]);
    assert_eq!(all_in(&root.join("20261030_124000")), ["notes.txt"]);
}

#[test]
fn leftover_only_folders_take_no_retention_slot() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    // 19 real games, oldest first.
    for g in 0..19 {
        write(
            &root.join(format!("20261030_10{g:02}00")),
            &["postgame_01.png"],
        );
    }
    // Three folders of post-game crash leftovers, newer than every real
    // game: a temp frame, a temp sidecar, both. (The shared rule from the
    // pre-game saves: staged frames are covered by its own tests.)
    write(&root.join("20261030_110000"), &[".postgame_01.png.tmp"]);
    write(&root.join("20261030_110100"), &[".postgame.json.tmp"]);
    write(
        &root.join("20261030_110200"),
        &[".postgame_03.png.tmp", ".postgame.json.tmp"],
    );
    // The 20th real game: nothing real is pushed out.
    write(&root.join("20261030_120000"), &["postgame_01.png"]);
    prune_pregame_games(root, PREGAME_KEEP_GAMES, &["20261030_120000".to_string()]);
    let real = names_in(root)
        .into_iter()
        .filter(|f| root.join(f).join("postgame_01.png").exists())
        .count();
    assert_eq!(real, PREGAME_KEEP_GAMES);
    assert!(
        root.join("20261030_100000").exists(),
        "the oldest game stays"
    );
    // A 21st game pushes out exactly the oldest real one.
    write(&root.join("20261030_130000"), &["postgame_01.png"]);
    prune_pregame_games(root, PREGAME_KEEP_GAMES, &["20261030_130000".to_string()]);
    assert!(!root.join("20261030_100000").exists());
    assert!(root.join("20261030_100100").exists());
}

#[test]
fn a_new_game_folder_clears_the_leftover_folders_and_keeps_every_real_game() {
    let dir = tempfile::tempdir().unwrap();
    let root = dir.path();
    let mut made = Vec::new();
    for g in 0..19 {
        let shot = PregameOp::Save(PregameShot {
            folder: format!("20261030_10{g:02}00"),
            name: postgame_name(0),
            frame: shown(picture(10, None)),
        });
        write_pregame_ops(root, &[shot], PREGAME_KEEP_GAMES, &mut made);
    }
    write(&root.join("20261030_110000"), &[".postgame_01.png.tmp"]);
    write(&root.join("20261030_110100"), &[".postgame.json.tmp"]);
    let note = NoteFile {
        folder: "20261030_120000".into(),
        name: POSTGAME_SIDECAR,
        doc: PostSidecar::default(),
    };
    write_note(root, &note, PREGAME_KEEP_GAMES, &mut made);
    assert!(!root.join("20261030_110000").exists());
    assert!(!root.join("20261030_110100").exists());
    let folders = names_in(root);
    assert_eq!(folders.len(), PREGAME_KEEP_GAMES, "{folders:?}");
    assert_eq!(folders[0], "20261030_100000", "no real game pushed out");
}

/// One poll tick the way the poll loop feeds it: the pre-game read with
/// the saver's own want, the pre-game step, then the post-game one.
fn loop_tick(
    dump: &mut PregameDump,
    now: Instant,
    rgb: image::RgbImage,
    tick: PostTick,
) -> PregameWant {
    let want = dump.want(now);
    let post_want = dump.post_want(now);
    let no_title = |_: &image::RgbImage| false;
    let seen = classify_with(want, &detect::GamePhase::Unknown, &rgb, no_title, no_title);
    let (pre, frame) = kept_frames(
        image::DynamicImage::ImageRgb8(rgb),
        Some((seen, keeps_frame(want, seen))),
        Some(post_want),
        tick.end.is_some(),
    );
    if let Some((seen, pre_frame)) = pre {
        dump.observe(now, seen, pre_frame);
    }
    PregameDump::post_after_decision(Some(dump), || now, || tick, frame, POLL, || true);
    want
}

#[tokio::test]
async fn a_loading_screen_after_the_rank_screen_ends_the_window_at_its_first_frame() {
    use super::tests_loading_stand_ins::entering_game_slab;
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).expect("on");
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let read = |end: EndSeen| PostTick {
        end: Some(end),
        ..PostTick::default()
    };
    // No window and no pre-game: the loading check does not run.
    let confirm = PostTick {
        confirmed: true,
        ..read(EndSeen::Title)
    };
    let want = loop_tick(&mut dump, at(0), picture(20, Some((0.4, 0.6))), confirm);
    assert!(!want.loading);
    assert!(dump.post_window_open());
    // The accolades, then the rank screen: the window looks for loading.
    let accolades = picture(70, Some((0.1, 0.3)));
    let want = loop_tick(&mut dump, at(1_000), accolades, read(EndSeen::Accolades));
    assert!(want.loading && !want.other_frame);
    let rank = picture(110, Some((0.6, 0.8)));
    loop_tick(&mut dump, at(2_000), rank, read(EndSeen::Rank));
    assert!(dump.post_window_open());
    // The first loading frame ends the window, by the pre-game check.
    let slab = entering_game_slab(1280, 720);
    let want = loop_tick(&mut dump, at(3_000), slab.clone(), PostTick::default());
    assert!(want.loading);
    assert!(!dump.post_window_open());
    // Later loading frames: no window, no loading check, nothing held.
    for ms in [4_000, 5_000] {
        let want = loop_tick(&mut dump, at(ms), slab.clone(), PostTick::default());
        assert!(!want.loading, "{ms}");
    }
    assert!(!dump.tracker().holds_loading());
    assert!(dump.finish(Duration::from_secs(30)).await);

    let root = pregame_dir(dir.path());
    let folders = names_in(&root);
    assert_eq!(folders.len(), 1, "{folders:?}");
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(&folders[0]).join(POSTGAME_SIDECAR)).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["ended"], "new_game_screen");
    let frames = doc["frames"].as_array().unwrap();
    assert_eq!(frames.len(), 3, "{frames:?}");
    for f in frames {
        assert!(
            f["t_s"].as_f64().unwrap() < 3.0,
            "a loading frame saved: {f}"
        );
        assert_ne!(f["label"], "after_end", "{f}");
    }
}

#[test]
fn the_rank_screens_first_frame_is_labelled_rank_update_once_the_next_tick_reads_it() {
    let mut run = open_window();
    // A frame with no read (the end reel), then the accolades.
    run.tick(2_500, AFTER, moving(1));
    let accolades = picture(70, Some((0.1, 0.3)));
    run.tick(
        3_500,
        ended(AFTER, EndSeen::Accolades, false),
        shown(accolades.clone()),
    );
    run.tick(
        4_500,
        ended(AFTER, EndSeen::Accolades, false),
        noisy(&accolades, 1),
    );
    // The rank screen's first frame reads nothing (its word has not held
    // still yet), the next tick reads it.
    let rank = picture(110, Some((0.6, 0.8)));
    run.tick(5_500, AFTER, shown(rank.clone()));
    assert_eq!(run.rec.labels().last(), Some(&"after_end"));
    let mut t = 6_500;
    while t <= 10_500 {
        run.tick(
            t,
            ended(AFTER, EndSeen::Rank, false),
            noisy(&rank, t as u32),
        );
        t += 1_000;
    }
    // Only a frame saved on the tick right before a rank read: here a
    // tick in between handed over no frame.
    run.tick(11_500, AFTER, moving(2));
    run.tracker
        .observe(run.at(12_500), AFTER, None, &mut run.rec);
    run.tick(13_500, ended(AFTER, EndSeen::Rank, false), noisy(&rank, 2));
    assert_eq!(
        run.rec.labels(),
        vec![
            "end_title",
            "after_end",
            "accolades",
            "rank_update",
            "rank_update",
            "after_end",
            "rank_update"
        ]
    );
    assert_eq!(run.rec.times(), vec![0.0, 1.0, 2.0, 4.0, 9.0, 10.0, 12.0]);
}

#[tokio::test]
async fn the_sidecar_labels_the_rank_screens_first_frame_rank_update() {
    let dir = tempfile::tempdir().unwrap();
    let mut dump = PregameDump::new(true, dir.path(), POLL).expect("on");
    let t0 = Instant::now();
    let at = |ms: u64| t0 + Duration::from_millis(ms);
    let read = |end: EndSeen| PostTick {
        end: Some(end),
        ..PostTick::default()
    };
    let confirm = PostTick {
        confirmed: true,
        ..read(EndSeen::Title)
    };
    loop_tick(&mut dump, at(0), picture(20, Some((0.4, 0.6))), confirm);
    let accolades = picture(70, Some((0.1, 0.3)));
    loop_tick(&mut dump, at(1_000), accolades, read(EndSeen::Accolades));
    let rank = picture(110, Some((0.6, 0.8)));
    loop_tick(&mut dump, at(2_000), rank.clone(), PostTick::default());
    // The same rank screen, read now: a near duplicate, not saved, yet
    // the frame before it is relabelled in the final note.
    loop_tick(&mut dump, at(3_000), rank, read(EndSeen::Rank));
    assert!(dump.post_window_open());
    assert!(dump.finish(Duration::from_secs(30)).await);

    let root = pregame_dir(dir.path());
    let folders = names_in(&root);
    assert_eq!(folders.len(), 1, "{folders:?}");
    let doc: serde_json::Value = serde_json::from_str(
        &std::fs::read_to_string(root.join(&folders[0]).join(POSTGAME_SIDECAR)).unwrap(),
    )
    .unwrap();
    assert_eq!(doc["ended"], "shutdown");
    let labels: Vec<&str> = doc["frames"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| f["label"].as_str().unwrap())
        .collect();
    assert_eq!(labels, vec!["end_title", "accolades", "rank_update"]);
}
