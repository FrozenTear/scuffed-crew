//! Security response headers, including the site Content-Security-Policy.
//!
//! The policy ships as `Content-Security-Policy-Report-Only` until an operator
//! sets `CSP_ENFORCE=1` (no rebuild). Report-only still shows violations in the
//! browser console, which is the rollout check.
//!
//! `frame-ancestors`, `object-src`, `base-uri`, and `form-action` are always
//! present. Clickjacking stays enforced by `X-Frame-Options: DENY` even while
//! the CSP itself is report-only.

use axum::http::{HeaderName, HeaderValue, header};
use base64::Engine;
use sha2::{Digest, Sha256};

/// `Content-Security-Policy` (enforcing).
const CSP_ENFORCE_HEADER: HeaderName = HeaderName::from_static("content-security-policy");
/// `Content-Security-Policy-Report-Only`.
const CSP_REPORT_ONLY_HEADER: HeaderName =
    HeaderName::from_static("content-security-policy-report-only");

/// Inputs for [`SecurityPolicy::from_config`]. Env strings are parsed here so
/// tests can cover the operator knobs without mutating process environment.
#[derive(Debug, Clone)]
pub struct SecurityConfig {
    /// `CSP_ENFORCE=1` (also `true` / `yes` / `on`). Anything else is report-only.
    pub enforce: bool,
    /// `NOSTR_RELAY_URL`. Browser chat opens a WebSocket to this origin when
    /// the team channel URL is `ws://` or `wss://`.
    pub nostr_relay_url: Option<String>,
    /// `CSP_EXTRA_CONNECT_SRC`: extra connect-src origins (space or comma
    /// separated). Use this for a browser-facing relay that is not
    /// `NOSTR_RELAY_URL` (for example a channel still pointing at the previous
    /// relay). Tokens with CSP metacharacters are dropped.
    pub extra_connect_src: Option<String>,
    /// `CSP_IMG_SRC`: extra img-src origins for admin-set external images
    /// (page background, team logo, article cover) that are not same-origin
    /// `/uploads` or the OAuth avatar CDNs.
    pub extra_img_src: Option<String>,
    /// `'sha256-…'` tokens (including quotes) for inline scripts.
    pub script_hashes: Vec<String>,
}

/// Resolved policy. Built once at startup and cloned into the header layer.
#[derive(Debug, Clone)]
pub struct SecurityPolicy {
    enforce: bool,
    connect_extras: Vec<String>,
    img_extras: Vec<String>,
    script_hashes: Vec<String>,
}

impl SecurityPolicy {
    /// Read operator config from the environment and hash inline scripts in
    /// the SPA shell (compiled-in `crates/app/index.html`, plus `dist/index.html`
    /// when the process is serving a built bundle).
    pub fn from_env() -> Self {
        Self::from_config(SecurityConfig {
            enforce: env_flag("CSP_ENFORCE"),
            nostr_relay_url: std::env::var("NOSTR_RELAY_URL").ok(),
            extra_connect_src: std::env::var("CSP_EXTRA_CONNECT_SRC").ok(),
            extra_img_src: std::env::var("CSP_IMG_SRC").ok(),
            script_hashes: bundled_script_hashes(),
        })
    }

    pub fn from_config(config: SecurityConfig) -> Self {
        let mut connect_extras = Vec::new();
        if let Some(relay) = config
            .nostr_relay_url
            .as_deref()
            .and_then(csp_websocket_source)
        {
            connect_extras.push(relay);
        }
        connect_extras.extend(parse_source_list(config.extra_connect_src.as_deref()));
        dedupe(&mut connect_extras);

        let mut img_extras = parse_source_list(config.extra_img_src.as_deref());
        dedupe(&mut img_extras);

        let mut script_hashes = config.script_hashes;
        script_hashes.retain(|h| h.starts_with("'sha256-") && h.ends_with('\''));
        dedupe(&mut script_hashes);

        Self {
            enforce: config.enforce,
            connect_extras,
            img_extras,
            script_hashes,
        }
    }

