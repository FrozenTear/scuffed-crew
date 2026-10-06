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
//! `\u0026`, `\u2028`, and `\u2029`. The script block is cached with the blob.
//! The rewritten head prefix is filled once for that blob and reused. The
//! shell stays `Cache-Control: no-cache`.
//!
//! A cache miss waits at most [`EMBED_SETTINGS_TIMEOUT`]. That includes waiting
//! on an in-flight read that has not yet passed the cap. Once that read is
//! past the cap, or refreshes are backing off, the miss returns immediately
//! with the stale blob or with no embed. The in-flight read keeps running
//! after the cap and can still store for the generation it started with. The
//! cache TTL is measured from when that read started. On timeout or error
//! the last blob from the current generation is served if one exists
//! (stale-on-error). A timeout records its backoff against that starting
//! generation, so a save during the wait does not suppress the next one. A
//! settings write drops the blob and drops it again when the write returns.
//! The response that overlapped the write may include the row it just read;
//! that row is not kept for a later request. Otherwise the tag is omitted and
//! the page is still served. The same
//! successful read rewrites `<title>` and the `og:title`, `og:site_name`,
//! description, and `og:description` meta contents from the trimmed
//! `org_name` and `site_description`. A value that is empty after trimming
//! leaves its own tags as built. The tag is a data block
//! (`type="application/json"`), not an executed script.

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
/// A missing file under `/assets/`, or a missing top-level file (one path
/// segment) whose extension is in [`STATIC_EXTENSIONS`], is a plain 404 with
/// `Cache-Control: no-store`. A multi-segment path outside `/assets/`
/// (`/wiki/config.json`, `/blog/foo.png`) still gets the shell, as do
/// extension-less client routes.
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
    CachedPublicSettings::from_parts(
        script_block,
        org_name.to_string(),
        site_description.to_string(),
    )
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
    let head = embed.rendered_head(&template.html, || {
        rewrite_document_head(
            &template.html[..idx],
            &embed.org_name,
            &embed.site_description,
        )
    });
    let mut out =
        String::with_capacity(head.len() + embed.script_block.len() + template.html.len() - idx);
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
/// fill. A value that is empty after trimming leaves its own tags alone:
/// `org_name` covers `<title>`, `og:title`, and `og:site_name`;
/// `site_description` covers the description and `og:description` metas.
/// The two checks are independent. Attribute order and incidental whitespace
/// do not matter. Tags that are absent stay absent.
fn rewrite_document_head(head: &str, org_name: &str, site_description: &str) -> String {
    let with_title = replace_title_text(head, org_name);
    replace_meta_contents(&with_title, org_name, site_description)
}

/// `None` when `value` is empty or only whitespace. Otherwise the trimmed
/// text, which is what gets written into the tag.
fn filled_setting(value: &str) -> Option<&str> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

fn replace_title_text(head: &str, org_name: &str) -> String {
    let Some(org_name) = filled_setting(org_name) else {
        return head.to_string();
    };
    let escaped = escape_html_text(org_name);
    let mut out = String::with_capacity(head.len() + escaped.len());
    let mut i = 0;
    while let Some(start) = find_tag(head, i, "<title") {
        let Some(content_start) = tag_gt_end(head.as_bytes(), start) else {
            break;
        };
        // Title text is not scanned for comments or scripts. A `<!--` or
        // `<script` in the text must not hide the real `</title>`.
        let Some(content_end) = find_title_close(head, content_start) else {
            break;
        };
        out.push_str(&head[i..content_start]);
        out.push_str(&escaped);
        i = content_end;
    }
    out.push_str(&head[i..]);
    out
}

fn replace_meta_contents(head: &str, org_name: &str, site_description: &str) -> String {
    let mut out = String::with_capacity(head.len());
    let mut i = 0;
    while let Some(start) = find_tag(head, i, "<meta") {
        let Some(end) = tag_gt_end(head.as_bytes(), start) else {
            break;
        };
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

struct ScannedAttr {
    name: String,
    value: String,
    /// Byte range of the attribute value inside `tag`, excluding quotes.
    value_range: std::ops::Range<usize>,
    quoted: bool,
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
            "og:title" | "og:site_name" => filled_setting(org_name),
            "og:description" => filled_setting(site_description),
            _ => None,
        }
    } else if attr_value("name").is_some_and(|name| name.eq_ignore_ascii_case("description")) {
        filled_setting(site_description)
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
    let mut out = String::with_capacity(tag.len() + escaped.len() + 2);
    out.push_str(&tag[..content.value_range.start]);
    if content.quoted {
        out.push_str(&escaped);
    } else {
        // An unquoted value cannot safely hold the replacement. Quote it.
        out.push('"');
        out.push_str(&escaped);
        out.push('"');
    }
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
                quoted: true,
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
                quoted: false,
            });
        }
    }
    attrs
}

fn find_head_close(html: &str) -> Option<usize> {
    find_tag(html, 0, "</head>")
}

/// Next `needle` that is a real tag, skipping comments and the contents of
/// `<script>` and `<style>`. `needle` includes the leading `<` (`<title`,
/// `<meta`, `</head>`, `</title>`).
fn find_tag(html: &str, from: usize, needle: &str) -> Option<usize> {
    let bytes = html.as_bytes();
    let needle = needle.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if bytes[i] != b'<' {
            i += 1;
            continue;
        }
        if let Some(next) = skip_raw_region(bytes, i) {
            i = next;
            continue;
        }
        if eq_ignore_ascii_case_at(bytes, i, needle)
            && (needle.ends_with(b">") || tag_name_ends_at(bytes, i + needle.len()))
        {
            return Some(i);
        }
        // Skip the rest of this tag, including quoted attribute values, so
        // `<!--` or `<script` inside quotes is not treated as markup.
        i = tag_gt_end(bytes, i).unwrap_or(i + 1);
    }
    None
}

/// Index just past the `>` that ends the tag whose `<` is at `open`.
/// Quoted attribute values are skipped, so a `>` inside them does not end the tag.
fn tag_gt_end(bytes: &[u8], open: usize) -> Option<usize> {
    let mut i = open + 1;
    while i < bytes.len() {
        match bytes[i] {
            b'"' | b'\'' => {
                let quote = bytes[i];
                i += 1;
                while i < bytes.len() && bytes[i] != quote {
                    i += 1;
                }
                if i < bytes.len() {
                    i += 1;
                }
            }
            b'>' => return Some(i + 1),
            _ => i += 1,
        }
    }
    None
}

