use dioxus::prelude::*;

use crate::components::member_gate::Wall;
use crate::components::ui::inline_confirm::{
    focus_element_by_id, focus_element_by_id_soon, key_cancels,
};
use crate::components::ui::{PageShell, PageWidth, Pill, PillTone, SkeletonBars, btn_class};
use crate::hooks::{ApiResource, LIST_MAX_PAGES};
use crate::routes::Route;

/// Rows per page of an admin list (the shared `Pager`'s `page_size`).
pub const ADMIN_PAGE_SIZE: usize = 20;

/// Shared pending-state renderer for admin list/detail resources.
///
/// Call this from the `None` arm of a resource's `data` match. When the last
/// fetch failed (`resource.error` is set) it renders a plain sentence that
/// keeps the server's reason, plus a Retry button that bumps the resource's
/// refresh counter; otherwise it renders a labelled wait ("Loading
/// applications...") over skeleton rows. A failed, 403 or 500 fetch never
/// shows an endless wait.
///
/// Takes `&ApiResource` (not by value): call sites hold a live read-guard on
/// `resource.data` inside the `match`, and `ApiResource<T>` is only `Copy` when
/// `T: Copy`, so a by-value pass would move-conflict with that borrow. The retry
/// button copies out the `refresh` Signal (always `Copy`) to bump it.
pub fn admin_pending<T: 'static>(resource: &ApiResource<T>, label: &str) -> Element {
    if let Some(err) = resource.error.read().as_ref().cloned() {
        let mut refresh = resource.refresh;
        let message = load_error_text(label, &err);
        rsx! {
            div { class: "fetch-error-wrap admin-pending", role: "alert",
                p { class: "fetch-error", "{message}" }
                button {
                    r#type: "button",
                    class: "fetch-error__retry",
                    onclick: move |_| refresh += 1,
                    "Retry"
                }
            }
        }
    } else {
        let waiting = loading_text(label);
        rsx! {
            div { class: "admin-pending",
                p { class: "admin-pending-text", role: "status", "{waiting}" }
                SkeletonBars { rows: 3 }
            }
        }
    }
}

/// [`admin_pending`] for a page's main list: the failure is a card (title,
/// the server's reason, Retry and a way back to the dashboard), as the spec
/// asks for a failed load. Waiting is the same labelled skeleton.
pub fn admin_list_pending<T: 'static>(resource: &ApiResource<T>, label: &str) -> Element {
    let Some(err) = resource.error.read().as_ref().cloned() else {
        return admin_pending(resource, label);
    };
    let mut refresh = resource.refresh;
    let reason = err.trim().to_string();
    let reason = if reason.is_empty() {
        "The server did not answer. Try again in a moment.".to_string()
    } else {
        reason
    };
    rsx! {
        div { class: "admin-pending", role: "alert",
            section { class: "empty-card admin-load-error",
                h2 { "Could not load {label}" }
                p { "{reason}" }
                div { class: "empty-card-actions",
                    button {
                        r#type: "button",
                        class: "btn is-primary",
                        onclick: move |_| refresh += 1,
                        "Retry"
                    }
                    Link { to: Route::AdminDashboard {}, class: "btn is-ghost", "Back to dashboard" }
                }
            }
        }
    }
}

/// "Loading applications...": the labelled wait of a list or a page.
pub fn loading_text(label: &str) -> String {
    format!("Loading {label}...")
}

/// A failed load in plain words, with the server's reason kept at the end.
pub fn load_error_text(label: &str, reason: &str) -> String {
    let reason = reason.trim();
    if reason.is_empty() {
        format!("Could not load {label}.")
    } else {
        format!("Could not load {label}. {reason}")
    }
}

/// Visible cap for a cursor list that stopped with a next cursor still set.
///
/// The status line is always rendered and only its text changes, so the cap is
/// announced when it appears. It is empty (and out of the layout) until
/// `truncated` is true, so pages under the page budget look the same. "Load
/// more" raises the budget by another auto-follow batch; it sits after the
/// status line, not inside it. Call this outside a live `resource.data.read()`
/// guard.
pub fn list_cap_notice<T: 'static>(resource: &ApiResource<T>, what: &str) -> Element {
    let truncated = *resource.truncated.read();
    let shown = *resource.shown.read();
    let text = cap_notice_text(truncated, shown, what);
    let mut budget = resource.page_budget;
    rsx! {
        div { class: if truncated { "list-cap-notice" } else { "list-cap-notice is-idle" },
            p { class: "list-cap-notice__text", role: "status", "{text}" }
            if truncated {
                button {
                    r#type: "button",
                    class: "list-cap-notice__more",
                    onclick: move |_| budget += LIST_MAX_PAGES,
                    "Load more"
                }
            }
        }
    }
}

/// Text of the cap status line. Empty while nothing was cut off.
fn cap_notice_text(truncated: bool, shown: usize, what: &str) -> String {
    if truncated {
        format!("Showing the first {shown} {what}.")
    } else {
        String::new()
    }
}

/// Client-side authorization denial for admin pages whose server tier is stricter
/// than the officer+ `AdminLayout` guard. Defense-in-depth: rendered when a user
/// without the required role reaches the page (e.g. by typing the URL directly).
///
/// It is the whole page: the wall card in the narrow container, with the
/// page's one H1. The person is signed in, so there is no sign-in link.
#[component]
pub fn AccessDenied(message: String) -> Element {
    crate::head::use_page_title(Some("No access".to_string()));
    rsx! {
        PageShell { width: PageWidth::Narrow, class: "admin-no-access",
            Wall {
                title: "You do not have access",
                lead: message,
                page_title: true,
                actions: rsx! {
                    Link { to: Route::AdminDashboard {}, class: btn_class(true), "Back to dashboard" }
                    Link { to: Route::Home {}, class: btn_class(false), "Return home" }
                },
            }
        }
    }
}

/// The admin table.
///
/// `num_cols` lists the indexes of right-aligned columns (numbers and dates);
/// give the matching cells `class: "num"`. `label` names the table for
/// assistive tech.
///
/// `cards: true` turns every row into a stacked card at phone width, with no
/// second copy of the markup. The cells then say what they are:
/// - `cell-select`: a row checkbox (bulk select), first on the top line;
/// - `cell-who`: the name of the row, top line of the card;
/// - `cell-badge`: the status pill, on its own line under the name;
/// - `cell-meta`: short facts joined on one line ("Tank · 10 Nov 2026");
/// - `cell-actions`: the cell that holds a [`RowActions`];
/// - any other cell is a full line; give it `"data-label": "Games"` so the
///   line carries its column name.
///
/// Without `cards` the table keeps its columns on phone and scrolls sideways
/// inside its own box.
#[component]
pub fn DataTable(
    headers: Vec<&'static str>,
    #[props(default)] num_cols: Vec<usize>,
    #[props(default = false)] cards: bool,
    #[props(default)] label: Option<String>,
    children: Element,
) -> Element {
    rsx! {
        div { class: if cards { "data-table-scroll is-cards" } else { "data-table-scroll" },
            table { class: "data-table", aria_label: label,
                thead {
                    tr {
                        for (index, h) in headers.iter().enumerate() {
                            th {
                                scope: "col",
                                class: if num_cols.contains(&index) { "num" },
                                "{h}"
                            }
                        }
                    }
                }
                tbody { {children} }
            }
        }
    }
}

/// Id of the "Actions" button of a [`RowActions`] with this id stem. Pass it
/// as `restore_focus_id` to a dialog an overflow action opens.
pub fn row_menu_toggle_id(id: &str) -> String {
    format!("{id}-toggle")
}

/// Id of the Delete button inside a [`RowActions`] with this id stem. A
/// delete dialog gives focus back to it, or to [`row_menu_toggle_id`] when
/// Delete is folded away (phone).
pub fn delete_button_id(id: &str) -> String {
    format!("{id}-delete")
}

/// Id of the overflow group the "Actions" button controls.
fn row_menu_group_id(id: &str) -> String {
    format!("{id}-more")
}

