use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::{
    ConfirmDialog, DataTable, StatusPill, Toast, admin_pending, list_cap_notice, use_toast,
};
use crate::hooks::{ModalController, use_api_list};
use crate::util::format_datetime;
use scuffed_api_client::ApiClient;
use scuffed_types::api::PatchApplicationRequest;

// Matches the enriched ApplicationListEntry JSON from GET /api/applications.
// Name/label fields are optional so the page still renders against an older server.
#[derive(Debug, Clone, PartialEq, Deserialize)]
struct Application {
    id: String,
    user_id: String,
    #[serde(default)]
    applicant_name: Option<String>,
    preferred_games: Vec<String>,
    #[serde(default)]
    preferred_game_names: Option<Vec<String>>,
    #[serde(default)]
    preferred_roles: Vec<String>,
    message: Option<String>,
    status: String,
    #[serde(default)]
    review_notes: Option<String>,
    created_at: String,
    #[serde(default)]
    updated_at: Option<String>,
}

impl Application {
    fn applicant_label(&self) -> String {
        self.applicant_name
            .clone()
            .unwrap_or_else(|| self.user_id.clone())
    }

    fn games_label(&self) -> String {
        match &self.preferred_game_names {
            Some(names) if !names.is_empty() => names.join(", "),
            _ => self.preferred_games.join(", "),
        }
    }
}

/// Blank optional fields render as the word None. A bare dash is not a value.
fn detail_text(value: &str) -> String {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        "None".to_string()
    } else {
        trimmed.to_string()
    }
}

fn list_or_none(items: &[String]) -> String {
    let joined = items
        .iter()
        .map(|item| item.trim())
        .filter(|item| !item.is_empty())
        .collect::<Vec<_>>()
        .join(", ");
    detail_text(&joined)
}

const APPLICATION_DETAIL_DIALOG_ID: &str = "application-detail-dialog";
const APPLICATION_DETAIL_TITLE_ID: &str = "application-detail-title";

/// Id of the View control that opened this application's detail dialog.
fn application_view_trigger_id(app_id: &str) -> String {
    format!("application-view-{app_id}")
}

fn is_application_dialog_dismiss_key(key: &Key) -> bool {
    *key == Key::Escape
}

fn focus_element_by_id(id: &str) {
    #[cfg(target_arch = "wasm32")]
    {
        use wasm_bindgen::JsCast;
        let Some(document) = web_sys::window().and_then(|window| window.document()) else {
            return;
        };
        let Some(el) = document.get_element_by_id(id) else {
            return;
        };
        if let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() {
            let _ = el.focus();
        }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        // Native tests have no DOM focus. The wasm branch above moves focus.
        let _ = id;
    }
}

#[component]
fn ApplicationDetailDialog(app: Application, on_close: EventHandler<()>) -> Element {
    let title = format!("Application: {}", app.applicant_label());
    let games = detail_text(&app.games_label());
    let roles = list_or_none(&app.preferred_roles);
    let message = detail_text(app.message.as_deref().unwrap_or(""));
    let notes = detail_text(app.review_notes.as_deref().unwrap_or(""));
    let submitted = format_datetime(&app.created_at);
    let updated = match app.updated_at.as_deref() {
        Some(iso) => detail_text(&format_datetime(iso)),
        None => "None".to_string(),
    };
    rsx! {
        div {
            class: "form-modal-overlay",
            onclick: move |_| on_close.call(()),
            div {
                class: "form-modal application-detail-modal",
                id: APPLICATION_DETAIL_DIALOG_ID,
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: APPLICATION_DETAIL_TITLE_ID,
                tabindex: "-1",
                onmounted: move |_| {
                    focus_element_by_id(APPLICATION_DETAIL_DIALOG_ID);
                },
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |evt| {
                    if is_application_dialog_dismiss_key(&evt.key()) {
                        evt.stop_propagation();
                        on_close.call(());
                    }
                },
                div {
                    class: "form-modal-header",
                    id: APPLICATION_DETAIL_TITLE_ID,
                    "{title}"
                }
                div { class: "form-modal-body",
                    div { class: "application-detail",
                        dl {
                            dt { "Status" }
                            dd { StatusPill { status: app.status.clone() } }
                            dt { "Games" }
                            dd { "{games}" }
                            dt { "Roles" }
                            dd { "{roles}" }
                            dt { "Message" }
                            dd { style: "white-space: pre-wrap;", "{message}" }
                            dt { "Review notes" }
                            dd { "{notes}" }
                            dt { "Submitted" }
                            dd { "{submitted}" }
                            dt { "Last update" }
                            dd { "{updated}" }
                        }
                    }
                }
                div { class: "form-modal-footer",
                    button {
                        class: "btn-cancel",
                        onclick: move |_| on_close.call(()),
                        "Close"
                    }
                }
            }
        }
    }
}

