//! The pinned Settings bar (#776): what it counts, its buttons, Save,
//! Discard, a refused save, the leave confirm, the pill's Stop prompt and
//! the bar's keys. Through the real messages on a live (not demo) app that
//! writes a config.toml in a temp dir. A verb's task is returned, not run,
//! so nothing touches the host's tracker.

use super::*;
use crate::settings::{SettingsField, SettingsToggle};
use crate::settings_update::{Msg as PageMsg, PageState, apply};
use stat_tracker::config::Config;

/// A live app on the 0.6 page, Settings open, nothing pending, the
/// Companion shortcut on and saved.
fn live_app() -> (tempfile::TempDir, TrackerApp) {
    let (dir, mut app) = crate::fixtures::test_app();
    app.fixture = None;
    app.nav_style = NavStyle::TopTabs;
    app.screen = Screen::Settings;
    let path = dir.path().join("config.toml");
    app.saved_config.save_at(&path).unwrap();
    app.saved_config = Config::read_at(&path).unwrap();
    app.saved_file = app.saved_config.clone();
    app.overlay_hotkey.enabled = true;
    app.settings = app.form_from_saved();
    app.page_state = PageState::new(app.saved_config.clone(), None, true);
    app.page_state.config_path = Some(path);
    app.daemon.pid = None;
    app.toast = None;
    (dir, app)
}

fn on_disk(dir: &tempfile::TempDir) -> Config {
    Config::read_at(&dir.path().join("config.toml")).unwrap()
}

fn bar(msg: Msg) -> Message {
    Message::SettingsPage(PageMsg::Bar(msg))
}

fn type_text(app: &mut TrackerApp, field: SettingsField, value: &str) {
    let _ = app.update(Message::SettingsText(field, value.into()));
}

fn pretend_running(app: &mut TrackerApp) {
    app.daemon.pid = Some(4242);
}

fn bar_words(app: &TrackerApp) -> Option<String> {
    shows(app).then(|| {
        let p = pending(app);
        words(p.drafts.len(), p.waiting)
    })
}

// ---- copy and buttons, pure ----

#[test]
fn the_words_keep_the_two_counts_apart() {
    assert_eq!(words(1, 0), "1 unsaved change");
    assert_eq!(words(3, 0), "3 unsaved changes");
    assert_eq!(words(0, 1), "1 change waiting");
    assert_eq!(words(0, 4), "4 changes waiting");
    assert_eq!(words(2, 1), "2 unsaved changes, 1 waiting for restart");
    assert_eq!(words(1, 3), "1 unsaved change, 3 waiting for restart");
}

#[test]
fn bar_copy_has_no_em_or_en_dash_and_no_ellipsis_character() {
    for copy in [
        DISCARD,
        SAVE,
        SAVE_AND_RESTART,
        SAVE_AND_START,
        RESTART_TRACKER,
        START_TRACKER,
        LEAVE_TITLE,
        STOP_TITLE,
        DISCARD_AND_LEAVE,
        DISCARD_AND_STOP,
        SAVE_AND_STOP,
        CANCEL,
    ] {
        for bad in ['\u{2014}', '\u{2013}', '\u{2026}'] {
            assert!(!copy.contains(bad), "{copy}");
        }
    }
}

fn p(drafts: &[Draft], waiting: usize, running: bool) -> Pending {
    Pending {
        drafts: drafts.to_vec(),
        waiting,
        running,
    }
}

#[test]
fn the_primary_button_follows_spec_2_3_and_2_4() {
    use Draft::*;
    // Running tracker.
    assert_eq!(p(&[Shortcut], 0, true).primary(), Some(Primary::Save));
    assert_eq!(
        p(&[Timings], 0, true).primary(),
        Some(Primary::SaveAndRestart)
    );
    assert_eq!(p(&[Sync], 0, true).primary(), Some(Primary::SaveAndRestart));
    assert_eq!(
        p(&[Shortcut], 2, true).primary(),
        Some(Primary::SaveAndRestart),
        "drafts plus waiting changes"
    );
    assert_eq!(p(&[], 2, true).primary(), Some(Primary::RestartTracker));
    assert_eq!(p(&[], 0, true).primary(), None);
    // Stopped tracker (D5, D6).
    assert_eq!(
        p(&[Timings], 0, false).primary(),
        Some(Primary::SaveAndStart)
    );
    assert_eq!(
        p(&[Shortcut], 1, false).primary(),
        Some(Primary::SaveAndStart)
    );
    assert_eq!(p(&[], 1, false).primary(), Some(Primary::StartTracker));
    assert_eq!(p(&[], 0, false).primary(), None);
}

#[test]
fn the_buttons_are_discard_then_save_then_the_primary() {
    use Draft::*;
    assert_eq!(
        p(&[Timings], 0, true).buttons(),
        [BarButton::Discard, BarButton::Primary]
    );
    assert_eq!(
        p(&[Timings], 0, false).buttons(),
        [BarButton::Discard, BarButton::Save, BarButton::Primary],
        "a stopped tracker gets a plain Save beside Save and start"
    );
    assert_eq!(p(&[], 3, true).buttons(), [BarButton::Primary]);
    assert!(p(&[], 0, true).buttons().is_empty());
}

