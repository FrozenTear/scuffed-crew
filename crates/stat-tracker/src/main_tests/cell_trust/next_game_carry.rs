//! Daemon tests: a new game does not store another game's final stats in
//! the cases #809 covers, and the known limits stay pinned.
//!
//! Runasapi, 17 Nov: game A (13c35793) ends 0/0/3/2007/772/1502; Lijiang
//! and Aatlis follow; game B (13c350d8) opens on a Tab 54 minutes after
//! A's last Tab, and that Tab's board was A's final board again, so B
//! stored it and its gate held B's own 0/0/2/648/307/600 at A's values. A
//! first board equal to a recent game's last stored board is not stored
//! (`finished_board_report`). Separately, a board carried in from the game
//! that closed (a deferred board, `ActiveGame::deferred`) whose numbers are
//! above this game's own sure reads is another game's board and is neither
//! written nor used (`carried_board_fits`). A child of `cell_trust.rs`,
//! kept apart so both files stay under the size limit.

use super::*;

/// Game A's last board, sure on every cell.
const A_FINAL: [u32; 6] = [0, 0, 3, 2007, 772, 1502];
/// Game B's real board.
const B_REAL: [u32; 6] = [0, 0, 2, 648, 307, 600];
/// How long after A's last Tab game B starts.
const B_GAP: Duration = Duration::from_secs(54 * 60);

/// Game A on the "o10" session: three Tabs that climb to [`A_FINAL`], then
/// a defeat. Returns the state, the clock and A's stored rows.
async fn game_a(ctx: &DaemonCtx) -> (SessionState, Instant, Vec<[u32; 6]>) {
    let mut now = test_now();
    let mut st = o10_session(now);
    for (gap, board) in [
        (0, [0, 0, 1, 500, 200, 400]),
        (90, [0, 0, 2, 1200, 480, 900]),
        (90, A_FINAL),
    ] {
        let tab = o10_tab(ctx, &mut st, &mut now, gap, all_sure(board), None).await;
        assert!(tab.recorded && !tab.split, "game A Tab: {tab:?}");
    }
    let g = st.active_game.as_mut().unwrap();
    g.record_outcome_at(detect::MatchOutcome::Defeat, now);
    let rows = o10_rows(ctx, "o10").await;
    assert_eq!(rows.last(), Some(&A_FINAL), "A ends on its final board");
    (st, now, rows)
}

/// Game A still holds its last board as a deferred board when B opens.
fn defer_a_final(st: &mut SessionState) {
    let g = st.active_game.as_mut().unwrap();
    g.deferred = Some(counters_from_cols(A_FINAL));
    g.deferred_hero = Some("Wrecking Ball".into());
    g.deferred_at = Some(Utc::now() - chrono::Duration::minutes(54));
}

/// One Tab of game B through the daemon's own Tab path
/// ([`tab_capture_request`], the capture, the report). `hero` is the hero
/// pack's read of the own row (`None`: not read).
async fn tab_b(
    ctx: &DaemonCtx,
    st: &mut SessionState,
    press: Instant,
    cells: O10Cells,
    hero: Option<&str>,
) {
    tab_b_row(ctx, st, press, cells, hero, None).await;
}

/// [`tab_b`] with the own row O9 found (`None`: taken as found).
async fn tab_b_row(
    ctx: &DaemonCtx,
    st: &mut SessionState,
    press: Instant,
    cells: O10Cells,
    hero: Option<&str>,
    own_row: Option<detect::own_row::OwnRow>,
) {
    let synced = std::cell::Cell::new(0);
    let mut analysis = o8_analysis(None, hero);
    analysis.own_row = own_row;
    let capture = InjectedCapture {
        analysis,
        board: Some(o10_board(cells)),
    };
    r9_tab(st, ctx, press, Ok(capture), &synced).await;
}

/// The session the open game is on, which must not be game A's.
fn game_b_id(st: &SessionState) -> String {
    let id = st.active_game.as_ref().unwrap().session_id.clone();
    assert_ne!(id, "o10", "game B opened a session of its own");
    id
}

/// Replay of the Runasapi pair with no deferred board: A ends, B opens on a
/// Tab 54 minutes later, first with no hero read, then Wrecking Ball. B
/// stores its own reads and A keeps its rows. Covers the Tab-opened path:
/// the fresh session has no gate, no reset baseline and no board time of
/// its own before its first Tab.
#[tokio::test]
async fn a_tab_opened_next_game_stores_its_own_reads() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (mut st, now, a_rows) = game_a(&ctx).await;
    let press = now + B_GAP;
    tab_b(&ctx, &mut st, press, all_sure(B_REAL), None).await;
    let b = game_b_id(&st);
    tab_b(
        &ctx,
        &mut st,
        press + Duration::from_secs(40),
        all_sure(B_REAL),
        Some("Wrecking Ball"),
    )
    .await;
    assert_eq!(game_b_id(&st), b, "B's second Tab stays on B");
    let g = st.active_game.as_ref().unwrap();
    let accepted = g.gate.map(|gate| gate_cols(gate.accepted));
    assert_eq!(accepted, Some(B_REAL), "B's gate is its own");
    assert_eq!(o10_rows(&ctx, &b).await, vec![B_REAL, B_REAL]);
    assert_eq!(o10_rows(&ctx, "o10").await, a_rows, "A's rows kept");
}

