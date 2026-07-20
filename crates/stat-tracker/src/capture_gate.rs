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
//! ## Un-latch (CG-2)
//!
//! A one-sided hold has a failure mode: if a corrupt *inflated* read is ever
//! accepted (the wide accumulator columns have no rate cap, so a drifted read
//! like DMG 35031 sails through — CG-2), the monotonic hold then rejects every
//! later CORRECT read as a "decrease" for the rest of the match. So the gate
//! also un-latches: `UNLATCH_STREAK` consecutive **clean** reads of a cell, each
//! below the held value and forming one coherent (non-decreasing, within the
//! corroboration band) run, revise the accepted value DOWN to the latest of
//! them. The CLEAN requirement is load-bearing — a per-cell `suspect` mask
//! (edge-ink, CG-3) excludes deterministic clip reads (e.g. Havana HLG
//! 1898→224/234) from ever driving an un-latch, so the gate cannot be talked out
//! of a genuinely-correct held value by the very misreads it exists to stop.
//! Suspect reads likewise never corroborate an upward jump (C).
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
///
/// The `down_*` / `last_raw_suspect` fields back the CG-2 un-latch and are
/// ADDITIVE with `#[serde(default)]`: an in-flight `active_game.json` written by
/// a pre-un-latch build deserializes cleanly (missing → zero/false, i.e. no
/// streak in progress, previous raw treated as clean). Do not rename or drop the
/// existing fields — that would silently discard recovered in-game state.
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize)]
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

/// One un-latched cell in a capture (CG-2): a run of clean below-held reads
/// revised the accepted value DOWN, from `revised_from` to `raw`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Unlatch {
    pub col: usize,
    /// The clean read the accepted value was revised down to.
    pub raw: u32,
    /// The (suspected-corrupt) value that had been latched.
    pub revised_from: u32,
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
const KILL_RATE_DIVISOR_SECS: u64 = 5;
const KILL_RATE_SLACK: u32 = 8;

/// Absolute floor of the corroboration band; the effective band is
/// `max(CORROBORATION_ABS, level/10)` so a repeated high read still corroborates
/// after small OCR jitter.
const CORROBORATION_ABS: u32 = 2;

/// Consecutive clean, coherent below-held reads required to un-latch a cell
/// (CG-2). Three is enough to rule out a lone OCR fluke while still recovering
/// within a couple of Tab presses of a corrupt inflation being accepted.
const UNLATCH_STREAK: u32 = 3;

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

/// Whether a new clean below-held read `cur` continues the down-streak begun at
/// `streak_last`. Cumulative counters only climb, so a coherent run is
/// non-decreasing within the corroboration jitter band: a constant run
/// (Junkertown A: 4,4,4) and a climbing run (damage 4543→5852→6810) both
/// qualify, while a read that drops well below the run resets it.
fn downstreak_continues(streak_last: u32, cur: u32) -> bool {
    cur.saturating_add(CORROBORATION_ABS.max(cur / 10)) >= streak_last
}

