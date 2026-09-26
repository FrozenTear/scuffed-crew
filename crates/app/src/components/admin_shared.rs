use dioxus::prelude::*;

use crate::hooks::{ApiResource, LIST_MAX_PAGES};
use crate::routes::Route;

/// Shared pending-state renderer for admin list/detail resources.
///
/// Call this from the `None` arm of a resource's `data` match. When the last
/// fetch failed (`resource.error` is set) it renders the error message plus a
/// Retry button that bumps the resource's refresh counter; otherwise it renders
/// the "Loading…" placeholder. Standardizes the error UX across every admin page
/// so a failed / 403 / 500 fetch never shows an infinite "Loading…".
///
/// Takes `&ApiResource` (not by value): call sites hold a live read-guard on
/// `resource.data` inside the `match`, and `ApiResource<T>` is only `Copy` when
/// `T: Copy`, so a by-value pass would move-conflict with that borrow. The retry
/// button copies out the `refresh` Signal (always `Copy`) to bump it.
pub fn admin_pending<T: 'static>(resource: &ApiResource<T>, label: &str) -> Element {
    if let Some(err) = resource.error.read().as_ref().cloned() {
        let mut refresh = resource.refresh;
        rsx! {
            p { class: "admin-loading", style: "color: var(--danger);",
                "Failed to load {label}: {err}"
            }
            button {
                class: "row-btn",
                onclick: move |_| refresh += 1,
                "Retry"
            }
        }
    } else {
        rsx! { p { class: "admin-loading", "Loading..." } }
    }
}

/// Visible cap for a cursor list that stopped with a next cursor still set.
///
/// Renders nothing until `truncated` is true, so pages under the page budget
/// look the same. "Load more" raises the budget by another auto-follow batch.
/// Call this outside a live `resource.data.read()` guard.
pub fn list_cap_notice<T: 'static>(resource: &ApiResource<T>, what: &str) -> Element {
    let truncated = *resource.truncated.read();
    if !truncated {
        return rsx! {};
    }
    let shown = *resource.shown.read();
    let what = what.to_string();
    let mut budget = resource.page_budget;
    rsx! {
        p { class: "list-cap-notice", role: "status",
            "Showing the first {shown} {what}. "
            button {
                r#type: "button",
                class: "list-cap-notice__more",
                onclick: move |_| budget += LIST_MAX_PAGES,
                "Load more"
            }
        }
    }
}

/// Client-side authorization denial for admin pages whose server tier is stricter
/// than the officer+ `AdminLayout` guard. Defense-in-depth: rendered when a user
/// without the required role reaches the page (e.g. by typing the URL directly).
#[component]
pub fn AccessDenied(message: String) -> Element {
    rsx! {
        div {
            style: "display:flex;flex-direction:column;align-items:center;justify-content:center;padding:4rem 1rem;color:var(--text-2);gap:0.5rem;text-align:center;",
            h2 { style: "color:var(--text);margin-bottom:0.5rem;", "Access Denied" }
            p { "{message}" }
            Link {
                to: Route::AdminDashboard {},
                style: "color:var(--accent);margin-top:0.5rem;",
                "Back to dashboard"
            }
        }
    }
}

#[component]
pub fn DataTable(headers: Vec<&'static str>, children: Element) -> Element {
    rsx! {
        // Horizontal scroll containment so wide admin tables don't blow out
        // the layout on narrow viewports (see research-admin-mobile-grok.md).
        div { class: "data-table-scroll",
            table { class: "data-table",
                thead {
                    tr {
                        for h in headers.iter() {
                            th { "{h}" }
                        }
                    }
                }
                tbody { {children} }
            }
        }
    }
}

#[component]
pub fn FormModal(
    title: String,
    open: bool,
    submitting: bool,
    on_close: EventHandler<()>,
    on_submit: EventHandler<()>,
    #[props(default = false)] danger: bool,
    #[props(default = false)] wide: bool,
    children: Element,
) -> Element {
    if !open {
        return rsx! {};
    }
    let save_class = if danger {
        "btn-save danger"
    } else {
        "btn-save"
    };
    let modal_class = if wide {
        "form-modal wide"
    } else {
        "form-modal"
    };
    rsx! {
        div {
            class: "form-modal-overlay",
            onclick: move |_| on_close.call(()),
            div {
                class: "{modal_class}",
                onclick: move |e| e.stop_propagation(),
                div { class: "form-modal-header", "{title}" }
                div { class: "form-modal-body", {children} }
                div { class: "form-modal-footer",
                    button {
                        class: "btn-cancel",
                        onclick: move |_| on_close.call(()),
                        "Cancel"
                    }
                    button {
                        class: "{save_class}",
                        disabled: submitting,
                        onclick: move |_| on_submit.call(()),
                        if submitting { "Saving..." } else { "Save" }
                    }
                }
            }
        }
    }
}

#[component]
pub fn ConfirmDialog(
    title: String,
    message: String,
    open: bool,
    #[props(default = false)] danger: bool,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
    #[props(default = None)] extra: Option<Element>,
) -> Element {
    if !open {
        return rsx! {};
    }
    let btn_class = if danger {
        "btn-save danger"
    } else {
        "btn-save"
    };
    rsx! {
        div {
            class: "form-modal-overlay",
            onclick: move |_| on_cancel.call(()),
            div {
                class: "form-modal",
                onclick: move |e| e.stop_propagation(),
                div { class: "form-modal-header", "{title}" }
                div { class: "confirm-body",
                    p { "{message}" }
                    if let Some(extra) = extra {
                        {extra}
                    }
                }
                div { class: "form-modal-footer",
                    button {
                        class: "btn-cancel",
                        onclick: move |_| on_cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "{btn_class}",
                        onclick: move |_| on_confirm.call(()),
                        "Confirm"
                    }
                }
            }
        }
    }
}

#[component]
pub fn StatusPill(status: String) -> Element {
    let class = format!("status-pill {status}");
    // Enum values like `in_progress` read as words; CSS uppercases them.
    let label = status.replace('_', " ");
    rsx! { span { class: "{class}", "{label}" } }
}

#[component]
pub fn RolePill(role: String) -> Element {
    let class = format!("role-pill {role}");
    rsx! { span { class: "{class}", "{role}" } }
}

#[component]
pub fn SummaryCard(
    value: String,
    label: &'static str,
    /// When set, the card is a click-through link (admin dashboard KPIs).
    #[props(default)]
    to: Option<Route>,
) -> Element {
    let class = if to.is_some() {
        "summary-card summary-card-link"
    } else {
        "summary-card"
    };
    let body = rsx! {
        div { class: "value", "{value}" }
        div { class: "label", "{label}" }
    };
    match to {
        Some(route) => rsx! {
            Link { to: route, class: "{class}", {body} }
        },
        None => rsx! {
            div { class: "{class}", {body} }
        },
    }
}
