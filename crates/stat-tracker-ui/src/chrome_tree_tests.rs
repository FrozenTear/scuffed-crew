//! The page scroll must survive a toast or a dialog coming and going, in
//! both layouts. iced keeps a widget's state (a scroll offset) only while
//! the widget sits at the same place in the tree with the same tag, so these
//! tests walk the laid-out tree from the root to the scrollable they follow
//! (the page pane, [`crate::focus_status::PANE_SCROLL_ID`], or the classic
//! Settings form) and compare that path, the offset and the content size.

use super::*;
use iced::advanced::Layout;
use iced::advanced::layout::{Limits, Node};
use iced::advanced::widget::operation::scrollable::AbsoluteOffset;
use iced::advanced::widget::operation::{self, Operation};
use iced::advanced::widget::{Id, Tree, tree};
use iced::widget::{scrollable, text};
use iced::{Rectangle, Size, Vector};

use crate::focus_status::PANE_SCROLL_ID;

/// A short window, so the demo page is taller than the pane.
const WINDOW: Size = Size::new(1280.0, 300.0);

/// The windows the click grid runs over.
const CLICK_WINDOWS: [Size; 3] = [WINDOW, Size::new(1280.0, 860.0), Size::new(1000.0, 768.0)];

/// A systemd-style start failure: it wraps to several lines at the toast
/// width, past the one-line bar the reserved floor is sized for.
const WRAPPED_TOAST: &str = concat!(
    r#"Job for scuffed-stat-tracker.service failed because the control process exited with error code. "#,
    r#"See "systemctl --user status scuffed-stat-tracker.service" and "#,
    r#""journalctl --user -xeu scuffed-stat-tracker.service" for details."#,
);

const INSET_WINDOWS: [Size; 3] = [WINDOW, Size::new(1280.0, 860.0), Size::new(720.0, 480.0)];

/// How far the tests scroll down before the change.
const SCROLL_Y: f32 = 120.0;

type View = for<'a> fn(&'a TrackerApp) -> Element<'a, Message>;

pub(crate) fn headless() -> iced::Renderer {
    use iced::advanced::renderer::Headless;
    let renderer = iced::futures::executor::block_on(<iced::Renderer as Headless>::new(
        theme::FONT_MEDIUM,
        iced::Pixels(theme::SIZE_BODY),
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

fn app() -> (tempfile::TempDir, TrackerApp) {
    let (dir, mut app) = crate::fixtures::test_app();
    app.nav_style = NavStyle::TopTabs;
    app.screen = Screen::Overview;
    app.toast = None;
    (dir, app)
}

/// A demo app in the classic sidebar layout on `screen`, on the fixture
/// with games, so Games is taller than the pane.
fn classic_app(screen: Screen) -> (tempfile::TempDir, TrackerApp) {
    let dir = tempfile::tempdir().expect("tempdir");
    let mut cli = crate::fixtures::test_cli(dir.path());
    cli.fixture = Some(crate::cli::FixtureKind::Games);
    let mut app = TrackerApp::new(cli).0;
    close_launch_notes(&mut app);
    app.nav_style = NavStyle::ClassicSidebar;
    app.screen = screen;
    app.toast = None;
    (dir, app)
}

/// Closes What's new, which a fixture app may open at start.
pub(super) fn close_launch_notes(app: &mut TrackerApp) {
    if app.notes_layer().is_some() {
        let _ = app.update(Message::DismissNotes);
    }
    assert!(app.notes_layer().is_none());
}

/// `top_tabs` as it was before the scroll-reset fix: the prompts layer
/// only while there is a toast or the restart bar, the window alone
/// otherwise.
fn old_top_tabs(app: &TrackerApp) -> Element<'_, Message> {
    responsive(move |size| match top_tabs_parts(app, size.width) {
        (window, Some(prompts)) => stack![window, prompts].width(Fill).height(Fill).into(),
        (window, None) => window.into(),
    })
    .width(Fill)
    .height(Fill)
    .into()
}

fn current_top_tabs(app: &TrackerApp) -> Element<'_, Message> {
    top_tabs(app)
}

/// The production window with no dialog open, in the app's layout.
fn whole_window(app: &TrackerApp) -> Element<'_, Message> {
    window(app)
}

fn a_dialog_layer<'a>() -> Dialog<'a> {
    Dialog::new(container(text("A dialog")), None)
}

/// The whole window with one dialog open, or none.
fn window_with(app: &TrackerApp, dialog: bool) -> Element<'_, Message> {
    with_layers(app, [dialog.then(a_dialog_layer)])
}

fn no_dialog(app: &TrackerApp) -> Element<'_, Message> {
    window_with(app, false)
}

fn a_dialog(app: &TrackerApp) -> Element<'_, Message> {
    window_with(app, true)
}

fn closed_three_slots(app: &TrackerApp) -> Element<'_, Message> {
    with_layers(app, [None, None, None])
}

/// A dialog over the page, with two closed slots under it and the usual
/// trailing What leaves slot, all empty.
fn dialog_with_empty_slots_above(app: &TrackerApp) -> Element<'_, Message> {
    with_layers(app, [None, None, Some(a_dialog_layer())])
}

/// The page and one dialog, with no empty slots and no focus layers.
fn dialog_only(app: &TrackerApp) -> Element<'_, Message> {
    let page = container(top_tabs(app))
        .width(Fill)
        .height(Fill)
        .style(theme::page_background);
    Stack::with_children([page.into(), container(text("A dialog")).into()])
        .width(Fill)
        .height(Fill)
        .into()
}

