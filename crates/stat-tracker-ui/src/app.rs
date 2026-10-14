use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use gtk::gdk::prelude::MonitorExt;
use iced::widget::image::Handle;
use iced::widget::{column, container, responsive, row, stack};
use iced::{Element, Fill, Length, Padding, Subscription, Task, window};
use stat_tracker::capture::CaptureBackend;
use stat_tracker::config::{Config, ShadowRecognizerControl};
use stat_tracker::report::LogOnlyCategory;

use crate::aggregate::GameFilter;
use crate::capture::{self, PreviewShot};
use crate::cli::{Cli, FixtureKind};
use crate::daemon::{self, DaemonVerb, DaemonView};
use crate::hotkey::{self, OverlayHotkey};
use crate::model::{
    EditField, EditForm, Game, Outcome, Role, RoleFilter, Screen, SeasonSel, local_hm,
};
use crate::notes::{self, ShownRelease};
use crate::overlay::{self, CompanionChild, OverlayHold};
use crate::restart_prompt::{self, BinStamp, RestartNeed, RestartWatch};
use crate::seasons::{self, SeasonCache};
use crate::settings::{self, SettingsField, SettingsForm, SettingsToggle};
use crate::snapshot::{self, games_from_snapshot};
use crate::theme::{self, PAGE_PAD_X, PAGE_PAD_Y, SIDEBAR_WIDTH};
use crate::tray::{self, TrayAction, TrayHandle};
use crate::update::{self, UpdateInfo, UpdatePlan, UpdateProgress};
use crate::widgets;

/// Shown when the tracker service is already running.
const SETTINGS_SAVED_RESTART: &str =
    "Settings saved. Restart the tracker for changes to take effect.";

/// Save must not replace a config.toml that did not parse.
const SETTINGS_SAVE_REFUSED: &str =
    "Could not read config.toml, so settings were not saved. Fix that file, then try again.";

/// The file parsed, but it is not the snapshot this form was built from.
const SETTINGS_RELOADED_FROM_DISK: &str = "\
config.toml changed on disk. Settings reloaded from the file, please review and save again.";

/// How long the unreadable-config toast stays up before it clears itself.
const REFUSED_TOAST_TIMEOUT: Duration = Duration::from_secs(8);

fn toast_if_config_unreadable(unreadable: bool) -> Option<&'static str> {
    unreadable.then_some(SETTINGS_SAVE_REFUSED)
}

/// Every Save re-reads the file. `Err` means it still does not parse.
fn reread_config_for_save(path: &std::path::Path) -> Result<Config, &'static str> {
    Config::read_at(path).map_err(|_| SETTINGS_SAVE_REFUSED)
}

/// Result of pressing Save against the config file.
#[derive(Debug, PartialEq)]
enum SettingsFileSave {
    /// The file does not parse. Nothing was written.
    Unreadable,
    /// The form was built while the file was unreadable, or the parsed file
    /// differs from that snapshot. Nothing was written.
    Reloaded(Config),
    /// The on-disk parse matched the snapshot, and the form was written.
    Saved(Config),
}

#[derive(Debug)]
enum SettingsSaveError {
    /// Sync URL or shortcut check failed. The file was not written.
    Blocked(String),
    /// The in-place write failed.
    Write(String),
}

/// Re-read `path`, then either refuse, reload, or write the form in place.
///
/// A write happens only when the file parses as `snapshot`. The form's sync
/// URL and shortcut checks run in `before_write`, after that match, so a
/// reload is not blocked by the empty defaults a broken startup file leaves
/// in the form. The write is `form.to_config(snapshot)` then [`Config::save_at`],
/// which edits `shadow_recognizer` in place when that is the only change.
fn save_settings_form(
    path: &std::path::Path,
    form_built_unreadable: bool,
    snapshot: &Config,
    form: &SettingsForm,
    before_write: impl FnOnce() -> Result<(), String>,
) -> Result<SettingsFileSave, SettingsSaveError> {
    let fresh = match reread_config_for_save(path) {
        Err(_) => return Ok(SettingsFileSave::Unreadable),
        Ok(fresh) => fresh,
    };
    if form_built_unreadable || fresh != *snapshot {
        return Ok(SettingsFileSave::Reloaded(fresh));
    }
    before_write().map_err(SettingsSaveError::Blocked)?;
    let next = form.to_config(snapshot);
    next.save_at(path)
        .map_err(|err| SettingsSaveError::Write(err.to_string()))?;
    Ok(SettingsFileSave::Saved(next))
}

/// Form fields from `fresh`, keeping the shortcut the member was editing.
fn form_after_disk_reload(
    previous: &SettingsForm,
    fresh: &Config,
    shadow: ShadowRecognizerControl,
) -> SettingsForm {
    let mut next = SettingsForm::from_config_and_shadow(fresh, shadow);
    next.overlay_hotkey = previous.overlay_hotkey.clone();
    next.overlay_hotkey_enabled = previous.overlay_hotkey_enabled;
    next
}

/// True when `after` differs from `before` only in `update_channel`.
///
/// Only this app reads that key, so the save does not need a tracker restart.
fn only_update_channel_changed(before: &Config, after: &Config) -> bool {
    if before.update_channel == after.update_channel {
        return false;
    }
    let mut same = before.clone();
    same.update_channel.clone_from(&after.update_channel);
    same == *after
}

/// True when the save flipped the alpha updates opt-in, so the update check
/// must run again on the new channel.
fn update_channel_switched(before: &Config, after: &Config) -> bool {
    before.wants_prerelease_updates() != after.wants_prerelease_updates()
}

fn toast_after_successful_save(daemon_up: bool) -> &'static str {
    if daemon_up {
        SETTINGS_SAVED_RESTART
    } else {
        "Settings saved"
    }
}

/// Dismiss clears the toast, including the unreadable-config error.
fn toast_after_dismiss() -> Option<String> {
    None
}

/// R2-1 on #241: true when this Tick should act on a quit request. While an
/// in-app update runs, the installer's stdout and stderr are pipes to this
/// process; quitting would close them and kill the installer with SIGPIPE
/// on its next line, mid-install. So the request is not even polled then:
/// it stays pending and fires on the first Tick after the update ends, if
/// the replacing launch is still waiting. A launch that gave up removed the
/// request and asked this GUI to show instead.
fn quit_request_due(progress: &UpdateProgress, poll: impl FnOnce() -> bool) -> bool {
    !matches!(progress, UpdateProgress::Running) && poll()
}

/// A toast that is not the training prompt. Replacing the text drops the
/// action, so "Send this fix" cannot sit on a later message.
fn show_plain_toast(toast: &mut Option<String>, action: &mut Option<Message>, text: String) {
    *toast = Some(text);
    *action = None;
}

fn refused_toast_expired(
    message: Option<&str>,
    shown_at: Option<SystemTime>,
    now: SystemTime,
) -> bool {
    message == Some(SETTINGS_SAVE_REFUSED)
        && shown_at.is_some_and(|start| {
            now.duration_since(start)
                .is_ok_and(|elapsed| elapsed >= REFUSED_TOAST_TIMEOUT)
        })
}

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    Navigate(Screen),
    SelectSeason(SeasonSel),
    ToggleRole(Role),
    SeasonsFetched(Result<Vec<scuffed_types::Season>, String>),
    ToggleGame(String),
    FilterHero(Option<String>),
    FilterMap(Option<String>),
    FilterOutcome(Option<Outcome>),
    SetOutcome {
        session_id: String,
        outcome: Outcome,
    },
    ConfirmDelete(String),
    DeleteSession(String),
    ToggleEdit,
    EditField(EditField, String),
    SaveEdit,
    PickMap {
        session_id: String,
        map: String,
    },
    PickHero {
        session_id: String,
        hero: String,
    },
    ResolveSegment {
        session_id: String,
        segment: u32,
        confirm: bool,
    },
    RetryUpload(String),
    SettingsText(SettingsField, String),
    SettingsToggle(SettingsToggle, bool),
    SelectOutput(Option<String>),
    SaveSettings,
    Daemon(DaemonVerb),
    DaemonDone(Result<String, String>),
    ToggleAutostart,
    AutostartReady(bool),
    BackendReady(CaptureBackend),
    OutputsReady(Vec<String>),
    CaptureNow,
    CaptureReady(Result<PreviewShot, String>),
    InstallModel,
    RebuildModel,
    ModelReady(Result<String, String>),
    /// The reader packs in the data dir, looked at off the UI thread.
    /// `turn_off` is a click that asked to turn classic OCR off and waits
    /// for this look.
    ReaderPacksChecked {
        packs: stat_tracker::ocr_switch::PackReadiness,
        turn_off: bool,
    },
    /// The OK button under a refused click on the classic OCR switch.
    DismissOcrRefusal,
    Vacuum,
    VacuumReady(Result<String, String>),
    ConfirmClear,
    ClearData,
    ClearReady(Result<String, String>),
    UpdateChecked(Option<UpdateInfo>),
    OpenUpdate(String),
    CopyUpdateCmd,
    OpenUninstall,
    DismissUninstall,
    ToggleUninstallData(bool),
    ConfirmUninstall,
    UninstallFinished(Result<crate::uninstall::UninstallReport, String>),
    CopyUninstallCmd,
    RunUpdate,
    UpdateFinished(Result<String, String>),
    /// "Restart now" after an update: start the new binary, then quit.
    RestartGui,
    /// The installed GUI's `--build-id` output (G3), for the binary as
    /// stamped when the probe started.
    GuiBuildProbed {
        stamp: BinStamp,
        result: Result<String, String>,
    },
    /// "Later" on the restart prompt.
    RestartLater,
    WindowOpened(window::Id),
    WindowClosed(window::Id),
    Tray(TrayAction),
    ToggleOverlay,
    OpenAbout,
    DismissNotes,
    DismissToast,
    OpenNotesLink(String),
    ToggleUpdateNotes,
    ToggleReleaseDetails {
        surface: notes::NotesSurface,
        version: String,
    },
    ShowOlderReleases(notes::NotesSurface),
    Setup(crate::setup_guide::SetupMessage),
    Report(crate::report::ReportMessage),
}

struct NotesDialog {
    subtitle: String,
    sections: Vec<ShownRelease>,
    /// Installed version written when this dialog closes. Set for the
    /// once-per-version prompt. About keeps it if that prompt was already open.
    remember: Option<String>,
}

/// Version already acknowledged in this process, plus whether a failed
/// `ui_state.json` write has been logged.
#[derive(Debug, Default)]
struct NotesSeen {
    version: Option<String>,
    write_error_logged: bool,
}

enum LaunchNotes {
    Show(NotesDialog),
    /// Nothing to show. `persist` is the installed version to mark seen now,
    /// so an empty note list does not open again this session.
    Quiet {
        persist: Option<String>,
    },
}

/// What tray Hide / Show must do. Close+open (not Hidden/minimize) is the
/// only path that drops the surface from Wayland/niri Alt-Tab.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum TrayWindowOp {
    Close(window::Id),
    Open,
    Focus(window::Id),
}

pub(crate) fn tray_hide_op(window_id: Option<window::Id>) -> Option<TrayWindowOp> {
    window_id.map(TrayWindowOp::Close)
}

pub(crate) fn tray_show_op(window_id: Option<window::Id>) -> TrayWindowOp {
    match window_id {
        Some(id) => TrayWindowOp::Focus(id),
        None => TrayWindowOp::Open,
    }
}

pub fn window_settings() -> window::Settings {
    let fit = match current_display_size() {
        Some((width, height)) => crate::layout::fit_window(width, height),
        None => crate::layout::fit_window(0.0, 0.0),
    };
    window::Settings {
        size: iced::Size::new(fit.width, fit.height),
        min_size: Some(iced::Size::new(
            crate::layout::WINDOW_MIN_W.min(fit.width),
            crate::layout::WINDOW_MIN_H.min(fit.height),
        )),
        ..window::Settings::default()
    }
}

/// Primary monitor size in px, when GTK has a display.
fn current_display_size() -> Option<(f32, f32)> {
    if !gtk::is_initialized() {
        return None;
    }
    let display = gtk::gdk::Display::default()?;
    let monitor = display.primary_monitor().or_else(|| display.monitor(0))?;
    let rect = monitor.geometry();
    let width = rect.width();
    let height = rect.height();
    if width <= 0 || height <= 0 {
        return None;
    }
    Some((width as f32, height as f32))
}