    /// CSP header value for this response's `Host` (same-host `ws:` / `wss:`).
    pub fn policy(&self, request_host: Option<&str>) -> String {
        let mut connect = vec!["'self'".to_string()];
        if let Some(host) = request_host.and_then(normalize_request_host) {
            // Same host the strategy editor and same-origin chat relay use
            // (`ws(s)://{location.host}/…`). Both schemes: the page picks
            // `wss:` on https and `ws:` on http.
            connect.push(format!("ws://{host}"));
            connect.push(format!("wss://{host}"));
        }
        connect.extend(self.connect_extras.iter().cloned());
        // `document::Link rel=preconnect` to Google Fonts is a connection hint.
        // Chrome checks preconnect against connect-src. The stylesheet itself
        // is style-src; the font files are font-src.
        connect.push("https://fonts.googleapis.com".to_string());
        connect.push("https://fonts.gstatic.com".to_string());
        dedupe(&mut connect);

        let mut img = vec![
            "'self'".to_string(),
            "data:".to_string(),
            "blob:".to_string(),
            // Discord OAuth avatars: crates/auth/src/server/discord.rs
            "https://cdn.discordapp.com".to_string(),
            // Google OAuth `picture` URLs are https://lh3.googleusercontent.com/…
            // (and sibling lhN hosts).
            "https://*.googleusercontent.com".to_string(),
        ];
        img.extend(self.img_extras.iter().cloned());
        dedupe(&mut img);

        let mut script = vec!["'self'".to_string(), "'wasm-unsafe-eval'".to_string()];
        script.extend(self.script_hashes.iter().cloned());

        // style-src keeps 'unsafe-inline' on purpose. The SPA shell
        // (`crates/app/index.html`) has a first-paint `<style>` block, and the
        // Dioxus app injects runtime `<style>` elements (brand theme, layouts,
        // pages). A later frontend change will move those to files; drop
        // 'unsafe-inline' then. Do not add it to script-src.
        //
        // https://fonts.googleapis.com is the Inter / Space Grotesk / JetBrains
        // Mono stylesheet (`crates/app/src/main.rs`). Font files load from
        // fonts.gstatic.com (font-src below).
        let script_src = script.join(" ");
        let img_src = img.join(" ");
        let connect_src = connect.join(" ");
        format!(
            "default-src 'self'; \
             script-src {script_src}; \
             style-src 'self' 'unsafe-inline' https://fonts.googleapis.com; \
             font-src 'self' https://fonts.gstatic.com; \
             img-src {img_src}; \
             connect-src {connect_src}; \
             worker-src 'none'; \
             object-src 'none'; \
             base-uri 'self'; \
             form-action 'self'; \
             frame-ancestors 'none'; \
             frame-src 'none'"
        )
    }
}

/// Apply the production header set. When `policy` is report-only, an existing
/// enforcing `Content-Security-Policy` (the `/uploads` sandbox on non-images)
/// is left in place. When enforcing, a response that already has an enforcing
/// CSP keeps it so the upload sandbox is not replaced by the page policy.
pub async fn apply(
    request: axum::extract::Request,
    next: axum::middleware::Next,
    policy: SecurityPolicy,
) -> axum::response::Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|v| v.to_str().ok())
        .map(str::to_string);

    let mut response = next.run(request).await;
    let headers = response.headers_mut();

    headers.insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    headers.insert(header::X_FRAME_OPTIONS, HeaderValue::from_static("DENY"));
    // Disable the legacy XSS filter — modern browsers ignore it and it can
    // introduce vulnerabilities in older ones.
    headers.insert(
        HeaderName::from_static("x-xss-protection"),
        HeaderValue::from_static("0"),
    );
    headers.insert(
        HeaderName::from_static("referrer-policy"),
        HeaderValue::from_static("strict-origin-when-cross-origin"),
    );
    headers.insert(
        HeaderName::from_static("permissions-policy"),
        HeaderValue::from_static("camera=(), microphone=(), geolocation=()"),
    );

    // Only send HSTS in production — prevents breaking local dev over HTTP.
    if std::env::var("PRODUCTION").is_ok() {
        headers.insert(
            header::STRICT_TRANSPORT_SECURITY,
            HeaderValue::from_static("max-age=63072000; includeSubDomains; preload"),
        );
    }

    let csp = policy.policy(host.as_deref());
    let Ok(csp_value) = HeaderValue::from_str(&csp) else {
        tracing::error!("CSP header value rejected; skipping CSP");
        return response;
    };
    if policy.enforce {
        if !headers.contains_key(CSP_ENFORCE_HEADER) {
            headers.insert(CSP_ENFORCE_HEADER, csp_value);
        }
    } else {
        headers.insert(CSP_REPORT_ONLY_HEADER, csp_value);
    }

    response
}

