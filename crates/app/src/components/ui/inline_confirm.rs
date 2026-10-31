//! Inline confirm strip for destructive or hard-to-undo actions.
//!
//! No modal. The strip replaces the action buttons it guards, takes focus on
//! Cancel first, and gives focus back to the trigger on Cancel or Esc. The
//! caller owns the "is the strip open" state and the trigger button, which
//! must carry the id `{id}-trigger`. See `0.6/redesign/ux-spec/games-delete.md`.

use dioxus::prelude::*;

pub const INLINE_CONFIRM_CSS: &str = r#"
.inline-confirm { border: 1px solid var(--danger); border-radius: var(--radius-md);
  padding: var(--space-3); background: transparent; color: var(--text);
  display: flex; flex-direction: column; gap: var(--space-2); }
.inline-confirm__title { font-size: var(--text-sm); font-weight: 600; margin: 0; }
.inline-confirm__body { font-size: var(--text-sm); color: var(--text-2); margin: 0; line-height: 1.5; }
.inline-confirm__actions { display: flex; justify-content: flex-end; gap: var(--space-2); }
.inline-confirm__cancel, .inline-confirm__go { font-family: var(--font-body); font-size: var(--text-sm);
  font-weight: 600; border-radius: var(--radius-md); padding: var(--space-2) var(--space-4);
  cursor: pointer; }
.inline-confirm__cancel { background: transparent; color: var(--text); border: 1px solid var(--border); }
.inline-confirm__cancel:focus-visible { outline: 2px solid var(--accent); outline-offset: 2px; }
.inline-confirm__go { background: var(--danger); color: var(--accent-fg); border: 1px solid var(--danger); }
.inline-confirm__go:hover:not(:disabled) { filter: brightness(1.08); }
.inline-confirm__cancel:disabled, .inline-confirm__go:disabled { opacity: .5; cursor: not-allowed; }
.inline-confirm__hint { font-size: var(--text-xs); color: var(--text-3); margin: 0; text-align: right; }
"#;

/// True for the key that cancels the strip. Pure so it can be tested.
pub fn key_cancels(key: &Key) -> bool {
    *key == Key::Escape
}

/// Move keyboard focus to the element with this id. Native tests have no DOM
/// focus, so only the wasm build does anything.
pub fn focus_element_by_id(id: &str) {
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

/// Focus an element after the next render, so a button that is about to be
/// mounted again (the trigger) exists by the time focus moves to it.
pub fn focus_element_by_id_soon(id: String) {
    spawn(async move {
        #[cfg(feature = "web")]
        {
            gloo_timers::future::TimeoutFuture::new(0).await;
        }
        focus_element_by_id(&id);
    });
}

#[component]
pub fn InlineConfirm(
    /// Id prefix. The trigger button needs id `{id}-trigger`. Cancel gets `{id}-cancel`.
    id: String,
    title: String,
    #[props(default)] body: Option<String>,
    /// The verb on the solid button, for example "Unlink" or "Withdraw".
    confirm_label: String,
    #[props(default = false)] busy: bool,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
) -> Element {
    let cancel_id = format!("{id}-cancel");
    let trigger_id = format!("{id}-trigger");
    let title_id = format!("{id}-title");

    {
        let cancel_id = cancel_id.clone();
        use_effect(move || focus_element_by_id(&cancel_id));
    }

    let cancel = use_callback(move |_: ()| {
        on_cancel.call(());
        focus_element_by_id_soon(trigger_id.clone());
    });

    rsx! {
        div {
            class: "inline-confirm",
            role: "group",
            "aria-labelledby": "{title_id}",
            onkeydown: move |e| {
                if key_cancels(&e.key()) {
                    e.stop_propagation();
                    cancel.call(());
                }
            },
            p { id: "{title_id}", class: "inline-confirm__title", "{title}" }
            if let Some(body) = body {
                p { class: "inline-confirm__body", "{body}" }
            }
            div { class: "inline-confirm__actions",
                button {
                    id: "{cancel_id}",
                    class: "inline-confirm__cancel",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| cancel.call(()),
                    "Cancel"
                }
                button {
                    class: "inline-confirm__go",
                    r#type: "button",
                    disabled: busy,
                    onclick: move |_| on_confirm.call(()),
                    "{confirm_label}"
                }
            }
            p { class: "inline-confirm__hint", "Esc cancels" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn strip() -> Element {
        rsx! {
            InlineConfirm {
                id: "unlink".to_string(),
                title: "Unlink this key?".to_string(),
                body: "You can link a key again later.".to_string(),
                confirm_label: "Unlink".to_string(),
                on_confirm: move |_| {},
                on_cancel: move |_| {},
            }
        }
    }

    fn busy_strip() -> Element {
        rsx! {
            InlineConfirm {
                id: "unlink".to_string(),
                title: "Unlink this key?".to_string(),
                confirm_label: "Unlink".to_string(),
                busy: true,
                on_confirm: move |_| {},
                on_cancel: move |_| {},
            }
        }
    }

    #[test]
    fn only_escape_cancels() {
        assert!(key_cancels(&Key::Escape));
        assert!(!key_cancels(&Key::Enter));
        assert!(!key_cancels(&Key::Character("x".to_string())));
    }

    #[test]
    fn strip_shows_title_body_verb_and_esc_hint() {
        let html = render(strip);
        assert!(html.contains("Unlink this key?"), "{html}");
        assert!(html.contains("You can link a key again later."), "{html}");
        assert!(html.contains(">Unlink</button>"), "{html}");
        assert!(html.contains(">Cancel</button>"), "{html}");
        assert!(html.contains("Esc cancels"), "{html}");
        assert!(html.contains("role=\"group\""), "{html}");
        assert!(html.contains("aria-labelledby=\"unlink-title\""), "{html}");
        assert!(html.contains("id=\"unlink-cancel\""), "{html}");
    }

    #[test]
    fn cancel_comes_before_the_verb_button() {
        let html = render(strip);
        let cancel = html.find(">Cancel</button>").expect("cancel");
        let verb = html.find(">Unlink</button>").expect("verb");
        assert!(cancel < verb, "Cancel is first in tab order: {html}");
    }

    #[test]
    fn busy_strip_disables_both_buttons_and_omits_missing_body() {
        let html = render(busy_strip);
        assert_eq!(html.matches("disabled").count(), 2, "{html}");
        assert!(!html.contains("inline-confirm__body"), "{html}");
    }
}
