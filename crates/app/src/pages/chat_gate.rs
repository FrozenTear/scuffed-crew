//! Shared notice for `/chat` and `/dm` when no relay is configured.

use dioxus::prelude::*;

use crate::components::member_gate::Wall;
use crate::components::ui::{PageHeader, PageShell, PageWidth, btn_class};
use crate::routes::Route;
use crate::state::RelayAvailability;

pub const CHAT_UNAVAILABLE: &str = "Chat isn't available on this site yet";
pub const CHAT_CHECKING: &str = "Checking whether chat is available.";
pub const CHAT_FAILED: &str = "Couldn't reach chat right now.";

/// Copy for a gate that is not the live chat page.
pub(crate) fn chat_status_copy(availability: RelayAvailability) -> &'static str {
    match availability {
        RelayAvailability::Off => CHAT_UNAVAILABLE,
        RelayAvailability::Failed => CHAT_FAILED,
        RelayAvailability::Unknown | RelayAvailability::On => CHAT_CHECKING,
    }
}

/// One short line under each gate title.
const CHAT_OFF_LEAD: &str = "Chat and direct messages are turned off on this site.";
const CHAT_FAILED_LEAD: &str = "Check your connection and try again.";

/// H1 while the health check runs. The same gate serves `/chat` and `/dm`.
const GATE_TITLE: &str = "Chat";

const CSS: &str = r#"
.chat-gate-status {
    margin: 0;
    color: var(--text-3);
    font-family: var(--font-body);
    font-size: var(--text-md);
    line-height: var(--leading-body);
}
"#;

/// Text of the gate's status line. The cards already show their own words, so
/// the line only speaks next to them; while checking it is the visible content.
fn gate_status(availability: RelayAvailability) -> &'static str {
    match availability {
        RelayAvailability::Off => "",
        other => chat_status_copy(other),
    }
}

fn is_checking(availability: RelayAvailability) -> bool {
    matches!(
        availability,
        RelayAvailability::Unknown | RelayAvailability::On
    )
}

