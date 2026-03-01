use leptos::ev;
use leptos::prelude::*;

/// Modal overlay component.
///
/// Renders children inside a centered overlay. Clicking the backdrop
/// calls `on_close`. Uses CSS display toggling rather than conditional
/// rendering so that `Children` (FnOnce) works with Leptos.
#[component]
pub fn Modal(
    /// Whether the modal is currently visible
    #[prop(into)]
    open: Signal<bool>,
    /// Called when the user clicks the backdrop or close button
    on_close: Callback<()>,
    children: Children,
) -> impl IntoView {
    let content = children();

    view! {
        <div
            class="sc-modal-backdrop"
            style:display=move || if open.get() { "flex" } else { "none" }
            on:click=move |_| on_close.run(())
        >
            <div class="sc-modal" on:click=|e: ev::MouseEvent| e.stop_propagation()>
                <button
                    class="sc-modal-close"
                    on:click=move |_| on_close.run(())
                    aria-label="Close"
                >
                    "\u{00d7}"
                </button>
                {content}
            </div>
        </div>
    }
}

pub const MODAL_STYLES: &str = r#"
.sc-modal-backdrop {
    position: fixed;
    inset: 0;
    background: rgba(0, 0, 0, 0.7);
    display: flex;
    align-items: center;
    justify-content: center;
    z-index: 1000;
    animation: sc-fade-in 0.2s ease;
}
.sc-modal {
    position: relative;
    background: var(--bg-card);
    border: 1px solid var(--border);
    border-radius: 12px;
    padding: 2rem;
    max-width: 90vw;
    max-height: 85vh;
    overflow-y: auto;
    animation: sc-scale-in 0.2s ease;
}
.sc-modal-close {
    position: absolute;
    top: 0.75rem;
    right: 0.75rem;
    background: none;
    border: none;
    color: var(--text-muted);
    font-size: 1.5rem;
    cursor: pointer;
    padding: 0.25rem 0.5rem;
    line-height: 1;
}
.sc-modal-close:hover { color: var(--text-bright); }
@keyframes sc-fade-in {
    from { opacity: 0; }
    to { opacity: 1; }
}
@keyframes sc-scale-in {
    from { opacity: 0; transform: scale(0.95); }
    to { opacity: 1; transform: scale(1); }
}
"#;
