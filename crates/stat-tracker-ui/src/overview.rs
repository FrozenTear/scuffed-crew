use chrono::{DateTime, Local, Utc};
use iced::widget::{Row, column, container, row, space, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::aggregate::aggregate;
use crate::app::{Message, TrackerApp};
use crate::model::{Game, SeasonSel};
use crate::theme::{
    self, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAGE_PAD_X, PAGE_PAD_Y, SIZE_BODY,
    SIZE_FEATURED, TEXT, TEXT_3,
};
use crate::widgets;

pub const TONIGHT_EMPTY: &str =
    "No games yet tonight — press Tab in-game to capture the scoreboard.";

pub fn view(app: &TrackerApp) -> Element<'_, Message> {
    let window = crate::seasons::window_for(&app.season, &app.seasons.seasons);
    let tonight = tonight_games(&app.games, app.clock);
    let stats = aggregate(&app.games, window, None);
    let all_time = aggregate(&app.games, None, None);

    let header = row![
        text("Scuffed Tracker")
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
        space().width(Fill),
        widgets::season_switch(&app.seasons.seasons, &app.season),
        widgets::status_stub(&app.live_status),
    ]
    .spacing(16)
    .align_y(Alignment::Center);

    let tonight_shelf = tonight_shelf(&tonight);
    let heroes_shelf = heroes_shelf(&stats);
    let bottom = row![
        widgets::season_panel(
            &stats.record,
            &all_time.record,
            !matches!(app.season, SeasonSel::AllTime),
        ),
        widgets::maps_panel(&stats.maps),
        widgets::health_panel(&app.health_status),
    ]
    .spacing(GRID_GAP);

    container(
        column![header, tonight_shelf, heroes_shelf, bottom]
            .spacing(24)
            .width(Fill),
    )
    .padding(Padding {
        top: PAGE_PAD_Y,
        bottom: PAGE_PAD_Y,
        left: PAGE_PAD_X,
        right: PAGE_PAD_X,
    })
    .width(Fill)
    .style(theme::page_background)
    .into()
}

fn tonight_shelf(games: &[&Game]) -> Element<'static, Message> {
    let mut col = column![widgets::label_text("Tonight")].spacing(GRID_GAP);
    if games.is_empty() {
        col = col.push(
            container(
                text(TONIGHT_EMPTY.to_string())
                    .size(SIZE_BODY)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            )
            .padding(theme::PAD_INNER)
            .width(Fill)
            .style(theme::surface_panel),
        );
        return col.into();
    }
    col = col.push(widgets::featured_game_card(games[0]));
    if games.len() > 1 {
        let mut rest = Row::new().spacing(GRID_GAP);
        for g in games.iter().skip(1) {
            rest = rest.push(widgets::compact_game_card(g));
        }
        col = col.push(rest);
    }
    col.into()
}

fn heroes_shelf(stats: &crate::aggregate::Aggregates) -> Element<'static, Message> {
    let header = row![
        widgets::label_text("Heroes"),
        space().width(Fill),
        text("all heroes →")
            .size(theme::SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(theme::ACCENT),
    ]
    .align_y(Alignment::Center);
    let mut col = column![header].spacing(GRID_GAP);
    if stats.heroes.is_empty() {
        col = col.push(
            container(
                text("No heroes in this window")
                    .size(SIZE_BODY)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            )
            .padding(theme::PAD_INNER)
            .width(Fill)
            .style(theme::surface_panel),
        );
        return col.into();
    }
    let mut cards = Row::new().spacing(GRID_GAP);
    for h in stats.heroes.iter().take(4) {
        cards = cards.push(widgets::hero_card(h));
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
