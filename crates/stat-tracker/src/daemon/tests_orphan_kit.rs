//! Tests for #810: a rank or end screen with no live game behind it must
//! not leave a report kit of its own (pictures with no reads and no stored
//! game), and a Tab on a finished game's own rank or end screen must not
//! close that game and open an empty one.
//!
//! The first test replays the 2026-11-21 orphan as the tracker logged it
//! (times are that night's, Europe/Oslo):
//!
//! ```text
//! 01:23:36  last board of the Ilios game
//! 01:23:50  "outcome recovered from rejected capture frame: adopting"
//!           outcome=Defeat, the Ilios game
//! 01:24:43  "result word (words-v1): Defeat ... context="rank screen""
//! 01:25:23  "Tab past the post-match grace window: the board decides
//!           whether this is the same game"
//! 01:25:23  "capture rejected: not a scoreboard"
//! 01:25:23  "closed session at new-game boundary ... reason="superseded
//!           by Tab-opened game"", the Ilios game
//! 01:25:23  "outcome recovered from rejected capture frame: adopting"
//!           outcome=Defeat, the new empty session
//! 01:25:25  "result word (words-v1): Defeat ... context="rank screen""
//! 01:28:01  "game closed with NO Tab captures: it was not recorded",
//!           the empty session
//! ```
//!
//! The empty session's kit held `outcome.png`, `rank.png` and
//! `rank_banner.png`: the poll ticks after 01:25:23 wrote them.

use stat_tracker::report::blank::CaptureTime;
use stat_tracker::report::kit;

use super::tests_report_kit::{Logged, frame, kit_reads, rank_pngs};
use crate::{main_tests::*, *};

const LEFT_OUT: &str = "report kit: rank screen crops left out";

/// A recorded game that ended in a Defeat `result_secs_ago` before `now`,
/// its last board `board_secs_ago` before `now`, on a map read from the
/// top bar: what [`finished_game_tab_probe`] needs, when the board is
/// recent enough.
fn finished_game(
    session_id: &str,
    now: Instant,
    result_secs_ago: u64,
    board_secs_ago: u64,
) -> ActiveGame {
    let mut g = game(detect::MatchOutcome::Defeat, Some(result_secs_ago), now);
    g.session_id = session_id.to_string();
    g.map = Some("Ilios".to_string());
    g.map_source = Some(boundary::MapSource::TopBar);
    g.gate = Some(GateState::default());
    g.last_stats_at = Some(now - Duration::from_secs(board_secs_ago));
    g.last_activity = now - Duration::from_secs(result_secs_ago);
    g
}

/// The kit an accepted board of `session_id` left: its reads.
fn kit_of_a_recorded_game(data_dir: &std::path::Path, session_id: &str) {
    write_report_kit(
        data_dir,
        session_id,
        &frame(),
        Some(6),
        &kit_reads(session_id),
        CaptureTime::now(),
        &[],
    );
    assert!(kit::has_reads(data_dir, session_id));
}

/// The names of the kit folders under `data_dir`, sorted.
fn kit_folders(data_dir: &std::path::Path) -> Vec<String> {
    let Ok(entries) = std::fs::read_dir(kit::kit_root(data_dir)) else {
        return Vec::new();
    };
    let mut names: Vec<String> = entries
        .filter_map(Result::ok)
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    names
}

/// The kit folders with no reads: pictures that belong to no stored game.
fn orphan_kits(data_dir: &std::path::Path) -> Vec<String> {
    kit_folders(data_dir)
        .into_iter()
        .filter(|session| !kit::has_reads(data_dir, session))
        .collect()
}

fn has_rank_crops(data_dir: &std::path::Path, session_id: &str) -> bool {
    let kit = kit::load(data_dir, session_id);
    kit.outcome_png.is_some() && kit.rank_png.is_some() && kit.rank_banner_png.is_some()
}

