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
use iced::mouse::{Button, Cursor, Event as Mouse, Interaction, ScrollDelta};
use iced::{Element, Event, Point, Rectangle, Size, Vector};

use crate::app::{Message, TrackerApp};
use crate::chrome::TopDialog;
use crate::layout::SCROLLBAR_WIDTH;
use crate::settings_open::OpenMsg;
use crate::settings_update::Msg;
use crate::setup_guide::SetupMessage;
use crate::theme::{GRID_GAP, SIZE_META};

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
    height: f32,
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
    /// Containers inside a scrollable, below its own content wrapper.
    boxes: Vec<(Rectangle, Rectangle)>,
}

impl Operation<()> for Collect {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        self.depth += 1;
        operate(self);
        self.depth -= 1;
        let depth = self.depth;
        self.scrolls.retain(|(d, _, _)| *d < depth);
    }

    fn container(&mut self, _id: Option<&Id>, bounds: Rectangle) {
        if let Some(&(depth, scroll, _)) = self.scrolls.last()
            && self.depth > depth + 1
        {
            self.boxes.push((bounds, scroll));
        }
    }

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
                height: bounds.height,
                scroll,
                overflows,
            });
        }
    }
}

/// Lays `element` out as the window-sized layer it is in the app and
/// returns every text block drawn inside a scrollable.
fn placed(element: Element<'_, Message>, window: (f32, f32)) -> Vec<Placed> {
    collect(element, window).placed
}

