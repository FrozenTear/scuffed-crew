//! `/link` lets a signed-in member approve a stat-tracker device code.
//!
//! The code is typed. A `code` or `user_code` query param is ignored, and the
//! login return path is only `/link`.

mod api;

use std::cell::Cell;

use dioxus::prelude::*;
use scuffed_api_client::ClientError;

use crate::routes::Route;
use crate::state::use_auth;
use api::PendingDeviceCode;

/// Shown for a wrong, expired, or already-used code. Those cases stay identical.
pub const CODE_DIDNT_WORK: &str = "That code didn't work. Check the app and try again.";

/// Plain-language 429. No status digit.
pub const RATE_LIMITED: &str = "Too many tries. Wait a moment and try again.";

pub const COULD_NOT_REACH: &str = "Couldn't reach the site. Try again.";

pub const APPROVED_COPY: &str = "Done, go back to the app";

pub const DENIED_COPY: &str = "Request denied";

/// Where login sends the member back. Never includes a device code.
pub const LOGIN_RETURN_PATH: &str = "/link";

const RETURN_STORAGE_KEY: &str = "scuffed.return-to-link";
const RETURN_STORAGE_VALUE: &str = "1";

thread_local! {
    static RETURN_TO_LINK: Cell<bool> = const { Cell::new(false) };
}

/// Uppercase and drop whitespace. Hyphens stay.
pub fn normalize_user_code(raw: &str) -> String {
    raw.chars()
        .filter(|c| !c.is_whitespace())
        .flat_map(char::to_uppercase)
        .collect()
}

/// First query value for `key`, if any. Accepts `?a=b`, `a=b`, or a full path.
fn query_param<'a>(search: &'a str, key: &str) -> Option<&'a str> {
    let after_hash = search.split('#').next().unwrap_or(search);
    let query = after_hash
        .split_once('?')
        .map(|(_, q)| q)
        .unwrap_or(after_hash);
    let query = query.strip_prefix('?').unwrap_or(query);
    if query.is_empty() {
        return None;
    }
    query.split('&').find_map(|pair| {
        let (k, v) = pair.split_once('=').unwrap_or((pair, ""));
        (k == key && !v.is_empty()).then_some(v)
    })
}