/// Notice for a chat or DM route that is not connected yet.
///
/// Off and failed are the shared wall card (the `d1-no-permission` frame)
/// with the message as the page's H1. The `role="status"` line is rendered in
/// every state and only its text changes, so "checking" and a failure are
/// each announced.
#[component]
pub fn ChatGateNotice(availability: RelayAvailability, on_retry: EventHandler<()>) -> Element {
    let status = gate_status(availability);
    let status_class = if is_checking(availability) {
        "chat-gate-status"
    } else {
        "visually-hidden"
    };
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-chat_gate-css", css: CSS }
        PageShell { width: PageWidth::Narrow, class: "chat-unavailable",
            match availability {
                RelayAvailability::Off => rsx! {
                    Wall {
                        page_title: true,
                        title: CHAT_UNAVAILABLE,
                        lead: CHAT_OFF_LEAD,
                        actions: rsx! {
                            Link { to: Route::Home {}, class: btn_class(true), "Return home" }
                        },
                    }
                },
                RelayAvailability::Failed => rsx! {
                    Wall {
                        page_title: true,
                        title: CHAT_FAILED,
                        lead: CHAT_FAILED_LEAD,
                        actions: rsx! {
                            button {
                                r#type: "button",
                                class: btn_class(true),
                                onclick: move |_| on_retry.call(()),
                                "Retry"
                            }
                            Link { to: Route::Home {}, class: btn_class(false), "Return home" }
                        },
                    }
                },
                RelayAvailability::Unknown | RelayAvailability::On => rsx! {
                    PageHeader { title: GATE_TITLE }
                },
            }
            p { class: status_class, role: "status", "{status}" }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::state::{
        AuthState, RelayAvailability, provide_site_settings, set_test_relay_availability,
    };

    thread_local! {
        static ROUTE_PATH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
    }

    struct ClearRelay;

    impl Drop for ClearRelay {
        fn drop(&mut self) {
            set_test_relay_availability(None);
            ROUTE_PATH.with(|slot| slot.borrow_mut().clear());
        }
    }

    fn shell() -> Element {
        let path = ROUTE_PATH.with(|slot| slot.borrow().clone());
        let history =
            use_hook(|| std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path(path)));
        let auth = use_signal(|| AuthState {
            user: None,
            loading: false,
        });
        use_context_provider(|| auth);
        provide_site_settings();
        rsx! {
            crate::components::ToastProvider {
                crate::theme::ThemeProvider {
                    dioxus::router::components::HistoryProvider {
                        history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                        Router::<crate::routes::Route> {}
                    }
                }
            }
        }
    }

    /// SSR escapes the apostrophe, so match the stable tail of the sentence.
    fn shows_unavailable(html: &str) -> bool {
        html.contains("chat-unavailable") && html.contains("available on this site yet")
    }

    /// The page body: what the shell's one `<main id="main">` holds.
    fn page_body(html: &str) -> &str {
        assert_eq!(
            html.matches("<main").count(),
            1,
            "one main landmark: {html}"
        );
        let start = html.find("<main").expect("main");
        let end = html[start..].find("</main>").expect("main closes");
        &html[start..start + end]
    }

    fn live_chat_page(html: &str) -> bool {
        html.contains("class=\"team-chat\"")
    }

    fn shows_failed(html: &str) -> bool {
        html.contains("reach chat right now.")
    }

    fn render_route(path: &str, availability: RelayAvailability) -> String {
        let _clear = ClearRelay;
        set_test_relay_availability(Some(availability));
        ROUTE_PATH.with(|slot| *slot.borrow_mut() = path.to_string());
        let mut dom = VirtualDom::new(shell);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn direct_chat_and_dm_routes_stay_quiet_when_the_relay_is_off() {
        for path in ["/chat", "/dm", "/dm/abc"] {
            let html = render_route(path, RelayAvailability::Off);
            assert!(
                shows_unavailable(&html),
                "{path} should show the notice: {html}"
            );
            assert!(
                !html.contains(CHAT_CHECKING),
                "{path} is settled off, not still checking: {html}"
            );
            assert!(
                !html.contains("chat-widget"),
                "{path} must not mount the widget: {html}"
            );
            assert!(
                !html.contains("New message"),
                "{path} must not offer a new DM: {html}"
            );
            assert!(
                !html.contains("Start a conversation"),
                "{path} must not offer a new DM: {html}"
            );
            assert!(
                !live_chat_page(&html),
                "{path} must not render the live chat page: {html}"
            );
            let body = page_body(&html);
            assert_eq!(body.matches("<h1").count(), 1, "{path} {body}");
            assert_eq!(body.matches("role=\"status\"").count(), 1, "{path} {body}");
            assert!(
                body.contains("<p class=\"visually-hidden\" role=\"status\"></p>"),
                "{path}: the card says it, the status line stays quiet: {body}"
            );
            assert!(body.contains("<section class=\"wall\""), "{path} {body}");
            assert!(body.contains(CHAT_OFF_LEAD), "{path} {body}");
            assert!(body.contains(">Return home</a>"), "{path} {body}");
            assert!(!body.contains("<button"), "{path} {body}");
            assert!(
                !html.contains("You must be signed in"),
                "{path} must not fall through to the DM login wall: {html}"
            );
        }
    }

    #[test]
    fn direct_chat_does_not_open_a_socket_while_health_is_unknown() {
        let html = render_route("/chat", RelayAvailability::Unknown);
        assert!(html.contains(CHAT_CHECKING), "{html}");
        assert!(!shows_unavailable(&html), "{html}");
        assert!(!html.contains("chat-widget"), "{html}");
        assert!(!live_chat_page(&html), "{html}");
        let body = page_body(&html);
        assert_eq!(body.matches("<h1").count(), 1, "{body}");
        assert!(body.contains("<h1>Chat</h1>"), "{body}");
        assert!(
            body.contains(&format!(
                "<p class=\"chat-gate-status\" role=\"status\">{CHAT_CHECKING}</p>"
            )),
            "while checking, the status line is the visible content: {body}"
        );
        assert!(!body.contains("class=\"wall\""), "{body}");
    }

    #[test]
    fn a_failed_health_check_keeps_chat_out_of_the_unavailable_copy() {
        for path in ["/chat", "/dm"] {
            let html = render_route(path, RelayAvailability::Failed);
            assert!(shows_failed(&html), "{path} {html}");
            let body = page_body(&html);
            assert!(
                body.contains("<button type=\"button\" class=\"btn is-primary\">Retry</button>"),
                "{path} needs a retry button: {body}"
            );
            assert_eq!(body.matches("<h1").count(), 1, "{path} {body}");
            assert_eq!(body.matches("role=\"status\"").count(), 1, "{path} {body}");
            assert!(
                body.contains("<p class=\"visually-hidden\" role=\"status\">"),
                "{path}: the failure is announced, the card shows it: {body}"
            );
            assert!(body.contains(CHAT_FAILED_LEAD), "{path} {body}");
            assert!(body.contains(">Return home</a>"), "{path} {body}");
            assert!(!html.contains(CHAT_CHECKING), "{path} {html}");
            assert!(!shows_unavailable(&html), "{path} {html}");
            assert!(!html.contains("chat-widget"), "{path} {html}");
            assert!(!live_chat_page(&html), "{path} {html}");
        }
    }

    #[test]
    fn a_failed_health_check_returns_to_checking_when_retry_starts() {
        use crate::state::relay::displayed_availability;

        let failed = Some(RelayAvailability::Failed);
        let offline = Err("offline".into());
        assert_eq!(chat_status_copy(RelayAvailability::Failed), CHAT_FAILED);
        assert_eq!(
            displayed_availability(failed, Some(&offline), false),
            RelayAvailability::Failed
        );
        let in_flight = displayed_availability(failed, None, true);
        assert_eq!(in_flight, RelayAvailability::Unknown);
        assert_eq!(chat_status_copy(in_flight), CHAT_CHECKING);
        assert_ne!(chat_status_copy(in_flight), CHAT_FAILED);
        assert_ne!(CHAT_FAILED, CHAT_UNAVAILABLE);
    }

    #[test]
    fn configured_relay_keeps_today_dm_and_chat_entry() {
        let dm = render_route("/dm", RelayAvailability::On);
        assert!(
            dm.contains("Log in to read your messages"),
            "logged-out DMs stay the login wall when a relay is configured: {dm}"
        );
        assert!(!shows_unavailable(&dm), "{dm}");

        let chat = render_route("/chat", RelayAvailability::On);
        assert!(
            live_chat_page(&chat),
            "configured /chat keeps the team chat page: {chat}"
        );
        assert!(!chat.contains("NIP-29"), "{chat}");
        assert!(!chat.contains("encrypt API"), "{chat}");
        assert!(!shows_unavailable(&chat), "{chat}");
    }

    /// A guest on a configured `/chat`: the full-height thread with its head
    /// bar, and the sign-in wall where the messages would be.
    #[test]
    fn live_chat_page_is_one_thread_under_the_shell_main() {
        let chat = render_route("/chat", RelayAvailability::On);
        let body = page_body(&chat);
        assert_eq!(body.matches("<h1").count(), 1, "{body}");
        assert!(body.contains("<h1>Team chat</h1>"), "{body}");
        assert!(
            body.contains("<section class=\"chat-thread\" aria-label=\"Team chat\">"),
            "{body}"
        );
        let page = body.find("class=\"team-chat\"").expect("page");
        let head = body.find("chat-thread__head").expect("head bar");
        let state = body.find("chat-thread__body").expect("state body");
        assert!(page < head && head < state, "{body}");
        assert!(
            body.contains("<h2 class=\"wall-title\">Log in required</h2>"),
            "the wall title is an H2 under the page's H1: {body}"
        );
        assert!(
            !body.contains("<select"),
            "no team picker before the teams load: {body}"
        );
        assert!(
            !body.contains("ui-shell"),
            "a full-height tool has no page column: {body}"
        );
        assert!(
            !chat.contains("<footer"),
            "the shell hides the footer: {chat}"
        );
    }

    #[test]
    fn gate_is_the_shared_wall_in_the_narrow_column() {
        use crate::components::ui::test_support::assert_sizes_from_tokens;

        assert_sizes_from_tokens(CSS);
        let html = render_route("/chat", RelayAvailability::Off);
        let body = page_body(&html);
        assert!(
            body.contains("<div class=\"ui-shell ui-shell--narrow chat-unavailable\">"),
            "{body}"
        );
        assert_eq!(gate_status(RelayAvailability::Off), "");
        assert_eq!(gate_status(RelayAvailability::Failed), CHAT_FAILED);
        assert_eq!(gate_status(RelayAvailability::Unknown), CHAT_CHECKING);
        assert_eq!(gate_status(RelayAvailability::On), CHAT_CHECKING);
        assert!(is_checking(RelayAvailability::Unknown));
        assert!(!is_checking(RelayAvailability::Off));
        assert!(!is_checking(RelayAvailability::Failed));
    }

    #[test]
    fn notice_copy_has_no_em_dash() {
        assert!(!CHAT_UNAVAILABLE.contains('\u{2014}'));
        assert!(!CHAT_CHECKING.contains('\u{2014}'));
        assert!(!CHAT_FAILED.contains('\u{2014}'));
        for copy in [CHAT_OFF_LEAD, CHAT_FAILED_LEAD, GATE_TITLE] {
            assert!(!copy.contains('\u{2014}') && !copy.contains('\u{2013}'));
        }
        assert_eq!(CHAT_UNAVAILABLE, "Chat isn't available on this site yet");
        assert_eq!(CHAT_FAILED, "Couldn't reach chat right now.");
    }
}
