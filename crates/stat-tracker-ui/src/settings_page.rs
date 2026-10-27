//! 0.6 Settings page (redesign variant A, top tabs; spec p6, p7, p9).
//!
//! Under the "Settings" heading: the status strip (status, version, last
//! capture; on the right the changes waiting and Restart tracker), "Run
//! setup again" and the legend, then the everyday cards on the left and
//! the ADVANCED rows in one panel on the right. Below
//! [`crate::settings_model::TWO_COLUMN_MIN`] the page is one column of at
//! most [`crate::settings_model::SINGLE_COLUMN_MAX`], left-aligned on the
//! page gutter, and the strip puts its right group on a second line.
//!
//! There is no Save: every change applies as it is made
//! ([`crate::settings_update`]). The old card layout with its Save stays in
//! [`crate::settings::view`] for `SCUFFED_TRACKER_LAYOUT=classic`; both
//! edit the same form and write through [`crate::settings_save`]. The
//! window chrome scrolls this page like every other tab.

use iced::widget::{Column, Row, button, column, container, image, row, space, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Padding, Shadow, Theme};

use crate::app::{Message, TrackerApp};
use crate::daemon::DaemonVerb;
use crate::form_widgets::DEMO_NOTE;
use crate::settings_cards;
use crate::settings_model::{
    AdvancedSection, AdvancedValues, COLUMN_GAP, EverydaySection, ReaderChoice,
    changes_waiting_words, columns_for, monitor_phrase, page_width, strip_version,
};
use crate::settings_rows::{Tag, changed_dot, hairline, link, tag};
use crate::settings_update::Msg;
use crate::theme::{
    self, ACCENT, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META, TEXT,
    TEXT_2, TEXT_3,
};
use crate::{tracker_status, update, widgets};

/// Space between the status items on the strip's left (spec about 24).
pub(crate) const STRIP_STATUS_GAP: f32 = 24.0;
/// Space between the strip's right-hand items (spec about 12).
pub(crate) const STRIP_ACTION_GAP: f32 = 12.0;
/// Padding inside a card and the strip (spec about 18 to 20).
const CARD_PAD: f32 = 20.0;
/// Space between cards (spec 16).
const CARD_GAP: f32 = 16.0;
/// Button corner (spec about 10).
const BUTTON_RADIUS: f32 = 10.0;
/// Space between the legend's dot and its words.
const LEGEND_DOT_GAP: f32 = 6.0;

/// The legend under the strip (spec p6).
const LEGEND_APPLY: &str = "Changes apply as you make them: typed values on Enter, Website connection on its Apply. Those tagged After restart wait for a tracker restart.";
const LEGEND_CHANGED: &str = "changed from default";
const RUN_SETUP_LABEL: &str = "Run setup again";

pub fn view(app: &TrackerApp, content_width: f32) -> Element<'_, Message> {
    let demo = app.fixture.is_some();
    let two_columns = columns_for(content_width) == 2;
    let mut page = column![].spacing(CARD_GAP).width(Fill);

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
        .push(status_strip(app, demo, two_columns))
        .push(setup_and_legend(demo));

    if two_columns {
        page = page.push(
            row![everyday_column(app, demo), advanced_column(app, demo)]
                .spacing(COLUMN_GAP)
                .align_y(Alignment::Start)
                .width(Fill),
        );
    } else {
        page = page
            .push(everyday_column(app, demo))
            .push(advanced_column(app, demo));
    }

    // Left-aligned on the gutter in both modes (spec p9), capped so the
    // controls stay near their labels.
    container(page.max_width(page_width(content_width)))
        .width(Fill)
        .align_left(Fill)
        .into()
}