/// One Tab whose frame the capture rejected as not a scoreboard, with
/// `on_frame` read on it (a result word on a rank or end screen, or
/// nothing), the way the daemon runs it: the session choice at the press,
/// then the probe settle and the report. Returns the request's own session
/// and whether the Tab was a probe of the finished game.
async fn rejected_tab(
    st: &mut SessionState,
    store: &storage::LocalStore,
    data_dir: &std::path::Path,
    press: Instant,
    on_frame: detect::MatchOutcome,
) -> (String, bool) {
    let req = tab_capture_request(st, store, data_dir, press, || std::future::ready(())).await;
    let (fresh, probe) = (req.session_id.clone(), req.finished_game_probe.is_some());
    let mut report = rejected_report(&fresh);
    report.outcome = on_frame;
    let result = Ok(report);
    let landed = press + Duration::from_millis(420);
    let sid = settle_tab_probe(st, store, data_dir, fresh.clone(), &result, landed, || {
        std::future::ready(())
    })
    .await;
    apply_capture_report(st, store, data_dir, &sid, result, landed).await;
    (fresh, probe)
}

/// The poll ticks that follow while the rank screen still shows: each one
/// offers the screen's crops to the open game, as the poll loop does.
fn rank_screen_ticks(st: &SessionState, data_dir: &std::path::Path) {
    for _ in 0..3 {
        record_rank_crops(data_dir, st.active_game.as_ref(), Some(&rank_pngs()));
    }
}

/// The 2026-11-21 orphan (the module doc has its log lines): a Tab on the
/// Ilios game's rank screen, 93 s after its Defeat and 107 s after its last
/// board. Before #810 the Tab closed the Ilios game and opened an empty
/// session, the frame's Defeat went onto that session, and the next poll
/// ticks wrote the rank screen into a kit of its own.
#[tokio::test]
async fn replay_2026_11_21_a_tab_on_the_rank_screen_opens_no_game_and_leaves_no_kit() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let press = test_now();
    let mut st = session(Some(finished_game("ilios", press, 93, 107)), press);
    kit_of_a_recorded_game(dir.path(), "ilios");

    let (fresh, probe) = rejected_tab(
        &mut st,
        &store,
        dir.path(),
        press,
        detect::MatchOutcome::Defeat,
    )
    .await;
    assert!(
        probe,
        "93 s after the result, 107 s after the last board: a probe"
    );
    rank_screen_ticks(&st, dir.path());

    let open = st.active_game.as_ref().expect("a game stays open");
    assert_eq!(
        open.session_id, "ilios",
        "the rank screen is the Ilios game's own: it stays open, no session {fresh} opens"
    );
    assert_eq!(open.outcome, detect::MatchOutcome::Defeat);
    assert!(st.tab_probe.is_none(), "the probe is settled");
    assert_eq!(
        orphan_kits(dir.path()),
        Vec::<String>::new(),
        "no kit of pictures without reads"
    );
    assert_eq!(
        kit_folders(dir.path()),
        ["ilios"],
        "one kit, the Ilios game's"
    );
    assert!(
        has_rank_crops(dir.path(), "ilios"),
        "the rank screen's crops go to the game it belongs to"
    );
}

const KEPT_OTHER_RESULT: &str =
    "result screen of a finished game shows another result: the stored result is kept";

/// A probe Tab on the finished game's result screen keeps the stored result
/// even when the frame shows the other word: a false latch (R9-2) whose real
/// end screen this is. That case is one INFO line with the session and both
/// results, so it shows in the journal. When the words agree, no line.
#[test]
fn a_result_screen_with_the_other_word_keeps_the_result_and_logs_it() {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    for (on_frame, differs) in [
        (detect::MatchOutcome::Victory, true),
        (detect::MatchOutcome::Defeat, false),
    ] {
        let dir = tempfile::tempdir().unwrap();
        let log = Logged::at(tracing::Level::INFO);
        let st = log.during(|| {
            rt.block_on(async {
                let store = storage::LocalStore::open(dir.path()).await.unwrap();
                let press = test_now();
                let mut st = session(Some(finished_game("ilios", press, 93, 107)), press);
                let (_, probe) = rejected_tab(&mut st, &store, dir.path(), press, on_frame).await;
                assert!(probe, "93 s after the result, 107 s after the last board");
                st
            })
        });

        let open = st
            .active_game
            .as_ref()
            .expect("the finished game stays open");
        assert_eq!(open.session_id, "ilios", "frame {on_frame}");
        assert_eq!(
            open.outcome,
            detect::MatchOutcome::Defeat,
            "frame {on_frame}: the stored result is kept"
        );
        let lines: Vec<String> = log
            .all()
            .into_iter()
            .filter(|l| l.contains(KEPT_OTHER_RESULT))
            .collect();
        if differs {
            assert_eq!(lines.len(), 1, "one line: {lines:?}");
            let line = &lines[0];
            assert!(line.contains(" INFO "), "{line}");
            assert!(line.contains("session_id=ilios"), "{line}");
            assert!(line.contains("stored_outcome=defeat"), "{line}");
            assert!(line.contains("frame_outcome=victory"), "{line}");
        } else {
            assert_eq!(lines, Vec::<String>::new(), "the words agree: no line");
        }
    }
}

