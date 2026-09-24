mod admin;
mod public;
mod strategy;

pub use admin::AdminLayout;
pub use public::PublicLayout;
pub use strategy::StrategyLayout;

use std::rc::Rc;

use dioxus::prelude::*;
use wasm_bindgen::JsCast;
use wasm_bindgen::closure::Closure;

type KeyHandler = Closure<dyn FnMut(web_sys::KeyboardEvent)>;

fn listener_fn(closure: &KeyHandler) -> &js_sys::Function {
    closure.as_ref().unchecked_ref()
}

/// Document `keydown` for the lifetime of the calling component.
///
/// Installed once and removed on unmount (`use_drop`). Do not `Closure::forget`.
/// The handler captured on the first render is the one that stays registered, so
/// it must read signals when the key fires rather than close over a copied bool.
fn use_document_keydown(on_key: impl FnMut(web_sys::KeyboardEvent) + 'static) {
    let listener = use_hook(move || {
        let mut on_key = on_key;
        let closure: Rc<KeyHandler> = Rc::new(Closure::wrap(Box::new(
            move |evt: web_sys::KeyboardEvent| on_key(evt),
        )
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

fn focus_element(id: &str) {
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
