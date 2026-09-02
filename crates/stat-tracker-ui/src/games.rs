use iced::widget::{column, row, space, text};
use iced::{Alignment, Element, Fill};

use crate::aggregate::{distinct_heroes, distinct_maps, filter_games, group_by_local_day};
use crate::app::{Message, TrackerApp};
use crate::model::Outcome;
use crate::theme::{FONT_BOLD, FONT_MEDIUM, GRID_GAP, SIZE_TITLE, TEXT, TEXT_3};
use crate::widgets;

pub fn view(app: &TrackerApp) -> Element<'_, Message> {
    let header_only = app.header_filter();
    let option_pool = filter_games(&app.games, &header_only);
    let heroes = distinct_heroes(&option_pool);
    let maps = distinct_maps(&option_pool);
    let shown = filter_games(&app.games, &app.games_filter());
    let groups = group_by_local_day(&shown);

    let mut col = column![
        text("Games").size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),
        widgets::label_text("Season inherits the header switch · role chips apply"),
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
        for chunk in group.games.chunks(2) {
            let mut pair = row![].spacing(GRID_GAP).width(Fill);
            let mut expanded_sid = None;
            for g in chunk {
                let selected = app.expanded.as_deref() == Some(g.session_id.as_str());
                if selected {
                    expanded_sid = Some(g.session_id.clone());
                }
                pair = pair.push(widgets::compact_game_card_clickable(g, selected));
            }
            if chunk.len() == 1 {
                pair = pair.push(space().width(Fill));
            }
            col = col.push(pair);
            if let Some(sid) = expanded_sid
                && let Some(g) = chunk.iter().find(|g| g.session_id == sid).cloned()
            {
                let confirm = app.confirm_delete.as_deref() == Some(sid.as_str());
                col = col.push(widgets::expanded_game_card(
                    &g,
                    app.editing && app.edit.session_id == sid,
                    &app.edit,
                    confirm,
                ));
            }
        }
    }
    col.into()
}

fn filter_bar(app: &TrackerApp, heroes: &[String], maps: &[String]) -> Element<'static, Message> {
    let mut hero_row = row![widgets::label_text("Hero")]
        .spacing(8)
        .align_y(Alignment::Center);
    hero_row = hero_row.push(widgets::filter_chip(
        "All".into(),
        app.filter_hero.is_none(),
        Message::FilterHero(None),
    ));
    for h in heroes {
        hero_row = hero_row.push(widgets::filter_chip(
            h.clone(),
            app.filter_hero.as_deref() == Some(h.as_str()),
            Message::FilterHero(Some(h.clone())),
        ));
    }

    let mut map_row = row![widgets::label_text("Map")]
        .spacing(8)
        .align_y(Alignment::Center);
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
    .align_y(Alignment::Center);

    column![hero_row, map_row, outcome_row]
        .spacing(8)
        .push(
            text(if app.fixture.is_some() {
                "Fixture mode: commands still write under this --data-dir /commands/"
            } else {
                "Actions write StoreCommand files; the daemon applies them and refreshes the snapshot"
            })
            .size(theme_meta())
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        )
        .into()
}

fn theme_meta() -> f32 {
    crate::theme::SIZE_META
}
