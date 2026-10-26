//! 0.6 Settings page (redesign variant A, top tabs).
//!
//! Status strip with Save and Restart on top, then everyday settings on the
//! left and folding ADVANCED rows on the right (one column below
//! [`crate::settings_model::TWO_COLUMN_MIN`]). The old card layout stays in [`crate::settings::view`]
//! for `SCUFFED_TRACKER_LAYOUT=classic`; both draw from the same form, so
//! Save writes the same config either way.
//!
//! Rows the tracker cannot store yet are drawn off and greyed with
//! [`WAITS_FOR_S11`]. Stream frame rate, pause between grabs, name reader
//! and OCR workers are read-only here until the next GUI milestone.

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{
    Column, Row, button, checkbox, column, container, row, scrollable, space, text,
};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Shadow, Theme};

use crate::app::{Message, TrackerApp};
use crate::daemon::DaemonVerb;
use crate::layout::{SCROLLBAR_WIDTH, SETTINGS_SCROLL_GUTTER};
use crate::settings::{
    self, CLASSIC_OCR_RESTART, DISMISS_REFUSAL_LABEL, NEW_READER_IMPLIED, NEW_READER_RESTART,
    SAVE_LABEL, SettingsField, action_btn, card_problem, checkbox_style, field_caption,
    field_input, hint, ocr_card_blocks, ocr_hint_lines, output_chip, pack_status_line,
};
use crate::settings_model::{
    AdvancedSection, EverydaySection, OCR_THREADS_ENV, ReaderChoice, SINGLE_COLUMN_MAX,
    WAITS_FOR_S11, columns_for, frame_rate_line, ocr_workers_line, ocr_workers_locked, pause_line,
};
use crate::theme::{
    self, ACCENT, BORDER, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAD_INNER, SIZE_BODY,
    SIZE_LABEL, SIZE_META, TEXT, TEXT_2, TEXT_3,
};
use crate::{update, widgets};

/// Help under the Report pictures row.
pub(crate) const REPORT_PICTURES_HELP: &str = "Stat-area crops in misread reports. Separate from Stat pictures. Default Off: reports show them unticked, so you choose each one. Names are never sent.";
/// Last line of Capture and reading.
pub(crate) const CAPTURE_READING_FOOTER: &str = "Only choices that work on this desktop are shown. Opening this page never captures; only Capture now does.";
/// Help under "Only capture while Overwatch is on screen".
pub(crate) const FOCUS_GATE_HELP: &str =
    "Skips grabs while another window covers the game, so nothing else on screen is read.";
/// Copy under the Website account summary.
pub(crate) const WEBSITE_ACCOUNT_HINT: &str =
    "Server address and token are under Advanced, Website connection.";
/// Copy under the two Sharing rows.
pub(crate) const SHARING_HINT: &str = "Both start Off and only share what you pick.";
/// Copy for the legend under the status strip.
pub(crate) const LEGEND_CHANGED: &str = "Changed from default";
pub(crate) const LEGEND_RESTART: &str = "Used after the tracker restarts";
pub(crate) const AFTER_RESTART_TAG: &str = "After restart";
pub(crate) const NEW_TAG: &str = "New in 0.6";
pub(crate) const RUN_SETUP_LABEL: &str = "Run setup again";
pub(crate) const SAVE_HINT: &str = "Writes config and the companion shortcut.";
pub(crate) const DEMO_NOTE: &str =
    "Demo mode: settings, the tracker service, and stored data are not changed.";

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let demo = app.fixture.is_some();
    let two_columns = columns_for(content_width - SETTINGS_SCROLL_GUTTER) == 2;
    let mut page = column![].spacing(GRID_GAP).width(Fill);

    if let Some(info) = &app.update
        && app.update_banner_visible()
    {
        page = page.push(update::banner(
            info,
            &app.update_progress,
            &app.update_plan,
            &app.update_notes,
            app.update_notes_ui(),
            app.update_notes_expanded,
            app.restart_need(),
            app.package_probe(),
        ));
    }
    if demo {
        page = page.push(widgets::empty_surface(DEMO_NOTE));
    }
    page = page.push(status_strip(app, demo)).push(setup_and_legend());

    if two_columns {
        page = page.push(
            row![everyday_column(app, demo), advanced_column(app, demo)]
                .spacing(GRID_GAP)
                .align_y(Alignment::Start)
                .width(Fill),
        );
    } else {
        page = page
            .push(everyday_column(app, demo))
            .push(advanced_column(app, demo));
    }

    let page = if two_columns {
        page
    } else {
        page.max_width(SINGLE_COLUMN_MAX)
    };
    scrollable(container(page).width(Fill).padding(Padding {
        right: SETTINGS_SCROLL_GUTTER,
        ..Padding::ZERO
    }))
    .direction(Direction::Vertical(
        Scrollbar::new()
            .width(SCROLLBAR_WIDTH)
            .scroller_width(SCROLLBAR_WIDTH),
    ))
    .width(Fill)
    .height(Fill)
    .into()
}