/// The fresh session a Tab opens after a finished game starts with no gate,
/// reset baseline or board time, whatever the closed game had.
#[test]
fn a_tab_opened_game_has_no_gate_of_the_game_before() {
    let now = test_now();
    let opened = tab_opened_game(
        "next".into(),
        None,
        Some((counters_from_cols(A_FINAL), None, None)),
        now,
    );
    assert!(opened.gate.is_none());
    assert!(opened.reset_baseline.is_none());
    assert!(opened.last_stats_at.is_none());
    let req = build_capture_request(&opened, true, now + Duration::from_secs(5));
    assert!(req.prev_gate.is_none(), "no gate to latch to");
    assert!(req.reset_baseline.is_none());
}

/// #809 replay: A still holds its final board as a deferred board when B's
/// Tab opens B. The carried board is above B's own sure reads, so it is not
/// B's earlier board: it is not written onto B and seeds nothing. B stores
/// its own reads.
#[tokio::test]
async fn a_deferred_board_above_the_next_games_reads_is_not_carried_on_a_tab_open() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (mut st, now, a_rows) = game_a(&ctx).await;
    defer_a_final(&mut st);
    let press = now + B_GAP;
    tab_b(&ctx, &mut st, press, all_sure(B_REAL), None).await;
    let b = game_b_id(&st);
    tab_b(
        &ctx,
        &mut st,
        press + Duration::from_secs(40),
        all_sure(B_REAL),
        Some("Wrecking Ball"),
    )
    .await;
    assert_eq!(game_b_id(&st), b);
    assert_eq!(
        o10_rows(&ctx, &b).await,
        vec![B_REAL, B_REAL],
        "B stores its own reads, never A's final board"
    );
    assert_eq!(o10_rows(&ctx, "o10").await, a_rows, "A's rows kept");
}

/// #809 replay in the shape the stored rows point to: B's first Tab has no
/// hero read and an own row that is suspect (it may be another player's),
/// with one held cell, while A's final board is carried. Every cell of a
/// suspect row is held by the gate, so before the fix all six took the
/// carried board: B's row stored 0/0/3/2007/772/1502 next to the carried
/// row itself. Now the carried board is not used, the suspect Tab stores
/// nothing, and the next trusted Tab (Wrecking Ball) stores B's own reads.
#[tokio::test]
async fn a_suspect_first_tab_of_the_next_game_takes_nothing_from_a_carried_board() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (mut st, now, a_rows) = game_a(&ctx).await;
    defer_a_final(&mut st);
    let press = now + B_GAP;
    let first = [
        Some((0, true)),
        sure(0),
        sure(2),
        sure(648),
        sure(307),
        sure(600),
    ];
    let suspect_row = Some(combine(Some(0), None, true));
    tab_b_row(&ctx, &mut st, press, first, None, suspect_row).await;
    let b = game_b_id(&st);
    tab_b(
        &ctx,
        &mut st,
        press + Duration::from_secs(40),
        all_sure(B_REAL),
        Some("Wrecking Ball"),
    )
    .await;
    assert_eq!(game_b_id(&st), b);
    let rows = o10_rows(&ctx, &b).await;
    assert!(!rows.contains(&A_FINAL), "A's final board on B: {rows:?}");
    assert_eq!(rows, vec![B_REAL], "B stores its own reads");
    assert_eq!(o10_rows(&ctx, "o10").await, a_rows, "A's rows kept");
}

/// #809 replay on the poll path: a map vote opens B while A holds its final
/// board as a deferred board, so B imports it (`apply_poll_decision`). B's
/// Tabs store B's own reads.
#[tokio::test]
async fn a_deferred_board_above_the_next_games_reads_is_not_carried_on_a_poll_open() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (mut st, now, a_rows) = game_a(&ctx).await;
    defer_a_final(&mut st);
    let vote = now + B_GAP - Duration::from_secs(120);
    apply_poll_decision(
        &mut st,
        &ctx.store,
        &ctx.data_dir,
        boundary::PollDecision::Open(boundary::OpenNew {
            reason: boundary::CloseReason::MapVote,
            seal_outcome: None,
            new_outcome: detect::MatchOutcome::Unknown,
            new_map: None,
            candidates: Vec::new(),
            new_result: None,
        }),
        vote,
        true,
    )
    .await;
    let b = game_b_id(&st);
    let g = st.active_game.as_ref().unwrap();
    assert!(g.deferred_imported, "A's board is carried");
    assert!(g.gate.is_none());
    assert!(g.reset_baseline.is_none());
    assert!(g.last_stats_at.is_none());
    let press = now + B_GAP;
    tab_b(&ctx, &mut st, press, all_sure(B_REAL), None).await;
    tab_b(
        &ctx,
        &mut st,
        press + Duration::from_secs(40),
        all_sure(B_REAL),
        Some("Wrecking Ball"),
    )
    .await;
    assert_eq!(game_b_id(&st), b);
    assert_eq!(
        o10_rows(&ctx, &b).await,
        vec![B_REAL, B_REAL],
        "B stores its own reads, never A's final board"
    );
    assert_eq!(o10_rows(&ctx, "o10").await, a_rows, "A's rows kept");
}

