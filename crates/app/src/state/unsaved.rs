//! Ask before leaving a page with an unsaved draft.
//!
//! [`use_unsaved_guard`] is on while its `dirty` argument is true. It covers
//! two ways out:
//!
//! - Reload, closing the tab, or typing another address: the browser's own
//!   `beforeunload` prompt.
//! - A click on any link, in the page or in the site chrome: a capture phase
//!   listener on the document runs before the router's own click handler,
//!   asks with [`LEAVE_PROMPT`], and stops the click when the answer is no.
//!
//! The browser's back and forward buttons are not covered: the router has
//! already moved by the time it hears of them.

/// The question a link click asks while a draft is unsaved.
pub const LEAVE_PROMPT: &str = "You have unsaved changes. Leave this page and drop them?";

/// What a click on a link carries, as far as leaving the page goes.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct LinkClick<'a> {
    pub href: Option<&'a str>,
    pub target: Option<&'a str>,
    pub download: bool,
    /// `MouseEvent.button`: 0 is the main button.
    pub button: i16,
    /// Ctrl, Meta, Shift or Alt was held, so the browser opens the link
    /// elsewhere or downloads it.
    pub modified: bool,
}

/// True when following the click unloads the draft: a plain main-button
/// click on a link that leaves this document in this tab. A link to a
/// fragment, a new tab, a new window or a download keeps the page.
pub fn click_leaves_page(click: &LinkClick<'_>) -> bool {
    let Some(href) = click.href.map(str::trim) else {
        return false;
    };
    if href.is_empty() || href.starts_with('#') {
        return false;
    }
    if click.download || click.button != 0 || click.modified {
        return false;
    }
    match click.target.map(str::trim) {
        None | Some("") => true,
        Some(target) => target.eq_ignore_ascii_case("_self"),
    }
}

/// Ask before leaving while `dirty` is true. Call it on every render of the
/// page that owns the draft; the listeners go when `dirty` turns false or
/// the page unmounts.
pub fn use_unsaved_guard(dirty: bool) {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    web::use_guard(dirty);
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    let _ = dirty;
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
mod web {
    use std::cell::RefCell;
    use std::rc::Rc;

    use dioxus::prelude::*;
    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    use super::{LEAVE_PROMPT, LinkClick, click_leaves_page};

    /// The two listeners, removed when this is dropped.
    struct Listeners {
        unload: Closure<dyn FnMut(web_sys::Event)>,
        click: Closure<dyn FnMut(web_sys::MouseEvent)>,
    }

    impl Listeners {
        fn install() -> Option<Self> {
            let window = web_sys::window()?;
            let document = window.document()?;
            let unload = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
                event.prevent_default();
                // Older browsers want a return value as well.
                let _ = js_sys::Reflect::set(&event, &"returnValue".into(), &"".into());
            });
            let click =
                Closure::<dyn FnMut(web_sys::MouseEvent)>::new(|event: web_sys::MouseEvent| {
                    let Some(element) = event
                        .target()
                        .and_then(|t| t.dyn_into::<web_sys::Element>().ok())
                    else {
                        return;
                    };
                    let Ok(Some(link)) = element.closest("a[href]") else {
                        return;
                    };
                    let href = link.get_attribute("href");
                    let target = link.get_attribute("target");
                    let click = LinkClick {
                        href: href.as_deref(),
                        target: target.as_deref(),
                        download: link.has_attribute("download"),
                        button: event.button(),
                        modified: event.ctrl_key()
                            || event.meta_key()
                            || event.shift_key()
                            || event.alt_key(),
                    };
                    if !click_leaves_page(&click) {
                        return;
                    }
                    let leave = web_sys::window()
                        .and_then(|w| w.confirm_with_message(LEAVE_PROMPT).ok())
                        .unwrap_or(true);
                    if !leave {
                        event.prevent_default();
                        event.stop_immediate_propagation();
                    }
                });
            window
                .add_event_listener_with_callback("beforeunload", unload.as_ref().unchecked_ref())
                .ok()?;
            document
                .add_event_listener_with_callback_and_bool(
                    "click",
                    click.as_ref().unchecked_ref(),
                    true,
                )
                .ok()?;
            Some(Self { unload, click })
        }
    }

    impl Drop for Listeners {
        fn drop(&mut self) {
            if let Some(window) = web_sys::window() {
                let _ = window.remove_event_listener_with_callback(
                    "beforeunload",
                    self.unload.as_ref().unchecked_ref(),
                );
                if let Some(document) = window.document() {
                    let _ = document.remove_event_listener_with_callback_and_bool(
                        "click",
                        self.click.as_ref().unchecked_ref(),
                        true,
                    );
                }
            }
        }
    }

    pub(super) fn use_guard(dirty: bool) {
        let slot = use_hook(|| Rc::new(RefCell::new(None::<Listeners>)));
        {
            let mut current = slot.borrow_mut();
            if !dirty {
                *current = None;
            } else if current.is_none() {
                *current = Listeners::install();
            }
        }
        use_drop(move || {
            slot.borrow_mut().take();
        });
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plain(href: &str) -> LinkClick<'_> {
        LinkClick {
            href: Some(href),
            ..LinkClick::default()
        }
    }

    #[test]
    fn a_plain_click_on_a_page_link_leaves() {
        for href in ["/identity", "/admin", "https://example.com/", "?tab=2"] {
            assert!(click_leaves_page(&plain(href)), "{href}");
        }
        let same_tab = LinkClick {
            target: Some("_self"),
            ..plain("/reports")
        };
        assert!(click_leaves_page(&same_tab));
    }

    #[test]
    fn clicks_that_keep_the_page_do_not_ask() {
        assert!(!click_leaves_page(&LinkClick::default()), "no href");
        assert!(!click_leaves_page(&plain("")));
        assert!(!click_leaves_page(&plain("#main")), "skip link");
        let new_tab = LinkClick {
            target: Some("_blank"),
            ..plain("/identity")
        };
        assert!(!click_leaves_page(&new_tab));
        let modified = LinkClick {
            modified: true,
            ..plain("/identity")
        };
        assert!(!click_leaves_page(&modified));
        let middle = LinkClick {
            button: 1,
            ..plain("/identity")
        };
        assert!(!click_leaves_page(&middle));
        let download = LinkClick {
            download: true,
            ..plain("/export")
        };
        assert!(!click_leaves_page(&download));
    }

    #[test]
    fn the_prompt_names_what_is_lost() {
        assert!(LEAVE_PROMPT.contains("unsaved changes"));
        assert!(LEAVE_PROMPT.ends_with('?'));
    }
}
