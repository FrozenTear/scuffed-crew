use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use chrono::{DateTime, Utc};
use iced::widget::scrollable;
use iced::{Element, Subscription, Task};

use crate::cli::{Cli, FixtureKind};
use crate::model::{Game, SeasonSel};
use crate::seasons::{self, SeasonCache};
use crate::snapshot::{self, games_from_snapshot};

#[derive(Debug, Clone)]
pub enum Message {
    Tick,
    SelectSeason(SeasonSel),
    SeasonsFetched(Result<Vec<scuffed_types::Season>, String>),
}

pub struct TrackerApp {
    pub data_dir: PathBuf,
    pub fixture: Option<FixtureKind>,
    pub games: Vec<Game>,
    pub seasons: SeasonCache,
    pub season: SeasonSel,
    pub live_status: String,
    pub health_status: String,
    pub clock: DateTime<Utc>,
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
        let fetch = if let Some(url) = cli.seasons_url.clone() {
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
            clock,
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
            Message::SelectSeason(sel) => {
                self.season = sel;
                Task::none()
            }
            Message::SeasonsFetched(result) => {
                match result {
                    Ok(list) => {
                        if let Err(e) = seasons::write_cache(&self.data_dir, &list) {
                            tracing::warn!(error = %e, "failed to write seasons.json cache");
                        }
                        let keep = self.season.clone();
                        self.seasons = SeasonCache {
                            seasons: list,
                            fetched_at: Some(Utc::now()),
                            from_network: true,
                        };
                        if keep
                            .as_id()
                            .is_some_and(|id| !self.seasons.seasons.iter().any(|s| s.id == id))
                        {
                            self.season = seasons::default_selection(&self.seasons.seasons);
                        } else if matches!(keep, SeasonSel::AllTime)
                            && self.season.as_id().is_none()
                        {
                            // already all-time; leave it
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
        }
    }

    pub fn view(&self) -> Element<'_, Message> {
        scrollable(crate::overview::view(self))
            .width(iced::Fill)
            .height(iced::Fill)
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
        return "Fixture · read-only".into();
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
    format!("{source} · {seasons_note}")
}