/// Fail-safe: B's healing and mitigation cells come in held (the digit
/// reader does not vouch for them) on the Tab that opens B while A's final
/// board is carried. They take B's own unsure reads, never A's 772 and
/// 1502, flagged unsure, and the game still gets its row: the first Tab
/// leaves a best guess, stored when B closes.
#[tokio::test]
async fn held_cells_of_the_next_game_store_its_own_unsure_reads() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (mut st, now, a_rows) = game_a(&ctx).await;
    defer_a_final(&mut st);
    let held = [
        sure(0),
        sure(0),
        sure(2),
        sure(648),
        Some((307, true)),
        Some((600, true)),
    ];
    tab_b(&ctx, &mut st, now + B_GAP, held, Some("Wrecking Ball")).await;
    let b = game_b_id(&st);
    retire_active_game(&mut st, &ctx.store, &ctx.data_dir, None, "test close").await;
    let rows = ctx.store.get_session_snapshots(&b).await.unwrap();
    assert_eq!(rows.len(), 1, "B is recorded: {rows:?}");
    assert_eq!(o10_stats(&rows[0]), B_REAL, "B's own reads, never A's");
    for field in ["h", "mit"] {
        assert!(
            rows[0].suspect_fields.iter().any(|f| f == field),
            "the held {field} is marked unsure: {:?}",
            rows[0].suspect_fields
        );
    }
    assert_eq!(o10_rows(&ctx, "o10").await, a_rows, "A's rows kept");
}

/// Control: a carried board that is this game's earlier board (at or below
/// every sure read) is still written onto the new game, then its own row.
#[tokio::test]
async fn a_carried_board_below_the_next_games_reads_is_still_written() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (mut st, now, _) = game_a(&ctx).await;
    let first = [0, 0, 1, 210, 90, 150];
    let g = st.active_game.as_mut().unwrap();
    g.deferred = Some(counters_from_cols(first));
    g.deferred_hero = Some("Wrecking Ball".into());
    g.deferred_at = Some(Utc::now() - chrono::Duration::seconds(60));
    tab_b(&ctx, &mut st, now + B_GAP, all_sure(B_REAL), None).await;
    let b = game_b_id(&st);
    let rows = o10_rows(&ctx, &b).await;
    assert_eq!(rows.len(), 2, "{rows:?}");
    assert!(rows.contains(&first), "carried board written: {rows:?}");
    assert!(rows.contains(&B_REAL), "B's own row is written: {rows:?}");
}

/// Lijiang's three stored boards (17 Nov, 21:31 to 21:43).
const LIJIANG: [[u32; 6]; 3] = [
    [0, 0, 0, 0, 0, 0],
    [11, 6, 0, 2285, 566, 977],
    [26, 10, 4, 5254, 1170, 1997],
];
/// Aatlis's one stored board (21:57:52).
const AATLIS: [u32; 6] = [3, 1, 0, 703, 109, 420];

/// One Tab through the daemon's Tab path with the top bar reading `map`.
/// Returns the probe verdict the report carried, as [`r9_tab`] does.
async fn tab_on(
    ctx: &DaemonCtx,
    st: &mut SessionState,
    press: Instant,
    cells: O10Cells,
    hero: Option<&str>,
    map: &str,
) -> Option<boundary::FinishedGameTab> {
    let synced = std::cell::Cell::new(0);
    let mut analysis = o8_analysis(None, hero);
    analysis.map_from_panel = Some(map.into());
    analysis.map_panel_raw = Some(map.to_uppercase());
    let mut board = o10_board(cells);
    if let Some(field) = board.fields.iter_mut().find(|f| f.name == "map") {
        field.value = Some(shadow::Value::Text(map.into()));
    }
    let capture = InjectedCapture {
        analysis,
        board: Some(board),
    };
    r9_tab(st, ctx, press, Ok(capture), &synced).await
}

/// A map vote opens a new game at `at`, closing the open one. Returns the
/// new game's session.
async fn map_vote(ctx: &DaemonCtx, st: &mut SessionState, at: Instant) -> String {
    if st.active_game.is_none() {
        let vote = boundary::StartScreen::MapVote {
            candidates: Vec::new(),
        };
        open_detected_game(st, &ctx.data_dir, &vote, at, Duration::ZERO);
        return st.active_game.as_ref().unwrap().session_id.clone();
    }
    let open = boundary::OpenNew {
        reason: boundary::CloseReason::MapVote,
        seal_outcome: None,
        new_outcome: detect::MatchOutcome::Unknown,
        new_map: None,
        candidates: Vec::new(),
        new_result: None,
    };
    let decision = boundary::PollDecision::Open(open);
    apply_poll_decision(st, &ctx.store, &ctx.data_dir, decision, at, true).await;
    st.active_game.as_ref().unwrap().session_id.clone()
}

