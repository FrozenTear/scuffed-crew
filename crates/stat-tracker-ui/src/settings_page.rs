//! 0.6 Settings page (redesign variant A, top tabs).
//!
//! Status strip with Save and Restart on top, then everyday settings on the
//! left and folding ADVANCED rows on the right (one column below
//! [`crate::settings_model::TWO_COLUMN_MIN`]). The old card layout stays in [`crate::settings::view`]
//! for `SCUFFED_TRACKER_LAYOUT=classic`; both draw from the same form, so
//! Save writes the same config either way.
//!
//! Rows the tracker cannot store yet are drawn off and greyed with
//! [`crate::settings_model::WAITS_FOR_S11`]. Capture and reading is drawn
//! by [`crate::capture_reading`]; the page's state and messages are in
//! [`crate::settings_update`].

use iced::widget::scrollable::{Direction, Scrollbar};
use iced::widget::{Row, button, column, container, row, scrollable, space, text};
use iced::{Alignment, Background, Border, Element, Fill, Padding, Shadow, Theme};

use crate::app::{Message, TrackerApp};
use crate::daemon::DaemonVerb;
use crate::form_widgets::{action_btn, hint};
use crate::layout::{SCROLLBAR_WIDTH, SETTINGS_SCROLL_GUTTER};
use crate::settings::{self, SAVE_LABEL};
use crate::settings_model::{
    AdvancedSection, AdvancedValues, EverydaySection, ReaderChoice, SINGLE_COLUMN_MAX,
    changes_waiting, changes_waiting_label, columns_for,
};
use crate::settings_rows::{self, AFTER_RESTART_TAG, changed_dot, stub_row, sub_heading, tag};
use crate::settings_update::Msg;
use crate::theme::{
    self, ACCENT, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, GRID_GAP, PAD_INNER, SIZE_BODY,
    SIZE_LABEL, SIZE_META, TEXT, TEXT_2, TEXT_3,
};
use crate::{update, widgets};

/// Copy under the Website account summary.
const WEBSITE_ACCOUNT_HINT: &str =
    "Server address and token are under Advanced, Website connection.";
/// Sub-heading of the Sharing rows inside Website account.
const SHARING_HEADING: &str = "Sharing · both start off";
/// "Read my rank after Comp games" help.
const RANK_HELP: &str = "Saves rank, rank change and its label. Applies from the next game.";
/// "Share stat pictures" help.
const STAT_PICTURES_HELP: &str = "Uploads the grayscale stat area after each game to help train the reader. No names. Kept 30 days. Report pictures are a separate setting in Capture and reading.";
/// Link under the Sharing rows. The page it opens is not built yet.
const WHAT_LEAVES_LABEL: &str = "What leaves your PC ›";
/// Copy for the legend under the status strip. The spec line says changes
/// apply as you make them; this page still has Save, so it says that.
const LEGEND_APPLY: &str = "Changes apply when you save, except those tagged After restart.";
const LEGEND_CHANGED: &str = "changed from default";
const RUN_SETUP_LABEL: &str = "Run setup again";
const DEMO_NOTE: &str =
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
    page = page
        .push(status_strip(app, demo))
        .push(setup_and_legend(demo));

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

    // The one-column page is no wider than SINGLE_COLUMN_MAX and centred.
    let page: Element<'_, Message> = if two_columns {
        page.into()
    } else {
        container(page.max_width(SINGLE_COLUMN_MAX))
            .width(Fill)
            .center_x(Fill)
            .into()
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

/// Tracker status and version on the left; on the right the changes
/// waiting for a restart, Restart tracker and Save.
fn status_strip(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let version = app
        .installed_version
        .clone()
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string());
    // Demo mode shows the sample tracker as running, like the top bar.
    let running = app.daemon.running() || demo;
    let mut strip = Row::new()
        .spacing(16)
        .align_y(Alignment::Center)
        .push(widgets::status_stub(
            if running {
                "Tracker running"
            } else {
                "Tracker stopped"
            },
            widgets::header_status_color(widgets::header_status_for(app)),
        ))
        .push(meta(format!("Version {version}")));
    if let Some(game) = app.games.first() {
        strip = strip.push(meta(format!(
            "Last game {}",
            crate::model::local_hm(game.played_at)
        )));
    }
    strip = strip.push(space().width(Fill));
    if app.daemon_busy {
        strip = strip.push(meta("Working…"));
    }
    let waiting = app.changes_waiting();
    if waiting > 0 {
        strip = strip
            .push(tag(AFTER_RESTART_TAG))
            .push(meta(changes_waiting_label(waiting)));
    }
    // Restart runs Start when the tracker is stopped, so the one button
    // always gets it running with the saved config.
    let verb = if running {
        DaemonVerb::Restart
    } else {
        DaemonVerb::Start
    };
    strip = strip.push(strip_btn(
        "Restart tracker",
        waiting > 0,
        (!demo && !app.daemon_busy).then_some(Message::Daemon(verb)),
    ));
    let unsaved = changes_waiting(
        &app.saved_config,
        &app.settings.to_config(&app.saved_config),
    ) > 0;
    strip = strip.push(strip_btn(
        SAVE_LABEL,
        unsaved,
        (!demo).then_some(Message::SaveSettings),
    ));
    container(strip)
        .padding(PAD_INNER)
        .width(Fill)
        .style(theme::surface_panel)
        .into()
}

