use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    ChipButton, ChipRow, EmptyCard, PageHeader, PageShell, PageWidth, Pager, page_slice,
    show_list_top,
};
use crate::components::{
    ADMIN_PAGE_SIZE, AdminDate, ConfirmDialog, DataTable, RowActions, StatusPill, Toast,
    admin_date_time, admin_list_pending, list_cap_notice, use_toast,
};
use crate::hooks::{ModalController, use_api_list};
use crate::pages::display_name::{
    ApplicantNameCell, DisplayNameAudience, applicant_name_error_id, described_by_when_error,
    display_name_error_message,
};
use crate::routes::Route;
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

    /// Preferred roles as words ("Tank, Damage"), whatever case the form
    /// stored them in.
    fn roles_label(&self) -> String {
        self.preferred_roles
            .iter()
            .map(|r| role_label(r))
            .filter(|r| !r.is_empty())
            .collect::<Vec<_>>()
            .join(", ")
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

/// One preferred role as a word: the known ids by name (`dps` is Damage),
/// anything else with a capital first letter.
fn role_label(raw: &str) -> String {
    let raw = raw.trim();
    match raw.to_ascii_lowercase().as_str() {
        "tank" => "Tank".to_string(),
        "dps" | "damage" => "Damage".to_string(),
        "support" | "healer" => "Support".to_string(),
        "flex" => "Flex".to_string(),
        _ => {
            let mut chars = raw.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
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

/// The decline question, naming the applicant when the row is known.
fn decline_message(applicant: Option<&str>) -> String {
    match applicant.map(str::trim).filter(|n| !n.is_empty()) {
        Some(name) => format!("Decline the application from {name}?"),
        None => "Decline this application?".to_string(),
    }
}

/// The list region: the pager's focus target after a page change.
const LIST_ID: &str = "admin-applications-list";
const REJECT_NOTES_ID: &str = "application-decline-notes";

/// Status chips: the stored status (empty for all) and the chip's word. A
/// stored `rejected` reads "Declined", as on its pill.
const STATUS_FILTERS: [(&str, &str); 6] = [
    ("", "All"),
    ("pending", "Pending"),
    ("trial", "Trial"),
    ("accepted", "Accepted"),
    ("rejected", "Declined"),
    ("withdrawn", "Withdrawn"),
];

/// Id of a status chip, from its status.
fn status_filter_id(status: &str) -> String {
    let name = if status.is_empty() { "all" } else { status };
    format!("application-filter-{name}")
}

/// The applications a status chip keeps. The empty filter keeps all.
fn filter_applications(list: &[Application], status: &str) -> Vec<Application> {
    list.iter()
        .filter(|app| status.is_empty() || app.status == status)
        .cloned()
        .collect()
}

/// Officers can act on the open pipeline: pending and trial (the server
/// validates transitions either way).
fn can_review(status: &str) -> bool {
    status == "pending" || status == "trial"
}

const APPLICATION_DETAIL_DIALOG_ID: &str = "application-detail-dialog";
const APPLICATION_DETAIL_TITLE_ID: &str = "application-detail-title";
const APPLICATION_DETAIL_CLOSE_ID: &str = "application-detail-close";

/// Id of the View control that opened this application's detail dialog.
fn application_view_trigger_id(app_id: &str) -> String {
    format!("application-view-{app_id}")
}

/// Tab stops inside the dialog, in DOM order.
/// The heading is `tabindex="-1"`: it takes focus when the dialog opens, and it is
/// the first stop, so Shift+Tab from there wraps to the last control.
fn application_dialog_tab_ids() -> &'static [&'static str] {
    &[APPLICATION_DETAIL_TITLE_ID, APPLICATION_DETAIL_CLOSE_ID]
}

/// Focus this when the dialog opens. Not the dialog shell: a shell with
/// `tabindex="-1"` sits outside the tab cycle, so Shift+Tab leaves the dialog.
fn application_dialog_initial_focus_id() -> &'static str {
    APPLICATION_DETAIL_TITLE_ID
}

#[derive(Debug, PartialEq, Eq)]
enum DialogKeyEffect {
    /// Escape closes the dialog and focus goes back to the View button.
    Dismiss {
        restore_focus: String,
    },
    /// Tab would leave the dialog, so focus moves to this stop instead.
    MoveFocus {
        id: &'static str,
    },
    Ignore,
}

/// Tab from the last stop wraps to the first. Shift+Tab from the first wraps to the last.
/// Any other Tab is left to the browser, which stays inside the same list.
fn application_dialog_tab_target(active_id: &str, shift: bool) -> Option<&'static str> {
    let ids = application_dialog_tab_ids();
    let len = ids.len();
    // The shell is not a tab stop. If focus is still there, treat it as the heading.
    let current = ids
        .iter()
        .position(|id| *id == active_id)
        .or_else(|| (active_id == APPLICATION_DETAIL_DIALOG_ID).then_some(0))?;
    let wraps = if shift {
        current == 0
    } else {
        current + 1 == len
    };
    if !wraps {
        return None;
    }
    let next = if shift { len - 1 } else { 0 };
    Some(ids[next])
}

fn application_dialog_key_effect(
    key: &Key,
    shift: bool,
    active_id: &str,
    trigger_id: &str,
) -> DialogKeyEffect {
    if *key == Key::Escape {
        return DialogKeyEffect::Dismiss {
            restore_focus: trigger_id.to_string(),
        };
    }
    if *key == Key::Tab
        && let Some(id) = application_dialog_tab_target(active_id, shift)
    {
        return DialogKeyEffect::MoveFocus { id };
    }
    DialogKeyEffect::Ignore
}

/// Shared by the dialog `onkeydown`. Escape focuses the View button, then closes.
/// A wrapping Tab focuses the stop on the other end of the dialog.
fn handle_application_dialog_key(
    key: &Key,
    shift: bool,
    active_id: &str,
    trigger_id: &str,
    mut on_dismiss: impl FnMut(),
    mut on_focus: impl FnMut(&str),
) -> bool {
    match application_dialog_key_effect(key, shift, active_id, trigger_id) {
        DialogKeyEffect::Dismiss { restore_focus } => {
            on_focus(&restore_focus);
            on_dismiss();
            true
        }
        DialogKeyEffect::MoveFocus { id } => {
            on_focus(id);
            true
        }
        DialogKeyEffect::Ignore => false,
    }
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

fn current_focus_id() -> Option<String> {
    #[cfg(target_arch = "wasm32")]
    {
        let id = web_sys::window()
            .and_then(|window| window.document())
            .and_then(|document| document.active_element())
            .map(|el| el.id())?;
        if id.is_empty() { None } else { Some(id) }
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        None
    }
}

#[component]
fn ApplicationDetailDialog(
    app: Application,
    trigger_id: String,
    on_close: EventHandler<()>,
) -> Element {
    let title = format!("Application: {}", app.applicant_label());
    let games = detail_text(&app.games_label());
    let roles = detail_text(&app.roles_label());
    let message = detail_text(app.message.as_deref().unwrap_or(""));
    let notes = detail_text(app.review_notes.as_deref().unwrap_or(""));
    let submitted = admin_date_time(&app.created_at);
    let updated = match app.updated_at.as_deref() {
        Some(iso) => detail_text(&admin_date_time(iso)),
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
                onmounted: move |_| {
                    focus_element_by_id(application_dialog_initial_focus_id());
                },
                onclick: move |e| e.stop_propagation(),
                onkeydown: move |evt| {
                    let shift = evt.modifiers().contains(Modifiers::SHIFT);
                    let active = current_focus_id()
                        .unwrap_or_else(|| application_dialog_initial_focus_id().to_string());
                    if handle_application_dialog_key(
                        &evt.key(),
                        shift,
                        &active,
                        &trigger_id,
                        || on_close.call(()),
                        focus_element_by_id,
                    ) {
                        evt.prevent_default();
                        evt.stop_propagation();
                    }
                },
                h2 {
                    class: "form-modal-header",
                    id: APPLICATION_DETAIL_TITLE_ID,
                    tabindex: "-1",
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
                            dd { class: "application-detail-message", "{message}" }
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
                        class: "btn is-ghost",
                        r#type: "button",
                        id: APPLICATION_DETAIL_CLOSE_ID,
                        onclick: move |_| on_close.call(()),
                        "Close"
                    }
                }
            }
        }
    }
}