/// The window stacked with its real prompts layer, and nothing else.
fn stacked_prompts(app: &TrackerApp) -> Element<'_, Message> {
    responsive(move |size| {
        let (window, prompts) = top_tabs_parts(app, size.width);
        stack![window, prompts.expect("a toast or the restart bar")]
            .width(Fill)
            .height(Fill)
            .into()
    })
    .width(Fill)
    .height(Fill)
    .into()
}

pub(super) fn show_restart_bar(app: &mut TrackerApp) {
    use crate::restart_prompt::{BinStamp, RestartWatch};
    let first = BinStamp {
        modified: None,
        dev: 1,
        ino: 1,
        len: 100,
    };
    let next = BinStamp {
        modified: None,
        dev: 1,
        ino: 2,
        len: 100,
    };
    let mut watch = RestartWatch::new("own-build-id", Some(first));
    let probe = watch.on_tick(Some(next), false).expect("probe");
    watch.on_probe(probe, Ok("other-build-id\n".into()));
    assert!(
        watch.prompt_pending(),
        "a different installed binary should show the bar"
    );
    app.set_restart_watch(watch);
    app.update_progress = crate::update::UpdateProgress::Idle;
    assert!(
        app.restart_bar_visible(),
        "the restart bar should be on for this test"
    );
}

fn hide_restart_bar(app: &mut TrackerApp) {
    app.set_restart_watch(crate::restart_prompt::RestartWatch::new(
        "own-build-id",
        None,
    ));
    assert!(!app.restart_bar_visible());
}

fn lay_out(
    element: &mut Element<'_, Message>,
    tree: &mut Tree,
    renderer: &iced::Renderer,
    size: Size,
) -> Node {
    element
        .as_widget_mut()
        .layout(tree, renderer, &Limits::new(Size::ZERO, size))
}

fn run(
    element: &mut Element<'_, Message>,
    tree: &mut Tree,
    node: &Node,
    renderer: &iced::Renderer,
    operation: &mut dyn Operation<()>,
) {
    element
        .as_widget_mut()
        .operate(tree, Layout::new(node), renderer, operation);
}

/// Which scrollable a test follows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Target {
    /// The page pane ([`PANE_SCROLL_ID`]).
    Pane,
    /// The classic Settings form. It has no id, so: the biggest scrollable
    /// whose content is taller than it.
    Form,
}

/// One scrollable as an operation sees it.
#[derive(Debug, Clone, Copy)]
struct Seen {
    pane: bool,
    /// The address of its state in the tree.
    state: usize,
    bounds: Rectangle,
    content: Rectangle,
    offset: Vector,
}

impl Target {
    fn pick(self, seen: &[Seen]) -> Option<Seen> {
        match self {
            Target::Pane => seen.iter().find(|s| s.pane).copied(),
            Target::Form => seen
                .iter()
                .filter(|s| !s.pane && s.content.height > s.bounds.height + 1.0)
                .max_by(|a, b| {
                    let area = |s: &Seen| s.bounds.width * s.bounds.height;
                    area(a).total_cmp(&area(b))
                })
                .copied(),
        }
    }
}

/// Every scrollable in the tree, in order; scrolls the one whose state is
/// at `scroll.0` to `scroll.1` on the way.
#[derive(Default)]
struct Scrollables {
    seen: Vec<Seen>,
    scroll: Option<(usize, f32)>,
}

impl Operation<()> for Scrollables {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content: Rectangle,
        offset: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        let address = state as *mut dyn operation::Scrollable as *const () as usize;
        if let Some((target, y)) = self.scroll
            && target == address
        {
            state.scroll_to(AbsoluteOffset {
                x: None,
                y: Some(y),
            });
        }
        self.seen.push(Seen {
            pane: id == Some(&Id::new(PANE_SCROLL_ID)),
            state: address,
            bounds,
            content,
            offset,
        });
    }
}

fn find(
    element: &mut Element<'_, Message>,
    tree: &mut Tree,
    node: &Node,
    renderer: &iced::Renderer,
    target: Target,
) -> Option<Seen> {
    let mut all = Scrollables::default();
    run(element, tree, node, renderer, &mut all);
    target.pick(&all.seen)
}

fn state_address(tree: &Tree) -> Option<usize> {
    match &tree.state {
        tree::State::Some(state) => {
            Some(&**state as *const dyn std::any::Any as *const () as usize)
        }
        tree::State::None => None,
    }
}

/// The (child index, tag) of every tree node from the root down to the one
/// whose state lives at `target`.
fn path_to(tree: &Tree, target: usize, path: &mut Vec<(usize, tree::Tag)>) -> bool {
    if state_address(tree) == Some(target) {
        return true;
    }
    for (i, child) in tree.children.iter().enumerate() {
        path.push((i, child.tag));
        if path_to(child, target, path) {
            return true;
        }
        path.pop();
    }
    false
}

/// The path of widget tags from the root to the followed scrollable.
fn scroll_path(
    app: &TrackerApp,
    view: View,
    target: Target,
    size: Size,
) -> Vec<(usize, tree::Tag)> {
    let renderer = headless();
    let mut element = view(app);
    let mut tree = Tree::new(&element);
    let node = lay_out(&mut element, &mut tree, &renderer, size);
    let found = find(&mut element, &mut tree, &node, &renderer, target)
        .unwrap_or_else(|| panic!("{target:?} is in the tree"));
    let mut path = vec![(0, tree.tag)];
    assert!(
        path_to(&tree, found.state, &mut path),
        "the scrollable's state is in the tree"
    );
    path
}