// ---- when it shows and what it counts ----

#[test]
fn the_bar_shows_only_with_something_pending_on_settings() {
    let (_dir, mut app) = live_app();
    assert!(!shows(&app), "nothing pending");
    type_text(&mut app, SettingsField::Cooldown, "45");
    assert_eq!(bar_words(&app).as_deref(), Some("1 unsaved change"));
    // The second timing field is the same group.
    type_text(&mut app, SettingsField::PollInterval, "7");
    assert_eq!(bar_words(&app).as_deref(), Some("1 unsaved change"));
    type_text(&mut app, SettingsField::SyncUrl, "https://crew.example");
    type_text(&mut app, SettingsField::OverlayHotkey, "Super+Shift+K");
    assert_eq!(bar_words(&app).as_deref(), Some("3 unsaved changes"));
    // Typing it back is not a change.
    type_text(&mut app, SettingsField::SyncUrl, "");
    assert_eq!(bar_words(&app).as_deref(), Some("2 unsaved changes"));
    // Another tab: the typed text is kept, but the bar is Settings only.
    app.screen = Screen::Overview;
    assert!(!shows(&app));
    assert_eq!(pending(&app).drafts.len(), 2);
}

#[test]
fn a_switch_flip_never_counts_as_unsaved() {
    let (dir, mut app) = live_app();
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::DebugOcr,
        true,
    )));
    assert!(on_disk(&dir).debug_ocr, "the switch wrote at once (Q7 = A)");
    assert!(pending(&app).drafts.is_empty());
    assert_eq!(bar_words(&app).as_deref(), Some("1 change waiting"));
    // The shortcut box is instant too, and is not a draft.
    let _ = app.update(crate::settings_update::apply_with(
        Message::SettingsToggle(SettingsToggle::OverlayHotkey, false),
        Commit::Shortcut,
    ));
    assert!(pending(&app).drafts.is_empty());
    // Flipping it back is the only undo, and the count follows.
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::DebugOcr,
        false,
    )));
    assert!(!shows(&app));
    // A flip never opens the leave confirm.
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::DebugOcr,
        true,
    )));
    let _ = app.update(Message::Navigate(Screen::Games));
    assert_eq!(app.screen, Screen::Games);
    assert!(app.page_state.bar.leave.is_none());
}

#[test]
fn restart_needed_changes_show_with_the_tracker_verb() {
    let (_dir, mut app) = live_app();
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::DebugOcr,
        true,
    )));
    assert_eq!(pending(&app).primary(), Some(Primary::StartTracker));
    pretend_running(&mut app);
    assert_eq!(pending(&app).primary(), Some(Primary::RestartTracker));
    let _ = app.update(bar(Msg::Primary));
    assert!(app.daemon_busy, "Restart tracker ran the verb");
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
    let _ = app.update(Message::DaemonDone(Ok("Tracker restarted".into())));
    assert_eq!(app.changes_waiting(), 0);
    assert!(!shows(&app), "the bar hides once the tracker loaded it");
}

#[test]
fn demo_and_classic_have_no_bar_and_no_confirm() {
    let (_dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    app.fixture = Some(crate::cli::FixtureKind::Empty);
    assert!(!shows(&app));
    assert!(drafts(&app).is_empty());
    app.fixture = None;
    app.nav_style = NavStyle::ClassicSidebar;
    assert!(!shows(&app));
    let _ = app.update(Message::Navigate(Screen::Games));
    assert_eq!(app.screen, Screen::Games, "classic keeps its own Save");
}

// ---- Save and Discard ----

#[test]
fn save_writes_every_typed_group_in_one_go() {
    let (dir, mut app) = live_app();
    pretend_running(&mut app);
    type_text(&mut app, SettingsField::OverlayHotkey, "Super+Shift+K");
    assert_eq!(pending(&app).primary(), Some(Primary::Save));
    let _ = app.update(bar(Msg::Primary));
    assert_eq!(app.overlay_hotkey.bind, "Super+Shift+K");
    assert!(!shows(&app), "an app-only save needs no restart");
    assert!(!app.daemon_busy, "Save never restarts");
    assert!(app.toast.is_none(), "a save that works is quiet");
    // Timings and the website, saved together, then the restart.
    type_text(&mut app, SettingsField::Cooldown, "45");
    type_text(&mut app, SettingsField::SyncUrl, "https://crew.example");
    type_text(&mut app, SettingsField::SyncToken, "tok");
    assert_eq!(pending(&app).primary(), Some(Primary::SaveAndRestart));
    let _ = app.update(bar(Msg::Primary));
    let disk = on_disk(&dir);
    assert_eq!(disk.auto_detect.cooldown_secs, 45);
    assert_eq!(
        disk.sync.as_ref().map(|s| s.server_url.as_str()),
        Some("https://crew.example")
    );
    assert!(pending(&app).drafts.is_empty());
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
}

#[test]
fn a_stopped_tracker_can_save_without_starting() {
    let (dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    assert_eq!(pending(&app).primary(), Some(Primary::SaveAndStart));
    let _ = app.update(bar(Msg::Save));
    assert_eq!(on_disk(&dir).auto_detect.cooldown_secs, 45);
    assert!(!app.daemon_busy);
    assert_eq!(bar_words(&app).as_deref(), Some("1 change waiting"));
    assert_eq!(pending(&app).primary(), Some(Primary::StartTracker));
    // Save and start: write, then Start.
    type_text(&mut app, SettingsField::PollInterval, "9");
    let _ = app.update(bar(Msg::Primary));
    assert_eq!(on_disk(&dir).auto_detect.poll_interval_secs, Some(9));
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Start));
}