fn env_flag(name: &str) -> bool {
    matches!(
        std::env::var(name)
            .ok()
            .as_deref()
            .map(str::trim)
            .map(str::to_ascii_lowercase)
            .as_deref(),
        Some("1") | Some("true") | Some("yes") | Some("on")
    )
}

/// Inline-script hashes from the source shell plus the built `dist/index.html`
/// when that file is present (Dioxus may append an external loader; any inline
/// script it leaves in the served file is hashed too).
fn bundled_script_hashes() -> Vec<String> {
    let mut hashes = inline_script_hashes(include_str!("../../app/index.html"));
    if let Ok(dist) = std::fs::read_to_string("dist/index.html") {
        for hash in inline_script_hashes(&dist) {
            if !hashes.contains(&hash) {
                hashes.push(hash);
            }
        }
    }
    hashes
}

/// SHA-256 CSP hashes of inline `<script>` bodies (no `src`).
///
/// Dioxus 0.7's CLI (`inject_loading_scripts`) inserts
/// `<script type="module" async src="/…/*.js">` — an external same-origin
/// module, covered by `'self'`, not an inline script. The wasm glue that
/// compiles the module lives in that JS file and needs `'wasm-unsafe-eval'`.
/// `crates/app/index.html` also has its own inline boot-skeleton script; that
/// one is hashed here instead of allowing `'unsafe-inline'`.
fn inline_script_hashes(html: &str) -> Vec<String> {
    let mut hashes = Vec::new();
    let mut rest = html;
    while let Some(open_at) = find_ignore_ascii_case(rest, "<script") {
        let after_name = open_at + "<script".len();
        let Some(gt) = rest[after_name..].find('>') else {
            break;
        };
        let open_tag = &rest[open_at..after_name + gt + 1];
        let body_start = after_name + gt + 1;
        let Some(close_at) = find_ignore_ascii_case(&rest[body_start..], "</script") else {
            break;
        };
        let body = &rest[body_start..body_start + close_at];
        rest = &rest[body_start + close_at + "</script".len()..];
        if !open_tag_has_src(open_tag) {
            hashes.push(csp_sha256(body));
        }
    }
    hashes
}

fn csp_sha256(body: &str) -> String {
    let digest = Sha256::digest(body.as_bytes());
    format!(
        "'sha256-{}'",
        base64::engine::general_purpose::STANDARD.encode(digest)
    )
}

fn open_tag_has_src(open_tag: &str) -> bool {
    open_tag.split_whitespace().any(|part| {
        let part = part.trim_end_matches('>').to_ascii_lowercase();
        part == "src" || part.starts_with("src=")
    })
}

fn find_ignore_ascii_case(haystack: &str, needle: &str) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle.as_bytes()))
}

fn parse_source_list(raw: Option<&str>) -> Vec<String> {
    raw.into_iter()
        .flat_map(|s| s.split(|c: char| c.is_whitespace() || c == ','))
        .filter(|t| !t.is_empty())
        .filter_map(csp_host_source)
        .collect()
}

/// `wss://relay.example/path` → `wss://relay.example`. Only `ws`/`wss`/`http`/`https`.
fn csp_host_source(raw: &str) -> Option<String> {
    let raw = raw.trim();
    let (scheme, rest) = raw.split_once("://")?;
    let scheme = scheme.to_ascii_lowercase();
    if !matches!(scheme.as_str(), "https" | "http" | "wss" | "ws") {
        return None;
    }
    let hostport = rest.split(['/', '?', '#']).next()?.trim();
    let hostport = normalize_request_host(hostport)?;
    Some(format!("{scheme}://{hostport}"))
}

