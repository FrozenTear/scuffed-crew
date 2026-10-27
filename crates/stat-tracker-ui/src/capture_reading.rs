//! Capture and reading, the first ADVANCED row of the 0.6 Settings page
//! (spec p7): a summary of what is in effect, then CAPTURE (method,
//! monitor, Check the picture, game processes, the stream rows, the focus
//! gate) and READING (reader, name reader, reading model, OCR workers,
//! report pictures), each as divided setting rows with the control on the
//! right. Every change applies at once; the row is tagged After restart.
//!
//! The pieces the classic Capture card shows too live in
//! [`crate::capture_shared`].

use iced::widget::{Column, button, column, container, image, row, text};
use iced::{Alignment, Element, Fill, Length, Padding};
use stat_tracker::config::{CAPTURE_ENV, CaptureRoute, OCR_ENV, SCREENCAST_PAUSE_ENV};

use crate::app::{Message, TrackerApp};
use crate::capture_shared;
use crate::form_widgets::hint;
use crate::settings::{
    self, CLASSIC_OCR_HINT_NOT_YET, KDE_DIRECT_HELP, NEW_READER_HINT, NoteTone, SessionKind,
    capture_route_chips, capture_route_label, nonempty, ocr_card_blocks, ocr_hint_lines,
};
use crate::settings_fixed::{fixed_dropdown, lock_line_with};
use crate::settings_model::{
    ConfigEdit, DefaultRow, FrameRate, OCR_THREADS_ENV, OCR_WORKERS_MAX, OcrWorkers, PauseRow,
    ReaderChoice, auto_monitor_label, auto_pause, differs_from_default, frame_rate_options,
    monitor_phrase, ocr_env_lock, ocr_worker_options, ocr_workers_help, pause_row,
    stream_rows_apply,
};
use crate::settings_rows::{
    self, Segment, Tag, coming_later_row, divided, dropdown, hairline, link, radio_list, row_title,
    segmented, setting_row, sub_heading, switch, title_line,
};
use crate::settings_update::{Msg, apply};
use crate::theme::{
    self, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, SIZE_LABEL, SIZE_META, TEXT, TEXT_2, TEXT_3,
};

/// Help under Check the picture.
const CHECK_PICTURE_HELP: &str =
    "Takes one capture and shows it, so you can see the scoreboard is in frame.";
/// Help under Monitor.
const MONITOR_HELP: &str = "The outputs the tracker can see.";
/// Help under the Report pictures row.
const REPORT_PICTURES_HELP: &str = "Stat-area crops in misread reports. Separate from Stat pictures. Default Off: reports show them unticked, so you choose each one. Names are never sent.";
/// Last line of Capture and reading.
const CAPTURE_READING_FOOTER: &str = "Choices this desktop cannot use are left out or shown off. Opening this page never captures; only Capture now does.";
/// Help under "Only capture while Overwatch is on screen".
const FOCUS_GATE_HELP: &str =
    "Skips grabs when the game is not visible on this monitor, so no other window is captured.";
/// Help under the Reader choice (spec wording).
const READER_HELP: &str = "New reader only never starts classic OCR.";
/// What New reader only gives up, under the Reader choice while it is picked.
const NEW_ONLY_COST: &str = "Without classic OCR a game whose map is never read is saved without a map until you pick one, a co-op event scoreboard can be saved as a game, and a game that ends with a stat never read is not saved.";
/// Shown while a pick waits for the reader pack check.
const CHECKING_PACKS: &str = "Checking the reader packs…";
/// Help under Name reader.
const NAME_READER_HELP: &str = "Reads your own name from the names pack. Off uses classic OCR.";
/// Help under Reading model (spec).
const READING_MODEL_HELP: &str = "Needed whenever classic OCR can run.";
/// Help under the stream rows when the stream runs here, and when not.
const STREAM_ACTIVE: &str = "Screen stream only. Active on this desktop.";
const STREAM_NOT_USED: &str = "Screen stream only. Not used with this capture method here.";
/// Help under Stream frame rate where the stream runs (spec p7), before
/// its Reset to default.
fn frame_rate_help() -> String {
    format!(
        "Screen stream only. Lower costs less. Default {}.",
        stat_tracker::config::SCREENCAST_MAX_FPS_DEFAULT
    )
}
/// The second line under a row the environment fixes.
pub(crate) const SERVICE_MAY_DIFFER: &str = "The tracker service may differ.";

