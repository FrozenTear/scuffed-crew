//! The status pill's words, menu, keys, confirm and error state.
//! The verbs' tasks are returned, not run, so nothing touches the host's
//! tracker.

use super::*;
use crate::nav::bar_status;
use crate::nav_fit::{ERROR_GAP, layout, one_line_width, pill_chevron, status_gap};
use crate::settings_model::AdvancedSection;

/// A live (not demo) app on the 0.6 layout, with the tracker down.
fn live() -> (tempfile::TempDir, TrackerApp) {
    let (dir, mut app) = crate::fixtures::test_app();
    app.fixture = None;
    app.nav_style = NavStyle::TopTabs;
    app.daemon.pid = None;
    app.game_running = false;
    app.live_game_session = None;
    app.toast = None;
    (dir, app)
}

/// [`live`] with the tracker up.
fn running() -> (tempfile::TempDir, TrackerApp) {
    let (dir, mut app) = live();
    app.daemon.pid = Some(4242);
    (dir, app)
}

fn send(app: &mut TrackerApp, msg: Msg) {
    let _ = app.update(Message::Pill(msg));
}

fn key(app: &mut TrackerApp, k: Key) {
    send(app, Msg::Key(k));
}

fn status(app: &TrackerApp) -> HeaderStatus {
    tracker_status::header_status_for(app)
}

/// The words, time and chevron the bar draws at `width`.
fn drawn(status: HeaderStatus, width: f32) -> (String, Option<String>, bool) {
    let fit = layout(width, status, Some("00:40"), 0);
    let (label, time) = bar_status(status, Some("00:40"), fit.status);
    (label, time, pill_chevron(status, fit.status, width))
}

#[test]
fn the_pill_words_follow_the_spec_at_720_1000_1280_and_1440() {
    use HeaderStatus::*;
    for width in [720.0, 1000.0] {
        assert_eq!(drawn(Waiting, width), ("Ready".into(), None, false));
        assert_eq!(drawn(Down, width), ("Stopped".into(), None, false));
        assert_eq!(drawn(Capturing, width), ("In game".into(), None, false));
        assert_eq!(
            drawn(StartFailed, width),
            ("Couldn't start".into(), None, false)
        );
        assert_eq!(drawn(TokenRejected, width).0, "Token rejected");
        assert_eq!(drawn(FocusUnknown, width), ("?".into(), None, false));
    }
    for width in [1280.0, 1440.0] {
        assert_eq!(
            drawn(Waiting, width),
            (
                "Tracker ready · last game".into(),
                Some("00:40".into()),
                true
            )
        );
        assert_eq!(drawn(Down, width).0, "Tracker stopped · last game");
        assert_eq!(
            drawn(StartFailed, width),
            ("Couldn't start the tracker".into(), None, true)
        );
        assert_eq!(
            drawn(TokenRejected, width).0,
            "Website token rejected · last game"
        );
        let unreadable = drawn(FocusUnknown, width);
        assert_eq!(unreadable.0, "Can't tell which window is in front");
        assert!(unreadable.2, "the unreadable pill's chevron opens the menu");
    }
    assert_eq!(drawn(Capturing, 1280.0).0, "Watching Overwatch ·");
    assert_eq!(drawn(Capturing, 1440.0).0, "Watching Overwatch · last game");
    assert_eq!(
        drawn(CapturingAnyWindow, 1440.0).0,
        "Capturing in any window · last game"
    );
    assert_eq!(
        drawn(Paused, 1280.0).0,
        "Paused while Overwatch isn't in front"
    );
    // A verb in flight says the same at every width, with no time and no
    // chevron.
    for (busy, words) in [
        (Starting, "Starting..."),
        (Restarting, "Restarting..."),
        (Stopping, "Stopping..."),
    ] {
        for width in [720.0, 1000.0, 1280.0, 1440.0] {
            assert_eq!(drawn(busy, width), (words.into(), None, false), "{width}");
        }
    }
}

