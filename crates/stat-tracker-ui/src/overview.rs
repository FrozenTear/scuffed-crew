use chrono::{DateTime, Local, Utc};
use iced::widget::{Row, button, column, row, space, text};
use iced::{Alignment, Element, Fill};

use crate::aggregate::{aggregate, aggregate_filtered};
use crate::app::{Message, TrackerApp};
use crate::layout::{heroes_columns, tonight_compact_columns};
use crate::model::{Game, Screen, SeasonSel};
use crate::theme::{self, FONT_SEMIBOLD, GRID_GAP};
use crate::widgets;

pub const TONIGHT_EMPTY: &str =
    "No games yet tonight — press Tab in-game to capture the scoreboard.";

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let filter = app.header_filter();
    let tonight_all = tonight_games(&app.games, app.clock);
    let tonight: Vec<&Game> = tonight_all
        .into_iter()
        .filter(|g| app.roles.matches(g.role))
        .collect();
    let stats = aggregate_filtered(&app.games, &filter);
    let all_time = aggregate(&app.games, None, None);

    let tonight_shelf = tonight_shelf(&tonight, tonight_compact_columns(content_width));
    let heroes_shelf = heroes_shelf(&stats, heroes_columns(content_width));
    let bottom = row![
        widgets::season_panel(
            &stats.record,
            &all_time.record,
            !matches!(app.season, SeasonSel::AllTime),
        ),
        widgets::maps_panel(&stats.maps),
        widgets::health_panel(
            &app.health_status,
            app.overlay_enabled,
            app.overlay_showing(),
        ),
    ]
    .spacing(GRID_GAP)
    .width(Fill);

    let mut col = column![].spacing(24).width(Fill);
    if let Some(info) = &app.update {
        col = col.push(crate::update::banner(info));
    }
    col = col.push(tonight_shelf).push(heroes_shelf).push(bottom);
    col.into()
}

fn tonight_shelf(games: &[&Game], compact_cols: usize) -> Element<'static, Message> {
    let cols = compact_cols.max(1);
    let mut col = column![widgets::label_text("Tonight")]
        .spacing(GRID_GAP)
        .width(Fill);
    if games.is_empty() {
        col = col.push(widgets::empty_surface(TONIGHT_EMPTY));
        return col.into();
    }
    col = col.push(widgets::featured_game_card(games[0]));
    let rest: Vec<_> = games.iter().skip(1).copied().collect();
    if !rest.is_empty() {
        for chunk in rest.chunks(cols) {
            let mut row = Row::new().spacing(GRID_GAP).width(Fill);
            for g in chunk {
                row = row.push(widgets::compact_game_card(g));
            }
            for _ in chunk.len()..cols {
                row = row.push(space().width(Fill));
            }
            col = col.push(row);
        }
    }
    col.into()
}

fn heroes_shelf(stats: &crate::aggregate::Aggregates, cols: usize) -> Element<'static, Message> {
    let header = row![
        widgets::label_text("Heroes"),
        space().width(Fill),
        button(
            text("all heroes →")
                .size(theme::SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(theme::ACCENT),
        )
        .style(|_, _| iced::widget::button::Style {
            background: None,
            text_color: theme::ACCENT,
            border: iced::Border::default(),
            shadow: iced::Shadow::default(),
            snap: false,
        })
        .on_press(Message::Navigate(Screen::Heroes)),
    ]
    .align_y(Alignment::Center);
    let mut col = column![header].spacing(GRID_GAP).width(Fill);
    if stats.heroes.is_empty() {
        col = col.push(widgets::empty_surface("No heroes in this window"));
        return col.into();
    }
    let mut cards = Row::new().spacing(GRID_GAP).width(Fill);
    let show = cols.max(1);
    let shown = stats.heroes.len().min(show);
    for h in stats.heroes.iter().take(show) {
        cards = cards.push(widgets::hero_card(h));
    }
    for _ in shown..show {
        cards = cards.push(space().width(Fill));
    }
    col.push(cards).into()
}

/// Games on `clock`'s local calendar day, newest first (one row per game).
pub fn tonight_games(games: &[Game], clock: DateTime<Utc>) -> Vec<&Game> {
    let today = clock.with_timezone(&Local).date_naive();
    games
        .iter()
        .filter(|g| g.played_at.with_timezone(&Local).date_naive() == today)
        .collect()
}