/// The actions of one row.
///
/// `children` are the always visible actions (at most two, so the phone card
/// keeps the name beside them). `more` holds the rest: it sits behind an
/// "Actions" button (`aria-expanded`, `aria-controls`) that opens a named
/// group in place. Escape closes the group and returns focus to the button;
/// choosing an action closes it too. Nothing floats, so nothing is clipped by
/// the table's scroll box.
///
/// `inline_on_desktop: true` is for a short set: on desktop every action is in
/// the row and there is no "Actions" button; phone folds `more` as above.
///
/// `id` is the id stem (record id plus a name, `"member-actions-{id}"`) and
/// `label` names the group ("Actions for Player01"). Both are needed only
/// with `more`.
#[component]
pub fn RowActions(
    #[props(default)] id: String,
    #[props(default)] label: String,
    #[props(default)] more: Option<Element>,
    #[props(default = false)] inline_on_desktop: bool,
    children: Element,
) -> Element {
    let mut open = use_signal(|| false);
    let Some(more) = more else {
        return rsx! {
            div { class: "row-actions",
                div { class: "row-actions-main", {children} }
            }
        };
    };
    let toggle_id = row_menu_toggle_id(&id);
    let group_id = row_menu_group_id(&id);
    let class = row_actions_class(open(), inline_on_desktop);
    let focus_back = toggle_id.clone();
    let focus_after_choice = toggle_id.clone();
    rsx! {
        div {
            class: "{class}",
            onkeydown: move |evt| {
                if open() && key_cancels(&evt.key()) {
                    evt.stop_propagation();
                    open.set(false);
                    focus_element_by_id(&focus_back);
                }
            },
            div { class: "row-actions-main",
                {children}
                button {
                    r#type: "button",
                    id: "{toggle_id}",
                    class: "btn is-ghost row-menu-toggle",
                    aria_expanded: if open() { "true" } else { "false" },
                    aria_controls: "{group_id}",
                    aria_label: "{label}",
                    onclick: move |_| open.toggle(),
                    "Actions"
                }
            }
            div {
                class: "row-menu-group",
                id: "{group_id}",
                role: "group",
                aria_label: "{label}",
                // An action was chosen: fold the group again. Its button is
                // about to leave the layout, so focus goes to "Actions".
                onclick: move |_| {
                    if open() {
                        open.set(false);
                        focus_element_by_id(&focus_after_choice);
                    }
                },
                {more}
            }
        }
    }
}

fn row_actions_class(open: bool, inline_on_desktop: bool) -> String {
    let mut class = String::from("row-actions");
    if inline_on_desktop {
        class.push_str(" is-inline-desktop");
    }
    if open {
        class.push_str(" is-open");
    }
    class
}

const MONTHS: [&str; 12] = [
    "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
];

/// `2026-11-10T19:00:00Z` as (`2026-11-10`, `10 Nov 2026`): the machine date
/// and the visible one, as the public lists write it. `None` when the value
/// does not start with a date. The date is the UTC one the server stored.
pub fn admin_short_date(iso: &str) -> Option<(String, String)> {
    let date = iso.get(..10)?;
    let mut parts = date.split('-');
    let (year, month, day) = (parts.next()?, parts.next()?, parts.next()?);
    if year.len() != 4 || month.len() != 2 || day.len() != 2 {
        return None;
    }
    year.parse::<u16>().ok()?;
    let month_name = MONTHS.get(month.parse::<usize>().ok()?.checked_sub(1)?)?;
    let day_number = day.parse::<u8>().ok().filter(|d| (1..=31).contains(d))?;
    Some((
        date.to_string(),
        format!("{day_number} {month_name} {year}"),
    ))
}

/// `2026-11-17T01:32:00Z` as `17 Nov 2026, 01:32 UTC`: the admin date with
/// its time, for detail views. A value that is not a timestamp is returned
/// as it is.
pub fn admin_date_time(iso: &str) -> String {
    let Some((_, day)) = admin_short_date(iso) else {
        return iso.to_string();
    };
    match iso.get(10..16) {
        Some(rest) if rest.starts_with('T') => format!("{day}, {} UTC", &rest[1..]),
        _ => day,
    }
}

/// A date cell value: `10 Nov 2026` in a `<time>` that keeps the machine
/// date and never wraps. A value that is not a timestamp is shown as it is.
#[component]
pub fn AdminDate(iso: String) -> Element {
    match admin_short_date(&iso) {
        Some((machine, human)) => rsx! {
            time { class: "date", datetime: "{machine}", "{human}" }
        },
        None => rsx! {
            span { class: "date", "{iso}" }
        },
    }
}

/// Text of a form's or a dialog's live line.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub enum SaveState {
    /// Nothing to say. The line stays in the tree, empty.
    #[default]
    Idle,
    Saving,
    /// A short confirmation ("Settings saved.").
    Saved(String),
    /// What failed, with the server's reason ("Could not save. name taken").
    Failed(String),
}

impl SaveState {
    pub fn text(&self) -> String {
        match self {
            SaveState::Idle => String::new(),
            SaveState::Saving => "Saving...".to_string(),
            SaveState::Saved(text) | SaveState::Failed(text) => text.clone(),
        }
    }

    pub fn is_error(&self) -> bool {
        matches!(self, SaveState::Failed(_))
    }

    /// A failure sentence for a failed request: `what` is the plain part
    /// ("Could not save settings"), then the short text from
    /// [`crate::client_error::user_message`] (the server's reason, or one
    /// plain sentence for a network or unreadable answer). It takes the
    /// error itself, so a raw browser error cannot reach the line.
    pub fn failed(what: &str, err: &scuffed_api_client::ClientError) -> Self {
        Self::failed_text(what, &crate::client_error::user_message(err))
    }

    /// [`SaveState::failed`] for a reason that is already page text (a
    /// check made on the page, not a request error).
    pub fn failed_text(what: &str, reason: &str) -> Self {
        let reason = reason.trim();
        if reason.is_empty() {
            SaveState::Failed(format!("{what}."))
        } else {
            SaveState::Failed(format!("{what}. {reason}"))
        }
    }
}

/// The save line of a form: one `role="status"` paragraph that is always
/// rendered, so a change of its text is announced. Put it beside the Save
/// button, inside `.admin-form-actions`. Set `focusable` when a flow moves
/// focus to the line (a bulk result after its dialog closes): it then takes
/// `tabindex="-1"`, so focus lands on it and not on the body.
#[component]
pub fn FormStatus(
    state: SaveState,
    #[props(default)] id: Option<String>,
    #[props(default = false)] focusable: bool,
) -> Element {
    let text = state.text();
    rsx! {
        p {
            id,
            class: if state.is_error() { "admin-form-status is-error" } else { "admin-form-status" },
            role: "status",
            tabindex: if focusable { "-1" },
            "{text}"
        }
    }
}

/// Keep Tab inside the open dialog: from the last stop it goes to the first,
/// and with Shift from the first to the last. True when focus was moved, so
/// the caller cancels the key's default. Shared by the admin dialogs and the
/// DM compose dialog.
#[cfg(target_arch = "wasm32")]
pub(crate) fn trap_tab_in(dialog_id: &str, shift: bool) -> bool {
    use wasm_bindgen::JsCast;

    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return false;
    };
    let Some(dialog) = document.get_element_by_id(dialog_id) else {
        return false;
    };
    let Ok(stops) = dialog.query_selector_all(
        "a[href], button:not([disabled]), input:not([disabled]), select:not([disabled]), textarea:not([disabled]), [tabindex=\"0\"]",
    ) else {
        return false;
    };
    let count = stops.length();
    if count == 0 {
        return false;
    }
    let stop = |index: u32| {
        stops
            .item(index)
            .and_then(|node| node.dyn_into::<web_sys::HtmlElement>().ok())
    };
    let (first, last) = (stop(0), stop(count - 1));
    let active = document.active_element();
    let is_active = |el: &Option<web_sys::HtmlElement>| match (el, &active) {
        (Some(el), Some(active)) => el.is_same_node(Some(active)),
        _ => false,
    };
    let inside = active
        .as_ref()
        .is_some_and(|active| dialog.contains(Some(active)));
    // The title holds focus when the dialog opens. It is not a tab stop, so
    // Shift+Tab from it goes to the last one.
    let on_title = active
        .as_ref()
        .is_some_and(|active| active.get_attribute("tabindex").as_deref() == Some("-1"));
    let target = match tab_wraps_to(
        shift,
        inside,
        is_active(&first) || on_title,
        is_active(&last),
    ) {
        Some(TabEdge::First) => first,
        Some(TabEdge::Last) => last,
        None => return false,
    };
    match target {
        Some(target) => {
            let _ = target.focus();
            true
        }
        None => false,
    }
}