fn live_game_session_now(
    data_dir: &Path,
    window_secs: u64,
    fixture: bool,
    now: DateTime<Utc>,
    process_running: bool,
) -> Option<String> {
    if fixture {
        return None;
    }
    overlay::in_progress_session_id(data_dir, window_secs, now, process_running)
}

pub struct TrackerApp {
    pub data_dir: PathBuf,
    pub fixture: Option<FixtureKind>,
    pub games: Vec<Game>,
    pub seasons: SeasonCache,
    pub season: SeasonSel,
    pub roles: RoleFilter,
    pub screen: Screen,
    pub live_status: String,
    pub health_status: String,
    pub clock: DateTime<Utc>,
    pub expanded: Option<String>,
    pub editing: bool,
    pub edit: EditForm,
    pub confirm_delete: Option<String>,
    pub filter_hero: Option<String>,
    pub filter_map: Option<String>,
    pub filter_outcome: Option<Outcome>,
    pub toast: Option<String>,
    /// Optional press on the toast. Cleared only when this toast is replaced.
    toast_action: Option<Message>,
    /// Sessions whose report kit was present the last time games were loaded.
    pub sessions_with_kit: HashSet<String>,
    /// When the unreadable-config toast was shown. Cleared with the toast.
    toast_shown_at: Option<SystemTime>,
    pub settings: SettingsForm,
    pub saved_config: Config,
    /// The on-disk config.toml did not parse. Save must not overwrite it.
    config_unreadable: bool,
    pub daemon: DaemonView,
    pub daemon_busy: bool,
    pub outputs: Vec<String>,
    pub backend: Option<CaptureBackend>,
    pub capturing: bool,
    pub preview: Option<(Handle, String)>,
    pub preview_error: Option<String>,
    pub update: Option<UpdateInfo>,
    pub update_progress: UpdateProgress,
    pub update_plan: UpdatePlan,
    /// Notes for the offered update, drawn inside the update prompt.
    pub update_notes: Vec<ShownRelease>,
    update_open_details: HashSet<String>,
    update_show_older: bool,
    pub update_notes_expanded: bool,
    dialog_open_details: HashSet<String>,
    dialog_show_older: bool,
    notes_dialog: Option<NotesDialog>,
    notes_seen: NotesSeen,
    uninstall_dialog: Option<crate::uninstall::UninstallDialog>,
    uninstall_busy: bool,
    /// Report a problem dialog, or the crash prompt shown at start.
    report: Option<crate::report::ReportUi>,
    /// Resolved once at startup so Settings can pin a tag without spawning
    /// the daemon on every frame.
    pub installed_version: Option<String>,
    pub confirm_clear: bool,
    pub tessdata_busy: bool,
    pub tessdata_installed: bool,
    /// Pack state, a refused toggle's reason, and the tracker's notice for
    /// the classic OCR switch.
    pub ocr_switch: settings::OcrSwitchView,
    pub vacuum_busy: bool,
    snapshot_mtime: Option<SystemTime>,
    seasons_url: Option<String>,
    last_seasons_attempt: Option<DateTime<Utc>>,
    window_id: Option<window::Id>,
    tick_count: u64,
    tray: Option<TrayHandle>,
    pub overlay_hold: OverlayHold,
    overlay_child: Option<CompanionChild>,
    overlay_spawn_blocked: bool,
    pub game_running: bool,
    /// Open session shown as the Games in-progress card. `None` once it closes.
    pub live_game_session: Option<String>,
    pub overlay_hotkey: OverlayHotkey,
    companion_toggle_mtime: Option<SystemTime>,
    pub setup: crate::setup_guide::GuideUi,
    /// This process's path at launch, for [`TrackerApp::restart_target`].
    own_exe: PathBuf,
    /// Watches the installed GUI binary for a changed build (G3).
    restart_watch: RestartWatch,
}

impl TrackerApp {
    pub fn new(cli: Cli) -> (Self, Task<Message>) {
        let snap = if let Some(kind) = cli.fixture {
            snapshot::install_fixture(&cli.data_dir, kind).unwrap_or_else(|e| {
                tracing::warn!(error = %e, "fixture install failed");
                snapshot::load_snapshot(&cli.data_dir)
            })
        } else {
            snapshot::load_snapshot(&cli.data_dir)
        };
        let games = games_from_snapshot(&snap);
        let mut seasons = seasons::load_cache(&cli.data_dir);
        if seasons.seasons.is_empty()
            && cli.fixture == Some(FixtureKind::Sample)
            && let Ok(list) = serde_json::from_str(crate::fixtures::sample_seasons_json())
        {
            seasons.seasons = list;
        }
        let persisted = seasons::load_ui_state(&cli.data_dir);
        let season = seasons::resolve_selection(persisted, &seasons.seasons);
        let clock = fixture_clock(cli.fixture, &games);
        let seasons_url = cli.seasons_url.clone();
        let now = Utc::now();
        let (fetch, last_seasons_attempt) =
            if seasons::should_fetch_seasons(cli.fixture.is_some(), seasons_url.as_deref()) {
                let url = seasons_url
                    .clone()
                    .expect("should_fetch_seasons requires a URL");
                (
                    Task::perform(seasons::fetch_seasons(url), Message::SeasonsFetched),
                    Some(now),
                )
            } else {
                (Task::none(), None)
            };

        let snapshot_mtime = snapshot::snapshot_mtime(&cli.data_dir);
        let live_status = live_status_for(&games);
        let config_existed = Config::config_path()
            .map(|path| path.exists())
            .unwrap_or(false);
        let (saved_config, config_unreadable) = match Config::load() {
            Ok(config) => (config, false),
            Err(e) => {
                tracing::warn!(
                    error = %e,
                    "config.toml did not parse; Settings will not overwrite it"
                );
                (Config::default(), true)
            }
        };
        let health_status = health_status_for(
            &cli.data_dir,
            &games,
            saved_config
                .sync
                .as_ref()
                .map(|s| (s.server_url.as_str(), s.token.as_str())),
        );
        let overlay_hotkey = seasons::load_overlay_hotkey(&cli.data_dir);
        let installed_version = update::current_version();
        let mut notes_seen = NotesSeen::default();
        let notes_dialog = match notes_dialog_for(&cli.data_dir, installed_version.as_deref(), None)
        {
            LaunchNotes::Show(dialog) => Some(dialog),
            LaunchNotes::Quiet {
                persist: Some(version),
            } => {
                remember_installed(&mut notes_seen, &cli.data_dir, &version);
                None
            }
            LaunchNotes::Quiet { persist: None } => None,
        };
        let mut settings = SettingsForm::from_config(&saved_config);
        settings.overlay_hotkey = overlay_hotkey.bind.clone();
        settings.overlay_hotkey_enabled = overlay_hotkey.enabled;
        let daemon = DaemonView {
            pid: daemon::daemon_running(&cli.data_dir),
            service_installed: daemon::service_file_installed(),
            autostart: false,
        };
        let live = cli.fixture.is_none();
        let (window_id, open) = window::open(window_settings());
        let process_running =
            cli.fixture.is_some() || overlay::process_is_running(&saved_config.game_process_names);
        let session_key = overlay::resolve_live_session_key(
            &cli.data_dir,
            cli.fixture.is_some(),
            saved_config.session_window_secs,
            now,
            process_running,
        );
        let live_game_session = live_game_session_now(
            &cli.data_dir,
            saved_config.session_window_secs,
            cli.fixture.is_some(),
            now,
            process_running,
        );
        let overlay_hold = overlay::reconcile_hold(
            OverlayHold::from_persisted(seasons::load_overlay_hidden_key(&cli.data_dir)),
            session_key.as_deref(),
        );
        if let Err(e) =
            seasons::save_overlay_hidden_key(&cli.data_dir, overlay_hold.persisted_key())
        {
            tracing::warn!(error = %e, "failed to persist overlay hold");
        }
        let game_running = session_key.is_some();
        let own_exe = std::env::current_exe().unwrap_or_default();
        // Fixture runs are dev screenshots: they never ask to restart.
        let installed_gui = if live {
            let target =
                crate::instance::relaunch_target(Some(&update::default_install_prefix()), &own_exe);
            restart_prompt::launch_baseline(
                restart_prompt::stamp(&target),
                crate::instance::relaunch_path(&own_exe) == target,
                restart_prompt::own_exe_identity(),
            )
        } else {
            None
        };
        let restart_watch = RestartWatch::new(restart_prompt::BUILD_ID, installed_gui);
        let setup = crate::setup_guide::GuideUi::startup(
            cli.fixture.is_none()
                && crate::setup_guide::should_auto_open(
                    config_existed,
                    saved_config.setup_completed,
                ),
            &saved_config,
            installed_version.as_deref(),
        );
        let mut app = Self {
            live_status,
            health_status,
            data_dir: cli.data_dir,
            fixture: cli.fixture,
            games,
            seasons,
            season,
            roles: RoleFilter::default(),
            screen: Screen::Overview,
            clock,
            expanded: None,
            editing: false,
            edit: EditForm::default(),
            confirm_delete: None,
            filter_hero: None,
            filter_map: None,
            filter_outcome: None,
            toast: None,
            toast_action: None,
            sessions_with_kit: HashSet::new(),
            toast_shown_at: None,
            settings,
            saved_config,
            config_unreadable,
            daemon,
            daemon_busy: false,
            outputs: Vec::new(),
            backend: None,
            capturing: false,
            preview: None,
            preview_error: None,
            update: None,
            installed_version,
            update_progress: UpdateProgress::Idle,
            update_plan: UpdatePlan::Blocked {
                reason: String::new(),
                hint: String::new(),
            },
            update_notes: Vec::new(),
            update_open_details: HashSet::new(),
            update_show_older: false,
            update_notes_expanded: update::update_notes_expanded_default(),
            dialog_open_details: HashSet::new(),
            dialog_show_older: false,
            notes_dialog,
            notes_seen,
            uninstall_dialog: None,
            uninstall_busy: false,
            report: None,
            confirm_clear: false,
            tessdata_busy: false,
            tessdata_installed: capture::tessdata_installed(),
            ocr_switch: settings::OcrSwitchView::default(),
            vacuum_busy: false,
            snapshot_mtime,
            seasons_url,
            last_seasons_attempt,
            window_id: Some(window_id),
            tick_count: 0,
            tray: tray::try_create(),
            overlay_hold,
            overlay_child: None,
            overlay_spawn_blocked: false,
            game_running,
            live_game_session,
            overlay_hotkey,
            companion_toggle_mtime: None,
            setup,
            own_exe,
            restart_watch,
        };
        app.refresh_kit_sessions();

        if let Some(err) = overlay::reconcile_companion(
            &mut app.overlay_child,
            overlay::overlay_visible(&app.overlay_hold, app.game_running),
            &app.data_dir,
            app.fixture,
            &mut app.overlay_spawn_blocked,
        ) {
            app.toast = Some(err);
        }
        if app.tray.is_none() && app.toast.is_none() && app.fixture.is_none() {
            app.toast = Some(tray::MISSING_TRAY_TOAST.to_string());
        }

        if app.fixture.is_none() {
            app.report = crate::report::crash_prompt(&app.data_dir);
        }

        let mut tasks = vec![fetch, open.map(Message::WindowOpened)];
        if live {
            tasks.push(check_reader_packs(app.data_dir.clone(), false));
            tasks.push(Task::perform(
                stat_tracker::capture::detect_backend(),
                Message::BackendReady,
            ));
            tasks.push(Task::perform(
                update::check_for_update(),
                Message::UpdateChecked,
            ));
            tasks.push(Task::perform(
                daemon::systemd_enabled(),
                Message::AutostartReady,
            ));
        }
        (app, Task::batch(tasks))
    }