/// Spec 1.3: at 1000 px the Error pill has no chevron and leaves at least
/// 16 px to the Settings tab.
#[test]
fn the_error_pill_at_1000_px_keeps_its_gap_to_the_settings_tab() {
    for count in [0, 1, 159] {
        let fit = layout(1000.0, HeaderStatus::StartFailed, None, count);
        assert!(fit.one_line, "{count}");
        assert!(!pill_chevron(HeaderStatus::StartFailed, fit.status, 1000.0));
        let need = one_line_width(1000.0, fit, HeaderStatus::StartFailed, None, count);
        let gap = 1000.0 - need + status_gap(HeaderStatus::StartFailed);
        assert!(gap >= ERROR_GAP, "{count}: {gap}");
    }
    assert_eq!(status_gap(HeaderStatus::StartFailed), ERROR_GAP);
}

#[test]
fn the_menu_lists_what_the_tracker_can_do() {
    let (_dir, mut app) = live();
    assert_eq!(items(&app), [Item::Start]);
    app.daemon.pid = Some(4242);
    assert_eq!(items(&app), [Item::Restart, Item::Stop]);
    app.pill.failure = Some(Failure {
        verb: DaemonVerb::Restart,
        reason: "systemctl failed".into(),
    });
    assert_eq!(items(&app), [Item::Troubleshooting, Item::Retry]);
    // The demo, the classic layout and any busy mark have no menu.
    app.pill.failure = None;
    app.fixture = Some(crate::cli::FixtureKind::Empty);
    assert!(items(&app).is_empty());
    app.fixture = None;
    app.nav_style = NavStyle::ClassicSidebar;
    assert!(items(&app).is_empty());
    app.nav_style = NavStyle::TopTabs;
    app.daemon_busy = true;
    assert!(items(&app).is_empty());
}

#[test]
fn a_click_opens_and_closes_the_menu() {
    let (_dir, mut app) = running();
    assert!(menu_layer(&app).is_none());
    send(&mut app, Msg::Toggle);
    assert_eq!(app.pill.menu, Some(0));
    assert!(menu_layer(&app).is_some());
    send(&mut app, Msg::Toggle);
    assert_eq!(app.pill.menu, None);
    send(&mut app, Msg::Toggle);
    send(&mut app, Msg::Close);
    assert_eq!(app.pill.menu, None, "a click outside closes it");
    // Choosing an item closes it too.
    send(&mut app, Msg::Toggle);
    send(&mut app, Msg::Choose(Item::Restart));
    assert_eq!(app.pill.menu, None);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
}

#[test]
fn the_keys_open_move_choose_and_close_the_menu() {
    let (_dir, mut app) = running();
    // Nothing has the keys until Tab lands on the pill.
    key(&mut app, Key::Enter);
    assert!(!app.pill.focused && app.pill.menu.is_none());
    key(&mut app, Key::Tab);
    assert!(app.pill.focused && owns_keys(&app));
    key(&mut app, Key::Space);
    assert_eq!(app.pill.menu, Some(0));
    key(&mut app, Key::Down);
    assert_eq!(app.pill.menu, Some(1));
    key(&mut app, Key::Down);
    assert_eq!(app.pill.menu, Some(0), "the arrows wrap");
    key(&mut app, Key::Up);
    assert_eq!(app.pill.menu, Some(1));
    key(&mut app, Key::Escape);
    assert_eq!(app.pill.menu, None);
    assert!(
        app.pill.focused,
        "Escape closes the menu, the pill keeps focus"
    );
    key(&mut app, Key::Enter);
    key(&mut app, Key::Down);
    key(&mut app, Key::Enter);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Stop));
    assert_eq!(app.pill.menu, None);
    // Tab leaves the pill.
    key(&mut app, Key::Tab);
    assert!(!app.pill.focused && !owns_keys(&app));
}

