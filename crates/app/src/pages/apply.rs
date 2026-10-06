use dioxus::prelude::*;
use serde::{Deserialize, Serialize};

use crate::components::ui::{BtnVariant, Button, Card, Pill, PillTone, Textarea};
use crate::components::{Toast, list_cap_notice, use_toast};
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

fn mine_view<T>(data: Option<Option<Option<&T>>>, error: Option<&str>) -> MineView {
    match data {
        None if error.is_none() => MineView::Pending,
        Some(Some(Some(_))) => MineView::Status,
        Some(Some(None)) if error.is_none() => MineView::Form,
        _ => MineView::Failed,
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

    let loading = auth().loading;
    let screen = apply_screen(loading, settings_phase);
    let org_name = s.as_ref().map(|x| x.org_name.clone());
    let mine_data = my_app.data.read();
    let mine_error = my_app.error.read();
    let mine_settled = mine_view(
        mine_data
            .as_ref()
            .map(|outer| outer.as_ref().map(|inner| inner.as_ref())),
        mine_error.as_deref(),
    );
    let mine = if my_app.data.state()() == UseResourceState::Pending {
        MineView::Pending
    } else {
        mine_settled
    };
    let status_app = mine_data
        .as_ref()
        .and_then(|outer| outer.as_ref())
        .and_then(|inner| inner.as_ref())
        .filter(|_| mine == MineView::Status)
        .cloned();

    rsx! {
        style { {APPLY_CSS} }
        div { class: "apply-page",
            h1 { class: "apply-title",
                if let Some(name) = org_name {
                    "Join {name}"
                } else {
                    "Join"
                }
            }

            if screen == ApplyScreen::Loading {
                p { class: "apply-loading", "Loading..." }
            } else if let Some(s) = s {
                {
                    let org_name = s.org_name.clone();

                    if !s.recruitment_open {
                        rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Recruitment Closed" }
                                p { class: "apply-card-desc", "{s.recruitment_message}" }
                            }
                        }
                    } else if !auth().is_logged_in() {
                        rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Log In to Apply" }
                                p { class: "apply-card-desc", "You need to sign in before submitting an application." }
                                div { class: "apply-auth-buttons",
                                    Link { to: Route::Login {}, class: "ui-btn ui-btn--primary ui-btn--md", "Sign in" }
                                    // Only in debug builds — route is not registered in production
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
                        }
                    } else if let Some(app) = status_app.clone() {
                        let status_tone = match app.status.as_str() {
                            "pending" => PillTone::Warn,
                            "trial" => PillTone::Accent,
                            "accepted" => PillTone::Ok,
                            "rejected" => PillTone::Danger,
                            _ => PillTone::Neutral,
                        };
                        let status_label = match app.status.as_str() {
                            "pending" => "Pending Review",
                            "trial" => "Trial Period",
                            "accepted" => "Accepted",
                            "rejected" => "Rejected",
                            "withdrawn" => "Withdrawn",
                            _ => &app.status,
                        };
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
                                    div { class: "apply-actions",
                                        Button {
                                            variant: BtnVariant::Ghost,
                                            disabled: submitting(),
                                            onclick: move |_| {
                                                #[cfg(target_arch = "wasm32")]
                                                let confirmed = web_sys::window()
                                                    .and_then(|w| w.confirm_with_message("Withdraw your application?").ok())
                                                    .unwrap_or(false);
                                                #[cfg(not(target_arch = "wasm32"))]
                                                let confirmed = true;
                                                if !confirmed {
                                                    return;
                                                }
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
                    } else if mine == MineView::Pending || mine == MineView::Failed {
                        rsx! {
                            div { role: "status",
                                if mine == MineView::Pending {
                                    p { class: "apply-loading", "Loading..." }
                                } else {
                                    p { class: "fetch-error", "Couldn't load your application." }
                                    button {
                                        r#type: "button",
                                        class: "fetch-error__retry",
                                        aria_label: "Retry loading your application",
                                        onclick: move |_| my_app.refresh += 1,
                                        "Retry"
                                    }
                                }
                            }
                        }
                    } else {
                        let games_state = games.data.read();
                        let games_error = games.error.read();
                        let games_phase = apply_fetch_phase(
                            games.data.state()(),
                            games_state.as_ref().is_none(),
                            games_error.is_some(),
                        );
                        let game_list = games_state
                            .as_ref()
                            .and_then(|g| g.as_ref())
                            .cloned()
                            .unwrap_or_default();
                        rsx! {
                            Card {
                                h2 { class: "apply-card-title", "Apply" }
                                p { class: "apply-card-desc", "Tell us which games you play and a bit about yourself." }

                                div { class: "apply-field",
                                    label { class: "apply-label", "Games" }
                                    if games_phase == SlotPhase::Loading || games_phase == SlotPhase::Failed {
                                        div { role: "status",
                                            if games_phase == SlotPhase::Loading {
                                                p { class: "apply-loading", "Loading..." }
                                            } else {
                                                p { class: "muted", "Couldn't load games." }
                                                button {
                                                    r#type: "button",
                                                    class: "fetch-error__retry is-compact",
                                                    aria_label: "Retry loading games",
                                                    onclick: move |_| games.refresh += 1,
                                                    "Retry"
                                                }
                                            }
                                        }
                                    } else {
                                        div { class: "apply-game-grid",
                                            for g in game_list.iter() {
                                                {
                                                    let gid = g.id.clone();
                                                    let gid2 = g.id.clone();
                                                    let is_selected = selected_games().contains(&gid);
                                                    let btn_class = if is_selected {
                                                        "apply-game-btn selected"
                                                    } else {
                                                        "apply-game-btn"
                                                    };
                                                    rsx! {
                                                        button {
                                                            class: "{btn_class}",
                                                            onclick: move |_| {
                                                                let gid = gid2.clone();
                                                                selected_games.write().retain(|x| x != &gid);
                                                                if !is_selected {
                                                                    selected_games.write().push(gid);
                                                                }
                                                            },
                                                            "{g.name}"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                    {list_cap_notice(&games, "games")}
                                }

                                div { class: "apply-field",
                                    label { class: "apply-label", "Message (optional)" }
                                    Textarea {
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
            mine_view(None::<Option<Option<&()>>>, None),
            MineView::Pending
        );
        assert_eq!(
            mine_view(Some(None::<Option<&()>>), Some("offline")),
            MineView::Failed
        );
        assert_eq!(mine_view(Some(Some(None::<&()>)), None), MineView::Form);
        assert_eq!(mine_view(Some(Some(Some(&()))), None), MineView::Status);
        assert_ne!(mine_view(None::<Option<Option<&()>>>, None), MineView::Form);
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
}
