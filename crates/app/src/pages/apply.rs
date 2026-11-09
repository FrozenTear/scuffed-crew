use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ui::{BtnVariant, Button, Card, InlineConfirm, Pill, PillTone, Textarea};
use crate::components::{Toast, list_cap_notice, retry_label, use_toast};
use crate::hooks::{use_api, use_api_list};
use crate::routes::Route;
use crate::state::auth::use_auth;
use crate::state::{loaded_site_settings, use_site_settings};
use crate::util::{FetchClass, classify_fetch};
use scuffed_api_client::ApiClient;
use scuffed_types::Game;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyScreen {
    Loading,
    Error,
    Ready,
}

/// Auth still booting, or settings still in flight, stays on the loading line.
/// A failed settings load is its own screen so `/apply` cannot sit on
/// "Loading..." after the request has already failed.
fn apply_screen(auth_loading: bool, phase: FetchClass) -> ApplyScreen {
    if auth_loading || phase == FetchClass::Loading {
        ApplyScreen::Loading
    } else if phase == FetchClass::Error {
        ApplyScreen::Error
    } else {
        ApplyScreen::Ready
    }
}

/// Logged-in application slot. Outer `None` is still in flight. The middle
/// `None` is a failed fetch. The inner `None` is a successful "no application".
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum MineView {
    Pending,
    Failed,
    Form,
    Status,
}

fn mine_view<T>(
    state: UseResourceState,
    data: Option<Option<Option<&T>>>,
    error: Option<&str>,
) -> MineView {
    // A refetch is Pending while the previous value is still a failure.
    if state == UseResourceState::Pending {
        return MineView::Pending;
    }
    match data {
        None if error.is_none() => MineView::Pending,
        Some(Some(Some(_))) => MineView::Status,
        Some(Some(None)) if error.is_none() => MineView::Form,
        _ => MineView::Failed,
    }
}

/// A signed in member has nothing to apply for. Someone with a live
/// application (pending or trial) still sees its status and the Withdraw
/// button, so a trial member keeps that control.
fn already_member(is_member: bool, mine: MineView, status: Option<&str>) -> bool {
    if !is_member {
        return false;
    }
    match mine {
        MineView::Form => true,
        MineView::Status => !matches!(status, Some("pending" | "trial")),
        MineView::Pending | MineView::Failed => false,
    }
}

/// Pill tone and label for an application status. A turned down application
/// is a neutral "Not accepted", not a red alarm.
fn status_pill(status: &str) -> (PillTone, String) {
    match status {
        "pending" => (PillTone::Warn, "Pending Review".to_string()),
        "trial" => (PillTone::Accent, "Trial Period".to_string()),
        "accepted" => (PillTone::Ok, "Accepted".to_string()),
        "rejected" => (PillTone::Neutral, "Not accepted".to_string()),
        "withdrawn" => (PillTone::Neutral, "Withdrawn".to_string()),
        other => (PillTone::Neutral, other.to_string()),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SlotPhase {
    Loading,
    Failed,
    Ready,
}

/// A refetch is loading while the resource is `Pending`, including when the
/// last value was a failure (`Some(None)` with the error already cleared).
/// Otherwise an empty slot with no error is the first load, and a settled
/// error is the failure screen.
fn apply_fetch_phase(state: UseResourceState, slot_absent: bool, error: bool) -> SlotPhase {
    if state == UseResourceState::Pending || (slot_absent && !error) {
        SlotPhase::Loading
    } else if error {
        SlotPhase::Failed
    } else {
        SlotPhase::Ready
    }
}

/// Which card the Ready screen shows. One decision feeds both the body and the
/// "your application" status line, so the two cannot drift apart.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ApplyBodyView {
    Closed,
    SignIn,
    Member,
    Status,
    MinePending,
    MineFailed,
    Form,
}

fn apply_body_view(
    recruitment_open: bool,
    logged_in: bool,
    is_member: bool,
    has_status: bool,
    mine: MineView,
) -> ApplyBodyView {
    if !recruitment_open {
        ApplyBodyView::Closed
    } else if !logged_in {
        ApplyBodyView::SignIn
    } else if is_member {
        ApplyBodyView::Member
    } else if has_status {
        ApplyBodyView::Status
    } else {
        match mine {
            MineView::Pending => ApplyBodyView::MinePending,
            MineView::Failed => ApplyBodyView::MineFailed,
            MineView::Form | MineView::Status => ApplyBodyView::Form,
        }
    }
}

/// Retry control for a slot. `None` hides it, `Some(retrying)` keeps it mounted.
/// After a click it stays on screen, disabled, while the refetch runs, so focus
/// does not drop to the page body.
fn retry_control(failed: bool, pending: bool, retrying: bool) -> Option<bool> {
    if failed {
        Some(false)
    } else if pending && retrying {
        Some(true)
    } else {
        None
    }
}

/// Text of the always-mounted "your application" status line and its Retry.
/// `body` is `None` until the Ready screen has settings, so the line is empty
/// on Loading and on the settings error screen.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MineSlot {
    text: &'static str,
    retry: Option<bool>,
}

