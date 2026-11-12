//! Daemon tests: the game's real start and end. A pre-game screen that
//! opens the game is its start; the result the tracker read itself is its
//! end, only when that read is fresh; both go on the stored rows through
//! the daemon paths that write them. A missed start or end still records
//! the game, with times that make the app warn. See `mod.rs`.

use super::*;
use stat_tracker::model::MatchTimes;

const MIN: Duration = Duration::from_secs(60);

/// Seconds from `a` to `b`, to the nearest second; `None` without both.
fn secs(a: Option<SurrealDatetime>, b: Option<SurrealDatetime>) -> Option<i64> {
    let a: chrono::DateTime<Utc> = a?.into();
    let b: chrono::DateTime<Utc> = b?.into();
    Some(((b - a).num_milliseconds() as f64 / 1000.0).round() as i64)
}

/// End minus start, to the nearest second; `None` without both.
fn length(t: &MatchTimes) -> Option<i64> {
    secs(t.started_at, t.ended_at)
}

/// `stored` is the wall time of `at`, within a second.
fn is_at(stored: Option<SurrealDatetime>, at: Instant) -> bool {
    stored.is_some_and(|s| {
        let s: chrono::DateTime<Utc> = s.into();
        (s - wall_clock(at)).num_milliseconds().abs() < 1000
    })
}

fn hint(outcome: detect::MatchOutcome, at: Instant) -> Option<ResultMark> {
    Some(ResultMark {
        outcome,
        confirmed: false,
        seen_at: at,
    })
}

fn confirmed(outcome: detect::MatchOutcome) -> ResolvedWordTick {
    ResolvedWordTick {
        signal: Some(outcome),
        signal_confirmed: true,
        accolade_map: None,
    }
}

async fn poll(
    st: &mut SessionState,
    store: &storage::LocalStore,
    dir: &std::path::Path,
    tick: ResolvedWordTick,
    now: Instant,
) {
    apply_poll_tick(st, store, dir, &tick, None, now, night_debounce(), true).await;
}

/// An open game `sid` with one stored row and a board, opened by a
/// pre-game screen at `start`, its last board taken at `last_board`. Its
/// times are on the row, the way the run loop stores them after a capture.
async fn open_game(
    store: &storage::LocalStore,
    sid: &str,
    start: Instant,
    last_board: Option<Instant>,
) -> ActiveGame {
    store.insert_match(unfinished_row(sid)).await.unwrap();
    let mut g = game(detect::MatchOutcome::Unknown, None, start);
    g.session_id = sid.into();
    g.gate = Some(GateState::default());
    g.note_match_start(start);
    g.last_stats_at = last_board;
    g.last_board_pressed_at = last_board;
    assert!(store_match_times(store, &g).await);
    g
}

async fn stored_times(store: &storage::LocalStore, sid: &str) -> Vec<Option<MatchTimes>> {
    let rows = store.get_session_snapshots(sid).await.unwrap();
    assert!(!rows.is_empty(), "game {sid} has rows");
    rows.into_iter().map(|r| r.match_times).collect()
}

/// Close `g` the way a new-game boundary does and read back its rows.
async fn close_and_read(
    g: ActiveGame,
    seal: Option<detect::MatchOutcome>,
) -> Vec<storage::PersonalMatch> {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let sid = g.session_id.clone();
    store.insert_match(unfinished_row(&sid)).await.unwrap();
    let mut st = session(Some(g), test_now());
    retire_active_game(&mut st, &store, dir.path(), seal, "test close").await;
    store.get_session_snapshots(&sid).await.unwrap()
}

// ---- A stale hint is not the end.

