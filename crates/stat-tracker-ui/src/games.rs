use iced::widget::{column, row, space, text};
use iced::{Alignment, Element, Fill};

use crate::aggregate::{
    DEFAULT_SESSION_GAP, distinct_heroes, distinct_maps, filter_games, group_by_session_gap,
};
use crate::app::{Message, TrackerApp};
use crate::layout::games_columns;
use crate::model::{Game, Outcome};
use crate::theme::{FONT_BOLD, GRID_GAP, SIZE_TITLE, TEXT};
use crate::widgets;

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let header_only = app.header_filter();
    let option_pool = filter_games(&app.games, &header_only);
    let heroes = distinct_heroes(&option_pool);
    let maps = distinct_maps(&option_pool);
    let shown = filter_games(&app.games, &app.games_filter());
    let groups = group_by_session_gap(&shown, DEFAULT_SESSION_GAP);
    let cols = games_columns(content_width);

    let mut col = column![
        text("Games").size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),
        filter_bar(app, &heroes, &maps),
    ]
    .spacing(GRID_GAP)
    .width(Fill);

    if groups.is_empty() {
        col = col.push(widgets::empty_surface("No games match these filters"));
        return col.into();
    }

    for group in groups {
        col = col.push(
            text(group.heading)
                .size(SIZE_TITLE)
                .font(FONT_BOLD)
                .color(TEXT),
        );
        col = col.push(sitting_cards(&group.games, app, cols));
    }
    col.into()
}

/// Compact shelf at `cols` (2–4). An expanded game *replaces* its compact card
/// (full-width) instead of duplicating below the row.
fn sitting_cards<'a>(games: &[Game], app: &'a TrackerApp, cols: usize) -> Element<'a, Message> {
    let cols = cols.max(1);
    let mut col = column![].spacing(GRID_GAP).width(Fill);
    let mut pending: Vec<&Game> = Vec::new();
    for g in games {
        let selected = app.expanded.as_deref() == Some(g.session_id.as_str());
        if selected {
            if !pending.is_empty() {
                col = col.push(flush_compact(&mut pending, cols));
            }
            let confirm = app.confirm_delete.as_deref() == Some(g.session_id.as_str());
            col = col.push(widgets::expanded_game_card(
                g,
                app.editing && app.edit.session_id == g.session_id,
                &app.edit,
                confirm,
            ));
        } else {
            pending.push(g);
            if pending.len() == cols {
                col = col.push(flush_compact(&mut pending, cols));
            }
        }
    }
    if !pending.is_empty() {
        col = col.push(flush_compact(&mut pending, cols));
    }
    col.into()
}

fn flush_compact(pending: &mut Vec<&Game>, cols: usize) -> Element<'static, Message> {
    let mut pair = row![].spacing(GRID_GAP).width(Fill);
    let n = pending.len();
    for p in pending.drain(..) {
        pair = pair.push(widgets::compact_game_card_clickable(p, false));
    }
    for _ in n..cols {
        pair = pair.push(space().width(Fill));
    }
    pair.into()
}

fn filter_bar(app: &TrackerApp, heroes: &[String], maps: &[String]) -> Element<'static, Message> {
    let mut hero_row = row![widgets::label_text("Hero")]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill);
    hero_row = hero_row.push(widgets::filter_chip(
        "All".into(),
        app.filter_hero.is_none(),
        Message::FilterHero(None),
    ));
    for h in heroes {
        hero_row = hero_row.push(widgets::filter_chip(
            crate::model::display_hero_name(h),
            app.filter_hero.as_deref() == Some(h.as_str()),
            Message::FilterHero(Some(h.clone())),
        ));
    }

    let mut map_row = row![widgets::label_text("Map")]
        .spacing(8)
        .align_y(Alignment::Center)
        .width(Fill);
    map_row = map_row.push(widgets::filter_chip(
        "All".into(),
        app.filter_map.is_none(),
        Message::FilterMap(None),
    ));
    for m in maps {
        map_row = map_row.push(widgets::filter_chip(
            m.clone(),
            app.filter_map.as_deref() == Some(m.as_str()),
            Message::FilterMap(Some(m.clone())),
        ));
    }

    let outcome_row = row![
        widgets::label_text("Outcome"),
        widgets::filter_chip(
            "All".into(),
            app.filter_outcome.is_none(),
            Message::FilterOutcome(None),
        ),
        widgets::filter_chip(
            "Win".into(),
            app.filter_outcome == Some(Outcome::Win),
            Message::FilterOutcome(Some(Outcome::Win)),
        ),
        widgets::filter_chip(
            "Loss".into(),
            app.filter_outcome == Some(Outcome::Loss),
            Message::FilterOutcome(Some(Outcome::Loss)),
        ),
        widgets::filter_chip(
            "Draw".into(),
            app.filter_outcome == Some(Outcome::Draw),
            Message::FilterOutcome(Some(Outcome::Draw)),
        ),
    ]
    .spacing(8)
    .align_y(Alignment::Center)
    .width(Fill);

    column![hero_row.wrap(), map_row.wrap(), outcome_row.wrap()]
        .spacing(8)
        .width(Fill)
        .into()
}