#[test]
fn the_key_listener_reads_plain_keys_only() {
    use iced::keyboard::key::Named;
    use iced::keyboard::{Key as K, Modifiers};
    let plain = Modifiers::empty();
    assert_eq!(pill_key(&K::Named(Named::Tab), plain), Some(Key::Tab));
    assert_eq!(
        pill_key(&K::Named(Named::Tab), Modifiers::SHIFT),
        Some(Key::Tab)
    );
    assert_eq!(pill_key(&K::Named(Named::Enter), plain), Some(Key::Enter));
    assert_eq!(pill_key(&K::Named(Named::Space), plain), Some(Key::Space));
    assert_eq!(pill_key(&K::Named(Named::ArrowUp), plain), Some(Key::Up));
    assert_eq!(
        pill_key(&K::Named(Named::ArrowDown), plain),
        Some(Key::Down)
    );
    assert_eq!(pill_key(&K::Named(Named::Escape), plain), Some(Key::Escape));
    assert_eq!(pill_key(&K::Named(Named::Tab), Modifiers::CTRL), None);
    assert_eq!(pill_key(&K::Character("a".into()), plain), None);
    let tab = K::Named(Named::Tab);
    assert!(key_message(&tab, plain, iced::event::Status::Ignored).is_some());
    assert!(
        key_message(&tab, plain, iced::event::Status::Captured).is_none(),
        "a field that took the key keeps it"
    );
}

#[test]
fn the_unreadable_pill_keeps_opening_settings_and_down_opens_the_menu() {
    let (_dir, mut app) = running();
    app.game_running = true;
    let unreadable = Some(stat_tracker::ocr_status::FocusGateStatus::Unreadable);
    let now = std::time::Instant::now();
    app.page_state.focus.observe(unreadable, now);
    let later = now + crate::focus_status::UNREADABLE_AFTER;
    app.page_state.focus.observe(unreadable, later);
    assert_eq!(status(&app), HeaderStatus::FocusUnknown);
    key(&mut app, Key::Tab);
    key(&mut app, Key::Enter);
    assert_eq!(app.screen, crate::model::Screen::Settings);
    assert_eq!(app.pill.menu, None);
    key(&mut app, Key::Down);
    assert_eq!(app.pill.menu, Some(0));
    assert_eq!(items(&app), [Item::Restart, Item::Stop]);
}

#[test]
fn stop_and_restart_ask_first_only_mid_game() {
    let (dir, mut app) = running();
    // Overwatch open on a menu is not a match in progress.
    app.game_running = true;
    assert!(!match_in_progress(&app));
    send(&mut app, Msg::Choose(Item::Stop));
    assert_eq!(app.pill.confirm, None);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Stop));

    // A live game on the Games tab asks first; Keep tracking is the default.
    let (_dir2, mut app) = running();
    app.live_game_session = Some("live".into());
    send(&mut app, Msg::Choose(Item::Stop));
    let asked = app.pill.confirm.expect("asks first");
    assert_eq!(asked.verb, DaemonVerb::Stop);
    assert!(!asked.on_action, "Keep tracking has the keyboard");
    assert!(confirm_layer(&app).is_some());
    assert!(owns_keys(&app));
    key(&mut app, Key::Enter);
    assert_eq!(app.pill.confirm, None);
    assert_eq!(app.pill.in_flight, None, "Enter keeps tracking");
    send(&mut app, Msg::Choose(Item::Restart));
    key(&mut app, Key::Escape);
    assert_eq!((app.pill.confirm, app.pill.in_flight), (None, None));
    send(&mut app, Msg::Choose(Item::Restart));
    send(&mut app, Msg::Keep);
    assert_eq!((app.pill.confirm, app.pill.in_flight), (None, None));
    send(&mut app, Msg::Choose(Item::Restart));
    key(&mut app, Key::Tab);
    assert!(app.pill.confirm.unwrap().on_action);
    key(&mut app, Key::Enter);
    assert_eq!(app.pill.confirm, None);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
    drop(dir);

    // The open game file asks too.
    let (dir, mut app) = running();
    std::fs::write(dir.path().join("active_game.json"), "{}").unwrap();
    assert!(match_in_progress(&app));
    send(&mut app, Msg::Choose(Item::Restart));
    assert_eq!(app.pill.confirm.map(|c| c.verb), Some(DaemonVerb::Restart));
    send(&mut app, Msg::Go);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
}