#[test]
fn discard_puts_typed_text_back_and_leaves_switches_alone() {
    let (dir, mut app) = live_app();
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::DebugOcr,
        true,
    )));
    let saved = app.settings.clone();
    type_text(&mut app, SettingsField::Cooldown, "45");
    type_text(&mut app, SettingsField::SyncUrl, "https://crew.example");
    type_text(&mut app, SettingsField::OverlayHotkey, "Super+Shift+K");
    let _ = app.update(bar(Msg::Discard));
    assert_eq!(app.settings, saved);
    assert!(app.settings.debug_ocr, "the switch stays");
    assert!(on_disk(&dir).debug_ocr);
    assert!(!app.daemon_busy, "Discard never restarts");
    assert_eq!(
        bar_words(&app).as_deref(),
        Some("1 change waiting"),
        "only the waiting change is left"
    );
}

// ---- a refused save (spec 2.6) ----

#[test]
fn a_refused_save_keeps_the_text_and_shows_on_the_bar() {
    let (dir, mut app) = live_app();
    pretend_running(&mut app);
    let before = on_disk(&dir);
    type_text(&mut app, SettingsField::PollInterval, "soon");
    let _ = app.update(bar(Msg::Primary));
    assert_eq!(on_disk(&dir), before, "nothing written");
    assert!(!app.daemon_busy, "no restart after a refusal");
    assert_eq!(app.settings.poll_interval_secs, "soon", "the text stays");
    assert!(shows(&app));
    let message = error(&app).expect("the refusal is on the bar");
    assert!(message.contains("Check every"), "{message}");
    assert!(app.toast.is_none(), "not also a toast: {:?}", app.toast);
    // Fixing the field clears the stale message; the retry is the same button.
    type_text(&mut app, SettingsField::PollInterval, "5");
    assert_eq!(error(&app), None);
    let _ = app.update(bar(Msg::Primary));
    assert_eq!(on_disk(&dir).auto_detect.poll_interval_secs, Some(5));
}

#[test]
fn an_unsafe_website_url_blocks_the_save() {
    let (dir, mut app) = live_app();
    type_text(&mut app, SettingsField::SyncUrl, "http://crew.example");
    type_text(&mut app, SettingsField::SyncToken, "tok");
    let _ = app.update(bar(Msg::Save));
    assert!(on_disk(&dir).sync.is_none());
    let want = crate::settings::sync_url_problem("http://crew.example").unwrap();
    assert_eq!(error(&app), Some(want));
}

/// Typed text in all three groups, the ones a refused save must keep.
fn type_all_groups(app: &mut TrackerApp) {
    type_text(app, SettingsField::Cooldown, "45");
    type_text(app, SettingsField::SyncUrl, "https://crew.example");
    type_text(app, SettingsField::SyncToken, "pasted-token");
    type_text(app, SettingsField::OverlayHotkey, "Super+Shift+K");
}

/// Every typed group is still typed, the refusal's message is on the bar
/// (not a toast), and no verb ran.
fn kept_on_the_bar(app: &TrackerApp, entry: &str, words: &str) {
    assert_eq!(app.settings.cooldown_secs, "45", "{entry}");
    assert_eq!(app.settings.sync_url, "https://crew.example", "{entry}");
    assert_eq!(app.settings.sync_token, "pasted-token", "{entry}");
    assert_eq!(app.settings.overlay_hotkey, "Super+Shift+K", "{entry}");
    assert_eq!(drafts(app).len(), 3, "{entry}");
    let message = error(app).unwrap_or_else(|| panic!("{entry}: no message on the bar"));
    assert!(message.contains(words), "{entry}: {message}");
    assert!(app.toast.is_none(), "{entry}: {:?}", app.toast);
    assert!(!app.daemon_busy, "{entry}: a verb ran");
    assert_eq!(app.pill.in_flight, None, "{entry}");
}

/// The ways typed text is saved (or saved first): the bar's Save (beside
/// Save and start, tracker stopped), its Save and restart, the leave
/// confirm's Save, and the pill's Restart, Start and Retry.
const SAVE_ENTRIES: [&str; 6] = [
    "bar Save",
    "bar Save and restart",
    "leave confirm Save",
    "pill Restart",
    "pill Start",
    "pill Retry",
];

