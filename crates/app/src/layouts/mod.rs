mod admin;
mod public;
mod strategy;

pub use admin::AdminLayout;
pub use public::PublicLayout;
pub use strategy::StrategyLayout;

#[cfg(target_arch = "wasm32")]
use dioxus::prelude::*;

/// Document `keydown` for the lifetime of the calling component.
///
/// Installed once and removed on unmount (`use_drop`). Do not `Closure::forget`.
/// The handler captured on the first render is the one that stays registered, so
/// it must read signals when the key fires rather than close over a copied bool.
///
/// `Closure::wrap` calls a wasm import (`__wbindgen_describe_cast`) that aborts
/// the process off `wasm32`. Native tests that mount these layouts skip the
/// listener entirely; they do not build a closure and then look for a window.
#[cfg(target_arch = "wasm32")]
pub(crate) fn use_document_keydown(on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {
    use_keydown_listener(false, on_key);
}

/// Like [`use_document_keydown`], but listens on `document` in the capture
/// phase, so the handler sees Tab before the browser moves focus. The strategy
/// editor uses this. It is removed on unmount like the bubbling version; a
/// leaked capture listener outlives the editor's signals and panics the app
/// on the next Space, Tab or Backspace.
#[cfg(target_arch = "wasm32")]
pub(crate) fn use_document_keydown_capture(on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {
    use_keydown_listener(true, on_key);
}

/// `capture == false`: `window`, bubbling. `capture == true`: `document`, capture.
#[cfg(target_arch = "wasm32")]
fn use_keydown_listener(capture: bool, on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {
    use std::rc::Rc;

    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    type KeyHandler = Closure<dyn FnMut(web_sys::KeyboardEvent)>;

    fn listener_fn(closure: &KeyHandler) -> &js_sys::Function {
        closure.as_ref().unchecked_ref()
    }

    let listener = use_hook(move || {
        // The browser calls this between renders, when the Dioxus scope stack
        // is empty. Handlers that `spawn` (Ctrl+S save, toasts) would panic,
        // and the release build aborts, so run them in this component's scope.
        let mut on_key = in_current_scope(on_key);
        let closure: Rc<KeyHandler> =
            Rc::new(Closure::wrap(Box::new(move |evt: web_sys::KeyboardEvent| {
                // Password managers and datalist autocomplete dispatch a plain
                // `Event` named "keydown". The closure type does not check
                // `instanceof KeyboardEvent`, and `key()` is a non-catch import:
                // a missing `key` string is `console.error`'d as
                // "expected a string argument". `dyn_ref` confirms the event;
                // `key()` is then a string, and an empty one is not a shortcut.
                let Some(keyboard) = evt.dyn_ref::<web_sys::KeyboardEvent>() else {
                    return;
                };
                if keyboard.key().is_empty() {
                    return;
                }
                on_key(evt);
            })
                as Box<dyn FnMut(web_sys::KeyboardEvent)>));
        if let Some(target) = keydown_target(capture) {
            let _ = target.add_event_listener_with_callback_and_bool(
                "keydown",
                listener_fn(&closure),
                capture,
            );
        }
        closure
    });
    let listener = listener.clone();
    use_drop(move || {
        if let Some(target) = keydown_target(capture) {
            // Removal must pass the same capture flag that was used to add.
            let _ = target.remove_event_listener_with_callback_and_bool(
                "keydown",
                listener_fn(&listener),
                capture,
            );
        }
    });
}

#[cfg(target_arch = "wasm32")]
fn keydown_target(capture: bool) -> Option<web_sys::EventTarget> {
    let window = web_sys::window()?;
    if capture {
        window.document().map(Into::into)
    } else {
        Some(window.into())
    }
}

/// `Closure::wrap` aborts off wasm32, so the listener is not installed.
/// Desktop still has a window; this stub is the non-wasm build, not a missing window.
/// Escape handlers in the public and admin layouts therefore do not run here.
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn use_document_keydown(_on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {}

/// Non-wasm stub; see [`use_document_keydown`].
#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn use_document_keydown_capture(_on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {}

/// Wrap a callback that a browser API (event listener, WebSocket, timer)
/// will call later, outside any render, so it runs in the calling
/// component's Dioxus scope.
///
/// Between events dioxus-web keeps the runtime alive but the scope stack is
/// empty, so `spawn`, `use_toast().show` and context lookups called from a
/// raw `web_sys` callback panic ("scope_stack.last().unwrap()"), which aborts
/// the release WASM. Call this during render (for example inside `use_hook`).
pub(crate) fn in_current_scope<E>(mut f: impl FnMut(E) + 'static) -> impl FnMut(E) + 'static {
    let runtime = dioxus::core::Runtime::current();
    let scope = dioxus::core::current_scope_id();
    move |arg: E| runtime.in_scope(scope, || f(arg))
}

pub(crate) fn focus_element(id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let Some(el) = document.get_element_by_id(id) else {
            return;
        };
        if let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() {
            let _ = el.focus();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Desktop still has a window. This build skips `HtmlElement::focus`
        // because that call lives in the wasm branch above.
        let _ = id;
    }
}

#[cfg(test)]
mod tests {
    use super::in_current_scope;
    use dioxus::prelude::*;
    use std::cell::RefCell;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    type Stored = Box<dyn FnMut(())>;

    thread_local! {
        static WRAPPED: RefCell<Option<Stored>> = const { RefCell::new(None) };
        static RAW: RefCell<Option<Stored>> = const { RefCell::new(None) };
        static SPAWNED: RefCell<u32> = const { RefCell::new(0) };
    }

    #[component]
    fn Holder() -> Element {
        use_hook(|| {
            let spawn_one = |_: ()| {
                spawn(async {});
                SPAWNED.with(|n| *n.borrow_mut() += 1);
            };
            WRAPPED.with(|slot| *slot.borrow_mut() = Some(Box::new(in_current_scope(spawn_one))));
            RAW.with(|slot| *slot.borrow_mut() = Some(Box::new(spawn_one)));
        });
        rsx! { div {} }
    }

    fn call(slot: &'static std::thread::LocalKey<RefCell<Option<Stored>>>) -> bool {
        let mut f = slot
            .with(|s| s.borrow_mut().take())
            .expect("callback stored");
        let ok = catch_unwind(AssertUnwindSafe(|| f(()))).is_ok();
        slot.with(|s| *s.borrow_mut() = Some(f));
        ok
    }

    /// F1: Ctrl+S in the editor called `spawn` from the keydown listener with
    /// an empty scope stack and panicked. The wrapped callback runs in its
    /// component's scope; the raw one shows the test would catch the bug.
    #[test]
    fn stored_callback_can_spawn_outside_render() {
        let mut dom = VirtualDom::new(Holder);
        dom.rebuild_in_place();
        // dioxus-web holds a runtime guard between events, with no scope.
        let _guard = dioxus::core::RuntimeGuard::new(dom.runtime());

        assert!(call(&WRAPPED), "wrapped callback must not panic");
        assert_eq!(SPAWNED.with(|n| *n.borrow()), 1);
        assert!(
            !call(&RAW),
            "an unwrapped callback that spawns outside render panics"
        );
    }
}