/// #809 replay of the 17 Nov journal, one daemon run. A (Runasapi, map
/// vote) ends on [`A_FINAL`]. A map vote opens Lijiang, which stores its
/// own boards, wins and closes quietly. A map vote opens a game that never
/// gets a Tab, and the next one opens Aatlis, with one Tab, left open. 25
/// minutes later a Tab opens B over Aatlis; that Tab shows A's final board
/// again (Runasapi, no hero read). 47 seconds later B's own board reads
/// 0/0/2/648/307/600 as Wrecking Ball. Before the fix B stored A's board
/// on both Tabs (the second held by B's gate at A's numbers).
#[tokio::test]
async fn a_finished_games_board_shown_again_is_not_the_next_games() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let t0 = test_now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut st = session(None, t0);
    let a = map_vote(&ctx, &mut st, t0).await;
    for secs in [216, 221, 232] {
        tab_on(&ctx, &mut st, at(secs), all_sure(A_FINAL), None, "Runasapi").await;
    }
    let lijiang = map_vote(&ctx, &mut st, at(325)).await;
    for (secs, board) in [401, 600, 900].into_iter().zip(LIJIANG) {
        let hero = (secs > 401).then_some("Wrecking Ball");
        tab_on(
            &ctx,
            &mut st,
            at(secs),
            all_sure(board),
            hero,
            "Lijiang Tower",
        )
        .await;
    }
    let g = st.active_game.as_mut().unwrap();
    g.record_outcome_at(detect::MatchOutcome::Victory, at(1050));
    let closed = close_quiet_session_and_sync(
        &mut st,
        &ctx.store,
        &ctx.data_dir,
        at(1177),
        ctx.finished_game_close,
        || std::future::ready(()),
    )
    .await;
    assert!(closed, "Lijiang closes quietly");
    assert!(st.active_game.is_none(), "nothing carried out of Lijiang");
    let empty = map_vote(&ctx, &mut st, at(1290)).await;
    let aatlis = map_vote(&ctx, &mut st, at(1830)).await;
    assert_ne!(empty, aatlis);
    let wb = Some("Wrecking Ball");
    tab_on(&ctx, &mut st, at(1994), all_sure(AATLIS), wb, "Aatlis").await;
    tab_on(&ctx, &mut st, at(3497), all_sure(A_FINAL), None, "Runasapi").await;
    let b = game_b_id(&st);
    assert!(![&a, &lijiang, &empty, &aatlis].contains(&&b), "B is new");
    tab_on(&ctx, &mut st, at(3544), all_sure(B_REAL), wb, "Runasapi").await;
    assert_eq!(game_b_id(&st), b, "B's second Tab stays on B");
    assert_eq!(
        o10_rows(&ctx, &b).await,
        vec![B_REAL],
        "B stores its own reads, never A's final board"
    );
    let g = st.active_game.as_ref().unwrap();
    let accepted = g.gate.map(|gate| gate_cols(gate.accepted));
    assert_eq!(accepted, Some(B_REAL), "B's gate is its own");
    assert_eq!(o10_rows(&ctx, &a).await, vec![A_FINAL; 3], "A unchanged");
    assert_eq!(o10_rows(&ctx, &lijiang).await, LIJIANG.to_vec());
    assert_eq!(o10_rows(&ctx, &aatlis).await, vec![AATLIS]);
    assert!(o10_rows(&ctx, &empty).await.is_empty());
}

/// The rule's bounds: a match counts on the cells read for sure only, needs
/// a board of some size, and needs every sure cell equal.
#[test]
fn a_finished_board_matches_on_its_sure_cells_only() {
    let none = [false; 6];
    let facts = |read: [u32; 6], held: [bool; 6]| board_facts(read, held, none, None);
    let a = counters_from_cols(A_FINAL);
    assert!(shows_finished_board(&facts(A_FINAL, none), a, none));
    assert!(!shows_finished_board(&facts(B_REAL, none), a, none));
    let mut held_h = none;
    held_h[4] = true;
    let mut off_in_h = A_FINAL;
    off_in_h[4] = 0;
    assert!(
        shows_finished_board(&facts(off_in_h, held_h), a, none),
        "a held cell has no say"
    );
    let mut off_in_d = A_FINAL;
    off_in_d[2] = 4;
    assert!(!shows_finished_board(&facts(off_in_d, none), a, none));
    let small = [0, 0, 1, 300, 0, 200];
    assert!(
        !shows_finished_board(&facts(small, none), counters_from_cols(small), none),
        "a near-empty board can be any game's"
    );
    let three_held = [true, true, true, false, false, false];
    assert!(!shows_finished_board(&facts(A_FINAL, three_held), a, none));
}

/// Shared [`BoardFacts`] for the tests in this file: `held` and `suspect`
/// per column, and the map this Tab read on its top bar (`None`: none).
fn board_facts(
    read: [u32; 6],
    held: [bool; 6],
    suspect: [bool; 6],
    map: Option<&'static str>,
) -> BoardFacts<'static> {
    BoardFacts {
        counters: counters_from_cols(read),
        suspect,
        trusted_cells: true,
        held,
        suspect_row: false,
        row_id: Some(0),
        hero: "Unknown",
        map_from_panel: map,
        panel_from_banner: false,
        parsed_map: "",
        frame_outcome: detect::MatchOutcome::Unknown,
    }
}

/// `held` or `suspect` mask with the given columns set.
fn cols_set(cols: &[usize]) -> [bool; 6] {
    std::array::from_fn(|col| cols.contains(&col))
}

/// The 1000 floor counts only the big cells compared. A small finished
/// board (2/1/1/1180/0/0) and a new Tab with the same five small cells but
/// its damage held compare 0 big: not a match, so the new game keeps its
/// first board. Read for sure, the damage counts.
#[test]
fn a_held_big_cell_does_not_count_toward_the_floor() {
    let finished = [2, 1, 1, 1180, 0, 0];
    let board = counters_from_cols(finished);
    let none = [false; 6];
    let mut read = finished;
    read[3] = 0;
    let held_dmg = board_facts(read, cols_set(&[3]), none, None);
    assert!(!shows_finished_board(&held_dmg, board, none));
    let sure = board_facts(finished, none, none, None);
    assert!(shows_finished_board(&sure, board, none));
    // A_FINAL with healing held still compares 2007 + 1502.
    let mut off_in_h = A_FINAL;
    off_in_h[4] = 0;
    let held_h = board_facts(off_in_h, cols_set(&[4]), none, None);
    assert!(shows_finished_board(
        &held_h,
        counters_from_cols(A_FINAL),
        none
    ));
}