/// The path of widget tags from the root to the page pane.
fn pane_path(app: &TrackerApp, view: View) -> Vec<(usize, tree::Tag)> {
    scroll_path(app, view, Target::Pane, WINDOW)
}

/// The path of widget tags from the root to the first widget whose text
/// is `needle`. Children are searched first, so a shrink-wrapped parent
/// with the same bounds does not hide the text node.
fn text_path(app: &TrackerApp, view: View, needle: &str) -> Vec<(usize, tree::Tag)> {
    let renderer = headless();
    let mut element = view(app);
    let mut tree = Tree::new(&element);
    let node = lay_out(&mut element, &mut tree, &renderer, WINDOW);
    struct FindText<'a> {
        needle: &'a str,
        bounds: Option<Rectangle>,
    }
    impl Operation<()> for FindText<'_> {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }

        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            if self.bounds.is_none() && text == self.needle {
                self.bounds = Some(bounds);
            }
        }
    }
    let mut find = FindText {
        needle,
        bounds: None,
    };
    run(&mut element, &mut tree, &node, &renderer, &mut find);
    let bounds = find
        .bounds
        .unwrap_or_else(|| panic!("{needle} is in the tree"));
    let mut path = vec![(0, tree.tag)];
    assert!(
        path_to_bounds(&tree, Layout::new(&node), bounds, &mut path),
        "{needle} is in the tree"
    );
    path
}

fn path_to_bounds(
    tree: &Tree,
    layout: Layout<'_>,
    target: Rectangle,
    path: &mut Vec<(usize, tree::Tag)>,
) -> bool {
    for (i, child) in layout.children().enumerate() {
        let Some(child_tree) = tree.children.get(i) else {
            continue;
        };
        path.push((i, child_tree.tag));
        if path_to_bounds(child_tree, child, target, path) {
            return true;
        }
        path.pop();
    }
    let b = layout.bounds();
    (b.x - target.x).abs() < 0.5
        && (b.y - target.y).abs() < 0.5
        && (b.width - target.width).abs() < 0.5
        && (b.height - target.height).abs() < 0.5
}

/// A readable name for a tag, for the failure messages.
fn tag_name(tag: tree::Tag) -> &'static str {
    let samples: [(&str, Element<'_, Message>); 2] = [
        ("stateless", space().into()),
        ("scrollable", scrollable(space()).into()),
    ];
    samples
        .iter()
        .find(|(_, sample)| sample.as_widget().tag() == tag)
        .map_or("other", |(name, _)| *name)
}

fn readable(path: &[(usize, tree::Tag)]) -> Vec<String> {
    path.iter()
        .map(|(i, tag)| format!("{i}:{}", tag_name(*tag)))
        .collect()
}

/// The followed scrollable after scrolling it down in `view`, and again
/// after calling `change` on the app and diffing the tree against `next`,
/// as iced does.
fn scroll_across(
    app: &mut TrackerApp,
    (view, next): (View, View),
    (target, size): (Target, Size),
    change: impl FnOnce(&mut TrackerApp),
) -> (Seen, Seen) {
    let renderer = headless();
    let mut tree;
    let before;
    {
        let mut element = view(app);
        tree = Tree::new(&element);
        let node = lay_out(&mut element, &mut tree, &renderer, size);
        let found = find(&mut element, &mut tree, &node, &renderer, target)
            .unwrap_or_else(|| panic!("{target:?} is in the tree"));
        let mut scroll = Scrollables {
            scroll: Some((found.state, SCROLL_Y)),
            ..Scrollables::default()
        };
        run(&mut element, &mut tree, &node, &renderer, &mut scroll);
        let node = lay_out(&mut element, &mut tree, &renderer, size);
        before = find(&mut element, &mut tree, &node, &renderer, target).expect("still there");
        assert!(
            before.offset.y > 0.0,
            "{target:?}: the page is taller than the view: {before:?}"
        );
    }
    change(app);
    let mut element = next(app);
    tree.diff(&element);
    let node = lay_out(&mut element, &mut tree, &renderer, size);
    let after = find(&mut element, &mut tree, &node, &renderer, target).expect("still there");
    (before, after)
}

/// The pane's scroll offset after scrolling it down in `view`, then calling
/// `change` on the app and diffing the tree against `next`.
fn offset_after(
    app: &mut TrackerApp,
    view: View,
    next: View,
    change: impl FnOnce(&mut TrackerApp),
) -> f32 {
    scroll_across(app, (view, next), (Target::Pane, WINDOW), change)
        .1
        .offset
        .y
}

fn show_toast(app: &mut TrackerApp) {
    app.toast = Some("Report sent".into());
}

/// The cause of the scroll-reset bug: in the old shape the path to the
/// pane is not the same with a toast, so iced gives the pane a fresh state.
#[test]
fn the_old_top_tabs_shape_moves_the_pane_when_a_toast_shows() {
    let (_dir, mut app) = app();
    let quiet = pane_path(&app, old_top_tabs);
    show_toast(&mut app);
    let toast = pane_path(&app, old_top_tabs);
    eprintln!("old, no toast: {:?}", readable(&quiet));
    eprintln!("old, toast:    {:?}", readable(&toast));
    assert_ne!(quiet, toast);
    app.toast = None;
    assert_eq!(
        offset_after(&mut app, old_top_tabs, old_top_tabs, show_toast),
        0.0
    );
}

