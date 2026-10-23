use super::catalog::guess_role;
use super::*;
use crate::model::{CellOcrResult, MatchOutcome, RowOcrResult};

fn cell(value: &str) -> CellOcrResult {
    CellOcrResult {
        value: value.to_string(),
        confidence: 80,
        suspect: false,
    }
}

fn row(name: Option<&str>, stats: [&str; 6]) -> RowOcrResult {
    RowOcrResult {
        name: name.map(cell),
        name_alternates: Vec::new(),
        stats: stats.iter().map(|s| cell(s)).collect(),
        mean_confidence: 80,
    }
}

fn valid_row(name: &str) -> RowOcrResult {
    row(Some(name), ["5", "3", "2", "4,316", "1,200", "899"])
}

fn garbage_row() -> RowOcrResult {
    row(None, ["", "x", "", "", "9o", ""])
}

#[test]
fn exact_map_joins_wrapped_lines() {
    assert_eq!(
        exact_map_in_text("New Junk\nCity").as_deref(),
        Some("New Junk City")
    );
    assert_eq!(exact_map_in_text("BUSAN").as_deref(), Some("Busan"));
    assert!(exact_map_in_text("not a map").is_none());
}

/// A row read through several name windows (5v5: in-match, perks,
/// centred). Only one window holds the name; the others read digits,
/// perk-icon junk or nothing.
fn multi_window_row(reads: [&str; 3]) -> RowOcrResult {
    let mut r = row(Some(reads[0]), ["5", "3", "2", "4,316", "1,200", "899"]);
    r.name_alternates = reads[1..].iter().map(|s| Some(cell(s))).collect();
    r
}

#[test]
fn player_row_matches_on_any_name_window() {
    // 2026-10-12 5v5 in-match boards: the old single window read the E/A
    // digits on every row. The name only shows in one of the windows.
    let rows = vec![
        multi_window_row(["0 3", "ELYSIA", ""]),
        multi_window_row(["3 1", "@ ©", "PLAYERONE"]),
        multi_window_row(["0", "", "SZEZI"]),
    ];
    assert_eq!(find_player_row_by_name(&rows, "PlayerOne"), Some(1));
}

#[test]
fn player_row_keeps_the_best_window_per_row() {
    // Row 0's best read is a near-miss in its third window; row 1 holds a
    // weaker read in its first. The best read per row must win, not the
    // first window's read.
    let rows = vec![
        multi_window_row(["0", "x", "PLAYERONF"]),
        multi_window_row(["PLAYERXXX", "", ""]),
    ];
    assert_eq!(find_player_row_by_name(&rows, "PlayerOne"), Some(0));
    // Junk in every window stays unmatched.
    let junk = vec![multi_window_row(["0 3", "@ ©", "99"])];
    assert_eq!(find_player_row_by_name(&junk, "PlayerOne"), None);
}

#[test]
fn name_match_reports_the_window_that_matched() {
    // In-match no-perk board: only window 0 holds the name.
    let rows = vec![
        multi_window_row(["0 3", "@ ©", "1"]),
        multi_window_row(["PLAYERONE", "ow", ""]),
    ];
    let m = match_player_row_by_name(&rows, "PlayerOne").unwrap();
    assert_eq!((m.row, m.window, m.sole_window), (1, 0, true));

    // Perk board: the name is in window 1, window 0 read the perk icon.
    let rows = vec![multi_window_row(["@ 3", "PLAYERONE", ""])];
    let m = match_player_row_by_name(&rows, "PlayerOne").unwrap();
    assert_eq!((m.window, m.sole_window), (1, true));

    // Two windows read the name: window 0 wins, but not as the sole one.
    let rows = vec![multi_window_row(["PLAYERONE", "PLAYERONF", ""])];
    let m = match_player_row_by_name(&rows, "PlayerOne").unwrap();
    assert_eq!((m.window, m.sole_window), (0, false));

    // Fuzzy only: the best window is reported.
    let rows = vec![multi_window_row(["PLAYXXONE", "x", "PLAYERONF"])];
    let m = match_player_row_by_name(&rows, "PlayerOne").unwrap();
    assert_eq!((m.window, m.sole_window), (2, false));
}

#[test]
fn a_failed_read_keeps_the_window_index() {
    // Window 1's read failed; the centred window (2) still reports as 2.
    let mut r = row(Some("0"), ["5", "3", "2", "4,316", "1,200", "899"]);
    r.name_alternates = vec![None, Some(cell("PLAYERONE"))];
    let m = match_player_row_by_name(&[r], "PlayerOne").unwrap();
    assert_eq!((m.row, m.window, m.sole_window), (0, 2, true));
}

