use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use iced::widget::{column, container, row, space};
use iced::{Alignment, Element, Fill, Padding, Subscription, Task};

use crate::aggregate::GameFilter;
use crate::cli::{Cli, FixtureKind};
use crate::model::{EditField, EditForm, Game, Outcome, Role, RoleFilter, Screen, SeasonSel};
use crate::seasons::{self, SeasonCache};
use crate::snapshot::{self, games_from_snapshot};
use crate::theme::{self, CONTENT_MAX, PAGE_PAD_X, PAGE_PAD_Y, SIDEBAR_WIDTH};
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
        let season = seasons::default_selection(&seasons.seasons);
        let clock = fixture_clock(cli.fixture, &games);
        let fetch =
            if seasons::should_fetch_seasons(cli.fixture.is_some(), cli.seasons_url.as_deref()) {
                let url = cli
                    .seasons_url
                    .clone()
                    .expect("should_fetch_seasons requires a URL");
                Task::perform(seasons::fetch_seasons(url), Message::SeasonsFetched)
            } else {
                Task::none()
            };

        let snapshot_mtime = snapshot::snapshot_mtime(&cli.data_dir);
        let live_status = live_status_for(&games, cli.fixture);
        let health_status = health_status_for(&cli.data_dir, cli.fixture, &seasons);
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
                        self.live_status = live_status_for(&self.games, self.fixture);
                        self.clock = Utc::now();
                    }
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
                        }
                        self.health_status =
                            health_status_for(&self.data_dir, self.fixture, &self.seasons);
                    }
                    Err(e) => {
                        tracing::info!(error = %e, "seasons fetch failed; using cache");
                        self.health_status =
                            health_status_for(&self.data_dir, self.fixture, &self.seasons);
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
                        Ok(()) => format!("Queued SetOutcome → {}", outcome.store_label()),
                        Err(e) => format!("Command failed: {e}"),
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
                        Ok(()) => "Queued DeleteSession".into(),
                        Err(e) => format!("Command failed: {e}"),
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
                            Ok(()) => "Queued EditMatch".into(),
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
                                "Queued ResolveSegment confirm".into()
                            } else {
                                "Queued ResolveSegment dismiss".into()
                            }
                        }
                        Err(e) => format!("Command failed: {e}"),
                    },
                );
                Task::none()
            }
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        let header = widgets::app_header(self);
        let nav = widgets::sidebar(self.screen);
        let body = match self.screen {
            Screen::Overview => crate::overview::view(self),
            Screen::Games => crate::games::view(self),
            Screen::Heroes => crate::heroes::view(self),
            Screen::Maps => crate::maps::view(self),
        };
        let toast = self.toast.as_ref().map(|t| widgets::toast_bar(t));

        let main = container(body)
            .width(Fill)
            .max_width(CONTENT_MAX)
            .padding(Padding {
                top: 0.0,
                bottom: PAGE_PAD_Y,
                left: PAGE_PAD_X,
                right: PAGE_PAD_X,
            });

        let mut stack = column![container(header).padding(Padding {
            top: PAGE_PAD_Y,
            bottom: 16.0,
            left: PAGE_PAD_X,
            right: PAGE_PAD_X,
        }),]
        .spacing(0)
        .width(Fill)
        .height(Fill);

        if let Some(t) = toast {
            stack = stack.push(container(t).padding(Padding {
                top: 0.0,
                bottom: 8.0,
                left: PAGE_PAD_X,
                right: PAGE_PAD_X,
            }));
        }

        stack = stack.push(
            row![
                container(nav).width(SIDEBAR_WIDTH).padding(Padding {
                    top: 0.0,
                    bottom: PAGE_PAD_Y,
                    left: PAGE_PAD_X,
                    right: 8.0,
                }),
                iced::widget::scrollable(container(
                    row![main, space().width(Fill)].align_y(Alignment::Start)
                ))
                .width(Fill)
                .height(Fill),
            ]
            .spacing(0)
            .width(Fill)
            .height(Fill),
        );

        container(stack)
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

fn live_status_for(games: &[Game], fixture: Option<FixtureKind>) -> String {
    if fixture.is_some() {
        return "Fixture · read-only snapshot".into();
    }
    if let Some(g) = games.first() {
        format!("Idle · last game {}", g.played_at.format("%H:%M"))
    } else {
        "Idle · waiting for snapshot".into()
    }
}

fn health_status_for(
    data_dir: &std::path::Path,
    fixture: Option<FixtureKind>,
    seasons: &SeasonCache,
) -> String {
    let source = if fixture.is_some() {
        "fixture snapshot"
    } else if data_dir.join("live_snapshot.json").exists() {
        "live_snapshot.json"
    } else {
        "no snapshot yet"
    };
    let seasons_note = if seasons.seasons.is_empty() {
        "seasons: none (picker hidden)"
    } else if seasons.from_network {
        "seasons: network"
    } else {
        "seasons: cache"
    };
    format!("{source} · {seasons_note} · commands → <data_dir>/commands/")
}
