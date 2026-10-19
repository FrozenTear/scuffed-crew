//! Per-cell capture gate for the scoreboard OCR pipeline.
//!
//! Overwatch scoreboard counters are cumulative within a single match, so a
//! later capture that reads a counter *below* the last accepted value is a
//! misread, not real play. Two field failure modes (2026-07-18 night shift,
//! see `docs/notes/night-shift-backlog.md` item 8 and the step-0 drift
//! analysis) corrupt real games:
//!
//! * **collapse** — a two-digit kill column clips to one digit ("13" → "1"),
//!   or a four-digit accumulator tail-clips ("2341" → "234"). Both are
//!   *decreases* versus the last accepted value.
//! * **inflation** — a ghost leading "9" walks in from an ability icon left of
//!   the elims column ("13" reads "91"/"93"/"99"). These *increase* past any
//!   plausible per-second rate, so a monotonic check alone cannot catch them —
//!   the rate cap here is load-bearing, not optional.
//!
//! This gate is per-cell and one-sided by design: it holds the previous
//! accepted value for a *single* cell that regresses (B) or that jumps beyond a
//! plausible rate without corroboration (C), while every genuinely-advancing
//! cell in the same capture passes through untouched. It is deliberately
//! DISTINCT from the whole-row game-split signal (`stats_regressed`, which
//! requires 2 of 3 of E/D/DMG to drop): a real new game resets every counter
//! and must still split, so the caller passes `split = true` and the gate then
//! accepts the raw read verbatim as the first capture of the new game.
//!
//! ## No downward revision
//!
//! Elims, assists, deaths, damage, healing, and mitigation do not decrease
//! inside one game once the value has been confirmed. A lower read keeps
//! the accepted value and names that field unsure (`e`, `a`, `d`, `dmg`,
//! `h`, `mit`). Three OCR reads of 1 must not replace a held 11.
//!
//! Two cases may still store a lower number. An unconfirmed latch (a
//! fallback or an implausible first read that never got a matching second
//! read) comes down only when two later reads name the same lower number.
//! One low read keeps the high value and flags that field. A high stored
//! while another column was held down is a row shift, and a later read of
//! the real row can replace it even after the shifted number repeated. A
//! value confirmed by two matching reads, and not a row shift, cannot go
//! down. A real new game is the whole-board reset in `boundary`, and the
//! caller passes `split = true` so this gate stores the new counters
//! instead of holding them.
//!
//! ## Single-field jump
//!
//! A rise in one column past [`JUMP_MAX`], while every other column stays
//! within [`QUIET_RISE`] of its accepted value, is unsure until the next
//! capture agrees, within the corroboration band. 13 read as 18 (+5 elims,
//! damage +100) stays 13. A second read of 18 stores 18. A correcting read
//! does not. Elims 4 to 9 with assists and deaths also up is ordinary play
//! and is stored. A drop in another column is the board moving, not a quiet
//! cell, so that rise is stored too. Two or more columns past their own
//! limits in the same capture are a real stomp and are not held by this
//! rule. The rate cap still holds a kill-column spike such as 9 to 91.
//!
//! ## Wide-column inflation (CG-4 B1 / B2)
//!
//! Pre-CG-4, wide accumulator columns (DMG/HLG/MIT) had **no** rate cap, so a
//! drifted inject (DMG 35031, HLG 22994) was accepted and latched. CG-4 closes
//! that gap without re-capping clean wide advances (real multi-k damage bursts
//! must never hold):
//!
//! * **B1** — when the *current* read is edge-ink suspect, wide cols get a
//!   calibrated rate ceiling (same HoldKind::RateCap path as kill cols).
//! * **B2** — trailing-digit inject heuristic (`accepted_digits+1` and prefix
//!   ≥ accepted) holds as `HoldKind::DigitInject`. Belt for `1681→22994`-style
//!   cases; does **not** cover `2782→22994` alone (prefix < accepted) — that
//!   needs B1 + Lane A geometry.
//!
//! ## Low-trust latch
//!
//! A raw-text fallback, or a first capture whose kill columns are past the
//! parser ceilings, is stored but marked `low_trust`. Those columns are
//! `unconfirmed`. Two later clean reads of the same lower number may replace
//! an unconfirmed column. One low read keeps the high value and flags the
//! field. A confirmed column never decreases.
//!
//! That replacement still goes through the same holds as any other capture:
//! a decrease of a *confirmed* column is the monotonic hold (B), a kill-column
//! jump past [`max_delta`] without corroboration is the rate cap (C), and a
//! trailing-digit inject is [`HoldKind::DigitInject`] (B2). A replacement
//! below the confirmed value a fallback moved off, or a wide column that is
//! the latched number with its last digit cut off, also falls through to B.
//! While any of those holds is in force, or any column is still unconfirmed,
//! `low_trust` stays set. A second fallback, or a read with edge-ink, does
//! not clear an unconfirmed column.
//!
//! ## Known residuals (documented, out of scope)
//!
//! * **F-CG2-1a** — the raw-continuity split vote anchors on `last_raw`, which
//!   is only trustworthy once at least one capture followed the injection. If a
//!   ≥120s capture gap lands on the very first capture AFTER a clean-slipping
//!   inflated read (`last_raw` still = the inject), a later clean read drops
//!   versus both anchors and can still split mid-game. Requires an idle Tab
//!   immediately after an inject that also evaded the edge-ink flag — compound
//!   odds are low. Revisit only if observed in the field.
//! * **CG-1a (mode c, row-shift).** A capture that reads a different
//!   player's row produces clean cells (edge-ink cannot flag them: the glyphs
//!   are well-centered, just the wrong player's). A higher wrong-row read can
//!   still latch. It no longer walks the accepted value down. The fix for the
//!   high latch is row identity (name-anchored row selection), tracked as the
//!   CG-1a follow-up.
//!
//! ## Fixtures
//!
//! Numeric replay tests below encode real `matches.jsonl` series. The pixel-level
//! edge-ink threshold that feeds the `suspect` mask was calibrated against 20
//! real drift frames at `crates/stat-tracker/test-data/drift-20260720/`
//! (gitignored, local-only — not in CI).

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Number of cumulative counters gated per capture: E, A, D, DMG, HLG, MIT.
pub const GATE_COLS: usize = 6;

/// The six cumulative scoreboard counters, in the positional order
/// `[elims, assists, deaths, damage, healing, mitigation]` — matching the
/// column order produced by `parse::stats_from_row`.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Counters {
    pub elims: u32,
    pub assists: u32,
    pub deaths: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
}

impl Counters {
    pub(crate) fn to_array(self) -> [u32; GATE_COLS] {
        [
            self.elims,
            self.assists,
            self.deaths,
            self.damage,
            self.healing,
            self.mitigation,
        ]
    }

    fn from_array(a: [u32; GATE_COLS]) -> Self {
        Counters {
            elims: a[0],
            assists: a[1],
            deaths: a[2],
            damage: a[3],
            healing: a[4],
            mitigation: a[5],
        }
    }

    /// `(elims, deaths, damage)` — the triple the whole-row game-split signal
    /// (`stats_regressed`) reads.
    pub fn edd(self) -> (u32, u32, u32) {
        (self.elims, self.deaths, self.damage)
    }
}

/// State the gate carries from one accepted capture to the next, within a
/// single game. `accepted` is the post-hold value actually stored; `last_raw`
/// is the raw OCR read (even when it was held), used for the
/// two-consecutive-reads corroboration of an implausible jump (C).
///
/// The `down_*` / `last_raw_suspect` / `low_trust` / `unconfirmed` /
/// `confirmed_floor` fields are ADDITIVE with `#[serde(default)]`: an
/// in-flight `active_game.json` written by an older build deserializes
/// cleanly (missing means zero/false, i.e. no streak in progress, previous
/// raw treated as clean, latch already trusted, no confirmed floor). Do not
/// rename or drop the existing fields. That would silently discard recovered
/// in-game state.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateState {
    pub accepted: Counters,
    pub last_raw: Counters,
    /// Per-column length of the current run of clean below-accepted reads.
    #[serde(default)]
    pub down_streak_len: [u32; GATE_COLS],
    /// Per-column latest read in that run (for the mutual-consistency band).
    #[serde(default)]
    pub down_streak_last: [u32; GATE_COLS],
    /// Whether each column's `last_raw` read was suspect — a suspect prior read
    /// must never corroborate the current capture's upward jump (C).
    #[serde(default)]
    pub last_raw_suspect: [bool; GATE_COLS],
    /// The latched counters are not confirmed by a clean per-cell read.
    ///
    /// Set when they came from the raw-text fallback, or when a kill column
    /// is above the same ceilings the parser uses (elims/assists above 99,
    /// deaths above 50), including the first capture of a game, which
    /// otherwise accepts the raw read verbatim. Two later clean reads of the
    /// same lower number replace the latch. One low read does not. Missing
    /// on a pre-0.4.21 `active_game.json` means already trusted.
    #[serde(default)]
    pub low_trust: bool,
    /// Which accepted columns came from a fallback or an implausible latch.
    /// Two clean reads of the same lower number may replace these. One low
    /// read keeps the accepted value. A column left false was already
    /// confirmed, so a decrease of it keeps that value. Missing on an older
    /// save means none.
    #[serde(default)]
    pub unconfirmed: [bool; GATE_COLS],
    /// Previous accepted value of a column that was confirmed, then moved by
    /// a fallback. A clean replacement below this floor is a clip of that
    /// confirmed value, so it takes the monotonic hold. Cleared when a clean
    /// read stores the column and confirms it. Missing on an older save
    /// means no floor.
    #[serde(default)]
    pub confirmed_floor: [u32; GATE_COLS],
    /// Which columns have [`Self::confirmed_floor`] set. A floor of 0 is a
    /// real confirmed zero, so the flag is not the value.
    #[serde(default)]
    pub has_confirmed_floor: [bool; GATE_COLS],
    /// The accepted value was stored while an adjacent column was held down,
    /// so the capture was a different player's row or a neighboring cell.
    /// Two later reads of the same lower number may replace it. One low read
    /// keeps the value. Missing on an older save means not a row shift.
    #[serde(default)]
    pub row_shift: [bool; GATE_COLS],
    /// The column has had no read at all in this game: every capture so far
    /// held it ([`HoldKind::Unread`], `reader = "new"`), so `accepted` is a
    /// placeholder 0 and not a number off the board. While any column is
    /// unread the game has no complete row: the capture path stores none
    /// ([`Self::unread_cols`]) and the boundary plan leaves the column out of
    /// every comparison. The first read of the column is stored as a first
    /// capture's would be, unless another column drops on that capture.
    /// Missing on an older save means every column was read.
    #[serde(default)]
    pub unread: [bool; GATE_COLS],
    /// Seconds between the column's last accepted read and the last capture.
    ///
    /// The gap of every capture that held the column ([`HoldKind::Unread`])
    /// is added, and so is the gap of every gate hold that follows until a
    /// read is accepted. The rate checks of the column's next read measure
    /// from its last accepted read, not from the last capture: the accepted
    /// value is that much older. Zero for a column that was read and
    /// accepted on the last capture, and always zero under ocr-v1, which has
    /// a number for every column on every capture. Missing on an older save
    /// means zero.
    #[serde(default)]
    pub held_secs: [u32; GATE_COLS],
    /// A first read of this still-unread column was seen and not taken,
    /// because another column dropped on that capture. Only for the line
    /// that says why a game was not stored: "never read" and "read, and not
    /// taken" are different things to tell the member. Missing on an older
    /// save means none.
    #[serde(default)]
    pub refused_first_read: [bool; GATE_COLS],
    /// The number the last stored row shows in this column when that is not
    /// [`Self::accepted`]: the digit reader's own unsure read of a cell it
    /// did not vouch for, stored flagged in place of the older accepted
    /// value (`reader = "new"`, set by the capture path after the gate, never
    /// by the gate). Only valid where [`Self::has_unsure_shown`] is set. The
    /// gate never reads it; the capture path compares the next row with it,
    /// so a stat never goes down unflagged from one stored row to the next.
    /// Missing on an older save means the stored row showed `accepted`.
    #[serde(default)]
    pub unsure_shown: [u32; GATE_COLS],
    /// Which columns have [`Self::unsure_shown`] set.
    #[serde(default)]
    pub has_unsure_shown: [bool; GATE_COLS],
}

impl GateState {
    /// The unread columns whose first read was seen and not taken
    /// ([`Self::refused_first_read`]), as `suspect_fields` names. A subset of
    /// [`Self::unread_cols`].
    pub fn refused_cols(&self) -> Vec<&'static str> {
        (0..GATE_COLS)
            .filter(|&col| self.unread[col] && self.refused_first_read[col])
            .map(|col| COL_FIELD[col])
            .collect()
    }

    /// The game has no row yet (a column is still unread) and no read column
    /// is confirmed by a trusted read as it stands now. That is a game read
    /// only on suspect own rows, and also a game whose trusted board had
    /// every one of its read columns moved by a later suspect board (a moved
    /// column is unconfirmed again). A trusted board then starts the game
    /// over ([`apply_gate_seeded`]): it is taken as a first capture, with no
    /// check against the boards before it. No row of the game is stored at
    /// that point, so no stored number changes.
    pub fn only_low_trust_reads(&self) -> bool {
        self.unread.iter().any(|&unread| unread)
            && (0..GATE_COLS).all(|col| self.unread[col] || self.unconfirmed[col])
    }
    /// The columns with no read yet in this game, as `suspect_fields` names.
    /// Empty once every column has had a sure read: only then does the
    /// capture path store a row for the game.
    pub fn unread_cols(&self) -> Vec<&'static str> {
        (0..GATE_COLS)
            .filter(|&col| self.unread[col])
            .map(|col| COL_FIELD[col])
            .collect()
    }
}

/// Why a cell was held, for per-capture observability logging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldKind {
    /// Cumulative counter decreased versus the last accepted value → misread
    /// collapse (B).
    Monotonic,
    /// Counter jumped beyond the plausible per-second rate and no corroborating
    /// prior read backed it → suspected inflation (C). Kill cols always; wide
    /// cols only when the current read is edge-ink suspect (CG-4 B1).
    RateCap,
    /// One column rose past [`JUMP_MAX`] and the previous raw read does not
    /// match it. The accepted value stays. A second agreeing read stores it.
    Jump,
    /// Cur has exactly one more digit than the accepted value, and dropping the
    /// trailing digit yields a non-decreasing "advance" of that accepted value
    /// (CG-4 B2). Classic OCR trailing-digit inject (`1681` → `16814` / `22994`
    /// from a lower accepted base). Does not fire on real rollovers like
    /// `9906` → `10311` (prefix `1031` < accepted).
    DigitInject,
    /// The capture has no number to store for this column (`reader = "new"`).
    /// Either the reader did not vouch for the cell (a suspect or empty
    /// digit read), or it is the first read of a never-read column on a
    /// capture where another column dropped: that board may be the next
    /// game's, so the read is not taken into this one. The accepted value
    /// stays, and the read is not recorded as the previous raw read, so it
    /// cannot corroborate, drop or un-latch anything.
    Unread,
}

/// One held cell in a capture — which column, why, and the raw→held swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hold {
    pub col: usize,
    pub kind: HoldKind,
    pub raw: u32,
    pub held: u32,
}

/// One un-latched cell in a capture (CG-2): a run of clean below-held reads
/// revised the accepted value DOWN, from `revised_from` to `raw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unlatch {
    pub col: usize,
    /// The clean read the accepted value was revised down to.
    pub raw: u32,
    /// The (suspected-corrupt) value that had been latched.
    pub revised_from: u32,
    /// True when two matching reads replaced an unconfirmed cell. A confirmed
    /// column is never revised down.
    pub replaced_unconfirmed: bool,
}

/// Result of gating one capture.
pub struct GateOutcome {
    /// Counters to actually store (raw where accepted, previous where held).
    pub accepted: Counters,
    /// State to carry into the next capture of this game.
    pub state: GateState,
    /// Cells that were held back from their raw read (empty = clean capture).
    pub holds: Vec<Hold>,
    /// Cells whose latched value was revised down by the un-latch (empty = none).
    pub unlatches: Vec<Unlatch>,
}

/// The kills-family columns (E, A, D): small, slow-growing integer counters.
const KILL_COLS: [usize; 3] = [0, 1, 2];

/// Rate cap for the kills family: a counter may climb by at most
/// `elapsed_secs / KILL_RATE_DIVISOR_SECS + KILL_RATE_SLACK` between two
/// accepted captures. Calibrated one-sided against the real 2026-07-18 series
/// so a genuine stomp is never rejected (E climbs 22→28 in ~90s = +6, cap ≥ 26;
/// 15→22 in ~41s = +7, cap ≥ 16) while the 9X ghost sits far above it
/// (E 9→91 in 75s: cap = 75/5 + 8 = 23, held).
pub(crate) const KILL_RATE_DIVISOR_SECS: u64 = 5;
pub(crate) const KILL_RATE_SLACK: u32 = 8;

