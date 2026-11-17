use dioxus::prelude::*;

use crate::components::admin_shared::{after_delete_focus_ids, focus_after_delete_soon};
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, FieldError, PageHeader, PageShell, PageWidth, Pager, Pill, PillTone, page_slice,
    show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AdminDate, ConfirmDialog, DataTable, FormModal, RowActions, Toast,
    admin_list_pending, delete_button_id, list_cap_notice, row_menu_toggle_id, use_toast,
};
use crate::hooks::{ModalController, use_api_list};
use scuffed_api_client::ApiClient;
use scuffed_types::api::{CreateAnnouncementRequest, UpdateAnnouncementRequest};

// Server field is `pinned` (API used to differ; accept both).
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Announcement {
    id: String,
    title: String,
    content: String,
    #[serde(default, alias = "is_pinned")]
    pinned: bool,
    created_at: String,
}

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-announcements-list";
/// The header's "New announcement" button.
const NEW_ID: &str = "announcements-new";
/// The empty card's "New announcement" button.
const NEW_EMPTY_ID: &str = "announcements-new-empty";

/// A record id as a DOM id part: characters outside `[A-Za-z0-9_-]` (the `:`
/// in `announcement:abc`) become `-`.
fn id_part(raw: &str) -> String {
    raw.trim()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect()
}

/// DOM id of one dialog field: the record id (`new` for an unsaved
/// announcement) plus the field name.
fn announcement_field_id(record_id: Option<&str>, field: &str) -> String {
    let key = record_id
        .filter(|id| !id.trim().is_empty())
        .unwrap_or("new");
    format!("announcement-{}-{}", id_part(key), id_part(field))
}

fn edit_button_id(id: &str) -> String {
    format!("announcement-edit-{}", id_part(id))
}

fn actions_id(id: &str) -> String {
    format!("announcement-actions-{}", id_part(id))
}

/// Field errors of the dialog, in plain words. Empty when the field is fine.
fn required_error(value: &str, what: &str) -> String {
    if value.trim().is_empty() {
        format!("Add {what}.")
    } else {
        String::new()
    }
}

