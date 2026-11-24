//! Tests for the roster sample count's cadence (`pregame_dump/pending.rs`
//! `TickGaps`): the median of the last three gaps between fed ticks, not
//! one gap, picks one or two samples. Synthetic ticks only.

use super::pregame_dump::*;
use super::tests_pregame_dump::Run;
use std::time::{Duration, Instant};

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

fn gaps(poll: Duration, ticks_ms: &[u64]) -> Duration {
    let start = Instant::now();
    let mut gaps = TickGaps::default();
    for &at in ticks_ms {
        gaps.tick(start + ms(at));
    }
    gaps.median(poll)
}

#[test]
fn the_median_takes_the_middle_of_the_last_three_gaps() {
    let poll = ms(1_500);
    assert_eq!(gaps(poll, &[]), poll, "no gap yet: the configured poll");
    assert_eq!(gaps(poll, &[0]), poll, "one tick: no gap yet");
    assert_eq!(
        gaps(poll, &[0, 4_000]),
        poll,
        "one slow gap, two unmeasured"
    );
    assert_eq!(
        gaps(poll, &[0, 1_500, 3_000, 6_000]),
        ms(1_500),
        "1.5, 1.5, 3"
    );
    assert_eq!(gaps(poll, &[0, 3_000, 6_000, 9_000]), ms(3_000), "3, 3, 3");
    assert_eq!(
        gaps(poll, &[0, 1_500, 4_500, 7_500]),
        ms(3_000),
        "1.5, 3, 3"
    );
    // Only the last three count: an old slow stretch is forgotten.
    let old_slow = [0, 4_000, 8_000, 12_000, 13_500, 15_000, 16_500];
    assert_eq!(gaps(poll, &old_slow), ms(1_500));
    // The configured poll fills the unmeasured slots at ocr-v1's 4 s.
    assert_eq!(gaps(ms(4_000), &[0]), ms(4_000));
}

#[test]
fn clear_forgets_the_gaps_and_the_last_tick() {
    let start = Instant::now();
    let mut gaps = TickGaps::default();
    for at in [0, 4_000, 8_000, 12_000] {
        gaps.tick(start + ms(at));
    }
    assert_eq!(gaps.median(ms(1_500)), ms(4_000));
    gaps.clear();
    assert_eq!(gaps.median(ms(1_500)), ms(1_500));
    // The first tick after a clear measures no gap from the old last tick.
    gaps.tick(start + ms(60_000));
    assert_eq!(gaps.median(ms(1_500)), ms(1_500));
}

#[test]
fn one_slow_tick_among_normal_ones_still_needs_two_samples() {
    // 1.5 s polls, then one missed poll (a 3 s gap) right before the
    // roster: one roster frame is not enough.
    let mut run = Run::new();
    for (i, at) in [0, 1_500, 3_000, 4_500].into_iter().enumerate() {
        run.tick(at, Seen::Other, i as u32);
    }
    run.tick(7_500, Seen::Roster, 10);
    assert!(
        !run.tracker.holds_pending_roster(),
        "one sample after a miss"
    );
    // The next normal poll still shows it: two in a row, accepted.
    run.tick(9_000, Seen::Roster, 11);
    assert!(run.tracker.holds_pending_roster());
}

#[test]
fn a_sustained_slow_cadence_takes_one_sample() {
    // Configured 1.5 s, but the route polls every 4 s for good.
    let mut run = Run::new();
    for (i, at) in [0, 4_000, 8_000].into_iter().enumerate() {
        run.tick(at, Seen::Other, i as u32);
    }
    run.tick(12_000, Seen::Roster, 10);
    assert!(
        run.tracker.holds_pending_roster(),
        "three 4 s gaps: one sample"
    );
}

#[test]
fn a_configured_slow_poll_takes_one_sample_from_the_first_tick() {
    let mut run = Run::polled(4_000);
    run.tick(0, Seen::Roster, 1);
    assert!(run.tracker.holds_pending_roster());
}

#[test]
fn a_release_starts_the_gaps_over() {
    let mut run = Run::new();
    for (i, at) in [0, 4_000, 8_000, 12_000].into_iter().enumerate() {
        run.tick(at, Seen::Other, i as u32);
    }
    run.release(13_000);
    run.tick(17_000, Seen::Roster, 10);
    assert!(
        !run.tracker.holds_pending_roster(),
        "the slow gaps before the release are gone"
    );
}