fn save_through(app: &mut TrackerApp, entry: &str) {
    use crate::status_pill::{Failure, Item, Msg as PillMsg};
    let choose = |app: &mut TrackerApp, item| {
        let _ = app.update(Message::Pill(PillMsg::Choose(item)));
    };
    match entry {
        "bar Save" => {
            assert_eq!(pending(app).primary(), Some(Primary::SaveAndStart));
            let _ = app.update(bar(Msg::Save));
        }
        "bar Save and restart" => {
            pretend_running(app);
            assert_eq!(pending(app).primary(), Some(Primary::SaveAndRestart));
            let _ = app.update(bar(Msg::Primary));
        }
        "leave confirm Save" => {
            let _ = app.update(Message::Navigate(Screen::Games));
            let _ = app.update(bar(Msg::LeaveSave));
            assert_eq!(app.screen, Screen::Settings, "a refused save stays");
        }
        "pill Restart" => {
            pretend_running(app);
            choose(app, Item::Restart);
        }
        "pill Start" => choose(app, Item::Start),
        "pill Retry" => {
            pretend_running(app);
            app.pill.failure = Some(Failure {
                verb: DaemonVerb::Restart,
                reason: "failed".into(),
            });
            choose(app, Item::Retry);
        }
        other => panic!("{other}"),
    }
}

/// A file changed on disk refuses the save at every entry point and keeps
/// every typed group (the regression on 30b795f2 kept only the shortcut).
/// The retry then writes the text against the fresh file.
#[test]
fn a_file_changed_on_disk_keeps_the_typed_text_at_every_entry() {
    for entry in SAVE_ENTRIES {
        let (dir, mut app) = live_app();
        type_all_groups(&mut app);
        let mut outside = on_disk(&dir);
        outside.debug_ocr = !outside.debug_ocr;
        outside.save_at(&dir.path().join("config.toml")).unwrap();
        save_through(&mut app, entry);
        kept_on_the_bar(&app, entry, "changed on disk");
        assert_ne!(on_disk(&dir).auto_detect.cooldown_secs, 45, "{entry}");
        assert_eq!(app.settings.debug_ocr, outside.debug_ocr, "{entry}");
        let _ = app.update(bar(Msg::Save));
        let disk = on_disk(&dir);
        assert_eq!(disk.auto_detect.cooldown_secs, 45, "{entry}");
        assert_eq!(disk.debug_ocr, outside.debug_ocr, "{entry}");
        assert_eq!(
            disk.sync.as_ref().map(|s| s.token.as_str()),
            Some("pasted-token"),
            "{entry}"
        );
    }
}

/// A config.toml that no longer parses refuses the save the same way.
#[test]
fn an_unreadable_file_keeps_the_typed_text_at_every_entry() {
    for entry in SAVE_ENTRIES {
        let (dir, mut app) = live_app();
        type_all_groups(&mut app);
        std::fs::write(dir.path().join("config.toml"), "this is = = not toml").unwrap();
        save_through(&mut app, entry);
        kept_on_the_bar(&app, entry, crate::app::SETTINGS_SAVE_REFUSED);
    }
}

/// A check that refuses one group (here Check every) keeps the other
/// groups too, and writes nothing.
#[test]
fn a_validation_refusal_keeps_the_other_groups_at_every_entry() {
    for entry in SAVE_ENTRIES {
        let (dir, mut app) = live_app();
        type_all_groups(&mut app);
        type_text(&mut app, SettingsField::PollInterval, "soon");
        let before = on_disk(&dir);
        save_through(&mut app, entry);
        assert_eq!(app.settings.poll_interval_secs, "soon", "{entry}");
        kept_on_the_bar(&app, entry, "Check every");
        assert_eq!(on_disk(&dir), before, "{entry}");
    }
}

/// A write that fails (the folder is read-only) keeps the typed text.
/// Skipped when the test runs as a user the read-only folder cannot stop.
#[test]
fn a_failed_write_keeps_the_typed_text_at_every_entry() {
    use std::os::unix::fs::PermissionsExt;
    for entry in SAVE_ENTRIES {
        let (dir, mut app) = live_app();
        type_all_groups(&mut app);
        let folder = dir.path();
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o555)).unwrap();
        if std::fs::write(folder.join("probe"), b"x").is_ok() {
            std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();
            eprintln!("skipped: a read-only folder does not stop this user");
            return;
        }
        save_through(&mut app, entry);
        std::fs::set_permissions(folder, std::fs::Permissions::from_mode(0o755)).unwrap();
        kept_on_the_bar(&app, entry, "Could not save settings");
    }
}

/// A refusal with the same words as a toast already showing (Enter was
/// refused the same way) still moves to the bar.
#[test]
fn a_refusal_already_toasting_still_moves_to_the_bar() {
    let (_dir, mut app) = live_app();
    type_text(&mut app, SettingsField::PollInterval, "soon");
    let _ = app.update(Message::SettingsPage(PageMsg::Commit(Commit::Timings)));
    let toast = app.toast.clone().expect("Enter's refusal toasts");
    let _ = app.update(bar(Msg::Save));
    assert_eq!(error(&app), Some(toast.as_str()));
    assert!(app.toast.is_none(), "{:?}", app.toast);
    // A save that works leaves an unrelated toast alone.
    app.toast = Some("Report sent".into());
    type_text(&mut app, SettingsField::PollInterval, "5");
    let _ = app.update(bar(Msg::Save));
    assert_eq!(app.toast.as_deref(), Some("Report sent"));
    assert_eq!(error(&app), None);
}

