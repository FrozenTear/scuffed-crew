//! Daemon tests: cell_trust. See `mod.rs`.

use super::*;

#[test]
fn the_new_reader_reads_no_stat_cell_and_calibrates_nothing() {
    let named = || {
        vec![ocr::RowOcrResult {
            name: None,
            name_alternates: Vec::new(),
            stats: Vec::new(),
            mean_confidence: 0,
        }]
    };
    // new: the cell pass (calibration, then every stat cell) never runs
    let rows = read_tab_rows(true, o10_never, named);
    assert_eq!(rows.len(), 1);
    assert!(rows[0].stats.is_empty());
    // ocr-v1: the cell pass runs, the name-only pass does not
    let rows = read_tab_rows(false, o8_rows, o10_never);
    assert_eq!(rows.len(), 10);
    assert_eq!(rows[0].stats.len(), 6);
}

#[test]
fn the_new_reader_never_runs_the_full_board_ocr() {
    type Text = Result<ocr::OcrResult, Box<dyn std::error::Error + Send + Sync>>;
    let text = |raw: &str| -> Text {
        Ok(ocr::OcrResult {
            raw_text: raw.into(),
            confidence: 77,
        })
    };
    // new: neither the "is it needed" check nor the OCR runs, even on a
    // Tab where ocr-v1 would need it
    let read = full_board_text(
        true,
        || panic!("needed() must not run"),
        || panic!("the full-board OCR must not run"),
    )
    .unwrap();
    assert_eq!((read.raw_text.as_str(), read.confidence), ("", 0));
    // ocr-v1, needed: the OCR text, or its error, comes back as it is
    let read = full_board_text(false, || true, || text("~ Defeat")).unwrap();
    assert_eq!((read.raw_text.as_str(), read.confidence), ("~ Defeat", 77));
    let failed = full_board_text(false, || true, || -> Text { Err("tesseract".into()) });
    assert_eq!(failed.unwrap_err().to_string(), "tesseract");
    // ocr-v1, not needed: skipped, the empty text
    let read = full_board_text(false, || false, || panic!("not needed")).unwrap();
    assert_eq!((read.raw_text.as_str(), read.confidence), ("", 0));
}

#[test]
fn the_board_is_read_only_under_the_new_reader() {
    assert_eq!(
        read_tab_board(false, || panic!("ocr-v1 reads no board")),
        None
    );
    let board = read_tab_board(true, || o8_board(None)).unwrap();
    assert_eq!(board.status, shadow::BoardStatus::Read);
    // a panic in the reader is "no board", not a failed Tab
    let board = read_tab_board(true, || panic!("board reader")).unwrap();
    assert_eq!(board.status, shadow::BoardStatus::NotFound);
    assert_eq!(board.team_size, None);
}

/// OCR sites T2, T3 and T7 run only through the reader switch, and the
/// trust gate and the stats take the board under the new reader.
/// Whitespace is ignored; hard-wiring a switch or a second OCR call
/// breaks this.
#[test]
fn the_cell_ocr_sites_go_through_the_reader_switch() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let src = include_str!("../main.rs");
    // In src/daemon/tab/vision.rs since 0.6 S3: found by name, wherever it lives.
    let body = norm(fn_source("analyze_frame"));
    assert!(body.contains(
        "let read_cells = || ocr::recognize_scoreboard_cells_pre_cropped(&scoreboard, team_size); let mut rows = read_tab_rows(use_new_reader, read_cells, || ocr::unread_rows(team_size));"
    ));
    assert_eq!(body.matches("recognize_scoreboard_cells").count(), 1);
    assert!(body.contains("let ocr = full_board_text( use_new_reader, || {"));
    assert!(body.contains("|| ocr::recognize(&img), );"));
    assert_eq!(body.matches("ocr::recognize(").count(), 1);
    let hc = src
        .find("\nasync fn handle_capture(")
        .expect("handle_capture");
    let hc_body = norm(src[hc..].split("\n}\n").next().unwrap());
    assert!(hc_body.contains(
        "let looks_like_scoreboard = match &new_board { Some(_) => board_refusal.is_none(), None => parse::looks_like_scoreboard(&rows), };"
    ));
    assert!(hc_body.contains(
        "let scoreboard_read = match &new_board { Some(board) => reader_apply::read_own_row_stats(board, board_row, &outcome_label) .map(|read| (read.matched, true, read.held)), None => parse::read_scoreboard("
    ));
    assert_eq!(hc_body.matches("parse::read_scoreboard(").count(), 1);
    assert!(hc_body.contains(
        "let suspect = if board_reader { held_cols } else { parse::player_row_suspect_mask(&rows, player_row_idx) };"
    ));
    assert!(hc_body.contains(
        "match &new_board { Some(board) => reader_apply::both_teams_have_stats(board, team_size), None => parse::both_teams_have_stats(&rows, team_size), },"
    ));
    assert!(hc_body.contains("let board_reader = ctx.use_new_reader && !ocr_fallback;"));
    assert!(hc_body.contains("let new_board = board_reader.then(|| {"));
    // the only other cell read: a board found empty, with OCR on
    assert!(body.contains("if let Some(reason) = ocr_fallback {"));
    assert!(body.contains("rows = read_cells();"));
}

