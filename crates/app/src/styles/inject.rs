//! One `<style>` node per id in `document.head`.
//!
//! A `<style>` element rendered inside a page is removed when that page
//! unmounts and inserted again on the next visit. These helpers keep a single
//! head node and reuse it.

use std::cell::RefCell;
use std::collections::HashMap;

use dioxus::prelude::*;

thread_local! {
    static LIVE_OWNERS: RefCell<HashMap<&'static str, usize>> = RefCell::new(HashMap::new());
}

#[cfg(test)]
thread_local! {
    static STYLE_CLEARS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
    static STYLE_ACQUIRES: std::cell::Cell<u32> = const { std::cell::Cell::new(0) };
}

/// One owner for this id. Call once per `StyleLive` instance, not every render.
fn acquire_live_owner(id: &'static str) {
    #[cfg(test)]
    STYLE_ACQUIRES.with(|count| count.set(count.get() + 1));
    LIVE_OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        *owners.entry(id).or_insert(0) += 1;
    });
}

/// `true` when this was the last owner and the node should be cleared.
fn release_live_owner(id: &'static str) -> bool {
    LIVE_OWNERS.with(|owners| {
        let mut owners = owners.borrow_mut();
        let Some(count) = owners.get_mut(id) else {
            return true;
        };
        *count = count.saturating_sub(1);
        if *count == 0 {
            owners.remove(id);
            true
        } else {
            false
        }
    })
}

#[cfg(test)]
fn live_owner_count(id: &str) -> usize {
    LIVE_OWNERS.with(|owners| owners.borrow().get(id).copied().unwrap_or(0))
}

#[cfg(test)]
fn style_acquire_count() -> u32 {
    STYLE_ACQUIRES.with(|count| count.get())
}

#[cfg(test)]
fn reset_style_acquires() {
    STYLE_ACQUIRES.with(|count| count.set(0));
}

#[cfg(test)]
fn take_style_clears() -> Vec<String> {
    STYLE_CLEARS.with(|clears| std::mem::take(&mut *clears.borrow_mut()))
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
    #[cfg(test)]
    STYLE_CLEARS.with(|clears| clears.borrow_mut().push(id.to_string()));
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
    upsert_style(id, &css);
    use_drop(move || {
        if release_live_owner(id) {
            clear_style(id);
        }
    });
    rsx! {}
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use dioxus::dioxus_core::NoOpMutations;
    use dioxus::prelude::*;

    use super::{
        StyleLive, StyleWrite, live_owner_count, reset_style_acquires, style_acquire_count,
        style_write, take_style_clears,
    };

    const SWAP_STYLE_ID: &str = "scuffed-css-home-swap-test";

    #[derive(Clone, Copy)]
    struct SwapHandles {
        ready: Signal<bool>,
        alive: Signal<bool>,
    }

    thread_local! {
        static SWAP_HANDLES: Cell<Option<SwapHandles>> = const { Cell::new(None) };
    }

    fn style_live_swap_probe() -> Element {
        let ready = use_signal(|| false);
        let alive = use_signal(|| true);
        use_hook(move || {
            SWAP_HANDLES.with(|slot| slot.set(Some(SwapHandles { ready, alive })));
        });
        if !alive() {
            return rsx! {};
        }
        if ready() {
            rsx! {
                div { class: "ready",
                    StyleLive { id: SWAP_STYLE_ID, css: "home-layers{}".to_string() }
                }
            }
        } else {
            rsx! {
                div { class: "pending",
                    StyleLive { id: SWAP_STYLE_ID, css: "home-layers{}".to_string() }
                }
            }
        }
    }

    fn pump(dom: &mut VirtualDom) {
        for _ in 0..4 {
            dom.render_immediate(&mut NoOpMutations);
        }
    }

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
        let _reset_clears = take_style_clears();
        reset_style_acquires();
        let mut dom = VirtualDom::new(style_live_swap_probe);
        dom.rebuild_in_place();
        pump(&mut dom);
        let pending = dioxus_ssr::render(&dom);
        assert!(pending.contains("pending"), "{pending}");
        assert_eq!(live_owner_count(SWAP_STYLE_ID), 1);
        assert_eq!(style_acquire_count(), 1);
        assert!(take_style_clears().is_empty());

        // Ready replaces pending. Dioxus 0.7.9 mounts the new StyleLive before
        // the old use_drop. The text matches, so that drop must not clear.
        dom.in_runtime(|| {
            let mut ready = SWAP_HANDLES
                .with(|slot| slot.get().expect("swap probe mounted"))
                .ready;
            ready.set(true);
        });
        pump(&mut dom);
        let ready_html = dioxus_ssr::render(&dom);
        assert!(ready_html.contains("ready"), "{ready_html}");
        assert!(!ready_html.contains("pending"), "{ready_html}");
        assert_eq!(
            style_acquire_count(),
            2,
            "ready must mount its own StyleLive"
        );
        assert_eq!(live_owner_count(SWAP_STYLE_ID), 1);
        assert!(
            take_style_clears().is_empty(),
            "the pending drop must not clear while ready is mounted"
        );

        dom.in_runtime(|| {
            let mut alive = SWAP_HANDLES
                .with(|slot| slot.get().expect("swap probe mounted"))
                .alive;
            alive.set(false);
        });
        pump(&mut dom);
        assert_eq!(live_owner_count(SWAP_STYLE_ID), 0);
        assert_eq!(take_style_clears(), vec![SWAP_STYLE_ID.to_string()]);
        SWAP_HANDLES.with(|slot| slot.set(None));
    }
}
