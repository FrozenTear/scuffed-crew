//! `/link` lets a signed-in member approve a stat-tracker device code.
//!
//! The code is typed. A `code` or `user_code` query param is ignored, and the
//! login return path is only `/link`.

mod api;

use std::cell::Cell;

use dioxus::prelude::*;

use scuffed_types::DeviceLinkLookupResponse;

use crate::routes::Route;
use crate::state::use_auth;
use api::LinkCallError;

/// Shown for a wrong, expired, or already-used code. Those cases stay identical.
pub const CODE_DIDNT_WORK: &str = "That code didn't work. Check the app and try again.";

/// Plain-language 429 when the server sent no `Retry-After` header.
pub const RATE_LIMITED: &str = "Too many tries. Wait a moment and try again.";

pub const NOT_A_MEMBER: &str = "You need to be an org member to link a device.";

pub const ORIGIN_UNCONFIRMED: &str =
    "This page couldn't confirm the sign-in. Reload and try again.";

pub const SOMETHING_WENT_WRONG: &str = "Something went wrong. Try again.";

pub const COULD_NOT_REACH: &str = "Couldn't reach the site. Try again.";

pub const APPROVED_COPY: &str = "Done, go back to the app";

pub const DENIED_COPY: &str = "Request denied";

/// Where login sends the member back. Never includes a device code.
pub const LOGIN_RETURN_PATH: &str = "/link";

pub const LINK_ERROR_ID: &str = "link-code-error";

pub const LINK_STEP_ENTER: &str = "link-step-enter";
pub const LINK_STEP_CONFIRM: &str = "link-step-confirm";
pub const LINK_STEP_APPROVED: &str = "link-step-approved";
pub const LINK_STEP_DENIED: &str = "link-step-denied";

/// Waits longer than this, or a missing header, use the plain 429 sentence.
const RETRY_AFTER_CAP_SECS: u64 = 3600;

const RETURN_STORAGE_KEY: &str = "scuffed.return-to-link";
const RETURN_STORAGE_VALUE: &str = "1";

thread_local! {
    static RETURN_TO_LINK: Cell<bool> = const { Cell::new(false) };
    /// Session copy of the return flag. Host tests read this. Wasm also writes
    /// `sessionStorage`, which is cleared through the same path.
    static STORED_RETURN_TO_LINK: Cell<bool> = const { Cell::new(false) };
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum UserNotice {
    Text(String),
    SignIn,
}

/// Wrong, expired, and used stay on one sentence. `problem` is not shown.
pub fn message_for_code_problem(problem: &str) -> &'static str {
    let _ignored = problem;
    CODE_DIDNT_WORK
}

fn is_rejected_code(status: u16) -> bool {
    matches!(status, 400 | 404 | 409 | 410)
}

/// Seconds from a `Retry-After` delay or HTTP-date.
///
/// `scuffed_types` has no `json_retry_after` on this branch, so the wrong-code
/// 429 is read from this header. The JSON body is not parsed for a wait.
/// `None` means missing, unreadable, or longer than [`RETRY_AFTER_CAP_SECS`].
pub fn retry_after_seconds(
    header: Option<&str>,
    now: chrono::DateTime<chrono::Utc>,
) -> Option<u64> {
    let raw = header?.trim();
    if raw.is_empty() {
        return None;
    }
    let secs = if raw.bytes().all(|byte| byte.is_ascii_digit()) {
        raw.parse().ok()?
    } else {
        let when = chrono::DateTime::parse_from_rfc2822(raw).ok()?;
        let delta = when.with_timezone(&chrono::Utc) - now;
        u64::try_from(delta.num_seconds().max(1)).unwrap_or(u64::MAX)
    };
    let secs = secs.max(1);
    (secs <= RETRY_AFTER_CAP_SECS).then_some(secs)
}

pub fn rate_limit_message_at(header: Option<&str>, now: chrono::DateTime<chrono::Utc>) -> String {
    match retry_after_seconds(header, now) {
        Some(1) => "Too many tries. Try again in 1 second.".to_string(),
        Some(secs) => format!("Too many tries. Try again in {secs} seconds."),
        None => RATE_LIMITED.to_string(),
    }
}

