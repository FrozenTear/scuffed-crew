//! The lone-read fallback of [`settle_held`]: a rank screen with no pair of
//! reads that agree still keeps the fields of one confident read taken late
//! after the screen's first read that no other read disagrees with, and
//! nothing else changes.

use super::tests::{MENU, PROGRESSION, RANK, first_seen_on, lit_word_crop, poll, shows};
use super::*;
use stat_tracker::detect::stability::FrameStability;
use stat_tracker::shadow::rank::OPEN_CONFIDENCE;

fn n(value: i64, confidence: f32) -> Option<RankNumber> {
    Some(RankNumber { value, confidence })
}

fn t(value: &str, confidence: f32) -> Option<RankText> {
    Some(RankText {
        value: value.to_string(),
        confidence,
    })
}

/// A full read with every field at confidence `c`.
fn read(pct: i64, c: f32) -> RankUpdate {
    RankUpdate {
        role: t("Support", c),
        role_rank: t("Diamond", c),
        division: n(3, c),
        progress_pct: n(pct, c),
        progress_change: n(-15, c),
        change_tags: Some(RankTags {
            value: vec!["Defeat".to_string()],
            confidence: c,
        }),
        higher_than_pct: n(88, c),
        hero_sr: n(3412, c),
        hero_sr_change: n(21, c),
        next_mode_hint: t("COMPETITIVE: ROLE QUEUE", c),
    }
}

/// Confident enough for the gate, and below it.
fn sure() -> f32 {
    (RANK_LONE_READ_MIN_CONFIDENCE + 1.0) / 2.0
}
fn weak() -> f32 {
    RANK_LONE_READ_MIN_CONFIDENCE - 0.05
}

fn secs(s: u64) -> Duration {
    Duration::from_secs(s)
}

/// An early read that gives only the role rank.
fn early() -> RankUpdate {
    RankUpdate {
        role_rank: t("Diamond", 1.0),
        ..RankUpdate::default()
    }
}

fn late() -> Duration {
    RANK_LONE_READ_MIN_AGE + Duration::from_secs(1)
}

/// A rank screen as the feed holds it: the first entry (`first`, a frame
/// the end detector read as the rank screen), a break, then the held-back
/// frame after the focus pause read once at +17 s, then the screen is gone.
fn screen(first: Option<RankUpdate>, lone: RankUpdate) -> Option<Settled> {
    let t0 = Instant::now();
    let now = Utc::now();
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    let mut tick =
        |seen, held_back, at| feed.observe_tick(seen, held_back, Some("g1"), now, t0 + secs(at));
    assert_eq!(tick(Some(first), false, 0), None);
    assert_eq!(tick(None, false, 1), None);
    assert_eq!(tick(Some(Some(lone)), true, 17), None);
    assert_eq!(tick(None, false, 18), None);
    tick(None, false, 19)
}

/// Dorado 2026-11-17: the first entry is a frame rank-v1 did not read, so
/// the one read is the screen's first read, at age zero.
fn dorado(lone: RankUpdate) -> Option<Settled> {
    screen(None, lone)
}

#[test]
fn the_dorado_case_keeps_nothing_as_its_one_read_starts_the_clock() {
    assert_eq!(dorado(read(40, 1.0)), None);
    // with an earlier read that does not disagree, the clock starts 17 s
    // before the late read
    let s = screen(Some(early()), read(40, sure())).expect("settled with a rank");
    assert_eq!(s.session, "g1");
    assert_eq!(s.rank, read(40, sure()), "the read's own confidence");
}

#[test]
fn a_lone_read_keeps_only_its_fields_that_pass_the_confidence_gate() {
    let mut lone = read(40, sure());
    lone.hero_sr = n(3412, weak());
    lone.division = None;
    let s = screen(Some(early()), lone).expect("settled with a rank");
    assert_eq!(s.rank.hero_sr, None, "under the gate");
    assert_eq!(s.rank.division, None, "not read");
    assert_eq!(s.rank.role_rank, t("Diamond", sure()));
    // a lone read under the gate on every field keeps nothing
    assert_eq!(screen(Some(early()), read(40, weak())), None);
}

#[test]
fn a_lone_read_under_the_min_age_keeps_nothing() {
    let early = RANK_LONE_READ_MIN_AGE - Duration::from_secs(1);
    let (r, lone) = settle_held(&[None, Some(read(40, 1.0))], &[Duration::ZERO, early]);
    assert!(r.is_empty() && lone.is_empty(), "{r:?}");
    // through the feed: a screen read once
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    feed.observe(
        Some(Some(read(40, 1.0))),
        Some("g1"),
        Utc::now(),
        Instant::now(),
    );
    assert_eq!(feed.close(), None);
    // the age counts from the first read, not the previous entry
    let (r, lone) = settle_held(&[None, Some(read(40, 1.0))], &[Duration::ZERO, late()]);
    assert_eq!(r, read(40, 1.0));
    assert_eq!(lone.len(), 10);
}

