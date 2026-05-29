use dioxus::prelude::*;

#[component]
pub fn DataTable(headers: Vec<&'static str>, children: Element) -> Element {
    rsx! {
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
    rsx! { span { class: "{class}", "{status}" } }
}

#[component]
pub fn RolePill(role: String) -> Element {
    let class = format!("role-pill {role}");
    rsx! { span { class: "{class}", "{role}" } }
}

#[component]
pub fn SummaryCard(value: String, label: &'static str) -> Element {
    rsx! {
        div { class: "summary-card",
            div { class: "value", "{value}" }
            div { class: "label", "{label}" }
        }
    }
}
