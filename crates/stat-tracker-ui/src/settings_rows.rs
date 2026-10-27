//! Row widgets for the 0.6 Settings page: the spec's setting row (title and
//! help on the left, the control on the right), chip lists with "+ Add",
//! dropdowns, read-only and waiting rows, tags and the changed dot.
//!
//! Only [`crate::settings_page`] and [`crate::capture_reading`] draw with
//! these; the classic card layout keeps its own cards. Both edit the same
//! form.

use iced::widget::{
    Column, Row, button, checkbox, column, container, pick_list, row, space, text, text_input,
};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Shadow, Theme};

use crate::app::{Message, TrackerApp};
use crate::form_widgets::{checkbox_style, field_caption, hint};
use crate::settings::SettingsField;
use crate::settings_model::{
    ChipList, SessionWindow, WAITS_FOR_S11, chips_from, session_window_options,
};
use crate::settings_update::Msg;
use crate::theme::{
    self, ACCENT, BORDER, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META, SURFACE,
    TEXT, TEXT_2, TEXT_3,
};

/// Tag on rows that apply after a tracker restart.
pub(crate) const AFTER_RESTART_TAG: &str = "After restart";
/// Tag on rows the 0.6 spec adds.
const NEW_TAG: &str = "New in 0.6";

/// Help under Game processes.
const GAME_PROCESSES_HELP: &str =
    "Capture runs only while one is running. An empty list turns the check off.";
/// Help under In-game names.
const NAMES_HELP: &str = "As shown in-game, without the #1234.";
/// Help under Session window.
const SESSION_WINDOW_HELP: &str = "Games closer together than this count as one session.";

/// The spec's setting row: title and help on the left, control on the
/// right. Below `stack_below` px the control goes under the text.
pub(crate) fn setting_row<'a>(
    title: Element<'a, Message>,
    help: Option<&'static str>,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    let mut left: Column<'a, Message> = column![title].spacing(2).width(Fill);
    if let Some(help) = help {
        left = left.push(hint(help));
    }
    row![left, control]
        .spacing(16)
        .align_y(Alignment::Center)
        .width(Fill)
        .into()
}

/// Row title text.
pub(crate) fn row_title(label: &'static str) -> Element<'static, Message> {
    text(label)
        .size(SIZE_BODY)
        .font(FONT_SEMIBOLD)
        .color(TEXT)
        .into()
}

/// A removable chip: label and ×.
fn chip(label: String, on_remove: Option<Message>) -> Element<'static, Message> {
    let mut inner = Row::new()
        .spacing(6)
        .align_y(Alignment::Center)
        .push(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT));
    let mut x = button(text("×").size(SIZE_BODY).font(FONT_SEMIBOLD).color(TEXT_2))
        .padding(0)
        .style(bare_btn);
    if let Some(msg) = on_remove {
        x = x.on_press(msg);
    }
    inner = inner.push(x);
    container(inner)
        .padding(Padding::from([4, 10]))
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: theme::RADIUS_CHIP.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// Dashed-look "+ Add" chip, or the open input.
fn add_chip<'a>(
    app: &'a TrackerApp,
    list: ChipList,
    label: &'static str,
    placeholder: &'static str,
    demo: bool,
) -> Element<'a, Message> {
    if let Some((open, draft)) = app.page_state.chip_draft()
        && *open == list
        && !demo
    {
        return row![
            text_input(placeholder, draft)
                .on_input(move |v| Message::SettingsPage(Msg::ChipDraft(list, Some(v))))
                .on_submit(Message::SettingsPage(Msg::ChipAdd))
                .padding(Padding::from([4, 10]))
                .size(SIZE_META)
                .style(theme::text_input_style)
                .width(160),
            small_btn("Add", Some(Message::SettingsPage(Msg::ChipAdd))),
            small_btn(
                "Cancel",
                Some(Message::SettingsPage(Msg::ChipDraft(list, None)))
            ),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into();
    }
    let mut b = button(
        text(label)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
    )
    .padding(Padding::from([4, 10]))
    .style(|_: &Theme, status| button::Style {
        background: theme::hover_tint(status),
        text_color: TEXT_2,
        border: Border {
            color: BORDER,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    });
    if !demo {
        b = b.on_press(Message::SettingsPage(Msg::ChipDraft(
            list,
            Some(String::new()),
        )));
    }
    b.into()
}

fn small_btn(label: &'static str, msg: Option<Message>) -> Element<'static, Message> {
    let mut b = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([4, 10]))
        .style(theme::ghost_btn());
    if let Some(msg) = msg {
        b = b.on_press(msg);
    }
    b.into()
}

fn bare_btn(_: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: if matches!(status, button::Status::Hovered) {
            TEXT
        } else {
            TEXT_2
        },
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

/// In-game names as chips. The config stores one name, so "+ Add name"
/// shows only while it is empty.
pub(crate) fn names_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut chips = Row::new().spacing(6).align_y(Alignment::Center);
    let name = app.settings.player_name.trim();
    if !name.is_empty() {
        chips = chips.push(chip(
            name.to_string(),
            (!demo).then_some(Message::SettingsPage(Msg::ChipRemove(ChipList::Names, 0))),
        ));
    }
    if name.is_empty() || matches!(app.page_state.chip_draft(), Some((ChipList::Names, _))) {
        chips = chips.push(add_chip(app, ChipList::Names, "+ Add name", "Name", demo));
    }
    setting_row(
        row_title("In-game names"),
        Some(NAMES_HELP),
        chips.wrap().into(),
    )
}

/// Session window dropdown. Writes seconds, as before.
pub(crate) fn session_window_row(app: &TrackerApp) -> Element<'_, Message> {
    let current = app.settings.session_window_secs.trim().parse::<u64>().ok();
    let options = session_window_options(current);
    let selected = current.map(SessionWindow);
    let pick = dropdown(options, selected, |w: SessionWindow| {
        Message::SettingsText(SettingsField::SessionWindow, w.0.to_string())
    });
    setting_row(row_title("Session window"), Some(SESSION_WINDOW_HELP), pick)
}

