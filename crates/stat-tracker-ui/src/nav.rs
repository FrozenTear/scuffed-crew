//! App navigation for the 0.6 redesign (variant A: top tabs).
//!
//! The nav is picked once at start from [`NavStyle`], so a sidebar can be
//! swapped in for 0.7 without touching the screens. `SCUFFED_TRACKER_LAYOUT=classic`
//! keeps the 0.5 sidebar and the old Settings page as a fallback.
//!
//! The tab model ([`tabs`], [`needs_look_count`], [`bar_status`]) is pure
//! so it can be tested without a window; how the bar fits a width is in
//! [`crate::nav_fit`]. The "need a look" chip sends [`Msg::ShowNeedsLook`],
//! handled here by [`update`].

use iced::widget::text::Wrapping;
use iced::widget::{Row, Stack, button, column, container, image, row, space, stack, text};
use iced::{
    Alignment, Background, Border, Color, Element, Fill, Length, Padding, Shadow, Task, Theme,
};

use crate::app::{Message, TrackerApp};
use crate::icons::{self, Icon};
use crate::model::{Game, RoleFilter, Screen, SeasonSel, game_card_in_progress};
use crate::nav_fit::{
    BADGE_PAD_X, CHEVRON, CHIP_GAP, CHIP_GLYPH_GAP, CHIP_PAD_X, DOT, DOT_GAP, ICON, ICON_GAP,
    MIN_GAP, SIZE_BADGE, SIZE_BRAND, SIZE_CHIP, SIZE_STATUS, SIZE_TAB, TAB_PAD, TAB_PAD_PLAIN,
    TAB_SPACING, TIME_GAP, TITLE_GAP, TITLE_GAP_WIDE, WARN_ICON,
};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_TITLE, TEXT,
    TEXT_2, TEXT_3, WARN,
};
use crate::tracker_status::{self, HeaderStatus};

/// Env var that picks the layout. `classic` keeps the 0.5 sidebar.
pub const LAYOUT_ENV: &str = "SCUFFED_TRACKER_LAYOUT";

/// The brand at the left of the bar.
pub const BRAND: &str = "Scuffed Tracker";

/// Window width (the bar spans the window) below which the tabs never
/// draw icons, as in the spec CSS. Above it they draw them when the whole
/// bar fits on one line with them ([`crate::nav_fit::layout`]).
pub const TAB_ICONS_MIN: f32 = 960.0;
/// Window width from which the status may say the most ("Tracker ready ·
/// last game 00:40"), when it fits. Below it, or when it does not fit, the
/// status is a short label such as "Ready".
pub const STATUS_LONG_MIN: f32 = 1200.0;

/// Page title under the bar (spec: about 24 px, extra bold).
pub const SIZE_HEADING: f32 = 24.0;

/// Height of the selected tab underline. It covers the bar's bottom
/// border and the pixel under it, as in the spec.
pub const UNDERLINE: f32 = 2.0;

/// Which navigation the window uses.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NavStyle {
    /// 0.6 default: tabs across the top, new Settings page.
    #[default]
    TopTabs,
    /// 0.5 layout: left sidebar, old Settings cards.
    ClassicSidebar,
}

impl NavStyle {
    /// Parse the env value. Anything other than `classic` is the default.
    pub fn from_env_value(value: Option<&str>) -> Self {
        match value.map(|v| v.trim().to_ascii_lowercase()).as_deref() {
            Some("classic") => NavStyle::ClassicSidebar,
            _ => NavStyle::TopTabs,
        }
    }

    /// A value that is set but names no layout (`classic`, `tabs`), which
    /// gives the default with a warning.
    pub fn is_unknown_value(value: Option<&str>) -> bool {
        value
            .map(|v| v.trim().to_ascii_lowercase())
            .is_some_and(|v| !v.is_empty() && v != "classic" && v != "tabs")
    }

    pub fn from_env() -> Self {
        let value = std::env::var(LAYOUT_ENV).ok();
        if Self::is_unknown_value(value.as_deref()) {
            tracing::warn!(
                value = value.as_deref().unwrap_or_default(),
                "{LAYOUT_ENV} is not classic or tabs; using the top tabs"
            );
        }
        Self::from_env_value(value.as_deref())
    }
}

/// A message from the top bar.
#[derive(Debug, Clone)]
pub enum Msg {
    /// The "need a look" chip: open Games with every counted game shown.
    ShowNeedsLook,
    /// Clear the "Needs a look" filter on Games.
    ClearNeedsLook,
}

