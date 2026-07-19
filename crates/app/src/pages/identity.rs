use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use scuffed_api_client::ApiClient;

use crate::components::{Toast, use_toast};

const PAGE_CSS: &str = r#"
    .identity-page {
        padding: 3rem 2rem;
        max-width: 640px;
        margin: 0 auto;
    }
    .identity-page-title {
        font-family: var(--font-head);
        font-size: 2.5rem;
        color: var(--text);
        letter-spacing: 3px;
        margin: 0 0 2rem;
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
        outline: none;
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

fn nip05_name(display_name: &str) -> String {
    display_name
        .to_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '_')
        .collect()
}

fn has_nip07_extension() -> bool {
    web_sys::window()
        .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("nostr")).ok())
        .map(|v| !v.is_undefined() && !v.is_null())
        .unwrap_or(false)
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

    let identity = use_resource(move || {
        let _ = refresh();
        async move { ApiClient::web().get_me().await.ok() }
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
                    toast.show(Toast::success("Nostr identity linked!"));
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
        // identity — confirm before the mode change (DR1-NOSTR-003).
        let confirmed = web_sys::window()
            .and_then(|w| {
                w.confirm_with_message(
                    "Importing this key sets your identity to EXTERNAL mode and replaces any \
                     currently linked Nostr key. The server will no longer manage a key for you. \
                     Continue?",
                )
                .ok()
            })
            .unwrap_or(false);
        if !confirmed {
            return;
        }
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
    let on_unlink = move |_| {
        working.set(true);
        spawn(async move {
            match ApiClient::web().delete("/api/nostr/identity").await {
                Ok(()) => {
                    toast.show(Toast::success("Nostr identity unlinked."));
                    refresh += 1;
                }
                Err(e) => toast.show(Toast::error(format!("Unlink failed: {e}"))),
            }
            working.set(false);
        });
    };

    let me_data = identity.read();
    let member = me_data
        .as_ref()
        .and_then(|o| o.as_ref())
        .and_then(|me| me.member.as_ref());

    let has_pubkey = member.map(|m| m.nostr_pubkey.is_some()).unwrap_or(false);
    let is_server_managed = member
        .map(|m| m.nostr_key_mode.as_deref() == Some("server_managed"))
        .unwrap_or(false);

    rsx! {
        style { {PAGE_CSS} }

        main { class: "identity-page",
            h1 { class: "identity-page-title", "Nostr Identity" }

            div { class: "identity-section",
                h2 { "Identity Status" }
                if let Some(m) = member {
                    if let Some(ref pubkey) = m.nostr_pubkey {
                        div { class: "identity-field",
                            label { class: "identity-label", "Public Key" }
                            code { class: "identity-mono", "{pubkey}" }
                        }
                        div { class: "identity-row",
                            div { class: "identity-field",
                                label { class: "identity-label", "Key Mode" }
                                span { class: "identity-pill",
                                    {m.nostr_key_mode.as_deref().unwrap_or("unknown")}
                                }
                            }
                            div { class: "identity-field",
                                label { class: "identity-label", "NIP-05" }
                                span { class: "identity-nip05",
                                    "{nip05_name(&m.display_name)}@scuffed.gg"
                                }
                            }
                        }
                        div { class: "identity-actions",
                            button {
                                class: "identity-btn identity-btn-danger",
                                disabled: working(),
                                onclick: on_unlink,
                                "Unlink Identity"
                            }
                        }
                    } else {
                        p { "No Nostr identity linked. Use one of the options below to get started." }
                    }
                } else {
                    p { "Loading..." }
                }
            }

            if !has_pubkey {
                div { class: "identity-section",
                    h2 { "Link Nostr Identity" }
                    if has_extension {
                        div { class: "identity-row",
                            div { class: "identity-field",
                                label { class: "identity-label", "Public Key (hex or npub)" }
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
                        p { "Install a NIP-07 browser extension (nos2x, Alby, or similar) to link your Nostr identity." }
                    }
                }
            }

            if is_server_managed {
                div { class: "identity-section",
                    h2 { "Key Backup (NIP-49)" }
                    p { "Export your key as an encrypted ncryptsec backup. Keep this password — you will need it to import the key later (which links it as an external identity)." }
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

            if is_server_managed {
                div { class: "identity-section",
                    h2 { "Import External Key (NIP-49)" }
                    p { class: "identity-warning",
                        "You currently have a server-managed key. Importing an external key is \
                         disabled to protect it — to switch to an external key, first Unlink your \
                         identity above, then import. Importing does not restore a key into \
                         server-managed mode."
                    }
                }
            } else {
                div { class: "identity-section",
                    h2 { "Import External Key (NIP-49)" }
                    p {
                        "Link an external Nostr key from an ncryptsec backup. This sets your \
                         identity to external mode — the server will not manage a key for you — \
                         and replaces any currently linked external key. The nsec is decrypted \
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
                    div { class: "identity-actions",
                        button {
                            class: "identity-btn identity-btn-primary",
                            disabled: working() || import_ncryptsec().trim().is_empty() || import_password().trim().is_empty(),
                            onclick: on_import_key,
                            if working() { "Importing..." } else { "Import Key" }
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
