//! The 0.6 filter chips under a page heading (spec p1 and p2): Season,
//! roles, Hero and Map. A chip that filters reads as an accent outline
//! with its value and "×" to clear it ("Season 5 ×"). One that does not
//! is a dropdown ("Hero ▾"). Overview draws them; Games and Heroes take
//! the same row. Below [`COLLAPSE_BELOW`] of page the row folds into
//! "Filters (n) ▾" plus the chips that filter (spec p8).
//!
//! The spec's "All queues" and "Crew: anyone" chips are left out until
//! the tracker stores a queue and a crew for each game.

use iced::widget::{Row, button, pick_list, row, text};
use iced::{Alignment, Background, Border, Element, Padding, Theme};

use crate::app::{Message, TrackerApp};
use crate::model::{Role, RoleFilter, SeasonSel};
use crate::theme::{self, ACCENT, BORDER, FONT_SEMIBOLD, SIZE_META, SURFACE, TEXT, TEXT_2};

/// A season the Season dropdown offers.
#[derive(Debug, Clone, PartialEq, Eq)]
struct SeasonPick {
    id: String,
    name: String,
}

impl std::fmt::Display for SeasonPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.name)
    }
}

/// A hero or map the dropdowns offer.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Pick {
    value: String,
    label: String,
}

impl std::fmt::Display for Pick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

/// The role a single-role filter holds, if it holds exactly one.
pub fn single_role(roles: RoleFilter) -> Option<Role> {
    let on: Vec<Role> = [Role::Tank, Role::Damage, Role::Support]
        .into_iter()
        .filter(|r| roles.is_on(*r))
        .collect();
    match on.as_slice() {
        [one] => Some(*one),
        _ => None,
    }
}

/// Page width below which the row folds (spec p8, about 1024 of window).
pub const COLLAPSE_BELOW: f32 = 1000.0;

/// How the row is drawn: every chip, or folded behind "Filters (n) ▾"
/// with the fold open or shut. `toggle` opens or shuts it.
#[derive(Debug, Clone)]
pub enum Fold {
    Wide,
    Narrow { open: bool, toggle: Message },
}

impl Fold {
    /// The fold for a page `content` px wide.
    pub fn for_width(content: f32, open: bool, toggle: Message) -> Self {
        if content.is_finite() && content < COLLAPSE_BELOW {
            Fold::Narrow { open, toggle }
        } else {
            Fold::Wide
        }
    }
}

/// How many filters are on: the "n" of "Filters (n)".
pub fn active_count(app: &TrackerApp) -> usize {
    [
        app.needs_look_filter && app.screen == crate::model::Screen::Games,
        matches!(app.season, SeasonSel::Season(_))
            && crate::seasons::show_season_picker(&app.seasons.seasons),
        !app.roles.unrestricted(),
        app.filter_hero.is_some(),
        app.filter_map.is_some(),
    ]
    .into_iter()
    .filter(|on| *on)
    .count()
}

/// The chip row. `heroes` and `maps` are the values the current season and
/// role leave, so a dropdown never offers a choice with no games. The row
/// also holds "Needs a look ×" while the top bar's filter is on.
pub fn bar<'a>(
    app: &'a TrackerApp,
    heroes: &[String],
    maps: &[String],
    fold: Fold,
) -> Element<'a, Message> {
    // Each chip with whether it filters now.
    let mut chips: Vec<(bool, Element<'a, Message>)> = Vec::new();
    // "Needs a look" filters Games only.
    if app.needs_look_filter && app.screen == crate::model::Screen::Games {
        chips.push((
            true,
            set_chip(
                "Needs a look".to_string(),
                Message::Nav(crate::nav::Msg::ClearNeedsLook),
            ),
        ));
    }
    if crate::seasons::show_season_picker(&app.seasons.seasons) {
        chips.push((matches!(app.season, SeasonSel::Season(_)), season_chip(app)));
    }
    chips.push((!app.roles.unrestricted(), role_chip(app.roles)));
    chips.push((
        app.filter_hero.is_some(),
        value_chip(
            "Hero",
            app.filter_hero.as_deref(),
            heroes,
            crate::model::display_hero_name,
            Message::FilterHero,
        ),
    ));
    chips.push((
        app.filter_map.is_some(),
        value_chip(
            "Map",
            app.filter_map.as_deref(),
            maps,
            |m: &str| m.to_string(),
            Message::FilterMap,
        ),
    ));
    let mut row = Row::new().spacing(8).align_y(Alignment::Center);
    match fold {
        Fold::Wide => {
            for (_, chip) in chips {
                row = row.push(chip);
            }
        }
        Fold::Narrow { open, toggle } => {
            row = row.push(fold_chip(active_count(app), open, toggle));
            for (on, chip) in chips {
                if on || open {
                    row = row.push(chip);
                }
            }
        }
    }
    row.wrap().into()
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

fn season_chip(app: &TrackerApp) -> Element<'_, Message> {
    if let SeasonSel::Season(id) = &app.season {
        let name = app
            .seasons
            .seasons
            .iter()
            .find(|s| &s.id == id)
            .map_or_else(|| "Season".to_string(), |s| s.name.clone());
        return set_chip(name, Message::SelectSeason(SeasonSel::AllTime));
    }
    let options: Vec<SeasonPick> = app
        .seasons
        .seasons
        .iter()
        .map(|s| SeasonPick {
            id: s.id.clone(),
            name: if s.is_current {
                format!("{} · current", s.name)
            } else {
                s.name.clone()
            },
        })
        .collect();
    dropdown_chip(options, "All time", |s: SeasonPick| {
        Message::SelectSeason(SeasonSel::Season(s.id))
    })
}

