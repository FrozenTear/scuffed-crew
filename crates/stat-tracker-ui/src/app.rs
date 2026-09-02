use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use iced::widget::image::Handle;
use iced::widget::{column, container, responsive, row};
use iced::{Element, Fill, Length, Padding, Subscription, Task, window};
use stat_tracker::capture::CaptureBackend;
use stat_tracker::config::Config;

use crate::aggregate::GameFilter;
use crate::capture::{self, PreviewShot};
use crate::cli::{Cli, FixtureKind};
use crate::daemon::{self, DaemonVerb, DaemonView};
use crate::model::{EditField, EditForm, Game, Outcome, Role, RoleFilter, Screen, SeasonSel};
use crate::seasons::{self, SeasonCache};
use crate::settings::{self, SettingsField, SettingsForm, SettingsToggle};
use crate::snapshot::{self, games_from_snapshot};
use crate::theme::{self, PAGE_PAD_X, PAGE_PAD_Y, SIDEBAR_WIDTH};
use crate::tray::{self, TrayAction, TrayHandle};
use crate::update::{self, UpdateInfo};
use crate::widgets;

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
    ResolveSegment {
        session_id: String,
        segment: u32,
        confirm: bool,
    },
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
    Vacuum,
    VacuumReady(Result<String, String>),
    ConfirmClear,
    ClearData,
    ClearReady(Result<String, String>),
    UpdateChecked(Option<UpdateInfo>),
    OpenUpdate(String),
    WindowReady(Option<window::Id>),
    Tray(TrayAction),
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
    pub settings: SettingsForm,
    pub saved_config: Config,
    pub daemon: DaemonView,
    pub daemon_busy: bool,
    pub outputs: Vec<String>,
    pub backend: Option<CaptureBackend>,
    pub capturing: bool,
    pub preview: Option<(Handle, String)>,
    pub preview_error: Option<String>,
    pub update: Option<UpdateInfo>,
    pub confirm_clear: bool,
    pub tessdata_busy: bool,
    pub tessdata_installed: bool,
    pub vacuum_busy: bool,
    snapshot_mtime: Option<SystemTime>,
    seasons_url: Option<String>,
    last_seasons_attempt: Option<DateTime<Utc>>,
    window_id: Option<window::Id>,
    tick_count: u64,
    tray: Option<TrayHandle>,
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
        let health_status = health_status_for(&cli.data_dir, &games);
        let saved_config = Config::load().unwrap_or_default();
        let settings = SettingsForm::from_config(&saved_config);
        let daemon = DaemonView {
            pid: daemon::daemon_running(&cli.data_dir),
            service_installed: daemon::service_file_installed(),
            autostart: false,
        };
        let live = cli.fixture.is_none();
        let app = Self {
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
            settings,
            saved_config,
            daemon,
            daemon_busy: false,
            outputs: Vec::new(),
            backend: None,
            capturing: false,
            preview: None,
            preview_error: None,
            update: None,
            confirm_clear: false,
            tessdata_busy: false,
            tessdata_installed: capture::tessdata_installed(),
            vacuum_busy: false,
            snapshot_mtime,
            seasons_url,
            last_seasons_attempt,
            window_id: None,
            tick_count: 0,
            tray: tray::try_create(),
        };

        let mut tasks = vec![fetch, window::oldest().map(Message::WindowReady)];
        if live {
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

    pub fn title(&self) -> String {
        "Scuffed Tracker".into()
    }

    pub fn subscription(&self) -> Subscription<Message> {
        iced::time::every(Duration::from_secs(1)).map(|_| Message::Tick)
    }

    pub fn season_window(&self) -> Option<crate::aggregate::SeasonWindow> {
        crate::seasons::window_for(&self.season, &self.seasons.seasons)
    }

    pub fn header_filter(&self) -> GameFilter {
        GameFilter::from_header(self.season_window(), self.roles)
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
                        self.health_status = health_status_for(&self.data_dir, &self.games);
                    }
                    Err(e) => {
                        tracing::info!(error = %e, "seasons fetch failed; using cache");
                        self.health_status = health_status_for(&self.data_dir, &self.games);
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
                self.toast = Some(
                    match crate::commands::set_outcome(&self.data_dir, &session_id, outcome) {
                        Ok(()) => format!("Outcome set to {}", outcome.store_label()),
                        Err(e) => format!("Could not save outcome: {e}"),
                    },
                );
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
                self.toast = Some(
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
                if let Some(sid) = &self.expanded
                    && let Some(g) = self.games.iter().find(|g| &g.session_id == sid)
                {
                    self.toast = Some(
                        match crate::commands::save_edit(&self.data_dir, g, &self.edit) {
                            Ok(()) => "Stats corrected".into(),
                            Err(e) => e,
                        },
                    );
                }
                self.editing = false;
                Task::none()
            }
            Message::ResolveSegment {
                session_id,
                segment,
                confirm,
            } => {
                self.toast = Some(
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
            Message::SettingsToggle(toggle, value) => {
                self.settings.set_toggle(toggle, value);
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
                let config = self.settings.to_config(&self.saved_config);
                match settings::save_config(&config) {
                    Ok(()) => {
                        let daemon_up = daemon::is_daemon_running(&self.data_dir);
                        self.saved_config = config;
                        self.settings = SettingsForm::from_config(&self.saved_config);
                        self.toast = Some(if daemon_up {
                            "Settings saved — restart the tracker for changes to take effect".into()
                        } else {
                            "Settings saved".into()
                        });
                    }
                    Err(e) => {
                        self.toast = Some(format!("Could not save settings: {e}"));
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
                self.toast = Some(match result {
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
                self.toast = Some(match result {
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
                self.toast = Some(match result {
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
                self.toast = Some(match result {
                    Ok(m) => m,
                    Err(e) => e,
                });
                if ok {
                    self.games.clear();
                    self.health_status = health_status_for(&self.data_dir, &self.games);
                }
                Task::none()
            }
            Message::UpdateChecked(info) => {
                self.update = info;
                Task::none()
            }
            Message::OpenUpdate(url) => {
                update::open_release_page(&url);
                Task::none()
            }
            Message::WindowReady(id) => {
                self.window_id = id;
                Task::none()
            }
            Message::Tray(action) => self.apply_tray(action),
        }
    }

    fn on_tick(&mut self) -> Task<Message> {
        if self.fixture.is_none() {
            let mtime = snapshot::snapshot_mtime(&self.data_dir);
            if mtime != self.snapshot_mtime {
                self.snapshot_mtime = mtime;
                let snap = snapshot::load_snapshot(&self.data_dir);
                self.games = games_from_snapshot(&snap);
                self.live_status = live_status_for(&self.games);
                self.health_status = health_status_for(&self.data_dir, &self.games);
                self.clock = Utc::now();
            }
            self.daemon = daemon::refresh_view(&self.data_dir, &self.daemon);
        }

        tray::pump_gtk();
        if let Some(handle) = &self.tray
            && let Some(action) = tray::poll(handle)
        {
            return self.apply_tray(action);
        }

        let mut tasks = Vec::new();
        if self.window_id.is_none() {
            tasks.push(window::oldest().map(Message::WindowReady));
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
            TrayAction::Quit => iced::exit(),
            TrayAction::Show => self.show_window(),
            TrayAction::Hide => self.hide_window(),
        }
    }

    fn show_window(&self) -> Task<Message> {
        let Some(id) = self.window_id else {
            return window::oldest().map(Message::WindowReady);
        };
        Task::batch([window::minimize(id, false), window::gain_focus(id)])
    }

    fn hide_window(&self) -> Task<Message> {
        let Some(id) = self.window_id else {
            return Task::none();
        };
        window::minimize(id, true)
    }

    pub fn view(&self) -> Element<'_, Message> {
        let header = widgets::app_header(self);
        let nav = widgets::sidebar(self.screen);

        // Sidebar is a fixed width. The remaining pane flexes with the window
        // (rev 3). Header chips live in that pane — no left-pinned 1400 cap
        // and no empty right band. Column counts come from `responsive`.
        let mut content = column![header].spacing(16).width(Fill);
        if let Some(t) = &self.toast {
            content = content.push(widgets::toast_bar(t));
        }
        content = content.push(
            responsive(|size| match self.screen {
                Screen::Overview => crate::overview::view(self, size.width),
                Screen::Games => crate::games::view(self, size.width),
                Screen::Heroes => crate::heroes::view(self, size.width),
                Screen::Maps => crate::maps::view(self),
                Screen::Seasons => crate::seasons::view(self),
                Screen::Settings => crate::settings::view(self),
            })
            .width(Fill)
            .height(Length::Shrink),
        );

        let main = container(content).width(Fill).padding(Padding {
            top: PAGE_PAD_Y,
            bottom: PAGE_PAD_Y,
            left: PAGE_PAD_X,
            right: PAGE_PAD_X,
        });

        let chrome = row![
            container(nav).width(SIDEBAR_WIDTH).padding(Padding {
                top: PAGE_PAD_Y,
                bottom: PAGE_PAD_Y,
                left: PAGE_PAD_X,
                right: 8.0,
            }),
            iced::widget::scrollable(main).width(Fill).height(Fill),
        ]
        .spacing(0)
        .width(Fill)
        .height(Fill);

        container(chrome)
            .width(Fill)
            .height(Fill)
            .style(theme::page_background)
            .into()
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
        format!("Last game {}", g.played_at.format("%H:%M"))
    } else {
        "Waiting for a capture".into()
    }
}

fn health_status_for(data_dir: &std::path::Path, games: &[Game]) -> String {
    if data_dir.join("live_snapshot.json").exists() || !games.is_empty() {
        "Ready".into()
    } else {
        "Waiting for a capture".into()
    }
}
