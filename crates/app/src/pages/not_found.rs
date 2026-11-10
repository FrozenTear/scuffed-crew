use dioxus::prelude::*;

use crate::routes::Route;

const NOT_FOUND_CSS: &str = r#"
    .nf-page {
        min-height: 50vh;
        display: flex;
        flex-direction: column;
        align-items: center;
        justify-content: center;
        text-align: center;
        padding: 4rem 1.5rem;
        color: var(--text);
    }
    .nf-code {
        font-family: var(--font-head);
        font-size: clamp(4rem, 18vw, 6.5rem);
        line-height: 1;
        letter-spacing: 0.04em;
        color: var(--text);
        margin: 0;
    }
    .nf-title {
        font-family: var(--font-head);
        font-size: 1.25rem;
        margin: 1rem 0 0.35rem;
        color: var(--text);
    }
    .nf-msg {
        color: var(--text-2);
        font-size: 0.95rem;
        max-width: 28rem;
        margin: 0 0 1.5rem;
        line-height: 1.5;
    }
    .nf-path {
        font-family: var(--font-mono);
        font-size: 0.72rem;
        color: var(--text-3);
        letter-spacing: 0.04em;
        margin-bottom: 1.5rem;
        word-break: break-all;
    }
    .nf-link {
        display: inline-flex;
        align-items: center;
        padding: 0.55rem 1.1rem;
        background: var(--accent);
        color: var(--accent-fg);
        font-family: var(--font-mono);
        font-size: 0.72rem;
        letter-spacing: 0.1em;
        text-transform: uppercase;
        text-decoration: none;
        border-radius: var(--radius-md, 8px);
    }
    .nf-link:hover { filter: brightness(1.08); }
    .nf-links {
        display: flex;
        flex-wrap: wrap;
        justify-content: center;
        gap: var(--space-3);
    }
    .nf-link-quiet {
        background: transparent;
        color: var(--text);
        border: 1px solid var(--border);
    }
"#;

/// One link under the not-found message. The first link in a panel is the
/// primary button, the rest are quiet outline buttons.
#[derive(Clone, PartialEq)]
pub struct NotFoundLink {
    pub label: &'static str,
    pub to: Route,
}

/// The one not-found view. Used by the catch-all route and by pages whose
/// record does not exist (match, member, game), so a missing thing always
/// reads the same and always offers a next step.
#[component]
pub fn NotFoundPanel(
    title: &'static str,
    message: &'static str,
    #[props(default)] path: String,
    links: Vec<NotFoundLink>,
) -> Element {
    rsx! {
        crate::styles::StyleOnce { id: "scuffed-css-pages-not_found-not-found-css", css: NOT_FOUND_CSS }
        div { class: "nf-page",
            h1 { class: "nf-code", "404" }
            p { class: "nf-title", "{title}" }
            p { class: "nf-msg", "{message}" }
            if !path.is_empty() {
                p { class: "nf-path", "{path}" }
            }
            div { class: "nf-links",
                for (i, link) in links.iter().enumerate() {
                    Link {
                        key: "{link.label}",
                        to: link.to.clone(),
                        class: if i == 0 { "nf-link" } else { "nf-link nf-link-quiet" },
                        "{link.label}"
                    }
                }
            }
        }
    }
}

#[component]
pub fn NotFound(segments: Vec<String>) -> Element {
    let path = if segments.is_empty() {
        String::new()
    } else {
        format!("/{}", segments.join("/"))
    };

    rsx! {
        NotFoundPanel {
            title: "Page not found",
            message: "That route doesn't exist. It may have moved, or the link is wrong.",
            path,
            links: vec![NotFoundLink { label: "Return home", to: Route::Home {} }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::state::AuthState;

    thread_local! {
        static ROUTE_PATH: std::cell::RefCell<String> = const { std::cell::RefCell::new(String::new()) };
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

    fn render_route(path: &str) -> String {
        ROUTE_PATH.with(|slot| *slot.borrow_mut() = path.to_string());
        let mut dom = VirtualDom::new(shell);
        dom.rebuild_in_place();
        dioxus_ssr::render(&dom)
    }

    #[test]
    fn unknown_path_renders_not_found_inside_the_public_layout() {
        let html = render_route("/definitely/not/a/page");
        assert!(html.contains("nf-page"), "{html}");
        assert!(html.contains("Page not found"), "{html}");
        assert!(html.contains("/definitely/not/a/page"), "{html}");
        assert!(
            html.contains("site-nav"),
            "the 404 must keep the site nav: {html}"
        );
    }

    #[test]
    fn unknown_admin_and_strategy_paths_also_keep_the_nav() {
        for path in ["/admin/nope", "/strategy/nope"] {
            let html = render_route(path);
            assert!(html.contains("nf-page"), "{path}: {html}");
            assert!(html.contains("site-nav"), "{path}: {html}");
        }
    }
}
