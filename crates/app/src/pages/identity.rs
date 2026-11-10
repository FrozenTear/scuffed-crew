use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use scuffed_api_client::ApiClient;

use crate::components::ui::InlineConfirm;
use crate::components::{Toast, is_http_status, use_toast};
use crate::routes::Route;
use crate::state::auth::use_auth;

/// The one name for this page: the h1, the head title and the account menu.
pub const ACCOUNT_KEYS: &str = "Account keys";

const INTRO: &str = "Your account key is your ID in chat and direct messages. \
    We can keep one for you, or you can bring your own.";
const HINT_PUBLIC_KEY: &str = "Your public key is safe to share. It is how others know it is you.";
const HINT_NIP05: &str = "A NIP-05 name looks like an email address and points to your key.";
const HINT_BACKUP: &str =
    "An ncryptsec backup is your key locked with a password. You need the password to use it.";
const HINT_ADVANCED: &str = "You rarely need these. Importing a key or unlinking changes how \
    chat and direct messages sign you in.";
const HINT_UNLINK: &str = "Chat and direct messages stop working until you link a key again.";

const PAGE_CSS: &str = r#"
    .identity-page {
        padding: 3rem 2rem;
        max-width: 760px;
        margin: 0 auto;
    }
    .identity-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0 0 0.75rem;
    }
    .identity-reports-link {
        margin: 0 0 1.5rem;
    }
    .identity-reports-link a {
        color: var(--accent);
        font-weight: 600;
        font-size: 0.9rem;
        text-decoration: none;
    }
    .identity-section {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 10px;
        padding: 1.5rem;
        margin-bottom: 1.25rem;
    }
    .identity-section h2 {
        font-family: var(--font-head);
        font-size: 1.15rem;
        font-weight: 700;
        color: var(--text);
        margin: 0 0 0.75rem;
    }
    .identity-section p {
        color: var(--text-2);
        font-size: 0.85rem;
        line-height: 1.5;
        margin: 0 0 1rem;
    }
    .identity-field {
        margin-bottom: 1rem;
    }
    .identity-label {
        display: block;
        font-size: 0.75rem;
        font-weight: 600;
        color: var(--text-3);
        text-transform: uppercase;
        letter-spacing: 0.05em;
        margin-bottom: 0.35rem;
    }
    .identity-input {
        width: 100%;
        padding: 0.5rem 0.75rem;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text);
        font-size: 0.875rem;
        transition: border-color 0.15s;
        box-sizing: border-box;
    }
    .identity-input:focus {
        border-color: var(--accent);
    }
    .identity-mono {
        font-family: var(--font-mono);
        font-size: 0.8rem;
        background: var(--bg);
        padding: 0.5rem 0.75rem;
        border-radius: 6px;
        word-break: break-all;
        display: block;
        color: var(--text);
        border: 1px solid var(--border);
    }
    .identity-pill {
        display: inline-block;
        font-size: 0.7rem;
        padding: 0.15rem 0.6rem;
        border-radius: var(--radius-pill);
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.04em;
        background: color-mix(in srgb, var(--accent) 20%, transparent);
        color: var(--accent);
    }
    .identity-nip05 {
        color: var(--text-2);
        font-size: 0.85rem;
    }
    .identity-nip05-unset {
        font-style: italic;
        opacity: 0.75;
    }
    .identity-row {
        display: flex;
        gap: 0.5rem;
        align-items: end;
    }
    .identity-row .identity-field { flex: 1; margin-bottom: 0; }
    .identity-btn {
        padding: 0.5rem 1rem;
        border: none;
        border-radius: 6px;
        font-size: 0.85rem;
        font-weight: 600;
        cursor: pointer;
        transition: filter 0.15s, opacity 0.15s;
    }
    .identity-btn:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }
    .identity-btn-primary {
        background: var(--accent);
        color: var(--accent-fg);
    }
    .identity-btn-primary:hover:not(:disabled) {
        filter: brightness(1.15);
    }
    .identity-btn-danger {
        background: var(--danger);
        color: var(--accent-fg);
    }
    .identity-btn-danger:hover:not(:disabled) {
        filter: brightness(1.15);
    }
    .identity-btn-outline {
        background: transparent;
        color: var(--text-2);
        border: 1px solid var(--border);
    }
    .identity-btn-outline:hover:not(:disabled) {
        border-color: var(--text);
        color: var(--text);
    }
    .identity-backup-box {
        margin-top: 1rem;
        padding: 1rem;
        background: var(--bg);
        border-radius: 8px;
        border: 1px solid var(--border);
    }
    .identity-warning {
        color: var(--warn);
        font-size: 0.75rem;
        margin-top: 0.5rem;
    }
    .identity-actions {
        display: flex;
        gap: 0.5rem;
        margin-top: 0.5rem;
    }
    .identity-section p.identity-hint {
        color: var(--text-3);
        font-size: var(--text-xs);
        margin: 0 0 1rem;
    }
    .identity-intro {
        color: var(--text-2);
        font-size: var(--text-base);
        line-height: 1.5;
        margin: 0 0 1rem;
    }
    .identity-advanced summary {
        cursor: pointer;
        font-family: var(--font-head);
        font-size: 1.15rem;
        font-weight: 700;
        color: var(--text);
    }
    .identity-advanced summary:focus-visible {
        outline: 2px solid var(--accent);
        outline-offset: 2px;
    }
    .identity-advanced[open] summary {
        margin-bottom: 0.75rem;
    }
    .identity-sub {
        border-top: 1px solid var(--border);
        padding-top: 1rem;
        margin-top: 1rem;
    }
    .identity-sub h3 {
        font-family: var(--font-head);
        font-size: var(--text-base);
        color: var(--text);
        margin: 0 0 0.5rem;
    }
    .identity-cta-actions {
        display: flex;
        gap: 0.6rem;
        flex-wrap: wrap;
        margin-top: 0.25rem;
    }
