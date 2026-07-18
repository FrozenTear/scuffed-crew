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
    fn to_array(self) -> [u32; GATE_COLS] {
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
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
pub struct GateState {
    pub accepted: Counters,
    pub last_raw: Counters,
}

/// Why a cell was held, for per-capture observability logging.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum HoldKind {
    /// Cumulative counter decreased versus the last accepted value → misread
    /// collapse (B).
    Monotonic,
    /// Counter jumped beyond the plausible per-second rate and no corroborating
    /// prior read backed it → suspected inflation (C).
    RateCap,
}

/// One held cell in a capture — which column, why, and the raw→held swap.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Hold {
    pub col: usize,
    pub kind: HoldKind,
    pub raw: u32,
    pub held: u32,
}

/// Result of gating one capture.
pub struct GateOutcome {
    /// Counters to actually store (raw where accepted, previous where held).
    pub accepted: Counters,
    /// State to carry into the next capture of this game.
    pub state: GateState,
    /// Cells that were held back from their raw read (empty = clean capture).
    pub holds: Vec<Hold>,
}

/// The kills-family columns (E, A, D): small, slow-growing integer counters.
const KILL_COLS: [usize; 3] = [0, 1, 2];

/// Rate cap for the kills family: a counter may climb by at most
/// `elapsed_secs / KILL_RATE_DIVISOR_SECS + KILL_RATE_SLACK` between two
/// accepted captures. Calibrated one-sided against the real 2026-07-18 series
/// so a genuine stomp is never rejected (E climbs 22→28 in ~90s = +6, cap ≥ 26;
/// 15→22 in ~41s = +7, cap ≥ 16) while the 9X ghost sits far above it
/// (E 9→91 in 75s: cap = 75/5 + 8 = 23, held).
const KILL_RATE_DIVISOR_SECS: u64 = 5;
const KILL_RATE_SLACK: u32 = 8;

/// Absolute floor of the corroboration band; the effective band is
/// `max(CORROBORATION_ABS, level/10)` so a repeated high read still corroborates
/// after small OCR jitter.
const CORROBORATION_ABS: u32 = 2;

fn is_kill_col(col: usize) -> bool {
    KILL_COLS.contains(&col)
}

