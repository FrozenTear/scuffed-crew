use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{CreateDaemonTokenRequest, CreateDaemonTokenResponse};

use crate::components::{DataTable, FormModal, Toast, member_pending, use_toast};
use crate::hooks::{ModalController, use_api};
use crate::state::use_auth;

#[derive(Debug, Clone, Deserialize)]
struct DaemonToken {
    id: String,
    #[allow(dead_code)]
    member_id: String,
    label: String,
    is_active: bool,
    created_at: DateTime<Utc>,
    last_used_at: Option<DateTime<Utc>>,
}

const TOKENS_CSS: &str = r#"
    .tokens-page {
        max-width: 900px;
        margin: 0 auto;
        padding: 2rem 1.5rem;
    }
    .tokens-page h1 {
        font-family: var(--font-head);
        font-size: 1.8rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .tracker-setup {
        background: var(--surface);
        border: 1px solid var(--border);
        border-radius: 8px;
        padding: 1rem;
        margin-bottom: 1.5rem;
    }
    .tracker-setup.has-token {
        border-color: var(--accent);
    }
    .tracker-setup h2 {
        font-family: var(--font-head);
        font-size: 1rem;
        color: var(--text);
        text-transform: uppercase;
        letter-spacing: 0.04em;
        margin: 0 0 0.5rem;
    }
    .tracker-setup p {
        color: var(--text-2);
        font-size: 0.85rem;
        margin: 0 0 0.5rem;
    }
    .tracker-setup a {
        color: var(--accent);
        font-size: 0.85rem;
    }
    .tracker-setup code {
        display: block;
        background: var(--bg);
        border: 1px solid var(--border);
        border-radius: 4px;
        padding: 0.6rem 0.75rem;
        font-family: var(--font-mono);
        font-size: 0.8rem;
        color: var(--accent);
        word-break: break-all;
        margin: 0.5rem 0;
        user-select: all;
    }
    .tracker-setup-actions {
        display: flex;
        gap: 0.5rem;
        flex-wrap: wrap;
        align-items: center;
        margin-top: 0.75rem;
    }
"#;

/// First-run sync setup is documented in the stat-tracker readme (Running).
const TRACKER_SETUP_DOC_PATH: &str = "crates/stat-tracker/README.md";
const TRACKER_SETUP_DOC_HREF: &str =
    "https://github.com/FrozenTear/scuffed-crew/blob/main/crates/stat-tracker/README.md";
const TRACKER_SETUP_TITLE: &str = "Set up the tracker";
/// Production origin when the page has no browser location (tests, non-web).
const TRACKER_SITE_FALLBACK: &str = "https://ow.scuffedcrew.no";
const COPY_FAILED_TOAST: &str = "Couldn't copy. Select the token and copy it by hand.";

fn tracker_site_address() -> String {
    // `location.origin()` touches wasm-only js imports, so host tests use the fallback.
    #[cfg(target_arch = "wasm32")]
    if let Some(origin) = web_sys::window().and_then(|window| window.location().origin().ok())
        && origin != "null"
        && (origin.starts_with("https://") || origin.starts_with("http://"))
    {
        return origin;
    }
    TRACKER_SITE_FALLBACK.to_string()
}

fn tracker_setup_body(site: &str) -> String {
    format!(
        "In the tracker app, open Settings, Website sync, and paste it as the Account token along with the site address ({site}). Newer versions also ask for it in the first-run setup."
    )
}

/// Clipboard text for a newly created token. The value is copied exactly.
fn token_clipboard_text(token: &str) -> String {
    token.to_string()
}

fn dispatch_copy(token: &str, on_copy: EventHandler<String>) {
    on_copy.call(token_clipboard_text(token));
}

fn format_date(dt: &DateTime<Utc>) -> String {
    dt.format("%b %d, %Y %H:%M").to_string()
}

async fn copy_daemon_token(text: &str) -> Result<(), &'static str> {
    use wasm_bindgen::{JsCast, JsValue};

    let window = web_sys::window().ok_or("no window")?;
    let navigator = js_sys::Reflect::get(&window, &JsValue::from_str("navigator"))
        .map_err(|_| "no navigator")?;
    if navigator.is_null() || navigator.is_undefined() {
        return Err("no navigator");
    }
    let clipboard = js_sys::Reflect::get(&navigator, &JsValue::from_str("clipboard"))
        .map_err(|_| "no clipboard")?;
    if clipboard.is_null() || clipboard.is_undefined() {
        return Err("no clipboard");
    }
    let write = js_sys::Reflect::get(&clipboard, &JsValue::from_str("writeText"))
        .map_err(|_| "no clipboard")?;
    let write = write
        .dyn_into::<js_sys::Function>()
        .map_err(|_| "no clipboard")?;
    let promise = write
        .call1(&clipboard, &JsValue::from_str(text))
        .map_err(|_| "copy failed")?;
    let promise = promise
        .dyn_into::<js_sys::Promise>()
        .map_err(|_| "copy failed")?;
    wasm_bindgen_futures::JsFuture::from(promise)
        .await
        .map_err(|_| "copy failed")?;
    Ok(())
}

