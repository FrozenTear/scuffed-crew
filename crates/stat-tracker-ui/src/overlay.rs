//! P3 companion overlay — design-doc §4.6.
//!
//! The main window stays on `iced::daemon` + tray (P4). The overlay is a
//! same-binary `--companion` process on `iced_layershell` (`Layer::Overlay`,
//! keyboard none, exclusive zone 0) so the game keeps input. Parent starts
//! and stops that child from [`overlay_visible`]. Manual hide is session-scoped
//! ([`OverlayHold`]): it sticks until the game ends, then the next launch
//! auto-shows. Esc is N/A (`KeyboardInteractivity::None`).

use std::path::{Path, PathBuf};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant, SystemTime};

use chrono::{DateTime, Utc};
use iced::widget::{column, container, row, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Shadow};
use scuffed_types::Season;
use stat_tracker::detect::game_running::GameProcessGate;

use crate::aggregate::{Record, aggregate};
use crate::cli::{Cli, FixtureKind};
use crate::model::{Game, Outcome, Role, SeasonSel, display_hero_name};
use crate::overview::tonight_games;
use crate::seasons;
use crate::snapshot::{self, games_from_snapshot};
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAD_INNER, SIZE_BODY,
    SIZE_LABEL, SIZE_META, SIZE_TITLE, STRIPE, TEXT, TEXT_2, TEXT_3,
};

/// Design §4.6 — width 360, margins 24, exclusive zone 0.
pub const OVERLAY_WIDTH: u32 = 360;
pub const OVERLAY_MARGIN: i32 = 24;
pub const OVERLAY_EXCLUSIVE_ZONE: i32 = 0;
pub const OVERLAY_NAMESPACE: &str = "scuffed-companion";

const LAST_GAME_HEIGHT: f32 = 196.0;
const MINI_HERO_HEIGHT: f32 = 52.0;
const RESPAWN_GRACE: Duration = Duration::from_secs(2);
const PROCESS_SESSION_KEY: &str = "process";
const FIXTURE_SESSION_KEY: &str = "fixture";

/// Manual hide is scoped to one live game. Auto-show resumes when that
/// process ends and the next one starts — we do not reopen mid-session.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum OverlayHold {
    #[default]
    Auto,
    Hidden(String),
}

impl OverlayHold {
    pub fn from_persisted(key: Option<String>) -> Self {
        match key.filter(|k| !k.is_empty()) {
            Some(k) => Self::Hidden(k),
            None => Self::Auto,
        }
    }

    pub fn persisted_key(&self) -> Option<&str> {
        match self {
            Self::Auto => None,
            Self::Hidden(k) => Some(k.as_str()),
        }
    }
}

/// After [`reconcile_hold`]: show only when a game is live and the user has
/// not hidden this session.
pub fn overlay_visible(hold: &OverlayHold, game_running: bool) -> bool {
    game_running && matches!(hold, OverlayHold::Auto)
}

/// Bind / release a session hide. `key` is [`live_session_key`].
///
/// - Game ended (`key` is `None`) → Auto (next launch shows).
/// - Same key, or `process` upgraded to a real `session_id` → stay Hidden.
/// - Different key → Auto (new game).
pub fn reconcile_hold(hold: OverlayHold, key: Option<&str>) -> OverlayHold {
    match (hold, key) {
        (OverlayHold::Auto, _) => OverlayHold::Auto,
        (OverlayHold::Hidden(_), None) => OverlayHold::Auto,
        (OverlayHold::Hidden(h), Some(c)) if h == c => OverlayHold::Hidden(h),
        (OverlayHold::Hidden(h), Some(c)) if h == PROCESS_SESSION_KEY => {
            OverlayHold::Hidden(c.to_string())
        }
        (OverlayHold::Hidden(_), Some(_)) => OverlayHold::Auto,
    }
}

