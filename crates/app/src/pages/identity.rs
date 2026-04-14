use dioxus::prelude::*;
use serde::{Deserialize, Serialize};
use wasm_bindgen::prelude::*;
use wasm_bindgen::JsCast;

use scuffed_api_client::ApiClient;
use crate::components::{Toast, use_toast, ConfirmDialog};
use crate::state::auth::use_auth;

#[derive(Clone, Debug, Deserialize)]
struct NostrIdentity {
    pubkey: String,
    linked_at: String,
}

#[derive(Clone, Debug, Deserialize)]
struct NostrChallenge {
    #[allow(dead_code)]
    challenge: String,
    event: ChallengeEvent,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
struct ChallengeEvent {
    kind: u32,
    created_at: u64,
    tags: Vec<Vec<String>>,
    content: String,
}

#[derive(Clone, Debug, Serialize)]
struct LinkRequest {
    signed_event: String,
}

// -- NIP-07 JS interop --

fn has_nip07() -> bool {
    let Some(window) = web_sys::window() else { return false };
    js_sys::Reflect::get(&window, &JsValue::from_str("nostr"))
        .map(|v| !v.is_undefined() && !v.is_null())
        .unwrap_or(false)
}

fn get_nostr_obj() -> Result<JsValue, String> {
    let window = web_sys::window().ok_or("No window object")?;
    let nostr = js_sys::Reflect::get(&window, &JsValue::from_str("nostr"))
        .map_err(|_| "Cannot access window.nostr")?;
    if nostr.is_undefined() || nostr.is_null() {
        return Err("NIP-07 extension not found. Install a Nostr signing extension (nos2x, Alby, etc.)".into());
    }
    Ok(nostr)
}

async fn nip07_get_public_key() -> Result<String, String> {
    let nostr = get_nostr_obj()?;
    let func: js_sys::Function = js_sys::Reflect::get(&nostr, &JsValue::from_str("getPublicKey"))
        .map_err(|_| "getPublicKey not available on extension")?
        .dyn_into()
        .map_err(|_| "getPublicKey is not a function")?;
    let promise: js_sys::Promise = func
        .call0(&nostr)
        .map_err(|_| "Failed to call getPublicKey")?
        .dyn_into()
        .map_err(|_| "getPublicKey did not return a Promise")?;
    let result = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| "Permission denied — you rejected the key request in your extension".to_string())?;
    result.as_string().ok_or("Extension returned invalid public key format".into())
}

async fn nip07_sign_event(event: &ChallengeEvent) -> Result<String, String> {
    let nostr = get_nostr_obj()?;
    let func: js_sys::Function = js_sys::Reflect::get(&nostr, &JsValue::from_str("signEvent"))
        .map_err(|_| "signEvent not available on extension")?
        .dyn_into()
        .map_err(|_| "signEvent is not a function")?;
    let event_json = serde_json::to_string(event).map_err(|e| format!("Serialize error: {e}"))?;
    let event_js = js_sys::JSON::parse(&event_json).map_err(|_| "Failed to parse event for signing")?;
    let promise: js_sys::Promise = func
        .call1(&nostr, &event_js)
        .map_err(|_| "Failed to call signEvent")?
        .dyn_into()
        .map_err(|_| "signEvent did not return a Promise")?;
    let result = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| "Signing denied — you rejected the signature request in your extension".to_string())?;
    let json = js_sys::JSON::stringify(&result)
        .map_err(|_| "Failed to serialize signed event")?;
    json.as_string().ok_or("Invalid signed event format".into())
}