/// What the chip sends. A later "Review games" button sends the same.
pub fn chip_message() -> Message {
    Message::Nav(Msg::ShowNeedsLook)
}

/// Handle a top bar message. [`Msg::ShowNeedsLook`] clears the hero, map,
/// outcome and role filters, picks All time and turns on the "Needs a
/// look" filter, so Games shows exactly the games the chip counted.
pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    match msg {
        Msg::ShowNeedsLook => {
            app.filter_hero = None;
            app.filter_map = None;
            app.filter_outcome = None;
            app.roles = RoleFilter::default();
            if app.season != SeasonSel::AllTime {
                app.season = SeasonSel::AllTime;
                if let Err(e) = crate::seasons::save_ui_state(&app.data_dir, &app.season) {
                    tracing::warn!(error = %e, "failed to write ui_state.json");
                }
            }
            app.needs_look_filter = true;
            app.update(Message::Navigate(Screen::Games))
        }
        Msg::ClearNeedsLook => {
            app.needs_look_filter = false;
            Task::none()
        }
    }
}

/// One tab in the top bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub screen: Screen,
    pub selected: bool,
    /// Count shown beside the label. Only Games has one, and only when
    /// some game needs a look.
    pub badge: Option<usize>,
}

/// Tabs in screen order with the current one selected.
pub fn tabs(current: Screen, needs_look: usize) -> Vec<Tab> {
    Screen::all()
        .into_iter()
        .map(|screen| Tab {
            screen,
            selected: screen == current,
            badge: (screen == Screen::Games && needs_look > 0).then_some(needs_look),
        })
        .collect()
}

/// Whether `game` counts in the chip and shows under its filter: the rule
/// is [`Game::needs_look`], with the open session (`live`) not counted for
/// its missing result.
pub fn counts_as_needs_look(game: &Game, live: Option<&str>) -> bool {
    game.needs_look(game_card_in_progress(&game.session_id, game.outcome, live))
}

/// Games that need a look across all seasons and roles (the chip's count).
pub fn needs_look_count(games: &[Game], live: Option<&str>) -> usize {
    games
        .iter()
        .filter(|g| counts_as_needs_look(g, live))
        .count()
}

/// Copy for the header chip.
pub fn needs_look_label(count: usize) -> String {
    if count == 1 {
        "1 needs a look".to_string()
    } else {
        format!("{count} need a look")
    }
}

/// The chip's words: the full label, or just the count when the bar is
/// short of room.
pub fn chip_text(count: usize, compact: bool) -> String {
    if compact {
        count.to_string()
    } else {
        needs_look_label(count)
    }
}

/// The status words in the top bar and the time, drawn as its own span.
/// Wide windows say what the tracker is doing and when the last game was
/// ("Tracker ready · last game" and "00:40"); otherwise it is a short
/// label and no time, never a bare time.
pub fn bar_status(
    status: HeaderStatus,
    last_game: Option<&str>,
    long: bool,
) -> (String, Option<String>) {
    if !long {
        return (tracker_status::short_label(status).to_string(), None);
    }
    let label = tracker_status::long_label(status);
    match last_game {
        Some(at) => (format!("{label} · last game"), Some(at.to_string())),
        None => (label.to_string(), None),
    }
}

/// Page title under the bar on every top tabs screen, Settings included.
pub fn page_heading(screen: Screen) -> Element<'static, Message> {
    text(screen.label())
        .size(SIZE_HEADING)
        .font(FONT_EXTRABOLD)
        .color(TEXT)
        .into()
}

/// The title a screen draws itself in the classic layout. The top tabs
/// layout draws [`page_heading`] above the filters instead, so this is
/// `None` there.
pub fn classic_title(app: &TrackerApp, screen: Screen) -> Option<Element<'static, Message>> {
    (app.nav_style == NavStyle::ClassicSidebar).then(|| {
        text(screen.label())
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT)
            .into()
    })
}

