//! Shared notice for `/chat` and `/dm` when no relay is configured.

use dioxus::prelude::*;

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

const CSS: &str = r#"
.chat-unavailable {
    min-height: calc(100vh - 48px);
    display: flex;
    align-items: center;
    justify-content: center;
    text-align: center;
    padding: 4rem 1.5rem;
    box-sizing: border-box;
}
.chat-unavailable h1 {
    font-family: var(--font-head);
    font-size: 1.25rem;
    font-weight: 500;
    color: var(--text);
    max-width: 28rem;
    margin: 0;
    line-height: 1.5;
}
.chat-unavailable__retry {
    margin-top: 1rem;
    background: transparent;
    border: 1px solid var(--border);
    color: var(--text);
    border-radius: 6px;
    padding: 0.45rem 0.9rem;
    font: inherit;
    cursor: pointer;
}
"#;

#[component]
pub fn ChatUnavailable(message: &'static str) -> Element {
    rsx! {
        style { {CSS} }
        div { class: "chat-unavailable",
            h1 { "{message}" }
        }
    }
}

/// Failed health check. The message is not the "unavailable" or "checking" copy.
#[component]
pub fn ChatCheckFailed(on_retry: EventHandler<()>) -> Element {
    rsx! {
        style { {CSS} }
        div { class: "chat-unavailable",
            h1 { "{CHAT_FAILED}" }
            button {
                class: "chat-unavailable__retry",
                r#type: "button",
                onclick: move |_| on_retry.call(()),
                "Retry"
            }
        }
    }
}

/// Notice for a chat or DM route that is not connected yet.
#[component]
pub fn ChatGateNotice(availability: RelayAvailability, on_retry: EventHandler<()>) -> Element {
    match availability {
        RelayAvailability::Off => rsx! { ChatUnavailable { message: CHAT_UNAVAILABLE } },
        RelayAvailability::Failed => rsx! { ChatCheckFailed { on_retry } },
        RelayAvailability::Unknown | RelayAvailability::On => {
            rsx! { ChatUnavailable { message: CHAT_CHECKING } }
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
                !html.contains("Team Chat"),
                "{path} must not render the live chat page: {html}"
            );
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
        assert!(!html.contains("Team Chat"), "{html}");
    }

    #[test]
    fn a_failed_health_check_keeps_chat_out_of_the_unavailable_copy() {
        for path in ["/chat", "/dm"] {
            let html = render_route(path, RelayAvailability::Failed);
            assert!(shows_failed(&html), "{path} {html}");
            assert!(
                html.contains("chat-unavailable__retry"),
                "{path} needs a retry button: {html}"
            );
            assert!(html.contains(">Retry<"), "{path} {html}");
            assert!(!html.contains(CHAT_CHECKING), "{path} {html}");
            assert!(!shows_unavailable(&html), "{path} {html}");
            assert!(!html.contains("chat-widget"), "{path} {html}");
            assert!(!html.contains("Team Chat"), "{path} {html}");
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
            dm.contains("You must be signed in"),
            "logged-out DMs stay the login wall when a relay is configured: {dm}"
        );
        assert!(!shows_unavailable(&dm), "{dm}");

        let chat = render_route("/chat", RelayAvailability::On);
        assert!(
            chat.contains("Team Chat"),
            "configured /chat keeps the team chat page: {chat}"
        );
        assert!(
            chat.contains("Messages for the team you select."),
            "members get a plain subtitle: {chat}"
        );
        assert!(!chat.contains("NIP-29"), "{chat}");
        assert!(!chat.contains("encrypt API"), "{chat}");
        assert!(!shows_unavailable(&chat), "{chat}");
    }

    #[test]
    fn notice_copy_has_no_em_dash() {
        assert!(!CHAT_UNAVAILABLE.contains('\u{2014}'));
        assert!(!CHAT_CHECKING.contains('\u{2014}'));
        assert!(!CHAT_FAILED.contains('\u{2014}'));
        assert_eq!(CHAT_UNAVAILABLE, "Chat isn't available on this site yet");
        assert_eq!(CHAT_FAILED, "Couldn't reach chat right now.");
    }
}
