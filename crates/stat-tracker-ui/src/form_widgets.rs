//! Small form widgets both Settings pages use: field captions, help copy,
//! chips, buttons and the checkbox look. The classic page
//! ([`crate::settings`]) and the 0.6 page ([`crate::settings_page`]) draw
//! with the same ones, so they stay alike.

use iced::widget::{button, container, text};
use iced::{Element, Padding};

use crate::app::Message;
use crate::settings::HELP_MEASURE;
use crate::theme::{self, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_LABEL, SIZE_META, TEXT, TEXT_2, TEXT_3};
use crate::widgets;

pub(crate) fn field_caption(label: &'static str) -> text::Text<'static> {
    text(label)
        .size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .color(TEXT_2)
}

/// Help copy. Wraps at [`HELP_MEASURE`] even in a wider card.
pub(crate) fn hint(copy: impl Into<String>) -> Element<'static, Message> {
    container(
        text(copy.into())
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
    )
    .max_width(HELP_MEASURE)
    .into()
}

pub(crate) fn output_chip(
    label: impl Into<String>,
    selected: bool,
    msg: Message,
) -> Element<'static, Message> {
    widgets::filter_chip(label.into(), selected, msg)
}

pub(crate) fn action_btn(
    label: &'static str,
    primary: bool,
    msg: Message,
) -> Element<'static, Message> {
    let label = text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT);
    if primary {
        button(label)
            .padding(Padding::from([8, 16]))
            .style(theme::chip(true))
            .on_press(msg)
            .into()
    } else {
        button(label)
            .padding(Padding::from([8, 16]))
            .style(theme::ghost_btn())
            .on_press(msg)
            .into()
    }
}

pub(crate) fn checkbox_style(
    _theme: &iced::Theme,
    status: iced::widget::checkbox::Status,
) -> iced::widget::checkbox::Style {
    let selected = matches!(
        status,
        iced::widget::checkbox::Status::Active { is_checked: true }
            | iced::widget::checkbox::Status::Hovered { is_checked: true }
            | iced::widget::checkbox::Status::Disabled { is_checked: true }
    );
    iced::widget::checkbox::Style {
        background: iced::Background::Color(if selected { theme::ACCENT } else { theme::BG }),
        icon_color: TEXT,
        border: iced::Border {
            color: if selected {
                theme::ACCENT
            } else {
                theme::BORDER
            },
            width: 1.0,
            radius: theme::inner_radius(),
        },
        text_color: Some(TEXT),
    }
}
