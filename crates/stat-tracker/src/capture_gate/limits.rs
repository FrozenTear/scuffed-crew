//! The rate model: per-column rate caps, jump and quiet limits, and the
//! corroboration band. `boundary` reads the four `pub(crate)` rate
//! constants.

use std::time::Duration;

use super::state::GATE_COLS;

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
///
/// [`HoldKind::Jump`]: super::HoldKind::Jump
pub(super) const JUMP_MAX: [u32; GATE_COLS] = [4, 4, 2, 4000, 2500, 2500];

/// Largest change in a *different* column that still counts as a quiet board.
///
/// A single-field jump is one misread cell. The check is absolute: assists
/// up by 4, mitigation up by a few hundred, or a latched spike falling back
/// all mean the rest of the board moved, and the rise is stored. Damage
/// +100 between two tabs of the same fight stays quiet. Deaths treat a
/// change of 1 as quiet, same as elims and assists.
pub(super) const QUIET_RISE: [u32; GATE_COLS] = [1, 1, 1, 200, 200, 200];

pub(super) fn is_kill_col(col: usize) -> bool {
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
pub(super) fn is_trailing_digit_inject(prev_acc: u32, cur: u32, elapsed: Duration) -> bool {
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
pub(super) fn max_delta(col: usize, elapsed: Duration, cur_suspect: bool) -> u32 {
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
pub(super) fn corroborates(prev_raw: u32, cur: u32) -> bool {
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
pub(super) fn sudden_single_field_jump(
    col: usize,
    prev: [u32; GATE_COLS],
    cur: [u32; GATE_COLS],
) -> bool {
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

pub(super) fn secs_u32(span: Duration) -> u32 {
    u32::try_from(span.as_secs()).unwrap_or(u32::MAX)
}