/// Capture method label on the 0.6 page. The classic page keeps its own.
pub(crate) fn route_label(route: CaptureRoute) -> &'static str {
    match route {
        CaptureRoute::Screenshot => "Screenshot",
        other => capture_route_label(other),
    }
}

/// The line under Capture method: what Automatic does on this session.
pub(crate) fn capture_method_help(session: SessionKind) -> &'static str {
    if session == SessionKind::X11 {
        "This is an X11 session. Automatic captures the screen directly with X11. Screenshot takes one picture per grab."
    } else {
        "Automatic uses the screen stream on KDE and GNOME, and direct capture on other desktops. Screenshot takes one picture per grab."
    }
}

/// The notes under the capture method, named for this page's chips.
fn page_route_notes(app: &TrackerApp) -> Vec<(String, NoteTone)> {
    settings::capture_route_notes(
        &app.desktop,
        app.session,
        app.settings.capture_route,
        app.kde_direct.as_ref(),
        app.ocr_switch.capture_notice.as_deref(),
    )
    .into_iter()
    .map(|(note, tone)| (note.replace("Older screenshot", "Screenshot"), tone))
    .collect()
}

/// Help lines under the Reader choice: the spec line (only while New
/// reader only is offered, see [`reader_options`]), then what the choice
/// costs or does. A build that still needs classic OCR says so.
pub(crate) fn reader_hints(choice: ReaderChoice, can_turn_off: bool) -> Vec<&'static str> {
    let mut lines = Vec::new();
    if can_turn_off || choice == ReaderChoice::NewOnly {
        lines.push(READER_HELP);
    }
    if !can_turn_off {
        lines.push(CLASSIC_OCR_HINT_NOT_YET);
    }
    match choice {
        ReaderChoice::NewOnly => lines.push(NEW_ONLY_COST),
        ReaderChoice::NewWithOcrBackup => lines.push(NEW_READER_HINT),
        ReaderChoice::ClassicOnly => {}
    }
    lines
}

/// The Reader radio's options. New reader only is left out while this
/// build cannot turn classic OCR off, unless it is already chosen.
pub(crate) fn reader_options(
    current: ReaderChoice,
    can_turn_off: bool,
) -> Vec<(ReaderChoice, &'static str)> {
    ReaderChoice::ALL
        .into_iter()
        .filter(|c| can_turn_off || *c != ReaderChoice::NewOnly || current == *c)
        .map(|c| (c, c.label()))
        .collect()
}

/// The Pause between grabs row on this desktop.
fn pause_now(app: &TrackerApp) -> PauseRow {
    pause_with(
        app,
        app.outside_var(SCREENCAST_PAUSE_ENV).as_deref(),
        app.outside_var("KDE_SESSION_VERSION").as_deref(),
    )
}

/// The Pause between grabs row with `pause_env` as
/// STAT_TRACKER_SCREENCAST_PAUSE and `kde_session_version` as
/// KDE_SESSION_VERSION, so a test never reads the host's environment.
fn pause_with(
    app: &TrackerApp,
    pause_env: Option<&str>,
    kde_session_version: Option<&str>,
) -> PauseRow {
    pause_row(
        stat_tracker::config::screencast_pause_override(
            app.saved_config.screencast_pause_between_grabs,
            pause_env,
        ),
        auto_pause(&app.desktop, kde_session_version),
    )
}

/// What the environment fixes outside the app, for the summary.
fn env_locks(app: &TrackerApp, pause: PauseRow) -> Vec<&'static str> {
    let mut locks = Vec::new();
    if app.capture_env_note().is_some() {
        locks.push("Capture method, by STAT_TRACKER_CAPTURE");
    }
    if pause.locked {
        locks.push("Pause between grabs, by STAT_TRACKER_SCREENCAST_PAUSE");
    }
    if app.ocr_control().env.is_some() {
        locks.push("Classic OCR, by STAT_TRACKER_OCR");
    }
    if ocr_env_lock(app.outside_var(OCR_THREADS_ENV).as_deref()).is_some() {
        locks.push("OCR workers, by STAT_TRACKER_OCR_THREADS");
    }
    locks
}

/// The summary box rows: label and value, from what is in effect. The
/// Reading line follows `STAT_TRACKER_OCR` when it is set, as the tracker
/// asks for it at start (the pack check can still keep classic OCR on).
pub(crate) fn summary_rows(app: &TrackerApp) -> Vec<(&'static str, String)> {
    let pause = pause_now(app);
    summary_rows_with(app, pause, env_locks(app, pause))
}

