use leptos::prelude::*;

/// Status pill for application statuses.
#[component]
pub fn StatusPill(status: String) -> impl IntoView {
    let class = format!("status-pill {status}");
    view! {
        <span class=class>{status}</span>
    }
}

/// Role pill for org roles.
#[component]
pub fn RolePill(role: String) -> impl IntoView {
    let class = format!("role-pill {role}");
    view! {
        <span class=class>{role}</span>
    }
}