/// WebSocket origin for `NOSTR_RELAY_URL` (`ws`/`wss` only — the browser client
/// connects only when the channel URL starts with those schemes).
fn csp_websocket_source(raw: &str) -> Option<String> {
    let source = csp_host_source(raw)?;
    if source.starts_with("ws://") || source.starts_with("wss://") {
        Some(source)
    } else {
        None
    }
}

fn normalize_request_host(hostport: &str) -> Option<String> {
    let hostport = hostport.trim();
    if !is_csp_host(hostport) {
        return None;
    }
    Some(hostport.to_string())
}

fn is_csp_host(hostport: &str) -> bool {
    if hostport.is_empty() || hostport.len() > 255 {
        return false;
    }
    let Some((host, port)) = split_host_port(hostport) else {
        return false;
    };
    if let Some(port) = port {
        if port.is_empty() || port.len() > 5 || !port.bytes().all(|b| b.is_ascii_digit()) {
            return false;
        }
        if port.parse::<u16>().is_err() {
            return false;
        }
    }
    valid_host(host)
}

fn split_host_port(hostport: &str) -> Option<(&str, Option<&str>)> {
    if let Some(rest) = hostport.strip_prefix('[') {
        let (addr, after) = rest.split_once(']')?;
        if after.is_empty() {
            return Some((addr, None));
        }
        let port = after.strip_prefix(':')?;
        if port.contains(']') {
            return None;
        }
        Some((addr, Some(port)))
    } else if hostport.matches(':').count() == 1 {
        let (host, port) = hostport.split_once(':')?;
        Some((host, Some(port)))
    } else if hostport.contains(':') {
        None
    } else {
        Some((hostport, None))
    }
}

fn valid_host(host: &str) -> bool {
    if host.is_empty() || host.len() > 253 {
        return false;
    }
    if host.contains(':') {
        return host
            .bytes()
            .all(|b| b.is_ascii_hexdigit() || b == b':' || b == b'.');
    }
    let host = if let Some(rest) = host.strip_prefix("*.") {
        if rest.is_empty() || rest.contains('*') {
            return false;
        }
        rest
    } else if host.contains('*') {
        return false;
    } else {
        host
    };
    host.split('.').all(|label| {
        !label.is_empty()
            && label.len() <= 63
            && !label.starts_with('-')
            && !label.ends_with('-')
            && label
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-')
    })
}

