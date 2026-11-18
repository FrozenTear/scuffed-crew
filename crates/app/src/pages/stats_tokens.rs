use dioxus::prelude::*;

use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_api_client::ApiClient;
use scuffed_types::api::{CreateDaemonTokenRequest, CreateDaemonTokenResponse};

use super::stats_member::{Col, STATS_VIEWS_CSS, STATS_VIEWS_CSS_ID, StatsTable};
use crate::components::ui::inline_confirm::{focus_element_by_id_soon, key_cancels};
use crate::components::ui::{
    EmptyCard, FieldError, InlineConfirm, PageHeader, PageShell, PageWidth,
};
use crate::components::{FormModal, Toast, member_pending, use_toast};
use crate::hooks::{ModalController, use_api};
use crate::layouts::focus_element;
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
/* The new token: its caption reads like the card's other lines. */
.tokens-page .tracker-new-token {
  display: grid;
  gap: var(--space-3);
  margin: 0;
  min-width: 0;
}
.tokens-page .tracker-setup p,
.tokens-page .tracker-new-token figcaption {
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
/* The Revoke confirm strip sits in the Actions cell in place of the button.
   The cell is nowrap; the strip wraps, and its title repeats the label. */
.tokens-page .token-revoke-confirm { min-width: 15em; white-space: normal; text-align: start; }
.tokens-page .token-revoke-confirm .inline-confirm__title { overflow-wrap: anywhere; }
/* The shared strip's buttons are 38px tall; here they meet the 44px target
   every other control on the page has, phone widths included. */
.tokens-page .token-revoke-confirm .inline-confirm__cancel,
.tokens-page .token-revoke-confirm .inline-confirm__go { min-height: var(--hit-target-min); }
/* Catches Esc from anywhere in the New token form; adds no box. */
.tokens-page .token-form-keys { display: contents; }
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

/// "12 Nov 2026, 21:00 UTC+1": the date style of the frames on the
/// viewer's own clock, with its offset, as the stats match page shows a
/// game's time.
fn format_date(dt: &DateTime<Utc>) -> String {
    super::stats::local_date_time(dt)
}

/// The "Your tokens" heading. After a Revoke, the row's button is gone, so
/// focus moves here instead of falling to the body.
const TOKENS_LIST_TITLE_ID: &str = "tokens-list-title";
/// The label input in the New token form, and its error line.
const LABEL_INPUT_ID: &str = "daemon-token-label";
const LABEL_ERROR_ID: &str = "daemon-token-label-err";
/// The two buttons that open the New token form: the header's, and the
/// empty list's. Esc or Cancel gives focus back to the one that opened it.
const NEW_TOKEN_BUTTON_ID: &str = "tokens-new";
const NEW_TOKEN_EMPTY_BUTTON_ID: &str = "tokens-new-empty";

/// Why the New token form refuses a label, if it does. The server needs a
/// label to tell tokens apart.
fn label_error(label: &str) -> Option<&'static str> {
    label
        .trim()
        .is_empty()
        .then_some("Give the token a label, for example my-desktop.")
}

/// Id prefix of a row's Revoke confirm strip: the token id plus the action,
/// never a counter. The row's Revoke button is `{prefix}-trigger`, the id
/// `InlineConfirm` gives focus back to on Cancel or Esc.
fn revoke_confirm_id(token_id: &str) -> String {
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

/// Id of a row's Revoke button.
fn revoke_button_id(token_id: &str) -> String {
    format!("{}-trigger", revoke_confirm_id(token_id))
}

/// The confirm strip's question, naming the token.
fn revoke_confirm_title(label: &str) -> String {
    format!("Revoke token {label}?")
}

const REVOKE_CONFIRM_BODY: &str = "The tracker using it stops uploading. This cannot be undone.";

/// After the strip renders, scroll all of it into view. Focus lands on
/// Cancel, which only brings Cancel into the table's sideways scroller; on
/// a phone the strip's Revoke would stay past the scroller's edge.
fn reveal_revoke_confirm_soon(prefix: String) {
    dioxus::core::spawn_forever(async move {
        #[cfg(feature = "web")]
        {
            gloo_timers::future::TimeoutFuture::new(0).await;
        }
        #[cfg(target_arch = "wasm32")]
        {
            use wasm_bindgen::JsCast;
            let Some(strip) = web_sys::window()
                .and_then(|window| window.document())
                .and_then(|document| document.get_element_by_id(&format!("{prefix}-title")))
                .and_then(|title| title.closest(".inline-confirm").ok().flatten())
            else {
                return;
            };
            let options = js_sys::Object::new();
            let _ = js_sys::Reflect::set(&options, &"block".into(), &"nearest".into());
            let _ = js_sys::Reflect::set(&options, &"inline".into(), &"nearest".into());
            let scroll = js_sys::Reflect::get(&strip, &"scrollIntoView".into()).ok();
            if let Some(scroll) = scroll
                .as_ref()
                .and_then(|f| f.dyn_ref::<js_sys::Function>())
            {
                let _ = scroll.call1(&strip, &options);
            }
        }
        #[cfg(not(target_arch = "wasm32"))]
        {
            let _ = prefix;
        }
    });
}

/// What the row's Actions cell shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RevokeCell {
    /// A revoked token: nothing to do.
    None,
    /// The Revoke button.
    Button,
    /// The confirm strip in place of the button.
    Confirm,
}