/// [`summary_rows`] with the pause row and the environment's locks given,
/// so a test never reads the host's environment.
fn summary_rows_with(
    app: &TrackerApp,
    pause: PauseRow,
    locks: Vec<&'static str>,
) -> Vec<(&'static str, String)> {
    let config = &app.saved_config;
    let monitor = monitor_phrase(config.capture_output.as_deref().unwrap_or(""), &app.outputs);
    // The method in effect (spec p7 "Screen stream on DP-3 (niri)"):
    // Automatic names what it uses on this desktop.
    let streams = stream_rows_apply(&app.desktop, app.session, config.capture_route);
    let method = match config.capture_route {
        CaptureRoute::Auto if streams => "Screen stream",
        CaptureRoute::Auto => "Direct capture",
        route => route_label(route),
    };
    let mut capture = format!("{method} on {monitor}");
    if !app.desktop.trim().is_empty() {
        capture.push_str(&format!(" ({})", app.desktop.trim()));
    }
    if streams && pause.pauses {
        capture.push_str(", paused between grabs");
    }
    capture.push('.');
    let mut in_effect = config.clone();
    in_effect.ocr = app.ocr_control().requested();
    let reading = match ReaderChoice::of(&in_effect) {
        ReaderChoice::NewOnly => "New reader saves stats. Classic OCR never starts.",
        // "Ready" only while the reading model classic OCR needs is there.
        ReaderChoice::NewWithOcrBackup if app.tessdata_installed => {
            "New reader saves stats. Classic OCR backup is ready."
        }
        ReaderChoice::NewWithOcrBackup => "New reader saves stats. Classic OCR is the backup.",
        ReaderChoice::ClassicOnly => "Classic OCR saves stats.",
    };
    let packs = match app.ocr_switch.packs {
        Some(p) => {
            let (ready, needed) = p.counts();
            if ready == needed {
                "All installed.".to_string()
            } else {
                format!("{ready} of {needed} ready.")
            }
        }
        None if app.fixture.is_some() => "Not checked in demo mode.".to_string(),
        None => "Checking…".to_string(),
    };
    let mut rows = vec![
        ("Capture", capture),
        ("Reading", reading.to_string()),
        ("Reader packs", packs),
    ];
    if !locks.is_empty() {
        rows.push(("Set outside the app", format!("{}.", locks.join("; "))));
    }
    rows
}

fn summary(app: &TrackerApp) -> Element<'static, Message> {
    let mut col = Column::new().spacing(6);
    for (label, value) in summary_rows(app) {
        col = col.push(
            row![
                text(label)
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(TEXT_3)
                    .width(132),
                text(value)
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT)
                    .width(Fill),
            ]
            .spacing(8),
        );
    }
    container(col)
        .padding(Padding::from([12, 14]))
        .width(Fill)
        .style(theme::stat_box)
        .into()
}

/// A muted line, or an amber or red one.
fn note(copy: impl Into<String>, tone: NoteTone) -> Element<'static, Message> {
    text(copy.into())
        .size(SIZE_LABEL)
        .font(FONT_MEDIUM)
        .color(match tone {
            NoteTone::Info => TEXT_3,
            NoteTone::Warn => theme::WARN,
        })
        .into()
}