#[cfg(not(target_arch = "wasm32"))]
pub(crate) fn trap_tab_in(dialog_id: &str, shift: bool) -> bool {
    let _ = (dialog_id, shift);
    false
}

#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
#[derive(Debug, PartialEq, Eq)]
enum TabEdge {
    First,
    Last,
}

/// Where a Tab that would leave the dialog goes instead. `None` leaves the
/// key to the browser.
#[cfg_attr(not(any(test, target_arch = "wasm32")), allow(dead_code))]
fn tab_wraps_to(shift: bool, inside: bool, at_first: bool, at_last: bool) -> Option<TabEdge> {
    if shift {
        (!inside || at_first).then_some(TabEdge::Last)
    } else {
        (!inside || at_last).then_some(TabEdge::First)
    }
}

/// Default id stem of a [`FormModal`]. One form dialog is open at a time, so
/// the ids are unique on the page.
const FORM_MODAL_ID: &str = "form-modal";
/// "Keep editing" of the discard prompt; focus goes there when it shows.
const FORM_MODAL_KEEP_ID: &str = "form-modal-keep";

/// What a request to close the form dialog (Escape, the backdrop, Cancel)
/// does.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum CloseRequest {
    /// Close now: nothing was typed.
    Close,
    /// A field changed: ask before the draft is dropped.
    AskDiscard,
    /// A save is running: the dialog stays, so the request cannot be sent
    /// twice and its answer is not lost.
    Ignore,
}

/// Whether the dialog holds a draft that closing would drop. A dialog that
/// knows its draft passes `dirty` (its fields against their baseline, which
/// a save resets); one that does not falls back to `typed`, set by any input
/// in the body. `dirty` matters for dialogs whose table saves right away
/// (Roster, Game accounts): a role change or a removal there is not a draft,
/// and an add that went through clears its form.
fn draft_unsaved(dirty: Option<bool>, typed: bool) -> bool {
    dirty.unwrap_or(typed)
}

fn close_request(submitting: bool, changed: bool) -> CloseRequest {
    if submitting {
        CloseRequest::Ignore
    } else if changed {
        CloseRequest::AskDiscard
    } else {
        CloseRequest::Close
    }
}

/// Move focus back to where a dialog was opened from, once it is gone, and
/// only when focus was lost with it (it sits on the body). A flow that moved
/// focus on purpose (a result line) keeps it. `ids` are tried in order and
/// the first one that is shown wins: a row's Delete button is in the row on
/// desktop, while on phone it is folded behind "Actions".
pub(crate) fn restore_focus_if_lost_soon(ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    dioxus::core::spawn_forever(async move {
        #[cfg(feature = "web")]
        {
            gloo_timers::future::TimeoutFuture::new(0).await;
        }
        restore_focus_if_lost(&ids);
    });
}

#[cfg(target_arch = "wasm32")]
fn restore_focus_if_lost(ids: &[String]) {
    use wasm_bindgen::JsCast;
    let Some(document) = web_sys::window().and_then(|window| window.document()) else {
        return;
    };
    let lost = match (document.active_element(), document.body()) {
        (None, _) => true,
        (Some(active), Some(body)) => active.is_same_node(Some(body.as_ref())),
        (Some(_), None) => false,
    };
    if !lost {
        return;
    }
    for id in ids {
        let Some(el) = document.get_element_by_id(id) else {
            continue;
        };
        let Ok(el) = el.dyn_into::<web_sys::HtmlElement>() else {
            continue;
        };
        // No offset parent: it (or a box around it) is `display: none`.
        if el.offset_parent().is_none() {
            continue;
        }
        let _ = el.focus();
        return;
    }
}

#[cfg(not(target_arch = "wasm32"))]
fn restore_focus_if_lost(ids: &[String]) {
    let _ = ids;
}

/// Where focus goes after a confirmed delete removes a row (B9): the targets
/// of the row after it, else of the row before it, then `fallback` (the add
/// button or the list) in order. `keys` is the list in the order it shows;
/// `targets` maps a row key to that row's focus targets, the one shown on
/// desktop first. A key not in `keys` gives the fallback alone.
pub(crate) fn after_delete_focus_ids(
    keys: &[String],
    removed: &str,
    targets: impl Fn(&str) -> Vec<String>,
    fallback: &[&str],
) -> Vec<String> {
    let mut ids = Vec::new();
    if let Some(at) = keys.iter().position(|k| k == removed) {
        if let Some(next) = keys.get(at + 1) {
            ids.extend(targets(next));
        }
        if let Some(prev) = at.checked_sub(1).and_then(|i| keys.get(i)) {
            ids.extend(targets(prev));
        }
    }
    ids.extend(fallback.iter().map(|id| id.to_string()));
    ids
}

/// After a confirmed delete went through: once `gone_id` (an element of the
/// deleted row) has left the page, that is the list refreshed, focus goes to
/// the first shown id of `ids` if it was lost with the row. Focus the member
/// moved on purpose stays. Gives up waiting after about three seconds and
/// then only moves focus that is already lost.
pub(crate) fn focus_after_delete_soon(gone_id: String, ids: Vec<String>) {
    focus_when_soon(move || !element_exists(&gone_id), ids);
}

/// [`focus_after_delete_soon`] for a row that may stay after its delete
/// (an admin hero icons row whose member still shares keeps a row with no
/// icons). Waits until `watch_id` (the row's count cell) has left the page or
/// reads `settled`, that is the refreshed list is on screen, then moves focus
/// that was lost to the first shown id of `ids`.
pub(crate) fn focus_after_refresh_soon(watch_id: String, settled: String, ids: Vec<String>) {
    focus_when_soon(
        move || list_refreshed(element_text(&watch_id).as_deref(), &settled),
        ids,
    );
}

/// The refreshed list is on screen: the watched cell is gone (the row went)
/// or shows the value the delete leaves (the row stayed).
fn list_refreshed(text: Option<&str>, settled: &str) -> bool {
    text.is_none_or(|t| t.trim() == settled)
}

/// Polls `done` every 50 ms for up to about three seconds, then moves focus
/// that was lost to the first shown id of `ids`.
fn focus_when_soon(done: impl Fn() -> bool + 'static, ids: Vec<String>) {
    if ids.is_empty() {
        return;
    }
    dioxus::core::spawn_forever(async move {
        #[cfg(feature = "web")]
        {
            for _ in 0..60 {
                gloo_timers::future::TimeoutFuture::new(50).await;
                if done() {
                    break;
                }
            }
        }
        #[cfg(not(feature = "web"))]
        let _ = &done;
        restore_focus_if_lost(&ids);
    });
}

#[cfg(target_arch = "wasm32")]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn element_text(id: &str) -> Option<String> {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        .map(|el| el.text_content().unwrap_or_default())
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn element_text(id: &str) -> Option<String> {
    let _ = id;
    None
}

#[cfg(target_arch = "wasm32")]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn element_exists(id: &str) -> bool {
    web_sys::window()
        .and_then(|window| window.document())
        .and_then(|document| document.get_element_by_id(id))
        .is_some()
}

#[cfg(not(target_arch = "wasm32"))]
#[cfg_attr(not(feature = "web"), allow(dead_code))]
fn element_exists(id: &str) -> bool {
    let _ = id;
    false
}

/// The ids a closed dialog gives focus back to, in order.
fn restore_ids(primary: Option<String>, fallback: Option<String>) -> Vec<String> {
    primary.into_iter().chain(fallback).collect()
}

