//! Tests for the app shell (`app`).

use super::{TrayWindowOp, tray_hide_op, tray_show_op};
use iced::window;
use stat_tracker::config::Config;
use tracing_subscriber::layer::SubscriberExt;

#[test]
fn setup_guide_keys_do_not_dismiss_notes_underneath() {
    use iced::keyboard::Key;
    use iced::keyboard::key::Named;

    let escape = Key::Named(Named::Escape);
    let enter = Key::Named(Named::Enter);
    match super::foreground_key_message(true, true, &escape) {
        Some(super::Message::Setup(crate::setup_guide::SetupMessage::Skip)) => {}
        other => panic!("escape should skip the guide, got {other:?}"),
    }
    match super::guide_shortcut(iced::event::Status::Captured, true, true, &escape) {
        None => {}
        other => panic!("a focused text input keeps escape, got {other:?}"),
    }
    match super::guide_shortcut(iced::event::Status::Captured, true, true, &enter) {
        None => {}
        other => panic!("a focused text input keeps enter, got {other:?}"),
    }
    match super::guide_shortcut(iced::event::Status::Ignored, true, true, &enter) {
        Some(super::Message::Setup(crate::setup_guide::SetupMessage::Next)) => {}
        other => panic!("enter should continue the guide, got {other:?}"),
    }
    match super::foreground_key_message(true, true, &enter) {
        Some(super::Message::Setup(crate::setup_guide::SetupMessage::Next)) => {}
        other => panic!("enter should continue the guide, got {other:?}"),
    }
    match super::foreground_key_message(false, true, &escape) {
        Some(super::Message::DismissNotes) => {}
        other => panic!("escape should close notes when the guide is closed, got {other:?}"),
    }
    assert!(super::foreground_key_message(false, true, &enter).is_none());
}

#[test]
fn the_refused_toast_has_no_em_or_en_dash() {
    let copy = super::SETTINGS_SAVE_REFUSED;
    assert!(!copy.contains('\u{2014}'), "{copy}");
    assert!(!copy.contains('\u{2013}'), "{copy}");
    assert_eq!(
        super::toast_if_config_unreadable(true),
        Some(super::SETTINGS_SAVE_REFUSED)
    );
    assert!(super::SETTINGS_SAVE_REFUSED.contains("not saved"));
    assert!(super::toast_if_config_unreadable(false).is_none());
}

#[test]
fn refused_toast_clears_on_dismiss_or_timeout() {
    let refused = super::SETTINGS_SAVE_REFUSED;
    assert!(super::toast_after_dismiss().is_none());
    let shown = std::time::SystemTime::UNIX_EPOCH;
    let almost = shown + super::REFUSED_TOAST_TIMEOUT - std::time::Duration::from_secs(1);
    let due = shown + super::REFUSED_TOAST_TIMEOUT;
    assert!(!super::refused_toast_expired(
        Some(refused),
        Some(shown),
        almost
    ));
    assert!(super::refused_toast_expired(
        Some(refused),
        Some(shown),
        due
    ));
    assert_ne!(
        crate::settings_save::toast_after_successful_save(true),
        refused
    );
}

#[test]
fn a_later_toast_drops_the_training_action() {
    let mut toast = Some("Outcome set to Victory".into());
    let mut action = Some(super::Message::DismissToast);
    super::show_plain_toast(&mut toast, &mut action, "The overlay did not start.".into());
    assert_eq!(toast.as_deref(), Some("The overlay did not start."));
    assert!(action.is_none());
}

#[test]
fn background_messages_keep_the_training_action() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    app.toast = Some("Outcome set to Victory".into());
    app.toast_action = Some(super::Message::DismissToast);
    let _ = app.update(super::Message::AutostartReady(true));
    let _ = app.update(super::Message::SeasonsFetched(Ok(Vec::new())));
    let _ = app.update(super::Message::Setup(
        crate::setup_guide::SetupMessage::PollReady(Err("down".into())),
    ));
    assert_eq!(app.toast.as_deref(), Some("Outcome set to Victory"));
    assert!(app.toast_action.is_some());
    let _ = app.update(super::Message::DaemonDone(Ok("restarted".into())));
    assert_eq!(app.toast.as_deref(), Some("restarted"));
    assert!(app.toast_action.is_none());
}

fn aur_owner() -> crate::uninstall::PackageOwner {
    crate::uninstall::PackageOwner {
        manager: crate::uninstall::PackageManager::Pacman,
        package: "scuffed-stat-tracker-bin".into(),
    }
}

