//! When a game's `stat_digits` set is cut: only from a Tab whose own
//! damage, healing or mitigation has reached [`STAT_DIGITS_MIN_BIG_STAT`]
//! (1000), so the set is not the spawn board of zeros. A Tab under it saves
//! nothing and is not marked offered, so a later Tab of the same game
//! retries; once a set is saved the game offers no more.

use super::*;
use stat_tracker::games::stat_reads::TabReads;
use stat_tracker::unsure_rule::StatRead;

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

/// The capture path fills the gate from this Tab's sure reads the gate
/// stored: a spawn board of zeros records and cuts no set, and the game's
/// later Tab past 1000 does.
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

// ---- only this Tab's sure accepted read counts

/// One own-row stat read for [`sure_accepted_big_stats`].
fn stat_read(value: Option<u32>, unsure: bool) -> Option<StatRead> {
    Some(StatRead {
        value,
        confidence: 0.9,
        unsure,
        reset: false,
    })
}

/// Reads of 12, 5, 3 and then `big` for damage, healing and mitigation.
fn tab_reads(big: [Option<StatRead>; 3]) -> TabReads {
    let small = |v| stat_read(Some(v), false);
    [small(12), small(5), small(3), big[0], big[1], big[2]]
}

#[test]
fn a_sure_read_the_gate_stored_counts_for_the_big_stat_gate() {
    let reads = tab_reads([
        stat_read(Some(3500), false),
        stat_read(Some(0), false),
        stat_read(Some(1200), false),
    ]);
    let stored = [12, 5, 3, 3500, 0, 1200];
    assert_eq!(
        sure_accepted_big_stats(&reads, &stored),
        [Some(3500), Some(0), Some(1200)]
    );
}

#[test]
fn a_held_unsure_or_unread_cell_never_counts_for_the_big_stat_gate() {
    // Damage held or gate held (unsure, the stored value), healing an
    // unsure read, mitigation never attempted.
    let reads = tab_reads([
        stat_read(Some(3500), true),
        stat_read(Some(1800), true),
        None,
    ]);
    let stored = [12, 5, 3, 3500, 1800, 1200];
    assert_eq!(sure_accepted_big_stats(&reads, &stored), [None; 3]);
    // A sure cell the reader has no number for does not count either.
    let reads = tab_reads([stat_read(None, false), None, None]);
    assert_eq!(sure_accepted_big_stats(&reads, &stored), [None; 3]);
}

#[test]
fn a_sure_read_the_gate_refused_never_counts_for_the_big_stat_gate() {
    // A rejoin's zero board: sure zeros, while the stored row keeps the
    // pre-crash totals. A sure 35 where the gate kept 3500 is refused too.
    let zeros = tab_reads([
        stat_read(Some(0), false),
        stat_read(Some(0), false),
        stat_read(Some(35), false),
    ]);
    let stored = [12, 5, 3, 4200, 9500, 3500];
    assert_eq!(sure_accepted_big_stats(&zeros, &stored), [None; 3]);
    // A sure jump above what the gate kept is refused too: only equal counts.
    let jumps = tab_reads([
        stat_read(Some(9999), false),
        stat_read(Some(9600), false),
        stat_read(Some(3600), false),
    ]);
    assert_eq!(sure_accepted_big_stats(&jumps, &stored), [None; 3]);
}

/// A game open as `session_id` on Busan, for [`next_tab`].
fn open_game(session_id: &str, now: Instant) -> SessionState {
    let mut active = game(detect::MatchOutcome::Unknown, None, now);
    active.session_id = session_id.into();
    active.map = Some("Busan".into());
    active.map_source = Some(boundary::MapSource::TopBar);
    active.session_created = false;
    session(Some(active), now)
}

/// One [`sure_tab`] of `board` on the game in `st`, 20 s after the last,
/// the way the run loop takes it: the capture gate carries from Tab to
/// Tab. Returns whether it recorded.
async fn next_tab(
    ctx: &DaemonCtx,
    st: &mut SessionState,
    now: &mut Instant,
    board: shadow::BoardRead,
) -> bool {
    *now += Duration::from_secs(20);
    *INJECTED_CAPTURE.lock().unwrap() = Some(InjectedCapture {
        analysis: sure_tab(Some("ana")),
        board: Some(board),
    });
    let req = build_capture_request(st.active_game.as_ref().unwrap(), false, *now);
    let sid = req.session_id.clone();
    let report = handle_capture(ctx, req).await.unwrap();
    assert!(!report.split, "the Tab stays on the game");
    let recorded = report.recorded;
    apply_capture_report(st, &ctx.store, &ctx.data_dir, &sid, Ok(report), *now).await;
    recorded
}

