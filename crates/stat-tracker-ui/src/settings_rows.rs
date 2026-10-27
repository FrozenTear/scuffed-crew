//! Row widgets for the 0.6 Settings page, after spec p6 and p7: the
//! setting row (changed dot, title and tags, help under it, the control on
//! the right), rows divided by hairlines, On/Off switches, a segmented
//! control, a radio list, chip lists with an accent "+ Add", dropdowns,
//! text fields that apply on Enter, tags and the changed dot.
//!
//! Only [`crate::settings_page`], [`crate::settings_cards`] and
//! [`crate::capture_reading`] draw with these; the classic card layout
//! keeps its own widgets. Both pages edit the same form.

use iced::widget::{
    Column, Row, button, column, container, pick_list, radio, row, rule, space, text, text_input,
    toggler,
};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Padding, Shadow, Theme};

use crate::app::{Message, TrackerApp};
use crate::form_widgets::hint;
use crate::settings::SettingsField;
use crate::settings_model::{
    COMING_LATER, ChipList, DefaultRow, SessionWindow, differs_from_default, parse_process_names,
    session_window_options,
};
use crate::settings_save::Commit;
use crate::settings_update::{Msg, apply, chip_input_id};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META,
    SURFACE, TEXT, TEXT_2, TEXT_3,
};

/// Space above and below each divided row (spec about 12 px).
pub(crate) const ROW_PAD_Y: f32 = 12.0;
/// Room the control keeps from the text on its left.
const CONTROL_GAP: f32 = 16.0;
/// Widest a chip list gets beside its title.
const CHIPS_MAX: f32 = 360.0;
/// Width of a text field on the page.
const FIELD_WIDTH: f32 = 220.0;

/// Help under Game processes.
const GAME_PROCESSES_HELP: &str =
    "Capture runs only while one is running. An empty list turns the check off.";
/// Shown under Game processes once the list is empty.
const NO_PROCESSES_NOTE: &str = "No processes listed, so the tracker captures whenever it runs.";
/// Help under In-game names.
const NAMES_HELP: &str = "As shown in-game, without the #1234.";
/// Help under Session window: the spec line (the 0.6 Games tab groups by
/// it), then what else this app uses it for. Ported from #555.
const SESSION_WINDOW_HELP: &str = "Games closer together than this count as one session. It is also how long a game still counts as live for the Companion.";

/// A tag after a row title.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Tag {
    /// "After restart": accent text and outline.
    AfterRestart,
    /// "New in 0.6": a filled neutral chip.
    New,
}

impl Tag {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Tag::AfterRestart => "After restart",
            Tag::New => "New in 0.6",
        }
    }
}

/// A small tag. After restart is accent outlined, New in 0.6 filled grey.
pub(crate) fn tag(tag: Tag) -> Element<'static, Message> {
    let (fg, fill, edge) = match tag {
        Tag::AfterRestart => (ACCENT, None, ACCENT),
        Tag::New => (TEXT_2, Some(Background::Color(BORDER)), BORDER),
    };
    container(
        text(tag.label())
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(fg),
    )
    .padding(Padding::from([2, 8]))
    .style(move |_: &Theme| container::Style {
        background: fill,
        border: Border {
            color: edge,
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

/// Row title: the changed dot first, then the title, then its tags.
pub(crate) fn title_line(
    label: impl Into<String>,
    changed: bool,
    tags: &[Tag],
) -> Element<'static, Message> {
    let mut line = Row::new().spacing(8).align_y(Alignment::Center);
    if changed {
        line = line.push(changed_dot());
    }
    line = line.push(
        text(label.into())
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    );
    for t in tags {
        line = line.push(tag(*t));
    }
    line.wrap().into()
}

/// Plain row title.
pub(crate) fn row_title(label: &'static str) -> Element<'static, Message> {
    title_line(label, false, &[])
}

/// The spec's setting row: title and help lines on the left, the control
/// on the right, centred on the text.
pub(crate) fn setting_row<'a>(
    title: Element<'a, Message>,
    help: Vec<Element<'a, Message>>,
    control: Element<'a, Message>,
) -> Element<'a, Message> {
    let mut left: Column<'a, Message> = column![title].spacing(3).width(Fill);
    for line in help {
        left = left.push(line);
    }
    row![left, control]
        .spacing(CONTROL_GAP)
        .align_y(Alignment::Center)
        .width(Fill)
        .into()
}

/// Rows with a 1 px hairline between them (none above the first or under
/// the last), each padded [`ROW_PAD_Y`] above and below.
pub(crate) fn divided<'a>(rows: Vec<Element<'a, Message>>) -> Element<'a, Message> {
    let count = rows.len();
    let mut col = Column::new().spacing(0).width(Fill);
    for (i, r) in rows.into_iter().enumerate() {
        let pad = Padding {
            top: if i == 0 { 0.0 } else { ROW_PAD_Y },
            bottom: if i + 1 == count { 0.0 } else { ROW_PAD_Y },
            ..Padding::ZERO
        };
        col = col.push(container(r).padding(pad).width(Fill));
        if i + 1 < count {
            col = col.push(hairline());
        }
    }
    col.into()
}

