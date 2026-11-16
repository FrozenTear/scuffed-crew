//! Shared pending/auth UX for member-only public pages.
//!
//! `use_api*` stores failures as `None` data + `error`. Pages that only match
//! `None => "Loading…"` then hang forever on 401/403. Gate on auth first, and
//! treat 401 as sign-in / 403 as apply-or-inactive; never infinite Loading.

use dioxus::prelude::*;

use crate::components::fetch_state::retry_label;
use crate::components::ui::btn_class;
use crate::hooks::ApiResource;
use crate::routes::Route;
use crate::state::auth::AuthState;

/// The no-access card of the `d1-no-permission` frame: a title, a short
/// explanation and a row of next actions, left-aligned inside a card that is
/// centred in the page column. The 404 view and the admin no-access state use
/// the same card. Spacing follows the frame: the lead sits `--space-3` under
/// the title and the actions `--space-4` under the lead, at every density.
/// The actions are the shared `btn` buttons.
pub const WALL_CSS: &str = r#"
    .wall {
        box-sizing: border-box;
        width: 100%;
        max-width: var(--content-max-narrow);
        min-width: 0;
        margin: 0 auto;
        padding: var(--space-section);
        display: flex;
        flex-direction: column;
        gap: 0;
        background: var(--surface);
        border: 1px solid var(--card-border);
        border-radius: var(--card-radius);
        box-shadow: var(--card-shadow);
        color: var(--text);
        font-family: var(--font-body);
        text-align: left;
    }
    .wall-kicker {
        margin: 0 0 var(--space-3);
        font-family: var(--font-head);
        font-weight: var(--font-label-weight);
        font-size: var(--text-xs);
        letter-spacing: var(--font-head-tracking);
        text-transform: uppercase;
        color: var(--accent-text);
    }
    .wall-title {
        margin: 0;
        font-family: var(--font-head);
        font-weight: var(--font-head-weight);
        letter-spacing: var(--font-head-tracking);
        text-transform: var(--font-head-transform);
        line-height: var(--leading-display);
        font-size: var(--text-2xl);
        color: var(--text);
        overflow-wrap: anywhere;
    }
    .wall .page-lead,
    .wall-lead {
        margin: var(--space-3) 0 0;
        color: var(--text-3);
        font-size: var(--text-md);
        line-height: var(--leading-body);
    }
    .wall-path {
        margin: var(--space-3) 0 0;
        font-family: var(--font-mono);
        font-size: var(--text-xs);
        color: var(--text-3);
        overflow-wrap: anywhere;
    }
    .wall-actions {
        display: flex;
        flex-wrap: wrap;
        align-items: center;
        gap: var(--space-2);
        margin-top: var(--space-4);
    }
    .member-gate-status {
        margin: 0;
        padding: var(--space-section) 0;
        color: var(--text-3);
        font-size: var(--text-md);
        text-align: center;
    }
"#;

const WALL_CSS_ID: &str = "scuffed-css-components-member_gate-wall-css";

/// The wall card. `page_title: true` makes the title the page's H1 (a whole
/// page that is a wall: 404, admin no-access). Otherwise the title is an H2
/// under the H1 of the page that embeds the wall.
#[component]
pub fn Wall(
    title: String,
    lead: String,
    #[props(default)] page_title: bool,
    /// Small label above the title ("Error 404").
    #[props(default)]
    kicker: String,
    /// The requested path, shown in the mono face under the lead.
    #[props(default)]
    path: String,
    /// Links or buttons with [`btn_class`].
    actions: Element,
) -> Element {
    rsx! {
        crate::styles::StyleOnce { id: WALL_CSS_ID, css: WALL_CSS }
        section { class: "wall",
            if !kicker.is_empty() {
                p { class: "wall-kicker", "{kicker}" }
            }
            if page_title {
                crate::components::ui::PageHeader { title, lead }
            } else {
                div { class: "wall-head",
                    h2 { class: "wall-title", "{title}" }
                    p { class: "wall-lead", "{lead}" }
                }
            }
            if !path.is_empty() {
                p { class: "wall-path", "{path}" }
            }
            div { class: "wall-actions", {actions} }
        }
    }
}

/// True when a `use_api*` error string is that HTTP status (`ClientError` Display).
pub fn is_http_status(err: &str, status: u16) -> bool {
    err.contains(&format!("HTTP error {status}:")) || err.contains(&format!("HTTP error: {status}"))
}

fn home_link() -> Element {
    rsx! {
        Link { to: Route::Home {}, class: btn_class(false), "Return home" }
    }
}

fn sign_in_panel(what: &str) -> Element {
    rsx! {
        Wall {
            title: "Sign in required",
            lead: "Sign in to view {what}.",
            actions: rsx! {
                Link { to: Route::Login {}, class: btn_class(true), "Sign in" }
                {home_link()}
            },
        }
    }
}

