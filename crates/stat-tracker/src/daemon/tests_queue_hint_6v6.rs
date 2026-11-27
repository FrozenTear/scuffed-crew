//! Role Queue 6v6 (#654): the next mode hint from a 6v6 role queue rank
//! screen keeps its `6V6`, agrees with a 6v6 role queue title in any word
//! order, and never agrees with the 5v5 `ROLE QUEUE` title or an open
//! queue title.

use super::*;
use crate::daemon::rank::queue_hint;

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-12-08T20:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn hint_from(banner: &str) -> Option<PendingHint> {
    Some(PendingHint {
        text: queue_hint(banner).to_string(),
        source: "rq6-a".to_string(),
        at: t0(),
        target: None,
        target_folder: None,
    })
}

fn next_game() -> ReadGame<'static> {
    ReadGame {
        session: "rq6-b",
        started: t0() + Duration::minutes(2),
        now: t0() + Duration::minutes(4),
    }
}

/// A 6v6 role queue banner hands on its `6V6`, in either word order and
/// with any separator; every other banner keeps today's hint.
#[test]
fn a_role_queue_banner_keeps_its_6v6() {
    for (banner, hint) in [
        ("COMPETITIVE: 6V6 ROLE QUEUE", "6V6 ROLE QUEUE"),
        ("COMPETITIVE: ROLE QUEUE 6V6", "ROLE QUEUE 6V6"),
        ("competitive-6v6-role-queue", "6v6-role-queue"),
        (" COMPETITIVE_6V6_ROLE_QUEUE ", "6V6_ROLE_QUEUE"),
        ("COMPETITIVE: 6V6 QUEUE ROLE", "6V6 QUEUE ROLE"),
        // Unchanged: open queue drops its 6V6, 5v5 role queue its 5V5.
        ("COMPETITIVE: 6V6 OPEN QUEUE", "OPEN QUEUE"),
        ("COMPETITIVE: 5V5 ROLE QUEUE", "ROLE QUEUE"),
        ("COMPETITIVE: ROLE QUEUE", "ROLE QUEUE"),
        // A size word with no queue after it, or naming both: as today.
        ("COMPETITIVE: 6V6", "COMPETITIVE: 6V6"),
        ("COMPETITIVE: 6V6 ROLE OPEN QUEUE", "ROLE OPEN QUEUE"),
    ] {
        assert_eq!(queue_hint(banner), hint, "{banner:?}");
    }
}

/// The 6v6 role queue hint agrees only with a 6v6 role queue title, in any
/// word order; the 5v5 hint never agrees with a 6v6 title, and the other
/// way round.
#[test]
fn a_6v6_role_hint_agrees_only_with_a_6v6_role_title() {
    let six = queue_hint("COMPETITIVE: 6V6 ROLE QUEUE");
    let six_late = queue_hint("COMPETITIVE: ROLE QUEUE 6V6");
    let five = queue_hint("COMPETITIVE: ROLE QUEUE");
    for title in [
        "6V6 ROLE QUEUE",
        "ROLE QUEUE 6V6",
        "6v6 Role Queue",
        "ROLE QUEUE: 6V6",
        "6V6-ROLE-QUEUE",
    ] {
        assert!(hint_agrees(six, title), "{six:?} {title:?}");
        assert!(hint_agrees(six_late, title), "{six_late:?} {title:?}");
        assert!(!hint_agrees(five, title), "{five:?} {title:?}");
    }
    for title in [
        "ROLE QUEUE",
        "OPEN QUEUE",
        "6V6 OPEN QUEUE",
        "COMPETITIVE: 6V6 ROLE QUEUE",
        "5V5 ROLE QUEUE",
        "6V6",
    ] {
        assert!(!hint_agrees(six, title), "{six:?} {title:?}");
        assert!(!hint_agrees(six_late, title), "{six_late:?} {title:?}");
    }
    assert!(hint_agrees(five, "ROLE QUEUE"));
    assert!(hint_agrees(five, "QUEUE ROLE"));
}

/// Through `settle_hint` and `apply_read`: a 6v6 role queue rank screen's
/// hint is dropped by a next game whose title reads the 5v5 `ROLE QUEUE`,
/// and confirmed (stored on that game) by a 6v6 role queue title in the
/// other word order.
#[test]
fn the_next_game_settles_a_6v6_role_hint_by_its_own_title() {
    let banner = "COMPETITIVE: 6V6 ROLE QUEUE";
    let mut pending = hint_from(banner);
    assert_eq!(
        settle_hint(&mut pending, Some("ROLE QUEUE"), next_game()),
        HintOutcome::Dropped("6V6 ROLE QUEUE".into())
    );
    assert_eq!(pending, None);

    let mut pending = hint_from(banner);
    let read = Field {
        value: Some("ROLE QUEUE 6V6".to_string()),
        confidence: 0.93,
        bbox: None,
    };
    let (title, outcome) = apply_read(&read, &mut pending, next_game());
    assert_eq!(outcome, HintOutcome::Confirmed("6V6 ROLE QUEUE".into()));
    assert_eq!(title.mode_hint.as_deref(), Some("6V6 ROLE QUEUE"));
    assert_eq!(title.value.as_deref(), Some("ROLE QUEUE 6V6"));

    // A 5v5 role queue rank screen's hint is dropped by a 6v6 role title.
    let mut pending = hint_from("COMPETITIVE: ROLE QUEUE");
    assert_eq!(
        settle_hint(&mut pending, Some("6V6 ROLE QUEUE"), next_game()),
        HintOutcome::Dropped("ROLE QUEUE".into())
    );
}