#[test]
fn two_reads_that_disagree_on_every_field_keep_nothing() {
    // Flashpoint 2026-11-14: entries=3 read=2, a transition frame
    let mut other = read(10, 1.0);
    other.role = t("Tank", 1.0);
    other.role_rank = t("Gold", 1.0);
    other.division = n(1, 1.0);
    other.progress_change = n(20, 1.0);
    other.change_tags = Some(RankTags {
        value: vec!["Victory".to_string()],
        confidence: 1.0,
    });
    other.higher_than_pct = n(40, 1.0);
    other.hero_sr = n(2100, 1.0);
    other.hero_sr_change = n(-3, 1.0);
    other.next_mode_hint = t("COMPETITIVE: 6V6 OPEN QUEUE", 1.0);
    let reads = [Some(other), None, Some(read(40, 1.0))];
    let ages = [late(), late(), late()];
    let (r, lone) = settle_held(&reads, &ages);
    assert!(r.is_empty() && lone.is_empty(), "{r:?}");
}

#[test]
fn a_lone_read_with_another_value_anywhere_keeps_nothing_for_that_field() {
    // the older read disagrees only on the progress, and is not next to it
    let mut older = read(40, 1.0);
    older.progress_pct = n(25, 1.0);
    let reads = [Some(older), None, None, Some(read(40, 1.0))];
    let ages = [Duration::ZERO, late(), late(), late()];
    let (r, lone) = settle_held(&reads, &ages);
    assert_eq!(r.progress_pct, None);
    assert!(!lone.iter().any(|l| l.field == "progress_pct"));
    assert_eq!(r.division, n(3, 1.0), "every other field agrees");
    // same values on both sides of a break: kept at the lower confidence
    let reads = [Some(read(40, 1.0)), None, Some(read(40, sure()))];
    let (r, _) = settle_held(&reads, &[Duration::ZERO, late(), late()]);
    assert_eq!(r, read(40, sure()));
    let reads = [Some(read(40, weak())), None, Some(read(40, 1.0))];
    let (r, _) = settle_held(&reads, &[Duration::ZERO, late(), late()]);
    assert!(r.is_empty(), "the lower confidence is under the gate");
}

#[test]
fn the_pair_rule_still_decides_where_it_keeps_a_field() {
    // an agreed pair stays as the pair rule keeps it, whatever the age
    for age in [Duration::ZERO, late()] {
        let reads = [Some(read(40, 0.3)), Some(read(40, 0.4))];
        let (r, lone) = settle_held(&reads, &[age, age]);
        assert_eq!(r, settle(&reads));
        assert_eq!(r, read(40, 0.3));
        assert!(lone.is_empty());
    }
    // a pair agreed on an older value and a newer read that disagrees
    // ([A, A, B]) keeps nothing, as before: B is not alone
    let reads = [
        Some(read(72, 1.0)),
        Some(read(72, 1.0)),
        Some(read(10, 1.0)),
    ];
    let (r, lone) = settle_held(&reads, &[late(), late(), late()]);
    assert_eq!(r.progress_pct, None);
    assert!(lone.is_empty());
    assert_eq!(r.division, n(3, 1.0));
}

#[test]
fn a_lone_read_at_exactly_the_min_age_is_kept_and_a_hair_under_is_not() {
    assert_eq!(RANK_LONE_READ_MIN_AGE, Duration::from_secs(12));
    let reads = [None, Some(read(40, 1.0))];
    let at = Duration::from_millis(12_000);
    let (r, lone) = settle_held(&reads, &[Duration::ZERO, at]);
    assert_eq!(r, read(40, 1.0), "at the gate");
    assert_eq!(lone.len(), 10);
    let under = Duration::from_millis(11_999);
    let (r, lone) = settle_held(&reads, &[Duration::ZERO, under]);
    assert!(r.is_empty() && lone.is_empty(), "under the gate: {r:?}");
}

#[test]
fn a_field_at_exactly_the_min_confidence_is_kept_and_a_hair_under_is_not() {
    // the same f32 the gate compares with, no rounding in between
    let at: f32 = 0.7;
    assert_eq!(at, RANK_LONE_READ_MIN_CONFIDENCE);
    let under: f32 = 0.699;
    assert!(under < RANK_LONE_READ_MIN_CONFIDENCE);
    let mut lone = read(40, 1.0);
    lone.hero_sr = n(3412, at);
    lone.division = n(3, under);
    let reads = [None, Some(lone)];
    let (r, kept) = settle_held(&reads, &[Duration::ZERO, late()]);
    assert_eq!(r.hero_sr, n(3412, at), "at the gate");
    assert!(kept.iter().any(|l| l.field == "hero_sr"));
    assert_eq!(r.division, None, "under the gate");
    assert!(!kept.iter().any(|l| l.field == "division"));
    // a read at the gate on every field keeps every field, a hair under
    // keeps nothing
    let (r, _) = settle_held(&[None, Some(read(40, at))], &[Duration::ZERO, late()]);
    assert_eq!(r, read(40, at));
    let (r, _) = settle_held(&[None, Some(read(40, under))], &[Duration::ZERO, late()]);
    assert!(r.is_empty(), "{r:?}");
}