/// A probe Tab on the Ilios game's rank screen (93 s after its Defeat, 107 s
/// after its last board) whose rejected frame shows Defeat, with `between`
/// run on the state while the capture runs, under `log`. Returns the state
/// and the fresh session of the Tab's request.
fn probe_tab_with_change_while_capturing(
    log: &Logged,
    between: impl FnOnce(&mut SessionState),
) -> (SessionState, String) {
    let dir = tempfile::tempdir().unwrap();
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap();
    log.during(|| {
        rt.block_on(async {
            let store = storage::LocalStore::open(dir.path()).await.unwrap();
            let press = test_now();
            let mut st = session(Some(finished_game("ilios", press, 93, 107)), press);
            let req =
                tab_capture_request(
                    &mut st,
                    &store,
                    dir.path(),
                    press,
                    || std::future::ready(()),
                )
                .await;
            assert!(req.finished_game_probe.is_some(), "a probe");
            let fresh = req.session_id.clone();
            between(&mut st);
            let mut report = rejected_report(&fresh);
            report.outcome = detect::MatchOutcome::Defeat;
            let result = Ok(report);
            let landed = press + Duration::from_millis(420);
            let sid = settle_tab_probe(
                &mut st,
                &store,
                dir.path(),
                fresh.clone(),
                &result,
                landed,
                || std::future::ready(()),
            )
            .await;
            apply_capture_report(&mut st, &store, dir.path(), &sid, result, landed).await;
            (st, fresh)
        })
    })
}

/// A result screen keeps only the game the probe was for. If another game
/// opened while the capture ran (a start screen), that game stays, the
/// Tab's report is not applied to it, and the log says so. The finished
/// game is not reopened either.
#[test]
fn a_result_screen_probe_after_another_game_opened_leaves_that_game_alone() {
    let log = Logged::at(tracing::Level::INFO);
    let (st, fresh) = probe_tab_with_change_while_capturing(&log, |st| {
        let mut other =
            ActiveGame::open_now("other".into(), detect::MatchOutcome::Unknown, Vec::new());
        other.awaiting_first_board = true;
        st.active_game = Some(other);
    });
    let open = st.active_game.as_ref().expect("the other game stays open");
    assert_eq!(open.session_id, "other", "not ilios, not {fresh}");
    assert_eq!(
        open.outcome,
        detect::MatchOutcome::Unknown,
        "the Tab's Defeat is not the other game's"
    );
    let warned = log.warned("another game opened while this Tab was read");
    assert_eq!(warned.len(), 1, "{:?}", log.all());
    assert!(warned[0].contains(&fresh), "{}", warned[0]);
}

/// With no game open when the capture lands (the finished game was closed
/// while it ran), a result screen opens the Tab's fresh session as before
/// #810, and that session takes the frame's Defeat.
#[test]
fn a_result_screen_probe_with_no_game_open_opens_the_fresh_session() {
    let log = Logged::at(tracing::Level::INFO);
    let (st, fresh) = probe_tab_with_change_while_capturing(&log, |st| {
        st.active_game = None;
    });
    let open = st.active_game.as_ref().expect("the fresh session opens");
    assert_eq!(open.session_id, fresh);
    assert_eq!(open.outcome, detect::MatchOutcome::Defeat);
    let adopted: Vec<String> = log
        .all()
        .into_iter()
        .filter(|l| l.contains("outcome recovered from rejected capture frame: adopting"))
        .collect();
    assert_eq!(adopted.len(), 1, "{:?}", log.all());
    assert!(adopted[0].contains(&fresh), "{}", adopted[0]);
}

