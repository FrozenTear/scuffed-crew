//! App navigation for the 0.6 redesign (variant A: top tabs).
//!
//! The nav is picked once at start from [`NavStyle`], so a sidebar can be
//! swapped in for 0.7 without touching the screens. `SCUFFED_TRACKER_LAYOUT=classic`
//! keeps the 0.5 sidebar and the old Settings page as a fallback.
//!
//! The tab model ([`tabs`], [`needs_look_count`], [`bar_lines`]) is pure so
//! it can be tested without a window.

use iced::widget::{Row, button, column, container, row, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Length, Padding, Shadow, Theme};

use crate::app::{Message, TrackerApp};
use crate::model::{Game, Screen};
use crate::settings_model::needs_look_label;
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_SEMIBOLD, PAGE_PAD_X, SIZE_BODY,
    SIZE_LABEL, SIZE_META, SIZE_TITLE, TEXT, TEXT_2, WARN,
};
use crate::widgets;

/// Env var that picks the layout. `classic` keeps the 0.5 sidebar.
pub const LAYOUT_ENV: &str = "SCUFFED_TRACKER_LAYOUT";

/// Window content width where the top bar fits on one line. Below it the
/// title and status take the first line and the tabs the second.
pub const ONE_LINE_MIN: f32 = 1080.0;

/// Height of the selected tab underline.
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

    pub fn from_env() -> Self {
        Self::from_env_value(std::env::var(LAYOUT_ENV).ok().as_deref())
    }
}

/// One tab in the top bar.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Tab {
    pub screen: Screen,
    pub label: &'static str,
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
            label: screen.label(),
            selected: screen == current,
            badge: (screen == Screen::Games && needs_look > 0).then_some(needs_look),
        })
        .collect()
}

/// A game needs a look when a map or hero is missing or the reader marked
/// a field as unsure. Same rule as the amber marks on the Games cards.
pub fn needs_look(game: &Game) -> bool {
    game.needs_review() || !game.suspect_fields.is_empty()
}

pub fn needs_look_count(games: &[Game]) -> usize {
    games.iter().filter(|g| needs_look(g)).count()
}

/// 1 or 2 lines for the top bar at `width`.
pub fn bar_lines(width: f32) -> u8 {
    if width.is_finite() && width >= ONE_LINE_MIN {
        1
    } else {
        2
    }
}

/// Top bar: title, tabs, capture status and the needs-a-look chip.
pub fn top_bar(app: &TrackerApp, width: f32) -> Element<'_, Message> {
    let count = needs_look_count(&app.games);
    let title = text("Scuffed Tracker")
        .size(SIZE_TITLE)
        .font(FONT_EXTRABOLD)
        .color(TEXT);
    let strip = tab_strip(&tabs(app.screen, count));

    let mut status = Row::new().spacing(12).align_y(Alignment::Center);
    status = status.push(widgets::status_stub(
        &app.live_status,
        widgets::header_status_color(widgets::header_status_for(app)),
    ));
    if count > 0 {
        status = status.push(needs_look_chip(count));
    }

    let body: Element<'_, Message> = if bar_lines(width) == 1 {
        row![title, strip, space().width(Fill), status]
            .spacing(24)
            .align_y(Alignment::Center)
            .width(Fill)
            .into()
    } else {
        column![
            row![title, space().width(Fill), status]
                .spacing(16)
                .align_y(Alignment::Center)
                .width(Fill),
            strip,
        ]
        .spacing(4)
        .width(Fill)
        .into()
    };

    column![
        container(body).width(Fill).padding(Padding {
            top: 12.0,
            bottom: 0.0,
            left: PAGE_PAD_X,
            right: PAGE_PAD_X,
        }),
        container(space().height(1))
            .width(Fill)
            .height(1)
            .style(|_: &Theme| container::Style {
                background: Some(Background::Color(BORDER)),
                ..container::Style::default()
            }),
    ]
    .spacing(0)
    .width(Fill)
    .into()
}

/// Season, role and companion controls that sat under the old header. They
/// filter the stats screens, so Settings does not show them.
pub fn page_controls(app: &TrackerApp) -> Element<'_, Message> {
    let mut controls = Row::new().spacing(12).align_y(Alignment::Center);
    if crate::seasons::show_season_picker(&app.seasons.seasons) {
        controls = controls.push(widgets::season_switch(&app.seasons.seasons, &app.season));
    }
    controls = controls.push(widgets::role_chips(app.roles));
    controls = controls.push(
        row![
            space().width(8),
            widgets::overlay_toggle(app.overlay_showing(), app.game_running),
        ]
        .spacing(0)
        .align_y(Alignment::Center),
    );
    controls.wrap().into()
}

