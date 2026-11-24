//! Dialog scrollbars. iced 0.14 draws a vertical scrollbar
//! [`SCROLLBAR_WIDTH`] px wide over the right edge of a scrollable's
//! content, so a dialog body that fills its scrollable runs its text under
//! the bar. Each dialog is laid out at 1280x860, 1000x768 and 720x480 with
//! the headless renderer. The walk only sees widgets that call
//! `operation.text` (plain text and checkbox labels). iced 0.14's
//! `rich_text` has no `operate`, so What's new Details markdown is not
//! checked, and text input values are not checked either. Every covered
//! text block inside a scrollable must end left of where the bar is drawn.

use std::collections::HashSet;

use iced::advanced::Layout;
use iced::advanced::Shell;
use iced::advanced::clipboard;
use iced::advanced::layout::Limits;
use iced::advanced::widget::operation::{Operation, Scrollable};
use iced::advanced::widget::{Id, Tree};
use iced::keyboard::key::Named;
use iced::keyboard::{self, Key};
use iced::mouse::{Button, Cursor, Event as Mouse};
use iced::{Element, Event, Point, Rectangle, Size, Vector};

use crate::app::Message;
use crate::layout::SCROLLBAR_WIDTH;
use crate::settings_open::OpenMsg;
use crate::settings_update::Msg;
use crate::setup_guide::SetupMessage;
use crate::theme::GRID_GAP;

/// The window sizes the dialogs are checked at, including the app minimum.
const WINDOWS: [(f32, f32); 3] = [(1280.0, 860.0), (1000.0, 768.0), (720.0, 480.0)];

fn headless() -> iced::Renderer {
    crate::games_bans_view::tests::headless()
}

/// One text block inside a scrollable, and that scrollable.
#[derive(Debug)]
struct Placed {
    text: String,
    text_right: f32,
    scroll: Rectangle,
    overflows: bool,
}

impl Placed {
    /// Left edge of the vertical bar: iced draws it in the last
    /// [`SCROLLBAR_WIDTH`] px of the scrollable.
    fn bar_left(&self) -> f32 {
        self.scroll.x + self.scroll.width - SCROLLBAR_WIDTH
    }
}

/// Walks the widget tree and keeps every text block with the innermost
/// scrollable it is drawn in. `depth` follows `traverse`, so a scrollable
/// is dropped once its own children have been walked.
#[derive(Default)]
struct Collect {
    depth: usize,
    scrolls: Vec<(usize, Rectangle, bool)>,
    placed: Vec<Placed>,
}

impl Operation<()> for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        self.depth += 1;
        operate(self);
        self.depth -= 1;
        let depth = self.depth;
        self.scrolls.retain(|(d, _, _)| *d < depth);
    }

    fn container(&mut self, _id: Option<&Id>, _bounds: Rectangle) {}

    fn scrollable(
        &mut self,
        _id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        _translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        let overflows = content_bounds.height > bounds.height + 0.5;
        self.scrolls.push((self.depth, bounds, overflows));
    }

    fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
        if let Some(&(_, scroll, overflows)) = self.scrolls.last() {
            self.placed.push(Placed {
                text: text.to_string(),
                text_right: bounds.x + bounds.width,
                scroll,
                overflows,
            });
        }
    }
}

/// Lays `element` out as the window-sized layer it is in the app and
/// returns every text block drawn inside a scrollable.
fn placed(mut element: Element<'_, Message>, window: (f32, f32)) -> Vec<Placed> {
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(Size::ZERO, Size::new(window.0, window.1)),
    );
    let mut collect = Collect::default();
    element
        .as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut collect);
    collect.placed
}