/// A misread VICTORY at 5:00, one progressed Tab at 6:00,
/// then the real VICTORY confirmed at 12:00. The poll confirm keeps the
/// 5:00 sighting on its mark, but that mark is stale: the end is the
/// confirm, and the rows say 12:00, not 5:00. The old rule (any mark of
/// the same result) stored 5:00.
#[tokio::test]
async fn a_stale_misread_is_not_the_end_of_a_game_confirmed_later() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let mut g = open_game(&store, "stale", t0, Some(t0 + 6 * MIN)).await;
    g.result_mark = hint(detect::MatchOutcome::Victory, t0 + 5 * MIN);
    g.progressed_boards = 1;
    let mut st = session(Some(g), t0);
    let confirm = t0 + 12 * MIN;
    poll(
        &mut st,
        &store,
        dir.path(),
        confirmed(detect::MatchOutcome::Victory),
        confirm,
    )
    .await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.outcome, detect::MatchOutcome::Victory);
    assert!(is_at(g.match_times.ended_at, confirm));
    for times in stored_times(&store, "stale").await {
        assert_eq!(length(&times.expect("times stored")), Some(720), "not 5:00");
    }
}

/// The end rule, one condition at a time. A fresh mark (no progressed
/// board after it, at most 2 minutes old, not before the last board) is
/// the end; any other mark is not, and the read itself is, unless even
/// that is before the last board. Every case but the first stored the
/// mark under the old rule.
#[test]
fn only_a_fresh_mark_is_the_end_and_never_one_before_the_last_board() {
    let t0 = test_now();
    let victory = detect::MatchOutcome::Victory;
    // (mark, last board, progressed after the mark, read at, expected end)
    let s = |n: u64| t0 + Duration::from_secs(n);
    let cases = [
        (s(700), s(600), 0, s(730), Some(s(700)), "fresh: the mark"),
        (s(601), s(600), 0, s(730), Some(s(730)), "over 2 min old"),
        (
            s(590),
            s(600),
            0,
            s(620),
            Some(s(620)),
            "before the last board",
        ),
        (
            s(700),
            s(600),
            1,
            s(730),
            Some(s(730)),
            "a progressed board after it",
        ),
        (
            s(700),
            s(800),
            0,
            s(730),
            None,
            "even the read is before the last board",
        ),
    ];
    for (mark, last, progressed, now, want, why) in cases {
        let mut g = game(detect::MatchOutcome::Unknown, None, t0);
        g.note_match_start(t0);
        g.last_board_pressed_at = Some(last);
        g.result_mark = hint(victory, mark);
        g.note_match_end(victory, now, progressed);
        match want {
            Some(at) => assert!(is_at(g.match_times.ended_at, at), "{why}"),
            None => assert_eq!(g.match_times.ended_at, None, "{why}"),
        }
    }
    // A mark of another result is not this read's; no board yet is fine.
    let mut g = game(detect::MatchOutcome::Unknown, None, t0);
    g.result_mark = hint(detect::MatchOutcome::Defeat, s(10));
    g.note_match_end(victory, s(20), 0);
    assert!(is_at(g.match_times.ended_at, s(20)));
    // The first end stays; an undecided result is no end.
    g.note_match_end(victory, s(90), 0);
    assert!(is_at(g.match_times.ended_at, s(20)));
    let mut open = game(detect::MatchOutcome::Unknown, None, t0);
    open.note_match_end(detect::MatchOutcome::Unknown, s(20), 0);
    assert_eq!(open.match_times.ended_at, None);
}

/// After a restart the last board's press is not kept: the report time
/// that stored it stands in for it.
#[test]
fn the_last_board_is_its_press_else_the_report_that_stored_it() {
    let t0 = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, t0);
    assert_eq!(g.last_tracked_frame(), None);
    g.last_stats_at = Some(t0 + MIN);
    assert_eq!(g.last_tracked_frame(), Some(t0 + MIN));
    g.last_board_pressed_at = Some(t0 + Duration::from_secs(58));
    assert_eq!(g.last_tracked_frame(), Some(t0 + Duration::from_secs(58)));
}

