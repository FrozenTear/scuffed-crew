//! Public crawler files and cache policy for the Dioxus SPA shell.
//!
//! `/robots.txt` and `/sitemap.xml` are explicit routes so they win over the
//! `dist/` catch-all (which would otherwise return `index.html` as 200 HTML).
//! Cache headers are applied only to that catch-all, never to `/api/*` or
//! `/uploads`.

use std::collections::HashSet;
use std::future::Future;
use std::pin::Pin;
use std::task::{Context, Poll};

use axum::extract::State;
use axum::http::{HeaderValue, Request, StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, SecondsFormat, Utc};
use scuffed_db::{ForumBoard, ForumBoardNode, ForumCategoryNode, MatchType, TournamentStatus};
use tower::Service;
use tower_http::services::{ServeDir, ServeFile};

use crate::state::AppState;

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

/// `dist/` with an `index.html` fallback, plus cache headers.
///
/// A hand-rolled service (rather than `middleware::from_fn`) so the future
/// stays `Send`. Axum's function middleware around `ServeDir` does not.
pub(crate) fn spa_service(dist_dir: &std::path::Path) -> WithStaticCache<ServeDir<ServeFile>> {
    let index = dist_dir.join("index.html");
    let files = ServeDir::new(dist_dir).fallback(ServeFile::new(index));
    WithStaticCache { inner: files }
}

#[derive(Clone)]
pub(crate) struct WithStaticCache<S> {
    inner: S,
}

impl<S, ReqBody, ResBody> Service<Request<ReqBody>> for WithStaticCache<S>
where
    S: Service<Request<ReqBody>, Response = axum::http::Response<ResBody>> + Clone + Send + 'static,
    S::Error: Send,
    S::Future: Send + 'static,
    ReqBody: Send + 'static,
    ResBody: Send + 'static,
{
    type Response = axum::http::Response<ResBody>;
    type Error = S::Error;
    type Future = Pin<Box<dyn Future<Output = Result<Self::Response, Self::Error>> + Send>>;

    fn poll_ready(&mut self, cx: &mut Context<'_>) -> Poll<Result<(), Self::Error>> {
        self.inner.poll_ready(cx)
    }

    fn call(&mut self, req: Request<ReqBody>) -> Self::Future {
        let path = req.uri().path().to_owned();
        let fut = self.inner.call(req);
        Box::pin(async move {
            let mut response = fut.await?;
            let content_type = response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|value| value.to_str().ok())
                .map(str::to_owned);
            let value = cache_control_value(&path, content_type.as_deref());
            response
                .headers_mut()
                .insert(header::CACHE_CONTROL, HeaderValue::from_static(value));
            Ok(response)
        })
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
        // A missing hashed file that falls through to index.html must not be frozen.
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
}