/// Reads the extra 5v5 windows produce on rows that are not the player's:
/// digits from the E/A columns, perk-icon and ult-ring junk, rank badge
/// numerals, and clipped plate edges.
const JUNK_READS: [[&str; 3]; 9] = [
    ["0 3", "©", "12"],
    ["IV", "@ O", "O"],
    ["0", "Q", "|"],
    ["3 1", "ow", "V"],
    ["87", "@", "64"],
    ["Il", "% ©", "8 3"],
    ["oO", "€", "a"],
    ["5 2", "”", "-"],
    ["|", "0 0 0", "e"],
];

fn junk_board_with(own_row: usize, own: [&str; 3]) -> Vec<RowOcrResult> {
    (0..10)
        .map(|i| {
            if i == own_row {
                multi_window_row(own)
            } else {
                multi_window_row(JUNK_READS[i % JUNK_READS.len()])
            }
        })
        .collect()
}

#[test]
fn short_names_do_not_match_junk_in_the_extra_windows() {
    for name in ["Kai", "Rex", "Yuna", "Zed", "Ana", "Mo3"] {
        // Junk in every window on every row: nothing matches.
        let rows: Vec<RowOcrResult> = (0..10)
            .map(|i| multi_window_row(JUNK_READS[i % JUNK_READS.len()]))
            .collect();
        assert_eq!(
            match_player_row_by_name(&rows, name),
            None,
            "{name}: junk read matched"
        );
        // The own row (team 1 row 3) reads the name in one window; the
        // other windows and rows are junk.
        for window in 0..3 {
            let mut own = ["0", "@", "|"];
            own[window] = name;
            let rows = junk_board_with(3, own);
            let m = match_player_row_by_name(&rows, name)
                .unwrap_or_else(|| panic!("{name} window {window}: own row not found"));
            assert_eq!((m.row, m.window), (3, window), "{name} window {window}");
        }
    }
}

#[test]
fn row_counts_only_for_a_validated_identified_row() {
    let rows = vec![valid_row("TEAMMATE"), valid_row("FROZEN")];
    assert!(row_counts(&rows, Some(1)));
    assert!(
        !row_counts(&rows, None),
        "an unidentified row does not count"
    );
    assert!(
        !row_counts(&rows, Some(9)),
        "a missing row index does not count"
    );
    let garbage = vec![garbage_row()];
    assert!(!row_counts(&garbage, Some(0)));
    let implausible = vec![row(
        Some("FROZEN"),
        ["118", "3", "2", "4,316", "1,200", "899"],
    )];
    assert!(
        !row_counts(&implausible, Some(0)),
        "a row stats_from_row rejects does not count"
    );
}

#[test]
fn identified_player_row_parses() {
    let rows = vec![valid_row("OTHER"), valid_row("FROZEN")];
    let parsed = parse_scoreboard_cells(&rows, Some(1), "", "victory", Some("FROZEN")).unwrap();
    assert_eq!(parsed.elims, 5);
    assert_eq!(parsed.damage, 4316);
    assert_eq!(parsed.outcome, "victory");
}

#[test]
fn unidentified_player_row_records_nothing() {
    // Valid rows exist, but none was positively identified as the player's.
    // The old "first plausible row" fallback recorded a teammate here.
    let rows = vec![valid_row("TEAMMATE"), valid_row("ANOTHER")];
    assert!(parse_scoreboard_cells(&rows, None, "", "victory", None).is_none());
    // A configured name that matches nothing must not change that.
    assert!(
        parse_scoreboard_cells(&rows, None, "no match here", "victory", Some("FROZEN")).is_none()
    );
}

#[test]
fn implausible_kill_columns_reject_the_row() {
    // A digit bleeding into the elims cell ("118" for a real ~18) must not
    // be recorded; the capture is dropped rather than poisoned.
    let rows = vec![row(
        Some("FROZEN"),
        ["118", "3", "2", "4,316", "1,200", "899"],
    )];
    assert!(parse_scoreboard_cells(&rows, Some(0), "", "defeat", Some("FROZEN")).is_none());
}

