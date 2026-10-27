//! Capture and reading, the first ADVANCED row of the 0.6 Settings page:
//! a summary, then CAPTURE (method, monitor, Check the picture, game
//! processes, stream rows) and READING (reader, packs, model, workers).
//!
//! The pieces the classic Capture card shows too live in
//! [`crate::capture_shared`].

use iced::widget::{Row, button, column, container, row, text};
use iced::{Element, Fill, Length, Padding};
use stat_tracker::config::CaptureRoute;

use crate::app::{Message, TrackerApp};
use crate::capture_shared;
use crate::form_widgets::{action_btn, hint, output_chip};
use crate::settings::{
    CLASSIC_OCR_RESTART, DISMISS_REFUSAL_LABEL, KDE_DIRECT_HELP, NEW_READER_IMPLIED,
    NEW_READER_RESTART, capture_route_chips, capture_route_label, card_problem, nonempty,
    ocr_card_blocks, ocr_hint_lines, pack_status_line, reading_model_block,
};
use crate::settings_model::{
    OCR_THREADS_ENV, ReaderChoice, frame_rate_line, ocr_workers_line, ocr_workers_locked,
    pause_line,
};
use crate::settings_rows::{
    self, dropdown, read_only_row, row_title, setting_row, stub_row, sub_heading,
};
use crate::settings_update::Msg;
use crate::theme::{
    self, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAD_INNER, SIZE_META, TEXT, TEXT_2,
};

/// Help under Capture method.
const CAPTURE_METHOD_HELP: &str =
    "Automatic uses the screen stream on niri. Screenshot takes one picture per grab.";
/// Help under Monitor.
const MONITOR_HELP: &str = "The outputs the tracker can see.";
/// Help under Check the picture.
const CHECK_PICTURE_HELP: &str =
    "Takes one capture and shows it, so you can see the scoreboard is in frame.";
/// Help under the Report pictures row.
const REPORT_PICTURES_HELP: &str = "Stat-area crops in misread reports. Separate from Stat pictures. Default Off: reports show them unticked, so you choose each one. Names are never sent.";
/// Last line of Capture and reading.
const CAPTURE_READING_FOOTER: &str = "Only choices that work on this desktop are shown. Opening this page never captures; only Capture now does.";
/// Help under "Only capture while Overwatch is on screen".
const FOCUS_GATE_HELP: &str =
    "Skips grabs when the game is not visible on this monitor, so no other window is captured.";
/// Help under the Reader choice (spec wording).
const READER_HELP: &str = "New reader only never starts classic OCR.";
/// Help under Name reader.
const NAME_READER_HELP: &str = "Reads your own name from the names pack. Off uses classic OCR.";
/// Help under Pause between grabs.
const PAUSE_HELP: &str = "Screen stream only.";
/// Help under Stream frame rate.
const FRAME_RATE_HELP: &str = "Screen stream only. Lower costs less. Default 4.";

/// Capture method label on the 0.6 page. The classic page keeps its own.
pub(crate) fn route_label(route: CaptureRoute) -> &'static str {
    match route {
        CaptureRoute::Screenshot => "Screenshot",
        other => capture_route_label(other),
    }
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
fn auto_monitor_label(outputs: &[String]) -> String {
    match outputs.first() {
        Some(first) => format!("Auto ({first})"),
        None => "Auto (first monitor)".to_string(),
    }
}

