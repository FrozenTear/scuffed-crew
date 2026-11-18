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
            p { class: "admin-loading", style: "color: var(--loss);",
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
/// The status line is always rendered and only its text changes, so the cap is
/// announced when it appears. It is empty (and out of the layout) until
/// `truncated` is true, so pages under the page budget look the same. "Load
/// more" raises the budget by another auto-follow batch; it sits after the
/// status line, not inside it. Call this outside a live `resource.data.read()`
/// guard.
pub fn list_cap_notice<T: 'static>(resource: &ApiResource<T>, what: &str) -> Element {
    let truncated = *resource.truncated.read();
    let shown = *resource.shown.read();
    let text = cap_notice_text(truncated, shown, what);
    let mut budget = resource.page_budget;
    rsx! {
        div { class: if truncated { "list-cap-notice" } else { "list-cap-notice is-idle" },
            p { class: "list-cap-notice__text", role: "status", "{text}" }
            if truncated {
                button {
                    r#type: "button",
                    class: "list-cap-notice__more",
                    onclick: move |_| budget += LIST_MAX_PAGES,
                    "Load more"
                }
            }
        }
    }
}

/// Text of the cap status line. Empty while nothing was cut off.
fn cap_notice_text(truncated: bool, shown: usize, what: &str) -> String {
    if truncated {
        format!("Showing the first {shown} {what}.")
    } else {
        String::new()
    }
}

