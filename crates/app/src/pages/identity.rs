use dioxus::prelude::*;

use crate::components::admin_shared::focus_after_delete_soon;
use serde::{Deserialize, Serialize};
use wasm_bindgen::JsCast;
use wasm_bindgen::prelude::*;

use scuffed_api_client::{ApiClient, ClientError};
use scuffed_types::MeResponse;

use super::profile_settings::{
    ACCOUNT_CSS, ACCOUNT_CSS_ID, AccountPending, AccountStatus, AccountView,
    account_view_from_auth, account_view_from_error,
};
use crate::components::ui::{InlineConfirm, PageHeader, PageShell, PageWidth, btn_class};
use crate::components::{SaveState, Toast, use_toast};
use crate::routes::Route;
use crate::state::auth::{AuthState, use_auth};

/// The one name for this page: the h1, the head title and the account menu.
pub const ACCOUNT_KEYS: &str = "Account keys";

const INTRO: &str = "Your account key is your ID in chat and direct messages. \
    We can keep one for you, or you can bring your own.";
/// The Key mode fact in plain words, the same words
/// `docs/external-clients.md` quotes. The stored values are
/// `server_managed` and `external` (`NostrKeyMode` in `crates/db`); a value
/// this build does not know reads with spaces for underscores.
fn key_mode_label(mode: Option<&str>) -> String {
    match mode {
        Some("server_managed") => "Server-managed".to_string(),
        Some("external") => "External".to_string(),
        Some(other) if !other.trim().is_empty() => other.replace('_', " "),
        _ => "Unknown".to_string(),
    }
}

const HINT_PUBLIC_KEY: &str = "Your public key is safe to share. It is how others know it is you.";
const HINT_NIP05: &str = "A NIP-05 name looks like an email address and points to your key.";
const HINT_BACKUP: &str =
    "An ncryptsec backup is your key locked with a password. You need the password to use it.";
const HINT_ADVANCED: &str = "You rarely need these. Importing a key or unlinking changes how \
    chat and direct messages sign you in.";
/// Under the collapsed "Advanced" title, so the visitor sees what it holds.
const ADVANCED_CUE: &str = "Import a key or unlink yours.";
const HINT_UNLINK: &str = "Chat and direct messages stop working until you link a key again.";

const NIP05_UNSET: &str = "Not set up on this site.";
const HINT_NIP05_UNSET: &str = "NIP-05 names are switched on by whoever runs this site. \
    Once they are, yours is your display name at this site's address.";
const HINT_NIP05_SET: &str = "It follows your display name. Change the name to change it.";

/// Fixed ids, one per field on the page.
const PUBLIC_KEY_ID: &str = "identity-public-key";
const PUBLIC_KEY_HINT_ID: &str = "identity-public-key-hint";
const NIP05_ID: &str = "identity-nip05";
const NIP05_HINT_ID: &str = "identity-nip05-hint";
const LINK_PUBKEY_ID: &str = "identity-link-pubkey";
const LINK_STATUS_ID: &str = "identity-link-status";
const BACKUP_PASSWORD_ID: &str = "identity-backup-password";
const BACKUP_PASSWORD_HINT_ID: &str = "identity-backup-password-hint";
const BACKUP_STATUS_ID: &str = "identity-backup-status";
const BACKUP_RESULT_ID: &str = "identity-backup-result";
const BACKUP_RESULT_HINT_ID: &str = "identity-backup-result-hint";
const IMPORT_NCRYPTSEC_ID: &str = "identity-import-ncryptsec";
const IMPORT_PASSWORD_ID: &str = "identity-import-password";
const IMPORT_STATUS_ID: &str = "identity-import-status";
/// Import key, and the Cancel of its confirm strip (`InlineConfirm` id
/// `import-key`).
const IMPORT_TRIGGER_ID: &str = "import-key-trigger";
const IMPORT_CANCEL_ID: &str = "import-key-cancel";
const UNLINK_STATUS_ID: &str = "identity-unlink-status";
const UNLINK_TRIGGER_ID: &str = "unlink-identity-trigger";