#[component]
fn TrackerSetupBox(
    token: Option<String>,
    on_copy: EventHandler<String>,
    on_dismiss: EventHandler<MouseEvent>,
) -> Element {
    let class = if token.is_some() {
        "tracker-setup has-token"
    } else {
        "tracker-setup"
    };
    let setup_body = token
        .as_ref()
        .map(|_| tracker_setup_body(&tracker_site_address()));
    let shown = token.clone();
    rsx! {
        div { class: "{class}",
            h2 { "{TRACKER_SETUP_TITLE}" }
            if let Some(body) = setup_body {
                p { "{body}" }
            }
            p {
                a {
                    href: "{TRACKER_SETUP_DOC_HREF}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "Stat tracker setup"
                }
            }
            if let Some(raw) = shown {
                p { "Your new token:" }
                code { "{raw}" }
                div { class: "tracker-setup-actions",
                    button {
                        class: "btn-add",
                        onclick: move |_| dispatch_copy(&raw, on_copy),
                        "Copy token"
                    }
                    button {
                        class: "btn-cancel",
                        onclick: move |evt| on_dismiss.call(evt),
                        "Dismiss"
                    }
                }
            }
        }
    }
}

#[component]
pub fn StatsTokens() -> Element {
    let auth = use_auth();
    let mut tokens = use_api::<Vec<DaemonToken>>("/api/stats/tokens");
    let mut toast = use_toast();

    let mut modal = ModalController::<String>::new();
    let mut form_label = use_signal(|| "default".to_string());

    let mut revealed_token: Signal<Option<String>> = use_signal(|| None);

    let open_create = move |_| {
        form_label.set("default".to_string());
        modal.show_empty();
    };

    let on_close = move |_| {
        modal.close();
    };

    let on_submit = move |_| {
        let label = form_label().trim().to_string();
        if label.is_empty() {
            return;
        }

        modal.start_submit();
        spawn(async move {
            let body = CreateDaemonTokenRequest { label };
            let result = ApiClient::web()
                .post_json::<_, CreateDaemonTokenResponse>("/api/stats/tokens", &body)
                .await;

            modal.end_submit();
            match result {
                Ok(resp) => {
                    toast.show(Toast::success("Token created."));
                    revealed_token.set(Some(resp.token));
                    modal.close();
                    tokens.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed: {e}")));
                }
            }
        });
    };

    let on_copy = move |raw: String| {
        spawn(async move {
            if copy_daemon_token(&raw).await.is_ok() {
                toast.show(Toast::success("Token copied."));
            } else {
                toast.show(Toast::error(COPY_FAILED_TOAST));
            }
        });
    };

    let on_dismiss = move |_: MouseEvent| {
        revealed_token.set(None);
    };

    let on_revoke = move |token_id: String| {
        spawn(async move {
            let result = ApiClient::web()
                .delete(&format!("/api/stats/tokens/{token_id}"))
                .await;
            match result {
                Ok(()) => {
                    toast.show(Toast::success("Token revoked."));
                    tokens.refresh += 1;
                }
                Err(e) => {
                    toast.show(Toast::error(format!("Failed: {e}")));
                }
            }
        });
    };

    rsx! {
        style { {TOKENS_CSS} }
        style { {crate::styles::admin::CSS} }

        div { class: "tokens-page",
            div { class: "admin-toolbar",
                h1 { "Daemon Tokens" }
                button { class: "btn-add", onclick: open_create, "+ New Token" }
            }

            TrackerSetupBox {
                token: revealed_token(),
                on_copy: on_copy,
                on_dismiss: on_dismiss,
            }

            {
                let data = tokens.data.read();
                let data = data.as_ref().and_then(|d| d.as_ref());
                match data {
                    None => member_pending(&auth(), &tokens, "daemon tokens"),
                    Some(list) if list.is_empty() => rsx! {
                        p { class: "empty-state", "No daemon tokens yet. Create one to start uploading stats." }
                    },
                    Some(list) => rsx! {
                        DataTable { headers: vec!["Label", "Status", "Created", "Last Used", "Actions"],
                            for token in list.iter() {
                                {
                                    let tid = token.id.clone();
                                    let status = if token.is_active { "Active" } else { "Revoked" };
                                    let status_class = if token.is_active { "status-pill active" } else { "status-pill inactive" };
                                    let created = format_date(&token.created_at);
                                    let used = token.last_used_at.as_ref().map(format_date).unwrap_or_else(|| "Never".into());
                                    rsx! {
                                        tr { key: "{token.id}",
                                            td { "{token.label}" }
                                            td { span { class: "{status_class}", "{status}" } }
                                            td { "{created}" }
                                            td { "{used}" }
                                            td {
                                                if token.is_active {
                                                    div { class: "row-actions",
                                                        button {
                                                            class: "row-btn danger",
                                                            onclick: move |_| on_revoke(tid.clone()),
                                                            "Revoke"
                                                        }
                                                    }
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    },
                }
            }

            FormModal {
                title: "Create Daemon Token".to_string(),
                open: modal.is_open(),
                submitting: modal.is_submitting(),
                on_close: on_close,
                on_submit: on_submit,

                div { class: "form-field",
                    label { class: "form-label", r#for: "daemon-token-label", "Label" }
                    input {
                        id: "daemon-token-label",
                        name: "daemon-token-label",
                        class: "form-input",
                        r#type: "text",
                        placeholder: "e.g. my-desktop",
                        value: "{form_label}",
                        oninput: move |e| form_label.set(e.value()),
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    std::thread_local! {
        static COPIED_TOKEN: std::cell::RefCell<Option<String>> =
            const { std::cell::RefCell::new(None) };
    }

    fn render(root: fn() -> Element) -> String {
        let mut dom = VirtualDom::new(root);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    fn assert_plain_copy(text: &str) {
        assert!(!text.contains('\u{2014}'), "em dash in {text}");
        assert!(!text.contains('\u{2013}'), "en dash in {text}");
    }

    fn assert_readme_link_noopener(html: &str) {
        let href_at = html.find(TRACKER_SETUP_DOC_HREF).expect("readme link");
        let end = (href_at + TRACKER_SETUP_DOC_HREF.len() + 160).min(html.len());
        let tag = &html[href_at.saturating_sub(80)..end];
        assert!(
            tag.contains("noopener"),
            "readme link missing rel noopener: {tag}"
        );
    }

    #[test]
    fn setup_copy_points_at_the_stat_tracker_readme() {
        let body = tracker_setup_body(TRACKER_SITE_FALLBACK);
        assert_eq!(TRACKER_SETUP_TITLE, "Set up the tracker");
        assert_eq!(
            body,
            "In the tracker app, open Settings, Website sync, and paste it as the Account token along with the site address (https://ow.scuffedcrew.no). Newer versions also ask for it in the first-run setup."
        );
        assert_eq!(
            COPY_FAILED_TOAST,
            "Couldn't copy. Select the token and copy it by hand."
        );
        assert_eq!(TRACKER_SETUP_DOC_PATH, "crates/stat-tracker/README.md");
        assert!(TRACKER_SETUP_DOC_HREF.contains(TRACKER_SETUP_DOC_PATH));
        assert_plain_copy(TRACKER_SETUP_TITLE);
        assert_plain_copy(&body);
        assert_plain_copy(COPY_FAILED_TOAST);
        let readme =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../stat-tracker/README.md");
        assert!(
            readme.is_file(),
            "setup doc missing at {}",
            readme.display()
        );
    }

    #[test]
    fn setup_box_hides_paste_text_until_a_token_exists() {
        fn view() -> Element {
            rsx! {
                TrackerSetupBox {
                    token: None,
                    on_copy: |_| {},
                    on_dismiss: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("Set up the tracker"), "{html}");
        assert!(html.contains(TRACKER_SETUP_DOC_PATH), "{html}");
        assert_readme_link_noopener(&html);
        assert!(!html.contains("Account token"), "{html}");
        assert!(!html.contains("Paste this token"), "{html}");
        assert!(!html.contains("first-run setup"), "{html}");
        assert!(!html.contains(">Copy token</button>"), "{html}");
        assert_plain_copy(&html);
    }

    #[test]
    fn setup_box_offers_copy_for_a_new_token() {
        fn view() -> Element {
            rsx! {
                TrackerSetupBox {
                    token: Some("sst_test_token".to_string()),
                    on_copy: |_| {},
                    on_dismiss: |_| {},
                }
            }
        }
        let html = render(view);
        assert!(html.contains("sst_test_token"), "{html}");
        assert!(html.contains(">Copy token</button>"), "{html}");
        assert!(html.contains(TRACKER_SITE_FALLBACK), "{html}");
        assert!(html.contains("Website sync"), "{html}");
        assert!(html.contains("Account token"), "{html}");
        assert!(html.contains("first-run setup"), "{html}");
        assert!(html.contains(TRACKER_SETUP_DOC_PATH), "{html}");
        assert_readme_link_noopener(&html);
        assert_plain_copy(&html);
    }

    fn copy_probe() -> Element {
        let token = "  sst_exact/token  ";
        let handler = EventHandler::new(|value: String| {
            COPIED_TOKEN.with(|slot| *slot.borrow_mut() = Some(value));
        });
        dispatch_copy(token, handler);
        rsx! { "" }
    }

    #[test]
    fn copy_copies_exactly_the_new_token_value() {
        let token = "  sst_exact/token  ";
        assert_eq!(token_clipboard_text(token), token);
        assert_ne!(token.trim(), token);
        COPIED_TOKEN.with(|slot| *slot.borrow_mut() = None);
        let mut dom = VirtualDom::new(copy_probe);
        dom.rebuild_in_place();
        let copied = COPIED_TOKEN.with(|slot| slot.borrow().clone());
        assert_eq!(copied.as_deref(), Some(token));
    }
}
