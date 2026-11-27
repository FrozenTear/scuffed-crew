//! A scrollable whose body keeps the Settings gutter free on the right
//! and whose bar is pinned to [`crate::layout::SCROLLBAR_WIDTH`], so line
//! ends stay clear of the bar whether or not it shows. The dialogs (What
//! leaves your PC, the setup guide, What's new, Report a problem, and
//! Uninstall) use this helper for that gap.
//!
//! The gutter only helps text that wraps. Plain `text` wraps on words, so
//! a token wider than the line (a path or URL with no spaces, a hash)
//! runs on under the bar and is cut there. [`wrap_text`] breaks such a
//! token on glyphs; the dialogs use it for every path, address, server
//! message and error they show.

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::text::{IntoFragment, Wrapping};
use iced::widget::{Scrollable, Text, container, scrollable, text};
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

/// A [`guttered_scrollable`] that fills the width and uses `height`.
pub(crate) fn guttered_area<'a>(
    body: impl Into<Element<'a, Message>>,
    height: impl Into<iced::Length>,
) -> Element<'a, Message> {
    guttered_scrollable(body).height(height).width(Fill).into()
}

/// `text` that wraps on words and, when one word is wider than the line,
/// on glyphs, so a long path, URL or hash stays left of the gutter.
pub(crate) fn wrap_text<'a>(value: impl IntoFragment<'a>) -> Text<'a> {
    text(value).wrapping(Wrapping::WordOrGlyph)
}
