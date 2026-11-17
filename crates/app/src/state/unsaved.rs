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
//! - Actions that are not links, such as Log out, call [`confirm_leave`]
//!   first, which asks the same question while any guard is on.
//!
//! A yes is asked once: when the link leaves the app (a page the router does
//! not handle), the browser's `beforeunload` prompt that follows is skipped.
//!
//! The browser's back and forward buttons are not covered: the router has
//! already moved by the time it hears of them.

use std::cell::Cell;

thread_local! {
    /// Guards on right now, on any page.
    static ARMED: Cell<usize> = const { Cell::new(0) };
    /// The member just said yes to leaving, so the unload prompt that a
    /// link out of the app raises next would ask a second time.
    static LEAVING: Cell<bool> = const { Cell::new(false) };
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

/// Counts as an armed guard while it lives.
struct Armed;

impl Armed {
    fn new() -> Self {
        ARMED.with(|n| n.set(n.get() + 1));
        Self
    }
}

impl Drop for Armed {
    fn drop(&mut self) {
        ARMED.with(|n| n.set(n.get().saturating_sub(1)));
    }
}

/// True while some page holds an unsaved draft.
pub fn any_unsaved() -> bool {
    ARMED.with(Cell::get) > 0
}

/// Ask [`LEAVE_PROMPT`] with `ask` when `armed`, and remember a yes so the
/// browser's unload prompt does not ask again. True means go on.
fn decide_leave(armed: bool, ask: impl FnOnce(&str) -> bool) -> bool {
    if !armed {
        return true;
    }
    let leave = ask(LEAVE_PROMPT);
    if leave {
        mark_leaving();
    }
    leave
}

/// For a way out that is not a link (Log out): true when there is no
/// unsaved draft, or the member says to drop it.
pub fn confirm_leave() -> bool {
    decide_leave(any_unsaved(), ask)
}

/// The unload prompt asks unless the member already said yes.
fn unload_asks() -> bool {
    !LEAVING.with(Cell::get)
}

/// Remember a yes until the current event is done. A link out of the app
/// raises `beforeunload` while its click is still being handled (checked in
/// Chromium), so the reset that is queued after it comes too late to make
/// that prompt ask; a link the router handles stays on the page, and the
/// reset re-arms the unload prompt for later.
fn mark_leaving() {
    LEAVING.with(|l| l.set(true));
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    web::reset_leaving_soon();
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    LEAVING.with(|l| l.set(false));
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

    use super::{Armed, LinkClick, ask, click_leaves_page, decide_leave, unload_asks};

    /// The two listeners, removed when this is dropped.
    struct Listeners {
        unload: Closure<dyn FnMut(web_sys::Event)>,
        click: Closure<dyn FnMut(web_sys::MouseEvent)>,
        _armed: Armed,
    }

    /// Clear the yes once the current event is done.
    pub(super) fn reset_leaving_soon() {
        let reset = Closure::once_into_js(|| super::LEAVING.with(|l| l.set(false)));
        if let Some(window) = web_sys::window() {
            let _ = window.set_timeout_with_callback(reset.unchecked_ref());
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
                    if !decide_leave(true, ask) {
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
            Some(Self {
                unload,
                click,
                _armed: Armed::new(),
            })
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

    /// Vision 694 r3 nit 3: Log out asks while a draft is unsaved, and only
    /// then; a no keeps the member signed in.
    #[test]
    fn a_way_out_asks_only_while_a_guard_is_on() {
        assert!(!any_unsaved());
        let mut asked = Vec::new();
        assert!(decide_leave(any_unsaved(), |q| {
            asked.push(q.to_string());
            false
        }));
        assert!(asked.is_empty(), "nothing to lose, nothing asked");

        let guard = Armed::new();
        assert!(any_unsaved());
        assert!(!decide_leave(any_unsaved(), |q| {
            asked.push(q.to_string());
            false
        }));
        assert_eq!(asked, [LEAVE_PROMPT], "asked once, with the prompt");
        assert!(decide_leave(any_unsaved(), |_| true));
        drop(guard);
        assert!(!any_unsaved(), "the guard going disarms it");
    }

    /// Vision 694 r3 nit 3: a no leaves the unload prompt armed. (A yes
    /// skips it until the click is done; that timing is checked in the
    /// browser, because the reset runs from the event loop.)
    #[test]
    fn a_no_keeps_the_unload_prompt() {
        let _guard = Armed::new();
        assert!(!decide_leave(true, |_| false));
        assert!(unload_asks());
        LEAVING.with(|l| l.set(true));
        assert!(!unload_asks(), "a yes skips the second prompt");
        LEAVING.with(|l| l.set(false));
    }

    #[test]
    fn the_prompt_names_what_is_lost() {
        assert!(LEAVE_PROMPT.contains("unsaved changes"));
        assert!(LEAVE_PROMPT.ends_with('?'));
    }
}