/// The typed field starts empty. `code` and `user_code` in the query are discarded.
pub fn code_seed_from_search(search: &str) -> String {
    let _ignored = query_param(search, "code").or_else(|| query_param(search, "user_code"));
    String::new()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkGate {
    Wait,
    SendToLogin,
    Show,
}

pub fn link_gate(loading: bool, logged_in: bool) -> LinkGate {
    if loading {
        LinkGate::Wait
    } else if logged_in {
        LinkGate::Show
    } else {
        LinkGate::SendToLogin
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LinkStep {
    Enter,
    Confirm { code: String },
    Approved,
    Denied,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkIntent {
    Lookup,
    Approve,
    Deny,
}

/// Opening the page never looks up or approves, even when the URL has a code.
pub fn intent_on_open(search: &str) -> Option<LinkIntent> {
    let _ = code_seed_from_search(search);
    None
}

/// Approve and deny run only from the confirm step, and only for that button.
pub fn intent_for_click(step: &LinkStep, intent: LinkIntent) -> Option<LinkIntent> {
    match (step, intent) {
        (LinkStep::Enter, LinkIntent::Lookup) => Some(LinkIntent::Lookup),
        (LinkStep::Confirm { .. }, LinkIntent::Approve) => Some(LinkIntent::Approve),
        (LinkStep::Confirm { .. }, LinkIntent::Deny) => Some(LinkIntent::Deny),
        _ => None,
    }
}

/// A successful lookup shows the device. It does not approve it.
pub fn step_after_lookup(step: &LinkStep, code: &str) -> LinkStep {
    match step {
        LinkStep::Enter => LinkStep::Confirm {
            code: code.to_string(),
        },
        other => other.clone(),
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum UserNotice {
    Text(&'static str),
    SignIn,
}

/// Wrong, expired, and used stay on one sentence. `problem` is not shown.
pub fn message_for_code_problem(problem: &str) -> &'static str {
    let _ignored = problem;
    CODE_DIDNT_WORK
}

fn json_error_owned(body: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_string))
}

pub fn user_notice(status: Option<u16>, body: &str) -> UserNotice {
    match status {
        Some(401) => UserNotice::SignIn,
        Some(429) => UserNotice::Text(RATE_LIMITED),
        Some(_) => {
            let owned = json_error_owned(body);
            let problem = owned.as_deref().unwrap_or(body);
            UserNotice::Text(message_for_code_problem(problem))
        }
        None => UserNotice::Text(COULD_NOT_REACH),
    }
}

fn notice_from_client(err: &ClientError) -> UserNotice {
    match err {
        ClientError::Http { status, body } => user_notice(Some(*status), body),
        _ => user_notice(None, ""),
    }
}

pub fn format_request_time(created_at: &chrono::DateTime<chrono::Utc>) -> String {
    created_at.format("%b %d, %Y %H:%M UTC").to_string()
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LinkReturn {
    /// Still signed out, or auth is loading. Leave the flag alone.
    Hold,
    /// Already on `/link`. Clear the flag.
    Clear,
    /// Signed in on another page. Follow the flag to `/link`.
    Follow,
}

pub fn link_return_action(loading: bool, logged_in: bool, pathname: &str) -> LinkReturn {
    if loading || !logged_in {
        return LinkReturn::Hold;
    }
    if pathname == LOGIN_RETURN_PATH || pathname == "/link/" {
        LinkReturn::Clear
    } else {
        LinkReturn::Follow
    }
}

pub(crate) fn arm_return_to_link() {
    RETURN_TO_LINK.with(|flag| flag.set(true));
    store_return_flag();
}

pub(crate) fn take_return_to_link() -> bool {
    let memory = RETURN_TO_LINK.with(|flag| flag.replace(false));
    memory || take_stored_return_flag()
}

fn store_return_flag() {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        if let Some(storage) = session_storage() {
            let _ = storage.set_item(RETURN_STORAGE_KEY, RETURN_STORAGE_VALUE);
        }
    }
}

fn take_stored_return_flag() -> bool {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        let Some(storage) = session_storage() else {
            return false;
        };
        let value = storage.get_item(RETURN_STORAGE_KEY).ok().flatten();
        let _ = storage.remove_item(RETURN_STORAGE_KEY);
        value.as_deref() == Some(RETURN_STORAGE_VALUE)
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        false
    }
}

#[cfg(all(feature = "web", target_arch = "wasm32"))]
fn session_storage() -> Option<web_sys::Storage> {
    web_sys::window()?.session_storage().ok().flatten()
}

fn browser_pathname() -> Option<String> {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        Some(
            web_sys::window()
                .and_then(|window| window.location().pathname().ok())
                .unwrap_or_default(),
        )
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        None
    }
}

fn current_search() -> String {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        web_sys::window()
            .and_then(|window| window.location().search().ok())
            .unwrap_or_default()
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        String::new()
    }
}

fn navigate_to_link() {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        let _ =
            web_sys::window().and_then(|window| window.location().set_href(LOGIN_RETURN_PATH).ok());
    }
}

/// After a full-page sign-in (OAuth lands on `/`), follow the return flag.
/// Host tests have no browser path, so this does nothing there.
pub(crate) fn redirect_after_login_if_needed(loading: bool, logged_in: bool) {
    let Some(path) = browser_pathname() else {
        return;
    };
    match link_return_action(loading, logged_in, &path) {
        LinkReturn::Hold => {}
        LinkReturn::Clear => {
            let _ = take_return_to_link();
        }
        LinkReturn::Follow => {
            if take_return_to_link() {
                navigate_to_link();
            }
        }
    }
}

