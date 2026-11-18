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

use crate::components::admin_shared::{
    after_delete_focus_ids, focus_after_delete_soon, focus_after_refresh_soon,
};

use scuffed_api_client::ApiClient;
use scuffed_types::api::HeroIconMemberCount;

use crate::components::ui::{
    EmptyCard, InlineConfirm, PageHeader, PageShell, Pill, PillTone, btn_class,
};
use crate::components::{DataTable, RowActions, admin_list_pending, list_cap_notice};
use crate::hero_icons_ui as text;
use crate::hooks::use_api_list;
use crate::routes::Route;
use crate::state::use_auth;

const CSS: &str = r#"
    .hero-icons-admin__name { overflow-wrap: anywhere; word-break: break-word; }
    .hero-icons-admin__cell-confirm .inline-confirm__title { overflow-wrap: anywhere; word-break: break-word; }
    .hero-icons-admin__off { display: inline-block; margin-inline-start: var(--space-2); }
    .hero-icons-admin__status { margin: 0; font-size: var(--text-sm); line-height: var(--leading-body); color: var(--text-3); }
    /* No text: out of the layout, still in the tree (a live line
       that is display: none drops out of what screen readers track). */
    .hero-icons-admin__status:empty {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
    @media (max-width: 820px) {
        .is-cards .data-table td.cell-actions .hero-icons-admin__cell-confirm { order: 8; flex: 0 0 100%; margin-top: var(--space-3); }
    }
"#;

/// Id of the empty card's link: focus lands there when the last row goes.
const EMPTY_NEXT_ID: &str = "hero-icons-admin-empty-next";

/// Where focus goes after a confirmed delete of `removed`'s icons, in order.
/// A member who turned sharing off has no row once the icons are gone, so
/// it is the next row's Delete, else the previous row's, else the empty
/// card's link. A member who still shares keeps a row with no icons, so its
/// own Delete comes first.
fn after_icon_delete_focus_ids(keys: &[String], removed: &str, sharing_off: bool) -> Vec<String> {
    let trigger =
        |member_id: &str| vec![format!("{}-trigger", text::admin_delete_prefix(member_id))];
    let mut ids = if sharing_off {
        Vec::new()
    } else {
        trigger(removed)
    };
    ids.extend(after_delete_focus_ids(
        keys,
        removed,
        trigger,
        &[EMPTY_NEXT_ID],
    ));
    ids
}

/// The empty list's next step: where a member turns sharing on.
const EMPTY_NEXT: &str = "Counts show up here once a member turns on Share hero icons in Edit profile, under Training crops, and the tracker sends icons. You can start with your own profile.";

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
        // B9: the confirm strip, and the focus in it, goes when the delete
        // ends. Focus moves only once the refreshed list is on screen: the
        // row's count cell is gone (sharing off: the server drops the row)
        // or reads 0 (still sharing: the row stays). See
        // `after_icon_delete_focus_ids` for the order.
        let prefix = text::admin_delete_prefix(&member.member_id);
        let own_trigger = format!("{prefix}-trigger");
        let keys: Vec<String> = members
            .data
            .peek()
            .as_ref()
            .and_then(|d| d.as_ref())
            .map(|rows| rows.iter().map(|r| r.member_id.clone()).collect())
            .unwrap_or_default();
        let focus_next =
            after_icon_delete_focus_ids(&keys, &member.member_id, text::sharing_is_off(&member));
        let count_id = text::admin_count_id(&member.member_id);
        spawn(async move {
            let path = format!("/api/hero-icons/members/{}", member.member_id);
            let strip_cancel = format!("{prefix}-cancel");
            match ApiClient::web().delete(&path).await {
                Ok(()) => {
                    action_status.set(text::admin_deleted(&member.display_name, member.icon_count));
                    focus_after_refresh_soon(count_id, "0".to_string(), focus_next);
                }
                Err(e) => {
                    action_status.set(text::failure_text("Deleting icons", &e));
                    focus_after_delete_soon(strip_cancel, vec![own_trigger]);
                }
            }
            pending.set(None);
            busy.set(false);
            refresh += 1;
        });
    });

    // The always-rendered status line: the result of the last delete, or the
    // count once loaded. While loading or after a failed load it is empty and
    // `admin_pending` speaks instead.
    let status = {
        let data = members.data.read();
        if !action_status.read().is_empty() {
            action_status.read().clone()
        } else {
            match data.as_ref().and_then(|d| d.as_ref()) {
                Some(rows) if !rows.is_empty() => text::admin_loaded(
                    rows.len(),
                    rows.iter().filter(|r| text::sharing_is_off(r)).count(),
                ),
                _ => String::new(),
            }
        }
    };

    let headers: Vec<&'static str> = if is_admin {
        vec!["Member", "Icons sent", "Actions"]
    } else {
        vec!["Member", "Icons sent"]
    };

    let list = {
        let data = members.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&members, "hero icon counts"),
            Some(rows) if rows.is_empty() => rsx! {
                EmptyCard {
                    title: "No hero icons yet",
                    message: format!("{} {EMPTY_NEXT}", text::ADMIN_EMPTY),
                    actions: rsx! {
                        Link {
                            to: Route::ProfileSettings {},
                            id: EMPTY_NEXT_ID,
                            class: btn_class(false),
                            "Open Edit profile"
                        }
                    },
                }
            },
            Some(rows) => rsx! {
                DataTable {
                    headers: headers.clone(),
                    num_cols: vec![1],
                    cards: true,
                    label: "Hero icons per member",
                    for row in rows.iter() {
                        // Keyed by the member record id: an unkeyed body made
                        // Dioxus rebuild every row after a removed one, which
                        // dropped the focused Delete.
                        Fragment { key: "{row.member_id}", {
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
                            let off = busy() || !can_delete;
                            rsx! {
                                tr { key: "{row.member_id}",
                                    td { class: "cell-who hero-icons-admin__name",
                                        "{label}"
                                        if sharing_off {
                                            span { id: "{sharing_id}", class: "hero-icons-admin__off",
                                                Pill { tone: PillTone::Neutral, "{text::SHARING_OFF_LABEL}" }
                                            }
                                        }
                                    }
                                    td { id: "{count_id}", class: "num", "data-label": "Icons sent", "{row.icon_count}" }
                                    if is_admin {
                                        td { class: "cell-actions",
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
                                                RowActions {
                                                    button {
                                                        id: "{trigger}",
                                                        class: "btn is-danger",
                                                        r#type: "button",
                                                        aria_label: "{aria}",
                                                        aria_disabled: if off { "true" },
                                                        onclick: move |_| {
                                                            if !off {
                                                                pending.set(Some(open_id.clone()));
                                                            }
                                                        },
                                                        "Delete"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        } }
                    }
                }
            },
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-hero_icons-css", css: CSS }
        PageShell { class: "admin-hero-icons-page",
            PageHeader { title: "Hero icons", lead: text::ADMIN_INTRO.to_string() }
            p {
                id: "{text::admin_status_id()}",
                class: "hero-icons-admin__status",
                role: "status",
                "{status}"
            }
            section { class: "admin-list", aria_label: "Hero icons per member", {list} }
            {list_cap_notice(&members, "members")}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::CSS;
    use crate::components::ui::test_support::assert_sizes_from_tokens;

    #[test]
    fn a_confirmed_delete_focuses_after_the_refreshed_list_renders() {
        // Vision S4 blocking 1 on PR 706: focus moved when the confirm strip
        // went, before the refresh, and the unkeyed loop then rebuilt the
        // rows after the removed one, so focus fell to the body.
        use super::{EMPTY_NEXT_ID, after_icon_delete_focus_ids};
        let keys: Vec<String> = ["a", "b", "c"].iter().map(|k| k.to_string()).collect();
        let t = |k: &str| format!("hero-icons-admin-{k}-delete-trigger");
        // Sharing off: the row goes. Next, else previous, else the empty card.
        assert_eq!(
            after_icon_delete_focus_ids(&keys, "b", true),
            [t("c"), t("a"), EMPTY_NEXT_ID.to_string()]
        );
        assert_eq!(
            after_icon_delete_focus_ids(&keys, "c", true),
            [t("b"), EMPTY_NEXT_ID.to_string()]
        );
        let only = vec!["a".to_string()];
        assert_eq!(
            after_icon_delete_focus_ids(&only, "a", true),
            [EMPTY_NEXT_ID.to_string()]
        );
        // Still sharing: the row stays with 0 icons, so its own Delete first.
        assert_eq!(
            after_icon_delete_focus_ids(&keys, "b", false),
            [t("b"), t("c"), t("a"), EMPTY_NEXT_ID.to_string()]
        );

        let src = include_str!("hero_icons.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        // The loop body is keyed by the member record id.
        let lp = &prod[prod.find("for row in rows.iter() {").expect("loop")..];
        let body = lp
            .lines()
            .skip(1)
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with("//"))
            .unwrap();
        assert!(body.starts_with("Fragment {"), "{body}");
        assert!(lp.contains(r#"key: "{row.member_id}","#), "{lp}");
        // Success waits on the row's count cell, not the confirm strip.
        let ok = &prod[prod.find("Ok(()) => {").unwrap()..prod.find("Err(e) => {").unwrap()];
        assert!(
            ok.contains(r#"focus_after_refresh_soon(count_id, "0".to_string(), focus_next);"#),
            "{ok}"
        );
        assert!(!ok.contains("strip_cancel"), "{ok}");
        // The empty card's link carries the fallback id.
        assert!(prod.contains("id: EMPTY_NEXT_ID,"));
    }

    #[test]
    fn empty_status_line_stays_in_the_tree() {
        // Vision nit 5 on PR 706: `display: none` dropped the live line.
        crate::styles::admin::assert_live_line_kept(
            super::CSS,
            ".hero-icons-admin__status:empty {",
        );
    }

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
            source.contains("td { class: \"cell-who hero-icons-admin__name\","),
            "the name cell must carry the wrap class"
        );
    }

    #[test]
    fn page_css_sizes_come_from_tokens() {
        assert_sizes_from_tokens(CSS);
    }

    #[test]
    fn page_has_one_h1_a_status_line_and_a_labelled_wait_in_the_default_shell() {
        use crate::components::admin_shared::test_page::render_admin_route;
        for role in [
            scuffed_types::OrgRole::Officer,
            scuffed_types::OrgRole::Admin,
        ] {
            let html = render_admin_route("/admin/hero-icons", Some(role));
            assert!(
                html.contains("class=\"ui-shell admin-hero-icons-page\""),
                "{html}"
            );
            assert_eq!(html.matches("<h1").count(), 1, "{html}");
            assert!(html.contains("<h1>Hero icons</h1>"), "{html}");
            assert_eq!(html.matches("<main").count(), 1, "{html}");
            // The status line is in the tree, empty, before anything loads.
            assert!(
                html.contains(
                    "<p id=\"hero-icons-admin-status\" class=\"hero-icons-admin__status\" role=\"status\"></p>"
                ),
                "{html}"
            );
            assert!(
                html.contains("role=\"status\">Loading hero icon counts...</p>"),
                "{html}"
            );
        }
    }
}
