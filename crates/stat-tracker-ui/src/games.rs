use iced::widget::{button, column, row, space, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::aggregate::{
    DEFAULT_SESSION_GAP, distinct_heroes, distinct_maps, filter_games, group_by_session_gap,
};
use crate::app::{Message, TrackerApp};
use crate::layout::games_columns;
use crate::model::{Game, Outcome, Screen, game_card_in_progress};
use crate::theme::{self, FONT_BOLD, FONT_SEMIBOLD, GRID_GAP, SIZE_META, SIZE_TITLE, TEXT};
use crate::widgets;

/// Copy of the clearable chip the top bar's "need a look" chip turns on.
pub const NEEDS_LOOK_FILTER_LABEL: &str = "Needs a look  ×";

/// The games the Games tab lists: the season, role, hero, map and outcome
/// filters, then the "Needs a look" filter when it is on.
pub fn shown_games(app: &TrackerApp) -> Vec<&Game> {
    let mut shown = filter_games(&app.games, &app.games_filter());
    if app.needs_look_filter {
        let live = app.live_game_session.as_deref();
        shown.retain(|g| crate::nav::counts_as_needs_look(g, live));
    }
    shown
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let header_only = app.header_filter();
    let option_pool = filter_games(&app.games, &header_only);
    let heroes = distinct_heroes(&option_pool);
    let maps = distinct_maps(&option_pool);
    let shown = shown_games(app);
    let groups = group_by_session_gap(&shown, DEFAULT_SESSION_GAP);
    let cols = games_columns(content_width);

    let mut col = column![].spacing(GRID_GAP).width(Fill);
    if let Some(title) = crate::nav::classic_title(app, Screen::Games) {
        col = col.push(title);
    }
    if app.games.iter().any(|game| game.upload_reject.is_some()) {
        col = col.push(retry_rejected_uploads_button());
    }
    if app.needs_look_filter {
        col = col.push(widgets::filter_chip(
            NEEDS_LOOK_FILTER_LABEL.into(),
            true,
            Message::Nav(crate::nav::Msg::ClearNeedsLook),
        ));
    }
    col = col.push(filter_bar(app, &heroes, &maps));

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

/// Compact shelf at `cols` (2-4, more on an ultrawide). An expanded game
/// *replaces* its compact card (full-width) instead of duplicating below the row.
fn sitting_cards<'a>(games: &[Game], app: &'a TrackerApp, cols: usize) -> Element<'a, Message> {
    let cols = cols.max(1);
    let live = app.live_game_session.as_deref();
    let mut col = column![].spacing(GRID_GAP).width(Fill);
    let mut pending: Vec<&Game> = Vec::new();
    for g in games {
        let selected = app.expanded.as_deref() == Some(g.session_id.as_str());
        if selected {
            if !pending.is_empty() {
                col = col.push(flush_compact(&mut pending, cols, live));
            }
            let confirm = app.confirm_delete.as_deref() == Some(g.session_id.as_str());
            let in_progress = game_card_in_progress(&g.session_id, g.outcome, live);
            col = col.push(widgets::expanded_game_card(
                g,
                app.editing && app.edit.session_id == g.session_id,
                &app.edit,
                confirm,
                in_progress,
                app.sessions_with_kit.contains(&g.session_id),
            ));
        } else {
            pending.push(g);
            if pending.len() == cols {
                col = col.push(flush_compact(&mut pending, cols, live));
            }
        }
    }
    if !pending.is_empty() {
        col = col.push(flush_compact(&mut pending, cols, live));
    }
    col.into()
}

fn flush_compact(
    pending: &mut Vec<&Game>,
    cols: usize,
    live: Option<&str>,
) -> Element<'static, Message> {
    let mut pair = row![].spacing(GRID_GAP).width(Fill);
    let n = pending.len();
    for p in pending.drain(..) {
        let in_progress = game_card_in_progress(&p.session_id, p.outcome, live);
        pair = pair.push(widgets::compact_game_card_clickable(p, false, in_progress));
    }
    for _ in n..cols {
        pair = pair.push(space().width(Fill));
    }
    pair.into()
}

fn retry_rejected_uploads_button() -> Element<'static, Message> {
    button(
        text("Retry rejected uploads")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([6, 14]))
    .style(theme::ghost_btn())
    .on_press(Message::RetryRejectedUploads)
    .into()
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