/// Every covered text block of `name` ends left of the bar, at each window
/// size, and (when `min_gap` is set) leaves at least that much space.
fn assert_clear<'a>(name: &str, min_gap: f32, build: impl Fn() -> Element<'a, Message>) {
    for window in WINDOWS {
        let placed = placed(build(), window);
        assert!(
            !placed.is_empty(),
            "{name} {window:?}: no text in a scrollable"
        );
        let tightest = placed
            .iter()
            .map(|p| p.bar_left() - p.text_right)
            .fold(f32::INFINITY, f32::min);
        let overflows = placed.iter().any(|p| p.overflows);
        eprintln!(
            "{name} {window:?}: {} texts, scroll {:?}, overflows {overflows}, \
             tightest gap to bar {tightest:.1}",
            placed.len(),
            placed[0].scroll,
        );
        for p in &placed {
            let gap = p.bar_left() - p.text_right;
            assert!(
                gap > 0.0,
                "{name} {window:?}: text ends at {:.1}, bar starts at {:.1} (gap {gap:.1}): {:?}",
                p.text_right,
                p.bar_left(),
                p.text
            );
            assert!(
                gap >= min_gap,
                "{name} {window:?}: gap is {gap:.1}, need at least {min_gap}: {:?}",
                p.text
            );
        }
    }
}

#[test]
fn what_leaves_your_pc_keeps_its_text_clear_of_the_bar() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(true))));
    assert_clear("what leaves", GRID_GAP, || {
        crate::what_leaves::layer(&app).expect("open").content
    });
}

#[test]
fn whats_new_keeps_its_text_clear_of_the_bar() {
    let sections = crate::notes::render(crate::notes::all_notes(crate::notes::BUNDLED_CHANGELOG));
    let closed = HashSet::new();
    let open: HashSet<String> = sections.iter().map(|s| s.version.clone()).collect();
    for (show_older, open_details) in [(false, &closed), (true, &open)] {
        assert_clear("what's new", GRID_GAP, || {
            crate::notes::dialog(
                "Release notes",
                &sections,
                crate::notes::NotesUi {
                    surface: crate::notes::NotesSurface::Dialog,
                    installed: Some("0.6.0"),
                    offered: None,
                    open_details,
                    show_older,
                },
            )
        });
    }
}

#[test]
fn report_a_problem_keeps_its_text_clear_of_the_bar() {
    use crate::report::{ReportDialog, ReportUi};
    use stat_tracker::report::{LogOnlyCategory, ReportZip};
    use std::sync::Arc;
    for dialog in [
        ReportDialog::new(None, true),
        ReportDialog::for_game(None, true),
    ] {
        let ui = ReportUi::Dialog(Box::new(dialog));
        assert_clear("report", GRID_GAP, || crate::report::view(&ui));
    }
    let mut crash = ReportDialog::new(Some(LogOnlyCategory::Crash), true);
    let long = "word ".repeat(80) + "end";
    crash.built = Some(Ok(Arc::new(ReportZip {
        bytes: vec![1, 2, 3],
        manifest_json: r#"{"game":null,"reason":{"category":"crash"}}"#.into(),
        files: Vec::new(),
        log_text: format!("{long}\n{long}\nmore than one line to test the wrap"),
        images: Vec::new(),
        left_out: Vec::new(),
    })));
    crash.show_log = true;
    let ui = ReportUi::Dialog(Box::new(crash));
    assert_clear("report preview and log", GRID_GAP, || {
        crate::report::view(&ui)
    });
}

#[test]
fn uninstall_keeps_its_text_clear_of_the_bar() {
    use crate::uninstall::UninstallDialog;
    let home = std::path::PathBuf::from(
        "/home/qa a b c d e f g h i j k l m n o p q r s t u v w x y z end",
    );
    let dialog = UninstallDialog::Confirm {
        prefix: home.join(".local"),
        data_dir: home.join(".local/share/scuffed-stat-tracker with spaces in the data dir path"),
        appimage: Some(home.join("Applications/Scuffed Stat Tracker x86_64.AppImage")),
        home,
        delete_data: true,
        fallback: true,
    };
    assert_clear("uninstall", GRID_GAP, || {
        crate::uninstall::view(&dialog, false)
    });
}

