//! Tests of the per-column holds: monotonic holds, the rate caps, jumps,
//! row shifts and the CG-4 B1/B2 rules.

use super::tests::{CLEAN, c, secs, state};
use super::*;

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
        assert!(
            out.holds
                .iter()
                .any(|h| h.col == 0 && h.kind == HoldKind::Monotonic && h.raw == 1 && h.held == 11)
        );
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
fn an_unconfirmed_high_yields_to_one_clean_lower_read() {
    // Fallback elims 11. One clean read of 1 replaces it and confirms
    // the column. A later 9 is ordinary growth. A drop back to 1 holds.
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
    assert_eq!(one.accepted.elims, 1);
    assert!(
        one.unlatches
            .iter()
            .any(|u| u.col == 0 && u.raw == 1 && u.replaced_unconfirmed)
    );
    assert!(!one.state.low_trust);
    let nine = apply_gate_with_trust(
        Some((one.state, secs(20))),
        c(9, 6, 3, 3200, 900, 500),
        CLEAN,
        false,
        true,
    );
    assert_eq!(nine.accepted.elims, 9);
    let after = apply_gate_with_trust(
        Some((nine.state, secs(20))),
        c(1, 6, 3, 3400, 900, 500),
        CLEAN,
        false,
        true,
    );
    assert_eq!(
        after.accepted.elims, 9,
        "the clean 1 confirmed, so a later drop holds"
    );
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