/// With the website token set outside the app the website fields are
/// locked: they never count as typed, and a Save of the other groups
/// leaves the file's sync as it is.
#[test]
fn a_locked_website_never_counts_and_is_never_written() {
    let (dir, mut app) = live_app();
    app.outside_env = Some(crate::settings_env::OutsideEnv {
        overlay: stat_tracker::config::EnvOverlay {
            token: true,
            ..Default::default()
        },
        vars: Vec::new(),
    });
    type_text(&mut app, SettingsField::SyncUrl, "https://crew.example");
    assert!(drafts(&app).is_empty());
    type_text(&mut app, SettingsField::Cooldown, "45");
    assert_eq!(drafts(&app), [Draft::Timings]);
    let _ = app.update(bar(Msg::Save));
    let disk = on_disk(&dir);
    assert_eq!(disk.auto_detect.cooldown_secs, 45);
    assert!(disk.sync.is_none(), "the locked website is never written");
    assert!(drafts(&app).is_empty());
}

// ---- leaving (spec 2.5, D8) ----

#[test]
fn a_tab_change_with_typed_text_asks_first() {
    let (dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    let _ = app.update(Message::Navigate(Screen::Games));
    assert_eq!(app.screen, Screen::Settings, "held");
    let confirm = app.page_state.bar.leave.expect("the confirm is open");
    assert_eq!(confirm.after, Leave::Screen(Screen::Games));
    assert_eq!(confirm.focus, LeaveButton::Cancel, "Cancel is the default");
    // Escape is Cancel: stay, text kept.
    let _ = leave_key(&mut app, Key::Escape);
    assert!(app.page_state.bar.leave.is_none());
    assert_eq!(app.screen, Screen::Settings);
    assert_eq!(app.settings.cooldown_secs, "45");
    // Discard and leave.
    let _ = app.update(Message::Navigate(Screen::Games));
    let _ = app.update(bar(Msg::LeaveDiscard));
    assert_eq!(app.screen, Screen::Games);
    assert!(drafts(&app).is_empty());
    assert_ne!(on_disk(&dir).auto_detect.cooldown_secs, 45);
}

#[test]
fn save_in_the_leave_confirm_writes_then_leaves() {
    let (dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    let _ = app.update(Message::Navigate(Screen::Overview));
    let _ = app.update(bar(Msg::LeaveSave));
    assert_eq!(on_disk(&dir).auto_detect.cooldown_secs, 45);
    assert_eq!(app.screen, Screen::Overview);
    // Waiting changes alone never block leaving.
    let _ = app.update(Message::Navigate(Screen::Settings));
    assert_eq!(app.changes_waiting(), 1);
    let _ = app.update(Message::Navigate(Screen::Games));
    assert_eq!(app.screen, Screen::Games);
}

#[test]
fn a_refused_save_in_the_leave_confirm_stays() {
    let (_dir, mut app) = live_app();
    type_text(&mut app, SettingsField::PollInterval, "soon");
    let _ = app.update(Message::Navigate(Screen::Overview));
    let _ = app.update(bar(Msg::LeaveSave));
    assert_eq!(app.screen, Screen::Settings);
    assert!(app.page_state.bar.leave.is_none());
    assert!(error(&app).is_some());
}

#[test]
fn the_leave_confirm_says_save_and_restart_when_2_4_applies() {
    let (dir, mut app) = live_app();
    pretend_running(&mut app);
    type_text(&mut app, SettingsField::Cooldown, "45");
    let pending = pending(&app);
    assert_eq!(
        leave_words(&pending, Leave::Screen(Screen::Games)),
        (LEAVE_TITLE, DISCARD_AND_LEAVE, SAVE_AND_RESTART)
    );
    assert_eq!(
        leave_words(&pending, Leave::Quit),
        (LEAVE_TITLE, DISCARD_AND_LEAVE, SAVE),
        "a quit would drop the restart"
    );
    let _ = app.update(Message::Navigate(Screen::Games));
    let _ = app.update(bar(Msg::LeaveSave));
    assert_eq!(on_disk(&dir).auto_detect.cooldown_secs, 45);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
    assert_eq!(app.screen, Screen::Games);
}

#[test]
fn quit_close_uninstall_setup_and_app_restart_ask_too() {
    let (_dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    let window = iced::window::Id::unique();
    for (message, after) in [
        (Message::Tray(crate::tray::TrayAction::Quit), Leave::Quit),
        (
            Message::WindowCloseRequested(window),
            Leave::CloseWindow(window),
        ),
        (Message::OpenUninstall, Leave::Uninstall),
        (
            Message::Setup(crate::setup_guide::SetupMessage::Open),
            Leave::SetupGuide,
        ),
        (Message::RestartGui, Leave::RestartApp),
    ] {
        let _ = app.update(message);
        assert_eq!(app.page_state.bar.leave.map(|c| c.after), Some(after));
        assert!(app.uninstall_dialog.is_none());
        assert!(!app.setup.open);
        assert!(!app.daemon_busy);
        let _ = app.update(bar(Msg::LeaveCancel));
    }
    // Discard and leave carries on with what was held.
    let _ = app.update(Message::Setup(crate::setup_guide::SetupMessage::Open));
    let _ = app.update(bar(Msg::LeaveDiscard));
    assert!(app.setup.open);
}

#[test]
fn hide_window_and_read_only_dialogs_do_not_ask() {
    let (_dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    let _ = app.update(Message::Tray(crate::tray::TrayAction::Hide));
    let _ = app.update(Message::OpenAbout);
    assert!(app.page_state.bar.leave.is_none());
    assert_eq!(app.settings.cooldown_secs, "45", "the text waits");
}

// ---- the pill with typed text (spec 1.5, D7) ----

#[test]
fn stop_on_the_pill_asks_to_save_first() {
    let (dir, mut app) = live_app();
    pretend_running(&mut app);
    type_text(&mut app, SettingsField::Cooldown, "45");
    let _ = crate::status_pill::request(&mut app, DaemonVerb::Stop);
    assert!(!app.daemon_busy, "held");
    let confirm = app.page_state.bar.leave.expect("asked");
    assert_eq!(confirm.after, Leave::StopTracker);
    assert_eq!(
        leave_words(&pending(&app), Leave::StopTracker),
        (STOP_TITLE, DISCARD_AND_STOP, SAVE_AND_STOP)
    );
    let _ = app.update(bar(Msg::LeaveSave));
    assert_eq!(on_disk(&dir).auto_detect.cooldown_secs, 45);
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Stop));
}

#[test]
fn restart_on_the_pill_saves_first_and_a_refusal_holds_it() {
    let (dir, mut app) = live_app();
    pretend_running(&mut app);
    type_text(&mut app, SettingsField::PollInterval, "soon");
    let _ = crate::status_pill::request(&mut app, DaemonVerb::Restart);
    assert!(!app.daemon_busy, "a refused save holds the restart");
    assert!(error(&app).is_some());
    type_text(&mut app, SettingsField::PollInterval, "6");
    let _ = crate::status_pill::request(&mut app, DaemonVerb::Restart);
    assert_eq!(on_disk(&dir).auto_detect.poll_interval_secs, Some(6));
    assert_eq!(app.pill.in_flight, Some(DaemonVerb::Restart));
}

// ---- keys (spec 2.8, D3) ----

fn key(app: &mut TrackerApp, key: Key) {
    let _ = app.update(Message::Pill(crate::status_pill::Msg::Key(key)));
}

#[test]
fn tab_goes_pill_then_discard_then_the_primary() {
    let (_dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    pretend_running(&mut app);
    key(&mut app, Key::Tab);
    assert!(app.pill.focused);
    key(&mut app, Key::Tab);
    assert!(!app.pill.focused);
    assert_eq!(app.page_state.bar.focus, Some(BarButton::Discard));
    key(&mut app, Key::Tab);
    assert_eq!(app.page_state.bar.focus, Some(BarButton::Primary));
    // Escape lets go and never discards.
    key(&mut app, Key::Escape);
    assert_eq!(app.page_state.bar.focus, None);
    assert_eq!(app.settings.cooldown_secs, "45");
    // Space on Discard discards.
    key(&mut app, Key::Tab);
    key(&mut app, Key::Tab);
    key(&mut app, Key::Space);
    assert!(drafts(&app).is_empty());
}

#[test]
fn with_no_bar_tab_still_leaves_the_pill() {
    let (_dir, mut app) = live_app();
    key(&mut app, Key::Tab);
    assert!(app.pill.focused);
    key(&mut app, Key::Tab);
    assert!(!app.pill.focused);
    assert_eq!(app.page_state.bar.focus, None);
}

// ---- After restart tags only where a change waits (D9) ----

#[test]
fn the_after_restart_tag_follows_the_waiting_change() {
    use crate::settings_model::AdvancedSection;
    let (_dir, mut app) = live_app();
    for section in AdvancedSection::ALL {
        assert!(!section_waiting(&app, section), "{section:?}");
    }
    assert_eq!(restart_tag(&app, "debug_ocr"), &[] as &[Tag]);
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::DebugOcr,
        true,
    )));
    assert_eq!(restart_tag(&app, "debug_ocr"), &[Tag::AfterRestart]);
    assert!(section_waiting(&app, AdvancedSection::Troubleshooting));
    assert!(!section_waiting(&app, AdvancedSection::CaptureReading));
    // An app-only key never waits, so it never gets the tag.
    let _ = app.update(apply(Message::SettingsText(
        SettingsField::SessionWindow,
        "10800".into(),
    )));
    assert!(!key_waiting(&app, "session_window_secs"));
    // One key per counted change: the tags and the count agree.
    assert_eq!(
        waiting_keys_between(app.page_state.restart_baseline(), &app.saved_config).len(),
        app.changes_waiting()
    );
    // A restart that loads it clears the tag.
    app.page_state.restart_started(&app.saved_config);
    app.page_state.daemon_done(true);
    assert_eq!(restart_tag(&app, "debug_ocr"), &[] as &[Tag]);
}