fn collect(mut element: Element<'_, Message>, window: (f32, f32)) -> Collect {
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
    collect
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
        let build = || {
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
        };
        assert_clear("what's new", GRID_GAP, build);
        // The cards' own 20 px padding keeps their text off the bar even
        // with no gutter, so the boxes in the body (the cards and the
        // column around them) must end left of it too.
        for window in WINDOWS {
            let boxes = collect(build(), window).boxes;
            assert!(!boxes.is_empty(), "{window:?}: no box in the body");
            for (boxed, scroll) in boxes {
                let bar_left = scroll.x + scroll.width - SCROLLBAR_WIDTH;
                assert!(
                    boxed.x + boxed.width <= bar_left,
                    "what's new {window:?}: a box {boxed:?} runs under the bar at {bar_left}"
                );
            }
        }
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

/// A host-header token with no spaces (every report includes a line like
/// `drm=card1:i915[DP-1:...,HDMI-A-1:...]`) must wrap on glyphs so it
/// stays left of the log gutter instead of running past it and being cut.
#[test]
fn report_log_wraps_a_no_space_host_header_line() {
    use crate::report::{ReportDialog, ReportUi};
    use stat_tracker::report::{LogOnlyCategory, ReportZip};
    use std::sync::Arc;
    let host = "drm=card1:i915[DP-1:3840x2160@60,HDMI-A-1:1920x1080@60,DP-2:2560x1440@144,HDMI-A-2:1280x720@60]";
    let token = host.to_string() + &"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".repeat(6);
    let mut crash = ReportDialog::new(Some(LogOnlyCategory::Crash), true);
    crash.built = Some(Ok(Arc::new(ReportZip {
        bytes: vec![1, 2, 3],
        manifest_json: r#"{"game":null,"reason":{"category":"crash"}}"#.into(),
        files: Vec::new(),
        log_text: token,
        images: Vec::new(),
        left_out: Vec::new(),
    })));
    crash.show_log = true;
    let ui = ReportUi::Dialog(Box::new(crash));
    assert_clear("report no-space log", GRID_GAP, || crate::report::view(&ui));
    // Layout boxes are clipped to the guttered width, so assert_clear
    // still passes if WordOrGlyph is dropped. The host-header token also
    // has hyphens (HDMI-A-1), which Word wrapping can break after. A
    // second, unbreakable token must actually wrap: its laid-out height
    // has to be more than one line.
    let nospace = "drm=".to_string() + &"ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789".repeat(8);
    let mut wrap_dialog = ReportDialog::new(Some(LogOnlyCategory::Crash), true);
    wrap_dialog.built = Some(Ok(Arc::new(ReportZip {
        bytes: vec![1, 2, 3],
        manifest_json: r#"{"game":null,"reason":{"category":"crash"}}"#.into(),
        files: Vec::new(),
        log_text: nospace,
        images: Vec::new(),
        left_out: Vec::new(),
    })));
    wrap_dialog.show_log = true;
    let wrap_ui = ReportUi::Dialog(Box::new(wrap_dialog));
    let mut short_dialog = ReportDialog::new(Some(LogOnlyCategory::Crash), true);
    short_dialog.built = Some(Ok(Arc::new(ReportZip {
        bytes: vec![1, 2, 3],
        manifest_json: r#"{"game":null,"reason":{"category":"crash"}}"#.into(),
        files: Vec::new(),
        log_text: "drm=one-line-token".into(),
        images: Vec::new(),
        left_out: Vec::new(),
    })));
    short_dialog.show_log = true;
    let short_ui = ReportUi::Dialog(Box::new(short_dialog));
    for window in WINDOWS {
        let rows = placed(crate::report::view(&wrap_ui), window);
        let token = rows
            .iter()
            .find(|p| p.text.contains("drm=ABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789"))
            .unwrap_or_else(|| {
                panic!("report unbreakable log {window:?}: no-space token is not in a scrollable")
            });
        let short_rows = placed(crate::report::view(&short_ui), window);
        let one = short_rows
            .iter()
            .find(|p| p.text.contains("drm=one-line-token"))
            .unwrap_or_else(|| {
                panic!("report short log {window:?}: one-line token is not in a scrollable")
            });
        assert!(
            token.height > one.height + 0.5,
            "{window:?}: no-space token is only {} tall (one line is {}); it should wrap",
            token.height,
            one.height
        );
    }
}

/// A path component or host with no break point: letters and digits only,
/// so Word wrapping cannot split it. QA saw a 192-character one cut at the
/// bar in Uninstall; this one is 384, at least three lines in any dialog.
fn unbreakable() -> String {
    "a1b2c3d4e5f6".repeat(32)
}

/// Stands in for [`unbreakable`] in the one-line build.
const SHORT: &str = "short-name";

/// Every text block holding [`unbreakable`] (each ends with it) stays
/// clear of the bar and is at least two lines taller than the same block
/// holding [`SHORT`]. Text boxes are clamped to the padded width, so
/// [`assert_clear`] alone passes on a token that runs on under the bar.
/// Word wrapping moves the token to a line of its own, one line more;
/// only breaking it on glyphs makes it two or more.
fn assert_token_wraps<'a>(
    name: &str,
    long: impl Fn() -> Element<'a, Message>,
    short: impl Fn() -> Element<'a, Message>,
) {
    assert_clear(name, GRID_GAP, &long);
    let token = unbreakable();
    for window in WINDOWS {
        let blocks = |element: Element<'a, Message>, needle: &str| -> Vec<Placed> {
            let all = placed(element, window).into_iter();
            all.filter(|p| p.text.contains(needle)).collect()
        };
        let (tall, one) = (blocks(long(), &token), blocks(short(), SHORT));
        assert!(
            !tall.is_empty() && tall.len() == one.len(),
            "{name} {window:?}: token in {} blocks, short name in {}",
            tall.len(),
            one.len()
        );
        for (tall, one) in tall.iter().zip(&one) {
            let line = one.height / one.text.lines().count() as f32;
            assert!(
                tall.height >= one.height + 2.0 * line - 0.5,
                "{name} {window:?}: the token block is {} tall, the short one {} \
                 ({line} a line): the token did not wrap: {:?}",
                tall.height,
                one.height,
                one.text
            );
        }
    }
}

/// Uninstall lists the data dir and the AppImage in its groups and the
/// data dir again under the checkbox. Each wraps a no-space name.
#[test]
fn uninstall_wraps_an_unbreakable_path_clear_of_the_bar() {
    use crate::uninstall::UninstallDialog;
    let confirm = |name: &str| {
        let home = std::path::PathBuf::from("/home/qa");
        UninstallDialog::Confirm {
            prefix: home.join(".local"),
            data_dir: home.join(".local/share").join(name),
            appimage: Some(home.join("Applications").join(format!("{name}.AppImage"))),
            home,
            delete_data: true,
            fallback: true,
        }
    };
    let (long, short) = (confirm(&unbreakable()), confirm(SHORT));
    assert_token_wraps(
        "uninstall",
        || crate::uninstall::view(&long, false),
        || crate::uninstall::view(&short, false),
    );
}

