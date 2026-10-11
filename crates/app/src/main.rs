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

    // `/link` arms a return flag before sending a signed-out member to login.
    // OAuth comes back on `/`, so follow that flag once the session exists.
    // The flag is only the path `/link`. It never carries a device code.
    use_effect(move || {
        let state = auth();
        pages::redirect_after_login_if_needed(state.loading, state.is_logged_in());
    });

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

    // Desktop only. Native Rust cannot call web_sys, so the canvas module is
    // injected as script text. This is not in the web bundle. The site CSP
    // does not need 'unsafe-eval' for it.
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

    /// String-eval in this crate is the desktop canvas bridge. The web bundle
    /// sets the document title with `web_sys` and must not grow a new
    /// `document::eval` / `Function` constructor.
    #[test]
    fn string_eval_call_sites_are_desktop_only() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut hits = Vec::new();
        collect_string_eval(&root, &root, &mut hits);
        let mut files: Vec<&str> = hits.iter().map(|(path, _, _)| path.as_str()).collect();
        files.sort();
        files.dedup();
        assert_eq!(
            files,
            vec![
                "src/components/strategy/desktop_map_canvas.rs",
                "src/main.rs",
            ],
            "unexpected string-eval sites: {hits:?}"
        );
        let main_hits: Vec<_> = hits
            .iter()
            .filter(|(path, _, _)| path == "src/main.rs")
            .collect();
        assert_eq!(main_hits.len(), 1, "{main_hits:?}");
        let canvas_hits: Vec<_> = hits
            .iter()
            .filter(|(path, _, _)| path.ends_with("desktop_map_canvas.rs"))
            .collect();
        assert_eq!(canvas_hits.len(), 5, "{canvas_hits:?}");

        let main = include_str!("main.rs");
        let eval_at = main
            .find("document::eval(DESKTOP_CANVAS_JS)")
            .expect("desktop inject");
        let cfg_at = main[..eval_at]
            .rfind("#[cfg(feature = \"desktop\")]")
            .expect("desktop cfg");
        assert!(
            !main[cfg_at..eval_at].contains("feature = \"web\""),
            "canvas inject must stay on the desktop target"
        );
        let title_fn = main
            .find("fn document_title_element")
            .expect("title helper");
        let title_body = &main[title_fn..];
        let title_end = title_body
            .find("\n#[cfg(test)]")
            .unwrap_or(title_body.len());
        let title_body = &title_body[..title_end];
        let web_at = title_body
            .find("all(feature = \"web\", target_arch = \"wasm32\")")
            .expect("web cfg");
        let desktop_at = title_body
            .find("not(all(feature = \"web\", target_arch = \"wasm32\"))")
            .expect("desktop cfg");
        assert!(web_at < desktop_at);
        assert!(
            !title_body[web_at..desktop_at].contains("document::Title"),
            "web title helper must not use document::Title"
        );
        assert!(
            title_body[desktop_at..].contains("document::Title"),
            "desktop window title still uses document::Title"
        );
        assert!(
            main.contains("document.set_title"),
            "web title must use web_sys Document::set_title"
        );
    }

    #[test]
    fn string_eval_scan_catches_bare_eval_raw_strings_and_new_no_args() {
        let bare = format!("{}{}", "ev", "al(\"x\")");
        assert!(line_has_string_eval(&bare), "{bare}");
        let raw = format!("let code = r#\"{}{}1)\"#;", "ev", "al(");
        assert!(line_has_string_eval(&raw), "{raw}");
        let hashed = format!("let code = r##\"{}{}1)\"##;", "ev", "al(");
        assert!(line_has_string_eval(&hashed), "{hashed}");
        let quoted = format!("let code = \"{}{}1)\";", "ev", "al(");
        assert!(
            !line_has_string_eval(&quoted),
            "a normal string mention is not a call: {quoted}"
        );
        let ctor = format!("js_sys::{}{}", "Function::new_", "no_args(\"return 1\")");
        assert!(line_has_string_eval(&ctor), "{ctor}");
        let ctor_raw = format!("let code = r#\"{ctor}\"#;");
        assert!(line_has_string_eval(&ctor_raw), "{ctor_raw}");
        assert!(!line_has_string_eval("collect_string_eval(&root)"));
        assert!(!line_has_string_eval("// eval(\"hidden\")"));
    }

    fn collect_string_eval(
        crate_src: &std::path::Path,
        dir: &std::path::Path,
        hits: &mut Vec<(String, usize, String)>,
    ) {
        let entries =
            std::fs::read_dir(dir).unwrap_or_else(|err| panic!("read {}: {err}", dir.display()));
        for entry in entries {
            let entry = entry.expect("dir entry");
            let path = entry.path();
            if path.is_dir() {
                collect_string_eval(crate_src, &path, hits);
                continue;
            }
            if path.extension().and_then(|ext| ext.to_str()) != Some("rs") {
                continue;
            }
            let text = std::fs::read_to_string(&path).expect("read rs");
            let rel = path
                .strip_prefix(crate_src.parent().expect("crates/app"))
                .expect("relative")
                .to_string_lossy()
                .replace('\\', "/");
            for (idx, line) in text.lines().enumerate() {
                let trimmed = line.trim_start();
                if trimmed.starts_with("//")
                    || trimmed.starts_with("///")
                    || trimmed.starts_with("//!")
                    || trimmed.starts_with('*')
                    || trimmed.starts_with("/*")
                    || trimmed.starts_with("*/")
                {
                    continue;
                }
                if line_has_string_eval(line) {
                    hits.push((rel.clone(), idx + 1, line.trim().to_string()));
                }
            }
        }
    }

    fn line_has_string_eval(line: &str) -> bool {
        let trimmed = line.trim_start();
        if trimmed.starts_with("//")
            || trimmed.starts_with("///")
            || trimmed.starts_with("//!")
            || trimmed.starts_with('*')
            || trimmed.starts_with("/*")
            || trimmed.starts_with("*/")
        {
            return false;
        }
        let code = code_keeping_raw_strings(line);
        if code.contains("document::eval")
            || code.contains("js_sys::eval")
            || code.contains("Function::new_with_args")
            || code.contains("Function::new_no_args")
            || code.contains("Function::new(")
        {
            return true;
        }
        bare_eval_call(&code)
    }

    /// `eval(` that is not the tail of an identifier or a path (`::eval`,
    /// `string_eval`). Raw-string bodies stay in `code`, so `r#"eval("#` counts.
    fn bare_eval_call(code: &str) -> bool {
        let mut rest = code;
        while let Some(at) = rest.find("eval(") {
            let bare = at == 0
                || rest[..at]
                    .chars()
                    .next_back()
                    .is_none_or(|ch| !(ch.is_ascii_alphanumeric() || ch == '_' || ch == ':'));
            if bare {
                return true;
            }
            rest = &rest[at + 4..];
        }
        false
    }

    /// Drop normal `"..."` strings. Keep `r"..."` and `r#"..."#` bodies so an
    /// eval hidden in a raw string is still visible to the scan.
    fn code_keeping_raw_strings(line: &str) -> String {
        let mut out = String::with_capacity(line.len());
        let mut chars = line.chars().peekable();
        while let Some(ch) = chars.next() {
            if ch == 'r' && matches!(chars.peek(), Some('"' | '#')) {
                out.push('r');
                let mut hashes = 0usize;
                while chars.peek() == Some(&'#') {
                    hashes += 1;
                    out.push('#');
                    chars.next();
                }
                if chars.peek() == Some(&'"') {
                    out.push('"');
                    chars.next();
                    while let Some(next) = chars.next() {
                        out.push(next);
                        if next != '"' {
                            continue;
                        }
                        let mut seen = 0usize;
                        let mut extra = String::new();
                        let mut matched = true;
                        while seen < hashes {
                            match chars.peek().copied() {
                                Some('#') => {
                                    chars.next();
                                    extra.push('#');
                                    seen += 1;
                                }
                                _ => {
                                    matched = false;
                                    break;
                                }
                            }
                        }
                        out.push_str(&extra);
                        if matched {
                            break;
                        }
                    }
                    continue;
                }
                continue;
            }
            if ch == '"' {
                while let Some(next) = chars.next() {
                    if next == '\\' {
                        chars.next();
                        continue;
                    }
                    if next == '"' {
                        break;
                    }
                }
                out.push(' ');
                continue;
            }
            out.push(ch);
        }
        out
    }
}