fn dedupe(items: &mut Vec<String>) {
    let mut seen = Vec::new();
    items.retain(|item| {
        if seen.contains(item) {
            false
        } else {
            seen.push(item.clone());
            true
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Request, StatusCode};
    use axum::routing::get;
    use tower::ServiceExt;

    fn directive<'a>(policy: &'a str, name: &str) -> &'a str {
        policy
            .split(';')
            .map(str::trim)
            .find(|d| d.starts_with(name) && d.as_bytes().get(name.len()) == Some(&b' '))
            .unwrap_or_else(|| panic!("missing directive {name} in {policy}"))
    }

    fn test_policy(enforce: bool) -> SecurityPolicy {
        SecurityPolicy::from_config(SecurityConfig {
            enforce,
            nostr_relay_url: Some("wss://relay.example.com/path".into()),
            extra_connect_src: Some("ws://strfry:7777, https://evil.example;default-src *".into()),
            extra_img_src: Some("https://images.example".into()),
            script_hashes: inline_script_hashes(include_str!("../../app/index.html")),
        })
    }

    #[test]
    fn boot_script_is_hashed_external_loader_is_not() {
        let html = include_str!("../../app/index.html");
        let open = html.find("<script>").expect("boot script");
        let body_start = open + "<script>".len();
        let body_end = html[body_start..]
            .find("</script>")
            .map(|n| body_start + n)
            .expect("boot script close");
        let expected = csp_sha256(&html[body_start..body_end]);

        let hashes = inline_script_hashes(html);
        assert_eq!(hashes, vec![expected.clone()]);
        // Known answer for the boot script in crates/app/index.html. A change
        // to that script must update this hash or the enforcing CSP will block it.
        assert_eq!(
            expected,
            "'sha256-9MxM3NkezF2qDsR/EtUZQ8F8U5geo0hZOsFpJn1Q+fM='"
        );

        let with_loader = format!(
            "{html}\n<script type=\"module\" async src=\"/assets/scuffed_app.js\"></script>\n"
        );
        assert_eq!(
            inline_script_hashes(&with_loader),
            hashes,
            "Dioxus external module loader must not be treated as an inline script"
        );
    }

    #[test]
    fn policy_contains_required_directives_and_derived_origins() {
        let policy = test_policy(false).policy(Some("ow.scuffedcrew.no"));
        assert!(policy.starts_with("default-src 'self'"));
        assert_eq!(directive(&policy, "object-src"), "object-src 'none'");
        assert_eq!(directive(&policy, "base-uri"), "base-uri 'self'");
        assert_eq!(directive(&policy, "form-action"), "form-action 'self'");
        assert_eq!(
            directive(&policy, "frame-ancestors"),
            "frame-ancestors 'none'"
        );
        assert_eq!(directive(&policy, "worker-src"), "worker-src 'none'");

        let script = directive(&policy, "script-src");
        assert!(script.contains("'self'"), "{script}");
        assert!(script.contains("'wasm-unsafe-eval'"), "{script}");
        assert!(
            !script.contains("unsafe-inline"),
            "script-src must hash the boot script, not allow unsafe-inline: {script}"
        );
        let boot = &inline_script_hashes(include_str!("../../app/index.html"))[0];
        assert!(script.contains(boot), "{script}");

        let style = directive(&policy, "style-src");
        assert!(style.contains("'self'"), "{style}");
        assert!(style.contains("'unsafe-inline'"), "{style}");
        assert!(style.contains("https://fonts.googleapis.com"), "{style}");

        let font = directive(&policy, "font-src");
        assert!(font.contains("'self'"), "{font}");
        assert!(font.contains("https://fonts.gstatic.com"), "{font}");

        let img = directive(&policy, "img-src");
        assert!(img.contains("'self'"), "{img}");
        assert!(img.contains("data:"), "{img}");
        assert!(img.contains("blob:"), "{img}");
        assert!(img.contains("https://cdn.discordapp.com"), "{img}");
        assert!(img.contains("https://*.googleusercontent.com"), "{img}");
        assert!(img.contains("https://images.example"), "{img}");

        let connect = directive(&policy, "connect-src");
        assert!(connect.contains("'self'"), "{connect}");
        assert!(connect.contains("ws://ow.scuffedcrew.no"), "{connect}");
        assert!(connect.contains("wss://ow.scuffedcrew.no"), "{connect}");
        assert!(
            connect.contains("wss://relay.example.com"),
            "NOSTR_RELAY_URL origin: {connect}"
        );
        assert!(connect.contains("ws://strfry:7777"), "{connect}");
        assert!(
            !connect.contains("evil"),
            "rejected extra source leaked: {connect}"
        );
        assert!(!policy.contains("default-src *"), "{policy}");
    }

    #[test]
    fn hostile_host_is_omitted() {
        let policy = test_policy(false).policy(Some("evil.example; script-src 'unsafe-inline'"));
        assert!(!policy.contains("evil.example"), "{policy}");
        assert!(!directive(&policy, "script-src").contains("unsafe-inline"));
    }

    #[test]
    fn http_relay_url_is_not_a_websocket_source() {
        let policy = SecurityPolicy::from_config(SecurityConfig {
            enforce: false,
            nostr_relay_url: Some("https://relay.example".into()),
            extra_connect_src: None,
            extra_img_src: None,
            script_hashes: Vec::new(),
        });
        let header = policy.policy(None);
        assert!(
            !directive(&header, "connect-src").contains("relay.example"),
            "{header}"
        );
    }

    #[tokio::test]
    async fn report_only_csp_and_nosniff_on_pages_and_uploads() {
        let policy = test_policy(false);
        let uploads = Router::new()
            .route(
                "/file.html",
                get(|| async {
                    (
                        [(header::CONTENT_TYPE, "text/html; charset=utf-8")],
                        "<html><script>alert(1)</script></html>",
                    )
                }),
            )
            .route(
                "/file.png",
                get(|| async { ([(header::CONTENT_TYPE, "image/png")], "png") }),
            )
            .layer(axum::middleware::from_fn(
                scuffed_site_server::uploads::upload_response_headers,
            ));
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .nest("/uploads", uploads)
            .layer(axum::middleware::from_fn(move |req, next| {
                let policy = policy.clone();
                async move { apply(req, next, policy).await }
            }));

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .header(header::HOST, "localhost:3030")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(health.status(), StatusCode::OK);
        assert_eq!(
            health
                .headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("nosniff"),
            "global security layer sets nosniff on every response, including ones ServeDir would return"
        );
        let report = health
            .headers()
            .get(CSP_REPORT_ONLY_HEADER)
            .and_then(|v| v.to_str().ok())
            .expect("report-only CSP");
        assert!(report.contains("frame-ancestors 'none'"), "{report}");
        assert!(report.contains("object-src 'none'"), "{report}");
        assert!(report.contains("base-uri 'self'"), "{report}");
        assert!(report.contains("form-action 'self'"), "{report}");
        assert!(report.contains("ws://localhost:3030"), "{report}");
        assert!(health.headers().get(CSP_ENFORCE_HEADER).is_none());
        assert!(health.headers().get(header::CONTENT_DISPOSITION).is_none());

        let png = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/uploads/file.png")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            png.headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            png.headers()
                .get(header::CONTENT_DISPOSITION)
                .and_then(|v| v.to_str().ok()),
            Some("inline")
        );
        assert!(png.headers().get(CSP_REPORT_ONLY_HEADER).is_some());
        assert!(
            png.headers().get(CSP_ENFORCE_HEADER).is_none(),
            "raster images must not get the upload sandbox CSP"
        );

        let html = app
            .oneshot(
                Request::builder()
                    .uri("/uploads/file.html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            html.headers()
                .get(header::X_CONTENT_TYPE_OPTIONS)
                .and_then(|v| v.to_str().ok()),
            Some("nosniff")
        );
        assert_eq!(
            html.headers()
                .get(header::CONTENT_DISPOSITION)
                .and_then(|v| v.to_str().ok()),
            Some("attachment")
        );
        assert_eq!(
            html.headers()
                .get(CSP_ENFORCE_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("default-src 'none'; sandbox")
        );
        assert!(
            html.headers().get(CSP_REPORT_ONLY_HEADER).is_some(),
            "page CSP stays report-only alongside the upload sandbox"
        );
    }

    #[tokio::test]
    async fn enforce_env_switches_header_name_without_clobbering_upload_sandbox() {
        let policy = test_policy(true);
        let uploads = Router::new()
            .route(
                "/file.html",
                get(|| async { ([(header::CONTENT_TYPE, "text/html")], "x") }),
            )
            .layer(axum::middleware::from_fn(
                scuffed_site_server::uploads::upload_response_headers,
            ));
        let app = Router::new()
            .route("/api/health", get(|| async { "ok" }))
            .nest("/uploads", uploads)
            .layer(axum::middleware::from_fn(move |req, next| {
                let policy = policy.clone();
                async move { apply(req, next, policy).await }
            }));

        let health = app
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/health")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let enforced = health
            .headers()
            .get(CSP_ENFORCE_HEADER)
            .and_then(|v| v.to_str().ok())
            .expect("enforcing CSP");
        assert!(enforced.contains("frame-ancestors 'none'"), "{enforced}");
        assert!(health.headers().get(CSP_REPORT_ONLY_HEADER).is_none());

        let html = app
            .oneshot(
                Request::builder()
                    .uri("/uploads/file.html")
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        assert_eq!(
            html.headers()
                .get(CSP_ENFORCE_HEADER)
                .and_then(|v| v.to_str().ok()),
            Some("default-src 'none'; sandbox"),
            "enforcing page CSP must not replace the upload sandbox"
        );
        assert!(html.headers().get(CSP_REPORT_ONLY_HEADER).is_none());
    }
}
