//! Public crawler files and cache policy for the Dioxus SPA shell.
//!
//! `/robots.txt` and `/sitemap.xml` are explicit routes so they win over the
//! `dist/` catch-all (which would otherwise return `index.html` as 200 HTML).
//! Cache headers are applied only to that catch-all, never to `/api/*` or
//! `/uploads`.
//!
//! # `sc-settings` embed
//!
//! Every HTML shell (`/`, `/index.html`, and any client route that serves
//! `index.html`) includes the anonymous `GET /api/settings` JSON immediately
//! before `</head>`:
//!
//! ```html
//! <script id="sc-settings" type="application/json">{...}</script>
//! ```
//!
//! The object is [`crate::routes::settings::load_anonymous_settings`] run
//! through `serde_json` — the same mapping and serializer as that route, so
//! the embed cannot grow a private field the public GET does not return.
//! `<`, `>`, `&`, U+2028, and U+2029 are written as `\u003c`, `\u003e`,
//! `\u0026`, `\u2028`, and `\u2029`. The escaped JSON and the script block are
//! cached together. The shell stays `Cache-Control: no-cache`.
//!
//! A cache miss waits at most [`EMBED_SETTINGS_TIMEOUT`]. On timeout or error
//! the last blob is served if one exists (stale-on-error); otherwise the tag
//! is omitted and the page is still served. The same successful read rewrites
//! `<title>` and the `og:title`, `og:site_name`, description, and
//! `og:description` meta contents from `org_name` and `site_description`.
//! The tag is a data block (`type="application/json"`), not an executed script.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::task::{Context, Poll};
use std::time::{Duration, Instant};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use scuffed_db::{ForumBoard, ForumBoardNode, ForumCategoryNode, MatchType, TournamentStatus};
use tower::Service;
use tower_http::services::ServeDir;

use crate::state::{AppState, CachedPublicSettings};

const SETTINGS_SCRIPT_OPEN: &str = "<script id=\"sc-settings\" type=\"application/json\">";

/// Hard cap so a large forum or member table cannot produce an unbounded document.
const SITEMAP_URL_CAP: usize = 2_000;
const ARTICLE_LIMIT: u32 = 500;
const MEMBER_LIMIT: u32 = 500;
const TOURNAMENT_LIMIT: u32 = 200;
const MATCH_RECENT_LIMIT: u32 = 200;
const MATCH_UPCOMING_LIMIT: u32 = 50;
const WIKI_LIMIT: u32 = 200;
const FORUM_BOARD_LIMIT: usize = 40;
const FORUM_THREADS_PER_BOARD: u32 = 25;

/// Indexable pages with no id segment. Detail URLs are added from the database.
///
/// Patch notes have no per-item site path: the public page is `/patch-notes`.
/// A patch row's `url` field is an external source link, not a page on this
/// host. `/strategy/patch-notes` is the same list and is never emitted.
/// Strategy comps have no public detail route (the editor is member-only).
/// `/strategy`, `/strategy/heroes`, and `/strategy/meta` are added only when
/// `strategies_enabled` is on — see [`push_strategy_routes`].
const PUBLIC_STATIC_PATHS: &[&str] = &[
    "/",
    "/members",
    "/news",
    "/apply",
    "/tournaments",
    "/blog",
    "/wiki",
    "/forum",
    "/leaderboards",
    "/events",
    "/community",
    "/feed",
    "/patch-notes",
];

/// Strategy browser routes gated by `SiteSettings.strategies_enabled`.
const STRATEGY_SITEMAP_PATHS: &[&str] = &["/strategy", "/strategy/heroes", "/strategy/meta"];

/// Path prefixes crawlers must not index.
///
/// Checked against `crates/app/src/routes.rs`. Every admin screen is under
/// `/admin` (`/admin/games`, `/admin/teams`, `/admin/settings`, …). Public
/// pages such as `/teams/:id`, `/members`, and `/blog` are not under those
/// prefixes. `/polls`, `/scrims`, and `/stats` require an org session.
const PRIVATE_PREFIXES: &[&str] = &[
    "/admin",
    "/api/",
    "/login",
    "/setup",
    "/identity",
    "/profile/",
    "/dm",
    "/chat",
    "/polls",
    "/scrims",
    "/stats",
    "/strategy/my",
    "/strategy/editor",
];

const HASHED_ASSET_CACHE: &str = "public, max-age=31536000, immutable";
const STATIC_ASSET_CACHE: &str = "public, max-age=86400";
const SHELL_CACHE: &str = "no-cache";

/// `REDIRECT_BASE_URL` (stored on [`crate::state::OAuthConfig`]) with trailing
/// slashes removed, so `Sitemap:` and `<loc>` values stay absolute and single-slash.
pub(crate) fn public_base_url(redirect_base_url: &str) -> String {
    redirect_base_url.trim().trim_end_matches('/').to_string()
}

pub(crate) fn render_robots(redirect_base_url: &str) -> String {
    let base = public_base_url(redirect_base_url);
    let mut out = String::from(
        "User-agent: *\n\
         Allow: /\n\
         \n\
         # Admin UI is entirely under /admin. Public pages (/members, /teams, /blog, …) stay allowed.\n",
    );
    for prefix in PRIVATE_PREFIXES {
        out.push_str("Disallow: ");
        out.push_str(prefix);
        out.push('\n');
    }
    out.push_str("\nSitemap: ");
    out.push_str(&base);
    out.push_str("/sitemap.xml\n");
    out
}

/// Cache policy for one SPA-fallback response.
///
/// HTML (the shell and every client route that falls through to `index.html`)
/// is `no-cache` so a new deploy is picked up on the next load. Dioxus 0.7
/// content-hashed files (`{name}-dxh{hash}.js`, and the same marker on `.wasm`
/// / `.css`) are immutable. Unhashed `.js`, `.wasm`, and `.css` also
/// revalidate — a deploy can replace them without a new filename. Other
/// unhashed files (favicon, images, fonts) get a one-day cache and are never
/// `immutable`.
pub(crate) fn cache_control_value(path: &str, content_type: Option<&str>) -> &'static str {
    if response_is_spa_shell(path, content_type) {
        SHELL_CACHE
    } else if is_dioxus_hashed_asset(path) {
        HASHED_ASSET_CACHE
    } else if is_unhashed_code_asset(path) {
        SHELL_CACHE
    } else {
        STATIC_ASSET_CACHE
    }
}

/// Unhashed script, module, and stylesheet files. Hashed `dxh` names are
/// classified earlier and stay immutable.
fn is_unhashed_code_asset(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    let file = path.rsplit('/').next().unwrap_or(path);
    let Some((_, ext)) = file.rsplit_once('.') else {
        return false;
    };
    matches!(ext.to_ascii_lowercase().as_str(), "js" | "wasm" | "css")
}