/// The strip's left group: the status with its check, Version (which
/// opens About), and the last capture.
fn strip_status(app: &TrackerApp) -> Element<'_, Message> {
    let running = app.daemon.running() || app.fixture.is_some();
    // Stopped is red whatever the token does; a rejected token turns a
    // running tracker's mark amber. The words always say whether it runs.
    let status = if !running {
        tracker_status::HeaderStatus::Down
    } else if tracker_status::sync_token_rejected(app) {
        tracker_status::HeaderStatus::TokenRejected
    } else {
        tracker_status::HeaderStatus::Waiting
    };
    let mark: Element<'_, Message> = if status == tracker_status::HeaderStatus::Waiting {
        settings_cards::check_icon(16.0)
    } else {
        container(space().width(8).height(8))
            .width(8)
            .height(8)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(tracker_status::header_status_color(
                    status,
                ))),
                border: Border {
                    radius: theme::RADIUS_CHIP.into(),
                    ..Border::default()
                },
                ..container::Style::default()
            })
            .into()
    };
    let words = text(tracker_status::strip_label(running))
        .size(SIZE_BODY)
        .font(FONT_BOLD)
        .color(TEXT);
    let version = button(meta_pair(
        "Version",
        strip_version(app.installed_version.as_deref()),
    ))
    // No side padding, so the gap to the status is STRIP_STATUS_GAP.
    .padding(Padding::from([2, 0]))
    .style(theme::quiet_btn(6.0))
    .on_press(Message::OpenAbout);
    let mut left = Row::new()
        .spacing(STRIP_STATUS_GAP)
        .align_y(Alignment::Center)
        .push(row![mark, words].spacing(8).align_y(Alignment::Center))
        .push(version);
    if let Some(at) = tracker_status::last_game_when(&app.games, app.clock) {
        left = left.push(meta_pair("Last capture", at));
    }
    left.wrap().into()
}

/// The strip's right group: Working, the waiting count with its tag, and
/// the tracker button last.
fn strip_actions(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let waiting = app.changes_waiting();
    let mut right = Row::new()
        .spacing(STRIP_ACTION_GAP)
        .align_y(Alignment::Center);
    if app.daemon_busy {
        right = right.push(meta("Working…"));
    }
    if waiting > 0 {
        let (count, words) = changes_waiting_words(waiting);
        right = right.push(tag(Tag::AfterRestart)).push(
            row![
                text(count).size(SIZE_META).font(FONT_BOLD).color(TEXT),
                text(words).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2),
            ]
            .align_y(Alignment::Center),
        );
    }
    // A stopped tracker gets Start tracker, a running one Restart tracker;
    // both get it running with the saved config.
    let running = app.daemon.running() || demo;
    let (verb, label) = strip_action(running);
    let primary = waiting > 0 || !running;
    let enabled = !demo && !app.daemon_busy;
    let mut b = button(text(label).size(SIZE_META).font(FONT_BOLD).color(TEXT))
        .padding(Padding::from([9, 18]));
    // Dimmed whenever it cannot be pressed, demo included.
    b = if primary {
        b.style(primary_btn(enabled))
    } else {
        b.style(ghost_btn(enabled))
    };
    if enabled {
        b = b.on_press(Message::Daemon(verb));
    }
    right.push(b).into()
}

/// Tracker status, version and last capture on the left; changes waiting
/// and Restart tracker on the right. One line in two-column mode; in one
/// column the right group goes on a second line, end-aligned (spec p9).
fn status_strip(app: &TrackerApp, demo: bool, two_columns: bool) -> Element<'_, Message> {
    let left = strip_status(app);
    let right = strip_actions(app, demo);
    let body: Element<'_, Message> = if two_columns {
        row![left, space().width(Fill), right]
            .spacing(STRIP_ACTION_GAP)
            .align_y(Alignment::Center)
            .into()
    } else {
        column![left, container(right).width(Fill).align_right(Fill)]
            .spacing(12)
            .into()
    };
    container(body)
        .padding(Padding::from([14.0, CARD_PAD]))
        .width(Fill)
        .style(theme::surface_panel)
        .into()
}

/// What the strip's tracker button runs and says.
fn strip_action(running: bool) -> (DaemonVerb, &'static str) {
    if running {
        (DaemonVerb::Restart, "Restart tracker")
    } else {
        (DaemonVerb::Start, "Start tracker")
    }
}

/// A dim label and a bold value ("Version 0.6.0", "Last capture 00:40").
fn meta_pair(label: &'static str, value: String) -> Element<'static, Message> {
    row![
        text(label).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_3),
        text(value).size(SIZE_META).font(FONT_BOLD).color(TEXT),
    ]
    .spacing(5)
    .align_y(Alignment::Center)
    .into()
}

fn faded(c: Color, enabled: bool) -> Color {
    if enabled {
        c
    } else {
        Color { a: c.a * 0.45, ..c }
    }
}