/// The floor's edge: 999 compared is too small, 1000 is enough, over any
/// mix of damage, healing and mitigation. An unsure big cell on the stored
/// row that differs from the read drops out of the sum too.
#[test]
fn the_floor_is_1000_over_the_compared_big_cells() {
    let none = [false; 6];
    let fits = |board: [u32; 6], unsure: [bool; 6]| {
        let facts = board_facts(board, none, none, None);
        shows_finished_board(&facts, counters_from_cols(board), unsure)
    };
    assert!(!fits([5, 2, 1, 999, 0, 0], none));
    assert!(fits([5, 2, 1, 1000, 0, 0], none));
    assert!(!fits([5, 2, 1, 500, 300, 199], none));
    assert!(fits([5, 2, 1, 500, 300, 200], none));
    // Only the damage's 600 is summed; the skipped healing's 400 would
    // bring the sum to 1000.
    let read = board_facts([5, 2, 1, 600, 500, 0], none, none, None);
    let stored = counters_from_cols([5, 2, 1, 600, 400, 0]);
    assert!(!shows_finished_board(&read, stored, cols_set(&[4])));
}

/// An unsure cell of the finished row that equals this Tab's sure read is
/// compared and counts toward the floor: X ends 20/5/6/9000/0/0 with its
/// damage unsure at 9000, and the same board read all sure is X's. Only
/// the damage reaches 1000, so skipping it would leave nothing to match.
#[test]
fn an_unsure_finished_cell_equal_to_the_read_counts_toward_the_floor() {
    let none = [false; 6];
    let x = [20, 5, 6, 9000, 0, 0];
    let shown = board_facts(x, none, none, None);
    let board = counters_from_cols(x);
    assert!(shows_finished_board(&shown, board, cols_set(&[3])));
    assert!(shows_finished_board(&shown, board, [true; 6]));
    let equal_h = board_facts([5, 2, 1, 600, 400, 0], none, none, None);
    let stored = counters_from_cols([5, 2, 1, 600, 400, 0]);
    assert!(shows_finished_board(&equal_h, stored, cols_set(&[4])));
}

/// The suspect half of "read for sure": a suspect cell has no say, like a
/// held one, and with three suspect cells too few are left to compare.
#[test]
fn a_suspect_cell_has_no_say_in_the_match() {
    let a = counters_from_cols(A_FINAL);
    let none = [false; 6];
    let mut off_in_h = A_FINAL;
    off_in_h[4] = 0;
    let suspect_h = board_facts(off_in_h, none, cols_set(&[4]), None);
    assert!(shows_finished_board(&suspect_h, a, none));
    let read_h = board_facts(off_in_h, none, none, None);
    assert!(
        !shows_finished_board(&read_h, a, none),
        "a sure cell differs"
    );
    let three = board_facts(A_FINAL, none, cols_set(&[0, 1, 2]), None);
    assert!(!shows_finished_board(&three, a, none));
    let mixed = board_facts(A_FINAL, cols_set(&[0]), cols_set(&[1, 2]), None);
    assert!(!shows_finished_board(&mixed, a, none));
}

/// A column the finished game's last row stored as unsure may hold an
/// older value than the board showed. Shown again and read sure with
/// another value, that column is not compared, so the board still matches.
#[test]
fn a_column_the_finished_row_stored_unsure_is_not_compared() {
    let none = [false; 6];
    let mut stored = A_FINAL;
    stored[4] = 480;
    let shown = board_facts(A_FINAL, none, none, None);
    let board = counters_from_cols(stored);
    assert!(shows_finished_board(&shown, board, cols_set(&[4])));
    assert!(!shows_finished_board(&shown, board, none));
}

/// The lower side of the two-of-three vote. Exactly one of elims, deaths
/// and damage read for sure below the carried board leaves it fitting (one
/// misread column does not drop a real first board); two below do not.
#[test]
fn one_sure_column_below_a_carried_board_still_fits() {
    let none = [false; 6];
    let carried = counters_from_cols([3, 0, 2, 900, 300, 400]);
    for read in [
        [2, 0, 2, 1000, 300, 400],
        [3, 0, 1, 1000, 300, 400],
        [3, 0, 2, 800, 300, 400],
    ] {
        let facts = board_facts(read, none, none, None);
        assert!(carried_board_fits(carried, &facts), "{read:?}");
    }
    let two_below = board_facts([2, 0, 1, 1000, 300, 400], none, none, None);
    assert!(!carried_board_fits(carried, &two_below));
}

/// A known limit kept on purpose (a stronger rule is 0.7 work): A's final
/// board carried, and B's first Tab reads 0/0/2/(damage held)/307/600.
/// Only deaths is sure and below, so nothing votes the board out: it fits,
/// as before #809, and can seed B's held damage with A's 2007.
#[test]
fn a_carried_board_with_one_sure_vote_column_below_fits_known_limit() {
    let none = [false; 6];
    let b = board_facts([0, 0, 2, 0, 307, 600], cols_set(&[3]), none, None);
    assert!(carried_board_fits(counters_from_cols(A_FINAL), &b));
    let all_sure = board_facts(B_REAL, none, none, None);
    assert!(!carried_board_fits(counters_from_cols(A_FINAL), &all_sure));
}

/// Stores a game `id` on `map` whose one row is `board`, its last capture
/// `mins_ago` minutes ago.
async fn stored_game(
    store: &storage::LocalStore,
    id: &str,
    map: &str,
    board: [u32; 6],
    mins_ago: i64,
) {
    stored_game_unsure(store, id, map, board, mins_ago, &[]).await;
}