/// The edit dialog: a titled modal with the form fields as children, Cancel
/// and a solid submit button, and a live line that carries the save state
/// and the server's reason for a failed save.
///
/// It opens with focus on its title and keeps Tab inside. Escape, the
/// backdrop and Cancel close it, except while it saves (they are ignored)
/// and once a field changed (it asks "Keep editing" or "Discard changes"
/// first). Pass `restore_focus_id` (the button that opened it) to get focus
/// back there when it closes, by any path; `restore_focus_fallback` is tried
/// when that one is not shown. `submit_label` replaces "Save". `error` is the
/// failure text; keep the server's reason in it. `dirty` is the dialog's
/// own unsaved state against its baseline; leave it out to ask after any
/// input.
#[component]
pub fn FormModal(
    title: String,
    open: bool,
    submitting: bool,
    on_close: EventHandler<()>,
    on_submit: EventHandler<()>,
    #[props(default = false)] danger: bool,
    #[props(default = false)] wide: bool,
    #[props(default)] submit_label: Option<String>,
    #[props(default)] error: Option<String>,
    #[props(default)] restore_focus_id: Option<String>,
    #[props(default)] restore_focus_fallback: Option<String>,
    #[props(default)] dirty: Option<bool>,
    children: Element,
) -> Element {
    if !open {
        return rsx! {};
    }
    rsx! {
        FormModalOpen {
            title,
            submitting,
            on_close,
            on_submit,
            danger,
            wide,
            submit_label,
            error,
            restore_focus: restore_ids(restore_focus_id, restore_focus_fallback),
            dirty,
            {children}
        }
    }
}

/// The open dialog. Split from `FormModal` so the focus effect runs once per
/// open, and the "changed" flag starts clean on every open.
#[component]
fn FormModalOpen(
    title: String,
    submitting: bool,
    on_close: EventHandler<()>,
    on_submit: EventHandler<()>,
    danger: bool,
    wide: bool,
    submit_label: Option<String>,
    error: Option<String>,
    restore_focus: Vec<String>,
    dirty: Option<bool>,
    children: Element,
) -> Element {
    let title_id = format!("{FORM_MODAL_ID}-title");
    let status_id = format!("{FORM_MODAL_ID}-status");
    let focus_title = title_id.clone();
    use_effect(move || focus_element_by_id(&focus_title));
    // Closed by any path (Cancel, Escape, a save that went through): focus
    // goes back to the opener, unless the flow already moved it.
    use_drop(move || restore_focus_if_lost_soon(restore_focus.clone()));

    // Set by any input or change inside the body, so a typed draft is never
    // dropped by one stray Escape.
    let mut changed = use_signal(|| false);
    let mut asking = use_signal(|| false);
    let request_close = use_callback(move |_: ()| {
        match close_request(submitting, draft_unsaved(dirty, changed())) {
            CloseRequest::Ignore => {}
            CloseRequest::Close => on_close.call(()),
            CloseRequest::AskDiscard => {
                asking.set(true);
                focus_element_by_id_soon(FORM_MODAL_KEEP_ID.to_string());
            }
        }
    });
    let keep_editing = use_callback(move |_: ()| {
        asking.set(false);
        focus_element_by_id_soon(format!("{FORM_MODAL_ID}-title"));
    });
    rsx! {
        div {
            class: "form-modal-overlay",
            onclick: move |_| request_close.call(()),
            onkeydown: move |evt| {
                if key_cancels(&evt.key()) {
                    evt.stop_propagation();
                    if asking() {
                        keep_editing.call(());
                    } else {
                        request_close.call(());
                    }
                } else if evt.key() == Key::Tab
                    && trap_tab_in(FORM_MODAL_ID, evt.modifiers().contains(Modifiers::SHIFT))
                {
                    evt.prevent_default();
                }
            },
            div {
                class: if wide { "form-modal wide" } else { "form-modal" },
                id: FORM_MODAL_ID,
                role: "dialog",
                aria_modal: "true",
                aria_labelledby: "{title_id}",
                aria_describedby: "{status_id}",
                onclick: move |e| e.stop_propagation(),
                h2 { id: "{title_id}", class: "form-modal-header", tabindex: "-1", "{title}" }
                div {
                    class: "form-modal-body",
                    oninput: move |_| changed.set(true),
                    onchange: move |_| changed.set(true),
                    {children}
                }
                FormModalFooter {
                    status_id: status_id.clone(),
                    submitting,
                    asking: asking(),
                    danger,
                    submit_label,
                    error,
                    on_cancel: move |_| request_close.call(()),
                    on_submit,
                    on_keep: move |_| keep_editing.call(()),
                    on_discard: move |_| on_close.call(()),
                }
            }
        }
    }
}

/// The footer of the form dialog: the live status line (always rendered,
/// only its text changes), then either Cancel and the submit button, or,
/// while it asks about a changed draft, "Keep editing" and "Discard changes".
#[component]
fn FormModalFooter(
    status_id: String,
    submitting: bool,
    asking: bool,
    danger: bool,
    submit_label: Option<String>,
    error: Option<String>,
    on_cancel: EventHandler<()>,
    on_submit: EventHandler<()>,
    on_keep: EventHandler<()>,
    on_discard: EventHandler<()>,
) -> Element {
    let state = match (&error, submitting) {
        (_, true) => SaveState::Saving,
        (Some(reason), false) => SaveState::Failed(reason.clone()),
        (None, false) => SaveState::Idle,
    };
    let status = if asking {
        "You have unsaved changes. Discard them?".to_string()
    } else {
        state.text()
    };
    let label = submit_label.unwrap_or_else(|| "Save".to_string());
    let label = if submitting {
        "Saving...".to_string()
    } else {
        label
    };
    rsx! {
        div { class: "form-modal-footer",
            p {
                id: "{status_id}",
                class: if state.is_error() && !asking { "form-modal-status is-error" } else { "form-modal-status" },
                role: "status",
                "{status}"
            }
            if asking {
                button {
                    id: FORM_MODAL_KEEP_ID,
                    class: "btn is-ghost",
                    r#type: "button",
                    onclick: move |_| on_keep.call(()),
                    "Keep editing"
                }
                button {
                    class: "btn is-danger",
                    r#type: "button",
                    onclick: move |_| on_discard.call(()),
                    "Discard changes"
                }
            } else {
                button {
                    class: "btn is-ghost",
                    r#type: "button",
                    // Busy, but still focusable. A save that is running
                    // cannot be walked away from.
                    aria_disabled: if submitting { "true" },
                    onclick: move |_| on_cancel.call(()),
                    "Cancel"
                }
                button {
                    class: if danger { "btn is-danger" } else { "btn is-primary" },
                    r#type: "button",
                    // Busy, but still focusable: focus is not dropped
                    // while the request runs.
                    aria_disabled: if submitting { "true" },
                    onclick: move |_| {
                        if !submitting {
                            on_submit.call(());
                        }
                    },
                    "{label}"
                }
            }
        }
    }
}

/// Id of the Cancel button, so focus can start there when the dialog opens.
const CONFIRM_CANCEL_ID: &str = "confirm-dialog-cancel";
const CONFIRM_TITLE_ID: &str = "confirm-dialog-title";
const CONFIRM_DIALOG_ID: &str = "confirm-dialog";

/// Modal confirm. Opens with focus on Cancel, keeps Tab inside, closes on
/// Esc, and names the action on the solid button (`confirm_label`, default
/// "Confirm"). Pass `restore_focus_id` to move focus back to the button that
/// opened it when the dialog closes (cancel or confirm), and
/// `restore_focus_fallback` for when that button is not shown.
#[component]
pub fn ConfirmDialog(
    title: String,
    message: String,
    open: bool,
    #[props(default = false)] danger: bool,
    #[props(default = "Confirm".to_string())] confirm_label: String,
    #[props(default)] restore_focus_id: Option<String>,
    #[props(default)] restore_focus_fallback: Option<String>,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
    #[props(default = None)] extra: Option<Element>,
) -> Element {
    if !open {
        return rsx! {};
    }
    rsx! {
        ConfirmDialogOpen {
            title,
            message,
            danger,
            confirm_label,
            restore_focus: restore_ids(restore_focus_id, restore_focus_fallback),
            on_confirm,
            on_cancel,
            extra,
        }
    }
}

