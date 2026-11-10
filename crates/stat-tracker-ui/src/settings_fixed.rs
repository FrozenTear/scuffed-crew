//! Controls on the 0.6 Settings page that show a value they cannot change
//! here: a dropdown the environment fixes or that a demo or this desktop
//! cannot use is drawn as a dimmed dropdown with its value (spec p7, OCR
//! workers "4 ▾"), not as plain text. Also the lock line with a tail on
//! the same line ("Set outside the app: STAT_TRACKER_OCR_THREADS  About
//! 23 MB each.").

use iced::widget::{container, image, row, text};
use iced::{Alignment, Background, Border, Element, Padding, Theme};

use crate::app::Message;
use crate::theme::{
    BORDER, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, SURFACE, TEXT_2, TEXT_3,
};

/// The arrow a dropdown draws: iced's own icon font, as `pick_list` uses
/// (the page font may have no "▾").
pub(crate) fn arrow() -> iced::widget::Text<'static> {
    use iced::advanced::text::Renderer as _;
    text(iced::Renderer::ARROW_DOWN_ICON.to_string()).font(iced::Renderer::ICON_FONT)
}

/// A sort header's arrow, from the same icon font: down when descending.
pub(crate) fn sort_arrow(desc: bool) -> iced::widget::Text<'static> {
    use iced::advanced::text::Renderer as _;
    let icon = if desc {
        iced::Renderer::SCROLL_DOWN_ICON
    } else {
        iced::Renderer::SCROLL_UP_ICON
    };
    text(icon.to_string()).font(iced::Renderer::ICON_FONT)
}

/// A dropdown that cannot be opened, showing `value`.
pub(crate) fn fixed_dropdown(value: impl Into<String>) -> Element<'static, Message> {
    container(
        row![
            text(value.into())
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT_3),
            arrow().size(SIZE_META).color(TEXT_3),
        ]
        .spacing(10)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([6, 12]))
    .style(|_: &Theme| container::Style {
        background: Some(Background::Color(iced::Color { a: 0.5, ..SURFACE })),
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// The lock line, then `tail` in plain help text on the same line.
pub(crate) fn lock_line_with(env: &'static str, tail: String) -> Element<'static, Message> {
    row![
        image(crate::icons::handle(crate::icons::Icon::Lock))
            .width(12)
            .height(12),
        text(format!("Set outside the app: {env}"))
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
        text(tail).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_3),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .wrap()
    .into()
}