pub(crate) fn is_dioxus_hashed_asset(path: &str) -> bool {
    let path = path.split('?').next().unwrap_or(path);
    let file = path.rsplit('/').next().unwrap_or(path);
    let stem = file.rsplit_once('.').map(|(stem, _)| stem).unwrap_or(file);
    dioxus_hash_stem(stem)
}

/// Dioxus fingerprints bundled files as `{stem}-dxh{hash}.{ext}`
/// (`ferrous_wave-dxhx13xj2j.png`). The hash is alphanumeric, not only hex.
fn dioxus_hash_stem(stem: &str) -> bool {
    let hash = if let Some(rest) = stem.strip_prefix("dxh") {
        rest
    } else if let Some(idx) = stem.rfind("-dxh") {
        &stem[idx + 4..]
    } else {
        return false;
    };
    hash.len() >= 4 && hash.chars().all(|c| c.is_ascii_alphanumeric())
}

fn response_is_spa_shell(path: &str, content_type: Option<&str>) -> bool {
    if content_type.is_some_and(|ct| ct.to_ascii_lowercase().contains("text/html")) {
        return true;
    }
    let path = path.split('?').next().unwrap_or(path);
    if path == "/" || path.ends_with('/') || path.ends_with(".html") {
        return true;
    }
    !looks_like_static_asset(path)
}

fn looks_like_static_asset(path: &str) -> bool {
    let file = path.rsplit('/').next().unwrap_or(path);
    matches!(
        file.rsplit_once('.'),
        Some((_, ext)) if !ext.is_empty() && !ext.eq_ignore_ascii_case("html")
    )
}

/// How long a shell waits for the settings read before serving stale or
/// omitting the block. Named so a slow database cannot hold the page open
/// for the query timeout.
const EMBED_SETTINGS_TIMEOUT: Duration = Duration::from_millis(300);

/// Missing-file extensions that are real static assets. A client-route slug
/// that merely contains a dot (for example `/wiki/foo.bar`) is not in this
/// list and still receives the shell.
const STATIC_EXTENSIONS: &[&str] = &[
    "js", "mjs", "css", "wasm", "map", "svg", "png", "jpg", "jpeg", "webp", "gif", "avif", "ico",
    "woff", "woff2", "ttf", "json",
];

/// `dist/` with an in-memory `index.html` shell, plus cache headers.
///
/// The template is read when the router is built. `</head>` is located then.
/// A missing file stays a 404. Any other read error is retried on the next
/// shell request and, until that succeeds, the request falls through to the
/// raw file. A new deploy replaces the process.
///
/// A missing file under `/assets/`, or a missing path whose extension is in
/// [`STATIC_EXTENSIONS`], is a plain 404 with `Cache-Control: no-store`.
/// Extension-less client routes, and dotted slugs that are not those
/// extensions, still get the shell.
///
/// The `dist/` root is canonicalized once here. Per-request lookups do not
/// canonicalize ordinary files.
///
/// A hand-rolled service (rather than `middleware::from_fn`) so the future
/// stays `Send`. Axum's function middleware around `ServeDir` does not.
pub(crate) fn spa_service(dist_dir: &Path, state: AppState) -> SpaService {
    let dist_root = dist_dir
        .canonicalize()
        .unwrap_or_else(|_| dist_dir.to_path_buf());
    let index = Arc::new(Mutex::new(IndexSlot::load(dist_dir)));
    let files = ServeDir::new(dist_dir);
    SpaService {
        dist_root,
        index,
        files,
        state,
    }
}

#[derive(Clone)]
struct ShellTemplate {
    html: Arc<str>,
    /// Byte index of `</head>` in `html`, located when the template is loaded.
    head_close: Option<usize>,
}

enum IndexState {
    Ready(ShellTemplate),
    /// `index.html` was not found. Client routes 404.
    Missing,
    /// The read failed for another reason. The next shell request tries again.
    Unreadable,
}

struct IndexSlot {
    state: IndexState,
    last_warn: Option<Instant>,
}

enum TemplateRead {
    Ready(ShellTemplate),
    Missing,
    Failed(std::io::Error),
}

enum ShellMiss {
    Missing,
    Unreadable,
}

impl IndexSlot {
    fn load(dist_dir: &Path) -> Self {
        match read_index_template(dist_dir) {
            TemplateRead::Ready(template) => Self {
                state: IndexState::Ready(template),
                last_warn: None,
            },
            TemplateRead::Missing => Self {
                state: IndexState::Missing,
                last_warn: None,
            },
            TemplateRead::Failed(err) => {
                tracing::warn!(
                    error = %err,
                    "could not preload dist/index.html; the next shell request will retry"
                );
                Self {
                    state: IndexState::Unreadable,
                    last_warn: Some(Instant::now()),
                }
            }
        }
    }

    fn can_serve_shell(&self) -> bool {
        !matches!(self.state, IndexState::Missing)
    }
}