/// The fix: the path to the pane is the same with a toast, without one,
/// and after the toast is gone again, so the scroll stays where it was.
#[test]
fn a_toast_keeps_the_pane_and_its_scroll() {
    let (_dir, mut app) = app();
    let quiet = pane_path(&app, current_top_tabs);
    show_toast(&mut app);
    let toast = pane_path(&app, current_top_tabs);
    app.toast = None;
    let cleared = pane_path(&app, current_top_tabs);
    eprintln!("new, any toast: {:?}", readable(&quiet));
    assert_eq!(
        quiet,
        toast,
        "{:?} vs {:?}",
        readable(&quiet),
        readable(&toast)
    );
    assert_eq!(quiet, cleared);
    let kept = offset_after(&mut app, current_top_tabs, current_top_tabs, show_toast);
    assert!(kept > 0.0, "the toast reset the scroll");
    let kept = offset_after(&mut app, current_top_tabs, current_top_tabs, |app| {
        app.toast = None
    });
    assert!(kept > 0.0, "clearing the toast reset the scroll");
}

/// The same check for a dialog opening over the page ([`with_layers`]).
#[test]
fn a_dialog_keeps_the_pane_and_its_scroll() {
    let (_dir, mut app) = app();
    let closed = pane_path(&app, no_dialog);
    let open = pane_path(&app, a_dialog);
    eprintln!("no dialog: {:?}", readable(&closed));
    eprintln!("a dialog:  {:?}", readable(&open));
    assert_eq!(
        closed,
        open,
        "{:?} vs {:?}",
        readable(&closed),
        readable(&open)
    );
    let kept = offset_after(&mut app, no_dialog, a_dialog, |_| ());
    assert!(kept > 0.0, "opening the dialog reset the scroll");
    let kept = offset_after(&mut app, a_dialog, no_dialog, |_| ());
    assert!(kept > 0.0, "closing the dialog reset the scroll");
}

/// The same path and scroll check for the restart bar, and for a toast
/// and the restart bar together. Both live in the one prompts slot, so
/// the path should match the toast-only path.
#[test]
fn a_restart_bar_keeps_the_pane_and_its_scroll() {
    let (_dir, mut app) = app();
    let quiet = pane_path(&app, current_top_tabs);
    show_restart_bar(&mut app);
    let bar = pane_path(&app, current_top_tabs);
    show_toast(&mut app);
    let both = pane_path(&app, current_top_tabs);
    eprintln!("new, restart bar: {:?}", readable(&bar));
    assert_eq!(quiet, bar, "{:?} vs {:?}", readable(&quiet), readable(&bar));
    assert_eq!(quiet, both);
    app.toast = None;
    // offset_after starts from the current app (bar showing) after we
    // reset the toast; go back to a quiet app first.
    hide_restart_bar(&mut app);
    let kept = offset_after(
        &mut app,
        current_top_tabs,
        current_top_tabs,
        show_restart_bar,
    );
    assert!(kept > 0.0, "the restart bar reset the scroll");
    let kept = offset_after(&mut app, current_top_tabs, current_top_tabs, |app| {
        show_toast(app);
    });
    assert!(
        kept > 0.0,
        "a toast on top of the restart bar reset the scroll"
    );
}

/// Opening a dialog that has empty slots under and after it keeps the
/// same path and scroll as a closed stack with the same number of slots.
#[test]
fn a_dialog_with_empty_slots_above_keeps_the_pane_and_its_scroll() {
    let (_dir, mut app) = app();
    let closed = pane_path(&app, closed_three_slots);
    let open = pane_path(&app, dialog_with_empty_slots_above);
    eprintln!("empty slots, no dialog: {:?}", readable(&closed));
    eprintln!("empty slots, a dialog:  {:?}", readable(&open));
    assert_eq!(
        closed,
        open,
        "{:?} vs {:?}",
        readable(&closed),
        readable(&open)
    );
    let kept = offset_after(
        &mut app,
        closed_three_slots,
        dialog_with_empty_slots_above,
        |_| (),
    );
    assert!(
        kept > 0.0,
        "opening a dialog under empty slots reset the scroll"
    );
}