/// Wide-column (DMG/HLG/MIT) rate ceiling applied **only when the current read
/// is edge-ink suspect** (CG-4 B1). Clean wide advances stay uncapped so a real
/// multi-thousand damage burst is never rejected. Cap =
/// `elapsed_secs * WIDE_RATE_PER_SEC + WIDE_RATE_SLACK`, calibrated one-sided
/// against the real series:
/// - Route66 DMG 10470→12672 in 90s (+2202) must pass if ever suspect
/// - Antarctic clean climb 6810→10311 in 60s (+3501) must pass if ever suspect
/// - Antarctic inject 3235→35031 in 20s (+31796) must hold when suspect
/// - Field HLG 2782→22994 (any short gap) must hold when suspect
pub(crate) const WIDE_RATE_PER_SEC: u32 = 80;
pub(crate) const WIDE_RATE_SLACK: u32 = 2500;

/// Absolute floor of the corroboration band; the effective band is
/// `max(CORROBORATION_ABS, level/10)` so a repeated high read still corroborates
/// after small OCR jitter.
const CORROBORATION_ABS: u32 = 2;

/// Largest one-capture rise stored immediately, per column.
///
/// Order is elims, assists, deaths, damage, healing, mitigation. A larger
/// rise in exactly one column is [`HoldKind::Jump`] until the next capture
/// matches, but only when every other column stays within [`QUIET_RISE`]
/// of the accepted value, up or down.
/// 13 read as 18 is +5 elims, past 4, so it waits. 9 to 13 is +4 and is
/// stored. Deaths allow +2. Damage allows +4000, so a clean burst from
/// 6810 to 10311 (+3501) still stores, and a five-digit inject does not.
/// Healing and mitigation allow +2500.
const JUMP_MAX: [u32; GATE_COLS] = [4, 4, 2, 4000, 2500, 2500];

/// Largest change in a *different* column that still counts as a quiet board.
///
/// A single-field jump is one misread cell. The check is absolute: assists
/// up by 4, mitigation up by a few hundred, or a latched spike falling back
/// all mean the rest of the board moved, and the rise is stored. Damage
/// +100 between two tabs of the same fight stays quiet. Deaths treat a
/// change of 1 as quiet, same as elims and assists.
const QUIET_RISE: [u32; GATE_COLS] = [1, 1, 1, 200, 200, 200];

/// Flat `suspect_fields` names, same order as [`Counters::to_array`].
pub const COL_FIELD: [&str; GATE_COLS] = ["e", "a", "d", "dmg", "h", "mit"];

fn is_kill_col(col: usize) -> bool {
    KILL_COLS.contains(&col)
}

fn digit_len(n: u32) -> u32 {
    if n == 0 { 1 } else { n.ilog10() + 1 }
}

/// CG-4 B2: trailing-digit injection heuristic.
///
/// Fires when all of:
/// 1. `cur` has exactly one more digit than `prev_acc`
/// 2. dropping the last digit of `cur` yields a value ≥ `prev_acc` (looks like
///    a cumulative advance with a garbage trailing digit glued on)
/// 3. the delta exceeds the same wide rate ceiling used by B1
///    (`elapsed * WIDE_RATE_PER_SEC + WIDE_RATE_SLACK`) — otherwise a genuine
///    long-gap climb that gains a digit (e.g. 1681→16900 after ≥10 min) or a
///    near-zero 1→2 digit climb (0..9 → 10..99) would latch forever with no
///    corroboration escape (Claude MED B2-FP review).
///
/// Does **not** fire on `2782→22994` alone (prefix `2299` < accepted) — that
/// needs B1 + geometry (Lane A). Does not fire on real rollovers
/// `9906→10311` (prefix `1031` < accepted).
fn is_trailing_digit_inject(prev_acc: u32, cur: u32, elapsed: Duration) -> bool {
    if cur <= prev_acc {
        return false;
    }
    if digit_len(cur) != digit_len(prev_acc) + 1 {
        return false;
    }
    let prefix = cur / 10;
    if prefix < prev_acc {
        return false;
    }
    let ceiling = (elapsed.as_secs() as u32)
        .saturating_mul(WIDE_RATE_PER_SEC)
        .saturating_add(WIDE_RATE_SLACK);
    cur.saturating_sub(prev_acc) > ceiling
}

/// Maximum plausible increase for column `col` over `elapsed`.
///
/// * Kill cols (E/A/D): always rate-capped.
/// * Wide cols (DMG/HLG/MIT): uncapped when the current read is clean; when
///   edge-ink suspect (CG-4 B1), apply the wide rate ceiling so a drifted
///   inject like HLG `22994` or DMG `35031` cannot latch.
fn max_delta(col: usize, elapsed: Duration, cur_suspect: bool) -> u32 {
    if is_kill_col(col) {
        ((elapsed.as_secs() / KILL_RATE_DIVISOR_SECS) as u32).saturating_add(KILL_RATE_SLACK)
    } else if cur_suspect {
        (elapsed.as_secs() as u32)
            .saturating_mul(WIDE_RATE_PER_SEC)
            .saturating_add(WIDE_RATE_SLACK)
    } else {
        u32::MAX
    }
}

/// Whether the previous raw read of a cell corroborates the current
/// (implausible) read — i.e. two consecutive captures agree on the new level.
fn corroborates(prev_raw: u32, cur: u32) -> bool {
    let band = CORROBORATION_ABS.max(cur / 10);
    prev_raw.abs_diff(cur) <= band
}

/// Whether `cur` is a real drop below the previous raw read `prev_raw` — below
/// it by more than the corroboration jitter band, so OCR jitter is not a drop.
///
/// Used by the split decision's raw-continuity vote (F-CG2-1): the gate's
/// `last_raw` tracks reality even while `accepted` is latched high (CG-2), so a
/// latch-recovery read (10311→10500) is continuous with `last_raw` and does NOT
/// drop, while a genuine new-game reset (~0) drops below both `accepted` and
/// `last_raw` and still splits.
pub fn raw_dropped(prev_raw: u32, cur: u32) -> bool {
    cur < prev_raw && !corroborates(prev_raw, cur)
}

/// True when `col` is the only column past [`JUMP_MAX`] and the others are quiet.
fn sudden_single_field_jump(col: usize, prev: [u32; GATE_COLS], cur: [u32; GATE_COLS]) -> bool {
    if cur[col].saturating_sub(prev[col]) <= JUMP_MAX[col] {
        return false;
    }
    let big = (0..GATE_COLS)
        .filter(|&i| cur[i].saturating_sub(prev[i]) > JUMP_MAX[i])
        .count();
    if big != 1 {
        return false;
    }
    !(0..GATE_COLS).any(|i| i != col && cur[i].abs_diff(prev[i]) > QUIET_RISE[i])
}