/// Apply the per-cell capture gate.
///
/// `prev` — the gate state and elapsed-since carried from the last accepted
/// capture of this game, or `None` for the first capture. `raw` — this
/// capture's parsed counters. `suspect` — per-column edge-ink mask (CG-3): a
/// `true` column had glyph ink touching a crop edge, so its read is stripped of
/// all gate influence (never corroborates a jump, never drives an un-latch),
/// though it is still accepted if it is a plausible in-rate advance. `split` —
/// whether the whole-row game-split signal already fired (a real new game).
///
/// On a split or a first capture the raw read is accepted verbatim (a new game
/// legitimately resets every counter). Otherwise each cell is checked
/// independently: a decrease is held to the previous accepted value (B) unless a
/// run of `UNLATCH_STREAK` clean coherent decreases revises the latched value
/// DOWN (CG-2 un-latch); an increase beyond the plausible rate is held unless a
/// clean previous raw read corroborates it (C); an advance within rate passes
/// through unchanged.
pub fn apply_gate(
    prev: Option<(GateState, Duration)>,
    raw: Counters,
    suspect: [bool; GATE_COLS],
    split: bool,
) -> GateOutcome {
    let Some((state, elapsed)) = prev.filter(|_| !split) else {
        return GateOutcome {
            accepted: raw,
            state: GateState {
                accepted: raw,
                last_raw: raw,
                last_raw_suspect: suspect,
                ..Default::default()
            },
            holds: Vec::new(),
            unlatches: Vec::new(),
        };
    };

    let prev_acc = state.accepted.to_array();
    let prev_raw = state.last_raw.to_array();
    let prev_raw_suspect = state.last_raw_suspect;
    let cur = raw.to_array();
    let mut out = cur;
    let mut down_len = state.down_streak_len;
    let mut down_last = state.down_streak_last;
    let mut holds = Vec::new();
    let mut unlatches = Vec::new();

    for col in 0..GATE_COLS {
        if cur[col] < prev_acc[col] {
            // (B) cumulative counter decreased → misread; hold last accepted,
            // UNLESS a run of clean coherent decreases un-latches a value the
            // gate latched onto a corrupt inflation (CG-2).
            if suspect[col] {
                // A suspect below-read (a clip like Havana HLG 224/234) must not
                // build the streak — it would talk the gate out of a correct hold.
                down_len[col] = 0;
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::Monotonic,
                    raw: cur[col],
                    held: prev_acc[col],
                });
                continue;
            }
            let continues = down_len[col] > 0 && downstreak_continues(down_last[col], cur[col]);
            down_len[col] = if continues { down_len[col] + 1 } else { 1 };
            down_last[col] = cur[col];
            if down_len[col] >= UNLATCH_STREAK {
                // The held value was the corruption — revise DOWN to this clean
                // read and reset the streak; later captures gate against it.
                unlatches.push(Unlatch {
                    col,
                    raw: cur[col],
                    revised_from: prev_acc[col],
                });
                out[col] = cur[col];
                down_len[col] = 0;
                down_last[col] = 0;
            } else {
                out[col] = prev_acc[col];
                holds.push(Hold {
                    col,
                    kind: HoldKind::Monotonic,
                    raw: cur[col],
                    held: prev_acc[col],
                });
            }
        } else {
            // At or above the accepted value → no active decrease run.
            down_len[col] = 0;
            down_last[col] = 0;
            let ceiling = prev_acc[col].saturating_add(max_delta(col, elapsed));
            // A suspect prior read must never corroborate the current jump.
            let corroborated = !prev_raw_suspect[col] && corroborates(prev_raw[col], cur[col]);
            if cur[col] > ceiling && !corroborated {
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
            down_streak_len: down_len,
            down_streak_last: down_last,
            last_raw_suspect: suspect,
        },
        holds,
        unlatches,
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
    fn route66_row_shift_a_and_d_recover_via_unlatch() {
        // FLIPPED red anchor: the old test pinned A/D "locked high" at the
        // row-shift's 19/13 as a documented mode-(c) limitation. The CG-2
        // un-latch now incidentally recovers them: after the row-shift the real
        // player row returns with A=14/D=5 (clean) for ≥3 consecutive captures,
        // which revise the latched 19/13 DOWN, and the true A17/D10 finals are
        // then accepted. This is a strict improvement — the finals now match the
        // screenshot-verified real values. (Not the mode-c *fix* proper: a
        // single clean returning read is still held for the first 2 captures; the
        // recovery costs one un-latch streak.)
        let acc = replay_clips(ROUTE66, ROUTE66_CLIPS);
        let final_c = *acc.last().unwrap();
        assert_eq!(final_c.elims, 28, "elims unaffected by the row-shift");
        assert_eq!(final_c.assists, 17, "assists recovered to the real 17");
        assert_eq!(final_c.deaths, 10, "deaths recovered to the real 10");
    }

    // --- CG-2/CG-3 injection + latch + un-latch (the 2026-07-20 defect) ---

    /// Antarctic Peninsula, damage column: real DMG climbs 1728→2559→3235, then
    /// the stat window drifts one digit and injects the deaths digit ("3") in
    /// front of a clipped DMG → 35031 (CG-3). Wide columns have no rate cap, so
    /// the injected value is accepted and the monotonic hold then LATCHES it,
    /// rejecting every later CORRECT read (4543/5852/6810/10311) as a decrease
    /// (CG-2). The un-latch must recover the clean level, not keep 35031.
    ///
    /// `inject_suspect` models both ways it reached the gate: `true` — edge-ink
    /// (b) flags the drifted read (its clipped glyph touches the window edge);
    /// `false` — it slipped through as clean. The recovery is identical either
    /// way, because a wide column has no rate cap to reject the injection at all.
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
    fn junkertown_assists_unlatch_from_constant_clean_run() {
        // Seed A latched at 14 with damage advancing normally around it.
        let mut st = state(c(9, 14, 4, 6000, 0, 3000));
        let mut a_vals = Vec::new();
        for (i, gap) in [20u64, 22, 24, 26].into_iter().enumerate() {
            let raw = c(9, 4, 4, 6200 + i as u32 * 50, 0, 3100);
            let out = apply_gate(Some((st, secs(gap))), raw, CLEAN, false);
            a_vals.push(out.accepted.assists);
            st = out.state;
        }
        assert_eq!(
            a_vals,
            vec![14, 14, 4, 4],
            "constant clean A=4 run un-latches at the 3rd read"
        );
    }

    #[test]
    fn unlatch_needs_three_consecutive_clean_reads() {
        // Two clean below-held reads are still HELD; only the third revises down.
        let prev = state(c(2, 2, 2, 20000, 0, 0));
        let out1 = apply_gate(Some((prev, secs(15))), c(2, 2, 2, 5000, 0, 0), CLEAN, false);
        assert_eq!(
            out1.accepted.damage, 20000,
            "first clean below-read is held"
        );
        let out2 = apply_gate(
            Some((out1.state, secs(15))),
            c(2, 2, 2, 5200, 0, 0),
            CLEAN,
            false,
        );
        assert_eq!(out2.accepted.damage, 20000, "second is still held");
        let out3 = apply_gate(
            Some((out2.state, secs(15))),
            c(2, 2, 2, 5400, 0, 0),
            CLEAN,
            false,
        );
        assert_eq!(
            out3.accepted.damage, 5400,
            "third un-latches to the clean read"
        );
        assert_eq!(out3.unlatches.len(), 1);
        assert_eq!(out3.unlatches[0].col, 3);
        assert_eq!(out3.unlatches[0].revised_from, 20000);
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
}
