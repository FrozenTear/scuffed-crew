use std::str::FromStr;
use std::sync::Mutex;

use dioxus::prelude::*;
use scuffed_api_client::ApiClient;
use scuffed_types::{
    AuthProvidersResponse, LocalLoginRequest, OkResponse, RegisterRequest, SetupStatusResponse,
};

use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};
use scuffed_types::{MeResponse, OrgRole, UserInfo};

const CSS: &str = r#"
.login-page {
    min-height: 100vh;
    display: flex;
    align-items: center;
    justify-content: center;
    padding: 2rem;
    background: var(--bg);
}
.login-card {
    width: 100%;
    max-width: 420px;
    background: var(--surface);
    border: 1px solid var(--border);
    border-radius: 8px;
    padding: 2rem;
}
.login-card h1 {
    font-family: var(--font-head);
    font-size: 1.5rem;
    color: var(--text);
    margin: 0 0 0.5rem;
}
.login-card p.lead {
    color: var(--text-2);
    font-size: 0.9rem;
    margin: 0 0 1.5rem;
}
.login-field {
    display: flex;
    flex-direction: column;
    gap: 0.35rem;
    margin-bottom: 1rem;
}
.login-field label {
    font-size: 0.75rem;
    color: var(--text-3);
    text-transform: uppercase;
    letter-spacing: 0.04em;
}
.login-field input {
    background: var(--bg);
    border: 1px solid var(--border);
    color: var(--text);
    padding: 0.6rem 0.75rem;
    border-radius: 6px;
    font-size: 1rem;
}
.login-error {
    color: var(--danger);
    font-size: 0.85rem;
    margin-bottom: 1rem;
}
.login-nostr-btn {
    width: 100%;
    padding: 0.6rem 1rem;
    border-radius: 6px;
    border: 1px solid var(--accent-soft);
    background: var(--surface);
    color: var(--accent);
    font-size: 0.9rem;
    font-weight: 600;
    cursor: pointer;
    transition: all 0.15s;
}
.login-nostr-btn:hover:not(:disabled) {
    border-color: var(--accent);
    background: var(--accent-soft);
}
.login-nostr-btn:disabled { opacity: 0.6; cursor: default; }
.login-nostr-hint {
    font-size: 0.72rem;
    color: var(--text-3);
    margin: 0.4rem 0 0;
    text-align: center;
}
.login-agecheck {
    display: flex;
    align-items: center;
    gap: 0.5rem;
    margin-bottom: 1rem;
    font-size: 0.85rem;
    color: var(--text-2);
}
.login-switch {
    margin-top: 1rem;
    text-align: center;
}
.login-linkish {
    background: none;
    border: none;
    color: var(--accent);
    font-size: 0.85rem;
    font-weight: 600;
    cursor: pointer;
    padding: 0;
}
.login-linkish:hover {
    text-decoration: underline;
}
.login-oauth {
    display: flex;
    flex-direction: column;
    gap: 0.5rem;
    margin-top: 1.25rem;
}
.login-oauth a {
    text-align: center;
    padding: 0.55rem;
    border: 1px solid var(--border);
    border-radius: 6px;
    color: var(--text-2);
    font-size: 0.9rem;
}
.login-oauth a:hover {
    color: var(--text);
    border-color: var(--accent);
}
.login-card button[type="submit"] {
    width: 100%;
}
"#;

fn me_to_user_info(me: &MeResponse) -> UserInfo {
    let role = me.member.as_ref().and_then(|m| match m.org_role.as_str() {
        "admin" => Some(OrgRole::Admin),
        "officer" => Some(OrgRole::Officer),
        "member" => Some(OrgRole::Member),
        "recruit" => Some(OrgRole::Recruit),
        _ => None,
    });
    UserInfo {
        id: me.user.id.clone(),
        username: me
            .member
            .as_ref()
            .map(|m| m.display_name.clone())
            .unwrap_or_else(|| me.user.username.clone()),
        avatar_url: me.user.avatar_url.clone(),
        role,
    }
}

/// Pull the server's `{"error": "..."}` message out of an HTTP error body.
fn body_error_or(body: &str, fallback: &str) -> String {
    serde_json::from_str::<serde_json::Value>(body)
        .ok()
        .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(String::from))
        .unwrap_or_else(|| fallback.to_string())
}