/// `aria-invalid` and `aria-describedby` for the code field.
/// Both are omitted when the field has no error.
pub fn code_field_aria(has_error: bool) -> (Option<&'static str>, Option<&'static str>) {
    if has_error {
        (Some("true"), Some(LINK_ERROR_ID))
    } else {
        (None, None)
    }
}

/// Element that should take focus after this step becomes visible.
pub fn step_focus_id(step: &LinkStep) -> &'static str {
    match step {
        LinkStep::Enter => LINK_STEP_ENTER,
        LinkStep::Confirm { .. } => LINK_STEP_CONFIRM,
        LinkStep::Approved => LINK_STEP_APPROVED,
        LinkStep::Denied => LINK_STEP_DENIED,
    }
}

fn focus_link_step(step: &LinkStep) {
    let id = step_focus_id(step);
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
        let _ = id;
    }
}

fn json_error_is(body: &str, code: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|value| value.get("error")?.as_str().map(str::to_owned))
        .is_some_and(|found| found == code)
}

pub fn rate_limit_message(header: Option<&str>) -> String {
    rate_limit_message_at(header, chrono::Utc::now())
}

/// A 403 `bad_origin` is its own sentence. A 429 uses `retry_after`, not the payload.
pub fn user_notice(status: Option<u16>, body: &str, retry_after: Option<&str>) -> UserNotice {
    match status {
        Some(401) => UserNotice::SignIn,
        Some(403) if json_error_is(body, "bad_origin") => {
            UserNotice::Text(ORIGIN_UNCONFIRMED.to_string())
        }
        Some(403) => UserNotice::Text(NOT_A_MEMBER.to_string()),
        Some(429) => UserNotice::Text(rate_limit_message(retry_after)),
        Some(status) if is_rejected_code(status) => {
            UserNotice::Text(message_for_code_problem("invalid code").to_string())
        }
        Some(_) => UserNotice::Text(SOMETHING_WENT_WRONG.to_string()),
        None => UserNotice::Text(COULD_NOT_REACH.to_string()),
    }
}

fn notice_from_call(err: &LinkCallError) -> UserNotice {
    user_notice(err.status, &err.body, err.retry_after.as_deref())
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
    let stored = take_stored_return_flag();
    memory || stored
}

/// Drop a pending `/link` return without following it.
pub(crate) fn clear_return_to_link() {
    RETURN_TO_LINK.with(|flag| flag.set(false));
    let _ = take_stored_return_flag();
}

pub(crate) fn return_to_link_pending() -> bool {
    RETURN_TO_LINK.with(|flag| flag.get()) || STORED_RETURN_TO_LINK.with(|flag| flag.get())
}

fn store_return_flag() {
    STORED_RETURN_TO_LINK.with(|flag| flag.set(true));
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        if let Some(storage) = session_storage() {
            let _ = storage.set_item(RETURN_STORAGE_KEY, RETURN_STORAGE_VALUE);
        }
    }
}

fn take_stored_return_flag() -> bool {
    let mirrored = STORED_RETURN_TO_LINK.with(|flag| flag.replace(false));
    mirrored || take_browser_return_flag()
}

