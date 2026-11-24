//! Bottom padding for the page so the last control can scroll clear of
//! the floating toast and restart bar. The reserved slot is at least
//! [`super::PROMPTS_INSET`], and grows to the laid-out height of the
//! visible prompts when a toast wraps taller than one line.

use iced::advanced::layout::{self, Layout, Limits};
use iced::advanced::overlay;
use iced::advanced::renderer;
use iced::advanced::widget::{Tree, Widget};
use iced::advanced::{Clipboard, Shell};
use iced::{Element, Event, Length, Padding, Rectangle, Size, Vector, mouse};

use crate::app::Message;

use super::PROMPTS_INSET;

/// Wraps `content` and adds bottom padding of at least [`PROMPTS_INSET`],
/// or the laid-out height of `slot` (the visible prompts plus the bottom
/// gap) when that is taller. `slot` is measured only; it is not drawn.
pub(super) fn clearance<'a>(
    content: impl Into<Element<'a, Message>>,
    slot: Element<'a, Message>,
) -> Element<'a, Message> {
    Element::new(Clearance {
        content: content.into(),
        slot,
    })
}

struct Clearance<'a> {
    content: Element<'a, Message>,
    slot: Element<'a, Message>,
}

impl Widget<Message, iced::Theme, iced::Renderer> for Clearance<'_> {
    fn children(&self) -> Vec<Tree> {
        vec![Tree::new(&self.content), Tree::new(&self.slot)]
    }

    fn diff(&self, tree: &mut Tree) {
        tree.diff_children(&[&self.content, &self.slot]);
    }

    fn size(&self) -> Size<Length> {
        self.content.as_widget().size()
    }

    fn layout(
        &mut self,
        tree: &mut Tree,
        renderer: &iced::Renderer,
        limits: &Limits,
    ) -> layout::Node {
        let slot_h = self
            .slot
            .as_widget_mut()
            .layout(&mut tree.children[1], renderer, limits)
            .size()
            .height;
        let bottom = slot_h.max(PROMPTS_INSET);
        layout::padded(
            limits,
            Length::Fill,
            self.content.as_widget().size().height,
            Padding {
                bottom,
                ..Padding::ZERO
            },
            |limits| {
                self.content
                    .as_widget_mut()
                    .layout(&mut tree.children[0], renderer, limits)
            },
        )
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
        let Some(content) = layout.children().next() else {
            return;
        };
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

    fn operate(
        &mut self,
        tree: &mut Tree,
        layout: Layout<'_>,
        renderer: &iced::Renderer,
        operation: &mut dyn iced::advanced::widget::Operation,
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
        layout.children().next().and_then(|content| {
            self.content.as_widget_mut().overlay(
                &mut tree.children[0],
                content,
                renderer,
                viewport,
                translation,
            )
        })
    }
}
