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
    // Title stays blank until the real org name arrives.
    let site_settings = state::provide_site_settings();
    let resolved = site_settings.resolved();
    let loaded_settings = state::loaded_site_settings(resolved.as_ref());
    // Leave the server-written <title>, og:title, and description alone until
    // real settings exist. An empty title would wipe the clan name, and a
    // synthesized "gaming clan" blurb is a template default.
    let page_title = loaded_settings.and_then(|s| {
        let title = state::document_title(Some(&s.org_name));
        if title.is_empty() { None } else { Some(title) }
    });
    let page_description = loaded_settings.and_then(|s| {
        let description = s.site_description.trim();
        if description.is_empty() {
            None
        } else {
            Some(description.to_string())
        }
    });
    let icon_href = match loaded_settings {
        Some(settings) if !settings.org_name.trim().is_empty() => {
            let initials = scuffed_types::org_initials(&settings.org_name);
            if initials == "CL" {
                asset!("/assets/favicon.svg").to_string()
            } else {
                theme::brand::org_favicon_data_uri(&initials)
            }
        }
        _ => asset!("/assets/favicon.svg").to_string(),
    };
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
        // Runtime head. index.html already has one title, one og:title, and
        // one description for the server to fill. Update them only after
        // settings exist.
        if let Some(title) = page_title.as_ref() {
            document::Title { "{title}" }
            document::Meta {
                property: "og:title",
                content: "{title}",
            }
        }
        if let Some(desc) = page_description.as_ref() {
            document::Meta {
                name: "description",
                content: "{desc}",
            }
            document::Meta {
                property: "og:description",
                content: "{desc}",
            }
        }
        document::Meta {
            name: "theme-color",
            content: "{theme::tokens::THEME_COLOR}",
        }
        document::Link {
            rel: "icon",
            href: "{icon_href}",
            r#type: "image/svg+xml",
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
