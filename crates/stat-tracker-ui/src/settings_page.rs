//! 0.6 Settings page (redesign variant A, top tabs; spec p6, p7, p9).
//!
//! Under the "Settings" heading: "Run setup again" and the legend, then
//! the everyday cards on the left and the ADVANCED rows in one panel on
//! the right. Below [`crate::settings_model::TWO_COLUMN_MIN`] the page is
//! one column of at most [`crate::settings_model::SINGLE_COLUMN_MAX`],
//! centred between the page gutters. The tracker's state and its Start,
//! Restart and Stop are on the status pill; the status strip that held
//! them went with #776.
//!
//! There is no Save for controls: every switch, chip and dropdown applies
//! as it is made ([`crate::settings_update`]). Typed text and changes
//! waiting for a tracker restart show on the pinned bar under the page
//! ([`crate::settings_bar`]). The old card layout with its Save stays in
//! [`crate::settings::view`] for `SCUFFED_TRACKER_LAYOUT=classic`; both
//! edit the same form and write through [`crate::settings_save`]. The
//! window chrome scrolls this page like every other tab.

use iced::widget::{Column, Row, button, column, container, image, row, space, text};
use iced::{Alignment, Background, Border, Color, Element, Fill, Length, Padding, Shadow, Theme};

use crate::app::{Message, TrackerApp};
use crate::form_widgets::DEMO_NOTE;
use crate::settings_cards;
use crate::settings_model::{
    AdvancedSection, AdvancedValues, COLUMN_GAP, EverydaySection, ReaderChoice, columns_for,
    monitor_phrase, page_width,
};
use crate::settings_rows::{Tag, changed_dot, hairline, link, tag};
use crate::settings_update::Msg;
use crate::theme::{self, ACCENT, FONT_BOLD, FONT_MEDIUM, SIZE_BODY, SIZE_LABEL, TEXT, TEXT_3};
use crate::{update, widgets};

/// Padding inside a card (spec about 18 to 20).
const CARD_PAD: f32 = 20.0;
/// Space between cards (spec 16).
const CARD_GAP: f32 = 16.0;
/// Button corner (spec about 10).
const BUTTON_RADIUS: f32 = 10.0;
/// Space between the legend's dot and its words.
const LEGEND_DOT_GAP: f32 = 6.0;

/// The legend under the page title (spec p6). The After restart tags now
/// show only where a change waits (#776), so the legend no longer points
/// at them (D9 on #776, accepted by FrozenTear 27 Nov 2026).
const LEGEND_APPLY: &str = "Changes apply as you make them. Some only take effect after a tracker restart; the bar at the bottom says when.";
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
    page = page.push(setup_and_legend(demo));

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

    // Capped so the controls stay near their labels. Two columns sit on
    // the gutter; one column is centred in the space it leaves (Robert's
    // call for narrow windows), so the gutter stays the least margin.
    let page = container(page.max_width(page_width(content_width))).width(Fill);
    if two_columns {
        page.align_left(Fill).into()
    } else {
        page.center_x(Fill).into()
    }
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
pub(crate) fn ghost_btn(enabled: bool) -> impl Fn(&Theme, button::Status) -> button::Style {
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
        let waiting = crate::settings_bar::section_waiting(app, section);
        let mut fold = column![fold_header(section, open, &values, waiting)]
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
        panel = panel.push(
            container(row![bar, fold].height(Length::Shrink).width(Fill))
                .id(crate::focus_status::section_row_id(section))
                .width(Fill),
        );
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
        store_bytes: app.page_state.opened.store_bytes,
        sync_host: config
            .sync
            .as_ref()
            .filter(|_| app.sync_on())
            .map(|s| crate::settings_model::url_host(&s.server_url)),
        debug_images: config.debug_ocr,
        pregame_screens: config.debug_save_pregame_screens,
        compare_frames: stat_tracker::capture::is_kde(&app.desktop)
            .then(|| config.debug_portal_frames_enabled()),
        extra_reader: app.settings.displayed_reader().file_flag(),
        report_pictures: config.share_report_pictures,
    }
}

