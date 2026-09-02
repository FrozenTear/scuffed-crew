use iced::widget::{Row, column, text};
use iced::{Element, Fill};

use crate::aggregate::aggregate_filtered;
use crate::app::{Message, TrackerApp};
use crate::layout::heroes_columns;
use crate::theme::{FONT_BOLD, GRID_GAP, SIZE_TITLE, TEXT};
use crate::widgets;

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let stats = aggregate_filtered(&app.games, &app.header_filter());
    let cols = heroes_columns(content_width);
    let mut col = column![text("Heroes").size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),]
        .spacing(GRID_GAP)
        .width(Fill);

    if stats.heroes.is_empty() {
        col = col.push(widgets::empty_surface("No heroes in this window"));
        return col.into();
    }

    for chunk in stats.heroes.chunks(cols) {
        let mut row = Row::new().spacing(GRID_GAP).width(Fill);
        for h in chunk {
            row = row.push(widgets::hero_card(h));
        }
        for _ in chunk.len()..cols {
            row = row.push(iced::widget::space().width(Fill));
        }
        col = col.push(row);
    }
    col.into()
}