#[component]
pub fn AdminAnnouncements() -> Element {
    let mut announcements = use_api_list::<Announcement>("/api/announcements");
    let mut toast = use_toast();
    let mut page = use_signal(|| 1usize);

    // Form modal state
    let mut modal = ModalController::<String>::new();
    let mut form_title = use_signal(String::new);
    let mut form_content = use_signal(String::new);
    let mut form_pinned = use_signal(|| false);
    let mut title_error = use_signal(String::new);
    let mut content_error = use_signal(String::new);
    let mut form_error = use_signal(|| None::<String>);
    // Id of the button that opened the dialog, for focus on Cancel.
    let mut opener = use_signal(|| None::<String>);

    // Delete confirm state
    let mut delete_modal = ModalController::<Announcement>::new();

    let mut reset_form = move || {
        title_error.set(String::new());
        content_error.set(String::new());
        form_error.set(None);
    };

    let mut open_create = move |from: &'static str| {
        form_title.set(String::new());
        form_content.set(String::new());
        form_pinned.set(false);
        reset_form();
        opener.set(Some(from.to_string()));
        modal.show_empty();
    };

    let mut open_edit = move |a: Announcement| {
        form_title.set(a.title);
        form_content.set(a.content);
        form_pinned.set(a.pinned);
        reset_form();
        opener.set(Some(edit_button_id(&a.id)));
        modal.show(a.id);
    };

    let mut open_delete = move |a: Announcement| {
        delete_modal.show(a);
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let title = form_title().trim().to_string();
        let content = form_content().trim().to_string();
        title_error.set(required_error(&title, "a title"));
        content_error.set(required_error(&content, "the announcement text"));
        if title.is_empty() || content.is_empty() {
            form_error.set(Some(
                "Fill in the fields marked below, then save again.".to_string(),
            ));
            return;
        }
        form_error.set(None);
        let pinned = form_pinned();
        let edit_id = modal.get_target();

        modal.start_submit();
        spawn(async move {
            let client = ApiClient::web();
            let result = if let Some(id) = edit_id {
                let body = UpdateAnnouncementRequest {
                    title,
                    content,
                    pinned,
                };
                client
                    .put_json::<_, Announcement>(&format!("/api/announcements/{id}"), &body)
                    .await
            } else {
                let body = CreateAnnouncementRequest {
                    title,
                    content,
                    pinned,
                };
                client
                    .post_json::<_, Announcement>("/api/announcements", &body)
                    .await
            };

            modal.end_submit();
            match result {
                Ok(_) => {
                    toast.show(Toast::success("Announcement saved."));
                    modal.close();
                    announcements.refresh += 1;
                }
                Err(e) => {
                    let message = format!("Could not save the announcement. {e}");
                    form_error.set(Some(message.clone()));
                    toast.show(Toast::error(message));
                }
            }
        });
    };

    let on_confirm_delete = move |_| {
        let Some(target) = delete_modal.get_target() else {
            return;
        };
        let id = target.id.clone();
        // B9: once the row is gone, focus the next row's Edit, else the
        // previous row's, else New announcement.
        let keys: Vec<String> = announcements
            .data
            .peek()
            .as_ref()
            .and_then(|d| d.as_ref())
            .map(|all| all.iter().map(|a| a.id.clone()).collect())
            .unwrap_or_default();
        let focus_next = after_delete_focus_ids(&keys, &id, |k| vec![edit_button_id(k)], &[NEW_ID]);
        delete_modal.close();
        spawn(async move {
            let client = ApiClient::web();
            match client.delete(&format!("/api/announcements/{id}")).await {
                Ok(_) => {
                    toast.show(Toast::success("Announcement deleted."));
                    announcements.refresh += 1;
                    focus_after_delete_soon(edit_button_id(&id), focus_next);
                }
                Err(e) => {
                    toast.show(Toast::error(format!(
                        "Could not delete the announcement. {e}"
                    )));
                }
            }
        });
    };

    let on_cancel_delete = move |_| {
        delete_modal.close();
    };

    let list = {
        let data = announcements.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&announcements, "announcements"),
            Some(all) => {
                let total = all.len();
                let rows = page_slice(all, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No announcements yet",
                            message: "Announcements show on the home page and the News page. Post the first one to tell the crew what is coming up.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    id: NEW_EMPTY_ID,
                                    class: "btn is-primary",
                                    onclick: move |_| open_create(NEW_EMPTY_ID),
                                    "New announcement"
                                }
                            },
                        }
                    } else {
                        AnnouncementsTable {
                            rows,
                            on_edit: move |a: Announcement| open_edit(a),
                            on_delete: move |a: Announcement| open_delete(a),
                        }
                    }
                    Pager {
                        id: "announcements-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "announcements",
                        noun_singular: "announcement",
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

    let record_id = modal.get_target();
    let delete_target = delete_modal.get_target();

    rsx! {
        PageShell { width: PageWidth::Wide, class: "admin-announcements-page",
            PageHeader {
                title: "Announcements",
                lead: "Post news for the crew. Announcements show on the home page and the News page; a pinned one carries a Pinned badge.",
                actions: rsx! {
                    button {
                        r#type: "button",
                        id: NEW_ID,
                        class: "btn is-primary",
                        onclick: move |_| open_create(NEW_ID),
                        "New announcement"
                    }
                },
            }
            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Announcements",
                {list}
            }
            {list_cap_notice(&announcements, "announcements")}
        }

        FormModal {
            title: if record_id.is_some() { "Edit announcement".to_string() } else { "New announcement".to_string() },
            open: modal.is_open(),
            submitting: modal.is_submitting(),
            submit_label: if record_id.is_some() { None } else { Some("Create".to_string()) },
            error: form_error(),
            restore_focus_id: opener(),
            on_close: on_close,
            on_submit: on_submit,
            AnnouncementFields {
                record_id,
                title: form_title,
                content: form_content,
                pinned: form_pinned,
                title_error: title_error(),
                content_error: content_error(),
            }
        }

        ConfirmDialog {
            title: "Delete announcement".to_string(),
            message: format!(
                "Delete \"{}\"? It leaves the home page and the News page. This cannot be undone.",
                delete_target.as_ref().map(|a| a.title.clone()).unwrap_or_default()
            ),
            open: delete_modal.is_open(),
            danger: true,
            confirm_label: "Delete".to_string(),
            // Delete sits in the row on desktop and behind "Actions" on phone:
            // focus goes back to whichever is shown.
            restore_focus_id: delete_target.as_ref().map(|a| delete_button_id(&actions_id(&a.id))),
            restore_focus_fallback: delete_target.as_ref().map(|a| row_menu_toggle_id(&actions_id(&a.id))),
            on_confirm: on_confirm_delete,
            on_cancel: on_cancel_delete,
        }
    }
}

