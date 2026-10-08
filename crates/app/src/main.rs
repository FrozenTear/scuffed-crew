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

/// `index.html` links this stable path. dx only copies assets Rust references,
/// and a hashed name would not match that href. The Containerfile ships the
/// dx output, so this is what puts the file in the image. `#[used]` keeps the
/// reference when nothing reads the static.
#[used]
static SITE_FAVICON: manganis::Asset = asset!(
    "/assets/favicon.svg",
    manganis::AssetOptions::builder().with_hash_suffix(false)
);

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
    // Description and Open Graph tags stay in index.html: `document::Meta`
    // would append a second copy and ignore later prop changes.
    //
    // The existing `<title>` is updated once a real org name exists.
    // On web, `Document::set_title` writes that element. `document::Title`
    // goes through Dioxus `WebDocument::set_title`, which runs
    // `document.title = ...` via `js_sys::Function::new_with_args`
    // (`new Function`). That is the string-as-JavaScript the report-only
    // CSP attributes to the scuffed-app bundle on home and /leaderboards.
    // Desktop keeps `document::Title` for the window title (no page CSP).
    let site_settings = state::provide_site_settings();
    let resolved = site_settings.resolved.read();
    let loaded_settings = state::loaded_site_settings(resolved.as_ref());
    let page_title = loaded_settings.and_then(|s| {
        let title = state::document_title(Some(&s.org_name));
        if title.is_empty() { None } else { Some(title) }
    });
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        use_effect(move || {
            let resolved = site_settings.resolved.read();
            let Some(title) = state::loaded_site_settings(resolved.as_ref()).and_then(|s| {
                let title = state::document_title(Some(&s.org_name));
                if title.is_empty() { None } else { Some(title) }
            }) else {
                return;
            };
            let Some(document) = web_sys::window().and_then(|window| window.document()) else {
                return;
            };
            document.set_title(&title);
        });
    }
    // Update the one `<link rel="icon">` from index.html. Pending leaves that
    // static href. A non-empty org name gets a data URI, including initials CL.
    // `document::Link` appends and then ignores href changes, so this effect
    // writes the existing element. Crawlers still see the shell's single tag.
    use_effect(move || {
        let resolved = site_settings.resolved.read();
        let settled_name = match resolved.as_ref() {
            None => None,
            Some(Ok(settings)) => Some(settings.org_name.clone()),
            Some(Err(_)) => Some(String::new()),
        };
        let Some(href) = theme::brand::runtime_favicon_href(settled_name.as_deref()) else {
            return;
        };
        // `web_sys::window()` panics off wasm. Desktop has no document to update.
        #[cfg(all(feature = "web", target_arch = "wasm32"))]
        {
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
        }
        #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
        {
            let _ = href;
        }
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
        // so the server can fill the one copy of each. Web title updates go
        // through `Document::set_title` above. Desktop still uses
        // `document::Title`, which replaces the window title.
        {document_title_element(page_title.as_deref())}
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

/// Desktop window title. The web build sets `document.title` with `web_sys`
/// instead, so Dioxus does not `eval` the assignment. See `App`.
fn document_title_element(title: Option<&str>) -> Element {
    #[cfg(all(feature = "web", target_arch = "wasm32"))]
    {
        let _ = title;
        rsx! {}
    }
    #[cfg(not(all(feature = "web", target_arch = "wasm32")))]
    {
        if let Some(title) = title {
            rsx! {
                document::Title { "{title}" }
            }
        } else {
            rsx! {}
        }
    }
}

#[cfg(test)]
mod tests {
    /// The live shell must contain exactly one of each rewritten head tag,
    /// including `og:site_name`. Comments are removed before those counts.
    #[test]
    fn index_html_has_one_of_each_rewritten_head_tag() {
        let html = include_str!("../index.html");
        let visible = strip_html_comments(html);
        for needle in [
            "<title>",
            "name=\"description\"",
            "property=\"og:title\"",
            "property=\"og:description\"",
            "property=\"og:site_name\"",
        ] {
            let count = visible.matches(needle).count();
            assert_eq!(count, 1, "{needle} appears {count} times");
        }
        // dx injects a loader at every `</body>`, including one inside a comment.
        assert_eq!(
            html.matches("</body>").count(),
            1,
            "index.html must not mention the closing body tag except the real one"
        );
        // `<!-->` and `<!--->` are empty comments. They must not swallow the
        // following text the way a scan for `-->` would.
        assert_eq!(strip_html_comments("a<!-->b<!--->c<!--x-->d"), "abcd");
    }

    /// dx still sees a closing body tag inside a comment, so that count stays
    /// on the raw file. Head-tag counts use the stripped text.
    fn strip_html_comments(html: &str) -> String {
        let mut out = String::with_capacity(html.len());
        let mut rest = html;
        while let Some(start) = rest.find("<!--") {
            out.push_str(&rest[..start]);
            rest = &rest[start + 4..];
            // Empty comments close immediately. A later `-->` is a different comment.
            if let Some(tail) = rest.strip_prefix('>').or_else(|| rest.strip_prefix("->")) {
                rest = tail;
                continue;
            }
            match rest.find("-->") {
                Some(end) => rest = &rest[end + 3..],
                None => return out,
            }
        }
        out.push_str(rest);
        out
    }
}