/// The prompts that come and go in the classic tests, each from a quiet
/// app, and how to take them away again.
type Prompt = (&'static str, fn(&mut TrackerApp));

const PROMPTS: [Prompt; 3] = [
    ("a toast", show_toast),
    ("the restart bar", show_restart_bar),
    ("a toast and the restart bar", |app| {
        show_toast(app);
        show_restart_bar(app);
    }),
];

fn quiet(app: &mut TrackerApp) {
    app.toast = None;
    hide_restart_bar(app);
}

/// The classic layout keeps `target` where it was when each prompt shows
/// and goes: the same path in the tree, the same scroll offset (above 0),
/// and the same content size, so nothing was pushed into the scrolled page.
fn classic_keeps_its_place(screen: Screen, target: Target, size: Size) {
    let (_dir, mut app) = classic_app(screen);
    let path = |app: &TrackerApp| scroll_path(app, whole_window, target, size);
    let quiet_path = path(&app);
    eprintln!("classic {screen:?}, quiet: {:?}", readable(&quiet_path));
    for (name, show) in PROMPTS {
        quiet(&mut app);
        show(&mut app);
        let shown = path(&app);
        assert_eq!(
            quiet_path,
            shown,
            "classic {screen:?}, {name}: {:?} vs {:?}",
            readable(&quiet_path),
            readable(&shown)
        );
        quiet(&mut app);
        for (when, from_quiet) in [("shows", true), ("goes", false)] {
            if !from_quiet {
                show(&mut app);
            }
            let views = (whole_window as View, whole_window as View);
            let change = |app: &mut TrackerApp| {
                if from_quiet {
                    show(app);
                } else {
                    quiet(app);
                }
            };
            let (before, after) = scroll_across(&mut app, views, (target, size), change);
            assert!(
                after.offset.y > 0.0,
                "classic {screen:?}: {name} {when}: reset"
            );
            assert_eq!(
                (before.offset, before.content.size()),
                (after.offset, after.content.size()),
                "classic {screen:?}: {name} {when}: the page moved or grew"
            );
            quiet(&mut app);
        }
    }
}

/// Classic Games scrolls the whole pane: a toast or the restart bar floats
/// over it, so the pane stays, scrolled, and its content is not pushed down.
#[test]
fn classic_games_keeps_the_pane_and_its_scroll() {
    classic_keeps_its_place(Screen::Games, Target::Pane, WINDOW);
}

/// Classic Settings scrolls its own form (no id; found by walking the
/// tree): a toast or the restart bar coming or going keeps its scroll.
#[test]
fn classic_settings_keeps_the_form_scroll() {
    classic_keeps_its_place(Screen::Settings, Target::Form, Size::new(1280.0, 500.0));
}

/// What clicking at each point of a 16 px grid over a `size` window
/// publishes.
fn clicks(app: &TrackerApp, view: View, size: Size) -> Vec<String> {
    use iced::advanced::{Shell, clipboard};
    use iced::mouse::{Button, Cursor, Event as Mouse};
    let renderer = headless();
    let mut element = view(app);
    let mut tree = Tree::new(&element);
    let node = lay_out(&mut element, &mut tree, &renderer, size);
    let viewport = Rectangle::new(iced::Point::ORIGIN, size);
    let mut published = Vec::new();
    for y in (4..size.height as u32).step_by(16) {
        for x in (4..size.width as u32).step_by(16) {
            let cursor = Cursor::Available(iced::Point::new(x as f32, y as f32));
            let mut messages = Vec::new();
            for event in [
                Mouse::ButtonPressed(Button::Left),
                Mouse::ButtonReleased(Button::Left),
            ] {
                let mut shell = Shell::new(&mut messages);
                element.as_widget_mut().update(
                    &mut tree,
                    &iced::Event::Mouse(event),
                    Layout::new(&node),
                    cursor,
                    &renderer,
                    &mut clipboard::Null,
                    &mut shell,
                    &viewport,
                );
            }
            published.extend(messages.iter().map(|m| format!("{x},{y}: {m:?}")));
        }
    }
    published
}

/// The window alone, with no layer over it.
fn bare_window(app: &TrackerApp) -> Element<'_, Message> {
    responsive(move |size| top_tabs_parts(app, size.width).0.into())
        .width(Fill)
        .height(Fill)
        .into()
}

/// The classic window alone: the sidebar and the pane, no prompts layer.
fn bare_classic(app: &TrackerApp) -> Element<'_, Message> {
    row![
        container(widgets::sidebar(app.screen))
            .width(SIDEBAR_WIDTH)
            .height(Fill)
            .padding(Padding {
                top: PAGE_PAD_Y,
                bottom: PAGE_PAD_Y,
                left: PAGE_PAD_X,
                right: 8.0,
            }),
        responsive(move |size| sidebar_pane_parts(app, size.width).0)
            .width(Fill)
            .height(Fill),
    ]
    .into()
}

fn classic(app: &TrackerApp) -> Element<'_, Message> {
    sidebar(app)
}

/// The empty layers take no clicks: every click on the page does what it
/// does with nothing over the page, in both layouts.
#[test]
fn the_empty_layers_let_clicks_through() {
    let (_dir, app) = app();
    let (_classic_dir, classic_app) = classic_app(Screen::Games);
    for size in CLICK_WINDOWS {
        let bare = clicks(&app, bare_window, size);
        assert!(!bare.is_empty(), "{size:?}: the grid hits the page");
        assert_eq!(clicks(&app, current_top_tabs, size), bare, "{size:?}");
        assert_eq!(clicks(&app, no_dialog, size), bare, "{size:?}");

        let bare = clicks(&classic_app, bare_classic, size);
        assert!(!bare.is_empty(), "{size:?}: the grid hits the page");
        assert_eq!(clicks(&classic_app, classic, size), bare, "{size:?}");
        assert_eq!(clicks(&classic_app, whole_window, size), bare, "{size:?}");
    }
}

/// Trailing empty slots over an open dialog do not steal clicks from
/// the dialog or the page, nor do the focus layers: the published clicks
/// match a stack that has only that dialog.
#[test]
fn an_open_dialog_with_empty_slots_keeps_its_clicks() {
    let (_dir, app) = app();
    for size in CLICK_WINDOWS {
        let only = clicks(&app, dialog_only, size);
        assert!(!only.is_empty(), "{size:?}: the grid hits the page");
        assert_eq!(clicks(&app, a_dialog, size), only, "{size:?}");
        let slots = clicks(&app, dialog_with_empty_slots_above, size);
        assert_eq!(slots, only, "{size:?}");
    }
}

/// Clicks beside a toast or the restart bar still reach the page, and
/// the production prompts slot matches a stack of just the window and
/// those prompts.
#[test]
fn a_toast_or_restart_bar_keeps_page_clicks() {
    let (_dir, mut app) = app();
    for size in CLICK_WINDOWS {
        quiet(&mut app);
        let bare = clicks(&app, bare_window, size);
        assert!(!bare.is_empty(), "{size:?}: the grid hits the page");
        for (name, show) in PROMPTS {
            quiet(&mut app);
            show(&mut app);
            let over = clicks(&app, current_top_tabs, size);
            assert_eq!(
                over,
                clicks(&app, stacked_prompts, size),
                "{size:?}: {name}"
            );
            assert!(
                shares_page_clicks(&bare, &over),
                "{size:?}: clicks beside {name} should still reach the page"
            );
        }
    }
}

