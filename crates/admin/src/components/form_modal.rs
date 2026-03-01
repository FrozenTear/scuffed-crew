use leptos::prelude::*;
use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::modal::Modal;

/// Form modal wrapping scuffed_ui::Modal.
/// Renders form fields inside modal with Cancel + Submit buttons and loading state.
#[component]
pub fn FormModal(
    #[prop(into)] open: Signal<bool>,
    on_close: Callback<()>,
    #[prop(into)] title: Signal<String>,
    on_submit: Callback<()>,
    #[prop(into, default = false.into())] submitting: Signal<bool>,
    children: Children,
) -> impl IntoView {
    view! {
        <Modal open=open on_close=on_close>
            <div class="form-modal">
                <h3 class="form-modal-title">{move || title.get()}</h3>
                <div class="admin-form">
                    {children()}
                </div>
                <div class="form-modal-actions">
                    <Button
                        variant=ButtonVariant::Ghost
                        on_click=Callback::new(move |_| on_close.run(()))
                    >
                        "Cancel"
                    </Button>
                    <Button
                        variant=ButtonVariant::Primary
                        on_click=Callback::new(move |_| on_submit.run(()))
                    >
                        {move || if submitting.get() { "Saving..." } else { "Save" }}
                    </Button>
                </div>
            </div>
        </Modal>
    }
}

pub const FORM_MODAL_STYLES: &str = r#"
.form-modal {
    min-width: 400px;
    max-width: 520px;
}
.form-modal-title {
    font-family: var(--font-display);
    font-size: 1.3rem;
    color: var(--text-bright);
    margin: 0 0 1.25rem 0;
    text-transform: uppercase;
    letter-spacing: 0.04em;
    padding-bottom: 0.75rem;
    border-bottom: 1px solid var(--border);
}
.form-modal .admin-form {
    margin-bottom: 1.5rem;
}
.form-modal-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
    padding-top: 0.75rem;
    border-top: 1px solid var(--border);
}
"#;
