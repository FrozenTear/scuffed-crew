//! Daemon tests: unread_columns. See `mod.rs`.

use super::*;

/// F266-1: a stat column the game has never read is a placeholder, and a
/// placeholder is never stored or uploaded as a number. The board is
/// read and gated but no row is written; the game's row appears with
/// the first board that leaves no column unread, with no rate cap or
/// jump hold against the placeholder.
#[tokio::test]
async fn new_reader_stores_no_row_while_a_stat_column_is_unread() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let first = [
        Some((27, true)),
        sure(5),
        sure(3),
        sure(3500),
        Some((351, true)),
        sure(450),
    ];
    // elims still not read on the second Tab; healing is
    let second = [None, sure(5), sure(3), sure(3520), sure(3400), sure(450)];
    let (st, seen) = o10_run(&ctx, vec![(0, first), (20, second)]).await;
    for tab in &seen {
        assert!(tab.pending && !tab.recorded && !tab.split, "{tab:?}");
    }
    assert_eq!(ctx.store.match_count().await.unwrap(), 0, "no row");
    assert!(ctx.store.get_all_sessions().await.unwrap().is_empty());
    assert!(ctx.store.get_unsynced().await.unwrap().is_empty());
    let game = st.active_game.as_ref().unwrap();
    assert!(!game.session_created, "the session row is not made yet");
    let gate = game.gate.expect("the game goes on from the gate");
    assert_eq!(gate.unread_cols(), vec!["e"], "healing has been read");
    assert_eq!(gate.accepted.healing, 3400);
    // the persisted game carries the pending state across a restart
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(active_game_path(dir.path())).unwrap()).unwrap();
    assert_eq!(
        persisted["gate"]["unread"],
        serde_json::json!([true, false, false, false, false, false])
    );
    assert_eq!(persisted["session_created"], false);

    // one more Tab reads elims: the row appears, complete and unflagged,
    // and the healing held on it carries the value read on Tab 2
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let third = [
        sure(25),
        sure(5),
        sure(3),
        sure(3550),
        Some((99, true)),
        sure(450),
    ];
    let (st, seen) = o10_run(&ctx, vec![(0, first), (20, second), (20, third)]).await;
    assert_eq!(
        seen.iter().map(|tab| tab.recorded).collect::<Vec<_>>(),
        vec![false, false, true]
    );
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(o10_stats(&rows[0]), [25, 5, 3, 3550, 3400, 450]);
    assert_eq!(rows[0].suspect_fields, vec!["h".to_string()]);
    assert!(st.active_game.as_ref().unwrap().session_created);
    assert_eq!(ctx.store.get_all_sessions().await.unwrap().len(), 1);
    // what would be uploaded has the read numbers and no placeholder
    let mut row = rows[0].clone();
    row.outcome = "victory".into();
    let upload = sync::upload_request(&[row], &[]);
    assert_eq!(upload.matches.len(), 1);
    assert_eq!(
        (upload.matches[0].elims, upload.matches[0].healing),
        (25, 3400)
    );
}

/// alpha.8 (replaces F266-1 at game end): a game where a column never
/// had a sure read stores the reader's best guess when it closes, and
/// lists the column in `suspect_fields` so the app marks it unsure.
/// While the game runs no row is written (the Tabs stay pending), so a
/// later sure read is still taken as the column's first read.
#[tokio::test]
async fn new_reader_game_with_a_never_read_column_stores_its_best_guess() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let tab = |e: u32, dmg: u32, h: u32| {
        [
            sure(e),
            sure(5),
            sure(3),
            sure(dmg),
            Some((h, true)),
            sure(450),
        ]
    };
    let (mut st, seen) = o10_run(
        &ctx,
        vec![(0, tab(12, 3500, 7000)), (20, tab(13, 3700, 7300))],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.pending), "{seen:?}");
    assert_eq!(
        ctx.store.match_count().await.unwrap(),
        0,
        "nothing mid-game"
    );
    retire_active_game(&mut st, &ctx.store, dir.path(), None, "test: game over").await;
    let rows = ctx.store.get_all_matches().await.unwrap();
    assert_eq!(rows.len(), 1, "the best guess is stored at game end");
    let row = &rows[0];
    assert_eq!(row.session_id, "o10");
    // The newest Tab's guess, the sure columns as read.
    assert_eq!((row.elims, row.damage, row.healing), (13, 3700, 7300));
    assert_eq!(
        crate::reader_apply::upload_suspect_fields(&row.suspect_fields),
        vec!["h".to_string()]
    );
    assert!(
        !dir.path()
            .join("debug")
            .join("unrecorded_games.jsonl")
            .exists(),
        "a stored game is not an unrecorded one"
    );

    // ocr-v1 on the same Tabs has a number for every cell and stores both
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), false).await;
    let (_, seen) = o10_run(
        &ctx,
        vec![(0, tab(12, 3500, 7000)), (20, tab(13, 3700, 7300))],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded && !tab.pending));
    assert_eq!(ctx.store.match_count().await.unwrap(), 2);
}

