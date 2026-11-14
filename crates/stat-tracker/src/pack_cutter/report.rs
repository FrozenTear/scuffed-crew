//! Held-out counts. The gate compares the learned candidate to the baseline
//! pack. A class the baseline pack does not contain is printed and is not
//! part of the gate.

use std::fmt::Write as _;

#[derive(Clone, Copy, Default)]
pub struct Counts {
    pub correct: usize,
    pub wrong: usize,
    pub unread: usize,
}

impl Counts {
    pub fn add(&mut self, outcome: Outcome) {
        match outcome {
            Outcome::Correct => self.correct += 1,
            Outcome::Wrong => self.wrong += 1,
            Outcome::Unread => self.unread += 1,
        }
    }
}

#[derive(Clone, Copy)]
pub enum Outcome {
    Correct,
    Wrong,
    Unread,
}

pub struct ScoreRow {
    pub label: String,
    pub baseline: Counts,
    pub learned: Counts,
    pub packed: Counts,
    /// False for a class the baseline pack does not contain.
    pub gated: bool,
}

pub struct EvalDone {
    pub report: String,
    pub pass: bool,
}

pub fn finish(
    set_name: &str,
    frames: usize,
    scored: usize,
    skipped: usize,
    rows: &[ScoreRow],
) -> EvalDone {
    let pass = !regressed(rows);
    EvalDone {
        report: render(set_name, frames, scored, skipped, rows, pass),
        pass,
    }
}

fn regressed(rows: &[ScoreRow]) -> bool {
    rows.iter().filter(|row| row.gated).any(|row| {
        row.learned.wrong > row.baseline.wrong || row.learned.correct < row.baseline.correct
    })
}

fn render(
    set_name: &str,
    frames: usize,
    scored: usize,
    skipped: usize,
    rows: &[ScoreRow],
    pass: bool,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "set: {set_name}");
    let _ = writeln!(out, "frames: {frames} scored: {scored} skipped: {skipped}");
    for row in rows {
        let _ = writeln!(out, "{}", row.label);
        write_counts(&mut out, "baseline", row.baseline);
        write_counts(&mut out, "candidate", row.learned);
        write_counts(&mut out, "candidate pack", row.packed);
    }
    let _ = writeln!(out, "overall");
    write_counts(&mut out, "baseline", total(rows, |row| row.baseline));
    write_counts(&mut out, "candidate", total(rows, |row| row.learned));
    write_counts(&mut out, "candidate pack", total(rows, |row| row.packed));
    let _ = writeln!(out, "result: {}", if pass { "pass" } else { "fail" });
    out
}

fn write_counts(out: &mut String, name: &str, counts: Counts) {
    let _ = writeln!(
        out,
        "{name}: {} / {} / {}",
        counts.correct, counts.wrong, counts.unread
    );
}

fn total(rows: &[ScoreRow], pick: impl Fn(&ScoreRow) -> Counts) -> Counts {
    let mut sum = Counts::default();
    for row in rows {
        let counts = pick(row);
        sum.correct += counts.correct;
        sum.wrong += counts.wrong;
        sum.unread += counts.unread;
    }
    sum
}
