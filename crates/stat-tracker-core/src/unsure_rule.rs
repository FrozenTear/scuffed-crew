//! The Unsure confirm rule (alpha.3 item 5, issue #619).
//!
//! The digit readers mark a stat cell unsure on a single read. Shown as is,
//! that flag comes up on live reads that settle a few Tabs later, when the
//! number turns out right. This rule looks at all reads of ONE stat cell of
//! one game, in order, and says whether the cell's value has settled:
//!
//! * [`Verdict::Confirmed`]: the latest read is sure and does not go below
//!   the value already confirmed, or an unsure read agrees with that value.
//! * [`Verdict::Unsettled`]: the cell has had unsure, empty or contradicting
//!   reads since its last confirmed value, but fewer than
//!   [`UNSURE_AFTER_READS`] of them. A single bad Tab lands here, not on
//!   Unsure.
//! * [`Verdict::Unsure`]: [`UNSURE_AFTER_READS`] or more such reads in a
//!   row, or the game is over and the cell is not confirmed.
//!
//! The six stats only grow within a game, so a sure read at or above the
//! confirmed value confirms it, and a read below it contradicts it. A known
//! reset ([`StatRead::reset`]) starts the cell over.
//!
//! **One read** is one taken board (one Tab the tracker took), not one
//! frame and not one retry of the same Tab. A taken board pushes one read
//! per own-row stat even when it is not stored (a fresh-match hold, or a
//! stat column with no sure read yet): the reader still read the cell. A
//! Tab whose own row is not identified, or whose cell was never attempted,
//! pushes nothing. [`UNSURE_AFTER_READS`] therefore counts Tabs, not time
//! or board progress; the tracker's debounce after a taken board keeps
//! reads a few seconds apart.
//!
//! Known limits (by design, see the PR's Risks):
//! * A sure read above the real value (the ghost leading "9" that the
//!   capture gate's rate and jump caps hold) is confirmed, and every right
//!   read below it after that is unsettled, then unsure. The rule is safe
//!   only when that gate hold reaches [`StatRead::unsure`].
//! * A cell that flaps (10, 12, 10, 12, or an unsure value that keeps
//!   coming back to the confirmed one) is re-confirmed each time the
//!   confirmed value is read again, so "in a row" restarts and it never
//!   flags live. At game end the last read decides.
//!
//! The rule is pure and cheap (one pass, no allocation). It does not change
//! any read, the readers, the capture gate or what is stored or uploaded:
//! when to show the flag is the tracker's call, built on this verdict.

/// Unsettled reads in a row (unsure, empty, or going down) before the
/// verdict becomes [`Verdict::Unsure`] during a game.
pub const UNSURE_AFTER_READS: u32 = 3;

/// One read of one stat cell, in game order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatRead {
    /// The digit reader's own number for this cell on this Tab, taken
    /// before the capture gate (`reader_apply::own_row_guesses` in the
    /// tracker), sure or not. `None` when the reader gave no number.
    ///
    /// It is never the stored row, `raw_guess` (which falls back to the
    /// held value) or the kit's `ocr_parsed`: those back-fill a held cell
    /// with the game's last accepted value, which would make every gate
    /// hold agree with the confirmed value and never flag.
    pub value: Option<u32>,
    /// The reader's confidence in this read, 0..=1 (cv: the cell's
    /// calibrated confidence; ocr-v1: Tesseract's 0..100 over 100).
    pub confidence: f32,
    /// The reader or the capture gate did not vouch for this read. True
    /// when any of these holds for the cell on this Tab:
    /// * the board field is `suspect` (a `CellTrust::Held` cell);
    /// * the capture gate held the column (`GateOutcome::holds`), which
    ///   includes the ghost "9" inflation its rate and jump caps catch;
    /// * the gate lowered or revised the field;
    /// * the own row is suspect or the header is dimmed
    ///   (`mark_own_row_suspect`);
    /// * the capture is low-trust.
    pub unsure: bool,
    /// The board was known to start over before this read (a new game on
    /// the same session): earlier reads no longer count.
    pub reset: bool,
}

impl StatRead {
    /// A read the reader vouched for.
    pub fn sure(value: u32, confidence: f32) -> Self {
        Self {
            value: Some(value),
            confidence,
            unsure: false,
            reset: false,
        }
    }

    /// A read the reader did not vouch for (`None`: no number at all).
    pub fn unsure(value: Option<u32>, confidence: f32) -> Self {
        Self {
            value,
            confidence,
            unsure: true,
            reset: false,
        }
    }
}

/// Whether a stat cell's value has settled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Verdict {
    Confirmed,
    Unsettled,
    Unsure,
}