fn mine_slot(body: Option<ApplyBodyView>, retrying: bool) -> MineSlot {
    let pending = body == Some(ApplyBodyView::MinePending);
    let failed = body == Some(ApplyBodyView::MineFailed);
    let text = if pending {
        "Loading..."
    } else if failed {
        "Couldn't load your application."
    } else {
        ""
    };
    MineSlot {
        text,
        retry: retry_control(failed, pending, retrying),
    }
}

fn games_status_text(phase: SlotPhase) -> &'static str {
    match phase {
        SlotPhase::Loading => "Loading...",
        SlotPhase::Failed => "Couldn't load games.",
        SlotPhase::Ready => "",
    }
}

fn toggle_game(selected: &mut Vec<String>, id: &str) {
    if let Some(pos) = selected.iter().position(|x| x == id) {
        selected.remove(pos);
    } else {
        selected.push(id.to_string());
    }
}

/// Retry kept outside any status region. Mounted and disabled while in flight.
fn retry_button(
    class: &'static str,
    aria_label: &'static str,
    retrying: bool,
    on_retry: EventHandler<()>,
) -> Element {
    rsx! {
        button {
            r#type: "button",
            class,
            aria_label,
            aria_busy: if retrying { "true" },
            disabled: retrying,
            onclick: move |_| on_retry.call(()),
            "{retry_label(retrying)}"
        }
    }
}

/// Always-mounted status line for the logged-in application fetch. Only its
/// text changes; Retry sits after it, outside the region.
#[component]
fn MineStatus(slot: MineSlot, on_retry: EventHandler<()>) -> Element {
    rsx! {
        p { class: "apply-status", role: "status", "{slot.text}" }
        if let Some(retrying) = slot.retry {
            div { class: "fetch-error-wrap",
                {retry_button("fetch-error__retry", "Retry loading your application", retrying, on_retry)}
            }
        }
    }
}

/// Games picker: a named group of toggle buttons. Its status line is mounted
/// in every games phase (Loading, Failed, Ready); only the text changes.
/// `games` is `(id, name)` pairs. `children` follows the grid.
#[component]
fn GamesField(
    phase: SlotPhase,
    retrying: bool,
    games: Vec<(String, String)>,
    selected: Vec<String>,
    on_toggle: EventHandler<String>,
    on_retry: EventHandler<()>,
    children: Element,
) -> Element {
    let retry = retry_control(
        phase == SlotPhase::Failed,
        phase == SlotPhase::Loading,
        retrying,
    );
    rsx! {
        fieldset { class: "apply-field apply-fieldset",
            legend { class: "apply-label", "Games" }
            p { class: "apply-status", role: "status", "{games_status_text(phase)}" }
            if let Some(retrying) = retry {
                {retry_button("fetch-error__retry is-compact", "Retry loading games", retrying, on_retry)}
            }
            if phase == SlotPhase::Ready {
                div { class: "apply-game-grid",
                    for (id, name) in games {
                        {
                            let is_selected = selected.contains(&id);
                            rsx! {
                                button {
                                    key: "{id}",
                                    r#type: "button",
                                    class: if is_selected { "apply-game-btn selected" } else { "apply-game-btn" },
                                    aria_pressed: if is_selected { "true" } else { "false" },
                                    onclick: move |_| on_toggle.call(id.clone()),
                                    "{name}"
                                }
                            }
                        }
                    }
                }
            }
            {children}
        }
    }
}