"#;

#[derive(Serialize)]
struct ChallengeBody {
    pubkey: String,
}

#[derive(Deserialize)]
struct ChallengeResponse {
    challenge: String,
    token: String,
    pubkey_hex: String,
    expires_in_secs: u64,
}

#[derive(Serialize)]
struct VerifyBody {
    token: String,
    signed_event: serde_json::Value,
}

#[derive(Serialize)]
struct ExportBackupBody {
    password: String,
}

#[derive(Deserialize)]
struct ExportBackupResponse {
    ncryptsec: String,
}

#[derive(Serialize)]
struct ImportKeyBody {
    ncryptsec: String,
    password: String,
}

fn truncate_pubkey(pk: &str) -> String {
    if pk.len() > 16 {
        format!("{}...{}", &pk[..8], &pk[pk.len() - 8..])
    } else {
        pk.to_string()
    }
}

fn has_nip07_extension() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("nostr")).ok())
        .map(|v| !v.is_undefined() && !v.is_null())
        .unwrap_or(false)
}

/// The collapsed group for rare, riskier actions. Closed until the visitor opens
/// it, so it never carries an `open` attribute of its own.
#[component]
fn AdvancedGroup(children: Element) -> Element {
    rsx! {
        details { class: "identity-section identity-advanced",
            summary { "Advanced" }
            p { class: "identity-hint", "{HINT_ADVANCED}" }
            {children}
        }
    }
}