/// `</title` in title text. Does not skip comments or raw elements.
fn find_title_close(html: &str, from: usize) -> Option<usize> {
    let bytes = html.as_bytes();
    let needle = b"</title";
    let mut i = from;
    while i < bytes.len() {
        if eq_ignore_ascii_case_at(bytes, i, needle) && tag_name_ends_at(bytes, i + needle.len()) {
            return Some(i);
        }
        i += 1;
    }
    None
}

fn eq_ignore_ascii_case_at(bytes: &[u8], i: usize, needle: &[u8]) -> bool {
    bytes
        .get(i..i + needle.len())
        .is_some_and(|window| window.eq_ignore_ascii_case(needle))
}

fn tag_name_ends_at(bytes: &[u8], after_name: usize) -> bool {
    match bytes.get(after_name).copied() {
        None => true,
        Some(b) => b.is_ascii_whitespace() || b == b'>' || b == b'/',
    }
}

/// If `i` is the start of a comment, `<script>`, or `<style>`, the index just
/// past that region. Unclosed regions run to the end of the document.
fn skip_raw_region(bytes: &[u8], i: usize) -> Option<usize> {
    if eq_ignore_ascii_case_at(bytes, i, b"<!--") {
        return Some(skip_comment(bytes, i));
    }
    let name = if is_raw_open(bytes, i, b"script") {
        "script"
    } else if is_raw_open(bytes, i, b"style") {
        "style"
    } else {
        return None;
    };
    let after_open = tag_gt_end(bytes, i).unwrap_or(bytes.len());
    Some(find_close_tag(bytes, after_open, name).unwrap_or(bytes.len()))
}

/// Index just past a comment that starts at `i` (`<!--`).
///
/// `<!-->` and `<!--->` are complete empty comments. Anything else runs until
/// `-->`, or to the end of the document when the comment is unclosed.
fn skip_comment(bytes: &[u8], i: usize) -> usize {
    let after = i + 4;
    if bytes.get(after) == Some(&b'>') {
        return after + 1;
    }
    if bytes.get(after) == Some(&b'-') && bytes.get(after + 1) == Some(&b'>') {
        return after + 2;
    }
    find_bytes_ci(bytes, after, b"-->")
        .map(|at| at + 3)
        .unwrap_or(bytes.len())
}

fn is_raw_open(bytes: &[u8], i: usize, name: &[u8]) -> bool {
    bytes.get(i) == Some(&b'<')
        && eq_ignore_ascii_case_at(bytes, i + 1, name)
        && tag_name_ends_at(bytes, i + 1 + name.len())
}

fn find_bytes_ci(bytes: &[u8], from: usize, needle: &[u8]) -> Option<usize> {
    let rest = bytes.get(from..)?;
    rest.windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
        .map(|rel| from + rel)
}

fn find_close_tag(bytes: &[u8], from: usize, name: &str) -> Option<usize> {
    let open = format!("</{name}");
    let open = open.as_bytes();
    let mut i = from;
    while i < bytes.len() {
        if eq_ignore_ascii_case_at(bytes, i, open) && tag_name_ends_at(bytes, i + open.len()) {
            return tag_gt_end(bytes, i);
        }
        i += 1;
    }
    None
}

async fn load_embed(state: &AppState) -> Option<Arc<CachedPublicSettings>> {
    if let Some(hit) = state.public_settings.fresh() {
        return Some(hit);
    }
    // A leader that already ran past the cap still holds the refresh lock and
    // has recorded backoff. The same is true while refreshes are suppressed
    // after an error. Waiting on that lock would cost every miss the full cap.
    if state.public_settings.refresh_suppressed() {
        return state.public_settings.stale();
    }
    // Recorded before the wait. A save that bumps the generation while this
    // request is in flight must not inherit the timeout backoff.
    let generation = state.public_settings.generation();
    // The cap covers waiting for an in-flight read that has not yet passed it,
    // and the read itself. Dropping this wait does not cancel the leader: that
    // read holds an owned lock in its own task and can still store when it
    // finishes.
    match tokio::time::timeout(EMBED_SETTINGS_TIMEOUT, load_embed_locked(state)).await {
        Ok(ready) => ready,
        Err(_elapsed) => {
            state.public_settings.note_refresh_failure(generation);
            state
                .public_settings
                .note_embed_failure("SPA shell settings embed timed out");
            state.public_settings.stale()
        }
    }
}

async fn load_embed_locked(state: &AppState) -> Option<Arc<CachedPublicSettings>> {
    // One reader. Waiters observe that reader's result instead of each
    // hitting the database. This wait sits inside [`EMBED_SETTINGS_TIMEOUT`].
    let flight = state.public_settings.refresh_lock_owned().await;
    if let Some(hit) = state.public_settings.fresh() {
        return Some(hit);
    }
    if state.public_settings.refresh_suppressed() {
        return state.public_settings.stale();
    }
    let generation = state.public_settings.generation();
    let task_state = state.clone();
    let task = tokio::spawn(async move {
        let _flight = flight;
        // TTL starts here, when the read starts, not when it stores.
        let started = tokio::time::Instant::now();
        match embed_read(&task_state).await {
            Ok(payload) => {
                let cached = Arc::new(payload);
                // A rejected store still returns this row for the response
                // that waited. It is not cached, so a later request cannot
                // keep a pre-save row.
                let _stored = task_state.public_settings.store(
                    generation,
                    CachedPublicSettings::clone(&cached),
                    started,
                );
                Some(cached)
            }
            Err(err) => {
                task_state.public_settings.note_refresh_failure(generation);
                task_state
                    .public_settings
                    .note_embed_failure(&format!("SPA shell settings embed skipped: {err}"));
                task_state.public_settings.stale()
            }
        }
    });
    match task.await {
        Ok(value) => value,
        Err(_) => state.public_settings.stale(),
    }
}

fn embed_read<'a>(
    state: &'a AppState,
) -> Pin<Box<dyn Future<Output = Result<CachedPublicSettings, String>> + Send + 'a>> {
    #[cfg(test)]
    if let Some(loader) = state.public_settings.loader() {
        return loader();
    }
    Box::pin(read_public_settings(&state.db))
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

