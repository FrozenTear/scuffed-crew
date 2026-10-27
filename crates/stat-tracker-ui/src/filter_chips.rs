//! The 0.6 filter chips under a page heading (spec p1 and p2): Season,
//! roles, Hero and Map. A chip that filters reads as an accent outline
//! with its value and "×" to clear it ("Season 5 ×"). One that does not
//! is a dropdown ("Hero ▾"). Games draws them first; Overview and Heroes
//! take the same row in their own PRs.
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

/// The chip row. `heroes` and `maps` are the values the current season and
/// role leave, so a dropdown never offers a choice with no games. The row
/// also holds "Needs a look ×" while the top bar's filter is on.
pub fn bar<'a>(app: &'a TrackerApp, heroes: &[String], maps: &[String]) -> Element<'a, Message> {
    let mut chips = Row::new().spacing(8).align_y(Alignment::Center);
    if app.needs_look_filter {
        chips = chips.push(set_chip(
            "Needs a look".to_string(),
            Message::Nav(crate::nav::Msg::ClearNeedsLook),
        ));
    }
    if crate::seasons::show_season_picker(&app.seasons.seasons) {
        chips = chips.push(season_chip(app));
    }
    chips = chips.push(role_chip(app.roles));
    chips = chips.push(value_chip(
        "Hero",
        app.filter_hero.as_deref(),
        heroes,
        crate::model::display_hero_name,
        |v| Message::FilterHero(v),
    ));
    chips = chips.push(value_chip(
        "Map",
        app.filter_map.as_deref(),
        maps,
        |m: &str| m.to_string(),
        |v| Message::FilterMap(v),
    ));
    chips.wrap().into()
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
            Message::GamesPage(crate::games_page::Msg::PickRole(None)),
        ),
        None if !roles.unrestricted() => set_chip(
            "Some roles".to_string(),
            Message::GamesPage(crate::games_page::Msg::PickRole(None)),
        ),
        None => dropdown_chip(
            vec![
                RolePick(Role::Tank),
                RolePick(Role::Damage),
                RolePick(Role::Support),
            ],
            "All roles",
            |r: RolePick| Message::GamesPage(crate::games_page::Msg::PickRole(Some(r.0))),
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
    fn one_role_on_is_a_single_role() {
        let none = RoleFilter::default();
        assert_eq!(single_role(none), None);
        let tank = none.toggle(Role::Tank);
        assert_eq!(single_role(tank), Some(Role::Tank));
        assert_eq!(single_role(tank.toggle(Role::Support)), None);
    }
}