/// The table of one page of applications. On phone every row is a card:
/// name and View on the top line, the status pill under the name, then role,
/// games and date on one line. Accept and Decline are in the row on desktop
/// and behind "Actions" on phone.
#[component]
fn ApplicationsTable(
    rows: Vec<Application>,
    /// The application whose accept failed on its display name, and why.
    name_error: Option<(String, String)>,
    on_accept: EventHandler<String>,
    on_reject: EventHandler<String>,
    on_view: EventHandler<Application>,
) -> Element {
    rsx! {
        DataTable {
            headers: vec!["Applicant", "Status", "Role", "Games", "Date", "Actions"],
            num_cols: vec![4],
            cards: true,
            label: "Applications",
            for app in rows.iter() {
                {
                    let id = app.id.clone();
                    let accept_id = app.id.clone();
                    let reject_id = app.id.clone();
                    let applicant = app.applicant_label();
                    let games = app.games_label();
                    let roles = app.roles_label();
                    let view_trigger = application_view_trigger_id(&app.id);
                    let view_app = app.clone();
                    let row_error = name_error
                        .as_ref()
                        .filter(|(row_id, _)| row_id == &id)
                        .map(|(_, message)| message.clone());
                    let name_error_id = applicant_name_error_id(&id);
                    let accept_described_by =
                        described_by_when_error(row_error.as_deref(), &name_error_id);
                    let review = can_review(&app.status).then(|| rsx! {
                        button {
                            r#type: "button",
                            class: "btn is-primary",
                            aria_label: "Accept {applicant}",
                            aria_describedby: accept_described_by.clone(),
                            onclick: move |_| on_accept.call(accept_id.clone()),
                            "Accept"
                        }
                        button {
                            r#type: "button",
                            class: "btn is-danger",
                            aria_label: "Decline {applicant}",
                            onclick: move |_| on_reject.call(reject_id.clone()),
                            "Decline"
                        }
                    });
                    rsx! {
                        tr { key: "{id}",
                            td { class: "cell-who",
                                ApplicantNameCell {
                                    name: applicant.clone(),
                                    error: row_error,
                                    error_id: name_error_id,
                                }
                            }
                            td { class: "cell-badge", StatusPill { status: app.status.clone() } }
                            td { class: "cell-meta", "{roles}" }
                            td { class: "cell-meta", "{games}" }
                            td { class: "cell-meta num", AdminDate { iso: app.created_at.clone() } }
                            td { class: "cell-actions",
                                RowActions {
                                    id: "application-actions-{id}",
                                    label: "Actions for {applicant}",
                                    inline_on_desktop: true,
                                    more: review,
                                    button {
                                        r#type: "button",
                                        class: "btn is-ghost",
                                        id: "{view_trigger}",
                                        aria_label: "View {applicant}",
                                        onclick: move |_| on_view.call(view_app.clone()),
                                        "View"
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
pub fn AdminApplications() -> Element {
    // Cursor-paginated list (auto-follows pages via use_api_list).
    let mut applications = use_api_list::<Application>("/api/applications");
    let mut toast = use_toast();
    let mut accept_name_error = use_signal(|| None::<(String, String)>);

    // Status chip and page of the list. Both live in memory.
    let mut status_filter = use_signal(|| "");
    let mut page = use_signal(|| 1usize);

    // Reject dialog state
    let mut reject_modal = ModalController::<String>::new();
    let mut reject_notes = use_signal(String::new);

    // Read-only detail drawer (any application, incl. closed ones)
    let mut view_open = use_signal(|| false);
    let mut view_target = use_signal(|| None::<Application>);

    let accept = move |id: String| {
        spawn(async move {
            accept_name_error.set(None);
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
                    accept_name_error.set(None);
                    toast.show(Toast::success("Application accepted"));
                    applications.refresh += 1;
                }
                Err(e) => {
                    if let Some(message) =
                        display_name_error_message(&e, DisplayNameAudience::Accept)
                    {
                        accept_name_error.set(Some((id, message.to_string())));
                    } else {
                        toast.show(Toast::error(format!("Failed to accept: {e}")));
                    }
                }
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
                    toast.show(Toast::success("Application declined"));
                    applications.refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Failed to decline: {e}"))),
            }
        });
    };

    // Cancel in the decline dialog returns to the row's View button, which
    // is on screen at every width.
    let reject_focus = reject_modal
        .get_target()
        .map(|id| application_view_trigger_id(&id));

    // The decline dialog names the applicant it is about.
    let reject_message = {
        let target = reject_modal.get_target();
        let data = applications.data.read();
        let name = target.and_then(|id| {
            data.as_ref()
                .and_then(|d| d.as_ref())
                .and_then(|all| all.iter().find(|a| a.id == id))
                .map(|a| a.applicant_label())
        });
        decline_message(name.as_deref())
    };
    let list_failed = applications.error.read().is_some()
        && applications
            .data
            .read()
            .as_ref()
            .and_then(|d| d.as_ref())
            .is_none();

    let list = {
        let data = applications.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => admin_list_pending(&applications, "applications"),
            Some(all) => {
                let shown = filter_applications(all, status_filter());
                let total = shown.len();
                let rows = page_slice(&shown, page(), ADMIN_PAGE_SIZE).to_vec();
                rsx! {
                    if all.is_empty() {
                        EmptyCard {
                            title: "No applications yet",
                            message: "New applications from the Apply page show up here for review.",
                            actions: rsx! {
                                Link { to: Route::Apply {}, class: "btn is-ghost", "Open the Apply page" }
                            },
                        }
                    } else if rows.is_empty() {
                        EmptyCard {
                            title: "No applications match these filters",
                            message: "Clear the filters to see the full list, or wait for a new application to arrive.",
                            actions: rsx! {
                                button {
                                    r#type: "button",
                                    class: "btn is-ghost",
                                    onclick: move |_| {
                                        status_filter.set("");
                                        page.set(1);
                                    },
                                    "Clear filters"
                                }
                            },
                        }
                    } else {
                        ApplicationsTable {
                            rows,
                            name_error: accept_name_error(),
                            on_accept: accept,
                            on_reject: move |id: String| reject_modal.show(id),
                            on_view: move |app: Application| {
                                view_target.set(Some(app));
                                view_open.set(true);
                            },
                        }
                    }
                    // Always rendered: its count line is the list's live line.
                    Pager {
                        id: "applications-pager",
                        page: page(),
                        total,
                        page_size: ADMIN_PAGE_SIZE,
                        noun: "applications",
                        noun_singular: "application",
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

    rsx! {
        PageShell { width: PageWidth::Wide, class: "admin-applications-page",
            PageHeader {
                title: "Applications",
                lead: "Filter by status. View an application to read it, then accept or decline it.",
            }

            // Nothing to filter while the list failed to load.
            if !list_failed {
            ChipRow { label: "Status filters", scroll: true,
                for (status, word) in STATUS_FILTERS {
                    ChipButton {
                        key: "{word}",
                        class: "chip",
                        id: status_filter_id(status),
                        label: word,
                        pressed: status_filter() == status,
                        onclick: move |_| {
                            status_filter.set(status);
                            page.set(1);
                        },
                    }
                }
            }
            }

            section {
                id: LIST_ID,
                class: "admin-list pager-target",
                tabindex: "-1",
                aria_label: "Applications",
                {list}
            }

            {list_cap_notice(&applications, "applications")}
        }

        {
            let dialog = if view_open() { view_target() } else { None };
            if let Some(app) = dialog {
                let trigger_id = application_view_trigger_id(&app.id);
                rsx! {
                    ApplicationDetailDialog {
                        app,
                        trigger_id: trigger_id.clone(),
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
            title: "Decline application".to_string(),
            message: reject_message,
            open: reject_modal.is_open(),
            danger: true,
            confirm_label: "Decline".to_string(),
            restore_focus_id: reject_focus,
            on_confirm: confirm_reject,
            on_cancel: move |_| {
                reject_modal.close();
                reject_notes.set(String::new());
            },
            extra: rsx! {
                div { class: "form-field",
                    label { class: "field-label", r#for: REJECT_NOTES_ID, "Decline notes (optional)" }
                    textarea {
                        class: "field-input",
                        id: REJECT_NOTES_ID,
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

    fn row(id: &str, name: &str, status: &str) -> Application {
        Application {
            id: id.into(),
            user_id: format!("user-{id}"),
            applicant_name: Some(name.into()),
            preferred_games: vec!["overwatch".into()],
            preferred_game_names: Some(vec!["Overwatch".into()]),
            preferred_roles: vec!["Tank".into()],
            message: Some("Hello".into()),
            status: status.into(),
            review_notes: None,
            created_at: "2026-11-10T12:00:00Z".into(),
            updated_at: None,
        }
    }

    #[test]
    fn page_has_one_h1_chips_and_a_labelled_wait_in_the_wide_shell() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/applications", Some(scuffed_types::OrgRole::Officer));
        assert!(
            html.contains("class=\"ui-shell ui-shell--wide admin-applications-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Applications</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        // The status chips are a named group; All is on.
        assert!(
            html.contains(
                "<div class=\"chip-row is-scroll\" role=\"group\" aria-label=\"Status filters\">"
            ),
            "{html}"
        );
        assert_eq!(
            html.matches("id=\"application-filter-").count(),
            6,
            "{html}"
        );
        assert!(
            html.contains("id=\"application-filter-all\" aria-pressed=\"true\""),
            "{html}"
        );
        assert!(
            html.contains("id=\"application-filter-rejected\" aria-pressed=\"false\""),
            "{html}"
        );
        assert!(html.contains(">Declined</button>"), "{html}");
        // Nothing has loaded yet: a labelled wait inside the list region.
        assert!(
            html.contains(
                "<section id=\"admin-applications-list\" class=\"admin-list pager-target\" tabindex=\"-1\" aria-label=\"Applications\">"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading applications...</p>"),
            "{html}"
        );
    }

    #[test]
    fn status_chips_filter_the_list() {
        let list = vec![
            row("a1", "Player04", "pending"),
            row("a2", "Player09", "trial"),
            row("a3", "Player12", "rejected"),
        ];
        assert_eq!(filter_applications(&list, "").len(), 3);
        let declined = filter_applications(&list, "rejected");
        assert_eq!(declined.len(), 1);
        assert_eq!(declined[0].id, "a3");
        assert!(filter_applications(&list, "accepted").is_empty());
        assert_eq!(status_filter_id(""), "application-filter-all");
        assert_eq!(status_filter_id("trial"), "application-filter-trial");
        // Every status the server stores has a chip, and "rejected" reads
        // "Declined" as on its pill.
        let words: Vec<&str> = STATUS_FILTERS.iter().map(|(_, word)| *word).collect();
        assert_eq!(
            words,
            [
                "All",
                "Pending",
                "Trial",
                "Accepted",
                "Declined",
                "Withdrawn"
            ]
        );
        assert!(can_review("pending") && can_review("trial"));
        assert!(!can_review("accepted") && !can_review("rejected") && !can_review("withdrawn"));
    }

    #[test]
    fn rows_carry_the_card_cells_and_fold_review_actions() {
        fn view() -> Element {
            rsx! {
                ApplicationsTable {
                    rows: vec![row("a1", "Player04", "pending"), row("a2", "Player10", "accepted")],
                    name_error: Some(("a1".to_string(), "That name is taken.".to_string())),
                    on_accept: |_| {},
                    on_reject: |_| {},
                    on_view: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("data-table-scroll is-cards"), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Date</th>"),
            "{html}"
        );
        let (first, second) = html
            .split_once("</tr><tr>")
            .map(|(a, b)| (a.rsplit("<tr>").next().unwrap().to_string(), b.to_string()))
            .expect("two body rows");
        // Name, then the status pill, then role, games and date.
        let who = first.find("class=\"cell-who\"").expect("who");
        let badge = first.find("class=\"cell-badge\"").expect("badge");
        let meta = first.find("class=\"cell-meta\"").expect("meta");
        assert!(who < badge && badge < meta, "{first}");
        assert!(
            first.contains("<span class=\"ui-pill ui-pill--warn\">Pending</span>"),
            "{first}"
        );
        assert!(
            first.contains("<time class=\"date\" datetime=\"2026-11-10\">10 Nov 2026</time>"),
            "{first}"
        );
        assert!(
            !first.contains("Hello"),
            "the message stays in the dialog: {first}"
        );
        // An open application: View in the row, Accept and Decline in the
        // named group that phone folds behind "Actions".
        assert!(
            first.contains("class=\"row-actions is-inline-desktop\""),
            "{first}"
        );
        assert!(
            first.contains("id=\"application-view-a1\" aria-label=\"View Player04\""),
            "{first}"
        );
        assert!(
            first.contains(
                "aria-controls=\"application-actions-a1-more\" aria-label=\"Actions for Player04\""
            ),
            "{first}"
        );
        let group = &first[first.find("class=\"row-menu-group\"").expect("group")..];
        assert!(group.contains(">Accept</button>"), "{group}");
        assert!(group.contains(">Decline</button>"), "{group}");
        // The failed accept is announced in the row and tied to Accept.
        assert!(
            first.contains("role=\"alert\">That name is taken.</p>"),
            "{first}"
        );
        assert!(
            group.contains("aria-describedby=\"application-name-error-a1\""),
            "{group}"
        );
        // A closed application has View only.
        assert!(second.contains(">View</button>"), "{second}");
        assert!(!second.contains("row-menu-group"), "{second}");
        assert!(!second.contains(">Accept</button>"), "{second}");
        assert_eq!(html.matches("id=\"application-view-").count(), 2, "{html}");
    }

    #[test]
    fn application_detail_modal_pins_width_rule() {
        let css = crate::styles::admin::CSS;
        // The dialog is as wide as the overlay lets it be, up to the small
        // container, and the overlay pads it on a 390px viewport.
        let rule = ".form-modal.application-detail-modal {\n        width: 100%;\n        max-width: var(--container-sm);";
        assert!(
            css.contains(rule),
            "application detail width must stay on the container token"
        );
        let overlay = css
            .split(".form-modal-overlay {")
            .nth(1)
            .and_then(|rest| rest.split('}').next())
            .expect("overlay rule");
        assert!(overlay.contains("padding: var(--space-4);"), "{overlay}");
        assert!(css.contains(".application-detail dl"));
        assert!(css.contains("grid-template-columns:"));

        fn view() -> Element {
            rsx! {
                ApplicationDetailDialog {
                    app: sample_application(),
                    trigger_id: application_view_trigger_id("app1"),
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
        assert_eq!(application_view_trigger_id("app1"), "application-view-app1");

        fn view() -> Element {
            let mut app = sample_application();
            app.message = Some("Line one\nLine two".into());
            rsx! {
                ApplicationDetailDialog {
                    app,
                    trigger_id: application_view_trigger_id("app1"),
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
            "dialog id: {dialog_tag}"
        );
        assert!(
            !dialog_tag.contains("tabindex"),
            "the shell must not take the initial focus: {dialog_tag}"
        );

        let header_at = html.find("form-modal-header").expect("heading");
        let header_end = html[header_at..].find("</h2>").expect("heading end") + header_at;
        let header = &html[header_at..header_end];
        assert!(
            header.contains("id=\"application-detail-title\""),
            "heading id: {header}"
        );
        assert!(
            header.contains("tabindex=\"-1\""),
            "heading takes focus on open: {header}"
        );
        assert!(
            header.contains("Application: Ada"),
            "labelled heading text: {header}"
        );

        let message_at = html.find("Line one").expect("message text");
        let message_tag_at = html[..message_at].rfind("<dd").expect("message dd");
        let message_tag_end =
            html[message_tag_at..].find('>').expect("message tag") + message_tag_at;
        let message_tag = &html[message_tag_at..message_tag_end];
        assert!(
            message_tag.contains("application-detail-message"),
            "message field class: {message_tag}"
        );
        // The wrap and line breaks come from the class rule, never an
        // inline style.
        assert!(!message_tag.contains("style="), "{message_tag}");
        let css = crate::styles::admin::CSS;
        assert!(
            css.contains(".application-detail dd.application-detail-message"),
            "message wrap rule missing: {css}"
        );
        assert!(
            css.contains("white-space: pre-wrap"),
            "message css keeps line breaks"
        );
        assert!(
            css.contains("overflow-wrap: anywhere"),
            "message css wraps long tokens"
        );
        assert!(
            html.contains("Line one\nLine two"),
            "message text keeps the newline: {html}"
        );
        assert!(!html.contains('\u{2014}'), "{html}");
        assert!(!html.contains("&mdash;"), "{html}");
    }

    #[test]
    fn roles_read_as_words_and_decline_names_the_applicant() {
        assert_eq!(role_label("dps"), "Damage");
        assert_eq!(role_label(" tank "), "Tank");
        assert_eq!(role_label("support"), "Support");
        assert_eq!(role_label("flex"), "Flex");
        assert_eq!(role_label("main tank"), "Main tank");
        assert_eq!(
            decline_message(Some("Applicant03")),
            "Decline the application from Applicant03?"
        );
        assert_eq!(decline_message(None), "Decline this application?");
    }

    #[test]
    fn escape_closes_the_dialog_and_returns_focus_to_view() {
        let trigger = application_view_trigger_id("app1");
        let steps = std::cell::RefCell::new(Vec::new());
        let handled = handle_application_dialog_key(
            &Key::Escape,
            false,
            APPLICATION_DETAIL_CLOSE_ID,
            &trigger,
            || steps.borrow_mut().push("close".to_string()),
            |id| steps.borrow_mut().push(format!("focus:{id}")),
        );
        assert!(handled);
        assert_eq!(
            steps.borrow().as_slice(),
            [
                "focus:application-view-app1".to_string(),
                "close".to_string(),
            ]
        );
        assert_eq!(trigger, "application-view-app1");

        steps.borrow_mut().clear();
        let handled = handle_application_dialog_key(
            &Key::Enter,
            false,
            APPLICATION_DETAIL_CLOSE_ID,
            &trigger,
            || steps.borrow_mut().push("close".to_string()),
            |id| steps.borrow_mut().push(format!("focus:{id}")),
        );
        assert!(!handled);
        assert!(steps.borrow().is_empty());

        let src = include_str!("applications.rs");
        let prod = src.split("mod tests").next().expect("tests module");
        let keydown_at = prod.find("onkeydown:").expect("dialog key handler");
        let keydown = &prod[keydown_at..keydown_at + 600];
        assert!(
            keydown.contains("handle_application_dialog_key("),
            "Escape must go through the close and focus handler: {keydown}"
        );
        assert!(
            keydown.contains("focus_element_by_id"),
            "the handler must move focus, not only recognize Escape: {keydown}"
        );
    }

    #[test]
    fn open_focuses_the_heading_and_shift_tab_stays_inside() {
        assert_eq!(
            application_dialog_initial_focus_id(),
            APPLICATION_DETAIL_TITLE_ID
        );
        assert_ne!(
            application_dialog_initial_focus_id(),
            APPLICATION_DETAIL_DIALOG_ID
        );

        let trigger = application_view_trigger_id("app1");
        let mut focused = None;
        let mut closed = false;
        let handled = handle_application_dialog_key(
            &Key::Tab,
            true,
            application_dialog_initial_focus_id(),
            &trigger,
            || closed = true,
            |id| focused = Some(id.to_string()),
        );
        assert!(handled);
        assert!(!closed);
        assert_eq!(focused.as_deref(), Some(APPLICATION_DETAIL_CLOSE_ID));

        // A shell that still holds focus must not let Shift+Tab escape either.
        focused = None;
        let handled = handle_application_dialog_key(
            &Key::Tab,
            true,
            APPLICATION_DETAIL_DIALOG_ID,
            &trigger,
            || closed = true,
            |id| focused = Some(id.to_string()),
        );
        assert!(handled);
        assert!(!closed);
        assert_eq!(focused.as_deref(), Some(APPLICATION_DETAIL_CLOSE_ID));

        let src = include_str!("applications.rs");
        let prod = src.split("mod tests").next().expect("tests module");
        let mounted_at = prod.find("onmounted:").expect("open focus");
        let mounted = &prod[mounted_at..mounted_at + 180];
        assert!(
            mounted.contains("application_dialog_initial_focus_id()"),
            "open must focus the heading: {mounted}"
        );
        assert!(
            !mounted.contains("APPLICATION_DETAIL_DIALOG_ID"),
            "open must not focus the dialog shell: {mounted}"
        );
    }

    #[test]
    fn tab_wraps_inside_the_application_dialog() {
        let ids = application_dialog_tab_ids();
        assert_eq!(
            ids,
            &[APPLICATION_DETAIL_TITLE_ID, APPLICATION_DETAIL_CLOSE_ID]
        );
        let trigger = application_view_trigger_id("app1");
        let first = ids[0];
        let last = ids[ids.len() - 1];

        let mut focused = None;
        let mut closed = false;
        let handled = handle_application_dialog_key(
            &Key::Tab,
            false,
            last,
            &trigger,
            || closed = true,
            |id| focused = Some(id.to_string()),
        );
        assert!(handled);
        assert!(!closed);
        assert_eq!(focused.as_deref(), Some(first));

        focused = None;
        let handled = handle_application_dialog_key(
            &Key::Tab,
            true,
            first,
            &trigger,
            || closed = true,
            |id| focused = Some(id.to_string()),
        );
        assert!(handled);
        assert!(!closed);
        assert_eq!(focused.as_deref(), Some(last));

        // Interior Tab stays with the browser so it can move to the next stop.
        let handled = handle_application_dialog_key(
            &Key::Tab,
            false,
            first,
            &trigger,
            || closed = true,
            |_| {},
        );
        assert!(!handled);
        assert!(!closed);
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
                    trigger_id: application_view_trigger_id("app1"),
                    on_close: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("Overwatch"), "{html}");
        assert!(html.contains("Tank"), "{html}");
        assert!(html.contains("Hello"), "{html}");
        assert!(html.contains("Ready"), "{html}");
        assert!(html.contains("2 Mar 2026, 08:15 UTC"), "{html}");
        assert!(!html.contains('\u{2014}'), "{html}");
    }
}