/// The lock line under a row the environment fixes (spec p7).
pub(crate) fn lock_line(env: &'static str) -> Element<'static, Message> {
    row![
        image(crate::icons::handle(crate::icons::Icon::Lock))
            .width(12)
            .height(12),
        text(format!("Set outside the app: {env}"))
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
    ]
    .spacing(6)
    .align_y(Alignment::Center)
    .into()
}

fn capture_rows(app: &TrackerApp, demo: bool) -> Vec<Element<'_, Message>> {
    let config = &app.saved_config;
    let route = config.capture_route;

    // Capture method.
    // STAT_TRACKER_CAPTURE wins over the choice, so the chips lock like
    // the other rows the environment fixes.
    let env_route = app.capture_env_note();
    let segments = capture_route_chips(&app.desktop, app.session, route)
        .into_iter()
        .map(|choice| Segment {
            label: route_label(choice),
            selected: route == choice,
            msg: (!demo && env_route.is_none()).then(|| apply(Message::SelectCaptureRoute(choice))),
        })
        .collect();
    let mut help = vec![hint(capture_method_help(app.session))];
    if capture_shared::shows_kde_direct_help(app) {
        help.push(hint(KDE_DIRECT_HELP));
    }
    for (line, tone) in page_route_notes(app) {
        help.push(note(line, tone));
    }
    if let Some(env) = env_route {
        help.push(lock_line(CAPTURE_ENV));
        help.push(
            text(env)
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(theme::DANGER)
                .into(),
        );
    }
    let method = setting_row(
        title_line(
            "Capture method",
            differs_from_default(config, DefaultRow::CaptureMethod),
            &[],
        ),
        help,
        segmented(segments),
    );

    // Monitor.
    let auto_label = auto_monitor_label(&app.outputs);
    let mut options = vec![MonitorPick {
        name: None,
        auto_label: auto_label.clone(),
    }];
    let current = nonempty(config.capture_output.as_deref().unwrap_or(""));
    let mut names = app.outputs.clone();
    if let Some(c) = &current
        && !names.contains(c)
    {
        names.push(c.clone());
    }
    options.extend(names.into_iter().map(|o| MonitorPick {
        name: Some(o),
        auto_label: auto_label.clone(),
    }));
    let selected = MonitorPick {
        name: current,
        auto_label,
    };
    let monitor = setting_row(
        title_line(
            "Monitor",
            differs_from_default(config, DefaultRow::Monitor),
            &[],
        ),
        vec![
            hint(MONITOR_HELP),
            // Until a backend is found, "Detected:" has nothing to name.
            hint(match app.backend {
                Some(_) => format!("Detected: {}", capture_shared::backend_label(app.backend)),
                None => "Detected: not yet".to_string(),
            }),
        ],
        dropdown(options, Some(selected), |m: MonitorPick| {
            apply(Message::SelectOutput(m.name))
        }),
    );

    // Check the picture. While no backend is known the button is drawn
    // disabled with its usual word; the Monitor row says "not yet".
    let label = if app.backend.is_some() {
        capture_shared::capture_button_label(true, app.capturing)
    } else {
        "Capture now"
    };
    let mut capture = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([7, 14]))
        .style(theme::ghost_btn());
    // Demo never captures: the button is drawn but does nothing.
    if !demo && capture_shared::can_capture(app) {
        capture = capture.on_press(Message::CaptureNow);
    }
    let mut check = column![setting_row(
        row_title("Check the picture"),
        vec![hint(CHECK_PICTURE_HELP)],
        capture.into(),
    )]
    .spacing(8);
    if let Some(err) = capture_shared::preview_error(app) {
        check = check.push(err);
    }
    if let Some((handle, when)) = &app.preview {
        check = check
            .push(hint(format!("Preview: {when}")))
            .push(
                iced::widget::image(handle.clone())
                    .width(Fill)
                    .height(Length::Fixed(280.0)),
            )
            .push(hint(capture_shared::PREVIEW_CAPTION));
    }

    let mut rows = vec![
        method,
        monitor,
        check.into(),
        settings_rows::processes_row(app, demo),
    ];
    rows.extend(stream_rows(app, demo));

    // Only capture while Overwatch is on screen: not stored yet; the spec
    // default is Auto.
    rows.push(setting_row(
        title_line(
            "Only capture while Overwatch is on screen",
            false,
            &[Tag::New],
        ),
        vec![
            hint(FOCUS_GATE_HELP),
            hint(crate::settings_model::COMING_LATER),
        ],
        segmented(vec![
            Segment {
                label: "Auto",
                selected: true,
                msg: None,
            },
            Segment {
                label: "Off",
                selected: false,
                msg: None,
            },
        ]),
    ));
    rows
}