    pub fn title(&self, _window: window::Id) -> String {
        "Scuffed Tracker".into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        let mut subs = vec![
            iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick),
            window::close_events().map(Message::WindowClosed),
        ];
        if self.overlay_hotkey.enabled
            && let Ok(bind) = hotkey::parse_bind(&self.overlay_hotkey.bind)
        {
            subs.push(hotkey::subscription(bind));
        }
        if self.report.is_some() {
            subs.push(iced::event::listen_with(report_dialog_escape));
        } else if self.uninstall_dialog.is_some() {
            subs.push(iced::event::listen_with(uninstall_dialog_escape));
        } else if self.setup.open {
            subs.push(iced::event::listen_with(setup_guide_keys));
        } else if self.notes_dialog.is_some() {
            subs.push(iced::event::listen_with(notes_dialog_escape));
        }
        Subscription::batch(subs)
    }

    pub fn season_window(&self) -> Option<crate::aggregate::SeasonWindow> {
        crate::seasons::window_for(&self.season, &self.seasons.seasons)
    }

    pub fn header_filter(&self) -> GameFilter {
        GameFilter::from_header(self.season_window(), self.roles)
    }

    pub fn overlay_showing(&self) -> bool {
        overlay::overlay_visible(&self.overlay_hold, self.game_running)
    }

    fn apply_overlay_policy(&mut self) {
        let now = Utc::now();
        let process_running = self.fixture.is_some()
            || overlay::process_is_running(&self.saved_config.game_process_names);
        let key = overlay::resolve_live_session_key(
            &self.data_dir,
            self.fixture.is_some(),
            self.saved_config.session_window_secs,
            now,
            process_running,
        );
        self.live_game_session = live_game_session_now(
            &self.data_dir,
            self.saved_config.session_window_secs,
            self.fixture.is_some(),
            now,
            process_running,
        );
        let next = overlay::reconcile_hold(self.overlay_hold.clone(), key.as_deref());
        if next != self.overlay_hold {
            self.overlay_hold = next;
            if let Err(e) =
                seasons::save_overlay_hidden_key(&self.data_dir, self.overlay_hold.persisted_key())
            {
                tracing::warn!(error = %e, "failed to persist overlay hold");
            }
        }
        self.game_running = key.is_some();
    }

    pub fn games_filter(&self) -> GameFilter {
        GameFilter {
            window: self.season_window(),
            roles: self.roles.selected_roles(),
            hero: self.filter_hero.clone(),
            map: self.filter_map.clone(),
            outcome: self.filter_outcome,
        }
    }

    pub fn update(&mut self, message: Message) -> Task<Message> {
        match message {
            Message::Tick => self.on_tick(),
            Message::Navigate(screen) => {
                self.screen = screen;
                if screen != Screen::Games {
                    self.expanded = None;
                    self.editing = false;
                    self.confirm_delete = None;
                }
                if screen != Screen::Settings {
                    self.confirm_clear = false;
                }
                if screen == Screen::Settings
                    && let Some(backend) = self.backend
                {
                    return Task::perform(capture::list_outputs(backend), Message::OutputsReady);
                }
                Task::none()
            }
            Message::SelectSeason(sel) => {
                self.season = sel;
                if let Err(e) = seasons::save_ui_state(&self.data_dir, &self.season) {
                    tracing::warn!(error = %e, "failed to write ui_state.json");
                }
                Task::none()
            }
            Message::ToggleRole(role) => {
                self.roles = self.roles.toggle(role);
                Task::none()
            }
            Message::SeasonsFetched(result) => {
                if self.fixture.is_some() {
                    return Task::none();
                }
                match result {
                    Ok(list) => {
                        let list =
                            seasons::apply_fetched_seasons(self.seasons.seasons.clone(), list);
                        if !list.is_empty()
                            && let Err(e) = seasons::write_cache(&self.data_dir, &list)
                        {
                            tracing::warn!(error = %e, "failed to write seasons.json cache");
                        }
                        let from_network = !list.is_empty();
                        let keep = self.season.clone();
                        self.seasons = SeasonCache {
                            seasons: list,
                            fetched_at: Some(Utc::now()),
                            from_network,
                        };
                        if keep
                            .as_id()
                            .is_some_and(|id| !self.seasons.seasons.iter().any(|s| s.id == id))
                        {
                            self.season = seasons::default_selection(&self.seasons.seasons);
                            if let Err(e) = seasons::save_ui_state(&self.data_dir, &self.season) {
                                tracing::warn!(error = %e, "failed to write ui_state.json");
                            }
                        }
                        self.health_status = self.health_now();
                    }
                    Err(e) => {
                        tracing::info!(error = %e, "seasons fetch failed; using cache");
                        self.health_status = self.health_now();
                    }
                }
                Task::none()
            }
            Message::ToggleGame(sid) => {
                if self.expanded.as_deref() == Some(sid.as_str()) {
                    self.expanded = None;
                    self.editing = false;
                    self.confirm_delete = None;
                } else {
                    self.expanded = Some(sid);
                    self.editing = false;
                    self.confirm_delete = None;
                    // The card's Report button reads this set. Refresh it
                    // here so a kit written after the last snapshot is visible.
                    self.refresh_kit_sessions();
                }
                Task::none()
            }
            Message::FilterHero(v) => {
                self.filter_hero = v;
                Task::none()
            }
            Message::FilterMap(v) => {
                self.filter_map = v;
                Task::none()
            }
            Message::FilterOutcome(v) => {
                self.filter_outcome = v;
                Task::none()
            }
            Message::SetOutcome {
                session_id,
                outcome,
            } => {
                let label = outcome.store_label();
                match crate::commands::set_outcome(&self.data_dir, &session_id, outcome) {
                    Ok(()) => {
                        self.replace_toast(format!("Outcome set to {label}"));
                        self.arm_training_toast(
                            &session_id,
                            crate::report::GameCategory::WrongResult,
                        );
                    }
                    Err(e) => {
                        self.replace_toast(format!("Could not save outcome: {e}"));
                    }
                }
                Task::none()
            }
            Message::ConfirmDelete(sid) => {
                if self.confirm_delete.as_deref() == Some(sid.as_str()) {
                    self.confirm_delete = None;
                } else {
                    self.confirm_delete = Some(sid);
                }
                Task::none()
            }
            Message::DeleteSession(sid) => {
                self.replace_toast(
                    match crate::commands::delete_session(&self.data_dir, &sid) {
                        Ok(()) => "Game deleted".into(),
                        Err(e) => format!("Could not delete game: {e}"),
                    },
                );
                self.confirm_delete = None;
                self.expanded = None;
                self.editing = false;
                Task::none()
            }
            Message::ToggleEdit => {
                if self.editing {
                    self.editing = false;
                } else if let Some(sid) = &self.expanded
                    && let Some(g) = self.games.iter().find(|g| &g.session_id == sid)
                {
                    self.edit = EditForm::from_game(g);
                    self.editing = true;
                }
                Task::none()
            }
            Message::EditField(field, value) => {
                self.edit.set(field, value);
                Task::none()
            }
            Message::SaveEdit => {
                let sid = self.expanded.clone();
                let saved = sid.as_ref().and_then(|sid| {
                    self.games
                        .iter()
                        .find(|g| &g.session_id == sid)
                        .map(|g| crate::commands::save_edit(&self.data_dir, g, &self.edit))
                });
                if let Some(sid) = sid {
                    match saved {
                        Some(Ok(())) => {
                            self.replace_toast("Stats corrected");
                            self.arm_training_toast(&sid, crate::report::GameCategory::WrongStats);
                        }
                        Some(Err(e)) => self.replace_toast(e),
                        None => {}
                    }
                }
                self.editing = false;
                Task::none()
            }
            Message::PickMap { session_id, map } => {
                match crate::commands::pick_map(&self.data_dir, &session_id, &map) {
                    Ok(()) => {
                        self.replace_toast("Map saved");
                        self.arm_training_toast(&session_id, crate::report::GameCategory::WrongMap);
                    }
                    Err(e) => self.replace_toast(format!("Could not save map: {e}")),
                }
                Task::none()
            }
            Message::PickHero { session_id, hero } => {
                match crate::commands::pick_hero(&self.data_dir, &session_id, &hero) {
                    Ok(()) => {
                        self.replace_toast("Hero saved");
                        self.arm_training_toast(
                            &session_id,
                            crate::report::GameCategory::WrongHero,
                        );
                    }
                    Err(e) => self.replace_toast(format!("Could not save hero: {e}")),
                }
                Task::none()
            }
            Message::RetryUpload(session_id) => {
                self.replace_toast(
                    match crate::commands::retry_upload(&self.data_dir, &session_id) {
                        Ok(()) => "Upload queued again".into(),
                        Err(e) => format!("Could not retry upload: {e}"),
                    },
                );
                Task::none()
            }
            Message::ResolveSegment {
                session_id,
                segment,
                confirm,
            } => {
                self.replace_toast(
                    match crate::commands::resolve_segment(
                        &self.data_dir,
                        &session_id,
                        segment,
                        confirm,
                    ) {
                        Ok(()) => {
                            if confirm {
                                "Hero swap confirmed".into()
                            } else {
                                "Hero swap dismissed".into()
                            }
                        }
                        Err(e) => format!("Could not update hero swap: {e}"),
                    },
                );
                Task::none()
            }
            Message::SettingsText(field, value) => {
                self.settings.set_text(field, value);
                Task::none()
            }
            Message::SettingsToggle(SettingsToggle::ClassicOcr, want_classic) => {
                self.toggle_classic_ocr(want_classic)
            }
            Message::SettingsToggle(toggle, value) => {
                self.settings.set_toggle(toggle, value);
                Task::none()
            }
            Message::DismissOcrRefusal => {
                self.ocr_switch.refusal = None;
                Task::none()
            }
            Message::ReaderPacksChecked { packs, turn_off } => {
                self.ocr_switch.packs = Some(packs);
                let needing_ocr = stat_tracker::ocr_switch::Read::needing_ocr();
                if turn_off {
                    match settings::classic_ocr_toggle(false, &packs, &needing_ocr) {
                        Ok(classic) => {
                            self.settings.classic_ocr = classic;
                            self.ocr_switch.refusal = None;
                        }
                        Err(reason) => {
                            self.settings.classic_ocr = true;
                            self.ocr_switch.refusal = Some(reason);
                        }
                    }
                } else {
                    self.ocr_switch.refusal = settings::refusal_after_pack_check(
                        self.ocr_switch.refusal.take(),
                        &packs,
                        &needing_ocr,
                    );
                }
                Task::none()
            }
            Message::SelectOutput(name) => {
                self.settings.capture_output = name.unwrap_or_default();
                Task::none()
            }
            Message::SaveSettings => {
                if self.fixture.is_some() {
                    return Task::none();
                }
                let config_path = match Config::config_path() {
                    Ok(path) => path,
                    Err(_) => {
                        self.config_unreadable = true;
                        self.show_refused_toast();
                        return Task::none();
                    }
                };
                let sync_url = self.settings.sync_url.clone();
                let saved_ocr = self.saved_config.ocr;
                let form_classic_ocr = self.settings.classic_ocr;
                let known_packs = self.ocr_switch.packs;
                let hotkey_enabled = self.settings.overlay_hotkey_enabled;
                let hotkey_bind = self.settings.overlay_hotkey.clone();
                let mut stored_hotkey = None;
                let outcome = save_settings_form(
                    &config_path,
                    self.config_unreadable,
                    &self.saved_config,
                    &self.settings,
                    || {
                        // Block the whole save. Writing the form would either store
                        // the cleartext URL or drop the sync block; the file on disk
                        // stays as it is until the URL is https, loopback http, or blank.
                        // This runs only after the file matches the form's snapshot, so
                        // a reload of a fixed file is not stopped by the default URL.
                        if let Some(problem) = settings::sync_url_problem(&sync_url) {
                            return Err(problem.to_string());
                        }
                        // Turning classic OCR off needs a tracker version
                        // that can, and the reader packs. The click looked;
                        // the save checks what that look found. The tracker
                        // itself decides again when it starts.
                        if let Some(reason) = settings::ocr_save_problem(
                            saved_ocr,
                            form_classic_ocr,
                            known_packs.as_ref(),
                            &stat_tracker::ocr_switch::Read::needing_ocr(),
                        ) {
                            return Err(reason);
                        }
                        let hotkey = OverlayHotkey::normalized(hotkey_enabled, &hotkey_bind);
                        stored_hotkey = Some(hotkey.validate_for_save()?);
                        Ok(())
                    },
                );
                match outcome {
                    Ok(SettingsFileSave::Unreadable) => {
                        self.config_unreadable = true;
                        self.show_refused_toast();
                    }
                    Ok(SettingsFileSave::Reloaded(fresh)) => {
                        self.settings = form_after_disk_reload(
                            &self.settings,
                            &fresh,
                            fresh.shadow_recognizer_control(),
                        );
                        self.saved_config = fresh;
                        self.config_unreadable = false;
                        self.health_status = self.health_now();
                        self.replace_toast(SETTINGS_RELOADED_FROM_DISK.to_string());
                        self.toast_shown_at = None;
                    }
                    Ok(SettingsFileSave::Saved(config)) => {
                        let Some(hotkey) = stored_hotkey else {
                            self.replace_toast(
                                "Could not save settings: shortcut was not checked".to_string(),
                            );
                            return Task::none();
                        };
                        if let Err(e) = seasons::save_overlay_hotkey(&self.data_dir, &hotkey) {
                            self.replace_toast(format!("Could not save companion shortcut: {e}"));
                            return Task::none();
                        }
                        let daemon_up = daemon::is_daemon_running(&self.data_dir);
                        let gui_only = only_update_channel_changed(&self.saved_config, &config);
                        let recheck = update_channel_switched(&self.saved_config, &config);
                        self.saved_config = config;
                        self.config_unreadable = false;
                        self.health_status = self.health_now();
                        self.overlay_hotkey = hotkey;
                        self.settings = SettingsForm::from_config(&self.saved_config);
                        self.settings.overlay_hotkey = self.overlay_hotkey.bind.clone();
                        self.settings.overlay_hotkey_enabled = self.overlay_hotkey.enabled;
                        self.replace_toast(toast_after_successful_save(daemon_up && !gui_only));
                        self.toast_shown_at = None;
                        if recheck {
                            // `check_for_update` reads the channel from config.toml,
                            // which now holds the new value.
                            return Task::perform(
                                update::check_for_update(),
                                Message::UpdateChecked,
                            );
                        }
                    }
                    Err(SettingsSaveError::Blocked(message)) => {
                        self.replace_toast(message);
                    }
                    Err(SettingsSaveError::Write(message)) => {
                        self.replace_toast(format!("Could not save settings: {message}"));
                    }
                }
                Task::none()
            }
            Message::Daemon(verb) => {
                if self.fixture.is_some() || self.daemon_busy {
                    return Task::none();
                }
                self.daemon_busy = true;
                let data_dir = self.data_dir.clone();
                let installed = self.daemon.service_installed;
                Task::perform(
                    daemon::run_verb(data_dir, verb, installed),
                    Message::DaemonDone,
                )
            }
            Message::DaemonDone(result) => {
                self.daemon_busy = false;
                self.replace_toast(match result {
                    Ok(m) => m,
                    Err(e) => e,
                });
                self.daemon = daemon::refresh_view(&self.data_dir, &self.daemon);
                Task::perform(daemon::systemd_enabled(), Message::AutostartReady)
            }
            Message::ToggleAutostart => {
                if self.fixture.is_some() || !self.daemon.service_installed {
                    return Task::none();
                }
                self.daemon_busy = true;
                Task::perform(
                    daemon::toggle_autostart(self.daemon.autostart),
                    Message::DaemonDone,
                )
            }
            Message::AutostartReady(enabled) => {
                self.daemon.autostart = enabled;
                Task::none()
            }
            Message::BackendReady(backend) => {
                self.backend = Some(backend);
                Task::perform(capture::list_outputs(backend), Message::OutputsReady)
            }
            Message::OutputsReady(outputs) => {
                self.outputs = outputs;
                Task::none()
            }
            Message::CaptureNow => {
                let backend = match capture::backend_ready(self.backend) {
                    Ok(b) => b,
                    Err(e) => {
                        self.preview_error = Some(e);
                        return Task::none();
                    }
                };
                self.capturing = true;
                self.preview_error = None;
                let output = settings::nonempty(&self.settings.capture_output);
                Task::perform(capture::capture_now(backend, output), Message::CaptureReady)
            }
            Message::CaptureReady(result) => {
                self.capturing = false;
                match result {
                    Ok(shot) => {
                        self.preview = Some((
                            Handle::from_rgba(shot.width, shot.height, shot.rgba),
                            shot.captured_at,
                        ));
                        self.preview_error = None;
                    }
                    Err(e) => self.preview_error = Some(e),
                }
                Task::none()
            }
            Message::InstallModel => {
                if self.fixture.is_some() || self.tessdata_busy {
                    return Task::none();
                }
                self.tessdata_busy = true;
                Task::perform(capture::install_reading_model(), Message::ModelReady)
            }
            Message::RebuildModel => {
                if self.fixture.is_some() || self.tessdata_busy {
                    return Task::none();
                }
                self.tessdata_busy = true;
                Task::perform(capture::rebuild_reading_model(), Message::ModelReady)
            }
            Message::ModelReady(result) => {
                self.tessdata_busy = false;
                self.tessdata_installed = capture::tessdata_installed();
                self.replace_toast(match result {
                    Ok(m) => m,
                    Err(e) => e,
                });
                Task::none()
            }
            Message::Vacuum => {
                if self.fixture.is_some() || self.vacuum_busy {
                    return Task::none();
                }
                self.vacuum_busy = true;
                Task::perform(
                    capture::vacuum_store(self.data_dir.clone()),
                    Message::VacuumReady,
                )
            }
            Message::VacuumReady(result) => {
                self.vacuum_busy = false;
                self.replace_toast(match result {
                    Ok(m) => m,
                    Err(e) => e,
                });
                Task::none()
            }
            Message::ConfirmClear => {
                self.confirm_clear = !self.confirm_clear;
                Task::none()
            }
            Message::ClearData => {
                if self.fixture.is_some() {
                    return Task::none();
                }
                self.confirm_clear = false;
                Task::perform(
                    capture::clear_store(self.data_dir.clone()),
                    Message::ClearReady,
                )
            }
            Message::ClearReady(result) => {
                let ok = result.is_ok();
                self.replace_toast(match result {
                    Ok(m) => m,
                    Err(e) => e,
                });
                if ok {
                    self.games.clear();
                    self.health_status = self.health_now();
                }
                Task::none()
            }
            Message::UpdateChecked(info) => {
                self.update_open_details.clear();
                self.update_show_older = false;
                self.update_notes_expanded = update::update_notes_expanded_default();
                self.update_notes = info
                    .as_ref()
                    .map(|i| {
                        let remote: Vec<notes::RemoteRelease<'_>> = i
                            .release_bodies
                            .iter()
                            .map(|body| notes::RemoteRelease {
                                version: body.version.as_str(),
                                body: body.body.as_str(),
                                published_at: body.published_at.as_deref(),
                            })
                            .collect();
                        notes::render(notes::notes_for_update(
                            notes::BUNDLED_CHANGELOG,
                            &i.current,
                            &i.latest,
                            &remote,
                        ))
                    })
                    .unwrap_or_default();
                self.update_plan = info
                    .as_ref()
                    .map(|i| update::evaluate_plan(&i.latest))
                    .unwrap_or_else(|| UpdatePlan::Blocked {
                        reason: String::new(),
                        hint: String::new(),
                    });
                self.update = info;
                Task::none()
            }
            Message::OpenAbout => {
                let remember = self.notes_dialog.as_ref().and_then(|d| d.remember.clone());
                self.dialog_open_details.clear();
                self.dialog_show_older = false;
                self.notes_dialog = Some(NotesDialog {
                    subtitle:
                        "Release notes bundled with this app. Install steps stay on the GitHub page."
                            .into(),
                    sections: notes::render(notes::all_notes(notes::BUNDLED_CHANGELOG)),
                    remember,
                });
                Task::none()
            }
            Message::DismissToast => {
                self.toast = toast_after_dismiss();
                self.toast_action = None;
                self.toast_shown_at = None;
                Task::none()
            }
            Message::DismissNotes => {
                if let Some(version) = self
                    .notes_dialog
                    .as_ref()
                    .and_then(|dialog| dialog.remember.clone())
                {
                    remember_installed(&mut self.notes_seen, &self.data_dir, &version);
                }
                self.notes_dialog = None;
                self.dialog_open_details.clear();
                self.dialog_show_older = false;
                Task::none()
            }
            Message::ToggleReleaseDetails { surface, version } => {
                let open = match surface {
                    notes::NotesSurface::Update => &mut self.update_open_details,
                    notes::NotesSurface::Dialog => &mut self.dialog_open_details,
                };
                if !open.remove(&version) {
                    open.insert(version);
                }
                Task::none()
            }
            Message::ToggleUpdateNotes => {
                self.update_notes_expanded = !self.update_notes_expanded;
                Task::none()
            }
            Message::ShowOlderReleases(surface) => {
                match surface {
                    notes::NotesSurface::Update => self.update_show_older = true,
                    notes::NotesSurface::Dialog => self.dialog_show_older = true,
                }
                Task::none()
            }
            Message::OpenNotesLink(url) => {
                if notes::notes_link_allowed(&url) {
                    update::open_release_page(&url);
                }
                Task::none()
            }
            Message::OpenUpdate(url) => {
                update::open_release_page(&url);
                Task::none()
            }
            Message::OpenUninstall => {
                let exe = std::env::current_exe().unwrap_or_default();
                let Some(home) =
                    dirs::home_dir().filter(|home| crate::uninstall::home_is_usable(home))
                else {
                    self.replace_toast("Could not find your home folder. Nothing was removed.");
                    self.toast_shown_at = Some(SystemTime::now());
                    return Task::none();
                };
                let owner = crate::uninstall::probe_package_owner(&exe);
                let mut dialog = crate::uninstall::open_dialog(&exe, &home, owner, &self.data_dir);
                if let crate::uninstall::UninstallDialog::Confirm { appimage, .. } = &mut dialog
                    && appimage.is_none()
                {
                    *appimage = crate::uninstall::appimage_override(
                        &exe,
                        std::env::var_os("APPIMAGE").map(PathBuf::from),
                    );
                }
                self.uninstall_dialog = Some(dialog);
                self.uninstall_busy = false;
                Task::none()
            }
            Message::DismissUninstall => {
                if !self.uninstall_busy {
                    self.uninstall_dialog = None;
                }
                Task::none()
            }
            Message::ToggleUninstallData(delete_data) => {
                if let Some(crate::uninstall::UninstallDialog::Confirm {
                    delete_data: slot, ..
                }) = &mut self.uninstall_dialog
                {
                    *slot = delete_data;
                }
                Task::none()
            }
            Message::ConfirmUninstall => {
                let Some(crate::uninstall::UninstallDialog::Confirm {
                    home,
                    prefix,
                    delete_data,
                    data_dir,
                    appimage,
                    ..
                }) = self.uninstall_dialog.clone()
                else {
                    return Task::none();
                };
                if self.uninstall_busy {
                    return Task::none();
                }
                self.uninstall_busy = true;
                let req = crate::uninstall::UninstallRequest {
                    home: home.clone(),
                    prefix: prefix.clone(),
                    data_dir,
                    appimage,
                    delete_data,
                    origin: crate::uninstall::InstallOrigin::Bootstrap { prefix },
                    systemctl: PathBuf::from("systemctl"),
                };
                Task::perform(uninstall_from_app(req), Message::UninstallFinished)
            }
            Message::UninstallFinished(result) => {
                self.uninstall_busy = false;
                match result {
                    Ok(report) if report.skipped => {
                        self.replace_toast("Nothing was removed.");
                        self.toast_shown_at = Some(SystemTime::now());
                        Task::none()
                    }
                    Ok(_) => {
                        overlay::stop_companion(&mut self.overlay_child, &self.data_dir);
                        iced::exit()
                    }
                    Err(err) => {
                        self.replace_toast(err);
                        self.toast_shown_at = Some(SystemTime::now());
                        Task::none()
                    }
                }
            }
            Message::CopyUninstallCmd => {
                let command = match &self.uninstall_dialog {
                    Some(crate::uninstall::UninstallDialog::Manual {
                        command: Some(command),
                        ..
                    }) => command.clone(),
                    _ => return Task::none(),
                };
                match crate::clipboard::copy_text(&command) {
                    Ok(backend) => {
                        self.replace_toast(format!(
                            "Copied the uninstall command via {}.",
                            backend.label()
                        ));
                        self.toast_shown_at = Some(SystemTime::now());
                        Task::none()
                    }
                    Err(e) => {
                        self.replace_toast(format!(
                            "Copied via the app clipboard. If paste is empty, {e}"
                        ));
                        self.toast_shown_at = Some(SystemTime::now());
                        iced::clipboard::write(command)
                    }
                }
            }
            Message::CopyUpdateCmd => {
                let cmd = update::install_command_for(
                    self.update.as_ref().map(|i| i.latest.as_str()),
                    self.installed_version.as_deref(),
                );
                match crate::clipboard::copy_text(&cmd) {
                    Ok(backend) => {
                        self.replace_toast(format!(
                            "Copied install command via {}",
                            backend.label()
                        ));
                        Task::none()
                    }
                    Err(e) => {
                        self.replace_toast(format!(
                            "Copied via the app clipboard. If paste is empty, {e}"
                        ));
                        iced::clipboard::write(cmd)
                    }
                }
            }
            Message::RunUpdate => {
                if matches!(self.update_progress, UpdateProgress::Running) {
                    return Task::none();
                }
                let Some(info) = self.update.clone() else {
                    return Task::none();
                };
                let plan = update::evaluate_plan(&info.latest);
                self.update_plan = plan.clone();
                if let UpdatePlan::Blocked { reason, hint } = &plan {
                    let msg = format!("{reason} {hint}");
                    self.update_progress = UpdateProgress::Failed(msg.clone());
                    self.replace_toast(msg);
                    return Task::none();
                }
                self.update_progress = UpdateProgress::Running;
                self.restart_watch.on_update_started();
                Task::perform(
                    update::run_in_app_update(update::UpdateRunRequest {
                        latest: info.latest,
                        data_dir: self.data_dir.clone(),
                        stop_daemon: self.daemon.running(),
                        service_installed: self.daemon.service_installed,
                    }),
                    Message::UpdateFinished,
                )
            }
            Message::UpdateFinished(result) => {
                match result {
                    Ok(msg) => {
                        self.replace_toast(update::success_line(&msg, self.restart_need()));
                        self.update_progress = UpdateProgress::Succeeded(msg);
                    }
                    Err(e) => {
                        self.update_progress = UpdateProgress::Failed(e.clone());
                        self.replace_toast(e);
                    }
                }
                self.daemon = daemon::refresh_view(&self.data_dir, &self.daemon);
                Task::none()
            }
            Message::GuiBuildProbed { stamp, result } => {
                let before = self.restart_need();
                self.restart_watch.on_probe(stamp, result);
                self.retoast_update_success(before);
                Task::none()
            }
            Message::RestartLater => {
                self.restart_watch.later();
                Task::none()
            }
            Message::RestartGui => {
                match crate::instance::spawn_replacement(Some(&self.restart_prefix())) {
                    Ok(()) => self.apply_tray(TrayAction::Quit),
                    Err(e) => {
                        self.replace_toast(format!(
                        "Could not start the new version ({e}). Quit Stat Tracker and start it again."
                    ));
                        Task::none()
                    }
                }
            }
            Message::WindowOpened(id) => {
                self.window_id = Some(id);
                window::gain_focus(id)
            }
            Message::WindowClosed(id) => {
                if self.window_id == Some(id) {
                    self.window_id = None;
                }
                // No tray and no window: nothing can restore us — exit.
                if self.window_id.is_none() && self.tray.is_none() {
                    overlay::stop_companion(&mut self.overlay_child, &self.data_dir);
                    iced::exit()
                } else {
                    Task::none()
                }
            }
            Message::Tray(action) => self.apply_tray(action),
            Message::ToggleOverlay => self.toggle_overlay(),
            Message::Setup(message) => self.on_setup(message),
            Message::Report(message) => self.on_report(message),
        }
    }

    fn report_dialog_mut(&mut self) -> Option<&mut crate::report::ReportDialog> {
        match &mut self.report {
            Some(crate::report::ReportUi::Dialog(dialog)) => Some(dialog.as_mut()),
            _ => None,
        }
    }

    fn replace_toast(&mut self, text: impl Into<String>) {
        show_plain_toast(&mut self.toast, &mut self.toast_action, text.into());
    }

    /// Prefix "Restart now" starts the new binary from: the one the in-app
    /// update installed to, else the default install prefix.
    pub fn restart_prefix(&self) -> PathBuf {
        update::restart_prefix(&self.update_plan)
    }

    /// The binary "Restart now" starts, and so the one the restart prompt
    /// watches.
    pub fn restart_target(&self) -> PathBuf {
        restart_prompt::watched_binary(&self.restart_prefix(), &self.own_exe)
    }

    pub fn restart_need(&self) -> RestartNeed {
        self.restart_watch.need()
    }

    /// Which prompt is drawn: the update banner or the restart bar, never
    /// both.
    fn prompt_layout(&self) -> restart_prompt::PromptLayout {
        restart_prompt::prompt_layout(
            &self.restart_watch,
            &self.update_progress,
            restart_prompt::update_banner_on_screen(self.update.is_some(), self.screen),
        )
    }

    pub fn restart_bar_visible(&self) -> bool {
        self.prompt_layout().bar
    }

    /// Overview and Settings draw the update banner only when this is true.
    pub fn update_banner_visible(&self) -> bool {
        self.prompt_layout().banner
    }

    /// After an in-app install the toast says "Press Restart now" until the
    /// installed build is read. If it turns out unchanged, say so instead.
    fn retoast_update_success(&mut self, before: RestartNeed) {
        if let Some(line) = restart_prompt::retoast(
            &self.update_progress,
            self.toast.as_deref(),
            before,
            self.restart_need(),
        ) {
            self.replace_toast(line);
        }
    }

    /// One stat of the installed GUI per Tick; a probe only when it changed.
    fn watch_installed_gui(&mut self) -> Option<Task<Message>> {
        if self.fixture.is_some() {
            return None;
        }
        let target = self.restart_target();
        let running = matches!(self.update_progress, UpdateProgress::Running);
        let before = self.restart_need();
        let probe = self
            .restart_watch
            .on_tick(restart_prompt::stamp(&target), running);
        // An install that left the GUI binary alone is settled here, with
        // no probe.
        self.retoast_update_success(before);
        let stamp = probe?;
        Some(Task::perform(
            restart_prompt::probe_build_id(target),
            move |result| Message::GuiBuildProbed { stamp, result },
        ))
    }

    fn refresh_kit_sessions(&mut self) {
        self.sessions_with_kit = self
            .games
            .iter()
            .filter(|game| crate::report::has_kit(&self.data_dir, &game.session_id))
            .map(|game| game.session_id.clone())
            .collect();
    }

    fn game_choices(&self) -> Vec<crate::report::GameChoice> {
        // Kit sessions stay listed. The newest games without a kit are capped
        // so a long history is not a column of disabled chips.
        const RECENT_WITHOUT_KIT: usize = 8;
        let mut without_kit = 0;
        let mut choices = Vec::new();
        for game in &self.games {
            // Every stored game can be reported: without a saved picture
            // the report carries the stored values and the session id.
            let has_picture = self.sessions_with_kit.contains(&game.session_id);
            let has_kit = true;
            if !has_picture {
                if without_kit == RECENT_WITHOUT_KIT {
                    continue;
                }
                without_kit += 1;
            }
            choices.push(crate::report::GameChoice {
                session_id: game.session_id.clone(),
                label: format!(
                    "{} · {} · {} {}{}",
                    game.display_hero(),
                    game.display_map(),
                    game.played_at
                        .with_timezone(&chrono::Local)
                        .format("%Y-%m-%d"),
                    local_hm(game.played_at),
                    if has_picture {
                        ""
                    } else {
                        " · no picture saved"
                    }
                ),
                has_kit,
            });
        }
        choices
    }

    fn arm_training_toast(&mut self, session_id: &str, category: crate::report::GameCategory) {
        if !self.games.iter().any(|g| g.session_id == session_id) {
            self.toast_action = None;
            return;
        }
        self.toast_action = Some(Message::Report(crate::report::ReportMessage::OpenGame {
            session_id: Some(session_id.to_string()),
            category,
            training: true,
            choices: Vec::new(),
        }));
        self.toast_shown_at = None;
    }

    /// Start a build for the dialog's current inputs. A result for an older
    /// generation is dropped when it lands. A game report loads its kit
    /// inside `build_game`; this function does not read crop bytes.
    fn rebuild_report(&mut self) -> Task<Message> {
        let mut config = self.saved_config.clone();
        config.data_dir = self.data_dir.clone();
        let capture_backend = self.backend.unwrap_or(CaptureBackend::None);
        let Some(dialog) = self.report_dialog_mut() else {
            return Task::none();
        };
        dialog.text_edited_at = None;
        dialog.kit_images.clear();
        let generation = dialog.bump();
        dialog.building = Some(generation);
        if let Some(category) = dialog.game_category {
            let req = crate::report::GameBuildRequest {
                config,
                session_id: dialog.game_session.clone().unwrap_or_default(),
                category,
                text: dialog.text.clone(),
                training: dialog.training,
                capture_backend,
            };
            return Task::perform(crate::report::build_game(req), move |result| {
                Message::Report(crate::report::ReportMessage::Built(generation, result))
            });
        }
        let req = crate::report::BuildRequest {
            config,
            category: dialog.category,
            text: dialog.text.clone(),
            training: dialog.training,
            resolution: crate::report::display_resolution(),
            capture_backend,
        };
        Task::perform(crate::report::build(req), move |result| {
            Message::Report(crate::report::ReportMessage::Built(generation, result))
        })
    }

    fn on_report(&mut self, message: crate::report::ReportMessage) -> Task<Message> {
        use crate::report::{
            GameReportCategory, ReportDialog, ReportMessage, ReportUi, SendOutcome, SendState,
        };
        match message {
            ReportMessage::Open(category) => {
                if self.report_dialog_mut().is_some_and(|d| d.busy()) {
                    return Task::none();
                }
                let linked = self.saved_config.sync.is_some();
                let category = category.unwrap_or(LogOnlyCategory::Other);
                self.report = Some(ReportUi::Dialog(Box::new(ReportDialog::new(
                    category, linked,
                ))));
                self.rebuild_report()
            }
            ReportMessage::OpenGame {
                session_id,
                category,
                training,
                choices,
            } => {
                if self.report_dialog_mut().is_some_and(|d| d.busy()) {
                    return Task::none();
                }
                if self
                    .report_dialog_mut()
                    .is_some_and(|d| !d.is_game() && !d.sent() && !d.text.trim().is_empty())
                {
                    self.replace_toast("Finish or close the open report first.");
                    return Task::none();
                }
                self.refresh_kit_sessions();
                let linked = self.saved_config.sync.is_some();
                let mut dialog = ReportDialog::new(LogOnlyCategory::Other, linked);
                dialog.game_category = Some(category);
                dialog.game_session = session_id;
                dialog.training = training;
                dialog.game_choices = if choices.is_empty() {
                    self.game_choices()
                } else {
                    choices
                };
                self.report = Some(ReportUi::Dialog(Box::new(dialog)));
                self.toast = None;
                self.toast_action = None;
                self.rebuild_report()
            }
            ReportMessage::CrashYes => {
                let linked = self.saved_config.sync.is_some();
                self.report = Some(ReportUi::Dialog(Box::new(ReportDialog::new(
                    LogOnlyCategory::Crash,
                    linked,
                ))));
                self.rebuild_report()
            }
            ReportMessage::CrashNo => {
                self.dismiss_crash_marker();
                self.report = None;
                Task::none()
            }
            ReportMessage::Close => {
                match &self.report {
                    Some(ReportUi::Dialog(d)) if d.busy() => return Task::none(),
                    // Escape on the prompt is not an answer; it asks again next start.
                    _ => self.report = None,
                }
                Task::none()
            }
            ReportMessage::Category(category) => {
                let Some(d) = self.report_dialog_mut() else {
                    return Task::none();
                };
                if d.category == category && d.game_category.is_none() {
                    return Task::none();
                }
                d.category = category;
                d.game_category = None;
                d.game_session = None;
                d.kit_images.clear();
                self.rebuild_report()
            }
            ReportMessage::GameCategory(category) => {
                let choices = self.game_choices();
                let Some(dialog) = self.report_dialog_mut() else {
                    return Task::none();
                };
                if dialog.busy() || dialog.sent() {
                    return Task::none();
                }
                dialog.game_category = Some(category);
                if dialog.game_choices.is_empty() {
                    dialog.game_choices = choices;
                }
                if category.allows_missing_kit() {
                    dialog.game_session = None;
                }
                dialog.kit_images.clear();
                self.rebuild_report()
            }
            ReportMessage::PickGame(session_id) => {
                let Some(d) = self.report_dialog_mut() else {
                    return Task::none();
                };
                if d.busy() || d.sent() {
                    return Task::none();
                }
                d.game_session = Some(session_id);
                d.kit_images.clear();
                self.rebuild_report()
            }
            ReportMessage::ToggleImageScale => {
                if let Some(d) = self.report_dialog_mut() {
                    d.image_actual = !d.image_actual;
                }
                Task::none()
            }
            ReportMessage::Training(on) => {
                let Some(d) = self.report_dialog_mut() else {
                    return Task::none();
                };
                d.training = on;
                self.rebuild_report()
            }
            ReportMessage::Text(value) => {
                if let Some(d) = self.report_dialog_mut() {
                    d.text = value
                        .chars()
                        .take(stat_tracker::report::bundle::REASON_TEXT_MAX_CHARS)
                        .collect();
                    d.text_edited_at = Some(std::time::Instant::now());
                    d.bump();
                }
                Task::none()
            }
            ReportMessage::ToggleLog => {
                if let Some(d) = self.report_dialog_mut() {
                    d.show_log = !d.show_log;
                }
                Task::none()
            }
            ReportMessage::Built(generation, result) => {
                if let Some(d) = self.report_dialog_mut()
                    && d.building == Some(generation)
                {
                    d.building = None;
                    if d.generation == generation {
                        d.kit_images = match (&result, d.is_game()) {
                            (Ok(zip), true) => crate::report::kit_images_from_zip(zip),
                            _ => Vec::new(),
                        };
                        d.built = Some(result);
                    }
                }
                Task::none()
            }
            ReportMessage::Send => {
                let sync = self.saved_config.sync.clone();
                let Some(d) = self.report_dialog_mut() else {
                    return Task::none();
                };
                let (Some(sync), Some(zip)) = (sync, d.zip()) else {
                    return Task::none();
                };
                if !d.can_send() {
                    return Task::none();
                }
                d.send = SendState::Sending;
                d.confirm_save = false;
                Task::perform(
                    crate::report::send(sync.server_url, sync.token, zip),
                    |outcome| Message::Report(ReportMessage::Sent(outcome)),
                )
            }
            ReportMessage::Sent(outcome) => {
                let sent = matches!(outcome, SendOutcome::Sent { .. });
                let data_dir = self.data_dir.clone();
                if let Some(d) = self.report_dialog_mut() {
                    if sent {
                        // The app keeps no copy after a successful send,
                        // and the game's saved pictures go too: the server
                        // accepted them.
                        d.built = None;
                        d.show_log = false;
                        if let Some(sid) = d.game_session.clone() {
                            stat_tracker::report::kit::remove(&data_dir, &sid);
                        }
                    }
                    d.send = SendState::Done(outcome);
                }
                if sent {
                    self.dismiss_crash_marker();
                }
                Task::none()
            }
            ReportMessage::Save => {
                let Some(d) = self.report_dialog_mut() else {
                    return Task::none();
                };
                if !d.can_save() {
                    return Task::none();
                }
                if !d.confirm_save {
                    d.confirm_save = true;
                    return Task::none();
                }
                let Some(zip) = d.zip() else {
                    return Task::none();
                };
                d.saving = true;
                Task::perform(crate::report::save(zip), |result| {
                    Message::Report(ReportMessage::Saved(result))
                })
            }
            ReportMessage::CancelSave => {
                if let Some(d) = self.report_dialog_mut() {
                    d.confirm_save = false;
                }
                Task::none()
            }
            ReportMessage::Saved(result) => {
                if let Some(d) = self.report_dialog_mut() {
                    d.saving = false;
                    d.confirm_save = false;
                    d.saved = Some(result);
                }
                Task::none()
            }
            ReportMessage::OpenMyReports(url) => {
                update::open_release_page(&url);
                Task::none()
            }
        }
    }

    fn dismiss_crash_marker(&mut self) {
        if let Err(error) = stat_tracker::report::marker::dismiss_last_crash(&self.data_dir) {
            tracing::warn!(%error, "could not remove last_crash.marker");
        }
    }

    fn on_setup(&mut self, message: crate::setup_guide::SetupMessage) -> Task<Message> {
        use crate::setup_guide::SetupMessage as Step;
        match message {
            Step::Open => {
                self.setup.reopen(&self.saved_config);
                Task::none()
            }
            Step::Skip => {
                let patch = self.setup.skip();
                self.persist_setup_patch(patch);
                Task::none()
            }
            Step::SkipStep => {
                if let Some(patch) = self.setup.skip_step() {
                    self.persist_setup_patch(patch);
                }
                Task::none()
            }
            Step::Back => {
                self.setup.back();
                Task::none()
            }
            Step::Next => {
                if let Some(patch) = self.setup.continue_step() {
                    self.persist_setup_patch(patch);
                }
                Task::none()
            }
            Step::SyncUrl(value) => {
                self.setup.set_sync_url(value);
                Task::none()
            }
            Step::PasteToken(value) => {
                self.setup.set_paste_token(value);
                Task::none()
            }
            Step::TogglePaste => {
                self.setup.toggle_paste();
                Task::none()
            }
            Step::RequestCapture => self.spawn_guide_capture(false),
            Step::CaptureReady(result) => {
                self.setup.capture_ready(result);
                Task::none()
            }
            Step::RequestTab => self.spawn_guide_capture(true),
            Step::TabReady(result) => {
                self.setup.tab_ready(result);
                Task::none()
            }
            Step::ConfirmScoreboard => {
                self.setup.confirm_scoreboard();
                Task::none()
            }
            Step::SignIn => {
                let Some((base, label, version)) = self.setup.begin_sign_in() else {
                    return Task::none();
                };
                Task::perform(
                    crate::setup_guide::start_link(base, label, version),
                    |result| Message::Setup(Step::LinkStarted(result)),
                )
            }
            Step::LinkStarted(result) => {
                self.setup.link_started(result);
                Task::none()
            }
            Step::PollReady(result) => {
                if let Some(patch) = self.setup.poll_ready(result) {
                    self.persist_setup_patch(patch);
                }
                Task::none()
            }
            Step::CheckToken => {
                let Some((base, token)) = self.setup.begin_check() else {
                    return Task::none();
                };
                Task::perform(crate::setup_guide::check_token(base, token), |result| {
                    Message::Setup(Step::TokenChecked(result))
                })
            }
            Step::TokenChecked(result) => {
                if let Some(patch) = self.setup.token_checked(result) {
                    self.persist_setup_patch(patch);
                }
                Task::none()
            }
            Step::OpenSite => {
                if let Some(url) = self.setup.prepare_site_url() {
                    update::open_release_page(&url);
                }
                Task::none()
            }
            Step::DownloadPack => {
                let Some(sync) = self.saved_config.sync.clone().filter(|sync| {
                    !sync.token.trim().is_empty() && !sync.server_url.trim().is_empty()
                }) else {
                    self.setup.pack_needs_sign_in();
                    return Task::none();
                };
                if !self.setup.begin_pack_fetch() {
                    return Task::none();
                }
                let dir = self.saved_config.data_dir.clone();
                Task::perform(
                    async move {
                        stat_tracker::packs::sync_reader_packs(
                            &sync.server_url,
                            &sync.token,
                            &dir,
                            stat_tracker::packs::PACK_MAX_BYTES,
                        )
                        .await
                        .guide_result()
                    },
                    |result| Message::Setup(Step::PackReady(result)),
                )
            }
            Step::PackReady(result) => {
                self.setup.download_ready(result);
                // The classic OCR switch shows which packs are there.
                check_reader_packs(self.data_dir.clone(), false)
            }
        }
    }

    fn spawn_guide_capture(&mut self, tab: bool) -> Task<Message> {
        let started = if tab {
            self.setup.begin_tab()
        } else {
            self.setup.begin_capture()
        };
        if !started {
            return Task::none();
        }
        let backend = match capture::backend_ready(self.backend) {
            Ok(backend) => backend,
            Err(_) => {
                let message = crate::setup_guide::CAPTURE_FAILED.to_string();
                if tab {
                    self.setup.tab_ready(Err(message));
                } else {
                    self.setup.capture_ready(Err(message));
                }
                return Task::none();
            }
        };
        let output = settings::nonempty(&self.settings.capture_output);
        Task::perform(
            crate::setup_guide::capture_test_frame(backend, output),
            move |result| {
                if tab {
                    Message::Setup(crate::setup_guide::SetupMessage::TabReady(result))
                } else {
                    Message::Setup(crate::setup_guide::SetupMessage::CaptureReady(result))
                }
            },
        )
    }

    fn persist_setup_patch(&mut self, patch: stat_tracker::config::SetupDiskPatch) {
        if self.fixture.is_some() {
            return;
        }
        if self.config_unreadable {
            self.show_refused_toast();
            return;
        }
        let path = match Config::config_path() {
            Ok(path) => path,
            Err(_) => {
                self.replace_toast(crate::setup_guide::SAVE_FAILED.to_string());
                return;
            }
        };
        match Config::apply_setup_patch(&path, &patch) {
            Ok(fresh) => {
                if patch.setup_completed.is_some() {
                    self.saved_config.setup_completed = fresh.setup_completed;
                }
                if patch.sync.is_some() {
                    self.saved_config.sync = fresh.sync.clone();
                    if let Some(sync) = &fresh.sync {
                        self.settings.sync_url = sync.server_url.clone();
                        self.settings.sync_token = sync.token.clone();
                    }
                    self.health_status = self.health_now();
                }
            }
            Err(_) => {
                self.replace_toast(crate::setup_guide::SAVE_FAILED.to_string());
            }
        }
    }

    fn show_refused_toast(&mut self) {
        self.replace_toast(
            toast_if_config_unreadable(true)
                .unwrap_or(SETTINGS_SAVE_REFUSED)
                .to_string(),
        );
        self.toast_shown_at = Some(SystemTime::now());
    }

    fn expire_refused_toast(&mut self) {
        if refused_toast_expired(
            self.toast.as_deref(),
            self.toast_shown_at,
            SystemTime::now(),
        ) {
            self.toast = None;
            self.toast_action = None;
            self.toast_shown_at = None;
        }
    }

    fn on_tick(&mut self) -> Task<Message> {
        // Checked before the rest of the tick, whose tray and hotkey paths
        // return early, so a typing pause always rebuilds on this tick.
        let report = if self
            .report_dialog_mut()
            .and_then(|d| d.text_edited_at)
            .is_some_and(|at| at.elapsed() >= crate::report::TEXT_REBUILD_DELAY)
        {
            self.rebuild_report()
        } else {
            Task::none()
        };
        Task::batch([report, self.tick_rest()])
    }

    fn tick_rest(&mut self) -> Task<Message> {
        self.expire_refused_toast();
        if self.fixture.is_none() {
            let mtime = snapshot::snapshot_mtime(&self.data_dir);
            if mtime != self.snapshot_mtime {
                self.snapshot_mtime = mtime;
                let snap = snapshot::load_snapshot(&self.data_dir);
                self.games = games_from_snapshot(&snap);
                self.refresh_kit_sessions();
                self.live_status = live_status_for(&self.games);
                self.health_status = self.health_now();
                self.clock = Utc::now();
            }
            self.daemon = daemon::refresh_view(&self.data_dir, &self.daemon);
            let status = stat_tracker::ocr_switch::read_status(&self.data_dir);
            self.ocr_switch.daemon_off =
                settings::daemon_runs_ocr_off(status.as_ref(), self.daemon.pid);
            self.ocr_switch.notice = settings::daemon_ocr_notice(status.as_ref(), self.daemon.pid);
            let health = settings::daemon_health_notice(status.as_ref(), self.daemon.pid);
            if health != self.ocr_switch.health_notice {
                self.ocr_switch.health_notice = health;
                self.health_status = self.health_now();
            }
        }

        tray::pump_gtk();
        if let Some(handle) = &self.tray
            && let Some(action) = tray::poll(handle)
        {
            return self.apply_tray(action);
        }

        if quit_request_due(&self.update_progress, || {
            crate::instance::consume_quit(&self.data_dir)
        }) {
            tracing::info!("a newer tracker GUI asked this one to quit");
            return self.apply_tray(TrayAction::Quit);
        }
        if crate::instance::consume_show(&self.data_dir) {
            return self.show_window();
        }

        if hotkey::consume_toggle(&self.data_dir, &mut self.companion_toggle_mtime) {
            return self.toggle_overlay();
        }

        self.apply_overlay_policy();
        let want_overlay = self.overlay_showing();
        if let Some(err) = overlay::reconcile_companion(
            &mut self.overlay_child,
            want_overlay,
            &self.data_dir,
            self.fixture,
            &mut self.overlay_spawn_blocked,
        ) {
            show_plain_toast(&mut self.toast, &mut self.toast_action, err);
        }

        let mut tasks = Vec::new();
        if let Some(probe) = self.watch_installed_gui() {
            tasks.push(probe);
        }
        if let Some((url, code)) = self.setup.poll_request_if_due(std::time::Instant::now()) {
            tasks.push(Task::perform(
                crate::setup_guide::poll_link(url, code),
                |result| Message::Setup(crate::setup_guide::SetupMessage::PollReady(result)),
            ));
        }

        self.tick_count = self.tick_count.saturating_add(1);
        if self.fixture.is_none() && self.tick_count.is_multiple_of(10) {
            tasks.push(Task::perform(
                daemon::systemd_enabled(),
                Message::AutostartReady,
            ));
        }

        let now = Utc::now();
        if seasons::should_refetch(
            self.fixture.is_some(),
            self.seasons_url.as_deref(),
            self.last_seasons_attempt,
            now,
        ) {
            let url = self
                .seasons_url
                .clone()
                .expect("should_refetch requires a URL");
            self.last_seasons_attempt = Some(now);
            tasks.push(Task::perform(
                seasons::fetch_seasons(url),
                Message::SeasonsFetched,
            ));
        }

        if tasks.is_empty() {
            Task::none()
        } else {
            Task::batch(tasks)
        }
    }

    fn apply_tray(&mut self, action: TrayAction) -> Task<Message> {
        match action {
            TrayAction::Quit => {
                overlay::stop_companion(&mut self.overlay_child, &self.data_dir);
                iced::exit()
            }
            TrayAction::Show => self.show_window(),
            TrayAction::Hide => self.hide_window(),
            TrayAction::ToggleOverlay => self.toggle_overlay(),
        }
    }

    fn toggle_overlay(&mut self) -> Task<Message> {
        self.apply_overlay_policy();
        let key = overlay::live_session_key(
            &self.data_dir,
            &self.saved_config.game_process_names,
            self.fixture.is_some(),
            self.saved_config.session_window_secs,
        );
        self.overlay_hold = overlay::toggle_hold(self.overlay_hold.clone(), key.as_deref());
        if let Err(e) =
            seasons::save_overlay_hidden_key(&self.data_dir, self.overlay_hold.persisted_key())
        {
            tracing::warn!(error = %e, "failed to persist overlay hold");
        }
        self.game_running = key.is_some();
        let want_overlay = self.overlay_showing();
        if let Some(err) = overlay::reconcile_companion(
            &mut self.overlay_child,
            want_overlay,
            &self.data_dir,
            self.fixture,
            &mut self.overlay_spawn_blocked,
        ) {
            show_plain_toast(&mut self.toast, &mut self.toast_action, err);
        }
        Task::none()
    }

    fn show_window(&mut self) -> Task<Message> {
        match tray_show_op(self.window_id) {
            TrayWindowOp::Focus(id) => window::gain_focus(id),
            TrayWindowOp::Open => {
                let (id, open) = window::open(window_settings());
                self.window_id = Some(id);
                open.map(Message::WindowOpened)
            }
            TrayWindowOp::Close(_) => Task::none(),
        }
    }

    fn hide_window(&mut self) -> Task<Message> {
        match tray_hide_op(self.window_id) {
            Some(TrayWindowOp::Close(id)) => {
                self.window_id = None;
                window::close(id)
            }
            _ => Task::none(),
        }
    }

    pub fn view(&self, _window: window::Id) -> Element<'_, Message> {
        let header = widgets::app_header(self);
        let nav = widgets::sidebar(self.screen);
        let settings_screen = self.screen == Screen::Settings;

        // Sidebar is a fixed width. The remaining pane flexes with the window
        // (rev 3). Header chips live in that pane. Column counts come from
        // `responsive`. Settings keeps the header and scrolls the form in the
        // leftover height. Other screens scroll the whole pane.
        let mut content = column![header]
            .spacing(16)
            .width(Fill)
            .height(if settings_screen {
                Fill
            } else {
                Length::Shrink
            });
        if let Some(t) = &self.toast {
            content = content.push(widgets::toast_bar(t, self.toast_action.clone()));
        }
        if self.restart_bar_visible() {
            content = content.push(restart_prompt::bar());
        }
        content = content.push(
            responsive(|size| match self.screen {
                Screen::Overview => crate::overview::view(self, size.width),
                Screen::Games => crate::games::view(self, size.width),
                Screen::Heroes => crate::heroes::view(self, size.width),
                Screen::Maps => crate::maps::view(self, size.width),
                Screen::Seasons => crate::seasons::view(self, size.width),
                Screen::Settings => crate::settings::view(self, size.width),
            })
            .width(Fill)
            .height(if settings_screen {
                Fill
            } else {
                Length::Shrink
            }),
        );

        let main = container(content)
            .width(Fill)
            .height(if settings_screen {
                Fill
            } else {
                Length::Shrink
            })
            .padding(Padding {
                top: PAGE_PAD_Y,
                bottom: PAGE_PAD_Y,
                left: PAGE_PAD_X,
                right: PAGE_PAD_X,
            });

        let page_body: Element<'_, Message> = if settings_screen {
            container(main).width(Fill).height(Fill).into()
        } else {
            iced::widget::scrollable(main)
                .width(Fill)
                .height(Fill)
                .into()
        };

        let chrome = row![
            container(nav)
                .width(SIDEBAR_WIDTH)
                .height(Fill)
                .padding(Padding {
                    top: PAGE_PAD_Y,
                    bottom: PAGE_PAD_Y,
                    left: PAGE_PAD_X,
                    right: 8.0,
                }),
            page_body,
        ]
        .spacing(0)
        .width(Fill)
        .height(Fill);

        let page = container(chrome)
            .width(Fill)
            .height(Fill)
            .style(theme::page_background);

        let with_notes = if let Some(dialog) = &self.notes_dialog {
            // `notes::dialog` includes the backdrop and is opaque, so a press
            // on the scrim or the card does not reach Update now underneath.
            stack![
                page,
                notes::dialog(&dialog.subtitle, &dialog.sections, self.dialog_notes_ui())
            ]
            .width(Fill)
            .height(Fill)
            .into()
        } else {
            page.into()
        };
        let with_guide = if self.setup.open {
            stack![with_notes, crate::setup_guide::view(&self.setup)]
                .width(Fill)
                .height(Fill)
                .into()
        } else {
            with_notes
        };
        let with_uninstall = if let Some(dialog) = &self.uninstall_dialog {
            stack![
                with_guide,
                crate::uninstall::view(dialog, self.uninstall_busy)
            ]
            .width(Fill)
            .height(Fill)
            .into()
        } else {
            with_guide
        };
        if let Some(report) = &self.report {
            stack![with_uninstall, crate::report::view(report)]
                .width(Fill)
                .height(Fill)
                .into()
        } else {
            with_uninstall
        }
    }

    pub fn update_notes_ui(&self) -> notes::NotesUi<'_> {
        notes::NotesUi {
            surface: notes::NotesSurface::Update,
            installed: self.installed_version.as_deref(),
            offered: self.update.as_ref().map(|info| info.latest.as_str()),
            open_details: &self.update_open_details,
            show_older: self.update_show_older,
        }
    }

    fn dialog_notes_ui(&self) -> notes::NotesUi<'_> {
        notes::NotesUi {
            surface: notes::NotesSurface::Dialog,
            installed: self.installed_version.as_deref(),
            offered: self.update.as_ref().map(|info| info.latest.as_str()),
            open_details: &self.dialog_open_details,
            show_older: self.dialog_show_older,
        }
    }

    fn health_now(&self) -> String {
        health_with_ocr_notice(
            health_status_for(
                &self.data_dir,
                &self.games,
                self.saved_config
                    .sync
                    .as_ref()
                    .map(|s| (s.server_url.as_str(), s.token.as_str())),
            ),
            self.ocr_switch.health_notice.as_deref(),
        )
    }

    /// A click on "Use classic OCR (Tesseract)". Turning it on is at once.
    /// Turning it off first looks at the reader packs in the tracker's data
    /// dir, off the UI thread: the box changes when that look comes back
    /// ([`Message::ReaderPacksChecked`]). With a pack missing, or a tracker
    /// version that cannot yet, the box stays ticked and the reason is shown
    /// next to it.
    fn toggle_classic_ocr(&mut self, want_classic: bool) -> Task<Message> {
        if self.fixture.is_some() {
            // Demo mode saves nothing and has no packs to look at.
            self.settings.classic_ocr = want_classic;
            return Task::none();
        }
        if want_classic {
            self.settings.classic_ocr = true;
            self.ocr_switch.refusal = None;
            return Task::none();
        }
        check_reader_packs(self.data_dir.clone(), true)
    }
}