async fn clipboard_write(text: &str) {
    let Some(window) = web_sys::window() else { return };
    let Ok(nav) = js_sys::Reflect::get(&window, &JsValue::from_str("navigator")) else { return };
    let Ok(clip) = js_sys::Reflect::get(&nav, &JsValue::from_str("clipboard")) else { return };
    let Ok(func) = js_sys::Reflect::get(&clip, &JsValue::from_str("writeText")) else { return };
    let Ok(func): Result<js_sys::Function, _> = func.dyn_into() else { return };
    let Ok(promise): Result<js_sys::Promise, _> = func
        .call1(&clip, &JsValue::from_str(text))
        .and_then(|v| v.dyn_into()) else { return };
    let _ = wasm_bindgen_futures::JsFuture::from(promise).await;
}

// -- Bech32 npub encoding --

const BECH32_CHARSET: &[u8] = b"qpzry9x8gf2tvdw0s3jn54khce6mua7l";

fn bech32_polymod(values: &[u32]) -> u32 {
    const GEN: [u32; 5] = [0x3b6a57b2, 0x26508e6d, 0x1ea119fa, 0x3d4233dd, 0x2a1462b3];
    let mut chk: u32 = 1;
    for &v in values {
        let b = chk >> 25;
        chk = ((chk & 0x1ffffff) << 5) ^ v;
        for (i, g) in GEN.iter().enumerate() {
            if (b >> i) & 1 == 1 {
                chk ^= g;
            }
        }
    }
    chk
}

fn bech32_hrp_expand(hrp: &str) -> Vec<u32> {
    let mut v: Vec<u32> = hrp.chars().map(|c| (c as u32) >> 5).collect();
    v.push(0);
    v.extend(hrp.chars().map(|c| (c as u32) & 31));
    v
}

fn convert_bits_8_to_5(data: &[u8]) -> Vec<u8> {
    let mut acc: u32 = 0;
    let mut bits: u32 = 0;
    let mut result = Vec::new();
    for &value in data {
        acc = (acc << 8) | (value as u32);
        bits += 8;
        while bits >= 5 {
            bits -= 5;
            result.push(((acc >> bits) & 31) as u8);
        }
    }
    if bits > 0 {
        result.push(((acc << (5 - bits)) & 31) as u8);
    }
    result
}

fn hex_to_npub(hex: &str) -> Result<String, String> {
    let bytes: Vec<u8> = (0..hex.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&hex[i..i.saturating_add(2)], 16))
        .collect::<Result<_, _>>()
        .map_err(|_| "Invalid hex pubkey")?;

    if bytes.len() != 32 {
        return Err("Public key must be 32 bytes".into());
    }

    let data5 = convert_bits_8_to_5(&bytes);
    let mut values = bech32_hrp_expand("npub");
    values.extend(data5.iter().map(|&d| d as u32));
    values.extend_from_slice(&[0, 0, 0, 0, 0, 0]);
    let polymod = bech32_polymod(&values) ^ 1;
    let checksum: Vec<u8> = (0..6).map(|i| ((polymod >> (5 * (5 - i))) & 31) as u8).collect();

    let mut result = String::from("npub1");
    for d in data5.iter().chain(checksum.iter()) {
        result.push(BECH32_CHARSET[*d as usize] as char);
    }
    Ok(result)
}

// -- Page CSS --