/// Pause between grabs and Stream frame rate.
fn stream_rows(app: &TrackerApp, demo: bool) -> Vec<Element<'_, Message>> {
    let config = &app.saved_config;
    let applies = stream_rows_apply(&app.desktop, app.session, config.capture_route);
    let where_line = if applies {
        STREAM_ACTIVE
    } else {
        STREAM_NOT_USED
    };

    let pause = pause_now(app);
    let mut pause_help = vec![hint(where_line)];
    if let Some(note) = pause.note.filter(|_| applies) {
        pause_help.push(hint(note));
    }
    if pause.locked {
        pause_help.push(lock_line(SCREENCAST_PAUSE_ENV));
        pause_help.push(hint(SERVICE_MAY_DIFFER));
    }
    let pause_toggle: Option<Box<dyn Fn(bool) -> Message>> = (!demo && applies && pause.can_toggle)
        .then(|| {
            Box::new(|on| Message::SettingsPage(Msg::Edit(ConfigEdit::PauseBetweenGrabs(on))))
                as Box<dyn Fn(bool) -> Message>
        });
    let pause_el = setting_row(
        title_line(
            "Pause between grabs",
            differs_from_default(config, DefaultRow::PauseBetweenGrabs),
            &[],
        ),
        pause_help,
        switch(pause.on, pause_toggle),
    );

    let fps = config.screencast_max_fps();
    let changed = differs_from_default(config, DefaultRow::FrameRate);
    let mut tail = row![hint(if applies {
        frame_rate_help()
    } else {
        STREAM_NOT_USED.to_string()
    })]
    .spacing(4)
    .align_y(Alignment::Center);
    if changed && applies {
        tail = tail.push(link(
            "Reset to default",
            (!demo).then_some(Message::SettingsPage(Msg::Edit(ConfigEdit::FrameRate(
                None,
            )))),
        ));
    }
    let control: Element<'_, Message> = if applies && !demo {
        dropdown(
            frame_rate_options(),
            Some(FrameRate(fps)),
            |r: FrameRate| Message::SettingsPage(Msg::Edit(ConfigEdit::FrameRate(Some(r.0)))),
        )
    } else {
        fixed_dropdown(FrameRate(fps).to_string())
    };
    let fps_el = setting_row(
        title_line("Stream frame rate", changed, &[Tag::AfterRestart]),
        vec![tail.into()],
        control,
    );
    vec![pause_el, fps_el]
}

fn reading_rows(app: &TrackerApp, demo: bool) -> Vec<Element<'_, Message>> {
    let config = &app.saved_config;
    let choice = ReaderChoice::from_switches(app.settings.new_reader, app.settings.classic_ocr);
    let can_turn_off =
        ocr_hint_lines(&stat_tracker::ocr_switch::Read::needing_ocr()).restart_and_packs;

    // Reader.
    let shown = app.page_state.reader_pending().unwrap_or(choice);
    let mut help: Vec<Element<'_, Message>> = reader_hints(shown, can_turn_off)
        .into_iter()
        .map(hint)
        .collect();
    if app.page_state.reader_pending().is_some() {
        help.push(note(CHECKING_PACKS, NoteTone::Info));
    }
    help.extend(settings::ocr_problem_lines(app, |problem| {
        note(problem, NoteTone::Warn)
    }));
    // STAT_TRACKER_OCR turns classic OCR on or off for the tracker it
    // starts, whatever is picked here.
    if app.ocr_control().env.is_some() {
        help.push(lock_line(OCR_ENV));
        help.push(hint(SERVICE_MAY_DIFFER));
    }
    let reader = setting_row(
        title_line(
            "Reader",
            differs_from_default(config, DefaultRow::Reader),
            &[],
        ),
        help,
        radio_list(&reader_options(choice, can_turn_off), shown, |c| {
            Message::SettingsPage(Msg::PickReader(c))
        }),
    );

    let name_toggle: Option<Box<dyn Fn(bool) -> Message>> = (!demo).then(|| {
        Box::new(|on| Message::SettingsPage(Msg::Edit(ConfigEdit::NameReader(on))))
            as Box<dyn Fn(bool) -> Message>
    });
    let name_reader = setting_row(
        title_line(
            "Name reader",
            differs_from_default(config, DefaultRow::NameReader),
            &[],
        ),
        vec![hint(NAME_READER_HELP)],
        switch(config.name_reader, name_toggle),
    );

    let mut rows = vec![reader, name_reader];
    if ocr_card_blocks(app.ocr_switch.daemon_off).reading_model {
        rows.push(reading_model_row(app, demo));
    }
    rows.push(ocr_workers_row(app, demo));
    rows.push(coming_later_row(
        "Report pictures",
        REPORT_PICTURES_HELP,
        &[Tag::New, Tag::AfterRestart],
        false,
    ));
    rows
}