fn read_index_template(dist_dir: &Path) -> TemplateRead {
    let path = dist_dir.join("index.html");
    match std::fs::read_to_string(&path) {
        Ok(html) => TemplateRead::Ready(prepare_shell_template(html)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => TemplateRead::Missing,
        Err(err) => TemplateRead::Failed(err),
    }
}

/// Locate `</head>` once. A template without it is still served; the embed
/// and the title rewrite are skipped, and the warning is this load only.
fn prepare_shell_template(html: String) -> ShellTemplate {
    let head_close = find_head_close(&html);
    if head_close.is_none() {
        tracing::warn!("SPA shell has no </head>; omitting sc-settings embed");
    }
    ShellTemplate {
        html: Arc::from(html),
        head_close,
    }
}

/// Escape a JSON text so it can sit inside `<script type="application/json">`.
///
/// `serde_json` leaves `<`, `>`, `&`, U+2028, and U+2029 raw. Any of those can
/// close the script element or break an HTML parser. The escapes are valid
/// JSON, so `JSON.parse` of the element text still yields the original value.
pub(crate) fn escape_json_for_html(json: &str) -> String {
    let mut out = String::with_capacity(json.len());
    for ch in json.chars() {
        match ch {
            '<' => out.push_str("\\u003c"),
            '>' => out.push_str("\\u003e"),
            '&' => out.push_str("\\u0026"),
            '\u{2028}' => out.push_str("\\u2028"),
            '\u{2029}' => out.push_str("\\u2029"),
            _ => out.push(ch),
        }
    }
    out
}

fn cached_settings(json: &str, org_name: &str, site_description: &str) -> CachedPublicSettings {
    let escaped_json = escape_json_for_html(json);
    let mut script_block =
        String::with_capacity(SETTINGS_SCRIPT_OPEN.len() + escaped_json.len() + "</script>".len());
    script_block.push_str(SETTINGS_SCRIPT_OPEN);
    script_block.push_str(&escaped_json);
    script_block.push_str("</script>");
    CachedPublicSettings {
        escaped_json,
        script_block,
        org_name: org_name.to_string(),
        site_description: site_description.to_string(),
    }
}

/// Shell HTML for one response.
///
/// Without settings, the template is returned untouched (title and meta stay
/// as built). With settings, `<title>` and the matching meta contents in the
/// head are replaced, then the already-rendered script block is inserted at
/// the `</head>` index captured when the template was loaded.
fn render_shell(template: &ShellTemplate, embed: Option<&CachedPublicSettings>) -> String {
    let Some(embed) = embed else {
        return template.html.to_string();
    };
    let Some(idx) = template.head_close else {
        return template.html.to_string();
    };
    let head = rewrite_document_head(
        &template.html[..idx],
        &embed.org_name,
        &embed.site_description,
    );
    // Hit path: serve the cached block. `escaped_json` is the same payload
    // already inside it, kept so a hit does not escape again.
    let block_len = embed.script_block.len().max(embed.escaped_json.len());
    let mut out = String::with_capacity(head.len() + block_len + template.html.len() - idx);
    out.push_str(&head);
    out.push_str(&embed.script_block);
    out.push_str(&template.html[idx..]);
    out
}

fn escape_html_text(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

/// Replace `<title>` text and the meta contents the anonymous settings can
/// fill. Attribute order and incidental whitespace do not matter. Tags that
/// are absent stay absent.
fn rewrite_document_head(head: &str, org_name: &str, site_description: &str) -> String {
    let with_title = replace_title_text(head, org_name);
    replace_meta_contents(&with_title, org_name, site_description)
}

fn replace_title_text(head: &str, org_name: &str) -> String {
    let lower = head.to_ascii_lowercase();
    let escaped = escape_html_text(org_name);
    let mut out = String::with_capacity(head.len() + escaped.len());
    let mut i = 0;
    while let Some(rel) = lower[i..].find("<title") {
        let start = i + rel;
        let after = start + "<title".len();
        if !tag_name_ends(head, after) {
            out.push_str(&head[i..after]);
            i = after;
            continue;
        }
        let Some(gt_rel) = head[after..].find('>') else {
            break;
        };
        let content_start = after + gt_rel + 1;
        let Some(close_rel) = lower[content_start..].find("</title") else {
            break;
        };
        let content_end = content_start + close_rel;
        out.push_str(&head[i..content_start]);
        out.push_str(&escaped);
        i = content_end;
    }
    out.push_str(&head[i..]);
    out
}

fn replace_meta_contents(head: &str, org_name: &str, site_description: &str) -> String {
    let lower = head.to_ascii_lowercase();
    let mut out = String::with_capacity(head.len());
    let mut i = 0;
    while let Some(rel) = lower[i..].find("<meta") {
        let start = i + rel;
        let after = start + "<meta".len();
        if !tag_name_ends(head, after) {
            out.push_str(&head[i..after]);
            i = after;
            continue;
        }
        let Some(gt_rel) = head[after..].find('>') else {
            break;
        };
        let end = after + gt_rel + 1;
        out.push_str(&head[i..start]);
        out.push_str(&rewrite_meta_tag(
            &head[start..end],
            org_name,
            site_description,
        ));
        i = end;
    }
    out.push_str(&head[i..]);
    out
}

fn tag_name_ends(html: &str, after_name: usize) -> bool {
    match html.as_bytes().get(after_name).copied() {
        None => true,
        Some(b) => b.is_ascii_whitespace() || b == b'>' || b == b'/',
    }
}

struct ScannedAttr {
    name: String,
    value: String,
    /// Byte range of the attribute value inside `tag`, excluding quotes.
    value_range: std::ops::Range<usize>,
}

fn rewrite_meta_tag(tag: &str, org_name: &str, site_description: &str) -> String {
    let attrs = scan_attrs(tag);
    let attr_value = |name: &str| {
        attrs
            .iter()
            .find(|attr| attr.name.eq_ignore_ascii_case(name))
            .map(|attr| attr.value.as_str())
    };
    let replacement = if let Some(property) = attr_value("property") {
        match property.to_ascii_lowercase().as_str() {
            "og:title" | "og:site_name" => Some(org_name),
            "og:description" => Some(site_description),
            _ => None,
        }
    } else if attr_value("name").is_some_and(|name| name.eq_ignore_ascii_case("description")) {
        Some(site_description)
    } else {
        None
    };
    let Some(new_value) = replacement else {
        return tag.to_string();
    };
    let Some(content) = attrs
        .iter()
        .find(|attr| attr.name.eq_ignore_ascii_case("content"))
    else {
        return tag.to_string();
    };
    let escaped = escape_html_text(new_value);
    let mut out = String::with_capacity(tag.len() + escaped.len());
    out.push_str(&tag[..content.value_range.start]);
    out.push_str(&escaped);
    out.push_str(&tag[content.value_range.end..]);
    out
}

fn scan_attrs(tag: &str) -> Vec<ScannedAttr> {
    let bytes = tag.as_bytes();
    let mut i = 0;
    while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
        i += 1;
    }
    let mut attrs = Vec::new();
    while i < bytes.len() {
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] == b'>' || bytes[i] == b'/' {
            break;
        }
        let name_start = i;
        while i < bytes.len()
            && bytes[i] != b'='
            && bytes[i] != b'>'
            && bytes[i] != b'/'
            && !bytes[i].is_ascii_whitespace()
        {
            i += 1;
        }
        let name = tag[name_start..i].to_string();
        if name.is_empty() {
            i += 1;
            continue;
        }
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() || bytes[i] != b'=' {
            continue;
        }
        i += 1;
        while i < bytes.len() && bytes[i].is_ascii_whitespace() {
            i += 1;
        }
        if i >= bytes.len() {
            break;
        }
        if bytes[i] == b'"' || bytes[i] == b'\'' {
            let quote = bytes[i];
            let start = i + 1;
            i = start;
            while i < bytes.len() && bytes[i] != quote {
                i += 1;
            }
            let end = i;
            if i < bytes.len() {
                i += 1;
            }
            attrs.push(ScannedAttr {
                name,
                value: tag[start..end].to_string(),
                value_range: start..end,
            });
        } else {
            let start = i;
            while i < bytes.len() && !bytes[i].is_ascii_whitespace() && bytes[i] != b'>' {
                i += 1;
            }
            attrs.push(ScannedAttr {
                name,
                value: tag[start..i].to_string(),
                value_range: start..i,
            });
        }
    }
    attrs
}