/// Some grid points publish the same message with the overlay as they
/// do on the bare page, so the overlay has not swallowed the whole window.
fn shares_page_clicks(bare: &[String], over: &[String]) -> bool {
    let over: std::collections::HashSet<&String> = over.iter().collect();
    bare.iter().any(|click| over.contains(click))
}

/// The reserved bottom inset is at least as tall as a toast and the
/// restart bar stacked with the bottom gap, including a long wrapped
/// toast, so the slot cannot cover the last control when both show.
///
/// A one-line toast, and a long message that would overflow
/// [`crate::widgets::TOAST_MAX`] if the cap were dropped, both at the
/// classic 720 pane (488 px) and at 640 px. 400 px is below the 720x480
/// window minimum and is not used here. The long toast is asserted at
/// the cap, and the slot (toast + restart bar + gap) still fits
/// [`super::PROMPTS_INSET`].
#[test]
fn the_prompts_inset_covers_a_toast_and_the_restart_bar() {
    use iced::advanced::layout::Limits;
    let renderer = headless();
    // WRAPPED_TOAST still fitted uncapped at these widths, so the cap
    // pin uses a longer message: many repeats, joined with spaces.
    let long = [WRAPPED_TOAST; 24].join(" ");
    for page in [488.0, 640.0] {
        for (name, toast) in [
            ("one line", "Report sent".to_string()),
            ("capped", long.clone()),
        ] {
            let mut slot = super::prompts_slot(&toast, page);
            let mut tree = Tree::new(&slot);
            let node = slot.as_widget_mut().layout(
                &mut tree,
                &renderer,
                &Limits::new(Size::ZERO, Size::new(page, 2000.0)),
            );
            let height = node.size().height;
            assert!(
                height <= super::PROMPTS_INSET,
                "{name}: inset {} is short of the slot {height} at page {page}",
                super::PROMPTS_INSET
            );
            assert!(
                height > super::PROMPTS_INSET / 2.0,
                "{name}: slot {height} looks too small; the inset may be guessing"
            );
            let mut bar = crate::widgets::toast_bar(&toast, None);
            let mut tree = Tree::new(&bar);
            let node = bar.as_widget_mut().layout(
                &mut tree,
                &renderer,
                &Limits::new(Size::ZERO, Size::new(page, 2000.0)),
            );
            let toast_h = node.size().height;
            assert!(
                toast_h <= crate::widgets::TOAST_MAX + 0.5,
                "{name}: toast bar {toast_h} taller than TOAST_MAX {} at page {page}",
                crate::widgets::TOAST_MAX
            );
            if name == "capped" {
                // Iced's layout height leaves out the 1px border on each
                // side, so a toast at the cap lays out at TOAST_MAX - 2.
                let layout_max = crate::widgets::TOAST_MAX - 2.0;
                assert!(
                    (toast_h - layout_max).abs() <= 0.5,
                    "{name}: toast bar {toast_h} should sit at the cap {layout_max} at page {page}"
                );
            }
        }
    }
}

/// When the toast overflows its four lines, the scrollbar sits in a
/// gutter: no glyph of the text is drawn under the bar. Measured at
/// 400 px, the width where this toast overflows the cap.
#[test]
fn the_overflowing_toast_keeps_its_text_clear_of_the_scrollbar() {
    use crate::layout::SCROLLBAR_WIDTH;
    use iced::advanced::Layout;
    use iced::advanced::layout::Limits;
    use iced::advanced::widget::operation::{Operation, Scrollable};

    struct Found {
        scroll: Rectangle,
        content: Rectangle,
        texts: Vec<(Rectangle, String)>,
    }
    #[derive(Default)]
    struct Collect {
        depth: usize,
        scroll: Option<(usize, Rectangle, Rectangle)>,
        texts: Vec<(Rectangle, String)>,
        found: Option<Found>,
    }
    impl Operation<()> for Collect {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            self.depth += 1;
            operate(self);
            self.depth -= 1;
            // Keep the innermost scrollable after leaving its subtree.
            // Clearing `scroll` alone would drop it before the assert.
            if let Some((d, bounds, content)) = self.scroll
                && d > self.depth
            {
                self.found = Some(Found {
                    scroll: bounds,
                    content,
                    texts: std::mem::take(&mut self.texts),
                });
                self.scroll = None;
            }
        }
        fn scrollable(
            &mut self,
            _id: Option<&Id>,
            bounds: Rectangle,
            content: Rectangle,
            _translation: Vector,
            _state: &mut dyn Scrollable,
        ) {
            self.scroll = Some((self.depth, bounds, content));
            self.texts.clear();
        }
        fn text(&mut self, _id: Option<&Id>, bounds: Rectangle, text: &str) {
            if self.scroll.is_some() {
                self.texts.push((bounds, text.to_string()));
            }
        }
    }

    let page = 400.0;
    let renderer = headless();
    let mut bar = crate::widgets::toast_bar(WRAPPED_TOAST, None);
    let mut tree = Tree::new(&bar);
    let node = bar.as_widget_mut().layout(
        &mut tree,
        &renderer,
        &Limits::new(Size::ZERO, Size::new(page, 400.0)),
    );
    let mut collect = Collect::default();
    bar.as_widget_mut()
        .operate(&mut tree, Layout::new(&node), &renderer, &mut collect);
    let found = collect.found.take();
    let Found {
        scroll,
        content,
        texts,
    } = found
        .or_else(|| {
            collect.scroll.map(|(_, bounds, content)| Found {
                scroll: bounds,
                content,
                texts: collect.texts.clone(),
            })
        })
        .expect("the overflowing toast scrolls");
    let overflows = content.height > scroll.height + 0.5;
    assert!(
        overflows,
        "the wrapped toast should overflow at {page} px so the bar shows (scroll {scroll:?}, content {content:?})"
    );
    assert!(!texts.is_empty(), "the toast text is in the tree");
    let bar_left = scroll.x + scroll.width - SCROLLBAR_WIDTH;
    for (bounds, text) in &texts {
        let right = bounds.x + bounds.width;
        let detail =
            format!("scroll={scroll:?} content={content:?} text={text:?} bounds={bounds:?}");
        assert!(
            right <= bar_left + 0.5,
            "toast text ends at {right}, under the bar which starts at {bar_left}; {detail}"
        );
    }
}