/// Reading model: its state on the right, and Install when it is missing.
fn reading_model_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let state: Element<'_, Message> = if app.tessdata_installed {
        row![
            image(crate::icons::handle(crate::icons::Icon::Check))
                .width(14)
                .height(14),
            text("Installed with the app")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2),
        ]
        .spacing(6)
        .align_y(Alignment::Center)
        .into()
    } else if app.tessdata_busy {
        text("Installing…")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT_3)
            .into()
    } else {
        let mut b = button(
            text("Install")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
        )
        .padding(Padding::from([7, 14]))
        .style(theme::ghost_btn());
        if !demo {
            b = b.on_press(Message::InstallModel);
        }
        b.into()
    };
    let mut help = vec![hint(READING_MODEL_HELP)];
    if !app.tessdata_installed {
        help.push(note(settings::reading_model_status(false), NoteTone::Warn));
    }
    setting_row(row_title("Reading model"), help, state)
}

/// OCR workers: Auto or 1 to 8, locked when the environment sets it.
fn ocr_workers_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let config = &app.saved_config;
    let env = app.outside_var(OCR_THREADS_ENV);
    let lock = ocr_env_lock(env.as_deref());
    let resolved = config.ocr_threads_resolved();
    // The dot follows what is saved, as on every row. A count the
    // environment sets is shown by the lock line, not the dot (spec p7).
    let title = title_line(
        "OCR workers",
        differs_from_default(&app.saved_file, DefaultRow::OcrWorkers),
        &[],
    );
    if let Some(n) = lock {
        // The tracker caps the count at 8 (`ocr_threads_resolved`).
        let runs = n.clamp(1, OCR_WORKERS_MAX);
        return setting_row(
            title,
            vec![
                lock_line_with(OCR_THREADS_ENV, ocr_workers_help(runs as usize, false)),
                hint(SERVICE_MAY_DIFFER),
            ],
            fixed_dropdown(runs.to_string()),
        );
    }
    let help = vec![hint(ocr_workers_help(
        resolved,
        config.ocr_threads.is_none(),
    ))];
    if demo {
        return setting_row(
            title,
            help,
            fixed_dropdown(OcrWorkers(config.ocr_threads).to_string()),
        );
    }
    setting_row(
        title,
        help,
        dropdown(
            ocr_worker_options(),
            Some(OcrWorkers(config.ocr_threads)),
            |w: OcrWorkers| Message::SettingsPage(Msg::Edit(ConfigEdit::OcrWorkers(w.0))),
        ),
    )
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