fn find_head_close(html: &str) -> Option<usize> {
    let needle = b"</head>";
    html.as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

async fn load_embed(state: &AppState) -> Option<Arc<CachedPublicSettings>> {
    if let Some(hit) = state.public_settings.fresh() {
        return Some(hit);
    }
    // One reader. Waiters observe that reader's result instead of each
    // hitting the database.
    let _flight = state.public_settings.refresh_lock().await;
    if let Some(hit) = state.public_settings.fresh() {
        return Some(hit);
    }
    if state.public_settings.refresh_suppressed() {
        return state.public_settings.stale();
    }
    #[cfg(test)]
    if state.public_settings.fail_loads() {
        state
            .public_settings
            .note_embed_failure("SPA shell settings embed skipped");
        return state.public_settings.stale();
    }
    let generation = state.public_settings.generation();
    let read = read_public_settings(&state.db);
    match tokio::time::timeout(EMBED_SETTINGS_TIMEOUT, read).await {
        Ok(Ok(payload)) => {
            let cached = Arc::new(payload);
            // `store` drops the blob when an invalidation landed during the
            // read. Prefer whatever is fresh for the newer generation.
            state
                .public_settings
                .store(generation, CachedPublicSettings::clone(&cached));
            Some(
                state
                    .public_settings
                    .fresh()
                    .unwrap_or_else(|| Arc::clone(&cached)),
            )
        }
        Ok(Err(err)) => {
            state.public_settings.note_refresh_failure(generation);
            state
                .public_settings
                .note_embed_failure(&format!("SPA shell settings embed skipped: {err}"));
            state.public_settings.stale()
        }
        Err(_elapsed) => {
            state.public_settings.note_refresh_failure(generation);
            state
                .public_settings
                .note_embed_failure("SPA shell settings embed timed out");
            state.public_settings.stale()
        }
    }
}

async fn read_public_settings(db: &scuffed_db::Database) -> Result<CachedPublicSettings, String> {
    let settings = crate::routes::settings::anonymous_settings_json(db).await?;
    Ok(cached_settings(
        &settings.json,
        &settings.org_name,
        &settings.site_description,
    ))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SpaRoute {
    /// Serve the preloaded `index.html` with the settings embed.
    Shell,
    /// An existing file under `dist/` (not the root index). `ServeDir` serves it.
    File,
    /// Do not serve the shell.
    NotFound,
}

enum DistLookup {
    Index,
    File,
    Missing,
    Rejected,
}

fn classify_spa_route(dist_root: &Path, url_path: &str, have_index: bool) -> SpaRoute {
    match dist_lookup(dist_root, url_path) {
        DistLookup::Index => {
            if have_index {
                SpaRoute::Shell
            } else {
                SpaRoute::NotFound
            }
        }
        DistLookup::File => SpaRoute::File,
        DistLookup::Missing => {
            // Client routes still get the shell. A missing stylesheet, script,
            // or anything under /assets/ must not.
            if have_index && !is_static_miss_path(url_path) {
                SpaRoute::Shell
            } else {
                SpaRoute::NotFound
            }
        }
        DistLookup::Rejected => SpaRoute::NotFound,
    }
}

/// Missing files under `/assets/`, or a missing path with a real static
/// extension, must 404 instead of falling through to `index.html`.
///
/// Wiki and article slugs that contain a dot stay on the shell unless the
/// final extension is in [`STATIC_EXTENSIONS`].
fn is_static_miss_path(url_path: &str) -> bool {
    let path = url_path.split('?').next().unwrap_or(url_path);
    let decoded = urlencoding::decode(path).unwrap_or(std::borrow::Cow::Borrowed(path));
    let path = decoded.as_ref();
    if path == "/assets" || path.starts_with("/assets/") {
        return true;
    }
    let file = path.rsplit('/').next().unwrap_or(path);
    let Some((_, ext)) = file.rsplit_once('.') else {
        return false;
    };
    if ext.is_empty() {
        return false;
    }
    let ext = ext.to_ascii_lowercase();
    STATIC_EXTENSIONS.iter().any(|known| *known == ext)
}

/// Resolve `url_path` under the already-canonical `dist_root`.
///
/// `dist_root` was canonicalized once at startup. Ordinary files are
/// `symlink_metadata` only. `canonicalize` runs only for a path component
/// that is itself a symlink, so a link cannot point outside `dist/`.
fn dist_lookup(dist_root: &Path, url_path: &str) -> DistLookup {
    let path = url_path.split('?').next().unwrap_or(url_path);
    let decoded = match urlencoding::decode(path) {
        Ok(value) => value.into_owned(),
        Err(_) => return DistLookup::Rejected,
    };
    let rel = decoded.trim_start_matches('/');
    if rel.contains('\0') || rel.split('/').any(|seg| seg == ".." || seg == ".") {
        return DistLookup::Rejected;
    }
    if rel.is_empty() || rel == "index.html" {
        return DistLookup::Index;
    }
    let mut cursor = dist_root.to_path_buf();
    let mut last_is_file = false;
    for component in Path::new(rel).components() {
        let std::path::Component::Normal(segment) = component else {
            return DistLookup::Rejected;
        };
        cursor.push(segment);
        if !cursor.starts_with(dist_root) {
            return DistLookup::Rejected;
        }
        match cursor.symlink_metadata() {
            Ok(meta) if meta.file_type().is_symlink() => {
                let Ok(canon) = cursor.canonicalize() else {
                    return DistLookup::Missing;
                };
                if !canon.starts_with(dist_root) {
                    return DistLookup::Rejected;
                }
                last_is_file = canon.is_file();
                cursor = canon;
            }
            Ok(meta) => last_is_file = meta.is_file(),
            Err(_) => return DistLookup::Missing,
        }
    }
    if last_is_file {
        DistLookup::File
    } else {
        // A directory (for example `/assets`) is not a file. The shell covers
        // it until a later check turns missing static paths into 404s.
        DistLookup::Missing
    }
}

fn shell_response(html: String, head_only: bool) -> Response<Body> {
    raw_shell_response(html.into_bytes(), head_only)
}

fn raw_shell_response(bytes: Vec<u8>, head_only: bool) -> Response<Body> {
    let len = bytes.len();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, HeaderValue::from_static(SHELL_CACHE))
        .header(header::CONTENT_LENGTH, len.to_string())
        .body(if head_only {
            Body::empty()
        } else {
            Body::from(bytes)
        })
        .expect("shell response headers are valid")
}

fn plain_not_found() -> Response<Body> {
    Response::builder()
        .status(StatusCode::NOT_FOUND)
        .header(header::CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(header::CACHE_CONTROL, HeaderValue::from_static("no-store"))
        .body(Body::from("not found"))
        .expect("not-found response headers are valid")
}

#[derive(Clone)]
pub(crate) struct SpaService {
    /// Canonical `dist/` directory, computed once when the router is built.
    dist_root: PathBuf,
    index: Arc<Mutex<IndexSlot>>,
    files: ServeDir,
    state: AppState,
}

impl<ReqBody> Service<Request<ReqBody>> for SpaService
where
    ReqBody: Send + 'static,
{
    type Response = Response<Body>;
    type Error = std::convert::Infallible;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        Service::<Request<ReqBody>>::poll_ready(&mut self.files, cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let method = req.method().clone();
        let shell_method = method == Method::GET || method == Method::HEAD;
        let route = if shell_method {
            let have_index = self
                .index
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .can_serve_shell();
            classify_spa_route(&self.dist_root, &path, have_index)
        } else {
            SpaRoute::File
        };
        let state = self.state.clone();
        let index = Arc::clone(&self.index);
        let dist_root = self.dist_root.clone();
        match route {
            SpaRoute::Shell => Box::pin(async move {
                match shell_html(&index, &dist_root, &state).await {
                    Ok(html) => Ok(shell_response(html, method == Method::HEAD)),
                    Err(ShellMiss::Missing) => Ok(plain_not_found()),
                    Err(ShellMiss::Unreadable) => {
                        // Preload failed. Serve the raw file for this request
                        // (no embed) and leave the slot retryable. An
                        // unreadable path, including a directory, is a plain
                        // 404 rather than a stuck shell.
                        let dist = dist_root.clone();
                        let head_only = method == Method::HEAD;
                        let raw = tokio::task::spawn_blocking(move || {
                            std::fs::read(dist.join("index.html"))
                        })
                        .await;
                        match raw {
                            Ok(Ok(bytes)) => Ok(raw_shell_response(bytes, head_only)),
                            _ => Ok(plain_not_found()),
                        }
                    }
                }
            }),
            SpaRoute::NotFound => Box::pin(async { Ok(plain_not_found()) }),
            SpaRoute::File => {
                let files = self.files.clone();
                Box::pin(async move { Ok(serve_dist(files, req, path).await) })
            }
        }
    }
}

async fn shell_html(
    index: &Mutex<IndexSlot>,
    dist_root: &Path,
    state: &AppState,
) -> Result<String, ShellMiss> {
    let ready = {
        let guard = index.lock().unwrap_or_else(|err| err.into_inner());
        match &guard.state {
            IndexState::Ready(template) => Some(template.clone()),
            IndexState::Missing => None,
            IndexState::Unreadable => None,
        }
    };
    if let Some(template) = ready {
        let embed = load_embed(state).await;
        return Ok(render_shell(&template, embed.as_deref()));
    }
    let missing = {
        let guard = index.lock().unwrap_or_else(|err| err.into_inner());
        matches!(guard.state, IndexState::Missing)
    };
    if missing {
        return Err(ShellMiss::Missing);
    }
    let dist_root = dist_root.to_path_buf();
    let read = tokio::task::spawn_blocking(move || read_index_template(&dist_root))
        .await
        .unwrap_or_else(|_| {
            TemplateRead::Failed(std::io::Error::other("index.html read task failed"))
        });
    let loaded = {
        let mut guard = index.lock().unwrap_or_else(|err| err.into_inner());
        match read {
            TemplateRead::Ready(template) => {
                guard.state = IndexState::Ready(template.clone());
                Ok(template)
            }
            TemplateRead::Missing => {
                guard.state = IndexState::Missing;
                Err(ShellMiss::Missing)
            }
            TemplateRead::Failed(err) => {
                guard.state = IndexState::Unreadable;
                let now = Instant::now();
                let due = guard.last_warn.is_none_or(|prev| {
                    now.saturating_duration_since(prev) >= Duration::from_secs(60)
                });
                if due {
                    guard.last_warn = Some(now);
                    tracing::warn!(
                        error = %err,
                        "could not read dist/index.html; serving the raw file if it is readable"
                    );
                }
                Err(ShellMiss::Unreadable)
            }
        }
    };
    let template = loaded?;
    let embed = load_embed(state).await;
    Ok(render_shell(&template, embed.as_deref()))
}

async fn serve_dist<ReqBody>(
    mut files: ServeDir,
    req: Request<ReqBody>,
    path: String,
) -> Response<Body>
where
    ReqBody: Send + 'static,
{
    let mut response = match files.call(req).await {
        Ok(response) => response,
        Err(err) => match err {},
    };
    let cache = if response.status() == StatusCode::NOT_FOUND {
        // A 404 must never be immutable, even for a dxh-looking path.
        "no-store"
    } else {
        let content_type = response
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .map(str::to_owned);
        cache_control_value(&path, content_type.as_deref())
    };
    response
        .headers_mut()
        .insert(header::CACHE_CONTROL, HeaderValue::from_static(cache));
    response.map(Body::new)
}

/// GET /robots.txt
pub async fn robots_txt(State(state): State<AppState>) -> impl IntoResponse {
    let body = render_robots(&state.oauth_config.redirect_base_url);
    (
        StatusCode::OK,
        [
            (header::CONTENT_TYPE, "text/plain; charset=utf-8"),
            (header::CACHE_CONTROL, "public, max-age=3600"),
        ],
        body,
    )
}

/// GET /sitemap.xml — published and otherwise public URLs only.
pub async fn sitemap_xml(State(state): State<AppState>) -> Response {
    match collect_sitemap(&state).await {
        Ok(body) => (
            StatusCode::OK,
            [
                (header::CONTENT_TYPE, "application/xml; charset=utf-8"),
                (header::CACHE_CONTROL, "public, max-age=3600"),
            ],
            body,
        )
            .into_response(),
        Err(err) => {
            tracing::error!(error = %err, "sitemap generation failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                [(header::CONTENT_TYPE, "text/plain; charset=utf-8")],
                "sitemap unavailable",
            )
                .into_response()
        }
    }
}

struct Entry {
    loc: String,
    lastmod: Option<DateTime<Utc>>,
}

struct Sitemap {
    base: String,
    entries: Vec<Entry>,
}

impl Sitemap {
    fn new(redirect_base_url: &str) -> Self {
        Self {
            base: public_base_url(redirect_base_url),
            entries: Vec::new(),
        }
    }

    fn has_room(&self) -> bool {
        self.entries.len() < SITEMAP_URL_CAP
    }

    fn push_path(&mut self, path: &str, lastmod: Option<DateTime<Utc>>) {
        if !self.has_room() {
            return;
        }
        self.entries.push(Entry {
            loc: format!("{}{path}", self.base),
            lastmod,
        });
    }
}

async fn collect_sitemap(state: &AppState) -> Result<String, scuffed_db::DbError> {
    let mut map = Sitemap::new(&state.oauth_config.redirect_base_url);
    for path in PUBLIC_STATIC_PATHS {
        map.push_path(path, None);
    }
    push_strategy_routes(state, &mut map).await?;
    push_articles(state, &mut map).await?;
    push_members(state, &mut map).await?;
    push_tournaments(state, &mut map).await?;
    push_teams(state, &mut map).await?;
    push_matches(state, &mut map).await?;
    push_wiki(state, &mut map).await?;
    push_forum(state, &mut map).await?;
    Ok(render_xml(&map.entries))
}

/// `/strategy`, `/strategy/heroes`, `/strategy/meta` when the clan flag is on.
///
/// Same read as the strategy API gate (`get_settings().strategies_enabled`).
/// A missing row is created with the schema default (`true`). A settings read
/// error fails the sitemap, matching the gate's fail-closed behavior.
/// `/strategy/patch-notes` is never listed; `/patch-notes` is the public URL.
async fn push_strategy_routes(
    state: &AppState,
    map: &mut Sitemap,
) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let settings = state.db.get_settings().await?;
    if !settings.strategies_enabled {
        return Ok(());
    }
    for path in STRATEGY_SITEMAP_PATHS {
        if !map.has_room() {
            break;
        }
        map.push_path(path, None);
    }
    Ok(())
}

/// Published blog posts only (`list_published_articles`). Drafts are omitted
/// even if a row leaked through — same rule as anonymous `GET /api/articles/:slug`.
async fn push_articles(state: &AppState, map: &mut Sitemap) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let articles = state.db.list_published_articles(ARTICLE_LIMIT, 0).await?;
    for article in articles {
        if !map.has_room() {
            break;
        }
        if !article.published || article.slug.is_empty() {
            continue;
        }
        map.push_path(
            &format!("/blog/{}", encode_segment(&article.slug)),
            Some(article.updated_at),
        );
    }
    Ok(())
}