/// Numbani 2026-11-13 with a count-up read: the end detector took a
/// progression frame for the rank screen (rank-v1 rejects it), the next
/// tick missed, the focus pause, then the first tick back is held back on
/// the real rank screen 12 s after the false fire and reads values still
/// counting up, then the player leaves the screen. Through the loop's own
/// held-back gate, as `numbani` in the rank tests runs it.
#[test]
fn a_false_fire_before_the_rank_screen_does_not_start_the_lone_read_clock() {
    // the review's scratch values; the final ones are 40 %, 3412 and +21
    let counting = RankUpdate {
        progress_pct: n(12, 1.0),
        hero_sr: n(3390, 1.0),
        hero_sr_change: n(5, 1.0),
        ..read(40, 1.0)
    };
    let mut feed = RankFeed::with_read(Arc::new(move |img: &RgbImage| {
        (img.get_pixel(0, 0).0[0] == RANK).then(|| counting.clone())
    }));
    let mut stability = FrameStability::default();
    let rank_screen = Some(OutcomeSource::RankScreen);
    let mut rank = lit_word_crop();
    rank.put_pixel(0, 0, image::Rgb([RANK, 0, 0]));
    let progression = shows(PROGRESSION);
    let menu = shows(MENU);
    let t0 = Instant::now();
    let mut tick = |feed: &mut RankFeed, source, img: &RgbImage, at| {
        if at == RANK_LONE_READ_MIN_AGE {
            // the focus pause before this tick: no tick reached the feed,
            // and the pause edge dropped the stability history
            stability.reset();
        }
        let first_seen = first_seen_on(img, &mut stability);
        poll(feed, source, first_seen, img, true, t0 + at)
    };
    let mut out = vec![
        tick(&mut feed, rank_screen, &progression, secs(0)),
        tick(&mut feed, None, &progression, secs(1)),
    ];
    assert!(feed.holds_screen(), "the false fire is held");
    let at = RANK_LONE_READ_MIN_AGE;
    out.push(tick(&mut feed, None, &rank, at));
    assert!(feed.holds_screen(), "the held-back frame was read");
    out.push(tick(&mut feed, None, &menu, at + secs(1)));
    out.push(tick(&mut feed, None, &menu, at + secs(2)));
    assert!(!feed.holds_screen(), "settled");
    assert!(out.iter().all(Option::is_none), "{out:?}");
}

#[test]
fn the_clock_keeps_its_start_when_the_first_read_leaves_the_held_ring() {
    let t0 = Instant::now();
    let now = Utc::now();
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    feed.observe(Some(Some(early())), Some("g1"), now, t0);
    // more rank screen frames rank-v1 did not read than the ring holds
    for k in 1..=RANK_HELD_READS as u64 {
        let at = t0 + Duration::from_millis(k * 500);
        feed.observe(Some(None), Some("g1"), now, at);
    }
    feed.observe(Some(Some(read(40, 1.0))), Some("g1"), now, t0 + secs(17));
    let held = feed.held.as_ref().expect("held");
    assert_eq!(held.reads.len(), RANK_HELD_READS);
    assert!(
        held.reads.iter().all(|(r, _)| r.as_ref() != Some(&early())),
        "the first read was trimmed"
    );
    feed.observe(None, Some("g1"), now, t0 + secs(18));
    let s = feed.observe(None, Some("g1"), now, t0 + secs(19));
    assert_eq!(s.expect("settled").rank, read(40, 1.0));
}

#[test]
fn a_clock_that_goes_back_keeps_nothing() {
    let t0 = Instant::now();
    let now = Utc::now();
    let first = t0 + secs(20);
    // a grab time before the first read is age zero, not a negative age;
    // the same read 12 s after it is kept
    for (at, kept) in [(t0, false), (first + RANK_LONE_READ_MIN_AGE, true)] {
        let mut feed = RankFeed::with_read(Arc::new(|_| None));
        feed.observe(Some(Some(early())), Some("g1"), now, first);
        feed.observe(None, Some("g1"), now, first + secs(1));
        feed.observe(Some(Some(read(40, 1.0))), Some("g1"), now, at);
        let rank = feed.close().map(|s| s.rank);
        assert_eq!(rank, kept.then(|| read(40, 1.0)), "{at:?}");
    }
}

/// The Open role is fixed at [`OPEN_CONFIDENCE`] and only means no role
/// icon was seen: it stays under the lone-read floor, so a lone read never
/// stores it.
#[test]
fn a_lone_read_never_keeps_the_open_role() {
    const { assert!(OPEN_CONFIDENCE < RANK_LONE_READ_MIN_CONFIDENCE) };
    let mut lone = read(40, 1.0);
    lone.role = t("Open", OPEN_CONFIDENCE);
    let (r, kept) = settle_held(&[None, Some(lone)], &[Duration::ZERO, late()]);
    assert_eq!(r.role, None);
    assert_eq!(r.role_rank, t("Diamond", 1.0));
    assert_eq!(kept.len(), 9);
    // what the log line names: the read's age and the confidence kept
    assert!(kept.iter().all(|l| l.age == late() && l.confidence == 1.0));
}