/// [`stored_game`] with `unsure` as the row's `suspect_fields`.
async fn stored_game_unsure(
    store: &storage::LocalStore,
    id: &str,
    map: &str,
    board: [u32; 6],
    mins_ago: i64,
    unsure: &[&str],
) {
    let at = SurrealDatetime::from(Utc::now() - chrono::Duration::minutes(mins_ago));
    let session = storage::MatchSession {
        session_id: id.into(),
        hero: "Ana".into(),
        map_name: map.into(),
        role: "Support".into(),
        started_at: at,
        last_capture_at: at,
        capture_count: 1,
        final_outcome: "defeat".into(),
    };
    store.create_session(&session).await.unwrap();
    let mut row = test_match(id, "defeat");
    row.map_name = map.into();
    [
        row.elims,
        row.assists,
        row.deaths,
        row.damage,
        row.healing,
        row.mitigation,
    ] = board;
    row.played_at = at;
    row.suspect_fields = unsure.iter().map(|f| f.to_string()).collect();
    store.insert_match(row).await.unwrap();
}

/// The request of a Tab that opened `session_id`: no board of its own yet.
fn first_board_req(session_id: &str) -> CaptureRequest {
    let now = test_now();
    let opened = tab_opened_game(session_id.into(), None, None, now);
    build_capture_request(&opened, true, now)
}

/// The window is the ten most recent other games, newest capture first.
/// The game the Tab is for is never compared and takes no place in the
/// ten; an eleventh other game is not compared.
#[tokio::test]
async fn a_first_board_is_compared_with_the_last_ten_other_games_only() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let none = [false; 6];
    let facts = board_facts(A_FINAL, none, none, Some("Runasapi"));
    let req = first_board_req("b");
    stored_game(&store, "a", "Runasapi", A_FINAL, 100).await;
    // The Tab's own game, newest, on the same board.
    stored_game(&store, "b", "Runasapi", A_FINAL, 1).await;
    for i in 0..9u32 {
        let board = [i, 0, 0, 1500 + i, 0, 0];
        stored_game(
            &store,
            &format!("other{i}"),
            "Busan",
            board,
            10 + i64::from(i),
        )
        .await;
    }
    assert_eq!(
        finished_game_shown(&store, &req, &facts).await.as_deref(),
        Some("a"),
        "a is the tenth other game; the own game is skipped"
    );
    stored_game(&store, "other9", "Busan", [9, 0, 0, 1509, 0, 0], 50).await;
    assert_eq!(
        finished_game_shown(&store, &req, &facts).await,
        None,
        "a is the eleventh other game"
    );
}

/// A Tab with no map read compares nothing, even with a game stored on no
/// map (whose blank map an empty read would equal); a map read matches the
/// stored map in any case; another map does not match.
#[tokio::test]
async fn a_first_board_with_no_map_read_is_not_compared() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    stored_game(&store, "a", "Runasapi", A_FINAL, 5).await;
    stored_game(&store, "blank", "", A_FINAL, 1).await;
    let none = [false; 6];
    let req = first_board_req("b");
    for (map, want) in [
        (None, None),
        (Some("Busan"), None),
        (Some("RUNASAPI"), Some("a")),
    ] {
        let facts = board_facts(A_FINAL, none, none, map);
        let shown = finished_game_shown(&store, &req, &facts).await;
        assert_eq!(shown.as_deref(), want, "{map:?}");
    }
}

/// The floor through the store: a finished Busan row 2/1/1/1180/0/0 and
/// a first board 2/1/1/(damage held)/0/0 compare no big cell, so the board
/// is the new game's.
#[tokio::test]
async fn a_small_finished_board_does_not_take_a_first_board_with_its_damage_held() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    stored_game(&store, "a", "Busan", [2, 1, 1, 1180, 0, 0], 5).await;
    let req = first_board_req("b");
    let none = [false; 6];
    let held = board_facts([2, 1, 1, 0, 0, 0], cols_set(&[3]), none, Some("Busan"));
    assert_eq!(finished_game_shown(&store, &req, &held).await, None);
    let sure = board_facts([2, 1, 1, 1180, 0, 0], none, none, Some("Busan"));
    assert_eq!(
        finished_game_shown(&store, &req, &sure).await.as_deref(),
        Some("a")
    );
}

/// A member's stat edit counts as the finished row's value. A's last row
/// read damage 2907, unsure, where the board showed 2007; the member
/// corrects it to 2007, which also clears the unsure mark. A's board shown
/// again is compared at the member's 2007 and refused, as before the edit.
#[tokio::test]
async fn a_finished_row_is_compared_at_the_members_edit() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut misread = A_FINAL;
    misread[3] = 2907;
    stored_game_unsure(&store, "a", "Runasapi", misread, 5, &["dmg"]).await;
    let req = first_board_req("b");
    let none = [false; 6];
    let facts = board_facts(A_FINAL, none, none, Some("Runasapi"));
    let before = finished_game_shown(&store, &req, &facts).await;
    assert_eq!(before.as_deref(), Some("a"), "before the edit");
    let edit = storage::MatchEdit {
        damage: Some(2007),
        ..Default::default()
    };
    store.edit_match("a", &edit).await.unwrap();
    let rows = store.get_session_snapshots("a").await.unwrap();
    let last = rows.last().unwrap();
    assert_eq!(last.corrected_damage, Some(2007));
    assert!(last.suspect_fields.is_empty(), "{:?}", last.suspect_fields);
    let after = finished_game_shown(&store, &req, &facts).await;
    assert_eq!(after.as_deref(), Some("a"), "after the edit");
}

