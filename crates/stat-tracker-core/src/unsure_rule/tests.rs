use super::*;

fn sure(v: u32) -> StatRead {
    StatRead::sure(v, 0.9)
}

fn unsure(v: u32) -> StatRead {
    StatRead::unsure(Some(v), 0.2)
}

fn assert_conf(got: Option<f32>, want: f32) {
    let got = got.expect("a confidence");
    assert!((got - want).abs() < 1e-6, "confidence {got}, want {want}");
}

fn verdict(reads: &[StatRead], over: bool) -> (Verdict, Option<u32>, Reason) {
    let c = confirm_stat(reads, over);
    (c.verdict, c.value, c.reason)
}

#[test]
fn sure_reads_settle() {
    let r = confirm_stat(&[sure(0), sure(3), sure(3), sure(5)], false);
    assert_eq!(r.verdict, Verdict::Confirmed);
    assert_eq!(r.value, Some(5));
    assert_eq!(r.reason, Reason::SureRead);
    assert_eq!(r.reason.as_str(), "sure_read");
    assert_conf(r.confidence, 0.9);
    assert_eq!(r.unsettled_reads, 0);
}

#[test]
fn no_reads_is_unsettled_live_and_unsure_at_end() {
    assert_eq!(
        verdict(&[], false),
        (Verdict::Unsettled, None, Reason::NoReads)
    );
    assert_eq!(
        verdict(&[], true),
        (Verdict::Unsure, None, Reason::UnsureAtEnd)
    );
}

#[test]
fn a_single_bad_tick_does_not_flag() {
    let reads = [sure(1200), unsure(1850)];
    assert_eq!(
        verdict(&reads, false),
        (Verdict::Unsettled, Some(1850), Reason::Pending)
    );
    // the next sure read above clears it
    let reads = [sure(1200), unsure(1850), sure(1300)];
    assert_eq!(
        verdict(&reads, false),
        (Verdict::Confirmed, Some(1300), Reason::SureRead)
    );
    // an empty read alone does not flag either, and shows the kept value
    let reads = [sure(1200), StatRead::unsure(None, 0.0)];
    assert_eq!(
        verdict(&reads, false),
        (Verdict::Unsettled, Some(1200), Reason::Pending)
    );
}

#[test]
fn unsettled_reads_in_a_row_become_unsure() {
    let mut reads = vec![sure(10)];
    for k in 0..UNSURE_AFTER_READS {
        reads.push(unsure(20 + k));
        let r = confirm_stat(&reads, false);
        assert_eq!(r.unsettled_reads, k + 1);
        if k + 1 < UNSURE_AFTER_READS {
            assert_eq!(r.verdict, Verdict::Unsettled);
        } else {
            assert_eq!(
                (r.verdict, r.reason),
                (Verdict::Unsure, Reason::StillUnsure)
            );
            assert_eq!(r.value, Some(20 + k));
        }
    }
}

#[test]
fn unsure_at_game_end() {
    let reads = [sure(4), unsure(5)];
    assert_eq!(
        verdict(&reads, true),
        (Verdict::Unsure, Some(5), Reason::UnsureAtEnd)
    );
    assert_eq!(confirm_stat(&reads, true).reason.as_str(), "unsure_at_end");
    // a confirmed cell stays confirmed at the end
    let reads = [unsure(5), sure(5)];
    assert_eq!(
        verdict(&reads, true),
        (Verdict::Confirmed, Some(5), Reason::SureRead)
    );
}

#[test]
fn a_later_higher_sure_read_confirms() {
    let reads = [unsure(7), unsure(7), sure(9)];
    assert_eq!(
        verdict(&reads, true),
        (Verdict::Confirmed, Some(9), Reason::SureRead)
    );
}

#[test]
fn a_sure_read_below_the_confirmed_value_is_not_confirmed() {
    // stats only grow: 3080 then 3060 means one of them is wrong
    let reads = [sure(3080), sure(3060)];
    assert_eq!(
        verdict(&reads, false),
        (Verdict::Unsettled, Some(3060), Reason::Pending)
    );
    let reads = [sure(3080), sure(3060), sure(3100)];
    assert_eq!(verdict(&reads, false).0, Verdict::Confirmed);
}

#[test]
fn an_unsure_read_equal_to_the_confirmed_value_agrees() {
    let reads = [sure(0), StatRead::unsure(Some(0), 0.1)];
    let r = confirm_stat(&reads, true);
    assert_eq!(
        (r.verdict, r.value, r.reason),
        (Verdict::Confirmed, Some(0), Reason::AgreesWithSure)
    );
    assert_conf(r.confidence, 0.9);
}

