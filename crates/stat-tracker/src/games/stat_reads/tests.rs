//! The per-game read log, its fresh-match hold, the verdicts on the rows
//! and the confidence line.

use super::*;
use crate::unsure_rule::Reason;

fn tab(read: StatRead) -> TabReads {
    [Some(read); 6]
}

fn values(list: &[SavedRead]) -> Vec<Option<u32>> {
    list.iter().map(|r| r.value).collect()
}

/// A taken board pushes one read per stat; a `None` stat pushes nothing.
#[test]
fn a_board_pushes_one_read_per_read_stat() {
    let mut log = StatReadLog::default();
    let mut t = tab(StatRead::sure(3, 0.9));
    t[2] = None;
    log.push("g1", false, &t);
    log.push("g1", false, &t);
    assert_eq!(values(&log.reads[0]), vec![Some(3), Some(3)]);
    assert!(
        log.reads[2].is_empty(),
        "an unattempted cell pushes nothing"
    );
}

/// A new game clears the old game's reads.
#[test]
fn a_new_game_clears_the_reads() {
    let mut log = StatReadLog::default();
    log.push("g1", false, &tab(StatRead::unsure(Some(9), 0.2)));
    log.push("g2", false, &tab(StatRead::sure(1, 0.8)));
    assert_eq!(log.session_id, "g2");
    assert_eq!(values(&log.reads[0]), vec![Some(1)]);
    assert_eq!(log.confirm("g1", false), [None; 6], "g1's log is gone");
}

/// A held fresh-match board pushes nothing onto the current game, and its
/// reads open the next game when the split carries the board.
#[test]
fn a_fresh_match_hold_carries_its_reads_to_the_next_game() {
    let mut log = StatReadLog::default();
    log.push("g1", false, &tab(StatRead::sure(20, 0.9)));
    log.hold(&tab(StatRead::unsure(Some(2), 0.3)), true);
    assert_eq!(values(&log.reads[0]), vec![Some(20)], "not on g1");
    log.push("g1", false, &tab(StatRead::sure(21, 0.9)));
    assert_eq!(values(&log.reads[0]), vec![Some(20), Some(21)]);
    log.push("g2", true, &tab(StatRead::sure(3, 0.9)));
    assert_eq!(values(&log.reads[0]), vec![Some(2), Some(3)]);
    assert!(log.held[0].is_empty());
}

/// A hold that keeps no board, or a new game the board is not carried
/// into, drops the held reads.
#[test]
fn a_dropped_hold_drops_its_reads() {
    let mut log = StatReadLog::default();
    log.push("g1", false, &tab(StatRead::sure(20, 0.9)));
    log.hold(&tab(StatRead::unsure(Some(2), 0.3)), true);
    log.hold(&tab(StatRead::unsure(Some(4), 0.3)), false);
    assert!(
        log.held[0].is_empty(),
        "a hold with no board keeps no reads"
    );
    log.hold(&tab(StatRead::unsure(Some(2), 0.3)), true);
    log.push("g2", false, &tab(StatRead::sure(3, 0.9)));
    assert_eq!(values(&log.reads[0]), vec![Some(3)]);
}

/// Only Unsure flags: three unsettled reads in a row live, an unconfirmed
/// stat at game end. A stat with no reads gets no verdict.
#[test]
fn verdicts_follow_the_rule() {
    let mut log = StatReadLog::default();
    let mut t = tab(StatRead::sure(5, 0.9));
    t[3] = Some(StatRead::unsure(Some(1200), 0.4));
    t[5] = None;
    log.push("g", false, &t);
    let live = log.confirm("g", false);
    assert_eq!(live[3].unwrap().verdict, Verdict::Unsettled);
    assert_eq!(live[0].unwrap().verdict, Verdict::Confirmed);
    assert_eq!(live[5], None);
    let end = log.confirm("g", true);
    assert_eq!(end[3].unwrap().verdict, Verdict::Unsure);
    log.push("g", false, &t);
    log.push("g", false, &t);
    let live = log.confirm("g", false);
    assert_eq!(live[3].unwrap().reason, Reason::StillUnsure);
}

/// The rows of the log's game lose stat flags the rule does not back and
/// gain the Unsure ones; other fields, stats with no verdict and other
/// games keep their flags.
#[test]
fn verdicts_replace_the_stat_flags_of_their_game_only() {
    let mut log = StatReadLog::default();
    let mut t = tab(StatRead::sure(5, 0.9));
    t[3] = Some(StatRead::unsure(Some(1200), 0.4));
    t[5] = None;
    log.push("g", false, &t);
    let v = log.confirm("g", true);
    let row = |sid: &str| {
        let mut m: PersonalMatch = serde_json::from_value(serde_json::json!({
            "hero": "Ana", "map_name": "", "game_mode": "", "role": "Support",
            "outcome": "", "elims": 5, "deaths": 5, "assists": 5, "damage": 900,
            "healing": 5, "mitigation": 5, "played_at": "2026-11-13T10:00:00Z",
            "session_id": sid,
        }))
        .unwrap();
        m.suspect_fields = vec!["r3.e".into(), "map".into(), "mit".into()];
        m
    };
    let mut rows = vec![row("g"), row("old")];
    apply_verdicts(&mut rows, "g", &v);
    assert_eq!(rows[0].suspect_fields, vec!["map", "mit", "dmg"]);
    assert_eq!(rows[1].suspect_fields, vec!["r3.e", "map", "mit"]);
}

/// The line gives the reader's confidence in the judged value, the read
/// number when it is not the stored one, and "No read" for no read.
#[test]
fn the_confidence_line() {
    let c = |value, confidence| StatConfirm {
        verdict: Verdict::Unsure,
        value,
        confidence,
        reason: Reason::UnsureAtEnd,
        unsettled_reads: 1,
    };
    assert_eq!(
        confidence_line(&c(Some(1200), Some(0.42)), 900),
        "Read 1200 at 42%"
    );
    assert_eq!(
        confidence_line(&c(Some(900), Some(0.42)), 900),
        "Read at 42%"
    );
    assert_eq!(confidence_line(&c(Some(900), None), 900), "No read");
    assert_eq!(confidence_line(&c(None, None), 0), "No read");
    assert_eq!(confidence_line(&c(Some(0), Some(0.0)), 0), "Read at 0%");
}

/// The log survives a write and a read; a broken file is an empty log
/// and the next update replaces it.
#[test]
fn the_log_file_round_trips_and_a_broken_one_is_empty() {
    let dir = tempfile::tempdir().unwrap();
    update(dir.path(), |l| {
        l.push("g", false, &tab(StatRead::sure(5, 0.5)))
    });
    assert_eq!(values(&load(dir.path()).reads[1]), vec![Some(5)]);
    std::fs::write(dir.path().join(FILE), b"{not json").unwrap();
    assert_eq!(load(dir.path()), StatReadLog::default());
    update(dir.path(), |l| {
        l.push("g", false, &tab(StatRead::sure(6, 0.5)))
    });
    assert_eq!(values(&load(dir.path()).reads[1]), vec![Some(6)]);
}