/// Filled accent button, dimmed while it cannot be pressed.
pub(crate) fn primary_btn(enabled: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |_theme, status| {
        let fill = match status {
            button::Status::Hovered | button::Status::Pressed => Color { a: 0.85, ..ACCENT },
            _ => ACCENT,
        };
        button::Style {
            background: Some(Background::Color(faded(fill, enabled))),
            text_color: faded(TEXT, enabled),
            border: Border {
                color: faded(ACCENT, enabled),
                width: 1.0,
                radius: BUTTON_RADIUS.into(),
            },
            shadow: Shadow::default(),
            snap: false,
        }
    }
}

/// Outlined button in the same shape, dimmed while it cannot be pressed.
fn ghost_btn(enabled: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme_, status| {
        let base = theme::ghost_btn()(theme_, status);
        button::Style {
            background: theme::hover_tint(status).or(base.background),
            text_color: faded(base.text_color, enabled),
            border: Border {
                color: faded(base.border.color, enabled),
                radius: BUTTON_RADIUS.into(),
                ..base.border
            },
            ..base
        }
    }
}

/// "Run setup again" on the left, the legend on the right. The legend
/// words take the room left and wrap, right-aligned; the changed-dot key
/// stays on one line, so the row never runs past the column (spec p9).
fn setup_and_legend(demo: bool) -> Element<'static, Message> {
    use iced::widget::text::{Alignment as TextAlign, Wrapping};
    Row::new()
        .spacing(16)
        .align_y(Alignment::Center)
        .push(link(
            RUN_SETUP_LABEL,
            (!demo).then_some(Message::Setup(crate::setup_guide::SetupMessage::Open)),
        ))
        .push(
            row![
                text(LEGEND_APPLY)
                    .size(SIZE_LABEL)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3)
                    .width(Fill)
                    .align_x(TextAlign::Right),
                changed_dot(),
                text(LEGEND_CHANGED)
                    .size(SIZE_LABEL)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3)
                    .wrapping(Wrapping::None),
            ]
            .spacing(LEGEND_DOT_GAP)
            .align_y(Alignment::Center)
            .width(Fill),
        )
        .width(Fill)
        .into()
}

fn everyday_column(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut col = column![].spacing(CARD_GAP).width(Fill);
    for section in EverydaySection::ALL {
        let body = match section {
            EverydaySection::PlayerSessions => settings_cards::player(app, demo),
            EverydaySection::Companion => settings_cards::companion(app, demo),
            EverydaySection::WebsiteAccount => settings_cards::website_account(app),
            EverydaySection::Updates => settings_cards::updates(app, demo),
        };
        col = col.push(card(section.title(), body));
    }
    col.into()
}

/// ADVANCED: the five rows in one panel with hairlines between them, and
/// an accent bar beside an open row.
fn advanced_column(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let values = advanced_values(app);
    let mut panel = Column::new().spacing(0).width(Fill);
    for (i, section) in AdvancedSection::ALL.into_iter().enumerate() {
        if i > 0 {
            panel = panel.push(hairline());
        }
        let open = app.page_state.is_open(section);
        let mut fold = column![fold_header(section, open, &values)]
            .spacing(0)
            .width(Fill);
        if open {
            fold = fold.push(
                container(advanced_body(section, app, demo))
                    // The header pads CARD_PAD - 3 after the 3 px bar; the
                    // body lines up with its title.
                    .padding(Padding {
                        top: 4.0,
                        bottom: CARD_PAD,
                        left: CARD_PAD - 3.0,
                        right: CARD_PAD,
                    })
                    .width(Fill),
            );
        }
        let bar = container(space().width(3))
            .width(3)
            .height(Fill)
            .style(move |_: &Theme| container::Style {
                background: open.then_some(Background::Color(ACCENT)),
                ..container::Style::default()
            });
        panel = panel.push(row![bar, fold].height(Length::Shrink).width(Fill));
    }
    column![
        text("ADVANCED")
            .size(SIZE_LABEL)
            .font(FONT_BOLD)
            .color(TEXT_3),
        container(panel)
            .width(Fill)
            .clip(true)
            .style(theme::surface_panel),
    ]
    .spacing(8)
    .width(Fill)
    .into()
}

