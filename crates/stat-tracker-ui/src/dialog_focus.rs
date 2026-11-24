//! Keyboard focus while a dialog is open.
//!
//! iced's `opaque` only stops mouse presses. Keyboard events go to every
//! layer of a `Stack`, so a page field that still had focus took the keys
//! typed into a dialog over it, and Enter in it saved Settings. Every layer
//! of the window ([`crate::chrome::with_layers`]) is therefore a [`Layer`]
//! in one of three roles ([`Role`]):
//!
//! - [`Role::Live`]: the page with no dialog over it. Everything passes.
//! - [`Role::Covered`]: the page, or a dialog, under the top dialog. It gets
//!   no keyboard or input method events. When it becomes covered it notes
//!   which of its focusable widgets had focus and lets go of it; when it is
//!   the live page or the top dialog again it gives that focus back. Focus
//!   operations ([`iced::widget::operation`]) do not see inside it, so a
//!   `focus` or Tab cycle can never land on a hidden field.
//! - [`Role::Top`]: the dialog on top. On its first event it focuses its
//!   first focusable widget, or itself when it has none (the layer reports
//!   itself as a focusable widget with [`DIALOG_ID`] while it holds focus).
//!   If it had nothing to focus and its content then gets something (the
//!   crash prompt turning into the report form), that gets the focus.
//!   Tab and Shift+Tab cycle its focusable widgets only. With an
//!   `on_escape` message the layer sends it on Esc and keeps the key;
//!   without one (the setup guide) Esc goes to its widgets, where a field
//!   may take it to let go of the focus, and if none does, on to the app's
//!   key listener, as before.
//!
//! Esc closes the top dialog; the layer under it is no longer covered, so
//! on its next event the field or control that had focus before the dialog
//! opened has it again. The roles are a flag on one widget type, so a layer
//! changing role keeps its place and state in the widget tree (the page
//! keeps its scroll).

use iced::advanced::layout::{self, Layout};
use iced::advanced::widget::operation::{Focusable, Operation, Scrollable, TextInput};
use iced::advanced::widget::{Id, Tree, Widget, tree};
use iced::advanced::{Clipboard, Shell, overlay, renderer};
use iced::keyboard::{self, key::Named};
use iced::{Element, Event, Length, Rectangle, Size, Vector, mouse};

use crate::app::Message;

/// The id a top dialog with focus on itself reports to focus operations.
pub(crate) const DIALOG_ID: &str = "dialog-layer";

/// What a layer of the window is doing now.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Role {
    /// The page, with no dialog open.
    Live,
    /// Under the top dialog: no keys, no focus.
    Covered,
    /// The top dialog: it takes focus and keeps Tab to itself.
    Top,
}

/// An open dialog: what it draws and the message Esc sends, if the layer
/// handles Esc itself.
pub(crate) struct Dialog<'a> {
    pub content: Element<'a, Message>,
    pub on_escape: Option<Message>,
}

impl<'a> Dialog<'a> {
    pub(crate) fn new(
        content: impl Into<Element<'a, Message>>,
        on_escape: Option<Message>,
    ) -> Self {
        Self {
            content: content.into(),
            on_escape,
        }
    }
}

/// One layer of the window in `role` (see the module docs).
pub(crate) fn layer<'a>(
    content: impl Into<Element<'a, Message>>,
    role: Role,
    on_escape: Option<Message>,
) -> Element<'a, Message> {
    Element::new(Layer {
        content: content.into(),
        role,
        on_escape,
    })
}

struct Layer<'a> {
    content: Element<'a, Message>,
    role: Role,
    on_escape: Option<Message>,
}

/// The focus a layer gives back when it is no longer covered.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
enum Held {
    #[default]
    Nothing,
    /// The `index`th of `total` focusable widgets.
    Child { index: usize, total: usize },
    /// The layer itself.
    Itself,
}

#[derive(Debug, Clone, Copy, Default)]
struct State {
    /// The role was [`Role::Covered`] at the last event.
    covered: bool,
    held: Held,
    /// A top dialog has given out its first focus.
    opened: bool,
    /// The layer itself has focus.
    itself: bool,
    /// How many focusable widgets a top dialog had at the last event.
    shown: usize,
}

/// Counts the focusable widgets and finds the focused one.
#[derive(Default)]
struct Census {
    total: usize,
    focused: Option<usize>,
}

impl Operation for Census {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if state.is_focused() {
            self.focused = Some(self.total);
        }
        self.total += 1;
    }
}

/// Focuses the `target`th focusable widget and unfocuses the rest; with
/// `None`, unfocuses them all.
struct FocusAt {
    target: Option<usize>,
    current: usize,
}

impl Operation for FocusAt {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        operate(self);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, state: &mut dyn Focusable) {
        if self.target == Some(self.current) {
            state.focus();
        } else {
            state.unfocus();
        }
        self.current += 1;
    }
}