fn role_chip(roles: RoleFilter) -> Element<'static, Message> {
    match single_role(roles) {
        Some(role) => set_chip(
            role.label().to_string(),
            Message::OverviewPage(crate::overview_page::Msg::PickRole(None)),
        ),
        None if !roles.unrestricted() => set_chip(
            "Some roles".to_string(),
            Message::OverviewPage(crate::overview_page::Msg::PickRole(None)),
        ),
        None => dropdown_chip(
            vec![
                RolePick(Role::Tank),
                RolePick(Role::Damage),
                RolePick(Role::Support),
            ],
            "All roles",
            |r: RolePick| Message::OverviewPage(crate::overview_page::Msg::PickRole(Some(r.0))),
        ),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RolePick(Role);

impl std::fmt::Display for RolePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.label())
    }
}

fn value_chip(
    name: &'static str,
    current: Option<&str>,
    values: &[String],
    label: fn(&str) -> String,
    msg: fn(Option<String>) -> Message,
) -> Element<'static, Message> {
    if let Some(v) = current {
        return set_chip(label(v), msg(None));
    }
    let options: Vec<Pick> = values
        .iter()
        .map(|v| Pick {
            value: v.clone(),
            label: label(v),
        })
        .collect();
    dropdown_chip(options, name, move |p: Pick| msg(Some(p.value)))
}

/// A filter that is on: its value and "×", pressing clears it.
fn set_chip(label: String, clear: Message) -> Element<'static, Message> {
    button(
        row![
            text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT),
            text("×").size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT_2),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([5, 12]))
    .style(|_: &Theme, status| button::Style {
        background: Some(Background::Color(iced::Color {
            a: if matches!(status, button::Status::Hovered) {
                0.28
            } else {
                0.16
            },
            ..ACCENT
        })),
        text_color: TEXT,
        border: Border {
            color: ACCENT,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..button::Style::default()
    })
    .on_press(clear)
    .into()
}

/// A filter that is off: a chip-shaped dropdown with `placeholder`.
fn dropdown_chip<T>(
    options: Vec<T>,
    placeholder: &'static str,
    on_pick: impl Fn(T) -> Message + 'static,
) -> Element<'static, Message>
where
    T: ToString + PartialEq + Clone + 'static,
{
    pick_list(options, None::<T>, on_pick)
        .placeholder(placeholder)
        .text_size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .padding(Padding::from([5, 12]))
        .style(|_: &Theme, status| iced::widget::pick_list::Style {
            text_color: TEXT,
            placeholder_color: TEXT_2,
            handle_color: TEXT_2,
            background: Background::Color(SURFACE),
            border: Border {
                color: match status {
                    iced::widget::pick_list::Status::Active => BORDER,
                    _ => ACCENT,
                },
                width: 1.0,
                radius: theme::RADIUS_CHIP.into(),
            },
        })
        .menu_style(|_: &Theme| iced::overlay::menu::Style {
            background: Background::Color(SURFACE),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            text_color: TEXT,
            selected_text_color: TEXT,
            selected_background: Background::Color(iced::Color { a: 0.25, ..ACCENT }),
            shadow: iced::Shadow::default(),
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_fold_starts_below_the_spec_width() {
        let toggle = Message::Tick;
        assert!(matches!(
            Fold::for_width(1200.0, false, toggle.clone()),
            Fold::Wide
        ));
        assert!(matches!(
            Fold::for_width(976.0, true, toggle),
            Fold::Narrow { open: true, .. }
        ));
    }

    #[test]
    fn the_count_names_only_filters_that_are_on() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert_eq!(active_count(&app), 0);
        app.filter_hero = Some("Ana".into());
        app.needs_look_filter = true;
        assert_eq!(active_count(&app), 1, "Needs a look filters Games only");
        app.screen = crate::model::Screen::Games;
        assert_eq!(active_count(&app), 2);
    }

    #[test]
    fn one_role_on_is_a_single_role() {
        let none = RoleFilter::default();
        assert_eq!(single_role(none), None);
        let tank = none.toggle(Role::Tank);
        assert_eq!(single_role(tank), Some(Role::Tank));
        assert_eq!(single_role(tank.toggle(Role::Support)), None);
    }
}