/// Tracker status, version, Restart tracker and Save.
fn status_strip(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let version = app
        .installed_version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    let mut strip = Row::new()
        .spacing(GRID_GAP)
        .align_y(Alignment::Center)
        .push(widgets::status_stub(
            if app.daemon.running() {
                "Tracker running"
            } else {
                "Tracker stopped"
            },
            widgets::header_status_color(widgets::header_status_for(app)),
        ))
        .push(meta(format!("Version {version}")));
    if app.daemon_busy {
        strip = strip.push(meta("Working…"));
    }
    if !demo {
        strip = strip.push(space().width(Fill)).push(meta(SAVE_HINT));
        if app.daemon.running() && !app.daemon_busy {
            strip = strip.push(action_btn(
                "Restart tracker",
                false,
                Message::Daemon(DaemonVerb::Restart),
            ));
        }
        strip = strip.push(action_btn(SAVE_LABEL, true, Message::SaveSettings));
    }
    container(strip.wrap())
        .padding(PAD_INNER)
        .width(Fill)
        .style(theme::surface_panel)
        .into()
}

fn setup_and_legend() -> Element<'static, Message> {
    Row::new()
        .spacing(GRID_GAP)
        .align_y(Alignment::Center)
        .push(action_btn(
            RUN_SETUP_LABEL,
            false,
            Message::Setup(crate::setup_guide::SetupMessage::Open),
        ))
        .push(space().width(Fill))
        .push(
            row![changed_dot(), meta(LEGEND_CHANGED)]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .push(
            row![tag(AFTER_RESTART_TAG), meta(LEGEND_RESTART)]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .wrap()
        .into()
}

fn everyday_column(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut col = column![].spacing(GRID_GAP).width(Fill);
    for section in EverydaySection::ALL {
        let body = match section {
            EverydaySection::PlayerSessions => settings::player_fields_row(
                &app.settings.player_name,
                &app.settings.session_window_secs,
                demo,
            ),
            EverydaySection::Companion => column![
                widgets::overlay_toggle(app.overlay_showing(), app.game_running),
                settings::companion_body(app, demo),
            ]
            .spacing(8)
            .into(),
            EverydaySection::WebsiteAccount => website_account(app),
            EverydaySection::Sharing => column![
                stub_row("Read my rank after Competitive games", None, false),
                stub_row("Share stat pictures", None, false),
                hint(SHARING_HINT),
            ]
            .spacing(8)
            .into(),
            EverydaySection::Updates => settings::updates_body(app, demo),
        };
        col = col.push(card(section.title(), body));
    }
    col.into()
}

fn website_account(app: &TrackerApp) -> Element<'_, Message> {
    let line = match app.saved_config.sync.as_ref() {
        Some(sync) if app.sync_on() => format!("Connected to {}", sync.server_url),
        _ => "Not connected. Games stay on this computer.".to_string(),
    };
    column![
        text(line).size(SIZE_BODY).font(FONT_MEDIUM).color(TEXT),
        stub_row("Upload games after each match", None, false),
        hint(WEBSITE_ACCOUNT_HINT),
    ]
    .spacing(8)
    .into()
}

fn advanced_column(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut col = column![
        text("ADVANCED")
            .size(SIZE_LABEL)
            .font(FONT_SEMIBOLD)
            .color(TEXT_3)
    ]
    .spacing(8)
    .width(Fill);
    for section in AdvancedSection::ALL {
        let open = app.advanced_open.is_open(section);
        let mut fold = column![fold_header(section, open)].spacing(0).width(Fill);
        if open {
            fold = fold.push(
                container(advanced_body(section, app, demo))
                    .padding(Padding {
                        top: 0.0,
                        bottom: PAD_INNER,
                        left: PAD_INNER,
                        right: PAD_INNER,
                    })
                    .width(Fill),
            );
        }
        col = col.push(container(fold).width(Fill).style(theme::surface_panel));
    }
    col.into()
}

