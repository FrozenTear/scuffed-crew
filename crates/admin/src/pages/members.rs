use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};

use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::modal::Modal;
use scuffed_ui::components::toast::{use_toast, Toast};

use crate::api;
use crate::components::confirm_dialog::ConfirmDialog;
use crate::components::data_table::DataTable;
use crate::components::form_modal::FormModal;
use crate::components::forms::{FormField, SelectField, TextAreaField};
use crate::components::status_pill::RolePill;
use crate::state::use_admin_state;

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
    org_role: String,
    #[allow(dead_code)]
    user_id: String,
    is_active: bool,
}

#[derive(Serialize)]
struct ChangeRoleBody {
    role: String,
}

#[derive(Serialize)]
struct UpdateMemberBody {
    is_active: Option<bool>,
}

#[derive(Debug, Clone, Deserialize)]
struct ModerationAction {
    id: String,
    #[allow(dead_code)]
    member_id: String,
    action_type: String,
    reason: String,
    issued_by: String,
    expires_at: Option<String>,
    is_active: bool,
    created_at: String,
}

#[derive(Serialize)]
struct CreateModerationBody {
    member_id: String,
    action_type: String,
    reason: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    expires_at: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct AttendanceStats {
    #[allow(dead_code)]
    member_id: String,
    attended: u32,
    no_show: u32,
    excused: u32,
    total: u32,
}

#[derive(Debug, Clone, Deserialize)]
struct GameAccount {
    id: String,
    #[allow(dead_code)]
    member_id: String,
    game_id: String,
    account_name: String,
    account_id: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct Game {
    id: String,
    name: String,
}

#[derive(Serialize)]
struct UpsertGameAccountBody {
    game_id: String,
    account_name: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    account_id: Option<String>,
}

#[derive(Serialize)]
struct UpdateAvatarBody {
    avatar_url: Option<String>,
}

#[derive(Debug, Clone, Deserialize)]
struct UploadResponse {
    url: String,
}

#[component]
pub fn MembersPage() -> impl IntoView {
    let refresh = RwSignal::new(0u32);
    let toast = use_toast();
    let state = use_admin_state();

    let members = LocalResource::new(move || {
        refresh.get();
        async { api::get::<Vec<Member>>("/api/members").await.ok() }
    });

    // Role edit modal state
    let role_open = RwSignal::new(false);
    let role_member_id = RwSignal::new(String::new());
    let role_member_name = RwSignal::new(String::new());
    let role_value = RwSignal::new(String::new());
    let role_submitting = RwSignal::new(false);

    // Activate/deactivate confirm state
    let toggle_open = RwSignal::new(false);
    let toggle_id = RwSignal::new(String::new());
    let toggle_name = RwSignal::new(String::new());
    let toggle_currently_active = RwSignal::new(true);

    // Moderation modal state
    let mod_open = RwSignal::new(false);
    let mod_member_id = RwSignal::new(String::new());
    let mod_member_name = RwSignal::new(String::new());
    let mod_history = RwSignal::new(Vec::<ModerationAction>::new());
    let mod_loading = RwSignal::new(false);

    // New moderation action form state
    let mod_form_open = RwSignal::new(false);
    let mod_action_type = RwSignal::new("warning".to_string());
    let mod_reason = RwSignal::new(String::new());
    let mod_duration_days = RwSignal::new(String::new());
    let mod_submitting = RwSignal::new(false);

    // Lift confirm state
    let lift_open = RwSignal::new(false);
    let lift_id = RwSignal::new(String::new());
    let lift_action_type = RwSignal::new(String::new());

    // Attendance stats modal state
    let stats_open = RwSignal::new(false);
    let stats_member_name = RwSignal::new(String::new());
    let stats_data = RwSignal::new(Option::<AttendanceStats>::None);
    let stats_loading = RwSignal::new(false);

    let open_stats_modal = move |member_id: String, member_name: String| {
        stats_member_name.set(member_name);
        stats_data.set(None);
        stats_loading.set(true);
        stats_open.set(true);
        spawn_local(async move {
            match api::get::<AttendanceStats>(&format!("/api/members/{member_id}/attendance/stats")).await {
                Ok(data) => stats_data.set(Some(data)),
                Err(e) => toast.show(Toast::error(format!("Failed to load stats: {e}"))),
            }
            stats_loading.set(false);
        });
    };

    // Game accounts modal state
    let acct_open = RwSignal::new(false);
    let acct_member_id = RwSignal::new(String::new());
    let acct_member_name = RwSignal::new(String::new());
    let acct_list = RwSignal::new(Vec::<GameAccount>::new());
    let acct_loading = RwSignal::new(false);
    let acct_games = RwSignal::new(Vec::<Game>::new());

    // Add account form
    let acct_form_open = RwSignal::new(false);
    let acct_game_id = RwSignal::new(String::new());
    let acct_name = RwSignal::new(String::new());
    let acct_id_val = RwSignal::new(String::new());
    let acct_submitting = RwSignal::new(false);

    let open_acct_modal = move |member_id: String, member_name: String| {
        acct_member_id.set(member_id.clone());
        acct_member_name.set(member_name);
        acct_list.set(vec![]);
        acct_loading.set(true);
        acct_open.set(true);
        spawn_local(async move {
            let (accounts, games) = (
                api::get::<Vec<GameAccount>>(&format!("/api/members/{member_id}/game-accounts")).await,
                api::get::<Vec<Game>>("/api/games").await,
            );
            if let Ok(a) = accounts { acct_list.set(a); }
            if let Ok(g) = games { acct_games.set(g); }
            acct_loading.set(false);
        });
    };

    let do_add_account = move || {
        let mid = acct_member_id.get();
        let game_id = acct_game_id.get();
        let account_name = acct_name.get();
        let account_id = acct_id_val.get();

        if game_id.is_empty() || account_name.trim().is_empty() {
            toast.show(Toast::error("Game and account name required"));
            return;
        }

        let body = UpsertGameAccountBody {
            game_id,
            account_name,
            account_id: if account_id.is_empty() { None } else { Some(account_id) },
        };
        acct_submitting.set(true);
        spawn_local(async move {
            match api::put::<_, GameAccount>(&format!("/api/members/{mid}/game-accounts"), &body).await {
                Ok(_) => {
                    toast.show(Toast::success("Account added"));
                    acct_form_open.set(false);
                    acct_game_id.set(String::new());
                    acct_name.set(String::new());
                    acct_id_val.set(String::new());
                    // Refresh list
                    if let Ok(a) = api::get::<Vec<GameAccount>>(&format!("/api/members/{mid}/game-accounts")).await {
                        acct_list.set(a);
                    }
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            acct_submitting.set(false);
        });
    };

    let do_delete_account = move |member_id: String, acct_id: String| {
        spawn_local(async move {
            match api::delete(&format!("/api/members/{member_id}/game-accounts/{acct_id}")).await {
                Ok(_) => {
                    toast.show(Toast::success("Account removed"));
                    if let Ok(a) = api::get::<Vec<GameAccount>>(&format!("/api/members/{member_id}/game-accounts")).await {
                        acct_list.set(a);
                    }
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    // Avatar upload modal state
    let avatar_open = RwSignal::new(false);
    let avatar_member_id = RwSignal::new(String::new());
    let avatar_member_name = RwSignal::new(String::new());
    let avatar_uploading = RwSignal::new(false);
    let avatar_preview = RwSignal::new(Option::<String>::None);
    let avatar_file = RwSignal::new(Option::<web_sys::File>::None);

    let open_avatar_modal = move |member_id: String, member_name: String| {
        avatar_member_id.set(member_id);
        avatar_member_name.set(member_name);
        avatar_preview.set(None);
        avatar_file.set(None);
        avatar_open.set(true);
    };

    let do_upload_avatar = move || {
        let mid = avatar_member_id.get();
        let file = avatar_file.get();
        let Some(file) = file else {
            toast.show(Toast::error("Select a file first"));
            return;
        };

        if file.size() > 2_000_000.0 {
            toast.show(Toast::error("File must be under 2MB"));
            return;
        }

        avatar_uploading.set(true);
        spawn_local(async move {
            match api::upload_file::<UploadResponse>("/api/upload/avatar", file).await {
                Ok(resp) => {
                    let body = UpdateAvatarBody { avatar_url: Some(resp.url) };
                    match api::put::<_, Member>(&format!("/api/members/{mid}"), &body).await {
                        Ok(_) => {
                            toast.show(Toast::success("Avatar updated"));
                            avatar_open.set(false);
                            refresh.update(|n| *n += 1);
                        }
                        Err(e) => toast.show(Toast::error(format!("Failed to update profile: {e}"))),
                    }
                }
                Err(e) => toast.show(Toast::error(format!("Upload failed: {e}"))),
            }
            avatar_uploading.set(false);
        });
    };

    let open_mod_modal = move |member_id: String, member_name: String| {
        mod_member_id.set(member_id.clone());
        mod_member_name.set(member_name);
        mod_history.set(vec![]);
        mod_loading.set(true);
        mod_open.set(true);
        spawn_local(async move {
            match api::get::<Vec<ModerationAction>>(&format!(
                "/api/members/{member_id}/moderation"
            ))
            .await
            {
                Ok(history) => mod_history.set(history),
                Err(e) => toast.show(Toast::error(format!("Failed to load history: {e}"))),
            }
            mod_loading.set(false);
        });
    };

    let do_change_role = move || {
        let id = role_member_id.get();
        let role = role_value.get();
        role_submitting.set(true);
        spawn_local(async move {
            let body = ChangeRoleBody { role: role.clone() };
            match api::patch::<_, Member>(&format!("/api/members/{id}/role"), &body).await {
                Ok(_) => {
                    toast.show(Toast::success(format!("Role changed to {role}")));
                    role_open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            role_submitting.set(false);
        });
    };

    let do_toggle_active = move || {
        let id = toggle_id.get();
        let new_active = !toggle_currently_active.get();
        spawn_local(async move {
            let body = UpdateMemberBody {
                is_active: Some(new_active),
            };
            match api::put::<_, Member>(&format!("/api/members/{id}"), &body).await {
                Ok(_) => {
                    let msg = if new_active {
                        "Member activated"
                    } else {
                        "Member deactivated"
                    };
                    toast.show(Toast::success(msg));
                    toggle_open.set(false);
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let do_create_mod_action = move || {
        let member_id = mod_member_id.get();
        let action_type = mod_action_type.get();
        let reason = mod_reason.get();
        let duration = mod_duration_days.get();

        if reason.trim().is_empty() {
            toast.show(Toast::error("Reason is required"));
            return;
        }

        // Calculate expires_at for suspensions with a duration
        let expires_at = if action_type == "suspension" && !duration.is_empty() {
            duration.parse::<u64>().ok().map(|days| {
                let dt = chrono::Utc::now() + chrono::Duration::days(days as i64);
                dt.to_rfc3339()
            })
        } else {
            None
        };

        mod_submitting.set(true);
        spawn_local(async move {
            let body = CreateModerationBody {
                member_id: member_id.clone(),
                action_type: action_type.clone(),
                reason,
                expires_at,
            };
            match api::post::<_, serde_json::Value>("/api/moderation", &body).await {
                Ok(_) => {
                    toast.show(Toast::success(format!(
                        "{} issued",
                        action_type.replace('_', " ")
                    )));
                    mod_form_open.set(false);
                    mod_action_type.set("warning".to_string());
                    mod_reason.set(String::new());
                    mod_duration_days.set(String::new());
                    // Refresh moderation history
                    match api::get::<Vec<ModerationAction>>(&format!(
                        "/api/members/{member_id}/moderation"
                    ))
                    .await
                    {
                        Ok(history) => mod_history.set(history),
                        Err(_) => {}
                    }
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            mod_submitting.set(false);
        });
    };

    let do_lift = move || {
        let id = lift_id.get();
        let member_id = mod_member_id.get();
        spawn_local(async move {
            match api::patch::<_, serde_json::Value>(
                &format!("/api/moderation/{id}/lift"),
                &serde_json::json!({}),
            )
            .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Moderation action lifted"));
                    lift_open.set(false);
                    // Refresh moderation history
                    match api::get::<Vec<ModerationAction>>(&format!(
                        "/api/members/{member_id}/moderation"
                    ))
                    .await
                    {
                        Ok(history) => mod_history.set(history),
                        Err(_) => {}
                    }
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
        });
    };

    let is_admin = move || state.is_admin();

    let role_title = Signal::derive(move || {
        format!("Change Role \u{2014} {}", role_member_name.get())
    });

    let toggle_title = Signal::derive(move || {
        let action = if toggle_currently_active.get() {
            "Deactivate"
        } else {
            "Activate"
        };
        format!("{action} Member")
    });

    let toggle_message = Signal::derive(move || {
        let name = toggle_name.get();
        if toggle_currently_active.get() {
            format!(
                "Are you sure you want to deactivate {name}? They will lose access to all protected endpoints."
            )
        } else {
            format!("Re-activate {name}?")
        }
    });

    let toggle_danger = Signal::derive(move || toggle_currently_active.get());

    let mod_title = Signal::derive(move || {
        format!("Moderation \u{2014} {}", mod_member_name.get())
    });

    let mod_form_title = Signal::derive(move || {
        format!("New Action \u{2014} {}", mod_member_name.get())
    });

    let lift_title = Signal::derive(move || {
        format!("Lift {}", lift_action_type.get())
    });

    let lift_message = Signal::derive(move || {
        format!(
            "Are you sure you want to lift this {}? The member will regain access.",
            lift_action_type.get()
        )
    });

    view! {
        <h1>"Members"</h1>
        {move || match members.get().flatten() {
            None => view! { <p>"Loading..."</p> }.into_any(),
            Some(list) if list.is_empty() => view! { <p>"No members yet."</p> }.into_any(),
            Some(list) => view! {
                <DataTable headers=vec!["Name", "Role", "Status", "Actions"]>
                    {list.into_iter().map(|m| {
                        let active_text = if m.is_active { "Active" } else { "Inactive" };
                        let id = m.id.clone();
                        let id2 = m.id.clone();
                        let id3 = m.id.clone();
                        let id_stats = m.id.clone();
                        let id_acct = m.id.clone();
                        let id_avatar = m.id.clone();
                        let name = m.display_name.clone();
                        let name2 = m.display_name.clone();
                        let name3 = m.display_name.clone();
                        let name_stats = m.display_name.clone();
                        let name_acct = m.display_name.clone();
                        let name_avatar = m.display_name.clone();
                        let role = m.org_role.clone();
                        let current_active = m.is_active;
                        view! {
                            <tr>
                                <td>{m.display_name.clone()}</td>
                                <td><RolePill role=m.org_role.clone()/></td>
                                <td>{active_text}</td>
                                <td class="table-actions">
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| {
                                            open_stats_modal(id_stats.clone(), name_stats.clone());
                                        })
                                    >
                                        "Stats"
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| {
                                            open_acct_modal(id_acct.clone(), name_acct.clone());
                                        })
                                    >
                                        "Accounts"
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| {
                                            open_avatar_modal(id_avatar.clone(), name_avatar.clone());
                                        })
                                    >
                                        "Avatar"
                                    </Button>
                                    <Button
                                        variant=ButtonVariant::Secondary
                                        on_click=Callback::new(move |_| {
                                            open_mod_modal(id3.clone(), name3.clone());
                                        })
                                    >
                                        "Mod"
                                    </Button>
                                    {if is_admin() {
                                        Some(view! {
                                            <Button
                                                variant=ButtonVariant::Secondary
                                                on_click=Callback::new(move |_| {
                                                    role_member_id.set(id.clone());
                                                    role_member_name.set(name.clone());
                                                    role_value.set(role.clone());
                                                    role_open.set(true);
                                                })
                                            >
                                                "Edit Role"
                                            </Button>
                                            <Button
                                                variant=if current_active { ButtonVariant::Danger } else { ButtonVariant::Primary }
                                                on_click=Callback::new(move |_| {
                                                    toggle_id.set(id2.clone());
                                                    toggle_name.set(name2.clone());
                                                    toggle_currently_active.set(current_active);
                                                    toggle_open.set(true);
                                                })
                                            >
                                                {if current_active { "Deactivate" } else { "Activate" }}
                                            </Button>
                                        })
                                    } else {
                                        None
                                    }}
                                </td>
                            </tr>
                        }
                    }).collect_view()}
                </DataTable>
            }.into_any(),
        }}

        // Role edit modal
        <FormModal
            open=role_open
            on_close=Callback::new(move |_| role_open.set(false))
            title=role_title
            on_submit=Callback::new(move |_| do_change_role())
            submitting=role_submitting
        >
            <SelectField
                label="Role"
                value=role_value
                options=vec![
                    ("admin", "Admin"),
                    ("officer", "Officer"),
                    ("member", "Member"),
                    ("recruit", "Recruit"),
                ]
            />
        </FormModal>

        // Activate/deactivate confirm
        <ConfirmDialog
            open=toggle_open
            on_confirm=Callback::new(move |_| do_toggle_active())
            on_cancel=Callback::new(move |_| toggle_open.set(false))
            title=toggle_title
            message=toggle_message
            danger=toggle_danger
        />

        // Moderation history modal
        <Modal open=mod_open on_close=Callback::new(move |_| mod_open.set(false))>
            <div style="min-width: 500px; max-width: 700px;">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem;">
                    <h3 style="font-family: var(--font-display); font-size: 1.2rem; color: var(--text-bright); text-transform: uppercase; margin: 0;">
                        {move || mod_title.get()}
                    </h3>
                    <Button
                        variant=ButtonVariant::Primary
                        on_click=Callback::new(move |_| {
                            mod_action_type.set("warning".to_string());
                            mod_reason.set(String::new());
                            mod_duration_days.set(String::new());
                            mod_form_open.set(true);
                        })
                    >
                        "New Action"
                    </Button>
                </div>
                {move || {
                    if mod_loading.get() {
                        view! { <p style="color: var(--text-muted);">"Loading..."</p> }.into_any()
                    } else {
                        let history = mod_history.get();
                        if history.is_empty() {
                            view! { <p style="color: var(--text-muted);">"No moderation history."</p> }.into_any()
                        } else {
                            view! {
                                <DataTable headers=vec!["Type", "Reason", "By", "Date", "Status", "Actions"]>
                                    {history.into_iter().map(|a| {
                                        let ts = a.created_at.chars().take(10).collect::<String>();
                                        let status = if a.is_active { "Active" } else { "Lifted" };
                                        let status_style = if a.is_active {
                                            "color: #f87171;"
                                        } else {
                                            "color: #9ca3af;"
                                        };
                                        let expires = a.expires_at.as_ref().map(|e| {
                                            format!(" (exp: {})", e.chars().take(10).collect::<String>())
                                        }).unwrap_or_default();
                                        let type_display = a.action_type.replace('_', " ");
                                        let type_bg = match a.action_type.as_str() {
                                            "ban" => "background: #ef444433; color: #f87171;",
                                            "suspension" => "background: #f59e0b33; color: #fbbf24;",
                                            "warning" => "background: #7c3aed33; color: #a78bfa;",
                                            _ => "background: #6b728033; color: #9ca3af;",
                                        };
                                        let can_lift = a.is_active && (a.action_type == "suspension" || a.action_type == "ban");
                                        let aid = a.id.clone();
                                        let atype = a.action_type.clone();
                                        view! {
                                            <tr>
                                                <td><span class="status-pill" style=type_bg>{type_display}</span></td>
                                                <td style="font-size: 0.85rem; max-width: 200px; overflow: hidden; text-overflow: ellipsis;">
                                                    {a.reason}
                                                </td>
                                                <td style="font-size: 0.8rem; font-family: var(--font-mono);">{a.issued_by}</td>
                                                <td style="white-space: nowrap; font-size: 0.8rem;">{ts}{expires}</td>
                                                <td style=status_style>{status}</td>
                                                <td class="table-actions">
                                                    {if can_lift && is_admin() {
                                                        Some(view! {
                                                            <Button
                                                                variant=ButtonVariant::Ghost
                                                                on_click=Callback::new(move |_| {
                                                                    lift_id.set(aid.clone());
                                                                    lift_action_type.set(atype.clone());
                                                                    lift_open.set(true);
                                                                })
                                                            >
                                                                "Lift"
                                                            </Button>
                                                        })
                                                    } else {
                                                        None
                                                    }}
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </DataTable>
                            }.into_any()
                        }
                    }
                }}
            </div>
        </Modal>

        // New moderation action form modal
        <FormModal
            open=mod_form_open
            on_close=Callback::new(move |_| mod_form_open.set(false))
            title=mod_form_title
            on_submit=Callback::new(move |_| do_create_mod_action())
            submitting=mod_submitting
        >
            <SelectField
                label="Action Type"
                value=mod_action_type
                options=vec![
                    ("note", "Note"),
                    ("warning", "Warning"),
                    ("suspension", "Suspension"),
                    ("ban", "Ban"),
                ]
            />
            <TextAreaField label="Reason" value=mod_reason rows=3/>
            {move || {
                (mod_action_type.get() == "suspension").then(|| view! {
                    <FormField label="Duration (days)" value=mod_duration_days input_type="number"/>
                })
            }}
        </FormModal>

        // Lift confirm dialog
        <ConfirmDialog
            open=lift_open
            on_confirm=Callback::new(move |_| do_lift())
            on_cancel=Callback::new(move |_| lift_open.set(false))
            title=lift_title
            message=lift_message
            danger=false
        />

        // Attendance stats modal
        <Modal open=stats_open on_close=Callback::new(move |_| stats_open.set(false))>
            <div style="min-width: 350px;">
                <h3 style="font-family: var(--font-display); font-size: 1.2rem; color: var(--text-bright); text-transform: uppercase; margin: 0 0 1rem 0;">
                    {move || format!("Stats \u{2014} {}", stats_member_name.get())}
                </h3>
                {move || {
                    if stats_loading.get() {
                        view! { <p style="color: var(--text-muted);">"Loading..."</p> }.into_any()
                    } else {
                        match stats_data.get() {
                            None => view! { <p style="color: var(--text-muted);">"No data."</p> }.into_any(),
                            Some(s) => {
                                let rate = if s.total > 0 {
                                    format!("{:.0}%", (s.attended as f64 / s.total as f64) * 100.0)
                                } else {
                                    "\u{2014}".to_string()
                                };
                                view! {
                                    <div style="display: grid; grid-template-columns: 1fr 1fr; gap: 0.75rem;">
                                        <div style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; padding: 1rem; text-align: center;">
                                            <div style="font-size: 1.5rem; font-weight: 700; color: #4ade80;">{s.attended}</div>
                                            <div style="font-size: 0.75rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 1px;">"Attended"</div>
                                        </div>
                                        <div style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; padding: 1rem; text-align: center;">
                                            <div style="font-size: 1.5rem; font-weight: 700; color: #f87171;">{s.no_show}</div>
                                            <div style="font-size: 0.75rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 1px;">"No Show"</div>
                                        </div>
                                        <div style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; padding: 1rem; text-align: center;">
                                            <div style="font-size: 1.5rem; font-weight: 700; color: #fbbf24;">{s.excused}</div>
                                            <div style="font-size: 0.75rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 1px;">"Excused"</div>
                                        </div>
                                        <div style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 8px; padding: 1rem; text-align: center;">
                                            <div style="font-size: 1.5rem; font-weight: 700; color: var(--accent-bright);">{rate}</div>
                                            <div style="font-size: 0.75rem; color: var(--text-muted); text-transform: uppercase; letter-spacing: 1px;">"Rate"</div>
                                        </div>
                                    </div>
                                }.into_any()
                            }
                        }
                    }
                }}
            </div>
        </Modal>

        // Game accounts modal
        <Modal open=acct_open on_close=Callback::new(move |_| acct_open.set(false))>
            <div style="min-width: 450px; max-width: 600px;">
                <div style="display: flex; justify-content: space-between; align-items: center; margin-bottom: 1rem;">
                    <h3 style="font-family: var(--font-display); font-size: 1.2rem; color: var(--text-bright); text-transform: uppercase; margin: 0;">
                        {move || format!("Accounts \u{2014} {}", acct_member_name.get())}
                    </h3>
                    <Button
                        variant=ButtonVariant::Primary
                        on_click=Callback::new(move |_| {
                            acct_game_id.set(String::new());
                            acct_name.set(String::new());
                            acct_id_val.set(String::new());
                            acct_form_open.set(true);
                        })
                    >
                        "Add Account"
                    </Button>
                </div>
                {move || {
                    if acct_loading.get() {
                        view! { <p style="color: var(--text-muted);">"Loading..."</p> }.into_any()
                    } else {
                        let accounts = acct_list.get();
                        let games = acct_games.get();
                        if accounts.is_empty() {
                            view! { <p style="color: var(--text-muted);">"No game accounts."</p> }.into_any()
                        } else {
                            view! {
                                <DataTable headers=vec!["Game", "Account", "ID", "Actions"]>
                                    {accounts.into_iter().map(|a| {
                                        let game_name = games.iter()
                                            .find(|g| g.id == a.game_id)
                                            .map(|g| g.name.clone())
                                            .unwrap_or_else(|| a.game_id.clone());
                                        let acct_id_display = a.account_id.clone().unwrap_or_default();
                                        let del_mid = acct_member_id.get();
                                        let del_aid = a.id.clone();
                                        view! {
                                            <tr>
                                                <td>{game_name}</td>
                                                <td>{a.account_name}</td>
                                                <td style="font-family: var(--font-mono); font-size: 0.8rem;">{acct_id_display}</td>
                                                <td class="table-actions">
                                                    <Button
                                                        variant=ButtonVariant::Danger
                                                        on_click=Callback::new(move |_| {
                                                            do_delete_account(del_mid.clone(), del_aid.clone());
                                                        })
                                                    >
                                                        "Delete"
                                                    </Button>
                                                </td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </DataTable>
                            }.into_any()
                        }
                    }
                }}
            </div>
        </Modal>

        // Add game account form modal
        <FormModal
            open=acct_form_open
            on_close=Callback::new(move |_| acct_form_open.set(false))
            title=Signal::derive(move || format!("Add Account \u{2014} {}", acct_member_name.get()))
            on_submit=Callback::new(move |_| do_add_account())
            submitting=acct_submitting
        >
            <div>
                <label style="color: var(--text-secondary); font-size: 0.85rem;">"Game"</label>
                <select
                    style="background: var(--bg-surface); border: 1px solid var(--border); border-radius: 6px; padding: 0.5rem 0.75rem; color: var(--text-primary); width: 100%; font-size: 0.9rem;"
                    prop:value=move || acct_game_id.get()
                    on:change=move |ev| acct_game_id.set(event_target_value(&ev))
                >
                    <option value="">"Select a game..."</option>
                    {move || acct_games.get().into_iter().map(|g| {
                        view! { <option value={g.id.clone()}>{g.name}</option> }
                    }).collect_view()}
                </select>
            </div>
            <FormField label="Account Name" value=acct_name/>
            <FormField label="Account ID (optional)" value=acct_id_val/>
        </FormModal>

        // Avatar upload modal
        <Modal open=avatar_open on_close=Callback::new(move |_| avatar_open.set(false))>
            <div style="min-width: 350px;">
                <h3 style="font-family: var(--font-display); font-size: 1.2rem; color: var(--text-bright); text-transform: uppercase; margin: 0 0 1rem 0;">
                    {move || format!("Avatar \u{2014} {}", avatar_member_name.get())}
                </h3>
                {move || avatar_preview.get().map(|url| view! {
                    <div style="text-align: center; margin-bottom: 1rem;">
                        <img
                            src=url
                            style="width: 96px; height: 96px; border-radius: 50%; object-fit: cover; border: 2px solid var(--border);"
                        />
                    </div>
                })}
                <input
                    type="file"
                    accept="image/*"
                    style="color: var(--text-primary); font-size: 0.9rem; margin-bottom: 1rem;"
                    on:change=move |ev| {
                        use wasm_bindgen::JsCast;
                        let target = event_target::<web_sys::HtmlInputElement>(&ev);
                        if let Some(files) = target.files() {
                            if let Some(file) = files.item(0) {
                                let file: web_sys::File = file.dyn_into().unwrap();
                                let url = web_sys::Url::create_object_url_with_blob(&file).ok();
                                avatar_preview.set(url);
                                avatar_file.set(Some(file));
                            }
                        }
                    }
                />
                <p style="font-size: 0.75rem; color: var(--text-muted); margin-bottom: 1rem;">"Max 2MB. Accepts image files."</p>
                <div style="display: flex; justify-content: flex-end; gap: 0.75rem;">
                    <Button
                        variant=ButtonVariant::Secondary
                        on_click=Callback::new(move |_| avatar_open.set(false))
                    >
                        "Cancel"
                    </Button>
                    <Button
                        variant=ButtonVariant::Primary
                        disabled=avatar_uploading.get()
                        on_click=Callback::new(move |_| do_upload_avatar())
                    >
                        {move || if avatar_uploading.get() { "Uploading..." } else { "Upload" }}
                    </Button>
                </div>
            </div>
        </Modal>
    }
}