/// The whole point of O10: a cell the digit reader does not vouch for
/// never takes an OCR number. The OCR cells of the same Tab (`o8_rows`:
/// 10 4 2 3000 800 400) are never the number stored. A suspect digit
/// read the gate would take is stored, flagged; a cell with no number
/// keeps the game's last accepted value, flagged.
#[tokio::test]
async fn new_reader_holds_a_suspect_cell_and_never_takes_the_ocr_cell() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let tabs = vec![
        // all six sure
        (
            o8_analysis(None, Some("ana")),
            Some(o10_board([
                sure(12),
                sure(5),
                sure(3),
                sure(3500),
                sure(700),
                sure(300),
            ])),
        ),
        // healing suspect (a plausible 750), mitigation with no number
        (
            o8_analysis(None, Some("ana")),
            Some(o10_board([
                sure(13),
                sure(5),
                sure(3),
                sure(3700),
                Some((750, true)),
                None,
            ])),
        ),
        // both read sure again
        (
            o8_analysis(None, Some("ana")),
            Some(o10_board([
                sure(13),
                sure(6),
                sure(3),
                sure(3900),
                sure(1000),
                sure(500),
            ])),
        ),
    ];
    let (rows, recorded) = o10_tabs(&ctx, tabs).await;
    assert_eq!(recorded, vec![true, true, true]);
    assert_eq!(o10_stats(&rows[0]), [12, 5, 3, 3500, 700, 300]);
    assert!(
        rows[0].suspect_fields.is_empty(),
        "{:?}",
        rows[0].suspect_fields
    );
    // healing stores the reader's own suspect 750 (flagged), not the
    // older 700; mitigation has no number and keeps 300 (flagged). Never
    // OCR's 800 and 400, and never a 0 written over a good value.
    assert_eq!(o10_stats(&rows[1]), [13, 5, 3, 3700, 750, 300]);
    assert_eq!(
        rows[1].suspect_fields,
        vec!["h".to_string(), "mit".to_string()]
    );
    assert_eq!(rows[1].recognizer, shadow::digits::RECOGNIZER_ID);
    assert_eq!(o10_stats(&rows[2]), [13, 6, 3, 3900, 1000, 500]);
    assert!(
        rows[2].suspect_fields.is_empty(),
        "{:?}",
        rows[2].suspect_fields
    );
}

/// A Tab the new reader's board does not vouch for stores nothing, even
/// when ocr-v1's cells and text read it perfectly. ocr-v1 itself, on the
/// same Tabs, stores its own read and never looks at the board.
#[tokio::test]
async fn new_reader_refuses_a_tab_its_board_does_not_vouch_for() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let not_found = || shadow::reader::not_found(shadow::BoardStatus::NotFound);
    let unsure_row = || {
        o10_board([
            Some((10, true)),
            Some((4, true)),
            Some((2, true)),
            Some((3000, true)),
            None,
            None,
        ])
    };
    let six_v_six = || {
        let mut board = o8_board(Some("ana"));
        board.team_size = Some(6);
        board
    };
    let tabs = || {
        vec![
            (o8_analysis(None, Some("ana")), Some(not_found())),
            (o8_analysis(None, Some("ana")), None),
            (o8_analysis(None, Some("ana")), Some(unsure_row())),
            (o8_analysis(None, Some("ana")), Some(six_v_six())),
        ]
    };
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (rows, recorded) = o10_tabs(&ctx, tabs()).await;
    assert_eq!(recorded, vec![false, false, false, false]);
    assert!(rows.is_empty(), "nothing is written: {}", rows.len());

    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), false).await;
    let (rows, recorded) = o10_tabs(&ctx, tabs()).await;
    assert_eq!(recorded, vec![true, true, true, true]);
    for row in &rows {
        assert_eq!(o10_stats(row), [10, 4, 2, 3000, 800, 400]);
        assert_eq!(row.recognizer, scuffed_types::RECOGNIZER_OCR_V1);
        assert!(row.suspect_fields.is_empty());
    }
}