/// The seal at the next game's start keeps the hint's sighting whatever
/// its age, but not one from before the game's last board: then the
/// game records with its start and no end. The old rule stored the mark.
#[tokio::test]
async fn a_sealed_hint_keeps_its_mark_unless_a_board_came_after_it() {
    let t0 = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, t0);
    g.session_id = "sealed".into();
    g.note_match_start(t0);
    g.last_board_pressed_at = Some(t0 + 8 * MIN);
    g.result_mark = hint(detect::MatchOutcome::Victory, t0 + 9 * MIN);
    let rows = close_and_read(g, Some(detect::MatchOutcome::Victory)).await;
    assert_eq!(rows.len(), 1);
    assert_eq!(rows[0].outcome, "victory");
    assert_eq!(length(rows[0].match_times.as_ref().unwrap()), Some(540));

    let mut g = game(detect::MatchOutcome::Unknown, None, t0);
    g.session_id = "sealed-early".into();
    g.note_match_start(t0);
    g.last_board_pressed_at = Some(t0 + 10 * MIN);
    g.result_mark = hint(detect::MatchOutcome::Victory, t0 + 5 * MIN);
    let rows = close_and_read(g, Some(detect::MatchOutcome::Victory)).await;
    assert_eq!(rows.len(), 1, "the game stays recorded");
    assert_eq!(rows[0].outcome, "victory");
    let times = rows[0].match_times.as_ref().expect("times stored");
    assert!(times.started_at.is_some());
    assert_eq!(times.ended_at, None, "not 5:00");
}

// ---- A result set in the app, through the command queue.

async fn run_commands(st: &mut SessionState, store: &storage::LocalStore, dir: &std::path::Path) {
    let backoff = std::sync::Arc::new(std::sync::Mutex::new(sync::SyncBackoff::default()));
    let mut sync_task = None;
    apply_queued_commands(st, store, dir, &mut sync_task, &backoff, None).await;
}

fn queue(dir: &std::path::Path, sid: &str, set: &str, edit: &str) {
    storage::queue_command(
        dir,
        &storage::StoreCommand::SetOutcome {
            session_id: sid.into(),
            outcome: set.into(),
        },
    )
    .unwrap();
    // Queued names sort by time: keep the edit after the set.
    std::thread::sleep(std::time::Duration::from_millis(2));
    storage::queue_command(
        dir,
        &storage::StoreCommand::EditMatch {
            session_id: sid.into(),
            edit: storage::MatchEdit {
                outcome: Some(edit.into()),
                ..Default::default()
            },
        },
    )
    .unwrap();
}

/// SetOutcome and EditMatch with an outcome, through the daemon's command
/// tick, on an open game with stored rows. A game with no end read stays
/// without one in memory, on its rows, and after it closes. A hand-set
/// result that noted an end, or a store write that dropped the times,
/// fails here.
#[tokio::test]
async fn a_result_set_in_the_app_is_no_end_through_the_command_queue() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let g = open_game(&store, "by-hand", t0, Some(t0 + MIN)).await;
    let mut st = session(Some(g), t0);
    queue(dir.path(), "by-hand", "victory", "defeat");
    run_commands(&mut st, &store, dir.path()).await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.outcome, detect::MatchOutcome::Defeat);
    assert!(g.outcome_locked);
    assert!(g.match_times.started_at.is_some());
    assert_eq!(g.match_times.ended_at, None);
    for times in stored_times(&store, "by-hand").await {
        let times = times.expect("times kept by the command writes");
        assert!(times.started_at.is_some());
        assert_eq!(times.ended_at, None);
    }
    retire_active_game(&mut st, &store, dir.path(), None, "test close").await;
    for times in stored_times(&store, "by-hand").await {
        let times = times.expect("times stored at close");
        assert!(times.started_at.is_some());
        assert_eq!(times.ended_at, None, "a hand-set result is no end");
    }
}