/// Active members only, matching `GET /api/public/members` (inactive profiles 404).
async fn push_members(state: &AppState, map: &mut Sitemap) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let members = state.db.list_members_paginated(MEMBER_LIMIT, 0).await?;
    for member in members {
        if !map.has_room() {
            break;
        }
        if !member.is_active || member.id.is_empty() {
            continue;
        }
        map.push_path(
            &format!("/members/{}", encode_segment(&member.id)),
            Some(member.joined_at),
        );
    }
    Ok(())
}

/// Non-draft tournaments, matching anonymous `GET /api/tournaments` (`include_drafts = false`).
async fn push_tournaments(state: &AppState, map: &mut Sitemap) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let tournaments = state
        .db
        .list_tournaments_paginated(None, None, TOURNAMENT_LIMIT, 0, false)
        .await?;
    for tournament in tournaments {
        if !map.has_room() {
            break;
        }
        if tournament.status == TournamentStatus::Draft || tournament.id.is_empty() {
            continue;
        }
        map.push_path(
            &format!("/tournaments/{}", encode_segment(&tournament.id)),
            Some(tournament.updated_at),
        );
    }
    Ok(())
}

/// Active teams, matching `list_teams` used by the public overview.
async fn push_teams(state: &AppState, map: &mut Sitemap) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let teams = state.db.list_teams().await?;
    for team in teams {
        if !map.has_room() {
            break;
        }
        if !team.is_active || team.id.is_empty() {
            continue;
        }
        map.push_path(
            &format!("/teams/{}", encode_segment(&team.id)),
            Some(team.created_at),
        );
    }
    Ok(())
}

