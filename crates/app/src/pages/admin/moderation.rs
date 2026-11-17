use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, PageHeader, PageShell, PageWidth, Pager, Pill, PillTone, show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AccessDenied, AdminDate, ConfirmDialog, DataTable, FormModal, RowActions,
    Toast, admin_list_pending, list_cap_notice, status_label, use_toast,
};
use crate::hooks::{ModalController, use_api_list, use_api_with};
use crate::state::use_auth;
use scuffed_api_client::ApiClient;
use scuffed_types::api::CreateModerationRequest;

// Server returns raw ids; names are optional (enriched when present).
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct ModerationAction {
    id: String,
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
    #[serde(default)]
    expires_at: Option<String>,
    #[serde(default)]
    lifted_at: Option<String>,
}

impl ModerationAction {
    /// The member's name: the server's, else the name from the full member
    /// list (banned and deactivated members included, which the server's own
    /// lookup misses), never the record id.
    fn member_label(&self, names: &[(String, String)]) -> String {
        self.member_name
            .clone()
            .filter(|n| !n.trim().is_empty())
            .or_else(|| name_for(names, &self.member_id))
            .unwrap_or_else(|| "Unknown member".to_string())
    }

    fn issuer_label(&self) -> String {
        self.issued_by_name
            .clone()
            .or_else(|| self.issued_by.clone())
            .unwrap_or_else(|| "Unknown".into())
    }
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

/// `member:abc` and `abc` are the same record.
fn bare_id(id: &str) -> &str {
    id.trim().strip_prefix("member:").unwrap_or(id.trim())
}

fn name_for(names: &[(String, String)], member_id: &str) -> Option<String> {
    names
        .iter()
        .find(|(id, _)| bare_id(id) == bare_id(member_id))
        .map(|(_, name)| name.clone())
        .filter(|n| !n.trim().is_empty())
}

#[derive(Debug, Clone, Deserialize)]
struct ModerationResponse {
    entries: Vec<ModerationAction>,
    total: u64,
}

#[derive(Debug, Clone, Deserialize)]
struct Member {
    id: String,
    display_name: String,
}

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-moderation-list";
const NEW_ACTION_ID: &str = "moderation-new";
const FIELD_MEMBER_ID: &str = "moderation-new-member";
const FIELD_TYPE_ID: &str = "moderation-new-type";
const FIELD_REASON_ID: &str = "moderation-new-reason";
const FIELD_EXPIRES_ID: &str = "moderation-new-expires";

/// `GET` path of one page. The server pages by offset and returns the total.
fn moderation_path(page: usize) -> String {
    let offset = page.saturating_sub(1) * ADMIN_PAGE_SIZE;
    format!("/api/moderation?limit={ADMIN_PAGE_SIZE}&offset={offset}")
}

/// Tone of an action type. The pill always carries the word.
fn action_tone(action_type: &str) -> PillTone {
    match action_type {
        "ban" => PillTone::Danger,
        "suspension" | "warning" => PillTone::Warn,
        _ => PillTone::Neutral,
    }
}

/// Whether an action still applies, as a word: active, lifted by someone, or
/// ended on its own. The server clears `is_active` only on Lift; an action
/// whose end date has passed stays `is_active` and simply stops counting
/// (its checks add `expires_at > time::now()`). So an inactive action was
/// lifted, and an active one past its end date has ended. Active is info, so
/// it never reads as the warn tone of the Warning and Suspension type pills.
fn action_state(
    action: &ModerationAction,
    now: chrono::DateTime<chrono::Utc>,
) -> (&'static str, PillTone) {
    if !action.is_active || action.lifted_at.is_some() {
        return ("Lifted", PillTone::Neutral);
    }
    let ended = action
        .expires_at
        .as_deref()
        .and_then(|t| chrono::DateTime::parse_from_rfc3339(t).ok())
        .is_some_and(|end| end.with_timezone(&chrono::Utc) <= now);
    if ended {
        ("Ended", PillTone::Neutral)
    } else {
        ("Active", PillTone::Info)
    }
}

/// Id of the Lift button of one action (focus returns there on Cancel).
fn lift_button_id(action_id: &str) -> String {
    format!("moderation-lift-{action_id}")
}

/// One page of actions. On phone every row is a card: the member and Lift on
/// top, the type pill under the name, then state, issuer and date on one
/// line, and the reason as a labelled line.
#[component]
fn ModerationTable(
    rows: Vec<ModerationAction>,
    /// (member id, display name) of every member, inactive ones included.
    #[props(default)]
    names: Vec<(String, String)>,
    can_lift: bool,
    on_lift: EventHandler<String>,
) -> Element {
    let now = chrono::Utc::now();
    rsx! {
        DataTable {
            headers: vec!["Member", "Type", "Reason", "State", "Issued by", "Date", "Actions"],
            num_cols: vec![5],
            cards: true,
            label: "Moderation actions",
            for action in rows.iter() {
                {
                    let id = action.id.clone();
                    let member = action.member_label(&names);
                    let issuer = action.issuer_label();
                    let (state_word, state_tone) = action_state(action, now);
                    // Only an action that still applies can be lifted.
                    let liftable = can_lift && state_word == "Active";
                    rsx! {
                        tr { key: "{id}",
                            td { class: "cell-who", "{member}" }
                            td { class: "cell-badge",
                                Pill { tone: action_tone(&action.action_type), "{status_label(&action.action_type)}" }
                            }
                            td { "data-label": "Reason", "{action.reason}" }
                            td { class: "cell-meta", Pill { tone: state_tone, "{state_word}" } }
                            td { class: "cell-meta", "{issuer}" }
                            td { class: "cell-meta num", AdminDate { iso: action.created_at.clone() } }
                            td { class: "cell-actions",
                                if liftable {
                                    RowActions {
                                        button {
                                            r#type: "button",
                                            id: lift_button_id(&id),
                                            class: "btn is-ghost",
                                            aria_label: "Lift {status_label(&action.action_type)} for {member}",
                                            onclick: move |_| on_lift.call(id.clone()),
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
    }
}

#[component]
pub fn AdminModeration() -> Element {
    let auth = use_auth();
    let mut page = use_signal(|| 1usize);
    let mut actions = use_api_with::<ModerationResponse>(move || moderation_path(page()));
    let members = use_api_list::<Member>("/api/members");
    // Names for the list, inactive members included: a ban deactivates, so
    // the people this page is mostly about are not in the active list.
    let all_members = use_api_list::<Member>("/api/members?include_inactive=true");
    let mut toast = use_toast();

    // Create modal state
    let mut modal = ModalController::<()>::new();
    let mut f_member_id = use_signal(String::new);
    let mut f_action_type = use_signal(|| "warning".to_string());
    let mut f_reason = use_signal(String::new);
    let mut f_duration = use_signal(String::new);
    let mut create_error: Signal<Option<String>> = use_signal(|| None);

    // Lift dialog state
    let mut lift_modal = ModalController::<String>::new();

    let mut open_create = move || {
        f_member_id.set(String::new());
        f_action_type.set("warning".to_string());
        f_reason.set(String::new());
        f_duration.set(String::new());
        create_error.set(None);
        modal.show_empty();
    };

    let on_submit = move |_| {
        let member_id = f_member_id().clone();
        let action_type = f_action_type().clone();
        let reason = f_reason().clone();
        let duration = f_duration().clone();

        if member_id.is_empty() || reason.is_empty() {
            let message = "Member and reason are required.";
            create_error.set(Some(message.to_string()));
            toast.show(Toast::error(message));
            return;
        }

        create_error.set(None);
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
                Err(e) => {
                    let message = format!("Failed to create: {e}");
                    create_error.set(Some(message.clone()));
                    toast.show(Toast::error(message));
                }
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

    let is_admin = auth().is_admin();

    if !auth().is_officer_or_above() {
        return rsx! {
            AccessDenied { message: "You need officer permissions to view moderation.".to_string() }
        };
    }

    let names: Vec<(String, String)> = all_members
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(|list| {
            list.iter()
                .map(|m| (m.id.clone(), m.display_name.clone()))
                .collect()
        })
        .unwrap_or_default();
    // With no actions at all, the empty card holds the one New action.
    let list_empty = actions
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .is_some_and(|r| r.total == 0);

    let list = {
        let data = actions.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&actions, "moderation actions"),
            Some(resp) => {
                let total = usize::try_from(resp.total).unwrap_or(usize::MAX);
                let rows = resp.entries.clone();
                rsx! {
                    if total == 0 {
                        EmptyCard {
                            title: "No moderation actions yet",
                            message: "Notes, warnings, suspensions and bans you record show up here. Use New action to record one.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    id: NEW_ACTION_ID,
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(),
                                    "New action"
                                }
                            },
                        }
                    } else if rows.is_empty() {
                        EmptyCard {
                            title: "Nothing on this page",
                            message: "The list got shorter since this page loaded. Go back to the first page.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    onclick: move |_| page.set(1),
                                    "First page"
                                }
                            },
                        }
                    } else {
                        ModerationTable {
                            rows,
                            names: names.clone(),
                            can_lift: is_admin,
                            on_lift: move |id: String| lift_modal.show(id),
                        }
                    }
                    // Always rendered: its count line is the list's live line.
                    Pager {
                        id: "moderation-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "actions",
                        noun_singular: "action",
                        show_page_count: true,
                        on_change: move |next| {
                            page.set(next);
                            show_list_top(LIST_ID);
                        },
                    }
                }
            }
        }
    };

    let lift_focus = lift_modal.get_target().map(|id| lift_button_id(&id));

    rsx! {
        PageShell { width: PageWidth::Wide, class: "admin-moderation-page",
            PageHeader {
                title: "Moderation",
                lead: "Notes, warnings, suspensions and bans, newest first. Admins can lift an active action.",
                actions: rsx! {
                    if !list_empty {
                        button {
                            r#type: "button",
                            id: NEW_ACTION_ID,
                            class: "btn is-primary",
                            onclick: move |_| open_create(),
                            "New action"
                        }
                    }
                },
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Moderation actions",
                {list}
            }

            {list_cap_notice(&members, "members")}
        }

        FormModal {
            title: "New moderation action".to_string(),
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            submit_label: Some("Create".to_string()),
            error: create_error(),
            restore_focus_id: Some(NEW_ACTION_ID.to_string()),
            on_close: move |_| modal.close(),
            on_submit: on_submit,

            NewActionFields {
                members: members
                    .data
                    .read()
                    .as_ref()
                    .and_then(|d| d.as_ref())
                    .map(|list| list.iter().map(|m| (m.id.clone(), m.display_name.clone())).collect())
                    .unwrap_or_default(),
                member_id: f_member_id(),
                action_type: f_action_type(),
                reason: f_reason(),
                duration: f_duration(),
                on_member: move |v| f_member_id.set(v),
                on_type: move |v| f_action_type.set(v),
                on_reason: move |v| f_reason.set(v),
                on_duration: move |v| f_duration.set(v),
            }
        }

        ConfirmDialog {
            title: "Lift action".to_string(),
            message: "Are you sure you want to lift this moderation action?".to_string(),
            open: lift_modal.is_open(),
            danger: false,
            confirm_label: "Lift".to_string(),
            restore_focus_id: lift_focus,
            on_confirm: confirm_lift,
            on_cancel: move |_| lift_modal.close(),
        }
    }
}

/// Fields of the new action dialog. Every control has a visible label tied
/// by a fixed page-unique id.
#[component]
fn NewActionFields(
    /// (id, display name) of the members to choose from.
    members: Vec<(String, String)>,
    member_id: String,
    action_type: String,
    reason: String,
    duration: String,
    on_member: EventHandler<String>,
    on_type: EventHandler<String>,
    on_reason: EventHandler<String>,
    on_duration: EventHandler<String>,
) -> Element {
    let expires_hint = format!("{FIELD_EXPIRES_ID}-hint");
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: FIELD_MEMBER_ID, "Member" }
            select {
                class: "field-input",
                id: FIELD_MEMBER_ID,
                value: "{member_id}",
                onchange: move |e| on_member.call(e.value()),
                option { value: "", selected: member_id.is_empty(), "Choose a member" }
                for (id, name) in members.iter() {
                    option { value: "{id}", selected: *id == member_id, "{name}" }
                }
            }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: FIELD_TYPE_ID, "Action type" }
            select {
                class: "field-input",
                id: FIELD_TYPE_ID,
                value: "{action_type}",
                onchange: move |e| on_type.call(e.value()),
                for (value, word) in [("note", "Note"), ("warning", "Warning"), ("suspension", "Suspension"), ("ban", "Ban")] {
                    option { value: "{value}", selected: action_type == value, "{word}" }
                }
            }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: FIELD_REASON_ID, "Reason" }
            textarea {
                class: "field-input",
                id: FIELD_REASON_ID,
                value: "{reason}",
                oninput: move |e| on_reason.call(e.value()),
            }
        }
        if action_type == "suspension" {
            div { class: "form-field",
                label { class: "field-label", r#for: FIELD_EXPIRES_ID, "Suspension ends" }
                input {
                    class: "field-input",
                    id: FIELD_EXPIRES_ID,
                    r#type: "text",
                    placeholder: "2026-04-01T00:00:00Z",
                    aria_describedby: "{expires_hint}",
                    value: "{duration}",
                    oninput: move |e| on_duration.call(e.value()),
                }
                p { class: "field-hint", id: "{expires_hint}",
                    "Date and time in UTC, for example 2026-04-01T00:00:00Z. Leave it empty for no end date."
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn action(id: &str, kind: &str, active: bool, lifted: bool) -> ModerationAction {
        ModerationAction {
            id: id.into(),
            member_id: format!("member-{id}"),
            member_name: Some("Player01".into()),
            action_type: kind.into(),
            reason: "Spam in chat".into(),
            issued_by: Some("user-x".into()),
            issued_by_name: Some("Player02".into()),
            is_active: active,
            created_at: "2026-11-10T12:00:00Z".into(),
            expires_at: None,
            lifted_at: lifted.then(|| "2026-11-11T12:00:00Z".into()),
        }
    }

    #[test]
    fn page_has_one_h1_a_new_action_button_and_a_labelled_wait() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/moderation", Some(scuffed_types::OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-moderation-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Moderation</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains("id=\"moderation-new\""), "{html}");
        assert!(
            html.contains(
                "<section id=\"admin-moderation-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Moderation actions\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading moderation actions...</p>"),
            "{html}"
        );
    }

    #[test]
    fn non_officers_get_the_no_access_page() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/moderation", Some(scuffed_types::OrgRole::Member));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(!html.contains("admin-moderation-page"), "{html}");
    }

    #[test]
    fn rows_are_cards_with_word_pills_and_lift_only_when_active() {
        fn view() -> Element {
            rsx! {
                ModerationTable {
                    rows: vec![action("a1", "ban", true, false), action("a2", "warning", false, true)],
                    can_lift: true,
                    on_lift: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Date</th>"),
            "{html}"
        );
        let (first, second) = html.split_once("</tr><tr").expect("two rows");
        assert!(
            first.contains("class=\"ui-pill ui-pill--danger\">Ban</span>"),
            "{first}"
        );
        assert!(first.contains(">Active</span>"), "{first}");
        assert!(first.contains("data-label=\"Reason\""), "{first}");
        assert!(first.contains(">Player02</td>"), "{first}");
        assert!(
            first.contains("id=\"moderation-lift-a1\" class=\"btn is-ghost\" aria-label=\"Lift Ban for Player01\""),
            "{first}"
        );
        assert!(second.contains(">Lifted</span>"), "{second}");
        assert!(!second.contains(">Lift</button>"), "{second}");
        assert!(
            first.contains("class=\"ui-pill ui-pill--info\">Active</span>"),
            "{first}"
        );
    }

    #[test]
    fn an_active_action_past_its_end_reads_ended_with_no_lift() {
        fn view() -> Element {
            let mut ended = action("a3", "suspension", true, false);
            ended.expires_at = Some("2020-01-01T00:00:00Z".into());
            rsx! {
                ModerationTable { rows: vec![ended], can_lift: true, on_lift: |_| {} }
            }
        }
        let html = render(view);
        assert!(html.contains(">Ended</span>"), "{html}");
        assert!(!html.contains(">Active</span>"), "{html}");
        assert!(!html.contains(">Lift</button>"), "{html}");
    }

    #[test]
    fn state_comes_from_what_the_api_sends() {
        let now = chrono::DateTime::parse_from_rfc3339("2026-11-17T00:00:00Z")
            .unwrap()
            .with_timezone(&chrono::Utc);
        // The real payload has no lifted_at.
        let mut lifted = action("a", "warning", false, false);
        assert_eq!(action_state(&lifted, now).0, "Lifted");
        lifted.expires_at = Some("2026-12-01T00:00:00Z".into());
        assert_eq!(action_state(&lifted, now).0, "Lifted");
        // Inactive is always a Lift: the server clears `is_active` only
        // there (Vision nit 9 on PR 706), even when the end date has passed.
        let mut lifted_late = action("b", "suspension", false, false);
        lifted_late.expires_at = Some("2026-11-10T00:00:00Z".into());
        assert_eq!(action_state(&lifted_late, now).0, "Lifted");
        // An active action past its end date has ended on its own.
        let mut ended = action("b", "suspension", true, false);
        ended.expires_at = Some("2026-11-10T00:00:00Z".into());
        assert_eq!(action_state(&ended, now), ("Ended", PillTone::Neutral));
        let mut running = action("b", "suspension", true, false);
        running.expires_at = Some("2026-12-01T00:00:00Z".into());
        assert_eq!(action_state(&running, now), ("Active", PillTone::Info));
        assert_eq!(
            action_state(&action("c", "ban", true, false), now),
            ("Active", PillTone::Info)
        );
    }

    #[test]
    fn members_show_by_name_never_by_record_id() {
        let mut banned = action("a", "ban", false, false);
        banned.member_name = None;
        banned.member_id = "mb0g1akg5eglvuix88qy".into();
        let names = vec![(
            "member:mb0g1akg5eglvuix88qy".to_string(),
            "Player07".to_string(),
        )];
        assert_eq!(banned.member_label(&names), "Player07");
        assert_eq!(banned.member_label(&[]), "Unknown member");
    }

    #[test]
    fn new_action_fields_are_labelled_by_id() {
        fn view() -> Element {
            rsx! {
                NewActionFields {
                    members: vec![("m1".to_string(), "Player01".to_string())],
                    member_id: String::new(),
                    action_type: "suspension".to_string(),
                    reason: String::new(),
                    duration: String::new(),
                    on_member: |_| {},
                    on_type: |_| {},
                    on_reason: |_| {},
                    on_duration: |_| {},
                }
            }
        }
        let html = render(view);
        for id in [
            FIELD_MEMBER_ID,
            FIELD_TYPE_ID,
            FIELD_REASON_ID,
            FIELD_EXPIRES_ID,
        ] {
            assert!(html.contains(&format!("for=\"{id}\"")), "{id}: {html}");
            assert_eq!(
                html.matches(&format!(" id=\"{id}\"")).count(),
                1,
                "{id}: {html}"
            );
        }
        assert!(
            html.contains("aria-describedby=\"moderation-new-expires-hint\""),
            "{html}"
        );
        assert!(!html.contains("style="), "{html}");
    }

    #[test]
    fn pages_by_offset_and_names_states() {
        assert_eq!(moderation_path(1), "/api/moderation?limit=20&offset=0");
        assert_eq!(moderation_path(3), "/api/moderation?limit=20&offset=40");
        let now = chrono::Utc::now();
        assert_eq!(
            action_state(&action("a", "note", true, false), now).0,
            "Active"
        );
        assert_eq!(
            action_state(&action("a", "note", false, true), now).0,
            "Lifted"
        );
        // Inactive with no end date: someone lifted it.
        assert_eq!(
            action_state(&action("a", "note", false, false), now).0,
            "Lifted"
        );
        assert_eq!(action_tone("note"), PillTone::Neutral);
    }
}