/// Top bar: title, tabs, status and the needs-a-look chip, on one line
/// when it fits ([`crate::nav_fit::layout`]), else on two.
pub fn top_bar(app: &TrackerApp, width: f32) -> Element<'_, Message> {
    let count = needs_look_count(&app.games, app.live_game_session.as_deref());
    let status = tracker_status::header_status_for(app);
    let last_game = tracker_status::last_game_when(&app.games, app.clock);
    let fit = crate::nav_fit::layout(width, status, last_game.as_deref(), count);
    let (label, time) = bar_status(status, last_game.as_deref(), fit.long_status);

    let mut right = Row::new().spacing(CHIP_GAP).align_y(Alignment::Center);
    right = right.push(bar_status_line(
        label,
        time,
        tracker_status::header_status_color(status),
    ));
    if count > 0 {
        right = right.push(needs_look_chip(count, fit.compact_chip));
    }
    let title = text(BRAND)
        .size(SIZE_BRAND)
        .font(FONT_EXTRABOLD)
        .color(TEXT)
        .wrapping(Wrapping::None);
    let strip = tab_strip(&tabs(app.screen, count), fit.icons);
    let title_gap = if fit.gutter > TITLE_GAP {
        TITLE_GAP_WIDE
    } else {
        TITLE_GAP
    };

    let body: Element<'_, Message> = if fit.one_line {
        row![
            title,
            space().width(title_gap),
            strip,
            space().width(Fill),
            right
        ]
        .align_y(Alignment::Center)
        .width(Fill)
        .into()
    } else {
        column![
            row![title, space().width(Fill), right]
                .spacing(MIN_GAP)
                .align_y(Alignment::Center)
                .width(Fill),
            strip,
        ]
        .spacing(4)
        .padding(Padding {
            top: 12.0,
            ..Padding::ZERO
        })
        .width(Fill)
        .into()
    };
    let body = container(body).width(Fill).padding(Padding {
        left: fit.gutter,
        right: fit.gutter,
        ..Padding::ZERO
    });
    // The border is a layer under the bar, one underline height above its
    // bottom, so the selected tab's underline is drawn over it.
    let border = column![
        space().height(Fill),
        container(space().height(1))
            .width(Fill)
            .height(1)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(BORDER)),
                ..container::Style::default()
            }),
        space().height(UNDERLINE - 1.0),
    ]
    .width(Fill)
    .height(Fill);
    Stack::new()
        .push(body)
        .push_under(border)
        .width(Fill)
        .into()
}

/// Dot, the status words in TEXT_3, and the time brighter in its own span.
fn bar_status_line(
    label: String,
    time: Option<String>,
    dot_color: Color,
) -> Element<'static, Message> {
    let mut words = Row::new().spacing(TIME_GAP).align_y(Alignment::Center);
    words = words.push(
        text(label)
            .size(SIZE_STATUS)
            .font(FONT_MEDIUM)
            .color(TEXT_3)
            .wrapping(Wrapping::None),
    );
    if let Some(time) = time {
        words = words.push(
            text(time)
                .size(SIZE_STATUS)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2)
                .wrapping(Wrapping::None),
        );
    }
    row![dot(dot_color), words]
        .spacing(DOT_GAP)
        .align_y(Alignment::Center)
        .into()
}

fn dot(color: Color) -> Element<'static, Message> {
    container(space().width(DOT).height(DOT))
        .width(DOT)
        .height(DOT)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 999.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

fn tab_strip(tabs: &[Tab], icons: bool) -> Element<'static, Message> {
    let mut strip = Row::new().spacing(TAB_SPACING).align_y(Alignment::End);
    for (i, tab) in tabs.iter().enumerate() {
        strip = strip.push(tab_button(tab, icons, i == 0));
    }
    strip.into()
}

/// 16 px stroke icon for a tab, as bright as its label
/// ([`crate::icons`]).
fn tab_icon(screen: Screen, selected: bool) -> Element<'static, Message> {
    image(icons::handle(Icon::Tab(screen, selected)))
        .width(ICON)
        .height(ICON)
        .into()
}

/// The chip's warning sign, drawn at 2x: Urbanist has no "⚠", and a
/// fallback font would draw it at another size and weight.
fn warn_icon() -> Element<'static, Message> {
    image(icons::handle(Icon::Warn))
        .width(WARN_ICON)
        .height(WARN_ICON)
        .into()
}

