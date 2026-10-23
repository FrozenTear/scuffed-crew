//! The gate's working set and persisted shape: [`Counters`] and
//! [`GateState`] (serde, part of `active_game.json`), and the per-capture
//! result types.

use serde::{Deserialize, Serialize};

/// Number of cumulative counters gated per capture: E, A, D, DMG, HLG, MIT.
pub const GATE_COLS: usize = 6;

/// The six cumulative scoreboard counters, in the positional order
/// `[elims, assists, deaths, damage, healing, mitigation]` — matching the
/// column order produced by `parse::board::stats_from_row`.
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

    pub(super) fn from_array(a: [u32; GATE_COLS]) -> Self {
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
    ///
    /// [`apply_gate_seeded`]: super::apply_gate_seeded
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
    ///
    /// [`JUMP_MAX`]: super::limits::JUMP_MAX
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

/// Flat `suspect_fields` names, same order as [`Counters::to_array`].
pub const COL_FIELD: [&str; GATE_COLS] = ["e", "a", "d", "dmg", "h", "mit"];