#[test]
fn start_retry_and_the_demo_never_ask() {
    let (_dir, mut app) = live();
    app.live_game_session = Some("live".into());
    assert!(!asks_first(&app, DaemonVerb::Start));
    send(&mut app, Msg::Choose(Item::Start));
    assert_eq!(app.pill.confirm, None);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Start));

    let (_dir, mut app) = running();
    app.live_game_session = Some("live".into());
    app.pill.failure = Some(Failure {
        verb: DaemonVerb::Restart,
        reason: "failed".into(),
    });
    send(&mut app, Msg::Choose(Item::Retry));
    assert_eq!(app.pill.confirm, None);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));

    let (_dir, mut demo) = crate::fixtures::test_app();
    demo.live_game_session = Some("live".into());
    assert!(!asks_first(&demo, DaemonVerb::Stop));
    send(&mut demo, Msg::Toggle);
    send(&mut demo, Msg::Choose(Item::Stop));
    assert_eq!(demo.pill.menu, None);
    assert_eq!(demo.pill.confirm, None);
    assert_eq!(demo.pill.in_flight, None);
    assert_eq!(status(&demo), HeaderStatus::Waiting, "the demo shows Ready");
}

#[test]
fn a_failed_start_keeps_the_error_until_the_next_verb() {
    let (_dir, mut app) = live();
    send(&mut app, Msg::Choose(Item::Start));
    assert_eq!(status(&app), HeaderStatus::Starting);
    let _ = app.update(Message::DaemonDone(Err("tracker exited at once".into())));
    assert_eq!(app.pill.in_flight, None);
    assert!(!app.daemon_busy);
    assert_eq!(status(&app), HeaderStatus::StartFailed);
    assert_eq!(app.toast.as_deref(), Some("tracker exited at once"));
    // The error outlives the toast.
    app.toast = None;
    assert_eq!(status(&app), HeaderStatus::StartFailed);
    assert_eq!(
        app.pill.failure,
        Some(Failure {
            verb: DaemonVerb::Start,
            reason: "tracker exited at once".into(),
        })
    );
    // The menu: the reason, Troubleshooting, then Retry.
    send(&mut app, Msg::Toggle);
    assert!(menu_layer(&app).is_some());
    assert_eq!(items(&app), [Item::Troubleshooting, Item::Retry]);
    let tip = crate::nav::pill_tip(&app, HeaderStatus::StartFailed, StatusForm::Long, None);
    assert!(tip.iter().any(|(line, _)| line == "tracker exited at once"));
    assert!(tip.iter().any(|(line, _)| line == MENU_HINT));
    send(&mut app, Msg::Choose(Item::Troubleshooting));
    assert_eq!(app.screen, crate::model::Screen::Settings);
    assert!(app.page_state.is_open(AdvancedSection::Troubleshooting));
    assert_eq!(status(&app), HeaderStatus::StartFailed);
    // Retry with the tracker down is Start, and clears the error.
    send(&mut app, Msg::Choose(Item::Retry));
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Start));
    assert_eq!(app.pill.failure, None);
    let _ = app.update(Message::DaemonDone(Ok("Tracker started".into())));
    assert_eq!(app.pill.failure, None);
    assert_eq!(app.toast.as_deref(), Some("Tracker started"));
}

#[test]
fn a_failed_restart_retries_restart_while_the_tracker_runs() {
    let (_dir, mut app) = running();
    app.pill.failure = Some(Failure {
        verb: DaemonVerb::Restart,
        reason: "unit failed".into(),
    });
    assert_eq!(retry_verb(&app), DaemonVerb::Restart);
    app.daemon.pid = None;
    assert_eq!(retry_verb(&app), DaemonVerb::Start);
    // A failed Stop toasts but is not Couldn't start.
    let (_dir, mut app) = running();
    send(&mut app, Msg::Choose(Item::Stop));
    let _ = app.update(Message::DaemonDone(Err("stop timed out".into())));
    assert_eq!(app.pill.failure, None);
    assert_eq!(app.toast.as_deref(), Some("stop timed out"));
}