fn offered_update() -> super::UpdateInfo {
    super::UpdateInfo {
        latest: "0.5.0".into(),
        current: "0.5.0-rc.7".into(),
        url: "https://example.invalid/release".into(),
        release_bodies: Vec::new(),
    }
}

// Only Pending and package-owned plans here: a script-install plan
// probes PATH and writes a probe file under the install prefix.
#[test]
fn the_package_probe_re_plans_unless_an_update_runs() {
    use super::{Message, UpdatePlan, UpdateProgress};
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    app.package_probed = false;
    let _ = app.update(Message::UpdateChecked(Some(offered_update())));
    assert_eq!(
        app.update_plan,
        UpdatePlan::Blocked {
            reason: crate::update::PACKAGE_PROBE_PENDING.into(),
            hint: String::new(),
        }
    );
    // Update now is a no-op until the probe answers.
    let _ = app.update(Message::RunUpdate);
    assert!(!matches!(app.update_progress, UpdateProgress::Running));

    // An update in progress keeps its plan.
    app.update_progress = UpdateProgress::Running;
    let _ = app.update(Message::PackageOwnerProbed(Some(aur_owner())));
    assert_eq!(app.package_owner, Some(aur_owner()));
    assert!(app.package_probed);
    assert!(matches!(
        &app.update_plan,
        UpdatePlan::Blocked { reason, .. } if reason == crate::update::PACKAGE_PROBE_PENDING
    ));

    app.update_progress = UpdateProgress::Idle;
    let _ = app.update(Message::PackageOwnerProbed(Some(aur_owner())));
    let UpdatePlan::Blocked { reason, hint } = &app.update_plan else {
        panic!("a package install is never Ready");
    };
    assert!(reason.contains("scuffed-stat-tracker-bin"));
    assert!(hint.contains("paru -Syu"));
}

#[test]
fn copy_update_command_does_nothing_on_a_package_install() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    app.package_owner = Some(aur_owner());
    app.update = Some(offered_update());
    app.toast = None;
    app.copy_text = |_| panic!("the test reached the clipboard");
    let _ = app.update(super::Message::CopyUpdateCmd);
    assert!(
        app.toast.is_none(),
        "no clipboard, no toast: {:?}",
        app.toast
    );
}

#[test]
fn copy_update_command_does_nothing_before_the_probe_answers() {
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    app.package_probed = false;
    app.package_owner = None;
    app.update = Some(offered_update());
    app.toast = None;
    app.copy_text = |_| panic!("the test reached the clipboard");
    let _ = app.update(super::Message::CopyUpdateCmd);
    assert!(
        app.toast.is_none(),
        "no clipboard, no toast: {:?}",
        app.toast
    );
    // Once the probe says "no package", the command is copied, through
    // the swapped copy function only.
    app.package_probed = true;
    app.copy_text = |cmd| {
        assert!(
            cmd.contains("STAT_TRACKER_TAG=stat-tracker-v0.5.0"),
            "{cmd}"
        );
        Ok(crate::clipboard::ClipboardBackend::WlCopy)
    };
    let _ = app.update(super::Message::CopyUpdateCmd);
    assert!(
        app.toast
            .as_deref()
            .is_some_and(|t| t.starts_with("Copied install command")),
        "{:?}",
        app.toast
    );
}

#[test]
fn a_running_update_leaves_the_quit_request_unpolled() {
    use crate::update::UpdateProgress;
    let polled = std::cell::Cell::new(0);
    let poll = || {
        polled.set(polled.get() + 1);
        true
    };
    assert!(!super::quit_request_due(&UpdateProgress::Running, poll));
    assert_eq!(
        polled.get(),
        0,
        "a running update must not consume the request"
    );
    for done in [
        UpdateProgress::Idle,
        UpdateProgress::Succeeded("ok".into()),
        UpdateProgress::Failed("no".into()),
    ] {
        assert!(super::quit_request_due(&done, poll), "{done:?}");
    }
    assert_eq!(polled.get(), 3);
    assert!(!super::quit_request_due(&UpdateProgress::Idle, || false));
}