/// The 0.6 spec's Capture and reading: a summary, then CAPTURE and READING.
pub(crate) fn body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    // Hairlines above and under each sub-heading (spec p7).
    column![
        summary(app),
        hairline(),
        sub_heading("Capture"),
        hairline(),
        divided(capture_rows(app, demo)),
        hairline(),
        sub_heading("Reading"),
        hairline(),
        divided(reading_rows(app, demo)),
        hint(CAPTURE_READING_FOOTER),
    ]
    .spacing(GRID_GAP)
    .width(Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_app() -> (tempfile::TempDir, TrackerApp) {
        crate::fixtures::test_app()
    }

    #[test]
    fn screenshot_label_is_short_on_the_new_page() {
        assert_eq!(route_label(CaptureRoute::Screenshot), "Screenshot");
        assert_eq!(route_label(CaptureRoute::Auto), "Automatic");
        assert_eq!(route_label(CaptureRoute::Screencast), "Screen stream");
    }

    #[test]
    fn capture_method_help_fits_the_session() {
        let x11 = capture_method_help(SessionKind::X11);
        let wayland = capture_method_help(SessionKind::Wayland);
        assert!(x11.contains("X11"));
        assert!(wayland.contains("KDE and GNOME"));
        for line in [x11, wayland] {
            assert!(!line.contains("niri"), "{line}");
            assert!(!line.contains("Older"), "{line}");
        }
    }

    #[test]
    fn no_note_on_this_page_says_older_screenshot() {
        let (_dir, mut app) = demo_app();
        for (desktop, session) in [
            ("GNOME", SessionKind::Wayland),
            ("GNOME", SessionKind::X11),
            ("KDE", SessionKind::X11),
        ] {
            app.desktop = desktop.into();
            app.session = session;
            app.settings.capture_route = CaptureRoute::Screenshot;
            let notes = page_route_notes(&app);
            assert!(!notes.is_empty(), "{desktop} {session:?}");
            for (line, _) in notes {
                assert!(!line.contains("Older screenshot"), "{line}");
            }
        }
    }

    #[test]
    fn reader_hints_follow_the_choice() {
        assert_eq!(
            reader_hints(ReaderChoice::NewOnly, true),
            [READER_HELP, NEW_ONLY_COST]
        );
        assert_eq!(
            reader_hints(ReaderChoice::NewWithOcrBackup, true),
            [READER_HELP, NEW_READER_HINT]
        );
        assert_eq!(reader_hints(ReaderChoice::ClassicOnly, true), [READER_HELP]);
        let held = reader_hints(ReaderChoice::ClassicOnly, false);
        assert!(held.contains(&CLASSIC_OCR_HINT_NOT_YET));
        assert!(
            !held.contains(&READER_HELP),
            "no line about an option that is not offered"
        );
        assert!(reader_hints(ReaderChoice::NewOnly, false).contains(&READER_HELP));
    }

    #[test]
    fn new_reader_only_is_offered_only_where_it_can_work() {
        let all = reader_options(ReaderChoice::NewWithOcrBackup, true);
        assert_eq!(all.len(), 3);
        let held = reader_options(ReaderChoice::NewWithOcrBackup, false);
        assert!(!held.iter().any(|(c, _)| *c == ReaderChoice::NewOnly));
        let kept = reader_options(ReaderChoice::NewOnly, false);
        assert!(kept.iter().any(|(c, _)| *c == ReaderChoice::NewOnly));
    }

    #[test]
    fn the_summary_names_each_thing_once() {
        let (_dir, app) = demo_app();
        let rows = summary_rows_with(&app, pause_with(&app, None, None), Vec::new());
        let labels: Vec<&str> = rows.iter().map(|(l, _)| *l).collect();
        assert_eq!(labels[..3], ["Capture", "Reading", "Reader packs"]);
        for (label, value) in &rows {
            assert!(
                !value.to_lowercase().starts_with(&label.to_lowercase()),
                "{label}: {value}"
            );
        }
    }

    /// The summary follows the session: on X11 the stream rows do not
    /// apply, so it never says the stream pauses, and a failed capture
    /// has its line.
    #[test]
    fn the_summary_and_error_follow_the_state() {
        let (_dir, mut app) = demo_app();
        app.desktop = "niri".into();
        app.session = SessionKind::Wayland;
        app.saved_config.capture_route = CaptureRoute::Screencast;
        let no_env =
            |app: &TrackerApp| summary_rows_with(app, pause_with(app, None, None), Vec::new());
        let wayland = no_env(&app);
        assert_eq!(
            wayland[0].1,
            format!(
                "Screen stream on {} (niri), paused between grabs.",
                monitor_phrase(
                    app.saved_config.capture_output.as_deref().unwrap_or(""),
                    &app.outputs
                )
            ),
            "{wayland:?}"
        );
        app.session = SessionKind::X11;
        app.preview_error = Some("No capture".into());
        let x11 = no_env(&app);
        assert!(!x11[0].1.contains("paused"), "{x11:?}");
        app.saved_config.capture_route = CaptureRoute::Auto;
        assert!(
            no_env(&app)[0].1.starts_with("Direct capture on"),
            "Auto on X11"
        );
        assert!(capture_shared::preview_error(&app).is_some());
        let _ = body(&app, false);
    }

    /// A demo shows no lock from outside, whatever the environment sets.
    /// A live app shows each one it sets. Neither reads the host's.
    #[test]
    fn only_a_live_app_shows_the_environment_locks() {
        let (_dir, mut app) = demo_app();
        app.outside_env = Some(crate::settings_env::OutsideEnv {
            vars: vec![
                (CAPTURE_ENV, "screenshot".into()),
                (OCR_ENV, "off".into()),
                (OCR_THREADS_ENV, "2".into()),
            ],
            ..Default::default()
        });
        assert!(env_locks(&app, pause_now(&app)).is_empty());
        assert_eq!(summary_rows(&app).len(), 3, "no outside row in a demo");
        assert_eq!(app.ocr_control().env, None);
        let _ = body(&app, true);
        app.fixture = None;
        assert_eq!(
            env_locks(&app, pause_now(&app)),
            [
                "Capture method, by STAT_TRACKER_CAPTURE",
                "Classic OCR, by STAT_TRACKER_OCR",
                "OCR workers, by STAT_TRACKER_OCR_THREADS",
            ]
        );
        assert_eq!(
            app.ocr_control().env,
            Some(stat_tracker::config::OcrSetting::Off)
        );
    }
}
