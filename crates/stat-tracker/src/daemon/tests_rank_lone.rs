//! The lone-read fallback of [`settle_held`]: a rank screen with no pair of
//! reads that agree still keeps the fields of one late confident read that
//! no other read disagrees with, and nothing else changes.

use super::*;
use chrono::TimeDelta;

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

fn secs(s: i64) -> TimeDelta {
    TimeDelta::seconds(s)
}

fn late() -> Duration {
    RANK_LONE_READ_MIN_AGE + Duration::from_secs(1)
}

/// Dorado 2026-11-17: a frame the end detector read as the rank screen
/// that rank-v1 did not read, a break, then the held-back frame after the
/// focus pause read once at +17 s, then the screen is gone.
fn dorado(lone: RankUpdate) -> Option<Settled> {
    let t0 = Utc::now();
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    assert_eq!(feed.observe_tick(Some(None), false, Some("g1"), t0), None);
    assert_eq!(
        feed.observe_tick(None, false, Some("g1"), t0 + secs(1)),
        None
    );
    let at = t0 + secs(17);
    assert_eq!(
        feed.observe_tick(Some(Some(lone)), true, Some("g1"), at),
        None
    );
    assert_eq!(feed.observe(None, Some("g1"), at + secs(1)), None);
    feed.observe(None, Some("g1"), at + secs(2))
}

#[test]
fn the_dorado_case_keeps_the_one_late_confident_read() {
    // the gate admits the real case
    assert!(RANK_LONE_READ_MIN_AGE <= Duration::from_secs(17));
    let s = dorado(read(40, sure())).expect("settled with a rank");
    assert_eq!(s.session, "g1");
    assert_eq!(s.rank, read(40, sure()), "the read's own confidence");
}

#[test]
fn a_lone_read_keeps_only_its_fields_that_pass_the_confidence_gate() {
    let mut lone = read(40, sure());
    lone.hero_sr = n(3412, weak());
    lone.division = None;
    let s = dorado(lone).expect("settled with a rank");
    assert_eq!(s.rank.hero_sr, None, "under the gate");
    assert_eq!(s.rank.division, None, "not read");
    assert_eq!(s.rank.role_rank, t("Diamond", sure()));
    // a lone read under the gate on every field keeps nothing
    assert_eq!(dorado(read(40, weak())), None);
}

#[test]
fn a_lone_read_under_the_min_age_keeps_nothing() {
    let early = RANK_LONE_READ_MIN_AGE - Duration::from_secs(1);
    let (r, lone) = settle_held(&[None, Some(read(40, 1.0))], &[Duration::ZERO, early]);
    assert!(r.is_empty() && lone.is_empty(), "{r:?}");
    // through the feed: a screen read once at its first sighting
    let t0 = Utc::now();
    let mut feed = RankFeed::with_read(Arc::new(|_| None));
    feed.observe(Some(Some(read(40, 1.0))), Some("g1"), t0);
    assert_eq!(feed.close(), None);
    // the age counts from the first sighting, not the previous entry
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
    assert!(!lone.contains(&"progress_pct"));
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
    assert!(kept.contains(&"hero_sr"));
    assert_eq!(r.division, None, "under the gate");
    assert!(!kept.contains(&"division"));
    // a read at the gate on every field keeps every field, a hair under
    // keeps nothing
    let (r, _) = settle_held(&[None, Some(read(40, at))], &[Duration::ZERO, late()]);
    assert_eq!(r, read(40, at));
    let (r, _) = settle_held(&[None, Some(read(40, under))], &[Duration::ZERO, late()]);
    assert!(r.is_empty(), "{r:?}");
}