/// Tray / main-window toggle. No-op when no game is live (nothing to hide).
/// Hide while live sticks until [`reconcile_hold`] sees the game end.
pub fn toggle_hold(hold: OverlayHold, key: Option<&str>) -> OverlayHold {
    let Some(k) = key else {
        return OverlayHold::Auto;
    };
    match hold {
        OverlayHold::Auto => OverlayHold::Hidden(k.to_string()),
        OverlayHold::Hidden(_) => OverlayHold::Auto,
    }
}

/// Identity of the live game, if any. `process` is used until `active_game.json`
/// has a session id so a hide-before-first-Tab stays hidden.
pub fn live_session_key(
    data_dir: &Path,
    process_names: &[String],
    fixture: bool,
) -> Option<String> {
    if fixture {
        return Some(FIXTURE_SESSION_KEY.into());
    }
    if let Some(id) = active_game_session_id(data_dir) {
        return Some(id);
    }
    if process_is_running(process_names) {
        return Some(PROCESS_SESSION_KEY.into());
    }
    None
}

pub fn active_game_session_id(data_dir: &Path) -> Option<String> {
    let bytes = std::fs::read(data_dir.join("active_game.json")).ok()?;
    let v: serde_json::Value = serde_json::from_slice(&bytes).ok()?;
    let id = v.get("session_id")?.as_str()?.trim();
    (!id.is_empty()).then(|| id.to_string())
}

/// Whether the daemon currently has an open game (`active_game.json`).
pub fn active_game_present(data_dir: &Path) -> bool {
    active_game_session_id(data_dir).is_some()
}

/// Process scan using the daemon's gate, but **empty names mean not running**.
/// (The daemon treats empty as "gate off / always capture"; the overlay must
/// not stay up all day when no process names are configured.)
pub fn process_is_running(names: &[String]) -> bool {
    if names.is_empty() {
        return false;
    }
    let mut gate = GameProcessGate::new(names);
    gate.is_running()
}

/// Fixture mode pretends the game is running so the toggle can open the
/// overlay for layout shots. Live mode uses `active_game.json` and, when
/// process names are set, the same `/proc` comm scan the daemon uses.
pub fn detect_game_running(data_dir: &Path, process_names: &[String], fixture: bool) -> bool {
    live_session_key(data_dir, process_names, fixture).is_some()
}

/// Layer-shell numbers the runner applies. Kept free of Wayland types so
/// unit tests do not need a compositor.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OverlayShellSpec {
    pub width: u32,
    pub height: u32,
    pub margin: i32,
    pub exclusive_zone: i32,
    pub layer_overlay: bool,
    pub keyboard_none: bool,
    pub events_transparent: bool,
    /// Capture output name from config; `None` = compositor active output.
    pub output: Option<String>,
}