fn revoke_cell(token: &DaemonToken, confirming: Option<&str>) -> RevokeCell {
    if !token.is_active {
        RevokeCell::None
    } else if confirming == Some(token.id.as_str()) {
        RevokeCell::Confirm
    } else {
        RevokeCell::Button
    }
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
                // `aria-labelledby` does not name a `code` element, so the
                // label and the token are one figure with a caption.
                figure { class: "tracker-new-token",
                    figcaption { "Your new token. Copy it now: it is shown only once." }
                    code { "{raw}" }
                }
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
    // The label's error line. Always rendered with the form; only its text
    // changes, so a new error is announced.
    let mut form_error = use_signal(String::new);

    let mut revealed_token: Signal<Option<String>> = use_signal(|| None);

    // The button that opened the form, so closing it can give focus back.
    let mut form_opener = use_signal(|| NEW_TOKEN_BUTTON_ID);

    let mut open_create = move |opener: &'static str| {
        form_label.set("default".to_string());
        form_error.set(String::new());
        form_opener.set(opener);
        modal.show_empty();
        // Into the form, so Esc and Tab act on it rather than on the page.
        focus_element_by_id_soon(LABEL_INPUT_ID.to_string());
    };

    // Cancel, a click outside, or Esc: close and return focus to the opener.
    let mut close_form = move || {
        modal.close();
        focus_element_by_id_soon(form_opener().to_string());
    };

    let on_close = move |_| close_form();

    let on_submit = move |_| {
        let label = form_label().trim().to_string();
        if let Some(error) = label_error(&label) {
            form_error.set(error.to_string());
            focus_element(LABEL_INPUT_ID);
            return;
        }
        form_error.set(String::new());

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

    // The token whose Revoke confirm strip is open, and whether its DELETE
    // is in flight. Revoke only opens the strip; the strip's Revoke sends.
    let mut confirming: Signal<Option<String>> = use_signal(|| None);
    let mut revoking = use_signal(|| false);

    let on_revoke = move |token_id: String| {
        if revoking() {
            return;
        }
        revoking.set(true);
        spawn(async move {
            let result = ApiClient::web()
                .delete(&format!("/api/stats/tokens/{token_id}"))
                .await;
            revoking.set(false);
            match result {
                Ok(()) => {
                    confirming.set(None);
                    toast.show(Toast::success("Token revoked."));
                    focus_element(TOKENS_LIST_TITLE_ID);
                    tokens.refresh += 1;
                }
                Err(e) => {
                    // The strip stays open, focus stays on its Revoke.
                    toast.show(Toast::error(format!("Failed: {e}")));
                }
            }
        });
    };

    let actions = RevokeActions {
        confirming: confirming.read().clone(),
        busy: revoking(),
        on_ask: EventHandler::new(move |token_id: String| {
            reveal_revoke_confirm_soon(revoke_confirm_id(&token_id));
            confirming.set(Some(token_id));
        }),
        on_confirm: EventHandler::new(on_revoke),
        on_cancel: EventHandler::new(move |_: ()| confirming.set(None)),
    };

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
                        id: NEW_TOKEN_BUTTON_ID,
                        class: "btn is-primary",
                        onclick: move |_| open_create(NEW_TOKEN_BUTTON_ID),
                        "New token"
                    }
                },
            }

            TrackerSetupBox {
                token: revealed_token(),
                on_copy: on_copy,
                on_dismiss: on_dismiss,
            }

            section { class: "ui-stack", aria_labelledby: TOKENS_LIST_TITLE_ID,
                h2 {
                    id: TOKENS_LIST_TITLE_ID,
                    class: "ui-section-title",
                    tabindex: "-1",
                    "Your tokens"
                }
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
                                        id: NEW_TOKEN_EMPTY_BUTTON_ID,
                                        class: "btn is-primary",
                                        onclick: move |_| open_create(NEW_TOKEN_EMPTY_BUTTON_ID),
                                        "Create a token"
                                    }
                                },
                            }
                        },
                        Some(list) => rsx! {
                            StatsTable {
                                labelled_by: TOKENS_LIST_TITLE_ID,
                                wide: true,
                                cols: vec![
                                    Col::Text("Label"),
                                    Col::Text("Status"),
                                    Col::Text("Created"),
                                    Col::Text("Last used"),
                                    Col::Actions("Actions"),
                                ],
                                for token in list.iter() {
                                    Fragment { key: "{token.id}", {token_row(token, &actions)} }
                                }
                            }
                        },
                    }
                }
            }

            // FormModal has no Esc of its own; this page's form closes on Esc
            // like the Revoke strip does.
            div {
                class: "token-form-keys",
                onkeydown: move |e| {
                    if modal.is_open() && key_cancels(&e.key()) {
                        e.stop_propagation();
                        close_form();
                    }
                },
                FormModal {
                    title: "Create tracker token".to_string(),
                    open: modal.is_open(),
                    submitting: modal.is_submitting(),
                    on_close: on_close,
                    on_submit: on_submit,

                    div { class: "token-form-field",
                        label { class: "field-label", r#for: LABEL_INPUT_ID, "Label" }
                        input {
                            id: LABEL_INPUT_ID,
                            name: LABEL_INPUT_ID,
                            class: "field-input",
                            r#type: "text",
                            placeholder: "For example my-desktop",
                            aria_invalid: if !form_error().is_empty() { "true" },
                            aria_describedby: LABEL_ERROR_ID,
                            value: "{form_label}",
                            oninput: move |e| {
                                form_label.set(e.value());
                                if !form_error.peek().is_empty() && label_error(&e.value()).is_none() {
                                    form_error.set(String::new());
                                }
                            },
                        }
                        FieldError { id: LABEL_ERROR_ID, text: form_error(), role: "status" }
                    }
                }
            }
        }
    }
}

