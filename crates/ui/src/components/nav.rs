use leptos::ev;
use leptos::prelude::*;

/// A navigation link definition
#[derive(Clone)]
pub struct NavLink {
    pub href: String,
    pub label: String,
    pub icon: Option<String>,
}

impl NavLink {
    pub fn new(href: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            href: href.into(),
            label: label.into(),
            icon: None,
        }
    }

    pub fn with_icon(mut self, icon: impl Into<String>) -> Self {
        self.icon = Some(icon.into());
        self
    }
}

/// Parameterized app navigation bar.
///
/// Takes a logo slot, a list of nav links, and a user menu slot so each
/// app can customize branding and content while sharing layout/behavior.
#[component]
pub fn AppNav(
    /// The logo / brand area (rendered in header-left)
    logo: Children,
    /// Navigation links
    #[prop(default = vec![])]
    links: Vec<NavLink>,
    /// User menu area (rendered in header-right), typically auth-dependent
    #[prop(optional)]
    user_menu: Option<Children>,
) -> impl IntoView {
    let (mobile_open, set_mobile_open) = signal(false);
    let close_nav = move |_: ev::MouseEvent| set_mobile_open.set(false);
    let links_clone = links.clone();

    view! {
        <header class="sc-nav">
            <div class="sc-nav-left">
                {logo()}
                <nav class="sc-nav-links">
                    {links.iter().map(|link| {
                        let href = link.href.clone();
                        let label = link.label.clone();
                        view! { <a href={href} class="sc-nav-link">{label}</a> }
                    }).collect_view()}
                </nav>
            </div>

            <div class="sc-nav-right">
                {user_menu.map(|f| f())}
            </div>

            <button
                class="sc-mobile-menu-btn"
                class:open=move || mobile_open.get()
                on:click=move |_| set_mobile_open.update(|v| *v = !*v)
                aria-label="Toggle navigation"
            >
                <span class="sc-hamburger-line"></span>
                <span class="sc-hamburger-line"></span>
                <span class="sc-hamburger-line"></span>
            </button>
        </header>

        <div
            class="sc-mobile-overlay"
            class:open=move || mobile_open.get()
            on:click=close_nav
        >
            <nav
                class="sc-mobile-drawer"
                class:open=move || mobile_open.get()
                on:click=|e: ev::MouseEvent| e.stop_propagation()
            >
                <div class="sc-mobile-links">
                    {links_clone.iter().map(|link| {
                        let href = link.href.clone();
                        let label = link.label.clone();
                        let icon = link.icon.clone();
                        view! {
                            <a href={href} class="sc-mobile-link" on:click=close_nav>
                                {icon.map(|i| view! { <span class="sc-mobile-icon">{i}</span> })}
                                {label}
                            </a>
                        }
                    }).collect_view()}
                </div>
            </nav>
        </div>
    }
}

pub const NAV_STYLES: &str = r#"
.sc-nav {
    display: flex;
    align-items: center;
    justify-content: space-between;
    padding: 0.75rem 2rem;
    background: var(--bg-surface);
    border-bottom: 1px solid var(--border);
    position: sticky;
    top: 0;
    z-index: 100;
}
.sc-nav-left {
    display: flex;
    align-items: center;
    gap: 2rem;
}
.sc-nav-links {
    display: flex;
    gap: 1.5rem;
}
.sc-nav-link {
    color: var(--text-secondary);
    text-decoration: none;
    font-family: var(--font-display);
    font-weight: 600;
    font-size: 0.85rem;
    text-transform: uppercase;
    letter-spacing: 0.05em;
    transition: color 0.2s;
}
.sc-nav-link:hover {
    color: var(--accent-bright);
}
.sc-nav-right {
    display: flex;
    align-items: center;
    gap: 1rem;
}
.sc-mobile-menu-btn {
    display: none;
    flex-direction: column;
    gap: 4px;
    background: none;
    border: none;
    cursor: pointer;
    padding: 0.5rem;
}
.sc-hamburger-line {
    width: 24px;
    height: 2px;
    background: var(--text-secondary);
    transition: all 0.3s;
}
.sc-mobile-overlay {
    display: none;
    position: fixed;
    inset: 0;
    background: rgba(0,0,0,0.6);
    z-index: 200;
}
.sc-mobile-overlay.open { display: block; }
.sc-mobile-drawer {
    position: fixed;
    top: 0;
    right: 0;
    bottom: 0;
    width: 280px;
    background: var(--bg-surface);
    border-left: 1px solid var(--border);
    padding: 2rem 1.5rem;
    transform: translateX(100%);
    transition: transform 0.3s ease;
}
.sc-mobile-drawer.open { transform: translateX(0); }
.sc-mobile-links {
    display: flex;
    flex-direction: column;
    gap: 0.25rem;
}
.sc-mobile-link {
    display: flex;
    align-items: center;
    gap: 0.75rem;
    padding: 0.75rem 1rem;
    color: var(--text-primary);
    text-decoration: none;
    font-family: var(--font-body);
    border-radius: 8px;
    transition: background 0.2s;
}
.sc-mobile-link:hover { background: var(--bg-elevated); }
.sc-mobile-icon { font-size: 1.1rem; }
@media (max-width: 768px) {
    .sc-nav-links, .sc-nav-right { display: none; }
    .sc-mobile-menu-btn { display: flex; }
}
"#;