#[component]
pub fn IdentitySettings() -> Element {
    let mut toast = use_toast();
    let mut refresh = use_signal(|| 0u32);

    let mut pubkey_input = use_signal(String::new);
    let mut backup_password = use_signal(String::new);
    let mut backup_result = use_signal(|| None::<String>);
    let mut import_ncryptsec = use_signal(String::new);
    let mut import_password = use_signal(String::new);
    let mut working = use_signal(|| false);
    let mut confirm_import = use_signal(|| false);
    let mut confirm_unlink = use_signal(|| false);

    let auth = use_auth();
    let identity = use_resource(move || {
        let _ = refresh();
        async move { ApiClient::web().get_me().await }
    });

    let has_extension = has_nip07_extension();

    // ─── NIP-07: Auto-detect pubkey from extension ───
    let on_detect_pubkey = move |_| {
        spawn(async move {
            match get_pubkey_from_extension().await {
                Ok(pk) => pubkey_input.set(pk),
                Err(e) => toast.show(Toast::error(format!("Extension error: {e}"))),
            }
        });
    };

    // ─── NIP-07: Link identity via challenge-response ───
    let on_link_identity = move |_| {
        let pubkey = pubkey_input().trim().to_string();
        if pubkey.is_empty() {
            toast.show(Toast::error("Enter a pubkey first."));
            return;
        }
        working.set(true);
        spawn(async move {
            match link_nostr_identity(&pubkey).await {
                Ok(()) => {
                    toast.show(Toast::success("Key linked."));
                    refresh += 1;
                    pubkey_input.set(String::new());
                }
                Err(e) => toast.show(Toast::error(format!("Link failed: {e}"))),
            }
            working.set(false);
        });
    };

    // ─── NIP-49: Export backup ───
    let on_export_backup = move |_| {
        let password = backup_password().trim().to_string();
        if password.len() < 12 {
            toast.show(Toast::error("Password must be at least 12 characters."));
            return;
        }
        working.set(true);
        spawn(async move {
            let body = ExportBackupBody { password };
            match ApiClient::web()
                .post_json::<_, ExportBackupResponse>("/api/nostr/export-backup", &body)
                .await
            {
                Ok(resp) => {
                    backup_result.set(Some(resp.ncryptsec));
                    backup_password.set(String::new());
                    toast.show(Toast::success("Key backup exported. Copy it now!"));
                }
                Err(e) => toast.show(Toast::error(format!("Export failed: {e}"))),
            }
            working.set(false);
        });
    };

    // ─── NIP-49: Import key ───
    let on_import_key = move |_| {
        let ncryptsec = import_ncryptsec().trim().to_string();
        let password = import_password().trim().to_string();
        if ncryptsec.is_empty() || password.is_empty() {
            toast.show(Toast::error("Both ncryptsec and password are required."));
            return;
        }
        // Importing links an *external* key and replaces any currently linked
        // identity, so ask first with the inline strip (DR1-NOSTR-003).
        confirm_import.set(true);
    };

    let do_import_key = move |_| {
        confirm_import.set(false);
        let ncryptsec = import_ncryptsec().trim().to_string();
        let password = import_password().trim().to_string();
        working.set(true);
        spawn(async move {
            let body = ImportKeyBody {
                ncryptsec,
                password,
            };
            match ApiClient::web()
                .post_json::<_, serde_json::Value>("/api/nostr/import-key", &body)
                .await
            {
                Ok(_) => {
                    toast.show(Toast::success("Key imported successfully!"));
                    refresh += 1;
                    import_ncryptsec.set(String::new());
                    import_password.set(String::new());
                }
                Err(e) => toast.show(Toast::error(format!("Import failed: {e}"))),
            }
            working.set(false);
        });
    };

    // ─── Unlink identity ───
    let do_unlink = move |_| {
        confirm_unlink.set(false);
        working.set(true);
        spawn(async move {
            match ApiClient::web().delete("/api/nostr/identity").await {
                Ok(()) => {
                    toast.show(Toast::success("Key unlinked."));
                    refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Unlink failed: {e}"))),
            }
            working.set(false);
        });
    };

    let me_data = identity.read();
    let me_ok = me_data.as_ref().and_then(|r| r.as_ref().ok());
    let member = me_ok.and_then(|me| me.member.as_ref());

    // Server-computed: `None` when the deploy has no valid public NIP-05
    // domain. Never rebuild this client-side; the client cannot know which
    // domain serves `/.well-known/nostr.json`, and guessing is what published
    // identities pointing at a domain we do not own.
    let nip05 = me_ok.and_then(|me| me.nip05.clone());

    let auth_state = auth();
    let identity_gate: Option<(&str, &str, Route, &str)> = match me_data.as_ref() {
        None if !auth_state.loading && !auth_state.is_logged_in() => Some((
            "Sign in to manage your account keys.",
            "Sign in",
            Route::Login {},
            "ui-btn ui-btn--primary ui-btn--md",
        )),
        None if !auth_state.loading && !auth_state.is_org_member() => Some((
            "You need to be an org member to manage account keys.",
            "Apply",
            Route::Apply {},
            "ui-btn ui-btn--primary ui-btn--md",
        )),
        Some(Err(e)) if is_http_status(&e.to_string(), 401) => Some((
            "Sign in to manage your account keys.",
            "Sign in",
            Route::Login {},
            "ui-btn ui-btn--primary ui-btn--md",
        )),
        Some(Ok(me)) if me.member.is_none() => Some((
            "You need to be an org member to manage account keys.",
            "Apply",
            Route::Apply {},
            "ui-btn ui-btn--primary ui-btn--md",
        )),
        _ => None,
    };
    let identity_fetch_error = me_data
        .as_ref()
        .and_then(|r| r.as_ref().err())
        .and_then(|e| {
            let msg = e.to_string();
            if is_http_status(&msg, 401) {
                None
            } else {
                Some(msg)
            }
        });

    let has_pubkey = member.map(|m| m.nostr_pubkey.is_some()).unwrap_or(false);
    let is_server_managed = member
        .map(|m| m.nostr_key_mode.as_deref() == Some("server_managed"))
        .unwrap_or(false);

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-identity-page-css", css: PAGE_CSS }

        main { class: "identity-page",
            h1 { class: "identity-page-title", "{ACCOUNT_KEYS}" }
            p { class: "identity-intro", "{INTRO}" }
            p { class: "identity-reports-link",
                Link { to: Route::MyReports {}, "My reports" }
            }

            div { class: "identity-section",
                h2 { "Your key" }
                if let Some(m) = member {
                    if let Some(ref pubkey) = m.nostr_pubkey {
                        div { class: "identity-field",
                            label { class: "identity-label", "Public key" }
                            code { class: "identity-mono", "{pubkey}" }
                        }
                        p { class: "identity-hint", "{HINT_PUBLIC_KEY}" }
                        div { class: "identity-row",
                            div { class: "identity-field",
                                label { class: "identity-label", "Key mode" }
                                span { class: "identity-pill",
                                    {m.nostr_key_mode.as_deref().unwrap_or("unknown")}
                                }
                            }
                            div { class: "identity-field",
                                label { class: "identity-label", "NIP-05" }
                                if let Some(ref id) = nip05 {
                                    span { class: "identity-nip05", "{id}" }
                                } else {
                                    span { class: "identity-nip05 identity-nip05-unset",
                                        "Not configured"
                                    }
                                }
                            }
                        }
                        p { class: "identity-hint", "{HINT_NIP05}" }
                    } else {
                        p { "No key is linked yet. Use one of the options below to get started." }
                    }
                } else if let Some((lead, label, route, btn_class)) = identity_gate {
                    p { "{lead}" }
                    div { class: "identity-cta-actions",
                        Link { to: route, class: "{btn_class}", "{label}" }
                    }
                } else if let Some(err) = identity_fetch_error {
                    p { "Couldn't load identity: {err}" }
                } else {
                    p { "Loading..." }
                }
            }

            if member.is_some() && !has_pubkey {
                div { class: "identity-section",
                    h2 { "Link a key" }
                    if has_extension {
                        div { class: "identity-row",
                            div { class: "identity-field",
                                label { class: "identity-label", "Public key (hex or npub)" }
                                input {
                                    class: "identity-input",
                                    r#type: "text",
                                    placeholder: "64-char hex or npub1...",
                                    value: "{pubkey_input}",
                                    oninput: move |e| pubkey_input.set(e.value()),
                                }
                            }
                            button {
                                class: "identity-btn identity-btn-outline",
                                onclick: on_detect_pubkey,
                                "Detect"
                            }
                        }
                        div { class: "identity-actions",
                            button {
                                class: "identity-btn identity-btn-primary",
                                disabled: working() || pubkey_input().trim().is_empty(),
                                onclick: on_link_identity,
                                if working() { "Linking..." } else { "Link & Verify" }
                            }
                        }
                    } else {
                        p { "Install a NIP-07 browser extension (nos2x, Alby, or similar) to link your key." }
                    }
                }
            }

            if member.is_some() && is_server_managed {
                div { class: "identity-section",
                    h2 { "Back up your key" }
                    p { "Export your key as an encrypted ncryptsec backup. Keep this password. You will need it to import the key later (which links it as an external identity)." }
                    p { class: "identity-hint", "{HINT_BACKUP}" }
                    div { class: "identity-field",
                        label { class: "identity-label", "Backup Password (min 12 chars)" }
                        input {
                            class: "identity-input",
                            r#type: "password",
                            placeholder: "Enter a strong password (12+ characters)",
                            value: "{backup_password}",
                            oninput: move |e| backup_password.set(e.value()),
                        }
                    }
                    div { class: "identity-actions",
                        button {
                            class: "identity-btn identity-btn-primary",
                            disabled: working() || backup_password().trim().len() < 12,
                            onclick: on_export_backup,
                            if working() { "Exporting..." } else { "Export Backup" }
                        }
                    }
                    if let Some(ref ncryptsec) = *backup_result.read() {
                        div { class: "identity-backup-box",
                            label { class: "identity-label", "Your ncryptsec backup:" }
                            code { class: "identity-mono", "{ncryptsec}" }
                            p { class: "identity-warning",
                                "Copy this now! It will not be shown again."
                            }
                        }
                    }
                }
            }

            if member.is_some() {
                AdvancedGroup {
                if is_server_managed {
                    div { class: "identity-sub",
                        h3 { "Import an external key" }
                        p { class: "identity-warning",
                            "You currently have a server-managed key. Importing an external key is \
                             disabled to protect it. To switch to an external key, first unlink your \
                             key below, then import. Importing does not restore a key into \
                             server-managed mode."
                        }
                    }
                } else {
                    div { class: "identity-sub",
                        h3 { "Import an external key" }
                        p {
                            "Link an external Nostr key from an ncryptsec backup. This sets your \
                             identity to external mode. The server will not manage a key for you. \
                             It replaces any currently linked external key. The nsec is decrypted \
                             only to derive your public key."
                        }
                        div { class: "identity-field",
                            label { class: "identity-label", "ncryptsec" }
                            input {
                                class: "identity-input",
                                r#type: "text",
                                placeholder: "ncryptsec1...",
                                value: "{import_ncryptsec}",
                                oninput: move |e| import_ncryptsec.set(e.value()),
                            }
                        }
                        div { class: "identity-field",
                            label { class: "identity-label", "Password" }
                            input {
                                class: "identity-input",
                                r#type: "password",
                                placeholder: "Backup password",
                                value: "{import_password}",
                                oninput: move |e| import_password.set(e.value()),
                            }
                        }
                        if confirm_import() {
                            InlineConfirm {
                                id: "import-key".to_string(),
                                title: "Replace your linked key?".to_string(),
                                body: "Importing sets your key to one you manage yourself. The server will no longer manage a key for you, and any key linked now is replaced.".to_string(),
                                confirm_label: "Import key".to_string(),
                                busy: working(),
                                on_confirm: do_import_key,
                                on_cancel: move |_| confirm_import.set(false),
                            }
                        } else {
                            div { class: "identity-actions",
                                button {
                                    id: "import-key-trigger",
                                    class: "identity-btn identity-btn-primary",
                                    disabled: working() || import_ncryptsec().trim().is_empty() || import_password().trim().is_empty(),
                                    onclick: on_import_key,
                                    if working() { "Importing..." } else { "Import Key" }
                                }
                            }
                        }
                    }
                }
                    if has_pubkey {
                        div { class: "identity-sub",
                            h3 { "Unlink your key" }
                            p { class: "identity-hint", "{HINT_UNLINK}" }
                        if confirm_unlink() {
                            InlineConfirm {
                                id: "unlink-identity".to_string(),
                                title: "Unlink this key?".to_string(),
                                body: "This removes the key from your account. Export a backup first if you want to keep it.".to_string(),
                                confirm_label: "Unlink".to_string(),
                                busy: working(),
                                on_confirm: do_unlink,
                                on_cancel: move |_| confirm_unlink.set(false),
                            }
                        } else {
                            div { class: "identity-actions",
                                button {
                                    id: "unlink-identity-trigger",
                                    class: "identity-btn identity-btn-danger",
                                    disabled: working(),
                                    onclick: move |_| confirm_unlink.set(true),
                                    "Unlink key"
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

/// Fetch pubkey from NIP-07 browser extension via `window.nostr.getPublicKey()`.
async fn get_pubkey_from_extension() -> Result<String, String> {
    let window = web_sys::window().ok_or("No window")?;
    let nostr = js_sys::Reflect::get(&window, &JsValue::from_str("nostr"))
        .map_err(|_| "No window.nostr")?;
    if nostr.is_undefined() || nostr.is_null() {
        return Err("NIP-07 extension not found".into());
    }

    let get_pk_fn = js_sys::Reflect::get(&nostr, &JsValue::from_str("getPublicKey"))
        .map_err(|_| "getPublicKey not found")?;
    let get_pk_fn: js_sys::Function = get_pk_fn
        .dyn_into()
        .map_err(|_| "getPublicKey is not a function")?;

    let promise = get_pk_fn
        .call0(&nostr)
        .map_err(|e| format!("getPublicKey call failed: {e:?}"))?;
    let promise: js_sys::Promise = promise
        .dyn_into()
        .map_err(|_| "getPublicKey did not return a promise")?;

    let result = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("getPublicKey rejected: {e:?}"))?;

    result
        .as_string()
        .ok_or_else(|| "getPublicKey did not return a string".into())
}

/// Full NIP-07 challenge-response flow to link a Nostr identity.
async fn link_nostr_identity(pubkey: &str) -> Result<(), String> {
    let client = ApiClient::web();

    // 1. Request challenge
    let challenge_resp = client
        .post_json::<_, ChallengeResponse>(
            "/api/nostr/challenge",
            &ChallengeBody {
                pubkey: pubkey.to_string(),
            },
        )
        .await
        .map_err(|e| format!("Challenge request failed: {e}"))?;

    // 2. Build unsigned event for NIP-07 to sign
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
        &JsValue::from_str(&challenge_resp.challenge),
    )
    .map_err(|_| "Failed to set content")?;

    let created_at = (chrono::Utc::now().timestamp()) as f64;
    js_sys::Reflect::set(
        &event_obj,
        &JsValue::from_str("created_at"),
        &JsValue::from_f64(created_at),
    )
    .map_err(|_| "Failed to set created_at")?;

    let tags = js_sys::Array::new();
    js_sys::Reflect::set(&event_obj, &JsValue::from_str("tags"), &tags)
        .map_err(|_| "Failed to set tags")?;

    // 3. Sign via NIP-07 extension
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

    // 4. Convert signed event to JSON
    let signed_json: serde_json::Value =
        serde_wasm_bindgen::from_value(signed_event).map_err(|e| format!("Parse error: {e}"))?;

    // 5. Verify with server
    let body = VerifyBody {
        token: challenge_resp.token,
        signed_event: signed_json,
    };
    client
        .post_json::<_, serde_json::Value>("/api/nostr/verify", &body)
        .await
        .map_err(|e| format!("Verification failed: {e}"))?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn page_is_called_account_keys_everywhere() {
        assert_eq!(ACCOUNT_KEYS, "Account keys");
        assert!(!ACCOUNT_KEYS.contains("Nostr"));
    }

    #[test]
    fn advanced_group_starts_collapsed_and_holds_its_children() {
        fn root() -> Element {
            rsx! {
                AdvancedGroup { p { id: "inside", "Unlink your key" } }
            }
        }
        let html = render(root);
        assert!(html.contains("<details"), "{html}");
        assert!(html.contains("<summary>Advanced</summary>"), "{html}");
        assert!(html.contains("Unlink your key"), "{html}");
        assert!(!html.contains(" open"), "must start collapsed: {html}");
    }

    #[test]
    fn explainers_are_short_plain_sentences() {
        for text in [
            INTRO,
            HINT_PUBLIC_KEY,
            HINT_NIP05,
            HINT_BACKUP,
            HINT_ADVANCED,
            HINT_UNLINK,
        ] {
            assert!(text.ends_with('.'), "{text}");
            assert!(text.len() < 160, "{text}");
            assert!(
                !text.contains('\u{2014}') && !text.contains('\u{2013}'),
                "{text}"
            );
        }
    }

    #[test]
    fn unlink_hint_does_not_repeat_the_confirm_text() {
        let src = include_str!("identity.rs");
        assert!(!HINT_UNLINK.contains("removes the key"), "{HINT_UNLINK}");
        assert!(!HINT_UNLINK.contains("Export a backup"), "{HINT_UNLINK}");
        assert!(!src.contains(&["Unlink", " Identity"].concat()));
    }
}