// Local minimal type for checking existing application status.
#[derive(Debug, Clone, Deserialize)]
struct Application {
    #[allow(dead_code)]
    id: String,
    status: String,
}

// Local request type (no shared equivalent for application submission).
#[derive(Serialize)]
struct ApplyBody {
    preferred_games: Vec<String>,
    preferred_roles: Vec<String>,
    message: Option<String>,
}

const APPLY_CSS: &str = r#"
    .apply-page { min-height: 100vh; padding: 2rem; max-width: 600px; margin: 0 auto; }
    .apply-title { font-family: var(--font-head); font-size: 2.5rem; color: var(--text); letter-spacing: 3px; text-align: center; margin-bottom: 2rem; }
    .apply-card-title { font-family: var(--font-head); font-weight: 700; font-size: 1.3rem; color: var(--text); margin: 0 0 0.5rem; }
    .apply-card-desc { color: var(--text-2); font-size: 0.9rem; line-height: 1.6; }
    .apply-auth-buttons { margin-top: 1.5rem; display: flex; gap: 0.75rem; flex-wrap: wrap; }
    .apply-status-row { margin: 1rem 0; }
    .apply-field { margin-top: 1.5rem; }
    .apply-label { font-family: var(--font-head); font-weight: 600; font-size: 0.85rem; color: var(--text); text-transform: uppercase; letter-spacing: 0.04em; display: block; margin-bottom: 0.5rem; }
    .apply-game-grid { display: flex; gap: 0.5rem; flex-wrap: wrap; }
    .apply-game-btn { padding: 0.4rem 1rem; border-radius: 6px; border: 1px solid var(--border); background: var(--surface); color: var(--text-2); font-size: 0.85rem; cursor: pointer; transition: all 0.15s; }
    .apply-game-btn:hover { border-color: var(--accent-soft); color: var(--text); }
    .apply-game-btn.selected { background: var(--accent); color: var(--accent-fg); border-color: var(--accent); }
    .apply-actions { margin-top: 1.5rem; }
    .apply-fieldset { border: 0; padding: 0; margin-inline: 0; min-width: 0; }
    .apply-fieldset > legend { padding: 0; }
    .apply-status { color: var(--text-3); font-size: 0.88rem; margin: 0; }
    .apply-status:not(:empty) { margin: 0.5rem 0; }
    .apply-loading { color: var(--text-3); text-align: center; padding: 2rem; }
"#;