/// The Revoke state and handlers every row shares.
#[derive(Clone)]
struct RevokeActions {
    /// Id of the token whose confirm strip is open.
    confirming: Option<String>,
    /// The confirmed DELETE is in flight.
    busy: bool,
    /// Revoke pressed: open the strip for this token.
    on_ask: EventHandler<String>,
    /// The strip's Revoke: send the DELETE for this token.
    on_confirm: EventHandler<String>,
    /// Cancel or Esc. `InlineConfirm` moves focus back to the Revoke button.
    on_cancel: EventHandler<()>,
}

/// One token row. The status is a word on a soft fill, and the Revoke button
/// names the token it acts on. Revoke opens a confirm strip in its place.
fn token_row(token: &DaemonToken, actions: &RevokeActions) -> Element {
    let tid = token.id.clone();
    let confirm_tid = token.id.clone();
    let on_ask = actions.on_ask;
    let on_confirm = actions.on_confirm;
    let on_cancel = actions.on_cancel;
    let cell = revoke_cell(token, actions.confirming.as_deref());
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
    let confirm_id = revoke_confirm_id(&token.id);
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
                match cell {
                    RevokeCell::Button => rsx! {
                        button {
                            r#type: "button",
                            id: "{revoke_id}",
                            class: "btn is-ghost",
                            aria_label: "Revoke token {token.label}",
                            onclick: move |_| on_ask.call(tid.clone()),
                            "Revoke"
                        }
                    },
                    RevokeCell::Confirm => rsx! {
                        div { class: "token-revoke-confirm",
                            InlineConfirm {
                                id: confirm_id,
                                title: revoke_confirm_title(&token.label),
                                body: REVOKE_CONFIRM_BODY.to_string(),
                                confirm_label: "Revoke".to_string(),
                                busy: actions.busy,
                                on_confirm: move |_| on_confirm.call(confirm_tid.clone()),
                                on_cancel: move |_| on_cancel.call(()),
                            }
                        }
                    },
                    RevokeCell::None => rsx! {
                        span { class: "visually-hidden", "No actions" }
                    },
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
    fn the_new_token_is_named_by_a_caption_not_by_aria_on_code() {
        let src = include_str!("stats_tokens.rs");
        let body = &src[..src.find("#[cfg(test)]").unwrap()];
        let at = body.find("code { \"{raw}\" }").expect("token");
        let figure = &body[body[..at].rfind("figure {").expect("figure")..at];
        assert!(figure.contains("figcaption {"), "{figure}");
        assert!(
            !body.contains("code { aria_labelledby"),
            "no aria name on code"
        );
        let rule = crate::components::ui::test_support::css_rule(
            TOKENS_CSS,
            ".tokens-page .tracker-new-token {",
        );
        assert!(
            crate::components::ui::test_support::has_decl(&rule, "margin: 0"),
            "{rule}"
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
        // Native tests run on UTC; the browser uses the viewer's clock.
        assert_eq!(format_date(&at), "2 Nov 2026, 20:05 UTC");
    }

    /// QA 705 nit 3: Created and Last used are on the viewer's clock with
    /// its offset, through the same helper as the stats match page, not a
    /// fixed UTC format.
    #[test]
    fn token_dates_use_the_match_page_local_time() {
        let prod = prod_src();
        let body = &prod[prod.find("fn format_date(").unwrap()..];
        let body = &body[..body.find("\n}\n").unwrap()];
        assert!(body.contains("super::stats::local_date_time(dt)"), "{body}");
        assert!(!prod.contains("UTC\")"), "no date is formatted in UTC here");
        let game = include_str!("stats/game.rs");
        assert!(game.contains("super::local_date_time(played_at)"));
    }

    /// QA 705 nit 1: the shared strip's buttons are 38px; on this page they
    /// take the 44px hit target, scoped to the tokens strip only.
    #[test]
    fn the_revoke_strip_buttons_meet_the_hit_target() {
        assert!(TOKENS_CSS.contains(
            ".tokens-page .token-revoke-confirm .inline-confirm__cancel,\n.tokens-page .token-revoke-confirm .inline-confirm__go { min-height: var(--hit-target-min); }"
        ));
        // The shared component is left as it is.
        assert!(
            !crate::components::ui::inline_confirm::INLINE_CONFIRM_CSS.contains("hit-target-min")
        );
        assert!(prod_src().contains(
            "div { class: \"token-revoke-confirm\",\n                            InlineConfirm {"
        ));
    }

    /// QA 705 nit 2: Esc closes the New token form, as it closes the Revoke
    /// strip, and focus goes back to the button that opened the form.
    #[test]
    fn esc_closes_the_new_token_form_and_returns_focus_to_its_opener() {
        assert!(key_cancels(&Key::Escape));
        assert!(!key_cancels(&Key::Enter));
        let prod = prod_src();
        // Opening records which button opened it and moves focus into the
        // form, so Esc reaches the form's key handler.
        let open = &prod[prod
            .find("let mut open_create = move |opener: &'static str| {")
            .unwrap()..];
        let open = &open[..open.find("\n    };\n").unwrap()];
        assert!(open.contains("form_opener.set(opener);"), "{open}");
        assert!(open.contains("modal.show_empty();"), "{open}");
        assert!(
            open.contains("focus_element_by_id_soon(LABEL_INPUT_ID.to_string());"),
            "{open}"
        );
        assert!(prod.contains("onclick: move |_| open_create(NEW_TOKEN_BUTTON_ID),"));
        assert!(prod.contains("onclick: move |_| open_create(NEW_TOKEN_EMPTY_BUTTON_ID),"));
        assert!(prod.contains("id: NEW_TOKEN_BUTTON_ID,"));
        assert!(prod.contains("id: NEW_TOKEN_EMPTY_BUTTON_ID,"));
        // Closing gives focus back to that button.
        let close = &prod[prod.find("let mut close_form = move || {").unwrap()..];
        let close = &close[..close.find("\n    };\n").unwrap()];
        assert!(close.contains("modal.close();"), "{close}");
        assert!(
            close.contains("focus_element_by_id_soon(form_opener().to_string());"),
            "{close}"
        );
        // Esc anywhere in the form closes it; Cancel and a click outside
        // take the same path.
        let keys = &prod[prod.find("class: \"token-form-keys\",").unwrap()..];
        let keys = &keys[..keys.find("FormModal {").unwrap()];
        assert!(
            keys.contains("if modal.is_open() && key_cancels(&e.key()) {"),
            "{keys}"
        );
        assert!(keys.contains("close_form();"), "{keys}");
        assert!(prod.contains("let on_close = move |_| close_form();"));
        assert!(prod.contains("on_close: on_close,"));
        // The wrapper adds no box to the page.
        assert!(TOKENS_CSS.contains(".tokens-page .token-form-keys { display: contents; }"));
    }

    /// Row actions that do nothing, with the strip open for `confirming`.
    fn noop_actions(confirming: Option<&str>) -> RevokeActions {
        RevokeActions {
            confirming: confirming.map(str::to_string),
            busy: false,
            on_ask: EventHandler::new(|_: String| {}),
            on_confirm: EventHandler::new(|_: String| {}),
            on_cancel: EventHandler::new(|_: ()| {}),
        }
    }

    fn prod_src() -> &'static str {
        include_str!("stats_tokens.rs")
            .split("#[cfg(test)]\nmod tests")
            .next()
            .unwrap()
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
                    {token_row(&token("a:1", true, true), &noop_actions(None))}
                    {token_row(&token("b2", false, false), &noop_actions(None))}
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
        assert!(html.contains("id=\"token-a_1-revoke-trigger\""), "{html}");
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

    /// QA 705 nit 6: Save with an empty label says why, in the field's
    /// error line. The line is always rendered with the form (only its text
    /// changes), the input points at it, and focus goes back to the input.
    #[test]
    fn an_empty_label_is_refused_with_a_message() {
        assert!(label_error("").is_some());
        assert!(label_error("   \t").is_some());
        assert_eq!(label_error(" my-desktop "), None);
        let message = label_error("").unwrap();
        assert!(!message.contains('\u{2014}'), "{message}");

        let src = include_str!("stats_tokens.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let submit = prod
            .split("let on_submit = move |_| {")
            .nth(1)
            .expect("submit handler");
        let refuse = &submit[..submit.find("modal.start_submit();").expect("submit")];
        assert!(
            refuse.contains("form_error.set(error.to_string());"),
            "{refuse}"
        );
        assert!(
            refuse.contains("focus_element(LABEL_INPUT_ID);"),
            "{refuse}"
        );
        assert!(refuse.contains("return;"), "{refuse}");
        let field = prod
            .split("div { class: \"token-form-field\",")
            .nth(1)
            .expect("label field");
        assert!(
            field.contains("aria_describedby: LABEL_ERROR_ID,"),
            "{field}"
        );
        // Unconditional: not inside an `if`, so the region never comes and goes.
        assert!(
            field.contains(
                "}\n                        FieldError { id: LABEL_ERROR_ID, text: form_error(), role: \"status\" }"
            ),
            "{field}"
        );
    }

    /// QA 705 nit 4: a successful Revoke removes the button that had focus,
    /// so focus moves to the list heading, which can take it.
    #[test]
    fn revoke_moves_focus_to_the_list_heading() {
        let src = include_str!("stats_tokens.rs");
        let prod = src.split("#[cfg(test)]\nmod tests").next().unwrap();
        let revoke = prod
            .split("let on_revoke = move |token_id: String| {")
            .nth(1)
            .expect("revoke handler");
        let ok_arm = &revoke[..revoke.find("Err(e) =>").expect("error arm")];
        assert!(
            ok_arm.contains("focus_element(TOKENS_LIST_TITLE_ID);"),
            "{ok_arm}"
        );
        assert!(prod.contains(
            "h2 {\n                    id: TOKENS_LIST_TITLE_ID,\n                    class: \"ui-section-title\",\n                    tabindex: \"-1\","
        ));
    }

    /// FrozenTear: Revoke asks first. Pressing Revoke opens a confirm strip
    /// in the row (in place of the button) and sends nothing; only the
    /// strip's own Revoke sends the DELETE, and on success focus moves to
    /// the "Your tokens" heading.
    #[test]
    fn revoke_asks_before_it_removes_the_token() {
        let active = token("a:1", true, true);
        let other = token("c3", true, false);
        let revoked = token("b2", false, false);
        assert_eq!(revoke_cell(&active, None), RevokeCell::Button);
        assert_eq!(revoke_cell(&active, Some("a:1")), RevokeCell::Confirm);
        assert_eq!(revoke_cell(&other, Some("a:1")), RevokeCell::Button);
        assert_eq!(revoke_cell(&revoked, Some("b2")), RevokeCell::None);

        fn view() -> Element {
            rsx! {
                StatsTable {
                    labelled_by: "tokens-list-title",
                    wide: true,
                    cols: vec![Col::Text("Label"), Col::Text("Status"), Col::Text("Created"), Col::Text("Last used"), Col::Actions("Actions")],
                    {token_row(&token("a:1", true, true), &noop_actions(Some("a:1")))}
                    {token_row(&token("c3", true, false), &noop_actions(Some("a:1")))}
                }
            }
        }
        let html = render(view);
        // The open row: a named group with Cancel and Revoke, no Revoke trigger.
        assert!(!html.contains("id=\"token-a_1-revoke-trigger\""), "{html}");
        assert!(
            html.contains("role=\"group\" aria-labelledby=\"token-a_1-revoke-title\""),
            "{html}"
        );
        assert!(
            html.contains("id=\"token-a_1-revoke-title\" class=\"inline-confirm__title\">Revoke token pc-a:1?</p>"),
            "{html}"
        );
        assert!(html.contains(REVOKE_CONFIRM_BODY), "{html}");
        assert!(
            html.contains("id=\"token-a_1-revoke-cancel\" class=\"inline-confirm__cancel\" type=\"button\">Cancel</button>"),
            "{html}"
        );
        assert!(
            html.contains("class=\"inline-confirm__go\" type=\"button\">Revoke</button>"),
            "{html}"
        );
        assert!(html.contains("Esc cancels"), "{html}");
        // The other row keeps its button.
        assert!(html.contains("id=\"token-c3-revoke-trigger\""), "{html}");
        assert_eq!(html.matches("role=\"group\"").count(), 1, "{html}");
        assert!(!html.contains("<a "), "no link wraps a button: {html}");
        assert!(!REVOKE_CONFIRM_BODY.contains('\u{2014}'));

        let prod = prod_src();
        // Revoke only opens the strip.
        let ask = prod
            .split("on_ask: EventHandler::new(move |token_id: String| {")
            .nth(1)
            .expect("ask handler");
        let ask = &ask[..ask.find("}),").expect("ask end")];
        assert!(ask.contains("confirming.set(Some(token_id));"), "{ask}");
        assert!(
            ask.contains("reveal_revoke_confirm_soon(revoke_confirm_id(&token_id));"),
            "the whole strip is scrolled into view, not only Cancel: {ask}"
        );
        assert!(!ask.contains("delete"), "{ask}");
        assert!(prod.contains("onclick: move |_| on_ask.call(tid.clone()),"));
        // The strip's Revoke sends the DELETE; success closes it and moves focus.
        assert!(prod.contains("on_confirm: EventHandler::new(on_revoke),"));
        assert!(prod.contains("on_confirm: move |_| on_confirm.call(confirm_tid.clone()),"));
        let revoke = prod
            .split("let on_revoke = move |token_id: String| {")
            .nth(1)
            .expect("revoke handler");
        let ok_arm = &revoke[..revoke.find("Err(e) =>").expect("error arm")];
        assert!(ok_arm.contains(".delete(&format!(\"/api/stats/tokens/{token_id}\"))"));
        assert!(ok_arm.contains("confirming.set(None);"), "{ok_arm}");
        assert!(ok_arm.contains("revoking.set(true);"), "{ok_arm}");
        assert!(
            ok_arm.contains("if revoking() {\n            return;"),
            "{ok_arm}"
        );
    }

    /// Cancel (or Esc) closes the strip without a DELETE and gives focus
    /// back to the row's Revoke button: `InlineConfirm` focuses
    /// `{id}-trigger`, which is exactly the button's id.
    #[test]
    fn cancel_closes_the_strip_and_returns_focus_to_revoke() {
        assert_eq!(
            revoke_button_id("a:1"),
            format!("{}-trigger", revoke_confirm_id("a:1"))
        );
        assert_eq!(revoke_button_id("a:1"), "token-a_1-revoke-trigger");
        assert!(crate::components::ui::inline_confirm::key_cancels(
            &Key::Escape
        ));

        let prod = prod_src();
        assert!(prod.contains("on_cancel: EventHandler::new(move |_: ()| confirming.set(None)),"));
        assert!(prod.contains("on_cancel: move |_| on_cancel.call(()),"));
        assert!(prod.contains("id: confirm_id,"));
        // The cancel path never touches the API.
        let cancel_line = prod
            .lines()
            .find(|l| l.contains("on_cancel: EventHandler::new("))
            .unwrap();
        assert!(!cancel_line.contains("delete"), "{cancel_line}");
        // Opening the strip does not unmount it on the next render: the row
        // keys stay the token ids.
        assert!(prod.contains("Fragment { key: \"{token.id}\", {token_row(token, &actions)} }"));
    }
}