/// The open dialog. Split from `ConfirmDialog` so the focus effect runs once
/// per open instead of on every render of a closed dialog.
#[component]
fn ConfirmDialogOpen(
    title: String,
    message: String,
    danger: bool,
    confirm_label: String,
    restore_focus: Vec<String>,
    on_confirm: EventHandler<()>,
    on_cancel: EventHandler<()>,
    extra: Option<Element>,
) -> Element {
    use_effect(|| focus_element_by_id(CONFIRM_CANCEL_ID));
    // Closed by Cancel, Escape or the confirm: focus goes back to the
    // opener (the first one shown), unless the flow already moved it.
    use_drop(move || restore_focus_if_lost_soon(restore_focus.clone()));

    let cancel = use_callback(move |_: ()| on_cancel.call(()));
    rsx! {
        div {
            class: "form-modal-overlay",
            onclick: move |_| cancel.call(()),
            onkeydown: move |e| {
                if key_cancels(&e.key()) {
                    e.stop_propagation();
                    cancel.call(());
                } else if e.key() == Key::Tab
                    && trap_tab_in(CONFIRM_DIALOG_ID, e.modifiers().contains(Modifiers::SHIFT))
                {
                    e.prevent_default();
                }
            },
            div {
                class: "form-modal",
                id: CONFIRM_DIALOG_ID,
                role: "dialog",
                "aria-modal": "true",
                "aria-labelledby": CONFIRM_TITLE_ID,
                onclick: move |e| e.stop_propagation(),
                h2 { id: CONFIRM_TITLE_ID, class: "form-modal-header", "{title}" }
                div { class: "confirm-body",
                    p { "{message}" }
                    if let Some(extra) = extra {
                        {extra}
                    }
                }
                div { class: "form-modal-footer",
                    button {
                        id: CONFIRM_CANCEL_ID,
                        class: "btn is-ghost",
                        r#type: "button",
                        onclick: move |_| cancel.call(()),
                        "Cancel"
                    }
                    button {
                        class: if danger { "btn is-danger" } else { "btn is-primary" },
                        r#type: "button",
                        onclick: move |_| on_confirm.call(()),
                        "{confirm_label}"
                    }
                }
            }
        }
    }
}

/// Tone of a status word. Unknown words are neutral.
pub fn status_tone(status: &str) -> PillTone {
    match status {
        "pending" | "in_progress" => PillTone::Warn,
        "trial" => PillTone::Info,
        "accepted" | "active" | "completed" => PillTone::Ok,
        "rejected" | "declined" | "inactive" => PillTone::Danger,
        "registration" => PillTone::Accent,
        _ => PillTone::Neutral,
    }
}

/// The word a status pill shows: `rejected` reads "Declined", and a stored
/// `in_progress` reads "In progress". The pill recipe upper-cases it.
pub fn status_label(status: &str) -> String {
    match status {
        "rejected" => "Declined".to_string(),
        other => {
            let words = other.replace('_', " ");
            let mut chars = words.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().chain(chars).collect(),
                None => String::new(),
            }
        }
    }
}

/// A status as a pill from the shared recipe: the tone is never the only
/// signal, the pill always carries the word.
#[component]
pub fn StatusPill(status: String) -> Element {
    let label = status_label(&status);
    rsx! { Pill { tone: status_tone(&status), "{label}" } }
}

/// A role as a neutral pill from the shared recipe.
#[component]
pub fn RolePill(role: String) -> Element {
    let label = status_label(&role);
    rsx! { Pill { tone: PillTone::Neutral, "{label}" } }
}

/// A count tile (the Home stat treatment): the value in the display face
/// with tabular figures, the label under it.
#[component]
pub fn SummaryCard(
    value: String,
    label: &'static str,
    /// When set, the card is a click-through link (admin dashboard KPIs).
    #[props(default)]
    to: Option<Route>,
) -> Element {
    let class = if to.is_some() {
        "summary-card summary-card-link"
    } else {
        "summary-card"
    };
    let body = rsx! {
        div { class: "value", "{value}" }
        div { class: "label", "{label}" }
    };
    match to {
        Some(route) => rsx! {
            Link { to: route, class: "{class}", {body} }
        },
        None => rsx! {
            div { class: "{class}", {body} }
        },
    }
}

