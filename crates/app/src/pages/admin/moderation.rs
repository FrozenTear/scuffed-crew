use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{
    AccessDenied, ConfirmDialog, DataTable, FormModal, StatusPill, Toast, admin_pending, use_toast,
};
use crate::hooks::{ModalController, use_api, use_api_list};
use crate::state::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::api::CreateModerationRequest;

// Server returns raw ids; names are optional (enriched when present).
#[derive(Debug, Clone, Deserialize)]
struct ModerationAction {
    id: String,
    #[allow(dead_code)]
    member_id: String,
    #[serde(default)]
    member_name: Option<String>,
    #[serde(deserialize_with = "action_type_string")]
    action_type: String,
    reason: String,
    #[serde(default)]
    issued_by: Option<String>,
    #[serde(default)]
    issued_by_name: Option<String>,
    is_active: bool,
    created_at: String,
    #[allow(dead_code)]
    #[serde(default)]
    expires_at: Option<String>,
    #[allow(dead_code)]
    #[serde(default)]
    lifted_at: Option<String>,
}

fn action_type_string<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: serde::Deserializer<'de>,
{
    use serde::Deserialize;
    // Accept string or object/enum-like value via JSON Value
    let v = serde_json::Value::deserialize(deserializer)?;
    Ok(match v {
        serde_json::Value::String(s) => s,
        other => other
            .as_str()
            .map(|s| s.to_string())
            .unwrap_or_else(|| other.to_string().trim_matches('"').to_string()),
    })
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
    let auth = use_auth();
    let mut actions = use_api::<ModerationResponse>("/api/moderation?limit=50&offset=0");
    let members = use_api_list::<Member>("/api/members");
    let mut toast = use_toast();

    // Create modal state
    let mut modal = ModalController::<()>::new();
    let mut f_member_id = use_signal(String::new);
    let mut f_action_type = use_signal(|| "warning".to_string());
    let mut f_reason = use_signal(String::new);
    let mut f_duration = use_signal(String::new);

    // Lift dialog state
    let mut lift_modal = ModalController::<String>::new();

    let open_create = move |_| {
        f_member_id.set(String::new());
        f_action_type.set("warning".to_string());
        f_reason.set(String::new());
        f_duration.set(String::new());
        modal.show_empty();
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

        modal.start_submit();
        spawn(async move {
            let expires_at = if duration.is_empty() {
                None
            } else {
                Some(duration)
            };
            let payload = CreateModerationRequest {
                member_id,
                action_type,
                reason,
                expires_at,
            };

            match ApiClient::web()
                .post_json::<_, ModerationAction>("/api/moderation", &payload)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Moderation action created"));
                    modal.close();
                    actions.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to create: {e}"))),
            }
            modal.end_submit();
        });
    };

    let confirm_lift = move |_| {
        let id = lift_modal.get_target().unwrap_or_default();
        lift_modal.close();
        spawn(async move {
            let path = format!("/api/moderation/{id}/lift");
            match ApiClient::web()
                .patch_json_empty(&path, &serde_json::json!({}))
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Action lifted"));
                    actions.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to lift: {e}"))),
            }
        });
    };

    if !auth().is_officer_or_above() {
        return rsx! {
            AccessDenied { message: "You need officer permissions to view moderation.".to_string() }
        };
    }

    rsx! {

        div { class: "admin-toolbar",
            h1 { "Moderation" }
            button { class: "btn-add", onclick: open_create, "+ New Action" }
        }

        {
            let data = actions.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&actions, "moderation actions"),
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
                                        td {
                                            {
                                                let label = action
                                                    .member_name
                                                    .clone()
                                                    .unwrap_or_else(|| action.member_id.clone());
                                                rsx! { "{label}" }
                                            }
                                        }
                                        td { StatusPill { status: action.action_type.clone() } }
                                        td { "{action.reason}" }
                                        td {
                                            {
                                                let label = action
                                                    .issued_by_name
                                                    .clone()
                                                    .or_else(|| action.issued_by.clone())
                                                    .unwrap_or_else(|| "—".into());
                                                rsx! { "{label}" }
                                            }
                                        }
                                        td { StatusPill { status: active_label.to_string() } }
                                        td { "{date}" }
                                        td {
                                            if is_active {
                                                div { class: "row-actions",
                                                    button {
                                                        class: "row-btn danger",
                                                        onclick: move |_| lift_modal.show(id.clone()),
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
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            on_close: move |_| modal.close(),
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
                    option { value: "note", "Note" }
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
            open: lift_modal.is_open(),
            danger: false,
            on_confirm: confirm_lift,
            on_cancel: move |_| lift_modal.close(),
        }
    }
}
