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
    dioxus::launch(App);
}

#[cfg(feature = "desktop")]
const DESKTOP_CANVAS_JS: &str = include_str!("../assets/desktop_canvas.js");

#[component]
fn App() -> Element {
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
    let loaded_settings = {
        let slot = site_settings.resource.read();
        state::loaded_site_settings(slot.as_ref()).cloned()
    };
    let page_title = state::document_title(loaded_settings.as_ref().map(|s| s.org_name.as_str()));
    let page_description = loaded_settings
        .as_ref()
        .map(|s| {
            let d = s.site_description.trim();
            if d.is_empty() {
                format!("{} — gaming clan", s.org_name)
            } else {
                d.to_string()
            }
        })
        .unwrap_or_else(|| "Gaming clan platform".into());
    let brand_theme_css = {
        use theme::brand::BrandConfig;
        let (dark, light) = loaded_settings
            .as_ref()
            .map(|s| (s.brand_accent_dark.clone(), s.brand_accent_light.clone()))
            .unwrap_or_default();
        theme::theme_css(&BrandConfig::from_settings(&dark, &light))
    };

    #[cfg(feature = "desktop")]
    {
        use_hook(|| {
            document::eval(DESKTOP_CANVAS_JS);
        });
    }

    rsx! {
        // Runtime head — org name from settings once loaded
        document::Title { "{page_title}" }
        document::Meta {
            name: "description",
            content: "{page_description}",
        }
        document::Meta {
            property: "og:title",
            content: "{page_title}",
        }
        document::Meta {
            property: "og:description",
            content: "{page_description}",
        }
        document::Meta {
            name: "theme-color",
            content: "{theme::tokens::THEME_COLOR}",
        }
        document::Link {
            rel: "icon",
            href: asset!("/assets/favicon.svg"),
            r#type: "image/svg+xml",
        }
        document::Stylesheet {
            href: asset!("/assets/tailwind.css")
        }
        document::Link {
            rel: "preconnect",
            href: "https://fonts.googleapis.com",
        }
        document::Link {
            rel: "preconnect",
            href: "https://fonts.gstatic.com",
            crossorigin: "anonymous",
        }
        document::Link {
            rel: "stylesheet",
            href: "https://fonts.googleapis.com/css2?family=Inter:wght@400;500;600;700&family=Space+Grotesk:wght@500;600;700&family=JetBrains+Mono:wght@500&display=swap",
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
