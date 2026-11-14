//! Scores for one labelled set. The pass/fail gate compares the candidate
//! pack to the baseline pack on every labelled row: no new wrong, and no
//! frame that was correct on the baseline and is unread or wrong on the
//! candidate. Leave-one-out learning is printed as generalisation and is
//! not the gate.

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

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Outcome {
    Correct,
    Wrong,
    Unread,
}

/// One frame, scored by the baseline pack, the leave-one-out learner, and
/// the candidate pack.
#[derive(Clone)]
pub struct Item {
    /// Rank `game/file`, or the queue-title label path.
    pub frame: String,
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
    /// Frames whose label exercises this row.
    pub labelled: usize,
    pub items: Vec<Item>,
}

pub struct EvalDone {
    pub report: String,
    pub pass: bool,
}

/// Everything `finish` prints, borrowed from the caller.
pub struct Inputs<'a> {
    pub set_name: &'a str,
    pub frames: usize,
    pub scored: usize,
    pub skipped: usize,
    pub rows: &'a [ScoreRow],
    pub dropped: &'a [String],
    pub allowed: &'a [String],
    pub unverified: &'a [String],
    pub changed: &'a [super::util::CountChange],
    pub class_folds: &'a [(String, usize)],
    pub candidate_sha: &'a str,
}

pub fn finish(inp: &Inputs<'_>) -> EvalDone {
    let pass = inp.dropped.is_empty() && inp.unverified.is_empty() && !regressed(inp.rows);
    EvalDone {
        report: render(inp, pass),
        pass,
    }
}

fn regressed(rows: &[ScoreRow]) -> bool {
    rows.iter()
        .any(|row| row.items.iter().any(Item::pack_regressed))
}

fn render(inp: &Inputs<'_>, pass: bool) -> String {
    let mut out = String::new();
    let _ = writeln!(out, "set: {}", inp.set_name);
    let _ = writeln!(
        out,
        "frames: {} scored: {} skipped: {}",
        inp.frames, inp.scored, inp.skipped
    );
    if !inp.candidate_sha.is_empty() {
        let _ = writeln!(
            out,
            "candidate templates.json sha256: {}",
            inp.candidate_sha
        );
    }
    if !inp.dropped.is_empty() {
        let _ = writeln!(out, "dropped classes: {}", inp.dropped.join(", "));
    }
    if !inp.allowed.is_empty() {
        let _ = writeln!(out, "allowed drops: {}", inp.allowed.join(", "));
    }
    if !inp.unverified.is_empty() {
        let _ = writeln!(out, "unverified classes: {}", inp.unverified.join(", "));
    }
    for (name, before, after) in inp.changed {
        let _ = writeln!(out, "changed class {name}: n {before} -> {after}");
    }
    for row in inp.rows {
        let _ = writeln!(out, "{}", row.label);
        let _ = writeln!(out, "folds: {}", row.folds);
        let _ = writeln!(out, "labelled: {}", row.labelled);
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
        let bad: Vec<&Item> = row
            .items
            .iter()
            .filter(|item| Item::pack_regressed(item))
            .collect();
        let _ = writeln!(out, "regressed: {}", bad.len());
        if !bad.is_empty() {
            let ids: Vec<&str> = bad.iter().map(|item| item.frame.as_str()).collect();
            let _ = writeln!(out, "regressed frames: {}", ids.join(", "));
        }
    }
    let _ = writeln!(out, "overall");
    let baseline = total(inp.rows, |item| item.baseline);
    let candidate = total(inp.rows, |item| item.packed);
    let learned = total(inp.rows, |item| item.learned);
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
    for (key, n) in inp.class_folds {
        let _ = writeln!(out, "template class {key}");
        let _ = writeln!(out, "folds: {n}");
    }
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
