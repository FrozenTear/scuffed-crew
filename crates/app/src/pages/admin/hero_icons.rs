//! Admin: hero icon counts per member (`/admin/hero-icons`).
//!
//! Icons are private training data. Officers see only how many icons each
//! opted-in member has sent. They never get the images and cannot delete.
//! Admins see the same count and can delete a member's icons. The server also
//! sends admins members who turned sharing off but still have hidden icons
//! (`opted_in` false). Those rows show a "Sharing off" label and the same
//! delete. No row ever loads an image. Officer lists are filtered by the
//! server, not here.

use dioxus::prelude::*;

use scuffed_api_client::ApiClient;
use scuffed_types::api::HeroIconMemberCount;

use crate::components::DataTable;
use crate::components::ui::{InlineConfirm, LoadError};
use crate::hero_icons_ui as text;
use crate::hooks::use_api_list;
use crate::state::use_auth;

const CSS: &str = r#"
    .hero-icons-admin__name { overflow-wrap: anywhere; word-break: break-word; }
    .hero-icons-admin__cell-confirm { min-width: 16rem; }
    .hero-icons-admin__cell-confirm .inline-confirm__title { overflow-wrap: anywhere; word-break: break-word; }
    @media (max-width: 640px) {
        .hero-icons-admin__cell-confirm { min-width: 0; width: 12rem; white-space: normal; }
        .hero-icons-admin__status ~ .data-table-scroll .data-table { min-width: 0; }
        .hero-icons-admin__status ~ .data-table-scroll .data-table th,
        .hero-icons-admin__status ~ .data-table-scroll .data-table td { padding-left: 0.5rem; padding-right: 0.5rem; }
    }
    .hero-icons-admin__off { display: inline-block; padding: 0 var(--space-2); border: 1px solid var(--border); border-radius: var(--radius-sm); font-size: var(--text-sm); color: var(--text-2); white-space: nowrap; }
    .hero-icons-admin__status { min-height: 1.5em; margin: 0 0 var(--space-3); font-size: var(--text-sm); color: var(--text-2); }
"#;