/// The page's Capture block: method chips, monitor dropdown, Check the
/// picture, and game processes.
fn capture_rows(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let route = app.settings.capture_route;
    let mut route_chips = Row::new().spacing(8);
    for choice in capture_route_chips(&app.desktop, app.session, route) {
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
    if capture_shared::shows_kde_direct_help(app) {
        body = body.push(hint(KDE_DIRECT_HELP));
    }
    body = capture_shared::push_route_notes(body, app);

    let auto_label = auto_monitor_label(&app.outputs);
    let mut options = vec![MonitorPick {
        name: None,
        auto_label: auto_label.clone(),
    }];
    options.extend(app.outputs.iter().map(|o| MonitorPick {
        name: Some(o.clone()),
        auto_label: auto_label.clone(),
    }));
    let current = nonempty(&app.settings.capture_output);
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

    // The page shows Capture now (disabled) while the backend is unknown.
    let label = capture_shared::capture_button_label(true, app.capturing);
    let mut capture = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([8, 16]))
        .style(theme::ghost_btn());
    // Demo never captures: the button is drawn but does nothing.
    if !demo && capture_shared::can_capture(app) {
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
    if let Some(err) = capture_shared::preview_error(app) {
        rows = rows.push(err);
    }
    if let Some((handle, when)) = &app.preview {
        rows = rows.push(hint(format!("Preview: {when}"))).push(
            iced::widget::image(handle.clone())
                .width(Fill)
                .height(Length::Fixed(280.0)),
        );
    }
    rows.push(settings_rows::processes_row(app, demo)).into()
}

/// The 0.6 spec's Capture and reading: a summary, then CAPTURE and READING.
pub(crate) fn body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let config = &app.saved_config;
    let choice = ReaderChoice::from_switches(app.settings.new_reader, app.settings.classic_ocr);
    let monitor = if app.settings.capture_output.is_empty() {
        "first monitor".to_string()
    } else {
        app.settings.capture_output.clone()
    };

    let summary = container(
        column![
            summary_line(
                "Capture",
                format!("{} on {monitor}", route_label(app.settings.capture_route)),
            ),
            summary_line("Reading", choice.label().to_string()),
            summary_line("Packs", pack_status_line(app.ocr_switch.packs.as_ref())),
        ]
        .spacing(4),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::stat_box);

    let mut body = column![summary, sub_heading("Capture"), capture_rows(app, demo)]
        .spacing(GRID_GAP)
        .width(Fill);

    let (pause, pause_changed) = pause_line(config.screencast_pause_override());
    body = body.push(read_only_row(
        "Pause between grabs",
        pause.to_string(),
        pause_changed,
        Some(PAUSE_HELP),
    ));
    let (fps, fps_changed) = frame_rate_line(config.screencast_max_fps());
    body = body.push(read_only_row(
        "Stream frame rate",
        fps,
        fps_changed,
        Some(FRAME_RATE_HELP),
    ));
    body = body.push(stub_row(
        "Only capture while Overwatch is on screen",
        Some(FOCUS_GATE_HELP),
        false,
    ));

    body = body.push(sub_heading("Reading"));
    let mut chips = Row::new().spacing(8);
    for c in ReaderChoice::ALL {
        chips = chips.push(output_chip(
            c.label(),
            c == choice,
            Message::SettingsPage(Msg::PickReader(c)),
        ));
    }
    body = body.push(column![row_title("Reader"), hint(READER_HELP), chips.wrap()].spacing(6));

    let mut reading = column![].spacing(4);
    let lines = ocr_hint_lines(&stat_tracker::ocr_switch::Read::needing_ocr());
    // Both restart notes say the same thing, so show the line once.
    if lines.restart_and_packs {
        reading = reading.push(hint(CLASSIC_OCR_RESTART));
    } else if choice.new_reader() {
        reading = reading.push(hint(NEW_READER_RESTART));
    }
    if let Some((problem, dismissable)) = card_problem(
        app.ocr_switch.refusal.as_deref(),
        app.ocr_switch.notice.as_deref(),
    ) {
        reading = reading.push(
            text(problem.to_string())
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(theme::WARN),
        );
        if dismissable {
            reading = reading.push(action_btn(
                DISMISS_REFUSAL_LABEL,
                false,
                Message::DismissOcrRefusal,
            ));
        }
    }
    if app.ocr_switch.daemon_off {
        reading = reading.push(hint(NEW_READER_IMPLIED));
    }
    body = body.push(reading);

    if ocr_card_blocks(app.ocr_switch.daemon_off).reading_model {
        body = body.push(reading_model_block(app, demo));
    }

    body = body.push(read_only_row(
        "Name reader",
        if config.name_reader {
            "On (default)"
        } else {
            "Off"
        }
        .to_string(),
        !config.name_reader,
        Some(NAME_READER_HELP),
    ));

    let env_threads = std::env::var(OCR_THREADS_ENV).ok();
    let locked = ocr_workers_locked(env_threads.as_deref());
    let workers = match env_threads.as_deref().map(|v| v.trim().parse::<usize>()) {
        Some(Ok(n)) if locked => n.clamp(1, 8),
        _ => config.ocr_threads_resolved(),
    };
    body = body.push(read_only_row(
        "OCR workers",
        ocr_workers_line(workers),
        locked || config.ocr_threads.is_some(),
        locked.then_some("Set by STAT_TRACKER_OCR_THREADS."),
    ));

    body = body.push(stub_row(
        "Report pictures",
        Some(REPORT_PICTURES_HELP),
        true,
    ));
    body = body.push(hint(CAPTURE_READING_FOOTER));
    body.into()
}

fn summary_line(label: &'static str, value: String) -> Element<'static, Message> {
    row![
        text(label)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2)
            .width(96),
        text(value).size(SIZE_META).font(FONT_MEDIUM).color(TEXT),
    ]
    .spacing(8)
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
    fn reader_help_uses_the_spec_wording() {
        assert_eq!(READER_HELP, "New reader only never starts classic OCR.");
        assert!(!READER_HELP.contains("Tesseract"));
    }

    #[test]
    fn report_pictures_copy_matches_the_spec() {
        assert_eq!(
            REPORT_PICTURES_HELP,
            "Stat-area crops in misread reports. Separate from Stat pictures. Default Off: reports show them unticked, so you choose each one. Names are never sent."
        );
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
            REPORT_PICTURES_HELP,
            CAPTURE_READING_FOOTER,
            FOCUS_GATE_HELP,
            READER_HELP,
            NAME_READER_HELP,
            PAUSE_HELP,
            FRAME_RATE_HELP,
        ] {
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{line}"
            );
        }
    }
}