/// At max scroll the last control in the pane (and the classic Settings
/// form) sits above the reserved inset, with a toast and the restart bar
/// showing and with them hidden. The space below the last control is
/// bounded above so a measured slot cannot eat the whole pane. Dismissing
/// a wrapped toast does not change the page's content height.
#[test]
fn the_last_control_scrolls_clear_of_the_prompts_slot() {
    for size in INSET_WINDOWS {
        for toast in ["Report sent", WRAPPED_TOAST] {
            // Settings on the games fixture is taller than the pane at
            // every inset window, including 1280x860. Overview is not.
            let (_dir, mut tabs) = classic_app(Screen::Settings);
            tabs.nav_style = NavStyle::TopTabs;
            last_control_clears(&mut tabs, whole_window, Target::Pane, size, toast);
            let (_classic_dir, mut classic) = classic_app(Screen::Games);
            last_control_clears(&mut classic, whole_window, Target::Pane, size, toast);
            let (_form_dir, mut form) = classic_app(Screen::Settings);
            last_control_clears(&mut form, whole_window, Target::Form, size, toast);
        }
    }
}

fn last_control_clears(
    app: &mut TrackerApp,
    view: View,
    target: Target,
    size: Size,
    toast: &'static str,
) {
    use iced::advanced::Layout;
    quiet(app);
    let renderer = headless();
    let measure = |app: &TrackerApp, label: &str| {
        let mut element = view(app);
        let mut tree = Tree::new(&element);
        let node = lay_out(&mut element, &mut tree, &renderer, size);
        let found = find(&mut element, &mut tree, &node, &renderer, target)
            .unwrap_or_else(|| panic!("{target:?} is in the tree"));
        let max = found.content.height - found.bounds.height;
        assert!(
            max > 1.0,
            "{label}: {target:?} does not overflow: {found:?}"
        );
        let mut scroll = Scrollables {
            scroll: Some((found.state, max)),
            ..Scrollables::default()
        };
        run(&mut element, &mut tree, &node, &renderer, &mut scroll);
        let node = lay_out(&mut element, &mut tree, &renderer, size);
        let after = find(&mut element, &mut tree, &node, &renderer, target)
            .unwrap_or_else(|| panic!("{target:?} is in the tree after scrolling"));
        let layout = Layout::new(&node);
        let lowest = lowest_inside_scrollable(layout, target, &after);
        // iced lays out the scrollable's content with the viewport's origin,
        // but does not apply the scroll offset in the layout nodes.
        let in_window = if lowest > after.bounds.y + after.bounds.height + 1.0 {
            lowest - after.offset.y
        } else {
            lowest
        };
        let slot_top = size.height - super::PROMPTS_INSET;
        assert!(
            in_window <= slot_top + 0.5,
            "{label}: last control at {in_window} (content {lowest}, offset {}, box {:?}, content-h {}) sits under the slot starting at {slot_top}",
            after.offset.y,
            after.bounds,
            after.content.height
        );
        let below = space_below_last(layout, &after, lowest);
        if target == Target::Pane {
            assert!(
                below <= super::PROMPTS_INSET + 32.0,
                "{label}: space below the last control is {below}, over the inset {}",
                super::PROMPTS_INSET
            );
            assert!(
                below < size.height - 32.0,
                "{label}: space below the last control is {below}, a full pane at height {}",
                size.height
            );
        }
        (after.content.height, below)
    };
    let (quiet_h, _) = measure(app, "quiet");
    app.toast = Some(toast.to_string());
    show_restart_bar(app);
    let (shown_h, _) = measure(app, "toast and restart bar");
    assert_eq!(
        shown_h, quiet_h,
        "showing {toast:?} changed the page content height from {quiet_h} to {shown_h}"
    );
    quiet(app);
    let (cleared_h, _) = measure(app, "cleared");
    assert_eq!(
        cleared_h, shown_h,
        "dismissing {toast:?} shifted the page content height from {shown_h} to {cleared_h}"
    );
}

