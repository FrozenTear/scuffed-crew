//! The page scroll must survive a toast or a dialog coming and going.
//! iced keeps a widget's state (the pane's scroll offset) only while
//! the widget sits at the same place in the tree with the same tag, so these
//! tests walk the laid-out tree from the root to the page pane
//! ([`crate::focus_status::PANE_SCROLL_ID`]) and compare that path.

use super::*;
use iced::advanced::Layout;
use iced::advanced::layout::{Limits, Node};
use iced::advanced::widget::operation::scrollable::{AbsoluteOffset, scroll_to};
use iced::advanced::widget::operation::{self, Operation};
use iced::advanced::widget::{Id, Tree, tree};
use iced::widget::{scrollable, text};
use iced::{Rectangle, Size, Vector};

use crate::focus_status::PANE_SCROLL_ID;

/// A short window, so the demo Overview is taller than the pane.
const WINDOW: Size = Size::new(1280.0, 300.0);

type View = for<'a> fn(&'a TrackerApp) -> Element<'a, Message>;

fn headless() -> iced::Renderer {
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

/// The whole window with one dialog open, or none.
fn window_with(app: &TrackerApp, dialog: bool) -> Element<'_, Message> {
    let layer = dialog.then(|| container(text("A dialog")).into());
    with_layers(app, [layer])
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
    let layer = container(text("A dialog")).into();
    with_layers(app, [None, None, Some(layer)])
}

/// The page and one dialog, with no empty slots after the dialog.
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

fn show_restart_bar(app: &mut TrackerApp) {
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
    watch.on_probe(
        probe,
        Ok("other-build-id
"
        .into()),
    );
    assert!(
        watch.prompt_pending(),
        "a different installed binary should show the bar"
    );
    app.restart_watch = watch;
    app.update_progress = crate::update::UpdateProgress::Idle;
    assert!(
        app.restart_bar_visible(),
        "the restart bar should be on for this test"
    );
}

fn lay_out(element: &mut Element<'_, Message>, tree: &mut Tree, renderer: &iced::Renderer) -> Node {
    element
        .as_widget_mut()
        .layout(tree, renderer, &Limits::new(Size::ZERO, WINDOW))
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

/// Finds the page pane: the address of its state in the tree, and how far
/// it is scrolled.
#[derive(Default)]
struct FindPane {
    state: Option<usize>,
    offset: Option<Vector>,
}

impl Operation<()> for FindPane {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
        operate(self);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        _bounds: Rectangle,
        _content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn operation::Scrollable,
    ) {
        if id == Some(&Id::new(PANE_SCROLL_ID)) {
            self.state = Some(state as *mut dyn operation::Scrollable as *const () as usize);
            self.offset = Some(translation);
        }
    }
}

