//! Admin: Nostr relay status (`/admin/relay`, admin only).
//!
//! One read of `/api/nostr/health`. The state is always a word in a pill with
//! a shape in front of it (a filled dot for online, a triangle for offline, a
//! ring for not set), so colour is never the only signal. An offline primary
//! relay shows an alert banner with no button of its own: "Check again" in
//! the page header is the retry.

use dioxus::prelude::*;
use serde::Deserialize;

use crate::components::ui::{
    EmptyCard, LoadError, PageHeader, PageShell, PageWidth, Pill, PillTone, btn_class,
};
use crate::components::{AccessDenied, StatusPill, admin_pending};
use crate::hooks::use_api;
use crate::routes::Route;
use crate::state::use_auth;

#[derive(Debug, Clone, PartialEq, Deserialize)]
struct RelayHealth {
    configured: bool,
    reachable: bool,
    relay_url: Option<String>,
    #[serde(default)]
    extra_relay_urls: Vec<String>,
    forum_backend: String,
}

fn forum_is_local(backend: &str) -> bool {
    backend.eq_ignore_ascii_case("local") || backend.is_empty()
}

/// The primary relay's state.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RelayState {
    Online,
    Offline,
    /// No URL, and the local forum does not need one.
    NotNeeded,
    /// No URL, but the forum is set to Nostr.
    NotSet,
}

impl RelayState {
    fn of(h: &RelayHealth) -> Self {
        match (h.configured, h.reachable) {
            (true, true) => RelayState::Online,
            (true, false) => RelayState::Offline,
            (false, _) if forum_is_local(&h.forum_backend) => RelayState::NotNeeded,
            (false, _) => RelayState::NotSet,
        }
    }

    fn word(self) -> &'static str {
        match self {
            RelayState::Online => "Online",
            RelayState::Offline => "Offline",
            RelayState::NotNeeded => "Not needed",
            RelayState::NotSet => "Not set",
        }
    }

    fn tone(self) -> PillTone {
        match self {
            RelayState::Online => PillTone::Ok,
            RelayState::Offline => PillTone::Danger,
            RelayState::NotNeeded => PillTone::Neutral,
            RelayState::NotSet => PillTone::Warn,
        }
    }

    /// Class of the shape in front of the word.
    fn shape(self) -> &'static str {
        match self {
            RelayState::Online => "relay-shape is-online",
            RelayState::Offline => "relay-shape is-offline",
            RelayState::NotNeeded | RelayState::NotSet => "relay-shape is-unset",
        }
    }
}

/// The always-rendered status line: one sentence on the primary relay.
fn summary_text(state: RelayState) -> &'static str {
    match state {
        RelayState::Online => "The primary relay is online.",
        // The offline alert says it; one live region is enough.
        RelayState::Offline => "",
        RelayState::NotNeeded => "No primary relay is set. The local forum does not need one.",
        RelayState::NotSet => "No primary relay is set, and the forum is set to Nostr.",
    }
}

const STATUS_ID: &str = "relay-status-line";