const IDENTITY_CSS: &str = r#"
    .identity-page {
        min-height: 100vh;
        padding: 2rem;
        max-width: 640px;
        margin: 0 auto;
    }
    .identity-title {
        font-family: 'Bebas Neue', sans-serif;
        font-size: 2.5rem;
        color: var(--text-bright);
        letter-spacing: 3px;
        text-align: center;
        margin-bottom: 0.5rem;
    }
    .identity-subtitle {
        text-align: center;
        color: var(--text-secondary);
        font-size: 0.9rem;
        margin-bottom: 2rem;
    }
    .identity-card {
        background: var(--bg-card);
        border: 1px solid var(--border);
        border-radius: 12px;
        padding: 2rem;
        margin-bottom: 1.5rem;
    }
    .identity-card-title {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 700;
        font-size: 1.3rem;
        color: var(--text-bright);
        margin: 0 0 0.75rem;
        display: flex;
        align-items: center;
        gap: 0.5rem;
    }
    .identity-card-desc {
        color: var(--text-secondary);
        font-size: 0.9rem;
        line-height: 1.6;
    }
    .identity-status {
        display: inline-flex;
        align-items: center;
        gap: 0.4rem;
        padding: 0.2rem 0.75rem;
        border-radius: 999px;
        font-size: 0.75rem;
        font-weight: 700;
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .identity-status.linked {
        background: #10b98133;
        color: #34d399;
    }
    .identity-status.not-linked {
        background: #6b728033;
        color: #9ca3af;
    }
    .identity-status.no-extension {
        background: #f59e0b33;
        color: #fbbf24;
    }

    /* Key display */
    .key-display {
        margin-top: 1.25rem;
    }
    .key-row {
        display: flex;
        align-items: center;
        gap: 0.5rem;
        margin-bottom: 0.75rem;
    }
    .key-label {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 600;
        font-size: 0.75rem;
        color: var(--text-muted);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        min-width: 3.5rem;
    }
    .key-value {
        flex: 1;
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 6px;
        padding: 0.5rem 0.75rem;
        color: var(--text-primary);
        font-family: var(--font-mono);
        font-size: 0.8rem;
        word-break: break-all;
        line-height: 1.4;
    }
    .key-copy {
        background: var(--bg-elevated);
        border: 1px solid var(--border);
        border-radius: 6px;
        color: var(--text-secondary);
        padding: 0.5rem 0.6rem;
        cursor: pointer;
        font-size: 0.8rem;
        transition: all 0.15s;
        flex-shrink: 0;
    }
    .key-copy:hover {
        color: var(--text-bright);
        border-color: var(--accent);
        background: var(--accent-soft);
    }
    .key-copy.copied {
        color: #34d399;
        border-color: #10b98155;
    }
    .key-linked-at {
        color: var(--text-muted);
        font-size: 0.8rem;
        margin-top: 0.5rem;
    }

    /* Buttons */
    .identity-btn {
        display: inline-flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.6rem 1.4rem;
        border-radius: 6px;
        font-size: 0.9rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        border: none;
        cursor: pointer;
        transition: all 0.2s;
    }
    .identity-btn:disabled {
        opacity: 0.5;
        cursor: not-allowed;
    }
    .identity-btn-primary {
        background: var(--accent);
        color: white;
    }
    .identity-btn-primary:hover:not(:disabled) {
        filter: brightness(1.15);
        box-shadow: 0 0 20px var(--accent-glow);
    }
    .identity-btn-danger {
        background: transparent;
        border: 1px solid #ef444466;
        color: #f87171;
    }
    .identity-btn-danger:hover:not(:disabled) {
        background: #ef444422;
        border-color: #f87171;
    }
    .identity-actions {
        margin-top: 1.5rem;
        display: flex;
        gap: 0.75rem;
        flex-wrap: wrap;
    }

    /* Spinner */
    .identity-spinner {
        display: inline-block;
        width: 16px;
        height: 16px;
        border: 2px solid var(--text-muted);
        border-top-color: var(--accent);
        border-radius: 50%;
        animation: identity-spin 0.6s linear infinite;
    }
    @keyframes identity-spin {
        to { transform: rotate(360deg); }
    }
    .identity-loading-row {
        display: flex;
        align-items: center;
        gap: 0.75rem;
        color: var(--text-secondary);
        font-size: 0.9rem;
        padding: 1rem 0;
    }

    /* Error */
    .identity-error {
        background: #ef444418;
        border: 1px solid #ef444444;
        border-radius: 8px;
        padding: 1rem;
        color: #fca5a5;
        font-size: 0.85rem;
        line-height: 1.5;
        margin-top: 1rem;
    }
    .identity-error-title {
        font-weight: 700;
        color: #f87171;
        margin-bottom: 0.25rem;
    }

    /* Extension help */
    .extension-help {
        margin-top: 1rem;
        padding: 1rem;
        background: var(--bg-surface);
        border: 1px solid var(--border);
        border-radius: 8px;
    }
    .extension-help-title {
        font-family: 'Rajdhani', sans-serif;
        font-weight: 600;
        font-size: 0.9rem;
        color: var(--text-bright);
        margin-bottom: 0.5rem;
    }
    .extension-help ul {
        list-style: none;
        padding: 0;
        margin: 0;
    }
    .extension-help li {
        color: var(--text-secondary);
        font-size: 0.85rem;
        padding: 0.25rem 0;
    }
    .extension-help li::before {
        content: "\2192 ";
        color: var(--accent);
    }

    /* Auth prompt */
    .identity-auth-prompt {
        text-align: center;
        padding: 1rem 0;
    }
    .identity-auth-prompt a {
        display: inline-flex;
        align-items: center;
        gap: 0.5rem;
        padding: 0.6rem 1.4rem;
        border-radius: 6px;
        font-size: 0.9rem;
        font-weight: 600;
        text-transform: uppercase;
        letter-spacing: 0.03em;
        text-decoration: none;
        transition: all 0.2s;
        background: var(--accent);
        color: white;
    }
    .identity-auth-prompt a:hover {
        filter: brightness(1.15);
    }

    /* Loading page state */
    .identity-loading {
        color: var(--text-muted);
        text-align: center;
        padding: 3rem;
    }

    @media (max-width: 640px) {
        .identity-page { padding: 1rem; }
        .identity-title { font-size: 2rem; }
        .identity-card { padding: 1.25rem; }
        .key-row { flex-direction: column; align-items: stretch; }
        .key-label { min-width: unset; }
        .key-copy { align-self: flex-end; }
        .identity-actions { flex-direction: column; }
        .identity-btn { justify-content: center; }
    }
"#;

#[derive(Clone, PartialEq)]
enum LinkState {
    Idle,
    DetectingExtension,
    GettingKey,
    RequestingChallenge,
    WaitingForSignature,
    SubmittingLink,
    Unlinking,
}

#[component]
pub fn Identity() -> Element {
    let auth = use_auth();
    let mut extension_available = use_signal(|| Option::<bool>::None);
    let mut linked_identity = use_signal(|| Option::<Option<NostrIdentity>>::None);
    let mut link_state = use_signal(|| LinkState::Idle);
    let mut error_msg = use_signal(|| Option::<String>::None);
    let mut confirm_unlink = use_signal(|| false);
    let copied_field = use_signal(|| Option::<String>::None);

    use_future(move || async move {
        let ext = has_nip07();
        extension_available.set(Some(ext));

        match ApiClient::web().fetch::<Option<NostrIdentity>>("/api/identity/nostr").await {
            Ok(identity) => linked_identity.set(Some(identity)),
            Err(scuffed_api_client::ClientError::Http { status: 404, .. }) => {
                linked_identity.set(Some(None));
            }
            Err(_) => linked_identity.set(Some(None)),
        }
    });

    let do_link = move |_| {
        error_msg.set(None);
        link_state.set(LinkState::DetectingExtension);

        spawn(async move {
            let result: Result<(), String> = async {
                if !has_nip07() {
                    return Err("NIP-07 extension not found. Install a Nostr signing extension and reload this page.".into());
                }

                link_state.set(LinkState::GettingKey);
                let _pubkey = nip07_get_public_key().await?;

                link_state.set(LinkState::RequestingChallenge);
                let challenge: NostrChallenge = ApiClient::web()
                    .post_json("/api/identity/nostr/challenge", &serde_json::json!({}))
                    .await
                    .map_err(|e| match e {
                        scuffed_api_client::ClientError::Http { status: 401, .. } => {
                            "Session expired. Please log in again.".to_string()
                        }
                        scuffed_api_client::ClientError::Http { status, body } => {
                            format!("Server error ({status}): {body}")
                        }
                        scuffed_api_client::ClientError::Network(msg) => {
                            format!("Network error: {msg}. Check your connection.")
                        }
                        e => format!("Failed to get challenge: {e}"),
                    })?;

                link_state.set(LinkState::WaitingForSignature);
                let signed = nip07_sign_event(&challenge.event).await?;

                link_state.set(LinkState::SubmittingLink);
                let body = LinkRequest { signed_event: signed };
                let identity: NostrIdentity = ApiClient::web()
                    .post_json("/api/identity/nostr/link", &body)
                    .await
                    .map_err(|e| match e {
                        scuffed_api_client::ClientError::Http { status: 410, .. } => {
                            "Challenge expired. Please try again.".to_string()
                        }
                        scuffed_api_client::ClientError::Http { status: 409, .. } => {
                            "This Nostr key is already linked to another account.".to_string()
                        }
                        scuffed_api_client::ClientError::Http { status, body } => {
                            format!("Link failed ({status}): {body}")
                        }
                        scuffed_api_client::ClientError::Network(msg) => {
                            format!("Network error: {msg}. Check your connection.")
                        }
                        e => format!("Failed to link: {e}"),
                    })?;

                linked_identity.set(Some(Some(identity)));
                let mut toast = use_toast();
                toast.show(Toast::success("Nostr identity linked"));
                Ok(())
            }
            .await;

            if let Err(msg) = result {
                error_msg.set(Some(msg));
            }
            link_state.set(LinkState::Idle);
        });
    };

    let do_unlink = move |_| {
        confirm_unlink.set(false);
        error_msg.set(None);
        link_state.set(LinkState::Unlinking);

        spawn(async move {
            match ApiClient::web().delete("/api/identity/nostr").await {
                Ok(()) => {
                    linked_identity.set(Some(None));
                    let mut toast = use_toast();
                    toast.show(Toast::success("Nostr identity unlinked"));
                }
                Err(e) => {
                    let msg = match e {
                        scuffed_api_client::ClientError::Http { status: 401, .. } => {
                            "Session expired. Please log in again.".to_string()
                        }
                        scuffed_api_client::ClientError::Network(msg) => {
                            format!("Network error: {msg}. Check your connection.")
                        }
                        e => format!("Failed to unlink: {e}"),
                    };
                    error_msg.set(Some(msg));
                }
            }
            link_state.set(LinkState::Idle);
        });
    };

    let is_busy = link_state() != LinkState::Idle;

    rsx! {
        style { {IDENTITY_CSS} }
        div { class: "identity-page",
            h1 { class: "identity-title", "Identity" }
            p { class: "identity-subtitle", "Link your Nostr key to prove ownership and unlock decentralized features." }

            {
                if auth().loading || extension_available().is_none() || linked_identity().is_none() {
                    rsx! { p { class: "identity-loading", "Loading..." } }
                } else if !auth().is_logged_in() {
                    rsx! {
                        div { class: "identity-card",
                            h2 { class: "identity-card-title", "Sign In Required" }
                            p { class: "identity-card-desc", "You need to be logged in to manage your Nostr identity." }
                            div { class: "identity-auth-prompt",
                                a { href: "/api/auth/discord/login", "Sign in with Discord" }
                            }
                        }
                    }
                } else {
                    let has_ext = extension_available().unwrap_or(false);
                    let identity = linked_identity().flatten();

                    rsx! {
                        // Nostr identity card
                        div { class: "identity-card",
                            h2 { class: "identity-card-title",
                                "Nostr Key"
                                if identity.is_some() {
                                    span { class: "identity-status linked", "Linked" }
                                } else if !has_ext {
                                    span { class: "identity-status no-extension", "No Extension" }
                                } else {
                                    span { class: "identity-status not-linked", "Not Linked" }
                                }
                            }

                            if let Some(ref id) = identity {
                                // Linked state — show keys
                                div { class: "key-display",
                                    {render_key_row("npub", &hex_to_npub(&id.pubkey).unwrap_or_else(|_| "encoding error".into()), copied_field)}
                                    {render_key_row("hex", &id.pubkey, copied_field)}
                                    p { class: "key-linked-at",
                                        "Linked {id.linked_at}"
                                    }
                                }

                                div { class: "identity-actions",
                                    button {
                                        class: "identity-btn identity-btn-danger",
                                        disabled: is_busy,
                                        onclick: move |_| confirm_unlink.set(true),
                                        "Unlink Key"
                                    }
                                }
                            } else if !has_ext {
                                // No extension installed
                                p { class: "identity-card-desc",
                                    "No NIP-07 browser extension detected. You need one to link your Nostr identity."
                                }
                                div { class: "extension-help",
                                    div { class: "extension-help-title", "Install a NIP-07 extension:" }
                                    ul {
                                        li { "nos2x — lightweight Chrome/Firefox extension" }
                                        li { "Alby — also supports Lightning payments" }
                                        li { "Flamingo — minimal Nostr signer" }
                                        li { "Nostr Connect — for advanced setups" }
                                    }
                                }
                                p { class: "identity-card-desc", style: "margin-top: 0.75rem;",
                                    "After installing, reload this page."
                                }
                            } else {
                                // Extension found, not linked
                                p { class: "identity-card-desc",
                                    "Your browser has a NIP-07 extension. Click below to link your Nostr public key to this account."
                                }

                                // Loading states during linking
                                if is_busy {
                                    div { class: "identity-loading-row",
                                        span { class: "identity-spinner" }
                                        span {
                                            match link_state() {
                                                LinkState::DetectingExtension => "Detecting extension...",
                                                LinkState::GettingKey => "Requesting public key from extension...",
                                                LinkState::RequestingChallenge => "Getting challenge from server...",
                                                LinkState::WaitingForSignature => "Waiting for you to sign in extension...",
                                                LinkState::SubmittingLink => "Verifying and linking...",
                                                LinkState::Unlinking => "Unlinking...",
                                                LinkState::Idle => "",
                                            }
                                        }
                                    }
                                }

                                div { class: "identity-actions",
                                    button {
                                        class: "identity-btn identity-btn-primary",
                                        disabled: is_busy,
                                        onclick: do_link,
                                        if is_busy { "Linking..." } else { "Link Nostr Key" }
                                    }
                                }
                            }

                            // Error display
                            if let Some(ref msg) = error_msg() {
                                div { class: "identity-error",
                                    div { class: "identity-error-title", "Something went wrong" }
                                    "{msg}"
                                }
                            }
                        }

                        // Unlink confirmation dialog
                        ConfirmDialog {
                            title: "Unlink Nostr Key".to_string(),
                            message: "This will remove the Nostr key from your account. You can re-link it later.".to_string(),
                            open: confirm_unlink(),
                            danger: true,
                            on_confirm: do_unlink,
                            on_cancel: move |_| confirm_unlink.set(false),
                        }
                    }
                }
            }
        }
    }
}

fn render_key_row(label: &str, value: &str, mut copied_field: Signal<Option<String>>) -> Element {
    let label_owned = label.to_string();
    let value_owned = value.to_string();
    let is_copied = copied_field().as_deref() == Some(label);
    let copy_class = if is_copied { "key-copy copied" } else { "key-copy" };
    let copy_label = if is_copied { "Copied" } else { "Copy" };

    rsx! {
        div { class: "key-row",
            span { class: "key-label", "{label}" }
            span { class: "key-value", "{value}" }
            button {
                class: "{copy_class}",
                onclick: move |_| {
                    let v = value_owned.clone();
                    let l = label_owned.clone();
                    spawn(async move {
                        clipboard_write(&v).await;
                        copied_field.set(Some(l));
                        gloo_timers::future::TimeoutFuture::new(2_000).await;
                        copied_field.set(None);
                    });
                },
                "{copy_label}"
            }
        }
    }
}