/// R2-1 of the #266 review, and what S1 decides for it. A game is left
/// pending under the new reader (healing never read), the member turns
/// classic OCR back on with `reader = "ocr-v1"` in the file, restarts,
/// and the game is resumed. `active_game.json` has no reader tag, so the
/// ocr-v1 run finds a gate state only the new reader writes.
///
/// The decision: the saved state is honoured as it is. The unread column
/// is neither cleared on load nor ignored under ocr-v1, because either
/// would let the placeholder 0 be stored as a number.
///
/// * A Tab on which another column drops: the first read of healing is
///   not taken and no row is written. (A game that was ocr-v1 from its
///   start stores that Tab. This is the one way an ocr-v1 run holds a
///   board on gate state.)
/// * The first Tab with no drop: OCR's number is the column's first
///   read, the row is written, and nothing on it is flagged.
#[tokio::test]
async fn a_pending_game_resumed_under_ocr_v1_keeps_its_unread_column() {
    async fn ocr_v1_tab(
        ctx: &DaemonCtx,
        st: &mut SessionState,
        cells: [u32; 6],
        now: Instant,
    ) -> bool {
        let mut analysis = o8_analysis(None, Some("ana"));
        for row in &mut analysis.rows {
            for (cell, value) in row.stats.iter_mut().zip(cells) {
                cell.value = value.to_string();
            }
        }
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
            analysis,
            board: None,
        });
        let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
        let sid = req.session_id.clone();
        let report = handle_capture(ctx, req).await.unwrap();
        assert!(!report.split, "the Tab must stay on this game");
        let recorded = report.recorded;
        apply_capture_report(st, &ctx.store, &ctx.data_dir, &sid, Ok(report), now).await;
        recorded
    }

    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), true).await;
    let pending = [
        sure(12),
        sure(5),
        sure(3),
        sure(3500),
        Some((7000, true)),
        sure(450),
    ];
    let (mut st, seen) = o10_run(&ctx, vec![(0, pending)]).await;
    assert!(seen[0].pending, "{:?}", seen[0]);

    // What the restart finds on disk: the gate, and no word of which
    // reader wrote it.
    let persisted: serde_json::Value =
        serde_json::from_slice(&std::fs::read(active_game_path(dir.path())).unwrap()).unwrap();
    assert_eq!(persisted.get("reader"), None);
    let saved: capture_gate::GateState = serde_json::from_value(persisted["gate"].clone()).unwrap();
    assert_eq!(saved.unread_cols(), vec!["h"]);
    st.active_game.as_mut().unwrap().gate = Some(saved);

    // The restart: classic OCR on, `reader = "ocr-v1"`.
    ctx.use_new_reader = false;
    let now = test_now() + Duration::from_secs(20);

    // Damage reads below the accepted 3500 on this Tab.
    let recorded = ocr_v1_tab(&ctx, &mut st, [13, 5, 3, 3400, 7300, 450], now).await;
    assert!(!recorded, "healing's first read is not taken on a drop");
    assert_eq!(ctx.store.match_count().await.unwrap(), 0, "no row");
    let gate = st.active_game.as_ref().unwrap().gate.expect("the gate");
    assert_eq!(gate.unread_cols(), vec!["h"], "still unread");

    // No drop: the column is read, and the row is OCR's.
    let now = now + Duration::from_secs(20);
    let recorded = ocr_v1_tab(&ctx, &mut st, [14, 5, 3, 3700, 7400, 450], now).await;
    assert!(recorded);
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(rows.len(), 1);
    assert_eq!(o10_stats(&rows[0]), [14, 5, 3, 3700, 7400, 450]);
    assert!(
        rows[0].suspect_fields.is_empty(),
        "{:?}",
        rows[0].suspect_fields
    );
    let gate = st.active_game.as_ref().unwrap().gate.expect("the gate");
    assert!(gate.unread_cols().is_empty());
}

/// F266-2, the reviewer's trace through `handle_capture`: game 1 has one
/// Tab with healing held, its end is missed, and game 2's first Tab
/// comes 60 s later with every cell sure. That board used to be appended
/// to game 1 and its healing stored there unflagged. Now the reset plan
/// sees the sure columns fall and holds the board off game 1; the next
/// fresh board splits, and game 2 gets both boards.
#[tokio::test]
async fn new_reader_next_games_first_board_does_not_fill_an_unread_column() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let game_one = [
        sure(30),
        sure(12),
        sure(9),
        sure(14000),
        Some((5000, true)),
        sure(900),
    ];
    let (st, seen) = o10_run(
        &ctx,
        vec![
            (0, game_one),
            (60, all_sure([1, 0, 0, 300, 100, 0])),
            (30, all_sure([3, 1, 0, 900, 400, 0])),
        ],
    )
    .await;
    assert!(seen[0].pending, "{:?}", seen[0]);
    assert!(
        seen[1].deferred && !seen[1].recorded && !seen[1].pending,
        "game 2's first board is held off game 1: {:?}",
        seen[1]
    );
    assert!(seen[2].split && seen[2].recorded, "{:?}", seen[2]);
    // game 1 never had a sure healing read: at its close it stores its
    // own best guess (5000, marked unsure), and above all not game 2's
    // 100 (alpha.8)
    assert_eq!(
        o10_rows(&ctx, "o10").await,
        vec![[30, 12, 9, 14000, 5000, 900]]
    );
    let new_id = st.active_game.as_ref().unwrap().session_id.clone();
    assert_ne!(new_id, "o10");
    assert_eq!(seen[2].session_id, new_id);
    assert_eq!(
        o10_rows(&ctx, &new_id).await,
        vec![[1, 0, 0, 300, 100, 0], [3, 1, 0, 900, 400, 0]]
    );
    assert_eq!(ctx.store.match_count().await.unwrap(), 3);

    // 30 s after game 1's Tab is too soon for the reset rule. The gate
    // is then the guard: the read columns drop, so the first read of
    // healing is not taken and game 1 still has no row.
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run(
        &ctx,
        vec![(0, game_one), (30, all_sure([1, 0, 0, 300, 100, 0]))],
    )
    .await;
    assert!(seen[1].pending && !seen[1].deferred, "{:?}", seen[1]);
    assert_eq!(ctx.store.match_count().await.unwrap(), 0);
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert_eq!(gate.unread_cols(), vec!["h"]);
    assert_eq!(gate.accepted.healing, 0);
    assert_eq!(gate.accepted.elims, 30);
    // At close the game stores game 1's own guess. The second Tab's
    // healing read was not taken (the other columns dropped), so its
    // 100 is not the guess (alpha.8).
    let mut st = st;
    retire_active_game(&mut st, &ctx.store, dir.path(), None, "test: game over").await;
    assert_eq!(
        o10_rows(&ctx, "o10").await,
        vec![[30, 12, 9, 14000, 5000, 900]]
    );
}