// ---- outside the update path: the tick (B1 of the #921 QA) ----

/// A real tray click arrives on the tick ([`crate::tray::poll_app`]), not as
/// a message. A Quit with typed text must still ask, as through `update`.
#[test]
fn a_tray_quit_polled_on_the_tick_asks_first() {
    let (_dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    crate::tray::TEST_CLICK.with(|c| c.set(Some(crate::tray::TrayAction::Quit)));
    let _ = app.update(Message::Tick);
    assert_eq!(
        app.page_state.bar.leave.map(|c| c.after),
        Some(Leave::Quit),
        "the polled Quit went past the leave confirm"
    );
    assert_eq!(app.settings.cooldown_secs, "45");
    // Polled Hide never asks (D8): the text stays in memory.
    let _ = app.update(bar(Msg::LeaveCancel));
    crate::tray::TEST_CLICK.with(|c| c.set(Some(crate::tray::TrayAction::Hide)));
    let _ = app.update(Message::Tick);
    assert!(app.page_state.bar.leave.is_none());
    // With nothing typed, a polled Quit does not ask.
    let _ = app.update(bar(Msg::Discard));
    crate::tray::TEST_CLICK.with(|c| c.set(Some(crate::tray::TrayAction::Quit)));
    let _ = app.update(Message::Tick);
    assert!(app.page_state.bar.leave.is_none());
}

/// A tray Quit while an in-app update runs is deferred by the quit gate;
/// with typed text it asks first, before any deferral.
#[test]
fn a_tray_quit_during_an_update_asks_first() {
    let (_dir, mut app) = live_app();
    app.update_progress = crate::update::UpdateProgress::Running;
    type_text(&mut app, SettingsField::Cooldown, "45");
    crate::tray::TEST_CLICK.with(|c| c.set(Some(crate::tray::TrayAction::Quit)));
    let _ = app.update(Message::Tick);
    assert_eq!(app.page_state.bar.leave.map(|c| c.after), Some(Leave::Quit));
}

/// A newer tracker GUI asking this one to quit (it replaces this one, and
/// stops it by signal if it does not go) is not held by the confirm: the
/// one outside request that skips it, on purpose. Listed in the #921
/// report for FrozenTear.
#[test]
fn a_quit_request_from_a_newer_gui_does_not_ask() {
    let (dir, mut app) = live_app();
    type_text(&mut app, SettingsField::Cooldown, "45");
    crate::instance::request_quit(dir.path()).unwrap();
    let _ = app.update(Message::Tick);
    assert!(app.page_state.bar.leave.is_none());
    assert!(
        !crate::instance::consume_quit(dir.path()),
        "the tick took it"
    );
}

// ---- the primary button's focus ring (R3 of the #921 QA) ----

#[test]
fn every_bar_button_shows_its_focus_ring() {
    let theme = iced::Theme::Dark;
    for accent in [false, true] {
        let plain = button_style(accent, true, false)(&theme, iced::widget::button::Status::Active);
        let ringed = button_style(accent, true, true)(&theme, iced::widget::button::Status::Active);
        let fill = match ringed.background {
            Some(iced::Background::Color(c)) => Some(c),
            _ => None,
        };
        assert_eq!(ringed.border.width, 2.0, "accent {accent}");
        assert_ne!(ringed.border.color, plain.border.color, "accent {accent}");
        assert_ne!(
            Some(ringed.border.color),
            fill,
            "accent {accent}: the ring is the fill colour"
        );
    }
}

// ---- After restart on every row with a waiting change (R4 of the #921 QA) ----

/// The texts of `view`, laid out at 1280 px.
fn texts_of(view: Element<'_, Message>) -> Vec<String> {
    use iced::advanced::widget::{Id, Operation, Tree};
    struct Texts(Vec<String>);
    impl Operation<()> for Texts {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }
        fn text(&mut self, _id: Option<&Id>, _bounds: iced::Rectangle, text: &str) {
            self.0.push(text.to_string());
        }
    }
    let renderer = crate::chrome::headless();
    let mut view = view;
    let mut tree = Tree::new(&view);
    let limits =
        iced::advanced::layout::Limits::new(iced::Size::ZERO, iced::Size::new(1280.0, 4000.0));
    let node = view.as_widget_mut().layout(&mut tree, &renderer, &limits);
    let mut texts = Texts(Vec::new());
    view.as_widget_mut().operate(
        &mut tree,
        iced::advanced::Layout::new(&node),
        &renderer,
        &mut texts,
    );
    texts.0
}

