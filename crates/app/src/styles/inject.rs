//! One `<style>` node per id in `document.head`.
//!
//! A `<style>` element rendered inside a page is removed when that page
//! unmounts and inserted again on the next visit. These helpers keep a single
//! head node and reuse it.

use std::cell::RefCell;
use std::collections::HashMap;

use dioxus::prelude::*;

#[derive(Debug, Default, Clone, PartialEq, Eq)]
struct LiveSheet {
    text: String,
    owners: usize,
}

thread_local! {
    static LIVE_SHEETS: RefCell<HashMap<&'static str, LiveSheet>> =
        RefCell::new(HashMap::new());
}

/// One owner for this id. Call once per `StyleLive` instance, not every render.
fn acquire_live_owner(id: &'static str) {
    LIVE_SHEETS.with(|sheets| {
        let mut sheets = sheets.borrow_mut();
        let sheet = sheets.entry(id).or_default();
        sheet.owners += 1;
    });
}

fn publish_live_css(id: &'static str, css: &str) {
    LIVE_SHEETS.with(|sheets| {
        let mut sheets = sheets.borrow_mut();
        if let Some(sheet) = sheets.get_mut(id)
            && sheet.text != css
        {
            sheet.text = css.to_string();
        }
    });
}

/// `true` when this was the last owner and the node should be cleared.
fn release_live_owner(id: &'static str) -> bool {
    LIVE_SHEETS.with(|sheets| {
        let mut sheets = sheets.borrow_mut();
        let Some(sheet) = sheets.get_mut(id) else {
            return true;
        };
        sheet.owners = sheet.owners.saturating_sub(1);
        if sheet.owners == 0 {
            sheets.remove(id);
            true
        } else {
            false
        }
    })
}

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

fn clear_style(id: &str) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        if let Some(element) = document.get_element_by_id(id) {
            element.set_text_content(Some(""));
        }
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = id;
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
/// The node is cleared when the last owner drops, so a public page background
/// does not stay on admin or strategy. Home renders two owners of
/// `scuffed-css-home` across its pending and ready templates. Dioxus 0.7.9
/// mounts the new one before dropping the old, and the new text matches, so
/// the old drop must not wipe the sheet.
#[component]
pub fn StyleLive(id: &'static str, css: String) -> Element {
    use_hook(|| acquire_live_owner(id));
    publish_live_css(id, &css);
    upsert_style(id, &css);
    use_drop(move || {
        if release_live_owner(id) {
            clear_style(id);
        }
    });
    rsx! {}
}

#[cfg(test)]
fn live_sheet_text(id: &'static str) -> Option<String> {
    LIVE_SHEETS.with(|sheets| sheets.borrow().get(id).map(|sheet| sheet.text.clone()))
}

#[cfg(test)]
mod tests {
    use super::{
        StyleWrite, acquire_live_owner, live_sheet_text, publish_live_css, release_live_owner,
        style_write,
    };

    #[test]
    fn style_write_creates_once_then_keeps_or_updates() {
        assert_eq!(style_write(false, None, "a"), StyleWrite::Create);
        assert_eq!(style_write(true, Some("a"), "a"), StyleWrite::Keep);
        assert_eq!(style_write(true, Some("a"), "b"), StyleWrite::Update);
        assert_eq!(style_write(true, None, "a"), StyleWrite::Update);
        // The last owner writes an empty sheet. The next mount must replace it.
        assert_eq!(style_write(true, Some(""), "a"), StyleWrite::Update);
    }

    #[test]
    fn pending_ready_swap_keeps_the_home_sheet() {
        let id = "scuffed-css-home-swap-test";
        let css = "home-layers{}";
        // Pending owner, then Ready. Dioxus 0.7.9 replace_inner creates the
        // new subtree before the old use_drop runs. The text matches, so a
        // clear from the old owner would leave home unstyled.
        acquire_live_owner(id);
        publish_live_css(id, css);
        acquire_live_owner(id);
        publish_live_css(id, css);
        assert!(
            !release_live_owner(id),
            "the pending owner must not clear while ready is mounted"
        );
        let text = live_sheet_text(id).expect("sheet still owned");
        assert_eq!(text, css);
        assert!(!text.is_empty());
        assert!(release_live_owner(id));
        assert!(live_sheet_text(id).is_none());
    }
}