#[component]
pub fn AdminHeroIcons() -> Element {
    let auth = use_auth();
    let is_admin = auth().is_admin();
    let members = use_api_list::<HeroIconMemberCount>("/api/hero-icons/members");
    let mut refresh = members.refresh;
    let mut pending = use_signal(|| None::<String>);
    let mut busy = use_signal(|| false);
    let mut action_status = use_signal(String::new);

    let delete_for = use_callback(move |member: HeroIconMemberCount| {
        busy.set(true);
        spawn(async move {
            let path = format!("/api/hero-icons/members/{}", member.member_id);
            match ApiClient::web().delete(&path).await {
                Ok(()) => {
                    action_status.set(text::admin_deleted(&member.display_name, member.icon_count))
                }
                Err(e) => action_status.set(text::failure_text("Deleting icons", &e)),
            }
            pending.set(None);
            busy.set(false);
            refresh += 1;
        });
    });

    let status = {
        let data = members.data.read();
        let failed = members.error.read().is_some();
        if !action_status.read().is_empty() {
            action_status.read().clone()
        } else {
            match data.as_ref().and_then(|d| d.as_ref()) {
                Some(rows) => text::admin_loaded(
                    rows.len(),
                    rows.iter().filter(|r| text::sharing_is_off(r)).count(),
                ),
                None if failed => "Could not load hero icon counts.".to_string(),
                None => text::ADMIN_LOADING.to_string(),
            }
        }
    };

    let headers: Vec<&'static str> = if is_admin {
        vec!["Member", "Icons sent", "Actions"]
    } else {
        vec!["Member", "Icons sent"]
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-hero_icons-css", css: CSS }
        div { class: "admin-toolbar",
            h1 { "Hero icons" }
        }
        p { class: "empty-state", "{text::ADMIN_INTRO}" }
        p {
            id: "{text::admin_status_id()}",
            class: "hero-icons-admin__status",
            role: "status",
            "{status}"
        }

        {
            let data = members.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None if members.error.read().is_some() => rsx! {
                    LoadError {
                        message: "Could not load hero icon counts.".to_string(),
                        on_retry: move |_| refresh += 1,
                    }
                },
                None => rsx! {},
                Some(rows) if rows.is_empty() => rsx! {
                    p { class: "empty-state", "{text::ADMIN_EMPTY}" }
                },
                Some(rows) => rsx! {
                    DataTable { headers: headers.clone(),
                        for row in rows.iter() {
                            {
                                let label = text::member_label(&row.display_name);
                                let count_id = text::admin_count_id(&row.member_id);
                                let prefix = text::admin_delete_prefix(&row.member_id);
                                let trigger = format!("{prefix}-trigger");
                                let aria = text::admin_delete_aria(&row.display_name);
                                let title = text::admin_delete_title(&row.display_name, row.icon_count);
                                let sharing_off = text::sharing_is_off(row);
                                let sharing_id = text::admin_sharing_id(&row.member_id);
                                let confirm_body = if sharing_off {
                                    text::ADMIN_DELETE_BODY_HIDDEN
                                } else {
                                    text::ADMIN_DELETE_BODY
                                };
                                let confirming = pending.read().as_deref() == Some(row.member_id.as_str());
                                let open_id = row.member_id.clone();
                                let go_row = row.clone();
                                let can_delete = row.icon_count > 0;
                                rsx! {
                                    tr { key: "{row.member_id}",
                                        td { class: "hero-icons-admin__name",
                                            "{label}"
                                            if sharing_off {
                                                " "
                                                span {
                                                    id: "{sharing_id}",
                                                    class: "hero-icons-admin__off",
                                                    "{text::SHARING_OFF_LABEL}"
                                                }
                                            }
                                        }
                                        td { id: "{count_id}", "{row.icon_count}" }
                                        if is_admin {
                                            td {
                                                if confirming {
                                                    div { class: "hero-icons-admin__cell-confirm",
                                                        InlineConfirm {
                                                            id: prefix.clone(),
                                                            title: title.clone(),
                                                            body: confirm_body.to_string(),
                                                            confirm_label: "Delete".to_string(),
                                                            busy: busy(),
                                                            on_confirm: move |_| delete_for.call(go_row.clone()),
                                                            on_cancel: {
                                                                let trigger = trigger.clone();
                                                                move |_| {
                                                                    pending.set(None);
                                                                    text::focus_when_rendered(trigger.clone());
                                                                }
                                                            },
                                                        }
                                                    }
                                                } else {
                                                    div { class: "row-actions",
                                                        button {
                                                            id: "{trigger}",
                                                            class: "row-btn danger",
                                                            r#type: "button",
                                                            "aria-label": "{aria}",
                                                            disabled: busy() || !can_delete,
                                                            onclick: move |_| pending.set(Some(open_id.clone())),
                                                            "Delete"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                    {crate::components::list_cap_notice(&members, "members")}
                },
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CSS;

    #[test]
    fn name_cell_wraps_long_unbroken_names() {
        let rule = CSS
            .lines()
            .find(|l| l.trim_start().starts_with(".hero-icons-admin__name"))
            .expect("name cell rule");
        assert!(rule.contains("overflow-wrap: anywhere"), "{rule}");
        assert!(rule.contains("word-break: break-word"), "{rule}");
        let confirm = CSS
            .lines()
            .find(|l| {
                l.trim_start()
                    .starts_with(".hero-icons-admin__cell-confirm .inline-confirm__title")
            })
            .expect("confirm title rule");
        assert!(
            confirm.contains("overflow-wrap: anywhere"),
            "the confirm title repeats the name and must wrap too, buttons must not: {confirm}"
        );
        let source = include_str!("hero_icons.rs");
        assert!(
            source.contains("td { class: \"hero-icons-admin__name\","),
            "the name cell must carry the wrap class"
        );
    }
}