/// The title just before each After restart tag in `texts`.
fn tagged_rows(texts: &[String]) -> Vec<String> {
    texts
        .windows(2)
        .filter(|w| w[1] == "After restart")
        .map(|w| w[0].clone())
        .collect()
}

#[test]
fn watch_timings_and_debug_images_rows_tag_while_their_change_waits() {
    let (_dir, mut app) = live_app();
    let service = |app: &TrackerApp| {
        tagged_rows(&texts_of(crate::settings_cards::tracker_service(
            app, false,
        )))
    };
    let trouble = |app: &TrackerApp| {
        tagged_rows(&texts_of(crate::settings_cards::troubleshooting(
            app, false,
        )))
    };
    assert!(service(&app).is_empty(), "{:?}", service(&app));
    assert!(trouble(&app).is_empty(), "{:?}", trouble(&app));
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::DebugOcr,
        true,
    )));
    assert_eq!(trouble(&app), ["Save debug images from each capture"]);
    type_text(&mut app, SettingsField::Cooldown, "45");
    let _ = app.update(bar(Msg::Save));
    assert_eq!(service(&app), ["Wait after detection (seconds)"]);
    let on = app.saved_config.auto_detect.enabled;
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::AutoDetect,
        !on,
    )));
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::AutoDetect,
        on,
    )));
    assert_eq!(
        service(&app),
        ["Wait after detection (seconds)"],
        "flipped back"
    );
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::AutoDetect,
        !on,
    )));
    assert!(
        service(&app).contains(&"Watch for match start and end".to_string()),
        "{:?}",
        service(&app)
    );
}