/// A split under `reader = "new"`, with the carried board (F266-1's
/// folded case, and two of the reviewer's mutants). Game 1 is stored;
/// two fresh-match boards follow and the second has healing held. The
/// held cell abstains from the reset, so the split still happens. The
/// new game's rows come from the new game only: the stored-row gate
/// starts over at the split, and the held healing takes the carried
/// board's 120, not a 0 under it and not game 1's 5000.
#[tokio::test]
async fn new_reader_split_with_a_held_cell_takes_the_carried_board() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let held_healing = [
        sure(3),
        sure(1),
        sure(0),
        sure(900),
        Some((9, true)),
        sure(0),
    ];
    let (st, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([30, 12, 9, 14000, 5000, 900])),
            (60, all_sure([1, 0, 0, 300, 120, 0])),
            (30, held_healing),
        ],
    )
    .await;
    assert!(seen[0].recorded, "{:?}", seen[0]);
    assert!(seen[1].deferred, "{:?}", seen[1]);
    assert!(seen[2].split && seen[2].recorded, "{:?}", seen[2]);
    assert_eq!(
        o10_rows(&ctx, "o10").await,
        vec![[30, 12, 9, 14000, 5000, 900]],
        "game 1 keeps its one row"
    );
    let new_id = st.active_game.as_ref().unwrap().session_id.clone();
    let rows = ctx.store.get_session_snapshots(&new_id).await.unwrap();
    assert_eq!(
        rows.iter().map(o10_stats).collect::<Vec<_>>(),
        vec![[1, 0, 0, 300, 120, 0], [3, 1, 0, 900, 120, 0]]
    );
    assert_eq!(rows[1].suspect_fields, vec!["h".to_string()]);
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(gate.unread_cols().is_empty());
    assert_eq!(gate.accepted.healing, 120);

    // a fresh-match board with a held cell is the first reset signal
    // but is not carried: it has no number for that cell. The split on
    // the next board opens the game from that board alone.
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([30, 12, 9, 14000, 5000, 900])),
            (60, held_healing),
            (30, all_sure([4, 1, 0, 1200, 300, 0])),
        ],
    )
    .await;
    assert!(seen[1].deferred && seen[2].split, "{seen:?}");
    let new_id = st.active_game.as_ref().unwrap().session_id.clone();
    assert_eq!(o10_rows(&ctx, &new_id).await, vec![[4, 1, 0, 1200, 300, 0]]);
    assert_eq!(ctx.store.match_count().await.unwrap(), 2);
}

/// A split whose first board has a held cell and no carried board: the
/// old game is closed, the new one is open and pending, and no row of
/// the new game exists until its column is read.
#[tokio::test]
async fn new_reader_split_onto_a_pending_game_stores_no_row_yet() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let held_healing = |e: u32, dmg: u32| {
        [
            sure(e),
            sure(1),
            sure(0),
            sure(dmg),
            Some((9, true)),
            sure(0),
        ]
    };
    let (st, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([30, 12, 9, 14000, 5000, 900])),
            (60, held_healing(1, 300)),
            (30, held_healing(3, 900)),
            (20, all_sure([4, 1, 0, 1300, 250, 0])),
        ],
    )
    .await;
    assert!(seen[1].deferred, "{:?}", seen[1]);
    assert!(
        seen[2].split && seen[2].pending && !seen[2].recorded,
        "{:?}",
        seen[2]
    );
    assert!(seen[3].recorded && !seen[3].split, "{:?}", seen[3]);
    let new_id = st.active_game.as_ref().unwrap().session_id.clone();
    assert_eq!(seen[2].session_id, new_id);
    assert_eq!(seen[3].session_id, new_id);
    assert_eq!(
        o10_rows(&ctx, "o10").await,
        vec![[30, 12, 9, 14000, 5000, 900]]
    );
    assert_eq!(o10_rows(&ctx, &new_id).await, vec![[4, 1, 0, 1300, 250, 0]]);
    assert!(st.active_game.as_ref().unwrap().session_created);
    // the pending split made no session row; the first stored row did
    let sessions = ctx.store.get_all_sessions().await.unwrap();
    assert!(
        sessions.iter().any(|s| s.session_id == new_id),
        "the new game's session row exists"
    );
    assert_eq!(sessions.len(), 2);

    // a fresh-match board that is not carried leaves no half of a held
    // board behind: no counters, and no hero or time for them
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([30, 12, 9, 14000, 5000, 900])),
            (60, held_healing(1, 300)),
        ],
    )
    .await;
    assert!(seen[1].deferred, "{:?}", seen[1]);
    let game = st.active_game.as_ref().unwrap();
    assert_eq!(game.reset_streak, 1, "it is the first reset signal");
    assert!(game.deferred.is_none());
    assert!(game.deferred_hero.is_none() && game.deferred_at.is_none());
}