/// Passes an operation on, without the focusable widgets and text inputs.
struct Hidden<'a>(&'a mut dyn Operation);

impl Operation for Hidden<'_> {
    fn traverse(&mut self, operate: &mut dyn FnMut(&mut dyn Operation)) {
        self.0.traverse(&mut |inner| operate(&mut Hidden(inner)));
    }

    fn container(&mut self, id: Option<&Id>, bounds: Rectangle) {
        self.0.container(id, bounds);
    }

    fn scrollable(
        &mut self,
        id: Option<&Id>,
        bounds: Rectangle,
        content_bounds: Rectangle,
        translation: Vector,
        state: &mut dyn Scrollable,
    ) {
        self.0
            .scrollable(id, bounds, content_bounds, translation, state);
    }

    fn text(&mut self, id: Option<&Id>, bounds: Rectangle, text: &str) {
        self.0.text(id, bounds, text);
    }

    fn custom(&mut self, id: Option<&Id>, bounds: Rectangle, state: &mut dyn std::any::Any) {
        self.0.custom(id, bounds, state);
    }

    fn focusable(&mut self, _id: Option<&Id>, _bounds: Rectangle, _state: &mut dyn Focusable) {}

    fn text_input(&mut self, _id: Option<&Id>, _bounds: Rectangle, _state: &mut dyn TextInput) {}
}

/// The layer itself as a focusable widget.
struct Itself<'a>(&'a mut bool);

impl Focusable for Itself<'_> {
    fn is_focused(&self) -> bool {
        *self.0
    }

    fn focus(&mut self) {
        *self.0 = true;
    }

    fn unfocus(&mut self) {
        *self.0 = false;
    }
}

impl Layer<'_> {
    fn run(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        if let Some(content) = layout.children().next() {
            self.content.as_widget_mut().operate(
                &mut tree.children[0],
                content,
                renderer,
                operation,
            );
        }
    }

    fn census(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer) -> Census {
        let mut census = Census::default();
        self.run(tree, layout, renderer, &mut census);
        census
    }

    fn focus_at(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        target: Option<usize>,
    ) {
        let mut focus = FocusAt { target, current: 0 };
        self.run(tree, layout, renderer, &mut focus);
    }

    /// Brings the focus in line with the role: let go of it when covered,
    /// give it back when not, and take the first focus once on top.
    /// Returns whether the focus moved.
    fn settle(&mut self, tree: &mut Tree, layout: Layout<'_>, renderer: &iced::Renderer) -> bool {
        let state = *tree.state.downcast_ref::<State>();
        let mut next = state;
        let mut moved = false;
        if self.role == Role::Covered && !state.covered {
            let census = self.census(tree, layout, renderer);
            next.held = match census.focused {
                Some(index) => Held::Child {
                    index,
                    total: census.total,
                },
                None if state.itself => Held::Itself,
                None => Held::Nothing,
            };
            if census.focused.is_some() {
                self.focus_at(tree, layout, renderer, None);
            }
            next.itself = false;
            next.covered = true;
            moved = true;
        } else if self.role != Role::Covered && state.covered {
            match state.held {
                Held::Child { index, total }
                    if self.census(tree, layout, renderer).total == total =>
                {
                    self.focus_at(tree, layout, renderer, Some(index));
                }
                Held::Itself => next.itself = true,
                _ => {}
            }
            next.held = Held::Nothing;
            next.covered = false;
            moved = true;
        }
        if self.role == Role::Top {
            let census = self.census(tree, layout, renderer);
            // On open, and when a dialog with nothing to focus gets some
            // (the crash prompt turning into the report form in the same
            // layer): its first focusable widget, or the dialog itself.
            let fresh = !next.opened || (next.shown == 0 && census.total > 0);
            if fresh && census.focused.is_none() {
                next.itself = census.total == 0;
                if !next.itself {
                    self.focus_at(tree, layout, renderer, Some(0));
                }
                moved = true;
            } else if census.focused.is_some() {
                // A click put a field in focus.
                next.itself = false;
            } else if census.total == 0 && !next.itself {
                // Its focused widget went away: the dialog keeps the focus.
                next.itself = true;
            }
            next.opened = true;
            next.shown = census.total;
        }
        *tree.state.downcast_mut::<State>() = next;
        moved
    }

    /// Tab or Shift+Tab (`back`) on the top dialog: the next focusable
    /// widget in it, round to the first, or the dialog itself if it has none.
    fn cycle(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        back: bool,
    ) {
        let census = self.census(tree, layout, renderer);
        let itself = census.total == 0;
        if !itself {
            let last = census.total - 1;
            let next = match (census.focused, back) {
                (None, false) => 0,
                (None, true) => last,
                (Some(i), false) => (i + 1) % census.total,
                (Some(i), true) => i.checked_sub(1).unwrap_or(last),
            };
            self.focus_at(tree, layout, renderer, Some(next));
        }
        tree.state.downcast_mut::<State>().itself = itself;
    }
}

