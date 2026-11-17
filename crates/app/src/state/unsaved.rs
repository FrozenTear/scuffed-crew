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
//! A yes is asked once: when the link leaves the app (a page the router does
//! not handle), the browser's `beforeunload` prompt that follows is skipped.
//! Chromium raises that prompt while the click is still being handled,
//! Firefox a task or more later, so the yes is kept until the prompt it
//! covers comes, and dropped as soon as it is clear the page stayed (see
//! [`mark_leaving`]).
//!
//! The browser's back and forward buttons are not covered: the router has
//! already moved by the time it hears of them.

use std::cell::Cell;

thread_local! {
    /// The yes the member just gave to leaving, by number, so the unload
    /// prompt that a link out of the app raises next does not ask again.
    static LEAVING: Cell<Option<u64>> = const { Cell::new(None) };
    /// The number the next yes gets.
    static NEXT_YES: Cell<u64> = const { Cell::new(0) };
}

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

/// Ask [`LEAVE_PROMPT`] with `ask`, and remember a yes so the browser's
/// unload prompt does not ask again. `Some` carries the yes (see
/// [`mark_leaving`]); `None` means the member said no.
fn decide_leave(ask: impl FnOnce(&str) -> bool) -> Option<u64> {
    ask(LEAVE_PROMPT).then(mark_leaving)
}

/// Remember a yes until the unload prompt it covers comes (it is used up
/// there, see [`unload_asks`]) or until the page is known to have stayed
/// ([`clear_leaving`]): the router or another handler took the click, or
/// no unload began in time. Numbered, so a late clear of an old yes leaves
/// a newer one alone.
fn mark_leaving() -> u64 {
    let id = NEXT_YES.with(|n| {
        let id = n.get().wrapping_add(1);
        n.set(id);
        id
    });
    LEAVING.with(|l| l.set(Some(id)));
    id
}

/// The page stayed after yes number `id`: the unload prompt asks again.
fn clear_leaving(id: u64) {
    LEAVING.with(|l| {
        if l.get() == Some(id) {
            l.set(None);
        }
    });
}

/// The unload prompt asks unless the member just said yes to the link that
/// raised it. A yes covers that one prompt, so it is used up here.
fn unload_asks() -> bool {
    LEAVING.with(Cell::take).is_none()
}

fn ask(question: &str) -> bool {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web_sys::window()
            .and_then(|w| w.confirm_with_message(question).ok())
            .unwrap_or(true)
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        let _ = question;
        true
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

    use super::{
        LEAVE_PROMPT, LinkClick, ask, clear_leaving, click_leaves_page, decide_leave, unload_asks,
    };

    /// How long a yes waits for the unload it covers when the click was not
    /// taken by the router. Both browsers start the unload long before this
    /// (Chromium inside the click, Firefox a task or so later); a link that
    /// never unloads (a fragment of this page, another app's scheme) gets
    /// the unload prompt back after this.
    const NO_UNLOAD_AFTER_MS: i32 = 2000;

    /// The two listeners, removed when this is dropped.
    struct Listeners {
        unload: Closure<dyn FnMut(web_sys::Event)>,
        click: Closure<dyn FnMut(web_sys::MouseEvent)>,
    }

    /// After yes number `id` to `event`: once every handler has seen the
    /// click, a prevented default (the router took it, the page stays)
    /// drops the yes at once; otherwise the page is on its way out and the
    /// yes waits for the unload prompt, up to [`NO_UNLOAD_AFTER_MS`].
    fn settle_yes(event: web_sys::MouseEvent, id: u64) {
        let Some(window) = web_sys::window() else {
            clear_leaving(id);
            return;
        };
        let after_click = Closure::once_into_js(move || {
            if event.default_prevented() {
                clear_leaving(id);
            }
        });
        let backstop = Closure::once_into_js(move || clear_leaving(id));
        if window
            .set_timeout_with_callback(after_click.unchecked_ref())
            .is_err()
            || window
                .set_timeout_with_callback_and_timeout_and_arguments_0(
                    backstop.unchecked_ref(),
                    NO_UNLOAD_AFTER_MS,
                )
                .is_err()
        {
            clear_leaving(id);
        }
    }

    impl Listeners {
        fn install() -> Option<Self> {
            let window = web_sys::window()?;
            let document = window.document()?;
            let unload = Closure::<dyn FnMut(web_sys::Event)>::new(|event: web_sys::Event| {
                if !unload_asks() {
                    return;
                }
                event.prevent_default();
                // Firefox and older browsers also look for a non-empty
                // return value; none of them show the text.
                let _ = js_sys::Reflect::set(&event, &"returnValue".into(), &LEAVE_PROMPT.into());
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
                    match decide_leave(ask) {
                        Some(id) => settle_yes(event, id),
                        None => {
                            event.prevent_default();
                            event.stop_immediate_propagation();
                        }
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

    /// A no is remembered nowhere, so the unload prompt still asks.
    #[test]
    fn a_no_keeps_the_unload_prompt() {
        let mut asked = Vec::new();
        assert_eq!(
            decide_leave(|q| {
                asked.push(q.to_string());
                false
            }),
            None
        );
        assert_eq!(asked, [LEAVE_PROMPT], "asked once, with the prompt");
        assert!(unload_asks());
        assert!(unload_asks());
    }

    /// QA 694 (e5c48067, Firefox): a yes held only until the click was done
    /// was gone by the time Firefox raised its unload prompt, so the member
    /// was asked twice. The yes now lasts until that prompt, which uses it
    /// up, so a later reload asks again.
    #[test]
    fn a_yes_skips_one_unload_prompt_whenever_it_comes() {
        let id = decide_leave(|_| true).expect("yes");
        assert!(!unload_asks(), "the prompt the link raises is skipped");
        assert!(unload_asks(), "the next unload asks again");
        clear_leaving(id);
        assert!(unload_asks(), "a late clear of a used yes changes nothing");
    }

    /// A link the router takes keeps the page, so its yes is dropped and a
    /// reload after it still asks; a stale clear does not drop a newer yes.
    #[test]
    fn a_yes_the_page_survives_is_dropped() {
        let first = decide_leave(|_| true).expect("yes");
        clear_leaving(first);
        assert!(unload_asks(), "router link: the unload prompt is back");

        let old = decide_leave(|_| true).expect("yes");
        clear_leaving(old);
        let newer = decide_leave(|_| true).expect("yes");
        clear_leaving(old);
        assert!(!unload_asks(), "the old yes's backstop left the new one");
        clear_leaving(newer);
        assert!(unload_asks());
    }

    #[test]
    fn the_prompt_names_what_is_lost() {
        assert!(LEAVE_PROMPT.contains("unsaved changes"));
        assert!(LEAVE_PROMPT.ends_with('?'));
    }
}
