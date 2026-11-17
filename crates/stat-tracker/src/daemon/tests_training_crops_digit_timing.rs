//! When a game's `stat_digits` set is cut: only from a Tab whose own
//! damage, healing or mitigation has reached [`STAT_DIGITS_MIN_BIG_STAT`]
//! (1000), so the set is not the spawn board of zeros. A Tab under it saves
//! nothing and is not marked offered, so a later Tab of the same game
//! retries; once a set is saved the game offers no more.

use super::*;

/// [`source`] with this Tab's own damage, healing and mitigation.
fn source_with(
    damage: Option<u32>,
    healing: Option<u32>,
    mitigation: Option<u32>,
) -> TabCropSource<'static> {
    TabCropSource {
        damage,
        healing,
        mitigation,
        ..source(None, None)
    }
}

fn queued_digits(dir: &Path) -> usize {
    sync::queued_crops(dir, UploadKind::StatDigits).len()
}

#[test]
fn stat_digits_qualify_at_exactly_1000_and_not_at_999() {
    assert_eq!(STAT_DIGITS_MIN_BIG_STAT, 1000);
    assert!(stat_digits_qualify(Some(1000), Some(0), Some(0)));
    assert!(stat_digits_qualify(Some(1000), Some(1000), Some(1000)));
    assert!(!stat_digits_qualify(Some(999), Some(0), Some(0)));
    assert!(!stat_digits_qualify(Some(999), Some(999), Some(999)));
    assert!(!stat_digits_qualify(Some(0), Some(0), Some(0)));
}

#[test]
fn stat_digits_qualify_with_any_one_big_stat_at_1000() {
    for others in [Some(0), None] {
        assert!(stat_digits_qualify(Some(1000), others, others));
        assert!(stat_digits_qualify(others, Some(1000), others));
        assert!(stat_digits_qualify(others, others, Some(1000)));
    }
    assert!(stat_digits_qualify(Some(0), Some(25_000), Some(999)));
}

#[test]
fn stat_digits_qualify_never_counts_an_unread_big_stat() {
    assert!(!stat_digits_qualify(None, None, None));
    assert!(!stat_digits_qualify(None, Some(999), None));
    assert!(!stat_digits_qualify(Some(0), None, Some(999)));
    assert!(stat_digits_qualify(None, None, Some(1000)));
    assert!(stat_digits_qualify(None, Some(1000), None));
}

#[tokio::test]
async fn a_tab_under_1000_saves_no_digit_set_and_a_later_tab_of_the_game_saves_it_once() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    // The spawn board, then a Tab just under the gate: nothing is cut.
    for (d, h, m) in [(0, 0, 0), (999, 999, 999)] {
        let tab = source_with(Some(d), Some(h), Some(m));
        assert!(!run(dir.path(), "dt-retry", &frame, tab).await);
        assert_eq!(queued_digits(dir.path()), 0);
    }
    // Neither marked the game offered: the first Tab that reaches 1000
    // saves the set.
    let big = source_with(Some(1000), Some(0), Some(0));
    assert!(run(dir.path(), "dt-retry", &frame, big).await);
    assert_eq!(
        queued_digits(dir.path()),
        6,
        "one set from the qualifying Tab"
    );
    // One set per game: a third, qualifying Tab saves nothing more.
    let later = source_with(Some(4200), Some(1800), Some(0));
    assert!(!run(dir.path(), "dt-retry", &frame, later).await);
    assert_eq!(queued_digits(dir.path()), 6);
}

#[tokio::test]
async fn a_tab_at_exactly_1000_saves_a_set_and_999_in_all_three_does_not() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    let under = source_with(Some(999), Some(999), Some(999));
    assert!(!run(dir.path(), "dt-999", &frame, under).await);
    assert_eq!(queued_digits(dir.path()), 0);
    let exact = source_with(Some(0), Some(1000), Some(0));
    assert!(run(dir.path(), "dt-1000", &frame, exact).await);
    assert_eq!(queued_digits(dir.path()), 6);
}

#[tokio::test]
async fn any_one_big_stat_at_1000_saves_a_set() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    let tabs = [
        source_with(Some(1000), Some(0), None),
        source_with(None, Some(1000), Some(0)),
        source_with(Some(0), None, Some(1000)),
    ];
    for (n, tab) in tabs.into_iter().enumerate() {
        let session = format!("dt-any-{n}");
        assert!(run(dir.path(), &session, &frame, tab).await, "{session}");
        assert_eq!(queued_digits(dir.path()), 6 * (n + 1), "{session}");
    }
}