/// A board the new reader found nothing on, read with ocr-v1 instead
/// (`ocr_fallback`), stores ocr-v1's read of that Tab, tagged `ocr-v1`.
/// The next Tab with a board the reader finds goes back to the board.
#[tokio::test]
async fn new_reader_board_found_empty_falls_back_to_ocr_v1_for_that_tab() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let mut fallback = o8_analysis(None, Some("ana"));
    fallback.board = Some(shadow::reader::not_found(shadow::BoardStatus::NotFound));
    fallback.ocr_fallback = true;
    let tabs = vec![
        (fallback, None),
        (
            o8_analysis(None, Some("ana")),
            Some(o10_board([
                sure(12),
                sure(5),
                sure(3),
                sure(3500),
                sure(900),
                sure(450),
            ])),
        ),
    ];
    let (rows, recorded) = o10_tabs(&ctx, tabs).await;
    assert_eq!(recorded, vec![true, true]);
    assert_eq!(o10_stats(&rows[0]), [10, 4, 2, 3000, 800, 400]);
    assert_eq!(rows[0].recognizer, scuffed_types::RECOGNIZER_OCR_V1);
    assert_eq!(o10_stats(&rows[1]), [12, 5, 3, 3500, 900, 450]);
    assert_eq!(rows[1].recognizer, shadow::digits::RECOGNIZER_ID);
}

/// Under the new reader the Tab's name-only rows have no stat cell, and
/// the board alone carries the capture: acceptance, stats, both teams.
#[tokio::test]
async fn new_reader_stores_from_the_board_with_no_ocr_cells_or_text() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    // what analyze_frame hands over under the new reader
    let mut analysis = o8_analysis(None, Some("ana"));
    analysis.rows = ocr::unread_rows(5);
    analysis.ocr = Ok(ocr::OcrResult {
        raw_text: String::new(),
        confidence: 0,
    });
    analysis.board = Some(o10_board([
        sure(12),
        sure(5),
        sure(3),
        sure(3500),
        sure(900),
        sure(450),
    ]));
    let (rows, recorded) = o10_tabs(&ctx, vec![(analysis, None)]).await;
    assert_eq!(recorded, vec![true]);
    assert_eq!(o10_stats(&rows[0]), [12, 5, 3, 3500, 900, 450]);
    assert_eq!(rows[0].recognizer, shadow::digits::RECOGNIZER_ID);
    assert_eq!(rows[0].hero, "Ana");
    assert_eq!(rows[0].map_name, "Busan");
    // the same Tab under ocr-v1 has nothing to read
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), false).await;
    let mut analysis = o8_analysis(None, Some("ana"));
    analysis.rows = ocr::unread_rows(5);
    let (rows, recorded) = o10_tabs(&ctx, vec![(analysis, None)]).await;
    assert_eq!(recorded, vec![false]);
    assert!(rows.is_empty());
}

/// The Junkenstein gate (an Adlersbrunn label with stats on one team only
/// is not stored as Eichenwalde) counts the teams on the board under the
/// new reader: its Tab rows have no stat cell to count.
#[tokio::test]
async fn new_reader_counts_both_teams_on_the_board_for_the_pve_gate() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let tab = |both_teams: bool| {
        let mut analysis = o8_analysis(None, Some("ana"));
        analysis.rows = ocr::unread_rows(5);
        analysis.map_from_panel = Some("Eichenwalde".into());
        analysis.map_panel_raw = Some("ADLERSBRUNN".into());
        let mut board = o8_board(Some("ana"));
        if both_teams {
            for suffix in reader_apply::STAT_SUFFIXES {
                board.fields.push(shadow::FieldRead {
                    name: format!("r7.{suffix}"),
                    value: Some(shadow::Value::Int(3)),
                    confidence: 0.9,
                    suspect: false,
                });
            }
        }
        (analysis, Some(board))
    };
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (rows, recorded) = o10_tabs_on(&ctx, vec![tab(true)], "Eichenwalde").await;
    assert_eq!(
        recorded,
        vec![true],
        "a board with both teams is a real game"
    );
    assert_eq!(rows[0].map_name, "Eichenwalde");
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (rows, recorded) = o10_tabs_on(&ctx, vec![tab(false)], "Eichenwalde").await;
    assert_eq!(recorded, vec![false], "one team only: the PvE board");
    assert!(rows.is_empty());
}