/// Maximum plausible increase for column `col` over `elapsed`. Wide accumulator
/// columns (DMG/HLG/MIT) get no upper cap — their only observed corruption is
/// the tail-clip *collapse*, which the monotonic hold catches, and a real burst
/// of several thousand damage in seconds must never be rejected.
fn max_delta(col: usize, elapsed: Duration) -> u32 {
    if is_kill_col(col) {
        ((elapsed.as_secs() / KILL_RATE_DIVISOR_SECS) as u32).saturating_add(KILL_RATE_SLACK)
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

/// Apply the per-cell capture gate.
///
/// `prev` — the gate state and elapsed-since carried from the last accepted
/// capture of this game, or `None` for the first capture. `raw` — this
/// capture's parsed counters. `split` — whether the whole-row game-split signal
/// already fired (a real new game).
///
/// On a split or a first capture the raw read is accepted verbatim (a new game
/// legitimately resets every counter). Otherwise each cell is checked
/// independently: a decrease is held to the previous accepted value (B); an
/// increase beyond the plausible rate is held unless the previous raw read
/// corroborates it (C); an advance within rate passes through unchanged.
pub fn apply_gate(prev: Option<(GateState, Duration)>, raw: Counters, split: bool) -> GateOutcome {
    let Some((state, elapsed)) = prev.filter(|_| !split) else {
        return GateOutcome {
            accepted: raw,
            state: GateState {
                accepted: raw,
                last_raw: raw,
            },
            holds: Vec::new(),
        };
    };

    let prev_acc = state.accepted.to_array();
    let prev_raw = state.last_raw.to_array();
    let cur = raw.to_array();
    let mut out = cur;
    let mut holds = Vec::new();

    for col in 0..GATE_COLS {
        if cur[col] < prev_acc[col] {
            // (B) cumulative counter decreased → misread; hold last accepted.
            out[col] = prev_acc[col];
            holds.push(Hold {
                col,
                kind: HoldKind::Monotonic,
                raw: cur[col],
                held: prev_acc[col],
            });
        } else {
            let ceiling = prev_acc[col].saturating_add(max_delta(col, elapsed));
            if cur[col] > ceiling && !corroborates(prev_raw[col], cur[col]) {
                // (C) implausible jump, uncorroborated → hold last accepted.
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::RateCap,
                    raw: cur[col],
                    held: prev_acc[col],
                });
            }
            // else: plausible advance (or corroborated jump) → keep cur[col].
        }
    }

    let accepted = Counters::from_array(out);
    GateOutcome {
        accepted,
        state: GateState {
            accepted,
            last_raw: raw,
        },
        holds,
    }
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

    // --- Focused unit tests (crisp mutation-check mapping) ---

    #[test]
    fn first_capture_accepts_raw() {
        let out = apply_gate(None, c(5, 3, 2, 4316, 1200, 899), false);
        assert_eq!(out.accepted, c(5, 3, 2, 4316, 1200, 899));
        assert!(out.holds.is_empty());
    }

    #[test]
    fn split_resets_gate_to_raw() {
        // Even with a "previous" high state, a split (real new game) accepts the
        // fresh low read verbatim.
        let prev = GateState {
            accepted: c(30, 10, 8, 12000, 3000, 5000),
            last_raw: c(30, 10, 8, 12000, 3000, 5000),
        };
        let out = apply_gate(Some((prev, secs(180))), c(0, 0, 0, 200, 50, 0), true);
        assert_eq!(out.accepted, c(0, 0, 0, 200, 50, 0));
        assert!(out.holds.is_empty());
    }

    #[test]
    fn monotonic_hold_holds_a_single_cell_decrease() {
        // MUTATION CHECK (B): remove the `cur < prev_acc` hold and deaths stores 5.
        // Deaths 6→5 (the real Havana final-D bug); elims still advances 9→13.
        let prev = GateState {
            accepted: c(9, 7, 6, 6622, 1520, 4549),
            last_raw: c(9, 7, 6, 6622, 1520, 4549),
        };
        let out = apply_gate(Some((prev, secs(45))), c(13, 9, 5, 7219, 1598, 4999), false);
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
        let prev = GateState {
            accepted: c(13, 9, 6, 8561, 1898, 5199),
            last_raw: c(13, 9, 6, 8561, 1898, 5199),
        };
        let out = apply_gate(Some((prev, secs(85))), c(22, 10, 5, 9906, 224, 5958), false);
        assert_eq!(out.accepted.healing, 1898, "clipped healing holds at 1898");
        assert_eq!(out.accepted.elims, 22, "real elims stomp still accepted");
        assert_eq!(out.accepted.deaths, 6, "clipped deaths holds at 6");
    }

    #[test]
    fn rate_cap_holds_uncorroborated_spike() {
        // MUTATION CHECK (C): remove the ceiling hold and elims stores 91.
        // E 9→91 in 75s with the prior raw read at 9 (no corroboration).
        let prev = GateState {
            accepted: c(9, 5, 3, 4119, 822, 2639),
            last_raw: c(9, 5, 3, 4119, 822, 2639),
        };
        let out = apply_gate(Some((prev, secs(75))), c(91, 6, 3, 5175, 968, 3227), false);
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
        };
        let out = apply_gate(Some((prev, secs(10))), c(91, 6, 3, 5175, 968, 3227), false);
        assert_eq!(out.accepted.elims, 91, "corroborated jump is accepted");
    }

    #[test]
    fn plausible_stomp_passes_ungated() {
        // The one-sided guarantee: a real fast climb within rate is never held.
        let prev = GateState {
            accepted: c(22, 10, 5, 9906, 1898, 5958),
            last_raw: c(22, 10, 5, 9906, 1898, 5958),
        };
        let out = apply_gate(
            Some((prev, secs(90))),
            c(28, 17, 10, 12672, 2544, 6428),
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
    fn replay(series: &[Step]) -> Vec<Counters> {
        let mut state: Option<(GateState, Duration)> = None;
        let mut accepted = Vec::new();
        for &(e, a, d, dmg, hlg, mit, gap) in series {
            let raw = c(e, a, d, dmg, hlg, mit);
            let prev = state.map(|(s, _): (GateState, Duration)| (s, Duration::from_secs(gap)));
            let out = apply_gate(prev, raw, false);
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

    #[test]
    fn havana_series_gate_holds_every_collapse_and_ghost() {
        let acc = replay(HAVANA);

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

    #[test]
    fn route66_series_elims_never_collapse_and_stomps_pass() {
        let acc = replay(ROUTE66);
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
    fn route66_row_shift_is_a_documented_limitation_not_a_fix() {
        // KNOWN LIMITATION (mode c, out of scope): the 22:07:50 row-shift read a
        // different player's row (A19/D13). The monotonic hold then locks A and D
        // at those inflated levels, so the gate's A/D FINALS regress versus the
        // real A17/D10. This test pins that behavior so a future row-shift fix
        // (item 8 mode c) has a red anchor to flip. Elims are unaffected.
        let acc = replay(ROUTE66);
        let final_c = *acc.last().unwrap();
        assert_eq!(final_c.elims, 28, "elims unaffected by the row-shift");
        assert_eq!(
            final_c.assists, 19,
            "assists locked high by the out-of-scope row-shift (real 17)"
        );
        assert_eq!(
            final_c.deaths, 13,
            "deaths locked high by the out-of-scope row-shift (real 10)"
        );
    }
}
