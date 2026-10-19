//! One `<style>` node per id in `document.head`.
//!
//! A `<style>` element rendered inside a page is removed when that page
//! unmounts and inserted again on the next visit. These helpers keep a single
//! head node and reuse it.

use dioxus::prelude::*;

/// What to do with the style node that already has `id`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum StyleWrite {
    Create,
    Update,
    Keep,
}

pub(crate) fn style_write(exists: bool, current: Option<&str>, next: &str) -> StyleWrite {
    if !exists {
        StyleWrite::Create
    } else if current == Some(next) {
        StyleWrite::Keep
    } else {
        StyleWrite::Update
    }
}

fn upsert_style(id: &str, css: &str) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let existing = document.get_element_by_id(id);
        let current = existing.as_ref().and_then(|element| element.text_content());
        match style_write(existing.is_some(), current.as_deref(), css) {
            StyleWrite::Keep => {}
            StyleWrite::Update => {
                if let Some(element) = existing {
                    element.set_text_content(Some(css));
                }
            }
            StyleWrite::Create => {
                let Ok(element) = document.create_element("style") else {
                    return;
                };
                let _ = element.set_attribute("id", id);
                element.set_text_content(Some(css));
                if let Ok(Some(head)) = document.query_selector("head") {
                    let _ = head.append_child(&element);
                }
            }
        }
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = (id, css);
    }
}

/// Insert static CSS the first time this id is seen. Later mounts do nothing.
#[component]
pub fn StyleOnce(id: &'static str, css: &'static str) -> Element {
    use_hook(move || {
        upsert_style(id, css);
    });
    rsx! {}
}

/// Keep one head node for CSS that changes (settings, homepage skin).
/// An empty string clears the rules without inserting a second tag.
#[component]
pub fn StyleLive(id: &'static str, css: String) -> Element {
    upsert_style(id, &css);
    rsx! {}
}

#[cfg(test)]
mod tests {
    use super::{StyleWrite, style_write};

    #[test]
    fn style_write_creates_once_then_keeps_or_updates() {
        assert_eq!(style_write(false, None, "a"), StyleWrite::Create);
        assert_eq!(style_write(true, Some("a"), "a"), StyleWrite::Keep);
        assert_eq!(style_write(true, Some("a"), "b"), StyleWrite::Update);
        assert_eq!(style_write(true, None, "a"), StyleWrite::Update);
    }
}
