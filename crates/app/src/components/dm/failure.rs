use dioxus::prelude::*;

use super::types::DmLoadFailure;
use crate::routes::Route;

const FAILURE_CSS: &str = r#"
.dm-failure {
    background: color-mix(in srgb, var(--danger) 8%, transparent);
    border: 1px solid color-mix(in srgb, var(--danger) 40%, transparent);
    border-radius: 8px;
    padding: 1rem 1.25rem;
    color: var(--danger);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
.dm-failure a {
    color: var(--danger);
    font-weight: 600;
    text-decoration: underline;
}
"#;

#[component]
pub fn DmFailureNotice(failure: DmLoadFailure) -> Element {
    let message = failure.message();
    let identity = matches!(failure, DmLoadFailure::IdentitySettings);
    rsx! {
        style { {FAILURE_CSS} }
        div { class: "dm-failure", role: "alert",
            "{message}"
            if identity {
                " "
                Link { to: Route::IdentitySettings {}, "Visit identity settings" }
                " to enable it."
            }
        }
    }
}