/// Flat suspect names for holds that kept the previous value.
///
/// A monotonic drop, a single-field jump and a cell the reader did not vouch
/// for are unsure. A rate-cap spike and a trailing-digit inject stay on
/// their own hold kinds and are not added here.
pub fn unsure_fields(holds: &[Hold]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for hold in holds {
        if !matches!(
            hold.kind,
            HoldKind::Monotonic | HoldKind::Jump | HoldKind::Unread
        ) {
            continue;
        }
        let name = COL_FIELD[hold.col];
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Flat names of every cell whose stored value is not this capture's read,
/// whatever the hold kind.
///
/// `reader = "new"` flags all of them: a rate-cap or trailing-digit hold
/// keeps an older number just as a drop does, and after a run of held
/// captures that number can be minutes old (F266-3). ocr-v1 keeps
/// [`unsure_fields`], which leaves those two kinds unflagged.
pub fn held_fields(holds: &[Hold]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for hold in holds {
        let name = COL_FIELD[hold.col];
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Flat names of the cells an un-latch revised down on this capture.
///
/// The gate lets two matching clean reads replace an unconfirmed column
/// with a lower number, and that is not a hold. Under `reader = "new"` a
/// suspect own row leaves every column it moved unconfirmed, so the next
/// reads can lower several stats at once (R3-1): the row that stores the
/// lower number names those fields.
pub fn revised_fields(unlatches: &[Unlatch]) -> Vec<&'static str> {
    let mut names = Vec::new();
    for unlatch in unlatches {
        let name = COL_FIELD[unlatch.col];
        if !names.contains(&name) {
            names.push(name);
        }
    }
    names
}

/// Flat names of the stats that are lower in `after` than in `before`.
///
/// `before` is the row stored just before in the same game. Whatever let
/// the number down (an un-latch, a restart, a board carried onto a split),
/// a stat that goes down inside one game is flagged under `reader = "new"`.
pub fn lowered_fields(before: Counters, after: Counters) -> Vec<&'static str> {
    let (before, after) = (before.to_array(), after.to_array());
    (0..GATE_COLS)
        .filter(|&col| after[col] < before[col])
        .map(|col| COL_FIELD[col])
        .collect()
}

/// Apply the per-cell capture gate.
///
/// `prev` — the gate state and elapsed-since carried from the last accepted
/// capture of this game, or `None` for the first capture. `raw` — this
/// capture's parsed counters. `suspect` — per-column edge-ink mask (CG-3): a
/// `true` column had glyph ink touching a crop edge, so its read is stripped of
/// all gate influence (never corroborates a jump, never drives an un-latch),
/// though a *suspect* wide-column advance is now also rate-capped (CG-4 B1)
/// and trailing-digit injects are held (CG-4 B2). `split` — whether the
/// whole-row game-split signal already fired (a real new game).
///
/// On a split or a first capture the raw read is accepted verbatim (a new game
/// legitimately resets every counter) and marked low-trust when the read is
/// untrusted or a kill column is past the ceilings. Otherwise each cell is
/// checked independently: a decrease keeps the previous accepted value and is
/// unsure; a trailing-digit inject is held (CG-4 B2); a single-field rise past
/// [`JUMP_MAX`] waits for a matching second read; an increase beyond the
/// plausible rate is held unless a clean previous raw read corroborates it
/// (C / CG-4 B1 for suspect wide cols); an advance within those limits passes
/// through unchanged.
///
/// `apply_gate` is [`apply_gate_with_trust`] with the read marked trusted.
pub fn apply_gate(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    suspect: [bool; GATE_COLS],
    split: bool,
) -> GateOutcome {
    apply_gate_with_trust(prev, raw, suspect, split, true)
}

fn kill_col_over_ceiling(col: usize, value: u32) -> bool {
    match col {
        0 => value > crate::parse::MAX_ELIMS,
        1 => value > crate::parse::MAX_ASSISTS,
        2 => value > crate::parse::MAX_DEATHS,
        _ => false,
    }
}

/// Kill columns past the shared parse ceilings.
///
/// A 0.4.20 save has no per-column trust bit, so only these columns are
/// marked. A shifted wide column that sits above its real value stays there:
/// a confirmed column does not decrease. Wide columns have no ceiling here.
fn mark_implausible(unconfirmed: &mut [bool; GATE_COLS], c: Counters) {
    let cols = c.to_array();
    for col in 0..GATE_COLS {
        if kill_col_over_ceiling(col, cols[col]) {
            unconfirmed[col] = true;
        }
    }
}

/// An untrusted read must not corroborate the next jump or build a down
/// streak. Recording every column suspect is the same rule the edge-ink
/// mask already uses.
fn raw_suspect_mask(suspect: [bool; GATE_COLS], trusted: bool) -> [bool; GATE_COLS] {
    if trusted { suspect } else { [true; GATE_COLS] }
}

fn fresh_state(raw: Counters, suspect: [bool; GATE_COLS], trusted: bool) -> GateState {
    // An untrusted first capture has not confirmed any column. A trusted
    // one only leaves the kill columns that are past the ceilings
    // unconfirmed. The other columns of that read were fine.
    let mut unconfirmed = [!trusted; GATE_COLS];
    mark_implausible(&mut unconfirmed, raw);
    GateState {
        accepted: raw,
        last_raw: raw,
        last_raw_suspect: raw_suspect_mask(suspect, trusted),
        low_trust: unconfirmed.iter().any(|&c| c),
        unconfirmed,
        ..Default::default()
    }
}

/// [`apply_gate`] with an explicit trust bit.
///
/// `trusted` is false when the six stats came from the raw-text fallback.
/// That latch is `low_trust` even when every column is in range, and so is
/// a first capture whose kill columns are implausible: the first capture
/// used to be stored as the baseline with no check, which is how a shifted
/// assists/deaths pair locked for the rest of the match.
///
/// A later read that is trusted and has no edge-ink suspect column may
/// replace an *unconfirmed* column with a lower number only when this raw
/// read equals the previous raw read. One low read keeps the high value.
/// The replacement still applies the monotonic hold (B) to a confirmed
/// column, the rate cap (C), and the trailing-digit inject hold (B2). It
/// also holds a replacement below the confirmed value a fallback moved off,
/// and a wide column whose clean read is the latched number with its last
/// digit cut off. `low_trust` stays set while any column is held or still
/// unconfirmed. A second fallback, or a read with edge-ink, does not clear
/// an unconfirmed column. A trusted decrease of a confirmed latch keeps the
/// old value. An untrusted read is recorded as suspect on every column, so
/// it does not corroborate the next jump or the next lower read.
pub fn apply_gate_with_trust(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    suspect: [bool; GATE_COLS],
    split: bool,
    trusted: bool,
) -> GateOutcome {
    apply_gate_held(prev, raw, suspect, [false; GATE_COLS], split, trusted)
}

/// [`apply_gate_with_trust`] with cells the reader did not vouch for.
///
/// `held[col]` means this capture has no number for the column
/// (`reader = "new"`: the digit read was suspect or empty, and nothing else
/// may supply it). `raw` for that column is ignored. The accepted value
/// stays ([`HoldKind::Unread`]), and the column's previous raw read, its
/// suspect bit and its streaks are carried over untouched, so the next
/// capture is compared with the last real read. A held column never votes:
/// it cannot corroborate a jump, make another column's jump look lone or
/// quiet, confirm an unconfirmed column, or tag a row shift.
///
/// On a first capture or a split there is no accepted value to keep. The
/// column is a placeholder 0 marked [`GateState::unread`], and the caller
/// stores no row for the game while any column is unread
/// ([`GateState::unread_cols`]). The first real read of the column is then
/// taken as a first capture's would be, with no rate cap or jump hold
/// against the placeholder, unless another column takes the monotonic hold
/// on that capture: a board whose read columns drop may be the next game's,
/// and its number must not land in this game's never-read column. That
/// read is an [`HoldKind::Unread`] hold and the column stays unread.
///
/// The rate checks of a column measure from its last accepted read
/// ([`GateState::held_secs`]), so the first sure read after a run of held
/// captures is not judged against one capture interval.
///
/// A game with no row yet whose reads so far were all low-trust
/// ([`GateState::only_low_trust_reads`]: a suspect own row, which may be
/// another player's) is started over by the first trusted board: that board
/// is gated as a first capture. Nothing of the game is stored yet, so there
/// is no row the trusted read has to agree with, and holding it against a
/// suspect row's numbers would keep the game from ever getting a row
/// (R3-2).
///
/// With no held column and no unread column this is exactly
/// [`apply_gate_with_trust`].
pub fn apply_gate_held(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    suspect: [bool; GATE_COLS],
    held: [bool; GATE_COLS],
    split: bool,
    trusted: bool,
) -> GateOutcome {
    apply_gate_seeded(prev, raw, suspect, held, split, trusted, None)
}

/// [`apply_gate_held`] for a capture that opens a game together with an
/// earlier board of that same game.
///
/// `seed` is that board (the fresh-match board the boundary plan held off
/// the previous game and now writes onto the new one) and how long before
/// this capture it was read. It is a complete board: every column on it was
/// a sure read. On a first capture or a split, a held column takes the
/// seed's value instead of the placeholder 0, as an [`HoldKind::Unread`]
/// hold, and is not unread: the game has a read of it, on the row stored
/// just before this one. Without the seed that row would show a number and
/// this one a 0 under it. The seed does nothing on any other capture, and
/// nothing for a column that was read.
pub fn apply_gate_seeded(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    mut suspect: [bool; GATE_COLS],
    held: [bool; GATE_COLS],
    split: bool,
    trusted: bool,
    seed: Option<(Counters, Duration)>,
) -> GateOutcome {
    for col in 0..GATE_COLS {
        suspect[col] |= held[col];
    }
    let restart = trusted && prev.is_some_and(|(state, _)| state.only_low_trust_reads());
    let Some((state, elapsed)) = prev.filter(|_| !split && !restart) else {
        let read = raw.to_array();
        let mut first = read;
        let mut unread = held;
        let mut held_secs = [0u32; GATE_COLS];
        let mut holds = Vec::new();
        for col in 0..GATE_COLS {
            if !held[col] {
                continue;
            }
            first[col] = 0;
            if let Some((board, age)) = seed {
                first[col] = board.to_array()[col];
                unread[col] = false;
                held_secs[col] = secs_u32(age);
                holds.push(Hold {
                    col,
                    kind: HoldKind::Unread,
                    raw: read[col],
                    held: first[col],
                });
            }
        }
        let accepted = Counters::from_array(first);
        let mut state = fresh_state(accepted, suspect, trusted);
        state.unread = unread;
        state.held_secs = held_secs;
        if seed.is_some() {
            // A seeded column's previous raw read is the seed's, a sure read.
            for (flag, &cell) in state.last_raw_suspect.iter_mut().zip(&held) {
                if cell {
                    *flag = !trusted;
                }
            }
        }
        return GateOutcome {
            accepted,
            state,
            holds,
            unlatches: Vec::new(),
        };
    };

    let prev_acc = state.accepted.to_array();
    let prev_raw = state.last_raw.to_array();
    let prev_raw_suspect = state.last_raw_suspect;
    let read = raw.to_array();
    // A held column reads as "no change", so no other column's check sees a
    // move in it. The first read of an unread column has no previous value:
    // `jump_prev` takes the read itself for that column, for the same reason.
    let mut cur = read;
    let mut jump_prev = prev_acc;
    let mut first_read = [false; GATE_COLS];
    let mut unread = state.unread;
    let mut refused_first_read = state.refused_first_read;
    for col in 0..GATE_COLS {
        if held[col] {
            cur[col] = prev_acc[col];
        } else if state.unread[col] {
            first_read[col] = true;
            jump_prev[col] = cur[col];
            unread[col] = false;
            refused_first_read[col] = false;
        }
    }
    // A column's accepted value is as old as its last accepted read. After a
    // run of held captures that is longer than `elapsed` (F266-3).
    let col_elapsed = |col: usize| elapsed + Duration::from_secs(u64::from(state.held_secs[col]));
    // The single-field jump check of `col`. Another column that is back
    // from a held run moved over that whole run, not on this capture: its
    // rise is neither a second jump nor a busy board for `col`.
    let lone_jump = |col: usize| {
        let mut before = jump_prev;
        for other in 0..GATE_COLS {
            if other != col && state.held_secs[other] > 0 {
                before[other] = cur[other];
            }
        }
        sudden_single_field_jump(col, before, cur)
    };
    let mut out = cur;
    let mut down_len = state.down_streak_len;
    let mut down_last = state.down_streak_last;
    let mut unconfirmed = state.unconfirmed;
    let mut confirmed_floor = state.confirmed_floor;
    let mut has_confirmed_floor = state.has_confirmed_floor;
    let mut row_shift = state.row_shift;
    let mut holds = Vec::new();
    let mut unlatches = Vec::new();
    // A fully clean per-cell read may replace unconfirmed columns. Confirmed
    // columns, and every increase, still go through B, C, and B2 below.
    let replacing = state.low_trust && trusted && !suspect.iter().any(|&s| s);

    for col in 0..GATE_COLS {
        if held[col] {
            // No number for this column on this capture. Streaks stay.
            holds.push(Hold {
                col,
                kind: HoldKind::Unread,
                raw: read[col],
                held: prev_acc[col],
            });
            continue;
        }
        if first_read[col] {
            // Nothing was accepted for this column yet: store the read.
            down_len[col] = 0;
            down_last[col] = 0;
            continue;
        }
        // A confirmed value stays. An unconfirmed high, and a row-shift high,
        // come down only when this raw read equals the previous raw read, so
        // one low read cannot replace either of them.
        let clean_match = trusted
            && !suspect.iter().any(|&flag| flag)
            && !prev_raw_suspect[col]
            && prev_raw[col] == cur[col];
        let matched_low = replacing && unconfirmed[col] && clean_match;
        let revise_down = matched_low || (row_shift[col] && clean_match);
        if cur[col] < prev_acc[col] && revise_down {
            // Below the value a fallback moved off a confirmed column, or a
            // wide column that is just the latched number with its last
            // digit cut off. Both are clips. Fall through to the monotonic
            // hold instead of storing them as the new trusted value.
            let below_floor = has_confirmed_floor[col] && cur[col] < confirmed_floor[col];
            let wide_clip = !is_kill_col(col)
                && is_trailing_digit_inject(cur[col], prev_acc[col], col_elapsed(col));
            if !below_floor && !wide_clip {
                unlatches.push(Unlatch {
                    col,
                    raw: cur[col],
                    revised_from: prev_acc[col],
                    replaced_unconfirmed: true,
                });
                out[col] = cur[col];
                down_len[col] = 0;
                down_last[col] = 0;
                continue;
            }
        }
        if cur[col] < prev_acc[col] {
            // Cumulative counters do not fall inside one game. Keep the
            // accepted value. A repeated lower read does not revise it down.
            down_len[col] = 0;
            down_last[col] = 0;
            out[col] = prev_acc[col];
            holds.push(Hold {
                col,
                kind: HoldKind::Monotonic,
                raw: cur[col],
                held: prev_acc[col],
            });
        } else {
            // At or above the accepted value → no active decrease run.
            down_len[col] = 0;
            down_last[col] = 0;

            // (CG-4 B2) Trailing-digit inject on wide cols only — kill cols
            // already have a tight rate cap (and 9→91 would otherwise re-label
            // as DigitInject). Hold before rate/corroboration so a repeated
            // inject cannot corroborate itself past the guard.
            if !is_kill_col(col)
                && cur[col] > prev_acc[col]
                && is_trailing_digit_inject(prev_acc[col], cur[col], col_elapsed(col))
            {
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::DigitInject,
                    raw: cur[col],
                    held: prev_acc[col],
                });
                continue;
            }

            let ceiling =
                prev_acc[col].saturating_add(max_delta(col, col_elapsed(col), suspect[col]));
            // A suspect prior read must never corroborate the current jump.
            let corroborated = !prev_raw_suspect[col] && corroborates(prev_raw[col], cur[col]);
            if cur[col] > ceiling && !corroborated {
                // (C / CG-4 B1) implausible jump, uncorroborated → hold last accepted.
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::RateCap,
                    raw: cur[col],
                    held: prev_acc[col],
                });
            } else if !corroborated && lone_jump(col) {
                // One column jumped past JUMP_MAX. Keep the old value until
                // the next capture matches.
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::Jump,
                    raw: cur[col],
                    held: prev_acc[col],
                });
            }
            // else: plausible advance (or corroborated jump) → keep cur[col].
        }
    }

    // The first read of a never-read column is not taken on a capture where
    // another column dropped. Nothing vouches for which game that board is
    // from: with the game end missed, it is the next game's first board, and
    // its number would land unflagged in this game's column (F266-2). The
    // column stays unread, so the game still has no complete row.
    let mut refused = [false; GATE_COLS];
    if holds.iter().any(|hold| hold.kind == HoldKind::Monotonic) {
        for col in 0..GATE_COLS {
            if !first_read[col] {
                continue;
            }
            first_read[col] = false;
            refused[col] = true;
            unread[col] = true;
            refused_first_read[col] = true;
            out[col] = prev_acc[col];
            holds.push(Hold {
                col,
                kind: HoldKind::Unread,
                raw: cur[col],
                held: prev_acc[col],
            });
        }
    }
    // No number for the column on this capture.
    let no_read = |col: usize| held[col] || refused[col];

    // A kill column that rose past JUMP_MAX is a row shift when it sits in
    // the same adjacent run of cells as a column held down this frame: the
    // next cell, or the next risen cell along that run. A drop on the far
    // side of the board does not tag it. Keep the tag while any drop is
    // still held, so two later reads of the real row can replace the high.
    let decreased = holds.iter().any(|h| h.kind == HoldKind::Monotonic);
    let dropped: Vec<usize> = holds
        .iter()
        .filter(|hold| hold.kind == HoldKind::Monotonic)
        .map(|hold| hold.col)
        .collect();
    let rose = |col: usize| {
        is_kill_col(col)
            && !first_read[col]
            && out[col] > prev_acc[col]
            && cur[col].saturating_sub(prev_acc[col]) > JUMP_MAX[col]
    };
    let mut beside_drop = [false; GATE_COLS];
    let mut grew = true;
    while grew {
        grew = false;
        for col in 0..GATE_COLS {
            if beside_drop[col] || !rose(col) {
                continue;
            }
            let touches = (0..GATE_COLS).any(|other| {
                other.abs_diff(col) == 1 && (dropped.contains(&other) || beside_drop[other])
            });
            if touches {
                beside_drop[col] = true;
                grew = true;
            }
        }
    }
    for col in 0..GATE_COLS {
        if no_read(col) {
            // No read: the tag is carried as it was.
            continue;
        }
        if beside_drop[col] {
            row_shift[col] = true;
        } else if out[col] < prev_acc[col] {
            row_shift[col] = false;
        } else if decreased && row_shift[col] && out[col] == prev_acc[col] {
            row_shift[col] = true;
        } else if !decreased {
            row_shift[col] = false;
        }
    }

    let accepted = Counters::from_array(out);
    // Adopting a fallback number marks that column unconfirmed. If the
    // column was confirmed, keep that value as a floor so a later clip
    // cannot replace it.
    if !trusted {
        for col in 0..GATE_COLS {
            if first_read[col] {
                // The placeholder under a first read is not a confirmed
                // value, so it leaves no floor.
                unconfirmed[col] = true;
                continue;
            }
            if out[col] == cur[col] && out[col] != prev_acc[col] {
                if !unconfirmed[col] {
                    confirmed_floor[col] = prev_acc[col];
                    has_confirmed_floor[col] = true;
                }
                unconfirmed[col] = true;
            }
        }
    }
    if replacing {
        for col in 0..GATE_COLS {
            if out[col] != cur[col] {
                continue;
            }
            // A ghost under an implausible kill latch is stored, but the
            // column stays unconfirmed until a non-suspect read already
            // agreed with it. Otherwise B would lock the ghost in.
            // Plausible columns, including a fallback assists of 40, still
            // confirm on this read.
            let over = kill_col_over_ceiling(col, prev_acc[col]);
            let agrees = !prev_raw_suspect[col] && cur[col] == prev_raw[col];
            if over && !agrees {
                continue;
            }
            unconfirmed[col] = false;
            has_confirmed_floor[col] = false;
            confirmed_floor[col] = 0;
        }
    }
    // A 0.4.20 save has no low_trust bit, so a shifted latch loads as
    // trusted. Keeping the implausible number marks those columns so the
    // next clean read can replace them. Wide columns are not marked; a
    // shifted one stays where it is: a confirmed column does not decrease.
    mark_implausible(&mut unconfirmed, accepted);
    let low_trust = unconfirmed.iter().any(|&c| c) || (state.low_trust && !holds.is_empty());
    // A held column was not read: its previous raw read and that read's
    // suspect bit stand for the next capture.
    let mut last_raw = read;
    let mut last_raw_suspect = raw_suspect_mask(suspect, trusted);
    let mut held_secs = [0u32; GATE_COLS];
    for col in 0..GATE_COLS {
        if no_read(col) {
            last_raw[col] = prev_raw[col];
            last_raw_suspect[col] = prev_raw_suspect[col];
        }
        // The accepted value gets older while the column is held, and while
        // a gate hold keeps a read out after such a run. A column with no
        // held run behind it keeps the capture interval, as under ocr-v1.
        let kept_out = out[col] != cur[col];
        if !unread[col] && (held[col] || (kept_out && state.held_secs[col] > 0)) {
            held_secs[col] = state.held_secs[col].saturating_add(secs_u32(elapsed));
        }
    }
    GateOutcome {
        accepted,
        state: GateState {
            accepted,
            last_raw: Counters::from_array(last_raw),
            down_streak_len: down_len,
            down_streak_last: down_last,
            last_raw_suspect,
            low_trust,
            unconfirmed,
            confirmed_floor,
            has_confirmed_floor,
            row_shift,
            unread,
            held_secs,
            refused_first_read,
            // Set by the capture path after the gate, not here.
            unsure_shown: [0; GATE_COLS],
            has_unsure_shown: [false; GATE_COLS],
        },
        holds,
        unlatches,
    }
}

