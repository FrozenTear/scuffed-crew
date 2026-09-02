use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use iced::widget::{column, container, responsive, row};
use iced::{Element, Fill, Length, Padding, Subscription, Task};

use crate::aggregate::GameFilter;
use crate::cli::{Cli, FixtureKind};
use crate::model::{EditField, EditForm, Game, Outcome, Role, RoleFilter, Screen, SeasonSel};
use crate::seasons::{self, SeasonCache};
use crate::snapshot::{self, games_from_snapshot};
use crate::theme::{self, PAGE_PAD_X, PAGE_PAD_Y, SIDEBAR_WIDTH};
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
    snapshot_mtime: Option<SystemTime>,
    seasons_url: Option<String>,
    last_seasons_attempt: Option<DateTime<Utc>>,
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
            snapshot_mtime,
            seasons_url,
            last_seasons_attempt,
        };
        (app, fetch)
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
            Message::Tick => {
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
                    return Task::perform(seasons::fetch_seasons(url), Message::SeasonsFetched);
                }
                Task::none()
            }
            Message::Navigate(screen) => {
                self.screen = screen;
                if screen != Screen::Games {
                    self.expanded = None;
                    self.editing = false;
                    self.confirm_delete = None;
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
        }
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