/// O9 and O10 on one board: O9 says which row is the member's and how
/// sure it is, O10 reads that row's cells. A suspect own row may be
/// another player's, so none of its numbers are stored and it does not
/// raise the totals. Every field off the row is flagged. The boundary
/// plan does not take it as a reset row. A held cell on it is discarded
/// with the rest of the row.
#[tokio::test]
async fn new_reader_suspect_own_row_with_held_cells_is_low_trust_and_flagged() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let sure_row = combine(Some(0), o9_lit(0, 1.5), true);
    let suspect_row = combine(Some(0), None, true);
    assert!(suspect_row.suspect() && !sure_row.suspect());
    let (sure_row, suspect_row) = (Some(sure_row), Some(suspect_row));
    let held_healing = |e: u32, dmg: u32| {
        [
            sure(e),
            sure(6),
            sure(3),
            sure(dmg),
            Some((77, true)),
            sure(450),
        ]
    };

    // a game with a row; then a suspect row with healing held
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run_rows(
        &ctx,
        vec![
            (0, all_sure([12, 5, 3, 3500, 700, 300]), sure_row),
            (20, held_healing(13, 3900), suspect_row),
        ],
    )
    .await;
    assert!(
        seen[0].recorded && seen[0].refresh_baseline,
        "{:?}",
        seen[0]
    );
    assert!(
        seen[1].recorded && seen[1].refresh_baseline,
        "a suspect row keeps the last row and does not raise the baseline: {:?}",
        seen[1]
    );
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    // none of the suspect row's numbers are stored: the row stays the
    // last sure board, and every field off the suspect row is flagged
    assert_eq!(o10_stats(&rows[1]), [12, 5, 3, 3500, 700, 300]);
    assert_eq!(
        rows[1].suspect_fields,
        reader_apply::OWN_ROW_FIELDS.to_vec()
    );
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(!gate.low_trust, "a suspect row does not latch");
    assert_eq!(gate.accepted.deaths, 3);
    assert_eq!(gate.accepted.damage, 3500);
    assert!(!gate.unconfirmed.iter().any(|&col| col));

    // a game whose first board is a suspect row with healing held: no
    // row. Nothing of the game is stored and nothing in it was read off
    // a trusted row, so the member's real row, on the first trusted
    // board, starts the game over: its numbers are the row, unflagged.
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let real = all_sure([12, 6, 3, 3500, 700, 450]);
    let (st, seen) = o10_run_rows(
        &ctx,
        vec![
            (0, held_healing(20, 8000), suspect_row),
            (20, real, sure_row),
            (20, real, sure_row),
        ],
    )
    .await;
    assert!(seen[0].pending, "{:?}", seen[0]);
    assert!(seen[1].recorded && seen[2].recorded, "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(rows.len(), 2);
    for row in &rows {
        assert_eq!(o10_stats(row), [12, 6, 3, 3500, 700, 450]);
        assert!(row.suspect_fields.is_empty(), "{:?}", row.suspect_fields);
    }
    assert!(!st.active_game.as_ref().unwrap().gate.unwrap().low_trust);
}

/// R262-E: the name fallback's rows index the digit reader's board, so
/// a set that is not the whole board names no row. Today's names read
/// always returns the whole board (R2-6); the rule is a guard.
#[test]
fn the_name_fallback_matches_only_on_a_full_set_of_rows() {
    let named = |name: &str| ocr::RowOcrResult {
        name: Some(ocr::CellOcrResult {
            value: name.into(),
            confidence: 90,
            suspect: false,
        }),
        name_alternates: Vec::new(),
        stats: Vec::new(),
        mean_confidence: 0,
    };
    let mut rows: Vec<ocr::RowOcrResult> = (0..10).map(|_| named("OTHER")).collect();
    rows[3] = named("MADEUP");
    assert_eq!(
        name_on_a_full_board(&rows, "MadeUp", 5).map(|m| m.row),
        Some(3)
    );
    // a set with a row missing is not matched at all
    rows.remove(0);
    assert_eq!(name_on_a_full_board(&rows, "MadeUp", 5), None);
    // and a 5v5 set is not a 6v6 board
    let rows: Vec<ocr::RowOcrResult> = (0..10).map(|_| named("MADEUP")).collect();
    assert_eq!(name_on_a_full_board(&rows, "MadeUp", 6), None);
}