#[test]
fn a_reset_starts_the_cell_over() {
    let mut fresh = sure(2);
    fresh.reset = true;
    // without the reset, 2 under 40 would contradict
    let reads = [sure(40), fresh];
    assert_eq!(
        verdict(&reads, false),
        (Verdict::Confirmed, Some(2), Reason::SureRead)
    );
    let mut fresh = unsure(2);
    fresh.reset = true;
    let reads = [sure(40), unsure(41), unsure(42), fresh];
    let r = confirm_stat(&reads, false);
    assert_eq!((r.verdict, r.unsettled_reads), (Verdict::Unsettled, 1));
}

#[test]
fn confidence_is_the_best_read_of_the_shown_value() {
    let reads = [
        sure(10),
        StatRead::unsure(Some(12), 0.30),
        StatRead::unsure(Some(12), 0.25),
    ];
    let r = confirm_stat(&reads, false);
    assert_eq!(r.value, Some(12));
    assert_conf(r.confidence, 0.30);
    // a new value starts its confidence over
    let reads = [
        sure(10),
        StatRead::unsure(Some(12), 0.30),
        StatRead::unsure(Some(13), 0.05),
    ];
    assert_conf(confirm_stat(&reads, false).confidence, 0.05);
    // out-of-range confidences are clamped
    let r = confirm_stat(&[StatRead::sure(1, 7.0)], false);
    assert_eq!(r.confidence, Some(1.0));
    let r = confirm_stat(&[StatRead::sure(1, f32::NAN)], false);
    assert_eq!(r.confidence, Some(0.0));
}

// Review of PR 668, finding 1: one literal case per surviving mutant.

#[test]
fn a_stat_that_stays_the_same_stays_confirmed() {
    // M1 (`v >= c` -> `v > c`): an unchanged sure read must not unsettle.
    let reads = [sure(3), sure(3), sure(3), sure(3)];
    let r = confirm_stat(&reads, false);
    assert_eq!(
        (r.verdict, r.value, r.reason, r.unsettled_reads),
        (Verdict::Confirmed, Some(3), Reason::SureRead, 0)
    );
}

#[test]
fn an_agreeing_unsure_read_clears_an_unsettled_run() {
    // M2: the agree arm resets the run.
    let reads = [sure(10), unsure(12), unsure(10)];
    let r = confirm_stat(&reads, false);
    assert_eq!(
        (r.verdict, r.value, r.reason, r.unsettled_reads),
        (Verdict::Confirmed, Some(10), Reason::AgreesWithSure, 0)
    );
}

#[test]
fn an_agreeing_unsure_read_raises_the_confidence() {
    // M3: an agreeing unsure read at 0.8 lifts a 0.4 confirmation.
    let reads = [StatRead::sure(5, 0.4), StatRead::unsure(Some(5), 0.8)];
    let r = confirm_stat(&reads, true);
    assert_eq!(
        (r.verdict, r.reason),
        (Verdict::Confirmed, Reason::AgreesWithSure)
    );
    assert_conf(r.confidence, 0.8);
}

#[test]
fn three_unsettled_reads_at_game_end_are_unsure_at_end() {
    // M4: game end wins over still_unsure.
    let reads = [sure(10), unsure(20), unsure(21), unsure(22)];
    let r = confirm_stat(&reads, true);
    assert_eq!(
        (r.verdict, r.value, r.reason, r.unsettled_reads),
        (Verdict::Unsure, Some(22), Reason::UnsureAtEnd, 3)
    );
}

#[test]
fn unsure_after_three_reads_not_two() {
    // M5: the tuned value is 3.
    assert_eq!(UNSURE_AFTER_READS, 3);
    let reads = [sure(10), unsure(20), unsure(21)];
    assert_eq!(
        verdict(&reads, false),
        (Verdict::Unsettled, Some(21), Reason::Pending)
    );
    let reads = [sure(10), unsure(20), unsure(21), unsure(22)];
    assert_eq!(
        verdict(&reads, false),
        (Verdict::Unsure, Some(22), Reason::StillUnsure)
    );
}