/// The same commands on a game whose end the tracker read keep that end:
/// in memory, on the rows, and after the close.
#[tokio::test]
async fn a_result_set_in_the_app_keeps_the_end_the_tracker_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let mut g = open_game(&store, "read-end", t0, Some(t0 + MIN)).await;
    g.result_mark = hint(detect::MatchOutcome::Victory, t0 + 10 * MIN);
    let mut st = session(Some(g), t0);
    poll(
        &mut st,
        &store,
        dir.path(),
        confirmed(detect::MatchOutcome::Victory),
        t0 + 10 * MIN + Duration::from_secs(4),
    )
    .await;
    let read = st.active_game.as_ref().unwrap().match_times.clone();
    assert_eq!(length(&read), Some(600));
    queue(dir.path(), "read-end", "defeat", "draw");
    run_commands(&mut st, &store, dir.path()).await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.outcome, detect::MatchOutcome::Draw);
    assert_eq!(g.match_times, read);
    retire_active_game(&mut st, &store, dir.path(), None, "test close").await;
    for times in stored_times(&store, "read-end").await {
        assert_eq!(times.as_ref(), Some(&read));
    }
}

/// A result set by hand before the end screen does not block
/// the end. A later confirmed read of the same result is the end; the
/// hand-set result and its lock stay. A read of another result is not.
#[tokio::test]
async fn a_result_set_by_hand_takes_the_end_from_a_later_matching_read() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let mut g = open_game(&store, "early-hand", t0, Some(t0 + MIN)).await;
    g.record_member_outcome(detect::MatchOutcome::Victory);
    let mut st = session(Some(g), t0);
    let defeat = detect::MatchOutcome::Defeat;
    poll(&mut st, &store, dir.path(), confirmed(defeat), t0 + 8 * MIN).await;
    assert_eq!(st.active_game.as_ref().unwrap().match_times.ended_at, None);
    let victory = detect::MatchOutcome::Victory;
    poll(
        &mut st,
        &store,
        dir.path(),
        confirmed(victory),
        t0 + 9 * MIN,
    )
    .await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.outcome, victory);
    assert!(g.outcome_locked, "still the hand-set result");
    assert_eq!(length(&g.match_times), Some(540));
    for times in stored_times(&store, "early-hand").await {
        assert_eq!(length(&times.unwrap()), Some(540));
    }
}

// ---- Starts and ends on the daemon paths.

/// A map vote, hero select or ban with nothing open opens a game with
/// its start; a Tab-opened game has none.
#[test]
fn a_pre_game_screen_opens_the_game_with_its_start_and_a_tab_does_not() {
    let now = test_now();
    for screen in [
        vote(&["Busan", "Ilios"]),
        boundary::StartScreen::HeroSelect,
        boundary::StartScreen::HeroBan,
    ] {
        let dir = tempfile::tempdir().unwrap();
        let mut st = session(None, now);
        open_detected_game(&mut st, dir.path(), &screen, now, Duration::from_secs(120));
        let g = st.active_game.as_ref().expect("the screen opens a game");
        assert!(is_at(g.match_times.started_at, now), "{screen:?}");
        assert_eq!(g.match_times.ended_at, None);
    }
    let tab = tab_opened_game("tab".into(), None, None, now);
    assert_eq!(tab.match_times, MatchTimes::default());
}

/// The poll boundary: a ban on a live game closes it (its rows keep its
/// start, with no end) and opens the next game with its start.
#[tokio::test]
async fn a_ban_on_a_live_game_starts_the_next_one() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let g = open_game(&store, "live", t0, Some(t0 + MIN)).await;
    let mut st = session(Some(g), t0);
    let ban = t0 + 15 * MIN;
    let tick = ResolvedWordTick {
        signal: None,
        signal_confirmed: false,
        accolade_map: None,
    };
    let screen = Some(boundary::StartScreen::HeroBan);
    apply_poll_tick(
        &mut st,
        &store,
        dir.path(),
        &tick,
        screen,
        ban,
        night_debounce(),
        true,
    )
    .await;
    let next = st.active_game.as_ref().unwrap();
    assert_ne!(next.session_id, "live");
    assert!(is_at(next.match_times.started_at, ban));
    assert_eq!(next.match_times.ended_at, None);
    for times in stored_times(&store, "live").await {
        let times = times.unwrap();
        assert!(is_at(times.started_at, t0));
        assert_eq!(times.ended_at, None, "the next game's start is no end");
    }
}