/// Public non-scrim matches, matching `PublicMatch::try_from_match` / `GET /api/public/matches/:id`.
async fn push_matches(state: &AppState, map: &mut Sitemap) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let recent = state
        .db
        .list_public_recent_matches(MATCH_RECENT_LIMIT)
        .await?;
    let upcoming = state
        .db
        .list_public_upcoming_matches(MATCH_UPCOMING_LIMIT)
        .await?;
    let mut seen = HashSet::new();
    for row in recent.into_iter().chain(upcoming) {
        if !map.has_room() {
            break;
        }
        if !row.is_public || matches!(row.match_type, MatchType::Scrim) || row.id.is_empty() {
            continue;
        }
        if !seen.insert(row.id.clone()) {
            continue;
        }
        let lastmod = row.played_at.or(row.scheduled_at);
        map.push_path(&format!("/matches/{}", encode_segment(&row.id)), lastmod);
    }
    Ok(())
}

/// Active wiki pages (`list_wiki_pages` already filters `is_active`).
async fn push_wiki(state: &AppState, map: &mut Sitemap) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let pages = state.db.list_wiki_pages(None, WIKI_LIMIT, 0).await?;
    for page in pages {
        if !map.has_room() {
            break;
        }
        if !page.is_active || page.topic.is_empty() {
            continue;
        }
        map.push_path(
            &format!("/wiki/{}", encode_segment(&page.topic)),
            Some(page.updated_at),
        );
    }
    Ok(())
}

/// Boards with no `min_role` (anonymous read) and their active threads.
/// Restricted boards are omitted, same as `enforce_board_access` for a guest.
async fn push_forum(state: &AppState, map: &mut Sitemap) -> Result<(), scuffed_db::DbError> {
    if !map.has_room() {
        return Ok(());
    }
    let tree = state.db.list_forum_tree().await?;
    let boards = public_boards(&tree);
    for board in boards.into_iter().take(FORUM_BOARD_LIMIT) {
        if !map.has_room() {
            break;
        }
        map.push_path(&format!("/forum/b/{}", encode_segment(&board.slug)), None);
        if !map.has_room() {
            break;
        }
        let threads = state
            .db
            .list_forum_threads(Some(&board.id), None, FORUM_THREADS_PER_BOARD, 0)
            .await?;
        for thread in threads {
            if !map.has_room() {
                break;
            }
            if !thread.is_active || thread.id.is_empty() {
                continue;
            }
            if thread.board_id.as_deref() != Some(board.id.as_str()) {
                continue;
            }
            map.push_path(
                &format!("/forum/t/{}", encode_segment(&thread.id)),
                Some(thread.updated_at),
            );
        }
    }
    Ok(())
}