fn fold_header(section: AdvancedSection, open: bool) -> Element<'static, Message> {
    let mut title = Row::new().spacing(8).align_y(Alignment::Center).push(
        text(section.title())
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(TEXT),
    );
    if section.after_restart() {
        title = title.push(tag(AFTER_RESTART_TAG));
    }
    button(
        row![
            column![
                title,
                text(section.summary())
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            ]
            .spacing(4)
            .width(Fill),
            text(if open { "Hide" } else { "Show" })
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2),
        ]
        .spacing(GRID_GAP)
        .align_y(Alignment::Center),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(fold_style)
    .on_press(Message::ToggleAdvanced(section))
    .into()
}

fn fold_style(_theme: &Theme, _status: button::Status) -> button::Style {
    button::Style {
        background: None,
        text_color: TEXT,
        border: Border::default(),
        shadow: Shadow::default(),
        snap: false,
    }
}

fn advanced_body<'a>(
    section: AdvancedSection,
    app: &'a TrackerApp,
    demo: bool,
) -> Element<'a, Message> {
    match section {
        AdvancedSection::CaptureReading => capture_reading_body(app, demo),
        AdvancedSection::TrackerService => column![
            settings::service_body(app, demo),
            sub_heading("Match watching"),
            settings::auto_detect_body(app, demo),
        ]
        .spacing(8)
        .into(),
        AdvancedSection::StoredData => settings::data_body(app, demo),
        AdvancedSection::WebsiteConnection => settings::sync_body(app, demo),
        AdvancedSection::Troubleshooting => column![
            settings::diagnostics_body(app, demo),
            settings::shadow_reader_block(app),
            troubleshooting_actions(),
        ]
        .spacing(GRID_GAP)
        .into(),
    }
}

/// Report a problem and About, then Uninstall. Uninstall is only here.
fn troubleshooting_actions() -> Element<'static, Message> {
    column![
        hint("Crash, low FPS, or anything else. You see the report before it is sent."),
        Row::new()
            .spacing(8)
            .align_y(Alignment::Center)
            .push(action_btn(
                "Report a problem",
                false,
                Message::Report(crate::report::ReportMessage::Open(None)),
            ))
            .push(action_btn("About", false, Message::OpenAbout))
            .push(space().width(Fill))
            .push(
                button(
                    text("Uninstall")
                        .size(SIZE_META)
                        .font(FONT_SEMIBOLD)
                        .color(theme::DANGER),
                )
                .padding(Padding::from([8, 16]))
                .style(theme::danger_btn(false))
                .on_press(Message::OpenUninstall),
            )
            .wrap(),
    ]
    .spacing(8)
    .into()
}