/// Look at the reader packs in `data_dir` off the UI thread. `turn_off`
/// carries a waiting click on the classic OCR switch.
fn check_reader_packs(data_dir: PathBuf, turn_off: bool) -> Task<Message> {
    Task::perform(
        async move { stat_tracker::ocr_switch::PackReadiness::on_disk(&data_dir) },
        move |packs| Message::ReaderPacksChecked { packs, turn_off },
    )
}

const HEALTH_READY: &str = "Ready";
const HEALTH_WAITING: &str = "Waiting for a capture";

/// Tracker health with the tracker's classic OCR notice: it runs with OCR
/// off and holds reads for a missing reader pack, or a read was stopped. A
/// refused off is not in it (`settings::daemon_health_notice`). The notice
/// takes the place of the two idle lines and follows a sync problem.
fn health_with_ocr_notice(base: String, notice: Option<&str>) -> String {
    match notice {
        None => base,
        Some(notice) if base == HEALTH_READY || base == HEALTH_WAITING => notice.to_string(),
        Some(notice) => format!("{base} {notice}"),
    }
}

fn fixture_clock(fixture: Option<FixtureKind>, games: &[Game]) -> DateTime<Utc> {
    if fixture == Some(FixtureKind::Sample) {
        games.first().map(|g| g.played_at).unwrap_or_else(Utc::now)
    } else {
        Utc::now()
    }
}

