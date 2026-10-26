//! Row widgets for the 0.6 Settings page: the spec's setting row (title and
//! help on the left, the control on the right), chip lists with "+ Add",
//! dropdowns, and the page's own Capture block.
//!
//! The classic card layout keeps [`crate::settings::capture_body`]; this
//! file only draws for [`crate::settings_page`]. Both edit the same form.

use iced::widget::{Column, Row, button, column, container, pick_list, row, text, text_input};
use iced::{Alignment, Background, Border, Element, Fill, Length, Padding, Shadow, Theme};
use stat_tracker::config::CaptureRoute;

use crate::app::{Message, TrackerApp};
use crate::settings::{
    self, KDE_DIRECT_HELP, NoteTone, SessionKind, SettingsField, capture_route_chips,
    capture_route_notes, hint, output_chip, show_kde_direct,
};
use crate::settings_model::{ChipList, SessionWindow, chips_from, session_window_options};
use crate::theme::{
    self, ACCENT, BORDER, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_META, SURFACE, TEXT, TEXT_2,
    TEXT_3,
};

/// Help under Capture method.
pub(crate) const CAPTURE_METHOD_HELP: &str =
    "Automatic uses the screen stream on niri. Screenshot takes one picture per grab.";
/// Help under Monitor.
pub(crate) const MONITOR_HELP: &str = "The outputs the tracker can see.";
/// Help under Check the picture.
pub(crate) const CHECK_PICTURE_HELP: &str =
    "Takes one capture and shows it, so you can see the scoreboard is in frame.";
/// Help under Game processes.
pub(crate) const GAME_PROCESSES_HELP: &str =
    "Capture runs only while one is running. An empty list turns the check off.";
/// Help under In-game names.
pub(crate) const NAMES_HELP: &str = "As shown in-game, without the #1234.";
/// Help under Session window.
pub(crate) const SESSION_WINDOW_HELP: &str =
    "Games closer together than this count as one session.";