fn public_boards(tree: &[ForumCategoryNode]) -> Vec<ForumBoard> {
    let mut out = Vec::new();
    for category in tree {
        if !category.category.is_active {
            continue;
        }
        push_public_boards(&category.boards, &mut out);
    }
    out
}

fn push_public_boards(nodes: &[ForumBoardNode], out: &mut Vec<ForumBoard>) {
    for node in nodes {
        if anonymous_board(&node.board) {
            out.push(node.board.clone());
        }
        for sub in &node.sub_boards {
            if anonymous_board(sub) {
                out.push(sub.clone());
            }
        }
    }
}

/// Anonymous forum reads succeed only when `min_role` is unset or blank.
fn anonymous_board(board: &ForumBoard) -> bool {
    board.is_active
        && board
            .min_role
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .is_none()
        && !board.slug.is_empty()
}

fn render_xml(entries: &[Entry]) -> String {
    let mut out = String::from(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n\
         <urlset xmlns=\"http://www.sitemaps.org/schemas/sitemap/0.9\">\n",
    );
    for entry in entries {
        out.push_str("  <url>\n    <loc>");
        out.push_str(&xml_escape(&entry.loc));
        out.push_str("</loc>\n");
        if let Some(ts) = entry.lastmod {
            out.push_str("    <lastmod>");
            out.push_str(&ts.to_rfc3339_opts(SecondsFormat::Secs, true));
            out.push_str("</lastmod>\n");
        }
        out.push_str("  </url>\n");
    }
    out.push_str("</urlset>\n");
    out
}

fn encode_segment(segment: &str) -> String {
    let mut out = String::with_capacity(segment.len());
    for byte in segment.bytes() {
        match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(byte as char);
            }
            _ => out.push_str(&format!("%{byte:02X}")),
        }
    }
    out
}