#[test]
fn fill_held_cells_puts_the_last_accepted_value_in_held_columns_only() {
    let mut parsed = test_match("s", "unknown");
    (parsed.elims, parsed.assists, parsed.deaths) = (13, 5, 3);
    (parsed.damage, parsed.healing, parsed.mitigation) = (3700, 0, 0);
    let held = [false, false, false, false, true, true];
    let prior = capture_gate::Counters {
        elims: 12,
        assists: 5,
        deaths: 3,
        damage: 3500,
        healing: 900,
        mitigation: 450,
    };
    fill_held_cells(&mut parsed, held, Some(prior));
    assert_eq!(o10_stats(&parsed), [13, 5, 3, 3700, 900, 450]);
    // no earlier board: 0, and a sure column is never touched
    fill_held_cells(&mut parsed, [true, false, false, false, true, false], None);
    assert_eq!(o10_stats(&parsed), [0, 5, 3, 3700, 0, 450]);
}

/// Report a6f3: healing 3143 was read for sure, then the Tab 105 s later
/// read 5298 below the suspect line. The game's last accepted 3143 was
/// stored under a board that showed 5298 and, with no later Tab, was the
/// game's final value. The unsure read is stored now, flagged, and a
/// later sure read replaces it.
#[tokio::test]
async fn a_held_cell_stores_its_unsure_read_not_the_older_value() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let held_h = [
        sure(7),
        sure(10),
        sure(2),
        sure(1268),
        Some((5298, true)),
        sure(518),
    ];
    let (st, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([4, 6, 2, 900, 3143, 300])),
            (105, held_h),
            (9, held_h),
            (60, all_sure([8, 11, 2, 1400, 5600, 560])),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    let stats: Vec<[u32; 6]> = rows.iter().map(o10_stats).collect();
    assert_eq!(
        stats,
        vec![
            [4, 6, 2, 900, 3143, 300],
            [7, 10, 2, 1268, 5298, 518],
            [7, 10, 2, 1268, 5298, 518],
            [8, 11, 2, 1400, 5600, 560],
        ]
    );
    assert!(rows[0].suspect_fields.is_empty());
    assert_eq!(rows[1].suspect_fields, vec!["h".to_string()]);
    assert_eq!(rows[2].suspect_fields, vec!["h".to_string()]);
    assert!(
        rows[3].suspect_fields.is_empty(),
        "{:?}",
        rows[3].suspect_fields
    );
    // the gate itself is unchanged: it still holds the last sure read
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert_eq!(gate.accepted.healing, 5600);
    assert!(!gate.has_unsure_shown.iter().any(|&shown| shown));
}

/// An unsure read the gate would not take keeps the accepted value,
/// flagged: one below it (a stat never falls in a game) and one past the
/// suspect rate cap (an injected digit).
#[tokio::test]
async fn a_held_cell_keeps_the_accepted_value_when_the_gate_would_hold_its_read() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let tab = |h: u32| {
        [
            sure(7),
            sure(10),
            sure(2),
            sure(1268),
            Some((h, true)),
            sure(518),
        ]
    };
    let (_, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([4, 6, 2, 900, 3143, 300])),
            (20, tab(2143)),
            (20, tab(52980)),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(o10_stats(&rows[1])[4], 3143, "below the accepted value");
    assert_eq!(o10_stats(&rows[2])[4], 3143, "past the suspect rate cap");
    assert_eq!(rows[1].suspect_fields, vec!["h".to_string()]);
    assert_eq!(rows[2].suspect_fields, vec!["h".to_string()]);
}

