use leptos::prelude::*;
use leptos::task::spawn_local;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;

use scuffed_auth::client::api::{fetch_json, post_json};
use scuffed_ui::components::button::{Button, ButtonVariant};
use scuffed_ui::components::toast::{Toast, use_toast};

use crate::app::use_site_auth;
use crate::components::Nav;
use crate::sections::Footer;

#[derive(Debug, Clone, Deserialize)]
struct MemberFull {
    #[allow(dead_code)]
    id: String,
    display_name: String,
    org_role: String,
    nostr_pubkey: Option<String>,
}

#[derive(Serialize)]
struct ChallengeBody {
    pubkey: String,
}

#[derive(Debug, Clone, Deserialize)]
struct ChallengeResponse {
    challenge: String,
    token: String,
    #[allow(dead_code)]
    pubkey_hex: String,
    #[allow(dead_code)]
    expires_in_secs: u64,
}

/// Run the full NIP-07 challenge-response flow to link a Nostr identity.
async fn link_nostr_identity(pubkey: &str) -> Result<(), String> {
    // 1. Request challenge from server
    let challenge_resp: ChallengeResponse = post_json(
        "/api/nostr/challenge",
        &ChallengeBody {
            pubkey: pubkey.to_string(),
        },
    )
    .await
    .map_err(|e| format!("Challenge request failed: {e}"))?;

    // 2. Check for NIP-07 extension
    let window = web_sys::window().ok_or("No browser window")?;
    let nostr = js_sys::Reflect::get(&window, &"nostr".into())
        .map_err(|_| "NIP-07 extension not detected".to_string())?;
    if nostr.is_undefined() || nostr.is_null() {
        return Err(
            "No Nostr browser extension found. Install nos2x, Alby, or another NIP-07 extension."
                .into(),
        );
    }

    // 3. Create unsigned event for the extension to sign.
    // Kind 22242 is ephemeral (NIP-42 AUTH range) — prevents the challenge from
    // being broadcast as a regular post if the extension relays it.
    let now = (js_sys::Date::now() / 1000.0) as u64;
    let unsigned = serde_json::json!({
        "kind": 22242,
        "created_at": now,
        "tags": [],
        "content": challenge_resp.challenge,
    });
    let event_js = js_sys::JSON::parse(&unsigned.to_string())
        .map_err(|_| "Failed to create event object".to_string())?;

    // 4. Call window.nostr.signEvent()
    let sign_fn = js_sys::Reflect::get(&nostr, &"signEvent".into())
        .map_err(|_| "signEvent method not found on NIP-07 extension".to_string())?;
    let sign_fn: js_sys::Function = sign_fn
        .dyn_into()
        .map_err(|_| "signEvent is not a function".to_string())?;
    let promise = sign_fn
        .call1(&nostr, &event_js)
        .map_err(|e| format!("signEvent call failed: {e:?}"))?;
    let promise: js_sys::Promise = promise
        .dyn_into()
        .map_err(|_| "signEvent did not return a promise".to_string())?;
    let signed_js = wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|e| format!("Signing was rejected or failed: {e:?}"))?;

    // 5. Convert signed event back to JSON
    let signed_str = js_sys::JSON::stringify(&signed_js)
        .map_err(|_| "Failed to serialize signed event".to_string())?;
    let signed_event: serde_json::Value = serde_json::from_str(&String::from(signed_str))
        .map_err(|_| "Failed to parse signed event JSON".to_string())?;

    // 6. Send to verification endpoint
    let verify_body = serde_json::json!({
        "token": challenge_resp.token,
        "signed_event": signed_event,
    });
    let _: serde_json::Value = post_json("/api/nostr/verify", &verify_body)
        .await
        .map_err(|e| format!("Verification failed: {e}"))?;

    Ok(())
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