impl Widget<Message, iced::Theme, iced::Renderer> for Layer<'_> {
    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn size_hint(&self) -> Size<Length> {
        self.content.as_widget().size_hint()
    }

    fn tag(&self) -> tree::Tag {
        tree::Tag::of::<State>()
    }

    fn state(&self) -> tree::State {
        tree::State::new(State::default())
    }

    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(std::slice::from_ref(&self.content));
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &layout::Limits,
    ) -> layout::Node {
        let content = self
            .content
            .as_widget_mut()
            .layout(&mut tree.children[0], renderer, limits);
        layout::Node::with_children(content.size(), vec![content])
    }

    fn draw(
        &self,
        tree: &Tree,
        renderer: &mut iced::Renderer,
        theme: &iced::Theme,
        style: &renderer::Style,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
    ) {
        if let Some(content) = layout.children().next() {
            self.content.as_widget().draw(
                &tree.children[0],
                renderer,
                theme,
                style,
                content,
                cursor,
                viewport,
            );
        }
    }

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn Operation,
    ) {
        match self.role {
            Role::Live => self.run(tree, layout, renderer, operation),
            Role::Covered => self.run(tree, layout, renderer, &mut Hidden(operation)),
            Role::Top => {
                let bounds = layout.bounds();
                operation.container(Some(&Id::new(DIALOG_ID)), bounds);
                operation.traverse(&mut |operation| {
                    let state = tree.state.downcast_mut::<State>();
                    if state.itself {
                        let id = Id::new(DIALOG_ID);
                        operation.focusable(Some(&id), bounds, &mut Itself(&mut state.itself));
                    }
                    self.run(tree, layout, renderer, operation);
                });
            }
        }
    }

    fn update(
        &mut self,
        tree: &mut Tree,
        event: &Event,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        renderer: &iced::Renderer,
        clipboard: &mut dyn Clipboard,
        shell: &mut Shell<'_, Message>,
        viewport: &Rectangle,
    ) {
        if self.settle(tree, layout, renderer) {
            shell.request_redraw();
        }
        match (self.role, event) {
            // Modifier changes still go through, so the page does not keep
            // a Shift that was let go while it was covered.
            (Role::Covered, Event::Keyboard(keyboard::Event::ModifiersChanged(_))) => {}
            (Role::Covered, Event::Keyboard(_) | Event::InputMethod(_)) => return,
            (
                Role::Top,
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(Named::Tab),
                    modifiers,
                    ..
                }),
            ) if !(modifiers.control() || modifiers.alt() || modifiers.logo()) => {
                self.cycle(tree, layout, renderer, modifiers.shift());
                shell.request_redraw();
                shell.capture_event();
                return;
            }
            (
                Role::Top,
                Event::Keyboard(keyboard::Event::KeyPressed {
                    key: keyboard::Key::Named(Named::Escape),
                    repeat,
                    ..
                }),
            ) if self.on_escape.is_some() => {
                // A held Esc closes once.
                if let Some(message) = self.on_escape.clone().filter(|_| !repeat) {
                    shell.publish(message);
                }
                shell.capture_event();
                return;
            }
            _ => {}
        }
        if let Some(content) = layout.children().next() {
            self.content.as_widget_mut().update(
                &mut tree.children[0],
                event,
                content,
                cursor,
                renderer,
                clipboard,
                shell,
                viewport,
            );
        }
    }

    fn mouse_interaction(
        &self,
        tree: &Tree,
        layout: Layout<'_>,
        cursor: mouse::Cursor,
        viewport: &Rectangle,
        renderer: &iced::Renderer,
    ) -> mouse::Interaction {
        layout
            .children()
            .next()
            .map_or(mouse::Interaction::None, |content| {
                self.content.as_widget().mouse_interaction(
                    &tree.children[0],
                    content,
                    cursor,
                    viewport,
                    renderer,
                )
            })
    }

    fn overlay<'b>(
        &'b mut self,
        tree: &'b mut Tree,
        layout: Layout<'b>,
        renderer: &iced::Renderer,
        viewport: &Rectangle,
        translation: Vector,
    ) -> Option<overlay::Element<'b, Message, iced::Theme, iced::Renderer>> {
        // A covered layer's menus and tooltips would still take keys.
        if self.role == Role::Covered {
            return None;
        }
        let content = layout.children().next()?;
        self.content.as_widget_mut().overlay(
            &mut tree.children[0],
            content,
            renderer,
            viewport,
            translation,
        )
    }
}