/// An older row's `rN.field` suspect name marks that column unsure too:
/// A's last row stored damage 2907, marked unsure as `r0.dmg`, and A's
/// board read sure with 2007 still matches on the other columns.
#[tokio::test]
async fn a_legacy_row_suspect_name_marks_its_column_unsure() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    let mut misread = A_FINAL;
    misread[3] = 2907;
    stored_game_unsure(&store, "a", "Runasapi", misread, 5, &["r0.dmg"]).await;
    let none = [false; 6];
    let facts = board_facts(A_FINAL, none, none, Some("Runasapi"));
    let shown = finished_game_shown(&store, &first_board_req("b"), &facts).await;
    assert_eq!(shown.as_deref(), Some("a"));
}

/// A request that carries a gate for the store (a split carries its
/// parent's) is not compared: a finished board shown again on a split is
/// stored as the split's first row, as before #809. A gate a session
/// awaiting its first board still holds is not its own
/// (`gate_prev_for_store`), so that board is compared.
#[tokio::test]
async fn a_request_with_a_gate_for_the_store_is_not_compared() {
    let dir = tempfile::tempdir().unwrap();
    let store = storage::LocalStore::open(dir.path()).await.unwrap();
    stored_game(&store, "a", "Runasapi", A_FINAL, 5).await;
    let none = [false; 6];
    let facts = board_facts(A_FINAL, none, none, Some("Runasapi"));
    let mut req = first_board_req("b");
    req.prev_gate = Some((GateState::default(), Duration::from_secs(5)));
    req.awaiting_first_board = false;
    assert_eq!(finished_game_shown(&store, &req, &facts).await, None);
    req.awaiting_first_board = true;
    assert_eq!(
        finished_game_shown(&store, &req, &facts).await.as_deref(),
        Some("a")
    );
}

/// Game A (map vote) on Runasapi: three Tabs, the last `last`. Returns A.
async fn runasapi_a(
    ctx: &DaemonCtx,
    st: &mut SessionState,
    at: impl Fn(u64) -> Instant,
    last: O10Cells,
) -> String {
    let a = map_vote(ctx, st, at(0)).await;
    for (secs, cells) in [
        (216, all_sure([0, 0, 1, 500, 200, 400])),
        (306, all_sure([0, 0, 2, 1200, 480, 900])),
        (396, last),
    ] {
        tab_on(ctx, st, at(secs), cells, None, "Runasapi").await;
    }
    a
}

/// The rule on a game a map vote opened. A's final board shown again on
/// C's first Tab is not stored and gives C no gate; C's own board is its
/// first row.
#[tokio::test]
async fn a_finished_board_shown_again_is_refused_on_a_poll_opened_game() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let t0 = test_now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut st = session(None, t0);
    let a = runasapi_a(&ctx, &mut st, at, all_sure(A_FINAL)).await;
    let c = map_vote(&ctx, &mut st, at(500)).await;
    assert_ne!(a, c);
    tab_on(&ctx, &mut st, at(600), all_sure(A_FINAL), None, "Runasapi").await;
    assert_eq!(game_b_id(&st), c, "the Tab stays on the poll-opened game");
    assert!(o10_rows(&ctx, &c).await.is_empty(), "A's board not on C");
    assert!(st.active_game.as_ref().unwrap().gate.is_none());
    let wb = Some("Wrecking Ball");
    tab_on(&ctx, &mut st, at(640), all_sure(B_REAL), wb, "Runasapi").await;
    assert_eq!(game_b_id(&st), c);
    assert_eq!(o10_rows(&ctx, &c).await, vec![B_REAL]);
    let a_rows = o10_rows(&ctx, &a).await;
    assert_eq!(a_rows.last(), Some(&A_FINAL), "A unchanged: {a_rows:?}");
}

/// An unsure stored column in the daemon: A's last Tab holds its healing,
/// so A's last row stores another healing, marked unsure. A's board shown
/// again, read sure (healing 772), still matches on the other columns and
/// is refused.
#[tokio::test]
async fn a_finished_board_with_an_unsure_stored_column_is_still_refused() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let t0 = test_now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut st = session(None, t0);
    let mut last = all_sure(A_FINAL);
    last[4] = Some((700, true));
    let a = runasapi_a(&ctx, &mut st, at, last).await;
    let a_rows = ctx.store.get_session_snapshots(&a).await.unwrap();
    let a_last = a_rows.last().unwrap();
    assert!(
        a_last.suspect_fields.iter().any(|f| f == "h"),
        "A's last healing is unsure: {:?}",
        a_last.suspect_fields
    );
    assert_ne!(
        a_last.healing, A_FINAL[4],
        "A's last row holds another healing"
    );
    let c = map_vote(&ctx, &mut st, at(500)).await;
    tab_on(&ctx, &mut st, at(600), all_sure(A_FINAL), None, "Runasapi").await;
    assert!(o10_rows(&ctx, &c).await.is_empty(), "A's board not on C");
    let wb = Some("Wrecking Ball");
    tab_on(&ctx, &mut st, at(640), all_sure(B_REAL), wb, "Runasapi").await;
    assert_eq!(game_b_id(&st), c);
    assert_eq!(o10_rows(&ctx, &c).await, vec![B_REAL]);
}

