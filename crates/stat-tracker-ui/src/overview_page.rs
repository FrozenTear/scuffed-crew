//! The 0.6 Overview (spec p1, and p8 at about 1024): the filter chips, then
//! the main column (the latest session, its newest game, the games before
//! it, the heroes) beside the right panel (season, Needs a look,
//! Companion). Below [`COLLAPSE_BELOW`] of page it is one column; below
//! a 1440 px window the chips fold into "Filters (n)" (spec r1.1, as on
//! Heroes, [`chips_fold`]). The 0.5 sidebar
//! layout keeps its own Overview ([`crate::overview`]).
//!
//! Rank, queue, length and crew are spec example data for fields the
//! tracker does not store yet, so they are not drawn.

use iced::widget::{Column, Row, button, column, row, text};
use iced::{Alignment, Background, Border, Padding, Theme};
use iced::{Element, Fill, Task};

use crate::aggregate::{GameFilter, SessionGroup, filter_games, group_by_session_gap};
use crate::app::{Message, TrackerApp};
use crate::model::{Game, Role, RoleFilter, Screen};
use crate::overview_chips;
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

/// Page width below which the Overview is one column (spec p8, about 1024
/// of window).
pub const COLLAPSE_BELOW: f32 = 1000.0;

/// Window width from which every chip shows (spec r1.1: all chips at
/// 1600, "Filters (n)" at 1366 and 1024).
const CHIPS_OPEN_FROM: f32 = 1440.0;

/// The window a page `content` px wide sits in: the page gutter is wider
/// from [`theme::WIDE_GUTTER_MIN`] up.
pub fn window_for(content: f32) -> f32 {
    let wide = content + 2.0 * theme::PAGE_PAD_X;
    if wide >= theme::WIDE_GUTTER_MIN {
        wide
    } else {
        content + 2.0 * theme::PAGE_PAD_X_NARROW
    }
}

/// Whether a page `content` px wide folds its chips behind "Filters (n)".
pub fn chips_fold(content: f32) -> bool {
    content.is_finite() && window_for(content) < CHIPS_OPEN_FROM
}

/// The chip row: every chip when there is room; otherwise "Filters (n)"
/// and the chips that filter, the rest while it is open. Heroes uses it
/// too.
pub(crate) fn chip_row<'a>(
    app: &'a TrackerApp,
    heroes: &[String],
    maps: &[String],
    narrow: bool,
) -> Element<'a, Message> {
    chip_row_except(app, heroes, maps, narrow, overview_chips::Hidden::default())
}

/// [`chip_row`] without the chips a page does not apply (`hide`), so none
/// shows or counts in "Filters (n)" without filtering anything.
pub(crate) fn chip_row_except<'a>(
    app: &'a TrackerApp,
    heroes: &[String],
    maps: &[String],
    narrow: bool,
    hide: overview_chips::Hidden,
) -> Element<'a, Message> {
    if !narrow {
        return overview_chips::bar_except(app, heroes, maps, true, hide);
    }
    let open = app.overview_page.filters_open;
    Row::new()
        .spacing(8)
        .align_y(Alignment::Start)
        .push(fold_chip(
            overview_chips::active_count_except(app, hide),
            open,
            Message::OverviewPage(Msg::ToggleFilters),
        ))
        .push(overview_chips::bar_except(app, heroes, maps, open, hide))
        .into()
}

/// A spec arrow (▾ ▴ ▲ ▼) in iced's own icon font: the page font has
/// none of them. Any other character is drawn as it is.
pub fn glyph(c: char) -> iced::widget::Text<'static> {
    use iced::advanced::text::Renderer as _;
    let icon = match c {
        '▾' => Some(iced::Renderer::ARROW_DOWN_ICON),
        '▴' | '▲' => Some(iced::Renderer::SCROLL_UP_ICON),
        '▼' => Some(iced::Renderer::SCROLL_DOWN_ICON),
        _ => None,
    };
    match icon {
        Some(i) => text(i.to_string()).font(iced::Renderer::ICON_FONT),
        None => text(c.to_string()),
    }
}

