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
fn use_document_keydown(on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {
    use std::rc::Rc;

    use wasm_bindgen::JsCast;
    use wasm_bindgen::closure::Closure;

    type KeyHandler = Closure<dyn FnMut(web_sys::KeyboardEvent)>;

    fn listener_fn(closure: &KeyHandler) -> &js_sys::Function {
        closure.as_ref().unchecked_ref()
    }

    let listener = use_hook(move || {
        let mut on_key = on_key;
        let closure: Rc<KeyHandler> =
            Rc::new(Closure::wrap(Box::new(move |evt: web_sys::KeyboardEvent| {
                // Password managers and datalist autocomplete dispatch a plain
                // `Event` named "keydown". The closure type does not check
                // `instanceof KeyboardEvent`, and `key()` is a non-catch import:
                // a missing `key` string is `console.error`'d as
                // "expected a string argument". Ignore those events.
                if !evt.is_instance_of::<web_sys::KeyboardEvent>() {
                    return;
                }
                let key_ok =
                    js_sys::Reflect::get(evt.as_ref(), &wasm_bindgen::JsValue::from_str("key"))
                        .ok()
                        .and_then(|value| value.as_string())
                        .is_some();
                if key_ok {
                    on_key(evt);
                }
            })
                as Box<dyn FnMut(web_sys::KeyboardEvent)>));
        if let Some(window) = web_sys::window() {
            let _ = window.add_event_listener_with_callback("keydown", listener_fn(&closure));
        }
        closure
    });
    let listener = listener.clone();
    use_drop(move || {
        if let Some(window) = web_sys::window() {
            let _ = window.remove_event_listener_with_callback("keydown", listener_fn(&listener));
        }
    });
}

#[cfg(not(target_arch = "wasm32"))]
fn use_document_keydown(_on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {}

fn focus_element(id: &str) {
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
        let _ = id;
    }
}