/// Account keys only. The cards, fields and save lines are the shared
/// account pieces in profile_settings.rs.
const PAGE_CSS: &str = r#"
.identity-facts {
    display: grid;
    grid-template-columns: repeat(2, minmax(0, 1fr));
    gap: var(--space-5);
    min-width: 0;
}
/* The summary drops the native marker (flex), so it draws its own chevron
   and says what the group holds while it is closed. */
.identity-advanced > summary {
    display: flex;
    align-items: center;
    justify-content: space-between;
    gap: var(--space-3);
    min-height: var(--hit-target-min);
    list-style: none;
    cursor: pointer;
}
.identity-advanced > summary::-webkit-details-marker { display: none; }
.identity-advanced-text {
    display: flex;
    flex-direction: column;
    gap: var(--space-1);
    min-width: 0;
}
.identity-advanced-title {
    font-family: var(--font-head);
    font-weight: var(--font-head-weight);
    letter-spacing: var(--font-head-tracking);
    line-height: var(--leading-display);
    font-size: var(--text-xl);
    color: var(--text);
}
.identity-advanced-chevron {
    flex: 0 0 auto;
    box-sizing: border-box;
    width: var(--space-3);
    height: var(--space-3);
    margin-inline-end: var(--space-1);
    border-inline-end: var(--focus-ring-width) solid var(--text-3);
    border-bottom: var(--focus-ring-width) solid var(--text-3);
    transform: rotate(45deg);
}
.identity-advanced[open] .identity-advanced-chevron { transform: rotate(-135deg); }
.identity-advanced > summary:focus-visible {
    outline: var(--focus-ring-width) solid var(--focus-ring);
    outline-offset: var(--focus-ring-offset);
    border-radius: var(--radius-control);
}
.identity-sub {
    display: flex;
    flex-direction: column;
    gap: var(--space-5);
    min-width: 0;
    padding-top: var(--space-5);
    border-top: 1px solid var(--border);
}
.identity-sub > * { margin: 0; }
.identity-warning {
    margin: 0;
    color: var(--warn);
    font-family: var(--font-body);
    font-size: var(--text-sm);
    line-height: var(--leading-body);
}
.identity-key-input { font-family: var(--font-mono); }
textarea.identity-key-input { min-height: calc(var(--hit-target-min) * 2); }
/* The npub wraps inside its box instead of running off a phone screen. */
textarea.field-input.identity-npub {
    min-height: var(--hit-target-min);
    field-sizing: content;
    resize: none;
    overflow-wrap: anywhere;
    word-break: break-all;
}
@media (max-width: 820px) {
    .identity-facts { grid-template-columns: minmax(0, 1fr); }
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

/// A 64-character hex public key as its NIP-19 `npub1...` form, the way the
/// rest of the product (DMs, chat) writes keys. `None` when the value is not
/// 32 bytes of hex.
fn npub_from_hex(hex: &str) -> Option<String> {
    let hex = hex.trim();
    if hex.len() != 64 || !hex.is_ascii() {
        return None;
    }
    let bytes = (0..32)
        .map(|i| u8::from_str_radix(&hex[i * 2..i * 2 + 2], 16).ok())
        .collect::<Option<Vec<u8>>>()?;
    let hrp = bech32::Hrp::parse("npub").ok()?;
    bech32::encode::<bech32::Bech32>(hrp, &bytes).ok()
}

fn truncate_pubkey(pk: &str) -> String {
    if pk.len() > 16 {
        format!("{}...{}", &pk[..8], &pk[pk.len() - 8..])
    } else {
        pk.to_string()
    }
}

fn has_nip07_extension() -> bool {
    // Only a browser has `window.nostr`. The native test build has no
    // window, and web-sys calls panic there.
    #[cfg(target_arch = "wasm32")]
    {
        web_sys::window()
            .and_then(|w| js_sys::Reflect::get(&w, &JsValue::from_str("nostr")).ok())
            .map(|v| !v.is_undefined() && !v.is_null())
            .unwrap_or(false)
    }
    #[cfg(not(target_arch = "wasm32"))]
    {
        false
    }
}

/// The collapsed group for rare, riskier actions. Closed until the visitor opens
/// it, so it never carries an `open` attribute of its own.
#[component]
fn AdvancedGroup(children: Element) -> Element {
    rsx! {
        details { class: "account-card identity-advanced",
            summary {
                span { class: "identity-advanced-text",
                    span { class: "identity-advanced-title", "Advanced" }
                    span { class: "field-hint", "{ADVANCED_CUE}" }
                }
                span { class: "identity-advanced-chevron", aria_hidden: "true" }
            }
            p { class: "field-hint", "{HINT_ADVANCED}" }
            {children}
        }
    }
}

/// What the page shows before its content, from the session and the load.
fn identity_view(auth: &AuthState, me: Option<&Result<MeResponse, ClientError>>) -> AccountView {
    if let Some(view) = account_view_from_auth(auth) {
        return view;
    }
    match me {
        None => AccountView::Loading,
        Some(Err(e)) => account_view_from_error(&crate::client_error::user_message(e)),
        Some(Ok(me)) if me.member.is_none() => AccountView::Apply,
        Some(Ok(_)) => AccountView::Ready,
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
    let mut link_state = use_signal(SaveState::default);
    let mut backup_state = use_signal(SaveState::default);
    let mut import_state = use_signal(SaveState::default);
    let mut unlink_state = use_signal(SaveState::default);

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
                Err(reason) => {
                    // The extension's own words (a String), not a request error.
                    link_state.set(SaveState::Failed(format!("Extension error: {reason}")));
                    toast.show(Toast::error(format!("Extension error: {reason}")));
                }
            }
        });
    };

    // ─── NIP-07: Link identity via challenge-response ───
    let on_link_identity = move |evt: FormEvent| {
        evt.prevent_default();
        if working() {
            return;
        }
        let pubkey = pubkey_input().trim().to_string();
        if pubkey.is_empty() {
            link_state.set(SaveState::Failed("Enter a public key first.".to_string()));
            toast.show(Toast::error("Enter a pubkey first."));
            return;
        }
        working.set(true);
        link_state.set(SaveState::Saving);
        spawn(async move {
            match link_nostr_identity(&pubkey).await {
                Ok(()) => {
                    link_state.set(SaveState::Saved("Key linked.".to_string()));
                    toast.show(Toast::success("Key linked."));
                    refresh += 1;
                    pubkey_input.set(String::new());
                }
                Err(reason) => {
                    // `link_nostr_identity` already maps its request errors
                    // through `user_message`; the rest is extension text.
                    // One lead, then the step that failed (Vision nit 11
                    // on 748: no "Link failed: Challenge request failed:").
                    let line = SaveState::failed_text("Could not link the key", &reason);
                    toast.show(Toast::error(line.text()));
                    link_state.set(line);
                }
            }
            working.set(false);
        });
    };

    // ─── NIP-49: Export backup ───
    let on_export_backup = move |evt: FormEvent| {
        evt.prevent_default();
        if working() {
            return;
        }
        let password = backup_password().trim().to_string();
        if password.len() < 12 {
            backup_state.set(SaveState::Failed(
                "Use a password of at least 12 characters.".to_string(),
            ));
            toast.show(Toast::error("Password must be at least 12 characters."));
            return;
        }
        working.set(true);
        backup_state.set(SaveState::Saving);
        spawn(async move {
            let body = ExportBackupBody { password };
            match ApiClient::web()
                .post_json::<_, ExportBackupResponse>("/api/nostr/export-backup", &body)
                .await
            {
                Ok(resp) => {
                    backup_result.set(Some(resp.ncryptsec));
                    backup_password.set(String::new());
                    backup_state.set(SaveState::Saved("Backup ready. Copy it now.".to_string()));
                    toast.show(Toast::success("Key backup exported. Copy it now!"));
                }
                Err(e) => {
                    backup_state.set(SaveState::failed("Could not export the backup", &e));
                    toast.show(Toast::error_from("Export failed", &e));
                }
            }
            working.set(false);
        });
    };

    // ─── NIP-49: Import key ───
    let on_import_key = move |evt: FormEvent| {
        evt.prevent_default();
        if working() {
            return;
        }
        let ncryptsec = import_ncryptsec().trim().to_string();
        let password = import_password().trim().to_string();
        if ncryptsec.is_empty() || password.is_empty() {
            import_state.set(SaveState::Failed(
                "Enter the ncryptsec backup and its password.".to_string(),
            ));
            toast.show(Toast::error("Both ncryptsec and password are required."));
            return;
        }
        import_state.set(SaveState::Idle);
        // Importing links an *external* key and replaces any currently linked
        // identity, so ask first with the inline strip (DR1-NOSTR-003).
        confirm_import.set(true);
    };

    let do_import_key = move |_| {
        confirm_import.set(false);
        // The confirm strip closes with focus in it, on success and failure
        // alike: Import key takes it back once the strip is gone.
        focus_after_delete_soon(
            IMPORT_CANCEL_ID.to_string(),
            vec![IMPORT_TRIGGER_ID.to_string()],
        );
        let ncryptsec = import_ncryptsec().trim().to_string();
        let password = import_password().trim().to_string();
        working.set(true);
        import_state.set(SaveState::Saving);
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
                    import_state.set(SaveState::Saved("Key imported.".to_string()));
                    toast.show(Toast::success("Key imported successfully!"));
                    refresh += 1;
                    import_ncryptsec.set(String::new());
                    import_password.set(String::new());
                }
                Err(e) => {
                    import_state.set(SaveState::failed("Could not import the key", &e));
                    toast.show(Toast::error_from("Import failed", &e));
                }
            }
            working.set(false);
        });
    };

    // ─── Unlink identity ───
    let do_unlink = move |_| {
        confirm_unlink.set(false);
        working.set(true);
        unlink_state.set(SaveState::Saving);
        spawn(async move {
            match ApiClient::web().delete("/api/nostr/identity").await {
                Ok(()) => {
                    unlink_state.set(SaveState::Saved("Key unlinked.".to_string()));
                    toast.show(Toast::success("Key unlinked."));
                    refresh += 1;
                    // B9: the unlink block goes with the key. Focus the
                    // public key field of Link a key once it is back.
                    focus_after_delete_soon(
                        UNLINK_TRIGGER_ID.to_string(),
                        vec![
                            LINK_PUBKEY_ID.to_string(),
                            "identity-link-heading".to_string(),
                        ],
                    );
                }
                Err(e) => {
                    unlink_state.set(SaveState::failed("Could not unlink the key", &e));
                    toast.show(Toast::error_from("Unlink failed", &e));
                    // The confirm strip closed with focus in it: back to
                    // Unlink key.
                    focus_after_delete_soon(
                        "unlink-identity-cancel".to_string(),
                        vec![UNLINK_TRIGGER_ID.to_string()],
                    );
                }
            }
            working.set(false);
        });
    };

    let me_data = identity.read();
    let view = identity_view(&auth(), me_data.as_ref());
    let me_ok = me_data.as_ref().and_then(|r| r.as_ref().ok());
    let member = me_ok.and_then(|me| me.member.as_ref());

    // Server-computed: `None` when the deploy has no valid public NIP-05
    // domain. Never rebuild this client-side; the client cannot know which
    // domain serves `/.well-known/nostr.json`, and guessing is what published
    // identities pointing at a domain we do not own.
    let nip05 = me_ok.and_then(|me| me.nip05.clone());

    let has_pubkey = member.map(|m| m.nostr_pubkey.is_some()).unwrap_or(false);
    let is_server_managed = member
        .map(|m| m.nostr_key_mode.as_deref() == Some("server_managed"))
        .unwrap_or(false);
    let busy = working();
    let link_blocked = busy || pubkey_input().trim().is_empty();
    let export_blocked = busy || backup_password().trim().len() < 12;
    let import_blocked =
        busy || import_ncryptsec().trim().is_empty() || import_password().trim().is_empty();

    rsx! {
        crate::styles::StyleOnce { id: ACCOUNT_CSS_ID, css: ACCOUNT_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-identity-page-css", css: PAGE_CSS }

        PageShell { width: PageWidth::Narrow, class: "identity-page",
            PageHeader { title: ACCOUNT_KEYS, lead: INTRO }

            if view != AccountView::Ready {
                AccountPending {
                    view: view.clone(),
                    noun: "your account keys",
                    task: "manage your account keys",
                    return_path: "/identity",
                    on_retry: move |_| refresh += 1,
                }
            } else if let Some(m) = member {
                section { class: "account-section", aria_labelledby: "identity-key-heading",
                    h2 { id: "identity-key-heading", class: "ui-section-title", "Your key" }
                    div { class: "account-card",
                        if let Some(ref pubkey) = m.nostr_pubkey {
                            div { class: "account-field",
                                label { class: "field-label", r#for: PUBLIC_KEY_ID, "Public key" }
                                textarea {
                                    id: PUBLIC_KEY_ID,
                                    class: "field-input identity-key-input identity-npub",
                                    rows: 2,
                                    readonly: true,
                                    spellcheck: false,
                                    value: npub_from_hex(pubkey).unwrap_or_else(|| pubkey.clone()),
                                    aria_describedby: PUBLIC_KEY_HINT_ID,
                                }
                                p { id: PUBLIC_KEY_HINT_ID, class: "field-hint", "{HINT_PUBLIC_KEY}" }
                            }
                            div { class: "identity-facts",
                                div { class: "account-field",
                                    p { class: "field-label", "Key mode" }
                                    p { class: "account-fact",
                                        "{key_mode_label(m.nostr_key_mode.as_deref())}"
                                    }
                                }
                                div { class: "account-field",
                                    if let Some(ref id) = nip05 {
                                        label { class: "field-label", r#for: NIP05_ID, "NIP-05 name" }
                                        input {
                                            id: NIP05_ID,
                                            class: "field-input",
                                            r#type: "text",
                                            readonly: true,
                                            spellcheck: false,
                                            value: "{id}",
                                            aria_describedby: NIP05_HINT_ID,
                                        }
                                        p { id: NIP05_HINT_ID, class: "field-hint",
                                            "{HINT_NIP05} {HINT_NIP05_SET} "
                                            Link { to: Route::ProfileSettings {}, "Change display name" }
                                        }
                                    } else {
                                        p { class: "field-label", "NIP-05 name" }
                                        p { class: "account-fact", "{NIP05_UNSET}" }
                                        p { id: NIP05_HINT_ID, class: "field-hint",
                                            "{HINT_NIP05} {HINT_NIP05_UNSET}"
                                        }
                                    }
                                }
                            }
                        } else {
                            p { class: "account-lead",
                                "No key is linked yet. Link one below, or ask an admin to set one up for you."
                            }
                        }
                    }
                }

                if !has_pubkey {
                    section { class: "account-section", aria_labelledby: "identity-link-heading",
                        // Focusable from script only: an unlink without a
                        // browser extension lands here (B9).
                        h2 { id: "identity-link-heading", class: "ui-section-title", tabindex: "-1", "Link a key" }
                        if has_extension {
                            form { class: "account-card", onsubmit: on_link_identity,
                                div { class: "account-field",
                                    label { class: "field-label", r#for: LINK_PUBKEY_ID, "Public key (hex or npub)" }
                                    input {
                                        id: LINK_PUBKEY_ID,
                                        class: "field-input identity-key-input",
                                        r#type: "text",
                                        spellcheck: false,
                                        autocomplete: "off",
                                        placeholder: "64-char hex or npub1...",
                                        value: "{pubkey_input}",
                                        oninput: move |e| pubkey_input.set(e.value()),
                                    }
                                }
                                div { class: "account-actions",
                                    button {
                                        r#type: "submit",
                                        class: btn_class(true),
                                        aria_disabled: if link_blocked { "true" },
                                        aria_describedby: LINK_STATUS_ID,
                                        if busy { "Linking..." } else { "Link and verify" }
                                    }
                                    button {
                                        r#type: "button",
                                        class: btn_class(false),
                                        onclick: on_detect_pubkey,
                                        "Detect from extension"
                                    }
                                    AccountStatus { id: LINK_STATUS_ID, state: link_state() }
                                }
                            }
                        } else {
                            div { class: "account-card",
                                p { class: "account-lead",
                                    "Install a NIP-07 browser extension (nos2x, Alby, or similar) to link your key."
                                }
                            }
                        }
                    }
                }

                if is_server_managed {
                    section { class: "account-section", aria_labelledby: "identity-backup-heading",
                        h2 { id: "identity-backup-heading", class: "ui-section-title", "Back up your key" }
                        form { class: "account-card", onsubmit: on_export_backup,
                            p { class: "account-lead",
                                "Export your key as an encrypted ncryptsec backup. Keep this password. You will need it to import the key later (which links it as an external identity)."
                            }
                            div { class: "account-field",
                                label { class: "field-label", r#for: BACKUP_PASSWORD_ID, "Backup password" }
                                input {
                                    id: BACKUP_PASSWORD_ID,
                                    class: "field-input",
                                    r#type: "password",
                                    autocomplete: "new-password",
                                    minlength: 12,
                                    aria_describedby: BACKUP_PASSWORD_HINT_ID,
                                    value: "{backup_password}",
                                    oninput: move |e| backup_password.set(e.value()),
                                }
                                p { id: BACKUP_PASSWORD_HINT_ID, class: "field-hint",
                                    "At least 12 characters. {HINT_BACKUP}"
                                }
                            }
                            div { class: "account-actions",
                                button {
                                    r#type: "submit",
                                    class: btn_class(true),
                                    aria_disabled: if export_blocked { "true" },
                                    aria_busy: if busy { "true" },
                                    // The password rule is the reason it is not ready yet.
                                    aria_describedby: "{BACKUP_PASSWORD_HINT_ID} {BACKUP_STATUS_ID}",
                                    if busy { "Exporting..." } else { "Export backup" }
                                }
                                AccountStatus { id: BACKUP_STATUS_ID, state: backup_state() }
                            }
                            if let Some(ref ncryptsec) = *backup_result.read() {
                                div { class: "account-field",
                                    label { class: "field-label", r#for: BACKUP_RESULT_ID, "Your ncryptsec backup" }
                                    textarea {
                                        id: BACKUP_RESULT_ID,
                                        class: "field-input identity-key-input",
                                        readonly: true,
                                        spellcheck: false,
                                        aria_describedby: BACKUP_RESULT_HINT_ID,
                                        value: "{ncryptsec}",
                                    }
                                    p { id: BACKUP_RESULT_HINT_ID, class: "identity-warning",
                                        "Copy this now. It will not be shown again."
                                    }
                                }
                            }
                        }
                    }
                }

                AdvancedGroup {
                    if is_server_managed {
                        div { class: "identity-sub",
                            h3 { class: "account-sub-title", "Import an external key" }
                            p { class: "identity-warning",
                                "You currently have a server-managed key. Importing an external key is \
                                 disabled to protect it. To switch to an external key, first unlink your \
                                 key below, then import. Importing does not restore a key into \
                                 server-managed mode."
                            }
                        }
                    } else {
                        form { class: "identity-sub", onsubmit: on_import_key,
                            h3 { class: "account-sub-title", "Import an external key" }
                            p { class: "account-lead",
                                "Link an external Nostr key from an ncryptsec backup. This sets your \
                                 identity to external mode. The server will not manage a key for you. \
                                 It replaces any currently linked external key. The nsec is decrypted \
                                 only to derive your public key."
                            }
                            div { class: "account-field",
                                label { class: "field-label", r#for: IMPORT_NCRYPTSEC_ID, "ncryptsec backup" }
                                input {
                                    id: IMPORT_NCRYPTSEC_ID,
                                    class: "field-input identity-key-input",
                                    r#type: "text",
                                    spellcheck: false,
                                    autocomplete: "off",
                                    placeholder: "ncryptsec1...",
                                    value: "{import_ncryptsec}",
                                    oninput: move |e| import_ncryptsec.set(e.value()),
                                }
                            }
                            div { class: "account-field",
                                label { class: "field-label", r#for: IMPORT_PASSWORD_ID, "Backup password" }
                                input {
                                    id: IMPORT_PASSWORD_ID,
                                    class: "field-input",
                                    r#type: "password",
                                    autocomplete: "current-password",
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
                                    busy: busy,
                                    on_confirm: do_import_key,
                                    on_cancel: move |_| confirm_import.set(false),
                                }
                            } else {
                                div { class: "account-actions",
                                    button {
                                        id: IMPORT_TRIGGER_ID,
                                        r#type: "submit",
                                        class: btn_class(true),
                                        aria_disabled: if import_blocked { "true" },
                                        aria_describedby: IMPORT_STATUS_ID,
                                        if busy { "Importing..." } else { "Import key" }
                                    }
                                }
                            }
                            AccountStatus { id: IMPORT_STATUS_ID, state: import_state() }
                        }
                    }
                    if has_pubkey {
                        div { class: "identity-sub",
                            h3 { class: "account-sub-title", "Unlink your key" }
                            p { class: "field-hint", "{HINT_UNLINK}" }
                            if confirm_unlink() {
                                InlineConfirm {
                                    id: "unlink-identity".to_string(),
                                    title: "Unlink this key?".to_string(),
                                    body: "This removes the key from your account. Export a backup first if you want to keep it.".to_string(),
                                    confirm_label: "Unlink".to_string(),
                                    busy: busy,
                                    on_confirm: do_unlink,
                                    on_cancel: move |_| confirm_unlink.set(false),
                                }
                            } else {
                                div { class: "account-actions",
                                    button {
                                        id: UNLINK_TRIGGER_ID,
                                        r#type: "button",
                                        class: "btn is-danger",
                                        aria_disabled: if busy { "true" },
                                        aria_describedby: UNLINK_STATUS_ID,
                                        onclick: move |_| {
                                            if !working() {
                                                confirm_unlink.set(true);
                                            }
                                        },
                                        "Unlink key"
                                    }
                                }
                            }
                            AccountStatus { id: UNLINK_STATUS_ID, state: unlink_state() }
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
        .map_err(|e| crate::client_error::with_lead("Could not start the key check.", &e))?;

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
        .map_err(|e| crate::client_error::with_lead("Could not verify the key.", &e))?;

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
    fn the_external_clients_guide_uses_the_button_words() {
        // Vision nit 1 on PR 706: the guide still said "Link & Verify",
        // "Detect" and "Export Backup".
        let guide = include_str!("../../../../docs/external-clients.md");
        let src = include_str!("identity.rs");
        let prod = src.split("mod tests").next().expect("production half");
        for label in ["Link and verify", "Detect from extension", "Export backup"] {
            assert!(
                prod.contains(&format!("\"{label}\"")),
                "{label} in identity.rs"
            );
            assert!(
                guide.contains(&format!("**{label}**")),
                "{label} in the guide"
            );
        }
        for old in ["Link & Verify", "**Detect**", "Export Backup"] {
            assert!(!guide.contains(old), "{old}");
        }
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
        assert!(html.contains(">Advanced</span>"), "{html}");
        assert!(
            html.contains(ADVANCED_CUE),
            "the closed group says what it holds: {html}"
        );
        assert!(
            html.contains("class=\"identity-advanced-chevron\" aria-hidden=\"true\""),
            "{html}"
        );
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
            ADVANCED_CUE,
            HINT_UNLINK,
            NIP05_UNSET,
            HINT_NIP05_UNSET,
            HINT_NIP05_SET,
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
    fn a_guest_gets_the_sign_in_wall_inside_the_page() {
        use crate::components::admin_shared::test_page::render_admin_route as render_route;
        use crate::components::member_gate::test_route::wall;
        let html = render_route("/identity", None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(html.contains(">Account keys</h1>"), "{html}");
        assert!(
            html.contains("ui-shell ui-shell--narrow identity-page"),
            "{html}"
        );
        let card = wall(&html);
        assert!(
            card.contains("Log in to manage your account keys."),
            "{card}"
        );
        assert!(card.contains("href=\"/login?return=/identity\""), "{card}");
        assert!(!html.contains("My reports</a></p>"), "{html}");
    }

    #[test]
    fn a_member_sees_a_labelled_wait() {
        use crate::components::admin_shared::test_page::render_admin_route as render_route;
        let html = render_route("/identity", Some(scuffed_types::OrgRole::Member));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(
            html.contains("role=\"status\">Loading your account keys</p>"),
            "{html}"
        );
    }

    #[test]
    fn public_keys_show_as_npub() {
        // The NIP-19 test vector.
        assert_eq!(
            npub_from_hex("3bf0c63fcb93463407af97a5e5ee64fa883d107ef9e558472c4eb9aaaefa459d")
                .as_deref(),
            Some("npub180cvv07tjdrrgpa0j7j7tmnyl2yr6yr7l8j4s3evf6u64th6gkwsyjh6w6")
        );
        assert_eq!(npub_from_hex("abc"), None);
        assert_eq!(npub_from_hex(&"zz".repeat(32)), None);
    }

    #[test]
    fn key_modes_read_as_words() {
        assert_eq!(key_mode_label(Some("server_managed")), "Server-managed");
        assert_eq!(key_mode_label(Some("external")), "External");
        assert_eq!(key_mode_label(Some("new_mode")), "new mode");
        assert_eq!(key_mode_label(Some("  ")), "Unknown");
        assert_eq!(key_mode_label(None), "Unknown");
    }

    #[test]
    fn page_css_uses_tokens() {
        crate::components::ui::test_support::assert_sizes_from_tokens(PAGE_CSS);
        let src = include_str!("identity.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        assert!(!prod.contains("Route::MyReports"), "no stray reports link");
        assert!(!prod.contains("main {"), "the shell owns main");
    }

    #[test]
    fn unlink_hint_does_not_repeat_the_confirm_text() {
        let src = include_str!("identity.rs");
        assert!(!HINT_UNLINK.contains("removes the key"), "{HINT_UNLINK}");
        assert!(!HINT_UNLINK.contains("Export a backup"), "{HINT_UNLINK}");
        assert!(!src.contains(&["Unlink", " Identity"].concat()));
    }

    /// Vision 694 r3 nit 7: the pill shows plain words, the same words
    /// `docs/external-clients.md` quotes, not the stored value.
    #[test]
    fn key_mode_reads_in_plain_words() {
        assert_eq!(key_mode_label(Some("server_managed")), "Server-managed");
        assert_eq!(key_mode_label(Some("external")), "External");
        assert_eq!(key_mode_label(None), "Unknown");
        let docs = include_str!("../../../../docs/external-clients.md");
        assert!(docs.contains("**Key mode** shows **Server-managed**"));
    }

    #[test]
    fn a_confirmed_import_hands_focus_back_to_import_key() {
        // Vision nit 2 on PR 706 round 4: confirming closed the strip that
        // held focus, and nothing moved it after, on success or failure.
        let src = include_str!("identity.rs");
        let prod = src.split("#[cfg(test)]").next().unwrap();
        let import = prod
            .split("let do_import_key = move |_| {")
            .nth(1)
            .and_then(|rest| rest.split("spawn(async move {").next())
            .expect("do_import_key before its request");
        assert!(
            import.contains(
                "focus_after_delete_soon(\n            IMPORT_CANCEL_ID.to_string(),\n            vec![IMPORT_TRIGGER_ID.to_string()],\n        );"
            ),
            "{import}"
        );
        // The ids are the ones InlineConfirm `import-key` and the trigger use.
        assert!(prod.contains("id: \"import-key\".to_string(),"));
        assert_eq!(IMPORT_CANCEL_ID, "import-key-cancel");
        assert!(prod.contains("id: IMPORT_TRIGGER_ID,"));
        assert_eq!(IMPORT_TRIGGER_ID, "import-key-trigger");
    }
}