/// The left-out line is said once per game, and the memory of games it was
/// said for is bounded: after 16 newer games the first is forgotten and its
/// next rank screen tick says it again. Other tests share the memory, which
/// only makes the first game forgotten sooner.
#[test]
fn the_left_out_memory_keeps_only_the_newest_games() {
    let dir = tempfile::tempdir().unwrap();
    let empty = |i: usize| {
        ActiveGame::open_now(
            format!("bound-{i:02}"),
            detect::MatchOutcome::Defeat,
            Vec::new(),
        )
    };
    let log = Logged::at(tracing::Level::INFO);
    log.during(|| {
        for i in 0..17 {
            record_rank_crops(dir.path(), Some(&empty(i)), Some(&rank_pngs()));
        }
        record_rank_crops(dir.path(), Some(&empty(0)), Some(&rank_pngs()));
    });
    let first: Vec<String> = log
        .all()
        .into_iter()
        .filter(|l| l.contains(LEFT_OUT) && l.contains("bound-00"))
        .collect();
    assert_eq!(first.len(), 2, "forgotten after 16 newer games: {first:?}");
    assert_eq!(kit_folders(dir.path()), Vec::<String>::new());
}

/// A probe Tab whose rejected frame shows no result (a menu, a Tab let go
/// early in the next game) still closes the finished game and opens the
/// fresh session, as before #810.
#[tokio::test]
async fn a_probe_tab_on_a_frame_with_no_result_still_opens_the_fresh_session() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let press = test_now();
    let mut st = session(Some(finished_game("ilios", press, 93, 107)), press);

    let (fresh, probe) = rejected_tab(
        &mut st,
        &store,
        dir.path(),
        press,
        detect::MatchOutcome::Unknown,
    )
    .await;
    assert!(probe);
    let open = st.active_game.as_ref().expect("the fresh session");
    assert_eq!(open.session_id, fresh);
    assert!(!open.finished() && !open.session_created);
}

/// The same Tab on the rank screen, but the finished game's last board is
/// too old for a probe (200 s), so the session opens at the press, before
/// any frame is seen, and takes the frame's Defeat. That empty session gets
/// no kit: the rank screen's crops are left out, and said so once.
#[tokio::test]
async fn an_empty_session_a_tab_on_the_rank_screen_opened_gets_no_kit() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let press = test_now();
    let mut st = session(Some(finished_game("oasis", press, 93, 200)), press);
    kit_of_a_recorded_game(dir.path(), "oasis");

    let (fresh, probe) = rejected_tab(
        &mut st,
        &store,
        dir.path(),
        press,
        detect::MatchOutcome::Defeat,
    )
    .await;
    assert!(
        !probe,
        "200 s after the last board: no probe, the press opens the session"
    );
    let open = st.active_game.as_ref().expect("the Tab's session");
    assert_eq!(open.session_id, fresh);
    assert_eq!(
        open.outcome,
        detect::MatchOutcome::Defeat,
        "the rank screen's word went onto the empty session, as on 2026-11-21"
    );
    assert!(!open.session_created, "and it has no stored board");

    let log = Logged::at(tracing::Level::INFO);
    log.during(|| rank_screen_ticks(&st, dir.path()));

    assert_eq!(
        orphan_kits(dir.path()),
        Vec::<String>::new(),
        "no kit of pictures without reads"
    );
    assert_eq!(
        kit_folders(dir.path()),
        ["oasis"],
        "only the recorded game's kit"
    );
    let left_out: Vec<String> = log
        .all()
        .into_iter()
        .filter(|l| l.contains(LEFT_OUT))
        .collect();
    assert_eq!(left_out.len(), 1, "said once for three ticks: {left_out:?}");
    assert!(left_out[0].contains(&fresh), "{}", left_out[0]);
}