#[test]
fn while_a_verb_runs_there_is_no_menu_and_clicks_are_ignored() {
    for (verb, words) in [
        (DaemonVerb::Start, "Starting..."),
        (DaemonVerb::Restart, "Restarting..."),
        (DaemonVerb::Stop, "Stopping..."),
    ] {
        let (_dir, mut app) = if verb == DaemonVerb::Start {
            live()
        } else {
            running()
        };
        let _ = app.update(Message::Daemon(verb));
        assert_eq!(app.pill.in_flight, Some(verb));
        assert_eq!(tracker_status::short_label(status(&app)), words);
        assert_eq!(tracker_status::long_label(status(&app)), words);
        assert_eq!(
            tracker_status::header_status_color(status(&app)),
            theme::WARN
        );
        assert!(items(&app).is_empty());
        send(&mut app, Msg::Toggle);
        assert_eq!(app.pill.menu, None);
        key(&mut app, Key::Tab);
        key(&mut app, Key::Enter);
        key(&mut app, Key::Down);
        assert_eq!(app.pill.menu, None);
        // A second verb is ignored; the first stays named.
        let _ = app.update(Message::Daemon(DaemonVerb::Restart));
        send(&mut app, Msg::Choose(Item::Restart));
        assert_eq!(app.pill.in_flight, Some(verb));
        let tip = crate::nav::pill_tip(&app, status(&app), StatusForm::Long, Some("00:40"));
        assert_eq!(tip, vec![(words.to_string(), TEXT)], "no menu line");
    }
}

/// Start on login keeps its own busy mark: the pill names no verb, no
/// verb starts meanwhile, and its answer is no pill error.
#[test]
fn start_on_login_keeps_its_own_busy_mark() {
    let (_dir, mut app) = running();
    app.daemon.service_installed = true;
    let _ = app.update(Message::ToggleAutostart);
    assert!(app.daemon_busy);
    assert_eq!(app.pill.in_flight, None);
    assert_eq!(status(&app), HeaderStatus::Waiting);
    assert!(items(&app).is_empty());
    let _ = app.update(Message::Daemon(DaemonVerb::Stop));
    assert_eq!(app.pill.in_flight, None, "no verb starts meanwhile");
    let _ = app.update(Message::DaemonDone(Err("systemctl failed".into())));
    assert!(!app.daemon_busy);
    assert_eq!(app.pill.failure, None);
    // And no Start on login while a verb runs.
    let _ = app.update(Message::Daemon(DaemonVerb::Restart));
    let _ = app.update(Message::ToggleAutostart);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
}

#[test]
fn restart_is_the_primary_item_while_changes_wait() {
    assert_eq!(applies_label(1), "Applies 1 change");
    assert_eq!(applies_label(3), "Applies 3 changes");
    let (_dir, app) = running();
    // Builds with and without waiting changes, open on either item.
    for at in [0, 1] {
        let _ = menu_card(&app, at);
    }
    let _ = menu_item(Item::Restart, true, 2);
    let _ = menu_item(Item::Restart, false, 0);
}

#[test]
fn the_confirm_copy_follows_spec_1_4() {
    assert_eq!(
        confirm_words(DaemonVerb::Stop),
        ("Stop the tracker?", "Stop tracker")
    );
    assert_eq!(
        confirm_words(DaemonVerb::Restart),
        ("Restart the tracker?", "Restart tracker")
    );
    assert_eq!(CONFIRM_BODY, "The current game won't be recorded.");
    assert_eq!(KEEP_TRACKING, "Keep tracking");
}

#[test]
fn the_pill_builds_in_every_state_and_form() {
    let (_dir, mut app) = running();
    for status in tracker_status::ALL {
        for form in [StatusForm::Long, StatusForm::Medium, StatusForm::Short] {
            for chevron in [false, true] {
                for (menu, focused) in [(None, false), (Some(0), false), (None, true)] {
                    app.pill.menu = menu;
                    app.pill.focused = focused;
                    let _ = pill(&app, status, form, "Ready".into(), None, chevron);
                }
            }
        }
    }
}

#[test]
fn the_tracker_service_row_points_to_the_pill() {
    let words = crate::settings_cards::TRACKER_SERVICE_POINTER;
    assert!(words.contains("status at the top"));
    let (_dir, app) = running();
    let _ = crate::settings_cards::tracker_service(&app, false);
}