fn take_browser_return_flag() -> bool {
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
.link-card h2.link-step,
.link-card p.note {
    color: var(--text-2);
    font-size: 0.9rem;
    font-weight: 400;
    font-family: inherit;
    margin: 0 0 1.25rem;
    line-height: 1.45;
}
.link-card h2.link-step:focus {
    outline: 2px solid var(--accent);
    outline-offset: 3px;
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
    let mut pending = use_signal(|| None::<DeviceLinkLookupResponse>);
    let mut notice: Signal<Option<String>> = use_signal(|| None);
    let mut busy = use_signal(|| false);

    use_effect(move || {
        if link_gate(auth().loading, auth().is_logged_in()) == LinkGate::SendToLogin {
            arm_return_to_link();
            nav.replace(Route::Login {});
        }
    });

    let mut focused_step = use_signal(|| None::<LinkStep>);
    use_effect(move || {
        let current = step();
        let previous = focused_step.peek().clone();
        if previous.as_ref() == Some(&current) {
            return;
        }
        let moved = previous.is_some();
        focused_step.set(Some(current.clone()));
        if moved {
            focus_link_step(&current);
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
            notice.set(Some("Enter the code from the app.".to_string()));
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
                        LinkCodeEntry {
                            code: code(),
                            notice: notice(),
                            busy: busy(),
                            on_lookup: on_lookup,
                            on_code: move |value| code.set(value),
                        }
                    },
                    LinkStep::Confirm { code: shown } => rsx! {
                        h2 {
                            id: LINK_STEP_CONFIRM,
                            class: "link-step",
                            tabindex: "-1",
                            "Check this request, then approve or deny it."
                        }
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
                            p { class: "link-error", role: "alert", "{text}" }
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
                        h2 {
                            id: LINK_STEP_APPROVED,
                            class: "link-step",
                            tabindex: "-1",
                            "{APPROVED_COPY}"
                        }
                        p { class: "note",
                            "The new token can be revoked on the "
                            Link { to: Route::StatsTokens {}, "tracker tokens list" }
                            "."
                        }
                    },
                    LinkStep::Denied => rsx! {
                        h2 {
                            id: LINK_STEP_DENIED,
                            class: "link-step",
                            tabindex: "-1",
                            "{DENIED_COPY}"
                        }
                    },
                }
            }
        }
    }
}

#[component]
fn LinkCodeEntry(
    code: String,
    notice: Option<String>,
    busy: bool,
    on_lookup: EventHandler<Event<FormData>>,
    on_code: EventHandler<String>,
) -> Element {
    let has_error = notice.is_some();
    let (code_invalid, code_described_by) = code_field_aria(has_error);
    rsx! {
        h2 {
            id: LINK_STEP_ENTER,
            class: "link-step",
            tabindex: "-1",
            "Type the short code shown in the app."
        }
        if let Some(text) = notice {
            p {
                id: LINK_ERROR_ID,
                class: "link-error",
                role: "alert",
                "{text}"
            }
        }
        form {
            onsubmit: move |evt| on_lookup.call(evt),
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
                    disabled: busy,
                    aria_invalid: code_invalid,
                    aria_describedby: code_described_by,
                    oninput: move |evt| on_code.call(evt.value()),
                }
            }
            button {
                class: "ui-btn ui-btn--primary ui-btn--md",
                r#type: "submit",
                disabled: busy,
                if busy { "Checking the code." } else { "Continue" }
            }
        }
    }
}