/// Missing files under `/assets/`, or a missing top-level file with a real
/// static extension, must 404 instead of falling through to `index.html`.
///
/// A multi-segment path outside `/assets/` is a client route even when the
/// last segment looks like a file (`/wiki/config.json`, `/blog/foo.png`).
fn is_static_miss_path(url_path: &str) -> bool {
    let path = url_path.split('?').next().unwrap_or(url_path);
    let decoded = urlencoding::decode(path).unwrap_or(std::borrow::Cow::Borrowed(path));
    let path = decoded.as_ref();
    if path == "/assets" || path.starts_with("/assets/") {
        return true;
    }
    let rel = path.trim_start_matches('/');
    if rel.is_empty() || rel.contains('/') {
        return false;
    }
    let Some((_, ext)) = rel.rsplit_once('.') else {
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
    fn static_miss_paths_are_assets_or_top_level_files() {
        for path in [
            "/assets/tailwind.css",
            "/assets/favicon.svg",
            "/assets/missing-dxhabc12345.js",
            "/assets",
            "/assets/",
            "/assets/hero.gif",
            "/nope.wasm",
            "/outside.wasm",
            "/bundle.mjs",
            "/favicon.ico",
            "/pic.gif",
            "/photo.AVIF",
        ] {
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
            "/dir/app.MJS",
            "/notes/data.json",
            "/font/face.woff2",
            "/fonts/missing.woff2",
            "/wiki/config.json",
            "/blog/foo.png",
            "/articles/v1.2.png",
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
    fn blank_description_keeps_template_copy_while_title_is_rewritten() {
        let head = "\
<title>The Scuffed Crew</title>
<meta name=\"description\" content=\"fallback description\">
<meta property=\"og:title\" content=\"The Scuffed Crew\">
<meta property=\"og:site_name\" content=\"The Scuffed Crew\">
<meta property=\"og:description\" content=\"fallback tagline\">";
        let out = rewrite_document_head(head, "  Boot Clan  ", " \n\t ");
        assert_eq!(
            out,
            "\
<title>Boot Clan</title>
<meta name=\"description\" content=\"fallback description\">
<meta property=\"og:title\" content=\"Boot Clan\">
<meta property=\"og:site_name\" content=\"Boot Clan\">
<meta property=\"og:description\" content=\"fallback tagline\">"
        );
    }

    #[test]
    fn blank_org_name_keeps_template_title() {
        let head = "\
<title>The Scuffed Crew</title>
<meta property=\"og:title\" content=\"The Scuffed Crew\">
<meta content=\"The Scuffed Crew\" property=\"og:site_name\">
<meta name=\"description\" content=\"fallback description\">
<meta property=\"og:description\" content=\"fallback tagline\">";
        let out = rewrite_document_head(head, "   ", "  A real tagline  ");
        assert_eq!(
            out,
            "\
<title>The Scuffed Crew</title>
<meta property=\"og:title\" content=\"The Scuffed Crew\">
<meta content=\"The Scuffed Crew\" property=\"og:site_name\">
<meta name=\"description\" content=\"A real tagline\">
<meta property=\"og:description\" content=\"A real tagline\">"
        );
        assert_eq!(rewrite_document_head(head, "", ""), head);
    }

    #[test]
    fn unquoted_content_is_rewritten_as_a_quoted_value() {
        let head = "<meta property=og:title content=Old>";
        let out = rewrite_document_head(head, "Boot <Clan> & \"Q\"", "");
        assert_eq!(
            out,
            "<meta property=og:title content=\"Boot &lt;Clan&gt; &amp; &quot;Q&quot;\">"
        );
    }

    #[test]
    fn comments_scripts_and_styles_are_not_scanned() {
        let html = "\
<!-- <title>Hidden</title> </head> -->
<script>var t = \"<title>Nope</title></head>\";</script>
<style>/* </head> <title>Nope</title> */</style>
<title>The Scuffed Crew</title>
<meta property=\"og:title\" content=\"The Scuffed Crew\">
</head><body>after</body>";
        let close = find_head_close(html).unwrap();
        assert!(html[close..].starts_with("</head><body>"));
        let head = &html[..close];
        let out = rewrite_document_head(head, "Boot", "Desc");
        assert!(out.contains("<title>Hidden</title>"));
        assert!(out.contains("<title>Nope</title>"));
        assert!(out.contains("<title>Boot</title>"));
        assert_eq!(out.matches("<title>Boot</title>").count(), 1);
        assert!(out.contains("property=\"og:title\" content=\"Boot\""));
        assert!(!out.contains("content=\"The Scuffed Crew\""));
    }

    #[test]
    fn quoted_markup_and_empty_comments_do_not_hide_the_head() {
        let html = "\
<meta name=\"description\" content=\"a <!-- b <script> <style> c\">
<meta property=\"og:title\" content='keep > this'>
<title>Plain</title>
</head><body>after";
        let close = find_head_close(html).unwrap();
        assert!(html[close..].starts_with("</head><body>"));
        let head = &html[..close];
        let out = rewrite_document_head(head, "Boot", "Desc");
        assert!(out.contains("content=\"Desc\""));
        assert!(out.contains("content='Boot'"));
        assert!(out.contains("<title>Boot</title>"));
        assert_eq!(out.matches("<title>Boot</title>").count(), 1);
        assert!(!out.contains("<title>Plain"));
        assert!(!head.contains("</head>"));

        // Title text is not a raw region. `<!--` and `<script` there must not
        // hide the real `</title>`.
        let titled = "<title>See <!-- not a comment <script> x</title>";
        assert_eq!(
            rewrite_document_head(titled, "Boot", ""),
            "<title>Boot</title>"
        );

        for prefix in ["<!-->", "<!--->"] {
            let marked = format!("{prefix}<title>The Scuffed Crew</title></head>");
            let end = find_head_close(&marked).unwrap();
            assert!(
                marked[end..].starts_with("</head>"),
                "{prefix} must not swallow the head"
            );
            let rewritten = rewrite_document_head(&marked[..end], "Boot", "");
            assert!(
                rewritten.contains("<title>Boot</title>"),
                "{prefix} hid the title: {rewritten}"
            );
            assert!(rewritten.starts_with(prefix), "{rewritten}");
        }
    }

    /// Rewrite the repo `crates/app/index.html`, and an optional second file.
    ///
    /// `SCUFFED_EXTRA_INDEX`, when set, is a path to another `index.html`
    /// (for example Site PR #150) checked with the same rules.
    /// `SCUFFED_REQUIRE_OG_SITE_NAME=1` requires that extra file to contain
    /// `og:site_name` exactly once. Any file that already contains the tag
    /// must have it exactly once either way; more than one copy fails. This
    /// branch's template has no `og:site_name` yet, so the bundled file stays
    /// green without the variable.
    #[test]
    fn real_app_index_rewrite_fills_each_present_tag_once() {
        let html = include_str!("../../../app/index.html");
        assert_real_index_rewrite(html, false);
        if let Ok(path) = std::env::var("SCUFFED_EXTRA_INDEX") {
            let extra = std::fs::read_to_string(&path).unwrap_or_else(|err| {
                panic!("read {path}: {err}");
            });
            let require_og_site_name = std::env::var("SCUFFED_REQUIRE_OG_SITE_NAME")
                .ok()
                .as_deref()
                == Some("1");
            assert_real_index_rewrite(&extra, require_og_site_name);
        }
    }

    fn assert_real_index_rewrite(html: &str, require_og_site_name: bool) {
        let org = "Boot <Clan> & \"Q\"";
        let desc = "Tag <line> & \"Q\"";
        let escaped_org = escape_html_text(org);
        let escaped_desc = escape_html_text(desc);
        let template = prepare_shell_template(html.to_string());
        let source_head = find_head_close(html).expect("index has </head>");
        let embed = cached_settings(r#"{"org_name":"x"}"#, org, desc);
        let out = render_shell(&template, Some(&embed));
        let head_at = find_head_close(&out).expect("rewritten index has </head>");
        assert_eq!(
            &out[head_at - embed.script_block.len()..head_at],
            embed.script_block.as_str()
        );
        assert!(out[head_at..].starts_with("</head>"));
        assert_eq!(&out[head_at..], &html[source_head..]);
        assert_eq!(
            out.matches(&format!("<title>{escaped_org}</title>"))
                .count(),
            1
        );
        assert!(!out.contains("{app_title}"));
        assert!(!out.contains("<title>The Scuffed Crew</title>"));
        assert_present_once(html, &out, MetaKind::Name("description"), &escaped_desc);
        assert_present_once(html, &out, MetaKind::Property("og:title"), &escaped_org);
        assert_present_once(
            html,
            &out,
            MetaKind::Property("og:description"),
            &escaped_desc,
        );
        let site_names = meta_values(html, MetaKind::Property("og:site_name"));
        if require_og_site_name || !site_names.is_empty() {
            assert_eq!(site_names.len(), 1, "og:site_name must appear exactly once");
            assert_eq!(
                meta_values(&out, MetaKind::Property("og:site_name")),
                vec![escaped_org.clone()]
            );
        } else {
            assert!(site_names.is_empty());
            assert!(meta_values(&out, MetaKind::Property("og:site_name")).is_empty());
        }
        assert_eq!(
            meta_values(html, MetaKind::Property("og:type")),
            meta_values(&out, MetaKind::Property("og:type"))
        );
        assert_eq!(
            meta_values(html, MetaKind::Name("theme-color")),
            meta_values(&out, MetaKind::Name("theme-color"))
        );
        assert_eq!(
            meta_values(html, MetaKind::Name("viewport")),
            meta_values(&out, MetaKind::Name("viewport"))
        );
    }

    #[derive(Clone, Copy)]
    enum MetaKind {
        Name(&'static str),
        Property(&'static str),
    }

    fn assert_present_once(source: &str, out: &str, kind: MetaKind, expected: &str) {
        let before = meta_values(source, kind);
        assert_eq!(
            before.len(),
            1,
            "template must contain this meta exactly once"
        );
        assert_eq!(meta_values(out, kind), vec![expected.to_string()]);
    }

    fn meta_values(html: &str, kind: MetaKind) -> Vec<String> {
        let mut values = Vec::new();
        let mut i = 0;
        while let Some(start) = find_tag(html, i, "<meta") {
            let Some(end) = tag_gt_end(html.as_bytes(), start) else {
                break;
            };
            let attrs = scan_attrs(&html[start..end]);
            let matches = match kind {
                MetaKind::Name(name) => attrs.iter().any(|attr| {
                    attr.name.eq_ignore_ascii_case("name") && attr.value.eq_ignore_ascii_case(name)
                }),
                MetaKind::Property(name) => attrs.iter().any(|attr| {
                    attr.name.eq_ignore_ascii_case("property")
                        && attr.value.eq_ignore_ascii_case(name)
                }),
            };
            if matches
                && let Some(content) = attrs
                    .iter()
                    .find(|attr| attr.name.eq_ignore_ascii_case("content"))
            {
                values.push(content.value.clone());
            }
            i = end;
        }
        values
    }

    #[test]
    fn multiline_og_description_with_content_on_its_own_line_is_rewritten() {
        // Same shape as the frontend shell: property and content on their
        // own lines, content not sharing a line with the tag name.
        let head = "\
<meta property=\"og:site_name\" content=\"The Scuffed Crew\">
<meta
      property=\"og:description\"
      content=\"Multi-game EMEA gaming org. Small teams, real structure, scheduled play nights.\"
    />";
        let out = rewrite_document_head(head, "  Clan  ", "Scheduled nights");
        assert_eq!(
            out,
            "\
<meta property=\"og:site_name\" content=\"Clan\">
<meta
      property=\"og:description\"
      content=\"Scheduled nights\"
    />"
        );
    }

    #[test]
    fn rendered_head_is_reused_for_the_same_template() {
        let cached = cached_settings(r#"{"org_name":"Boot"}"#, "Boot", "Desc");
        let head: Arc<str> =
            Arc::from("<title>Old</title><meta name=\"description\" content=\"Old\">");
        let other: Arc<str> =
            Arc::from("<title>Other</title><meta name=\"description\" content=\"Other\">");
        let first = cached.rendered_head(&head, || rewrite_document_head(&head, "Boot", "Desc"));
        let second = cached.rendered_head(&head, || panic!("rebuilt the cached head"));
        assert!(std::sync::Arc::ptr_eq(&first, &second));
        assert!(first.contains("<title>Boot</title>"));
        assert!(first.contains("content=\"Desc\""));
        let third = cached.rendered_head(&other, || rewrite_document_head(&other, "Boot", "Desc"));
        assert!(third.contains("<title>Boot</title>"));
        assert!(third.contains("content=\"Desc\""));
        assert!(!std::sync::Arc::ptr_eq(&first, &third));
    }

    #[test]
    fn cached_settings_escape_json_once() {
        let raw = r#"{"site_description":"</script>&"}"#;
        let cached = cached_settings(raw, "Clan", "</script>&");
        assert!(cached.script_block.contains("\\u003c/script\\u003e"));
        assert!(!cached.script_block.contains("</script>&"));
        let again = cached_settings(raw, "Clan", "</script>&");
        assert_eq!(cached.script_block, again.script_block);
    }

    fn shell_html() -> &'static str {
        "<!DOCTYPE html><html><head><title>The Scuffed Crew</title>\
<meta name=\"description\" content=\"fallback description\">\
<meta property=\"og:title\" content=\"The Scuffed Crew\">\
<meta property=\"og:site_name\" content=\"The Scuffed Crew\">\
<meta property=\"og:description\" content=\"fallback tagline\">\
</head><body>SPA-SHELL-MARKER</body></html>"
    }

    struct ShellFixture {
        root: std::path::PathBuf,
    }

    impl ShellFixture {
        async fn new(label: &str) -> (Self, crate::state::AppState, axum::Router) {
            let root =
                std::env::temp_dir().join(format!("scuffed-seo-{label}-{}", uuid::Uuid::new_v4()));
            std::fs::create_dir_all(root.join("dist")).unwrap();
            std::fs::write(root.join("dist/index.html"), shell_html()).unwrap();
            let state = crate::test_support::test_state().await;
            let app = crate::create_router_with_dist(state.clone(), root.join("dist"));
            (Self { root }, state, app)
        }
    }

    impl Drop for ShellFixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.root);
        }
    }

    fn named_settings(org: &str) -> CachedPublicSettings {
        cached_settings(&format!(r#"{{"org_name":"{org}"}}"#), org, "tagline")
    }

    async fn body_of(app: axum::Router, method: Method, uri: &str) -> (StatusCode, Vec<u8>) {
        let response = tower::ServiceExt::oneshot(
            app,
            Request::builder()
                .method(method)
                .uri(uri)
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        let status = response.status();
        let bytes = http_body_util::BodyExt::collect(response.into_body())
            .await
            .unwrap()
            .to_bytes();
        (status, bytes.to_vec())
    }

    async fn shell_text(app: axum::Router, uri: &str) -> String {
        let (status, bytes) = body_of(app, Method::GET, uri).await;
        assert_eq!(status, StatusCode::OK);
        String::from_utf8(bytes).unwrap()
    }

    /// Poll a spawned shell request until it finishes.
    ///
    /// One yield is not enough: the request awaits the lock and the loader.
    /// Stop after a bounded number of turns so a missed timeout fails the
    /// test instead of parking it.
    async fn drive<T>(task: &mut tokio::task::JoinHandle<T>) {
        for _ in 0..64 {
            if task.is_finished() {
                return;
            }
            tokio::task::yield_now().await;
        }
    }

    /// Sender that releases a pre-save read. Taken once by the write hook.
    type ReleaseSender = Arc<Mutex<Option<tokio::sync::oneshot::Sender<()>>>>;

    /// Pre-save row that signals when the read has started, then waits.
    ///
    /// The returned sender releases that wait. Both ends live behind a mutex
    /// so the loader (an `Fn`) and the write hooks can each take them once.
    fn blocking_pre_save_loader(
        state: &crate::state::AppState,
        calls: &Arc<std::sync::atomic::AtomicUsize>,
    ) -> (tokio::sync::oneshot::Receiver<()>, ReleaseSender) {
        let calls = Arc::clone(calls);
        let (entered_tx, entered_rx) = tokio::sync::oneshot::channel();
        let entered_tx = Arc::new(std::sync::Mutex::new(Some(entered_tx)));
        let (release_tx, release_rx) = tokio::sync::oneshot::channel();
        let release_tx = Arc::new(std::sync::Mutex::new(Some(release_tx)));
        let release_rx = Arc::new(tokio::sync::Mutex::new(Some(release_rx)));
        state.public_settings.set_loader(Some(Arc::new(move || {
            let calls = Arc::clone(&calls);
            let entered_tx = Arc::clone(&entered_tx);
            let release_rx = Arc::clone(&release_rx);
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if let Some(tx) = entered_tx
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .take()
                {
                    let _ = tx.send(());
                }
                if let Some(rx) = release_rx.lock().await.take() {
                    let _ = rx.await;
                }
                Ok(named_settings("My Clan"))
            })
        })));
        (entered_rx, release_tx)
    }

    fn counting_loader(
        calls: &Arc<std::sync::atomic::AtomicUsize>,
        result: Result<CachedPublicSettings, String>,
    ) -> crate::state::EmbedLoader {
        let calls = Arc::clone(calls);
        Arc::new(move || {
            let calls = Arc::clone(&calls);
            let result = result.clone();
            Box::pin(async move {
                calls.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                result
            })
        })
    }

    #[tokio::test]
    async fn slow_loader_serves_stale_within_the_cap() {
        let (_tree, state, app) = ShellFixture::new("slow-loader").await;
        assert!(state.public_settings.store(
            state.public_settings.generation(),
            named_settings("Kept Clan"),
            tokio::time::Instant::now(),
        ));
        state.public_settings.expire_for_test();
        state.public_settings.set_loader(Some(Arc::new(|| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(named_settings("Slow Clan"))
            })
        })));
        tokio::time::pause();
        let mut task = tokio::spawn(shell_text(app, "/"));
        // `sleep` while paused auto-advances and runs timers in order.
        // `advance` alone can move the clock before this request arms its cap.
        tokio::time::sleep(EMBED_SETTINGS_TIMEOUT).await;
        drive(&mut task).await;
        assert!(
            task.is_finished(),
            "a loader slower than the cap must not hold the shell open"
        );
        let body = task.await.unwrap();
        assert!(body.contains("Kept Clan"), "{body}");
        assert!(body.contains("sc-settings"), "{body}");
        assert!(!body.contains("Slow Clan"), "{body}");
        // The request already returned. The leader still holds the refresh
        // lock and can store when its read finishes.
        tokio::time::sleep(Duration::from_secs(30)).await;
        for _ in 0..64 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            state
                .public_settings
                .stale()
                .map(|blob| blob.org_name.clone()),
            Some("Slow Clan".to_string()),
            "a read that outlives the cap must still fill the cache"
        );
        assert!(
            state.public_settings.fresh().is_none(),
            "the TTL starts when the read starts, so a 30s read is already stale"
        );
    }

    #[tokio::test]
    async fn miss_during_slow_leader_returns_without_waiting_the_cap() {
        let (_tree, state, app) = ShellFixture::new("slow-miss").await;
        assert!(state.public_settings.store(
            state.public_settings.generation(),
            named_settings("Kept Clan"),
            tokio::time::Instant::now(),
        ));
        state.public_settings.expire_for_test();
        state.public_settings.set_loader(Some(Arc::new(|| {
            Box::pin(async {
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(named_settings("Slow Clan"))
            })
        })));
        tokio::time::pause();
        let mut first = tokio::spawn(shell_text(app.clone(), "/"));
        tokio::time::sleep(EMBED_SETTINGS_TIMEOUT).await;
        drive(&mut first).await;
        assert!(first.is_finished());
        let _ = first.await.unwrap();
        assert!(
            state.public_settings.refresh_suppressed(),
            "the timed-out leader must record backoff before the next miss"
        );

        let mut second = tokio::spawn(shell_text(app, "/"));
        // No clock advance. A miss that waits on the lock would still be inside
        // the cap and this drive would leave it unfinished.
        drive(&mut second).await;
        assert!(
            second.is_finished(),
            "a miss during a read that already passed the cap must not wait out the cap"
        );
        let body = second.await.unwrap();
        assert!(body.contains("Kept Clan"), "{body}");
        assert!(body.contains("sc-settings"), "{body}");
        assert!(!body.contains("Slow Clan"), "{body}");
    }

    #[tokio::test]
    async fn loader_error_serves_the_stale_blob() {
        let (_tree, state, app) = ShellFixture::new("err-stale").await;
        assert!(state.public_settings.store(
            state.public_settings.generation(),
            named_settings("Kept Clan"),
            tokio::time::Instant::now(),
        ));
        state.public_settings.expire_for_test();
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        state
            .public_settings
            .set_loader(Some(counting_loader(&calls, Err("db down".into()))));
        let body = shell_text(app, "/").await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(body.contains("Kept Clan"), "{body}");
        assert!(body.contains("sc-settings"), "{body}");
    }

    #[tokio::test]
    async fn loader_error_without_stale_omits_the_embed() {
        let (_tree, state, app) = ShellFixture::new("err-empty").await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        state
            .public_settings
            .set_loader(Some(counting_loader(&calls, Err("db down".into()))));
        let body = shell_text(app, "/").await;
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(body.contains("SPA-SHELL-MARKER"), "{body}");
        assert!(!body.contains("sc-settings"), "{body}");
        assert!(body.contains("<title>The Scuffed Crew</title>"), "{body}");
        assert!(body.contains("content=\"The Scuffed Crew\""), "{body}");
    }

    #[tokio::test]
    async fn concurrent_misses_call_the_loader_once() {
        let (_tree, state, app) = ShellFixture::new("singleflight").await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (release_tx, release_rx) = tokio::sync::watch::channel(false);
        let release_rx = Arc::new(tokio::sync::Mutex::new(release_rx));
        let calls_loader = Arc::clone(&calls);
        state.public_settings.set_loader(Some(Arc::new(move || {
            let calls_loader = Arc::clone(&calls_loader);
            let release_rx = Arc::clone(&release_rx);
            Box::pin(async move {
                calls_loader.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let mut rx = release_rx.lock().await;
                let _ = rx.wait_for(|go| *go).await;
                Ok(named_settings("Once Clan"))
            })
        })));
        tokio::time::pause();
        let mut tasks = Vec::new();
        for _ in 0..8 {
            tasks.push(tokio::spawn(shell_text(app.clone(), "/")));
        }
        for _ in 0..64 {
            if calls.load(std::sync::atomic::Ordering::SeqCst) > 0 {
                break;
            }
            tokio::task::yield_now().await;
        }
        for _ in 0..8 {
            tokio::task::yield_now().await;
        }
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "concurrent misses must share one read"
        );
        release_tx.send(true).unwrap();
        for task in &mut tasks {
            drive(task).await;
            assert!(
                task.is_finished(),
                "shared read did not finish while paused"
            );
        }
        for task in tasks {
            let body = task.await.unwrap();
            assert!(body.contains("Once Clan"), "{body}");
        }
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
    }

    #[tokio::test]
    async fn refresh_backoff_suppresses_reads_until_it_elapses() {
        let (_tree, state, app) = ShellFixture::new("backoff").await;
        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let mode = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let calls_loader = Arc::clone(&calls);
        let mode_loader = Arc::clone(&mode);
        state.public_settings.set_loader(Some(Arc::new(move || {
            let calls_loader = Arc::clone(&calls_loader);
            let mode_loader = Arc::clone(&mode_loader);
            Box::pin(async move {
                calls_loader.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                if mode_loader.load(std::sync::atomic::Ordering::SeqCst) {
                    Ok(named_settings("Recovered Clan"))
                } else {
                    Err("db down".into())
                }
            })
        })));
        tokio::time::pause();
        let mut first = tokio::spawn(shell_text(app.clone(), "/"));
        drive(&mut first).await;
        assert!(first.is_finished());
        let first_body = first.await.unwrap();
        assert!(!first_body.contains("sc-settings"), "{first_body}");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);

        let mut second = tokio::spawn(shell_text(app.clone(), "/"));
        drive(&mut second).await;
        assert!(second.is_finished());
        let second_body = second.await.unwrap();
        assert!(!second_body.contains("sc-settings"), "{second_body}");
        assert_eq!(
            calls.load(std::sync::atomic::Ordering::SeqCst),
            1,
            "backoff must skip another read"
        );

        tokio::time::advance(crate::state::EMBED_REFRESH_BACKOFF).await;
        mode.store(true, std::sync::atomic::Ordering::SeqCst);
        let mut third = tokio::spawn(shell_text(app, "/"));
        drive(&mut third).await;
        assert!(third.is_finished());
        let third_body = third.await.unwrap();
        assert!(third_body.contains("Recovered Clan"), "{third_body}");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    #[tokio::test]
    async fn queued_read_falls_back_within_the_cap() {
        let (_tree, state, app) = ShellFixture::new("queued-cap").await;
        assert!(state.public_settings.store(
            state.public_settings.generation(),
            named_settings("Kept Clan"),
            tokio::time::Instant::now(),
        ));
        state.public_settings.expire_for_test();
        let _guard = state.public_settings.refresh_lock().await;
        tokio::time::pause();
        let mut task = tokio::spawn(shell_text(app, "/wiki/config.json"));
        tokio::time::sleep(EMBED_SETTINGS_TIMEOUT).await;
        drive(&mut task).await;
        assert!(
            task.is_finished(),
            "a request waiting on the refresh lock must fall back within the cap"
        );
        let body = task.await.unwrap();
        assert!(body.contains("Kept Clan"), "{body}");
        assert!(body.contains("sc-settings"), "{body}");
    }

    #[tokio::test]
    async fn read_during_save_does_not_keep_the_pre_save_blob() {
        let (_tree, state, app) = ShellFixture::new("save-race").await;
        let user = state
            .db
            .create_local_user("embed-admin", "unused-hash")
            .await
            .unwrap();
        state
            .db
            .create_member(&user.id, "Embed Admin", scuffed_db::OrgRole::Admin)
            .await
            .unwrap();
        state
            .db
            .create_session(&user.id, "embed-admin-token", 24)
            .await
            .unwrap();

        let primed = shell_text(app.clone(), "/").await;
        assert!(primed.contains("My Clan"), "{primed}");

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (entered_rx, release_tx) = blocking_pre_save_loader(&state, &calls);
        let entered_rx = Arc::new(tokio::sync::Mutex::new(Some(entered_rx)));
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let done_tx = Arc::new(std::sync::Mutex::new(Some(done_tx)));
        let done_rx = Arc::new(tokio::sync::Mutex::new(Some(done_rx)));
        let app_for_read = app.clone();
        state.public_settings.set_write_hook(Some(Arc::new(move || {
            let entered_rx = Arc::clone(&entered_rx);
            let done_tx = Arc::clone(&done_tx);
            let app_for_read = app_for_read.clone();
            Box::pin(async move {
                tokio::spawn(async move {
                    let body = shell_text(app_for_read, "/").await;
                    if let Some(tx) = done_tx.lock().unwrap_or_else(|err| err.into_inner()).take() {
                        let _ = tx.send(body);
                    }
                });
                let rx = entered_rx.lock().await.take().expect("entered receiver");
                tokio::time::timeout(Duration::from_secs(5), rx)
                    .await
                    .expect("loader entered")
                    .expect("entered channel");
            })
        })));
        let cache = state.public_settings.clone();
        state
            .public_settings
            .set_after_write_hook(Some(Arc::new(move || {
                let release_tx = Arc::clone(&release_tx);
                let done_rx = Arc::clone(&done_rx);
                let cache = cache.clone();
                Box::pin(async move {
                    let tx = release_tx
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .take()
                        .expect("release sender");
                    tx.send(()).expect("release the racing read");
                    let rx = done_rx.lock().await.take().expect("done receiver");
                    let body = tokio::time::timeout(Duration::from_secs(5), rx)
                        .await
                        .expect("racing read finished")
                        .expect("racing read channel");
                    assert!(
                        body.contains("sc-settings") && body.contains("My Clan"),
                        "the overlapping response still includes the row it read: {body}"
                    );
                    assert_eq!(
                        cache.fresh().map(|blob| blob.org_name.clone()),
                        Some("My Clan".to_string()),
                        "the read must store after the write returns and before the handler returns"
                    );
                })
            })));

        let response = tower::ServiceExt::oneshot(
            app.clone(),
            Request::builder()
                .method(Method::PUT)
                .uri("/api/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "sc_session=embed-admin-token")
                .body(Body::from(r#"{"org_name":"New Clan"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            state
                .public_settings
                .fresh()
                .map(|blob| blob.org_name != "My Clan")
                .unwrap_or(true),
            "old settings must not stay fresh after the save"
        );
        assert!(
            state.public_settings.stale().is_none(),
            "a pre-save blob must not remain as the stale fallback"
        );

        state
            .public_settings
            .set_loader(Some(counting_loader(&calls, Err("db down".into()))));
        let failed = shell_text(app.clone(), "/").await;
        assert!(
            !failed.contains("My Clan"),
            "stale-on-error must not serve the pre-save row: {failed}"
        );
        assert!(!failed.contains("sc-settings"), "{failed}");

        state.public_settings.set_loader(None);
        state.public_settings.invalidate();
        let saved = shell_text(app, "/").await;
        assert!(saved.contains("New Clan"), "{saved}");
        assert!(!saved.contains("My Clan"), "{saved}");
    }

    #[tokio::test]
    async fn overlapping_read_serves_its_row_without_storing_it() {
        let (_tree, state, app) = ShellFixture::new("save-reject").await;
        let user = state
            .db
            .create_local_user("embed-admin", "unused-hash")
            .await
            .unwrap();
        state
            .db
            .create_member(&user.id, "Embed Admin", scuffed_db::OrgRole::Admin)
            .await
            .unwrap();
        state
            .db
            .create_session(&user.id, "embed-reject-token", 24)
            .await
            .unwrap();

        let primed = shell_text(app.clone(), "/").await;
        assert!(primed.contains("My Clan"), "{primed}");

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (entered_rx, release_tx) = blocking_pre_save_loader(&state, &calls);
        let entered_rx = Arc::new(tokio::sync::Mutex::new(Some(entered_rx)));
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let done_tx = Arc::new(std::sync::Mutex::new(Some(done_tx)));
        let done_rx = Arc::new(tokio::sync::Mutex::new(Some(done_rx)));
        let app_for_read = app.clone();
        let cache = state.public_settings.clone();
        state.public_settings.set_write_hook(Some(Arc::new(move || {
            let entered_rx = Arc::clone(&entered_rx);
            let release_tx = Arc::clone(&release_tx);
            let done_tx = Arc::clone(&done_tx);
            let done_rx = Arc::clone(&done_rx);
            let app_for_read = app_for_read.clone();
            let cache = cache.clone();
            Box::pin(async move {
                tokio::spawn(async move {
                    let body = shell_text(app_for_read, "/").await;
                    if let Some(tx) = done_tx.lock().unwrap_or_else(|err| err.into_inner()).take() {
                        let _ = tx.send(body);
                    }
                });
                let rx = entered_rx.lock().await.take().expect("entered receiver");
                tokio::time::timeout(Duration::from_secs(5), rx)
                    .await
                    .expect("loader entered")
                    .expect("entered channel");
                // The in-flight read already snapshotted the previous generation.
                cache.invalidate();
                let tx = release_tx
                    .lock()
                    .unwrap_or_else(|err| err.into_inner())
                    .take()
                    .expect("release sender");
                tx.send(()).expect("release the racing read");
                let rx = done_rx.lock().await.take().expect("done receiver");
                let body = tokio::time::timeout(Duration::from_secs(5), rx)
                    .await
                    .expect("racing read finished")
                    .expect("racing read channel");
                assert!(
                    body.contains("sc-settings") && body.contains("My Clan"),
                    "a rejected store still returns the row for that response: {body}"
                );
                assert!(
                    cache.fresh().is_none(),
                    "the rejected row must not be cached"
                );
            })
        })));

        let response = tower::ServiceExt::oneshot(
            app.clone(),
            Request::builder()
                .method(Method::PUT)
                .uri("/api/settings")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::COOKIE, "sc_session=embed-reject-token")
                .body(Body::from(r#"{"org_name":"New Clan"}"#))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(state.public_settings.fresh().is_none());
        assert!(state.public_settings.stale().is_none());

        state
            .public_settings
            .set_loader(Some(counting_loader(&calls, Err("db down".into()))));
        let failed = shell_text(app, "/").await;
        assert!(!failed.contains("My Clan"), "{failed}");
        assert!(!failed.contains("sc-settings"), "{failed}");
    }

    #[tokio::test]
    async fn setup_read_during_save_does_not_keep_the_pre_setup_blob() {
        let (_tree, state, app) = ShellFixture::new("setup-race").await;
        let primed = shell_text(app.clone(), "/").await;
        assert!(primed.contains("My Clan"), "{primed}");

        let calls = Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let (entered_rx, release_tx) = blocking_pre_save_loader(&state, &calls);
        let entered_rx = Arc::new(tokio::sync::Mutex::new(Some(entered_rx)));
        let (done_tx, done_rx) = tokio::sync::oneshot::channel();
        let done_tx = Arc::new(std::sync::Mutex::new(Some(done_tx)));
        let done_rx = Arc::new(tokio::sync::Mutex::new(Some(done_rx)));
        let app_for_read = app.clone();
        state.public_settings.set_write_hook(Some(Arc::new(move || {
            let entered_rx = Arc::clone(&entered_rx);
            let done_tx = Arc::clone(&done_tx);
            let app_for_read = app_for_read.clone();
            Box::pin(async move {
                tokio::spawn(async move {
                    let body = shell_text(app_for_read, "/").await;
                    if let Some(tx) = done_tx.lock().unwrap_or_else(|err| err.into_inner()).take() {
                        let _ = tx.send(body);
                    }
                });
                let rx = entered_rx.lock().await.take().expect("entered receiver");
                tokio::time::timeout(Duration::from_secs(5), rx)
                    .await
                    .expect("loader entered")
                    .expect("entered channel");
            })
        })));
        let cache = state.public_settings.clone();
        state
            .public_settings
            .set_after_write_hook(Some(Arc::new(move || {
                let release_tx = Arc::clone(&release_tx);
                let done_rx = Arc::clone(&done_rx);
                let cache = cache.clone();
                Box::pin(async move {
                    let tx = release_tx
                        .lock()
                        .unwrap_or_else(|err| err.into_inner())
                        .take()
                        .expect("release sender");
                    tx.send(()).expect("release the racing read");
                    let rx = done_rx.lock().await.take().expect("done receiver");
                    let body = tokio::time::timeout(Duration::from_secs(5), rx)
                        .await
                        .expect("racing read finished")
                        .expect("racing read channel");
                    assert!(
                        body.contains("sc-settings") && body.contains("My Clan"),
                        "the overlapping setup response still includes the row it read: {body}"
                    );
                    assert_eq!(
                        cache.fresh().map(|blob| blob.org_name.clone()),
                        Some("My Clan".to_string()),
                        "the read must store after setup's write returns and before the handler returns"
                    );
                })
            })));

        let response = tower::ServiceExt::oneshot(
            app.clone(),
            Request::builder()
                .method(Method::POST)
                .uri("/api/auth/setup")
                .header(header::CONTENT_TYPE, "application/json")
                .header("x-forwarded-for", "127.0.0.1")
                .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                    [127, 0, 0, 1],
                    40000,
                ))))
                .body(Body::from(
                    r#"{"username":"firstadmin","password":"a-strong-password","org_name":"Boot Clan"}"#,
                ))
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(response.status(), StatusCode::OK, "POST /api/auth/setup");
        assert_eq!(calls.load(std::sync::atomic::Ordering::SeqCst), 1);
        assert!(
            state
                .public_settings
                .fresh()
                .map(|blob| blob.org_name != "My Clan")
                .unwrap_or(true),
            "old settings must not stay fresh after setup"
        );
        assert!(
            state.public_settings.stale().is_none(),
            "a pre-setup blob must not remain as the stale fallback"
        );

        state.public_settings.set_loader(None);
        state.public_settings.invalidate();
        let saved = shell_text(app, "/").await;
        assert!(saved.contains("Boot Clan"), "{saved}");
        assert!(!saved.contains("My Clan"), "{saved}");
    }

    #[tokio::test]
    async fn timeout_records_backoff_against_the_generation_at_read_start() {
        let (_tree, state, app) = ShellFixture::new("timeout-gen").await;
        let entered = Arc::new(std::sync::atomic::AtomicBool::new(false));
        let entered_loader = Arc::clone(&entered);
        state.public_settings.set_loader(Some(Arc::new(move || {
            let entered_loader = Arc::clone(&entered_loader);
            Box::pin(async move {
                entered_loader.store(true, std::sync::atomic::Ordering::SeqCst);
                tokio::time::sleep(Duration::from_secs(30)).await;
                Ok(named_settings("Late Clan"))
            })
        })));
        tokio::time::pause();
        let mut task = tokio::spawn(shell_text(app, "/"));
        for _ in 0..64 {
            if entered.load(std::sync::atomic::Ordering::SeqCst) {
                break;
            }
            tokio::task::yield_now().await;
        }
        assert!(
            entered.load(std::sync::atomic::Ordering::SeqCst),
            "loader did not start before the generation moved"
        );
        assert!(
            !task.is_finished(),
            "the read timed out before the generation moved"
        );
        state.public_settings.invalidate();
        tokio::time::sleep(EMBED_SETTINGS_TIMEOUT).await;
        drive(&mut task).await;
        assert!(
            task.is_finished(),
            "the shell must still return when the read outlives the cap"
        );
        let body = task.await.unwrap();
        assert!(!body.contains("Late Clan"), "{body}");
        assert!(
            !state.public_settings.refresh_suppressed(),
            "backoff must be recorded against the generation captured at read start"
        );
    }

    #[tokio::test]
    async fn head_on_the_shell_service_has_an_empty_body() {
        // Axum's router clears every HEAD body, so a check through the router
        // cannot see a shell that forgot to send an empty body. Call the
        // service itself and read the bytes.
        let (tree, state, _app) = ShellFixture::new("head-bytes").await;
        let files = spa_service(&tree.root.join("dist"), state);
        let head_response = tower::ServiceExt::oneshot(
            files.clone(),
            Request::builder()
                .method(Method::HEAD)
                .uri("/wiki/foo.bar")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        let get_response = tower::ServiceExt::oneshot(
            files,
            Request::builder()
                .method(Method::GET)
                .uri("/wiki/foo.bar")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
        assert_eq!(head_response.status(), StatusCode::OK);
        assert_eq!(get_response.status(), StatusCode::OK);
        let content_length = get_response
            .headers()
            .get(header::CONTENT_LENGTH)
            .and_then(|value| value.to_str().ok())
            .and_then(|value| value.parse::<usize>().ok());
        let head_bytes = http_body_util::BodyExt::collect(head_response.into_body())
            .await
            .unwrap()
            .to_bytes();
        let get_bytes = http_body_util::BodyExt::collect(get_response.into_body())
            .await
            .unwrap()
            .to_bytes();
        assert_eq!(head_bytes.len(), 0);
        assert!(!get_bytes.is_empty());
        assert_eq!(content_length, Some(get_bytes.len()));
    }
}