fn secs_u32(span: Duration) -> u32 {
    u32::try_from(span.as_secs()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn c(e: u32, a: u32, d: u32, dmg: u32, hlg: u32, mit: u32) -> Counters {
        Counters {
            elims: e,
            assists: a,
            deaths: d,
            damage: dmg,
            healing: hlg,
            mitigation: mit,
        }
    }

    fn secs(n: u64) -> Duration {
        Duration::from_secs(n)
    }

    /// No cell suspect — the common case in the focused unit tests.
    const CLEAN: [bool; GATE_COLS] = [false; GATE_COLS];

    /// A gate state seeded with `accepted == last_raw == v` and no streak, as
    /// every focused test's `prev` began before the un-latch fields existed.
    fn state(v: Counters) -> GateState {
        GateState {
            accepted: v,
            last_raw: v,
            ..Default::default()
        }
    }

    // --- Focused unit tests (crisp mutation-check mapping) ---

    #[test]
    fn first_capture_accepts_raw() {
        let out = apply_gate(None, c(5, 3, 2, 4316, 1200, 899), CLEAN, false);
        assert_eq!(out.accepted, c(5, 3, 2, 4316, 1200, 899));
        assert!(out.holds.is_empty());
        assert!(!out.state.low_trust);
    }

    #[test]
    fn fallback_latch_yields_to_two_matching_lower_reads() {
        // In-range but wrong assists/deaths from the text fallback. One
        // clean lower read keeps them and flags the fields. The same lower
        // read again replaces them.
        let first = apply_gate_with_trust(None, c(2, 40, 18, 450, 0, 2), CLEAN, false, false);
        assert!(first.state.low_trust);
        assert_eq!(first.accepted.assists, 40);
        let lower = c(2, 0, 0, 1105, 259, 450);
        let second =
            apply_gate_with_trust(Some((first.state, secs(20))), lower, CLEAN, false, true);
        assert_eq!(second.accepted.assists, 40);
        assert_eq!(second.accepted.deaths, 18);
        assert_eq!(second.accepted.damage, 1105);
        assert!(unsure_fields(&second.holds).contains(&"a"));
        assert!(unsure_fields(&second.holds).contains(&"d"));
        assert!(second.state.low_trust);
        let third =
            apply_gate_with_trust(Some((second.state, secs(20))), lower, CLEAN, false, true);
        assert_eq!(third.accepted, lower);
        assert!(!third.state.low_trust);
        assert!(
            third
                .unlatches
                .iter()
                .any(|u| u.col == 1 && u.raw == 0 && u.revised_from == 40)
        );
        assert!(
            third
                .unlatches
                .iter()
                .any(|u| u.col == 2 && u.raw == 0 && u.revised_from == 18)
        );
    }

    #[test]
    fn implausible_first_capture_does_not_lock() {
        // Shifted timer digits: A 1105, D 259 on the first capture of the
        // game. That used to become the baseline. One clean lower read keeps
        // them. The next read of the same numbers replaces them, trusted
        // source or not.
        let shifted = c(0, 1105, 259, 450, 0, 2);
        let clean = c(8, 1, 2, 3993, 989, 1583);
        for trusted in [false, true] {
            let first = apply_gate_with_trust(None, shifted, CLEAN, false, trusted);
            assert!(first.state.low_trust, "trusted={trusted}");
            if trusted {
                assert!(first.state.unconfirmed[1] && first.state.unconfirmed[2]);
                assert!(
                    !first.state.unconfirmed[0] && !first.state.unconfirmed[3],
                    "a trusted first capture only unconfirms the kill columns past the ceilings"
                );
            }
            let second =
                apply_gate_with_trust(Some((first.state, secs(30))), clean, CLEAN, false, true);
            assert_eq!(second.accepted.elims, 8, "trusted={trusted}");
            assert_eq!(second.accepted.assists, 1105, "trusted={trusted}");
            assert_eq!(second.accepted.deaths, 259, "trusted={trusted}");
            assert!(
                unsure_fields(&second.holds).contains(&"a"),
                "trusted={trusted}"
            );
            assert!(
                unsure_fields(&second.holds).contains(&"d"),
                "trusted={trusted}"
            );
            assert!(second.state.low_trust, "trusted={trusted}");
            assert!(second.state.unconfirmed[1] && second.state.unconfirmed[2]);
            let third =
                apply_gate_with_trust(Some((second.state, secs(20))), clean, CLEAN, false, true);
            assert_eq!(third.accepted, clean, "trusted={trusted}");
            assert!(!third.state.low_trust, "trusted={trusted}");
        }
    }

    #[test]
    fn another_fallback_does_not_replace_a_low_trust_latch() {
        let first = apply_gate_with_trust(None, c(2, 40, 18, 450, 0, 2), CLEAN, false, false);
        let second = apply_gate_with_trust(
            Some((first.state, secs(10))),
            c(2, 1, 2, 500, 10, 10),
            CLEAN,
            false,
            false,
        );
        assert_eq!(second.accepted.assists, 40);
        assert_eq!(second.accepted.deaths, 18);
        assert!(second.state.low_trust);
    }

    #[test]
    fn a_suspect_read_does_not_clear_a_low_trust_latch() {
        let first = apply_gate_with_trust(None, c(2, 40, 18, 450, 0, 2), CLEAN, false, false);
        let mut suspect = CLEAN;
        suspect[1] = true;
        let second = apply_gate_with_trust(
            Some((first.state, secs(10))),
            c(2, 1, 2, 500, 10, 10),
            suspect,
            false,
            true,
        );
        assert_eq!(second.accepted.assists, 40, "suspect assists stays held");
        assert!(second.state.low_trust);
    }

    #[test]
    fn clean_replacement_holds_a_kill_rate_ghost() {
        // E 9 → 91 in 20s. Cap is 20/5 + 8 = 12. A trusted first capture holds
        // it; a fallback latch must hold it too, and stay low-trust.
        let ghost = c(91, 0, 0, 0, 0, 0);
        let trusted = apply_gate(None, c(9, 0, 0, 0, 0, 0), CLEAN, false);
        let trusted_next = apply_gate(Some((trusted.state, secs(20))), ghost, CLEAN, false);
        assert_eq!(trusted_next.accepted.elims, 9);
        assert!(!trusted_next.state.low_trust);

        let fallback = apply_gate_with_trust(None, c(9, 0, 0, 0, 0, 0), CLEAN, false, false);
        assert!(fallback.state.low_trust);
        let next =
            apply_gate_with_trust(Some((fallback.state, secs(20))), ghost, CLEAN, false, true);
        assert_eq!(next.accepted.elims, 9);
        assert!(
            next.holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::RateCap && h.raw == 91)
        );
        assert!(next.state.low_trust);
    }

    #[test]
    fn clean_replacement_holds_a_trailing_digit_inject() {
        let first = apply_gate_with_trust(None, c(2, 0, 0, 5200, 0, 0), CLEAN, false, false);
        let next = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(2, 0, 0, 52004, 0, 0),
            CLEAN,
            false,
            true,
        );
        assert_eq!(next.accepted.damage, 5200);
        assert!(
            next.holds
                .iter()
                .any(|h| h.col == 3 && h.kind == HoldKind::DigitInject && h.raw == 52004)
        );
        assert!(next.state.low_trust);
    }

    #[test]
    fn a_mid_game_fallback_marks_only_the_adopted_columns() {
        let first = apply_gate(None, c(13, 5, 4, 5000, 0, 900), CLEAN, false);
        assert!(!first.state.low_trust);
        let adopted = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(13, 8, 4, 5200, 0, 1000),
            CLEAN,
            false,
            false,
        );
        assert!(adopted.state.low_trust);
        assert_eq!(adopted.accepted.assists, 8);
        assert_eq!(adopted.accepted.damage, 5200);
        assert!(adopted.state.unconfirmed[1]);
        assert!(adopted.state.unconfirmed[3]);
        assert!(adopted.state.unconfirmed[5]);
        assert!(!adopted.state.unconfirmed[0]);

        let drop = apply_gate_with_trust(
            Some((adopted.state, secs(20))),
            c(1, 8, 4, 5200, 0, 1000),
            CLEAN,
            false,
            true,
        );
        assert_eq!(
            drop.accepted.elims, 13,
            "a confirmed column still monotonic-holds"
        );
        assert!(
            drop.holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::Monotonic)
        );
        assert!(drop.state.low_trust);

        let ghost = apply_gate_with_trust(
            Some((adopted.state, secs(20))),
            c(93, 8, 4, 52004, 0, 1000),
            CLEAN,
            false,
            true,
        );
        assert_eq!(ghost.accepted.elims, 13);
        assert_eq!(ghost.accepted.damage, 5200);
        assert!(
            ghost
                .holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::RateCap)
        );
        assert!(
            ghost
                .holds
                .iter()
                .any(|h| h.col == 3 && h.kind == HoldKind::DigitInject)
        );
        assert!(ghost.state.low_trust);
    }

    #[test]
    fn a_fully_held_fallback_does_not_mark_the_latch_low_trust() {
        let first = apply_gate(None, c(13, 5, 4, 5000, 0, 900), CLEAN, false);
        let held = apply_gate_with_trust(
            Some((first.state, secs(10))),
            c(1, 1, 1, 100, 0, 100),
            CLEAN,
            false,
            false,
        );
        assert_eq!(held.accepted, first.accepted);
        assert!(!held.state.low_trust);
        assert!(!held.state.unconfirmed.iter().any(|&c| c));
    }

    #[test]
    fn a_pre_0_4_21_implausible_save_recovers_on_the_second_clean_read() {
        // 0.4.20 has no low_trust field. The shifted row loads as trusted,
        // so the first clean read holds it and only then marks the kill
        // columns unconfirmed. The second clean read replaces them.
        let loaded = state(c(0, 1105, 259, 450, 0, 2));
        assert!(!loaded.low_trust);
        let clean = c(8, 1, 2, 3993, 989, 1583);
        let first = apply_gate_with_trust(Some((loaded, secs(30))), clean, CLEAN, false, true);
        assert_eq!(first.accepted.assists, 1105);
        assert_eq!(first.accepted.deaths, 259);
        assert!(first.state.low_trust);
        assert!(first.state.unconfirmed[1]);
        assert!(first.state.unconfirmed[2]);
        let second =
            apply_gate_with_trust(Some((first.state, secs(20))), clean, CLEAN, false, true);
        assert_eq!(second.accepted, clean);
        assert!(!second.state.low_trust);
    }

    #[test]
    fn a_clip_below_a_confirmed_floor_stays_held_and_the_real_read_lands() {
        // R1. A fallback moved columns off values a clean read had already
        // confirmed. A later clean clip of those columns must not become
        // the trusted latch, or the inject hold locks the real damage out.
        let first = apply_gate(None, c(13, 5, 4, 5000, 0, 900), CLEAN, false);
        let adopted = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(14, 8, 4, 5200, 0, 1000),
            CLEAN,
            false,
            false,
        );
        assert!(adopted.state.has_confirmed_floor[0]);
        assert_eq!(adopted.state.confirmed_floor[0], 13);
        assert_eq!(adopted.state.confirmed_floor[1], 5);
        assert_eq!(adopted.state.confirmed_floor[3], 5000);
        assert_eq!(adopted.state.confirmed_floor[5], 900);
        assert!(!adopted.state.has_confirmed_floor[2]);

        let clip = apply_gate_with_trust(
            Some((adopted.state, secs(20))),
            c(1, 8, 4, 520, 0, 1000),
            CLEAN,
            false,
            true,
        );
        assert_eq!(clip.accepted.elims, 14);
        assert_eq!(clip.accepted.damage, 5200);
        assert!(
            clip.holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::Monotonic && h.raw == 1)
        );
        assert!(
            clip.holds
                .iter()
                .any(|h| h.col == 3 && h.kind == HoldKind::Monotonic && h.raw == 520)
        );
        assert!(clip.state.low_trust);

        let real = apply_gate_with_trust(
            Some((clip.state, secs(20))),
            c(14, 8, 4, 5300, 0, 1000),
            CLEAN,
            false,
            true,
        );
        assert_eq!(real.accepted.damage, 5300);
        assert_eq!(real.accepted.elims, 14);
        assert!(!real.state.has_confirmed_floor[3]);
    }

    #[test]
    fn a_wide_trailing_digit_clip_of_a_fallback_first_capture_stays_held() {
        // R5. The first capture is a correct fallback damage figure. One
        // clean read that drops the last digit must not become trusted, or
        // DigitInject holds every later real read.
        let first = apply_gate_with_trust(None, c(8, 1, 2, 12672, 0, 0), CLEAN, false, false);
        assert_eq!(first.accepted.damage, 12672);
        assert!(first.state.low_trust);
        let clip = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(8, 1, 2, 1267, 0, 0),
            CLEAN,
            false,
            true,
        );
        assert_eq!(clip.accepted.damage, 12672);
        assert!(
            clip.holds
                .iter()
                .any(|h| h.col == 3 && h.kind == HoldKind::Monotonic && h.raw == 1267)
        );
        assert!(clip.state.low_trust);

        let mut state = clip.state;
        let later = [13000, 14000, 15500, 17000, 19000, 21000];
        let gaps = [30, 40, 50, 60, 70, 90];
        for (dmg, gap) in later.into_iter().zip(gaps) {
            let next = apply_gate_with_trust(
                Some((state, secs(gap))),
                c(8, 1, 2, dmg, 0, 0),
                CLEAN,
                false,
                true,
            );
            assert_eq!(next.accepted.damage, dmg, "gap {gap}");
            state = next.state;
        }
        assert_eq!(state.accepted.damage, 21000);
        assert!(!state.low_trust);
    }

    #[test]
    fn a_wrong_high_fallback_above_the_floor_corrects_on_the_second_read() {
        // The fallback is in range and above the confirmed value. One clean
        // read below that latch, and still above the floor, keeps the high
        // value. The same read again replaces it.
        let first = apply_gate(None, c(13, 5, 4, 5000, 0, 900), CLEAN, false);
        let adopted = apply_gate_with_trust(
            Some((first.state, secs(120))),
            c(40, 12, 4, 8000, 0, 900),
            CLEAN,
            false,
            false,
        );
        assert_eq!(adopted.accepted.elims, 40);
        assert_eq!(adopted.accepted.damage, 8000);
        assert_eq!(adopted.state.confirmed_floor[0], 13);
        assert_eq!(adopted.state.confirmed_floor[3], 5000);
        let lower = c(19, 8, 4, 5900, 0, 900);
        let once =
            apply_gate_with_trust(Some((adopted.state, secs(20))), lower, CLEAN, false, true);
        assert_eq!(once.accepted.elims, 40);
        assert_eq!(once.accepted.damage, 8000);
        assert!(unsure_fields(&once.holds).contains(&"e"));
        assert!(once.state.low_trust);
        let clean = apply_gate_with_trust(Some((once.state, secs(20))), lower, CLEAN, false, true);
        assert_eq!(clean.accepted, lower);
        assert!(!clean.state.low_trust);
        assert!(!clean.state.has_confirmed_floor.iter().any(|&set| set));
    }

    #[test]
    fn a_fallback_raw_read_does_not_corroborate_a_ghost() {
        // D2. The second read is another fallback, held, and its raw 91
        // must not corroborate the clean ghost that follows.
        let first = apply_gate_with_trust(None, c(9, 0, 0, 0, 0, 0), CLEAN, false, false);
        let second = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(91, 0, 0, 0, 0, 0),
            CLEAN,
            false,
            false,
        );
        assert_eq!(second.accepted.elims, 9);
        assert!(second.state.last_raw_suspect.iter().all(|&s| s));
        let third = apply_gate_with_trust(
            Some((second.state, secs(20))),
            c(91, 0, 0, 0, 0, 0),
            CLEAN,
            false,
            true,
        );
        assert_eq!(third.accepted.elims, 9);
        assert!(
            third
                .holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::RateCap && h.raw == 91)
        );
        assert!(third.state.low_trust);

        // D3. A trusted latch, a held fallback ghost, then the same ghost
        // from a clean read. The fallback raw still does not corroborate.
        let trusted = apply_gate(None, c(13, 5, 4, 5000, 0, 900), CLEAN, false);
        let fallback = apply_gate_with_trust(
            Some((trusted.state, secs(20))),
            c(91, 5, 4, 5000, 0, 900),
            CLEAN,
            false,
            false,
        );
        assert_eq!(fallback.accepted.elims, 13);
        assert!(fallback.state.last_raw_suspect[0]);
        let ghost = apply_gate_with_trust(
            Some((fallback.state, secs(20))),
            c(91, 5, 4, 5000, 0, 900),
            CLEAN,
            false,
            true,
        );
        assert_eq!(ghost.accepted.elims, 13);
        assert!(
            ghost
                .holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::RateCap)
        );
    }

    #[test]
    fn three_fallback_decreases_do_not_unlatch_a_confirmed_column() {
        // G4. Three lower fallback reads must not walk a confirmed column
        // down. An untrusted read does not build the streak.
        let mut state = apply_gate(None, c(13, 5, 4, 5000, 0, 900), CLEAN, false).state;
        for _ in 0..3 {
            let next = apply_gate_with_trust(
                Some((state, secs(20))),
                c(1, 5, 4, 5000, 0, 900),
                CLEAN,
                false,
                false,
            );
            assert_eq!(next.accepted.elims, 13);
            assert!(next.unlatches.is_empty());
            assert!(!next.state.unconfirmed[0]);
            assert!(!next.state.low_trust);
            state = next.state;
        }
    }

    #[test]
    fn a_held_unconfirmed_column_stays_unconfirmed_until_two_lower_reads_match() {
        // A clean ghost is held, and that column stays unconfirmed. One
        // lower read keeps the latch. The same lower read again replaces it.
        let first = apply_gate_with_trust(None, c(9, 0, 0, 0, 0, 0), CLEAN, false, false);
        let held = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(91, 0, 0, 0, 0, 0),
            CLEAN,
            false,
            true,
        );
        assert_eq!(held.accepted.elims, 9);
        assert!(held.state.unconfirmed[0]);
        assert!(held.state.low_trust);
        let once = apply_gate_with_trust(
            Some((held.state, secs(20))),
            c(2, 0, 0, 0, 0, 0),
            CLEAN,
            false,
            true,
        );
        assert_eq!(once.accepted.elims, 9);
        assert!(unsure_fields(&once.holds).contains(&"e"));
        assert!(once.state.unconfirmed[0]);
        let replaced = apply_gate_with_trust(
            Some((once.state, secs(20))),
            c(2, 0, 0, 0, 0, 0),
            CLEAN,
            false,
            true,
        );
        assert_eq!(replaced.accepted.elims, 2);
        assert!(
            replaced
                .unlatches
                .iter()
                .any(|u| u.col == 0 && u.raw == 2 && u.replaced_unconfirmed)
        );
        assert!(!replaced.state.unconfirmed[0]);
        assert!(!replaced.state.low_trust);
    }

    #[test]
    fn an_implausible_ghost_does_not_replace_an_unconfirmed_high_in_one_read() {
        // C3. A single 91 under an unconfirmed 1105 stays at 1105. Two
        // matching reads of 1 replace it. There is no confirmed floor on a
        // first implausible capture.
        let first = apply_gate(None, c(0, 1105, 2, 450, 0, 2), CLEAN, false);
        assert!(first.state.unconfirmed[1]);
        assert!(!first.state.has_confirmed_floor[1]);
        let ghost = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(0, 91, 2, 450, 0, 2),
            CLEAN,
            false,
            true,
        );
        assert_eq!(ghost.accepted.assists, 1105);
        assert!(unsure_fields(&ghost.holds).contains(&"a"));
        assert!(ghost.state.unconfirmed[1]);
        assert!(ghost.state.low_trust);
        let once = apply_gate_with_trust(
            Some((ghost.state, secs(20))),
            c(0, 1, 2, 450, 0, 2),
            CLEAN,
            false,
            true,
        );
        assert_eq!(once.accepted.assists, 1105);
        assert!(unsure_fields(&once.holds).contains(&"a"));
        let real = apply_gate_with_trust(
            Some((once.state, secs(20))),
            c(0, 1, 2, 450, 0, 2),
            CLEAN,
            false,
            true,
        );
        assert_eq!(real.accepted.assists, 1);
        assert!(
            real.unlatches
                .iter()
                .any(|u| u.col == 1 && u.raw == 1 && u.replaced_unconfirmed)
        );
    }

    #[test]
    fn a_shifted_wide_column_on_an_old_save_does_not_decrease() {
        // F5. A 0.4.20 save only marks kill columns past the ceilings. A
        // wide column that is merely too high stays there. Three lower
        // reads do not revise it down.
        let loaded = state(c(8, 1, 2, 20000, 989, 1583));
        let clean = c(8, 1, 2, 3993, 989, 1583);
        let mut state = loaded;
        for n in 1..=3 {
            let next = apply_gate_with_trust(Some((state, secs(20))), clean, CLEAN, false, true);
            assert_eq!(next.accepted.damage, 20000, "read {n}");
            assert!(next.unlatches.is_empty());
            assert!(unsure_fields(&next.holds).contains(&"dmg"));
            state = next.state;
        }
    }

    #[test]
    fn split_resets_gate_to_raw() {
        // Even with a "previous" high state, a split (real new game) accepts the
        // fresh low read verbatim.
        let prev = state(c(30, 10, 8, 12000, 3000, 5000));
        let out = apply_gate(Some((prev, secs(180))), c(0, 0, 0, 200, 50, 0), CLEAN, true);
        assert_eq!(out.accepted, c(0, 0, 0, 200, 50, 0));
        assert!(out.holds.is_empty());
    }

    #[test]
    fn monotonic_hold_holds_a_single_cell_decrease() {
        // MUTATION CHECK (B): remove the `cur < prev_acc` hold and deaths stores 5.
        // Deaths 6→5 (the real Havana final-D bug); elims still advances 9→13.
        let prev = state(c(9, 7, 6, 6622, 1520, 4549));
        let out = apply_gate(
            Some((prev, secs(45))),
            c(13, 9, 5, 7219, 1598, 4999),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.deaths, 6, "decreased deaths must hold at 6");
        assert_eq!(
            out.accepted.elims, 13,
            "a real advance in another cell is kept"
        );
        assert_eq!(out.accepted.assists, 9);
        assert!(
            out.holds
                .iter()
                .any(|h| h.col == 2 && h.kind == HoldKind::Monotonic)
        );
    }

    #[test]
    fn monotonic_hold_catches_tail_clip_on_wide_column() {
        // HLG 1898 → 224 (tail-clip of 2241). Wide columns have no rate cap, so
        // only the monotonic hold protects them.
        let prev = state(c(13, 9, 6, 8561, 1898, 5199));
        let out = apply_gate(
            Some((prev, secs(85))),
            c(22, 10, 5, 9906, 224, 5958),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.healing, 1898, "clipped healing holds at 1898");
        assert_eq!(out.accepted.elims, 22, "real elims stomp still accepted");
        assert_eq!(out.accepted.deaths, 6, "clipped deaths holds at 6");
    }

    #[test]
    fn rate_cap_holds_uncorroborated_spike() {
        // MUTATION CHECK (C): remove the ceiling hold and elims stores 91.
        // E 9→91 in 75s with the prior raw read at 9 (no corroboration).
        let prev = state(c(9, 5, 3, 4119, 822, 2639));
        let out = apply_gate(
            Some((prev, secs(75))),
            c(91, 6, 3, 5175, 968, 3227),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.elims, 9, "ghost 91 held at 9");
        assert_eq!(out.accepted.assists, 6, "real assist advance kept");
        assert!(
            out.holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::RateCap)
        );
    }

    #[test]
    fn rate_cap_accepts_corroborated_level() {
        // Two consecutive captures agree on the high level → accept it (the C
        // corroboration valve, so a truly fast burst is not held forever).
        let prev = GateState {
            accepted: c(9, 5, 3, 4119, 822, 2639),
            last_raw: c(90, 5, 3, 4119, 822, 2639),
            ..Default::default()
        };
        let out = apply_gate(
            Some((prev, secs(10))),
            c(91, 6, 3, 5175, 968, 3227),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.elims, 91, "corroborated jump is accepted");
    }

    #[test]
    fn suspect_prior_read_never_corroborates_a_jump() {
        // Same as above, but the prior raw 90 was SUSPECT (edge ink) — it must
        // NOT corroborate the current 91, so the ghost jump is held at 9. This is
        // the deterministic-clip resurrection guard on the upward path.
        let prev = GateState {
            accepted: c(9, 5, 3, 4119, 822, 2639),
            last_raw: c(90, 5, 3, 4119, 822, 2639),
            last_raw_suspect: [true, false, false, false, false, false],
            ..Default::default()
        };
        let out = apply_gate(
            Some((prev, secs(10))),
            c(91, 6, 3, 5175, 968, 3227),
            CLEAN,
            false,
        );
        assert_eq!(
            out.accepted.elims, 9,
            "suspect prior read must not corroborate"
        );
    }

    #[test]
    fn plausible_stomp_passes_ungated() {
        // The one-sided guarantee: a real fast climb within rate is never held.
        let prev = state(c(22, 10, 5, 9906, 1898, 5958));
        let out = apply_gate(
            Some((prev, secs(90))),
            c(28, 17, 10, 12672, 2544, 6428),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.elims, 28);
        assert_eq!(out.accepted.deaths, 10);
        assert!(out.holds.is_empty(), "a genuine stomp produces no holds");
    }

    // --- Real-series replay fixtures (07-18 night shift, numeric only) ---

    /// (elims, assists, deaths, damage, healing, mitigation, elapsed_secs_since_prev)
    type Step = (u32, u32, u32, u32, u32, u32, u64);

    /// Replay a raw series through the gate (no split fires — these are single
    /// sessions in the store) and return the accepted counters per step.
    ///
    /// `clips` lists `(step_index, col)` cells that the fixture comments identify
    /// as a clip/collapse — the reads edge-ink (b) would flag `suspect`. Marking
    /// them here is faithful to what the live pipeline now feeds the gate: a
    /// collapse like E "13"→"1" or a tail-clip HLG "2241"→"224" jams a stroke
    /// against a crop edge. Suspect reads are held but never build the un-latch
    /// streak, so a persistent clip can never talk the gate out of a correct hold.
    fn replay_clips(series: &[Step], clips: &[(usize, usize)]) -> Vec<Counters> {
        let mut state: Option<(GateState, Duration)> = None;
        let mut accepted = Vec::new();
        for (i, &(e, a, d, dmg, hlg, mit, gap)) in series.iter().enumerate() {
            let raw = c(e, a, d, dmg, hlg, mit);
            let mut suspect = [false; GATE_COLS];
            for &(ci, col) in clips {
                if ci == i {
                    suspect[col] = true;
                }
            }
            let prev = state.map(|(s, _): (GateState, Duration)| (s, Duration::from_secs(gap)));
            let out = apply_gate(prev, raw, suspect, false);
            accepted.push(out.accepted);
            state = Some((out.state, Duration::from_secs(gap)));
        }
        accepted
    }

    // Havana Victory b1b263e994d1f7f8 — real matches.jsonl series. Raw elims
    // spikes to 91 (ghost) then collapses to 1 for ~14 captures; final captures
    // clip deaths (6→5) and healing (1898→224/234). The corrupt STORED finals
    // were D=5, HLG=234.
    const HAVANA: &[Step] = &[
        (0, 0, 1, 499, 214, 0, 0),
        (0, 0, 1, 499, 214, 0, 10),
        (1, 1, 2, 975, 347, 850, 60),
        (1, 1, 3, 1172, 397, 0, 19),
        (1, 1, 3, 1172, 397, 0, 5),
        (4, 3, 3, 2013, 366, 4, 73),
        (5, 5, 3, 3862, 772, 2453, 111),
        (9, 5, 3, 4119, 822, 2639, 20),
        (91, 6, 3, 5175, 968, 3227, 75), // ghost inflation
        (1, 6, 3, 5175, 968, 3227, 14),  // collapse
        (13, 6, 3, 5175, 968, 3227, 17), // real E=13 recovered
        (1, 6, 3, 5175, 968, 3227, 36),
        (1, 6, 3, 5175, 968, 3227, 27),
        (1, 6, 3, 5175, 968, 3227, 9),
        (1, 6, 4, 5290, 1018, 3377, 23),
        (1, 6, 4, 5457, 1018, 3522, 27),
        (1, 6, 4, 5457, 1018, 3522, 6),
        (1, 6, 4, 5457, 1018, 3522, 6),
        (1, 6, 4, 5729, 1120, 3697, 39),
        (1, 7, 5, 5844, 1320, 3972, 32),
        (1, 7, 5, 6352, 1470, 4149, 63),
        (1, 7, 6, 6622, 1520, 4549, 33),
        (1, 7, 6, 6622, 1520, 4549, 7),
        (1, 7, 6, 6622, 1520, 4549, 8),
        (1, 7, 6, 6622, 1520, 4549, 11),
        (7, 9, 5, 7219, 1598, 4999, 45), // D clip 6→5
        (9, 9, 5, 8561, 1898, 5199, 71),
        (22, 10, 5, 9906, 224, 5958, 85),  // HLG clip 2241→224
        (23, 10, 5, 10009, 234, 5958, 13), // HLG clip 2341→234, D clip 6→5
        (23, 10, 5, 10009, 234, 5958, 4),
        (23, 10, 5, 10009, 234, 5958, 19),
    ];

    /// Clips edge-ink (b) would flag in HAVANA: every E "…"→"1" collapse (col 0),
    /// the persistent D 6→5 clip (col 2), and the HLG 2241/2341 tail-clips (col 4).
    /// Marking them faithful to the live pipeline keeps them from driving un-latch.
    const HAVANA_CLIPS: &[(usize, usize)] = &[
        (9, 0),
        (11, 0),
        (12, 0),
        (13, 0),
        (14, 0),
        (15, 0),
        (16, 0),
        (17, 0),
        (18, 0),
        (19, 0),
        (20, 0),
        (21, 0),
        (22, 0),
        (23, 0),
        (24, 0),
        (25, 2),
        (26, 2),
        (27, 2),
        (28, 2),
        (29, 2),
        (30, 2),
        (27, 4),
        (28, 4),
        (29, 4),
        (30, 4),
    ];

    #[test]
    fn havana_series_gate_holds_every_collapse_and_ghost() {
        let acc = replay_clips(HAVANA, HAVANA_CLIPS);

        // Elims: ghost 91 (idx 8) never stored; after 13 is seen (idx 10) elims
        // never drops back to 1; final settles at 23.
        let elims: Vec<u32> = acc.iter().map(|c| c.elims).collect();
        assert_eq!(elims[8], 9, "ghost 91 held at 9");
        assert_eq!(elims[9], 9, "collapse to 1 held at 9");
        assert_eq!(elims[10], 13, "real E=13 accepted");
        for (i, &e) in elims.iter().enumerate().skip(10) {
            assert!(
                e >= 11,
                "elims collapsed to {e} at step {i} after 13 was seen"
            );
        }
        // No stored 9X ghost anywhere.
        assert!(
            elims.iter().all(|&e| !(90..=99).contains(&e)),
            "a 9X ghost was stored: {elims:?}"
        );

        let final_c = *acc.last().unwrap();
        // Gate finals BEAT the corrupt stored finals (D was 5, HLG was 234).
        assert_eq!(final_c.elims, 23, "final elims");
        assert_eq!(
            final_c.deaths, 6,
            "final deaths recovered to 6 (stored was 5)"
        );
        assert_eq!(
            final_c.healing, 1898,
            "final healing holds ≥ last good 1898 (stored was clipped 234)"
        );
        assert!(
            final_c.healing > 234,
            "gate healing must beat the corrupt stored 234"
        );
        assert_eq!(final_c.damage, 10009, "damage read cleanly throughout");
        assert_eq!(final_c.assists, 10);
        assert_eq!(final_c.mitigation, 5958);
    }

    // Route 66 Defeat b1b265a623ed99c6 — real series. Elims oscillate
    // 1↔11/12/15 then climb 22→28; screenshot-verified real final E28 D10 A17.
    // Note the 22:07:50 row-shift glitch (raw E9/A19/D13 reads a different
    // player's row) — mode (c), OUT OF SCOPE for this gate.
    const ROUTE66: &[Step] = &[
        (0, 0, 0, 0, 0, 0, 0),
        (5, 2, 0, 1506, 400, 474, 99),
        (5, 2, 0, 1614, 460, 854, 35),
        (7, 3, 1, 3357, 689, 388, 91),
        (7, 3, 1, 3673, 689, 388, 3),
        (7, 3, 1, 3673, 689, 388, 17),
        (7, 3, 1, 4385, 989, 1938, 66),
        (7, 3, 2, 4385, 989, 2038, 25),
        (7, 3, 2, 4385, 989, 2038, 6),
        (7, 3, 2, 4385, 1039, 2038, 11),
        (7, 3, 2, 4385, 1039, 2038, 5),
        (5, 3, 2, 5232, 1139, 2568, 44),  // E collapse 7→5
        (1, 6, 4, 5943, 1289, 2927, 43),  // E collapse →1
        (11, 6, 4, 5943, 1289, 2927, 44), // real E=11
        (1, 6, 4, 5943, 1289, 2927, 24),
        (1, 7, 6, 6429, 1485, 3363, 96),
        (1, 7, 6, 6429, 1485, 3363, 6),
        (1, 7, 7, 7103, 1565, 3763, 34),
        (12, 7, 7, 7103, 1565, 3763, 5), // real E=12
        (12, 7, 7, 7103, 1565, 3763, 4),
        (12, 7, 7, 7103, 1565, 3763, 17),
        (15, 10, 7, 7677, 1731, 3863, 46), // real E=15
        (1, 1, 8, 8360, 1881, 4138, 44),   // E and A collapse
        (1, 1, 8, 8360, 1881, 4138, 7),
        (1, 1, 8, 8360, 1881, 4138, 7),
        (1, 1, 8, 8769, 2131, 4488, 55),
        (9, 19, 13, 9921, 2181, 4648, 32), // row-shift glitch (mode c)
        (9, 19, 13, 9921, 2181, 4648, 11),
        (9, 19, 13, 9921, 2181, 4648, 3),
        (22, 14, 5, 10065, 2244, 5065, 41), // real player row again
        (22, 14, 5, 10065, 2244, 5065, 2),
        (22, 14, 5, 10065, 2244, 5065, 12),
        (22, 14, 5, 10470, 2344, 5664, 29),
        (28, 17, 10, 12672, 2544, 6428, 90), // real final elims
        (28, 17, 10, 12672, 2544, 6428, 3),
    ];

    /// Clips edge-ink (b) would flag in ROUTE66: every E "…"→"5"/"1" collapse
    /// (col 0). The 22:07:50 row-shift (idx 26-28) is mode (c) — a whole
    /// different player's row, which per-glyph edge-ink does NOT detect — but its
    /// E=9 sits below the held 15, so leaving it "clean" would let the un-latch
    /// misfire and transiently drop elims to 9. We mark the row-shift's E cell
    /// suspect to keep that out-of-scope failure from perturbing the elims path;
    /// the A/D recovery below comes from the LEGIT returning reads (idx 29+),
    /// which are not marked.
    const ROUTE66_CLIPS: &[(usize, usize)] = &[
        (11, 0),
        (12, 0),
        (14, 0),
        (15, 0),
        (16, 0),
        (17, 0),
        (22, 0),
        (23, 0),
        (24, 0),
        (25, 0),
        (22, 1),
        (23, 1),
        (24, 1),
        (25, 1),
        (26, 0),
        (27, 0),
        (28, 0),
    ];

    #[test]
    fn route66_series_elims_never_collapse_and_stomps_pass() {
        let acc = replay_clips(ROUTE66, ROUTE66_CLIPS);
        let elims: Vec<u32> = acc.iter().map(|c| c.elims).collect();

        // Every real elims level (11, 12, 15, 22, 28) is accepted...
        assert_eq!(elims[13], 11, "real E=11 accepted");
        assert_eq!(elims[18], 12, "real E=12 accepted");
        assert_eq!(elims[21], 15, "real E=15 accepted");
        assert_eq!(elims[29], 22, "real E=22 accepted");
        assert_eq!(*elims.last().unwrap(), 28, "real final E=28 accepted");

        // ...and elims never collapses back to 1 once 11 has been seen.
        for (i, &e) in elims.iter().enumerate().skip(13) {
            assert!(
                e >= 11,
                "elims collapsed to {e} at step {i} after 11 was seen"
            );
        }
    }

    #[test]
    fn route66_row_shift_returns_to_the_real_assists_and_deaths() {
        // The row-shift reads assists 19 and deaths 13 while elims are held
        // down. That high is a different row, so the real row's 17 and 10
        // replace it. A value confirmed by two matching reads, with no row
        // shift, still cannot fall.
        let acc = replay_clips(ROUTE66, ROUTE66_CLIPS);
        let final_c = *acc.last().unwrap();
        assert_eq!(final_c.elims, 28, "elims unaffected by the row-shift");
        assert_eq!(final_c.assists, 17, "assists return to the real row");
        assert_eq!(final_c.deaths, 10, "deaths return to the real row");
    }

    // --- CG-2/CG-3 injection + latch + un-latch (the 2026-07-20 defect) ---

    /// Antarctic Peninsula, damage column: real DMG climbs 1728→2559→3235, then
    /// the stat window drifts one digit and injects the deaths digit ("3") in
    /// front of a clipped DMG → 35031 (CG-3). Pre-CG-4 the inject latched and
    /// un-latch recovered; post-CG-4 B1 (suspect rate) + B2 (trailing digit)
    /// reject the inject at acceptance so the later clean climb is never blocked.
    /// Final still settles at the real 10311 either path.
    ///
    /// `inject_suspect` models both ways it reached the gate: `true` — edge-ink
    /// flags the drifted read; `false` — it slipped through as clean (B2 still
    /// holds the digit inject).
    fn antarctic_damage_recovers(inject_suspect: bool) -> u32 {
        // (e, a, d, dmg, hlg, mit, gap), then per-step suspect on the DMG col (3).
        let series: &[Step] = &[
            (4, 2, 1, 1728, 0, 900, 0),
            (6, 2, 2, 2559, 0, 1400, 30),
            (8, 2, 3, 3235, 0, 1800, 25),
            (8, 2, 4, 35031, 0, 2100, 20), // CG-3 injection: "3"+clip(5_31)
            (9, 2, 4, 4543, 0, 2300, 18),  // clean again — latched-out as a decrease
            (10, 2, 4, 5852, 0, 2600, 22),
            (11, 2, 4, 6810, 0, 2900, 24),
            (12, 2, 5, 10311, 0, 3400, 60),
        ];
        let inject_idx = 3usize;
        let mut state: Option<(GateState, Duration)> = None;
        for (i, &(e, a, d, dmg, hlg, mit, gap)) in series.iter().enumerate() {
            let raw = c(e, a, d, dmg, hlg, mit);
            let mut suspect = [false; GATE_COLS];
            if i == inject_idx && inject_suspect {
                suspect[3] = true;
            }
            let prev = state.map(|(s, _): (GateState, Duration)| (s, Duration::from_secs(gap)));
            let out = apply_gate(prev, raw, suspect, false);
            state = Some((out.state, Duration::from_secs(gap)));
        }
        state.expect("series is non-empty").0.accepted.damage
    }

    #[test]
    fn antarctic_injection_latch_unlatches_to_clean_damage() {
        // Both injection variants recover: the latched 35031 is revised down by
        // the run of clean reads and the final settles at the real 10311.
        for inject_suspect in [true, false] {
            let dmg = antarctic_damage_recovers(inject_suspect);
            assert_eq!(
                dmg, 10311,
                "damage must recover to the clean level (inject_suspect={inject_suspect})"
            );
            assert_ne!(dmg, 35031, "the injected inflation must not survive");
        }
    }

    /// Junkertown assists column: A latches high at 14 (an inflation that passed
    /// the kill-column rate cap), then the real row reads a steady clean A=4 for
    /// several captures. Three consecutive clean below-held reads un-latch A back
    /// to 4 — the constant-run case, the counterpart to Antarctic's climbing run.
    #[test]
    fn junkertown_assists_stay_held_across_a_constant_lower_run() {
        // Seed A latched at 14 with damage advancing normally around it.
        let mut st = state(c(9, 14, 4, 6000, 0, 3000));
        let mut a_vals = Vec::new();
        for (i, gap) in [20u64, 22, 24, 26].into_iter().enumerate() {
            let raw = c(9, 4, 4, 6200 + i as u32 * 50, 0, 3100);
            let out = apply_gate(Some((st, secs(gap))), raw, CLEAN, false);
            a_vals.push(out.accepted.assists);
            assert!(unsure_fields(&out.holds).contains(&"a"));
            st = out.state;
        }
        assert_eq!(
            a_vals,
            vec![14, 14, 14, 14],
            "a constant lower run does not revise assists down"
        );
    }

    #[test]
    fn three_lower_reads_do_not_revise_a_confirmed_value_down() {
        let prev = state(c(2, 2, 2, 20000, 0, 0));
        let out1 = apply_gate(Some((prev, secs(15))), c(2, 2, 2, 5000, 0, 0), CLEAN, false);
        assert_eq!(out1.accepted.damage, 20000);
        let out2 = apply_gate(
            Some((out1.state, secs(15))),
            c(2, 2, 2, 5200, 0, 0),
            CLEAN,
            false,
        );
        assert_eq!(out2.accepted.damage, 20000);
        let out3 = apply_gate(
            Some((out2.state, secs(15))),
            c(2, 2, 2, 5400, 0, 0),
            CLEAN,
            false,
        );
        assert_eq!(out3.accepted.damage, 20000);
        assert!(out3.unlatches.is_empty());
        assert!(unsure_fields(&out3.holds).contains(&"dmg"));
    }

    #[test]
    fn elims_drop_from_11_to_1_stays_held() {
        // Field case: elims 11 were read as 1. Three clean reads of 1 used
        // to un-latch the cell down. The stored value stays 11, and elims
        // is unsure.
        let mut st = state(c(11, 4, 2, 3000, 800, 400));
        for _ in 0..3 {
            let out = apply_gate(
                Some((st, secs(20))),
                c(1, 4, 2, 3100, 800, 400),
                CLEAN,
                false,
            );
            assert_eq!(out.accepted.elims, 11);
            assert!(out.unlatches.is_empty());
            assert_eq!(unsure_fields(&out.holds), vec!["e"]);
            assert!(out.holds.iter().any(|h| h.col == 0
                && h.kind == HoldKind::Monotonic
                && h.raw == 1
                && h.held == 11));
            st = out.state;
        }
    }

    #[test]
    fn elims_jump_from_13_to_18_waits_and_a_correction_drops_it() {
        let prev = state(c(13, 4, 2, 3000, 800, 400));
        let jumped = apply_gate(
            Some((prev, secs(20))),
            c(18, 4, 2, 3100, 800, 400),
            CLEAN,
            false,
        );
        assert_eq!(jumped.accepted.elims, 13, "one 18 does not count");
        assert_eq!(unsure_fields(&jumped.holds), vec!["e"]);
        assert!(
            jumped
                .holds
                .iter()
                .any(|h| h.kind == HoldKind::Jump && h.raw == 18 && h.held == 13)
        );
        let corrected = apply_gate(
            Some((jumped.state, secs(20))),
            c(14, 4, 2, 3200, 820, 400),
            CLEAN,
            false,
        );
        assert_eq!(
            corrected.accepted.elims, 14,
            "the correcting read is stored"
        );
        assert!(
            corrected.holds.iter().all(|h| h.col != 0),
            "the 18 is not latched: {:?}",
            corrected.holds
        );
        let again = apply_gate(
            Some((prev, secs(20))),
            c(18, 4, 2, 3100, 800, 400),
            CLEAN,
            false,
        );
        let matched = apply_gate(
            Some((again.state, secs(20))),
            c(18, 4, 2, 3200, 800, 400),
            CLEAN,
            false,
        );
        assert_eq!(matched.accepted.elims, 18, "a second matching 18 counts");
    }

    #[test]
    fn each_stat_stores_at_the_jump_limit_and_waits_one_past() {
        let base = [10u32, 10, 10, 8000, 8000, 8000];
        for col in 0..GATE_COLS {
            let mut at_limit = base;
            at_limit[col] = base[col] + JUMP_MAX[col];
            let stored = apply_gate(
                Some((state(Counters::from_array(base)), secs(30))),
                Counters::from_array(at_limit),
                CLEAN,
                false,
            );
            assert_eq!(
                stored.accepted.to_array()[col],
                at_limit[col],
                "col {col} at JUMP_MAX stores"
            );
            assert!(
                stored.holds.iter().all(|h| h.col != col),
                "col {col} at the limit is not held: {:?}",
                stored.holds
            );

            let mut one_past = base;
            one_past[col] = base[col] + JUMP_MAX[col] + 1;
            let waited = apply_gate(
                Some((state(Counters::from_array(base)), secs(30))),
                Counters::from_array(one_past),
                CLEAN,
                false,
            );
            assert_eq!(
                waited.accepted.to_array()[col],
                base[col],
                "col {col} one past JUMP_MAX waits"
            );
            assert!(
                waited
                    .holds
                    .iter()
                    .any(|h| h.col == col && h.kind == HoldKind::Jump),
                "col {col} one past is a jump hold: {:?}",
                waited.holds
            );
        }
    }

    #[test]
    fn damage_burst_of_3501_stores() {
        // 6810 to 10311 is under the damage limit of 4000, so a clean burst
        // stores on the first read.
        let prev = state(c(11, 2, 4, 6810, 0, 2900));
        let out = apply_gate(
            Some((prev, secs(60))),
            c(12, 2, 5, 10311, 0, 3400),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.damage, 10311);
        assert!(
            out.holds.iter().all(|h| h.col != 3),
            "a +3501 damage burst is not held: {:?}",
            out.holds
        );
    }

    #[test]
    fn a_row_shift_high_can_fall_and_a_matched_high_cannot() {
        // Elims drop while assists and deaths jump past the limit. Assists
        // sit next to that drop, and deaths sit next to assists, so both are
        // one shifted run. Two reads of the real row replace it. One does
        // not. A matched assists with no row shift stays.
        let prev = state(c(15, 10, 7, 7000, 1500, 3000));
        let shifted = apply_gate(
            Some((prev, secs(30))),
            c(9, 19, 13, 8000, 1600, 3200),
            CLEAN,
            false,
        );
        assert_eq!(shifted.accepted.elims, 15, "the dropped elims stay");
        assert_eq!(shifted.accepted.assists, 19);
        assert_eq!(shifted.accepted.deaths, 13);
        assert!(
            shifted.state.row_shift[1],
            "assists sit next to the elims drop"
        );
        assert!(
            shifted.state.row_shift[2],
            "deaths continue that same adjacent run"
        );
        let again = apply_gate(
            Some((shifted.state, secs(10))),
            c(9, 19, 13, 8100, 1600, 3200),
            CLEAN,
            false,
        );
        assert!(again.state.row_shift[1], "a repeated shift stays a shift");
        let once = apply_gate(
            Some((again.state, secs(20))),
            c(16, 11, 7, 8200, 1700, 3300),
            CLEAN,
            false,
        );
        assert_eq!(once.accepted.assists, 19, "one lower read keeps the shift");
        assert_eq!(once.accepted.deaths, 13, "one lower read keeps the shift");
        assert!(unsure_fields(&once.holds).contains(&"a"));
        assert!(unsure_fields(&once.holds).contains(&"d"));
        let real = apply_gate(
            Some((once.state, secs(20))),
            c(16, 11, 7, 8300, 1700, 3300),
            CLEAN,
            false,
        );
        assert_eq!(
            real.accepted.assists, 11,
            "the second read replaces the shift"
        );
        assert_eq!(
            real.accepted.deaths, 7,
            "the second read replaces the shift"
        );
        let matched = apply_gate(
            Some((state(c(16, 11, 7, 8200, 1700, 3300)), secs(20))),
            c(16, 11, 7, 8300, 1700, 3300),
            CLEAN,
            false,
        );
        let held = apply_gate(
            Some((matched.state, secs(20))),
            c(16, 8, 7, 8400, 1700, 3300),
            CLEAN,
            false,
        );
        assert_eq!(held.accepted.assists, 11, "two matching reads cannot fall");
        assert!(unsure_fields(&held.holds).contains(&"a"));
    }

    #[test]
    fn a_row_shift_stays_on_one_low_read() {
        // Elims drop and the next cell, assists, jumps. That is a row shift.
        // One later read of 1 keeps 19 and flags assists.
        let prev = state(c(15, 10, 7, 7000, 1500, 3000));
        let shifted = apply_gate(
            Some((prev, secs(30))),
            c(9, 19, 7, 8000, 1600, 3200),
            CLEAN,
            false,
        );
        assert!(shifted.state.row_shift[1]);
        assert_eq!(shifted.accepted.assists, 19);
        let one = apply_gate(
            Some((shifted.state, secs(20))),
            c(15, 1, 7, 8100, 1600, 3200),
            CLEAN,
            false,
        );
        assert_eq!(one.accepted.assists, 19, "one read of 1 keeps the shift");
        assert!(unsure_fields(&one.holds).contains(&"a"));
        assert!(one.unlatches.is_empty());
    }

    #[test]
    fn a_drop_far_from_a_rise_is_not_a_row_shift() {
        // Mitigation falls in the same frame elims jump. Those columns are
        // not adjacent, so the elims rise is not tagged as a row shift.
        let prev = state(c(10, 4, 2, 3000, 800, 2000));
        let out = apply_gate(
            Some((prev, secs(30))),
            c(20, 4, 2, 3200, 800, 100),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.mitigation, 2000, "the mitigation drop is held");
        assert!(
            !out.state.row_shift.iter().any(|&tagged| tagged),
            "a far drop does not tag the rise: {:?}",
            out.state.row_shift
        );
    }

    #[test]
    fn an_unconfirmed_high_stays_on_one_low_read_and_falls_on_two_matching_reads() {
        // Fallback elims 11 never got a second read. One read of 1 keeps 11
        // and flags elims. Two reads of 9 then store 9.
        let latched = apply_gate_with_trust(None, c(11, 4, 2, 3000, 800, 400), CLEAN, false, false);
        assert!(latched.state.unconfirmed[0]);
        assert_eq!(latched.accepted.elims, 11);
        let one = apply_gate_with_trust(
            Some((latched.state, secs(20))),
            c(1, 4, 2, 3100, 800, 400),
            CLEAN,
            false,
            true,
        );
        assert_eq!(one.accepted.elims, 11);
        assert!(unsure_fields(&one.holds).contains(&"e"));
        let nine = apply_gate_with_trust(
            Some((one.state, secs(20))),
            c(9, 4, 2, 3200, 800, 400),
            CLEAN,
            false,
            true,
        );
        assert_eq!(nine.accepted.elims, 11, "one read of 9 is not a pair");
        assert!(unsure_fields(&nine.holds).contains(&"e"));
        let lowered = apply_gate_with_trust(
            Some((nine.state, secs(20))),
            c(9, 4, 2, 3300, 800, 400),
            CLEAN,
            false,
            true,
        );
        assert_eq!(lowered.accepted.elims, 9);
        assert!(
            lowered
                .unlatches
                .iter()
                .any(|u| u.col == 0 && u.raw == 9 && u.replaced_unconfirmed)
        );
        let after = apply_gate_with_trust(
            Some((lowered.state, secs(20))),
            c(1, 4, 2, 3400, 800, 400),
            CLEAN,
            false,
            true,
        );
        assert_eq!(after.accepted.elims, 9, "the matched 9 is confirmed");
        assert!(unsure_fields(&after.holds).contains(&"e"));
    }

    #[test]
    fn ordinary_growth_across_columns_is_not_a_single_field_jump() {
        // E4 to E9 over three minutes, with assists and deaths up too.
        // That is play, not one misread cell.
        let prev = state(c(4, 3, 2, 1500, 400, 300));
        let out = apply_gate(
            Some((prev, secs(180))),
            c(9, 7, 4, 3800, 900, 800),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.elims, 9);
        assert!(out.holds.iter().all(|h| h.kind != HoldKind::Jump));
    }

    #[test]
    fn suspect_below_reads_never_unlatch() {
        // The resurrection guard: a persistent CLIP (suspect) below a correct
        // held value must never un-latch it, no matter how many frames it repeats.
        let prev = state(c(2, 2, 2, 0, 1898, 0));
        let clip = [false, false, false, false, true, false]; // HLG suspect
        let mut st = (prev, secs(15));
        for _ in 0..6 {
            let out = apply_gate(Some((st.0, secs(15))), c(2, 2, 2, 0, 234, 0), clip, false);
            assert_eq!(
                out.accepted.healing, 1898,
                "suspect clip must never un-latch"
            );
            assert!(out.unlatches.is_empty());
            st = (out.state, secs(15));
        }
    }

    // --- CG-4 B1 / B2: wide-col suspect rate + trailing-digit inject ---

    #[test]
    fn cg4_b1_suspect_wide_advance_is_rate_capped() {
        // Field-shaped: HLG accepted 2782, raw 22994 (MIT digit bleed), edge-ink
        // suspect, short gap. Pre-CG-4 latched 22994; B1 holds at 2782.
        let prev = state(c(12, 4, 3, 8000, 2782, 5119));
        let hlg_suspect = [false, false, false, false, true, false];
        let out = apply_gate(
            Some((prev, secs(20))),
            c(12, 4, 3, 8200, 22994, 5200),
            hlg_suspect,
            false,
        );
        assert_eq!(out.accepted.healing, 2782, "suspect HLG inject held");
        assert!(
            out.holds
                .iter()
                .any(|h| h.col == 4 && h.kind == HoldKind::RateCap)
        );
        assert_eq!(out.accepted.damage, 8200, "clean DMG advance kept");
    }

    #[test]
    fn cg4_b1_clean_wide_burst_still_uncapped() {
        // One-sided guarantee: a clean multi-k damage climb must never hold.
        let prev = state(c(12, 4, 3, 6810, 2000, 3000));
        let out = apply_gate(
            Some((prev, secs(60))),
            c(12, 4, 3, 10311, 2200, 3400),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.damage, 10311);
        assert!(
            out.holds.iter().all(|h| h.col != 3),
            "clean wide burst must not RateCap: {:?}",
            out.holds
        );
    }

    #[test]
    fn cg4_b1_genuine_suspect_wide_stomp_passes() {
        // Route66-scale DMG climb (+2202 / 90s) even if flagged suspect must pass.
        let prev = state(c(22, 14, 5, 10470, 2344, 5664));
        let dmg_suspect = [false, false, false, true, false, false];
        let out = apply_gate(
            Some((prev, secs(90))),
            c(28, 17, 10, 12672, 2544, 6428),
            dmg_suspect,
            false,
        );
        assert_eq!(out.accepted.damage, 12672, "genuine suspect stomp passes");
        assert!(
            !out.holds.iter().any(|h| h.col == 3),
            "no hold on in-rate suspect DMG: {:?}",
            out.holds
        );
    }

    #[test]
    fn cg4_b2_trailing_digit_inject_holds() {
        // Plan example: 1681 → 22994 (prefix 2299 ≥ 1681, +1 digit).
        let prev = state(c(5, 2, 1, 4000, 1681, 2000));
        let out = apply_gate(
            Some((prev, secs(15))),
            c(5, 2, 1, 4100, 22994, 2100),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.healing, 1681);
        assert!(
            out.holds
                .iter()
                .any(|h| h.col == 4 && h.kind == HoldKind::DigitInject)
        );
    }

    #[test]
    fn cg4_b2_real_digit_rollover_does_not_fire() {
        // Plan counterexample: 9906 → 10311 (prefix 1031 < accepted).
        let prev = state(c(22, 10, 5, 9906, 1898, 5958));
        let out = apply_gate(
            Some((prev, secs(13))),
            c(23, 10, 5, 10311, 1898, 5958),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.damage, 10311);
        assert!(
            !out.holds.iter().any(|h| h.kind == HoldKind::DigitInject),
            "real rollover must not DigitInject: {:?}",
            out.holds
        );
    }

    #[test]
    fn cg4_b2_primary_field_case_needs_b1_not_digit_alone() {
        // Review clarify: 2782 → 22994 does NOT fire B2 (prefix 2299 < 2782).
        // Without suspect, gate still accepts (geometry/B1 must cover).
        let prev = state(c(12, 4, 3, 8000, 2782, 5119));
        let out = apply_gate(
            Some((prev, secs(20))),
            c(12, 4, 3, 8200, 22994, 5200),
            CLEAN,
            false,
        );
        assert!(
            !out.holds
                .iter()
                .any(|h| h.col == 4 && h.kind == HoldKind::DigitInject),
            "B2 must not claim the 2782→22994 primary case"
        );
        // The rise is one column past JUMP_MAX, so the first read stays held.
        // A repeated clean inject still stores on the matching second read,
        // which is why a suspect flag (B1) is still required for that case.
        assert_eq!(out.accepted.healing, 2782, "the first inject waits");
        assert!(
            out.holds
                .iter()
                .any(|h| h.col == 4 && h.kind == HoldKind::Jump)
        );
        let again = apply_gate(
            Some((out.state, secs(20))),
            c(12, 4, 3, 8200, 22994, 5200),
            CLEAN,
            false,
        );
        assert_eq!(
            again.accepted.healing, 22994,
            "a repeated clean inject stores on the second read"
        );
    }

    #[test]
    fn cg4_b2_sparse_tab_genuine_10x_growth_does_not_latch() {
        // Claude MED B2-FP (a): acc 1681, genuine 16900 after ≥10 min Tab gap.
        // Pre-fix: +1 digit + prefix 1690≥1681 → DigitInject forever.
        // With rate conjunct: delta 15219 @ 600s < ceiling 50500 → pass.
        let prev = state(c(5, 2, 1, 4000, 1681, 2000));
        let out = apply_gate(
            Some((prev, secs(600))),
            c(8, 3, 2, 12000, 16900, 5000),
            CLEAN,
            false,
        );
        assert_eq!(
            out.accepted.healing, 16900,
            "genuine long-gap climb must pass"
        );
        assert!(
            !out.holds
                .iter()
                .any(|h| h.col == 4 && h.kind == HoldKind::DigitInject),
            "sparse-Tab 10x must not DigitInject: {:?}",
            out.holds
        );
    }

    #[test]
    fn cg4_b2_near_zero_one_to_two_digit_does_not_fire() {
        // Claude MED B2-FP (b): acc 0..9, genuine cur 10..99 early game.
        // Delta ≤99 < WIDE_RATE_SLACK 2500 → never fires even at short gap.
        let prev = state(c(0, 0, 0, 50, 5, 0));
        let out = apply_gate(
            Some((prev, secs(10))),
            c(1, 0, 0, 200, 47, 100),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.healing, 47);
        assert!(
            !out.holds.iter().any(|h| h.kind == HoldKind::DigitInject),
            "near-zero 1→2 digit climb must not DigitInject: {:?}",
            out.holds
        );
    }

    /// Synthetic 07-22 Numbani-shaped HLG series (string-level, from field
    /// notes): climbs to ~2782, injects 22994 as edge-ink suspect, then clean
    /// reads resume near the real level. B1 must hold the inject; finals must
    /// not settle at 22994. Havana/Route66/Antarctic fixtures stay green above.
    #[test]
    fn cg4_b4_numbani_hlg_suspect_inject_held() {
        // (e, a, d, dmg, hlg, mit, gap)
        let series: &[Step] = &[
            (4, 1, 1, 2000, 400, 800, 0),
            (6, 2, 1, 3500, 900, 1500, 40),
            (8, 2, 2, 5200, 1600, 2800, 50),
            (10, 3, 2, 7000, 2200, 4000, 45),
            (12, 4, 3, 8000, 2782, 5119, 40),  // last good HLG ~2782
            (12, 4, 3, 8200, 22994, 5200, 20), // inject (suspect)
            (13, 4, 3, 8500, 2900, 5400, 25),  // clean recovery reads
            (14, 5, 3, 9000, 3100, 5600, 30),
            (15, 5, 4, 9500, 3300, 5800, 28),
        ];
        let inject_idx = 5usize;
        let mut st: Option<(GateState, Duration)> = None;
        let mut hlg_hist = Vec::new();
        for (i, &(e, a, d, dmg, hlg, mit, gap)) in series.iter().enumerate() {
            let raw = c(e, a, d, dmg, hlg, mit);
            let mut suspect = [false; GATE_COLS];
            if i == inject_idx {
                suspect[4] = true; // HLG edge-ink
            }
            let prev = st.map(|(s, _)| (s, Duration::from_secs(gap)));
            let out = apply_gate(prev, raw, suspect, false);
            hlg_hist.push(out.accepted.healing);
            st = Some((out.state, Duration::from_secs(gap)));
        }
        assert_eq!(hlg_hist[4], 2782);
        assert_eq!(hlg_hist[5], 2782, "suspect inject must not latch");
        assert_eq!(*hlg_hist.last().unwrap(), 3300, "clean climb resumes");
        assert!(
            hlg_hist.iter().all(|&h| h != 22994),
            "22994 must never be stored: {hlg_hist:?}"
        );
    }

    // ----- alpha.7 O10: cells the reader did not vouch for (`held`) -----

    /// Column 4 (healing) held, nothing else.
    const HELD_H: [bool; GATE_COLS] = [false, false, false, false, true, false];

    #[test]
    fn a_held_column_keeps_the_accepted_value_whatever_the_read_says() {
        let prev = state(c(11, 4, 2, 3000, 800, 400));
        // the reader's unsure guess for healing is 8000; it is not a number
        let gate = apply_gate_held(
            Some((prev, secs(20))),
            c(12, 4, 2, 3200, 8000, 450),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert_eq!(gate.accepted, c(12, 4, 2, 3200, 800, 450));
        assert_eq!(
            gate.holds,
            vec![Hold {
                col: 4,
                kind: HoldKind::Unread,
                raw: 8000,
                held: 800,
            }]
        );
        assert_eq!(unsure_fields(&gate.holds), vec!["h"], "the cell is flagged");
        // the guess is not the previous raw read of the next capture
        assert_eq!(gate.state.last_raw, c(12, 4, 2, 3200, 800, 450));
        assert!(!gate.state.last_raw_suspect[4]);
        assert!(!gate.state.low_trust && gate.state.unread == [false; GATE_COLS]);
    }

    #[test]
    fn a_held_read_never_corroborates_the_next_jump() {
        let prev = state(c(11, 4, 2, 3000, 800, 400));
        let held = apply_gate_held(
            Some((prev, secs(20))),
            c(11, 4, 2, 3000, 5000, 400),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        // the next sure read is the same 5000, alone on a quiet board: with
        // the held 5000 as its previous raw read it would be corroborated
        let next = apply_gate(
            Some((held.state, secs(20))),
            c(11, 4, 2, 3000, 5000, 400),
            CLEAN,
            false,
        );
        assert_eq!(next.accepted.healing, 800, "one sure 5000 still waits");
        assert_eq!(next.holds[0].kind, HoldKind::Jump);
        // and a second sure 5000 is what stores it, as for any other jump
        let third = apply_gate(
            Some((next.state, secs(20))),
            c(11, 4, 2, 3000, 5000, 400),
            CLEAN,
            false,
        );
        assert_eq!(third.accepted.healing, 5000);
    }

    #[test]
    fn a_first_capture_stores_zero_for_a_held_column_and_then_its_first_read() {
        let held = [true, false, false, false, true, false];
        let first = apply_gate_held(None, c(7, 1, 0, 900, 351, 0), CLEAN, held, false, true);
        assert_eq!(first.accepted, c(0, 1, 0, 900, 0, 0), "no guess is stored");
        assert_eq!(first.state.unread, held);
        assert!(first.holds.is_empty() && !first.state.low_trust);
        // 20 s later both cells read sure. Against the placeholder 0 the 25
        // elims are past the rate cap and the 3511 healing is a lone jump.
        let second = apply_gate(
            Some((first.state, secs(20))),
            c(25, 1, 0, 950, 3511, 0),
            CLEAN,
            false,
        );
        assert_eq!(second.accepted, c(25, 1, 0, 950, 3511, 0));
        assert!(second.holds.is_empty(), "{:?}", second.holds);
        assert_eq!(second.state.unread, [false; GATE_COLS]);
        assert_eq!(second.state.row_shift, [false; GATE_COLS]);
        // from here the column is an ordinary one: a drop is held
        let third = apply_gate(
            Some((second.state, secs(20))),
            c(25, 1, 0, 950, 35, 0),
            CLEAN,
            false,
        );
        assert_eq!(third.accepted.healing, 3511);
    }

    #[test]
    fn a_column_held_twice_stays_unread_until_it_is_read() {
        let first = apply_gate_held(None, c(3, 1, 0, 900, 351, 0), CLEAN, HELD_H, false, true);
        let again = apply_gate_held(
            Some((first.state, secs(20))),
            c(3, 1, 0, 950, 77, 0),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert_eq!(again.accepted.healing, 0);
        assert!(again.state.unread[4]);
        let read = apply_gate(
            Some((again.state, secs(20))),
            c(3, 1, 0, 950, 3511, 0),
            CLEAN,
            false,
        );
        assert_eq!(read.accepted.healing, 3511);
    }

    #[test]
    fn the_first_read_of_an_unread_column_is_not_a_move_on_the_board() {
        // healing was never read; damage is at 3000
        let first = apply_gate_held(None, c(3, 1, 0, 3000, 0, 0), CLEAN, HELD_H, false, true);
        // healing reads 3000 for the first time while damage alone jumps
        // 5000: the first read does not make that jump one of two
        let second = apply_gate(
            Some((first.state, secs(20))),
            c(3, 1, 0, 8000, 3000, 0),
            CLEAN,
            false,
        );
        assert_eq!(second.accepted, c(3, 1, 0, 3000, 3000, 0));
        assert_eq!(
            second.holds,
            vec![Hold {
                col: 3,
                kind: HoldKind::Jump,
                raw: 8000,
                held: 3000,
            }]
        );
        // elims unread, then read as 20 on the frame where assists drop:
        // with a column down the first read is not taken at all (F266-2),
        // so it is no row-shift rise next to that drop either
        let held_e = [true, false, false, false, false, false];
        let first = apply_gate_held(None, c(0, 10, 2, 3000, 0, 0), CLEAN, held_e, false, true);
        let second = apply_gate(
            Some((first.state, secs(20))),
            c(20, 2, 2, 3000, 0, 0),
            CLEAN,
            false,
        );
        assert_eq!((second.accepted.elims, second.accepted.assists), (0, 10));
        assert!(second.state.unread[0], "still unread");
        assert!(
            !second.state.row_shift[0],
            "a first read is not a shifted row"
        );
        // the next board, with assists back, gives elims its first read
        let third = apply_gate(
            Some((second.state, secs(20))),
            c(21, 10, 2, 3100, 0, 0),
            CLEAN,
            false,
        );
        assert_eq!((third.accepted.elims, third.accepted.assists), (21, 10));
        assert!(third.holds.is_empty(), "{:?}", third.holds);
        assert!(!third.state.row_shift[0]);
    }

    #[test]
    fn an_untrusted_first_read_is_unconfirmed_and_leaves_no_floor() {
        let first = apply_gate_held(None, c(3, 1, 0, 900, 0, 0), CLEAN, HELD_H, false, true);
        let second = apply_gate_with_trust(
            Some((first.state, secs(20))),
            c(3, 1, 0, 900, 500, 0),
            CLEAN,
            false,
            false,
        );
        assert_eq!(second.accepted.healing, 500);
        assert!(second.state.unconfirmed[4] && second.state.low_trust);
        assert!(
            !second.state.has_confirmed_floor[4],
            "the placeholder 0 is not a confirmed value"
        );
    }

    #[test]
    fn a_held_column_keeps_its_row_shift_tag() {
        let mut prev = state(c(30, 4, 2, 3000, 800, 400));
        prev.row_shift[0] = true;
        let held_e = [true, false, false, false, false, false];
        let held = apply_gate_held(
            Some((prev, secs(20))),
            c(30, 4, 2, 3000, 800, 400),
            CLEAN,
            held_e,
            false,
            true,
        );
        assert!(
            held.state.row_shift[0],
            "no read, so nothing clears the tag"
        );
        // a real read of the same number, with no drop held, does clear it
        let read = apply_gate(
            Some((prev, secs(20))),
            c(30, 4, 2, 3000, 800, 400),
            CLEAN,
            false,
        );
        assert!(!read.state.row_shift[0]);
    }

    #[test]
    fn a_split_does_not_carry_a_held_column_into_the_new_game() {
        let prev = state(c(30, 12, 9, 14000, 5000, 900));
        // the caller fills a held column with the old game's value
        let gate = apply_gate_held(
            Some((prev, secs(200))),
            c(1, 0, 0, 300, 5000, 0),
            CLEAN,
            HELD_H,
            true,
            true,
        );
        assert_eq!(gate.accepted, c(1, 0, 0, 300, 0, 0));
        assert!(gate.state.unread[4]);
    }

    #[test]
    fn a_held_column_is_not_a_clean_read_for_an_unconfirmed_latch() {
        // a fallback latched assists 40; one clean 0 keeps it, a second
        // clean 0 replaces it
        let first = apply_gate_with_trust(None, c(2, 40, 1, 450, 0, 2), CLEAN, false, false);
        let once = apply_gate(
            Some((first.state, secs(20))),
            c(2, 0, 1, 500, 0, 2),
            CLEAN,
            false,
        );
        assert_eq!(once.accepted.assists, 40);
        // ...but not on a capture with a held cell: that board is not clean
        let held = apply_gate_held(
            Some((once.state, secs(20))),
            c(2, 0, 1, 500, 0, 2),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert_eq!(
            held.accepted.assists, 40,
            "a board with a held cell replaces nothing"
        );
        let clean = apply_gate(
            Some((once.state, secs(20))),
            c(2, 0, 1, 500, 0, 2),
            CLEAN,
            false,
        );
        assert_eq!(
            clean.accepted.assists, 0,
            "the same read, fully clean, does"
        );
    }

    #[test]
    fn a_game_with_no_held_cell_never_uses_the_held_state() {
        // ocr-v1 has a number for every column on every capture. Through a
        // noisy game (drops, a rate cap, a fallback, an inject, an un-latch)
        // nothing of the held machinery may switch on: no column is unread,
        // no held time builds up behind a gate hold, no cell is `Unread`.
        let reads = [
            (c(3, 1, 0, 900, 0, 120), CLEAN, true),
            (c(5, 1, 1, 1500, 0, 300), CLEAN, true),
            (
                c(1, 1, 1, 15031, 0, 300),
                [false, false, false, true, false, false],
                true,
            ),
            (c(40, 18, 1, 1600, 0, 310), CLEAN, false),
            (c(6, 2, 1, 1700, 0, 22994), CLEAN, true),
            (c(6, 2, 1, 1700, 0, 350), CLEAN, true),
            (c(6, 2, 1, 1700, 0, 350), CLEAN, true),
        ];
        let mut state: Option<GateState> = None;
        let mut kinds = Vec::new();
        for (i, (raw, suspect, trusted)) in reads.into_iter().enumerate() {
            let prev = state.map(|s| (s, secs(20)));
            let out = apply_gate_with_trust(prev, raw, suspect, false, trusted);
            assert_eq!(out.state.unread, [false; GATE_COLS], "step {i}");
            assert_eq!(out.state.held_secs, [0; GATE_COLS], "step {i}");
            assert!(out.state.unread_cols().is_empty(), "step {i}");
            assert!(
                out.holds.iter().all(|hold| hold.kind != HoldKind::Unread),
                "step {i}: {:?}",
                out.holds
            );
            // ocr-v1 flags a drop and a lone jump only; the new reader's
            // list is that plus the rate-cap and inject holds
            let unsure = unsure_fields(&out.holds);
            let all = held_fields(&out.holds);
            assert!(unsure.iter().all(|name| all.contains(name)), "step {i}");
            kinds.extend(out.holds.iter().map(|hold| hold.kind));
            state = Some(out.state);
        }
        // the game above really does exercise the holds this is about
        for kind in [
            HoldKind::Monotonic,
            HoldKind::RateCap,
            HoldKind::DigitInject,
        ] {
            assert!(kinds.contains(&kind), "{kind:?} never fired: {kinds:?}");
        }
    }

    /// Healing held over a 5 minute run, the rest of the board read.
    fn after_a_held_healing_run() -> GateState {
        let first = apply_gate(None, c(11, 4, 2, 3000, 800, 400), CLEAN, false);
        let held = apply_gate_held(
            Some((first.state, secs(300))),
            c(20, 8, 4, 9000, 0, 1500),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert_eq!(held.accepted, c(20, 8, 4, 9000, 800, 1500));
        assert_eq!(held.state.held_secs, [0, 0, 0, 0, 300, 0]);
        held.state
    }

    /// F266-3 (R266-A), the reviewer's first trace: 800, held, then a sure
    /// 8,500 after 20 s. Against one 20 s interval that is a trailing-digit
    /// inject, held with no flag and with no way out. The accepted 800 is
    /// 320 s old, and over 320 s the rise is ordinary.
    #[test]
    fn a_sure_read_after_a_held_run_is_not_a_trailing_digit_inject() {
        let run = after_a_held_healing_run();
        // the board moved on this capture: the read is stored at once
        let busy = apply_gate(
            Some((run, secs(20))),
            c(21, 8, 4, 9600, 8500, 1500),
            CLEAN,
            false,
        );
        assert_eq!(busy.accepted.healing, 8500, "{:?}", busy.holds);
        assert!(busy.holds.is_empty());
        assert_eq!(busy.state.held_secs, [0; GATE_COLS]);
        // on a quiet board it is a lone jump: kept one capture, FLAGGED,
        // and the held time keeps counting behind that hold
        let quiet = apply_gate(
            Some((run, secs(20))),
            c(20, 8, 4, 9100, 8500, 1500),
            CLEAN,
            false,
        );
        assert_eq!(quiet.accepted.healing, 800);
        assert_eq!(
            quiet.holds,
            vec![Hold {
                col: 4,
                kind: HoldKind::Jump,
                raw: 8500,
                held: 800,
            }]
        );
        assert_eq!(unsure_fields(&quiet.holds), vec!["h"]);
        assert_eq!(quiet.state.held_secs[4], 320);
        // the next sure read agrees and is stored: no inject hold repeats
        let next = apply_gate(
            Some((quiet.state, secs(20))),
            c(20, 8, 4, 9200, 8700, 1500),
            CLEAN,
            false,
        );
        assert_eq!(next.accepted.healing, 8700, "{:?}", next.holds);
        assert_eq!(next.state.held_secs, [0; GATE_COLS]);

        // the same reads with no held run are an inject, as on ocr-v1; the
        // new reader's list flags that hold, ocr-v1's does not
        let prev = state(c(20, 8, 4, 9000, 800, 1500));
        let inject = apply_gate(
            Some((prev, secs(20))),
            c(20, 8, 4, 9100, 8500, 1500),
            CLEAN,
            false,
        );
        assert_eq!(inject.accepted.healing, 800);
        assert_eq!(inject.holds[0].kind, HoldKind::DigitInject);
        assert!(unsure_fields(&inject.holds).is_empty());
        assert_eq!(held_fields(&inject.holds), vec!["h"]);
        assert_eq!(inject.state.held_secs, [0; GATE_COLS]);
    }

    /// F266-3, the second trace: elims 10, held, then a sure 25 after 20 s.
    /// One interval allows 10 + 20/5 + 8 = 22, a rate-cap hold with no
    /// flag. The 10 is 320 s old.
    #[test]
    fn a_sure_read_after_a_held_run_is_not_over_the_kill_rate_cap() {
        let held_e = [true, false, false, false, false, false];
        let first = apply_gate(None, c(10, 4, 2, 3000, 800, 400), CLEAN, false);
        let held = apply_gate_held(
            Some((first.state, secs(300))),
            c(0, 9, 5, 9000, 4000, 1500),
            CLEAN,
            held_e,
            false,
            true,
        );
        assert_eq!(held.accepted.elims, 10);
        assert_eq!(held.state.held_secs[0], 300);
        // a second held capture adds its own gap
        let again = apply_gate_held(
            Some((held.state, secs(15))),
            c(0, 9, 5, 9100, 4000, 1500),
            CLEAN,
            held_e,
            false,
            true,
        );
        assert_eq!(again.state.held_secs[0], 315);
        // assists moved too: stored at once
        let busy = apply_gate(
            Some((held.state, secs(20))),
            c(25, 11, 5, 9300, 4000, 1500),
            CLEAN,
            false,
        );
        assert_eq!(busy.accepted.elims, 25, "{:?}", busy.holds);
        // alone on a quiet board: a flagged lone jump, then stored
        let quiet = apply_gate(
            Some((held.state, secs(20))),
            c(25, 9, 5, 9100, 4000, 1500),
            CLEAN,
            false,
        );
        assert_eq!(quiet.accepted.elims, 10);
        assert_eq!(quiet.holds[0].kind, HoldKind::Jump);
        assert_eq!(unsure_fields(&quiet.holds), vec!["e"]);
        let next = apply_gate(
            Some((quiet.state, secs(20))),
            c(25, 9, 5, 9200, 4000, 1500),
            CLEAN,
            false,
        );
        assert_eq!(next.accepted.elims, 25);

        // no held run: 25 is past 22, a rate cap; only the new reader's
        // list flags it
        let prev = state(c(10, 9, 5, 9000, 4000, 1500));
        let capped = apply_gate(
            Some((prev, secs(20))),
            c(25, 9, 5, 9100, 4000, 1500),
            CLEAN,
            false,
        );
        assert_eq!(capped.accepted.elims, 10);
        assert_eq!(capped.holds[0].kind, HoldKind::RateCap);
        assert!(unsure_fields(&capped.holds).is_empty());
        assert_eq!(held_fields(&capped.holds), vec!["e"]);
    }

    /// A column back from a held run moved over that whole run. Its rise
    /// must not make another column's lone jump look like one of two.
    #[test]
    fn a_column_back_from_a_held_run_does_not_vouch_for_another_jump() {
        let run = after_a_held_healing_run();
        // healing reads 3,800 (+3,000 over 320 s) while damage alone jumps
        // 9,000 to 15,000 in 20 s
        let out = apply_gate(
            Some((run, secs(20))),
            c(20, 8, 4, 15000, 3800, 1500),
            CLEAN,
            false,
        );
        assert_eq!(out.accepted.damage, 9000, "the damage jump still waits");
        assert!(
            out.holds
                .iter()
                .any(|hold| hold.col == 3 && hold.kind == HoldKind::Jump),
            "{:?}",
            out.holds
        );
    }

    /// F266-2: a game whose healing was never read, its end missed, and the
    /// next game's first board 60 s later. Every read column drops. The
    /// healing on that board is the next game's and is not taken into this
    /// one, where nothing would ever flag it.
    #[test]
    fn the_first_read_of_an_unread_column_is_not_taken_when_the_board_drops() {
        let first = apply_gate_held(
            None,
            c(30, 12, 9, 14000, 0, 900),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert_eq!(first.state.unread_cols(), vec!["h"]);
        let next_game = apply_gate(
            Some((first.state, secs(60))),
            c(1, 0, 0, 300, 100, 0),
            CLEAN,
            false,
        );
        assert_eq!(next_game.accepted, c(30, 12, 9, 14000, 0, 900));
        assert_eq!(
            next_game.state.unread_cols(),
            vec!["h"],
            "the column is still unread, so the game still has no row"
        );
        assert!(next_game.holds.contains(&Hold {
            col: 4,
            kind: HoldKind::Unread,
            raw: 100,
            held: 0,
        }));
        assert!(held_fields(&next_game.holds).contains(&"h"));
        // the refused read is not the previous raw read either
        assert_eq!(next_game.state.last_raw.healing, 0);
        // and the state remembers it was seen, for the not-stored line
        assert_eq!(next_game.state.refused_cols(), vec!["h"]);
        assert!(
            first.state.refused_cols().is_empty(),
            "never read is not refused"
        );
        // one dropped column is enough to refuse it
        let one_drop = apply_gate(
            Some((first.state, secs(60))),
            c(31, 12, 9, 300, 100, 900),
            CLEAN,
            false,
        );
        assert!(one_drop.state.unread[4]);
        assert_eq!(one_drop.accepted.healing, 0);
        // a board of this game, no column down: the first read is stored
        let same_game = apply_gate(
            Some((first.state, secs(60))),
            c(31, 12, 9, 14200, 100, 900),
            CLEAN,
            false,
        );
        assert_eq!(same_game.accepted.healing, 100);
        assert!(same_game.state.unread_cols().is_empty());
        assert!(same_game.holds.is_empty());
        // a refused read that is taken later is no longer "refused"
        let taken = apply_gate(
            Some((next_game.state, secs(60))),
            c(31, 12, 9, 14200, 100, 900),
            CLEAN,
            false,
        );
        assert!(taken.state.refused_cols().is_empty());
        assert_eq!(taken.state.refused_first_read, [false; GATE_COLS]);
    }

    /// The board carried onto a new session is a row of that game. A cell
    /// the splitting capture has no number for takes the carried board's:
    /// not a 0 under the number on the row before, and not the previous
    /// game's value the caller filled it with.
    #[test]
    fn a_seeded_first_capture_takes_the_carried_board_in_a_held_column() {
        let old_game = state(c(30, 12, 9, 14000, 5000, 900));
        let carried = c(1, 0, 0, 300, 120, 0);
        let split = apply_gate_seeded(
            Some((old_game, secs(200))),
            c(3, 1, 0, 900, 5000, 0),
            CLEAN,
            HELD_H,
            true,
            true,
            Some((carried, secs(50))),
        );
        assert_eq!(split.accepted, c(3, 1, 0, 900, 120, 0));
        assert!(split.state.unread_cols().is_empty(), "the game has a read");
        assert_eq!(
            split.holds,
            vec![Hold {
                col: 4,
                kind: HoldKind::Unread,
                raw: 5000,
                held: 120,
            }]
        );
        assert_eq!(split.state.held_secs, [0, 0, 0, 0, 50, 0]);
        assert_eq!(split.state.last_raw.healing, 120);
        assert!(!split.state.last_raw_suspect[4]);
        // the same on a session's first capture (no split, no gate yet)
        let first = apply_gate_seeded(
            None,
            c(3, 1, 0, 900, 0, 0),
            CLEAN,
            HELD_H,
            false,
            true,
            Some((carried, secs(50))),
        );
        assert_eq!(first.accepted.healing, 120);
        // without a carried board there is nothing to take: unread
        let bare = apply_gate_held(
            Some((old_game, secs(200))),
            c(3, 1, 0, 900, 5000, 0),
            CLEAN,
            HELD_H,
            true,
            true,
        );
        assert_eq!(bare.accepted.healing, 0);
        assert_eq!(bare.state.unread_cols(), vec!["h"]);
        // the seed is for held columns of a first capture only: a read
        // column keeps its read, and a later capture ignores the seed
        let later = apply_gate_seeded(
            Some((old_game, secs(20))),
            c(31, 12, 9, 14100, 0, 900),
            CLEAN,
            HELD_H,
            false,
            true,
            Some((carried, secs(50))),
        );
        assert_eq!(later.accepted, c(31, 12, 9, 14100, 5000, 900));
    }

    /// R3-2: a suspect own row opened the game (untrusted, healing held),
    /// higher than the member's real row. Trusted reads of the real row then
    /// sit below it on every board and never repeat, so they would be
    /// monotonic holds for the rest of the game and healing would never get
    /// its first read. Nothing of the game is stored yet: the first trusted
    /// board starts it over.
    #[test]
    fn a_trusted_board_restarts_a_game_that_has_only_low_trust_reads() {
        let first = apply_gate_held(
            None,
            c(20, 6, 3, 8000, 77, 450),
            CLEAN,
            HELD_H,
            false,
            false,
        );
        assert!(first.state.only_low_trust_reads());
        assert_eq!(first.state.unread_cols(), vec!["h"]);
        // another low-trust board does not restart anything
        let again = apply_gate_held(
            Some((first.state, secs(30))),
            c(21, 6, 3, 8200, 77, 450),
            CLEAN,
            HELD_H,
            false,
            false,
        );
        assert!(again.state.only_low_trust_reads());
        assert_eq!(again.accepted.elims, 21);
        // ...also not a lower one that reads healing: it is gated against
        // the first, so the game still has no row
        let lower = apply_gate_with_trust(
            Some((first.state, secs(30))),
            c(10, 6, 3, 4000, 500, 450),
            CLEAN,
            false,
            false,
        );
        assert_eq!(lower.accepted, c(20, 6, 3, 8000, 0, 450));
        assert_eq!(lower.state.unread_cols(), vec!["h"]);
        assert_eq!(lower.state.refused_cols(), vec!["h"]);
        // the member's real row, trusted: taken as a first capture
        let real = apply_gate(
            Some((again.state, secs(30))),
            c(12, 6, 3, 3500, 700, 450),
            CLEAN,
            false,
        );
        assert_eq!(real.accepted, c(12, 6, 3, 3500, 700, 450));
        assert!(real.holds.is_empty() && real.unlatches.is_empty());
        assert!(real.state.unread_cols().is_empty());
        assert!(!real.state.low_trust && !real.state.only_low_trust_reads());
        assert_eq!(real.state, state(c(12, 6, 3, 3500, 700, 450)));
        // a trusted board with a held cell restarts too, and stays pending
        let held = apply_gate_held(
            Some((again.state, secs(30))),
            c(12, 6, 3, 3500, 0, 450),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert_eq!(held.accepted, c(12, 6, 3, 3500, 0, 450));
        assert_eq!(held.state.unread_cols(), vec!["h"]);
        assert!(!held.state.only_low_trust_reads(), "e..mit are confirmed");

        // a game that has a row is never restarted: a suspect row in the
        // middle of it leaves low-trust columns, and a lower trusted read
        // is still held
        let game = apply_gate(None, c(12, 5, 3, 3500, 700, 300), CLEAN, false);
        let suspect = apply_gate_with_trust(
            Some((game.state, secs(20))),
            c(20, 9, 5, 8000, 2000, 900),
            CLEAN,
            false,
            false,
        );
        assert!(suspect.state.low_trust && !suspect.state.only_low_trust_reads());
        let lower = apply_gate(
            Some((suspect.state, secs(20))),
            c(13, 5, 3, 3700, 720, 300),
            CLEAN,
            false,
        );
        assert_eq!(lower.accepted, c(20, 9, 5, 8000, 2000, 900));
        // and a pending game a trusted board opened is not restarted either
        let trusted_first = apply_gate_held(
            None,
            c(30, 12, 9, 14000, 0, 900),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert!(!trusted_first.state.only_low_trust_reads());
    }

    /// The names for a row whose number went down: the un-latched cells,
    /// and any stat lower than on the row before.
    #[test]
    fn a_revised_or_lowered_stat_is_named() {
        let game = apply_gate(None, c(12, 5, 3, 3500, 700, 300), CLEAN, false);
        let suspect = apply_gate_with_trust(
            Some((game.state, secs(20))),
            c(20, 9, 5, 8000, 2000, 900),
            CLEAN,
            false,
            false,
        );
        let once = apply_gate(
            Some((suspect.state, secs(20))),
            c(13, 5, 3, 3700, 720, 300),
            CLEAN,
            false,
        );
        assert!(once.unlatches.is_empty());
        assert_eq!(held_fields(&once.holds).len(), 6, "all six held, flagged");
        // the second matching read replaces all six with lower numbers:
        // that is no hold, so `held_fields` alone would flag nothing
        let twice = apply_gate(
            Some((once.state, secs(20))),
            c(13, 5, 3, 3700, 720, 300),
            CLEAN,
            false,
        );
        assert_eq!(twice.accepted, c(13, 5, 3, 3700, 720, 300));
        assert!(twice.holds.is_empty());
        let all = vec!["e", "a", "d", "dmg", "h", "mit"];
        assert_eq!(revised_fields(&twice.unlatches), all);
        assert_eq!(lowered_fields(once.accepted, twice.accepted), all);
        assert!(lowered_fields(twice.accepted, twice.accepted).is_empty());
        assert_eq!(
            lowered_fields(c(9, 3, 9, 100, 5, 900), c(9, 4, 0, 100, 5, 150)),
            vec!["d", "mit"]
        );
    }

    /// Reviewer mutant: `unread` carried across a split. The new game's
    /// unread columns are this capture's held cells and nothing else.
    #[test]
    fn unread_does_not_survive_a_split() {
        let first = apply_gate_held(
            None,
            c(30, 12, 9, 14000, 0, 900),
            CLEAN,
            HELD_H,
            false,
            true,
        );
        assert!(first.state.unread[4]);
        let split = apply_gate(
            Some((first.state, secs(200))),
            c(1, 0, 0, 300, 100, 0),
            CLEAN,
            true,
        );
        assert_eq!(split.accepted, c(1, 0, 0, 300, 100, 0));
        assert_eq!(split.state.unread, [false; GATE_COLS]);
        // a split with another column held: that one, not the old one
        let held_e = [true, false, false, false, false, false];
        let split = apply_gate_held(
            Some((first.state, secs(200))),
            c(7, 0, 0, 300, 100, 0),
            CLEAN,
            held_e,
            true,
            true,
        );
        assert_eq!(split.state.unread, held_e);
        assert_eq!(split.state.held_secs, [0; GATE_COLS]);
    }

    /// Reviewer mutant: the held guess written into `confirmed_floor`. An
    /// untrusted capture leaves a floor under the column it moved, and
    /// nothing under a column it has no number for.
    #[test]
    fn an_untrusted_capture_leaves_no_floor_under_a_held_cell() {
        let prev = state(c(11, 4, 2, 3000, 800, 400));
        let out = apply_gate_held(
            Some((prev, secs(20))),
            c(11, 4, 2, 3300, 8000, 400),
            CLEAN,
            HELD_H,
            false,
            false,
        );
        assert_eq!(out.accepted, c(11, 4, 2, 3300, 800, 400));
        // damage moved on an untrusted read: unconfirmed, floor 3000
        assert!(out.state.unconfirmed[3] && out.state.has_confirmed_floor[3]);
        assert_eq!(out.state.confirmed_floor[3], 3000);
        // healing was not read: still confirmed at 800, no floor, no guess
        assert!(!out.state.unconfirmed[4]);
        assert!(!out.state.has_confirmed_floor[4]);
        assert_eq!(out.state.confirmed_floor[4], 0);
    }

    #[test]
    fn a_save_from_before_the_unread_column_loads_as_all_read() {
        let now = serde_json::to_value(state(c(11, 4, 2, 3000, 800, 400))).unwrap();
        let mut old = now.clone();
        for field in ["unread", "held_secs", "refused_first_read"] {
            old.as_object_mut()
                .unwrap()
                .remove(field)
                .expect("the field is saved");
        }
        let loaded: GateState = serde_json::from_value(old).unwrap();
        assert_eq!(loaded.unread, [false; GATE_COLS]);
        assert_eq!(loaded.held_secs, [0; GATE_COLS]);
        assert_eq!(loaded, state(c(11, 4, 2, 3000, 800, 400)));
        // a pending game survives a restart: both fields round-trip
        let mut pending = state(c(30, 12, 9, 14000, 0, 900));
        pending.unread[4] = true;
        pending.held_secs[0] = 315;
        let back: GateState =
            serde_json::from_value(serde_json::to_value(pending).unwrap()).unwrap();
        assert_eq!(back, pending);
        assert_eq!(back.unread_cols(), vec!["h"]);
    }
}
