use iced::widget::{Row, column, space, text};
use iced::{Element, Fill};

use crate::aggregate::aggregate_filtered;
use crate::app::{Message, TrackerApp};
use crate::layout::maps_columns;
use crate::theme::{FONT_BOLD, GRID_GAP, SIZE_TITLE, TEXT};
use crate::widgets;

pub const MAPS_EMPTY: &str = "No maps in this window";

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let stats = aggregate_filtered(&app.games, &app.header_filter());
    let cols = maps_columns(content_width);
    let mut col = column![text("Maps").size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),]
        .spacing(GRID_GAP)
        .width(Fill);

    if stats.maps.is_empty() {
        col = col.push(widgets::empty_surface(MAPS_EMPTY));
        return col.into();
    }

    for chunk in stats.maps.chunks(cols) {
        let mut row = Row::new().spacing(GRID_GAP).width(Fill);
        for m in chunk {
            row = row.push(widgets::map_card(m));
        }
        for _ in chunk.len()..cols {
            row = row.push(space().width(Fill));
        }
        col = col.push(row);
    }
    col.into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::theme::{self, PAGE_PAD_X, PAGE_PAD_Y};

    #[test]
    fn empty_copy_stays_user_facing() {
        assert_eq!(MAPS_EMPTY, "No maps in this window");
        assert!(!MAPS_EMPTY.contains("aggregate"));
        assert!(!MAPS_EMPTY.contains("fixture"));
    }

    #[test]
    fn maps_cards_use_redesign_tokens() {
        assert_eq!(PAGE_PAD_Y, 24.0);
        assert_eq!(PAGE_PAD_X, 32.0);
        assert_eq!(theme::RADIUS_CARD, 16.0);
        assert_eq!(theme::HEIGHT_MAP, 148.0);
        assert_eq!(theme::STRIPE, 4.0);
        assert_eq!(theme::ACCENT, {
            iced::Color::from_rgb(
                0x8f as f32 / 255.0,
                0x73 as f32 / 255.0,
                0xff as f32 / 255.0,
            )
        });
    }
}
