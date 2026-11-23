//! Daemon tests: a new game never stores another game's final stats
//! (#809). Runasapi, 17 Nov: game A (13c35793) ends 0/0/3/2007/772/1502;
//! Lijiang and Aatlis follow; game B (13c350d8) opens on a Tab 54 minutes
//! after A's last Tab, and that Tab's board was A's final board again, so
//! B stored it and its gate held B's own 0/0/2/648/307/600 at A's values.
//! A first board equal to a recent game's last stored board is not stored
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
async fn tab_on(
    ctx: &DaemonCtx,
    st: &mut SessionState,
    press: Instant,
    cells: O10Cells,
    hero: Option<&str>,
    map: &str,
) {
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
    r9_tab(st, ctx, press, Ok(capture), &synced).await;
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
    let facts = |read: [u32; 6], held: [bool; 6]| BoardFacts {
        counters: counters_from_cols(read),
        suspect: [false; 6],
        trusted_cells: true,
        held,
        suspect_row: false,
        row_id: Some(0),
        hero: "Unknown",
        map_from_panel: None,
        panel_from_banner: false,
        parsed_map: "",
        frame_outcome: detect::MatchOutcome::Unknown,
    };
    let a = counters_from_cols(A_FINAL);
    let none = [false; 6];
    assert!(shows_finished_board(&facts(A_FINAL, none), a));
    assert!(!shows_finished_board(&facts(B_REAL, none), a));
    let mut held_h = none;
    held_h[4] = true;
    let mut off_in_h = A_FINAL;
    off_in_h[4] = 0;
    assert!(
        shows_finished_board(&facts(off_in_h, held_h), a),
        "a held cell has no say"
    );
    let mut off_in_d = A_FINAL;
    off_in_d[2] = 4;
    assert!(!shows_finished_board(&facts(off_in_d, none), a));
    let small = [0, 0, 1, 300, 0, 200];
    assert!(
        !shows_finished_board(&facts(small, none), counters_from_cols(small)),
        "a near-empty board can be any game's"
    );
    let three_sure = [true, true, true, false, false, false];
    assert!(!shows_finished_board(&facts(A_FINAL, three_sure), a));
}