// ---- B2 and R5 of the #921 QA (FrozenTear, 27 Nov) ----

/// Opens one read-only dialog over Settings.
fn open_dialog(app: &mut TrackerApp, which: &str) {
    match which {
        "What leaves your PC" => app.page_state.opened.what_leaves_open = true,
        "Report a problem" => {
            app.report = Some(crate::report::ReportUi::CrashPrompt { last_seen: None });
        }
        "About" => {
            let _ = app.update(Message::OpenAbout);
        }
        other => panic!("{other}"),
    }
    assert!(crate::chrome::top_dialog(app).is_some(), "{which} is open");
}

/// A close with no tray, or a tray Quit, with typed text and a read-only
/// dialog open: the dialog closes and the leave confirm is in front.
#[test]
fn quit_and_close_close_a_read_only_dialog_before_asking() {
    let window = iced::window::Id::unique();
    for which in ["What leaves your PC", "Report a problem", "About"] {
        for (message, after) in [
            (
                Message::WindowCloseRequested(window),
                Leave::CloseWindow(window),
            ),
            (Message::Tray(crate::tray::TrayAction::Quit), Leave::Quit),
        ] {
            let (_dir, mut app) = live_app();
            type_text(&mut app, SettingsField::Cooldown, "45");
            open_dialog(&mut app, which);
            let _ = app.update(message);
            assert_eq!(
                crate::chrome::top_dialog(&app),
                None,
                "{which}: still open over the confirm"
            );
            assert_eq!(app.page_state.bar.leave.map(|c| c.after), Some(after));
            assert_eq!(app.settings.cooldown_secs, "45");
        }
    }
}

/// With nothing typed, a close or Quit leaves an open dialog alone (the
/// app just closes or quits, as before).
#[test]
fn with_nothing_typed_a_close_leaves_the_dialog_alone() {
    let (_dir, mut app) = live_app();
    open_dialog(&mut app, "About");
    let _ = app.update(Message::WindowCloseRequested(iced::window::Id::unique()));
    assert!(crate::chrome::top_dialog(&app).is_some());
    assert!(app.page_state.bar.leave.is_none());
}

/// R5: turning Watch for match start and end off hides the timings, so a
/// typed timing is dropped: out of the count, never written, and the
/// field shows the saved value when it comes back.
#[test]
fn a_typed_timing_is_dropped_when_watch_hides_it() {
    let (dir, mut app) = live_app();
    pretend_running(&mut app);
    let saved = app.settings.cooldown_secs.clone();
    let on = app.saved_config.auto_detect.enabled;
    assert!(on, "the fixture starts with Watch on");
    type_text(&mut app, SettingsField::Cooldown, "90");
    assert_eq!(bar_words(&app).as_deref(), Some("1 unsaved change"));
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::AutoDetect,
        false,
    )));
    assert!(drafts(&app).is_empty(), "{:?}", drafts(&app));
    assert_eq!(bar_words(&app).as_deref(), Some("1 change waiting"));
    assert_eq!(pending(&app).primary(), Some(Primary::RestartTracker));
    let _ = app.update(bar(Msg::Primary));
    assert_ne!(on_disk(&dir).auto_detect.cooldown_secs, 90);
    let _ = app.update(apply(Message::SettingsToggle(
        SettingsToggle::AutoDetect,
        true,
    )));
    assert_eq!(app.settings.cooldown_secs, saved, "the saved value shows");
    assert_ne!(on_disk(&dir).auto_detect.cooldown_secs, 90);
}