/// A fold's header. `waiting`: a saved change in it waits for a tracker
/// restart, so it carries the After restart tag (#776).
fn fold_header(
    section: AdvancedSection,
    open: bool,
    values: &AdvancedValues,
    waiting: bool,
) -> Element<'static, Message> {
    let mut title = Row::new().spacing(8).align_y(Alignment::Center).push(
        text(section.title())
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(TEXT),
    );
    if waiting {
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_model::{SINGLE_COLUMN_MAX, TWO_COLUMN_MIN};
    use iced::Rectangle;
    use iced::advanced::Layout;
    use iced::advanced::layout::{Limits, Node};
    use iced::advanced::widget::operation::Operation;
    use iced::advanced::widget::{Id, Tree};

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

    /// Render fix 2 (spec p9): at 1024 the one column stops at its cap,
    /// and the two-column page starts at the gutter.
    #[test]
    fn the_one_column_page_is_capped_and_two_columns_left_aligned() {
        let (_dir, app) = app();
        let node = lay_out(view(&app, 976.0), 976.0);
        let page = &node.children()[0];
        assert!((page.bounds().width - SINGLE_COLUMN_MAX).abs() < 0.5);
        let wide = lay_out(view(&app, 3400.0), 3400.0);
        let page = &wide.children()[0];
        assert!(page.bounds().x.abs() < 0.5);
        assert!((page.bounds().width - crate::settings_model::PAGE_MAX).abs() < 0.5);
    }

    /// Robert's call: on a narrow window (1024, so 976 inside the 24 px
    /// gutters) the one column is centred, with equal space each side.
    /// Narrower than the cap it fills, so the gutter is the least margin.
    #[test]
    fn the_one_column_page_is_centred_at_1024() {
        let (_dir, app) = app();
        let window = 1024.0;
        let gutter = theme::page_gutter(window);
        let content = window - 2.0 * gutter;
        let node = lay_out(view(&app, content), content);
        let page = node.children()[0].bounds();
        let left = gutter + page.x;
        let right = window - (gutter + page.x + page.width);
        assert!((left - right).abs() <= 1.0, "{left} vs {right}");
        assert!(left > gutter, "{page:?}");
        let narrow = lay_out(view(&app, 700.0), 700.0);
        let page = narrow.children()[0].bounds();
        assert!(page.x.abs() < 0.5 && (page.width - 700.0).abs() < 0.5);
    }

    /// Render fix (spec p9): at 1024 the legend wraps its words inside
    /// the column and keeps the changed-dot key on one line.
    #[test]
    fn the_one_column_legend_stays_inside_the_column() {
        let (_dir, app) = app();
        let node = lay_out(view(&app, 976.0), 976.0);
        let page = &node.children()[0];
        // Demo note, then "Run setup again" and the legend.
        let legend = &page.children()[1];
        let words = &legend.children()[1];
        let right = legend.bounds().x + words.bounds().x + words.bounds().width;
        assert!(right <= page.bounds().width + 0.5, "{right}");
        // The changed-dot key is one line, not squeezed into a stack.
        let key = &words.children()[2];
        assert!(key.bounds().height < 20.0, "{:?}", key.bounds());
    }

    struct Collect {
        ids: Vec<(Id, Rectangle)>,
        texts: Vec<(String, Rectangle)>,
    }

    impl Operation<()> for Collect {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }

        fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
            if let Some(id) = id {
                self.ids.push((id.clone(), bounds));
            }
        }

        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            self.texts.push((text.to_string(), bounds));
        }
    }

    fn operate_page(app: &TrackerApp, width: f32) -> Collect {
        let renderer = headless();
        let mut element = view(app, width);
        let mut tree = Tree::new(&element);
        let node = element.as_widget_mut().layout(
            &mut tree,
            &renderer,
            &Limits::new(iced::Size::ZERO, iced::Size::new(width, 4000.0)),
        );
        let mut collect = Collect {
            ids: Vec::new(),
            texts: Vec::new(),
        };
        element
            .as_widget_mut()
            .operate(&mut tree, Layout::new(&node), &renderer, &mut collect);
        collect
    }

    fn bounds_contain(outer: Rectangle, inner: Rectangle) -> bool {
        inner.x >= outer.x
            && inner.y >= outer.y
            && inner.x + inner.width <= outer.x + outer.width + 0.5
            && inner.y + inner.height <= outer.y + outer.height + 0.5
    }

    /// The moved switch sits under READERS in Capture and reading, below
    /// the focus gate (the last CAPTURE row), and Troubleshooting no longer
    /// has it. With the env override the lock line sits inside that row.
    #[test]
    fn the_bans_picks_row_is_drawn_in_capture_and_reading() {
        use crate::bans_picks_setting::ROW_ID;
        use crate::settings_env::{BANS_PICKS_ENV, OutsideEnv};

        let width = 976.0;
        let gate_id = Id::new(crate::focus_status::ROW_ID);
        let row_id = Id::new(ROW_ID);

        let (_dir, mut page) = app();
        page.fixture = None;
        page.outside_env = Some(OutsideEnv {
            vars: vec![(BANS_PICKS_ENV, "off".to_string())],
            ..OutsideEnv::default()
        });
        page.page_state
            .show_section(AdvancedSection::CaptureReading);

        let found = operate_page(&page, width);
        let gate = found
            .ids
            .iter()
            .find(|(id, _)| *id == gate_id)
            .map(|(_, b)| *b)
            .expect("the focus gate is on the open Capture and reading page");
        let row = found
            .ids
            .iter()
            .find(|(id, _)| *id == row_id)
            .map(|(_, b)| *b)
            .expect("Read bans and picks is on Capture and reading");
        assert!(
            gate.y < row.y,
            "focus gate at y={} should sit above the bans row at y={}",
            gate.y,
            row.y
        );
        let lock = format!("Set outside the app: {BANS_PICKS_ENV}");
        let lock_bounds = found
            .texts
            .iter()
            .find(|(text, _)| text == &lock)
            .map(|(_, b)| *b)
            .expect("the lock line is drawn");
        assert!(
            bounds_contain(row, lock_bounds),
            "lock line {lock_bounds:?} should sit inside the row {row:?}"
        );

        let (_dir_ts, mut troubleshooting) = app();
        troubleshooting
            .page_state
            .show_section(AdvancedSection::Troubleshooting);
        let ts = operate_page(&troubleshooting, width);
        assert!(
            ts.ids.iter().all(|(id, _)| *id != row_id),
            "Troubleshooting must not draw the Read bans and picks row"
        );
    }

    /// Every reader switch sits together under READERS in Capture and
    /// reading, below every CAPTURE row: Reader first, then names, bans and
    /// picks, rank and the extra number reader, then the classic OCR rows
    /// (Reading model, OCR workers) and Report pictures. Website account
    /// and Troubleshooting no longer draw the rank and extra number reader
    /// rows.
    #[test]
    fn the_reader_rows_sit_together_under_readers() {
        use crate::settings::SHADOW_READER_LABEL;
        use crate::settings_cards::BANS_PICKS_LABEL;
        use crate::settings_keys::RANK_LABEL;

        let width = 976.0;
        let (_dir, mut page) = app();
        page.fixture = None;
        page.outside_env = Some(crate::settings_env::OutsideEnv::default());
        // A running tracker: the Reading model row is drawn, so its place
        // below the reader switches is checked, not skipped.
        page.ocr_switch.daemon_off = false;
        assert!(crate::settings::ocr_card_blocks(page.ocr_switch.daemon_off).reading_model);
        assert!(crate::settings_model::row_shown("share_report_pictures"));
        page.page_state
            .show_section(AdvancedSection::CaptureReading);
        let found = operate_page(&page, width);
        let y_of = |label: &str| {
            found
                .texts
                .iter()
                .find(|(text, _)| text == label)
                .map(|(_, b)| b.y)
                .unwrap_or_else(|| panic!("{label} is drawn"))
        };
        let heading = y_of("READERS");
        let gate = found
            .ids
            .iter()
            .find(|(id, _)| *id == Id::new(crate::focus_status::ROW_ID))
            .map(|(_, b)| b.y)
            .expect("the focus gate, the last CAPTURE row");
        assert!(gate < heading, "CAPTURE ends before READERS");
        // Reader comes first, right under the heading: no row sits between.
        let reader = y_of("Reader");
        assert!(heading < reader, "Reader at {reader} follows READERS");
        let between: Vec<_> = found
            .texts
            .iter()
            .filter(|(text, b)| b.y > heading && b.y < reader && !text.is_empty())
            .collect();
        assert!(between.is_empty(), "no row before Reader: {between:?}");
        let mut last = reader;
        for label in [
            "Name reader",
            BANS_PICKS_LABEL,
            RANK_LABEL,
            SHADOW_READER_LABEL,
        ] {
            let y = y_of(label);
            assert!(last < y, "{label} at {y} follows {last}");
            last = y;
        }
        // The classic OCR rows and Report pictures come after every reader
        // switch, in this order.
        for label in ["Reading model", "OCR workers", "Report pictures"] {
            let y = y_of(label);
            assert!(last < y, "{label} at {y} follows {last}");
            last = y;
        }
        // The bans and picks row keeps its id, so its deep link and
        // outline still find it.
        let row_id = Id::new(crate::bans_picks_setting::ROW_ID);
        assert!(found.ids.iter().any(|(id, _)| *id == row_id));

        let (_dir_closed, mut closed) = app();
        closed.fixture = None;
        closed.outside_env = Some(crate::settings_env::OutsideEnv::default());
        let everyday = operate_page(&closed, width);
        assert!(
            everyday.texts.iter().all(|(text, _)| text != RANK_LABEL),
            "Website account no longer draws the rank row"
        );
        closed
            .page_state
            .show_section(AdvancedSection::Troubleshooting);
        let ts = operate_page(&closed, width);
        assert!(
            ts.texts.iter().all(|(text, _)| text != SHADOW_READER_LABEL),
            "Troubleshooting no longer draws the extra number reader"
        );
    }

    /// Each rank variable that is set draws its "Set outside the app" lock
    /// line inside the Read my rank row, with the service note once; with
    /// neither set the row draws none.
    #[test]
    fn the_rank_row_draws_a_lock_line_per_variable_set() {
        use crate::capture_reading::SERVICE_MAY_DIFFER;
        use crate::settings::SHADOW_READER_LABEL;
        use crate::settings_env::{OutsideEnv, QUEUE_TITLE_ENV, RANK_ENV};
        use crate::settings_keys::RANK_LABEL;

        let rank_lock = format!("Set outside the app: {RANK_ENV}");
        let queue_lock = format!("Set outside the app: {QUEUE_TITLE_ENV}");
        let drawn = |vars: Vec<(&'static str, String)>| {
            let (_dir, mut page) = app();
            page.fixture = None;
            page.outside_env = Some(OutsideEnv {
                vars,
                ..OutsideEnv::default()
            });
            page.page_state
                .show_section(AdvancedSection::CaptureReading);
            let found = operate_page(&page, 976.0);
            let y_of = |label: &str| {
                found
                    .texts
                    .iter()
                    .find(|(text, _)| text == label)
                    .map(|(_, b)| b.y)
                    .unwrap_or_else(|| panic!("{label} is drawn"))
            };
            let (top, next) = (y_of(RANK_LABEL), y_of(SHADOW_READER_LABEL));
            // the rank row's lines: under its title, above the next row
            found
                .texts
                .into_iter()
                .filter(|(_, b)| b.y > top && b.y < next)
                .map(|(text, _)| text)
                .collect::<Vec<_>>()
        };
        let both = drawn(vec![
            (RANK_ENV, "on".to_string()),
            (QUEUE_TITLE_ENV, "off".to_string()),
        ]);
        let at = |text: &str| both.iter().position(|t| t == text);
        let (rank, queue) = (at(&rank_lock), at(&queue_lock));
        assert!(rank.is_some() && queue.is_some(), "{both:?}");
        assert!(rank < queue, "the rank lock line comes first: {both:?}");
        let note = both.iter().filter(|t| *t == SERVICE_MAY_DIFFER).count();
        assert_eq!(note, 1, "{both:?}");
        let queue_only = drawn(vec![(QUEUE_TITLE_ENV, "on".to_string())]);
        assert!(queue_only.contains(&queue_lock), "{queue_only:?}");
        assert!(!queue_only.contains(&rank_lock), "{queue_only:?}");
        // blank is unset: no lock line, no service note
        let none = drawn(vec![(RANK_ENV, " ".to_string())]);
        for text in [&rank_lock, &queue_lock, &SERVICE_MAY_DIFFER.to_string()] {
            assert!(!none.contains(text), "{text} in {none:?}");
        }
    }

    /// Signed in, the Website account card draws the training crop section:
    /// the consent per kind, where to change it, the tracker's warning and
    /// the delete button.
    #[test]
    fn the_training_crop_section_is_drawn_under_website_account() {
        use crate::training_card::TrainingMsg;
        use scuffed_types::api::training_crops::TrainingCropConsent;

        let (_dir, mut page) = app();
        page.fixture = None;
        page.saved_config.sync = Some(stat_tracker::config::SyncConfig {
            server_url: "https://crew.example".into(),
            token: "tok".into(),
        });
        let _ = crate::settings_open::on_open(&mut page);
        let _ = page.update(Message::SettingsPage(Msg::Open(
            crate::settings_open::OpenMsg::Training(TrainingMsg::ConsentRead {
                key: ("https://crew.example".into(), "tok".into()),
                value: Ok(TrainingCropConsent {
                    map_name: true,
                    ..TrainingCropConsent::default()
                }),
            }),
        )));
        page.daemon.training_warning = Some("Training crops: could not send them.".into());
        let texts: Vec<String> = operate_page(&page, 976.0)
            .texts
            .into_iter()
            .map(|(text, _)| text)
            .collect();
        for want in [
            crate::training_card::HEADING,
            "Hero icons: off",
            "Stat digits: off",
            "Map names: on",
            "Training crops: could not send them.",
            "Delete my training crops",
        ] {
            assert!(texts.iter().any(|t| t == want), "{want} in {texts:?}");
        }
        assert!(
            texts
                .iter()
                .any(|t| t.contains("https://crew.example/profile/edit"))
        );
    }
}