fn live_status_for(games: &[Game]) -> String {
    if let Some(g) = games.first() {
        format!("Last game {}", crate::model::local_hm(g.played_at))
    } else {
        "Waiting for a capture".into()
    }
}

fn report_dialog_escape(
    event: iced::Event,
    _status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    notes::escape_closes_notes(&key).then_some(Message::Report(crate::report::ReportMessage::Close))
}

fn uninstall_dialog_escape(
    event: iced::Event,
    _status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    notes::escape_closes_notes(&key).then_some(Message::DismissUninstall)
}

async fn uninstall_from_app(
    req: crate::uninstall::UninstallRequest,
) -> Result<crate::uninstall::UninstallReport, String> {
    match daemon::stop_daemon(&req.data_dir).await {
        Ok(()) => {}
        Err(err) if err == "Tracker is not running" => {}
        Err(_) => {
            return Err("Could not stop the tracker. Nothing was removed.".into());
        }
    }
    if daemon::daemon_running(&req.data_dir).is_some() {
        return Err("The tracker is still running. Nothing was removed.".into());
    }
    crate::uninstall::apply(&req)
}

fn notes_dialog_escape(
    event: iced::Event,
    _status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    foreground_key_message(false, true, &key)
}

fn setup_guide_keys(
    event: iced::Event,
    status: iced::event::Status,
    _window: window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    // The guide is open, so Escape must not dismiss notes underneath it.
    // A text field that already took the key keeps it.
    guide_shortcut(status, true, true, &key)
}