pub fn overlay_shell_spec(height: u32, capture_output: Option<&str>) -> OverlayShellSpec {
    OverlayShellSpec {
        width: OVERLAY_WIDTH,
        height: height.max(200),
        margin: OVERLAY_MARGIN,
        exclusive_zone: OVERLAY_EXCLUSIVE_ZONE,
        layer_overlay: true,
        keyboard_none: true,
        events_transparent: true,
        output: capture_output
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(str::to_string),
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayLastGame {
    pub map_name: String,
    pub hero: String,
    pub role: Role,
    pub outcome: Outcome,
    pub elims: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
    pub played_at: DateTime<Utc>,
}

impl OverlayLastGame {
    fn from_game(game: &Game) -> Self {
        Self {
            map_name: game.map_name.clone(),
            hero: game.display_hero(),
            role: game.role,
            outcome: game.outcome,
            elims: game.elims,
            deaths: game.deaths,
            assists: game.assists,
            damage: game.damage,
            healing: game.healing,
            mitigation: game.mitigation,
            played_at: game.played_at,
        }
    }

    pub fn has_stat_line(&self) -> bool {
        self.elims + self.deaths + self.assists + self.damage + self.healing + self.mitigation > 0
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayHero {
    pub hero: String,
    pub role: Role,
    pub record: Record,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OverlayModel {
    pub season_label: String,
    pub record: Record,
    pub last_game: Option<OverlayLastGame>,
    pub tonight_outcomes: Vec<Outcome>,
    pub top_heroes: Vec<OverlayHero>,
    pub last_tab: String,
    pub sync_label: String,
    pub live: bool,
}

impl OverlayModel {
    pub fn win_rate_line(&self) -> String {
        format!(
            "{} · {:.0}% · {}",
            self.season_label,
            self.record.win_rate_pct(),
            self.record.wl_label()
        )
    }
}

pub fn build_model(
    games: &[Game],
    seasons: &[Season],
    season: &SeasonSel,
    clock: DateTime<Utc>,
    sync_on: bool,
    live: bool,
) -> OverlayModel {
    let window = seasons::window_for(season, seasons);
    let season_stats = aggregate(games, window, None);
    let tonight: Vec<&Game> = tonight_games(games, clock);
    let tonight_owned: Vec<Game> = tonight.iter().copied().cloned().collect();
    let tonight_stats = aggregate(&tonight_owned, None, None);

    let last_game = tonight
        .first()
        .copied()
        .or_else(|| games.first())
        .map(OverlayLastGame::from_game);

    let last_tab = last_game
        .as_ref()
        .map(|g| format!("Last Tab {}", g.played_at.format("%H:%M")))
        .unwrap_or_else(|| "No Tab yet".into());

    OverlayModel {
        season_label: season_label(season, seasons),
        record: season_stats.record,
        last_game,
        tonight_outcomes: tonight.iter().map(|g| g.outcome).collect(),
        top_heroes: tonight_stats
            .heroes
            .into_iter()
            .take(3)
            .map(|h| OverlayHero {
                hero: display_hero_name(&h.hero),
                role: h.role,
                record: h.record,
            })
            .collect(),
        last_tab,
        sync_label: if sync_on {
            "Sync on".into()
        } else {
            "Sync off".into()
        },
        live,
    }
}

fn season_label(sel: &SeasonSel, seasons: &[Season]) -> String {
    match sel {
        SeasonSel::AllTime => "All time".into(),
        SeasonSel::Season(id) => seasons
            .iter()
            .find(|s| &s.id == id)
            .map(|s| s.name.clone())
            .unwrap_or_else(|| "Season".into()),
    }
}

/// Height of the layer surface. Layer-shell needs an explicit size; this
/// tracks the card stack so the panel is "height to content".
pub fn content_height(model: &OverlayModel) -> u32 {
    let mut h: u32 = 16 + 48 + 12;
    h += if model.last_game.is_some() { 196 } else { 56 };
    h += 12 + 28;
    h += 12 + 18;
    if model.top_heroes.is_empty() {
        h += 36;
    } else {
        h += model.top_heroes.len() as u32 * 56;
    }
    h += 12 + 36 + 16;
    h.clamp(200, 900)
}

pub fn companion_copy(showing: bool, game_running: bool) -> &'static str {
    if showing {
        "Companion showing"
    } else if game_running {
        "Companion hidden"
    } else {
        "Companion waiting for game"
    }
}

// ── parent spawn ─────────────────────────────────────────────────────────

pub struct CompanionChild {
    child: Child,
    spawned_at: Instant,
}

impl Drop for CompanionChild {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

fn companion_pid_path(data_dir: &Path) -> PathBuf {
    data_dir.join("companion.pid")
}

fn cmdline_has_companion(pid: u32) -> bool {
    let path = format!("/proc/{pid}/cmdline");
    let Ok(raw) = std::fs::read(&path) else {
        return false;
    };
    raw.split(|b| *b == 0).any(|arg| arg == b"--companion")
}

fn stop_stale_companion(data_dir: &Path) {
    let path = companion_pid_path(data_dir);
    let Ok(text) = std::fs::read_to_string(&path) else {
        return;
    };
    let Ok(pid) = text.trim().parse::<u32>() else {
        let _ = std::fs::remove_file(&path);
        return;
    };
    if cmdline_has_companion(pid) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }
    let _ = std::fs::remove_file(&path);
}

pub fn spawn_companion(
    data_dir: &Path,
    fixture: Option<FixtureKind>,
) -> anyhow::Result<CompanionChild> {
    stop_stale_companion(data_dir);
    let exe = std::env::current_exe()?;
    let mut cmd = Command::new(exe);
    cmd.arg("--companion")
        .arg("--data-dir")
        .arg(data_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit());
    if let Some(kind) = fixture {
        cmd.arg("--fixture").arg(match kind {
            FixtureKind::Empty => "empty",
            FixtureKind::Sample => "sample",
        });
    }
    let child = cmd.spawn()?;
    let _ = std::fs::write(companion_pid_path(data_dir), child.id().to_string());
    Ok(CompanionChild {
        child,
        spawned_at: Instant::now(),
    })
}

pub fn stop_companion(child: &mut Option<CompanionChild>, data_dir: &Path) {
    if let Some(mut c) = child.take() {
        let _ = c.child.kill();
        let _ = c.child.wait();
    }
    let _ = std::fs::remove_file(companion_pid_path(data_dir));
}

/// Start / keep / stop the overlay process. Returns a user-facing error once
/// when the child dies immediately (typical on hosts with no compositor).
pub fn reconcile_companion(
    child: &mut Option<CompanionChild>,
    want: bool,
    data_dir: &Path,
    fixture: Option<FixtureKind>,
    spawn_blocked: &mut bool,
) -> Option<String> {
    if !want {
        *spawn_blocked = false;
        stop_companion(child, data_dir);
        return None;
    }
    if *spawn_blocked {
        return None;
    }
    if let Some(c) = child.as_mut() {
        match c.child.try_wait() {
            Ok(Some(status)) => {
                let quick = c.spawned_at.elapsed() < RESPAWN_GRACE;
                let _ = child.take();
                let _ = std::fs::remove_file(companion_pid_path(data_dir));
                if quick {
                    *spawn_blocked = true;
                    return Some(if status.success() {
                        "Companion overlay closed".into()
                    } else {
                        "Companion overlay needs a Wayland compositor with layer-shell".into()
                    });
                }
            }
            Ok(None) => return None,
            Err(_) => {
                stop_companion(child, data_dir);
            }
        }
    }
    match spawn_companion(data_dir, fixture) {
        Ok(c) => {
            *child = Some(c);
            None
        }
        Err(e) => {
            *spawn_blocked = true;
            Some(format!("Could not start companion overlay: {e}"))
        }
    }
}

// ── iced view (shared by the companion process) ──────────────────────────

#[derive(Debug, Clone, Copy)]
pub enum OverlayViewMessage {
    Tick,
}

pub fn view<'a>(model: &'a OverlayModel) -> Element<'a, OverlayViewMessage> {
    let mut body = column![header_row(model)].spacing(GRID_GAP).width(Fill);
    body = body.push(last_game_card(model.last_game.as_ref()));
    body = body.push(tonight_strip(&model.tonight_outcomes));
    body = body.push(heroes_block(&model.top_heroes));
    body = body.push(footer_row(model));

    container(body)
        .padding(PAD_INNER)
        .width(OVERLAY_WIDTH as f32)
        .style(theme::companion_panel)
        .clip(true)
        .into()
}

fn header_row(model: &OverlayModel) -> Element<'_, OverlayViewMessage> {
    row![
        status_dot(model.live),
        text(model.win_rate_line())
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT)
            .width(Fill)
            .wrapping(iced::widget::text::Wrapping::Word),
    ]
    .spacing(10)
    .align_y(Alignment::Center)
    .into()
}

