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

fn t0() -> DateTime<Utc> {
    DateTime::parse_from_rfc3339("2026-11-03T12:00:00Z")
        .unwrap()
        .with_timezone(&Utc)
}

fn mins(m: i64) -> DateTime<Utc> {
    t0() + Duration::minutes(m)
}

/// A hint read on `source` at `t0`.
fn hint(text: &str, source: &str) -> Option<PendingHint> {
    Some(PendingHint {
        text: text.to_string(),
        source: source.to_string(),
        at: t0(),
        target: None,
        target_folder: None,
    })
}

/// `session` first recorded at minute `started`, read at minute `now`.
fn game(session: &str, started: i64, now: i64) -> ReadGame<'_> {
    ReadGame {
        session,
        started: mins(started),
        now: mins(now),
    }
}

#[test]
fn a_confident_read_sets_the_queue() {
    let mut pending = None;
    let (t, hint) = apply_read(
        &field(Some("ROLE QUEUE"), 0.91),
        &mut pending,
        game("g2", 2, 3),
    );
    assert_eq!(t.value.as_deref(), Some("ROLE QUEUE"));
    assert_eq!(t.confidence, 0.91);
    assert!(!t.unsure);
    assert_eq!(t.warning(), None);
    assert_eq!(hint, HintOutcome::NoHint);
    // Exactly at the reader's bar is confident.
    let (t, _) = apply_read(
        &field(Some("OPEN QUEUE"), MIN_SCORE),
        &mut pending,
        game("g2", 2, 3),
    );
    assert_eq!((t.value.as_deref(), t.unsure), (Some("OPEN QUEUE"), false));
}

#[test]
fn a_weak_or_failed_read_warns_with_no_queue() {
    for weak in [
        Field::none(),
        field(None, 0.97),
        // a value under the reader's bar counts as weak too
        field(Some("ROLE QUEUE"), MIN_SCORE - 0.01),
        field(Some("ROLE QUEUE"), f32::NAN),
        field(Some("   "), 0.99),
    ] {
        let mut pending = None;
        let (t, _) = apply_read(&weak, &mut pending, game("g2", 2, 3));
        assert_eq!((t.value.as_deref(), t.confidence), (None, 0.0), "{weak:?}");
        assert!(t.unsure);
        assert_eq!(t.warning(), Some(stat_tracker::model::QUEUE_TITLE_WARNING));
    }
    assert!(!stat_tracker::model::QUEUE_TITLE_WARNING.contains('\u{2014}'));
}

#[test]
fn the_next_game_confirms_or_drops_the_hint() {
    // Agrees: confirmed and stored on the game.
    let mut pending = hint("open", "g1");
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.9),
        &mut pending,
        game("g2", 2, 3),
    );
    assert_eq!(outcome, HintOutcome::Confirmed("open".into()));
    assert_eq!(t.mode_hint.as_deref(), Some("open"));
    assert_eq!(pending, None);
    // Disagrees: dropped, nothing stored.
    let mut pending = hint("ROLE QUEUE", "g1");
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.9),
        &mut pending,
        game("g2", 2, 3),
    );
    assert_eq!(outcome, HintOutcome::Dropped("ROLE QUEUE".into()));
    assert_eq!((t.mode_hint, pending), (None, None));
}

/// Changed on purpose: a weak read used to leave the hint for whatever game
/// read confidently next. The hint names the next game only, so the next
/// game's weak read expires it.
#[test]
fn the_next_game_expires_the_hint_on_a_weak_read() {
    let mut pending = hint("open", "g1");
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.5),
        &mut pending,
        game("g2", 2, 3),
    );
    assert_eq!(outcome, HintOutcome::Expired("open".into()));
    assert_eq!((t.mode_hint, pending), (None, None));
}

#[test]
fn two_weak_games_then_a_confident_one_store_no_hint() {
    let mut pending = hint("open", "g1");
    let (_, outcome) = apply_read(&Field::none(), &mut pending, game("g2", 2, 3));
    assert_eq!(outcome, HintOutcome::Expired("open".into()));
    let (_, outcome) = apply_read(&Field::none(), &mut pending, game("g3", 15, 16));
    assert_eq!(outcome, HintOutcome::NoHint);
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g4", 25, 26),
    );
    assert_eq!(outcome, HintOutcome::NoHint);
    assert_eq!(
        t.mode_hint, None,
        "the hint was the second game's, not this one's"
    );
}

