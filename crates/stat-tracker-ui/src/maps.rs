use iced::widget::{column, text};
use iced::{Element, Fill};

use crate::aggregate::aggregate_filtered;
use crate::app::{Message, TrackerApp};
use crate::theme::{FONT_BOLD, GRID_GAP, PAD_INNER, SIZE_TITLE, TEXT};
use crate::widgets;

pub fn view(app: &TrackerApp) -> Element<'_, Message> {
    let stats = aggregate_filtered(&app.games, &app.header_filter());
    let mut col = column![text("Maps").size(SIZE_TITLE).font(FONT_BOLD).color(TEXT),]
        .spacing(GRID_GAP)
        .width(Fill);

    if stats.maps.is_empty() {
        col = col.push(widgets::empty_surface("No maps in this window"));
        return col.into();
    }

    let mut list = column![].spacing(GRID_GAP);
    for m in &stats.maps {
        list = list.push(
            iced::widget::container(widgets::map_row(m))
                .padding(PAD_INNER)
                .width(Fill)
                .style(crate::theme::surface_panel),
        );
    }
    col.push(list).into()
}