#[test]
fn quit_gate_defers_a_quit_until_the_update_ends() {
    use crate::update::UpdateProgress;
    let update = crate::update::UpdateGate::new();
    let mut gate = super::QuitGate::default();
    assert!(
        !gate.request(&UpdateProgress::Running, &update),
        "no exit mid-update"
    );
    assert!(gate.update_finished(), "the waiting quit runs");
    assert!(!gate.update_finished(), "only once");

    for done in [
        UpdateProgress::Idle,
        UpdateProgress::Succeeded("ok".into()),
        UpdateProgress::Failed("no".into()),
    ] {
        let mut gate = super::QuitGate::default();
        assert!(gate.request(&done, &update), "{done:?} exits at once");
        assert!(!gate.update_finished(), "{done:?} leaves nothing pending");
    }

    let mut gate = super::QuitGate::default();
    assert!(!gate.request(&UpdateProgress::Running, &update));
    gate.cancel();
    assert!(!gate.update_finished(), "Show cancelled the quit");
}

/// F3 on #425: a relaunch that shows the window again must drop a
/// quit that waits for the update, as tray Show does.
#[test]
fn showing_the_window_again_drops_a_waiting_quit() {
    use crate::update::UpdateProgress;
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    app.update_gate = Box::leak(Box::new(crate::update::UpdateGate::new()));
    app.update_progress = UpdateProgress::Running;
    let _ = app.update(super::Message::Tray(crate::tray::TrayAction::Quit));
    assert!(app.quit_gate.deferred);
    let _ = app.on_show_request();
    assert!(!app.quit_gate.deferred, "the relaunch cancelled the quit");
    let _ = app.update(super::Message::UpdateFinished(Ok("Installed.".into())));
    assert!(!app.quit_gate.deferred);
}

/// F4 on #425: Quit while the update waits for the game exits at once
/// and cancels the update; nothing was stopped yet.
#[test]
fn quit_while_the_update_waits_for_the_game_exits_and_cancels_it() {
    use crate::update::UpdateProgress;
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    let gate: &'static crate::update::UpdateGate =
        Box::leak(Box::new(crate::update::UpdateGate::new()));
    app.update_gate = gate;
    crate::update::test_set_waiting(gate);
    app.update_progress = UpdateProgress::Running;
    let _ = app.update(super::Message::Tray(crate::tray::TrayAction::Quit));
    assert!(!app.quit_gate.deferred, "the quit did not wait");
    assert!(!gate.waiting(), "the waiting update was cancelled");
    assert!(!gate.cancel_waiting(), "it is cancelled, not waiting");
}

fn fixture_app(tmp: &std::path::Path) -> super::TrackerApp {
    super::TrackerApp::new(crate::fixtures::test_cli(tmp)).0
}

/// GUI-2: tray Quit used to call `iced::exit()` mid-update and drop the
/// installer with the tracker stopped.
#[test]
fn tray_quit_during_an_update_waits_for_it() {
    use crate::update::UpdateProgress;
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    app.update_gate = Box::leak(Box::new(crate::update::UpdateGate::new()));
    app.update_progress = UpdateProgress::Running;
    let _ = app.update(super::Message::Tray(crate::tray::TrayAction::Quit));
    assert!(app.quit_gate.deferred, "the quit waits for the update");
    assert!(app.window_id.is_none(), "the window is hidden meanwhile");
    let _ = app.update(super::Message::UpdateFinished(Ok("Installed.".into())));
    assert!(
        !app.quit_gate.deferred,
        "the quit ran when the update ended"
    );

    app.update_progress = UpdateProgress::Idle;
    let _ = app.update(super::Message::Tray(crate::tray::TrayAction::Quit));
    assert!(!app.quit_gate.deferred, "no update: quit at once");
}

#[test]
fn closing_the_last_window_without_a_tray_during_an_update_waits() {
    use crate::update::UpdateProgress;
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    app.tray = None;
    app.update_gate = Box::leak(Box::new(crate::update::UpdateGate::new()));
    let id = app.window_id.expect("window open");
    app.update_progress = UpdateProgress::Running;
    let _ = app.update(super::Message::WindowClosed(id));
    assert!(app.quit_gate.deferred);
    let _ = app.update(super::Message::UpdateFinished(Err("failed".into())));
    assert!(!app.quit_gate.deferred);
}