fn status_dot<'a>(live: bool) -> Element<'a, OverlayViewMessage> {
    let color = if live { theme::OK } else { TEXT_3 };
    container(space().width(8).height(8))
        .style(move |_| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 999.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn last_game_card(game: Option<&OverlayLastGame>) -> Element<'_, OverlayViewMessage> {
    let Some(game) = game else {
        return container(
            text("No games yet tonight — press Tab in-game to capture the scoreboard.")
                .size(SIZE_BODY)
                .font(FONT_MEDIUM)
                .color(TEXT_3)
                .width(Fill)
                .wrapping(iced::widget::text::Wrapping::Word),
        )
        .padding(PAD_INNER)
        .width(Fill)
        .style(theme::surface_panel)
        .into();
    };

    let mut col = column![
        label(game.role.label()),
        text(game.map_name.clone())
            .size(SIZE_TITLE)
            .font(FONT_EXTRABOLD)
            .color(TEXT)
            .width(Fill)
            .wrapping(iced::widget::text::Wrapping::Word),
        text(format!(
            "{}  ·  {}",
            game.hero,
            game.played_at.format("%H:%M")
        ))
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_2),
        text(game.outcome.label().to_ascii_uppercase())
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(theme::outcome_color(game.outcome)),
    ]
    .spacing(4);

    if game.has_stat_line() {
        col = col.push(stat_line(game));
    }

    overlay_card_shell(game.role, game.outcome, col.into(), LAST_GAME_HEIGHT)
}

