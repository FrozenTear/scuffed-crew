use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use scuffed_api_client::ApiClient;
use scuffed_types::MeResponse;

use crate::components::{Toast, use_toast};

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
        if password.len() < 8 {
            toast.show(Toast::error("Password must be at least 8 characters."));
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

    let has_pubkey = member
        .map(|m| m.nostr_pubkey.is_some())
        .unwrap_or(false);
    let is_server_managed = member
        .map(|m| m.nostr_key_mode.as_deref() == Some("server_managed"))
        .unwrap_or(false);

    rsx! {
        div { class: "admin-toolbar",
            h1 { "Nostr Identity" }
        }

        // ─── Current Identity Status ───
        div { class: "form-section",
            h2 { "Identity Status" }
            if let Some(m) = member {
                if let Some(ref pubkey) = m.nostr_pubkey {
                    div { class: "form-inline",
                        div { class: "form-field",
                            label { class: "form-label", "Public Key" }
                            code { class: "form-input",
                                style: "font-family: monospace; background: var(--surface-2); padding: 0.5rem; border-radius: 4px; display: block; word-break: break-all;",
                                "{pubkey}"
                            }
                            small {
                                style: "color: var(--text-muted);",
                                "Truncated: {truncate_pubkey(pubkey)}"
                            }
                        }
                        div { class: "form-field",
                            label { class: "form-label", "Key Mode" }
                            span {
                                style: "padding: 0.25rem 0.75rem; border-radius: 4px; background: var(--surface-2); font-size: 0.875rem;",
                                {m.nostr_key_mode.as_deref().unwrap_or("unknown")}
                            }
                        }
                        div { class: "form-field",
                            label { class: "form-label", "NIP-05" }
                            span {
                                style: "color: var(--text-secondary);",
                                "{nip05_name(&m.display_name)}@scuffed.gg"
                            }
                        }
                    }
                    div { class: "form-inline",
                        button {
                            class: "btn-danger",
                            disabled: working(),
                            onclick: on_unlink,
                            "Unlink Identity"
                        }
                    }
                } else {
                    p { style: "color: var(--text-muted);", "No Nostr identity linked." }
                }
            } else {
                p { style: "color: var(--text-muted);", "Loading..." }
            }
        }

        // ─── NIP-07: Link via Extension ───
        if !has_pubkey {
            div { class: "form-section",
                h2 { "Link Nostr Identity" }
                if has_extension {
                    div { class: "form-inline",
                        div { class: "form-field",
                            label { class: "form-label", "Public Key (hex or npub)" }
                            div { style: "display: flex; gap: 0.5rem;",
                                input {
                                    class: "form-input",
                                    r#type: "text",
                                    placeholder: "64-char hex or npub1...",
                                    value: "{pubkey_input}",
                                    oninput: move |e| pubkey_input.set(e.value()),
                                }
                                button {
                                    class: "btn-add",
                                    onclick: on_detect_pubkey,
                                    "Detect (NIP-07)"
                                }
                            }
                        }
                    }
                    div { class: "form-inline",
                        button {
                            class: "btn-add",
                            disabled: working() || pubkey_input().trim().is_empty(),
                            onclick: on_link_identity,
                            if working() { "Linking..." } else { "Link & Verify" }
                        }
                    }
                } else {
                    p {
                        style: "color: var(--text-muted);",
                        "Install a NIP-07 browser extension (nos2x, Alby, or similar) to link your Nostr identity."
                    }
                }
            }
        }

        // ─── NIP-49: Key Backup ───
        if is_server_managed {
            div { class: "form-section",
                h2 { "Key Backup (NIP-49)" }
                p {
                    style: "color: var(--text-muted); font-size: 0.875rem; margin-bottom: 1rem;",
                    "Export your key as an encrypted ncryptsec backup. You will need the password to restore it."
                }
                div { class: "form-inline",
                    div { class: "form-field",
                        label { class: "form-label", "Backup Password (min 8 chars)" }
                        input {
                            class: "form-input",
                            r#type: "password",
                            placeholder: "Enter a strong password",
                            value: "{backup_password}",
                            oninput: move |e| backup_password.set(e.value()),
                        }
                    }
                }
                div { class: "form-inline",
                    button {
                        class: "btn-add",
                        disabled: working() || backup_password().trim().len() < 8,
                        onclick: on_export_backup,
                        if working() { "Exporting..." } else { "Export Backup" }
                    }
                }
                if let Some(ref ncryptsec) = *backup_result.read() {
                    div {
                        style: "margin-top: 1rem; padding: 1rem; background: var(--surface-2); border-radius: 8px; border: 1px solid var(--border);",
                        label { class: "form-label", "Your ncryptsec backup:" }
                        code {
                            style: "display: block; word-break: break-all; font-size: 0.75rem; padding: 0.5rem; background: var(--surface-1); border-radius: 4px; font-family: monospace;",
                            "{ncryptsec}"
                        }
                        p {
                            style: "color: var(--warning); font-size: 0.75rem; margin-top: 0.5rem;",
                            "Copy this now! It will not be shown again."
                        }
                    }
                }
            }
        }

        // ─── NIP-49: Import Key ───
        div { class: "form-section",
            h2 { "Import Key (NIP-49)" }
            p {
                style: "color: var(--text-muted); font-size: 0.875rem; margin-bottom: 1rem;",
                "Restore a key from an ncryptsec backup."
            }
            div { class: "form-inline",
                div { class: "form-field",
                    label { class: "form-label", "ncryptsec" }
                    input {
                        class: "form-input",
                        r#type: "text",
                        placeholder: "ncryptsec1...",
                        value: "{import_ncryptsec}",
                        oninput: move |e| import_ncryptsec.set(e.value()),
                    }
                }
                div { class: "form-field",
                    label { class: "form-label", "Password" }
                    input {
                        class: "form-input",
                        r#type: "password",
                        placeholder: "Backup password",
                        value: "{import_password}",
                        oninput: move |e| import_password.set(e.value()),
                    }
                }
            }
            div { class: "form-inline",
                button {
                    class: "btn-add",
                    disabled: working() || import_ncryptsec().trim().is_empty() || import_password().trim().is_empty(),
                    onclick: on_import_key,
                    if working() { "Importing..." } else { "Import Key" }
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