/// Escape and Enter drive the guide unless a text input already handled them.
pub fn guide_shortcut(
    status: iced::event::Status,
    guide_open: bool,
    notes_open: bool,
    key: &iced::keyboard::Key,
) -> Option<Message> {
    if guide_open && status == iced::event::Status::Captured {
        return None;
    }
    foreground_key_message(guide_open, notes_open, key)
}

/// While the setup guide is open it owns Escape and Enter.
pub fn foreground_key_message(
    guide_open: bool,
    notes_open: bool,
    key: &iced::keyboard::Key,
) -> Option<Message> {
    if guide_open {
        return crate::setup_guide::guide_key_action(key).map(Message::Setup);
    }
    if notes_open && notes::escape_closes_notes(key) {
        Some(Message::DismissNotes)
    } else {
        None
    }
}

fn notes_dialog_for(
    data_dir: &std::path::Path,
    installed: Option<&str>,
    seen_memory: Option<&str>,
) -> LaunchNotes {
    let from_file = seasons::load_last_seen_version(data_dir);
    let effective = seen_memory.or(from_file.as_deref());
    let change = notes::classify_seen(effective, installed);
    if change == notes::SeenChange::Unchanged {
        return LaunchNotes::Quiet { persist: None };
    }
    let Some(installed) = installed else {
        return LaunchNotes::Quiet { persist: None };
    };
    let sections = notes::notes_for_change(notes::BUNDLED_CHANGELOG, change, effective, installed);
    if sections.is_empty() {
        return LaunchNotes::Quiet {
            persist: Some(installed.to_string()),
        };
    }
    LaunchNotes::Show(NotesDialog {
        subtitle: notes::launch_subtitle(change).to_string(),
        sections: notes::render(sections),
        remember: Some(installed.to_string()),
    })
}

