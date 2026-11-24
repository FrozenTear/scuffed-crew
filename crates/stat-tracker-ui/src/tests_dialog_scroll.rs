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
use iced::advanced::layout::Limits;
use iced::advanced::widget::operation::{Operation, Scrollable};
use iced::advanced::widget::{Id, Tree};
use iced::{Element, Rectangle, Size, Vector};

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
            assert!(
                p.text_right <= p.bar_left() + 0.5,
                "{name} {window:?}: text ends at {:.1}, bar starts at {:.1}: {:?}",
                p.text_right,
                p.bar_left(),
                p.text
            );
            assert!(
                p.bar_left() - p.text_right >= min_gap - 0.5,
                "{name} {window:?}: gap is {:.1}, need at least {min_gap}: {:?}",
                p.bar_left() - p.text_right,
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
        crate::what_leaves::layer(&app).expect("open")
    });
}

#[test]
fn whats_new_keeps_its_text_clear_of_the_bar() {
    let sections = crate::notes::render(crate::notes::all_notes(crate::notes::BUNDLED_CHANGELOG));
    let closed = HashSet::new();
    let open: HashSet<String> = sections.iter().map(|s| s.version.clone()).collect();
    for (show_older, open_details) in [(false, &closed), (true, &open)] {
        assert_clear("what's new", 0.0, || {
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
    for dialog in [
        ReportDialog::new(None, true),
        ReportDialog::for_game(None, true),
    ] {
        let ui = ReportUi::Dialog(Box::new(dialog));
        assert_clear("report", 0.0, || crate::report::view(&ui));
    }
}

#[test]
fn uninstall_keeps_its_text_clear_of_the_bar() {
    use crate::uninstall::UninstallDialog;
    let home = std::path::PathBuf::from("/home/a-rather-long-account-name-for-testing");
    let dialog = UninstallDialog::Confirm {
        prefix: home.join(".local"),
        data_dir: home.join(".local/share/scuffed-stat-tracker"),
        appimage: Some(home.join("Applications/Scuffed-Stat-Tracker-x86_64.AppImage")),
        home,
        delete_data: true,
        fallback: true,
    };
    assert_clear("uninstall", 0.0, || crate::uninstall::view(&dialog, false));
}

/// Each step the guide reaches with Skip this step, from the first.
#[test]
fn the_setup_guide_keeps_its_text_clear_of_the_bar() {
    let (_dir, mut app) = crate::fixtures::test_app();
    let _ = app.update(Message::Setup(SetupMessage::Open));
    for step in 0..4 {
        assert_clear(&format!("setup guide step {step}"), GRID_GAP, || {
            crate::setup_guide::view(&app.setup)
        });
        let _ = app.update(Message::Setup(SetupMessage::SkipStep));
    }
}