/// Shown when OAuth sends a brand-new account to `/login?error=registration_closed`.
const REGISTRATION_CLOSED_BANNER: &str =
    "The crew isn't taking new sign-ups right now. Existing members can still sign in.";

/// Map a `/login` `error` query value to banner copy.
///
/// Only `registration_closed` is recognized. Any other value, including a
/// missing parameter, returns `None` so the page stays as it is today.
fn login_error_banner(code: Option<&str>) -> Option<&'static str> {
    match code {
        Some("registration_closed") => Some(REGISTRATION_CLOSED_BANNER),
        _ => None,
    }
}

/// First `error` value in a URL search string (`?error=...` or `error=...`).
fn login_error_code(search: &str) -> Option<&str> {
    let query = search.strip_prefix('?').unwrap_or(search);
    if query.is_empty() {
        return None;
    }
    query.split('&').find_map(|pair| {
        let (key, value) = pair.split_once('=').unwrap_or((pair, ""));
        (key == "error").then_some(value)
    })
}

fn login_banner_from_search(search: &str) -> Option<&'static str> {
    login_error_banner(login_error_code(search))
}

/// Banner for a browser URL in the shape `history.current_route()` returns
/// (`pathname` + `search` + `hash`). Only the login route is considered.
///
/// Dioxus router 0.7 (`RouterContext::new`) parses that string, then if
/// `route.to_string()` differs it `history.replace`s the canonical route.
/// `/login` declares no query, so `/login?error=registration_closed` becomes
/// `/login` before [`Login`] mounts. A `location.search` read inside the page
/// is already empty. Callers must pass the URL from before that replace.
fn login_banner_from_browser_url(url: &str) -> Option<&'static str> {
    let without_hash = url.split_once('#').map(|(path, _)| path).unwrap_or(url);
    let (path, search) = without_hash.split_once('?').unwrap_or((without_hash, ""));
    if !is_login_route(path) {
        return None;
    }
    login_banner_from_search(search)
}

/// `Route::Login` after dropping a trailing slash (`/login/`).
///
/// Parsing goes through [`Route`] so a query segment added to the login route
/// stays in one place. The query itself is not part of this check: [`Route`]
/// ignores undeclared query params, which is the bug this snapshot exists for.
fn is_login_route(path: &str) -> bool {
    let trimmed = path.trim_end_matches('/');
    let path = if trimmed.is_empty() { "/" } else { trimmed };
    matches!(Route::from_str(path), Ok(Route::Login {}))
}

/// Process-wide snapshot of the banner taken before the router rewrites the
/// address bar. One slot for the whole process: the WASM app is a single page,
/// so that matches one full load. A server that rendered many documents in one
/// process would need a per-request slot instead.
enum LoginBannerSlot {
    /// [`capture_initial_login_banner`] has not run.
    Pending,
    /// Captured, not yet shown.
    Ready(Option<&'static str>),
    /// [`Login`] already consumed it. Later mounts in this process see nothing.
    Taken,
}

static LOGIN_BANNER: Mutex<LoginBannerSlot> = Mutex::new(LoginBannerSlot::Pending);

fn login_banner_lock() -> std::sync::MutexGuard<'static, LoginBannerSlot> {
    LOGIN_BANNER.lock().unwrap_or_else(|err| err.into_inner())
}

/// Read `window.location` once, before [`dioxus::launch`] or before [`crate::routes::Route`]
/// is mounted. Later calls keep the first snapshot, including after [`Login`] takes it.
pub(crate) fn capture_initial_login_banner() {
    capture_login_banner_from_url(&initial_browser_url());
}

/// Record the banner for `url` (`pathname` + `search` + `hash`). The first call
/// wins; a later call does not refill the slot after [`take_captured_login_banner`].
fn capture_login_banner_from_url(url: &str) {
    let mut slot = login_banner_lock();
    if !matches!(*slot, LoginBannerSlot::Pending) {
        return;
    }
    *slot = LoginBannerSlot::Ready(login_banner_from_browser_url(url));
}

fn initial_browser_url() -> String {
    #[cfg(target_arch = "wasm32")]
    {
        current_browser_url()
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        String::new()
    }
}