/// GUI-7: a re-check that fails keeps the banner; a new version
/// replaces it and clears an old error.
#[test]
fn a_periodic_recheck_only_replaces_the_offer_with_a_new_version() {
    use crate::update::{UpdateInfo, UpdateProgress};
    let tmp = tempfile::tempdir().unwrap();
    let mut app = fixture_app(tmp.path());
    let info = |v: &str| UpdateInfo {
        latest: v.into(),
        current: "0.5.0-rc.5".into(),
        url: String::new(),
        release_bodies: Vec::new(),
    };
    app.update = Some(info("0.5.0-rc.6"));
    let _ = app.update(super::Message::UpdateRechecked(None));
    assert_eq!(
        app.update.as_ref().map(|i| i.latest.as_str()),
        Some("0.5.0-rc.6")
    );

    app.update_progress = UpdateProgress::Failed("old error".into());
    let _ = app.update(super::Message::UpdateRechecked(Some(info("0.5.0-rc.7"))));
    assert_eq!(
        app.update.as_ref().map(|i| i.latest.as_str()),
        Some("0.5.0-rc.7")
    );
    assert_eq!(app.update_progress, UpdateProgress::Idle);
}

#[test]
fn hide_closes_the_open_window_and_is_noop_when_already_hidden() {
    let id = window::Id::unique();
    assert_eq!(tray_hide_op(Some(id)), Some(TrayWindowOp::Close(id)));
    assert_eq!(tray_hide_op(None), None);
}

struct WarnCount(std::sync::Arc<std::sync::atomic::AtomicUsize>);