/// Report a problem: the Saved to path and a send failure that names an
/// address, each with a no-space token.
#[test]
fn report_wraps_an_unbreakable_path_and_reason_clear_of_the_bar() {
    use crate::report::{ReportDialog, ReportUi, SendOutcome, SendState};
    let ui = |name: &str| {
        let mut dialog = ReportDialog::new(None, true);
        dialog.saved = Some(Ok(std::path::PathBuf::from("/home/qa/reports").join(name)));
        let reason = format!("Could not reach https://{name}");
        dialog.send = SendState::Done(SendOutcome::Failed(reason));
        ReportUi::Dialog(Box::new(dialog))
    };
    let (long, short) = (ui(&unbreakable()), ui(SHORT));
    assert_token_wraps(
        "report",
        || crate::report::view(&long),
        || crate::report::view(&short),
    );
}

/// The setup guide's Sync step shows the site's answer to Check token.
#[test]
fn the_setup_guide_wraps_an_unbreakable_server_message_clear_of_the_bar() {
    let guide = |name: &str| {
        let (dir, mut app) = crate::fixtures::test_app();
        let _ = app.update(Message::Setup(SetupMessage::Open));
        let _ = app.update(Message::Setup(SetupMessage::SkipStep)); // Overwatch
        let _ = app.update(Message::Setup(SetupMessage::SkipStep)); // Sync
        let message = format!("Could not reach https://{name}");
        let _ = app.setup.token_checked(Err(message));
        (dir, app)
    };
    let ((_long_dir, long), (_short_dir, short)) = (guide(&unbreakable()), guide(SHORT));
    assert_token_wraps(
        "setup guide",
        || crate::setup_guide::view(&long.setup),
        || crate::setup_guide::view(&short.setup),
    );
}

/// What's new: a release summary and a highlight with a no-space token
/// (a remote release body can carry a long link).
#[test]
fn whats_new_wraps_an_unbreakable_summary_clear_of_the_bar() {
    let release = |name: &str| crate::notes::ShownRelease {
        version: "0.7.0".into(),
        summary: format!("See https://{name}"),
        highlights: vec![format!("Moved to {name}")],
        items: Vec::new(),
        published_on: None,
    };
    let (long, short) = ([release(&unbreakable())], [release(SHORT)]);
    let closed = HashSet::new();
    let dialog = |sections| {
        crate::notes::dialog(
            "Release notes",
            sections,
            crate::notes::NotesUi {
                surface: crate::notes::NotesSurface::Dialog,
                installed: Some("0.6.0"),
                offered: None,
                open_details: &closed,
                show_older: false,
            },
        )
    };
    assert_token_wraps("what's new", || dialog(&long), || dialog(&short));
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

/// Close stays inside the window and the card, at its own size, at every
/// size the dialog is checked at.
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
        let card = card_bounds(
            crate::what_leaves::layer(&app).expect("open").content,
            window,
        );
        // Inside the card and at its own size: squeezed, Close is laid out
        // 0 x 0 and the button is gone while the box is still in the window.
        let inside = card.contains(close_box.position())
            && card.contains(Point::new(
                close_box.x + close_box.width,
                close_box.y + close_box.height,
            ));
        assert!(
            inside && close_box.width >= SIZE_META && close_box.height >= SIZE_META,
            "{window:?}: Close at {close_box:?} is squeezed or outside the card {card:?}"
        );
        let window_cap = window.1 - 28.0 * 2.0;
        let cap = window_cap.min(560.0);
        assert!(
            card.height <= cap + 0.5,
            "{window:?}: card height {} exceeds cap {cap}",
            card.height
        );
        // On a tall window, Fill would stretch the card to the layer.
        // On 720x480 the cap is the layer itself, so the card may use it.
        if 560.0 < window_cap {
            assert!(
                card.height < window_cap - 0.5,
                "{window:?}: card height {} fills the layer ({window_cap})",
                card.height
            );
        }
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
fn what_leaves_top(app: &TrackerApp) -> Element<'_, Message> {
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

fn text_bounds(mut element: Element<'_, Message>, window: (f32, f32), needle: &str) -> Rectangle {
    let renderer = headless();
    let mut tree = Tree::new(&element);
    let node = element.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(Size::ZERO, Size::new(window.0, window.1)),
    );
    struct FindText<'a> {
        needle: &'a str,
        found: Option<Rectangle>,
    }
    impl Operation<()> for FindText<'_> {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }
        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            if text == self.needle {
                self.found = Some(bounds);
            }
        }
    }
    let mut find = FindText {
        needle,
        found: None,
    };
    element
        .as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut find);
    find.found
        .unwrap_or_else(|| panic!("{needle:?} is laid out at {window:?}"))
}

fn is_close(messages: &[Message]) -> bool {
    messages.iter().any(|m| {
        matches!(
            m,
            Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(false)))
        )
    })
}

