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
//! fallback, or a value stored while the capture was low-trust) yields to
//! one later clean read, even when that read is lower. A high stored while
//! another column was held down is a row shift, and a later read of the
//! real row can replace it when that read repeats. A value confirmed by a
//! clean read, and not a row shift, cannot go down. A read whose every cell
//! is suspect, including a suspect own row, does not seed or raise the
//! baseline and stores none of its numbers. A real new game is the
//! whole-board reset in `boundary`, and the caller passes `split = true` so
//! this gate stores the new counters instead of holding them.
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
//! `unconfirmed`. One later clean read, with no suspect cell, replaces an
//! unconfirmed column even when the read is lower. A confirmed column never
//! decreases. A read with every cell suspect does not seed that latch and
//! does not raise it.
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
    /// otherwise accepts the raw read verbatim. One later clean read of a
    /// lower number replaces the latch. A read that is itself suspect does
    /// not. Missing on a pre-0.4.21 `active_game.json` means already trusted.
    #[serde(default)]
    pub low_trust: bool,
    /// Which accepted columns came from a fallback or an implausible latch.
    /// One clean read of a lower number replaces these. A suspect or held
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
    /// Raw counters from a read the gate discarded (every cell suspect).
    ///
    /// Not a monotonic value: `accepted` stays where it was. The boundary
    /// plan may anchor its reset baseline on this, so a later fresh board
    /// still splits, and then it is cleared off the gate that is carried
    /// forward. Missing means the accepted counters are the anchor.
    #[serde(default)]
    pub boundary_anchor: Option<Counters>,
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
    /// True when one clean read replaced an unconfirmed cell. A confirmed
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
/// The gate lets one clean read replace an unconfirmed column with a lower
/// number, and that is not a hold. The row that stores the lower number
/// names those fields.
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
/// untrusted or a kill column is past the ceilings. A read whose every cell
/// is suspect is the exception: none of its numbers are accepted, on a first
/// capture or later. Otherwise each cell is
/// checked independently: a decrease of a confirmed column keeps the previous
/// accepted value and is unsure; an unconfirmed column yields to one clean
/// read even when that read is lower; a trailing-digit inject is held (CG-4 B2);
/// a single-field rise past
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
/// A later read that is trusted and has no suspect cell replaces an
/// *unconfirmed* column on that one read, even when the number is lower.
/// A confirmed column still takes the monotonic hold (B). The rate cap (C)
/// and the trailing-digit inject hold (B2) still apply, including to a
/// low-trust column whose clean read jumps past them. A replacement below
/// the confirmed value a fallback moved off, and a wide column whose clean
/// read is the latched number with its last digit cut off, also fall
/// through to B. A row-shift high still waits for the same lower number to
/// repeat. `low_trust` stays set while any column is held or still
/// unconfirmed. A second fallback, or a read with edge-ink, does not clear
/// an unconfirmed column. An untrusted read is recorded as suspect on every
/// column, so it does not corroborate the next jump. A read with every cell
/// suspect does not seed or raise the baseline.
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
    mut held: [bool; GATE_COLS],
    split: bool,
    trusted: bool,
    seed: Option<(Counters, Duration)>,
) -> GateOutcome {
    let already_unread = held.iter().all(|&cell| cell);
    for col in 0..GATE_COLS {
        suspect[col] |= held[col];
    }
    // Every cell is suspect: another player's row, or ink on every cell.
    // None of these numbers may seed or raise the monotonic baseline, and
    // none are stored. An unread cell keeps the last accepted value, or
    // stays a placeholder when the game has no row yet. The raw counters
    // stay on `boundary_anchor` so the reset baseline can still move.
    let discard_row = suspect.iter().all(|&cell| cell);
    if discard_row {
        held = [true; GATE_COLS];
    }
    let boundary_anchor = (discard_row && !already_unread).then_some(raw);
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
        state.boundary_anchor = boundary_anchor;
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
        // A confirmed value stays. A low-trust column yields to one clean
        // read even when that read is lower. A row-shift high still waits
        // for the same lower number to repeat.
        let clean_board = trusted && !suspect.iter().any(|&flag| flag);
        let clean_match = clean_board && !prev_raw_suspect[col] && prev_raw[col] == cur[col];
        let replace_low = clean_board && unconfirmed[col] && cur[col] < prev_acc[col];
        let revise_down = replace_low || (row_shift[col] && clean_match);
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
            // agreed with it, or this clean read replaced the latch.
            // Otherwise B would lock the ghost in. Plausible columns,
            // including a fallback assists of 40, still confirm on this read.
            let over = kill_col_over_ceiling(col, prev_acc[col]);
            let agrees = !prev_raw_suspect[col] && cur[col] == prev_raw[col];
            let replaced = unlatches
                .iter()
                .any(|unlatch| unlatch.col == col && unlatch.replaced_unconfirmed);
            if over && !agrees && !replaced {
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
            boundary_anchor,
        },
        holds,
        unlatches,
    }
}

fn secs_u32(span: Duration) -> u32 {
    u32::try_from(span.as_secs()).unwrap_or(u32::MAX)
}

#[cfg(test)]
mod tests;

#[cfg(test)]
mod tests_holds;

#[cfg(test)]
mod tests_replay;

#[cfg(test)]
mod tests_unread;