impl<S> tracing_subscriber::Layer<S> for WarnCount
where
    S: tracing::Subscriber,
{
    fn on_event(
        &self,
        event: &tracing::Event<'_>,
        _ctx: tracing_subscriber::layer::Context<'_, S>,
    ) {
        if *event.metadata().level() == tracing::Level::WARN {
            self.0.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
}

fn shown_launch(
    dir: &std::path::Path,
    installed: Option<&str>,
    seen_memory: Option<&str>,
) -> Option<super::NotesDialog> {
    match super::notes_dialog_for(dir, installed, seen_memory) {
        super::LaunchNotes::Show(dialog) => Some(dialog),
        super::LaunchNotes::Quiet { .. } => None,
    }
}

#[test]
fn launch_notes_follow_the_stored_version() {
    let dir = tempfile::tempdir().expect("tempdir");
    let first = shown_launch(dir.path(), Some("0.4.23"), None).expect("first run");
    assert_eq!(first.remember.as_deref(), Some("0.4.23"));
    assert_eq!(first.sections[0].version, "0.4.23");
    assert!(first.sections[0].summary.contains("private log"));

    super::seasons::save_last_seen_version(dir.path(), "0.4.23").expect("save");
    assert!(shown_launch(dir.path(), Some("0.4.23"), None).is_none());

    super::seasons::save_last_seen_version(dir.path(), "0.4.19").expect("older");
    let upgraded = shown_launch(dir.path(), Some("0.4.23"), None).expect("upgrade");
    let versions: Vec<_> = upgraded
        .sections
        .iter()
        .map(|section| section.version.as_str())
        .collect();
    assert_eq!(versions, ["0.4.23", "0.4.22", "0.4.21", "0.4.20"]);

    super::seasons::save_last_seen_version(dir.path(), "0.4.23").expect("newer");
    let downgraded = shown_launch(dir.path(), Some("0.4.20"), None).expect("downgrade");
    assert_eq!(downgraded.sections.len(), 1);
    assert_eq!(downgraded.sections[0].version, "0.4.20");
}

#[test]
fn failed_ui_state_write_keeps_the_version_seen_for_the_session() {
    let dir = tempfile::tempdir().expect("tempdir");
    let blocker = dir.path().join("not-a-directory");
    std::fs::write(&blocker, b"x").expect("blocker file");

    let count = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let subscriber = tracing_subscriber::registry().with(WarnCount(count.clone()));
    tracing::subscriber::with_default(subscriber, || {
        let mut seen = super::NotesSeen::default();
        super::remember_installed(&mut seen, &blocker, "0.4.23");
        assert_eq!(seen.version.as_deref(), Some("0.4.23"));
        assert!(seen.write_error_logged);
        assert!(shown_launch(&blocker, Some("0.4.23"), seen.version.as_deref()).is_none());
        super::remember_installed(&mut seen, &blocker, "0.4.23");
        assert!(seen.write_error_logged);
    });
    assert_eq!(
        count.load(std::sync::atomic::Ordering::Relaxed),
        1,
        "a failed ui_state write is logged once per session"
    );

    let fresh = tempfile::tempdir().expect("tempdir");
    assert!(shown_launch(fresh.path(), Some("0.4.23"), Some("0.4.23")).is_none());
}

#[test]
fn show_opens_when_hidden_and_focuses_when_visible() {
    let id = window::Id::unique();
    assert_eq!(tray_show_op(None), TrayWindowOp::Open);
    assert_eq!(tray_show_op(Some(id)), TrayWindowOp::Focus(id));
}

#[test]
fn sync_counts_as_on_only_with_a_url_and_a_token() {
    use stat_tracker::config::SyncConfig;
    let with = |url: &str, token: &str| Config {
        sync: Some(SyncConfig {
            server_url: url.into(),
            token: token.into(),
        }),
        ..Config::default()
    };
    assert!(!super::sync_on_for(&Config::default()));
    assert!(!super::sync_on_for(&with("", "tok")));
    assert!(!super::sync_on_for(&with("https://crew.example", "  ")));
    assert!(super::sync_on_for(&with("https://crew.example", "tok")));
}

#[test]
fn the_classic_ocr_notice_replaces_an_idle_health_line_and_follows_a_sync_problem() {
    let notice = "Reading on hold: the maps-v1 reader pack is not installed.";
    for idle in [super::HEALTH_READY, super::HEALTH_WAITING] {
        assert_eq!(super::health_with_ocr_notice(idle.into(), None), idle);
        assert_eq!(
            super::health_with_ocr_notice(idle.into(), Some(notice)),
            notice
        );
    }
    let paused = "Sync paused: Website URL must use https.";
    assert_eq!(super::health_with_ocr_notice(paused.into(), None), paused);
    assert_eq!(
        super::health_with_ocr_notice(paused.into(), Some(notice)),
        format!("{paused}\n{notice}")
    );
    let declined = "Screen share was declined: no captures until Overwatch is restarted";
    let idle = super::health_with_ocr_notice(super::HEALTH_READY.into(), None);
    assert_eq!(
        super::health_with_ocr_notice(idle, Some(declined)),
        declined
    );
    assert!(!declined.contains('\u{2014}'));
}

#[test]
fn health_names_an_insecure_saved_sync_url() {
    let dir = std::env::temp_dir().join(format!("sst-health-m20-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("live_snapshot.json"), b"{}").unwrap();
    let paused = super::health_status_for(&dir, &[], Some(("http://example.com", "tok")));
    assert!(
        paused.starts_with("Sync paused:"),
        "existing cleartext config must be visible in tracker health, got {paused}"
    );
    assert!(!paused.contains('—'), "{paused}");
    assert!(!paused.contains('–'), "{paused}");
    assert!(paused.contains("https"), "{paused}");
    assert_eq!(
        super::health_status_for(&dir, &[], Some(("https://crew.example", "tok"))),
        "Ready"
    );
    assert_eq!(
        super::health_status_for(&dir, &[], Some(("http://127.0.0.1:3030", "tok"))),
        "Ready"
    );
    assert_eq!(
        super::health_status_for(&dir, &[], Some(("http://localhost", "tok"))),
        "Ready"
    );
    assert_eq!(
        super::health_status_for(&dir, &[], Some(("http://[::1]", "tok"))),
        "Ready"
    );
    stat_tracker::sync::write_auth_pause(&dir, "https://crew.example", "old-token").unwrap();
    let rejected = super::health_status_for(&dir, &[], Some(("https://crew.example", "old-token")));
    assert_eq!(rejected, stat_tracker::sync::SYNC_TOKEN_REJECTED_STATUS);
    assert!(rejected.contains("Sync token rejected"), "{rejected}");
    assert!(rejected.contains("Settings"), "{rejected}");
    assert_eq!(
        super::health_status_for(&dir, &[], Some(("https://crew.example", "new-token"))),
        "Ready",
        "a changed token clears the rejected state"
    );
    assert_eq!(
        super::health_status_for(&dir, &[], Some(("https://other.example", "old-token"))),
        "Ready",
        "a changed URL clears the rejected state"
    );
    stat_tracker::sync::write_auth_pause_reason(
        &dir,
        "https://crew.example",
        "expired-token",
        stat_tracker::sync::AuthPauseReason::Expired,
    )
    .unwrap();
    let expired =
        super::health_status_for(&dir, &[], Some(("https://crew.example", "expired-token")));
    assert_eq!(expired, stat_tracker::sync::SYNC_TOKEN_EXPIRED_STATUS);
    assert_eq!(expired, "Sync token expired, relink in Settings");
    assert!(expired.contains("relink"), "{expired}");
    assert!(expired.contains("Settings"), "{expired}");
    assert_eq!(
        super::health_status_for(&dir, &[], Some(("https://crew.example", "other-token"))),
        "Ready",
        "a changed token clears the expired state"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