fn stat_line(game: &OverlayLastGame) -> Element<'static, OverlayViewMessage> {
    // Two rows of three — a 6-across row crushes labels at 360px.
    column![
        row![
            stat_box("E", game.elims),
            stat_box("D", game.deaths),
            stat_box("A", game.assists),
        ]
        .spacing(6)
        .width(Fill),
        row![
            stat_box("DMG", game.damage),
            stat_box("HEAL", game.healing),
            stat_box("MIT", game.mitigation),
        ]
        .spacing(6)
        .width(Fill),
    ]
    .spacing(6)
    .width(Fill)
    .into()
}

fn stat_box(label_s: &'static str, value: u32) -> Element<'static, OverlayViewMessage> {
    container(
        column![
            text(label_s)
                .size(SIZE_LABEL)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3),
            text(format_stat(value))
                .size(SIZE_META)
                .font(FONT_BOLD)
                .color(TEXT),
        ]
        .spacing(2)
        .align_x(Alignment::Center)
        .width(Fill),
    )
    .padding(Padding::from([4, 6]))
    .width(Fill)
    .style(theme::stat_box)
    .into()
}

fn format_stat(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", n as f32 / 1000.0)
    } else {
        n.to_string()
    }
}

fn tonight_strip(outcomes: &[Outcome]) -> Element<'static, OverlayViewMessage> {
    let mut row = row![label("Tonight")].spacing(6).align_y(Alignment::Center);
    if outcomes.is_empty() {
        row = row.push(text("—").size(SIZE_META).font(FONT_MEDIUM).color(TEXT_3));
    } else {
        for outcome in outcomes {
            row = row.push(container(space().width(10).height(16)).style(theme::stripe(*outcome)));
        }
    }
    row.into()
}

fn heroes_block(heroes: &[OverlayHero]) -> Element<'static, OverlayViewMessage> {
    let mut col = column![label("Top heroes tonight")].spacing(8).width(Fill);
    if heroes.is_empty() {
        col = col.push(
            text("No heroes yet")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    } else {
        for h in heroes {
            col = col.push(mini_hero(h));
        }
    }
    col.into()
}

fn mini_hero(hero: &OverlayHero) -> Element<'static, OverlayViewMessage> {
    let body = row![
        column![
            label(hero.role.label()),
            text(hero.hero.clone())
                .size(SIZE_BODY)
                .font(FONT_SEMIBOLD)
                .color(TEXT)
                .width(Fill)
                .wrapping(iced::widget::text::Wrapping::Word),
        ]
        .spacing(2)
        .width(Fill),
        text(format!("{:.0}%", hero.record.win_rate_pct()))
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(TEXT),
        text(hero.record.games_label())
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    ]
    .spacing(8)
    .align_y(Alignment::Center);
    overlay_card_shell(hero.role, Outcome::Unknown, body.into(), MINI_HERO_HEIGHT)
}