#[component]
pub fn AdminApplications() -> Element {
    // Cursor-paginated list (auto-follows pages via use_api_list).
    let mut applications = use_api_list::<Application>("/api/applications");
    let mut toast = use_toast();

    // Reject dialog state
    let mut reject_modal = ModalController::<String>::new();
    let mut reject_notes = use_signal(String::new);

    // Read-only detail drawer (any application, incl. closed ones)
    let mut view_open = use_signal(|| false);
    let mut view_target = use_signal(|| None::<Application>);

    let accept = move |id: String| {
        spawn(async move {
            let body = PatchApplicationRequest {
                status: "accepted".to_string(),
                review_notes: None,
            };
            let path = format!("/api/applications/{id}");
            match ApiClient::web()
                .patch_json::<_, Application>(&path, &body)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Application accepted"));
                    applications.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to accept: {e}"))),
            }
        });
    };

    let confirm_reject = move |_| {
        let id = reject_modal.get_target().unwrap_or_default();
        let notes = reject_notes().clone();
        reject_modal.close();
        reject_notes.set(String::new());
        spawn(async move {
            let body = PatchApplicationRequest {
                status: "rejected".to_string(),
                review_notes: if notes.is_empty() { None } else { Some(notes) },
            };
            let path = format!("/api/applications/{id}");
            match ApiClient::web()
                .patch_json::<_, Application>(&path, &body)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Application rejected"));
                    applications.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to reject: {e}"))),
            }
        });
    };

    rsx! {

        h1 { "Applications" }

        {
            let data = applications.data.read();
            let data = data.as_ref().and_then(|d| d.as_ref());
            match data {
                None => admin_pending(&applications, "applications"),
                Some(list) if list.is_empty() => rsx! {
                    p { class: "empty-state", "No applications." }
                },
                Some(list) => rsx! {
                    DataTable { headers: vec!["Applicant", "Games", "Message", "Status", "Date", "Actions"],
                        for app in list.iter() {
                            {
                                let id = app.id.clone();
                                let id2 = app.id.clone();
                                let applicant = app.applicant_label();
                                let games = app.games_label();
                                let msg = app.message.clone().unwrap_or_default();
                                let date: String = app.created_at.chars().take(10).collect();
                                // Officers can act on the open pipeline: pending and trial
                                // (server validates transitions either way).
                                let can_action = app.status == "pending" || app.status == "trial";
                                let view_trigger = application_view_trigger_id(&app.id);
                                let view_app = app.clone();
                                rsx! {
                                    tr { key: "{id}",
                                        td { "{applicant}" }
                                        td { "{games}" }
                                        td { "{msg}" }
                                        td { StatusPill { status: app.status.clone() } }
                                        td { "{date}" }
                                        td {
                                            div { class: "row-actions",
                                                if can_action {
                                                    button {
                                                        class: "row-btn primary",
                                                        onclick: move |_| accept(id.clone()),
                                                        "Accept"
                                                    }
                                                    button {
                                                        class: "row-btn danger",
                                                        onclick: move |_| reject_modal.show(id2.clone()),
                                                        "Reject"
                                                    }
                                                }
                                                button {
                                                    class: "row-btn",
                                                    id: "{view_trigger}",
                                                    onclick: move |_| {
                                                        view_target.set(Some(view_app.clone()));
                                                        view_open.set(true);
                                                    },
                                                    "View"
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

        {list_cap_notice(&applications, "applications")}

        {
            let dialog = if view_open() { view_target() } else { None };
            if let Some(app) = dialog {
                let trigger_id = application_view_trigger_id(&app.id);
                rsx! {
                    ApplicationDetailDialog {
                        app,
                        on_close: move |_| {
                            focus_element_by_id(&trigger_id);
                            view_open.set(false);
                        },
                    }
                }
            } else {
                rsx! {}
            }
        }

        ConfirmDialog {
            title: "Reject Application".to_string(),
            message: "Are you sure you want to reject this application?".to_string(),
            open: reject_modal.is_open(),
            danger: true,
            on_confirm: confirm_reject,
            on_cancel: move |_| {
                reject_modal.close();
                reject_notes.set(String::new());
            },
            extra: rsx! {
                div { class: "form-field", style: "margin-top: 0.75rem;",
                    label { class: "form-label", "Rejection Notes (optional)" }
                    textarea {
                        class: "form-textarea",
                        value: "{reject_notes}",
                        oninput: move |e| reject_notes.set(e.value()),
                    }
                }
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_application() -> Application {
        Application {
            id: "app1".into(),
            user_id: "user1".into(),
            applicant_name: Some("Ada".into()),
            preferred_games: vec![],
            preferred_game_names: None,
            preferred_roles: vec![],
            message: None,
            status: "pending".into(),
            review_notes: Some("   ".into()),
            created_at: "2026-03-01T12:00:00Z".into(),
            updated_at: None,
        }
    }

    fn render(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn application_detail_modal_pins_width_rule() {
        let css = crate::styles::admin::CSS;
        let rule = ".form-modal.application-detail-modal {\n        width: min(640px, 92vw);\n        max-width: min(640px, 92vw);";
        assert!(
            css.contains(rule),
            "application detail width must stay min(640px, 92vw) on the form-modal shell"
        );
        assert!(css.contains(".application-detail dl"));
        assert!(css.contains("grid-template-columns:"));

        fn view() -> Element {
            rsx! {
                ApplicationDetailDialog {
                    app: sample_application(),
                    on_close: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(
            html.contains("form-modal application-detail-modal"),
            "markup must carry the class the width rule targets: {html}"
        );
        assert!(html.contains("<dl"));
        assert!(html.contains("<dt"));
        assert!(html.contains("<dd"));
        assert!(!html.contains("modal-content"), "{html}");
        assert!(!html.contains('\u{2014}'), "{html}");
        assert!(!html.contains("&mdash;"), "{html}");
        assert!(html.contains(">None<"), "{html}");
        assert!(html.contains("Roles"), "{html}");
        assert!(html.contains("Review notes"), "{html}");
    }

    #[test]
    fn application_detail_dialog_labels_itself_and_keeps_message_line_breaks() {
        assert!(is_application_dialog_dismiss_key(&Key::Escape));
        assert!(!is_application_dialog_dismiss_key(&Key::Enter));
        assert_eq!(application_view_trigger_id("app1"), "application-view-app1");

        fn view() -> Element {
            let mut app = sample_application();
            app.message = Some("Line one\nLine two".into());
            rsx! {
                ApplicationDetailDialog {
                    app,
                    on_close: |_| {},
                }
            }
        }
        let html = render(view);
        let dialog_at = html.find("application-detail-modal").expect("dialog class");
        let dialog_tag_end = html[dialog_at..].find('>').expect("dialog tag") + dialog_at;
        let dialog_tag = &html[dialog_at..dialog_tag_end];
        assert!(
            dialog_tag.contains("role=\"dialog\""),
            "dialog role: {dialog_tag}"
        );
        assert!(
            dialog_tag.contains("aria-modal=\"true\""),
            "aria-modal: {dialog_tag}"
        );
        assert!(
            dialog_tag.contains("aria-labelledby=\"application-detail-title\""),
            "aria-labelledby: {dialog_tag}"
        );
        assert!(
            dialog_tag.contains("id=\"application-detail-dialog\""),
            "focus target id: {dialog_tag}"
        );
        assert!(
            dialog_tag.contains("tabindex=\"-1\""),
            "dialog must be focusable: {dialog_tag}"
        );

        let header_at = html.find("form-modal-header").expect("heading");
        let header_end = html[header_at..].find("</div>").expect("heading end") + header_at;
        let header = &html[header_at..header_end];
        assert!(
            header.contains("id=\"application-detail-title\""),
            "heading id: {header}"
        );
        assert!(
            header.contains("Application: Ada"),
            "labelled heading text: {header}"
        );

        assert!(
            html.contains("white-space:pre-wrap") || html.contains("white-space: pre-wrap"),
            "message keeps line breaks: {html}"
        );
        assert!(
            html.contains("Line one\nLine two"),
            "message text keeps the newline: {html}"
        );
        assert!(!html.contains('\u{2014}'), "{html}");
        assert!(!html.contains("&mdash;"), "{html}");
    }

    #[test]
    fn empty_detail_values_say_none_and_filled_values_stay() {
        assert_eq!(detail_text(""), "None");
        assert_eq!(detail_text("  "), "None");
        assert_eq!(detail_text(" flex "), "flex");
        assert_eq!(list_or_none(&[]), "None");
        assert_eq!(
            list_or_none(&["Tank".into(), " ".into(), "Support".into()]),
            "Tank, Support"
        );

        fn view() -> Element {
            let mut app = sample_application();
            app.preferred_games = vec!["overwatch".into()];
            app.preferred_game_names = Some(vec!["Overwatch".into()]);
            app.preferred_roles = vec!["Tank".into()];
            app.message = Some("Hello".into());
            app.review_notes = Some("Ready".into());
            app.updated_at = Some("2026-03-02T08:15:00Z".into());
            rsx! {
                ApplicationDetailDialog {
                    app,
                    on_close: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("Overwatch"), "{html}");
        assert!(html.contains("Tank"), "{html}");
        assert!(html.contains("Hello"), "{html}");
        assert!(html.contains("Ready"), "{html}");
        assert!(html.contains("2026-03-02 08:15"), "{html}");
        assert!(!html.contains('\u{2014}'), "{html}");
    }
}