/// Mark this version seen for the rest of the process, then try the file.
/// A write error is logged once, and the in-memory flag still suppresses
/// the dialog if launch notes are decided again this session.
fn remember_installed(seen: &mut NotesSeen, data_dir: &std::path::Path, version: &str) {
    seen.version = Some(version.to_string());
    if let Err(error) = seasons::save_last_seen_version(data_dir, version) {
        log_seen_write_error(seen, &error);
    }
}

fn log_seen_write_error(seen: &mut NotesSeen, error: &dyn std::fmt::Display) {
    if seen.write_error_logged {
        return;
    }
    tracing::warn!(error = %error, "failed to remember installed version");
    seen.write_error_logged = true;
}

fn health_status_for(
    data_dir: &std::path::Path,
    games: &[Game],
    sync: Option<(&str, &str)>,
) -> String {
    if let Some((url, token)) = sync {
        let url = url.trim();
        if !url.is_empty()
            && let Err(e) = stat_tracker::config::validate_sync_server_url(url)
        {
            return format!("Sync paused: {}", e.message());
        }
        if !url.is_empty() && stat_tracker::sync::auth_pause_matches(data_dir, url, token) {
            return stat_tracker::sync::SYNC_TOKEN_REJECTED_STATUS.to_string();
        }
    }
    if data_dir.join("live_snapshot.json").exists() || !games.is_empty() {
        HEALTH_READY.into()
    } else {
        HEALTH_WAITING.into()
    }
}