/// What the closed Advanced rows summarise, from what is saved.
fn advanced_values(app: &TrackerApp) -> AdvancedValues {
    let config = &app.saved_config;
    AdvancedValues {
        route: crate::capture_reading::route_label(config.capture_route),
        monitor: monitor_phrase(config.capture_output.as_deref().unwrap_or(""), &app.outputs),
        reader: ReaderChoice::of(config),
        autostart: app.daemon.autostart,
        auto_detect: config.auto_detect.enabled,
        poll_ms: config
            .auto_detect
            .poll_interval(config.reader.is_new())
            .as_millis() as u64,
        cooldown_secs: config.auto_detect.cooldown_secs,
        games: app.games.len(),
        sync_host: config
            .sync
            .as_ref()
            .filter(|_| app.sync_on())
            .map(|s| crate::settings_model::url_host(&s.server_url)),
        debug_images: config.debug_ocr,
        compare_frames: stat_tracker::capture::is_kde(&app.desktop)
            .then(|| config.debug_portal_frames_enabled()),
        extra_reader: app.settings.displayed_reader().file_flag(),
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
        title = title.push(tag(Tag::AfterRestart));
    }
    let chevron = if open {
        crate::icons::handle(crate::icons::Icon::FoldOpen)
    } else {
        crate::icons::handle(crate::icons::Icon::FoldClosed)
    };
    button(
        row![
            column![
                title,
                text(section.summary(values))
                    .size(SIZE_LABEL)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            ]
            .spacing(4)
            .width(Fill),
            image(chevron).width(14).height(14),
        ]
        .spacing(16)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([14.0, CARD_PAD - 3.0]))
    .width(Fill)
    .style(theme::quiet_btn(0.0))
    .on_press(Message::SettingsPage(Msg::ToggleAdvanced(section)))
    .into()
}

fn advanced_body<'a>(
    section: AdvancedSection,
    app: &'a TrackerApp,
    demo: bool,
) -> Element<'a, Message> {
    match section {
        AdvancedSection::CaptureReading => crate::capture_reading::body(app, demo),
        AdvancedSection::TrackerService => settings_cards::tracker_service(app, demo),
        AdvancedSection::StoredData => settings_cards::stored_data(app, demo),
        AdvancedSection::WebsiteConnection => settings_cards::website_connection(app, demo),
        AdvancedSection::Troubleshooting => settings_cards::troubleshooting(app, demo),
    }
}

fn card<'a>(title: &'static str, body: Element<'a, Message>) -> Element<'a, Message> {
    container(column![text(title).size(15).font(FONT_BOLD).color(TEXT), body,].spacing(14))
        .padding(Padding::from([18.0, CARD_PAD]))
        .width(Fill)
        .style(theme::surface_panel)
        .into()
}