/// "Filters (1) ▾": opens or shuts the folded chips (▴ while open).
fn fold_chip(count: usize, open: bool, toggle: Message) -> Element<'static, Message> {
    button(
        row![
            text(format!("Filters ({count})"))
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
            glyph(if open { '▴' } else { '▾' })
                .size(SIZE_META)
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

/// The gap that splits sessions: the Session window setting, as the spec's
/// Games footnote says ("Sessions split after 3 hours without a game").
/// The 0.5 Games page still splits at a fixed 30 minutes. A value too big
/// for a duration (the field takes any number) is no limit at all.
pub fn session_gap(window_secs: u64) -> chrono::TimeDelta {
    i64::try_from(window_secs)
        .ok()
        .and_then(chrono::TimeDelta::try_seconds)
        .unwrap_or(chrono::TimeDelta::MAX)
}

/// `games` less the one still being played: its result is not in, so the
/// season card and the heroes strip leave it out instead of counting it
/// as undecided. The session card counts it as Playing
/// ([`crate::overview_main::session_title`]) and the newest-game card
/// shows it as Playing.
pub fn settled<'a>(app: &TrackerApp, games: &[&'a Game]) -> Vec<&'a Game> {
    settled_of(games.iter().copied(), app.live_game_session.as_deref())
}

/// [`settled`] for any games, given the session being played.
pub fn settled_of<'a>(
    games: impl IntoIterator<Item = &'a Game>,
    live: Option<&str>,
) -> Vec<&'a Game> {
    games
        .into_iter()
        .filter(|g| !crate::model::game_card_in_progress(&g.session_id, g.outcome, live))
        .collect()
}

/// Whether the filter row shows: not before the first game (ux.md
/// section 17), when there is nothing to filter, and not while the game
/// list is still loading ([`crate::games_state::loading`]).
pub fn shows_filters(app: &TrackerApp) -> bool {
    !crate::games_state::loading(app) && !app.games.is_empty()
}

/// The latest session of `games` (newest first), split by [`session_gap`].
pub fn latest_session(app: &TrackerApp, games: &[&Game]) -> Option<SessionGroup> {
    group_by_session_gap(games, session_gap(app.saved_config.session_window_secs))
        .into_iter()
        .next()
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
    // No filter row before the first game (ux.md section 17).
    if shows_filters(app) {
        page = page.push(chip_row(app, &heroes, &maps, chips_fold(content_width)));
    }

    let main_w = if narrow {
        content_width
    } else {
        content_width - SIDE_W - GAP
    };
    let session = latest_session(app, &shown);
    let settled_games = settled(app, &shown);
    let main = column![
        crate::overview_main::view(app, session.as_ref()),
        crate::overview_heroes::view(app, &settled_games, main_w),
    ]
    .spacing(GAP)
    .width(Fill);
    let side = crate::overview_side::view(app, &settled_games);
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
            crate::overview_chips::single_role(app.roles),
            Some(Role::Tank)
        );
        let _ = app.update(Message::OverviewPage(Msg::PickRole(None)));
        assert!(app.roles.unrestricted());
    }

    #[test]
    fn the_game_being_played_is_left_out_of_the_counts() {
        let (_dir, mut app) = crate::fixtures::test_app();
        let mut games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        games[0].outcome = crate::model::Outcome::Unknown;
        app.live_game_session = Some(games[0].session_id.clone());
        let refs: Vec<&Game> = games.iter().collect();
        let kept = settled(&app, &refs);
        assert_eq!(kept.len(), games.len() - 1);
        assert!(kept.iter().all(|g| g.session_id != games[0].session_id));
        app.live_game_session = None;
        assert_eq!(
            settled(&app, &refs).len(),
            games.len(),
            "a closed game counts"
        );
    }

    #[test]
    fn a_huge_session_window_never_panics() {
        assert_eq!(session_gap(u64::MAX), chrono::TimeDelta::MAX);
        assert_eq!(session_gap(i64::MAX as u64), chrono::TimeDelta::MAX);
        assert_eq!(session_gap(1800), chrono::TimeDelta::minutes(30));
        let (_dir, mut app) = crate::fixtures::test_app();
        app.saved_config.session_window_secs = u64::MAX;
        let _ = view(&app, 1300.0);
    }

    #[test]
    fn chips_fold_below_a_1440_window() {
        assert!(!chips_fold(1600.0 - 64.0));
        assert!(chips_fold(1366.0 - 48.0));
        assert!(chips_fold(1024.0 - 48.0));
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
        app.games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        app.filter_outcome = Some(crate::model::Outcome::Win);
        assert_eq!(filter(&app).outcome, Some(crate::model::Outcome::Win));
        assert!(shows_filters(&app));
        // Folded at 1300 px of page, open at 1600, and one column at 900.
        assert!(chips_fold(1300.0));
        assert!(!chips_fold(1600.0));
        for width in [1300.0, 1600.0, 900.0] {
            let _ = view(&app, width);
        }
    }

    #[test]
    fn the_filter_row_waits_for_the_first_game() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert!(app.games.is_empty());
        assert!(!shows_filters(&app));
        app.games = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ));
        assert!(shows_filters(&app));
    }
}