fn apply_client_error(
    notice: &mut Signal<Option<String>>,
    nav: &dioxus_router::Navigator,
    err: LinkCallError,
) {
    match notice_from_call(&err) {
        UserNotice::SignIn => {
            arm_return_to_link();
            nav.replace(Route::Login {});
        }
        UserNotice::Text(text) => notice.set(Some(text)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scuffed_types::{DeviceLinkLookupResponse, DeviceLinkUserCodeRequest};
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

        let didnt = UserNotice::Text(CODE_DIDNT_WORK.to_string());
        assert_eq!(
            user_notice(Some(400), r#"{"error":"invalid code"}"#, None),
            didnt
        );
        assert_eq!(
            user_notice(Some(404), r#"{"error":"expired"}"#, None),
            didnt
        );
        assert_eq!(user_notice(Some(409), r#"{"error":"used"}"#, None), didnt);
        assert_eq!(user_notice(Some(410), "used", None), didnt);
        assert!(
            !CODE_DIDNT_WORK.contains("invalid code"),
            "server wording leaked"
        );
        assert_eq!(
            user_notice(
                Some(429),
                r#"{"error":"rate_limited","retry_after":90}"#,
                None
            ),
            UserNotice::Text(RATE_LIMITED.to_string())
        );
        assert_eq!(RATE_LIMITED, "Too many tries. Wait a moment and try again.");
        assert!(!RATE_LIMITED.chars().any(|c| c.is_ascii_digit()));
        assert_ne!(
            user_notice(Some(429), "expired", None),
            user_notice(Some(404), "expired", None)
        );
    }

    #[test]
    fn rate_limit_uses_retry_after_and_ignores_the_body() {
        let header = user_notice(
            Some(429),
            r#"{"error":"rate_limited","retry_after":9}"#,
            Some("90"),
        );
        assert_eq!(
            header,
            UserNotice::Text("Too many tries. Try again in 90 seconds.".to_string())
        );
        let shown = match header {
            UserNotice::Text(text) => text,
            UserNotice::SignIn => panic!("429 is not a sign-in"),
        };
        assert!(!shown.contains("rate_limited"));
        assert!(!shown.contains("retry_after"));

        let governor = user_notice(Some(429), "Too Many Requests! Wait for 9s", Some("2"));
        assert_eq!(
            governor,
            UserNotice::Text("Too many tries. Try again in 2 seconds.".to_string())
        );
        let governor_text = match governor {
            UserNotice::Text(text) => text,
            UserNotice::SignIn => panic!("429 is not a sign-in"),
        };
        assert!(!governor_text.contains('9'));
        assert!(!governor_text.contains("Too Many"));

        assert_eq!(
            user_notice(Some(429), "Too Many Requests! Wait for 9s", None),
            UserNotice::Text(RATE_LIMITED.to_string())
        );
        assert_eq!(retry_after_seconds(Some("0"), chrono::Utc::now()), Some(1));
        assert_eq!(
            rate_limit_message(Some("  45  ")),
            "Too many tries. Try again in 45 seconds."
        );
        assert_eq!(
            rate_limit_message(Some("1")),
            "Too many tries. Try again in 1 second."
        );
        assert_eq!(
            rate_limit_message(Some("3600")),
            "Too many tries. Try again in 3600 seconds."
        );
        assert_eq!(rate_limit_message(Some("3601")), RATE_LIMITED);
        assert_eq!(retry_after_seconds(Some("3601"), chrono::Utc::now()), None);
        assert_eq!(rate_limit_message(None), RATE_LIMITED);

        let now = chrono::DateTime::parse_from_rfc3339("2026-10-09T13:00:00Z")
            .expect("now")
            .with_timezone(&chrono::Utc);
        let when = "Fri, 09 Oct 2026 13:00:30 GMT";
        assert_eq!(retry_after_seconds(Some(when), now), Some(30));
        assert_eq!(
            rate_limit_message_at(Some(when), now),
            "Too many tries. Try again in 30 seconds."
        );
        let far = "Fri, 09 Oct 2026 15:00:01 GMT";
        assert_eq!(retry_after_seconds(Some(far), now), None);
        assert_eq!(rate_limit_message_at(Some(far), now), RATE_LIMITED);
    }

    fn render_entry(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn error_entry() -> Element {
        rsx! {
            LinkCodeEntry {
                code: "ABCD".to_string(),
                notice: Some("Enter the code from the app.".to_string()),
                busy: false,
                on_lookup: |_| {},
                on_code: |_| {},
            }
        }
    }

    fn clean_entry() -> Element {
        rsx! {
            LinkCodeEntry {
                code: "ABCD".to_string(),
                notice: None,
                busy: false,
                on_lookup: |_| {},
                on_code: |_| {},
            }
        }
    }

    #[test]
    fn error_state_exposes_alert_and_invalid_code_field() {
        let html = render_entry(error_entry);
        assert!(html.contains("role=\"alert\""), "{html}");
        assert!(html.contains("id=\"link-code-error\""), "{html}");
        assert!(
            html.contains("aria-invalid=\"true\""),
            "the code field is invalid while an error is showing: {html}"
        );
        assert!(
            html.contains("aria-describedby=\"link-code-error\""),
            "the input points at the error: {html}"
        );
        let input = html
            .split("<input")
            .nth(1)
            .expect("code input")
            .split('>')
            .next()
            .expect("input tag");
        assert!(
            input.contains("aria-invalid=\"true\""),
            "aria-invalid belongs on the input: {input}"
        );
        assert!(
            input.contains("aria-describedby=\"link-code-error\""),
            "aria-describedby belongs on the input: {input}"
        );
    }

    #[test]
    fn clean_code_field_omits_aria_invalid() {
        let html = render_entry(clean_entry);
        assert!(!html.contains("role=\"alert\""), "{html}");
        assert!(
            !html.contains("aria-invalid"),
            "a valid field omits aria-invalid: {html}"
        );
        assert!(
            !html.contains("aria-describedby"),
            "a valid field has no error to describe: {html}"
        );
        assert!(html.contains("id=\"link-code\""), "{html}");
    }

    #[test]
    fn steps_take_focus_on_their_heading() {
        assert_eq!(step_focus_id(&LinkStep::Enter), LINK_STEP_ENTER);
        assert_eq!(
            step_focus_id(&LinkStep::Confirm {
                code: "ABCD".into()
            }),
            LINK_STEP_CONFIRM
        );
        assert_eq!(step_focus_id(&LinkStep::Approved), LINK_STEP_APPROVED);
        assert_eq!(step_focus_id(&LinkStep::Denied), LINK_STEP_DENIED);
        assert_ne!(LINK_STEP_ENTER, LINK_STEP_CONFIRM);
    }

    #[test]
    fn member_and_server_errors_do_not_echo_the_body() {
        assert_eq!(
            user_notice(Some(403), r#"{"error":"Not an org member"}"#, None),
            UserNotice::Text(NOT_A_MEMBER.to_string())
        );
        let member = match user_notice(Some(403), "Not an org member", None) {
            UserNotice::Text(text) => text,
            UserNotice::SignIn => panic!("403 is not a sign-in"),
        };
        assert!(!member.contains("Not an org member"));
        assert_ne!(member, CODE_DIDNT_WORK);

        assert_eq!(
            user_notice(Some(403), r#"{"error":"bad_origin"}"#, None),
            UserNotice::Text(ORIGIN_UNCONFIRMED.to_string())
        );
        let origin = match user_notice(Some(403), r#"{"error":"bad_origin"}"#, None) {
            UserNotice::Text(text) => text,
            UserNotice::SignIn => panic!("403 is not a sign-in"),
        };
        assert_eq!(
            origin,
            "This page couldn't confirm the sign-in. Reload and try again."
        );
        assert!(!origin.contains("bad_origin"));
        assert_ne!(origin, NOT_A_MEMBER);

        assert_eq!(
            user_notice(Some(500), r#"{"error":"Internal error"}"#, None),
            UserNotice::Text(SOMETHING_WENT_WRONG.to_string())
        );
        let server = match user_notice(Some(500), "Internal error", None) {
            UserNotice::Text(text) => text,
            UserNotice::SignIn => panic!("500 is not a sign-in"),
        };
        assert!(!server.contains("Internal error"));
        assert_eq!(user_notice(Some(401), "nope", None), UserNotice::SignIn);
        assert_eq!(
            user_notice(None, "network", None),
            UserNotice::Text(COULD_NOT_REACH.to_string())
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

        let body = DeviceLinkUserCodeRequest {
            user_code: normalize_user_code("  wdjb mjht "),
        };
        assert_eq!(body.user_code, "WDJBMJHT");
        let json = serde_json::to_value(&body).expect("json");
        assert_eq!(json, serde_json::json!({ "user_code": "WDJBMJHT" }));
    }

    #[test]
    fn pending_lookup_fields_and_outcome_copy() {
        let parsed: DeviceLinkLookupResponse = serde_json::from_str(
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
            NOT_A_MEMBER,
            ORIGIN_UNCONFIRMED,
            SOMETHING_WENT_WRONG,
            COULD_NOT_REACH,
            APPROVED_COPY,
            DENIED_COPY,
            "Too many tries. Try again in 90 seconds.",
            "Too many tries. Try again in 1 second.",
            "Too many tries. Try again in 3600 seconds.",
            "Enter the code from the app.",
        ] {
            assert!(!text.contains('\u{2014}'), "{text}");
            assert!(!text.contains('\u{2013}'), "{text}");
        }
    }

    #[test]
    fn return_flag_round_trip_stores_no_code() {
        clear_return_to_link();
        assert!(!return_to_link_pending());
        assert!(!take_return_to_link());
        arm_return_to_link();
        assert!(return_to_link_pending());
        assert!(take_return_to_link());
        assert!(
            !return_to_link_pending(),
            "consuming the flag clears the session copy too"
        );
        assert!(!take_return_to_link());
    }
}