const CSS: &str = r#"
.link-page {
    min-height: 70vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem 1rem;
}
.link-card {
    width: 100%;
    max-width: 440px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 2rem;
}
.link-card h1 {
    font-family: var(--font-head);
    font-size: 1.5rem;
    color: var(--text);
    margin: 0 0 0.5rem;
}
.link-card p.lead,
.link-card p.note {
    color: var(--text-2);
    font-size: 0.9rem;
    margin: 0 0 1.25rem;
    line-height: 1.45;
}
.link-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 1rem;
}
.link-field label {
    font-size: 0.75rem;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.link-field input {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 0.6rem 0.75rem;
    border-radius: 6px;
    font-size: 1.1rem;
    font-family: var(--font-mono);
    letter-spacing: 0.08em;
}
.link-error {
    color: var(--danger);
    font-size: 0.85rem;
    margin: 0 0 1rem;
}
.link-facts {
    margin: 0 0 1.25rem;
}
.link-facts div {
    display: flex;
    justify-content: space-between;
    gap: 1rem;
    padding: 0.45rem 0;
    border-bottom: 1px solid var(--border);
    font-size: 0.9rem;
}
.link-facts dt {
    color: var(--text-3);
    margin: 0;
}
.link-facts dd {
    color: var(--text);
    margin: 0;
    text-align: right;
    word-break: break-word;
}
.link-actions {
    display: flex;
    gap: 0.6rem;
    flex-wrap: wrap;
}
.link-actions .ui-btn {
    flex: 1;
}
.link-card a {
    color: var(--accent);
    font-weight: 600;
}
.link-wait {
    color: var(--text-2);
    text-align: center;
}
"#;

#[component]
pub fn LinkDevice() -> Element {
    let auth = use_auth();
    let nav = use_navigator();
    let mut code = use_signal(|| {
        let search = current_search();
        let _ = intent_on_open(&search);
        code_seed_from_search(&search)
    });
    let mut step = use_signal(|| LinkStep::Enter);
    let mut pending = use_signal(|| None::<PendingDeviceCode>);
    let mut notice: Signal<Option<&'static str>> = use_signal(|| None);
    let mut busy = use_signal(|| false);

    use_effect(move || {
        if link_gate(auth().loading, auth().is_logged_in()) == LinkGate::SendToLogin {
            arm_return_to_link();
            nav.replace(Route::Login {});
        }
    });

    let gate = link_gate(auth().loading, auth().is_logged_in());
    if gate != LinkGate::Show {
        return rsx! {
            style { {CSS} }
            div { class: "link-page",
                p { class: "link-wait",
                    if gate == LinkGate::Wait {
                        "Checking sign-in."
                    } else {
                        "Sending you to sign in."
                    }
                }
            }
        };
    }

    let on_lookup = move |evt: Event<FormData>| {
        evt.prevent_default();
        if intent_for_click(&step(), LinkIntent::Lookup).is_none() {
            return;
        }
        let normalized = normalize_user_code(&code());
        if normalized.is_empty() {
            notice.set(Some("Enter the code from the app."));
            return;
        }
        notice.set(None);
        busy.set(true);
        spawn(async move {
            match api::lookup_pending_code(&normalized).await {
                Ok(device) => {
                    pending.set(Some(device));
                    step.set(step_after_lookup(&LinkStep::Enter, &normalized));
                }
                Err(err) => apply_client_error(&mut notice, &nav, err),
            }
            busy.set(false);
        });
    };

    let on_approve = move |_| {
        let current = step();
        if intent_for_click(&current, LinkIntent::Approve).is_none() {
            return;
        }
        let LinkStep::Confirm { code } = current else {
            return;
        };
        notice.set(None);
        busy.set(true);
        spawn(async move {
            match api::approve_pending_code(&code).await {
                Ok(()) => step.set(LinkStep::Approved),
                Err(err) => apply_client_error(&mut notice, &nav, err),
            }
            busy.set(false);
        });
    };

    let on_deny = move |_| {
        let current = step();
        if intent_for_click(&current, LinkIntent::Deny).is_none() {
            return;
        }
        let LinkStep::Confirm { code } = current else {
            return;
        };
        notice.set(None);
        busy.set(true);
        spawn(async move {
            match api::deny_pending_code(&code).await {
                Ok(()) => step.set(LinkStep::Denied),
                Err(err) => apply_client_error(&mut notice, &nav, err),
            }
            busy.set(false);
        });
    };

    rsx! {
        style { {CSS} }
        div { class: "link-page",
            div { class: "link-card",
                h1 { "Link the stat tracker" }
                match step() {
                    LinkStep::Enter => rsx! {
                        p { class: "lead", "Type the short code shown in the app." }
                        if let Some(text) = notice() {
                            p { class: "link-error", "{text}" }
                        }
                        form { onsubmit: on_lookup,
                            div { class: "link-field",
                                label { r#for: "link-code", "Device code" }
                                input {
                                    id: "link-code",
                                    name: "link-code",
                                    r#type: "text",
                                    autocomplete: "off",
                                    autocapitalize: "characters",
                                    spellcheck: false,
                                    maxlength: 32,
                                    value: "{code}",
                                    disabled: busy(),
                                    oninput: move |e| code.set(e.value()),
                                }
                            }
                            button {
                                class: "ui-btn ui-btn--primary ui-btn--md",
                                r#type: "submit",
                                disabled: busy(),
                                if busy() { "Checking the code." } else { "Continue" }
                            }
                        }
                    },
                    LinkStep::Confirm { code: shown } => rsx! {
                        p { class: "lead", "Check this request, then approve or deny it." }
                        if let Some(device) = pending() {
                            dl { class: "link-facts",
                                div {
                                    dt { "Device" }
                                    dd { "{device.device_label}" }
                                }
                                div {
                                    dt { "App version" }
                                    dd { "{device.app_version}" }
                                }
                                div {
                                    dt { "Requested" }
                                    dd { "{format_request_time(&device.created_at)}" }
                                }
                                div {
                                    dt { "Code" }
                                    dd { "{shown}" }
                                }
                            }
                        }
                        if let Some(text) = notice() {
                            p { class: "link-error", "{text}" }
                        }
                        div { class: "link-actions",
                            button {
                                class: "ui-btn ui-btn--primary ui-btn--md",
                                r#type: "button",
                                disabled: busy(),
                                onclick: on_approve,
                                "Approve"
                            }
                            button {
                                class: "ui-btn ui-btn--ghost ui-btn--md",
                                r#type: "button",
                                disabled: busy(),
                                onclick: on_deny,
                                "Deny"
                            }
                        }
                    },
                    LinkStep::Approved => rsx! {
                        p { class: "lead", "{APPROVED_COPY}" }
                        p { class: "note",
                            "The new token can be revoked on the "
                            Link { to: Route::StatsTokens {}, "tracker tokens list" }
                            "."
                        }
                    },
                    LinkStep::Denied => rsx! {
                        p { class: "lead", "{DENIED_COPY}" }
                    },
                }
            }
        }
    }
}

fn apply_client_error(
    notice: &mut Signal<Option<&'static str>>,
    nav: &dioxus_router::Navigator,
    err: ClientError,
) {
    match notice_from_client(&err) {
        UserNotice::SignIn => {
            arm_return_to_link();
            nav.replace(Route::Login {});
        }
        UserNotice::Text(text) => notice.set(Some(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::api::UserCodeRequest;
    use super::*;
    use std::str::FromStr;

    #[test]
    fn query_code_is_ignored() {
        assert_eq!(query_param("?code=ABCD", "code"), Some("ABCD"));
        assert_eq!(
            query_param("/link?user_code=secret", "user_code"),
            Some("secret")
        );
        assert_eq!(code_seed_from_search("?code=ABCD"), "");
        assert_eq!(code_seed_from_search("/link?code=ABCD"), "");
        assert_eq!(code_seed_from_search("?user_code=secret"), "");
        assert_eq!(code_seed_from_search("?code=ABCD&user_code=ZZZZ"), "");
        assert_eq!(code_seed_from_search("/link?code=APPROVE-ME#x"), "");
        assert_eq!(intent_on_open("/link?code=APPROVE-ME"), None);

        let parsed = Route::from_str("/link?code=ABCD").expect("link route");
        assert_eq!(parsed, Route::LinkDevice {});
        assert_eq!(parsed.to_string(), "/link");
        assert!(!parsed.to_string().contains('?'));
        assert!(!parsed.to_string().contains("ABCD"));

        assert_eq!(LOGIN_RETURN_PATH, "/link");
        assert!(!LOGIN_RETURN_PATH.contains('?'));
        assert_eq!(RETURN_STORAGE_VALUE, "1");
        assert_eq!(link_gate(false, false), LinkGate::SendToLogin);
        assert_eq!(link_return_action(false, true, "/"), LinkReturn::Follow);
        assert_eq!(link_return_action(false, true, "/link"), LinkReturn::Clear);
        assert_eq!(link_return_action(false, false, "/link"), LinkReturn::Hold);
    }

    #[test]
    fn approve_requires_the_button() {
        assert_eq!(intent_on_open("?code=ABCD"), None);
        assert_eq!(
            intent_for_click(&LinkStep::Enter, LinkIntent::Approve),
            None
        );
        assert_eq!(intent_for_click(&LinkStep::Enter, LinkIntent::Deny), None);
        assert_eq!(
            intent_for_click(&LinkStep::Enter, LinkIntent::Lookup),
            Some(LinkIntent::Lookup)
        );

        let confirm = step_after_lookup(&LinkStep::Enter, "ABCD");
        assert_eq!(
            confirm,
            LinkStep::Confirm {
                code: "ABCD".into()
            }
        );
        assert!(!matches!(confirm, LinkStep::Approved));
        assert_eq!(
            intent_for_click(&confirm, LinkIntent::Approve),
            Some(LinkIntent::Approve)
        );
        assert_eq!(
            intent_for_click(&confirm, LinkIntent::Deny),
            Some(LinkIntent::Deny)
        );
        assert_eq!(intent_for_click(&confirm, LinkIntent::Lookup), None);
        assert_eq!(
            intent_for_click(&LinkStep::Approved, LinkIntent::Approve),
            None
        );
    }

    #[test]
    fn identical_error_text_for_wrong_expired_and_used() {
        let wrong = message_for_code_problem("wrong");
        let expired = message_for_code_problem("expired");
        let used = message_for_code_problem("used");
        assert_eq!(wrong, CODE_DIDNT_WORK);
        assert_eq!(wrong, "That code didn't work. Check the app and try again.");
        assert_eq!(wrong, expired);
        assert_eq!(expired, used);
        for problem in ["wrong", "expired", "used"] {
            let text = message_for_code_problem(problem);
            assert!(
                !text.to_lowercase().contains(problem),
                "{problem} leaked into {text}"
            );
        }

        assert_eq!(
            user_notice(Some(400), r#"{"error":"wrong"}"#),
            UserNotice::Text(CODE_DIDNT_WORK)
        );
        assert_eq!(
            user_notice(Some(404), r#"{"error":"expired"}"#),
            UserNotice::Text(CODE_DIDNT_WORK)
        );
        assert_eq!(
            user_notice(Some(409), r#"{"error":"used"}"#),
            UserNotice::Text(CODE_DIDNT_WORK)
        );
        assert_eq!(
            user_notice(Some(410), "used"),
            UserNotice::Text(CODE_DIDNT_WORK)
        );
        assert_eq!(
            user_notice(Some(429), r#"{"error":"slow down"}"#),
            UserNotice::Text(RATE_LIMITED)
        );
        assert_eq!(RATE_LIMITED, "Too many tries. Wait a moment and try again.");
        assert!(!RATE_LIMITED.chars().any(|c| c.is_ascii_digit()));
        assert_ne!(
            user_notice(Some(429), "expired"),
            user_notice(Some(404), "expired")
        );
    }

    #[test]
    fn normalisation_uppercases_and_strips_spaces() {
        assert_eq!(normalize_user_code(" ab cd "), "ABCD");
        assert_eq!(normalize_user_code("ab-cd"), "AB-CD");
        assert_eq!(normalize_user_code("Wd jb-mjht"), "WDJB-MJHT");
        assert_eq!(normalize_user_code("a\nb\tc"), "ABC");
        assert_eq!(normalize_user_code("a\u{00a0}b"), "AB");
        assert_eq!(normalize_user_code("   "), "");

        let body = UserCodeRequest::from_raw("  wdjb mjht ");
        assert_eq!(body.user_code, "WDJBMJHT");
        let json = serde_json::to_value(&body).expect("json");
        assert_eq!(json, serde_json::json!({ "user_code": "WDJBMJHT" }));
    }

    #[test]
    fn pending_lookup_fields_and_outcome_copy() {
        let parsed: PendingDeviceCode = serde_json::from_str(
            r#"{"device_label":"Kitchen PC","app_version":"1.2.3","created_at":"2026-10-09T13:18:00Z"}"#,
        )
        .expect("pending device");
        assert_eq!(parsed.device_label, "Kitchen PC");
        assert_eq!(parsed.app_version, "1.2.3");
        assert_eq!(
            format_request_time(&parsed.created_at),
            "Oct 09, 2026 13:18 UTC"
        );
        assert_eq!(APPROVED_COPY, "Done, go back to the app");
        assert_eq!(DENIED_COPY, "Request denied");
        assert_eq!(Route::StatsTokens {}.to_string(), "/stats/tokens");
        for text in [
            CODE_DIDNT_WORK,
            RATE_LIMITED,
            COULD_NOT_REACH,
            APPROVED_COPY,
            DENIED_COPY,
        ] {
            assert!(!text.contains('\u{2014}'), "{text}");
            assert!(!text.contains('\u{2013}'), "{text}");
        }
    }

    #[test]
    fn return_flag_round_trip_stores_no_code() {
        assert!(!take_return_to_link());
        arm_return_to_link();
        assert!(take_return_to_link());
        assert!(!take_return_to_link());
    }
}