/// Room between the last control and the bottom of the scrollable's
/// padded content (the reserved inset, plus any trailing spacing).
fn space_below_last(layout: Layout<'_>, found: &Seen, lowest: f32) -> f32 {
    fn is_found(layout: Layout<'_>, found: &Seen) -> bool {
        let bounds = layout.bounds();
        (bounds.y - found.bounds.y).abs() < 1.0
            && (bounds.width - found.bounds.width).abs() < 1.0
            && (bounds.height - found.bounds.height).abs() < 1.0
    }
    fn content_box(layout: Layout<'_>, content_h: f32) -> Option<Layout<'_>> {
        if (layout.bounds().height - content_h).abs() < 1.0 {
            return Some(layout);
        }
        layout
            .children()
            .find_map(|child| content_box(child, content_h))
    }
    fn walk(layout: Layout<'_>, found: &Seen, lowest: f32) -> Option<f32> {
        if is_found(layout, found) {
            let padded = content_box(layout, found.content.height)?;
            return Some((padded.bounds().y + padded.bounds().height) - lowest);
        }
        layout
            .children()
            .find_map(|child| walk(child, found, lowest))
    }
    walk(layout, found, lowest).expect("the scrollable is in the layout")
}

/// The lowest edge of a child inside `target`'s padded content. Layout
/// nodes still include the scroll offset only as state, so the value is
/// in unscrolled window coordinates (the viewport's origin).
fn lowest_inside_scrollable(layout: Layout<'_>, _target: Target, found: &Seen) -> f32 {
    fn is_found(layout: Layout<'_>, found: &Seen) -> bool {
        let bounds = layout.bounds();
        (bounds.y - found.bounds.y).abs() < 1.0
            && (bounds.width - found.bounds.width).abs() < 1.0
            && (bounds.height - found.bounds.height).abs() < 1.0
    }
    /// Bottom of this node or any descendant.
    fn max_y(layout: Layout<'_>) -> f32 {
        let bounds = layout.bounds();
        layout
            .children()
            .map(max_y)
            .fold(bounds.y + bounds.height, f32::max)
    }
    /// The padded content box is as tall as the scrollable's content;
    /// skip clip wrappers to find it, then measure its children so the
    /// reserved bottom padding is not counted as a control.
    fn content_box(layout: Layout<'_>, content_h: f32) -> Option<Layout<'_>> {
        if (layout.bounds().height - content_h).abs() < 1.0 {
            return Some(layout);
        }
        layout
            .children()
            .find_map(|child| content_box(child, content_h))
    }
    fn walk(layout: Layout<'_>, found: &Seen) -> Option<f32> {
        if is_found(layout, found) {
            let padded = content_box(layout, found.content.height)?;
            let bottom = padded
                .children()
                .map(max_y)
                .fold(f32::NEG_INFINITY, f32::max);
            return Some(if bottom.is_finite() {
                bottom
            } else {
                padded.bounds().y
            });
        }
        layout.children().find_map(|child| walk(child, found))
    }
    walk(layout, found).expect("the scrollable is in the layout")
}

/// The status pill's menu and confirm are layers that are always
/// there, so opening or closing either keeps the pane and its scroll.
#[test]
fn the_pill_menu_and_confirm_keep_the_pane_and_its_scroll() {
    use crate::daemon::DaemonVerb;
    use crate::status_pill::Confirm;
    let (_dir, mut app) = app();
    // A live app with the tracker up, so the pill has a menu.
    app.fixture = None;
    app.daemon.pid = Some(4242);
    let closed = pane_path(&app, no_dialog);
    app.pill.menu = Some(0);
    assert!(crate::status_pill::menu_layer(&app).is_some());
    let menu = pane_path(&app, no_dialog);
    app.pill.menu = None;
    app.pill.confirm = Some(Confirm {
        verb: DaemonVerb::Stop,
        on_action: false,
    });
    assert!(crate::status_pill::confirm_layer(&app).is_some());
    let confirm = pane_path(&app, no_dialog);
    app.pill.confirm = None;
    eprintln!("pill closed: {:?}", readable(&closed));
    assert_eq!(
        closed,
        menu,
        "{:?} vs {:?}",
        readable(&closed),
        readable(&menu)
    );
    assert_eq!(
        closed,
        confirm,
        "{:?} vs {:?}",
        readable(&closed),
        readable(&confirm)
    );
    // The pane is already the first stack child, so a pane-only path
    // still matches if the empty closed pill layers are dropped. The
    // dialog sits after those slots; its path stays put only when the
    // empty layers keep their place.
    app.pill.menu = None;
    app.pill.confirm = None;
    let dialog_closed = text_path(&app, a_dialog, "A dialog");
    app.pill.menu = Some(0);
    let dialog_menu = text_path(&app, a_dialog, "A dialog");
    app.pill.menu = None;
    app.pill.confirm = Some(Confirm {
        verb: DaemonVerb::Stop,
        on_action: false,
    });
    let dialog_confirm = text_path(&app, a_dialog, "A dialog");
    app.pill.confirm = None;
    eprintln!("dialog closed: {:?}", readable(&dialog_closed));
    assert_eq!(
        dialog_closed,
        dialog_menu,
        "{:?} vs {:?}",
        readable(&dialog_closed),
        readable(&dialog_menu)
    );
    assert_eq!(
        dialog_closed,
        dialog_confirm,
        "{:?} vs {:?}",
        readable(&dialog_closed),
        readable(&dialog_confirm)
    );
    let kept = offset_after(&mut app, no_dialog, no_dialog, |app| {
        app.pill.menu = Some(0)
    });
    assert!(kept > 0.0, "opening the menu reset the scroll");
    let kept = offset_after(&mut app, no_dialog, no_dialog, |app| app.pill.menu = None);
    assert!(kept > 0.0, "closing the menu reset the scroll");
    let kept = offset_after(&mut app, no_dialog, no_dialog, |app| {
        app.pill.confirm = Some(Confirm {
            verb: DaemonVerb::Restart,
            on_action: false,
        })
    });
    assert!(kept > 0.0, "opening the confirm reset the scroll");
    let kept = offset_after(&mut app, no_dialog, no_dialog, |app| {
        app.pill.confirm = None
    });
    assert!(kept > 0.0, "closing the confirm reset the scroll");
}