/// R3-1 with a stored unsure read: the next row is compared with what
/// the stored row showed, so a sure read below the unsure one (still
/// above the accepted value) is flagged, not stored as a silent drop.
#[tokio::test]
async fn a_sure_read_below_a_stored_unsure_read_is_flagged() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let mut now = test_now();
    let mut st = o10_session(now);
    let mut seen = Vec::new();
    seen.push(
        o10_tab(
            &ctx,
            &mut st,
            &mut now,
            0,
            all_sure([4, 6, 2, 900, 3143, 300]),
            None,
        )
        .await,
    );
    let held_h = [
        sure(5),
        sure(7),
        sure(2),
        sure(1000),
        Some((5298, true)),
        sure(320),
    ];
    seen.push(o10_tab(&ctx, &mut st, &mut now, 60, held_h, None).await);
    // read 2 stored the unsure 5298, flagged, and the game remembers it
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(o10_stats(&rows[1]), [5, 7, 2, 1000, 5298, 320]);
    assert_eq!(rows[1].suspect_fields, vec!["h".to_string()]);
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert_eq!(
        gate.accepted.healing, 3143,
        "the gate keeps its accepted value"
    );
    assert!(gate.has_unsure_shown[4]);
    assert_eq!(gate.unsure_shown[4], 5298);
    assert!(!gate.has_unsure_shown[..4].iter().any(|&shown| shown));
    assert!(!gate.has_unsure_shown[5]);
    // a restart between reads 2 and 3: the saved game is reloaded with
    // the shown value
    let recovered = recover_active_game(dir.path()).expect("the saved game reloads");
    let gate = recovered.gate.expect("the reloaded game keeps its gate");
    assert!(gate.has_unsure_shown[4]);
    assert_eq!(gate.unsure_shown[4], 5298);
    assert_eq!(gate.accepted.healing, 3143);
    st.active_game = Some(recovered);
    // read 3 is all sure, with healing below the stored 5298 (still
    // above the accepted 3143): stored, and flagged as lowered
    seen.push(
        o10_tab(
            &ctx,
            &mut st,
            &mut now,
            20,
            all_sure([5, 7, 2, 1050, 5200, 330]),
            None,
        )
        .await,
    );
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    assert!(seen.iter().all(|tab| !tab.split), "{seen:?}");
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    assert_eq!(rows.len(), 3);
    assert_eq!(o10_stats(&rows[2]), [5, 7, 2, 1050, 5200, 330]);
    assert_eq!(rows[2].suspect_fields, vec!["h".to_string()]);
    // a sure read clears the mark
    let gate = st.active_game.as_ref().unwrap().gate.unwrap();
    assert!(!gate.has_unsure_shown.iter().any(|&shown| shown));
}

/// Unsure reads stored for held deaths and damage sit above the game's
/// accepted values. A later sure board below them, still at or above the
/// accepted values, is the same game: no split, one session, and the
/// sure row names the stats that went down from the stored row.
#[tokio::test]
async fn a_sure_board_below_stored_unsure_reads_stays_one_game_and_is_flagged() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let (st, seen) = o10_run(
        &ctx,
        vec![
            (0, all_sure([4, 6, 2, 900, 3000, 300])),
            (
                60,
                [
                    sure(5),
                    sure(7),
                    Some((3, true)),
                    Some((1500, true)),
                    sure(3100),
                    sure(320),
                ],
            ),
            (20, all_sure([5, 7, 2, 1200, 3150, 330])),
        ],
    )
    .await;
    assert!(seen.iter().all(|tab| tab.recorded), "{seen:?}");
    assert!(seen.iter().all(|tab| !tab.split), "{seen:?}");
    assert!(seen.iter().all(|tab| !tab.deferred), "{seen:?}");
    assert!(seen.iter().all(|tab| tab.session_id == "o10"), "{seen:?}");
    assert_eq!(
        st.active_game.as_ref().map(|g| g.session_id.as_str()),
        Some("o10")
    );
    let rows = ctx.store.get_session_snapshots("o10").await.unwrap();
    let stats: Vec<[u32; 6]> = rows.iter().map(o10_stats).collect();
    assert_eq!(
        stats,
        vec![
            [4, 6, 2, 900, 3000, 300],
            [5, 7, 3, 1500, 3100, 320],
            [5, 7, 2, 1200, 3150, 330],
        ]
    );
    assert_eq!(
        rows[1].suspect_fields,
        vec!["d".to_string(), "dmg".to_string()]
    );
    assert_eq!(
        rows[2].suspect_fields,
        vec!["d".to_string(), "dmg".to_string()]
    );
}