#[test]
fn match_from_stats_takes_the_six_stats_in_board_order() {
    let m = match_from_stats([1, 2, 3, 4000, 5000, 6000], "victory");
    assert_eq!(
        (
            m.elims,
            m.assists,
            m.deaths,
            m.damage,
            m.healing,
            m.mitigation
        ),
        (1, 2, 3, 4000, 5000, 6000)
    );
    assert_eq!((m.hero.as_str(), m.role.as_str()), ("Unknown", "Unknown"));
    assert_eq!((m.map_name.as_str(), m.game_mode.as_str()), ("", ""));
    assert_eq!(m.outcome, "victory");
    assert!(m.suspect_fields.is_empty() && !m.synced);
}

#[test]
fn raw_text_fallback_is_name_anchored() {
    // No per-cell row index, but the player's line is present in the
    // full-image OCR text → stats come from that line, not an arbitrary one.
    let raw = "SOMEONE 9 9 9 9999 9999 9999\nFROZEN 7 1 3 5,155 1,326 3,316";
    let read = read_scoreboard(&[], None, raw, "defeat", Some("FROZEN")).unwrap();
    assert!(!read.trusted_cells);
    assert_eq!(read.matched.elims, 7);
    assert_eq!(read.matched.mitigation, 3316);
}

#[test]
fn timer_on_the_player_line_does_not_shift_columns() {
    // Row 0 sits at the same height as the hero panel's OBJ CONTEST TIME.
    // The old last-6 window turned
    // "2 0 0 1,105 259 450 ... 00:02" into E 0, A 1105, D 259, DMG 450.
    let raw = "74 FROZEN Giant Troll 2 0 0 1,105 259 450 00:02";
    let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
    assert!(!read.trusted_cells);
    let p = &read.matched;
    assert_eq!(
        (
            p.elims,
            p.assists,
            p.deaths,
            p.damage,
            p.healing,
            p.mitigation
        ),
        (2, 0, 0, 1105, 259, 450)
    );
}

