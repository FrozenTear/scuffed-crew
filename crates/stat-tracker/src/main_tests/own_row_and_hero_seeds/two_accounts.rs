//! Daemon tests: two of the member's saved accounts on one board (alpha.3
//! item 15). The text fallback's names, and the ocr-v1 own row with the
//! highlighted row's own name read. A child of `own_row_and_hero_seeds.rs`,
//! kept apart so both files stay under the size limit.

use super::*;

/// The raw-text fallback tries every saved name: an alt's line is read
/// when the main name is on no line, and a name with no line stays a
/// "row not found".
#[test]
fn the_text_fallback_reads_an_alt_names_line() {
    let raw = "Ana\nBusan\nÖSTER 10 4 2 3,000 800 400";
    let names = ["FrozenTear".to_string(), "öster".to_string()];
    let read = read_scoreboard_named(&[], None, None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
    let main_only = ["FrozenTear".to_string()];
    assert!(matches!(
        read_scoreboard_named(&[], None, None, raw, "victory", &main_only),
        Err(parse::ScoreboardMiss::PlayerRowNotFound)
    ));
}

/// The main name in a chat line makes that name "found but unreadable"
/// (the text fallback skips chat). With no own row the next saved name is
/// still tried, so an alt's stat line is read. With no alt line the miss
/// stays "unreadable", not "row not found". A highlighted row whose name
/// reads as the alt reads the alt's line past the chat line too.
#[test]
fn a_main_name_in_chat_does_not_hide_the_alts_line() {
    let raw = "[Team] FrozenTear: 1 2 3 4 5 6\nÖSTER 10 4 2 3,000 800 400";
    let names = ["FrozenTear".to_string(), "öster".to_string()];
    let chat_only = "[Team] FrozenTear: 1 2 3 4 5 6";
    let read = read_scoreboard_named(&[], None, None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
    assert!(!read.trusted_cells);
    assert!(matches!(
        read_scoreboard_named(&[], None, None, chat_only, "victory", &names),
        Err(parse::ScoreboardMiss::CellsUnreadable)
    ));
    let alt_row = [two_account_row(Some("ÖSTER"), false)];
    let read = read_scoreboard_named(&alt_row, Some(0), None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
    assert!(!read.trusted_cells);
    assert!(matches!(
        read_scoreboard_named(&alt_row, Some(0), None, chat_only, "victory", &names),
        Err(parse::ScoreboardMiss::CellsUnreadable)
    ));
}

/// One OCR row for the two-account tests: `name` as its name read, and
/// stat cells that parse (`readable`) or are blank.
fn two_account_row(name: Option<&str>, readable: bool) -> ocr::RowOcrResult {
    let stats = ["5", "3", "2", "4,316", "1,200", "899"];
    lobby_row(name, readable.then_some(stats))
}

/// One OCR row of a lobby: `name` as its name read, and `stats` as its six
/// stat cells (`None`: blank cells that do not parse).
fn lobby_row(name: Option<&str>, stats: Option<[&str; 6]>) -> ocr::RowOcrResult {
    let cell = |value: &str| ocr::CellOcrResult {
        value: value.into(),
        confidence: 80,
        suspect: false,
    };
    ocr::RowOcrResult {
        name: name.map(cell),
        name_alternates: Vec::new(),
        stats: stats.unwrap_or([""; 6]).iter().map(|v| cell(v)).collect(),
        mean_confidence: 80,
    }
}

/// Alpha.3 item 15. A highlighted own row whose cells do not parse, on a
/// board with two of the member's saved names: the raw text holds a
/// readable line for the OTHER saved name only. The row reads as the main
/// name, as no saved name, or was not read at all: none of these reads the
/// other account's line. The read is "unreadable".
#[test]
fn an_unreadable_own_row_does_not_read_the_other_saved_accounts_line() {
    let names = ["Main".to_string(), "Alt".to_string()];
    let raw = "Ana\nBusan\nALT 10 4 2 3,000 800 400";
    for name in [Some("MAIN"), Some("Stranger"), None] {
        let rows = [two_account_row(name, false)];
        assert!(
            matches!(
                read_scoreboard_named(&rows, Some(0), None, raw, "victory", &names),
                Err(parse::ScoreboardMiss::CellsUnreadable)
            ),
            "row name {name:?}"
        );
    }
    // No OCR rows at all: the row's name was never read. This used to read
    // Alt's line; it is now unreadable on purpose.
    assert!(matches!(
        read_scoreboard_named(&[], Some(0), None, raw, "victory", &names),
        Err(parse::ScoreboardMiss::CellsUnreadable)
    ));
}

/// The same board, with that row's own name on a readable text line: the
/// row's own line is read (low trust), not the other account's. Cells
/// that parse still win over any line. One saved name reads its line
/// beside the row as before, whatever the row's name read says.
#[test]
fn a_highlighted_row_reads_its_own_line_when_its_cells_do_not_parse() {
    let names = ["Main".to_string(), "Alt".to_string()];
    let raw = "[Team] Alt: 1 2 3 4 5 6\nMAIN 3 1 4 1,500 900 260\nALT 10 4 2 3,000 800 400";
    for (name, want) in [("ALT", (10, 3000)), ("MAIN", (3, 1500))] {
        let rows = [two_account_row(Some(name), false)];
        let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", &names).unwrap();
        assert_eq!((read.matched.elims, read.matched.damage), want, "{name}");
        assert!(!read.trusted_cells, "{name}");
    }
    let rows = [two_account_row(Some("MAIN"), true)];
    let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (5, 4316));
    assert!(read.trusted_cells);
    let main_only = ["Main".to_string()];
    let rows = [two_account_row(None, false)];
    let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", &main_only).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (3, 1500));
}

/// Site's probe on PR review (ocr-v1): "Alpha" sits inside "Alpha2". The
/// highlighted row reads ALPHA2 and its cells do not parse; the text holds
/// both accounts' lines. The own line (3, 1500) is read in either saved
/// order. The old first-substring rule named the row "Alpha" with Alpha
/// listed first and read the other account's line (10, 3000).
#[test]
fn an_overlapping_saved_name_reads_the_highlighted_rows_own_line() {
    let raw = "Ana\nBusan\nALPHA 10 4 2 3,000 800 400\nALPHA2 3 1 4 1,500 900 260";
    let rows = [two_account_row(Some("ALPHA2"), false)];
    let main_first = ["Alpha".to_string(), "Alpha2".to_string()];
    let alt_first = ["Alpha2".to_string(), "Alpha".to_string()];
    for names in [&main_first, &alt_first] {
        assert_eq!(
            parse::best_name_for_row(&rows[0], &names[..]),
            Some("Alpha2"),
            "{names:?}"
        );
        let read = read_scoreboard_named(&rows, Some(0), None, raw, "victory", names).unwrap();
        assert_eq!(
            (read.matched.elims, read.matched.damage),
            (3, 1500),
            "{names:?}"
        );
        assert!(!read.trusted_cells, "{names:?}");
    }
}

/// A lobby of ten rows for the ocr-v1 own-row tests: `other` (another of
/// the member's saved names) readable on row 7 with (10, 3000), `own` on
/// the highlighted row 2 with `own_stats`, strangers elsewhere.
fn lobby(own: &str, own_stats: Option<[&str; 6]>, other: &str) -> Vec<ocr::RowOcrResult> {
    let other_stats = ["10", "4", "2", "3,000", "800", "400"];
    let stranger = ["1", "1", "1", "100", "100", "100"];
    (0..10)
        .map(|i| match i {
            2 => lobby_row(Some(own), own_stats),
            7 => lobby_row(Some(other), Some(other_stats)),
            _ => lobby_row(Some(&format!("Stranger{i}")), Some(stranger)),
        })
        .collect()
}

/// The highlighted row's own stat cells in [`lobby`]: (7, 2100).
const OWN_STATS: [&str; 6] = ["7", "2", "1", "2,100", "300", "50"];

/// The ocr-v1 own row as the daemon chooses it: `analyze_frame`'s name
/// match is [`ocr_v1_name_match`], here with the names switch off (the
/// board match is the per-cell OCR read), then the highlight.
fn ocr_v1_own_row(
    rows: &[ocr::RowOcrResult],
    highlight: usize,
    names: &[String],
) -> (Option<usize>, Option<parse::NameMatch>) {
    let choice = find_own_row(
        false,
        || {
            let board = || {
                names::match_named_rows(rows, names, 5, names::BoardMatch::Cells)
                    .map(|found| (found, names::NameRecognizer::OcrV1))
            };
            ocr_v1_name_match(rows, Some(highlight), names, board).map(|(found, _)| found)
        },
        || panic!("seeds read under ocr-v1"),
        || panic!("name fallback under ocr-v1"),
        o9_lit(highlight, 1.5),
        None,
        false,
    );
    (choice.row_idx, choice.name_match)
}

/// Site's blocking finding 1, cells readable: FrozenTear plays on Alt
/// (highlighted, row 2) while Main, another saved name, sits readable on
/// row 7. The highlighted row wins and its own cells are stored, whatever
/// the saved order, and also when the row's name read is fuzzy ("A1T":
/// Alt 0.67, Main 0.25) while Main is an exact hit (the old board match
/// took row 7).
#[test]
fn ocr_v1_takes_the_highlighted_row_over_another_saved_name_on_the_board() {
    let main_first = ["Main".to_string(), "Alt".to_string()];
    let alt_first = ["Alt".to_string(), "Main".to_string()];
    for names in [&main_first, &alt_first] {
        let alt = names.iter().position(|n| n == "Alt").unwrap();
        for read_as in ["ALT", "A1T"] {
            let rows = lobby(read_as, Some(OWN_STATS), "MAIN");
            let (row, found) = ocr_v1_own_row(&rows, 2, names);
            assert_eq!(row, Some(2), "{names:?} {read_as}");
            assert_eq!(found.map(|m| m.name), Some(alt), "{names:?} {read_as}");
            let read = read_scoreboard_named(&rows, row, found, "", "victory", names).unwrap();
            assert_eq!((read.matched.elims, read.matched.damage), (7, 2100));
            assert!(read.trusted_cells);
        }
    }
    // A highlighted row that no saved name reads on leaves the board match
    // as before: Main's row. One saved name never asks the highlight.
    let rows = lobby("Stranger", Some(OWN_STATS), "MAIN");
    assert_eq!(
        highlighted_own_name(&rows, Some(2), &main_first),
        HighlightedName::Board
    );
    assert_eq!(ocr_v1_own_row(&rows, 2, &main_first).0, Some(7));
    let main_only = ["Main".to_string()];
    assert_eq!(
        highlighted_own_name(&rows, Some(2), &main_only),
        HighlightedName::Board
    );
}

/// Site's blocking finding 1, cells unreadable: the same lobby with the
/// highlighted Alt row's cells blank. The text fallback reads Alt's own
/// line, or, with no Alt line, nothing; never Main's line.
#[test]
fn ocr_v1_reads_the_highlighted_rows_line_not_another_saved_names_row() {
    let main_first = ["Main".to_string(), "Alt".to_string()];
    let alt_first = ["Alt".to_string(), "Main".to_string()];
    let both = "Ana\nBusan\nMAIN 10 4 2 3,000 800 400\nALT 3 1 4 1,500 900 260";
    let main_line = "Ana\nBusan\nMAIN 10 4 2 3,000 800 400";
    for names in [&main_first, &alt_first] {
        let rows = lobby("ALT", None, "MAIN");
        let (row, found) = ocr_v1_own_row(&rows, 2, names);
        assert_eq!(row, Some(2), "{names:?}");
        let read = read_scoreboard_named(&rows, row, found, both, "victory", names).unwrap();
        assert_eq!((read.matched.elims, read.matched.damage), (3, 1500));
        assert!(!read.trusted_cells);
        assert!(
            matches!(
                read_scoreboard_named(&rows, row, found, main_line, "victory", names),
                Err(parse::ScoreboardMiss::CellsUnreadable)
            ),
            "{names:?}"
        );
    }
}

/// An unresolved highlighted read (`read` on row 2, saved names read on it
/// but none wins) keeps the highlight as the own row with no name, in both
/// saved orders, while `other`, another saved name, sits clean and readable
/// on row 7. Cells that parse are the highlighted row's (7, 2100); blank
/// cells give "unreadable" with only `other`'s line in the text, never
/// that line. The board match alone would have taken row 7, so putting it
/// back for this read fails here.
fn assert_unresolved_highlight(saved: [&str; 2], read: &str, other: &str) {
    let first: Vec<String> = saved.iter().map(|n| n.to_string()).collect();
    let swapped: Vec<String> = first.iter().rev().cloned().collect();
    let other_line = format!("Ana\nBusan\n{other} 10 4 2 3,000 800 400");
    for names in [&first, &swapped] {
        for own_stats in [Some(OWN_STATS), None] {
            let rows = lobby(read, own_stats, other);
            let why = format!("{names:?} {read} cells {}", own_stats.is_some());
            assert_eq!(
                highlighted_own_name(&rows, Some(2), names),
                HighlightedName::Unresolved,
                "{why}"
            );
            let board = names::match_named_rows(&rows, &names[..], 5, names::BoardMatch::Cells);
            assert_eq!(board.map(|m| m.row), Some(7), "{why}");
            let (row, found) = ocr_v1_own_row(&rows, 2, names);
            assert_eq!((row, found), (Some(2), None), "{why}");
            let tab = read_scoreboard_named(&rows, row, found, &other_line, "victory", names);
            match own_stats {
                Some(_) => {
                    let tab = tab.unwrap();
                    assert_eq!((tab.matched.elims, tab.matched.damage), (7, 2100), "{why}");
                    assert!(tab.trusted_cells, "{why}");
                }
                None => assert!(
                    matches!(tab, Err(parse::ScoreboardMiss::CellsUnreadable)),
                    "{why}"
                ),
            }
        }
    }
}

/// Site's finding 1 repro, renamed: FrozenTear plays Alpha2 on the
/// highlighted row, read "ALPHE2" (one letter misread). Both saved names
/// read it fuzzily at or above the floor with no clear lead (Alpha2 0.83,
/// Alpha 0.80: lead 0.03, under 0.15), while Alpha is a clean whole word on
/// row 7. The board match used to take row 7 at full trust.
#[test]
fn ocr_v1_keeps_the_highlight_on_a_close_read_of_the_longer_name() {
    assert_unresolved_highlight(["Alpha", "Alpha2"], "ALPHE2", "ALPHA");
}

/// The mirror: FrozenTear plays Alpha, read "ALPHE" (Alpha 0.80, Alpha2
/// 0.67: lead 0.13, under 0.15), with Alpha2 clean on row 7.
#[test]
fn ocr_v1_keeps_the_highlight_on_a_close_read_of_the_shorter_name() {
    assert_unresolved_highlight(["Alpha", "Alpha2"], "ALPHE", "ALPHA2");
}

/// The review's MAlN lobby: the highlighted row reads "MAlN" (Main 0.75,
/// Alt 0.67: lead 0.08, so the scorer names nothing) and Alt is readable
/// on row 7. The board match used to name Alt's row; the highlight stays.
#[test]
fn ocr_v1_keeps_the_highlight_on_a_maln_read_with_alt_on_the_board() {
    assert_unresolved_highlight(["Main", "Alt"], "MAlN", "ALT");
}

/// Site's re-review nit 1: the highlighted row reads "ALPHAZ" (Alpha2 with
/// the 2 misread). Alpha is a substring of it and Alpha2 reads it fuzzily
/// at 0.83, so neither name is given, and the text fallback does not read
/// Alpha's line from the board text. Before, the substring named the row
/// Alpha and its unreadable cells read Alpha's (10, 3000).
#[test]
fn ocr_v1_keeps_the_highlight_when_a_shorter_names_substring_meets_the_longer_name() {
    assert_unresolved_highlight(["Alpha", "Alpha2"], "ALPHAZ", "ALPHA");
    // The same row given by the highlight alone (no match at all): its own
    // read names nothing, so the text fallback gets no name either.
    let raw = "Ana\nBusan\nALPHA 10 4 2 3,000 800 400";
    for names in [
        ["Alpha".to_string(), "Alpha2".to_string()],
        ["Alpha2".to_string(), "Alpha".to_string()],
    ] {
        let rows = [two_account_row(Some("ALPHAZ"), false)];
        assert!(
            matches!(
                read_scoreboard_named(&rows, Some(0), None, raw, "victory", &names),
                Err(parse::ScoreboardMiss::CellsUnreadable)
            ),
            "{names:?}"
        );
    }
}

/// The highlighted row's name can sit in another name window (5v5 perk
/// and centred layouts): the match reports that window.
#[test]
fn the_highlighted_rows_name_is_read_in_any_name_window() {
    let names = ["Main".to_string(), "Alt".to_string()];
    let mut rows = lobby("0 3", None, "MAIN");
    rows[2].name_alternates = vec![
        Some(ocr::CellOcrResult {
            value: "ALT".into(),
            confidence: 80,
            suspect: false,
        }),
        None,
    ];
    let HighlightedName::Named(found) = highlighted_own_name(&rows, Some(2), &names) else {
        panic!("named");
    };
    assert_eq!((found.row, found.window, found.name), (2, 1, 1));
}

/// Nit 4 of the review: when the names pack (or the highlight's own read)
/// matched the row, the text fallback searches for that saved name, not
/// for what Tesseract's per-cell name read of the row says. A match for
/// another row, or a name index past the list, falls back to the row's
/// own read.
#[test]
fn a_pack_matched_row_reads_the_matched_names_line() {
    let names = ["Main".to_string(), "Alt".to_string()];
    let raw = "Ana\nBusan\nMAIN 3 1 4 1,500 900 260\nALT 10 4 2 3,000 800 400";
    let rows = [two_account_row(Some("MAIN"), false)];
    let pack = |row, name| {
        Some(parse::NameMatch {
            row,
            name,
            ..parse::NameMatch::default()
        })
    };
    for (found, want) in [
        (pack(0, 1), (10, 3000)),
        (pack(0, 0), (3, 1500)),
        (pack(3, 1), (3, 1500)),
        (pack(0, 9), (3, 1500)),
        (None, (3, 1500)),
    ] {
        let read = read_scoreboard_named(&rows, Some(0), found, raw, "victory", &names).unwrap();
        assert_eq!((read.matched.elims, read.matched.damage), want, "{found:?}");
    }
    // A pack match on a row whose Tesseract read names no saved name.
    let rows = [two_account_row(Some("Stranger"), false)];
    let read = read_scoreboard_named(&rows, Some(0), pack(0, 1), raw, "victory", &names).unwrap();
    assert_eq!((read.matched.elims, read.matched.damage), (10, 3000));
}

/// The daemon's ocr-v1 name match is the shared [`ocr_v1_name_match`]
/// (the one the tests above drive), which asks the highlighted row first,
/// gives no match for an unresolved read, and only then runs the board
/// match, handed in as a closure. The match is passed on to the text
/// fallback.
#[test]
fn the_ocr_v1_name_match_asks_the_highlighted_row_first() {
    let norm = |s: &str| s.split_whitespace().collect::<Vec<_>>().join(" ");
    let body = norm(fn_source("analyze_frame"));
    assert_eq!(body.matches("ocr_v1_name_match(").count(), 1);
    assert_eq!(body.matches("highlighted_own_name(").count(), 0);
    let board = body.find("let board = || { let hit = names::match_player_row(");
    let call = body.find("= ocr_v1_name_match(&rows, lit, &own_names, board)?;");
    assert!(board.expect("board closure") < call.expect("shared decision"));
    let decide = norm(fn_source("ocr_v1_name_match"));
    assert!(decide.contains("match highlighted_own_name(rows, highlight, saved) {"));
    assert!(decide.contains("HighlightedName::Unresolved => None,"));
    assert!(decide.contains("HighlightedName::Board => board(),"));
    let hc_body = norm(fn_source("handle_capture"));
    assert!(hc_body.contains("read_scoreboard_named( &rows, player_row_idx, name_match,"));
}

/// Fail safe for alpha.3 item 15, through `handle_capture` (ocr-v1, two
/// saved names). A Tab whose highlighted row (named Main) has no readable
/// cells, while the text holds Alt's stat line, stores nothing and logs a
/// WARN. A row that names no saved name does the same and also logs why at
/// INFO; a readable Tab logs no such line. The game stays open, and the
/// next Tab with readable cells is recorded on it. Alt's numbers are not
/// stored.
#[tokio::test]
async fn an_unreadable_highlighted_row_warns_and_the_game_keeps_recording() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let log = test_log::TestLog::default();
    let sub = tracing_subscriber::fmt()
        .with_writer(log.clone())
        .with_ansi(false)
        .with_max_level(tracing::Level::INFO)
        .finish();
    let _sub = tracing::subscriber::set_default(sub);
    tracing::callsite::rebuild_interest_cache();
    let own_row = |name: &str, readable: bool| {
        let mut analysis = o8_analysis(None, Some("zenyatta"));
        analysis.rows[0] = two_account_row(Some(name), readable);
        analysis.ocr = Ok(ocr::OcrResult {
            raw_text: "Ana\nBusan\nALT 25 5 1 9,000 100 200".into(),
            confidence: 80,
        });
        analysis
    };
    let dir = tempfile::tempdir().unwrap();
    let mut ctx = o8_ctx(dir.path(), false).await;
    ctx.own_names = vec!["Main".into(), "Alt".into()];
    let mut now = test_now();
    let mut active = game(detect::MatchOutcome::Unknown, None, now);
    active.session_id = "two".into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    let mut st = session(Some(active), now);

    let tabs = [("MAIN", false), ("STRANGER", false), ("MAIN", true)];
    for (tab, (name, readable)) in tabs.into_iter().enumerate() {
        *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
            analysis: own_row(name, readable),
            board: None,
        });
        let req = build_capture_request(st.active_game.as_ref().unwrap(), false, now);
        let report = handle_capture(&ctx, req).await.unwrap();
        assert_eq!(report.recorded, readable, "tab {tab}");
        if !readable {
            assert!(report.ignore_row, "tab {tab}");
        }
        apply_capture_report(&mut st, &ctx.store, &ctx.data_dir, "two", Ok(report), now).await;
        let open = st.active_game.as_ref().expect("the game stays open");
        assert_eq!(open.session_id, "two", "tab {tab}");
        now += Duration::from_secs(20);
    }

    let text = String::from_utf8(log.0.lock().unwrap().clone()).unwrap();
    let warned = text
        .lines()
        .filter(|l| l.contains("WARN") && l.contains("capture rejected: stat cells unreadable"))
        .count();
    assert_eq!(warned, 2, "{text}");
    let why = text
        .lines()
        .filter(|l| l.contains("INFO") && l.contains("names none of the saved names"))
        .count();
    assert_eq!(why, 1, "{text}");
    let rows = ctx.store.get_session_snapshots("two").await.unwrap();
    assert_eq!(rows.len(), 1, "{rows:?}");
    assert_eq!((rows[0].elims, rows[0].damage), (5, 4316));
}