fn meta(copy: impl Into<String>) -> Element<'static, Message> {
    text(copy.into())
        .size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .color(TEXT_3)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_model::{SINGLE_COLUMN_MAX, TWO_COLUMN_MIN};
    use iced::advanced::layout::{Limits, Node};
    use iced::advanced::widget::Tree;

    fn app() -> (tempfile::TempDir, TrackerApp) {
        crate::fixtures::test_app()
    }

    fn headless() -> iced::Renderer {
        use iced::advanced::renderer::Headless;
        let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
            FONT_MEDIUM,
            iced::Pixels(SIZE_BODY),
            Some("tiny-skia"),
        ))
        .expect("the tiny-skia renderer needs no display");
        for bytes in [
            theme::FONT_BYTES_MEDIUM,
            theme::FONT_BYTES_SEMIBOLD,
            theme::FONT_BYTES_BOLD,
            theme::FONT_BYTES_EXTRABOLD,
        ] {
            iced::advanced::graphics::text::font_system()
                .write()
                .expect("font system lock")
                .load_font(std::borrow::Cow::Borrowed(bytes));
        }
        renderer
    }

    /// Lay the page out at `width` x 4000 px.
    fn lay_out(mut element: Element<'_, Message>, width: f32) -> Node {
        let renderer = headless();
        let mut tree = Tree::new(&element);
        element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &Limits::new(iced::Size::ZERO, iced::Size::new(width, 4000.0)),
        )
    }

    /// Opening every ADVANCED row makes the page taller at every width.
    #[test]
    fn every_open_row_adds_its_body_at_every_width() {
        let (_dir, mut app) = app();
        let widths = [640.0, 976.0, TWO_COLUMN_MIN + 1.0, 3400.0];
        let height = |app: &TrackerApp, width: f32| {
            lay_out(view(app, width), width).children()[0]
                .bounds()
                .height
        };
        let closed: Vec<f32> = widths.iter().map(|&w| height(&app, w)).collect();
        for section in AdvancedSection::ALL {
            let _ = app.update(Message::SettingsPage(Msg::ToggleAdvanced(section)));
        }
        for (width, closed) in widths.into_iter().zip(closed) {
            assert!(height(&app, width) > closed, "{width}: {closed}");
        }
    }

    /// The live page (not demo): live controls and no demo note, so the
    /// strip comes first, one child less than in demo.
    #[test]
    fn the_live_page_draws_with_every_row_open() {
        let (_dir, mut app) = app();
        let demo = lay_out(view(&app, 976.0), 976.0);
        for section in AdvancedSection::ALL {
            let _ = app.update(Message::SettingsPage(Msg::ToggleAdvanced(section)));
        }
        app.fixture = None;
        for width in [640.0, 976.0, TWO_COLUMN_MIN + 1.0, 3400.0] {
            let _ = view(&app, width);
        }
        for section in AdvancedSection::ALL {
            let _ = app.update(Message::SettingsPage(Msg::ToggleAdvanced(section)));
        }
        let live = lay_out(view(&app, 976.0), 976.0);
        assert_eq!(
            live.children()[0].children().len() + 1,
            demo.children()[0].children().len()
        );
    }

    /// Render fix 2 (spec p9): at 1024 the one column starts at the
    /// gutter and stops at its cap; it is not centred.
    #[test]
    fn the_one_column_page_is_left_aligned_and_capped() {
        let (_dir, app) = app();
        let node = lay_out(view(&app, 976.0), 976.0);
        let page = &node.children()[0];
        assert!(page.bounds().x.abs() < 0.5, "{:?}", page.bounds());
        assert!((page.bounds().width - SINGLE_COLUMN_MAX).abs() < 0.5);
        let wide = lay_out(view(&app, 3400.0), 3400.0);
        let page = &wide.children()[0];
        assert!(page.bounds().x.abs() < 0.5);
        assert!((page.bounds().width - crate::settings_model::PAGE_MAX).abs() < 0.5);
    }

    /// M9: with a change waiting, the one-column strip puts its actions on
    /// a second line and nothing runs past the column.
    #[test]
    fn the_one_column_strip_fits_with_a_change_waiting() {
        let (_dir, mut app) = app();
        app.saved_config.auto_detect.cooldown_secs += 1;
        assert_eq!(app.changes_waiting(), 1);
        let node = lay_out(view(&app, 976.0), 976.0);
        let page = &node.children()[0];
        // Demo note, then the strip.
        let strip = &page.children()[1];
        let inner = &strip.children()[0];
        let lines = inner.children();
        assert_eq!(lines.len(), 2, "status, then the actions");
        let actions = &lines[1].children()[0];
        let right_edge = lines[1].bounds().x + actions.bounds().x + actions.bounds().width;
        assert!(right_edge <= inner.bounds().width + 0.5, "{right_edge}");
        assert!(actions.bounds().x > 0.0, "end-aligned");
    }

    /// Render fix (spec p9): at 1024 the legend wraps its words inside
    /// the column and keeps the changed-dot key on one line.
    #[test]
    fn the_one_column_legend_stays_inside_the_column() {
        let (_dir, app) = app();
        let node = lay_out(view(&app, 976.0), 976.0);
        let page = &node.children()[0];
        // Demo note, the strip, then "Run setup again" and the legend.
        let legend = &page.children()[2];
        let words = &legend.children()[1];
        let right = legend.bounds().x + words.bounds().x + words.bounds().width;
        assert!(right <= page.bounds().width + 0.5, "{right}");
        // The changed-dot key is one line, not squeezed into a stack.
        let key = &words.children()[2];
        assert!(key.bounds().height < 20.0, "{:?}", key.bounds());
    }

    #[test]
    fn a_stopped_tracker_gets_start_not_restart() {
        assert_eq!(strip_action(true), (DaemonVerb::Restart, "Restart tracker"));
        assert_eq!(strip_action(false), (DaemonVerb::Start, "Start tracker"));
    }
}
