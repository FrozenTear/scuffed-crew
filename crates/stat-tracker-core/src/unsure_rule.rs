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
//! The rule is pure and cheap (one pass, no allocation). It does not change
//! any read, the readers, the capture gate or what is stored or uploaded:
//! when to show the flag is the tracker's call, built on this verdict.

/// Unsettled reads in a row (unsure, empty, or going down) before the
/// verdict becomes [`Verdict::Unsure`] during a game.
pub const UNSURE_AFTER_READS: u32 = 3;

/// One read of one stat cell, in game order.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct StatRead {
    /// The number read, `None` when the cell gave none.
    pub value: Option<u32>,
    /// The reader's confidence in this read, 0..=1 (cv: the cell's
    /// calibrated confidence; ocr-v1: Tesseract's 0..100 over 100).
    pub confidence: f32,
    /// The reader or the capture gate did not vouch for this read.
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
    /// No read since the start or the last reset.
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
    /// The reader's confidence in `value`, 0..=1: the highest confidence of
    /// the reads since the last confirmed value that read `value`, or of the
    /// confirming read when the cell is confirmed. 0 with no such read. It
    /// is the reader's own number (near miss vs bad read, #619), not a
    /// probability that `value` is right.
    pub confidence: f32,
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
    let mut last_conf: f32 = 0.0;
    let mut reason = Reason::NoReads;
    for read in reads {
        if read.reset {
            confirmed = None;
            unsettled = 0;
            last = None;
            last_conf = 0.0;
        }
        let conf = clamp01(read.confidence);
        let at_or_above = |v: u32| confirmed.is_none_or(|(c, _)| v >= c);
        match read.value {
            Some(v) if !read.unsure && at_or_above(v) => {
                confirmed = Some((v, conf));
                unsettled = 0;
                reason = Reason::SureRead;
            }
            Some(v) if read.unsure && confirmed.is_some_and(|(c, _)| c == v) => {
                if let Some((_, c)) = confirmed.as_mut() {
                    *c = c.max(conf);
                }
                unsettled = 0;
                reason = Reason::AgreesWithSure;
            }
            _ => {
                // Same value as the last unsettled read: keep its best
                // confidence; a new value starts over.
                if unsettled == 0 || read.value != last {
                    last_conf = 0.0;
                }
                if read.value.is_some() {
                    last_conf = last_conf.max(conf);
                }
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
        (Some(v), c)
    } else {
        match last {
            Some(v) => (Some(v), last_conf),
            None => (confirmed.map(|(v, _)| v), 0.0),
        }
    };
    StatConfirm {
        verdict,
        value,
        confidence,
        reason,
        unsettled_reads: unsettled,
    }
}

fn clamp01(x: f32) -> f32 {
    if x.is_finite() {
        x.clamp(0.0, 1.0)
    } else {
        0.0
    }
}

#[cfg(test)]
mod tests;