#[test]
fn seven_numbers_and_a_shifted_extra_stat_are_refused() {
    let seven = "FROZEN 8 1 2 3,993 989 1,583 35";
    assert_eq!(
        read_scoreboard(&[], None, seven, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
    // Damage dropped, and an accuracy percent supplies the sixth number.
    let shifted = "FROZEN 8 1 2 989 1,583 ACCURACY 35%";
    assert_eq!(
        read_scoreboard(&[], None, shifted, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
    // Last six of this line are in the ceilings (E1 A2 D30 DMG989 H1583
    // MIT35). Taking the last six would accept it. The first six of the
    // line above fail the deaths ceiling, so that line does not pin this.
    let last_six = "FROZEN 8 1 2 30 989 1,583 35";
    assert_eq!(
        read_scoreboard(&[], None, last_six, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
    // Six numbers and a percent. The run length is fine. The percent
    // after the run is what refuses it.
    let percent = "FROZEN 8 1 2 989 1,583 35%";
    assert_eq!(
        read_scoreboard(&[], None, percent, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
    // A hero-panel label after a correct row. Refusing it is intentional.
    let labeled = "FROZEN 8 1 2 3,993 989 1,583 OBJ CONTEST TIME 00:02";
    assert_eq!(
        read_scoreboard(&[], None, labeled, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
    // The label and the percent are both gone. Six numbers are still
    // accepted, shifted. Nothing in the line says 35 is not mitigation.
    let unlabeled = "FROZEN 8 1 2 989 1,583 35";
    let read = read_scoreboard(&[], None, unlabeled, "unknown", Some("FROZEN")).unwrap();
    assert_eq!(read.matched.elims, 8);
    assert_eq!(read.matched.damage, 989);
    assert_eq!(read.matched.mitigation, 35);
    let overflow = "FROZEN 2 0 0 1,105 259 450 99999999999";
    assert_eq!(
        read_scoreboard(&[], None, overflow, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
}

#[test]
fn a_short_or_empty_name_does_not_read_another_row() {
    let banana = "BANANA 9 9 9 9999 9999 9999";
    assert_eq!(
        read_scoreboard(&[], None, banana, "unknown", Some("Ana")).unwrap_err(),
        ScoreboardMiss::PlayerRowNotFound
    );
    let someone = "SOMEONE 1 2 3 4 5 6";
    assert_eq!(
        read_scoreboard(&[], None, someone, "unknown", Some("")).unwrap_err(),
        ScoreboardMiss::PlayerRowNotFound
    );
    assert_eq!(
        read_scoreboard(&[], None, someone, "unknown", Some("   ")).unwrap_err(),
        ScoreboardMiss::PlayerRowNotFound
    );
}

#[test]
fn a_join_line_does_not_hide_the_stat_line() {
    let raw = "FROZEN joined the game\nFROZEN 8 1 2 3,993 989 1,583";
    let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
    assert!(!read.trusted_cells);
    assert_eq!(read.matched.elims, 8);
    assert_eq!(read.matched.damage, 3993);
    assert_eq!(read.matched.mitigation, 1583);
}

#[test]
fn a_chat_line_does_not_hide_the_stat_line() {
    // The row's own line failed, and a chat line before it has six
    // numbers after the name. Those are not the scoreboard.
    let raw = "[Team] FROZEN: 1 2 3 4 5 6\nFROZEN 8 1 2 3,993 989 1,583";
    let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
    assert_eq!(read.matched.elims, 8);
    assert_eq!(read.matched.assists, 1);
    assert_eq!(read.matched.damage, 3993);
    assert_eq!(read.matched.mitigation, 1583);
}

#[test]
fn suffix_after_a_name_keeps_char_boundaries_when_casefold_widths_cancel() {
    // İ (U+0130) lowercases to two code points. K (U+212A) lowercases to
    // one byte. The old fast path treated equal byte lengths as aligned
    // indexes and sliced into the stats.
    let raw = "İİFROZEN K 12 0 0 100 200 300";
    let read = read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap();
    assert_eq!(read.matched.elims, 12);
    assert_eq!(read.matched.assists, 0);
    assert_eq!(read.matched.mitigation, 300);
}

#[test]
fn shifted_fallback_numbers_are_rejected() {
    // The six numbers the old window emitted, with the name in front.
    // Assists 1105 and deaths 259 are not a scoreboard.
    let raw = "FROZEN 0 1105 259 450 0 2";
    assert_eq!(
        read_scoreboard(&[], None, raw, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
    // A dropped zero plus the timer is not six stats. Do not slide.
    let dropped = "FROZEN 2 1,105 259 450 00:02";
    assert_eq!(
        read_scoreboard(&[], None, dropped, "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
}

#[test]
fn identified_row_with_an_empty_cell_is_unreadable_not_missing() {
    let rows = vec![row(Some("FROZEN"), ["2", "", "0", "1105", "259", "450"])];
    assert_eq!(
        read_scoreboard(&rows, Some(0), "", "unknown", Some("FROZEN")).unwrap_err(),
        ScoreboardMiss::CellsUnreadable
    );
    assert_eq!(
        read_scoreboard(
            &[],
            None,
            "no name on this frame",
            "unknown",
            Some("FROZEN")
        )
        .unwrap_err(),
        ScoreboardMiss::PlayerRowNotFound
    );
}

#[test]
fn explicit_zero_cells_are_a_trusted_read() {
    let rows = vec![row(Some("FROZEN"), ["0", "0", "0", "0", "0", "0"])];
    let read = read_scoreboard(&rows, Some(0), "", "unknown", Some("FROZEN")).unwrap();
    assert!(read.trusted_cells);
    assert_eq!(read.matched.elims, 0);
    assert_eq!(read.matched.assists, 0);
    assert_eq!(read.matched.deaths, 0);
}

#[test]
fn wrecking_ball_text_is_not_kings_row() {
    // "wrecKING ball" used to substring-match the King's Row pattern and
    // fabricate map reads on every Wrecking Ball game.
    assert_eq!(
        match_map_in_text("WRECKING BALL\n31% WEAPON ACCURACY"),
        None
    );
    assert_eq!(
        match_map_in_text("KING'S ROW").as_deref(),
        Some("King's Row")
    );
    // OCR commonly drops the apostrophe — fuzzy pass must still match.
    assert_eq!(
        match_map_in_text("KINGS ROW").as_deref(),
        Some("King's Row")
    );
}

#[test]
fn shion_is_detected() {
    // New S3 Damage hero. Text detection (career panel / scoreboard OCR) is
    // the primary path and needs no portrait reference.
    assert_eq!(match_hero_in_text("SHION").as_deref(), Some("Shion"));
    assert_eq!(guess_role("Shion"), "Damage");
}

#[test]
fn dmon_is_a_tank() {
    // Added 2026-08-18 (WL-5, USER-confirmed role). Career-panel text is
    // the primary path; the portrait reference seeds itself on the first
    // game (see handle_capture's missing-reference auto-collect).
    assert_eq!(match_hero_in_text("D.MON").as_deref(), Some("D.Mon"));
    assert_eq!(guess_role("D.Mon"), "Tank");
    assert_eq!(guess_role("dmon"), "Tank");
    assert_eq!(guess_role("D.Va"), "Tank");
}

/// Season 5 (2026-10-06): Doctrine is Support, Sombra moved to Support.
/// An unknown name is Unknown, not Damage. Roadhog stays Tank.
#[test]
fn season5_roles_and_unknown_names_are_not_damage() {
    assert_eq!(guess_role("Doctrine"), "Support");
    assert_eq!(guess_role("doctrine"), "Support");
    assert_eq!(guess_role("Sombra"), "Support");
    assert_eq!(guess_role("sombra"), "Support");
    assert_eq!(guess_role("Roadhog"), "Tank");
    assert_eq!(guess_role("Wrecking Ball"), "Tank");
    assert_eq!(guess_role("wrecking-ball"), "Tank");
    assert_eq!(guess_role("NotAHero"), "Unknown");
    assert_eq!(guess_role(""), "Unknown");
    assert_ne!(guess_role("NotAHero"), "Damage");
}

/// Site role lookup and the role stamped on a captured game stay aligned.
/// A stored game keeps whatever was captured, including pre-Season 5 Sombra
/// games that were Damage.
#[test]
fn role_for_hero_name_matches_guess_role() {
    let pins = [
        ("D.Mon", "Tank"),
        ("d.mon", "Tank"),
        ("dmon", "Tank"),
        ("Jetpack Cat", "Support"),
        ("Doctrine", "Support"),
        ("Sombra", "Support"),
    ];
    for (name, role) in pins {
        assert_eq!(guess_role(name), role, "tracker {name}");
        assert_eq!(
            scuffed_types::role_for_hero_name(name)
                .map(|r| r.to_string())
                .as_deref(),
            Some(role),
            "site {name}"
        );
    }
    for name in scuffed_types::HEROES {
        let site = scuffed_types::role_for_hero_name(name)
            .map(|role| role.to_string())
            .unwrap_or_else(|| panic!("{name} has no site role"));
        assert_eq!(site, guess_role(name), "{name}");
    }
}

#[test]
fn recent_maps_are_detected() {
    // Neon Junction (Hybrid, S3) and Aatlis (Flashpoint, S17) — both were
    // missing from the canonical MAPS list and never got stored.
    assert_eq!(
        match_map_in_text("NEON JUNCTION").as_deref(),
        Some("Neon Junction")
    );
    assert_eq!(match_map_in_text("AATLIS").as_deref(), Some("Aatlis"));
}

#[test]
fn accented_and_unaccented_map_names_canonicalize() {
    // Live career/OCR names arrive both with and without Portuguese
    // accents. Canonical store form stays the unaccented MAPS display name.
    assert_eq!(match_map_in_text("Paraíso").as_deref(), Some("Paraiso"));
    assert_eq!(match_map_in_text("Paraiso").as_deref(), Some("Paraiso"));
    assert_eq!(match_map_in_text("Esperança").as_deref(), Some("Esperanca"));
    assert_eq!(match_map_in_text("Esperanca").as_deref(), Some("Esperanca"));
    assert_eq!(
        match_map_in_text("Neon Junction").as_deref(),
        Some("Neon Junction")
    );
    assert_eq!(
        match_map_in_text("NEON JUNCTION").as_deref(),
        Some("Neon Junction")
    );
}

#[test]
fn scoreboard_check_accepts_real_rows() {
    let rows: Vec<RowOcrResult> = (0..10).map(|_| valid_row("X")).collect();
    assert!(looks_like_scoreboard(&rows));
    // Early-game all-zero rows are still clean cells.
    let zeros: Vec<RowOcrResult> = (0..10)
        .map(|_| row(Some("X"), ["0", "0", "0", "0", "0", "0"]))
        .collect();
    assert!(looks_like_scoreboard(&zeros));
    // A poorly-OCR'd but real scoreboard: only 3 of 10 rows readable.
    let mut sparse: Vec<RowOcrResult> = (0..3).map(|_| valid_row("X")).collect();
    sparse.extend((0..7).map(|_| garbage_row()));
    assert!(looks_like_scoreboard(&sparse));
}

#[test]
fn scoreboard_check_rejects_garbage_frames() {
    assert!(!looks_like_scoreboard(&[]));
    let rows: Vec<RowOcrResult> = (0..10).map(|_| garbage_row()).collect();
    assert!(!looks_like_scoreboard(&rows));
    // 1-2 valid-looking rows among garbage (e.g. the rank screen flukes
    // digit cells, a desktop frame with a number column) is not enough.
    let mut mixed: Vec<RowOcrResult> = (0..2).map(|_| valid_row("X")).collect();
    mixed.extend((0..8).map(|_| garbage_row()));
    assert!(!looks_like_scoreboard(&mixed));
}

#[test]
fn known_map_names_keep_the_table_spellings() {
    let names = known_map_names();
    assert_eq!(names.first().copied(), Some("King's Row"));
    assert_eq!(
        names.iter().filter(|name| **name == "Eichenwalde").count(),
        1
    );
    assert!(names.contains(&"Paraiso"));
    assert!(names.contains(&"Esperanca"));
    assert!(!names.contains(&"Paraíso"));
    assert!(!names.contains(&"Esperança"));
    assert!(names.contains(&"Watchpoint: Grímsvötn"));
    assert!(names.contains(&"Château Guillard"));
    assert!(names.contains(&"King's Row"));
    for name in &names {
        assert!(map_is_known(name), "{name}");
    }
}

#[test]
fn blank_or_unknown_map_mode_and_hero_are_suspect() {
    assert_eq!(review_suspect_fields("", "", "Ana"), vec!["map", "mode"]);
    assert_eq!(
        review_suspect_fields("Not a map", "", "Ana"),
        vec!["map", "mode"]
    );
    assert_eq!(
        review_suspect_fields("Not a map", "Escort", "Ana"),
        vec!["map"]
    );
    assert_eq!(
        review_suspect_fields("Busan", "", "Ana"),
        Vec::<&str>::new()
    );
    assert_eq!(review_suspect_fields("Busan", "", "Unknown"), vec!["hero"]);
    assert_eq!(review_suspect_fields("Busan", "", "unknown"), vec!["hero"]);
    assert_eq!(review_suspect_fields("Busan", "", "  "), vec!["hero"]);
    assert!(review_suspect_fields("Busan", "", "Ana").is_empty());
    assert_eq!(uploaded_game_mode("Busan", ""), "Control");
}

#[test]
fn text_map_sets_game_mode_on_the_parsed_row() {
    let rows = vec![valid_row("FROZEN")];
    let parsed = parse_scoreboard_cells(
        &rows,
        Some(0),
        "DORADO\nFROZEN 5 3 2 4,316 1,200 899",
        "victory",
        Some("FROZEN"),
    )
    .unwrap();
    assert_eq!(parsed.map_name, "Dorado");
    assert_eq!(parsed.game_mode, "Escort");
    assert_eq!(stored_game_mode("Busan"), "Control");
    assert_eq!(stored_game_mode("Château Guillard"), "Deathmatch");
}

#[test]
fn both_teams_need_a_stat_row_before_an_alias_is_trusted() {
    let team: Vec<_> = (0..10).map(|_| valid_row("X")).collect();
    assert!(both_teams_have_stats(&team, 5));
    let one_team: Vec<_> = (0..4).map(|_| valid_row("X")).collect();
    assert!(
        !both_teams_have_stats(&one_team, 5),
        "a 4-player co-op board has no enemy half"
    );
    let mut enemy_blank: Vec<_> = team.into_iter().take(5).collect();
    enemy_blank.extend((0..5).map(|_| garbage_row()));
    assert!(!both_teams_have_stats(&enemy_blank, 5));
}

#[test]
fn reads_result_word_from_header_lines() {
    assert_eq!(
        outcome_from_board_header("~ Defeat\n\u{2014} J | J |"),
        MatchOutcome::Defeat
    );
    assert_eq!(
        outcome_from_board_header("\n  VICTORY   RIALTO\nE A D DMG"),
        MatchOutcome::Victory
    );
    assert_eq!(outcome_from_board_header("Draw"), MatchOutcome::Draw);
}

#[test]
fn ignores_words_deeper_than_the_header_and_substrings() {
    // Chat / names below the header never decide a game.
    assert_eq!(
        outcome_from_board_header("E A D DMG\nFROZEN 12 3 4\nSOOT: ez victory"),
        MatchOutcome::Unknown
    );
    // Whole word only: "VICTORYLANE" is a name, not a result.
    assert_eq!(
        outcome_from_board_header("VICTORYLANE 3 4"),
        MatchOutcome::Unknown
    );
    assert_eq!(outcome_from_board_header(""), MatchOutcome::Unknown);
}