/// A click on the card (title, body text, or 24 px panel padding) stays
/// on the card; only a click on the dimmed backdrop closes the dialog.
#[test]
fn what_leaves_card_clicks_do_not_close() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(true))));
    const BODY: &str = "Game uploads, while Upload games is on";
    for window in WINDOWS {
        let layer = || what_leaves_top(&app);
        let card = card_bounds(layer(), window);
        let title = text_bounds(layer(), window, "What leaves your PC");
        let body = text_bounds(layer(), window, BODY);
        let on_title = Point::new(title.x + title.width / 2.0, title.y + title.height / 2.0);
        let on_text = Point::new(body.x + body.width.min(40.0), body.y + body.height / 2.0);
        let on_pad = Point::new(card.x + 12.0, card.y + 12.0);
        let on_dim = Point::new(8.0, window.1 / 2.0);
        assert!(
            card.contains(on_title) && card.contains(on_text) && card.contains(on_pad),
            "{window:?}: a card click is outside the card {card:?}:              title {on_title:?}, text {on_text:?}, pad {on_pad:?}"
        );
        assert!(
            !card.contains(on_dim),
            "{window:?}: backdrop click {on_dim:?} is inside the card {card:?}"
        );
        for (name, point) in [("title", on_title), ("text", on_text), ("padding", on_pad)] {
            let messages = press_at(layer(), window, point);
            assert!(
                !is_close(&messages),
                "{window:?}: a click on the card {name} closed the dialog: {messages:?}"
            );
        }
        let dim = press_at(layer(), window, on_dim);
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

/// The window sizes the backdrop tests run at: the default and the minimum.
const BACKDROP_WINDOWS: [(f32, f32); 2] = [(1280.0, 860.0), (720.0, 480.0)];

/// A demo app on Settings in the top tabs layout, with `which` open over
/// it (or no dialog). Settings is taller than the pane at both sizes.
fn over_settings(which: Option<TopDialog>) -> (tempfile::TempDir, TrackerApp) {
    use crate::uninstall::UninstallDialog;
    let (dir, mut app) = crate::fixtures::test_app();
    if app.notes_layer().is_some() {
        let _ = app.update(Message::DismissNotes);
    }
    app.nav_style = crate::nav::NavStyle::TopTabs;
    app.screen = crate::model::Screen::Settings;
    app.toast = None;
    assert_eq!(crate::chrome::top_dialog(&app), None);
    match which {
        None => {}
        Some(TopDialog::WhatLeaves) => {
            let _ = app.update(Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(true))));
        }
        Some(TopDialog::Report) => {
            let dialog = crate::report::ReportDialog::new(None, true);
            app.report = Some(crate::report::ReportUi::Dialog(Box::new(dialog)));
        }
        Some(TopDialog::Uninstall) => {
            let home = std::path::PathBuf::from("/home/qa");
            app.uninstall_dialog = Some(UninstallDialog::Confirm {
                prefix: home.join(".local"),
                data_dir: home.join(".local/share/scuffed-stat-tracker"),
                appimage: None,
                home,
                delete_data: false,
                fallback: false,
            });
        }
        Some(TopDialog::Guide) => {
            let _ = app.update(Message::Setup(SetupMessage::Open));
        }
        Some(TopDialog::Notes) => {
            let _ = app.update(Message::OpenAbout);
        }
    }
    assert_eq!(crate::chrome::top_dialog(&app), which);
    (dir, app)
}

/// The page pane's bounds and scroll offset.
#[derive(Default)]
struct Pane(Option<(Rectangle, f32)>);

impl Operation<()> for Pane {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        _state: &mut dyn Scrollable,
    ) {
        if id == Some(&Id::new(crate::focus_status::PANE_SCROLL_ID)) {
            self.0 = Some((bounds, translation.y));
        }
    }
}

