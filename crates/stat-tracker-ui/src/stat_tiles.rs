//! The six flat stat tiles (spec p1 and p2): E, D, A, DMG, HEAL and MIT,
//! each with its label and value, and an amber outline with "⚠ UNSURE" on
//! a value the reader was not sure of and no hand edit has fixed since.
//!
//! One shared component for the Games expanded row and the Overview's
//! newest game. Callers use [`row`] only, so Tracker's shared tiles from
//! the Games PR can replace this module whole.

use iced::widget::text::Wrapping;
use iced::widget::{Row, column, container, image, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, Padding, Theme};

use crate::app::Message;
use crate::icons::{self, Icon};
use crate::model::Game;
use crate::theme::{self, BORDER, FONT_BOLD, FONT_MEDIUM, SIZE_LABEL, TEXT, TEXT_2, TEXT_3};
use stat_tracker::suspect::{flat_suspect_name, suspect_names_for_edit};

/// The six stat tiles: label, the field the reader marks, and the value.
const TILES: [(&str, &str); 6] = [
    ("E", "elims"),
    ("D", "deaths"),
    ("A", "assists"),
    ("DMG", "damage"),
    ("HEAL", "healing"),
    ("MIT", "mitigation"),
];

/// The six tiles for `g`, wrapping on a narrow page.
pub fn row(g: &Game) -> Element<'static, Message> {
    let mut tiles = Row::new().spacing(8);
    for (label, field) in TILES {
        tiles = tiles.push(tile(label, stat(g, field), unsure_now(g, field)));
    }
    tiles.wrap().into()
}

/// The flat suspect names still standing on `g`, once each: every stored
/// name (`dmg`, or an older row's `r3.dmg`) flattened by
/// [`flat_suspect_name`], less the ones a hand edit settled
/// ([`suspect_names_for_edit`] of each edited field).
pub fn still_unsure(g: &Game) -> Vec<&str> {
    let settled: Vec<&str> = g
        .edited_fields
        .iter()
        .flat_map(|e| suspect_names_for_edit(e).iter().copied())
        .collect();
    let mut out: Vec<&str> = Vec::new();
    for name in &g.suspect_fields {
        if let Some(flat) = flat_suspect_name(name)
            && !settled.contains(&flat)
            && !out.contains(&flat)
        {
            out.push(flat);
        }
    }
    out
}

/// The stored field `storage_field` (`damage`, `elims`, `map_name`, ...)
/// is still unsure: the reader marked it and no hand edit has settled it.
pub fn unsure_now(g: &Game, storage_field: &str) -> bool {
    let names = suspect_names_for_edit(storage_field);
    still_unsure(g).iter().any(|n| names.contains(n))
}

fn stat(g: &Game, field: &str) -> u32 {
    match field {
        "elims" => g.elims,
        "deaths" => g.deaths,
        "assists" => g.assists,
        "damage" => g.damage,
        "healing" => g.healing,
        _ => g.mitigation,
    }
}

/// "8.9k" from 1000 up.
pub fn short_num(n: u32) -> String {
    if n >= 1000 {
        format!("{:.1}k", f64::from(n) / 1000.0)
    } else {
        n.to_string()
    }
}

/// What a tile can carry besides its label and value: a small badge in
/// its head (`"EDITED"`), extra one-line notes under the value in their
/// own colours (they never wrap), and `fill` to take the row's width
/// instead of hugging the value.
#[derive(Debug, Clone, Default)]
pub struct TileExtras {
    pub badge: Option<&'static str>,
    pub lines: Vec<(String, Color)>,
    pub fill: bool,
}

/// A stat tile; an unsure one has an amber outline and "⚠ UNSURE".
fn tile(label: &'static str, value: u32, unsure: bool) -> Element<'static, Message> {
    tile_with(label, short_num(value), unsure, TileExtras::default())
}

/// A stat tile with `extras` ([`TileExtras`]). The Games expanded row and
/// the Overview's newest game both draw their tiles with this.
pub fn tile_with(
    label: &'static str,
    value: String,
    unsure: bool,
    extras: TileExtras,
) -> Element<'static, Message> {
    let mut head = Row::new().spacing(4).align_y(Alignment::Center).push(
        text(label)
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(if unsure { theme::WARN } else { TEXT_3 }),
    );
    if unsure {
        head = head
            .push(image(icons::handle(Icon::Warn)).width(10).height(10))
            .push(
                text("UNSURE")
                    .size(SIZE_LABEL)
                    .font(FONT_BOLD)
                    .color(theme::WARN),
            );
    }
    if let Some(badge) = extras.badge {
        head = head.push(
            container(text(badge).size(SIZE_LABEL).font(FONT_BOLD).color(TEXT_2))
                .padding(Padding::from([0, 6]))
                .style(|_: &Theme| container::Style {
                    border: Border {
                        color: BORDER,
                        width: 1.0,
                        radius: theme::RADIUS_CHIP.into(),
                    },
                    ..container::Style::default()
                }),
        );
    }
    let mut body = column![head, text(value).size(22).font(FONT_BOLD).color(TEXT)].spacing(4);
    for (line, color) in extras.lines {
        body = body.push(
            text(line)
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(color)
                .wrapping(Wrapping::None),
        );
    }
    let mut tile = container(body)
        .padding(Padding::from([10, 14]))
        .style(move |_: &Theme| container::Style {
            background: Some(Background::Color(theme::BG)),
            border: Border {
                color: if unsure { theme::WARN } else { BORDER },
                width: 1.0,
                radius: 10.0.into(),
            },
            ..container::Style::default()
        });
    if extras.fill {
        tile = tile.width(Fill);
    }
    tile.into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn extras_default_to_a_plain_tile() {
        let plain = TileExtras::default();
        assert!(plain.badge.is_none() && plain.lines.is_empty() && !plain.fill);
        let _ = tile_with(
            "DMG",
            "8.9k".into(),
            true,
            TileExtras {
                badge: Some("EDITED"),
                lines: vec![("OCR 8,912 → 9,912".into(), theme::WARN)],
                fill: true,
            },
        );
    }

    #[test]
    fn numbers_shorten_from_a_thousand() {
        assert_eq!(short_num(8_912), "8.9k");
        assert_eq!(short_num(28), "28");
    }

    #[test]
    fn a_hand_edit_settles_an_unsure_tile() {
        let mut g = crate::snapshot::games_from_snapshot(&crate::fixtures::snapshot(
            crate::cli::FixtureKind::Sample,
        ))
        .remove(0);
        // The reader stores short names, and older rows a row prefix.
        g.suspect_fields = vec!["dmg".into(), "r3.h".into(), "r3.dmg".into(), "bogus".into()];
        g.edited_fields.clear();
        assert!(unsure_now(&g, "damage"));
        assert!(unsure_now(&g, "healing"));
        assert!(!unsure_now(&g, "elims"));
        assert_eq!(
            still_unsure(&g),
            ["dmg", "h"],
            "once each, unknown names dropped"
        );
        g.edited_fields = vec!["damage".into()];
        assert!(!unsure_now(&g, "damage"));
        assert_eq!(still_unsure(&g), ["h"]);
        let _ = row(&g);
    }
}
