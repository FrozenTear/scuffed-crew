//! Real-series replay fixtures (numeric only, from `matches.jsonl`):
//! Havana, Route 66, Antarctic and Numbani.

use super::tests::c;
use super::*;

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