#[component]
pub fn Apply() -> Element {
    let auth = use_auth();
    let mut toast = use_toast();

    let mut settings = use_site_settings();
    let resolved = settings.resolved.read();
    let settings_phase = classify_fetch(resolved.as_ref());
    let s = loaded_site_settings(resolved.as_ref());
    let mut games = use_api_list::<Game>("/api/games");
    let mut my_app = use_api::<Option<Application>>("/api/applications/mine");

    let mut selected_games = use_signal(Vec::<String>::new);
    let mut message = use_signal(String::new);
    let mut submitting = use_signal(|| false);
    let mut confirm_withdraw = use_signal(|| false);
    // Set by a Retry click; cleared once that refetch settles.
    let mut mine_retrying = use_signal(|| false);
    let mut games_retrying = use_signal(|| false);
    use_effect(move || {
        if my_app.data.state()() != UseResourceState::Pending {
            mine_retrying.set(false);
        }
    });
    use_effect(move || {
        if games.data.state()() != UseResourceState::Pending {
            games_retrying.set(false);
        }
    });

    let loading = auth().loading;
    let screen = apply_screen(loading, settings_phase);
    let org_name = s.as_ref().map(|x| x.org_name.clone());
    let mine_data = my_app.data.read();
    let mine_error = my_app.error.read();
    let mine = mine_view(
        my_app.data.state()(),
        mine_data
            .as_ref()
            .map(|outer| outer.as_ref().map(|inner| inner.as_ref())),
        mine_error.as_deref(),
    );
    let status_app = mine_data
        .as_ref()
        .and_then(|outer| outer.as_ref())
        .and_then(|inner| inner.as_ref())
        .filter(|_| mine == MineView::Status)
        .cloned();
    let body_view = s
        .as_ref()
        .filter(|_| screen == ApplyScreen::Ready)
        .map(|s| {
            let is_member = already_member(
                auth().is_org_member(),
                mine,
                status_app.as_ref().map(|a| a.status.as_str()),
            );
            apply_body_view(
                s.recruitment_open,
                auth().is_logged_in(),
                is_member,
                status_app.is_some(),
                mine,
            )
        });
    let slot = mine_slot(body_view, mine_retrying());

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-apply-apply-css", css: APPLY_CSS }
        div { class: "apply-page",
            h1 { class: "apply-title",
                if let Some(name) = org_name {
                    "Join {name}"
                } else {
                    "Join"
                }
            }

            // Mounted on every screen (Loading, Error, Ready) so a failed
            // application fetch is announced as a text change. Empty when idle.
            MineStatus {
                slot,
                on_retry: move |_| {
                    mine_retrying.set(true);
                    my_app.refresh += 1;
                },
            }

            if screen == ApplyScreen::Loading {
                p { class: "apply-loading", "Loading..." }
            } else if let (Some(s), Some(view)) = (s, body_view) {
                {
                    let org_name = s.org_name.clone();
                    match (view, status_app.clone()) {
                        (ApplyBodyView::Closed, _) => rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Recruitment Closed" }
                                p { class: "apply-card-desc", "{s.recruitment_message}" }
                            }
                        },
                        (ApplyBodyView::SignIn, _) => rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Log In to Apply" }
                                p { class: "apply-card-desc", "You need to sign in before submitting an application." }
                                div { class: "apply-auth-buttons",
                                    Link { to: Route::Login {}, class: "ui-btn ui-btn--primary ui-btn--md", "Sign in" }
                                    // Only in debug builds: route is not registered in production
                                    if cfg!(debug_assertions) {
                                        a {
                                            href: "/api/dev/login",
                                            class: "ui-btn ui-btn--md",
                                            style: "background: var(--surface); border: 1px solid var(--border); color: var(--text);",
                                            "Dev login"
                                        }
                                    }
                                }
                            }
                        },
                        (ApplyBodyView::Member, _) => rsx! {
                            Card {
                                h2 { class: "apply-card-title", "You are already in {org_name}" }
                                p { class: "apply-card-desc", "Your membership is active, so there is nothing to apply for." }
                                div { class: "apply-auth-buttons",
                                    Link { to: Route::ProfileSettings {}, class: "ui-btn ui-btn--primary ui-btn--md", "Open your profile" }
                                    Link { to: Route::Members {}, class: "ui-btn ui-btn--md", "See the crew" }
                                }
                            }
                        },
                        (ApplyBodyView::Status, Some(app)) => {
                            let (status_tone, status_label) = status_pill(app.status.as_str());
                            let desc = match app.status.as_str() {
                                "pending" => {
                                    "Your application is being reviewed. We'll get back to you soon."
                                        .to_string()
                                }
                                "trial" => {
                                    "You're in your trial period. Show up, have fun, and be yourself."
                                        .to_string()
                                }
                                "accepted" => format!("Welcome aboard! You're a member of {org_name}."),
                                "rejected" => {
                                    "Unfortunately your application was not accepted at this time."
                                        .to_string()
                                }
                                "withdrawn" => {
                                    "You withdrew this application. You can re-apply later if recruitment is open."
                                        .to_string()
                                }
                                _ => String::new(),
                            };
                            let can_withdraw = app.status == "pending" || app.status == "trial";
                            rsx! {
                                Card {
                                    h2 { class: "apply-card-title", "Application Status" }
                                    div { class: "apply-status-row",
                                        Pill { tone: status_tone, "{status_label}" }
                                    }
                                    p { class: "apply-card-desc", "{desc}" }
                                    if can_withdraw {
                                        if confirm_withdraw() {
                                            InlineConfirm {
                                                id: "withdraw-application".to_string(),
                                                title: "Withdraw your application?".to_string(),
                                                body: "You can apply again later if recruitment is open.".to_string(),
                                                confirm_label: "Withdraw".to_string(),
                                                busy: submitting(),
                                                on_confirm: move |_| {
                                                    confirm_withdraw.set(false);
                                                    submitting.set(true);
                                                    spawn(async move {
                                                        match ApiClient::web()
                                                            .post_json_empty(
                                                                "/api/applications/mine/withdraw",
                                                                &serde_json::json!({}),
                                                            )
                                                            .await
                                                        {
                                                            Ok(_) => {
                                                                toast.show(Toast::success("Application withdrawn"));
                                                                my_app.refresh += 1;
                                                            }
                                                            Err(e) => {
                                                                toast.show(Toast::error(format!("Failed: {e}")));
                                                            }
                                                        }
                                                        submitting.set(false);
                                                    });
                                                },
                                                on_cancel: move |_| confirm_withdraw.set(false),
                                            }
                                        } else {
                                            div { class: "apply-actions",
                                                Button {
                                                    id: "withdraw-application-trigger".to_string(),
                                                    variant: BtnVariant::Ghost,
                                                    disabled: submitting(),
                                                    onclick: move |_| confirm_withdraw.set(true),
                                                    if submitting() {
                                                        "Withdrawing..."
                                                    } else {
                                                        "Withdraw application"
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                        // The status line and its Retry live in `MineStatus` above.
                        (ApplyBodyView::Status, None)
                        | (ApplyBodyView::MinePending | ApplyBodyView::MineFailed, _) => rsx! {},
                        (ApplyBodyView::Form, _) => {
                            let games_state = games.data.read();
                            let games_error = games.error.read();
                            let games_phase = apply_fetch_phase(
                                games.data.state()(),
                                games_state.as_ref().is_none(),
                                games_error.is_some(),
                            );
                            let game_list: Vec<(String, String)> = games_state
                                .as_ref()
                                .and_then(|g| g.as_ref())
                                .map(|g| g.iter().map(|g| (g.id.clone(), g.name.clone())).collect())
                                .unwrap_or_default();
                            rsx! {
                                Card {
                                    h2 { class: "apply-card-title", "Apply" }
                                    p { class: "apply-card-desc", "Tell us which games you play and a bit about yourself." }

                                    GamesField {
                                        phase: games_phase,
                                        retrying: games_retrying(),
                                        games: game_list,
                                        selected: selected_games(),
                                        on_toggle: move |id: String| toggle_game(&mut selected_games.write(), &id),
                                        on_retry: move |_| {
                                            games_retrying.set(true);
                                            games.refresh += 1;
                                        },
                                        {list_cap_notice(&games, "games")}
                                    }

                                    div { class: "apply-field",
                                        label { class: "apply-label", r#for: "apply-message", "Message (optional)" }
                                        Textarea {
                                            id: "apply-message".to_string(),
                                            name: "apply-message".to_string(),
                                            value: message(),
                                            placeholder: "Tell us about yourself, your experience, what you're looking for...",
                                            oninput: move |e: FormEvent| message.set(e.value()),
                                        }
                                    }

                                    div { class: "apply-actions",
                                        Button {
                                            variant: BtnVariant::Primary,
                                            disabled: submitting(),
                                            onclick: move |_| {
                                                let games = selected_games();
                                                let msg = message();
                                                if games.is_empty() {
                                                    toast.show(Toast::error("Select at least one game"));
                                                    return;
                                                }
                                                submitting.set(true);
                                                spawn(async move {
                                                    let body = ApplyBody {
                                                        preferred_games: games,
                                                        preferred_roles: vec![],
                                                        message: if msg.trim().is_empty() { None } else { Some(msg) },
                                                    };
                                                    match ApiClient::web().post_json_empty("/api/applications", &body).await {
                                                        Ok(_) => {
                                                            toast.show(Toast::success("Application submitted!"));
                                                            my_app.refresh += 1;
                                                        }
                                                        Err(e) => {
                                                            toast.show(Toast::error(format!("Failed: {e}")));
                                                        }
                                                    }
                                                    submitting.set(false);
                                                });
                                            },
                                            if submitting() {
                                                "Submitting..."
                                            } else {
                                                "Submit Application"
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
            } else {
                div { class: "fetch-error-wrap", role: "alert",
                    p { class: "fetch-error", "Couldn't load site settings." }
                    button {
                        r#type: "button",
                        class: "fetch-error__retry",
                        aria_label: "Retry loading site settings",
                        onclick: move |_| settings.refresh += 1,
                        "Retry"
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rejected_is_a_neutral_not_accepted_chip() {
        let (tone, label) = status_pill("rejected");
        assert_eq!(label, "Not accepted");
        assert!(tone == PillTone::Neutral);
        assert!(!label.contains("Rejected"));
        assert!(status_pill("pending").0 == PillTone::Warn);
        assert!(status_pill("accepted").0 == PillTone::Ok);
        assert_eq!(status_pill("mystery").1, "mystery");
    }

    #[test]
    fn members_are_told_they_are_already_in() {
        // Old rejected or withdrawn applications and the empty form both give way.
        assert!(already_member(true, MineView::Form, None));
        assert!(already_member(true, MineView::Status, Some("rejected")));
        assert!(already_member(true, MineView::Status, Some("withdrawn")));
        assert!(already_member(true, MineView::Status, Some("accepted")));
        // A live application keeps its status and Withdraw button.
        assert!(!already_member(true, MineView::Status, Some("pending")));
        assert!(!already_member(true, MineView::Status, Some("trial")));
        // No flash while the application is still loading, or when it failed.
        assert!(!already_member(true, MineView::Pending, None));
        assert!(!already_member(true, MineView::Failed, None));
        // Non members never see it.
        assert!(!already_member(false, MineView::Form, None));
        assert!(!already_member(false, MineView::Status, Some("rejected")));
    }

    #[test]
    fn settings_failure_is_not_an_endless_loading_state() {
        assert_eq!(apply_screen(false, FetchClass::Error), ApplyScreen::Error);
        assert_eq!(
            apply_screen(false, FetchClass::Loading),
            ApplyScreen::Loading
        );
        assert_eq!(apply_screen(true, FetchClass::Ready), ApplyScreen::Loading);
        assert_eq!(apply_screen(false, FetchClass::Ready), ApplyScreen::Ready);
        assert_ne!(apply_screen(false, FetchClass::Error), ApplyScreen::Loading);
    }

    #[test]
    fn logged_in_apply_waits_for_the_existing_application() {
        assert_eq!(
            mine_view(UseResourceState::Ready, None::<Option<Option<&()>>>, None),
            MineView::Pending
        );
        assert_eq!(
            mine_view(
                UseResourceState::Ready,
                Some(None::<Option<&()>>),
                Some("offline")
            ),
            MineView::Failed
        );
        // Deleting the Pending override leaves this as Failed.
        assert_eq!(
            mine_view(
                UseResourceState::Pending,
                Some(None::<Option<&()>>),
                Some("offline")
            ),
            MineView::Pending
        );
        assert_eq!(
            mine_view(UseResourceState::Ready, Some(Some(None::<&()>)), None),
            MineView::Form
        );
        assert_eq!(
            mine_view(UseResourceState::Ready, Some(Some(Some(&()))), None),
            MineView::Status
        );
        assert_ne!(
            mine_view(UseResourceState::Ready, None::<Option<Option<&()>>>, None),
            MineView::Form
        );
    }

    #[test]
    fn retry_in_flight_stays_loading_when_the_last_value_failed() {
        // `use_api_list` clears the error when a refetch starts and leaves
        // `Some(None)` in the slot. That used to paint an empty games grid.
        assert_eq!(
            apply_fetch_phase(UseResourceState::Pending, false, false),
            SlotPhase::Loading
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Pending, false, true),
            SlotPhase::Loading
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Ready, false, true),
            SlotPhase::Failed
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Ready, true, false),
            SlotPhase::Loading
        );
        assert_eq!(
            apply_fetch_phase(UseResourceState::Ready, false, false),
            SlotPhase::Ready
        );
    }

    #[test]
    fn body_view_follows_the_page_decision_order() {
        use ApplyBodyView as V;
        // Closed and signed out win over everything else.
        assert_eq!(
            apply_body_view(false, true, true, true, MineView::Failed),
            V::Closed
        );
        assert_eq!(
            apply_body_view(true, false, false, false, MineView::Failed),
            V::SignIn
        );
        assert_eq!(
            apply_body_view(true, true, true, true, MineView::Status),
            V::Member
        );
        assert_eq!(
            apply_body_view(true, true, false, true, MineView::Status),
            V::Status
        );
        assert_eq!(
            apply_body_view(true, true, false, false, MineView::Pending),
            V::MinePending
        );
        assert_eq!(
            apply_body_view(true, true, false, false, MineView::Failed),
            V::MineFailed
        );
        assert_eq!(
            apply_body_view(true, true, false, false, MineView::Form),
            V::Form
        );
    }

    #[test]
    fn mine_slot_text_only_shows_for_the_application_fetch() {
        use ApplyBodyView as V;
        // Loading and error screens: mounted but empty, no Retry.
        assert_eq!(
            mine_slot(None, false),
            MineSlot {
                text: "",
                retry: None
            }
        );
        assert_eq!(
            mine_slot(None, true),
            MineSlot {
                text: "",
                retry: None
            }
        );
        for view in [V::Closed, V::SignIn, V::Member, V::Status, V::Form] {
            assert_eq!(
                mine_slot(Some(view), false),
                MineSlot {
                    text: "",
                    retry: None
                }
            );
        }
        assert_eq!(
            mine_slot(Some(V::MinePending), false),
            MineSlot {
                text: "Loading...",
                retry: None
            }
        );
        assert_eq!(
            mine_slot(Some(V::MineFailed), false),
            MineSlot {
                text: "Couldn't load your application.",
                retry: Some(false)
            }
        );
        // A Retry click keeps the button mounted and disabled while in flight.
        assert_eq!(
            mine_slot(Some(V::MinePending), true),
            MineSlot {
                text: "Loading...",
                retry: Some(true)
            }
        );
    }

    #[test]
    fn retry_stays_mounted_only_for_a_failure_or_its_refetch() {
        assert_eq!(retry_control(true, false, false), Some(false));
        assert_eq!(retry_control(true, false, true), Some(false));
        assert_eq!(retry_control(false, true, true), Some(true));
        assert_eq!(retry_control(false, true, false), None);
        assert_eq!(retry_control(false, false, true), None);
    }

    #[test]
    fn game_toggle_adds_then_removes() {
        let mut selected = vec!["ow".to_string()];
        toggle_game(&mut selected, "val");
        assert_eq!(selected, ["ow", "val"]);
        toggle_game(&mut selected, "ow");
        assert_eq!(selected, ["val"]);
        assert_eq!(games_status_text(SlotPhase::Ready), "");
        assert_eq!(games_status_text(SlotPhase::Loading), "Loading...");
        assert_eq!(games_status_text(SlotPhase::Failed), "Couldn't load games.");
    }

    fn html_of(view: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(view);
        dom.rebuild_in_place();
        dom.render_immediate(&mut dioxus::dioxus_core::NoOpMutations);
        dioxus_ssr::render(&dom)
    }

    /// Exactly one status region holding `text`, with any Retry after it.
    fn assert_one_status(html: &str, text: &str) {
        assert_eq!(html.matches(r#"role="status""#).count(), 1, "{html}");
        let text = text.replace('\'', "&#39;");
        let region = format!(r#"role="status">{text}</p>"#);
        let at = html
            .find(&region)
            .unwrap_or_else(|| panic!("{region} in {html}"));
        if let Some(button) = html.find("<button") {
            assert!(button >= at + region.len(), "Retry inside status: {html}");
        }
    }

    #[test]
    fn mine_status_is_one_text_region_with_retry_outside() {
        fn loading() -> Element {
            rsx! { MineStatus { slot: mine_slot(None, false), on_retry: |_| {} } }
        }
        fn pending() -> Element {
            let slot = mine_slot(Some(ApplyBodyView::MinePending), false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn failed() -> Element {
            let slot = mine_slot(Some(ApplyBodyView::MineFailed), false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn retrying() -> Element {
            let slot = mine_slot(Some(ApplyBodyView::MinePending), true);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }
        fn ready() -> Element {
            let slot = mine_slot(Some(ApplyBodyView::Form), false);
            rsx! { MineStatus { slot, on_retry: |_| {} } }
        }

        for idle in [html_of(loading), html_of(ready)] {
            assert_one_status(&idle, "");
            assert!(!idle.contains("<button"), "{idle}");
        }

        let html = html_of(pending);
        assert_one_status(&html, "Loading...");
        assert!(!html.contains("<button"), "{html}");

        let html = html_of(failed);
        assert_one_status(&html, "Couldn't load your application.");
        assert!(html.contains(">Retry</button>"), "{html}");
        assert!(!html.contains("disabled"), "{html}");

        let html = html_of(retrying);
        assert_one_status(&html, "Loading...");
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert!(html.contains("disabled"), "{html}");
        assert!(html.contains(r#"aria-busy="true""#), "{html}");
    }

    fn games() -> Vec<(String, String)> {
        vec![
            ("game-ow".to_string(), "Overwatch".to_string()),
            ("game-val".to_string(), "Valorant".to_string()),
        ]
    }

    #[test]
    fn games_field_is_a_named_group_with_one_status_region() {
        fn loading() -> Element {
            rsx! {
                GamesField {
                    phase: SlotPhase::Loading,
                    retrying: false,
                    games: vec![],
                    selected: vec![],
                    on_toggle: |_| {},
                    on_retry: |_| {},
                }
            }
        }
        fn failed() -> Element {
            rsx! {
                GamesField {
                    phase: SlotPhase::Failed,
                    retrying: false,
                    games: vec![],
                    selected: vec![],
                    on_toggle: |_| {},
                    on_retry: |_| {},
                }
            }
        }
        fn retrying() -> Element {
            rsx! {
                GamesField {
                    phase: SlotPhase::Loading,
                    retrying: true,
                    games: vec![],
                    selected: vec![],
                    on_toggle: |_| {},
                    on_retry: |_| {},
                }
            }
        }
        fn ready() -> Element {
            rsx! {
                GamesField {
                    phase: SlotPhase::Ready,
                    retrying: false,
                    games: games(),
                    selected: vec!["game-val".to_string()],
                    on_toggle: |_| {},
                    on_retry: |_| {},
                }
            }
        }

        let html = html_of(loading);
        assert_one_status(&html, "Loading...");
        assert!(!html.contains("<button"), "{html}");

        let html = html_of(failed);
        assert_one_status(&html, "Couldn't load games.");
        assert!(html.contains(">Retry</button>"), "{html}");
        assert!(
            html.contains(r#"aria-label="Retry loading games""#),
            "{html}"
        );
        assert!(!html.contains("disabled"), "{html}");

        let html = html_of(retrying);
        assert_one_status(&html, "Loading...");
        assert!(html.contains(">Retrying...</button>"), "{html}");
        assert!(html.contains("disabled"), "{html}");

        let html = html_of(ready);
        assert_one_status(&html, "");
        assert!(html.contains("<fieldset"), "{html}");
        assert!(html.contains(">Games</legend>"), "{html}");
        assert!(!html.contains("<label"), "{html}");
        assert!(!html.contains(">Retry"), "{html}");
        assert_eq!(html.matches(r#"aria-pressed="false""#).count(), 1, "{html}");
        assert_eq!(html.matches(r#"aria-pressed="true""#).count(), 1, "{html}");
        let button_for = |name: &str| {
            let end = html.find(&format!(">{name}</button>")).unwrap();
            let start = html[..end].rfind("<button").unwrap();
            html[start..end].to_string()
        };
        assert!(
            button_for("Valorant").contains(r#"aria-pressed="true""#),
            "{html}"
        );
        assert!(
            button_for("Overwatch").contains(r#"aria-pressed="false""#),
            "{html}"
        );
        assert!(
            button_for("Overwatch").contains(r#"type="button""#),
            "{html}"
        );
    }

    #[test]
    fn rejected_application_is_not_accepted_and_neutral() {
        let (tone, label) = status_pill("rejected");
        assert_eq!(label, "Not accepted");
        assert!(tone == PillTone::Neutral);
        assert!(status_pill("pending").0 == PillTone::Warn);
        assert_eq!(status_pill("pending").1, "Pending Review");
    }
}
