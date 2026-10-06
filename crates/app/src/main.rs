// Many components, the canvas rendering subsystem, and helper tables are built
// ahead of the routes/pages that will consume them (features pending wiring
// after the desktop-canvas merge). Allow dead_code crate-wide rather than delete
// in-progress work; tighten once everything is wired up.
#![allow(dead_code)]

mod canvas;
mod components;
mod hooks;
mod keybindings;
mod layouts;
mod pages;
mod routes;
mod state;
mod styles;
mod theme;
mod util;

use dioxus::prelude::*;

use components::ToastProvider;
use routes::Route;
use state::AuthState;

fn main() {
    // The router drops undeclared query params as soon as it mounts. Snapshot
    // `/login?error=registration_closed` while the address bar still has it.
    // Login consumes that snapshot on its first mount. One process-wide slot;
    // see `LoginBannerSlot`.
    // Untested call site: login tests cover the probe harness, not `main`.
    pages::capture_initial_login_banner();
    dioxus::launch(App);
}

#[cfg(feature = "desktop")]
const DESKTOP_CANVAS_JS: &str = include_str!("../assets/desktop_canvas.js");

#[component]
fn App() -> Element {
    // Once per App mount, before `Router` rewrites the URL. `main` already
    // snapshotted on a normal boot, so this is a no-op then. A re-render must
    // not take the lock again. Same `LoginBannerSlot` as `main`.
    // Untested call site: login tests cover the probe harness, not `App`.
    use_hook(|| {
        pages::capture_initial_login_banner();
    });

    // Provide auth state to entire app
    let auth = use_signal(AuthState::new);
    use_context_provider(|| auth);
    state::auth::use_auth_init();

    // Redirect to first-boot setup when no admin exists yet.
    use_future(|| async move {
        use scuffed_api_client::ApiClient;
        use scuffed_types::SetupStatusResponse;
        if let Ok(status) = ApiClient::web()
            .fetch::<SetupStatusResponse>("/api/auth/setup-status")
            .await
            && status.needs_setup
        {
            let path = web_sys::window()
                .and_then(|w| w.location().pathname().ok())
                .unwrap_or_default();
            if path != "/setup" {
                let _ = web_sys::window().and_then(|w| w.location().set_href("/setup").ok());
            }
        }
    });

    // One settings fetch for the document head and every public consumer.
    // `document::Title` updates the existing `<title>` once a real org name
    // exists. Description and Open Graph tags stay in index.html: `document::Meta`
    // would append a second copy and ignore later prop changes.
    let site_settings = state::provide_site_settings();
    let resolved = site_settings.resolved();
    let loaded_settings = state::loaded_site_settings(resolved.as_ref());
    let page_title = loaded_settings.and_then(|s| {
        let title = state::document_title(Some(&s.org_name));
        if title.is_empty() { None } else { Some(title) }
    });
    // Update the one `<link rel="icon">` from index.html. Pending leaves that
    // static href. A non-empty org name gets a data URI, including initials CL.
    // `document::Link` appends and then ignores href changes, so this effect
    // writes the existing element. Crawlers still see the shell's single tag.
    use_effect(move || {
        let resolved = site_settings.resolved();
        let settled_name = match resolved.as_ref() {
            None => None,
            Some(Ok(settings)) => Some(settings.org_name.clone()),
            Some(Err(_)) => Some(String::new()),
        };
        let Some(href) = theme::brand::runtime_favicon_href(settled_name.as_deref()) else {
            return;
        };
        let Some(window) = web_sys::window() else {
            return;
        };
        let Some(document) = window.document() else {
            return;
        };
        let Ok(Some(link)) = document.query_selector("link[rel='icon']") else {
            return;
        };
        let _ = link.set_attribute("href", &href);
    });
    // Unknown settings use a gray accent. Product purple is a real brand and
    // must not paint before the embedded block or `/api/settings` says so.
    let brand_theme_css = {
        use theme::brand::BrandConfig;
        match loaded_settings.as_ref() {
            Some(s) => theme::theme_css(&BrandConfig::from_settings(
                &s.brand_accent_dark,
                &s.brand_accent_light,
            )),
            None => theme::theme_css(&BrandConfig::pending()),
        }
    };

    #[cfg(feature = "desktop")]
    {
        use_hook(|| {
            document::eval(DESKTOP_CANVAS_JS);
        });
    }

    rsx! {
        // index.html owns description, og:title, og:description, and og:site_name
        // so the server can fill the one copy of each. Title is the exception:
        // `document::Title` replaces the text of the existing element.
        if let Some(title) = page_title.as_ref() {
            document::Title { "{title}" }
        }
        document::Meta {
            name: "theme-color",
            content: "{theme::tokens::THEME_COLOR}",
        }
        // Preload in index.html starts the download without blocking boot paint.
        // Applying it here avoids an inline onload, which script-src would block.
        document::Link {
            rel: "stylesheet",
            href: "https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Space+Grotesk:wght@500;600;700&family=JetBrains+Mono:wght@500&display=swap",
        }
        document::Stylesheet {
            href: asset!("/assets/tailwind.css")
        }
        style { "{brand_theme_css}" }
        style { {styles::common::CSS} }
        style { {components::ui::ui_css()} }
        theme::ThemeProvider {
            ToastProvider {
                Router::<Route> {}
            }
        }
    }
}