/// A tab. The side padding sits outside the column that holds the content
/// and the underline, so the underline is exactly as wide as the icon and
/// label (spec) while the whole padded tab stays the click target. The
/// first tab has no left padding: the title gap (one line) or the gutter
/// (two lines) is its room, so on two lines it lines up with the brand.
fn tab_button(tab: &Tab, icons: bool, first: bool) -> Element<'static, Message> {
    let mut inner = Row::new().spacing(ICON_GAP).align_y(Alignment::Center);
    if icons {
        inner = inner.push(tab_icon(tab.screen, tab.selected));
    }
    inner = inner.push(tab_label(tab.screen.label(), tab.selected));
    if let Some(n) = tab.badge {
        inner = inner.push(count_badge(n));
    }
    let selected = tab.selected;
    // Fill in a shrink column takes the width of the content row. The
    // column, its container and the button are set back to Shrink after
    // the push: a Fill child would otherwise make them Fill and every tab
    // would get an equal slot.
    let underline = container(space().height(UNDERLINE))
        .width(Fill)
        .height(UNDERLINE)
        .style(move |_: &Theme| container::Style {
            background: selected.then_some(Background::Color(ACCENT)),
            ..container::Style::default()
        });
    let content = column![
        container(inner).padding(Padding {
            top: 20.0,
            bottom: 16.0,
            ..Padding::ZERO
        }),
        underline,
    ]
    .spacing(0)
    .width(Length::Shrink);
    let pad = if icons { TAB_PAD } else { TAB_PAD_PLAIN };
    button(
        container(content)
            .padding(Padding {
                left: if first { 0.0 } else { pad },
                right: pad,
                ..Padding::ZERO
            })
            .width(Length::Shrink),
    )
    .width(Length::Shrink)
    .padding(0)
    .style(tab_style)
    .on_press(Message::Navigate(tab.screen))
    .into()
}

