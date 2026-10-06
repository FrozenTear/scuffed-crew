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
//! `\u0026`, `\u2028`, and `\u2029`. The shell stays `Cache-Control: no-cache`.
//! A settings read failure omits the tag and still serves the page. The tag
//! is a data block (`type="application/json"`), not an executed script.

use std::collections::HashSet;
use std::future::Future;
use std::path::{Path, PathBuf};
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};

use axum::body::Body;
use axum::extract::State;
use axum::http::{HeaderValue, Method, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use scuffed_db::{ForumBoard, ForumBoardNode, ForumCategoryNode, MatchType, TournamentStatus};
use tower::Service;
use tower_http::services::ServeDir;

use crate::state::AppState;

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

/// `dist/` with an in-memory `index.html` shell, plus cache headers.
///
/// The template is read once at router build. A new deploy replaces the
/// process, so the file is not watched. Missing `index.html` keeps the old
/// behaviour: real files are served, everything else is 404 (no embed).
///
/// A missing file under `/assets/`, or any missing path with a static
/// extension (`.js`, `.css`, `.wasm`, images, fonts, …), is a plain 404 with
/// `Cache-Control: no-store`. Extension-less client routes still get the shell.
///
/// A hand-rolled service (rather than `middleware::from_fn`) so the future
/// stays `Send`. Axum's function middleware around `ServeDir` does not.
pub(crate) fn spa_service(dist_dir: &Path, state: AppState) -> SpaService {
    let index_html = read_index_template(dist_dir);
    let files = ServeDir::new(dist_dir);
    SpaService {
        dist_dir: dist_dir.to_path_buf(),
        index_html,
        files,
        state,
    }
}

fn read_index_template(dist_dir: &Path) -> Option<Arc<str>> {
    let path = dist_dir.join("index.html");
    match std::fs::read_to_string(&path) {
        Ok(html) => Some(Arc::from(html)),
        Err(err) if err.kind() == std::io::ErrorKind::NotFound => None,
        Err(err) => {
            tracing::warn!(
                error = %err,
                path = %path.display(),
                "could not preload dist/index.html; SPA shell embed disabled"
            );
            None
        }
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

/// Insert the settings data block immediately before `</head>`.
///
/// `escaped_json` is already escaped by [`escape_json_for_html`]. `None` (the
/// settings read failed) returns `html` unchanged.
pub(crate) fn inject_settings_script(html: &str, escaped_json: Option<&str>) -> String {
    let Some(json) = escaped_json else {
        return html.to_string();
    };
    let Some(idx) = find_head_close(html) else {
        tracing::warn!("SPA shell has no </head>; omitting sc-settings embed");
        return html.to_string();
    };
    let mut out = String::with_capacity(html.len() + SETTINGS_SCRIPT_OPEN.len() + json.len() + 9);
    out.push_str(&html[..idx]);
    out.push_str(SETTINGS_SCRIPT_OPEN);
    out.push_str(json);
    out.push_str("</script>");
    out.push_str(&html[idx..]);
    out
}

fn find_head_close(html: &str) -> Option<usize> {
    let needle = b"</head>";
    html.as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

async fn load_embed_json(state: &AppState) -> Option<String> {
    if state.public_settings.fail_loads() {
        tracing::warn!("SPA shell settings embed skipped");
        return None;
    }
    let (generation, cached) = state.public_settings.fresh();
    if let Some(json) = cached {
        return Some(escape_json_for_html(&json));
    }
    match crate::routes::settings::anonymous_settings_json(&state.db).await {
        Ok(json) => {
            state.public_settings.store(generation, json.clone());
            Some(escape_json_for_html(&json))
        }
        Err(err) => {
            tracing::warn!(error = %err, "SPA shell settings embed skipped");
            None
        }
    }
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

fn classify_spa_route(dist: &Path, url_path: &str, have_index: bool) -> SpaRoute {
    match dist_lookup(dist, url_path) {
        DistLookup::Index => {
            if have_index {
                SpaRoute::Shell
            } else {
                SpaRoute::NotFound
            }
        }
        DistLookup::File => SpaRoute::File,
        DistLookup::Missing => {
            // Client routes (no static extension, not under /assets/) still
            // get the shell. A missing stylesheet or script must not.
            if have_index && !is_static_miss_path(url_path) {
                SpaRoute::Shell
            } else {
                SpaRoute::NotFound
            }
        }
        DistLookup::Rejected => SpaRoute::NotFound,
    }
}

/// Missing files under `/assets/`, or any missing path with a static
/// extension, must 404 instead of falling through to `index.html`.
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
    matches!(
        ext.to_ascii_lowercase().as_str(),
        "js" | "mjs"
            | "css"
            | "wasm"
            | "map"
            | "svg"
            | "png"
            | "jpg"
            | "jpeg"
            | "webp"
            | "ico"
            | "woff"
            | "woff2"
            | "ttf"
            | "json"
    )
}

fn dist_lookup(dist: &Path, url_path: &str) -> DistLookup {
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
    let candidate = dist.join(rel);
    let Ok(canon) = candidate.canonicalize() else {
        return DistLookup::Missing;
    };
    let Ok(dist_canon) = dist.canonicalize() else {
        return DistLookup::Missing;
    };
    if !canon.starts_with(&dist_canon) {
        return DistLookup::Rejected;
    }
    if canon.is_file() {
        DistLookup::File
    } else {
        // A directory (for example `/assets`) is not a file. The shell covers
        // it until a later check turns missing static paths into 404s.
        DistLookup::Missing
    }
}

fn shell_response(html: String, head_only: bool) -> Response<Body> {
    let len = html.len();
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "text/html; charset=utf-8")
        .header(header::CACHE_CONTROL, HeaderValue::from_static(SHELL_CACHE))
        .header(header::CONTENT_LENGTH, len.to_string())
        .body(if head_only {
            Body::empty()
        } else {
            Body::from(html)
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
    dist_dir: PathBuf,
    index_html: Option<Arc<str>>,
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
            classify_spa_route(&self.dist_dir, &path, self.index_html.is_some())
        } else {
            SpaRoute::File
        };
        let state = self.state.clone();
        let index_html = self.index_html.clone();
        match route {
            SpaRoute::Shell => Box::pin(async move {
                let Some(template) = index_html else {
                    return Ok(plain_not_found());
                };
                let embed = load_embed_json(&state).await;
                let html = inject_settings_script(template.as_ref(), embed.as_deref());
                Ok(shell_response(html, method == Method::HEAD))
            }),
            SpaRoute::NotFound => Box::pin(async { Ok(plain_not_found()) }),
            SpaRoute::File => {
                let clone = self.files.clone();
                let mut files = std::mem::replace(&mut self.files, clone);
                Box::pin(async move {
                    let mut response = files.call(req).await?;
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
                    Ok(response.map(Body::new))
                })
            }
        }
    }
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
        for path in [
            "/",
            "/index.html",
            "/strategies/foo",
            "/admin/settings",
            "/blog/hello",
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

    #[test]
    fn inject_settings_script_sits_immediately_before_head() {
        let html = "<html><head><title>x</title></HEAD><body></body></html>";
        let out = inject_settings_script(html, Some(r#"{"a":1}"#));
        assert_eq!(
            out,
            "<html><head><title>x</title><script id=\"sc-settings\" type=\"application/json\">{\"a\":1}</script></HEAD><body></body></html>"
        );
    }

    #[test]
    fn inject_settings_script_omits_block_without_json_or_head() {
        let html = "<html><head></head><body>SPA-SHELL-MARKER</body></html>";
        assert_eq!(inject_settings_script(html, None), html);
        let no_head = "<html><body>SPA-SHELL-MARKER</body></html>";
        assert_eq!(inject_settings_script(no_head, Some("{}")), no_head);
    }
}
