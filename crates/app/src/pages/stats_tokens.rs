use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{CreateDaemonTokenRequest, CreateDaemonTokenResponse};

use super::stats_member::{Col, STATS_VIEWS_CSS, STATS_VIEWS_CSS_ID, StatsTable};
use crate::components::ui::{EmptyCard, PageHeader, PageShell, PageWidth};
use crate::components::{FormModal, Toast, member_pending, use_toast};
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
.tokens-page .tracker-setup {
  box-sizing: border-box;
  display: flex;
  flex-direction: column;
  gap: var(--space-3);
  min-width: 0;
  padding: var(--card-padding);
  background: var(--card-bg);
  color: var(--card-fg);
  border: 1px solid var(--card-border);
  border-radius: var(--card-radius);
  box-shadow: var(--card-shadow);
}
/* A new token is on show: a stronger border plus the words "Your new token". */
.tokens-page .tracker-setup.has-token { border-color: var(--accent-solid); }
.tokens-page .tracker-setup p {
  margin: 0;
  max-width: var(--content-max-narrow);
  color: var(--text-3);
  font-family: var(--font-body);
  font-size: var(--text-md);
  line-height: var(--leading-body);
}
.tokens-page .tracker-setup a {
  display: inline-flex;
  align-items: center;
  min-height: var(--hit-target-min);
  color: var(--accent-text);
  text-decoration: underline;
}
.tokens-page .tracker-setup a:focus-visible {
  outline: var(--focus-ring-width) solid var(--focus-ring);
  outline-offset: var(--focus-ring-offset);
}
.tokens-page .tracker-setup code {
  display: block;
  padding: var(--space-3);
  background: var(--surface-sunken);
  border: 1px solid var(--border);
  border-radius: var(--radius-control);
  font-family: var(--font-mono);
  font-size: var(--text-sm);
  color: var(--text);
  overflow-wrap: anywhere;
  user-select: all;
}
.tokens-page .tracker-setup-actions {
  display: flex;
  flex-wrap: wrap;
  align-items: center;
  gap: var(--space-2);
}
.tokens-page .token-date { white-space: nowrap; color: var(--table-row-muted); }
/* The dialog may render outside the page container. */
.token-form-field {
  display: flex;
  flex-direction: column;
  gap: var(--space-2);
  min-width: 0;
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

/// "12 Nov 2026, 20:00 UTC", the date style of the frames.
fn format_date(dt: &DateTime<Utc>) -> String {
    dt.format("%-d %b %Y, %H:%M UTC").to_string()
}

/// Id of a row's Revoke button: the token id plus the action, never a counter.
fn revoke_button_id(token_id: &str) -> String {
    let safe: String = token_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    format!("token-{safe}-revoke")
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
        section { class: "{class}", aria_labelledby: "tracker-setup-title",
            h2 { id: "tracker-setup-title", class: "ui-section-title", "{TRACKER_SETUP_TITLE}" }
            if let Some(body) = setup_body {
                p { "{body}" }
            }
            p {
                a {
                    href: "{TRACKER_SETUP_DOC_HREF}",
                    target: "_blank",
                    rel: "noopener noreferrer",
                    "Stat tracker setup"
                    span { class: "visually-hidden", " (opens in a new tab)" }
                }
            }
            if let Some(raw) = shown {
                p { id: "tracker-new-token-label", "Your new token. Copy it now: it is shown only once." }
                code { aria_labelledby: "tracker-new-token-label", "{raw}" }
                div { class: "tracker-setup-actions",
                    button {
                        r#type: "button",
                        class: "btn is-primary",
                        onclick: move |_| dispatch_copy(&raw, on_copy),
                        "Copy token"
                    }
                    button {
                        r#type: "button",
                        class: "btn is-ghost",
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

    let on_revoke = EventHandler::new(on_revoke);

    // Read once here, so no read guard is held while the children render.
    let token_count = tokens
        .data
        .read()
        .as_ref()
        .and_then(|d| d.as_ref())
        .map(Vec::len);
    let status = match token_count {
        None => String::new(),
        Some(0) => "No tracker tokens yet.".to_string(),
        Some(1) => "1 tracker token.".to_string(),
        Some(n) => format!("{n} tracker tokens."),
    };

    rsx! {
        crate::styles::StyleOnce { id: STATS_VIEWS_CSS_ID, css: STATS_VIEWS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-pages-stats_tokens-tokens-css", css: TOKENS_CSS }
        crate::styles::StyleOnce { id: "scuffed-css-admin-shared", css: crate::styles::admin::CSS }

        PageShell { width: PageWidth::Default, class: "tokens-page",
            PageHeader {
                title: "Tracker tokens",
                lead: "A token lets the stat tracker upload your games to this site. Revoke one and the tracker using it stops uploading.",
                actions: rsx! {
                    button {
                        r#type: "button",
                        id: "tokens-new",
                        class: "btn is-primary",
                        onclick: open_create,
                        "New token"
                    }
                },
            }

            TrackerSetupBox {
                token: revealed_token(),
                on_copy: on_copy,
                on_dismiss: on_dismiss,
            }

            section { class: "ui-stack", aria_labelledby: "tokens-list-title",
                h2 { id: "tokens-list-title", class: "ui-section-title", "Your tokens" }
                p { class: "visually-hidden", role: "status", "{status}" }
                {
                    let data = tokens.data.read();
                    let data = data.as_ref().and_then(|d| d.as_ref());
                    match data {
                        None => member_pending(&auth(), &tokens, "tracker tokens"),
                        Some(list) if list.is_empty() => rsx! {
                            EmptyCard {
                                title: "No tracker tokens yet",
                                message: "Create a token, then paste it into the tracker to start uploading your games.",
                                actions: rsx! {
                                    button {
                                        r#type: "button",
                                        id: "tokens-new-empty",
                                        class: "btn is-primary",
                                        onclick: open_create,
                                        "Create a token"
                                    }
                                },
                            }
                        },
                        Some(list) => rsx! {
                            StatsTable {
                                labelled_by: "tokens-list-title",
                                wide: true,
                                cols: vec![
                                    Col::Text("Label"),
                                    Col::Text("Status"),
                                    Col::Text("Created"),
                                    Col::Text("Last used"),
                                    Col::Actions("Actions"),
                                ],
                                for token in list.iter() {
                                    {token_row(token, on_revoke)}
                                }
                            }
                        },
                    }
                }
            }

            FormModal {
                title: "Create tracker token".to_string(),
                open: modal.is_open(),
                submitting: modal.is_submitting(),
                on_close: on_close,
                on_submit: on_submit,

                div { class: "token-form-field",
                    label { class: "field-label", r#for: "daemon-token-label", "Label" }
                    input {
                        id: "daemon-token-label",
                        name: "daemon-token-label",
                        class: "field-input",
                        r#type: "text",
                        placeholder: "For example my-desktop",
                        value: "{form_label}",
                        oninput: move |e| form_label.set(e.value()),
                    }
                }
            }
        }
    }
}

/// One token row. The status is a word on a soft fill, and the Revoke button
/// names the token it acts on.
fn token_row(token: &DaemonToken, on_revoke: EventHandler<String>) -> Element {
    let tid = token.id.clone();
    let (status, status_class) = if token.is_active {
        ("Active", "status-pill active")
    } else {
        ("Revoked", "status-pill inactive")
    };
    let created = format_date(&token.created_at);
    let created_at = token.created_at.to_rfc3339();
    let used = token.last_used_at.as_ref().map(format_date);
    let used_at = token.last_used_at.as_ref().map(DateTime::to_rfc3339);
    let revoke_id = revoke_button_id(&token.id);
    rsx! {
        tr { key: "{token.id}",
            th { scope: "row", "{token.label}" }
            td { span { class: "{status_class}", "{status}" } }
            td { class: "token-date", time { datetime: "{created_at}", "{created}" } }
            td { class: "token-date",
                if let (Some(used), Some(used_at)) = (used, used_at) {
                    time { datetime: "{used_at}", "{used}" }
                } else {
                    "Never"
                }
            }
            td { class: "stats-table-actions",
                if token.is_active {
                    button {
                        r#type: "button",
                        id: "{revoke_id}",
                        class: "btn is-ghost",
                        aria_label: "Revoke token {token.label}",
                        onclick: move |_| on_revoke.call(tid.clone()),
                        "Revoke"
                    }
                } else {
                    span { class: "visually-hidden", "No actions" }
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

    fn token(id: &str, active: bool, used: bool) -> DaemonToken {
        use chrono::TimeZone;
        let at = Utc.with_ymd_and_hms(2026, 11, 2, 20, 5, 0).unwrap();
        DaemonToken {
            id: id.to_string(),
            member_id: "member:1".to_string(),
            label: format!("pc-{id}"),
            is_active: active,
            created_at: at,
            last_used_at: used.then_some(at),
        }
    }

    #[test]
    fn dates_read_like_the_frames() {
        use chrono::TimeZone;
        let at = Utc.with_ymd_and_hms(2026, 11, 2, 20, 5, 0).unwrap();
        assert_eq!(format_date(&at), "2 Nov 2026, 20:05 UTC");
    }

    #[test]
    fn rows_say_the_status_in_words_and_name_the_revoke_button() {
        fn view() -> Element {
            rsx! {
                h2 { id: "tokens-list-title", "Your tokens" }
                StatsTable {
                    labelled_by: "tokens-list-title",
                    wide: true,
                    cols: vec![Col::Text("Label"), Col::Text("Status"), Col::Text("Created"), Col::Text("Last used"), Col::Actions("Actions")],
                    {token_row(&token("a:1", true, true), EventHandler::new(|_: String| {}))}
                    {token_row(&token("b2", false, false), EventHandler::new(|_: String| {}))}
                }
            }
        }
        let html = render(view);
        assert!(
            html.contains("<span class=\"status-pill active\">Active</span>"),
            "{html}"
        );
        assert!(
            html.contains("<span class=\"status-pill inactive\">Revoked</span>"),
            "{html}"
        );
        assert!(
            html.contains(
                "<time datetime=\"2026-11-02T20:05:00+00:00\">2 Nov 2026, 20:05 UTC</time>"
            ),
            "{html}"
        );
        assert!(html.contains("Never"), "{html}");
        // Only the active token can be revoked. Its button id comes from the
        // token id, and its name says which token it revokes.
        assert_eq!(html.matches("<button").count(), 1, "{html}");
        assert!(html.contains("id=\"token-a_1-revoke\""), "{html}");
        assert!(
            html.contains("aria-label=\"Revoke token pc-a:1\""),
            "{html}"
        );
        assert!(
            html.contains("role=\"region\" aria-labelledby=\"tokens-list-title\" tabindex=\"0\""),
            "{html}"
        );
        assert!(!html.contains("<a "), "no link wraps a button: {html}");
        // The Actions header is right-aligned over the Revoke buttons.
        assert!(
            html.contains("class=\"stats-table-actions\">Actions</th>"),
            "{html}"
        );
        assert_plain_copy(&html);
    }

    #[test]
    fn page_css_uses_tokens_only() {
        for line in TOKENS_CSS.lines() {
            let decl = line.trim();
            if decl.starts_with("/*") || !decl.contains(':') {
                continue;
            }
            assert!(!decl.contains("rem") && !decl.contains('#'), "{decl}");
            assert!(!decl.contains("px") || decl.contains("1px solid"), "{decl}");
        }
        let src = include_str!("stats_tokens.rs");
        assert!(!src.contains(&["<", "main"].concat()) && !src.contains(&["main", " {"].concat()));
    }

    #[test]
    fn copy_says_tracker_tokens() {
        let src = include_str!("stats_tokens.rs");
        assert!(!src.contains(&["Daemon", " Token"].concat()));
    }
}