fn xml_escape(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            _ => out.push(ch),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn robots_allows_public_pages_and_blocks_private_prefixes() {
        let body = render_robots("https://crew.example.test/");
        assert!(body.contains("Sitemap: https://crew.example.test/sitemap.xml"));
        assert!(!body.contains("scuffedcrew"));
        let rules = disallow_rules(&body);
        for path in [
            "/",
            "/members",
            "/members/abc",
            "/teams/abc",
            "/blog",
            "/blog/hello",
            "/patch-notes",
            "/strategy",
            "/strategy/heroes",
            "/strategy/meta",
            "/strategy/patch-notes",
            "/leaderboards",
            "/news",
            "/apply",
            "/tournaments",
            "/wiki",
            "/forum",
            "/events",
            "/community",
            "/feed",
        ] {
            assert!(!path_disallowed(path, &rules), "{path} should stay allowed");
        }
        for path in [
            "/admin",
            "/admin/teams",
            "/admin/games",
            "/admin/settings",
            "/api/health",
            "/login",
            "/setup",
            "/identity",
            "/profile/edit",
            "/dm",
            "/dm/peer",
            "/chat",
            "/polls",
            "/scrims",
            "/stats",
            "/stats/tokens",
            "/strategy/my",
            "/strategy/editor",
            "/strategy/editor/abc",
        ] {
            assert!(path_disallowed(path, &rules), "{path} should be disallowed");
        }
    }

    #[test]
    fn hashed_assets_are_immutable_and_shell_is_not() {
        assert!(is_dioxus_hashed_asset(
            "/assets/ferrous_wave-dxhx13xj2j.png"
        ));
        assert!(is_dioxus_hashed_asset("/assets/app-dxhabc12345.js"));
        assert!(is_dioxus_hashed_asset("/assets/app-dxhabc12345.wasm"));
        assert!(is_dioxus_hashed_asset("/assets/tailwind-dxhdeadbeef.css"));
        assert!(is_dioxus_hashed_asset("/assets/dxhabc12345.js"));
        assert!(is_dioxus_hashed_asset("/assets/app-dxhabc12345.js?v=1"));
        assert!(!is_dioxus_hashed_asset("/assets/favicon.svg"));
        assert!(!is_dioxus_hashed_asset("/assets/plain.js"));
        assert!(!is_dioxus_hashed_asset("/assets/app.wasm"));
        assert!(!is_dioxus_hashed_asset("/index.html"));
        assert!(!is_dioxus_hashed_asset("/assets/not-dxh.js"));
        assert!(!is_dioxus_hashed_asset("/assets/file-dxhabc.js"));

        assert_eq!(
            cache_control_value("/assets/app-dxhabc12345.js", Some("text/javascript")),
            HASHED_ASSET_CACHE
        );
        assert_eq!(
            cache_control_value("/assets/app-dxhabc12345.wasm", Some("application/wasm")),
            HASHED_ASSET_CACHE
        );
        assert_eq!(
            cache_control_value("/assets/tailwind-dxhdeadbeef.css", Some("text/css")),
            HASHED_ASSET_CACHE
        );
        // If a hashed path were ever served as HTML, it still must not be frozen.
        assert_eq!(
            cache_control_value(
                "/assets/missing-dxhabc12345.js",
                Some("text/html; charset=utf-8")
            ),
            SHELL_CACHE
        );
        assert_eq!(
            cache_control_value("/index.html", Some("text/html")),
            SHELL_CACHE
        );
        assert_eq!(cache_control_value("/", Some("text/html")), SHELL_CACHE);
        assert_eq!(
            cache_control_value("/blog/hello", Some("text/html")),
            SHELL_CACHE
        );
        assert_eq!(
            cache_control_value("/assets/favicon.svg", Some("image/svg+xml")),
            STATIC_ASSET_CACHE
        );
        assert_eq!(
            cache_control_value("/assets/mark.png", Some("image/png")),
            STATIC_ASSET_CACHE
        );
        assert_eq!(
            cache_control_value("/assets/plain.js", Some("text/javascript")),
            SHELL_CACHE
        );
        assert_eq!(
            cache_control_value("/assets/app.wasm", Some("application/wasm")),
            SHELL_CACHE
        );
        assert_eq!(
            cache_control_value("/assets/plain.css", Some("text/css")),
            SHELL_CACHE
        );
        assert!(!STATIC_ASSET_CACHE.contains("immutable"));
        assert!(!SHELL_CACHE.contains("immutable"));
    }

    fn disallow_rules(body: &str) -> Vec<&str> {
        body.lines()
            .filter_map(|line| line.trim().strip_prefix("Disallow:"))
            .map(str::trim)
            .collect()
    }

    fn path_disallowed(path: &str, rules: &[&str]) -> bool {
        rules.iter().any(|rule| path.starts_with(rule))
    }

    #[test]
    fn static_miss_paths_are_assets_or_known_extensions() {
        for path in [
            "/assets/tailwind.css",
            "/assets/favicon.svg",
            "/assets/missing-dxhabc12345.js",
            "/assets",
            "/assets/",
            "/nope.wasm",
            "/dir/app.MJS",
            "/notes/data.json",
            "/font/face.woff2",
        ] {
            assert!(is_static_miss_path(path), "{path}");
        }
        for path in ["/pic.gif", "/photo.AVIF", "/assets/hero.gif"] {
            assert!(is_static_miss_path(path), "{path}");
        }
        for path in [
            "/",
            "/index.html",
            "/strategies/foo",
            "/admin/settings",
            "/blog/hello",
            "/wiki/foo.bar",
            "/articles/v1.2-notes",
            "/robots.txt",
        ] {
            assert!(!is_static_miss_path(path), "{path}");
        }
    }

    #[test]
    fn escape_json_for_html_keeps_json_and_blocks_script_breakout() {
        let raw = "{\"d\":\"</script><script>alert(1)</script>\u{2028}&\u{2029}>\"}";
        let escaped = escape_json_for_html(raw);
        assert!(!escaped.contains('<'));
        assert!(!escaped.contains('>'));
        assert!(!escaped.contains('&'));
        assert!(!escaped.contains('\u{2028}'));
        assert!(!escaped.contains('\u{2029}'));
        assert!(escaped.contains("\\u003c"));
        assert!(escaped.contains("\\u003e"));
        assert!(escaped.contains("\\u0026"));
        assert!(escaped.contains("\\u2028"));
        assert!(escaped.contains("\\u2029"));
        let parsed: serde_json::Value = serde_json::from_str(&escaped).unwrap();
        assert_eq!(
            parsed["d"],
            "</script><script>alert(1)</script>\u{2028}&\u{2029}>"
        );
    }

    fn template_with(html: &str) -> ShellTemplate {
        prepare_shell_template(html.to_string())
    }

    fn embed(org_name: &str, site_description: &str, json: &str) -> CachedPublicSettings {
        cached_settings(json, org_name, site_description)
    }

    #[test]
    fn render_shell_inserts_cached_block_immediately_before_head() {
        let template = template_with("<html><head><title>x</title></HEAD><body></body></html>");
        let settings = embed("Clan", "tag", r#"{"a":1}"#);
        let out = render_shell(&template, Some(&settings));
        assert_eq!(
            out,
            "<html><head><title>Clan</title><script id=\"sc-settings\" type=\"application/json\">{\"a\":1}</script></HEAD><body></body></html>"
        );
        assert!(template.head_close.is_some());
    }

    #[test]
    fn render_shell_leaves_template_when_settings_or_head_are_missing() {
        let html = "<html><head><title>The Scuffed Crew</title></head><body>SPA-SHELL-MARKER</body></html>";
        let template = template_with(html);
        assert_eq!(render_shell(&template, None), html);
        let no_head = template_with("<html><body>SPA-SHELL-MARKER</body></html>");
        assert!(no_head.head_close.is_none());
        let settings = embed("Clan", "tag", "{}");
        assert_eq!(
            render_shell(&no_head, Some(&settings)),
            "<html><body>SPA-SHELL-MARKER</body></html>"
        );
    }

    #[test]
    fn head_rewrite_escapes_clan_name_and_ignores_attribute_order() {
        let head = "\
<head>
<title>The Scuffed Crew</title>
<meta
  name=\"description\"
  content=\"The Scuffed Crew — fallback\">
<meta property=\"og:title\" content=\"The Scuffed Crew\" />
<meta content=\"The Scuffed Crew\" property=\"og:site_name\">
<meta property=\"og:description\" content=\"fallback tagline\">
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
</head>";
        let org_name = r#"</title><script>alert(1)</script>" onload="alert(1)"#;
        let description = r#"<img src=x onerror=alert(1)> & "quotes""#;
        let out = rewrite_document_head(head, org_name, description);
        assert!(!out.contains("<script>alert"));
        assert!(!out.contains("onload=\"alert"));
        assert!(out.contains("&lt;/title&gt;&lt;script&gt;alert(1)&lt;/script&gt;"));
        assert!(out.contains("&quot; onload=&quot;alert(1)"));
        assert!(out.contains("&lt;img src=x onerror=alert(1)&gt; &amp; &quot;quotes&quot;"));
        assert!(!out.contains("The Scuffed Crew"));
        assert!(!out.contains("fallback tagline"));
        assert!(out.contains("content=\"width=device-width, initial-scale=1\""));
        assert!(out.contains("property=\"og:site_name\""));
    }

    #[test]
    fn cached_settings_escape_json_once() {
        let raw = r#"{"site_description":"</script>&"}"#;
        let cached = cached_settings(raw, "Clan", "</script>&");
        assert!(cached.escaped_json.contains("\\u003c/script\\u003e"));
        assert!(cached.script_block.contains(&cached.escaped_json));
        assert!(!cached.script_block.contains("</script>&"));
        let again = cached_settings(raw, "Clan", "</script>&");
        assert_eq!(cached.script_block, again.script_block);
    }

    #[tokio::test]
    async fn settings_read_failure_omits_embed_and_leaves_template_title() {
        let root =
            std::env::temp_dir().join(format!("scuffed-seo-unit-fail-{}", uuid::Uuid::new_v4()));
        std::fs::create_dir_all(root.join("dist")).unwrap();
        let html = "<!DOCTYPE html><html><head><title>The Scuffed Crew</title><meta property=\"og:title\" content=\"The Scuffed Crew\"></head><body>SPA-SHELL-MARKER</body></html>";
        std::fs::write(root.join("dist/index.html"), html).unwrap();
        let state = crate::test_support::test_state().await;
        state.public_settings.set_fail_loads(true);
        let app = crate::create_router_with_dist(state, root.join("dist"));
        let response = tower::ServiceExt::oneshot(
            app,
            axum::http::Request::builder()
                .uri("/")
                .body(axum::body::Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), axum::http::StatusCode::OK);
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let body = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(body.contains("SPA-SHELL-MARKER"));
        assert!(!body.contains("sc-settings"), "{body}");
        assert!(body.contains("<title>The Scuffed Crew</title>"), "{body}");
        assert!(body.contains("content=\"The Scuffed Crew\""), "{body}");
        let _ = std::fs::remove_dir_all(root);
    }
}
