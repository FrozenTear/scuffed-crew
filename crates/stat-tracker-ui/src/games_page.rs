//! The 0.6 Games page (spec p2): the filter chips with the Sessions / Flat
//! list switch on the right, then each session's heading over its table
//! ([`crate::games_session`], [`crate::games_table`]), or one table in
//! Flat list. A row opens the game's card under it, with edit, delete and
//! the result picker, as the 0.5 page did. The 0.5 sidebar layout keeps
//! its own Games page ([`crate::games`]).

use iced::widget::{Column, Row, button, column, text};
use iced::{Alignment, Element, Fill, Padding, Task};

use crate::aggregate::{distinct_heroes, distinct_maps, filter_games, group_by_session_gap};
use crate::app::{Message, TrackerApp};
use crate::form_widgets::hint;
use crate::games_table::{self, Col, RowState, Sort};
use crate::model::{Game, Role, RoleFilter};
use crate::settings_model::SessionWindow;
use crate::settings_rows::{Segment, segmented};
use crate::theme::{self, FONT_SEMIBOLD, GRID_GAP, SIZE_META, TEXT};
use crate::widgets;

/// Under the last table (spec p2), after the split rule.
const SORT_NOTE: &str = "Columns sort; unsure and empty values always sort last.";
/// No games stored at all.
pub const EMPTY_NONE: &str = "No games yet. A game shows here once the tracker saves it.";
/// Games stored, none left by the filters.
pub const EMPTY_FILTERED: &str = "No games match these filters.";

/// A message from the Games page.
#[derive(Debug, Clone)]
pub enum Msg {
    /// Sessions (`false`) or Flat list (`true`).
    Flat(bool),
    /// A column header: sort by it, or flip its direction.
    SortBy(Col),
    /// The role chip: one role, or all (`None`).
    PickRole(Option<Role>),
}

/// What the Games page keeps between frames.
#[derive(Debug, Clone, Copy, Default)]
pub struct GamesPage {
    pub flat: bool,
    pub sort: Sort,
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    match msg {
        Msg::Flat(flat) => app.games_page.flat = flat,
        Msg::SortBy(col) => app.games_page.sort = app.games_page.sort.clicked(col),
        Msg::PickRole(role) => {
            app.roles = role.map_or_else(RoleFilter::default, |r| RoleFilter::default().toggle(r));
        }
    }
    Task::none()
}

/// "Sessions split after 3 hours without a game." from the Session window
/// setting, which the grouping follows.
pub fn split_note(window_secs: u64) -> String {
    if window_secs == 0 {
        return format!("Session window is off, so each game is its own session. {SORT_NOTE}");
    }
    format!(
        "Sessions split after {} without a game. {SORT_NOTE}",
        SessionWindow(window_secs)
    )
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let pool = filter_games(&app.games, &app.header_filter());
    let heroes = distinct_heroes(&pool);
    let maps = distinct_maps(&pool);
    let mut shown = crate::games::shown_games(app);

    let mut page = Column::new().spacing(GRID_GAP).width(Fill);
    page = page.push(
        Row::new()
            .spacing(12)
            .align_y(Alignment::Start)
            .push(
                iced::widget::container(crate::filter_chips::bar(app, &heroes, &maps)).width(Fill),
            )
            .push(view_switch(app.games_page.flat)),
    );
    if app.games.iter().any(|g| g.upload_reject.is_some()) {
        page = page.push(retry_rejected_uploads_button());
    }
    if shown.is_empty() {
        let copy = if app.games.is_empty() {
            EMPTY_NONE
        } else {
            EMPTY_FILTERED
        };
        return page.push(widgets::empty_surface(copy)).into();
    }

    let cols = games_table::columns(content_width);
    let sort = app.games_page.sort;
    let state = RowState {
        live: app.live_game_session.as_deref(),
        expanded: app.expanded.as_deref(),
    };
    let detail = |g: &Game| detail_card(app, g);
    if app.games_page.flat {
        games_table::sort_games(&mut shown, sort);
        page = page.push(games_table::table(&shown, &cols, sort, &state, detail));
    } else {
        let gap = chrono::TimeDelta::seconds(
            i64::try_from(app.saved_config.session_window_secs).unwrap_or(i64::MAX),
        );
        for group in group_by_session_gap(&shown, gap) {
            let mut rows: Vec<&Game> = group.games.iter().collect();
            games_table::sort_games(&mut rows, sort);
            page = page.push(
                column![
                    crate::games_session::header(&group),
                    games_table::table(&rows, &cols, sort, &state, detail),
                ]
                .spacing(10),
            );
        }
    }
    page.push(hint(split_note(app.saved_config.session_window_secs)))
        .into()
}

/// The game's card under its row: the 0.5 expanded card, with edit,
/// delete, the result picker and the review panel.
fn detail_card<'a>(app: &'a TrackerApp, g: &Game) -> Element<'a, Message> {
    let in_progress = crate::model::game_card_in_progress(
        &g.session_id,
        g.outcome,
        app.live_game_session.as_deref(),
    );
    widgets::expanded_game_card(
        g,
        app.editing && app.edit.session_id == g.session_id,
        &app.edit,
        app.confirm_delete.as_deref() == Some(g.session_id.as_str()),
        in_progress,
        app.sessions_with_kit.contains(&g.session_id),
    )
}

/// Sessions | Flat list.
fn view_switch(flat: bool) -> Element<'static, Message> {
    segmented(vec![
        Segment {
            label: "Sessions",
            selected: !flat,
            msg: Some(Message::GamesPage(Msg::Flat(false))),
        },
        Segment {
            label: "Flat list",
            selected: flat,
            msg: Some(Message::GamesPage(Msg::Flat(true))),
        },
    ])
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_split_note_follows_the_session_window() {
        assert!(split_note(10_800).starts_with("Sessions split after 3 hours without a game."));
        assert!(split_note(1800).contains("30 min"));
        assert!(split_note(0).contains("each game is its own session"));
    }

    #[test]
    fn the_switch_and_sort_live_in_page_state() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert!(!app.games_page.flat);
        let _ = app.update(Message::GamesPage(Msg::Flat(true)));
        assert!(app.games_page.flat);
        let _ = app.update(Message::GamesPage(Msg::SortBy(Col::Map)));
        assert_eq!(
            app.games_page.sort,
            Sort {
                col: Col::Map,
                desc: false
            }
        );
    }

    #[test]
    fn the_role_chip_picks_one_role_or_clears() {
        let (_dir, mut app) = crate::fixtures::test_app();
        let _ = app.update(Message::GamesPage(Msg::PickRole(Some(Role::Support))));
        assert_eq!(
            crate::filter_chips::single_role(app.roles),
            Some(Role::Support)
        );
        let _ = app.update(Message::GamesPage(Msg::PickRole(None)));
        assert!(app.roles.unrestricted());
    }

    #[test]
    fn empty_states_tell_no_games_from_none_left() {
        let (_dir, app) = crate::fixtures::test_app();
        assert!(app.games.is_empty());
        let _ = view(&app, 1200.0);
        assert_ne!(EMPTY_NONE, EMPTY_FILTERED);
    }
}
