//! The Overview's heroes strip (spec p1): "HEROES · SEASON 5 · 3 OR MORE
//! GAMES", up to four cards, and "+ 3 under 3 games · All heroes ›".
//!
//! Small-sample rule: a hero shows its win rate with the shared bar
//! ([`crate::winrate_bar`]) only from 10 games. From 3 to 9 games it shows
//! its record ("2-5") with one dash per game out of ten. Under 3 games it
//! is not shown, only counted in the link.

use iced::widget::{Column, Row, button, column, container, row, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Theme};

use crate::aggregate::{HeroAgg, aggregate_filtered};
use crate::app::{Message, TrackerApp};
use crate::model::Screen;
use crate::overview_main::{portrait, role_label};
use crate::overview_side::{RATE_MIN, rate};
use crate::theme::{
    self, BORDER, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META,
    TEXT, TEXT_2, TEXT_3,
};
use crate::widgets;

/// Fewest games a hero needs for a card.
pub const CARD_MIN: usize = 3;
/// Dashes on a record card: one per game, out of [`RATE_MIN`].
const DASHES: usize = RATE_MIN;
/// Cards across: four on a wide page, two on a narrow one.
const WIDE_FROM: f32 = 760.0;

/// Heroes with a card, and how many are left out for having too few
/// games. Unknown heroes never get a card.
pub fn split(heroes: &[HeroAgg]) -> (Vec<&HeroAgg>, usize) {
    let known = heroes.iter().filter(|h| {
        let n = h.hero.trim();
        !n.is_empty() && !n.eq_ignore_ascii_case("unknown")
    });
    let (cards, small): (Vec<&HeroAgg>, Vec<&HeroAgg>) =
        known.partition(|h| h.record.games >= CARD_MIN);
    (cards, small.len())
}

pub fn view<'a>(app: &'a TrackerApp, width: f32) -> Element<'a, Message> {
    let stats = aggregate_filtered(&app.games, &crate::overview_page::filter(app));
    let (cards, small) = split(&stats.heroes);
    let scope = match &app.season {
        crate::model::SeasonSel::AllTime => "All time".to_string(),
        crate::model::SeasonSel::Season(id) => app
            .seasons
            .seasons
            .iter()
            .find(|s| &s.id == id)
            .map_or_else(|| "Season".to_string(), |s| s.name.clone()),
    };
    let mut link = String::new();
    if small > 0 {
        link.push_str(&format!("+ {small} under {CARD_MIN} games · "));
    }
    link.push_str("All heroes ›");
    let header = row![
        widgets::label_text(&format!("Heroes · {scope} · {CARD_MIN} or more games")),
        space().width(Fill),
        button(text(link).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT_2))
            .padding(0)
            .style(theme::quiet_btn(4.0))
            .on_press(Message::Navigate(Screen::Heroes)),
    ]
    .align_y(Alignment::Center);
    let col = Column::new().spacing(8).push(header);
    if cards.is_empty() {
        return col
            .push(widgets::empty_surface(&format!(
                "No hero has {CARD_MIN} or more games in these filters yet."
            )))
            .into();
    }
    let across = if width >= WIDE_FROM { 4 } else { 2 };
    let mut r = Row::new().spacing(crate::overview_page::GAP).width(Fill);
    for h in cards.iter().take(across) {
        r = r.push(card(h));
    }
    for _ in cards.len().min(across)..across {
        r = r.push(space().width(Fill));
    }
    col.push(r).into()
}

fn card(h: &HeroAgg) -> Element<'static, Message> {
    let mut head = column![].spacing(2);
    if let Some(label) = role_label(h.role) {
        head = head.push(
            text(label)
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(theme::role_color(h.role)),
        );
    }
    head = head.push(
        text(crate::model::display_hero_name(&h.hero))
            .size(SIZE_META)
            .font(FONT_BOLD)
            .color(TEXT),
    );
    let (big, meter): (String, Element<'static, Message>) = match rate(&h.record) {
        Some(p) => (format!("{p}%"), crate::winrate_bar::bar(p as f32)),
        None => (h.record.wl_label(), dashes(h.record.games)),
    };
    container(
        column![
            row![portrait(36.0), head]
                .spacing(10)
                .align_y(Alignment::Center),
            text(big).size(24).font(FONT_EXTRABOLD).color(TEXT),
            meter,
            text(h.record.games_label())
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        ]
        .spacing(8),
    )
    .padding(Padding::from([14, 16]))
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

fn block(color: iced::Color, width: iced::Length, height: f32) -> Element<'static, Message> {
    container(space().height(height))
        .width(width)
        .height(height)
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(color)),
            border: Border {
                radius: 1.0.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

/// One dash per game out of ten: played dashes in grey, the rest faint.
fn dashes(games: usize) -> Element<'static, Message> {
    let mut r = Row::new().spacing(4).width(Fill).align_y(Alignment::Center);
    for i in 0..DASHES {
        let color = if i < games {
            TEXT_3
        } else {
            iced::Color { a: 0.5, ..BORDER }
        };
        r = r.push(block(color, iced::Length::Fill, 3.0));
    }
    r.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::aggregate::Record;
    use crate::model::Role;

    fn hero(name: &str, games: usize, wins: usize, losses: usize) -> HeroAgg {
        HeroAgg {
            hero: name.into(),
            role: Role::Tank,
            record: Record {
                games,
                wins,
                losses,
                draws: 0,
            },
        }
    }

    #[test]
    fn heroes_under_three_games_are_only_counted() {
        let heroes = [
            hero("Wrecking Ball", 88, 45, 41),
            hero("D.Va", 7, 2, 5),
            hero("Orisa", 2, 1, 1),
            hero("Unknown", 9, 0, 0),
        ];
        let (cards, small) = split(&heroes);
        assert_eq!(cards.len(), 2);
        assert_eq!(small, 1);
    }

    #[test]
    fn rates_start_at_ten_games() {
        assert_eq!(rate(&hero("A", 9, 5, 4).record), None);
        assert_eq!(rate(&hero("A", 10, 5, 4).record), Some(56));
        assert_eq!(rate(&hero("A", 88, 45, 41).record), Some(52));
    }

    #[test]
    fn the_strip_draws_on_an_empty_demo() {
        let (_dir, app) = crate::fixtures::test_app();
        let _ = view(&app, 900.0);
    }
}