#[component]
pub fn SettingsPage() -> impl IntoView {
    let auth = use_site_auth();
    let toast = use_toast();

    // Member data with refresh
    let refresh = RwSignal::new(0u32);
    let member = LocalResource::new(move || {
        refresh.get();
        let m = auth.member.get();
        async move {
            let m = m?;
            fetch_json::<MemberFull>(&format!("/api/members/{}", m.id))
                .await
                .ok()
        }
    });

    // Form state
    let pubkey_input = RwSignal::new(String::new());
    let linking = RwSignal::new(false);
    let unlinking = RwSignal::new(false);
    let has_nip07 = RwSignal::new(false);

    // Check for NIP-07 extension on mount
    spawn_local(async move {
        // Small delay to allow extensions to inject
        gloo_timers::future::sleep(std::time::Duration::from_millis(200)).await;
        if let Some(window) = web_sys::window() {
            let nostr = js_sys::Reflect::get(&window, &"nostr".into()).ok();
            has_nip07.set(nostr.is_some_and(|v| !v.is_undefined() && !v.is_null()));
        }
    });

    let do_link = move || {
        let pk = pubkey_input.get();
        if pk.trim().is_empty() {
            toast.show(Toast::error("Enter your Nostr public key"));
            return;
        }
        linking.set(true);
        spawn_local(async move {
            match link_nostr_identity(&pk).await {
                Ok(()) => {
                    toast.show(Toast::success("Nostr identity verified and linked!"));
                    pubkey_input.set(String::new());
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(e)),
            }
            linking.set(false);
        });
    };

    let do_unlink = move || {
        unlinking.set(true);
        spawn_local(async move {
            match scuffed_auth::client::api::delete("/api/nostr/identity").await {
                Ok(()) => {
                    toast.show(Toast::success("Nostr identity unlinked"));
                    refresh.update(|n| *n += 1);
                }
                Err(e) => toast.show(Toast::error(format!("Failed: {e}"))),
            }
            unlinking.set(false);
        });
    };

    view! {
        <Nav/>
        <main class="settings-page">
            <h1 class="settings-title">"Settings"</h1>

            {move || {
                let loading = auth.loading.get();

                if loading {
                    return view! { <p class="settings-loading">"Loading..."</p> }.into_any();
                }

                if !auth.is_logged_in() || !auth.is_member() {
                    return view! {
                        <div class="settings-card">
                            <h2 class="settings-card-title">"Sign In Required"</h2>
                            <p class="settings-card-desc">"You need to be a member to access settings."</p>
                            <a href="/api/auth/discord/login" class="btn btn-primary">"Sign in with Discord"</a>
                        </div>
                    }.into_any();
                }

                let m = member.get().flatten();
                match m {
                    None => view! { <p class="settings-loading">"Loading profile..."</p> }.into_any(),
                    Some(m) => {
                        let has_pubkey = m.nostr_pubkey.is_some();
                        let nip05 = format!("{}@scuffedclan.gg", nip05_name(&m.display_name));

                        view! {
                            // Profile summary
                            <div class="settings-card">
                                <h2 class="settings-card-title">"Profile"</h2>
                                <div class="settings-field-row">
                                    <span class="settings-field-label">"Display Name"</span>
                                    <span class="settings-field-value">{m.display_name.clone()}</span>
                                </div>
                                <div class="settings-field-row">
                                    <span class="settings-field-label">"Role"</span>
                                    <span class="settings-field-value">{m.org_role.clone()}</span>
                                </div>
                            </div>

                            // Nostr identity card
                            <div class="settings-card">
                                <h2 class="settings-card-title">"Nostr Identity"</h2>

                                {if has_pubkey {
                                    let pk = m.nostr_pubkey.unwrap();
                                    let pk_display = truncate_pubkey(&pk);
                                    view! {
                                        <div class="nostr-linked">
                                            <div class="nostr-status">
                                                <span class="nostr-status-dot verified"></span>
                                                <span class="nostr-status-text">"Verified"</span>
                                            </div>
                                            <div class="settings-field-row">
                                                <span class="settings-field-label">"NIP-05"</span>
                                                <span class="settings-field-value nostr-nip05">{nip05}</span>
                                            </div>
                                            <div class="settings-field-row">
                                                <span class="settings-field-label">"Public Key"</span>
                                                <span class="settings-field-value mono">{pk_display}</span>
                                            </div>
                                            <div class="settings-actions">
                                                <Button
                                                    variant=ButtonVariant::Danger
                                                    disabled=unlinking.get()
                                                    on_click=Callback::new(move |_| do_unlink())
                                                >
                                                    {move || if unlinking.get() { "Unlinking..." } else { "Unlink Identity" }}
                                                </Button>
                                            </div>
                                        </div>
                                    }.into_any()
                                } else {
                                    view! {
                                        <div class="nostr-unlinked">
                                            <p class="settings-card-desc">
                                                "Link your Nostr public key to get a verified "
                                                <strong>"NIP-05"</strong>
                                                " identity: "
                                                <span class="nostr-nip05">{nip05}</span>
                                            </p>

                                            {move || if has_nip07.get() {
                                                view! {
                                                    <div class="nostr-link-form">
                                                        <div class="settings-field">
                                                            <label class="settings-field-label">"Public Key"</label>
                                                            <input
                                                                class="settings-input"
                                                                type="text"
                                                                placeholder="npub1... or 64-char hex"
                                                                prop:value=move || pubkey_input.get()
                                                                on:input=move |ev| pubkey_input.set(event_target_value(&ev))
                                                            />
                                                        </div>
                                                        <div class="settings-actions">
                                                            <Button
                                                                variant=ButtonVariant::Primary
                                                                disabled=linking.get()
                                                                on_click=Callback::new(move |_| do_link())
                                                            >
                                                                {move || if linking.get() { "Verifying..." } else { "Link & Verify" }}
                                                            </Button>
                                                        </div>
                                                        <p class="settings-hint">
                                                            "Your browser extension will ask you to sign a challenge to prove key ownership."
                                                        </p>
                                                    </div>
                                                }.into_any()
                                            } else {
                                                view! {
                                                    <div class="nostr-no-ext">
                                                        <p class="settings-card-desc">
                                                            "To link your Nostr identity, you need a NIP-07 browser extension like "
                                                            <strong>"nos2x"</strong>
                                                            " or "
                                                            <strong>"Alby"</strong>
                                                            "."
                                                        </p>
                                                    </div>
                                                }.into_any()
                                            }}
                                        </div>
                                    }.into_any()
                                }}
                            </div>
                        }.into_any()
                    }
                }
            }}
        </main>
        <Footer/>
    }
}