/// Why the verdict is what it is, as a short machine name ([`Reason::as_str`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reason {
    /// No reads at all (an empty slice). A reset rides on a read, so it
    /// never leads here.
    NoReads,
    /// The latest read is sure and not below the confirmed value.
    SureRead,
    /// The latest read is unsure but equals the confirmed value.
    AgreesWithSure,
    /// Fewer than [`UNSURE_AFTER_READS`] unsettled reads since the last
    /// confirmed value.
    Pending,
    /// [`UNSURE_AFTER_READS`] or more unsettled reads in a row.
    StillUnsure,
    /// The game is over and the cell is not confirmed.
    UnsureAtEnd,
}

impl Reason {
    pub fn as_str(self) -> &'static str {
        match self {
            Reason::NoReads => "no_reads",
            Reason::SureRead => "sure_read",
            Reason::AgreesWithSure => "agrees_with_sure",
            Reason::Pending => "pending",
            Reason::StillUnsure => "still_unsure",
            Reason::UnsureAtEnd => "unsure_at_end",
        }
    }
}

/// The rule's answer for one stat cell.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatConfirm {
    pub verdict: Verdict,
    /// The value the verdict is about: the latest read's number when it
    /// has one, else the confirmed value. `None` when neither exists.
    pub value: Option<u32>,
    /// The reader's confidence in `value`, 0..=1. It is the reader's own
    /// number (near miss vs bad read, #619), not a probability that `value`
    /// is right, and not a confidence in the stored number.
    ///
    /// * Confirmed: the confidence of the latest sure read that confirmed
    ///   `value`, raised by any later unsure read that agreed with it. A
    ///   later sure read of the same value replaces it (it may go down):
    ///   the latest sure read is the freshest.
    /// * Otherwise: the highest confidence among the reads since the last
    ///   confirmed value (the current unsettled run) that read `value`,
    ///   wherever they sit in the run. `None` when no read in the run read
    ///   `value` (no reads, or the last read was empty and `value` is the
    ///   kept confirmed value).
    pub confidence: Option<f32>,
    pub reason: Reason,
    /// Unsettled reads in a row since the last confirmed value.
    pub unsettled_reads: u32,
}

/// The verdict for one stat cell from all its reads so far, oldest first.
/// `game_over` is true once the game has ended (the last read is the final
/// board): a cell not confirmed then is [`Verdict::Unsure`].
pub fn confirm_stat(reads: &[StatRead], game_over: bool) -> StatConfirm {
    // The confirmed value and the confidence of the read that confirmed it.
    let mut confirmed: Option<(u32, f32)> = None;
    let mut unsettled: u32 = 0;
    let mut last: Option<u32> = None;
    // Where the current unsettled run starts (reads after the last read
    // that confirmed or agreed, or after the last reset).
    let mut run_start = 0;
    let mut reason = Reason::NoReads;
    for (i, read) in reads.iter().enumerate() {
        if read.reset {
            confirmed = None;
            unsettled = 0;
            run_start = i;
        }
        let conf = clamp01(read.confidence);
        let at_or_above = |v: u32| confirmed.is_none_or(|(c, _)| v >= c);
        match read.value {
            Some(v) if !read.unsure && at_or_above(v) => {
                confirmed = Some((v, conf));
                unsettled = 0;
                run_start = i + 1;
                reason = Reason::SureRead;
            }
            Some(v) if read.unsure && confirmed.is_some_and(|(c, _)| c == v) => {
                if let Some((_, c)) = confirmed.as_mut() {
                    *c = c.max(conf);
                }
                unsettled = 0;
                run_start = i + 1;
                reason = Reason::AgreesWithSure;
            }
            _ => {
                unsettled = unsettled.saturating_add(1);
                reason = Reason::Pending;
            }
        }
        last = read.value;
    }
    let (verdict, reason) = match (unsettled, reason) {
        (_, Reason::NoReads) if game_over => (Verdict::Unsure, Reason::UnsureAtEnd),
        (_, Reason::NoReads) => (Verdict::Unsettled, Reason::NoReads),
        (0, r) => (Verdict::Confirmed, r),
        _ if game_over => (Verdict::Unsure, Reason::UnsureAtEnd),
        (n, _) if n >= UNSURE_AFTER_READS => (Verdict::Unsure, Reason::StillUnsure),
        _ => (Verdict::Unsettled, Reason::Pending),
    };
    let (value, confidence) = if verdict == Verdict::Confirmed {
        let (v, c) = confirmed.expect("confirmed verdict has a value");
        (Some(v), Some(c))
    } else {
        let value = last.or(confirmed.map(|(v, _)| v));
        let best = reads[run_start..]
            .iter()
            .filter(|r| value.is_some() && r.value == value)
            .map(|r| clamp01(r.confidence))
            .reduce(f32::max);
        (value, best)
    };
    StatConfirm {
        verdict,
        value,
        confidence,
        reason,
        unsettled_reads: unsettled,
    }
}

/// 0..=1. NaN, infinities, negatives and -0.0 all give +0.0.
fn clamp01(x: f32) -> f32 {
    if x.is_finite() && x > 0.0 {
        x.min(1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