fn tab_strip(tabs: &[Tab]) -> Element<'static, Message> {
    let mut strip = Row::new().spacing(4).align_y(Alignment::End);
    for tab in tabs {
        strip = strip.push(tab_button(tab));
    }
    strip.into()
}

fn tab_button(tab: &Tab) -> Element<'static, Message> {
    let label = text(tab.label)
        .size(SIZE_BODY)
        .font(if tab.selected {
            FONT_EXTRABOLD
        } else {
            FONT_SEMIBOLD
        })
        .color(if tab.selected { TEXT } else { TEXT_2 });
    let mut inner = Row::new().spacing(8).align_y(Alignment::Center).push(label);
    if let Some(n) = tab.badge {
        inner = inner.push(count_badge(n));
    }
    let selected = tab.selected;
    // Fill in a shrink column takes the width of the label, so the
    // underline is exactly as wide as the tab. The column and the button
    // are set back to Shrink after the push: a Fill child would otherwise
    // make them Fill and every tab would get an equal slot.
    let underline = container(space().height(UNDERLINE))
        .width(Fill)
        .height(UNDERLINE)
        .style(move |_: &Theme| container::Style {
            background: selected.then_some(Background::Color(ACCENT)),
            ..container::Style::default()
        });
    button(
        column![
            container(inner).padding(Padding {
                top: 12.0,
                bottom: 10.0,
                left: 12.0,
                right: 12.0,
            }),
            underline,
        ]
        .spacing(0)
        .width(Length::Shrink),
    )
    .width(Length::Shrink)
    .padding(0)
    .style(tab_style)
    .on_press(Message::Navigate(tab.screen))
    .into()
}

fn tab_style(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: TEXT,
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

/// Amber outline count on the Games tab. Amber only means "needs a look".
fn count_badge(n: usize) -> Element<'static, Message> {
    container(
        text(n.to_string())
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(WARN),
    )
    .padding(Padding {
        top: 1.0,
        bottom: 1.0,
        left: 6.0,
        right: 6.0,
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

/// Header chip that opens Games, where the amber rows are.
fn needs_look_chip(n: usize) -> Element<'static, Message> {
    button(
        text(needs_look_label(n))
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(WARN),
    )
    .padding(Padding {
        top: 4.0,
        bottom: 4.0,
        left: 12.0,
        right: 12.0,
    })
    .style(|_: &Theme, _status| button::Style {
        background: None,
        text_color: WARN,
        border: Border {
            color: WARN,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    })
    .on_press(Message::Navigate(Screen::Games))
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{GameOcr, Outcome, Role};
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
    }

    #[test]
    fn tabs_follow_screen_order_with_one_selected() {
        for current in Screen::all() {
            let list = tabs(current, 0);
            let screens: Vec<Screen> = list.iter().map(|t| t.screen).collect();
            assert_eq!(screens, Screen::all().to_vec());
            assert_eq!(list.iter().filter(|t| t.selected).count(), 1);
            assert_eq!(list.iter().find(|t| t.selected).unwrap().screen, current);
            for t in &list {
                assert_eq!(t.label, t.screen.label());
            }
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
    fn needs_look_counts_missing_and_unsure_games() {
        let ready = game("Ana", "Busan");
        let no_map = game("Ana", "");
        let no_hero = game("Unknown", "Busan");
        let mut unsure = game("Kiriko", "Ilios");
        unsure.suspect_fields = vec!["dmg".into()];
        assert!(!needs_look(&ready));
        assert!(needs_look(&no_map));
        assert!(needs_look(&no_hero));
        assert!(needs_look(&unsure));
        assert_eq!(
            needs_look_count(&[ready.clone(), no_map, no_hero, unsure]),
            3
        );
        assert_eq!(needs_look_count(&[ready]), 0);
        assert_eq!(needs_look_count(&[]), 0);
    }

    #[test]
    fn bar_is_one_line_on_wide_windows() {
        assert_eq!(bar_lines(ONE_LINE_MIN), 1);
        assert_eq!(bar_lines(1920.0), 1);
        assert_eq!(bar_lines(ONE_LINE_MIN - 1.0), 2);
        assert_eq!(bar_lines(640.0), 2);
        assert_eq!(bar_lines(f32::NAN), 2);
    }
}