fn find_pane(
    element: &mut Element<'_, Message>,
    tree: &mut Tree,
    node: &Node,
    renderer: &iced::Renderer,
) -> FindPane {
    let mut find = FindPane::default();
    run(element, tree, node, renderer, &mut find);
    find
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

/// The path of widget tags from the root to the page pane.
fn pane_path(app: &TrackerApp, view: View) -> Vec<(usize, tree::Tag)> {
    let renderer = headless();
    let mut element = view(app);
    let mut tree = Tree::new(&element);
    let node = lay_out(&mut element, &mut tree, &renderer);
    let pane = find_pane(&mut element, &mut tree, &node, &renderer)
        .state
        .expect("the page pane is in the tree");
    let mut path = vec![(0, tree.tag)];
    assert!(
        path_to(&tree, pane, &mut path),
        "the pane state is in the tree"
    );
    path
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

/// The pane's scroll offset after scrolling it down in `view`, then calling
/// `change` on the app and diffing the tree against `next`, as iced does.
fn offset_after(
    app: &mut TrackerApp,
    view: View,
    next: View,
    change: impl FnOnce(&mut TrackerApp),
) -> f32 {
    let renderer = headless();
    let mut tree;
    {
        let mut element = view(app);
        tree = Tree::new(&element);
        let node = lay_out(&mut element, &mut tree, &renderer);
        let mut scroll = scroll_to::<()>(
            Id::new(PANE_SCROLL_ID),
            AbsoluteOffset {
                x: None,
                y: Some(120.0),
            },
        );
        run(&mut element, &mut tree, &node, &renderer, &mut scroll);
        let node = lay_out(&mut element, &mut tree, &renderer);
        let before = find_pane(&mut element, &mut tree, &node, &renderer).offset;
        let before = before.expect("the pane is there").y;
        assert!(before > 0.0, "the page is taller than the pane: {before}");
    }
    change(app);
    let mut element = next(app);
    tree.diff(&element);
    let node = lay_out(&mut element, &mut tree, &renderer);
    let after = find_pane(&mut element, &mut tree, &node, &renderer).offset;
    after.expect("the pane is there").y
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
    app.restart_watch = crate::restart_prompt::RestartWatch::new("own-build-id", None);
    assert!(!app.restart_bar_visible());
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

/// What clicking at each point of a grid over the window publishes.
fn clicks(app: &TrackerApp, view: View) -> Vec<String> {
    use iced::advanced::{Shell, clipboard};
    use iced::mouse::{Button, Cursor, Event as Mouse};
    let renderer = headless();
    let mut element = view(app);
    let mut tree = Tree::new(&element);
    let node = lay_out(&mut element, &mut tree, &renderer);
    let viewport = Rectangle::new(iced::Point::ORIGIN, WINDOW);
    let mut published = Vec::new();
    for y in (4..300).step_by(16) {
        for x in (4..1280).step_by(16) {
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

/// The empty layers take no clicks: every click on the page does what it
/// does with nothing over the page.
#[test]
fn the_empty_layers_let_clicks_through() {
    let (_dir, app) = app();
    let bare = clicks(&app, bare_window);
    assert!(!bare.is_empty(), "the grid hits something on the page");
    assert_eq!(clicks(&app, current_top_tabs), bare);
    assert_eq!(clicks(&app, no_dialog), bare);
}

/// Trailing empty slots over an open dialog do not steal clicks from
/// the dialog or the page: the published clicks match a stack that has
/// only that dialog.
#[test]
fn an_open_dialog_with_empty_slots_keeps_its_clicks() {
    let (_dir, app) = app();
    let only = clicks(&app, dialog_only);
    assert!(!only.is_empty(), "the grid hits the page or the dialog");
    assert_eq!(clicks(&app, a_dialog), only);
    assert_eq!(clicks(&app, dialog_with_empty_slots_above), only);
}

/// Clicks beside a toast or the restart bar still reach the page, and
/// the production prompts slot matches a stack of just the window and
/// those prompts.
#[test]
fn a_toast_or_restart_bar_keeps_page_clicks() {
    let (_dir, mut app) = app();
    let bare = clicks(&app, bare_window);
    assert!(!bare.is_empty(), "the grid hits something on the page");

    show_toast(&mut app);
    let toast = clicks(&app, current_top_tabs);
    assert_eq!(toast, clicks(&app, stacked_prompts));
    assert!(
        shares_page_clicks(&bare, &toast),
        "clicks beside the toast should still reach the page"
    );

    app.toast = None;
    show_restart_bar(&mut app);
    let bar = clicks(&app, current_top_tabs);
    assert_eq!(bar, clicks(&app, stacked_prompts));
    assert!(
        shares_page_clicks(&bare, &bar),
        "clicks beside the restart bar should still reach the page"
    );

    show_toast(&mut app);
    let both = clicks(&app, current_top_tabs);
    assert_eq!(both, clicks(&app, stacked_prompts));
    assert!(
        shares_page_clicks(&bare, &both),
        "clicks beside the toast and the restart bar should still reach the page"
    );
}

/// Some grid points publish the same message with the overlay as they
/// do on the bare page, so the overlay has not swallowed the whole window.
fn shares_page_clicks(bare: &[String], over: &[String]) -> bool {
    let over: std::collections::HashSet<&String> = over.iter().collect();
    bare.iter().any(|click| over.contains(click))
}

/// A focused field on the page keeps that focus when a dialog opens, now
/// the page is not remounted. This records the current behaviour; it does
/// not change focus. QA should still Tab and type with a Games edit field
/// focused and a dialog open.
#[test]
fn a_dialog_leaves_page_focus_in_place() {
    use iced::advanced::widget::operation::focusable::{self, Focusable};
    use iced::widget::text_input;

    const FIELD: &str = "page-field";

    fn field_stack(dialog: bool) -> Element<'static, Message> {
        let field = text_input("Name", "held").id(Id::new(FIELD)).into();
        let dialog_layer = if dialog {
            container(text("A dialog")).into()
        } else {
            empty_layer()
        };
        Stack::with_children([field, empty_layer(), dialog_layer, empty_layer()])
            .width(Fill)
            .height(Fill)
            .into()
    }

    fn focused(
        element: &mut Element<'_, Message>,
        tree: &mut Tree,
        node: &Node,
        renderer: &iced::Renderer,
    ) -> bool {
        let mut find = FocusOf {
            target: Id::new(FIELD),
            focused: None,
        };
        element
            .as_widget_mut()
            .operate(tree, Layout::new(node), renderer, &mut find);
        find.focused == Some(true)
    }

    struct FocusOf {
        target: Id,
        focused: Option<bool>,
    }

    impl Operation<()> for FocusOf {
        fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation<()>)) {
            operate(self);
        }

        fn focusable(&mut self, id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
            if id == Some(&self.target) {
                self.focused = Some(state.is_focused());
            }
        }
    }

    let renderer = headless();
    let mut element = field_stack(false);
    let mut tree = Tree::new(&element);
    let node = lay_out(&mut element, &mut tree, &renderer);
    let mut focus = focusable::focus::<()>(Id::new(FIELD));
    run(&mut element, &mut tree, &node, &renderer, &mut focus);
    assert!(
        focused(&mut element, &mut tree, &node, &renderer),
        "the field should be focused before the dialog opens"
    );

    let mut next = field_stack(true);
    tree.diff(&next);
    let node = lay_out(&mut next, &mut tree, &renderer);
    assert!(
        focused(&mut next, &mut tree, &node, &renderer),
        "opening the dialog should not remount the page field or drop its focus"
    );
}