/// An end screen with no game open: the poll confirmed DEFEAT on the
/// accolade screen, a Tab on that screen opened a session that took the
/// result, and the rank screen came next. Before #810 that session got the
/// rank screen's crops as a kit, with no board ever seen.
#[tokio::test]
async fn an_end_screen_with_no_live_game_leaves_no_kit() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let press = test_now();
    let mut st = session(None, press);
    // Two accolade ticks, 3 s apart, with no game open: a confirmed result
    // kept for the next session.
    for secs_ago in [20, 17] {
        let at = press - Duration::from_secs(secs_ago);
        let tick = resolve_word_tick(
            &mut st.word_outcome_streak,
            Some((
                detect::MatchOutcome::Defeat,
                detect::match_end::OutcomeSource::ResultWord,
            )),
            at,
            None,
        );
        apply_poll_tick(
            &mut st,
            &store,
            dir.path(),
            &tick,
            None,
            at,
            Duration::from_secs(120),
            true,
        )
        .await;
    }
    assert!(st.active_game.is_none() && st.pending_outcome.is_some());

    let (fresh, probe) = rejected_tab(
        &mut st,
        &store,
        dir.path(),
        press,
        detect::MatchOutcome::Defeat,
    )
    .await;
    assert!(!probe);
    let open = st.active_game.as_ref().expect("the Tab's session");
    assert_eq!(open.session_id, fresh);
    assert!(open.finished() && !open.session_created);

    rank_screen_ticks(&st, dir.path());
    assert_eq!(
        kit_folders(dir.path()),
        Vec::<String>::new(),
        "no board was ever seen: no kit"
    );
}

/// The rule itself: the crops go to a game with a stored board, and to a
/// game a start screen opened that still waits for its first board (a
/// post-match Tab can bring it). A game a Tab opened with no board gets
/// none.
#[test]
fn rank_crops_need_a_stored_board_or_a_start_screen_game() {
    let dir = tempfile::tempdir().unwrap();
    let open = |session: &str| {
        ActiveGame::open_now(
            session.to_string(),
            detect::MatchOutcome::Defeat,
            Vec::new(),
        )
    };

    let mut recorded = open("recorded");
    recorded.session_created = true;
    assert!(rank_crops_belong_to(&recorded));
    record_rank_crops(dir.path(), Some(&recorded), Some(&rank_pngs()));

    let mut started = open("started");
    started.awaiting_first_board = true;
    assert!(rank_crops_belong_to(&started));
    record_rank_crops(dir.path(), Some(&started), Some(&rank_pngs()));

    let empty = open("empty");
    assert!(
        is_empty_tab_game(&empty),
        "a Tab opened it and no board came"
    );
    assert!(!rank_crops_belong_to(&empty));
    record_rank_crops(dir.path(), Some(&empty), Some(&rank_pngs()));

    assert_eq!(kit_folders(dir.path()), ["recorded", "started"]);
}

/// The session rule itself: only a report with no board, no verdict and a
/// result on the frame is a result screen.
#[test]
fn only_a_rejected_frame_with_a_result_is_a_result_screen() {
    let report = |outcome| {
        let mut r = rejected_report("fresh");
        r.outcome = outcome;
        r
    };
    assert!(probe_tab_shows_result_screen(&Ok(report(
        detect::MatchOutcome::Defeat
    ))));
    assert!(probe_tab_shows_result_screen(&Ok(report(
        detect::MatchOutcome::Victory
    ))));
    // a rejected frame with no result on it
    assert!(!probe_tab_shows_result_screen(&Ok(report(
        detect::MatchOutcome::Unknown
    ))));
    // a failed capture
    assert!(!probe_tab_shows_result_screen(&Err("grab failed".into())));
    // a board: stored, or compared with the finished game's
    let mut stored = report(detect::MatchOutcome::Defeat);
    stored.recorded = true;
    assert!(!probe_tab_shows_result_screen(&Ok(stored)));
    let mut compared = report(detect::MatchOutcome::Defeat);
    compared.finished_game_tab = Some(boundary::FinishedGameTab::NewGame);
    assert!(!probe_tab_shows_result_screen(&Ok(compared)));
}
