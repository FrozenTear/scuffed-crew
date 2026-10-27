//! The 0.6 Overview (spec p1, and p8 at about 1024): the filter chips, then
//! the main column (the latest session, its newest game, the games before
//! it, the heroes) beside the right panel (season, Needs a look,
//! Companion). Below [`COLLAPSE_BELOW`] of page it is
//! one column and the chips fold into "Filters (n)". The 0.5 sidebar
//! layout keeps its own Overview ([`crate::overview`]).
//!
//! Rank, queue, length and crew are spec example data for fields the
//! tracker does not store yet, so they are not drawn.

use iced::widget::{Column, Row, button, column, row, text};
use iced::{Alignment, Background, Border, Padding, Theme};
use iced::{Element, Fill, Task};

use crate::aggregate::{GameFilter, SessionGroup, filter_games, group_by_session_gap};
use crate::app::{Message, TrackerApp};
use crate::filter_chips;
use crate::model::{Game, Role, RoleFilter, Screen};
use crate::theme::{self, BORDER, FONT_SEMIBOLD, SIZE_META, SURFACE, TEXT, TEXT_2};

/// Width of the right panel (spec about 300).
pub const SIDE_W: f32 = 300.0;
/// Space between the columns and between the cards.
pub const GAP: f32 = 16.0;

/// A message from the Overview.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The role chip: one role, or all (`None`).
    PickRole(Option<Role>),
    /// "Filters (n)": open or shut the folded chips.
    ToggleFilters,
    /// Open this game on Games ("Check", a row of Earlier tonight).
    OpenGame(String),
}

/// What the Overview keeps between frames.
#[derive(Debug, Clone, Copy, Default)]
pub struct OverviewPage {
    pub filters_open: bool,
}

pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    match msg {
        Msg::PickRole(role) => {
            app.roles = role.map_or_else(RoleFilter::default, |r| RoleFilter::default().toggle(r));
        }
        Msg::ToggleFilters => app.overview_page.filters_open = !app.overview_page.filters_open,
        Msg::OpenGame(id) => {
            let task = app.update(Message::Navigate(Screen::Games));
            app.expanded = Some(id);
            return task;
        }
    }
    Task::none()
}

/// The games the Overview counts: every chip the filter row shows (season,
/// role, hero, map and a result chip set on Games). "Needs a look" filters
/// Games only.
pub fn filter(app: &TrackerApp) -> GameFilter {
    app.games_filter()
}

/// Page width below which the chips fold behind "Filters (n)" (spec p8,
/// about 1024 of window).
pub const COLLAPSE_BELOW: f32 = 1000.0;

/// The chip row: on a wide page every chip; below [`COLLAPSE_BELOW`]
/// "Filters (n)" and the chips that filter, the rest while it is open.
fn chip_row<'a>(
    app: &'a TrackerApp,
    heroes: &[String],
    maps: &[String],
    narrow: bool,
) -> Element<'a, Message> {
    if !narrow {
        return filter_chips::bar(app, heroes, maps, true);
    }
    let open = app.overview_page.filters_open;
    Row::new()
        .spacing(8)
        .align_y(Alignment::Start)
        .push(fold_chip(
            filter_chips::active_count(app),
            open,
            Message::OverviewPage(Msg::ToggleFilters),
        ))
        .push(filter_chips::bar(app, heroes, maps, open))
        .into()
}

/// "Filters (1) ▾": opens or shuts the folded chips.
fn fold_chip(count: usize, open: bool, toggle: Message) -> Element<'static, Message> {
    button(
        row![
            text(format!("Filters ({count})"))
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
            text(if open { "↑" } else { "↓" })
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([5, 12]))
    .style(|_: &Theme, status| button::Style {
        background: Some(Background::Color(
            if matches!(status, button::Status::Hovered) {
                BORDER
            } else {
                SURFACE
            },
        )),
        text_color: TEXT,
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..button::Style::default()
    })
    .on_press(toggle)
    .into()
}

/// The latest session of `games` (newest first), split by the Session
/// window setting as Games does.
pub fn latest_session(app: &TrackerApp, games: &[&Game]) -> Option<SessionGroup> {
    let gap = chrono::TimeDelta::seconds(
        i64::try_from(app.saved_config.session_window_secs).unwrap_or(i64::MAX),
    );
    group_by_session_gap(games, gap).into_iter().next()
}

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let filter = filter(app);
    let shown = filter_games(&app.games, &filter);
    let pool = filter_games(&app.games, &app.header_filter());
    let heroes = crate::aggregate::distinct_heroes(&pool);
    let maps = crate::aggregate::distinct_maps(&pool);
    let narrow = content_width.is_finite() && content_width < COLLAPSE_BELOW;

    let mut page = Column::new().spacing(GAP).width(Fill);
    if let Some(info) = &app.update
        && app.update_banner_visible()
    {
        page = page.push(crate::update::banner(
            info,
            &app.update_progress,
            &app.update_plan,
            &app.update_notes,
            app.update_notes_ui(),
            app.update_notes_expanded,
            app.restart_need(),
            app.package_probe(),
        ));
    }
    page = page.push(chip_row(app, &heroes, &maps, narrow));

    let main_w = if narrow {
        content_width
    } else {
        content_width - SIDE_W - GAP
    };
    let session = latest_session(app, &shown);
    let main = column![
        crate::overview_main::view(app, session.as_ref(), main_w),
        crate::overview_heroes::view(app, &shown, main_w),
    ]
    .spacing(GAP)
    .width(Fill);
    let side = crate::overview_side::view(app, &shown);
    if narrow {
        page = page.push(main).push(side);
    } else {
        page = page.push(
            Row::new()
                .spacing(GAP)
                .push(main)
                .push(iced::widget::container(side).width(SIDE_W)),
        );
    }
    page.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_filter_fold_and_role_chip_live_in_page_state() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert!(!app.overview_page.filters_open);
        let _ = app.update(Message::OverviewPage(Msg::ToggleFilters));
        assert!(app.overview_page.filters_open);
        let _ = app.update(Message::OverviewPage(Msg::PickRole(Some(Role::Tank))));
        assert_eq!(
            crate::filter_chips::single_role(app.roles),
            Some(Role::Tank)
        );
        let _ = app.update(Message::OverviewPage(Msg::PickRole(None)));
        assert!(app.roles.unrestricted());
    }

    #[test]
    fn opening_a_game_goes_to_games_with_it_open() {
        let (_dir, mut app) = crate::fixtures::test_app();
        let _ = app.update(Message::OverviewPage(Msg::OpenGame("s1".into())));
        assert_eq!(app.screen, Screen::Games);
        assert_eq!(app.expanded.as_deref(), Some("s1"));
    }

    #[test]
    fn the_overview_follows_the_chips_it_shows() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.filter_outcome = Some(crate::model::Outcome::Win);
        assert_eq!(filter(&app).outcome, Some(crate::model::Outcome::Win));
        let _ = view(&app, 1300.0);
        let _ = view(&app, 900.0);
    }
}
