//! A scrollable whose body keeps the Settings gutter free on the right
//! and whose bar is pinned to [`crate::layout::SCROLLBAR_WIDTH`], so line
//! ends stay clear of the bar whether or not it shows.

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Scrollable, container, scrollable};
use iced::{Element, Fill, Padding};

use crate::app::Message;
use crate::layout::{SCROLLBAR_WIDTH, SETTINGS_SCROLL_GUTTER};

/// Pads `body` on the right by [`SETTINGS_SCROLL_GUTTER`] and pins the
/// vertical bar width, so text does not run under the overlay bar.
pub(crate) fn guttered_scrollable<'a>(
    body: impl Into<Element<'a, Message>>,
) -> Scrollable<'a, Message> {
    scrollable(
        container(body)
            .padding(Padding {
                right: SETTINGS_SCROLL_GUTTER,
                ..Padding::ZERO
            })
            .width(Fill),
    )
    .direction(Direction::Vertical(
        Scrollbar::new()
            .width(SCROLLBAR_WIDTH)
            .scroller_width(SCROLLBAR_WIDTH),
    ))
}