/// An unsure stored cell equal to the board is compared, through the
/// daemon. Game X on Busan ends 20/5/6/9000/0/0 with its last Tab's damage
/// unsure at 9000, so X's last row stores 9000 with `dmg` unsure. Y's
/// first Tab on Busan shows that board, all sure: only the damage reaches
/// the 1000 floor, and it is X's, so the board is refused.
#[tokio::test]
async fn a_finished_board_with_an_equal_unsure_stored_cell_is_refused() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let t0 = test_now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut st = session(None, t0);
    let x_final = [20, 5, 6, 9000, 0, 0];
    let x = map_vote(&ctx, &mut st, at(0)).await;
    let mut last = all_sure(x_final);
    last[3] = Some((9000, true));
    for (secs, cells) in [
        (216, all_sure([10, 2, 3, 4000, 0, 0])),
        (306, all_sure([18, 4, 5, 9000, 0, 0])),
        (396, last),
    ] {
        tab_on(&ctx, &mut st, at(secs), cells, None, "Busan").await;
    }
    let x_rows = ctx.store.get_session_snapshots(&x).await.unwrap();
    let x_last = x_rows.last().unwrap();
    assert_eq!(x_last.damage, 9000, "X's last damage");
    assert!(
        x_last.suspect_fields.iter().any(|f| f == "dmg"),
        "X's last damage is unsure: {:?}",
        x_last.suspect_fields
    );
    let y = map_vote(&ctx, &mut st, at(500)).await;
    assert_ne!(x, y);
    tab_on(&ctx, &mut st, at(600), all_sure(x_final), None, "Busan").await;
    assert_eq!(game_b_id(&st), y, "the Tab stays on Y");
    assert!(o10_rows(&ctx, &y).await.is_empty(), "X's board not on Y");
    assert!(st.active_game.as_ref().unwrap().gate.is_none());
    let wb = Some("Wrecking Ball");
    tab_on(&ctx, &mut st, at(640), all_sure(B_REAL), wb, "Busan").await;
    assert_eq!(game_b_id(&st), y);
    assert_eq!(o10_rows(&ctx, &y).await, vec![B_REAL]);
}

/// The rule on the probe path. Aatlis has a result and a board under two
/// minutes old when a Tab past the grace window shows A's final board on
/// Runasapi. The probe calls it a new game, the rule refuses the board,
/// the fresh session opens with no row and no gate, and B's own board is
/// its first row.
#[tokio::test]
async fn a_finished_board_shown_again_is_refused_on_the_probe_path() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let t0 = test_now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut st = session(None, t0);
    let a = runasapi_a(&ctx, &mut st, at, all_sure(A_FINAL)).await;
    let aatlis = map_vote(&ctx, &mut st, at(500)).await;
    let wb = Some("Wrecking Ball");
    tab_on(&ctx, &mut st, at(600), all_sure(AATLIS), wb, "Aatlis").await;
    let g = st.active_game.as_mut().unwrap();
    g.record_outcome_at(detect::MatchOutcome::Victory, at(610));
    let press = at(690);
    assert!(finished_game_tab_probe(st.active_game.as_ref(), press));
    let verdict = tab_on(&ctx, &mut st, press, all_sure(A_FINAL), None, "Runasapi").await;
    assert_eq!(verdict, Some(boundary::FinishedGameTab::NewGame));
    let b = game_b_id(&st);
    assert!(![&a, &aatlis].contains(&&b), "a fresh session opens");
    assert!(o10_rows(&ctx, &b).await.is_empty(), "A's board not on B");
    assert!(st.active_game.as_ref().unwrap().gate.is_none());
    tab_on(&ctx, &mut st, at(730), all_sure(B_REAL), wb, "Runasapi").await;
    assert_eq!(game_b_id(&st), b);
    assert_eq!(o10_rows(&ctx, &b).await, vec![B_REAL]);
    assert_eq!(o10_rows(&ctx, &aatlis).await, vec![AATLIS]);
}

/// Game A ends on [`A_FINAL`] on Runasapi; a map vote opens C, whose first
/// Tab shows exactly [`A_FINAL`], all sure, on `map`. Returns C's rows and
/// its gate.
async fn a_final_on_the_next_games_first_tab(map: &str) -> (Vec<[u32; 6]>, Option<[u32; 6]>) {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let t0 = test_now();
    let at = |secs: u64| t0 + Duration::from_secs(secs);
    let mut st = session(None, t0);
    let a = runasapi_a(&ctx, &mut st, at, all_sure(A_FINAL)).await;
    let c = map_vote(&ctx, &mut st, at(500)).await;
    assert_ne!(a, c);
    tab_on(&ctx, &mut st, at(600), all_sure(A_FINAL), None, map).await;
    assert_eq!(game_b_id(&st), c, "the Tab stays on C ({map})");
    let a_rows = o10_rows(&ctx, &a).await;
    assert_eq!(a_rows.last(), Some(&A_FINAL), "A unchanged: {a_rows:?}");
    let gate = st.active_game.as_ref().unwrap().gate;
    (
        o10_rows(&ctx, &c).await,
        gate.map(|gate| gate_cols(gate.accepted)),
    )
}

/// #835: the finished-board rule matches on the map too, through the
/// daemon. The same first Tab, A's final board all sure, is refused on
/// Runasapi (A's map) and stored as C's first row, with C's gate at it, on
/// Busan. The map is the only difference.
#[tokio::test]
async fn a_finished_board_read_on_another_map_is_the_next_games_first_row() {
    let (rows, gate) = a_final_on_the_next_games_first_tab("Runasapi").await;
    assert!(rows.is_empty(), "A's board on A's map not on C: {rows:?}");
    assert_eq!(gate, None, "no gate from A's board on A's map");
    let (rows, gate) = a_final_on_the_next_games_first_tab("Busan").await;
    assert_eq!(rows, vec![A_FINAL], "the board on Busan is C's own");
    assert_eq!(gate, Some(A_FINAL), "C's gate is its first row");
}
