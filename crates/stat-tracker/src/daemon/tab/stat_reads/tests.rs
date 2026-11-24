//! Tests of the per-board stat reads (unsure rule).

use super::*;

fn field(name: &str, value: Option<u32>, confidence: f32, suspect: bool) -> shadow::FieldRead {
    shadow::FieldRead {
        name: name.into(),
        value: value.map(shadow::Value::Int),
        confidence,
        suspect,
    }
}

fn board(fields: Vec<shadow::FieldRead>) -> shadow::BoardRead {
    shadow::BoardRead {
        status: shadow::BoardStatus::Read,
        team_size: Some(5),
        fields,
        elapsed_ms: 1,
        header_dimmed: false,
    }
}

const OWN: reader_apply::OwnRow = reader_apply::OwnRow::Identified {
    index: 2,
    team_size: 5,
};

/// The value is the reader's own number, sure or not; a suspect cell or
/// a held column is unsure; an unread placeholder pushes nothing.
#[test]
fn a_board_gives_the_readers_own_reads() {
    let b = board(vec![
        field("r2.e", Some(7), 0.9, false),
        field("r2.a", Some(3), 0.4, true),
        field("r2.d", None, 0.0, true),
        field("r2.dmg", None, 0.3, true),
        field("r2.h", Some(500), 0.8, false),
    ]);
    let mut held = [false; 6];
    held[4] = true;
    let t = board_reads(&b, OWN, held, false, true);
    assert_eq!(t[0], Some(StatRead::sure(7, 0.9)));
    assert_eq!(t[1], Some(StatRead::unsure(Some(3), 0.4)));
    assert_eq!(t[2], None, "never attempted");
    assert_eq!(t[3], Some(StatRead::unsure(None, 0.3)));
    assert_eq!(t[4], Some(StatRead::unsure(Some(500), 0.8)));
    assert_eq!(t[5], None, "no field");
}

/// No identified own row: nothing is pushed. A suspect own row, a
/// dimmed header or untrusted cells make every read unsure.
#[test]
fn row_level_doubt() {
    let mut b = board(vec![field("r2.e", Some(7), 0.9, false)]);
    let none = board_reads(&b, reader_apply::OwnRow::Uncertain, [false; 6], false, true);
    assert_eq!(none, [None; 6]);
    assert!(
        board_reads(&b, OWN, [false; 6], true, true)[0]
            .unwrap()
            .unsure
    );
    assert!(
        board_reads(&b, OWN, [false; 6], false, false)[0]
            .unwrap()
            .unsure
    );
    b.header_dimmed = true;
    assert!(
        board_reads(&b, OWN, [false; 6], false, true)[0]
            .unwrap()
            .unsure
    );
}

/// A gate hold, a low-trust capture or a suspect stored field makes
/// the read unsure.
#[test]
fn the_gate_marks_reads_unsure() {
    let fresh = || [Some(StatRead::sure(1, 0.9)); 6];
    let mut gate = capture_gate::GateOutcome {
        accepted: Counters::default(),
        state: capture_gate::GateState::default(),
        holds: vec![capture_gate::Hold {
            col: 1,
            kind: capture_gate::HoldKind::Unread,
            raw: 0,
            held: 0,
        }],
        unlatches: Vec::new(),
    };
    let mut t = fresh();
    mark_gate(&mut t, &gate, &["dmg".to_string()]);
    let unsure: Vec<bool> = t.iter().map(|r| r.unwrap().unsure).collect();
    assert_eq!(unsure, vec![false, true, false, true, false, false]);
    gate.state.low_trust = true;
    let mut t = fresh();
    mark_gate(&mut t, &gate, &[]);
    assert!(t.iter().all(|r| r.unwrap().unsure));
}
