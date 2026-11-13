use super::*;

fn sure(v: u32) -> StatRead {
    StatRead::sure(v, 0.9)
}

fn unsure(v: u32) -> StatRead {
    StatRead::unsure(Some(v), 0.2)
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
    assert!((r.confidence - 0.9).abs() < 1e-6);
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
    assert!((r.confidence - 0.9).abs() < 1e-6);
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
    assert!((r.confidence - 0.30).abs() < 1e-6);
    // a new value starts its confidence over
    let reads = [
        sure(10),
        StatRead::unsure(Some(12), 0.30),
        StatRead::unsure(Some(13), 0.05),
    ];
    assert!((confirm_stat(&reads, false).confidence - 0.05).abs() < 1e-6);
    // out-of-range confidences are clamped
    let r = confirm_stat(&[StatRead::sure(1, 7.0)], false);
    assert_eq!(r.confidence, 1.0);
    let r = confirm_stat(&[StatRead::sure(1, f32::NAN)], false);
    assert_eq!(r.confidence, 0.0);
}