const CSS: &str = r#"
    .admin-relay-page .relay-status-line {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    /* No text: out of the layout, still in the tree (a live line
       that is display: none drops out of what screen readers track). */
    .admin-relay-page .relay-status-line:empty {
        position: absolute;
        width: 1px;
        height: 1px;
        padding: 0;
        margin: -1px;
        overflow: hidden;
        clip: rect(0, 0, 0, 0);
        white-space: nowrap;
        border: 0;
    }
    .admin-relay-page .relay-facts {
        display: grid;
        grid-template-columns: max-content minmax(0, 1fr);
        align-items: center;
        gap: var(--space-3) var(--space-5);
        margin: 0;
    }
    .admin-relay-page .relay-facts dt {
        color: var(--text-3);
        font-size: var(--text-sm);
        font-weight: var(--font-label-weight);
    }
    .admin-relay-page .relay-facts dd {
        margin: 0;
        min-width: 0;
        color: var(--text);
        font-size: var(--text-md);
        overflow-wrap: anywhere;
    }
    .admin-relay-page .relay-facts dd.num { font-variant-numeric: tabular-nums; }
    .admin-relay-page .relay-list {
        margin: 0;
        padding: 0;
        list-style: none;
        display: flex;
        flex-direction: column;
        gap: var(--space-2);
    }
    .admin-relay-page .relay-list code,
    .admin-relay-page .relay-facts code {
        font-family: var(--font-mono);
        font-size: var(--text-sm);
        overflow-wrap: anywhere;
    }
    .admin-relay-page .relay-note {
        margin: 0;
        color: var(--text-3);
        font-size: var(--text-sm);
        line-height: var(--leading-body);
    }
    .relay-shape {
        display: inline-block;
        flex: 0 0 auto;
        width: var(--space-2);
        height: var(--space-2);
        margin-inline-end: var(--space-1);
        background: currentColor;
        vertical-align: middle;
    }
    .relay-shape.is-online { border-radius: var(--radius-full); }
    .relay-shape.is-offline { clip-path: polygon(50% 0, 100% 100%, 0 100%); }
    .relay-shape.is-unset {
        box-sizing: border-box;
        background: transparent;
        border: 1px solid currentColor;
        border-radius: var(--radius-full);
    }
    /* Forced colours drop background fills: the filled shapes keep a
       system colour, so Online (dot) and Offline (triangle) still show. */
    @media (forced-colors: active) {
        .relay-shape { forced-color-adjust: none; background: CanvasText; }
        .relay-shape.is-unset { background: transparent; border-color: CanvasText; }
    }
    @media (max-width: 820px) {
        .admin-relay-page .relay-facts {
            grid-template-columns: minmax(0, 1fr);
            gap: var(--space-1);
        }
        .admin-relay-page .relay-facts dd + dt { margin-top: var(--space-3); }
    }
"#;

/// The state as a pill: a shape, then the word.
#[component]
fn RelayStatePill(state: RelayState) -> Element {
    rsx! {
        Pill { tone: state.tone(),
            span { class: state.shape(), aria_hidden: "true" }
            "{state.word()}"
        }
    }
}

#[component]
pub fn AdminRelay() -> Element {
    let auth = use_auth();
    let health = use_api::<RelayHealth>("/api/nostr/health");
    let mut refresh = health.refresh;

    if !auth().is_admin() {
        return rsx! {
            AccessDenied { message: "You need admin permissions to view relay status.".to_string() }
        };
    }

    let (body, summary) = {
        let data = health.data.read();
        match data.as_ref().and_then(|d| d.as_ref()) {
            None => (admin_pending(&health, "relay status"), ""),
            Some(h) => {
                let state = RelayState::of(h);
                (
                    rsx! { RelayDetails { health: h.clone() } },
                    summary_text(state),
                )
            }
        }
    };

    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-admin-relay-css", css: CSS }
        PageShell { width: PageWidth::Default, class: "admin-relay-page",
            PageHeader {
                title: "Relay status",
                lead: "The Nostr relay carries direct messages, profile publishing and the Nostr forum. The site checks the primary relay each time this page loads.",
                actions: rsx! {
                    button {
                        r#type: "button",
                        id: "relay-refresh",
                        class: "btn is-ghost",
                        aria_describedby: STATUS_ID,
                        onclick: move |_| refresh += 1,
                        "Check again"
                    }
                },
            }
            p { id: STATUS_ID, class: "relay-status-line", role: "status", "{summary}" }
            {body}
        }
    }
}