/// Client-side authorization denial for admin pages whose server tier is stricter
/// than the officer+ `AdminLayout` guard. Defense-in-depth: rendered when a user
/// without the required role reaches the page (e.g. by typing the URL directly).
#[component]
pub fn AccessDenied(message: String) -> Element {
    rsx! {
        div {
            style: "display:flex;flex-direction:column;align-items:center;justify-content:center;padding:4rem 1rem;color:var(--text-3);gap:0.5rem;text-align:center;",
            h2 { style: "color:var(--text);margin-bottom:0.5rem;", "Access Denied" }
            p { "{message}" }
            Link {
                to: Route::AdminDashboard {},
                style: "color:var(--accent-text);margin-top:0.5rem;",
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

/// Id of the Cancel button, so focus can start there when the dialog opens.
const CONFIRM_CANCEL_ID: &str = "confirm-dialog-cancel";
const CONFIRM_TITLE_ID: &str = "confirm-dialog-title";

/// Modal confirm. Opens with focus on Cancel, closes on Esc, and names the
/// action on the solid button (`confirm_label`, default "Confirm"). Pass
/// `restore_focus_id` to move focus back to the button that opened it.
#[component]
pub fn ConfirmDialog(
    title: String,
    message: String,
    open: bool,
    #[props(default = false)] danger: bool,
    #[props(default = "Confirm".to_string())] confirm_label: String,
    #[props(default)] restore_focus_id: Option<String>,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
    #[props(default = None)] extra: Option<Element>,
) -> Element {
    if !open {
        return rsx! {};
    }
    rsx! {
        ConfirmDialogOpen {
            title,
            message,
            danger,
            confirm_label,
            restore_focus_id,
            on_confirm,
            on_cancel,
            extra,
        }
    }
}

/// The open dialog. Split from `ConfirmDialog` so the focus effect runs once
/// per open instead of on every render of a closed dialog.
#[component]
fn ConfirmDialogOpen(
    title: String,
    message: String,
    danger: bool,
    confirm_label: String,
    restore_focus_id: Option<String>,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
    extra: Option<Element>,
) -> Element {
    use crate::components::ui::inline_confirm::{
        focus_element_by_id, focus_element_by_id_soon, key_cancels,
    };

    use_effect(|| focus_element_by_id(CONFIRM_CANCEL_ID));

    let cancel = use_callback(move |_: ()| {
        on_cancel.call(());
        if let Some(id) = restore_focus_id.clone() {
            focus_element_by_id_soon(id);
        }
    });
    let btn_class = if danger {
        "btn-save danger"
    } else {
        "btn-save"
    };
    rsx! {
        div {
            class: "form-modal-overlay",
            onclick: move |_| cancel.call(()),
            onkeydown: move |e| {
                if key_cancels(&e.key()) {
                    e.stop_propagation();
                    cancel.call(());
                }
            },
            div {
                class: "form-modal",
                role: "dialog",
                "aria-modal": "true",
                "aria-labelledby": CONFIRM_TITLE_ID,
                onclick: move |e| e.stop_propagation(),
                div { id: CONFIRM_TITLE_ID, class: "form-modal-header", "{title}" }
                div { class: "confirm-body",
                    p { "{message}" }
                    if let Some(extra) = extra {
                        {extra}
                    }
                }
                div { class: "form-modal-footer",
                    button {
                        id: CONFIRM_CANCEL_ID,
                        class: "btn-cancel",
                        r#type: "button",
                        onclick: move |_| cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: "{btn_class}",
                        r#type: "button",
                        onclick: move |_| on_confirm.call(()),
                        "{confirm_label}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn StatusPill(status: String) -> Element {
    let class = format!("status-pill {status}");
    rsx! { span { class: "{class}", "{status}" } }
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

#[cfg(test)]
mod confirm_dialog_tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn closed_dialog_renders_nothing() {
        fn app() -> Element {
            rsx! {
                ConfirmDialog {
                    title: "Delete it?".to_string(),
                    message: "Gone for good.".to_string(),
                    open: false,
                    on_confirm: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        assert!(!render(app).contains("Delete it?"));
    }

    #[test]
    fn open_dialog_is_a_labelled_modal_with_a_named_action() {
        fn app() -> Element {
            rsx! {
                ConfirmDialog {
                    title: "Delete it?".to_string(),
                    message: "Gone for good.".to_string(),
                    open: true,
                    danger: true,
                    confirm_label: "Delete".to_string(),
                    on_confirm: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(html.contains("role=\"dialog\""), "{html}");
        assert!(html.contains("aria-modal=\"true\""), "{html}");
        assert!(
            html.contains("aria-labelledby=\"confirm-dialog-title\""),
            "{html}"
        );
        assert!(html.contains("id=\"confirm-dialog-cancel\""), "{html}");
        assert!(html.contains(">Delete</button>"), "{html}");
        assert!(!html.contains(">Confirm</button>"), "{html}");
    }

    #[test]
    fn default_label_is_still_confirm() {
        fn app() -> Element {
            rsx! {
                ConfirmDialog {
                    title: "Sure?".to_string(),
                    message: "Really.".to_string(),
                    open: true,
                    on_confirm: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        assert!(render(app).contains(">Confirm</button>"));
    }
}

#[cfg(test)]
mod list_cap_notice_tests {
    use super::*;

    #[test]
    fn cap_text_is_empty_until_the_list_is_cut_off() {
        assert_eq!(cap_notice_text(false, 40, "events"), "");
        assert_eq!(
            cap_notice_text(true, 1000, "events"),
            "Showing the first 1000 events."
        );
    }

    #[component]
    fn Probe(truncated: bool) -> Element {
        let resource = ApiResource::<Vec<u8>> {
            data: use_resource(|| async { Some(Vec::new()) }),
            refresh: use_signal(|| 0u64),
            error: use_signal(|| None),
            truncated: use_signal(|| truncated),
            shown: use_signal(|| 1000usize),
            page_budget: use_signal(|| LIST_MAX_PAGES),
        };
        list_cap_notice(&resource, "events")
    }

    fn render(truncated: bool) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, ProbeProps { truncated });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn status_line_is_always_rendered_and_load_more_sits_outside_it() {
        let idle = render(false);
        assert_eq!(
            idle,
            "<div class=\"list-cap-notice is-idle\">\
             <p class=\"list-cap-notice__text\" role=\"status\"></p></div>"
        );

        let cut = render(true);
        assert!(
            cut.contains(
                "<p class=\"list-cap-notice__text\" role=\"status\">Showing the first 1000 events.</p>\
                 <button type=\"button\" class=\"list-cap-notice__more\">Load more</button>"
            ),
            "{cut}"
        );
        assert_eq!(cut.matches("role=\"status\"").count(), 1, "{cut}");
    }
}