/// Wheels down twice on a backdrop point over the page pane, in the 28 px
/// every dialog layer keeps around its card, and returns the pane offset
/// before and after, and the window's cursor at that point.
fn wheel_on_backdrop(app: &TrackerApp, window: (f32, f32)) -> (f32, f32, Interaction) {
    let renderer = headless();
    let mut element = crate::chrome::window(app);
    let mut tree = Tree::new(&element);
    let size = Size::new(window.0, window.1);
    let node = element
        .as_widget_mut()
        .layout(&mut tree, &renderer, &Limits::new(Size::ZERO, size));
    let pane = |element: &mut Element<'_, Message>, tree: &mut Tree| {
        let mut pane = Pane::default();
        element
            .as_widget_mut()
            .operate(tree, Layout::new(&node), &renderer, &mut pane);
        pane.0.expect("the page pane is laid out")
    };
    let (bounds, before) = pane(&mut element, &mut tree);
    let point = Point::new(bounds.x + 8.0, bounds.y + bounds.height / 2.0);
    assert!(
        point.x < 28.0,
        "{window:?}: {point:?} is not in the layer pad"
    );
    let viewport = Rectangle::new(Point::ORIGIN, size);
    let wheel = Event::Mouse(Mouse::WheelScrolled {
        delta: ScrollDelta::Lines { x: 0.0, y: -3.0 },
    });
    for _ in 0..2 {
        let mut messages = Vec::new();
        element.as_widget_mut().update(
            &mut tree,
            &wheel,
            Layout::new(&node),
            Cursor::Available(point),
            &renderer,
            &mut clipboard::Null,
            &mut Shell::new(&mut messages),
            &viewport,
        );
    }
    let (_, after) = pane(&mut element, &mut tree);
    let cursor = element.as_widget().mouse_interaction(
        &tree,
        Layout::new(&node),
        Cursor::Available(point),
        &viewport,
        &renderer,
    );
    (before, after, cursor)
}

/// The cursor `content` reports at each point of a 20 px grid over the
/// window, laid out as the window-sized layer it is in the app.
fn layer_cursors(
    mut content: Element<'_, Message>,
    window: (f32, f32),
) -> Vec<(Point, Interaction)> {
    let renderer = headless();
    let mut tree = Tree::new(&content);
    let size = Size::new(window.0, window.1);
    let node = content
        .as_widget_mut()
        .layout(&mut tree, &renderer, &Limits::new(Size::ZERO, size));
    let viewport = Rectangle::new(Point::ORIGIN, size);
    let mut out = Vec::new();
    for x in (2..window.0 as u32).step_by(20) {
        for y in (2..window.1 as u32).step_by(20) {
            let point = Point::new(x as f32, y as f32);
            let cursor = content.as_widget().mouse_interaction(
                &tree,
                Layout::new(&node),
                Cursor::Available(point),
                &viewport,
                &renderer,
            );
            out.push((point, cursor));
        }
    }
    out
}

/// A dialog covers the page for the mouse, not only for presses: a wheel
/// on its dimmed backdrop does not scroll the page under it, and the layer
/// reports a cursor at every point, so iced's Stack hides the cursor from
/// the page (no hand cursor or hover through it). The bare page scrolls
/// under the same wheel, so the test can see a leak.
#[test]
fn dialog_backdrops_keep_wheel_and_hover_off_the_page() {
    let dialogs = [
        TopDialog::WhatLeaves,
        TopDialog::Report,
        TopDialog::Uninstall,
        TopDialog::Guide,
        TopDialog::Notes,
    ];
    for window in BACKDROP_WINDOWS {
        let (_dir, app) = over_settings(None);
        let (before, after, _) = wheel_on_backdrop(&app, window);
        assert!(
            after > before + 1.0,
            "{window:?}: the bare page did not scroll ({before} to {after})"
        );
        for which in dialogs {
            let (_dir, app) = over_settings(Some(which));
            let (before, after, cursor) = wheel_on_backdrop(&app, window);
            assert_eq!(
                after, before,
                "{which:?} {window:?}: a wheel on the backdrop scrolled the page"
            );
            assert_eq!(
                cursor,
                Interaction::Idle,
                "{which:?} {window:?}: the window cursor on the backdrop"
            );
            let content = match which {
                TopDialog::WhatLeaves => crate::what_leaves::layer(&app).expect("open").content,
                TopDialog::Report => crate::report::view(app.report.as_ref().expect("open")),
                TopDialog::Uninstall => {
                    crate::uninstall::view(app.uninstall_dialog.as_ref().expect("open"), false)
                }
                TopDialog::Guide => crate::setup_guide::view(&app.setup),
                TopDialog::Notes => app.notes_layer().expect("open"),
            };
            let through: Vec<_> = layer_cursors(content, window)
                .into_iter()
                .filter(|(_, cursor)| *cursor == Interaction::None)
                .map(|(point, _)| point)
                .collect();
            assert!(
                through.is_empty(),
                "{which:?} {window:?}: {} points let the page set the cursor, first {:?}",
                through.len(),
                through.first()
            );
        }
    }
}