fn footer_row(model: &OverlayModel) -> Element<'_, OverlayViewMessage> {
    row![
        chip(&model.last_tab),
        space().width(Fill),
        chip(&model.sync_label),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .into()
}

fn chip<'a>(label_s: &'a str) -> Element<'a, OverlayViewMessage> {
    container(
        text(label_s.to_string())
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
    )
    .padding(Padding::from([6, 12]))
    .style(|_t| container::Style {
        background: Some(Background::Color(theme::SURFACE)),
        border: Border {
            color: theme::BORDER,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        shadow: Shadow::default(),
        ..container::Style::default()
    })
    .into()
}

fn label(s: &str) -> text::Text<'static> {
    text(s.to_ascii_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_SEMIBOLD)
        .color(TEXT_3)
}

fn overlay_card_shell(
    role: Role,
    outcome: Outcome,
    content: Element<'static, OverlayViewMessage>,
    height: f32,
) -> Element<'static, OverlayViewMessage> {
    container(
        row![
            container(space().width(STRIPE).height(height))
                .width(STRIPE)
                .height(height)
                .style(theme::stripe(outcome)),
            container(content)
                .padding(Padding::from([8, 10]))
                .width(Fill)
                .height(height),
        ]
        .spacing(0)
        .width(Fill)
        .height(height),
    )
    .style(theme::role_card(role))
    .width(Fill)
    .height(height)
    .clip(true)
    .into()
}

// ── companion process (`--companion`) ────────────────────────────────────

pub(crate) struct OverlayApp {
    data_dir: PathBuf,
    fixture: Option<FixtureKind>,
    pub(crate) model: OverlayModel,
    snapshot_mtime: Option<SystemTime>,
    clock: DateTime<Utc>,
}

impl OverlayApp {
    pub(crate) fn load(cli: &Cli) -> Self {
        if let Some(kind) = cli.fixture {
            let _ = snapshot::install_fixture(&cli.data_dir, kind);
        }
        let snap = snapshot::load_snapshot(&cli.data_dir);
        let games = games_from_snapshot(&snap);
        let seasons = seasons::load_cache(&cli.data_dir);
        let persisted = seasons::load_ui_state(&cli.data_dir);
        let season = seasons::resolve_selection(persisted, &seasons.seasons);
        let clock = if cli.fixture == Some(FixtureKind::Sample) {
            games.first().map(|g| g.played_at).unwrap_or_else(Utc::now)
        } else {
            Utc::now()
        };
        let config = stat_tracker::config::Config::load().unwrap_or_default();
        let sync_on = config.sync.is_some();
        let model = build_model(
            &games,
            &seasons.seasons,
            &season,
            clock,
            sync_on,
            detect_game_running(
                &cli.data_dir,
                &config.game_process_names,
                cli.fixture.is_some(),
            ),
        );
        Self {
            data_dir: cli.data_dir.clone(),
            fixture: cli.fixture,
            model,
            snapshot_mtime: snapshot::snapshot_mtime(&cli.data_dir),
            clock,
        }
    }

    pub(crate) fn refresh(&mut self) {
        if self.fixture.is_some() {
            return;
        }
        let mtime = snapshot::snapshot_mtime(&self.data_dir);
        if mtime == self.snapshot_mtime {
            return;
        }
        self.snapshot_mtime = mtime;
        let snap = snapshot::load_snapshot(&self.data_dir);
        let games = games_from_snapshot(&snap);
        let seasons = seasons::load_cache(&self.data_dir);
        let persisted = seasons::load_ui_state(&self.data_dir);
        let season = seasons::resolve_selection(persisted, &seasons.seasons);
        self.clock = Utc::now();
        let config = stat_tracker::config::Config::load().unwrap_or_default();
        self.model = build_model(
            &games,
            &seasons.seasons,
            &season,
            self.clock,
            config.sync.is_some(),
            detect_game_running(&self.data_dir, &config.game_process_names, false),
        );
    }
}