/// A game that opens decided read its end. A different-map
/// end screen opens one at the read, with no start. A stat reset opens
/// one with neither.
#[tokio::test]
async fn a_game_the_end_screen_opens_has_its_end_and_a_reset_split_has_none() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    for (reason, outcome) in [
        (
            boundary::CloseReason::EndScreenMap,
            detect::MatchOutcome::Victory,
        ),
        (
            boundary::CloseReason::StatReset,
            detect::MatchOutcome::Unknown,
        ),
    ] {
        let mut st = session(Some(game(detect::MatchOutcome::Unknown, None, t0)), t0);
        let open = boundary::OpenNew {
            reason,
            seal_outcome: None,
            new_outcome: outcome,
            new_map: Some("Ilios".into()),
            candidates: Vec::new(),
            new_result: None,
        };
        let at = t0 + 20 * MIN;
        let decision = boundary::PollDecision::Open(open);
        apply_poll_decision(&mut st, &store, dir.path(), decision, at, true).await;
        let g = st.active_game.as_ref().unwrap();
        assert_eq!(g.match_times.started_at, None, "{reason:?}");
        if outcome.is_decided() {
            assert!(is_at(g.match_times.ended_at, at), "{reason:?}");
        } else {
            assert_eq!(g.match_times.ended_at, None, "{reason:?}");
        }
    }
}

/// A gap split whose board shows the result ends the new game
/// at the Tab press; an inherited pending result ends it when that
/// result was confirmed.
#[tokio::test]
async fn a_split_or_an_inherited_result_is_the_new_games_end() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let old = open_game(&store, "before-split", t0, Some(t0 + MIN)).await;
    store.insert_match(unfinished_row("split")).await.unwrap();
    let mut st = session(Some(old), t0);
    let press = t0 + 20 * MIN;
    st.tab_press = Some(press);
    let mut report = rejected_report("split");
    report.split = true;
    report.recorded = true;
    report.outcome = detect::MatchOutcome::Victory;
    let landed = press + Duration::from_secs(5);
    apply_capture_report(
        &mut st,
        &store,
        dir.path(),
        "before-split",
        Ok(report),
        landed,
    )
    .await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.session_id, "split");
    assert_eq!(g.match_times.started_at, None);
    assert!(
        is_at(g.match_times.ended_at, press),
        "the press, not the report"
    );
    store_open_game_times(&st, &store, "split").await;
    for times in stored_times(&store, "split").await {
        assert!(is_at(times.unwrap().ended_at, press));
    }

    let mut st = session(None, t0);
    st.pending_outcome = Some((detect::MatchOutcome::Defeat, t0));
    open_tab_game(
        &mut st,
        &store,
        dir.path(),
        "inherits".into(),
        t0 + 30 * Duration::from_secs(1),
    )
    .await;
    let g = st.active_game.as_ref().unwrap();
    assert_eq!(g.outcome, detect::MatchOutcome::Defeat);
    assert_eq!(g.match_times.started_at, None);
    assert!(is_at(g.match_times.ended_at, t0));
}