/// Under the new reader every cell whose stored value is not this
/// capture's read is flagged, a rate-cap hold too (F266-3). ocr-v1
/// leaves a rate-cap hold unflagged, as before.
#[tokio::test]
async fn new_reader_flags_a_rate_cap_hold_and_ocr_v1_does_not() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (_, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([10, 4, 2, 3000, 800, 400])),
            // +40 elims in 20 s, the rest of the board moving
            (20, all_sure([50, 6, 3, 3900, 1500, 800])),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(o10_stats(&rows[1]), [10, 6, 3, 3900, 1500, 800]);
    assert_eq!(rows[1].suspect_fields, vec!["e".to_string()]);

    let mut parsed = test_match("s", "unknown");
    (parsed.elims, parsed.assists, parsed.deaths) = (10, 6, 3);
    let kept = capture_gate::Counters {
        elims: 10,
        assists: 6,
        deaths: 3,
        damage: parsed.damage,
        healing: parsed.healing,
        mitigation: parsed.mitigation,
    };
    let holds = [capture_gate::Hold {
        col: 0,
        kind: capture_gate::HoldKind::RateCap,
        raw: 50,
        held: 10,
    }];
    note_gate_suspects(&mut parsed, kept, &capture_gate::unsure_fields(&holds));
    assert!(parsed.suspect_fields.is_empty(), "ocr-v1's list");
    note_gate_suspects(&mut parsed, kept, &capture_gate::held_fields(&holds));
    assert_eq!(parsed.suspect_fields, vec!["e".to_string()]);
}

/// `analyze_frame` itself under `reader = "new"`, on a frame the
/// preflight takes for a Tab: the vision pass comes back with no OCR
/// stat cell, no name cell (no player name, or a blank one), no board
/// text, and the reader's board. Until now this rested on the helper
/// tests and on two source-text counts.
#[test]
fn analyze_frame_under_the_new_reader_reads_no_cell_and_no_board_text() {
    let dir = tempfile::tempdir().unwrap();
    for name in [None, Some("  ".to_string())] {
        let matcher = std::sync::Arc::new(detect::hero_portrait::PortraitMatcher::load(dir.path()));
        let flags = FrameFlags {
            allow_banner_recovery: false,
            session_map_known: false,
            shadow_on: false,
            use_new_reader: true,
        };
        let out = analyze_frame(
            o10_tab_frame(),
            matcher,
            name,
            detect::MatchOutcome::Unknown,
            flags,
            Vec::new(),
            &names::NameSwitch::off(),
        );
        let FrameAnalysisOutcome::Analyzed(analysis) = out else {
            panic!("the stand-in frame did not pass the Tab preflight");
        };
        let analysis = *analysis;
        assert!(analysis.own_row.is_some(), "the own row is O9's");
        assert_eq!(analysis.rows.len(), analysis.team_size * 2);
        // The stand-in frame is blank, so the reader cannot size its
        // board and, with classic OCR on, that Tab falls back to ocr-v1's
        // cell pass, which may read stat and name cells. The full-board
        // text is still never read.
        let board = analysis.board.as_ref().expect("the board is the reader's");
        let fallback = reader_apply::ocr_fallback_reason(board, ocr::gate::is_off());
        assert_eq!(fallback, Some("team size is unknown"));
        assert!(analysis.ocr_fallback, "a blank board falls back to ocr-v1");
        let text = analysis.ocr.expect("no OCR, so no OCR error");
        assert_eq!((text.raw_text.as_str(), text.confidence), ("", 0));
        assert!(analysis.board.is_some(), "the board is the reader's");
        assert_eq!(analysis.career_hero, None);
        assert_eq!(analysis.name_match, None);
    }
}

/// A held cell abstains from the reset and the row still re-anchors the
/// reset baseline (F266-2). Before, `row_counts` was false for any board
/// with a held cell: no baseline, so no reset for the whole game.
#[test]
fn a_board_with_a_held_cell_still_anchors_the_reset_baseline() {
    let (game, now) = request_with_text_fallback();
    let req = build_capture_request(&game, false, now);
    let clean = banner_facts("Busan", true);
    assert!(plan_from_board(&req, &clean).refresh_baseline);
    let mut held = banner_facts("Busan", true);
    held.held[4] = true;
    held.suspect[4] = true;
    let plan = plan_from_board(&req, &held);
    assert!(plan.refresh_baseline, "{plan:?}");
    assert_eq!(plan.baseline_row, Some(2));
    // a raw-text fallback row still does not
    let mut fallback = banner_facts("Busan", true);
    fallback.trusted_cells = false;
    assert!(!plan_from_board(&req, &fallback).refresh_baseline);

    // a game with boards read and no row yet can still gap-split: it
    // has a gate of its own, so "no session row" is not "no board yet"
    let mut pending = game;
    pending.session_created = false;
    pending.map_source = Some(boundary::MapSource::TopBar);
    pending.gate = Some(GateState {
        accepted: Counters {
            elims: 30,
            assists: 12,
            deaths: 9,
            damage: 14000,
            healing: 0,
            mitigation: 900,
        },
        last_raw: Counters {
            elims: 30,
            assists: 12,
            deaths: 9,
            damage: 14000,
            healing: 0,
            mitigation: 900,
        },
        unread: [false, false, false, false, true, false],
        ..GateState::default()
    });
    let later = now + Duration::from_secs(200);
    let req = build_capture_request(&pending, false, later);
    assert!(req.create_session);
    let mut next = banner_facts("Ilios", true);
    next.hero = "Reinhardt";
    next.counters = Counters {
        elims: 1,
        assists: 0,
        deaths: 0,
        damage: 300,
        healing: 100,
        mitigation: 0,
    };
    let plan = plan_from_board(&req, &next);
    assert!(plan.split, "{plan:?}");
    assert_eq!(
        plan.close_reason,
        Some(boundary::CloseReason::StatRegression)
    );
}

