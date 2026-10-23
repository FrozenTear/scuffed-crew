//! Shared fixtures for the `capture_gate` tests (`c`, `secs`, `CLEAN`,
//! `state`), and the focused tests of the first capture, the fallback and
//! low-trust latches, and the named revised/lowered fields.

use super::*;

pub(super) fn c(e: u32, a: u32, d: u32, dmg: u32, hlg: u32, mit: u32) -> Counters {
    Counters {
        elims: e,
        assists: a,
        deaths: d,
        damage: dmg,
        healing: hlg,
        mitigation: mit,
    }
}

pub(super) fn secs(n: u64) -> Duration {
    Duration::from_secs(n)
}

/// No cell suspect — the common case in the focused unit tests.
pub(super) const CLEAN: [bool; GATE_COLS] = [false; GATE_COLS];

/// A gate state seeded with `accepted == last_raw == v` and no streak, as
/// every focused test's `prev` began before the un-latch fields existed.
pub(super) fn state(v: Counters) -> GateState {
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
fn fallback_latch_yields_lower_columns_to_one_clean_read() {
    // In-range but wrong assists/deaths from the text fallback. One
    // clean read replaces the latch, including the lower columns.
    let first = apply_gate_with_trust(None, c(2, 40, 18, 450, 0, 2), CLEAN, false, false);
    assert!(first.state.low_trust);
    assert_eq!(first.accepted.assists, 40);
    let lower = c(2, 0, 0, 1105, 259, 450);
    let second = apply_gate_with_trust(Some((first.state, secs(20))), lower, CLEAN, false, true);
    assert_eq!(second.accepted, lower);
    assert!(!second.state.low_trust);
    assert!(
        second
            .unlatches
            .iter()
            .any(|u| u.col == 1 && u.raw == 0 && u.revised_from == 40)
    );
    assert!(
        second
            .unlatches
            .iter()
            .any(|u| u.col == 2 && u.raw == 0 && u.revised_from == 18)
    );
}

#[test]
fn implausible_first_capture_does_not_lock() {
    // Shifted timer digits: A 1105, D 259 on the first capture of the
    // game. One clean read replaces them, trusted source or not.
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
        assert_eq!(second.accepted, clean, "trusted={trusted}");
        assert!(!second.state.low_trust, "trusted={trusted}");
    }
}

/// Soot 15:37:06. The Tab read another player's row. Every cell was
/// suspect. Raw elims 21, assists 1, healing 338 and mitigation 369 were
/// held; deaths 15 and damage 9441 used to latch as low-trust, and the
/// real board twelve seconds later (deaths 9, damage 4172) was then held
/// against those bad totals. The saved game was deaths 15, damage 9.4k,
/// marked unsure. The true board is 8 / 16 / 9 / 4172 / 8717 / 1152.
#[test]
fn an_all_suspect_wrong_row_does_not_seed_and_one_clean_read_is_the_board() {
    let bad = c(21, 1, 15, 9441, 338, 369);
    let clean = c(8, 16, 9, 4172, 8717, 1152);
    let all = [true; GATE_COLS];
    for trusted in [false, true] {
        let seeded = apply_gate_with_trust(None, bad, all, false, trusted);
        assert_eq!(seeded.state.boundary_anchor, Some(bad), "trusted={trusted}");
        assert_ne!(seeded.accepted.deaths, 15, "trusted={trusted}");
        assert_ne!(seeded.accepted.damage, 9441, "trusted={trusted}");
        assert_eq!(seeded.accepted, c(0, 0, 0, 0, 0, 0), "trusted={trusted}");
        assert_eq!(
            seeded.state.unread_cols().len(),
            GATE_COLS,
            "trusted={trusted}"
        );
        let after = apply_gate(Some((seeded.state, secs(12))), clean, CLEAN, false);
        assert_eq!(after.accepted, clean, "trusted={trusted}");
        assert_eq!(after.accepted.deaths, 9, "trusted={trusted}");
        assert_eq!(after.accepted.damage, 4172, "trusted={trusted}");
        assert!(!after.state.low_trust, "trusted={trusted}");
        assert!(
            after
                .holds
                .iter()
                .all(|hold| hold.kind != HoldKind::Monotonic),
            "trusted={trusted}: {:?}",
            after.holds
        );
    }

    // A confirmed row already exists. The bad read must not raise it.
    let prior = apply_gate(None, c(8, 16, 8, 3000, 8000, 1000), CLEAN, false);
    let raised = apply_gate(Some((prior.state, secs(12))), bad, all, false);
    assert_eq!(raised.accepted.deaths, 8);
    assert_eq!(raised.accepted.damage, 3000);
    assert!(!raised.state.low_trust);
    let fixed = apply_gate(Some((raised.state, secs(12))), clean, CLEAN, false);
    assert_eq!(fixed.accepted.deaths, 9);
    assert_eq!(fixed.accepted.damage, 4172);
    assert_eq!(fixed.accepted, clean);
    assert!(!fixed.state.low_trust);
}