/// A 1 px rule in the border colour.
pub(crate) fn hairline() -> Element<'static, Message> {
    rule::horizontal(1)
        .style(|_: &Theme| rule::Style {
            color: BORDER,
            radius: 0.0.into(),
            fill_mode: rule::FillMode::Full,
            snap: true,
        })
        .into()
}

/// An On/Off switch with its word on the right (spec p6). `on_toggle`
/// `None` draws it disabled and dimmed.
pub(crate) fn switch<'a>(
    on: bool,
    on_toggle: Option<Box<dyn Fn(bool) -> Message + 'a>>,
) -> Element<'a, Message> {
    let enabled = on_toggle.is_some();
    let mut t = toggler(on).size(20).style(switch_style);
    if let Some(f) = on_toggle {
        t = t.on_toggle(f);
    }
    let word = text(if on { "On" } else { "Off" })
        .size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .color(match (enabled, on) {
            (true, true) => TEXT,
            (true, false) => TEXT_2,
            (false, _) => TEXT_3,
        })
        .width(Length::Fixed(24.0));
    row![t, word].spacing(8).align_y(Alignment::Center).into()
}

/// [`theme::companion_toggle`], faded when disabled so a switch that
/// cannot be used does not look live.
pub(crate) fn switch_style(theme_: &Theme, status: toggler::Status) -> toggler::Style {
    let mut style = theme::companion_toggle(theme_, status);
    if matches!(status, toggler::Status::Disabled { .. }) {
        let fade = |c: Color| Color { a: c.a * 0.4, ..c };
        if let Background::Color(c) = style.background {
            style.background = Background::Color(fade(c));
        }
        if let Background::Color(c) = style.foreground {
            style.foreground = Background::Color(fade(c));
        }
        style.background_border_color = fade(style.background_border_color);
    }
    style
}

/// One option of a [`segmented`] control.
pub(crate) struct Segment {
    pub label: &'static str,
    pub selected: bool,
    pub msg: Option<Message>,
}

/// Joined buttons, one selected (spec Capture method, Auto/Off).
pub(crate) fn segmented(segments: Vec<Segment>) -> Element<'static, Message> {
    let mut r = Row::new().spacing(0).align_y(Alignment::Center);
    for seg in segments {
        let enabled = seg.msg.is_some();
        let selected = seg.selected;
        let mut b = button(text(seg.label).size(SIZE_META).font(FONT_SEMIBOLD).color(
            if selected {
                TEXT
            } else if enabled {
                TEXT_2
            } else {
                TEXT_3
            },
        ))
        .padding(Padding::from([6, 12]))
        .style(move |_: &Theme, status| button::Style {
            background: if selected {
                Some(Background::Color(Color { a: 0.18, ..ACCENT }))
            } else {
                theme::hover_tint(status)
            },
            text_color: TEXT,
            border: Border {
                color: if selected { ACCENT } else { Color::TRANSPARENT },
                width: 1.0,
                radius: 7.0.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        });
        if let Some(msg) = seg.msg {
            b = b.on_press(msg);
        }
        r = r.push(b);
    }
    container(r)
        .padding(1)
        .style(|_: &Theme| container::Style {
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..container::Style::default()
        })
        .into()
}