#[test]
fn negative_and_minus_zero_confidences_are_plus_zero() {
    // M8: negatives clamp to 0 (Tesseract's -1 over 100 included).
    let r = confirm_stat(&[StatRead::sure(1, -0.5)], false);
    assert_eq!(r.confidence, Some(0.0));
    let r = confirm_stat(&[StatRead::sure(1, -0.01)], false);
    assert_eq!(r.confidence, Some(0.0));
    // finding 9: -0.0 must not print as "-0%"
    let r = confirm_stat(&[StatRead::sure(1, -0.0)], false);
    assert!(r.confidence.expect("a confidence").is_sign_positive());
    for x in [f32::INFINITY, f32::NEG_INFINITY] {
        assert_eq!(
            confirm_stat(&[StatRead::sure(1, x)], false).confidence,
            Some(0.0)
        );
    }
    assert_eq!(
        confirm_stat(&[StatRead::sure(1, 1.5)], false).confidence,
        Some(1.0)
    );
}

#[test]
fn confidence_is_the_best_read_of_the_value_anywhere_in_the_run() {
    // finding 3: another value or an empty read in between does not reset it
    let reads = [
        sure(10),
        StatRead::unsure(Some(12), 0.30),
        StatRead::unsure(None, 0.0),
        StatRead::unsure(Some(12), 0.25),
    ];
    assert_conf(confirm_stat(&reads, false).confidence, 0.30);
    let reads = [
        StatRead::unsure(Some(14), 0.3),
        StatRead::unsure(None, 0.9),
        StatRead::unsure(Some(14), 0.0),
    ];
    assert_conf(confirm_stat(&reads, false).confidence, 0.3);
    // reads before the last confirmed value do not count
    let reads = [
        StatRead::unsure(Some(12), 0.7),
        sure(11),
        StatRead::unsure(Some(12), 0.2),
    ];
    assert_conf(confirm_stat(&reads, false).confidence, 0.2);
    // a later sure read of the same value replaces the confidence
    let reads = [StatRead::sure(3, 0.9), StatRead::sure(3, 0.4)];
    assert_conf(confirm_stat(&reads, false).confidence, 0.4);
}

#[test]
fn no_read_of_the_shown_value_has_no_confidence() {
    // finding 4: an empty last read keeps 1200 but did not read it
    let reads = [StatRead::sure(1200, 0.9), StatRead::unsure(None, 0.0)];
    let r = confirm_stat(&reads, true);
    assert_eq!(
        (r.verdict, r.value, r.confidence),
        (Verdict::Unsure, Some(1200), None)
    );
    assert_eq!(confirm_stat(&[], false).confidence, None);
    let empty = [StatRead::unsure(None, 0.5); 3];
    let r = confirm_stat(&empty, false);
    assert_eq!(
        (r.verdict, r.value, r.confidence),
        (Verdict::Unsure, None, None)
    );
}

#[test]
fn an_agreeing_read_ends_the_run_for_confidence() {
    // After an agreeing read, an empty read keeps 10 but did not read it.
    let reads = [
        sure(10),
        StatRead::unsure(Some(10), 0.5),
        StatRead::unsure(None, 0.0),
    ];
    let r = confirm_stat(&reads, false);
    assert_eq!(
        (r.verdict, r.value, r.confidence),
        (Verdict::Unsettled, Some(10), None)
    );
}

#[test]
fn reads_before_an_agreeing_read_do_not_count_for_confidence() {
    // The 0.7 read of 12 is before the agreeing 10.
    let reads = [
        sure(10),
        StatRead::unsure(Some(12), 0.7),
        StatRead::unsure(Some(10), 0.5),
        unsure(12),
    ];
    let r = confirm_stat(&reads, false);
    assert_eq!((r.verdict, r.value), (Verdict::Unsettled, Some(12)));
    assert_conf(r.confidence, 0.2);
}

#[test]
fn a_reset_starts_the_confidence_over() {
    // The 0.8 read of 41 is before the reset; the reset read itself counts.
    let mut fresh = unsure(41);
    fresh.reset = true;
    let reads = [sure(40), StatRead::unsure(Some(41), 0.8), fresh];
    let r = confirm_stat(&reads, false);
    assert_eq!(
        (r.verdict, r.value, r.unsettled_reads),
        (Verdict::Unsettled, Some(41), 1)
    );
    assert_conf(r.confidence, 0.2);
}

#[test]
fn unsettled_confidence_is_clamped() {
    let r = confirm_stat(&[StatRead::unsure(Some(12), 1.5)], false);
    assert_eq!(r.confidence, Some(1.0));
    let r = confirm_stat(&[StatRead::unsure(Some(12), f32::NAN)], false);
    assert_eq!(r.confidence, Some(0.0));
}