/// The rows: title, Pinned badge, date, then Edit in the row and Delete
/// behind "Actions" on phone.
#[component]
fn AnnouncementsTable(
    rows: Vec<Announcement>,
    on_edit: EventHandler<Announcement>,
    on_delete: EventHandler<Announcement>,
) -> Element {
    rsx! {
        DataTable {
            headers: vec!["Title", "Pinned", "Created", "Actions"],
            num_cols: vec![2],
            cards: true,
            label: "Announcements".to_string(),
            for a in rows {
                {
                    let edit = a.clone();
                    let delete = a.clone();
                    let title = a.title.clone();
                    let edit_id = edit_button_id(&a.id);
                    rsx! {
                        tr { key: "{a.id}",
                            td { class: "cell-who", "{a.title}" }
                            td { class: "cell-badge",
                                if a.pinned {
                                    Pill { tone: PillTone::Info, "Pinned" }
                                }
                            }
                            td { class: "cell-meta num", AdminDate { iso: a.created_at.clone() } }
                            td { class: "cell-actions",
                                RowActions {
                                    id: actions_id(&a.id),
                                    label: "Actions for {title}",
                                    inline_on_desktop: true,
                                    more: rsx! {
                                        button {
                                            r#type: "button",
                                            id: delete_button_id(&actions_id(&a.id)),
                                            class: "btn is-danger",
                                            aria_label: "Delete {title}",
                                            onclick: move |_| on_delete.call(delete.clone()),
                                            "Delete"
                                        }
                                    },
                                    button {
                                        r#type: "button",
                                        id: "{edit_id}",
                                        class: "btn is-ghost",
                                        aria_label: "Edit {title}",
                                        onclick: move |_| on_edit.call(edit.clone()),
                                        "Edit"
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

/// The dialog fields. Ids come from the record id plus the field name; each
/// error line is always rendered and tied to its control.
#[component]
fn AnnouncementFields(
    record_id: Option<String>,
    title: Signal<String>,
    content: Signal<String>,
    pinned: Signal<bool>,
    title_error: String,
    content_error: String,
) -> Element {
    let key = record_id.as_deref();
    let title_id = announcement_field_id(key, "title");
    let title_error_id = announcement_field_id(key, "title-error");
    let content_id = announcement_field_id(key, "content");
    let content_error_id = announcement_field_id(key, "content-error");
    let pinned_id = announcement_field_id(key, "pinned");
    rsx! {
        div { class: "form-field",
            label { class: "field-label", r#for: "{title_id}", "Title" }
            input {
                id: "{title_id}",
                class: "field-input",
                r#type: "text",
                value: "{title}",
                aria_invalid: if !title_error.is_empty() { "true" },
                aria_describedby: "{title_error_id}",
                oninput: move |e| title.set(e.value()),
            }
            FieldError { id: title_error_id.clone(), text: title_error.clone() }
        }
        div { class: "form-field",
            label { class: "field-label", r#for: "{content_id}", "Text" }
            textarea {
                id: "{content_id}",
                class: "field-input",
                value: "{content}",
                aria_invalid: if !content_error.is_empty() { "true" },
                aria_describedby: "{content_error_id}",
                oninput: move |e| content.set(e.value()),
            }
            FieldError { id: content_error_id.clone(), text: content_error.clone() }
        }
        label { class: "admin-check-row", r#for: "{pinned_id}",
            input {
                id: "{pinned_id}",
                r#type: "checkbox",
                checked: pinned(),
                onchange: move |e| pinned.set(e.checked()),
            }
            "Pin this announcement"
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::admin_shared::test_page::render_admin_route;

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// Every `<label for="x">` has a control with `id="x"`.
    fn assert_labels_tied(html: &str) {
        let mut seen = 0;
        for part in html.split("<label").skip(1) {
            let head = part.split('>').next().unwrap_or_default();
            let target = head
                .split("for=\"")
                .nth(1)
                .and_then(|rest| rest.split('"').next())
                .unwrap_or_else(|| panic!("label without for: {head}"));
            assert!(
                html.contains(&format!("id=\"{target}\"")),
                "no control for {target}: {html}"
            );
            seen += 1;
        }
        assert!(seen > 0, "{html}");
    }

    fn row(id: &str, title: &str, pinned: bool) -> Announcement {
        Announcement {
            id: id.into(),
            title: title.into(),
            content: "Scrims move to Friday.".into(),
            pinned,
            created_at: "2026-11-10T19:00:00Z".into(),
        }
    }

    #[test]
    fn page_has_one_h1_and_a_labelled_wait_in_the_wide_shell() {
        let html = render_admin_route(
            "/admin/announcements",
            Some(scuffed_types::OrgRole::Officer),
        );
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-announcements-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Announcements</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains("id=\"announcements-new\""), "{html}");
        assert!(
            html.contains(
                "<section id=\"admin-announcements-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Announcements\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading announcements...</p>"),
            "{html}"
        );
        assert!(!html.contains('\u{2014}'), "{html}");
    }

    #[test]
    fn rows_are_cards_with_a_worded_pin_and_folded_delete() {
        fn view() -> Element {
            rsx! {
                AnnouncementsTable {
                    rows: vec![row("announcement:a1", "Scrim night", true), row("a2", "Patch day", false)],
                    on_edit: |_| {},
                    on_delete: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Created</th>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"ui-pill ui-pill--info\">Pinned</span>"),
            "{html}"
        );
        assert_eq!(html.matches(">Pinned</span>").count(), 1, "{html}");
        assert!(
            html.contains("id=\"announcement-edit-announcement-a1\" class=\"btn is-ghost\" aria-label=\"Edit Scrim night\""),
            "{html}"
        );
        assert!(
            html.contains("aria-controls=\"announcement-actions-announcement-a1-more\""),
            "{html}"
        );
        let group = &html[html.find("class=\"row-menu-group\"").expect("group")..];
        assert!(
            group.contains("aria-label=\"Delete Scrim night\""),
            "{group}"
        );
        // The delete dialog gives focus back to this id (Vision 9, PR 706).
        assert!(
            group.contains("id=\"announcement-actions-announcement-a1-delete\""),
            "{group}"
        );
        assert_eq!(
            delete_button_id(&actions_id("announcement:a1")),
            "announcement-actions-announcement-a1-delete"
        );
        assert!(
            html.contains("<time class=\"date\" datetime=\"2026-11-10\">10 Nov 2026</time>"),
            "{html}"
        );
    }

    #[test]
    fn dialog_fields_are_labelled_and_errors_are_always_rendered() {
        fn view() -> Element {
            let title = use_signal(String::new);
            let content = use_signal(String::new);
            let pinned = use_signal(|| false);
            rsx! {
                AnnouncementFields {
                    record_id: Some("announcement:a1".to_string()),
                    title,
                    content,
                    pinned,
                    title_error: "Add a title.".to_string(),
                    content_error: String::new(),
                }
            }
        }
        let html = render(view);
        assert_labels_tied(&html);
        assert!(
            html.contains("id=\"announcement-announcement-a1-title\""),
            "{html}"
        );
        assert!(
            html.contains("aria-describedby=\"announcement-announcement-a1-title-error\""),
            "{html}"
        );
        assert!(html.contains("Add a title.</p>"), "{html}");
        // The content error line is there before it has text.
        assert!(
            html.contains(
                "<p class=\"field-error\" id=\"announcement-announcement-a1-content-error\"></p>"
            ),
            "{html}"
        );
        assert!(!html.contains("style="), "{html}");
    }

    #[test]
    fn field_ids_come_from_the_record() {
        assert_eq!(
            announcement_field_id(None, "title"),
            "announcement-new-title"
        );
        assert_eq!(
            announcement_field_id(Some(" "), "title"),
            "announcement-new-title"
        );
        assert_ne!(
            announcement_field_id(Some("a1"), "title"),
            announcement_field_id(Some("a2"), "title")
        );
        assert_eq!(required_error(" ", "a title"), "Add a title.");
        assert_eq!(required_error("x", "a title"), "");
    }
}