fn apply_panel(what: &str) -> Element {
    rsx! {
        Wall {
            title: "Membership required",
            lead: "You need to be an org member to view {what}.",
            actions: rsx! {
                Link { to: Route::Apply {}, class: btn_class(true), "Apply" }
                {home_link()}
            },
        }
    }
}

fn inactive_panel(what: &str) -> Element {
    rsx! {
        Wall {
            title: "Membership inactive",
            lead: "Your membership is not active, so {what} is not available.",
            actions: rsx! {
                Link { to: Route::Home {}, class: btn_class(true), "Return home" }
                Link { to: Route::ProfileSettings {}, class: btn_class(false), "Open your profile" }
            },
        }
    }
}

/// The load failed for a reason a retry may fix. The raw client error is not
/// shown. While `retrying`, the card stays up and its button stays focusable
/// (`aria-disabled`, not `disabled`), so focus is not dropped to the page.
fn retry_panel(what: &str, retrying: bool, mut refresh: Signal<u64>) -> Element {
    rsx! {
        Wall {
            title: "Couldn't load {what}",
            lead: "Check your connection and try again.",
            actions: rsx! {
                button {
                    r#type: "button",
                    class: btn_class(true),
                    aria_busy: if retrying { "true" },
                    aria_disabled: if retrying { "true" },
                    onclick: move |_| {
                        if !retrying {
                            refresh += 1;
                        }
                    },
                    "{retry_label(retrying)}"
                }
                {home_link()}
            },
        }
    }
}

/// What `member_pending` shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum GateView {
    SignIn,
    Apply,
    Inactive,
    Loading,
    /// The fetch failed; `retrying` while a Retry is in flight.
    Failed {
        retrying: bool,
    },
}

/// `error` is the last fetch error. `failed_before` is true when the slot
/// holds a failed load; `pending` when a fetch is in flight. `use_api` clears
/// the error when a refetch starts, so a failed slot that is pending again is
/// a retry, and the error card stays up for it.
fn gate_view(
    auth: &AuthState,
    error: Option<&str>,
    failed_before: bool,
    pending: bool,
) -> GateView {
    if !auth.loading && !auth.is_logged_in() {
        return GateView::SignIn;
    }
    if !auth.loading && !auth.is_org_member() {
        return GateView::Apply;
    }
    match error {
        Some(err) if is_http_status(err, 401) => GateView::SignIn,
        Some(err) if is_http_status(err, 403) => {
            if auth.is_org_member() {
                GateView::Inactive
            } else {
                GateView::Apply
            }
        }
        Some(_) => GateView::Failed { retrying: false },
        None if failed_before && pending => GateView::Failed { retrying: true },
        None => GateView::Loading,
    }
}

/// Text of the gate's status line. It is rendered in every state; only this
/// text changes, so loading, a failure and a retry are each announced.
fn gate_status(view: GateView, what: &str) -> String {
    match view {
        GateView::Loading | GateView::Failed { retrying: true } => format!("Loading {what}"),
        GateView::Failed { retrying: false } => format!("Couldn't load {what}."),
        GateView::SignIn | GateView::Apply | GateView::Inactive => String::new(),
    }
}

/// Pending UI for a member-only `use_api*` whose `data` holds no rows yet.
/// Call from the `None` arm only.
pub fn member_pending<T: 'static>(
    auth: &AuthState,
    resource: &ApiResource<T>,
    what: &str,
) -> Element {
    let error = resource.error.read().as_ref().cloned();
    let failed_before = matches!(resource.data.read().as_ref(), Some(None));
    let pending = resource.data.state()() == UseResourceState::Pending;
    let view = gate_view(auth, error.as_deref(), failed_before, pending);
    let status = gate_status(view, what);
    // The line is the visible content while loading. Next to a card it only
    // speaks: the card already shows the words.
    let status_class = if view == GateView::Loading {
        "member-gate-status"
    } else {
        "visually-hidden"
    };
    let card = match view {
        GateView::SignIn => sign_in_panel(what),
        GateView::Apply => apply_panel(what),
        GateView::Inactive => inactive_panel(what),
        GateView::Failed { retrying } => retry_panel(what, retrying, resource.refresh),
        GateView::Loading => rsx! {},
    };

    rsx! {
        crate::styles::StyleOnce { id: WALL_CSS_ID, css: WALL_CSS }
        p { class: status_class, role: "status", "{status}" }
        {card}
    }
}

/// Renders a route inside the real router and site shell, as a guest or as
/// the given user. Shared by the wall tests here, in `pages/not_found.rs` and
/// in `layouts/admin.rs`.
#[cfg(test)]
pub(crate) mod test_route {
    use dioxus::prelude::*;
    use scuffed_types::{OrgRole, UserInfo};