#[cfg(test)]
mod tests {
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
    fn save_toasts_have_no_em_or_en_dash() {
        for copy in [
            super::SETTINGS_SAVED_RESTART,
            super::SETTINGS_SAVE_REFUSED,
            super::SETTINGS_RELOADED_FROM_DISK,
            "Settings saved",
        ] {
            assert!(!copy.contains('—'), "{copy}");
            assert!(!copy.contains('–'), "{copy}");
        }
        assert_eq!(
            super::toast_if_config_unreadable(true),
            Some(super::SETTINGS_SAVE_REFUSED)
        );
        assert!(super::SETTINGS_SAVE_REFUSED.contains("not saved"));
        assert!(super::toast_if_config_unreadable(false).is_none());
        assert!(super::SETTINGS_SAVED_RESTART.contains("Restart the tracker"));
    }

    #[test]
    fn alpha_updates_save_rechecks_and_needs_no_tracker_restart() {
        use crate::settings::{SettingsForm, SettingsToggle};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let raw = "\
# keep this comment
data_dir = \"/tmp/sst-alpha-save\"
player_name = \"the streamer\"
";
        std::fs::write(&path, raw).expect("seed");
        let snapshot = Config::parse_file_contents(raw).expect("parse");
        let mut form = SettingsForm::from_config_and_shadow(
            &snapshot,
            Config::shadow_control(snapshot.shadow_recognizer, None),
        );
        form.set_toggle(SettingsToggle::PrereleaseUpdates, true);
        let saved = super::save_settings_form(&path, false, &snapshot, &form, || Ok(()))
            .expect("save writes");
        let super::SettingsFileSave::Saved(written) = saved else {
            panic!("expected a write, got {saved:?}");
        };
        let on_disk = std::fs::read_to_string(&path).expect("file");
        assert!(on_disk.contains("# keep this comment"), "{on_disk}");
        assert!(
            on_disk.contains("update_channel = \"prerelease\""),
            "{on_disk}"
        );
        assert!(
            Config::parse_file_contents(&on_disk)
                .expect("reparse")
                .wants_prerelease_updates(),
            "the next update check reads prerelease from disk"
        );
        assert!(super::update_channel_switched(&snapshot, &written));
        assert!(super::only_update_channel_changed(&snapshot, &written));
        assert_eq!(super::toast_after_successful_save(false), "Settings saved");

        let mut other = written.clone();
        other.debug_ocr = !other.debug_ocr;
        assert!(!super::only_update_channel_changed(&written, &other));
        assert!(!super::update_channel_switched(&written, &other));
        assert!(!super::only_update_channel_changed(&written, &written));

        let mut respelled = written.clone();
        respelled.update_channel = "PRERELEASE".into();
        assert!(!super::update_channel_switched(&written, &respelled));
    }

    #[test]
    fn save_reloads_a_fixed_or_hand_edited_config_instead_of_overwriting_it() {
        use crate::settings::{SettingsForm, SettingsToggle};

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let broken = "\
this is not toml
token = \"secret-token-must-stay\"
";
        std::fs::write(&path, broken).expect("seed broken file");

        // Startup: the file did not parse, so the form is defaults.
        let mut snapshot = Config::default();
        let mut form = SettingsForm::from_config_and_shadow(
            &snapshot,
            Config::shadow_control(snapshot.shadow_recognizer, None),
        );
        let mut form_built_unreadable = true;
        assert!(form.player_name.is_empty());
        assert!(form.sync_token.is_empty());

        let mut checked_before_reload = false;
        let refused =
            super::save_settings_form(&path, form_built_unreadable, &snapshot, &form, || {
                checked_before_reload = true;
                Ok(())
            })
            .expect("refuse is not a write error");
        assert!(!checked_before_reload);
        assert_eq!(refused, super::SettingsFileSave::Unreadable);
        assert_eq!(std::fs::read_to_string(&path).expect("unchanged"), broken);

        let fixed = "\
# scoreboard name
data_dir = \"/tmp/sst-fixed-on-disk\"
player_name = \"the streamer\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = \"https://crew.example\"
token = \"secret-token-must-stay\"
";
        std::fs::write(&path, fixed).expect("fix the file");

        // (a) First Save reloads the fixed file and does not write the defaults.
        let mut checked_before_reload = false;
        let reloaded =
            super::save_settings_form(&path, form_built_unreadable, &snapshot, &form, || {
                checked_before_reload = true;
                Err("sync check must not run before a reload".into())
            })
            .expect("reload is not a write error");
        assert!(!checked_before_reload);
        let super::SettingsFileSave::Reloaded(fresh) = reloaded else {
            panic!("first save after the fix must reload, got {reloaded:?}");
        };
        assert_eq!(std::fs::read_to_string(&path).expect("file"), fixed);
        form = super::form_after_disk_reload(
            &form,
            &fresh,
            Config::shadow_control(fresh.shadow_recognizer, None),
        );
        snapshot = fresh;
        form_built_unreadable = false;
        assert_eq!(form.player_name, "the streamer");
        assert_eq!(form.sync_token, "secret-token-must-stay");
        assert_eq!(form.sync_url, "https://crew.example");
        assert!(!snapshot.shadow_recognizer);

        // Second Save, after the member toggles the reader, writes only that key.
        form.set_toggle(SettingsToggle::ShadowRecognizer, true);
        let saved =
            super::save_settings_form(&path, form_built_unreadable, &snapshot, &form, || Ok(()))
                .expect("second save writes");
        let super::SettingsFileSave::Saved(written) = saved else {
            panic!("second save must write the toggle, got {saved:?}");
        };
        assert!(written.shadow_recognizer);
        let text = std::fs::read_to_string(&path).expect("saved");
        let loaded = Config::read_at(&path).expect("load after save");
        assert!(loaded.shadow_recognizer);
        assert_eq!(
            loaded.player_name.as_deref(),
            Some("the streamer"),
            "the reloaded name must survive the toggle write"
        );
        assert_eq!(
            loaded.sync.as_ref().map(|sync| sync.token.as_str()),
            Some("secret-token-must-stay")
        );
        let header = text.find("[auto_detect]").expect("tables");
        assert_eq!(
            &text[header..],
            &fixed[fixed.find("[auto_detect]").expect("tables")..]
        );
        assert_eq!(text[..header].matches("shadow_recognizer").count(), 1);
        assert!(text[..header].contains("shadow_recognizer = true"));
        assert!(text.contains("# scoreboard name"));
        assert!(text.contains("player_name = \"the streamer\""));

        // (b) A hand edit while the form is open is reloaded, not overwritten.
        let hand_edited = fixed.replace(
            "player_name = \"the streamer\"",
            "player_name = \"the other account\"",
        );
        let hand_edited = hand_edited.replace(
            "token = \"secret-token-must-stay\"",
            "token = \"token-edited-on-disk\"",
        );
        std::fs::write(&path, &hand_edited).expect("hand edit");
        form.set_toggle(SettingsToggle::ShadowRecognizer, false);
        let outcome =
            super::save_settings_form(&path, form_built_unreadable, &snapshot, &form, || {
                Err("must not write over a hand edit".into())
            })
            .expect("hand edit reloads");
        let super::SettingsFileSave::Reloaded(fresh) = outcome else {
            panic!("a hand edit must reload, got {outcome:?}");
        };
        assert_eq!(
            std::fs::read_to_string(&path).expect("hand edit kept"),
            hand_edited
        );
        form = super::form_after_disk_reload(
            &form,
            &fresh,
            Config::shadow_control(fresh.shadow_recognizer, None),
        );
        assert_eq!(form.player_name, "the other account");
        assert_eq!(form.sync_token, "token-edited-on-disk");
        assert!(!form.sync_token.is_empty());

        // The startup flag reloads even when the parsed values already match.
        let defaults_path = dir.path().join("defaults.toml");
        let defaults = Config::default();
        let defaults_text = toml::to_string_pretty(&defaults).expect("pretty");
        std::fs::write(&defaults_path, &defaults_text).expect("write defaults");
        let round_trip = Config::read_at(&defaults_path).expect("read defaults");
        assert_eq!(round_trip, defaults);
        let defaults_form = SettingsForm::from_config_and_shadow(
            &defaults,
            Config::shadow_control(defaults.shadow_recognizer, None),
        );
        let mut dirty = defaults_form;
        dirty.set_toggle(SettingsToggle::ShadowRecognizer, true);
        let flagged = super::save_settings_form(&defaults_path, true, &defaults, &dirty, || {
            Err("must not write while the form was built unreadable".into())
        })
        .expect("flagged reload");
        assert!(matches!(flagged, super::SettingsFileSave::Reloaded(_)));
        assert_eq!(
            std::fs::read_to_string(&defaults_path).expect("defaults"),
            defaults_text
        );
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
        assert_ne!(super::toast_after_successful_save(true), refused);
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
        let cli = crate::cli::Cli {
            data_dir: tmp.path().to_path_buf(),
            fixture: Some(crate::cli::FixtureKind::Empty),
            seasons_url: None,
            companion: false,
            help: false,
            version: false,
            replace: false,
        };
        let (mut app, _open) = super::TrackerApp::new(cli);
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
            format!("{paused} {notice}")
        );
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
        let rejected =
            super::health_status_for(&dir, &[], Some(("https://crew.example", "old-token")));
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
        let _ = std::fs::remove_dir_all(&dir);
    }
}