#[test]
fn the_source_game_and_an_older_game_never_settle_the_hint() {
    // The game the hint was read on (a Tab first pressed after its result).
    let mut pending = hint("open", "g1");
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g1", -10, 1),
    );
    assert_eq!(outcome, HintOutcome::Pending("open".into()));
    assert_eq!(t.mode_hint, None);
    // A different game that started before the hint was read.
    let (_, outcome) = apply_read(&Field::none(), &mut pending, game("g0", -20, 1));
    assert_eq!(outcome, HintOutcome::Pending("open".into()));
    // The next game still gets it.
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g2", 2, 3),
    );
    assert_eq!(outcome, HintOutcome::Confirmed("open".into()));
    assert_eq!(t.mode_hint.as_deref(), Some("open"));
}

#[test]
fn a_hint_older_than_the_game_window_expires() {
    let window = GAME_WINDOW_MINUTES;
    // Read too late, even on its own game.
    let mut pending = hint("open", "g1");
    let (_, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g1", -10, window + 1),
    );
    assert_eq!(outcome, HintOutcome::Expired("open".into()));
    assert_eq!(pending, None);
    // A game that started long after the hint never confirms it.
    let mut pending = hint("open", "g1");
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g5", window + 1, window + 1),
    );
    assert_eq!(outcome, HintOutcome::Expired("open".into()));
    assert_eq!(t.mode_hint, None);
    // Inside the window it still settles.
    let mut pending = hint("open", "g1");
    let (_, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g2", window - 1, window),
    );
    assert_eq!(outcome, HintOutcome::Confirmed("open".into()));
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
        // hint_agrees stays strict: a raw banner does not agree. The rank
        // wiring drops COMPETITIVE (and a 5V5 or 6V6 after it) in
        // queue_hint before it sets the hint.
        ("COMPETITIVE: ROLE QUEUE", "ROLE QUEUE"),
        ("QUEUE", "QUEUE"),
        ("", "OPEN QUEUE"),
        ("OPEN QUEUE PLUS", "OPEN QUEUE"),
    ] {
        assert!(!hint_agrees(hint, title), "{hint:?} {title:?}");
    }
    assert_eq!(queue_key("open_queue"), "OPEN");
    assert_eq!(queue_key("5V5 ROLE QUEUE"), "5V5 ROLE");
    assert_eq!(queue_key("COMPETITIVE: ROLE QUEUE"), "COMPETITIVE ROLE");
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
        "mapvote_first.png",
        "ban_results.png",
        "loading.png",
        "heroban_last",
    ] {
        assert_eq!(policy_slot(other), None, "{other}");
    }
}

#[test]
fn a_hint_tied_to_a_game_never_read_expires_on_a_later_game() {
    // Frames were held for g2 after the hint: the hint is g2's.
    let tied = |target: &str| {
        hint("open", "g1").map(|h| PendingHint {
            target: Some(target.to_string()),
            ..h
        })
    };
    // g2 was never read; g3, a confident agreeing read, does not get it.
    let mut pending = tied("g2");
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g3", 4, 5),
    );
    assert_eq!(outcome, HintOutcome::Expired("open".into()));
    assert_eq!((t.mode_hint, pending), (None, None));
    // g2 itself still settles it.
    let mut pending = tied("g2");
    let (t, outcome) = apply_read(
        &field(Some("OPEN QUEUE"), 0.93),
        &mut pending,
        game("g2", 2, 3),
    );
    assert_eq!(outcome, HintOutcome::Confirmed("open".into()));
    assert_eq!(t.mode_hint.as_deref(), Some("open"));
    // The source and an older game still leave it waiting.
    let mut pending = tied("g2");
    let (_, outcome) = apply_read(&Field::none(), &mut pending, game("g1", -10, 1));
    assert_eq!(outcome, HintOutcome::Pending("open".into()));
    let (_, outcome) = apply_read(&Field::none(), &mut pending, game("g0", -20, 1));
    assert_eq!(outcome, HintOutcome::Pending("open".into()));
    assert!(pending.is_some());
}