/// Each step the guide reaches with Skip this step, from the first.
#[test]
fn the_setup_guide_keeps_its_text_clear_of_the_bar() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::Setup(SetupMessage::Open));
    assert_clear("setup guide step 0", GRID_GAP, || {
        crate::setup_guide::view(&app.setup)
    });
    app.setup.capture_ready(Ok(crate::setup_guide::MemoryFrame {
        width: 1920,
        height: 1080,
        preview_width: 2,
        preview_height: 1,
        rgba: vec![0, 0, 0, 255, 0, 0, 0, 255],
    }));
    assert_clear("setup guide capture picture", GRID_GAP, || {
        crate::setup_guide::view(&app.setup)
    });
    for step in 1..4 {
        let _ = app.update(Message::Setup(SetupMessage::SkipStep));
        assert_clear(&format!("setup guide step {step}"), GRID_GAP, || {
            crate::setup_guide::view(&app.setup)
        });
    }
    // SkipStep from capture (with a picture) advances one step; after the
    // loop we have walked later steps. Open the token field on a fresh
    // guide at the Sync step.
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::Setup(SetupMessage::Open));
    let _ = app.update(Message::Setup(SetupMessage::SkipStep)); // Overwatch
    let _ = app.update(Message::Setup(SetupMessage::SkipStep)); // Sync
    app.setup.toggle_paste();
    app.setup.set_paste_token("a-token-from-the-website".into());
    assert_clear("setup guide token open", GRID_GAP, || {
        crate::setup_guide::view(&app.setup)
    });
}

/// Close stays inside the window at every size the dialog is checked at.
#[test]
fn what_leaves_keeps_its_close_button_in_view() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(true))));
    for window in WINDOWS {
        let close_box = close_bounds(
            crate::what_leaves::layer(&app).expect("open").content,
            window,
        )
        .unwrap_or_else(|| panic!("Close is laid out at {window:?}"));
        assert!(
            close_box.y >= 0.0
                && close_box.x >= 0.0
                && close_box.y + close_box.height <= window.1 + 0.5
                && close_box.x + close_box.width <= window.0 + 0.5,
            "Close at {close_box:?} is outside {window:?}"
        );
    }
}

/// The Close widget's bounds, if a "Close" text block is laid out.
fn close_bounds(mut element: Element<'_, Message>, window: (f32, f32)) -> Option<Rectangle> {
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(Size::ZERO, Size::new(window.0, window.1)),
    );
    struct FindClose(Option<Rectangle>);
    impl Operation<()> for FindClose {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }
        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            if text == "Close" {
                self.0 = Some(bounds);
            }
        }
    }
    let mut find = FindClose(None);
    element
        .as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut find);
    find.0
}

/// The What leaves layer as it sits in the window: a top dialog_focus
/// layer with the Esc message from [`Dialog::new`].
fn what_leaves_top(app: &crate::app::TrackerApp) -> Element<'_, Message> {
    let dialog = crate::what_leaves::layer(app).expect("open");
    crate::dialog_focus::layer(
        dialog.content,
        crate::dialog_focus::Role::Top,
        dialog.on_escape,
    )
}

/// Messages a left click at `point` publishes on the laid-out layer.
fn press_at(mut element: Element<'_, Message>, window: (f32, f32), point: Point) -> Vec<Message> {
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let size = Size::new(window.0, window.1);
    let node = element
        .as_widget_mut()
        .layout(&mut tree, &renderer, &Limits::new(Size::ZERO, size));
    let viewport = Rectangle::new(Point::ORIGIN, size);
    let mut messages = Vec::new();
    let mut shell = Shell::new(&mut messages);
    element.as_widget_mut().update(
        &mut tree,
        &Event::Mouse(Mouse::ButtonPressed(Button::Left)),
        Layout::new(&node),
        Cursor::Available(point),
        &renderer,
        &mut clipboard::Null,
        &mut shell,
        &viewport,
    );
    messages
}

fn esc_event() -> Event {
    Event::Keyboard(keyboard::Event::KeyPressed {
        key: Key::Named(Named::Escape),
        modified_key: Key::Named(Named::Escape),
        physical_key: keyboard::key::Physical::Unidentified(
            keyboard::key::NativeCode::Unidentified,
        ),
        location: keyboard::Location::Standard,
        modifiers: keyboard::Modifiers::empty(),
        text: None,
        repeat: false,
    })
}

