use dioxus::prelude::*;

use crate::routes::Route;

const NAV_CSS: &str = r#"
    .site-nav {
        position: fixed;
        top: 0;
        left: 0;
        right: 0;
        z-index: 100;
        display: flex;
        align-items: center;
        justify-content: space-between;
        padding: 0 2rem;
        height: 60px;
        background: rgba(8, 8, 12, 0.85);
        backdrop-filter: blur(12px);
        border-bottom: 1px solid var(--border);
    }
    .nav-mark {
        display: flex;
        align-items: center;
        gap: 0.6rem;
        font-family: var(--font-display-hero);
        font-size: 1.1rem;
        color: var(--text-bright);
        text-transform: uppercase;
        letter-spacing: 0.04em;
    }
    .nav-icon {
        width: 32px;
        height: 32px;
        background: var(--accent);
        border-radius: 6px;
        display: flex;
        align-items: center;
        justify-content: center;
        font-weight: 700;
        font-size: 0.75rem;
        color: white;
    }
    .nav-links {
        display: flex;
        list-style: none;
        gap: 0.25rem;
    }
    .nav-links a {
        padding: 0.4rem 0.75rem;
        color: var(--text-secondary);
        font-size: 0.85rem;
        border-radius: 6px;
        transition: color 0.15s, background 0.15s;
    }
    .nav-links a:hover {
        color: var(--text-bright);
        background: var(--bg-card);
    }
    .nav-cta {
        background: var(--accent) !important;
        color: white !important;
        padding: 0.4rem 1rem !important;
        font-weight: 600;
    }
    .nav-cta:hover {
        filter: brightness(1.15);
    }
    .nav-hamburger {
        display: none;
        flex-direction: column;
        gap: 5px;
        background: none;
        border: none;
        cursor: pointer;
        padding: 4px;
    }
    .nav-hamburger span {
        width: 22px;
        height: 2px;
        background: var(--text-secondary);
        transition: transform 0.2s, opacity 0.2s;
    }
    .nav-hamburger.open span:nth-child(1) { transform: rotate(45deg) translate(5px, 5px); }
    .nav-hamburger.open span:nth-child(2) { opacity: 0; }
    .nav-hamburger.open span:nth-child(3) { transform: rotate(-45deg) translate(5px, -5px); }
    .nav-overlay {
        display: none;
        position: fixed;
        inset: 0;
        z-index: 99;
        background: rgba(8, 8, 12, 0.97);
        flex-direction: column;
        align-items: center;
        justify-content: center;
        gap: 1.25rem;
    }
    .nav-overlay.open { display: flex; }
    .nav-overlay a {
        color: var(--text-bright);
        font-size: 1.2rem;
        font-family: var(--font-display);
    }
    .site-footer {
        border-top: 1px solid var(--border);
        padding: 2rem;
        text-align: center;
        color: var(--text-muted);
        font-size: 0.8rem;
    }
    @media (max-width: 768px) {
        .nav-links { display: none; }
        .nav-hamburger { display: flex; }
    }
"#;

#[component]
pub fn PublicLayout() -> Element {
    let mut menu_open = use_signal(|| false);

    let hamburger_class = if menu_open() {
        "nav-hamburger open"
    } else {
        "nav-hamburger"
    };

    let overlay_class = if menu_open() {
        "nav-overlay open"
    } else {
        "nav-overlay"
    };

    rsx! {
        style { {NAV_CSS} }
        nav { class: "site-nav",
            Link { to: Route::Home {}, class: "nav-mark",
                div { class: "nav-icon", "SC" }
                span { "The Scuffed Crew" }
            }
            ul { class: "nav-links",
                li { Link { to: Route::Members {}, "Members" } }
                li { Link { to: Route::News {}, "News" } }
                li { Link { to: Route::Tournaments {}, "Tournaments" } }
                li { Link { to: Route::StrategyBrowse {}, "Strategy" } }
                li { Link { to: Route::Apply {}, class: "nav-cta", "Apply" } }
            }
            button {
                class: hamburger_class,
                aria_label: "Toggle menu",
                onclick: move |_| menu_open.toggle(),
                span {}
                span {}
                span {}
            }
        }

        div { class: overlay_class,
            Link { to: Route::Members {}, onclick: move |_| menu_open.set(false), "Members" }
            Link { to: Route::News {}, onclick: move |_| menu_open.set(false), "News" }
            Link { to: Route::Tournaments {}, onclick: move |_| menu_open.set(false), "Tournaments" }
            Link { to: Route::StrategyBrowse {}, onclick: move |_| menu_open.set(false), "Strategy" }
            Link { to: Route::Apply {}, onclick: move |_| menu_open.set(false),
                class: "nav-cta",
                style: "display: inline-block; padding: 0.5rem 1.5rem; border-radius: 6px; margin-top: 1rem;",
                "Apply"
            }
        }

        main { style: "padding-top: 60px; min-height: 100vh;",
            Outlet::<Route> {}
        }

        footer { class: "site-footer",
            "© 2026 The Scuffed Crew · Est. EMEA"
        }
    }
}