/// The label reserves its extra bold width under a transparent copy, so
/// picking a tab does not move its neighbours.
fn tab_label(label: &'static str, selected: bool) -> Element<'static, Message> {
    stack![
        text(label)
            .size(SIZE_TAB)
            .font(FONT_EXTRABOLD)
            .color(Color::TRANSPARENT)
            .wrapping(Wrapping::None),
        text(label)
            .size(SIZE_TAB)
            .font(if selected {
                FONT_EXTRABOLD
            } else {
                FONT_SEMIBOLD
            })
            .color(if selected { TEXT } else { TEXT_2 })
            .wrapping(Wrapping::None),
    ]
    .into()
}

/// Tabs show a light tint on hover and press, like the other controls.
fn tab_style(theme: &Theme, status: button::Status) -> button::Style {
    theme::quiet_btn(6.0)(theme, status)
}

/// Amber outline count on the Games tab. Amber only means "needs a look".
fn count_badge(n: usize) -> Element<'static, Message> {
    container(
        text(n.to_string())
            .size(SIZE_BADGE)
            .font(FONT_BOLD)
            .color(WARN)
            .wrapping(Wrapping::None),
    )
    .padding(Padding {
        top: 1.0,
        bottom: 1.0,
        left: BADGE_PAD_X,
        right: BADGE_PAD_X,
    })
    .style(|_: &Theme| container::Style {
        border: Border {
            color: WARN,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// "⚠ 159 need a look ›" (or "⚠ 159 ›" when short of room). Opens Games
/// with every counted game shown ([`Msg::ShowNeedsLook`]).
fn needs_look_chip(count: usize, compact: bool) -> Element<'static, Message> {
    // The words take the button's text colour, so WARN is set once.
    let content = row![
        warn_icon(),
        text(chip_text(count, compact))
            .size(SIZE_CHIP)
            .font(FONT_BOLD)
            .wrapping(Wrapping::None),
        text(CHEVRON).size(SIZE_CHIP).font(FONT_BOLD),
    ]
    .spacing(CHIP_GLYPH_GAP)
    .align_y(Alignment::Center);
    button(content)
        .padding(Padding {
            top: 6.0,
            bottom: 6.0,
            left: CHIP_PAD_X,
            right: CHIP_PAD_X,
        })
        .style(chip_style)
        .on_press(chip_message())
        .into()
}

/// Muted amber border (spec), an amber tint on hover and press.
fn chip_style(_theme: &Theme, status: button::Status) -> button::Style {
    let tint = match status {
        button::Status::Hovered => 0.12,
        button::Status::Pressed => 0.2,
        button::Status::Active | button::Status::Disabled => 0.0,
    };
    button::Style {
        background: (tint > 0.0).then_some(Background::Color(Color { a: tint, ..WARN })),
        text_color: WARN,
        border: Border {
            color: Color { a: 0.55, ..WARN },
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GameOcr, Outcome, Role};
    use crate::tracker_status::ALL;
    use chrono::Utc;

    fn game(hero: &str, map: &str) -> Game {
        Game {
            session_id: format!("{hero}-{map}"),
            hero: hero.into(),
            map_name: map.into(),
            role: Role::Support,
            outcome: Outcome::Win,
            elims: 1,
            deaths: 0,
            assists: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
            played_at: Utc::now(),
            edited: false,
            edited_fields: Vec::new(),
            ocr: GameOcr::default(),
            segments: Vec::new(),
            upload_reject: None,
            suspect_fields: Vec::new(),
            synced: false,
        }
    }

    fn app() -> (tempfile::TempDir, TrackerApp) {
        crate::fixtures::test_app()
    }

    #[test]
    fn env_picks_classic_only_when_asked() {
        assert_eq!(NavStyle::from_env_value(None), NavStyle::TopTabs);
        assert_eq!(NavStyle::from_env_value(Some("")), NavStyle::TopTabs);
        assert_eq!(NavStyle::from_env_value(Some("tabs")), NavStyle::TopTabs);
        assert_eq!(
            NavStyle::from_env_value(Some("classic")),
            NavStyle::ClassicSidebar
        );
        assert_eq!(
            NavStyle::from_env_value(Some(" Classic ")),
            NavStyle::ClassicSidebar
        );
        assert_eq!(NavStyle::default(), NavStyle::TopTabs);
        for quiet in [None, Some(""), Some(" "), Some("tabs"), Some("Classic")] {
            assert!(!NavStyle::is_unknown_value(quiet), "{quiet:?}");
        }
        for loud in [Some("sidebar"), Some("classik"), Some("0.5")] {
            assert!(NavStyle::is_unknown_value(loud), "{loud:?}");
            assert_eq!(NavStyle::from_env_value(loud), NavStyle::TopTabs);
        }
    }

    #[test]
    fn tabs_follow_the_spec_order_with_one_selected() {
        let spec = [
            (Screen::Overview, "Overview"),
            (Screen::Games, "Games"),
            (Screen::Heroes, "Heroes"),
            (Screen::Maps, "Maps"),
            (Screen::Seasons, "Seasons"),
            (Screen::Settings, "Settings"),
        ];
        for (current, _) in spec {
            let list = tabs(current, 0);
            let got: Vec<(Screen, &str)> =
                list.iter().map(|t| (t.screen, t.screen.label())).collect();
            assert_eq!(got, spec.to_vec());
            assert_eq!(list.iter().filter(|t| t.selected).count(), 1);
            assert_eq!(list.iter().find(|t| t.selected).unwrap().screen, current);
        }
    }

    #[test]
    fn only_games_carries_the_badge() {
        let list = tabs(Screen::Overview, 3);
        for t in &list {
            if t.screen == Screen::Games {
                assert_eq!(t.badge, Some(3));
            } else {
                assert_eq!(t.badge, None, "{:?}", t.screen);
            }
        }
        assert!(tabs(Screen::Overview, 0).iter().all(|t| t.badge.is_none()));
    }

    #[test]
    fn needs_look_copy_counts() {
        assert_eq!(needs_look_label(1), "1 needs a look");
        assert_eq!(needs_look_label(159), "159 need a look");
        assert_eq!(chip_text(159, false), "159 need a look");
        assert_eq!(chip_text(159, true), "159");
    }

    #[test]
    fn needs_look_counts_missing_fields_and_closed_games_without_a_result() {
        let ready = game("Ana", "Busan");
        let no_map = game("Ana", "");
        let no_hero = game("Unknown", "Busan");
        let mut unsure = game("Kiriko", "Ilios");
        unsure.suspect_fields = vec!["dmg".into()];
        let mut undecided = game("Mercy", "Oasis");
        undecided.outcome = Outcome::Unknown;
        let mut live = game("Lucio", "Nepal");
        live.outcome = Outcome::Unknown;
        let open = Some(live.session_id.as_str());

        assert!(!counts_as_needs_look(&ready, open));
        assert!(counts_as_needs_look(&no_map, open));
        assert!(counts_as_needs_look(&no_hero, open));
        assert!(
            !counts_as_needs_look(&unsure, open),
            "unsure fields alone have their own marks"
        );
        assert!(counts_as_needs_look(&undecided, open), "closed, no result");
        assert!(
            !counts_as_needs_look(&live, open),
            "the open game has no result yet"
        );
        assert!(
            counts_as_needs_look(&live, None),
            "once it closes without one it counts"
        );
        let all = [
            ready.clone(),
            no_map,
            no_hero,
            unsure,
            undecided,
            live.clone(),
        ];
        assert_eq!(needs_look_count(&all, Some(live.session_id.as_str())), 3);
        assert_eq!(needs_look_count(&all, None), 4);
        assert_eq!(needs_look_count(&[ready], None), 0);
        assert_eq!(needs_look_count(&[], None), 0);
    }

    #[test]
    fn status_is_long_only_when_asked_and_never_a_bare_time() {
        assert_eq!(
            bar_status(HeaderStatus::Waiting, Some("00:40"), true),
            (
                "Tracker ready · last game".to_string(),
                Some("00:40".to_string())
            )
        );
        assert_eq!(
            bar_status(HeaderStatus::Waiting, None, true),
            ("Tracker ready".to_string(), None)
        );
        assert_eq!(
            bar_status(HeaderStatus::Capturing, Some("00:40"), true).0,
            "Watching Overwatch · last game"
        );
        assert_eq!(
            bar_status(HeaderStatus::TokenRejected, Some("00:40"), true).0,
            "Website token rejected · last game"
        );
        let short = [
            (HeaderStatus::Capturing, "In game"),
            (HeaderStatus::Waiting, "Ready"),
            (HeaderStatus::Down, "Stopped"),
            (HeaderStatus::TokenRejected, "Token rejected"),
        ];
        for (status, word) in short {
            assert_eq!(
                bar_status(status, Some("00:40"), false),
                (word.to_string(), None)
            );
        }
        for status in ALL {
            for long in [false, true] {
                let (label, time) = bar_status(status, Some("00:40"), long);
                assert!(!label.contains("00:40"), "the time is its own span");
                assert_eq!(time.is_some(), long);
            }
        }
    }

    #[test]
    fn the_chip_opens_games_with_every_counted_game_shown() {
        let (_dir, mut app) = app();
        let mut games = Vec::new();
        for i in 0..12i64 {
            let mut g = game(if i % 3 == 0 { "Unknown" } else { "Ana" }, "Busan");
            g.session_id = format!("s{i}");
            g.role = if i % 2 == 0 {
                Role::Tank
            } else {
                Role::Support
            };
            g.outcome = if i % 4 == 1 {
                Outcome::Unknown
            } else {
                Outcome::Loss
            };
            g.played_at = Utc::now() - chrono::Duration::days(i * 40);
            games.push(g);
        }
        app.games = games;
        app.filter_hero = Some("Ana".into());
        app.filter_map = Some("Busan".into());
        app.filter_outcome = Some(Outcome::Win);
        app.roles = app.roles.toggle(Role::Damage);
        app.season = SeasonSel::Season("old".into());
        app.screen = Screen::Settings;
        let counted = needs_look_count(&app.games, app.live_game_session.as_deref());
        assert!(counted > 0);

        let _ = app.update(chip_message());
        assert_eq!(app.screen, Screen::Games);
        assert!(app.needs_look_filter);
        let shown = crate::games::shown_games(&app);
        assert_eq!(shown.len(), counted);
        for g in &app.games {
            if counts_as_needs_look(g, app.live_game_session.as_deref()) {
                assert!(shown.iter().any(|s| s.session_id == g.session_id));
            }
        }

        let _ = app.update(Message::Nav(Msg::ClearNeedsLook));
        assert!(!app.needs_look_filter);
        assert_eq!(crate::games::shown_games(&app).len(), app.games.len());
    }

    #[test]
    fn the_bar_builds_at_every_width_and_count() {
        let (_dir, mut app) = app();
        for count in [0usize, 1, 159] {
            app.games = (0..count)
                .map(|i| {
                    let mut g = game("Ana", "Busan");
                    g.session_id = format!("s{i}");
                    g.outcome = Outcome::Unknown;
                    g
                })
                .collect();
            app.live_game_session = None;
            assert_eq!(needs_look_count(&app.games, None), count);
            for screen in Screen::all() {
                app.screen = screen;
                for width in (720..=1600).step_by(40) {
                    let _ = top_bar(&app, width as f32);
                }
                let _ = page_heading(screen);
            }
        }
        assert!(classic_title(&app, Screen::Games).is_none());
        app.nav_style = NavStyle::ClassicSidebar;
        assert!(classic_title(&app, Screen::Games).is_some());
    }
}