/// R3-1, claude-r3's experiment e4 through `handle_capture`: a game whose
/// every Tab has a suspect own row, its end missed, then the next game
/// on the same map and hero with sure rows 40 s apart. The suspect rows
/// store no numbers. They still anchor the baseline, so the first sure
/// board of game 2 is held off game 1 and the second splits.
#[tokio::test]
async fn new_reader_all_suspect_game_then_a_sure_game_splits() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let suspect = Some(combine(Some(0), None, true));
    let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
    let game_one = [
        [10, 4, 3, 5000, 1800, 300],
        [20, 8, 6, 9500, 3400, 600],
        [30, 12, 9, 14000, 5000, 900],
    ];
    let game_two = [
        [1, 0, 0, 300, 100, 0],
        [3, 1, 0, 900, 400, 0],
        [5, 2, 1, 1900, 700, 100],
        [7, 3, 1, 2800, 900, 150],
    ];
    let tabs = |second: Option<detect::own_row::OwnRow>| {
        let mut tabs = vec![
            (0, all_sure(game_one[0]), suspect),
            (60, all_sure(game_one[1]), suspect),
            (60, all_sure(game_one[2]), suspect),
            (200, all_sure(game_two[0]), second),
        ];
        for board in &game_two[1..] {
            tabs.push((40, all_sure(*board), second));
        }
        tabs
    };

    // e4: game 2 has sure rows. Game 1's rows are all suspect, so none
    // of those numbers are stored. They still anchor the baseline.
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run_rows(&ctx, tabs(sure_row)).await;
    for tab in &seen[..3] {
        assert!(!tab.recorded && tab.refresh_baseline, "{tab:?}");
    }
    assert!(seen[3].deferred && !seen[3].recorded, "{:?}", seen[3]);
    assert!(seen[4].split && seen[4].recorded, "{:?}", seen[4]);
    let one = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert!(one.is_empty(), "a suspect game stores no stats");
    let new_id = st.active_game.as_ref().unwrap().session_id.clone();
    assert_ne!(new_id, "o10");
    let two = ctx.store.get_session_snapshots(&new_id).await.unwrap();
    assert_eq!(
        two.iter().map(o10_stats).collect::<Vec<_>>(),
        game_two.to_vec(),
        "game 2 has its own session, with all four boards"
    );
    assert_eq!(o10_unflagged_drops(&two), Vec::<String>::new());

    // e1: game 2 has suspect rows too. A suspect row never commits the
    // split by itself, so two boards are held off game 1 and the third
    // splits. Nothing of game 2 lands in game 1.
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run_rows(&ctx, tabs(suspect)).await;
    assert!(seen[3].deferred && seen[4].deferred, "{seen:?}");
    assert!(seen[5].split && !seen[5].recorded, "{:?}", seen[5]);
    assert!(
        o10_rows(&ctx, "o10").await.is_empty(),
        "game 1 stores no suspect stats"
    );
    let new_id = st.active_game.as_ref().unwrap().session_id.clone();
    let two = ctx.store.get_session_snapshots(&new_id).await.unwrap();
    assert!(
        two.is_empty(),
        "a suspect fresh board is not stored: {two:?}"
    );
    assert_eq!(ctx.store.match_count().await.unwrap(), 0);
}

/// One wrong suspect row in the middle of a sure game does not split it.
/// The row looks like a fresh match on the member's own row index: it is
/// held off the game as a reset signal, nothing of it is stored, and the
/// next real board stays in the game.
#[tokio::test]
async fn new_reader_one_wrong_suspect_row_does_not_split_a_sure_game() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let suspect = Some(combine(Some(0), None, true));
    let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run_rows(
        &ctx,
        vec![
            (0, all_sure([12, 5, 3, 3500, 700, 300]), sure_row),
            (60, all_sure([14, 6, 3, 4300, 900, 400]), sure_row),
            (60, all_sure([1, 0, 1, 300, 0, 0]), suspect),
            (60, all_sure([15, 6, 4, 4700, 950, 450]), sure_row),
            (60, all_sure([16, 7, 4, 5100, 1000, 500]), sure_row),
        ],
    )
    .await;
    assert!(
        seen[2].deferred && !seen[2].recorded && !seen[2].split,
        "{:?}",
        seen[2]
    );
    assert!(seen.iter().all(|tab| !tab.split), "{seen:?}");
    assert!(seen[3].recorded && seen[4].recorded, "{seen:?}");
    let game = st.active_game.as_ref().unwrap();
    assert_eq!(game.session_id, "o10");
    assert_eq!(game.reset_streak, 0, "the real board ended the streak");
    assert!(game.deferred.is_none(), "a suspect board is not carried");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(
        rows.iter().map(o10_stats).collect::<Vec<_>>(),
        vec![
            [12, 5, 3, 3500, 700, 300],
            [14, 6, 3, 4300, 900, 400],
            [15, 6, 4, 4700, 950, 450],
            [16, 7, 4, 5100, 1000, 500],
        ]
    );
    assert_eq!(ctx.store.get_all_sessions().await.unwrap().len(), 1);
    assert!(rows.iter().all(|row| row.suspect_fields.is_empty()));
}

