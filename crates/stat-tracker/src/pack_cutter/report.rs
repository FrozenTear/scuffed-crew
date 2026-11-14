//! Scores for one labelled set. The pass/fail gate compares the candidate
//! pack to the baseline pack on the same frames: no new wrong, and no frame
//! that was correct on the baseline and is unread or wrong on the candidate.
//! Leave-one-out learning is printed as generalisation and is not the gate.
//! A class the baseline pack does not contain is reported and is not gated.

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

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Correct,
    Wrong,
    Unread,
}

/// One frame, scored by the baseline pack, the leave-one-out learner, and
/// the candidate pack.
#[derive(Clone, Copy)]
pub struct Item {
    pub baseline: Outcome,
    pub learned: Outcome,
    pub packed: Outcome,
}

impl Item {
    pub fn counts(items: &[Item], pick: impl Fn(&Item) -> Outcome) -> Counts {
        let mut counts = Counts::default();
        for item in items {
            counts.add(pick(item));
        }
        counts
    }

    /// Candidate pack versus baseline pack. A correct that becomes unread or
    /// wrong is a regression, and so is any new wrong.
    pub fn pack_regressed(item: &Item) -> bool {
        let lost_correct = item.baseline == Outcome::Correct && item.packed != Outcome::Correct;
        let new_wrong = item.packed == Outcome::Wrong && item.baseline != Outcome::Wrong;
        lost_correct || new_wrong
    }
}

pub struct ScoreRow {
    pub label: String,
    pub folds: usize,
    pub items: Vec<Item>,
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
    dropped: &[String],
) -> EvalDone {
    let pass = dropped.is_empty() && !regressed(rows);
    EvalDone {
        report: render(set_name, frames, scored, skipped, rows, dropped, pass),
        pass,
    }
}

fn regressed(rows: &[ScoreRow]) -> bool {
    rows.iter()
        .filter(|row| row.gated)
        .any(|row| row.items.iter().any(Item::pack_regressed))
}

fn render(
    set_name: &str,
    frames: usize,
    scored: usize,
    skipped: usize,
    rows: &[ScoreRow],
    dropped: &[String],
    pass: bool,
) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "set: {set_name}");
    let _ = writeln!(out, "frames: {frames} scored: {scored} skipped: {skipped}");
    if !dropped.is_empty() {
        let _ = writeln!(out, "dropped classes: {}", dropped.join(", "));
    }
    for row in rows {
        let _ = writeln!(out, "{}", row.label);
        let _ = writeln!(out, "folds: {}", row.folds);
        let baseline = Item::counts(&row.items, |item| item.baseline);
        let candidate = Item::counts(&row.items, |item| item.packed);
        let learned = Item::counts(&row.items, |item| item.learned);
        write_counts(&mut out, "baseline", baseline);
        write_counts(&mut out, "candidate", candidate);
        let _ = writeln!(
            out,
            "generalisation: learned {} / {} / {}, baseline {} / {} / {}",
            learned.correct,
            learned.wrong,
            learned.unread,
            baseline.correct,
            baseline.wrong,
            baseline.unread
        );
    }
    let _ = writeln!(out, "overall");
    let baseline = total(rows, |item| item.baseline);
    let candidate = total(rows, |item| item.packed);
    let learned = total(rows, |item| item.learned);
    write_counts(&mut out, "baseline", baseline);
    write_counts(&mut out, "candidate", candidate);
    let _ = writeln!(
        out,
        "generalisation: learned {} / {} / {}, baseline {} / {} / {}",
        learned.correct,
        learned.wrong,
        learned.unread,
        baseline.correct,
        baseline.wrong,
        baseline.unread
    );
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

fn total(rows: &[ScoreRow], pick: impl Fn(&Item) -> Outcome) -> Counts {
    let mut sum = Counts::default();
    for row in rows {
        let counts = Item::counts(&row.items, &pick);
        sum.correct += counts.correct;
        sum.wrong += counts.wrong;
        sum.unread += counts.unread;
    }
    sum
}