/// A strip button. Without a message it is drawn but does nothing (demo).
fn strip_btn(
    label: &'static str,
    primary: bool,
    msg: Option<Message>,
) -> Element<'static, Message> {
    let mut b = button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([8, 16]));
    b = if primary {
        b.style(primary_btn)
    } else {
        b.style(theme::ghost_btn())
    };
    if let Some(msg) = msg {
        b = b.on_press(msg);
    }
    b.into()
}

/// Filled accent button. Same look when disabled, so the demo shows it.
fn primary_btn(_theme: &Theme, status: button::Status) -> button::Style {
    button::Style {
        background: Some(Background::Color(match status {
            button::Status::Hovered | button::Status::Pressed => iced::Color { a: 0.85, ..ACCENT },
            _ => ACCENT,
        })),
        text_color: TEXT,
        border: Border {
            color: ACCENT,
            width: 1.0,
            radius: theme::RADIUS_CHIP.into(),
        },
        shadow: Shadow::default(),
        snap: false,
    }
}

/// "Run setup again" on the left, the legend on the right.
fn setup_and_legend(demo: bool) -> Element<'static, Message> {
    let mut setup = button(
        text(RUN_SETUP_LABEL)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(ACCENT),
    )
    .padding(0)
    .style(fold_style);
    if !demo {
        setup = setup.on_press(Message::Setup(crate::setup_guide::SetupMessage::Open));
    }
    Row::new()
        .spacing(GRID_GAP)
        .align_y(Alignment::Center)
        .push(setup)
        .push(space().width(Fill))
        .push(meta(LEGEND_APPLY))
        .push(
            row![changed_dot(), meta(LEGEND_CHANGED)]
                .spacing(6)
                .align_y(Alignment::Center),
        )
        .into()
}

fn everyday_column(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut col = column![].spacing(GRID_GAP).width(Fill);
    for section in EverydaySection::ALL {
        let body = match section {
            EverydaySection::PlayerSessions => column![
                settings_rows::names_row(app, demo),
                settings_rows::session_window_row(app),
            ]
            .spacing(14)
            .into(),
            EverydaySection::Companion => column![
                widgets::overlay_toggle(app.overlay_showing(), app.game_running),
                settings::companion_body(app, demo),
            ]
            .spacing(8)
            .into(),
            EverydaySection::WebsiteAccount => website_account(app),
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
        space().height(4),
        sub_heading(SHARING_HEADING),
        stub_row("Read my rank after Comp games", Some(RANK_HELP), false),
        stub_row("Share stat pictures", Some(STAT_PICTURES_HELP), false),
        // The page this opens is not built yet (PLAN-0.6 carry-over).
        text(WHAT_LEAVES_LABEL)
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(ACCENT),
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
    let values = advanced_values(app);
    for section in AdvancedSection::ALL {
        let open = app.page_state.is_open(section);
        let mut fold = column![fold_header(section, open, &values)]
            .spacing(0)
            .width(Fill);
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

/// What the closed Advanced rows summarise, from the form as it stands.
fn advanced_values(app: &TrackerApp) -> AdvancedValues {
    let config = app.settings.to_config(&app.saved_config);
    let reader = ReaderChoice::from_switches(app.settings.new_reader, app.settings.classic_ocr);
    let monitor = settings::nonempty(&app.settings.capture_output).unwrap_or_else(|| {
        app.outputs
            .first()
            .cloned()
            .unwrap_or_else(|| "first monitor".to_string())
    });
    let retention = config.report_kit_retention();
    AdvancedValues {
        route: crate::capture_reading::route_label(config.capture_route),
        monitor,
        reader,
        autostart: app.daemon.autostart,
        auto_detect: config.auto_detect.enabled,
        poll_ms: config
            .auto_detect
            .poll_interval(reader.new_reader())
            .as_millis() as u64,
        cooldown_secs: config.auto_detect.cooldown_secs,
        finished_close_secs: config.finished_game_close_secs,
        games: app.games.len(),
        sync_on: crate::app::sync_on_for(&config),
        debug_images: config.debug_ocr,
        compare_frames: config.debug_portal_frames,
        reader_log: config.shadow_recognizer,
        report_max_games: retention.max_games,
        report_max_mb: retention.max_bytes / (1024 * 1024),
    }
}

fn fold_header(
    section: AdvancedSection,
    open: bool,
    values: &AdvancedValues,
) -> Element<'static, Message> {
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
                text(section.summary(values))
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            ]
            .spacing(4)
            .width(Fill),
            text(if open { "⌄" } else { "›" })
                .size(18)
                .font(FONT_SEMIBOLD)
                .color(TEXT_2),
        ]
        .spacing(GRID_GAP)
        .align_y(Alignment::Center),
    )
    .padding(PAD_INNER)
    .width(Fill)
    .style(fold_style)
    .on_press(Message::SettingsPage(Msg::ToggleAdvanced(section)))
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
        AdvancedSection::CaptureReading => crate::capture_reading::body(app, demo),
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

fn meta(copy: impl Into<String>) -> Element<'static, Message> {
    text(copy.into())
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3)
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
            let _ = app.update(Message::SettingsPage(Msg::ToggleAdvanced(section)));
        }
        for width in [640.0, 1600.0] {
            let _ = view(&app, width);
        }
    }

    #[test]
    fn copy_has_no_em_dashes() {
        for line in [
            WEBSITE_ACCOUNT_HINT,
            SHARING_HEADING,
            RANK_HELP,
            STAT_PICTURES_HELP,
            WHAT_LEAVES_LABEL,
            LEGEND_APPLY,
            LEGEND_CHANGED,
            RUN_SETUP_LABEL,
            DEMO_NOTE,
        ] {
            assert!(!line.contains('\u{2014}'), "{line}");
        }
    }
}