/// A missed start: a Tab-opened game stores empty times on
/// the row its first capture wrote, so the app warns instead of reading
/// it as an older row. A result on a rejected board later ends it at the
/// press and writes that to the rows at once; the close keeps both.
#[tokio::test]
async fn a_tab_opened_game_records_with_no_start_and_its_end_from_a_board() {
    let mut night = Night::new().await;
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    let sid = night.id();
    store_open_game_times(&night.st, &night.store, &sid).await;
    let rows = night.store.get_session_snapshots(&sid).await.unwrap();
    let stored = rows.len();
    assert!(stored >= 1, "the capture stored the row");
    for row in &rows {
        assert_eq!(row.match_times, Some(MatchTimes::default()));
    }
    let shown = stat_tracker::games::model::Game::from_match(&rows[0]);
    assert_eq!(shown.match_times, Some(Default::default()));

    let press = night.now + 10 * MIN;
    night.st.tab_press = Some(press);
    let mut report = rejected_report(&sid);
    report.outcome = detect::MatchOutcome::Defeat;
    let landed = press + Duration::from_secs(4);
    apply_capture_report(
        &mut night.st,
        &night.store,
        night.dir.path(),
        &sid,
        Ok(report),
        landed,
    )
    .await;
    for times in stored_times(&night.store, &sid).await {
        let times = times.unwrap();
        assert_eq!(times.started_at, None);
        assert!(
            is_at(times.ended_at, press),
            "written at the read, at the press"
        );
    }
    retire_active_game(
        &mut night.st,
        &night.store,
        night.dir.path(),
        None,
        "test close",
    )
    .await;
    let rows = night.store.get_session_snapshots(&sid).await.unwrap();
    assert_eq!(rows.len(), stored, "the game stays recorded");
    for row in &rows {
        assert_eq!(row.outcome, "defeat");
        assert!(is_at(row.match_times.as_ref().unwrap().ended_at, press));
    }
}

/// The most used end on a Tab: a stored board (recorded, not a split,
/// with a gate) that shows the result ends the game at the press, and the
/// end is written to the rows at once.
#[tokio::test]
async fn a_stored_board_that_shows_the_result_ends_the_game_at_the_press() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let g = open_game(&store, "board-end", t0, Some(t0 + MIN)).await;
    let mut st = session(Some(g), t0);
    let press = t0 + 10 * MIN;
    st.tab_press = Some(press);
    let mut report = rejected_report("board-end");
    report.recorded = true;
    report.outcome = detect::MatchOutcome::Victory;
    report.gate_state = Some(GateState::default());
    assert!(!report.split);
    apply_capture_report(
        &mut st,
        &store,
        dir.path(),
        "board-end",
        Ok(report),
        press + Duration::from_secs(4),
    )
    .await;
    let g = st.active_game.as_ref().expect("the game stays open");
    assert_eq!(g.outcome, detect::MatchOutcome::Victory);
    assert!(is_at(g.match_times.ended_at, press), "the end is the press");
    for times in stored_times(&store, "board-end").await {
        let times = times.expect("times stored");
        assert!(is_at(times.started_at, t0));
        assert!(is_at(times.ended_at, press), "written at the read");
    }
}

/// The run loop writes the open game's times after each stored capture,
/// before it cuts the live snapshot, so the snapshot carries them.
#[test]
fn the_run_loop_stores_the_times_before_the_snapshot() {
    let body = fn_source("run_loop");
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(at("store_open_game_times(") < at("refresh_snapshot_after_capture("));
}

/// The shutdown write of an imported hold's times marks the snapshot when
/// it stores them. For a finished game the close that follows finds the
/// times already stored and would not mark it.
#[test]
fn the_shutdown_hold_times_write_refreshes_the_snapshot() {
    let body: String = fn_source("close_session_on_shutdown")
        .split_whitespace()
        .collect();
    assert!(
        body.contains(
            "ifstore_match_times(store,game).await{refresh_snapshot(store,data_dir).await;}"
        ),
        "the times write must mark the snapshot"
    );
}

/// A missed end: a game a hero select opened, with a stored board and no
/// result, closes at the next game's ban. It stays recorded with its
/// start and no end, so the app warns.
#[tokio::test]
async fn a_game_with_no_result_records_with_its_start_and_no_end() {
    let mut night = Night::new().await;
    let start = night.now;
    night.poll_screen(boundary::StartScreen::HeroSelect).await;
    let sid = night.id();
    night.advance(MIN);
    night
        .tab_once(
            night_counters(14, 22, 6, 2400, 9800, 400),
            "Zenyatta",
            Some(2),
            Some("Busan"),
            NIGHT_CLEAN,
        )
        .await;
    assert_eq!(night.id(), sid, "the Tab is the game the screen opened");
    store_open_game_times(&night.st, &night.store, &sid).await;
    night.advance(12 * MIN);
    night.poll_screen(boundary::StartScreen::HeroBan).await;
    assert_ne!(night.id(), sid);
    let rows = night.store.get_session_snapshots(&sid).await.unwrap();
    assert!(!rows.is_empty(), "the game stays recorded");
    for row in &rows {
        let times = row.match_times.as_ref().expect("times stored");
        assert!(is_at(times.started_at, start));
        assert_eq!(times.ended_at, None);
    }
}

