//! The queue title decisions on injected reads: what the record gets, and
//! what becomes of the pending next mode hint.

use super::*;

fn field(value: Option<&str>, confidence: f32) -> Field<String> {
    Field {
        value: value.map(str::to_string),
        confidence,
        bbox: None,
    }
}

fn open_queue() -> QueueTitle {
    QueueTitle {
        value: Some("OPEN QUEUE".into()),
        confidence: 0.95,
        ..QueueTitle::default()
    }
}

#[test]
fn a_confident_read_sets_the_queue() {
    let mut pending = None;
    let (t, hint) = apply_read(None, &field(Some("ROLE QUEUE"), 0.91), &mut pending);
    assert_eq!(t.value.as_deref(), Some("ROLE QUEUE"));
    assert_eq!(t.confidence, 0.91);
    assert!(!t.unsure);
    assert_eq!(t.warning(), None);
    assert_eq!(hint, HintOutcome::NoHint);
    // Over an earlier warning or value, the new confident read wins.
    let warned = QueueTitle {
        unsure: true,
        ..open_queue()
    };
    let (t, _) = apply_read(Some(warned), &field(Some("ROLE QUEUE"), 0.9), &mut pending);
    assert_eq!((t.value.as_deref(), t.unsure), (Some("ROLE QUEUE"), false));
}

#[test]
fn a_weak_or_failed_read_keeps_the_queue_and_warns() {
    for weak in [
        Field::none(),
        field(None, 0.97),
        // a value under the reader's bar counts as weak too
        field(Some("ROLE QUEUE"), MIN_SCORE - 0.01),
        field(Some("   "), 0.99),
    ] {
        let mut pending = None;
        let (t, _) = apply_read(Some(open_queue()), &weak, &mut pending);
        assert_eq!(t.value.as_deref(), Some("OPEN QUEUE"), "{weak:?}");
        assert_eq!(t.confidence, 0.95);
        assert!(t.unsure);
        assert_eq!(t.warning(), Some(stat_tracker::model::QUEUE_TITLE_WARNING));
        // No queue before: still none, with the warning.
        let (t, _) = apply_read(None, &weak, &mut pending);
        assert_eq!(t.value, None);
        assert!(t.warning().is_some());
    }
    assert!(!stat_tracker::model::QUEUE_TITLE_WARNING.contains('\u{2014}'));
}

#[test]
fn the_read_confirms_drops_or_keeps_the_pending_hint() {
    // Agrees: confirmed and stored on the game.
    let mut pending = Some("open".to_string());
    let (t, hint) = apply_read(None, &field(Some("OPEN QUEUE"), 0.9), &mut pending);
    assert_eq!(hint, HintOutcome::Confirmed("open".into()));
    assert_eq!(t.mode_hint.as_deref(), Some("open"));
    assert_eq!(pending, None);
    // Disagrees: dropped, nothing stored.
    let mut pending = Some("ROLE QUEUE".to_string());
    let (t, hint) = apply_read(None, &field(Some("OPEN QUEUE"), 0.9), &mut pending);
    assert_eq!(hint, HintOutcome::Dropped("ROLE QUEUE".into()));
    assert_eq!((t.mode_hint, pending), (None, None));
    // Weak: still pending for the next read.
    let mut pending = Some("open".to_string());
    let (t, hint) = apply_read(None, &field(Some("OPEN QUEUE"), 0.5), &mut pending);
    assert_eq!(hint, HintOutcome::Pending("open".into()));
    assert_eq!(t.mode_hint, None);
    assert_eq!(pending.as_deref(), Some("open"));
}

#[test]
fn agreement_is_the_same_queue_word() {
    for (hint, title) in [
        ("OPEN QUEUE", "OPEN QUEUE"),
        ("open queue", "OPEN QUEUE"),
        ("Open_Queue", "OPEN QUEUE"),
        ("OPEN-QUEUE", "OPEN QUEUE"),
        ("open", "OPEN QUEUE"),
        (" ROLE  ", "ROLE QUEUE"),
    ] {
        assert!(hint_agrees(hint, title), "{hint:?} {title:?}");
    }
    for (hint, title) in [
        ("ROLE", "OPEN QUEUE"),
        ("ROLE QUEUE", "OPEN QUEUE"),
        ("COMPETITIVE", "ROLE QUEUE"),
        ("QUEUE", "QUEUE"),
        ("", "OPEN QUEUE"),
        ("OPEN QUEUE PLUS", "OPEN QUEUE"),
    ] {
        assert!(!hint_agrees(hint, title), "{hint:?} {title:?}");
    }
    assert_eq!(queue_key("open_queue"), "OPEN");
    assert_eq!(queue_key("5V5 ROLE QUEUE"), "5V5 ROLE");
}

#[test]
fn only_the_policy_frames_are_held() {
    assert_eq!(
        policy_slot("heroban_last.png"),
        Some((Screen::HeroBan, "last"))
    );
    assert_eq!(
        policy_slot("heroban_first.png"),
        Some((Screen::HeroBan, "first"))
    );
    assert_eq!(
        policy_slot("heroselect_last.png"),
        Some((Screen::HeroSelect, "last"))
    );
    for other in [
        "heroselect_first.png",
        "mapvote_last.png",
        "banresults.png",
        "heroban_last",
    ] {
        assert_eq!(policy_slot(other), None, "{other}");
    }
}