/// Messages Esc publishes on the laid-out What leaves dialog layer.
fn press_esc(mut element: Element<'_, Message>, window: (f32, f32)) -> Vec<Message> {
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let size = Size::new(window.0, window.1);
    let node = element
        .as_widget_mut()
        .layout(&mut tree, &renderer, &Limits::new(Size::ZERO, size));
    let viewport = Rectangle::new(Point::ORIGIN, size);
    let mut messages = Vec::new();
    let mut shell = Shell::new(&mut messages);
    element.as_widget_mut().update(
        &mut tree,
        &esc_event(),
        Layout::new(&node),
        Cursor::Unavailable,
        &renderer,
        &mut clipboard::Null,
        &mut shell,
        &viewport,
    );
    messages
}

/// The padded What leaves card: the container whose top-left is the title
/// inset by the 24 px panel padding.
fn card_bounds(mut element: Element<'_, Message>, window: (f32, f32)) -> Rectangle {
    const PANEL_PAD: f32 = 24.0;
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(Size::ZERO, Size::new(window.0, window.1)),
    );
    struct FindCard {
        title: Option<Rectangle>,
        containers: Vec<Rectangle>,
    }
    impl Operation<()> for FindCard {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }
        fn container(&mut self, _id: Option<&Id>, bounds: Rectangle) {
            self.containers.push(bounds);
        }
        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            if text == "What leaves your PC" {
                self.title = Some(bounds);
            }
        }
    }
    let mut find = FindCard {
        title: None,
        containers: Vec::new(),
    };
    element
        .as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut find);
    let title = find
        .title
        .unwrap_or_else(|| panic!("title is laid out at {window:?}"));
    find.containers
        .into_iter()
        .find(|b| {
            (b.x - (title.x - PANEL_PAD)).abs() < 0.5 && (b.y - (title.y - PANEL_PAD)).abs() < 0.5
        })
        .unwrap_or_else(|| panic!("padded card at {window:?}, title {title:?}"))
}

fn is_close(messages: &[Message]) -> bool {
    messages.iter().any(|m| {
        matches!(
            m,
            Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(false)))
        )
    })
}

/// A click on the card's 24 px panel padding stays on the card; only a
/// click on the dimmed backdrop closes the dialog.
#[test]
fn what_leaves_card_padding_does_not_close() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(true))));
    for window in WINDOWS {
        let card = card_bounds(
            crate::what_leaves::layer(&app).expect("open").content,
            window,
        );
        let on_pad = Point::new(card.x + 12.0, card.y + 12.0);
        let on_dim = Point::new(8.0, window.1 / 2.0);
        assert!(
            card.contains(on_pad),
            "{window:?}: pad click {on_pad:?} is outside the card {card:?}"
        );
        assert!(
            !card.contains(on_dim),
            "{window:?}: backdrop click {on_dim:?} is inside the card {card:?}"
        );
        let pad = press_at(
            crate::what_leaves::layer(&app).expect("open").content,
            window,
            on_pad,
        );
        assert!(
            !is_close(&pad),
            "{window:?}: a click on the card padding closed the dialog: {pad:?}"
        );
        let dim = press_at(
            crate::what_leaves::layer(&app).expect("open").content,
            window,
            on_dim,
        );
        assert!(
            is_close(&dim),
            "{window:?}: a click on the backdrop did not close the dialog: {dim:?}"
        );
    }
}

/// Esc closes What leaves once, through the dialog layer's on_escape
/// (Dialog::new(..., Some(close()))), not a second app subscription.
#[test]
fn what_leaves_escape_closes_once() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(true))));
    let messages = press_esc(what_leaves_top(&app), WINDOWS[0]);
    let closes = messages
        .iter()
        .filter(|m| {
            matches!(
                m,
                Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(false)))
            )
        })
        .count();
    assert_eq!(
        closes, 1,
        "Esc should close What leaves once, got {messages:?}"
    );
}