/// A board that shows the result was wrong clears the end with the
/// result, in memory and on the rows. The start stays.
#[tokio::test]
async fn unlatching_a_false_result_clears_its_end() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let mut g = open_game(&store, "unlatch", t0, Some(t0 + MIN)).await;
    g.record_outcome_at(detect::MatchOutcome::Victory, t0 + 2 * MIN);
    g.note_match_end(detect::MatchOutcome::Victory, t0 + 2 * MIN, 0);
    assert!(store_match_times(&store, &g).await);
    let mut st = session(Some(g), t0);
    unlatch_false_outcome(&mut st, &store, dir.path(), t0 + 3 * MIN).await;
    let g = st.active_game.as_ref().unwrap();
    assert!(!g.finished());
    assert_eq!(g.match_times.ended_at, None);
    for times in stored_times(&store, "unlatch").await {
        let times = times.unwrap();
        assert!(is_at(times.started_at, t0));
        assert_eq!(times.ended_at, None);
    }
}

/// Stopping the tracker is never a game's end: an open game stays open on
/// disk with no end, and a game with a hand-set result closes with none.
#[tokio::test]
async fn a_shutdown_is_never_an_end() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    // Past instants: a restart maps the skeleton's wall times back.
    let t0 = Instant::now();
    let mut g = open_game(&store, "still-on", t0, Some(t0)).await;
    g.map = Some("Busan".into());
    let mut st = session(Some(g), t0);
    close_session_on_shutdown(&mut st, &store, dir.path()).await;
    let back = startup_session(dir.path()).active_game.expect("resumes");
    assert_eq!(back.session_id, "still-on");
    assert_eq!(back.match_times.ended_at, None);

    let mut g = open_game(&store, "hand-set", t0, Some(t0)).await;
    g.record_member_outcome(detect::MatchOutcome::Victory);
    let mut st = session(Some(g), t0);
    close_session_on_shutdown(&mut st, &store, dir.path()).await;
    assert!(st.active_game.is_none());
    for times in stored_times(&store, "hand-set").await {
        let times = times.unwrap();
        assert!(times.started_at.is_some());
        assert_eq!(times.ended_at, None);
    }
}

/// A capture report that lands while the tracker stops stores
/// the open game's times on its rows, as the run loop does.
#[tokio::test]
async fn a_report_drained_at_shutdown_stores_the_times() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    store.insert_match(unfinished_row("drained")).await.unwrap();
    let mut g = game(detect::MatchOutcome::Unknown, None, t0);
    g.session_id = "drained".into();
    g.session_created = false;
    g.note_match_start(t0);
    let mut st = session(Some(g), t0);
    let (tx, mut rx) = tokio::sync::mpsc::unbounded_channel();
    let mut report = rejected_report("drained");
    report.recorded = true;
    tx.send(("drained".to_string(), Ok(report))).unwrap();
    apply_landed_capture_reports(&mut st, &mut rx, &store, dir.path()).await;
    for times in stored_times(&store, "drained").await {
        let times = times.expect("the drain stores the times");
        assert!(is_at(times.started_at, t0));
    }
}