/// A stat never goes down unflagged in a stored row, whatever the trust
/// bits say (R3-1). A suspect row with higher numbers is not stored and
/// does not raise the baseline. The member's next clean row continues
/// from the last sure board, so it is not a drop and it is not unsure.
#[tokio::test]
async fn new_reader_flags_every_stat_that_goes_down_in_a_stored_row() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let suspect = Some(combine(Some(0), None, true));
    let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (_, seen) = o10_run_rows(
        &ctx,
        vec![
            (0, all_sure([12, 5, 3, 3500, 700, 300]), sure_row),
            (20, all_sure([20, 9, 5, 8000, 2000, 900]), suspect),
            (20, all_sure([13, 5, 3, 3700, 720, 300]), sure_row),
            (20, all_sure([13, 5, 3, 3700, 720, 300]), sure_row),
            (20, all_sure([14, 6, 3, 4000, 800, 350]), sure_row),
        ],
    )
    .await;
    assert!(
        seen.iter().all(|tab| tab.recorded && !tab.split),
        "{seen:?}"
    );
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    // the suspect row: previous numbers, every own-row field flagged
    assert_eq!(o10_stats(&rows[1]), [12, 5, 3, 3500, 700, 300]);
    assert_eq!(
        rows[1].suspect_fields,
        reader_apply::OWN_ROW_FIELDS.to_vec()
    );
    // the next clean reads continue from that board, unflagged
    assert_eq!(o10_stats(&rows[2]), [13, 5, 3, 3700, 720, 300]);
    assert!(
        rows[2].suspect_fields.is_empty(),
        "{:?}",
        rows[2].suspect_fields
    );
    assert_eq!(o10_stats(&rows[3]), [13, 5, 3, 3700, 720, 300]);
    assert!(
        rows[3].suspect_fields.is_empty(),
        "{:?}",
        rows[3].suspect_fields
    );
    assert_eq!(o10_stats(&rows[4]), [14, 6, 3, 4000, 800, 350]);
    assert!(
        rows[4].suspect_fields.is_empty(),
        "{:?}",
        rows[4].suspect_fields
    );
    assert_eq!(o10_unflagged_drops(&rows), Vec::<String>::new());
}

/// R3-2, claude-r3's experiment h1: a suspect, higher first board with
/// healing held, then four complete sure reads of the member's row with
/// moving numbers. That stored 0 rows from 5 Tabs. A game with no row
/// yet and only low-trust reads is now started over by the first
/// trusted board, so the four real boards are stored.
#[tokio::test]
async fn new_reader_first_trusted_board_restarts_a_game_only_suspect_rows_read() {
    use detect::own_row::combine;
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let suspect = Some(combine(Some(0), None, true));
    let sure_row = Some(combine(Some(0), o9_lit(0, 1.5), true));
    let first = [
        sure(20),
        sure(6),
        sure(3),
        sure(8000),
        Some((77, true)),
        sure(450),
    ];
    let real = [
        [12, 6, 3, 3500, 700, 450],
        [13, 6, 3, 3700, 720, 450],
        [14, 7, 4, 4100, 800, 500],
        [15, 7, 4, 4500, 900, 520],
    ];
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let mut tabs = vec![(0, first, suspect)];
    tabs.extend(real.iter().map(|board| (30, all_sure(*board), sure_row)));
    let (_, seen) = o10_run_rows(&ctx, tabs).await;
    assert!(seen[0].pending, "{:?}", seen[0]);
    assert!(
        seen[1..].iter().all(|tab| tab.recorded && !tab.split),
        "{seen:?}"
    );
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(
        rows.iter().map(o10_stats).collect::<Vec<_>>(),
        real.to_vec()
    );
    assert!(rows.iter().all(|row| row.suspect_fields.is_empty()));
}

/// R3-3: nothing that runs before the capture gate reads a held cell.
/// The boundary plan gives the same plan whatever number sits in a held
/// column: the game's last value (what `fill_held_cells` puts there), a
/// 0, or garbage. That is why `fill_held_cells` is a belt and not a rule.
#[test]
fn the_plan_does_not_read_a_held_cell() {
    let (mut game, now) = request_with_text_fallback();
    let accepted = Counters {
        elims: 30,
        assists: 12,
        deaths: 9,
        damage: 14000,
        healing: 5000,
        mitigation: 900,
    };
    let state = GateState {
        accepted,
        last_raw: accepted,
        ..GateState::default()
    };
    game.gate = Some(state);
    game.reset_baseline = Some(state);
    game.baseline_row = Some(2);
    game.baseline_at = Some(now);
    let req = build_capture_request(&game, false, now + Duration::from_secs(60));
    let arr = |c: Counters| {
        [
            c.elims,
            c.assists,
            c.deaths,
            c.damage,
            c.healing,
            c.mitigation,
        ]
    };
    // (the sure cells, the held columns)
    let boards: [(Counters, &[usize]); 4] = [
        // the game goes on, healing held
        (
            Counters {
                elims: 31,
                assists: 12,
                deaths: 9,
                damage: 14400,
                healing: 0,
                mitigation: 900,
            },
            &[4],
        ),
        // a fresh-match board, mitigation and assists held
        (
            Counters {
                elims: 1,
                assists: 0,
                deaths: 0,
                damage: 300,
                healing: 100,
                mitigation: 0,
            },
            &[1, 5],
        ),
        // a fresh-match board with elims held: deaths and damage decide
        (
            Counters {
                elims: 0,
                assists: 0,
                deaths: 0,
                damage: 300,
                healing: 100,
                mitigation: 0,
            },
            &[0],
        ),
        // damage held on a board that otherwise moved on
        (
            Counters {
                elims: 33,
                assists: 13,
                deaths: 10,
                damage: 0,
                healing: 5600,
                mitigation: 1000,
            },
            &[3],
        ),
    ];
    for (sure_cells, held_cols) in boards {
        let plan_with = |fill: u32| {
            let mut facts = banner_facts("Busan", true);
            let mut cells = arr(sure_cells);
            for &col in held_cols {
                cells[col] = fill;
                facts.held[col] = true;
                facts.suspect[col] = true;
            }
            facts.counters = Counters {
                elims: cells[0],
                assists: cells[1],
                deaths: cells[2],
                damage: cells[3],
                healing: cells[4],
                mitigation: cells[5],
            };
            plan_from_board(&req, &facts)
        };
        let filled = {
            let mut facts = banner_facts("Busan", true);
            let mut cells = arr(sure_cells);
            for &col in held_cols {
                cells[col] = arr(accepted)[col];
                facts.held[col] = true;
                facts.suspect[col] = true;
            }
            facts.counters = Counters {
                elims: cells[0],
                assists: cells[1],
                deaths: cells[2],
                damage: cells[3],
                healing: cells[4],
                mitigation: cells[5],
            };
            plan_from_board(&req, &facts)
        };
        for fill in [0, 1, 77, 99_999] {
            assert_eq!(
                plan_with(fill),
                filled,
                "held {held_cols:?} filled with {fill}"
            );
        }
    }
}