/// Game processes as chips with "+ Add".
pub(crate) fn processes_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut chips = Row::new().spacing(6).align_y(Alignment::Center);
    for (i, name) in chips_from(&app.settings.game_process_names)
        .into_iter()
        .enumerate()
    {
        chips = chips.push(chip(
            name,
            (!demo).then_some(Message::SettingsPage(Msg::ChipRemove(
                ChipList::Processes,
                i,
            ))),
        ));
    }
    chips = chips.push(add_chip(
        app,
        ChipList::Processes,
        "+ Add",
        "Overwatch.exe",
        demo,
    ));
    column![
        row_title("Game processes"),
        hint(GAME_PROCESSES_HELP),
        chips.wrap()
    ]
    .spacing(6)
    .into()
}

/// Dropdown in the page's chip style, with a ▾ handle.
pub(crate) fn dropdown<'a, T>(
    options: Vec<T>,
    selected: Option<T>,
    on_select: impl Fn(T) -> Message + 'a,
) -> Element<'a, Message>
where
    T: ToString + PartialEq + Clone + 'a,
{
    pick_list(options, selected, on_select)
        .text_size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .padding(Padding::from([6, 12]))
        .style(|_: &Theme, status| iced::widget::pick_list::Style {
            text_color: TEXT,
            placeholder_color: TEXT_3,
            handle_color: TEXT_2,
            background: Background::Color(SURFACE),
            border: Border {
                color: match status {
                    iced::widget::pick_list::Status::Active => BORDER,
                    _ => ACCENT,
                },
                width: 1.0,
                radius: 8.0.into(),
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
            shadow: Shadow::default(),
        })
        .into()
}

/// Small caps heading inside a card or fold.
pub(crate) fn sub_heading(label: &'static str) -> Element<'static, Message> {
    text(label.to_ascii_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_SEMIBOLD)
        .color(TEXT_3)
        .into()
}

/// A setting shown but not editable here yet. The accent dot marks a value
/// that differs from the default.
pub(crate) fn read_only_row(
    label: &'static str,
    value: String,
    changed: bool,
    note: Option<&'static str>,
) -> Element<'static, Message> {
    let mut caption = Row::new()
        .spacing(6)
        .align_y(Alignment::Center)
        .push(field_caption(label));
    if changed {
        caption = caption.push(changed_dot());
    }
    let mut col: Column<'static, Message> = column![
        caption,
        text(value).size(SIZE_BODY).font(FONT_MEDIUM).color(TEXT),
    ]
    .spacing(4);
    if let Some(note) = note {
        col = col.push(hint(note));
    }
    col.into()
}

/// A switch the 0.6 spec shows that waits for S11. Drawn off and disabled.
pub(crate) fn stub_row(
    label: &'static str,
    help: Option<&'static str>,
    after_restart: bool,
) -> Element<'static, Message> {
    let mut line = Row::new()
        .spacing(8)
        .align_y(Alignment::Center)
        .push(
            checkbox(false)
                .label(label)
                .size(16)
                .text_size(SIZE_BODY)
                .style(checkbox_style),
        )
        .push(tag(NEW_TAG));
    if after_restart {
        line = line.push(tag(AFTER_RESTART_TAG));
    }
    let mut col = column![line.wrap()].spacing(4);
    if let Some(help) = help {
        col = col.push(hint(help));
    }
    col.push(hint(WAITS_FOR_S11)).into()
}

/// Small outlined label, neutral colours.
pub(crate) fn tag(label: &'static str) -> Element<'static, Message> {
    container(
        text(label)
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
    )
    .padding(Padding {
        top: 2.0,
        bottom: 2.0,
        left: 8.0,
        right: 8.0,
    })
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

/// Accent dot for "changed from default".
pub(crate) fn changed_dot() -> Element<'static, Message> {
    container(space().width(8).height(8))
        .width(8)
        .height(8)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(ACCENT)),
            border: Border {
                radius: theme::RADIUS_CHIP.into(),
                ..Border::default()
            },
            ..container::Style::default()
        })
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn copy_has_no_dashes() {
        for line in [
            GAME_PROCESSES_HELP,
            NAMES_HELP,
            SESSION_WINDOW_HELP,
            AFTER_RESTART_TAG,
            NEW_TAG,
        ] {
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{line}"
            );
        }
    }
}
