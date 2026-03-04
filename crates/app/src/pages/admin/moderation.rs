use dioxus::prelude::*;
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::CreateModerationRequest;
use crate::components::{DataTable, FormModal, ConfirmDialog, StatusPill, Toast, use_toast, ADMIN_SHARED_CSS};
use crate::hooks::use_api;

// Local response types with API-enriched fields (joined names, computed fields).
#[derive(Debug, Clone, Deserialize)]
struct ModerationAction {
    id: String,
    #[allow(dead_code)]
    member_id: String,
    member_name: String,
    action_type: String,
    reason: String,
    issued_by_name: String,
    is_active: bool,
    created_at: String,
    #[allow(dead_code)]
    expires_at: Option<String>,
    #[allow(dead_code)]
    lifted_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModerationResponse {
    entries: Vec<ModerationAction>,
    #[allow(dead_code)]
    total: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
}

#[component]
pub fn AdminModeration() -> Element {
    let mut actions = use_api::<ModerationResponse>("/api/moderation?limit=50&offset=0");
    let members = use_api::<Vec<Member>>("/api/members");
    let mut toast = use_toast();

    // Create modal state
    let mut modal_open = use_signal(|| false);
    let mut submitting = use_signal(|| false);
    let mut f_member_id = use_signal(String::new);
    let mut f_action_type = use_signal(|| "warning".to_string());
    let mut f_reason = use_signal(String::new);
    let mut f_duration = use_signal(String::new);

    // Lift dialog state
    let mut lift_id = use_signal(|| None::<String>);

    let open_create = move |_| {
        f_member_id.set(String::new());
        f_action_type.set("warning".to_string());
        f_reason.set(String::new());
        f_duration.set(String::new());
        modal_open.set(true);
    };

    let on_submit = move |_| {
        let member_id = f_member_id().clone();
        let action_type = f_action_type().clone();
        let reason = f_reason().clone();
        let duration = f_duration().clone();

        if member_id.is_empty() || reason.is_empty() {
            toast.show(Toast::error("Member and reason are required"));
            return;
        }

        submitting.set(true);
        spawn(async move {
            let expires_at = if duration.is_empty() { None } else { Some(duration) };
            let payload = CreateModerationRequest {
                member_id,
                action_type,
                reason,
                expires_at,
            };

            match ApiClient::web().post_json::<_, ModerationAction>("/api/moderation", &payload).await {
                Ok(_) => {
                    toast.show(Toast::success("Moderation action created"));
                    modal_open.set(false);
                    actions.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to create: {e}"))),
            }
            submitting.set(false);
        });
    };

    let confirm_lift = move |_| {
        let id = lift_id().clone().unwrap_or_default();
        lift_id.set(None);
        spawn(async move {
            let path = format!("/api/moderation/{id}/lift");
            match ApiClient::web().patch_json_empty(&path, &serde_json::json!({})).await {
                Ok(_) => {
                    toast.show(Toast::success("Action lifted"));
                    actions.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to lift: {e}"))),
            }
        });
    };

    rsx! {
        style { {ADMIN_SHARED_CSS} }

        h1 { "Moderation" }

        div { class: "admin-toolbar",
            span {}
            button { class: "btn-add", onclick: open_create, "+ New Action" }
        }

        {
            let data = actions.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => rsx! { p { class: "admin-loading", "Loading..." } },
                Some(resp) if resp.entries.is_empty() => rsx! {
                    p { class: "empty-state", "No moderation actions." }
                },
                Some(resp) => rsx! {
                    DataTable { headers: vec!["Member", "Type", "Reason", "Issued By", "Active", "Date", "Actions"],
                        for action in resp.entries.iter() {
                            {
                                let id = action.id.clone();
                                let date: String = action.created_at.chars().take(10).collect();
                                let active_label = if action.is_active { "active" } else { "inactive" };
                                let is_active = action.is_active;
                                rsx! {
                                    tr { key: "{id}",
                                        td { "{action.member_name}" }
                                        td { StatusPill { status: action.action_type.clone() } }
                                        td { "{action.reason}" }
                                        td { "{action.issued_by_name}" }
                                        td { StatusPill { status: active_label.to_string() } }
                                        td { "{date}" }
                                        td {
                                            if is_active {
                                                div { class: "row-actions",
                                                    button {
                                                        class: "row-btn danger",
                                                        onclick: move |_| lift_id.set(Some(id.clone())),
                                                        "Lift"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                },
            }
        }

        FormModal {
            title: "New Moderation Action".to_string(),
            open: modal_open(),
            submitting: submitting(),
            on_close: move |_| modal_open.set(false),
            on_submit: on_submit,

            div { class: "form-field",
                label { class: "form-label", "Member" }
                select {
                    class: "form-select",
                    value: "{f_member_id}",
                    onchange: move |e| f_member_id.set(e.value()),
                    option { value: "", "-- Select Member --" }
                    {
                        let data = members.data.read();
                        let data = data.as_ref().and_then(|d| d.as_ref());
                        match data {
                            Some(list) => rsx! {
                                for m in list.iter() {
                                    option { value: "{m.id}", "{m.display_name}" }
                                }
                            },
                            None => rsx! {},
                        }
                    }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Action Type" }
                select {
                    class: "form-select",
                    value: "{f_action_type}",
                    onchange: move |e| f_action_type.set(e.value()),
                    option { value: "warning", "Warning" }
                    option { value: "suspension", "Suspension" }
                    option { value: "ban", "Ban" }
                }
            }
            div { class: "form-field",
                label { class: "form-label", "Reason" }
                textarea {
                    class: "form-textarea",
                    value: "{f_reason}",
                    oninput: move |e| f_reason.set(e.value()),
                }
            }
            if f_action_type() == "suspension" {
                div { class: "form-field",
                    label { class: "form-label", "Duration (e.g. 2026-04-01T00:00:00Z)" }
                    input {
                        class: "form-input",
                        r#type: "text",
                        placeholder: "Expiry datetime (ISO 8601)",
                        value: "{f_duration}",
                        oninput: move |e| f_duration.set(e.value()),
                    }
                }
            }
        }

        ConfirmDialog {
            title: "Lift Action".to_string(),
            message: "Are you sure you want to lift this moderation action?".to_string(),
            open: lift_id().is_some(),
            danger: false,
            on_confirm: confirm_lift,
            on_cancel: move |_| lift_id.set(None),
        }
    }
}
