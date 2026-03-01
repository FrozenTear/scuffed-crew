use leptos::prelude::*;
use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::modal::Modal;

/// Confirmation dialog wrapping scuffed_ui::Modal.
/// Used before destructive actions like delete/deactivate.
#[component]
pub fn ConfirmDialog(
    #[prop(into)] open: Signal<bool>,
    on_confirm: Callback<()>,
    on_cancel: Callback<()>,
    #[prop(into)] title: Signal<String>,
    #[prop(into)] message: Signal<String>,
    #[prop(into, default = false.into())] danger: Signal<bool>,
    #[prop(optional)] children: Option<Children>,
) -> impl IntoView {
    let confirm_variant = if danger.get_untracked() {
        ButtonVariant::Danger
    } else {
        ButtonVariant::Primary
    };

    view! {
        <Modal open=open on_close=on_cancel>
            <div class="confirm-dialog">
                <h3 class="confirm-title">{move || title.get()}</h3>
                <p class="confirm-message">{move || message.get()}</p>
                {children.map(|c| c())}
                <div class="confirm-actions">
                    <Button
                        variant=ButtonVariant::Ghost
                        on_click=Callback::new(move |_| on_cancel.run(()))
                    >
                        "Cancel"
                    </Button>
                    <Button
                        variant=confirm_variant
                        on_click=Callback::new(move |_| on_confirm.run(()))
                    >
                        "Confirm"
                    </Button>
                </div>
            </div>
        </Modal>
    }
}

pub const CONFIRM_DIALOG_STYLES: &str = r#"
.confirm-dialog {
    min-width: 320px;
    max-width: 420px;
}
.confirm-title {
    font-family: var(--font-display);
    font-size: 1.2rem;
    color: var(--text-bright);
    margin: 0 0 0.75rem 0;
    text-transform: uppercase;
}
.confirm-message {
    color: var(--text-secondary);
    font-size: 0.9rem;
    line-height: 1.5;
    margin: 0 0 1.25rem 0;
}
.confirm-actions {
    display: flex;
    justify-content: flex-end;
    gap: 0.75rem;
}
"#;