/// Everything under the header once the health check answered.
#[component]
fn RelayDetails(health: RelayHealth) -> Element {
    let h = health;
    let state = RelayState::of(&h);
    let local_forum = forum_is_local(&h.forum_backend);
    let relay_count = usize::from(h.configured) + h.extra_relay_urls.len();
    let configured_word = if h.configured { "Yes" } else { "No" };
    // F-AUI-003: an empty URL with "configured" was lying; say what is true.
    let url_display = h
        .relay_url
        .clone()
        .filter(|u| !u.trim().is_empty())
        .unwrap_or_else(|| {
            if local_forum {
                "Not set (optional for the local forum)".into()
            } else {
                "Not set".into()
            }
        });
    let unconfigured_help = if local_forum {
        "The forum runs locally, so a Nostr relay is optional. Set NOSTR_RELAY_URL on the server only when you need relay features: profile publishing, direct messages or the Nostr forum."
    } else {
        "The forum is set to Nostr but no primary relay URL is set. Set the NOSTR_RELAY_URL environment variable on the server to a ws:// or wss:// URL, then check again."
    };
    let offline_help = if local_forum {
        "It is set but did not answer. The local forum still works. Check the URL and the network if you need relay features."
    } else {
        "It is set but did not answer. Check the URL, the firewall, and that the relay process is running."
    };
    let forum_word = if h.forum_backend.trim().is_empty() {
        "local".to_string()
    } else {
        h.forum_backend.clone()
    };

    let banner = match state {
        RelayState::Offline => rsx! {
            // "Check again" in the header is the one retry; no second button.
            LoadError {
                message: format!("The primary relay is offline. {offline_help}"),
            }
        },
        RelayState::NotNeeded | RelayState::NotSet => rsx! {
            EmptyCard {
                title: if state == RelayState::NotSet { "No relay is set" } else { "No relay needed" },
                message: unconfigured_help.to_string(),
                actions: rsx! {
                    Link { to: Route::AdminSettings {}, class: btn_class(false), "Open settings" }
                },
            }
        },
        RelayState::Online => rsx! {},
    };

    rsx! {
        {banner}
        section { class: "form-section", aria_labelledby: "relay-primary-title",
            h2 { id: "relay-primary-title", "Primary relay" }
            div { class: "form-section-card",
                dl { class: "relay-facts",
                    dt { "Status" }
                    dd { RelayStatePill { state } }
                    dt { "Address" }
                    dd { code { "{url_display}" } }
                    dt { "Configured" }
                    dd { "{configured_word}" }
                    dt { "Forum mode" }
                    dd { StatusPill { status: forum_word } }
                    dt { "Relays in total" }
                    dd { class: "num", "{relay_count}" }
                }
            }
        }
        section { class: "form-section", aria_labelledby: "relay-extra-title",
            h2 { id: "relay-extra-title", "Extra relays" }
            div { class: "form-section-card",
                if h.extra_relay_urls.is_empty() {
                    p { class: "relay-note", "No extra relays. Add them in Settings, under Forum, Extra relay URLs." }
                } else {
                    ul { class: "relay-list",
                        for url in h.extra_relay_urls.iter() {
                            li { key: "{url}", code { "{url}" } }
                        }
                    }
                    p { class: "relay-note", "Change extra relays in Settings, under Forum, Extra relay URLs." }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, render_routed};

    fn health(configured: bool, reachable: bool, backend: &str) -> RelayHealth {
        RelayHealth {
            configured,
            reachable,
            relay_url: configured.then(|| "wss://relay.example.test".to_string()),
            extra_relay_urls: vec!["wss://extra.example.test".into()],
            forum_backend: backend.into(),
        }
    }

    #[test]
    fn relay_shapes_survive_forced_colours() {
        // Vision nit 10 on PR 706.
        use crate::components::ui::test_support::media_block;
        let forced = media_block(CSS, "@media (forced-colors: active)");
        assert!(
            forced.contains(".relay-shape { forced-color-adjust: none; background: CanvasText; }"),
            "{forced}"
        );
        assert!(
            forced.contains(
                ".relay-shape.is-unset { background: transparent; border-color: CanvasText; }"
            ),
            "{forced}"
        );
    }

    #[test]
    fn empty_status_line_stays_in_the_tree() {
        // Vision nit 5 on PR 706: `display: none` dropped the live line.
        crate::styles::admin::assert_live_line_kept(
            super::CSS,
            ".admin-relay-page .relay-status-line:empty {",
        );
    }

    #[test]
    fn page_has_one_h1_a_status_line_and_a_labelled_wait() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/relay", Some(scuffed_types::OrgRole::Admin));
        assert!(
            html.contains("class=\"ui-shell admin-relay-page\""),
            "{html}"
        );
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("<h1>Relay status</h1>"), "{html}");
        assert_eq!(html.matches("<main").count(), 1, "{html}");
        assert!(
            html.contains(
                "<p id=\"relay-status-line\" class=\"relay-status-line\" role=\"status\"></p>"
            ),
            "{html}"
        );
        assert!(
            html.contains("role=\"status\">Loading relay status...</p>"),
            "{html}"
        );
        // Nothing in the page itself is styled inline (the shared toast
        // host outside the page is not this page's markup).
        let page = &html[html.find("admin-relay-page").expect("page")..];
        let page = &page[..page.find("</main>").expect("main end")];
        assert!(!page.contains("style="), "{page}");
    }

    #[test]
    fn officers_get_the_no_access_page() {
        use crate::components::admin_shared::test_page::render_admin_route;
        let html = render_admin_route("/admin/relay", Some(scuffed_types::OrgRole::Officer));
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        assert!(html.contains("You need admin permissions"), "{html}");
    }

    #[test]
    fn every_state_has_a_word_and_a_shape() {
        assert_eq!(
            RelayState::of(&health(true, true, "nostr")),
            RelayState::Online
        );
        assert_eq!(
            RelayState::of(&health(true, false, "local")),
            RelayState::Offline
        );
        assert_eq!(
            RelayState::of(&health(false, false, "local")),
            RelayState::NotNeeded
        );
        assert_eq!(
            RelayState::of(&health(false, false, "")),
            RelayState::NotNeeded
        );
        assert_eq!(
            RelayState::of(&health(false, false, "nostr")),
            RelayState::NotSet
        );
        let shapes: Vec<&str> = [RelayState::Online, RelayState::Offline, RelayState::NotSet]
            .iter()
            .map(|s| s.shape())
            .collect();
        assert_eq!(shapes.len(), 3);
        assert!(shapes[0] != shapes[1] && shapes[1] != shapes[2] && shapes[0] != shapes[2]);
    }

    #[test]
    fn online_relay_shows_the_word_and_shape_and_no_error() {
        let html = render_routed(|| {
            rsx! { RelayDetails { health: health(true, true, "nostr") } }
        });
        assert!(
            html.contains(
                "<span class=\"ui-pill ui-pill--ok\"><span class=\"relay-shape is-online\" aria-hidden=\"true\"></span>Online</span>"
            ),
            "{html}"
        );
        assert!(
            html.contains("<code>wss://relay.example.test</code>"),
            "{html}"
        );
        assert!(
            html.contains("<code>wss://extra.example.test</code>"),
            "{html}"
        );
        assert!(!html.contains("fetch-error"), "{html}");
        assert_eq!(html.matches("<h2").count(), 2, "{html}");
    }

    #[test]
    fn offline_relay_is_an_alert_without_its_own_retry() {
        let html = render_routed(|| {
            rsx! { RelayDetails { health: health(true, false, "nostr") } }
        });
        assert!(html.contains("The primary relay is offline."), "{html}");
        // One way to check again: the header button, not a second Retry.
        assert!(!html.contains(">Retry</button>"), "{html}");
        // Vision round 7 on 748: the no-Retry banner is an alert, and the
        // status line says nothing while it shows, so it is said once.
        assert!(
            html.contains("<div class=\"fetch-error-wrap\" role=\"alert\">"),
            "{html}"
        );
        assert_eq!(summary_text(RelayState::Offline), "");
        assert!(html.contains("relay-shape is-offline"), "{html}");
        assert!(html.contains(">Offline</span>"), "{html}");
    }

    #[test]
    fn missing_relay_is_an_empty_state_with_a_next_step() {
        let html = render_routed(|| {
            rsx! { RelayDetails { health: health(false, false, "nostr") } }
        });
        assert!(html.contains("No relay is set"), "{html}");
        assert!(html.contains("NOSTR_RELAY_URL"), "{html}");
        assert!(html.contains("href=\"/admin/settings\""), "{html}");
        assert!(html.contains(">Not set</span>"), "{html}");
    }

    #[test]
    fn page_css_sizes_come_from_tokens() {
        assert_sizes_from_tokens(CSS);
    }
}