#[tokio::test]
async fn an_unread_big_stat_never_meets_the_gate() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    let unread = source_with(None, None, None);
    assert!(!run(dir.path(), "dt-unread", &frame, unread).await);
    let unread_and_low = source_with(None, Some(999), None);
    assert!(!run(dir.path(), "dt-unread", &frame, unread_and_low).await);
    assert_eq!(queued_digits(dir.path()), 0);
    // An unread value next to a qualifying read still saves the set.
    let read_one = source_with(None, None, Some(1000));
    assert!(run(dir.path(), "dt-unread", &frame, read_one).await);
    assert_eq!(queued_digits(dir.path()), 6);
}

#[tokio::test]
async fn a_new_game_saves_its_own_digit_set_after_the_last_game_saved_one() {
    let dir = tempfile::tempdir().unwrap();
    set_consent(dir.path(), false, true);
    let frame = tab_frame();
    let big = || source_with(Some(3500), Some(0), Some(0));
    assert!(run(dir.path(), "dt-game-1", &frame, big()).await);
    assert!(!run(dir.path(), "dt-game-1", &frame, big()).await);
    assert_eq!(queued_digits(dir.path()), 6);
    // The next game (a new session id) starts under the gate, then saves.
    let spawn = source_with(Some(0), Some(0), Some(0));
    assert!(!run(dir.path(), "dt-game-2", &frame, spawn).await);
    assert!(run(dir.path(), "dt-game-2", &frame, big()).await);
    assert_eq!(queued_digits(dir.path()), 12);
}

/// The capture path fills the gate from the stored row: a spawn board of
/// zeros records and cuts no set, and the game's later Tab past 1000 does.
#[tokio::test]
async fn a_recorded_spawn_tab_waits_for_a_big_stat_before_its_digit_set() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), false, true);
    let spawn = o10_board([sure(0), sure(0), sure(0), sure(0), sure(0), sure(0)]);
    let (recorded, _) = record_tab(&ctx, "dt-capture", sure_tab(Some("ana")), spawn).await;
    assert!(recorded, "a spawn Tab still records");
    // Give a blocking cut, had one started, time to land.
    tokio::time::sleep(Duration::from_millis(300)).await;
    assert_eq!(queued_digits(dir.path()), 0, "no set from the spawn Tab");
    let (recorded, _) = record_tab(&ctx, "dt-capture", sure_tab(Some("ana")), sure_board()).await;
    assert!(recorded);
    let six = || queued_digits(dir.path()) >= 6;
    assert!(wait_for(six).await, "the Tab past 1000 saves the set");
    assert_eq!(queued_digits(dir.path()), 6);
}

/// The capture path reads healing and mitigation for the gate too, not
/// only damage: a board where only one of them is past 1000 saves a set.
#[tokio::test]
async fn a_recorded_tab_with_only_healing_or_mitigation_past_1000_saves_a_set() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), false, true);
    let boards = [
        (
            "dt-heal",
            [sure(2), sure(9), sure(1), sure(0), sure(1200), sure(0)],
        ),
        (
            "dt-mit",
            [sure(4), sure(1), sure(2), sure(0), sure(0), sure(1200)],
        ),
    ];
    for (n, (session, cells)) in boards.into_iter().enumerate() {
        let (recorded, _) =
            record_tab(&ctx, session, sure_tab(Some("ana")), o10_board(cells)).await;
        assert!(recorded, "{session}");
        let set = || queued_digits(dir.path()) >= 6 * (n + 1);
        assert!(wait_for(set).await, "{session} saves a set");
    }
}

/// The digit gate holds back only the digit set: a below-gate Tab still
/// cuts its hero icon.
#[tokio::test]
async fn a_recorded_spawn_tab_still_keeps_its_hero_icon() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    set_consent(dir.path(), true, true);
    let spawn = o10_board([sure(0), sure(0), sure(0), sure(0), sure(0), sure(0)]);
    let tab = panel_tab((1600, 200, 1800, 400));
    let (recorded, _) = record_tab(&ctx, "dt-hero", tab, spawn).await;
    assert!(recorded);
    let hero = || sync::queued_crops(dir.path(), UploadKind::HeroIcon).len() == 1;
    assert!(
        wait_for(hero).await,
        "the hero icon is not gated on big stats"
    );
    assert_eq!(queued_digits(dir.path()), 0);
}
