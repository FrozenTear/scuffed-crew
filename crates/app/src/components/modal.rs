use dioxus::prelude::*;

const MODAL_CSS: &str = r#"
    .modal-backdrop {
        position: fixed;
        inset: 0;
        background: var(--overlay);
        z-index: 1000;
        display: flex;
        align-items: center;
        justify-content: center;
        animation: modal-fade-in 0.2s ease-out;
    }
    .modal-content {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 2rem;
        max-width: 90vw;
        max-height: 85vh;
        overflow-y: auto;
        position: relative;
        animation: modal-scale-in 0.2s ease-out;
    }
    .modal-close {
        position: absolute;
        top: 0.75rem;
        right: 0.75rem;
        background: none;
        border: none;
        color: var(--text-3);
        font-size: 1.2rem;
        cursor: pointer;
        padding: 0.25rem 0.5rem;
        border-radius: 4px;
    }
    .modal-close:hover {
        color: var(--text);
        background: var(--surface-2);
    }
    @keyframes modal-fade-in {
        from { opacity: 0; }
        to { opacity: 1; }
    }
    @keyframes modal-scale-in {
        from { opacity: 0; transform: scale(0.95); }
        to { opacity: 1; transform: scale(1); }
    }
"#;

#[component]
pub fn Modal(open: Signal<bool>, on_close: EventHandler<()>, children: Element) -> Element {
    if !open() {
        return rsx! {};
    }

    rsx! {
        style { {MODAL_CSS} }
        div {
            class: "modal-backdrop",
            onclick: move |_| on_close.call(()),
            div {
                class: "modal-content",
                onclick: move |e| e.stop_propagation(),
                button {
                    class: "modal-close",
                    onclick: move |_| on_close.call(()),
                    "\u{00d7}"
                }
                {children}
            }
        }
    }
}