    use crate::routes::Route;
    use crate::state::AuthState;

    thread_local! {
        static SCENE: std::cell::RefCell<(String, Option<OrgRole>)> =
            const { std::cell::RefCell::new((String::new(), None)) };
    }

    fn shell() -> Element {
        let (path, role) = SCENE.with(|slot| slot.borrow().clone());
        let history =
            use_hook(|| std::rc::Rc::new(dioxus::history::MemoryHistory::with_initial_path(path)));
        let auth = use_signal(|| AuthState {
            user: role.map(|role| UserInfo {
                id: "u1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role: Some(role),
            }),
            loading: false,
        });
        use_context_provider(|| auth);
        crate::state::provide_site_settings();
        rsx! {
            crate::theme::ThemeProvider {
                dioxus::router::components::HistoryProvider {
                    history: move |_| history.clone() as std::rc::Rc<dyn dioxus::history::History>,
                    Router::<Route> {}
                }
            }
        }
    }

    /// `role: None` is a guest.
    pub fn render_route(path: &str, role: Option<OrgRole>) -> String {
        SCENE.with(|slot| *slot.borrow_mut() = (path.to_string(), role));
        let mut dom = VirtualDom::new(shell);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    /// The first `<section class="wall">` of the page.
    pub fn wall(html: &str) -> &str {
        let start = html
            .find("<section class=\"wall\"")
            .unwrap_or_else(|| panic!("no wall in {html}"));
        let end = html[start..].find("</section>").expect("wall closes");
        &html[start..start + end]
    }
}

#[cfg(test)]
mod tests {
    use super::test_route::{render_route, wall};
    use super::*;
    use crate::components::ui::test_support::{assert_sizes_from_tokens, css_rule, has_decl};
    use scuffed_types::{OrgRole, UserInfo};

    #[test]
    fn wall_sizes_and_type_come_from_tokens() {
        assert_sizes_from_tokens(WALL_CSS);
        let card = css_rule(WALL_CSS, ".wall {");
        for decl in [
            "max-width: var(--content-max-narrow)",
            "margin: 0 auto",
            "padding: var(--space-section)",
            "background: var(--surface)",
            "border: 1px solid var(--card-border)",
            "border-radius: var(--card-radius)",
        ] {
            assert!(has_decl(&card, decl), "missing {decl}: {card}");
        }
    }

    /// The frame's spacing, the same at every density: lead `--space-3` under
    /// the title, actions `--space-4` under the lead.
    #[test]
    fn wall_spacing_follows_the_frame() {
        assert!(has_decl(&css_rule(WALL_CSS, ".wall {"), "gap: 0"));
        let lead = css_rule(WALL_CSS, ".wall .page-lead,\n    .wall-lead {");
        assert!(has_decl(&lead, "margin: var(--space-3) 0 0"), "{lead}");
        let actions = css_rule(WALL_CSS, ".wall-actions {");
        assert!(
            has_decl(&actions, "margin-top: var(--space-4)"),
            "{actions}"
        );
        // The actions are the shared buttons, not a wall-only copy.
        assert!(!WALL_CSS.contains("wall-btn"), "use the shared btn");
    }

    #[test]
    fn wall_ink_clears_text_contrast() {
        use crate::components::ui::test_support::assert_ink_clears_aa;
        assert_ink_clears_aa("--text", "--surface", 4.5);
        assert_ink_clears_aa("--text-3", "--surface", 4.5);
        assert_ink_clears_aa("--accent-text", "--surface", 4.5);
        assert_ink_clears_aa("--on-accent", "--accent-solid", 4.5);
    }

    #[test]
    fn guest_on_a_member_page_gets_a_sign_in_wall_under_the_page_h1() {
        let html = render_route("/polls", None);
        assert_eq!(html.matches("<h1").count(), 1, "{html}");
        let wall = wall(&html);
        assert!(
            wall.contains("<h2 class=\"wall-title\">Sign in required</h2>"),
            "{wall}"
        );
        assert!(wall.contains("Sign in to view polls."), "{wall}");
        assert_eq!(wall.matches("<a ").count(), 2, "{wall}");
        assert!(!wall.contains("<button"), "{wall}");
        let sign_in = wall.find("href=\"/login\"").expect("sign in link");
        let home = wall.find(">Return home<").expect("home link");
        assert!(sign_in < home, "{wall}");
        assert!(wall.contains("class=\"btn is-primary\""), "{wall}");
        // The gate's status line is there, with nothing to say next to a card.
        assert!(
            html.contains("<p class=\"visually-hidden\" role=\"status\"></p>"),
            "{html}"
        );
    }

    #[test]
    fn a_member_gets_a_loading_status_not_a_wall() {
        let html = render_route("/polls", Some(OrgRole::Member));
        assert!(!html.contains("<section class=\"wall\""), "{html}");
        assert!(
            html.contains("<p class=\"member-gate-status\" role=\"status\">Loading polls</p>"),
            "{html}"
        );
    }