/// A fresh hold archived at a stale start is its own game with
/// empty times, so it warns; an imported hold takes its session's.
#[tokio::test]
async fn an_archived_fresh_hold_gets_empty_times() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let old = Utc::now() - chrono::Duration::hours(2);
    let hold = persisted_with_hold("stale", detect::MatchOutcome::Unknown, None, false, 7, old);
    begin_from_persisted(dir.path(), &store, hold).await;
    let rows = store.get_all_matches().await.unwrap();
    let held = rows
        .iter()
        .find(|r| r.elims == 7)
        .expect("the hold is kept");
    assert_ne!(held.session_id, "stale", "a fresh hold is its own game");
    assert_eq!(held.match_times, Some(MatchTimes::default()));

    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut hold = persisted_with_hold(
        "imported",
        detect::MatchOutcome::Defeat,
        Some("Nepal"),
        true,
        8,
        old,
    );
    let skeleton = MatchTimes {
        started_at: Some((old - chrono::Duration::minutes(15)).into()),
        ended_at: Some((old - chrono::Duration::minutes(1)).into()),
    };
    hold.match_times = skeleton.clone();
    begin_from_persisted(dir.path(), &store, hold).await;
    let rows = store.get_all_matches().await.unwrap();
    let held = rows
        .iter()
        .find(|r| r.elims == 8)
        .expect("the hold is kept");
    assert_eq!(held.session_id, "imported");
    assert_eq!(
        held.match_times,
        Some(skeleton),
        "the session's first row takes the skeleton's times"
    );
}

/// An imported hold written at shutdown is its game's first row, so it
/// gets the game's times there, also when the game is not finished and
/// stays open on disk.
#[tokio::test]
async fn an_imported_hold_written_at_shutdown_stores_the_times() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let t0 = test_now();
    let mut g = game(detect::MatchOutcome::Unknown, None, t0);
    g.session_id = "carried-in".into();
    g.map = Some("Busan".into());
    g.session_created = false;
    g.note_match_start(t0);
    g.deferred = Some(fresh_counters(3));
    g.deferred_hero = Some("Ana".into());
    g.deferred_imported = true;
    assert!(!g.finished());
    let mut st = session(Some(g), t0);
    close_session_on_shutdown(&mut st, &store, dir.path()).await;
    let stored = stored_times(&store, "carried-in").await;
    assert!(!stored.is_empty(), "the hold is written");
    for times in stored {
        let times = times.expect("the hold's row gets the times");
        assert!(is_at(times.started_at, t0));
        assert_eq!(times.ended_at, None);
    }
}

/// A game with no stored row of its own writes nothing, even
/// onto rows already stored under its id.
#[tokio::test]
async fn a_game_with_no_stored_row_writes_no_times() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    store.insert_match(unfinished_row("test")).await.unwrap();
    let mut g = game(detect::MatchOutcome::Unknown, None, test_now());
    g.session_created = false;
    g.note_match_start(test_now());
    assert!(!store_match_times(&store, &g).await);
    assert_eq!(stored_times(&store, "test").await, vec![None]);
}

/// The close writes the sealed outcome, then the times, then
/// marks the snapshot once, also when only the times changed.
#[test]
fn the_close_writes_the_outcome_then_the_times_then_marks_the_snapshot() {
    let body = fn_source("commit_retired_game");
    let at = |needle: &str| body.find(needle).unwrap_or_else(|| panic!("{needle}"));
    assert!(at("set_session_outcome") < at("store_match_times"));
    assert!(at("store_match_times") < at("refresh_snapshot_game_finished"));
    assert!(body.contains("else if times_changed"));
}

#[test]
fn the_start_and_end_survive_a_restart() {
    let dir = tempfile::tempdir().unwrap();
    let now = Instant::now();
    let mut g = game(detect::MatchOutcome::Unknown, None, now);
    g.session_created = false;
    g.note_match_start(now);
    g.record_outcome_at(detect::MatchOutcome::Victory, now);
    g.note_match_end(detect::MatchOutcome::Victory, now, 0);
    let want = g.match_times.clone();
    assert!(want.ended_at.is_some());
    persist_active_game(dir.path(), Some(&g));
    let back = startup_session(dir.path())
        .active_game
        .expect("the game resumes");
    assert_eq!(back.match_times, want);
}