#[cfg(test)]
mod confirm_dialog_tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn closed_dialog_renders_nothing() {
        fn app() -> Element {
            rsx! {
                ConfirmDialog {
                    title: "Delete it?".to_string(),
                    message: "Gone for good.".to_string(),
                    open: false,
                    on_confirm: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        assert!(!render(app).contains("Delete it?"));
    }

    #[test]
    fn open_dialog_is_a_labelled_modal_with_a_named_action() {
        fn app() -> Element {
            rsx! {
                ConfirmDialog {
                    title: "Delete it?".to_string(),
                    message: "Gone for good.".to_string(),
                    open: true,
                    danger: true,
                    confirm_label: "Delete".to_string(),
                    on_confirm: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(html.contains("role=\"dialog\""), "{html}");
        assert!(html.contains("aria-modal=\"true\""), "{html}");
        assert!(
            html.contains("aria-labelledby=\"confirm-dialog-title\""),
            "{html}"
        );
        assert!(html.contains("id=\"confirm-dialog-cancel\""), "{html}");
        assert!(html.contains(">Delete</button>"), "{html}");
        assert!(!html.contains(">Confirm</button>"), "{html}");
    }

    #[test]
    fn default_label_is_still_confirm() {
        fn app() -> Element {
            rsx! {
                ConfirmDialog {
                    title: "Sure?".to_string(),
                    message: "Really.".to_string(),
                    open: true,
                    on_confirm: |_| {},
                    on_cancel: |_| {},
                }
            }
        }
        assert!(render(app).contains(">Confirm</button>"));
    }
}

/// Page tests for admin routes: the route rendered with the contexts the
/// real app provides (session, site settings, theme, toasts). The first
/// render is the loading state, since no request is answered in a test.
#[cfg(test)]
pub(crate) mod test_page {
    use dioxus::prelude::*;
    use scuffed_types::{OrgRole, UserInfo};

    use crate::routes::Route;
    use crate::state::AuthState;

    thread_local! {
        static SCENE: std::cell::RefCell<(String, Option<OrgRole>)> =
            const { std::cell::RefCell::new((String::new(), None)) };
    }

    fn shell() -> Element {
        let (path, role) = SCENE.with(|slot| slot.borrow().clone());
        let history =
            use_hook(|| std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path(path)));
        let auth = use_signal(|| AuthState {
            user: role.map(|role| UserInfo {
                id: "u1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role: Some(role),
            }),
            loading: false,
        });
        use_context_provider(|| auth);
        crate::state::provide_site_settings();
        rsx! {
            crate::theme::ThemeProvider {
                crate::components::ToastProvider {
                    dioxus::router::components::HistoryProvider {
                        history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                        Router::<Route> {}
                    }
                }
            }
        }
    }

    /// `role: None` is a guest.
    pub fn render_admin_route(path: &str, role: Option<OrgRole>) -> String {
        SCENE.with(|slot| *slot.borrow_mut() = (path.to_string(), role));
        let mut dom = VirtualDom::new(shell);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }
}

#[cfg(test)]
mod admin_pattern_tests {
    use super::*;
    use crate::components::ui::test_support::{render, render_routed};

    #[test]
    fn table_marks_right_aligned_columns_and_card_mode() {
        fn app() -> Element {
            rsx! {
                DataTable {
                    headers: vec!["Applicant", "Date", "Actions"],
                    num_cols: vec![1],
                    cards: true,
                    label: "Applications".to_string(),
                    tr {
                        td { class: "cell-who", "Player04" }
                        td { class: "cell-meta num", AdminDate { iso: "2026-11-10T19:00:00Z".to_string() } }
                        td { class: "cell-actions",
                            RowActions { button { r#type: "button", class: "btn is-ghost", "View" } }
                        }
                    }
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains("<div class=\"data-table-scroll is-cards\"><table class=\"data-table\" aria-label=\"Applications\">"),
            "{html}"
        );
        assert!(html.contains("<th scope=\"col\">Applicant</th>"), "{html}");
        assert!(
            html.contains("<th scope=\"col\" class=\"num\">Date</th>"),
            "{html}"
        );
        assert!(
            html.contains("<time class=\"date\" datetime=\"2026-11-10\">10 Nov 2026</time>"),
            "{html}"
        );
        // No overflow actions: no "Actions" button and no group.
        assert!(
            html.contains(
                "<div class=\"row-actions\"><div class=\"row-actions-main\"><button type=\"button\" class=\"btn is-ghost\">View</button></div></div>"
            ),
            "{html}"
        );
        assert!(!html.contains("aria-expanded"), "{html}");
    }

    #[test]
    fn plain_table_keeps_its_old_shape() {
        fn app() -> Element {
            rsx! {
                DataTable { headers: vec!["Name"], tr { td { "Player01" } } }
            }
        }
        let html = render(app);
        assert!(
            html.starts_with(
                "<div class=\"data-table-scroll\"><table class=\"data-table\"><thead>"
            ),
            "{html}"
        );
    }

    #[test]
    fn overflow_actions_sit_behind_a_named_disclosure() {
        fn app() -> Element {
            rsx! {
                RowActions {
                    id: "member-actions-m1".to_string(),
                    label: "Actions for Player01".to_string(),
                    more: rsx! {
                        button { r#type: "button", class: "btn is-ghost", "Change role" }
                        button { r#type: "button", class: "btn is-danger", "Deactivate" }
                    },
                    button { r#type: "button", class: "btn is-ghost", "Edit" }
                }
            }
        }
        let html = render(app);
        assert!(html.starts_with("<div class=\"row-actions\">"), "{html}");
        // The button says what it opens, its state, and which group.
        assert!(
            html.contains(
                "<button type=\"button\" id=\"member-actions-m1-toggle\" class=\"btn is-ghost row-menu-toggle\" aria-expanded=\"false\" aria-controls=\"member-actions-m1-more\" aria-label=\"Actions for Player01\">Actions</button>"
            ),
            "{html}"
        );
        // The group is always in the tree, so aria-controls resolves.
        assert!(
            html.contains(
                "<div class=\"row-menu-group\" id=\"member-actions-m1-more\" role=\"group\" aria-label=\"Actions for Player01\">"
            ),
            "{html}"
        );
        // Always visible actions come before the "Actions" button.
        let edit = html.find(">Edit<").expect("edit");
        let toggle = html.find(">Actions<").expect("toggle");
        let role = html.find(">Change role<").expect("overflow action");
        assert!(edit < toggle && toggle < role, "{html}");
        assert_eq!(
            row_menu_toggle_id("member-actions-m1"),
            "member-actions-m1-toggle"
        );
        assert_eq!(row_actions_class(true, false), "row-actions is-open");
        assert_eq!(
            row_actions_class(false, true),
            "row-actions is-inline-desktop"
        );
    }

    #[test]
    fn status_and_role_pills_use_the_shared_recipe() {
        for (status, tone, word) in [
            ("pending", PillTone::Warn, "Pending"),
            ("trial", PillTone::Info, "Trial"),
            ("accepted", PillTone::Ok, "Accepted"),
            ("rejected", PillTone::Danger, "Declined"),
            ("withdrawn", PillTone::Neutral, "Withdrawn"),
            ("in_progress", PillTone::Warn, "In progress"),
        ] {
            assert_eq!(status_tone(status), tone, "{status}");
            assert_eq!(status_label(status), word, "{status}");
        }
        fn app() -> Element {
            rsx! {
                StatusPill { status: "rejected".to_string() }
                RolePill { role: "officer".to_string() }
            }
        }
        let html = render(app);
        assert_eq!(
            html,
            "<span class=\"ui-pill ui-pill--danger\">Declined</span>\
             <span class=\"ui-pill ui-pill--neutral\">Officer</span>"
        );
    }

    #[test]
    fn dates_read_day_month_year_and_odd_values_pass_through() {
        assert_eq!(
            admin_short_date("2026-11-10T19:00:00Z"),
            Some(("2026-11-10".to_string(), "10 Nov 2026".to_string()))
        );
        assert_eq!(admin_short_date("soon"), None);
        fn app() -> Element {
            rsx! { AdminDate { iso: "soon".to_string() } }
        }
        assert_eq!(render(app), "<span class=\"date\">soon</span>");
    }

    #[test]
    fn save_line_is_always_rendered_and_keeps_the_reason() {
        fn idle() -> Element {
            rsx! { FormStatus { state: SaveState::Idle } }
        }
        assert_eq!(
            render(idle),
            "<p class=\"admin-form-status\" role=\"status\"></p>"
        );
        fn failed() -> Element {
            rsx! {
                FormStatus {
                    state: SaveState::failed(
                        "Could not save settings",
                        &scuffed_api_client::ClientError::http(400, r#"{"error":"name too long"}"#, None),
                    ),
                    id: "settings-save-status".to_string(),
                }
            }
        }
        assert_eq!(
            render(failed),
            "<p id=\"settings-save-status\" class=\"admin-form-status is-error\" role=\"status\">Could not save settings. HTTP error 400: name too long</p>"
        );
        assert_eq!(SaveState::Saving.text(), "Saving...");
        assert_eq!(
            SaveState::failed_text("Could not save", " "),
            SaveState::Failed("Could not save.".to_string())
        );
    }

    #[test]
    fn form_modal_is_a_labelled_dialog_with_a_live_line() {
        fn app() -> Element {
            rsx! {
                FormModal {
                    title: "Edit team".to_string(),
                    open: true,
                    submitting: false,
                    error: "Could not save the team. HTTP error 409: name taken".to_string(),
                    on_close: |_| {},
                    on_submit: |_| {},
                    p { "fields" }
                }
            }
        }
        let html = render(app);
        assert!(html.contains("role=\"dialog\""), "{html}");
        assert!(html.contains("aria-modal=\"true\""), "{html}");
        assert!(
            html.contains("aria-labelledby=\"form-modal-title\""),
            "{html}"
        );
        assert!(
            html.contains("<h2 id=\"form-modal-title\" class=\"form-modal-header\" tabindex=\"-1\">Edit team</h2>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<p id=\"form-modal-status\" class=\"form-modal-status is-error\" role=\"status\">Could not save the team. HTTP error 409: name taken</p>"
            ),
            "{html}"
        );
        assert!(html.contains(">Save</button>"), "{html}");
        assert!(!html.contains(" disabled"), "{html}");

        fn busy() -> Element {
            rsx! {
                FormModal {
                    title: "Edit team".to_string(),
                    open: true,
                    submitting: true,
                    submit_label: "Create".to_string(),
                    on_close: |_| {},
                    on_submit: |_| {},
                    p { "fields" }
                }
            }
        }
        let html = render(busy);
        assert!(html.contains("aria-disabled=\"true\""), "{html}");
        // Cancel is busy too: a running save cannot be walked away from
        // (Vision blocking 3 on PR 706).
        assert!(
            html.contains(
                "<button class=\"btn is-ghost\" type=\"button\" aria-disabled=\"true\">Cancel</button>"
            ),
            "{html}"
        );
        assert!(html.contains(">Saving...</button>"), "{html}");
        assert!(html.contains("role=\"status\">Saving...</p>"), "{html}");

        fn closed() -> Element {
            rsx! {
                FormModal {
                    title: "Edit team".to_string(),
                    open: false,
                    submitting: false,
                    on_close: |_| {},
                    on_submit: |_| {},
                    p { "fields" }
                }
            }
        }
        assert_eq!(render(closed), "");
    }

    #[test]
    fn a_dialog_that_knows_its_draft_does_not_ask_after_a_save() {
        // Vision S4 on PR 706: Roster and Game accounts asked "unsaved
        // changes" after edits that had already saved, because the typed
        // flag was the only baseline and nothing reset it. A dialog that
        // passes `dirty` is judged by its draft alone.
        assert!(!draft_unsaved(Some(false), true));
        assert!(draft_unsaved(Some(true), false));
        // Without `dirty`, any input still counts.
        assert!(draft_unsaved(None, true));
        assert!(!draft_unsaved(None, false));
        assert_eq!(
            close_request(false, draft_unsaved(Some(false), true)),
            CloseRequest::Close
        );

        let src = include_str!("admin_shared.rs");
        let open = src
            .split("fn FormModalOpen(")
            .nth(1)
            .and_then(|rest| rest.split("fn FormModalFooter(").next())
            .expect("FormModalOpen");
        assert!(open.contains("draft_unsaved(dirty, changed())"), "{open}");
        let modal = src
            .split("pub fn FormModal(")
            .nth(1)
            .and_then(|rest| rest.split("fn FormModalOpen(").next())
            .expect("FormModal");
        assert!(modal.contains("#[props(default)] dirty: Option<bool>,"));
        assert!(modal.contains("            dirty,\n"));
    }

    #[test]
    fn every_s4d_list_loop_keys_its_items() {
        // Vision nit 1 on PR 706: a loop body that is a block or a helper
        // call gives Dioxus no key, so it rebuilds every row after a removed
        // one and drops the focus inside it. Each item is keyed by its record
        // id with a `Fragment` (or the element itself).
        for (name, src) in [
            (
                "components/admin_shared.rs",
                include_str!("admin_shared.rs"),
            ),
            ("components/dm/compose.rs", include_str!("dm/compose.rs")),
            (
                "components/dm/conversation_list.rs",
                include_str!("dm/conversation_list.rs"),
            ),
            (
                "components/dm/message_thread.rs",
                include_str!("dm/message_thread.rs"),
            ),
            ("components/post/card.rs", include_str!("post/card.rs")),
            ("layouts/admin.rs", include_str!("../layouts/admin.rs")),
            (
                "pages/admin/announcements.rs",
                include_str!("../pages/admin/announcements.rs"),
            ),
            (
                "pages/admin/applications.rs",
                include_str!("../pages/admin/applications.rs"),
            ),
            (
                "pages/admin/articles.rs",
                include_str!("../pages/admin/articles.rs"),
            ),
            (
                "pages/admin/audit_log.rs",
                include_str!("../pages/admin/audit_log.rs"),
            ),
            (
                "pages/admin/forum.rs",
                include_str!("../pages/admin/forum.rs"),
            ),
            (
                "pages/admin/games.rs",
                include_str!("../pages/admin/games.rs"),
            ),
            (
                "pages/admin/hero_icons.rs",
                include_str!("../pages/admin/hero_icons.rs"),
            ),
            (
                "pages/admin/matches.rs",
                include_str!("../pages/admin/matches.rs"),
            ),
            (
                "pages/admin/member_matches.rs",
                include_str!("../pages/admin/member_matches.rs"),
            ),
            (
                "pages/admin/members.rs",
                include_str!("../pages/admin/members.rs"),
            ),
            (
                "pages/admin/moderation.rs",
                include_str!("../pages/admin/moderation.rs"),
            ),
            (
                "pages/admin/patch_notes.rs",
                include_str!("../pages/admin/patch_notes.rs"),
            ),
            (
                "pages/admin/relay.rs",
                include_str!("../pages/admin/relay.rs"),
            ),
            (
                "pages/admin/reports.rs",
                include_str!("../pages/admin/reports.rs"),
            ),
            (
                "pages/admin/schedule.rs",
                include_str!("../pages/admin/schedule.rs"),
            ),
            (
                "pages/admin/seasons.rs",
                include_str!("../pages/admin/seasons.rs"),
            ),
            (
                "pages/admin/settings.rs",
                include_str!("../pages/admin/settings.rs"),
            ),
            (
                "pages/admin/teams.rs",
                include_str!("../pages/admin/teams.rs"),
            ),
            (
                "pages/admin/tournaments.rs",
                include_str!("../pages/admin/tournaments.rs"),
            ),
            (
                "pages/article_author.rs",
                include_str!("../pages/article_author.rs"),
            ),
            ("pages/dm.rs", include_str!("../pages/dm.rs")),
            (
                "pages/forum_thread.rs",
                include_str!("../pages/forum_thread.rs"),
            ),
            (
                "pages/hero_icon_settings.rs",
                include_str!("../pages/hero_icon_settings.rs"),
            ),
            ("pages/members.rs", include_str!("../pages/members.rs")),
            (
                "pages/profile_settings.rs",
                include_str!("../pages/profile_settings.rs"),
            ),
            (
                "pages/stat_reports.rs",
                include_str!("../pages/stat_reports.rs"),
            ),
            ("pages/team_page.rs", include_str!("../pages/team_page.rs")),
        ] {
            let body = src.split("#[cfg(test)]").next().unwrap();
            let lines: Vec<&str> = body.lines().collect();
            for (i, line) in lines.iter().enumerate() {
                let t = line.trim();
                if !(t.starts_with("for ") && t.contains(" in ") && t.ends_with('{')) {
                    continue;
                }
                let next = lines[i + 1..]
                    .iter()
                    .map(|l| l.trim())
                    .find(|l| !l.is_empty() && !l.starts_with("//"))
                    .unwrap_or_default();
                assert!(
                    !next.starts_with('{'),
                    "{name}:{}: unkeyed loop body `{next}`",
                    i + 1
                );
            }
        }
    }

    #[test]
    fn a_refreshed_list_is_a_gone_cell_or_its_settled_value() {
        assert!(list_refreshed(None, "0"), "the row left");
        assert!(list_refreshed(Some(" 0 "), "0"), "the row stayed, now 0");
        assert!(!list_refreshed(Some("12"), "0"), "the old list is still up");
        // Both helpers share the one wait-then-restore body.
        let src = include_str!("admin_shared.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert_eq!(
            prod.matches("focus_when_soon(").count(),
            3,
            "two callers and the fn"
        );
        assert!(prod.contains("focus_when_soon(move || !element_exists(&gone_id), ids);"));
    }

    #[test]
    fn a_confirmed_delete_focuses_the_next_row_else_the_previous_else_the_fallback() {
        // Every delete flow S4d touches hands focus on once its row is gone.
        for (name, src) in [
            (
                "announcements",
                include_str!("../pages/admin/announcements.rs"),
            ),
            ("articles", include_str!("../pages/admin/articles.rs")),
            ("patch_notes", include_str!("../pages/admin/patch_notes.rs")),
            ("schedule", include_str!("../pages/admin/schedule.rs")),
            ("seasons", include_str!("../pages/admin/seasons.rs")),
            ("teams roster", include_str!("../pages/admin/teams.rs")),
            ("tournaments", include_str!("../pages/admin/tournaments.rs")),
            (
                "members game accounts",
                include_str!("../pages/admin/members.rs"),
            ),
            (
                "hero icons admin",
                include_str!("../pages/admin/hero_icons.rs"),
            ),
            (
                "hero icon settings",
                include_str!("../pages/hero_icon_settings.rs"),
            ),
            ("identity unlink", include_str!("../pages/identity.rs")),
            ("my reports", include_str!("../pages/my_reports.rs")),
        ] {
            let prod = src.split("#[cfg(test)]").next().unwrap();
            assert!(prod.contains("focus_after_delete_soon("), "{name}");
        }

        // B9: the deleted row's button leaves the page with it.
        let keys: Vec<String> = ["a", "b", "c"].iter().map(|k| k.to_string()).collect();
        let targets = |k: &str| vec![format!("{k}-edit"), format!("{k}-toggle")];
        assert_eq!(
            after_delete_focus_ids(&keys, "b", targets, &["new"]),
            ["c-edit", "c-toggle", "a-edit", "a-toggle", "new"]
        );
        // The last row: the row before it.
        assert_eq!(
            after_delete_focus_ids(&keys, "c", targets, &["new"]),
            ["b-edit", "b-toggle", "new"]
        );
        // The only row: the fallback.
        let one = vec!["a".to_string()];
        assert_eq!(
            after_delete_focus_ids(&one, "a", targets, &["list", "new"]),
            ["list", "new"]
        );
        // Not in the list (it moved): the fallback alone.
        assert_eq!(
            after_delete_focus_ids(&keys, "z", targets, &["new"]),
            ["new"]
        );
        // The focus move waits for the row to leave and only moves lost focus.
        let src = include_str!("admin_shared.rs");
        let body = src
            .split("pub(crate) fn focus_after_delete_soon(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("focus_after_delete_soon");
        assert!(body.contains("!element_exists(&gone_id)"), "{body}");
        let wait = src
            .split("fn focus_when_soon(")
            .nth(1)
            .and_then(|rest| rest.split("\n}\n").next())
            .expect("focus_when_soon");
        assert!(wait.contains("if done() {"), "{wait}");
        assert!(wait.contains("restore_focus_if_lost(&ids)"), "{wait}");
    }

    #[test]
    fn form_modal_close_is_ignored_while_saving_and_asks_once_a_field_changed() {
        // Vision blocking 3 on PR 706: one Escape dropped a typed draft, and
        // the dialog could be closed (and resubmitted) mid-save.
        assert_eq!(close_request(true, false), CloseRequest::Ignore);
        assert_eq!(close_request(true, true), CloseRequest::Ignore);
        assert_eq!(close_request(false, true), CloseRequest::AskDiscard);
        assert_eq!(close_request(false, false), CloseRequest::Close);

        // Escape, the backdrop and Cancel all go through the same request,
        // and the body marks the draft changed on any input or change.
        let src = include_str!("admin_shared.rs");
        let open = src
            .split("fn FormModalOpen(")
            .nth(1)
            .and_then(|rest| rest.split("fn FormModalFooter(").next())
            .expect("FormModalOpen");
        assert!(
            open.contains("onclick: move |_| request_close.call(())"),
            "backdrop"
        );
        assert!(open.contains("request_close.call(());"), "Escape");
        assert!(
            open.contains("on_cancel: move |_| request_close.call(())"),
            "Cancel"
        );
        assert!(open.contains("oninput: move |_| changed.set(true)"));
        assert!(open.contains("onchange: move |_| changed.set(true)"));
        assert!(
            !open.contains("on_close.call(());\n                    }"),
            "no direct close on Escape"
        );
    }

    #[test]
    fn form_modal_asks_before_discarding_with_the_live_line() {
        fn app() -> Element {
            rsx! {
                FormModalFooter {
                    status_id: "form-modal-status".to_string(),
                    submitting: false,
                    asking: true,
                    danger: false,
                    submit_label: None,
                    error: Some("HTTP error 409: slug taken".to_string()),
                    on_cancel: |_| {},
                    on_submit: |_| {},
                    on_keep: |_| {},
                    on_discard: |_| {},
                }
            }
        }
        let html = render(app);
        assert!(
            html.contains(
                "<p id=\"form-modal-status\" class=\"form-modal-status\" role=\"status\">You have unsaved changes. Discard them?</p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<button id=\"form-modal-keep\" class=\"btn is-ghost\" type=\"button\">Keep editing</button>"),
            "{html}"
        );
        assert!(
            html.contains("class=\"btn is-danger\" type=\"button\">Discard changes</button>"),
            "{html}"
        );
        assert!(!html.contains(">Save</button>"), "{html}");
        assert!(!html.contains(">Cancel</button>"), "{html}");
    }

    #[test]
    fn closed_dialogs_give_focus_back_by_any_path() {
        // Vision blocking 9 on PR 706: a save or a confirm left focus on the
        // body, and a cancel aimed at a hidden "Actions" button.
        assert_eq!(
            restore_ids(Some("row-delete".into()), Some("row-toggle".into())),
            ["row-delete", "row-toggle"]
        );
        assert_eq!(restore_ids(None, Some("t".into())), ["t"]);
        assert!(restore_ids(None, None).is_empty());
        let src = include_str!("admin_shared.rs");
        for (start, end) in [
            ("fn FormModalOpen(", "fn FormModalFooter("),
            ("fn ConfirmDialogOpen(", "pub fn status_tone("),
        ] {
            let body = src
                .split(start)
                .nth(1)
                .and_then(|rest| rest.split(end).next())
                .expect(start);
            assert!(
                body.contains(
                    "use_drop(move || restore_focus_if_lost_soon(restore_focus.clone()));"
                ),
                "{start}"
            );
        }
    }

    #[test]
    fn tab_wraps_at_the_ends_of_a_dialog_only() {
        assert_eq!(tab_wraps_to(false, true, false, true), Some(TabEdge::First));
        assert_eq!(tab_wraps_to(true, true, true, false), Some(TabEdge::Last));
        assert_eq!(tab_wraps_to(false, true, false, false), None);
        assert_eq!(tab_wraps_to(true, true, false, false), None);
        // Focus that left the dialog comes back in.
        assert_eq!(
            tab_wraps_to(false, false, false, false),
            Some(TabEdge::First)
        );
    }

    #[test]
    fn detail_dates_read_like_the_list_dates_plus_the_time() {
        assert_eq!(
            admin_date_time("2026-11-17T01:32:09Z"),
            "17 Nov 2026, 01:32 UTC"
        );
        assert_eq!(admin_date_time("2026-11-17"), "17 Nov 2026");
        assert_eq!(admin_date_time("soon"), "soon");
    }

    #[test]
    fn pending_states_are_labelled_and_keep_the_reason() {
        assert_eq!(loading_text("applications"), "Loading applications...");
        assert_eq!(
            load_error_text("applications", "HTTP error 500: boom"),
            "Could not load applications. HTTP error 500: boom"
        );
        assert_eq!(load_error_text("teams", ""), "Could not load teams.");
    }

    #[test]
    fn access_denied_is_a_whole_page_with_one_h1_and_no_sign_in() {
        let html = render_routed(|| {
            rsx! { AccessDenied { message: "You need admin permissions to view settings.".to_string() } }
        });
        assert!(
            html.contains("class=\"ui-shell ui-shell--narrow admin-no-access\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>You do not have access</h1>"), "{html}");
        assert!(
            html.contains("You need admin permissions to view settings."),
            "{html}"
        );
        assert!(!html.contains("Log in"), "{html}");
        assert!(html.contains(">Back to dashboard</a>"), "{html}");
    }
}

#[cfg(test)]
mod list_cap_notice_tests {
    use super::*;

    #[test]
    fn cap_text_is_empty_until_the_list_is_cut_off() {
        assert_eq!(cap_notice_text(false, 40, "events"), "");
        assert_eq!(
            cap_notice_text(true, 1000, "events"),
            "Showing the first 1000 events."
        );
    }

    #[component]
    fn Probe(truncated: bool) -> Element {
        let resource = ApiResource::<Vec<u8>> {
            data: use_resource(|| async { Some(Vec::new()) }),
            refresh: use_signal(|| 0u64),
            error: use_signal(|| None),
            truncated: use_signal(|| truncated),
            shown: use_signal(|| 1000usize),
            page_budget: use_signal(|| LIST_MAX_PAGES),
        };
        list_cap_notice(&resource, "events")
    }

    fn render(truncated: bool) -> String {
        let mut dom = VirtualDom::new_with_props(Probe, ProbeProps { truncated });
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn status_line_is_always_rendered_and_load_more_sits_outside_it() {
        let idle = render(false);
        assert_eq!(
            idle,
            "<div class=\"list-cap-notice is-idle\">\
             <p class=\"list-cap-notice__text\" role=\"status\"></p></div>"
        );

        let cut = render(true);
        assert!(
            cut.contains(
                "<p class=\"list-cap-notice__text\" role=\"status\">Showing the first 1000 events.</p>\
                 <button type=\"button\" class=\"list-cap-notice__more\">Load more</button>"
            ),
            "{cut}"
        );
        assert_eq!(cut.matches("role=\"status\"").count(), 1, "{cut}");
    }
}