    fn member() -> AuthState {
        AuthState {
            user: Some(UserInfo {
                id: "u1".into(),
                username: "Player01".into(),
                avatar_url: None,
                role: Some(OrgRole::Member),
            }),
            loading: false,
        }
    }

    #[test]
    fn gate_view_follows_auth_then_the_fetch() {
        let guest = AuthState {
            user: None,
            loading: false,
        };
        let booting = AuthState {
            user: None,
            loading: true,
        };
        let member = member();
        assert_eq!(gate_view(&guest, None, false, true), GateView::SignIn);
        assert_eq!(gate_view(&booting, None, false, true), GateView::Loading);
        assert_eq!(gate_view(&member, None, false, true), GateView::Loading);
        assert_eq!(
            gate_view(&member, Some("HTTP error 401: Unauthorized"), true, false),
            GateView::SignIn
        );
        assert_eq!(
            gate_view(&member, Some("HTTP error 403: Forbidden"), true, false),
            GateView::Inactive
        );
        assert_eq!(
            gate_view(&member, Some("HTTP error 500: Internal error"), true, false),
            GateView::Failed { retrying: false }
        );
        // Retry: the error is cleared and the fetch is in flight again. The
        // card stays, so the focused button is not unmounted.
        assert_eq!(
            gate_view(&member, None, true, true),
            GateView::Failed { retrying: true }
        );
        // First load: nothing failed before.
        assert_eq!(gate_view(&member, None, false, true), GateView::Loading);
    }

    #[test]
    fn gate_status_text_changes_with_the_state() {
        assert_eq!(gate_status(GateView::Loading, "polls"), "Loading polls");
        assert_eq!(
            gate_status(GateView::Failed { retrying: false }, "polls"),
            "Couldn't load polls."
        );
        assert_eq!(
            gate_status(GateView::Failed { retrying: true }, "polls"),
            "Loading polls"
        );
        for view in [GateView::SignIn, GateView::Apply, GateView::Inactive] {
            assert_eq!(gate_status(view, "polls"), "");
        }
    }

    #[derive(Props, Clone, PartialEq)]
    struct RetryProps {
        retrying: bool,
    }

    #[allow(non_snake_case)]
    fn RetryProbe(props: RetryProps) -> Element {
        let refresh = use_signal(|| 0u64);
        retry_panel("polls", props.retrying, refresh)
    }

    fn retry_card(retrying: bool) -> String {
        let html = crate::components::ui::test_support::render_routed(move || {
            rsx! { RetryProbe { retrying } }
        });
        wall(&html).to_string()
    }

    #[test]
    fn failed_load_is_a_plain_sentence_and_retry_keeps_focus() {
        let card = retry_card(false);
        assert!(
            card.contains("<h2 class=\"wall-title\">Couldn&#39;t load polls</h2>"),
            "{card}"
        );
        // No raw client error in the lead.
        assert!(
            card.contains("<p class=\"wall-lead\">Check your connection and try again.</p>"),
            "{card}"
        );
        assert!(!card.contains("HTTP error"), "{card}");
        assert!(
            card.contains("<button type=\"button\" class=\"btn is-primary\">Retry</button>"),
            "{card}"
        );

        let busy = retry_card(true);
        assert!(busy.contains(">Retrying...</button>"), "{busy}");
        assert!(busy.contains("aria-disabled=\"true\""), "{busy}");
        assert!(busy.contains("aria-busy=\"true\""), "{busy}");
        assert!(!busy.contains(" disabled"), "{busy}");
        assert!(busy.contains(">Return home</a>"), "{busy}");
    }

    #[test]
    fn http_status_matches_client_error_display() {
        assert!(is_http_status("HTTP error 401: Unauthorized", 401));
        assert!(is_http_status("HTTP error: 401", 401));
        assert!(is_http_status("HTTP error 403: Not an org member", 403));
        assert!(!is_http_status("HTTP error 500: Internal error", 401));
        assert!(!is_http_status("Network error: failed", 401));
        // Must not treat 4010 / substring noise as 401
        assert!(!is_http_status("HTTP error 4010: weird", 401));
    }

    #[test]
    fn logged_out_is_not_org_member() {
        let a = AuthState {
            user: None,
            loading: false,
        };
        assert!(!a.is_logged_in());
        assert!(!a.is_org_member());
    }

    #[test]
    fn recruit_is_org_member() {
        let a = AuthState {
            user: Some(UserInfo {
                id: "u1".into(),
                username: "dev".into(),
                avatar_url: None,
                role: Some(OrgRole::Recruit),
            }),
            loading: false,
        };
        assert!(a.is_org_member());
    }
}