/// Layer-shell runner. Needs a Wayland compositor; this Cloud Agent VM does not
/// have one. Robert accepts the overlay on niri.
#[cfg(feature = "companion")]
pub fn run_companion(cli: Cli) -> anyhow::Result<()> {
    crate::overlay_shell::run(cli)
}

#[cfg(not(feature = "companion"))]
pub fn run_companion(_cli: Cli) -> anyhow::Result<()> {
    anyhow::bail!(
        "this build was compiled without the `companion` feature (iced_layershell). Rebuild with --features companion."
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cli::FixtureKind;
    use crate::fixtures;
    use crate::model::Game;
    use crate::snapshot::games_from_snapshot;
    use chrono::{TimeZone, Utc};

    fn sample_games() -> Vec<Game> {
        games_from_snapshot(&fixtures::snapshot(FixtureKind::Sample))
    }

    fn sample_seasons() -> Vec<Season> {
        serde_json::from_str(fixtures::sample_seasons_json()).unwrap()
    }

    #[test]
    fn visible_only_when_game_live_and_not_session_hidden() {
        assert!(!overlay_visible(&OverlayHold::Auto, false));
        assert!(overlay_visible(&OverlayHold::Auto, true));
        assert!(!overlay_visible(
            &OverlayHold::Hidden("sess-1".into()),
            true
        ));
        assert!(!overlay_visible(
            &OverlayHold::Hidden("sess-1".into()),
            false
        ));
    }

    #[test]
    fn manual_hide_sticks_until_game_ends_then_next_launch_shows() {
        let key = Some("sess-1");
        let hidden = toggle_hold(OverlayHold::Auto, key);
        assert_eq!(hidden, OverlayHold::Hidden("sess-1".into()));
        assert!(!overlay_visible(&hidden, true));

        let still = reconcile_hold(hidden.clone(), key);
        assert_eq!(still, hidden, "do not reopen mid-session");
        assert!(!overlay_visible(&still, true));

        let ended = reconcile_hold(still, None);
        assert_eq!(ended, OverlayHold::Auto);
        assert!(!overlay_visible(&ended, false));

        let next = reconcile_hold(ended, Some("sess-2"));
        assert_eq!(next, OverlayHold::Auto);
        assert!(overlay_visible(&next, true));
    }

    #[test]
    fn hide_before_first_tab_survives_session_id() {
        let hidden = toggle_hold(OverlayHold::Auto, Some("process"));
        let upgraded = reconcile_hold(hidden, Some("sess-tab"));
        assert_eq!(upgraded, OverlayHold::Hidden("sess-tab".into()));
        assert!(!overlay_visible(&upgraded, true));
    }

    #[test]
    fn new_session_clears_previous_hide() {
        let hold = OverlayHold::Hidden("sess-old".into());
        let next = reconcile_hold(hold, Some("sess-new"));
        assert_eq!(next, OverlayHold::Auto);
        assert!(overlay_visible(&next, true));
    }

    #[test]
    fn unhide_mid_session_and_toggle_without_game() {
        let hidden = OverlayHold::Hidden("sess-1".into());
        assert_eq!(toggle_hold(hidden, Some("sess-1")), OverlayHold::Auto);
        assert_eq!(
            toggle_hold(OverlayHold::Hidden("x".into()), None),
            OverlayHold::Auto
        );
    }

    #[test]
    fn empty_process_names_are_not_running() {
        assert!(!process_is_running(&[]));
        assert!(!process_is_running(&["no-such-process-zzz.exe".into()]));
    }

    #[test]
    fn fixture_counts_as_game_running() {
        let dir = std::env::temp_dir().join(format!(
            "sst-overlay-fix-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(detect_game_running(&dir, &[], true));
        assert!(!detect_game_running(&dir, &[], false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn active_game_json_opens_the_gate() {
        let dir = std::env::temp_dir().join(format!(
            "sst-overlay-ag-{}",
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir_all(&dir).unwrap();
        assert!(!active_game_present(&dir));
        std::fs::write(dir.join("active_game.json"), r#"{"session_id":"s1"}"#).unwrap();
        assert!(active_game_present(&dir));
        assert!(detect_game_running(&dir, &[], false));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn shell_spec_matches_design_doc() {
        let spec = overlay_shell_spec(412, Some("DP-1"));
        assert_eq!(spec.width, 360);
        assert_eq!(spec.margin, 24);
        assert_eq!(spec.exclusive_zone, 0);
        assert!(spec.layer_overlay);
        assert!(spec.keyboard_none);
        assert!(spec.events_transparent);
        assert_eq!(spec.output.as_deref(), Some("DP-1"));
        assert_eq!(overlay_shell_spec(100, Some("  ")).output, None);
        assert_eq!(overlay_shell_spec(100, None).height, 200);
    }

    #[test]
    fn sample_model_has_header_last_game_strip_and_heroes() {
        let games = sample_games();
        let seasons = sample_seasons();
        let clock = Utc.with_ymd_and_hms(2026, 9, 2, 22, 0, 0).unwrap();
        let model = build_model(
            &games,
            &seasons,
            &SeasonSel::Season("season-17".into()),
            clock,
            true,
            true,
        );
        assert_eq!(model.season_label, "Season 17");
        assert_eq!(model.record.wins, 2);
        assert_eq!(model.record.losses, 1);
        assert!(
            model.win_rate_line().contains("67%"),
            "{}",
            model.win_rate_line()
        );
        assert!(
            model.win_rate_line().contains("2–1"),
            "{}",
            model.win_rate_line()
        );
        let last = model.last_game.as_ref().expect("last game");
        assert_eq!(last.map_name, "King's Row");
        assert_eq!(last.hero, "Junker Queen");
        assert_eq!(last.outcome, Outcome::Win);
        assert!(last.has_stat_line());
        assert_eq!(model.tonight_outcomes.len(), 3);
        assert_eq!(
            model.tonight_outcomes,
            vec![Outcome::Win, Outcome::Loss, Outcome::Win]
        );
        assert_eq!(model.top_heroes.len(), 3);
        let names: Vec<&str> = model.top_heroes.iter().map(|h| h.hero.as_str()).collect();
        assert_eq!(names, ["Ana", "Ashe", "Junker Queen"]);
        assert_eq!(model.last_tab, "Last Tab 21:14");
        assert_eq!(model.sync_label, "Sync on");
        assert!(content_height(&model) >= 200);
        assert!(content_height(&model) <= 900);
    }

    #[test]
    fn empty_model_copy() {
        let clock = Utc.with_ymd_and_hms(2026, 9, 2, 22, 0, 0).unwrap();
        let model = build_model(&[], &[], &SeasonSel::AllTime, clock, false, false);
        assert_eq!(model.season_label, "All time");
        assert!(model.last_game.is_none());
        assert!(model.tonight_outcomes.is_empty());
        assert!(model.top_heroes.is_empty());
        assert_eq!(model.last_tab, "No Tab yet");
        assert_eq!(model.sync_label, "Sync off");
        assert_eq!(companion_copy(false, false), "Companion waiting for game");
        assert_eq!(companion_copy(false, true), "Companion hidden");
        assert_eq!(companion_copy(true, true), "Companion showing");
    }

    #[test]
    fn overlay_view_builds_for_empty_and_sample() {
        let clock = Utc.with_ymd_and_hms(2026, 9, 2, 22, 0, 0).unwrap();
        let empty = build_model(&[], &[], &SeasonSel::AllTime, clock, false, false);
        let _ = view(&empty);
        let sample = build_model(
            &sample_games(),
            &sample_seasons(),
            &SeasonSel::Season("season-17".into()),
            clock,
            true,
            true,
        );
        let _ = view(&sample);
    }
}