/// Claude's Capture and reading spec: a summary, then CAPTURE and READING.
fn capture_reading_body(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
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
                format!(
                    "{} on {monitor}",
                    settings::capture_route_label(app.settings.capture_route)
                ),
            ),
            summary_line("Reading", choice.label().to_string()),
            summary_line("Packs", pack_status_line(app.ocr_switch.packs.as_ref())),
        ]
        .spacing(4),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::stat_box);

    let mut body = column![
        summary,
        sub_heading("Capture"),
        settings::capture_body(app, demo)
    ]
    .spacing(GRID_GAP)
    .width(Fill);

    body = body.push(field_input(
        "Game process names",
        "Overwatch.exe",
        &app.settings.game_process_names,
        SettingsField::ProcessNames,
        false,
        demo,
        Some("Comma-separated. Empty = always allow capture."),
    ));

    let (pause, pause_changed) = pause_line(config.screencast_pause_override());
    body = body.push(read_only_row(
        "Pause between grabs",
        pause.to_string(),
        pause_changed,
        None,
    ));
    let (fps, fps_changed) = frame_rate_line(config.screencast_max_fps());
    body = body.push(read_only_row(
        "Stream frame rate",
        fps,
        fps_changed,
        Some("Screen-share grabs per second, 1 to 5."),
    ));
    body = body.push(stub_row(
        "Only capture while Overwatch is on screen",
        Some(FOCUS_GATE_HELP),
        false,
    ));

    body = body.push(sub_heading("Reading"));
    let mut chips = Row::new().spacing(8);
    for c in ReaderChoice::ALL {
        chips = chips.push(output_chip(c.label(), c == choice, Message::PickReader(c)));
    }
    body = body.push(chips.wrap());

    let mut reading = column![].spacing(4);
    let lines = ocr_hint_lines(&stat_tracker::ocr_switch::Read::needing_ocr());
    reading = reading.push(hint(lines.hint));
    if lines.restart_and_packs {
        reading = reading.push(hint(CLASSIC_OCR_RESTART));
    }
    if choice.new_reader() {
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
        body = body.push(settings::reading_model_block(app, demo));
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
        None,
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

fn card<'a>(title: &'static str, body: Element<'a, Message>) -> Element<'a, Message> {
    container(
        column![
            text(title).size(SIZE_BODY).font(FONT_BOLD).color(TEXT),
            body,
        ]
        .spacing(8),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(theme::surface_panel)
    .into()
}

fn sub_heading(label: &'static str) -> Element<'static, Message> {
    text(label.to_ascii_uppercase())
        .size(SIZE_LABEL)
        .font(FONT_SEMIBOLD)
        .color(TEXT_3)
        .into()
}

fn meta(copy: impl Into<String>) -> Element<'static, Message> {
    text(copy.into())
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3)
        .into()
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

/// A setting shown but not editable here yet. The accent dot marks a value
/// that differs from the default.
fn read_only_row(
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
fn stub_row(
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
fn tag(label: &'static str) -> Element<'static, Message> {
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
fn changed_dot() -> Element<'static, Message> {
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

    fn app() -> (tempfile::TempDir, TrackerApp) {
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::cli::Cli {
            data_dir: dir.path().to_path_buf(),
            fixture: Some(crate::cli::FixtureKind::Empty),
            seasons_url: None,
            companion: false,
            help: false,
            version: false,
            replace: false,
        };
        let app = TrackerApp::new(cli).0;
        (dir, app)
    }

    #[test]
    fn page_draws_at_both_widths_with_every_row_open() {
        let (_dir, mut app) = app();
        for width in [
            640.0,
            crate::settings_model::TWO_COLUMN_MIN + SETTINGS_SCROLL_GUTTER + 1.0,
        ] {
            let _ = view(&app, width);
        }
        for section in AdvancedSection::ALL {
            app.advanced_open.toggle(section);
        }
        for width in [640.0, 1600.0] {
            let _ = view(&app, width);
        }
    }

    #[test]
    fn reader_pick_sets_both_switches() {
        let (_dir, mut app) = app();
        for choice in ReaderChoice::ALL {
            let _ = app.update(Message::PickReader(choice));
            assert_eq!(
                ReaderChoice::from_switches(app.settings.new_reader, app.settings.classic_ocr),
                choice
            );
        }
    }

    #[test]
    fn advanced_rows_fold_through_update() {
        let (_dir, mut app) = app();
        assert!(!app.advanced_open.any_open());
        let _ = app.update(Message::ToggleAdvanced(AdvancedSection::StoredData));
        assert!(app.advanced_open.is_open(AdvancedSection::StoredData));
        let _ = app.update(Message::ToggleAdvanced(AdvancedSection::StoredData));
        assert!(!app.advanced_open.any_open());
    }

    #[test]
    fn copy_has_no_em_dashes() {
        for line in [
            REPORT_PICTURES_HELP,
            CAPTURE_READING_FOOTER,
            FOCUS_GATE_HELP,
            WEBSITE_ACCOUNT_HINT,
            SHARING_HINT,
            LEGEND_CHANGED,
            LEGEND_RESTART,
            AFTER_RESTART_TAG,
            NEW_TAG,
            RUN_SETUP_LABEL,
            SAVE_HINT,
            DEMO_NOTE,
        ] {
            assert!(!line.contains('\u{2014}'), "{line}");
        }
    }

    #[test]
    fn report_pictures_copy_matches_the_spec() {
        assert_eq!(
            REPORT_PICTURES_HELP,
            "Stat-area crops in misread reports. Separate from Stat pictures. Default Off: reports show them unticked, so you choose each one. Names are never sent."
        );
    }
}