/// The game's last stored damage, healing and mitigation.
async fn stored_big(ctx: &DaemonCtx, session_id: &str) -> [u32; 3] {
    let rows = ctx.store.get_session_snapshots(session_id).await.unwrap();
    let last = rows.last().expect("a stored row");
    [last.damage, last.healing, last.mitigation]
}

/// Let a blocking cut, had one started, land, then count the queued cells.
async fn settled_digits(dir: &Path) -> usize {
    tokio::time::sleep(Duration::from_millis(300)).await;
    queued_digits(dir)
}

/// An earlier Tab stored damage 3500 but saved no set. Later sure Tabs
/// with that cell held by the gate (a suspect read of it, or none) store
/// 3500 again; that is not this Tab's read, so no set is saved and the
/// game is not marked offered. The first Tab with a sure accepted read
/// past 1000 then saves the set, once.
#[tokio::test]
async fn a_held_big_stat_saves_no_digit_set_until_a_sure_accepted_read() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let mut now = test_now();
    let mut st = open_game("dt-held", now);
    // The first big Tab saves no set (stat_digits consent still off).
    set_consent(dir.path(), false, false);
    let first = o10_board([sure(12), sure(5), sure(3), sure(3500), sure(700), sure(300)]);
    assert!(next_tab(&ctx, &mut st, &mut now, first).await);
    assert_eq!(stored_big(&ctx, "dt-held").await, [3500, 700, 300]);
    set_consent(dir.path(), false, true);
    let held = [Some((3500, true)), Some((35, true)), None];
    for damage in held {
        let board = o10_board([sure(12), sure(5), sure(3), damage, sure(700), sure(300)]);
        assert!(next_tab(&ctx, &mut st, &mut now, board).await, "{damage:?}");
        assert_eq!(stored_big(&ctx, "dt-held").await[0], 3500, "{damage:?}");
        assert_eq!(settled_digits(dir.path()).await, 0, "held {damage:?}");
    }
    // Not marked offered: a sure accepted read past 1000 saves the set.
    let sure_big = o10_board([sure(13), sure(5), sure(3), sure(3600), sure(700), sure(300)]);
    assert!(next_tab(&ctx, &mut st, &mut now, sure_big).await);
    assert_eq!(stored_big(&ctx, "dt-held").await[0], 3600);
    let six = || queued_digits(dir.path()) >= 6;
    assert!(wait_for(six).await, "the sure accepted Tab saves the set");
    // Once: a further sure Tab past 1000 saves nothing more.
    let again = o10_board([sure(14), sure(5), sure(3), sure(3700), sure(700), sure(300)]);
    assert!(next_tab(&ctx, &mut st, &mut now, again).await);
    assert_eq!(settled_digits(dir.path()).await, 6);
}

/// Crash and rejoin (2026-10-15 Rialto): the tracker restarts, so nothing
/// is offered in memory, and the first Tab after the rejoin is a sure
/// board of zeros while the game keeps its pre-crash totals over 1000.
/// Those totals are not on screen: no set is saved from that Tab.
#[tokio::test]
async fn a_rejoin_zero_board_with_held_pre_crash_totals_saves_no_digit_set() {
    let _serial = INJECTED_CAPTURE_SERIAL.lock().await;
    let dir = tempfile::tempdir().unwrap();
    let ctx = o8_ctx(dir.path(), true).await;
    let mut now = test_now();
    let mut st = open_game("dt-rejoin", now);
    set_consent(dir.path(), false, false);
    let before = o10_board([
        sure(12),
        sure(5),
        sure(3),
        sure(4200),
        sure(9500),
        sure(1500),
    ]);
    assert!(next_tab(&ctx, &mut st, &mut now, before).await);
    // The restart restores the unfinished game and opens the rejoin window.
    mark_restart_recovered(&mut st, now);
    assert!(
        st.active_game
            .as_ref()
            .unwrap()
            .restart_recovered_at
            .is_some()
    );
    set_consent(dir.path(), false, true);
    let zeros = o10_board([sure(0), sure(0), sure(0), sure(0), sure(0), sure(0)]);
    assert!(
        next_tab(&ctx, &mut st, &mut now, zeros).await,
        "the zero board records"
    );
    let carry = st.active_game.as_ref().unwrap().rejoin_carry;
    assert!(
        carry.is_some(),
        "the zero board continued the game as a rejoin"
    );
    assert_eq!(
        stored_big(&ctx, "dt-rejoin").await,
        [4200, 9500, 1500],
        "the stored totals never go down"
    );
    assert_eq!(
        settled_digits(dir.path()).await,
        0,
        "no set from the zero board"
    );
}