/// A right-hand radio list (spec Reader).
pub(crate) fn radio_list<V>(
    options: &[(V, &'static str)],
    selected: V,
    on_pick: impl Fn(V) -> Message,
) -> Element<'static, Message>
where
    V: Copy + Eq + 'static,
{
    let mut col = Column::new().spacing(8);
    for (value, label) in options {
        col = col.push(
            radio(*label, *value, Some(selected), &on_pick)
                .size(16)
                .spacing(8)
                .text_size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .style(radio_style),
        );
    }
    col.into()
}

fn radio_style(_: &Theme, status: iced::widget::radio::Status) -> iced::widget::radio::Style {
    use iced::widget::radio::Status;
    let (selected, hot) = match status {
        Status::Active { is_selected } => (is_selected, false),
        Status::Hovered { is_selected } => (is_selected, true),
    };
    iced::widget::radio::Style {
        background: Background::Color(SURFACE),
        dot_color: ACCENT,
        border_width: 1.0,
        border_color: if selected {
            ACCENT
        } else if hot {
            TEXT_3
        } else {
            BORDER
        },
        text_color: Some(if selected { TEXT } else { TEXT_2 }),
    }
}

/// A switch row the 0.6 spec shows that the tracker cannot store yet:
/// drawn at the spec default, disabled, with [`COMING_LATER`].
pub(crate) fn coming_later_row(
    label: &'static str,
    help: &'static str,
    tags: &[Tag],
    spec_default_on: bool,
) -> Element<'static, Message> {
    setting_row(
        title_line(label, false, tags),
        vec![hint(help), hint(COMING_LATER)],
        switch(spec_default_on, None),
    )
}

/// A removable chip: label and a × with room around it to hit.
fn chip(label: String, on_remove: Option<Message>) -> Element<'static, Message> {
    let mut x = button(text("×").size(SIZE_BODY).font(FONT_SEMIBOLD).color(TEXT_2))
        .padding(Padding::from([0, 4]))
        .style(|_: &Theme, status| button::Style {
            background: theme::hover_tint(status),
            text_color: TEXT_2,
            border: Border {
                radius: theme::RADIUS_CHIP.into(),
                ..Border::default()
            },
            shadow: Shadow::default(),
            snap: false,
        });
    if let Some(msg) = on_remove {
        x = x.on_press(msg);
    }
    container(
        row![
            text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT),
            x
        ]
        .spacing(2)
        .align_y(Alignment::Center),
    )
    .padding(Padding {
        top: 3.0,
        bottom: 3.0,
        left: 10.0,
        right: 4.0,
    })
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

/// An accent text button with no border ("+ Add name", "Run setup again",
/// "Reset to default"). Without a message it is drawn muted.
pub(crate) fn link(label: impl Into<String>, msg: Option<Message>) -> Element<'static, Message> {
    let enabled = msg.is_some();
    let mut b = button(
        text(label.into())
            .size(SIZE_META)
            .font(FONT_BOLD)
            .color(if enabled { ACCENT } else { TEXT_3 }),
    )
    // No side padding, so the words line up with the gutter and the rows.
    .padding(Padding::from([2, 0]))
    .style(theme::quiet_btn(6.0));
    if let Some(msg) = msg {
        b = b.on_press(msg);
    }
    b.into()
}

/// The "+ Add" link, or the open input with Add and Cancel.
fn add_chip<'a>(
    app: &'a TrackerApp,
    list: ChipList,
    label: &'static str,
    placeholder: &'static str,
    demo: bool,
) -> Element<'a, Message> {
    if let Some((open, draft)) = app.page_state.chip_draft()
        && *open == list
    {
        return row![
            text_input(placeholder, draft)
                .id(chip_input_id(list))
                .on_input(move |v| Message::SettingsPage(Msg::ChipDraft(list, Some(v))))
                .on_submit(Message::SettingsPage(Msg::ChipAdd))
                .padding(Padding::from([4, 10]))
                .size(SIZE_META)
                .style(theme::text_input_style)
                .width(150),
            link("Add", Some(Message::SettingsPage(Msg::ChipAdd))),
            link(
                "Cancel",
                Some(Message::SettingsPage(Msg::ChipDraft(list, None)))
            ),
        ]
        .spacing(4)
        .align_y(Alignment::Center)
        .into();
    }
    link(
        label,
        (!demo).then_some(Message::SettingsPage(Msg::ChipDraft(
            list,
            Some(String::new()),
        ))),
    )
}