/// Capture method label on the 0.6 page. The classic page keeps its own.
pub(crate) fn route_label(route: CaptureRoute) -> &'static str {
    match route {
        CaptureRoute::Screenshot => "Screenshot",
        other => settings::capture_route_label(other),
    }
}

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
    if let Some((open, draft)) = &app.chip_draft
        && *open == list
        && !demo
    {
        return row![
            text_input(placeholder, draft)
                .on_input(move |v| Message::ChipDraft(list, Some(v)))
                .on_submit(Message::ChipAdd)
                .padding(Padding::from([4, 10]))
                .size(SIZE_META)
                .style(theme::text_input_style)
                .width(160),
            small_btn("Add", Some(Message::ChipAdd)),
            small_btn("Cancel", Some(Message::ChipDraft(list, None))),
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
    .style(|_: &Theme, _| button::Style {
        background: None,
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
        b = b.on_press(Message::ChipDraft(list, Some(String::new())));
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
            (!demo).then_some(Message::ChipRemove(ChipList::Names, 0)),
        ));
    }
    if name.is_empty() || matches!(&app.chip_draft, Some((ChipList::Names, _))) {
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
fn processes_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut chips = Row::new().spacing(6).align_y(Alignment::Center);
    for (i, name) in chips_from(&app.settings.game_process_names)
        .into_iter()
        .enumerate()
    {
        chips = chips.push(chip(
            name,
            (!demo).then_some(Message::ChipRemove(ChipList::Processes, i)),
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

/// One entry in the Monitor dropdown. `None` is Auto.
#[derive(Debug, Clone, PartialEq, Eq)]
struct MonitorPick {
    name: Option<String>,
    auto_label: String,
}

impl std::fmt::Display for MonitorPick {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match &self.name {
            Some(n) => f.write_str(n),
            None => f.write_str(&self.auto_label),
        }
    }
}

/// "Auto (DP-3)" when the first output is known, else "Auto (first monitor)".
pub(crate) fn auto_monitor_label(outputs: &[String]) -> String {
    match outputs.first() {
        Some(first) => format!("Auto ({first})"),
        None => "Auto (first monitor)".to_string(),
    }
}

/// The page's Capture block: method chips, monitor dropdown, Check the
/// picture, and game processes.
pub(crate) fn capture_rows(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let route = app.settings.capture_route;
    let session = app.session;
    let mut route_chips = Row::new().spacing(8);
    for choice in capture_route_chips(&app.desktop, session, route) {
        route_chips = route_chips.push(output_chip(
            route_label(choice),
            route == choice,
            Message::SelectCaptureRoute(choice),
        ));
    }
    let mut body = column![
        row_title("Capture method"),
        hint(CAPTURE_METHOD_HELP),
        route_chips.wrap()
    ]
    .spacing(6)
    .width(Fill);
    if session != SessionKind::X11 && show_kde_direct(&app.desktop, route) {
        body = body.push(hint(KDE_DIRECT_HELP));
    }
    for (note, tone) in capture_route_notes(
        &app.desktop,
        session,
        route,
        app.kde_direct.as_ref(),
        app.ocr_switch.capture_notice.as_deref(),
    ) {
        body = body.push(
            text(note)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(match tone {
                    NoteTone::Info => TEXT_3,
                    NoteTone::Warn => theme::WARN,
                }),
        );
    }

    let auto_label = auto_monitor_label(&app.outputs);
    let mut options = vec![MonitorPick {
        name: None,
        auto_label: auto_label.clone(),
    }];
    options.extend(app.outputs.iter().map(|o| MonitorPick {
        name: Some(o.clone()),
        auto_label: auto_label.clone(),
    }));
    let current = settings::nonempty(&app.settings.capture_output);
    if let Some(c) = &current
        && !app.outputs.contains(c)
    {
        options.push(MonitorPick {
            name: Some(c.clone()),
            auto_label: auto_label.clone(),
        });
    }
    let selected = MonitorPick {
        name: current,
        auto_label,
    };
    let monitor = dropdown(options, Some(selected), |m: MonitorPick| {
        Message::SelectOutput(m.name)
    });

    let label = if app.capturing {
        "Capturing…"
    } else {
        "Capture now"
    };
    let mut capture = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([8, 16]))
        .style(theme::ghost_btn());
    // Demo never captures: the button is drawn but does nothing.
    if !demo && app.backend.is_some() && !app.capturing {
        capture = capture.on_press(Message::CaptureNow);
    }

    let mut rows = column![
        body,
        setting_row(row_title("Monitor"), Some(MONITOR_HELP), monitor),
        setting_row(
            row_title("Check the picture"),
            Some(CHECK_PICTURE_HELP),
            capture.into()
        ),
    ]
    .spacing(14)
    .width(Fill);
    if let Some(err) = &app.preview_error {
        rows = rows.push(
            text(err.clone())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::DANGER),
        );
    }
    if let Some((handle, when)) = &app.preview {
        rows = rows.push(hint(format!("Preview: {when}"))).push(
            iced::widget::image(handle.clone())
                .width(Fill)
                .height(Length::Fixed(280.0)),
        );
    }
    rows.push(processes_row(app, demo)).into()
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn screenshot_label_is_short_on_the_new_page() {
        assert_eq!(route_label(CaptureRoute::Screenshot), "Screenshot");
        assert_eq!(route_label(CaptureRoute::Auto), "Automatic");
        assert_eq!(route_label(CaptureRoute::Screencast), "Screen stream");
    }

    #[test]
    fn auto_monitor_names_the_first_output() {
        assert_eq!(auto_monitor_label(&["DP-3".into()]), "Auto (DP-3)");
        assert_eq!(auto_monitor_label(&[]), "Auto (first monitor)");
    }

    #[test]
    fn copy_matches_the_spec_and_has_no_dashes() {
        assert_eq!(
            CAPTURE_METHOD_HELP,
            "Automatic uses the screen stream on niri. Screenshot takes one picture per grab."
        );
        for line in [
            CAPTURE_METHOD_HELP,
            MONITOR_HELP,
            CHECK_PICTURE_HELP,
            GAME_PROCESSES_HELP,
            NAMES_HELP,
            SESSION_WINDOW_HELP,
        ] {
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{line}"
            );
        }
    }
}
