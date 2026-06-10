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

    #[cfg(feature = "desktop")]
    {
        use_hook(|| {
            document::eval(DESKTOP_CANVAS_JS);
        });
    }

    rsx! {
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
        style { {theme::theme_css_current()} }
        style { {styles::common::CSS} }
        theme::ThemeProvider {
            ToastProvider {
                Router::<Route> {}
            }
        }
    }
}