/// A chip list beside its title, wrapping within [`CHIPS_MAX`].
fn chip_control<'a>(chips: Row<'a, Message>) -> Element<'a, Message> {
    container(chips.spacing(6).align_y(Alignment::Center).wrap())
        .max_width(CHIPS_MAX)
        .into()
}

/// In-game names as chips. The config stores one name, so "+ Add name"
/// shows only while it is empty, and a new name replaces it.
pub(crate) fn names_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut chips = Row::new();
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
        title_line("In-game names", false, &[Tag::AfterRestart]),
        vec![hint(NAMES_HELP)],
        chip_control(chips),
    )
}

/// Session window dropdown. Writes seconds, as before.
pub(crate) fn session_window_row(app: &TrackerApp) -> Element<'_, Message> {
    let current = app.saved_config.session_window_secs;
    let pick = dropdown(
        session_window_options(Some(current)),
        Some(SessionWindow(current)),
        |w: SessionWindow| {
            apply(Message::SettingsText(
                SettingsField::SessionWindow,
                w.0.to_string(),
            ))
        },
    );
    setting_row(
        title_line(
            "Session window",
            differs_from_default(&app.saved_config, DefaultRow::SessionWindow),
            &[],
        ),
        vec![hint(SESSION_WINDOW_HELP)],
        pick,
    )
}

/// Game processes as chips with "+ Add".
pub(crate) fn processes_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let names = parse_process_names(&app.settings.game_process_names);
    let empty = names.is_empty();
    let mut chips = Row::new();
    for (i, name) in names.into_iter().enumerate() {
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
    let mut help = vec![hint(GAME_PROCESSES_HELP)];
    if empty {
        help.push(
            text(NO_PROCESSES_NOTE)
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(theme::WARN)
                .into(),
        );
    }
    setting_row(
        title_line(
            "Game processes",
            differs_from_default(&app.saved_config, DefaultRow::GameProcesses),
            &[],
        ),
        help,
        chip_control(chips),
    )
}

/// A text field that writes on Enter (`commit`). Demo draws it read-only.
pub(crate) fn page_field<'a>(
    placeholder: &'static str,
    value: &'a str,
    field: SettingsField,
    commit: Option<Commit>,
    secret: bool,
    demo: bool,
) -> Element<'a, Message> {
    let mut input = text_input(placeholder, value)
        .padding(Padding::from([6, 10]))
        .size(SIZE_META)
        .style(theme::text_input_style)
        .secure(secret)
        .width(FIELD_WIDTH);
    if !demo {
        input = input.on_input(move |v| Message::SettingsText(field, v));
        if let Some(commit) = commit {
            input = input.on_submit(Message::SettingsPage(Msg::Commit(commit)));
        }
    }
    input.into()
}

/// Dropdown in the page's style, with iced's default arrow handle in
/// TEXT_2.
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
            selected_background: Background::Color(Color { a: 0.25, ..ACCENT }),
            shadow: Shadow::default(),
        })
        .into()
}

/// Small caps heading inside a card or fold.
pub(crate) fn sub_heading(label: &'static str) -> Element<'static, Message> {
    text(label.to_ascii_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_BOLD)
        .color(TEXT_3)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tags_have_their_own_tone_and_spec_words() {
        assert_eq!(Tag::AfterRestart.label(), "After restart");
        assert_eq!(Tag::New.label(), "New in 0.6");
    }

    #[test]
    fn a_disabled_switch_is_faded() {
        let theme = theme::iced_theme();
        let live = switch_style(&theme, toggler::Status::Active { is_toggled: true });
        let off = switch_style(&theme, toggler::Status::Disabled { is_toggled: true });
        let (Background::Color(a), Background::Color(b)) = (live.background, off.background) else {
            panic!("solid tracks");
        };
        assert!(b.a < a.a);
    }
}
