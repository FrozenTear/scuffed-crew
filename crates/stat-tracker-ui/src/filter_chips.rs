//! The 0.6 filter chips on the Games tab (spec p2), ported from #555:
//! Season, queues, roles, Crew, Hero and Map. A chip that filters reads as
//! an accent outline with its value and "×" to clear it ("Season 5 ×").
//! One that does not is a dropdown ("Hero" and its arrow). Roles are
//! multi-select: each role on is its own chip, and the dropdown adds
//! another. Behind "Filters (n)" only the chips that filter show; opening
//! it shows the dropdowns too. The messages are the app's own, so any tab
//! can use the chips.
//!
//! "All queues" and "Crew: anyone" are drawn disabled until the tracker
//! stores a queue and a crew for each game.

use iced::widget::{Row, button, container, pick_list, row, text};
use iced::{Alignment, Background, Border, Element, Padding, Theme};

use crate::app::{Message, TrackerApp};
use crate::model::{Outcome, Role, RoleFilter, SeasonSel};
use crate::theme::{self, ACCENT, BORDER, FONT_SEMIBOLD, SIZE_META, SURFACE, TEXT, TEXT_2, TEXT_3};

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

/// The roles the dropdown can still add, in the usual order.
pub fn roles_to_add(roles: RoleFilter) -> Vec<Role> {
    Role::all_playable()
        .into_iter()
        .filter(|r| !roles.is_on(*r))
        .collect()
}

/// Filters that are on: each is a "×" chip.
pub fn active_count(app: &TrackerApp) -> usize {
    usize::from(app.needs_look_filter)
        + usize::from(
            matches!(app.season, SeasonSel::Season(_))
                && crate::seasons::show_season_picker(&app.seasons.seasons),
        )
        + app.roles.selected_roles().len()
        + usize::from(app.filter_hero.is_some())
        + usize::from(app.filter_map.is_some())
        + usize::from(app.filter_outcome.is_some())
}

/// The chip row. `heroes` and `maps` are the values the current season and
/// role leave, so a dropdown never offers a choice with no games. With
/// `open` false only the chips that filter show. The row also holds
/// "Needs a look ×" while the top bar's filter is on.
pub fn bar<'a>(
    app: &'a TrackerApp,
    heroes: &[String],
    maps: &[String],
    open: bool,
) -> Element<'a, Message> {
    let mut chips = Row::new().spacing(8).align_y(Alignment::Center);
    if app.needs_look_filter {
        chips = chips.push(set_chip(
            "Needs a look".to_string(),
            Message::Nav(crate::nav::Msg::ClearNeedsLook),
        ));
    }
    let season_set = matches!(app.season, SeasonSel::Season(_));
    if crate::seasons::show_season_picker(&app.seasons.seasons) && (open || season_set) {
        chips = chips.push(season_chip(app));
    }
    if open {
        chips = chips.push(disabled_chip("All queues"));
    }
    for role in app.roles.selected_roles() {
        chips = chips.push(set_chip(
            role.label().to_string(),
            Message::ToggleRole(role),
        ));
    }
    let add = roles_to_add(app.roles);
    if open && !add.is_empty() {
        let label = if app.roles.unrestricted() {
            "All roles"
        } else {
            "Add role"
        };
        let picks = add.into_iter().map(RolePick).collect();
        chips = chips.push(dropdown_chip(picks, label, |r: RolePick| {
            Message::ToggleRole(r.0)
        }));
    }
    if open {
        chips = chips.push(disabled_chip("Crew: anyone"));
    }
    if open || app.filter_hero.is_some() {
        chips = chips.push(value_chip(
            "Hero",
            app.filter_hero.as_deref(),
            heroes,
            crate::model::display_hero_name,
            Message::FilterHero,
        ));
    }
    if open || app.filter_map.is_some() {
        chips = chips.push(value_chip(
            "Map",
            app.filter_map.as_deref(),
            maps,
            |m: &str| m.to_string(),
            Message::FilterMap,
        ));
    }
    if let Some(outcome) = app.filter_outcome {
        chips = chips.push(set_chip(
            outcome.label().to_string(),
            Message::FilterOutcome(None),
        ));
    } else if open {
        let picks = [Outcome::Win, Outcome::Loss, Outcome::Draw]
            .into_iter()
            .map(OutcomePick)
            .collect();
        chips = chips.push(dropdown_chip(picks, "Any result", |o: OutcomePick| {
            Message::FilterOutcome(Some(o.0))
        }));
    }
    chips.wrap().into()
}

/// A filter the tracker has no data for yet: greyed, does nothing. The
/// arrow is the icon font's, as the page font has no "▾".
fn disabled_chip(label: &'static str) -> Element<'static, Message> {
    container(
        row![
            text(label)
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3),
            crate::settings_fixed::arrow().size(SIZE_META).color(TEXT_3),
        ]
        .spacing(6)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([5, 12]))
    .style(|_: &Theme| container::Style {
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        ..container::Style::default()
    })
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct RolePick(Role);

impl std::fmt::Display for RolePick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.0.label())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OutcomePick(Outcome);

impl std::fmt::Display for OutcomePick {
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
    fn the_role_dropdown_adds_the_roles_not_on() {
        let none = RoleFilter::default();
        assert_eq!(roles_to_add(none), Role::all_playable().to_vec());
        let tank = none.toggle(Role::Tank);
        assert_eq!(roles_to_add(tank), vec![Role::Damage, Role::Support]);
        let all = tank.toggle(Role::Damage).toggle(Role::Support);
        assert!(roles_to_add(all).is_empty());
    }
}