/// The same bad totals, stored as a low-trust latch (a text fallback, or
/// a row that latched before this discard). One clean read replaces
/// deaths and damage even though both are lower. Assists 1 to 16 in 12
/// seconds is still past the kill-rate cap.
#[test]
fn a_low_trust_latch_yields_to_one_clean_lower_read() {
    let bad = c(21, 1, 15, 9441, 338, 369);
    let clean = c(8, 16, 9, 4172, 8717, 1152);
    let latched = apply_gate_with_trust(None, bad, CLEAN, false, false);
    assert!(latched.state.low_trust);
    assert_eq!(latched.accepted.deaths, 15);
    assert_eq!(latched.accepted.damage, 9441);
    let fixed = apply_gate(Some((latched.state, secs(12))), clean, CLEAN, false);
    assert_eq!(fixed.accepted.deaths, 9);
    assert_eq!(fixed.accepted.damage, 4172);
    assert!(
        fixed
            .holds
            .iter()
            .all(|hold| hold.col != 2 && hold.col != 3),
        "deaths and damage are not held: {:?}",
        fixed.holds
    );
    assert!(
        fixed
            .holds
            .iter()
            .any(|hold| hold.col == 1 && hold.kind == HoldKind::RateCap)
    );
}

/// A clean read that confirmed a higher total still holds a later lower
/// read. Low-trust is what yields; a confirmed column does not.
#[test]
fn a_clean_higher_then_lower_stays_held() {
    let board = c(8, 16, 9, 4172, 8717, 1152);
    let first = apply_gate(None, board, CLEAN, false);
    let higher = c(10, 16, 12, 5000, 8717, 1152);
    let up = apply_gate(Some((first.state, secs(30))), higher, CLEAN, false);
    assert_eq!(up.accepted, higher);
    assert!(!up.state.low_trust);
    let down = apply_gate(Some((up.state, secs(12))), board, CLEAN, false);
    assert_eq!(down.accepted.deaths, 12);
    assert_eq!(down.accepted.damage, 5000);
    assert_eq!(down.accepted.elims, 10);
    assert!(
        down.holds
            .iter()
            .any(|h| h.col == 2 && h.kind == HoldKind::Monotonic && h.raw == 9)
    );
    assert!(
        down.holds
            .iter()
            .any(|h| h.col == 3 && h.kind == HoldKind::Monotonic && h.raw == 4172)
    );
    assert!(!down.state.low_trust);
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
    let next = apply_gate_with_trust(Some((fallback.state, secs(20))), ghost, CLEAN, false, true);
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
    let second = apply_gate_with_trust(Some((first.state, secs(20))), clean, CLEAN, false, true);
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
fn a_wrong_high_fallback_above_the_floor_corrects_on_one_clean_read() {
    // The fallback is in range and above the confirmed value. One clean
    // read below that latch, and still above the floor, replaces it.
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
    let once = apply_gate_with_trust(Some((adopted.state, secs(20))), lower, CLEAN, false, true);
    assert_eq!(once.accepted, lower);
    assert!(!once.state.low_trust);
    assert!(!once.state.has_confirmed_floor.iter().any(|&set| set));
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
fn a_rate_capped_ghost_stays_unconfirmed_and_one_clean_lower_read_replaces_it() {
    // A clean ghost is held, and that column stays unconfirmed. One
    // later clean read of a lower number replaces the latch.
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
    assert_eq!(once.accepted.elims, 2);
    assert!(
        once.unlatches
            .iter()
            .any(|u| u.col == 0 && u.raw == 2 && u.replaced_unconfirmed)
    );
    assert!(!once.state.unconfirmed[0]);
    assert!(!once.state.low_trust);
}

#[test]
fn an_unconfirmed_high_yields_to_one_clean_lower_read_even_when_that_read_is_91() {
    // A single clean 91 under an unconfirmed 1105 replaces it. 91 is
    // under the assist ceiling, so that read confirms the column. A
    // later 1 is then a confirmed drop and stays held. There is no
    // confirmed floor on a first implausible capture.
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
    assert_eq!(ghost.accepted.assists, 91);
    assert!(
        ghost
            .unlatches
            .iter()
            .any(|u| u.col == 1 && u.raw == 91 && u.replaced_unconfirmed)
    );
    assert!(!ghost.state.unconfirmed[1]);
    assert!(!ghost.state.low_trust);
    let once = apply_gate_with_trust(
        Some((ghost.state, secs(20))),
        c(0, 1, 2, 450, 0, 2),
        CLEAN,
        false,
        true,
    );
    assert_eq!(once.accepted.assists, 91, "the clean 91 is confirmed");
    assert!(unsure_fields(&once.holds).contains(&"a"));
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
    // one clean read replaces all six. That is no hold, so
    // `held_fields` alone would flag nothing.
    assert_eq!(once.accepted, c(13, 5, 3, 3700, 720, 300));
    assert!(once.holds.is_empty());
    let all = vec!["e", "a", "d", "dmg", "h", "mit"];
    assert_eq!(revised_fields(&once.unlatches), all);
    assert_eq!(lowered_fields(suspect.accepted, once.accepted), all);
    let twice = apply_gate(
        Some((once.state, secs(20))),
        c(13, 5, 3, 3700, 720, 300),
        CLEAN,
        false,
    );
    assert!(twice.unlatches.is_empty());
    assert!(twice.holds.is_empty());
    assert!(lowered_fields(once.accepted, twice.accepted).is_empty());
    assert_eq!(
        lowered_fields(c(9, 3, 9, 100, 5, 900), c(9, 4, 0, 100, 5, 150)),
        vec!["d", "mit"]
    );
}