/// Consume the snapshot. The first [`Login`] mount after a full-page load shows
/// it; every later mount in this process gets `None` until a new page load.
fn take_captured_login_banner() -> Option<&'static str> {
    let mut slot = login_banner_lock();
    debug_assert!(
        !matches!(*slot, LoginBannerSlot::Pending),
        "capture_initial_login_banner must run before Login mounts"
    );
    match std::mem::replace(&mut *slot, LoginBannerSlot::Taken) {
        LoginBannerSlot::Ready(banner) => banner,
        LoginBannerSlot::Taken | LoginBannerSlot::Pending => None,
    }
}

#[cfg(test)]
fn reset_captured_login_banner() {
    *login_banner_lock() = LoginBannerSlot::Pending;
}

/// `pathname + search + hash`, matching `dioxus_web::history::WebHistory`.
#[cfg(target_arch = "wasm32")]
fn current_browser_url() -> String {
    let Some(window) = web_sys::window() else {
        return String::new();
    };
    let location = window.location();
    let path = location.pathname().unwrap_or_default();
    let search = location.search().unwrap_or_default();
    let hash = location.hash().unwrap_or_default();
    format!("{path}{search}{hash}")
}

#[component]
pub fn Login() -> Element {
    let mut username = use_signal(String::new);
    let mut password = use_signal(String::new);
    let mut password2 = use_signal(String::new);
    let mut confirm_age = use_signal(|| false);
    let mut registering = use_signal(|| false);
    // Take once, inside the signal initializer, so a re-render does not consume
    // it again and a later navigation to plain `/login` does not see it.
    // Reading `location.search` here is too late: the router has already
    // replaced the URL with `/login`.
    let mut error = use_signal(|| take_captured_login_banner().map(String::from));
    let mut submitting = use_signal(|| false);
    let mut auth = use_auth();
    let nav = use_navigator();

    let setup = use_resource(|| async move {
        ApiClient::web()
            .fetch::<SetupStatusResponse>("/api/auth/setup-status")
            .await
            .ok()
    });

    let providers = use_resource(|| async move {
        ApiClient::web()
            .fetch::<AuthProvidersResponse>("/api/auth/providers")
            .await
            .ok()
    });

    use_effect(move || {
        if let Some(Some(s)) = setup.value()()
            && s.needs_setup
        {
            nav.replace(Route::Setup {});
        }
    });

    let on_submit = move |evt: Event<FormData>| {
        evt.prevent_default();
        error.set(None);
        let user = username();
        let pass = password();
        if user.trim().is_empty() || pass.is_empty() {
            error.set(Some("Enter username and password".into()));
            return;
        }
        submitting.set(true);
        spawn(async move {
            let client = ApiClient::web();
            let body = LocalLoginRequest {
                username: user,
                password: pass,
            };
            match client
                .post_json::<_, OkResponse>("/api/auth/local/login", &body)
                .await
            {
                Ok(_) => {
                    match client.get_me().await {
                        Ok(me) => {
                            let is_member = me.member.is_some();
                            auth.set(AuthState {
                                user: Some(me_to_user_info(&me)),
                                loading: false,
                            });
                            // Align with register / Nostr: bare accounts go to Apply.
                            if is_member {
                                nav.replace(Route::Home {});
                            } else {
                                nav.replace(Route::Apply {});
                            }
                        }
                        Err(_) => {
                            nav.replace(Route::Home {});
                        }
                    }
                }
                Err(_) => {
                    error.set(Some("Invalid username or password".into()));
                    submitting.set(false);
                }
            }
        });
    };

    let on_register = move |evt: Event<FormData>| {
        evt.prevent_default();
        error.set(None);
        let user = username().trim().to_string();
        let pass = password();
        if user.is_empty() || pass.is_empty() {
            error.set(Some("Enter username and password".into()));
            return;
        }
        if pass != password2() {
            error.set(Some("Passwords do not match".into()));
            return;
        }
        if !confirm_age() {
            error.set(Some("You must confirm the age requirement".into()));
            return;
        }
        submitting.set(true);
        spawn(async move {
            let client = ApiClient::web();
            let body = RegisterRequest {
                username: user,
                password: pass,
                confirm_min_age: true,
            };
            match client
                .post_json::<_, OkResponse>("/api/auth/local/register", &body)
                .await
            {
                Ok(_) => {
                    if let Ok(me) = client.get_me().await {
                        auth.set(AuthState {
                            user: Some(me_to_user_info(&me)),
                            loading: false,
                        });
                    }
                    // New accounts exist to join — funnel straight to the application.
                    nav.replace(Route::Apply {});
                }
                Err(e) => {
                    error.set(Some(match e {
                        // 409 is a conflict (including a taken name). The copy
                        // does not say the name is already taken.
                        scuffed_api_client::ClientError::Http { status: 409, .. } => {
                            "Could not create account. Try a different username.".into()
                        }
                        scuffed_api_client::ClientError::Http { status: 400, body } => {
                            body_error_or(&body, "Check your input")
                        }
                        scuffed_api_client::ClientError::Http { status: 403, .. } => {
                            "Registration is currently closed".into()
                        }
                        _ => "Registration failed — try again".into(),
                    }));
                    submitting.set(false);
                }
            }
        });
    };

    let on_nostr_login = move |_| {
        error.set(None);
        submitting.set(true);
        spawn(async move {
            match nostr_login_flow().await {
                Ok(()) => {
                    let client = ApiClient::web();
                    match client.get_me().await {
                        Ok(me) => {
                            let is_member = me.member.is_some();
                            auth.set(AuthState {
                                user: Some(me_to_user_info(&me)),
                                loading: false,
                            });
                            // New/bare users go straight to the application funnel.
                            if is_member {
                                nav.replace(Route::Home {});
                            } else {
                                nav.replace(Route::Apply {});
                            }
                        }
                        Err(_) => {
                            nav.replace(Route::Home {});
                        }
                    }
                }
                Err(msg) => {
                    error.set(Some(msg));
                    submitting.set(false);
                }
            }
        });
    };

    let p = providers.value()().flatten();
    let show_local = p.as_ref().map(|x| x.local).unwrap_or(true);
    let show_discord = p.as_ref().map(|x| x.discord).unwrap_or(false);
    let show_google = p.as_ref().map(|x| x.google).unwrap_or(false);
    let show_register = p.as_ref().map(|x| x.register).unwrap_or(false);
    let show_nostr = p.as_ref().map(|x| x.nostr).unwrap_or(false) && has_nip07_extension();
    let min_age = p.as_ref().map(|x| x.min_age).unwrap_or(16);

    rsx! {
        style { {CSS} }
        div { class: "login-page",
            div { class: "login-card",
                h1 { if registering() { "Create account" } else { "Sign in" } }
                p { class: "lead",
                    if registering() {
                        "No email needed — just pick a username and password."
                    } else {
                        "Sign in to continue."
                    }
                }
                if let Some(err) = error() {
                    p { class: "login-error", "{err}" }
                }
                if registering() && show_register {
                    form { onsubmit: on_register,
                        div { class: "login-field",
                            label { r#for: "reg-user", "Username" }
                            input {
                                id: "reg-user",
                                r#type: "text",
                                autocomplete: "username",
                                maxlength: 32,
                                value: "{username}",
                                oninput: move |e| username.set(e.value()),
                            }
                        }
                        div { class: "login-field",
                            label { r#for: "reg-pass", "Password" }
                            input {
                                id: "reg-pass",
                                r#type: "password",
                                autocomplete: "new-password",
                                value: "{password}",
                                oninput: move |e| password.set(e.value()),
                            }
                        }
                        div { class: "login-field",
                            label { r#for: "reg-pass2", "Confirm password" }
                            input {
                                id: "reg-pass2",
                                r#type: "password",
                                autocomplete: "new-password",
                                value: "{password2}",
                                oninput: move |e| password2.set(e.value()),
                            }
                        }
                        div { class: "login-agecheck",
                            input {
                                id: "reg-age",
                                r#type: "checkbox",
                                checked: confirm_age(),
                                onchange: move |e| confirm_age.set(e.checked()),
                            }
                            label { r#for: "reg-age", "I am {min_age} or older" }
                        }
                        button {
                            class: "ui-btn ui-btn--primary ui-btn--md",
                            r#type: "submit",
                            disabled: submitting(),
                            if submitting() { "Creating account…" } else { "Create account" }
                        }
                    }
                    p { class: "login-switch",
                        button {
                            class: "login-linkish",
                            onclick: move |_| { registering.set(false); error.set(None); },
                            "Have an account? Sign in"
                        }
                    }
                }
                if !registering() && show_local {
                    form { onsubmit: on_submit,
                        div { class: "login-field",
                            label { r#for: "login-user", "Username" }
                            input {
                                id: "login-user",
                                r#type: "text",
                                autocomplete: "username",
                                value: "{username}",
                                oninput: move |e| username.set(e.value()),
                            }
                        }
                        div { class: "login-field",
                            label { r#for: "login-pass", "Password" }
                            input {
                                id: "login-pass",
                                r#type: "password",
                                autocomplete: "current-password",
                                value: "{password}",
                                oninput: move |e| password.set(e.value()),
                            }
                        }
                        button {
                            class: "ui-btn ui-btn--primary ui-btn--md",
                            r#type: "submit",
                            disabled: submitting(),
                            if submitting() { "Signing in…" } else { "Sign in" }
                        }
                    }
                    if show_register {
                        p { class: "login-switch",
                            button {
                                class: "login-linkish",
                                onclick: move |_| { registering.set(true); error.set(None); },
                                "New here? Create an account"
                            }
                        }
                    }
                }
                if show_nostr && !registering() {
                    div { class: "login-oauth",
                        button {
                            class: "login-nostr-btn",
                            disabled: submitting(),
                            onclick: on_nostr_login,
                            "Sign in with Nostr"
                        }
                        p { class: "login-nostr-hint",
                            "Uses your NIP-07 browser extension — no account details shared."
                        }
                    }
                }
                if show_discord || show_google {
                    div { class: "login-oauth",
                        if show_discord {
                            a { href: "/api/auth/discord/login", "Sign in with Discord" }
                        }
                        if show_google {
                            a { href: "/api/auth/google/login", "Sign in with Google" }
                        }
                    }
                }
                if !show_local && !show_discord && !show_google {
                    p { class: "lead", "No login methods are configured." }
                    if cfg!(debug_assertions) {
                        a {
                            href: "/api/dev/login",
                            style: "color: var(--accent);",
                            "Dev login (in-memory only)"
                        }
                    }
                }
            }
        }
    }
}

// ─── Nostr NIP-07 login ──────────────────────────────────────────────────────

fn has_nip07_extension() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &wasm_bindgen::JsValue::from_str("nostr")).ok())
        .map(|v| !v.is_undefined() && !v.is_null())
        .unwrap_or(false)
}

#[derive(serde::Deserialize)]
struct NostrLoginChallenge {
    challenge: String,
    token: String,
}

#[derive(serde::Serialize)]
struct NostrLoginVerifyBody {
    token: String,
    signed_event: serde_json::Value,
}

/// Full NIP-07 login dance: challenge → extension signs → verify → session.
async fn nostr_login_flow() -> Result<(), String> {
    use wasm_bindgen::{JsCast, JsValue};

    let client = ApiClient::web();
    let ch = client
        .fetch::<NostrLoginChallenge>("/api/auth/nostr/challenge")
        .await
        .map_err(|e| format!("Challenge request failed: {e}"))?;

    let window = web_sys::window().ok_or("No window")?;
    let nostr = js_sys::Reflect::get(&window, &JsValue::from_str("nostr"))
        .map_err(|_| "NIP-07 extension not found")?;

    let event_obj = js_sys::Object::new();
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("kind"),
        &JsValue::from_f64(22242.0),
    )
    .map_err(|_| "Failed to set kind")?;
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("content"),
        &JsValue::from_str(&ch.challenge),
    )
    .map_err(|_| "Failed to set content")?;
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("created_at"),
        &JsValue::from_f64(chrono::Utc::now().timestamp() as f64),
    )
    .map_err(|_| "Failed to set created_at")?;
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("tags"),
        &js_sys::Array::new(),
    )
    .map_err(|_| "Failed to set tags")?;

    let sign_fn = js_sys::Reflect::get(&nostr, &JsValue::from_str("signEvent"))
        .map_err(|_| "signEvent not found")?;
    let sign_fn: js_sys::Function = sign_fn
        .dyn_into()
        .map_err(|_| "signEvent is not a function")?;
    let promise = sign_fn
        .call1(&nostr, &event_obj)
        .map_err(|e| format!("signEvent call failed: {e:?}"))?;
    let promise: js_sys::Promise = promise
        .dyn_into()
        .map_err(|_| "signEvent did not return a promise")?;
    let signed_event = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("signEvent rejected: {e:?}"))?;
    let signed_json: serde_json::Value =
        serde_wasm_bindgen::from_value(signed_event).map_err(|e| format!("Parse error: {e}"))?;

    client
        .post_json::<_, OkResponse>(
            "/api/auth/nostr/verify",
            &NostrLoginVerifyBody {
                token: ch.token,
                signed_event: signed_json,
            },
        )
        .await
        .map_err(|e| match e {
            scuffed_api_client::ClientError::Http { status: 403, .. } => {
                "Registration is currently closed".to_string()
            }
            other => format!("Verification failed: {other}"),
        })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::str::FromStr;

    #[test]
    fn registration_closed_maps_to_the_login_banner() {
        assert_eq!(
            login_error_banner(Some("registration_closed")),
            Some(
                "The crew isn't taking new sign-ups right now. Existing members can still sign in."
            )
        );
    }

    #[test]
    fn unknown_or_absent_error_codes_show_nothing() {
        assert_eq!(login_error_banner(None), None);
        assert_eq!(login_error_banner(Some("")), None);
        assert_eq!(login_error_banner(Some("invalid")), None);
        assert_eq!(login_error_banner(Some("access_denied")), None);
        assert_eq!(login_error_banner(Some("Registration_closed")), None);
        assert_eq!(login_error_banner(Some("registration_closed ")), None);
    }

    #[test]
    fn search_string_uses_only_the_error_parameter() {
        let banner = Some(
            "The crew isn't taking new sign-ups right now. Existing members can still sign in.",
        );
        assert_eq!(
            login_banner_from_search("?error=registration_closed"),
            banner
        );
        assert_eq!(
            login_banner_from_search("?foo=1&error=registration_closed&bar=2"),
            banner
        );
        assert_eq!(
            login_banner_from_search("error=registration_closed"),
            banner
        );
        assert_eq!(login_banner_from_search(""), None);
        assert_eq!(login_banner_from_search("?"), None);
        assert_eq!(login_banner_from_search("?error="), None);
        assert_eq!(login_banner_from_search("?error"), None);
        assert_eq!(login_banner_from_search("?error=nope"), None);
        assert_eq!(login_banner_from_search("?foo=registration_closed"), None);
        // A later duplicate does not override an earlier unknown code.
        assert_eq!(
            login_banner_from_search("?error=nope&error=registration_closed"),
            None
        );
    }

    /// `Route::Login` has no query segment, so parsing keeps the variant and
    /// `Display` drops `?error=`. The banner mapping still reads the original URL.
    #[test]
    fn route_display_drops_undeclared_login_query() {
        let original = "/login?error=registration_closed";
        let parsed = Route::from_str(original).expect("login matches with an undeclared query");
        assert_eq!(parsed, Route::Login {});
        let normalized = parsed.to_string();
        assert_eq!(normalized, "/login");
        assert_ne!(normalized, original);
        assert_eq!(login_banner_from_browser_url(&normalized), None);
        assert_eq!(
            login_banner_from_browser_url(original),
            Some(REGISTRATION_CLOSED_BANNER)
        );
        assert_eq!(
            login_banner_from_browser_url("/login?foo=1&error=registration_closed&bar=2"),
            Some(REGISTRATION_CLOSED_BANNER)
        );
        assert_eq!(
            login_banner_from_browser_url("/login/?error=registration_closed"),
            Some(REGISTRATION_CLOSED_BANNER)
        );
        assert_eq!(
            login_banner_from_browser_url("/login?error=registration_closed#gone"),
            Some(REGISTRATION_CLOSED_BANNER)
        );
    }

    #[test]
    fn initial_url_banner_ignores_plain_login_unknown_codes_and_other_paths() {
        assert_eq!(login_banner_from_browser_url("/login"), None);
        assert_eq!(login_banner_from_browser_url("/login?"), None);
        assert_eq!(login_banner_from_browser_url("/login?error="), None);
        assert_eq!(login_banner_from_browser_url("/login?error=nope"), None);
        assert_eq!(
            login_banner_from_browser_url("/login?error=Registration_closed"),
            None
        );
        assert_eq!(
            login_banner_from_browser_url("/login?error=registration_closed "),
            None
        );
        assert_eq!(
            login_banner_from_browser_url("/apply?error=registration_closed"),
            None
        );
        assert_eq!(
            login_banner_from_browser_url("/?error=registration_closed"),
            None
        );
    }

    /// Same `/login` shape as [`crate::routes::Route::Login`]: no query segment,
    /// so [`dioxus_router::RouterContext`] (via [`Router`]) replaces the history
    /// with `/login` before [`Login`] mounts. The banner has to come from the
    /// pre-replace snapshot, and a second mount in this process must not see it.
    #[derive(Clone, Routable, Debug, PartialEq)]
    #[rustfmt::skip]
    enum LoginBannerRoute {
        #[route("/login")]
        Login {},
        #[route("/")]
        BannerProbeHome {},
    }

    #[component]
    fn BannerProbeHome() -> Element {
        rsx! { "home" }
    }

    thread_local! {
        static PROBE_URL: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
        static PROBE_HISTORY: std::cell::RefCell<Option<std::rc::Rc<dioxus::history::MemoryHistory>>> =
            const { std::cell::RefCell::new(None) };
    }

    fn login_banner_probe() -> Element {
        let history = use_hook(|| {
            let url = PROBE_URL.with(|slot| slot.borrow().clone());
            let history = std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path(url));
            // Same function `main` uses, fed the history URL while it still has
            // the query. RouterContext::new has not replaced it yet.
            capture_login_banner_from_url(&history.current_route());
            PROBE_HISTORY.with(|slot| *slot.borrow_mut() = Some(history.clone()));
            history
        });
        let auth = use_signal(crate::state::auth::AuthState::new);
        use_context_provider(|| auth);
        rsx! {
            dioxus::router::components::HistoryProvider {
                history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                Router::<LoginBannerRoute> {}
            }
        }
    }

    /// dioxus-ssr escapes `'` as `&#39;`. The banner copy contains one.
    fn html_has_registration_closed_banner(html: &str) -> bool {
        let escaped = REGISTRATION_CLOSED_BANNER.replace('\'', "&#39;");
        html.contains(REGISTRATION_CLOSED_BANNER) || html.contains(&escaped)
    }

    fn mount_login(url: &str) -> (String, String) {
        PROBE_URL.with(|slot| *slot.borrow_mut() = url.to_string());
        PROBE_HISTORY.with(|slot| *slot.borrow_mut() = None);
        let mut dom = VirtualDom::new(login_banner_probe);
        // Rebuild only. Polling tasks would run the login page's fetches.
        dom.rebuild_in_place();
        let html = dioxus_ssr::render(&dom);
        let route = PROBE_HISTORY.with(|slot| {
            slot.borrow()
                .as_ref()
                .expect("probe history")
                .current_route()
        });
        (html, route)
    }

    #[test]
    fn registration_closed_banner_renders_once_after_router_strips_the_query() {
        // The slot is process-wide. Hold a separate lock so a parallel test
        // cannot capture or take it mid-render. Do not hold `LOGIN_BANNER`:
        // capture and take lock that mutex themselves.
        static TEST_LOCK: Mutex<()> = Mutex::new(());
        let _guard = TEST_LOCK.lock().unwrap_or_else(|err| err.into_inner());
        reset_captured_login_banner();

        let original = "/login?error=registration_closed";
        let (html, route) = mount_login(original);
        assert_eq!(
            route, "/login",
            "router must replace the undeclared query before Login paints"
        );
        assert_ne!(route, original);
        assert!(
            html_has_registration_closed_banner(&html),
            "banner must come from the pre-replace snapshot, html={html}"
        );
        assert!(
            html.contains("Sign in"),
            "login form should still render, html={html}"
        );

        // Same process, new Login mount (in-app navigation back to /login).
        let (again, again_route) = mount_login(original);
        assert_eq!(again_route, "/login");
        assert!(
            !html_has_registration_closed_banner(&again),
            "the snapshot is read-once, html={again}"
        );
        assert!(again.contains("Sign in"), "html={again}");

        reset_captured_login_banner();
        let (plain, plain_route) = mount_login("/login");
        assert_eq!(plain_route, "/login");
        assert!(!html_has_registration_closed_banner(&plain), "html={plain}");

        reset_captured_login_banner();
        let (unknown, unknown_route) = mount_login("/login?error=nope");
        assert_eq!(unknown_route, "/login");
        assert!(
            !html_has_registration_closed_banner(&unknown),
            "html={unknown}"
        );
        drop(_guard);
    }
}