/// CR266-2, as a property of the code: under `reader = "new"` a stat
/// that is lower on a stored row than on the row before it in the same
/// game is flagged on that row. Random games through the stored row's
/// gate (`hold_new_reader_stats`): growth, clips, ghost digits, resets,
/// held cells, suspect (untrusted) rows, splits with and without a
/// carried board. A row counts as stored when the game has no unread
/// column, which is the capture path's rule.
#[test]
fn new_reader_never_lowers_a_stored_stat_without_a_flag_on_random_games() {
    let mut seed = 0x2545_F491_4F6C_DD1Du64;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 7;
        seed ^= seed << 17;
        seed
    };
    let counters = |a: [u32; 6]| Counters {
        elims: a[0],
        assists: a[1],
        deaths: a[2],
        damage: a[3],
        healing: a[4],
        mitigation: a[5],
    };
    let (mut rows, mut lowered, mut pending, mut untrusted) = (0u64, 0u64, 0u64, 0u64);
    for game in 0..4_000u32 {
        let mut gate: Option<GateState> = None;
        // the row stored just before in this game, if any
        let mut before: Option<[u32; 6]> = None;
        let mut cur = [0u32; 6];
        for step in 0..(4 + next() % 12) {
            for (col, value) in cur.iter_mut().enumerate() {
                let wide = col >= 3;
                let grow = if wide { next() % 1500 } else { next() % 4 } as u32;
                *value = match next() % 100 {
                    0..=59 => value.saturating_add(grow),
                    60..=74 => *value,
                    // another row's numbers, a clip, a ghost digit, a reset
                    75..=82 => *value / 2,
                    83..=87 => *value / 10,
                    88..=91 => value.saturating_mul(10).saturating_add(next() as u32 % 10),
                    92..=95 => next() as u32 % if wide { 30_000 } else { 60 },
                    _ => 0,
                };
            }
            let held: [bool; 6] = std::array::from_fn(|_| next() % 8 == 0);
            let trusted = next() % 5 != 0;
            let split = gate.is_some() && next() % 25 == 0;
            let opens = split || gate.is_none();
            let carried = (opens && next() % 2 == 0).then(|| {
                let board: [u32; 6] =
                    std::array::from_fn(|col| next() as u32 % if col >= 3 { 3_000 } else { 6 });
                (counters(board), Duration::from_secs(40))
            });
            let secs = [3u64, 20, 20, 45, 120, 400][(next() % 6) as usize];
            let mut parsed = test_match("s", "unknown");
            parsed.suspect_fields = Vec::new();
            (parsed.elims, parsed.assists, parsed.deaths) = (cur[0], cur[1], cur[2]);
            (parsed.damage, parsed.healing, parsed.mitigation) = (cur[3], cur[4], cur[5]);
            let out = hold_new_reader_stats(
                &mut parsed,
                gate.map(|state| (state, Duration::from_secs(secs))),
                held,
                held,
                split,
                trusted,
                carried,
            );
            if opens {
                // a carried board is the row stored just before this one
                before = carried.map(|(board, _)| {
                    [
                        board.elims,
                        board.assists,
                        board.deaths,
                        board.damage,
                        board.healing,
                        board.mitigation,
                    ]
                });
            }
            gate = Some(out.state);
            untrusted += u64::from(!trusted);
            if !out.state.unread_cols().is_empty() {
                // no row yet: with a carried board every held column is
                // seeded, so this is a game with nothing stored
                assert!(carried.is_none() || !opens, "game {game} step {step}");
                pending += 1;
                continue;
            }
            let stored = o10_stats(&parsed);
            if let Some(before) = before {
                for (col, name) in capture_gate::COL_FIELD.iter().enumerate() {
                    if stored[col] < before[col] {
                        lowered += 1;
                        assert!(
                            parsed.suspect_fields.iter().any(|f| f == name),
                            "game {game} step {step}: {name} went {} to {} with flags {:?}",
                            before[col],
                            stored[col],
                            parsed.suspect_fields
                        );
                    }
                }
            }
            before = Some(stored);
            rows += 1;
        }
    }
    // the run really has stored rows, lowered stats and the hard cases
    assert!(rows > 10_000, "{rows}");
    assert!(lowered > 100, "{lowered}");
    assert!(pending > 100 && untrusted > 1_000, "{pending} {untrusted}");
}
