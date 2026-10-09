use std::sync::Arc;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, HeroStats, MemberLeaderboardRow, Season};
use scuffed_types::api::{CreateSeasonRequest, UpdateSeasonRequest};
use scuffed_types::{HeroAgg, MemberLeaderboardRow as TypesMemberRow, resolve_hero_query};

use crate::extractors::AdminUser;
use crate::leaderboard_cache::{
    CacheError, FailKind, LeaderboardKey, LoadError, LoadedBoard, OnError, truncate_to_requested,
};
use crate::routes::audit_log::audit;
use crate::state::AppState;

fn hero_stats_to_agg(h: HeroStats) -> HeroAgg {
    let winrate = if h.matches > 0 {
        h.wins as f32 / h.matches as f32
    } else {
        0.0
    };
    HeroAgg {
        hero: h.hero,
        games: h.matches,
        wins: h.wins,
        losses: h.losses,
        draws: h.draws,
        winrate,
        avg_elims: h.avg_elims,
        avg_deaths: h.avg_deaths,
    }
}

fn map_lb_row(r: MemberLeaderboardRow) -> TypesMemberRow {
    TypesMemberRow {
        member_id: r.member_id,
        display_name: r.display_name,
        games: r.games,
        winrate: r.winrate,
        kd: r.kd,
    }
}

#[derive(Deserialize)]
pub struct HeroesQuery {
    #[serde(default = "default_top")]
    pub top: u32,
}

fn default_top() -> u32 {
    3
}

/// GET /api/public/members/:id/heroes?top=3 — public top heroes (no auth).
///
/// `top=0` returns **all** heroes (hero-stats W2 B4). Default remains 3.
/// Non-zero values are clamped to 1..=50.
pub async fn public_member_heroes(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HeroesQuery>,
) -> Result<Json<Vec<HeroAgg>>, (StatusCode, Json<ErrorResponse>)> {
    let top = if q.top == 0 { 0 } else { q.top.clamp(1, 50) };
    // Same visibility as the public profile: missing and inactive are both 404.
    let member = state
        .db
        .get_member_safe(&id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        .filter(|m| m.is_active);
    if member.is_none() {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Member not found".into(),
            }),
        ));
    }
    let heroes = state.db.top_heroes(&id, top).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    Ok(Json(heroes.into_iter().map(hero_stats_to_agg).collect()))
}

#[derive(Deserialize)]
pub struct LeaderboardQuery {
    #[serde(default = "default_metric")]
    pub metric: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Optional season id — filters aggregates to `played_at` in [starts_at, ends_at).
    pub season: Option<String>,
    /// Optional hero filter (hero-stats W3 B2). Empty/omitted = all heroes.
    /// Must match a canonical [`HEROES`] entry (case-insensitive); unknown → 400.
    pub hero: Option<String>,
}

fn default_metric() -> String {
    "winrate".into()
}

fn default_limit() -> u32 {
    25
}

/// Resolve query `hero=` to a canonical HEROES display name.
/// Empty / whitespace-only → no filter. Unknown name → Err (caller returns 400).
fn resolve_leaderboard_hero(raw: Option<&str>) -> Result<Option<&'static str>, ()> {
    resolve_hero_query(raw)
}

type ApiError = (StatusCode, Json<ErrorResponse>);

fn internal_error() -> ApiError {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal error".into(),
        }),
    )
}

fn bad_request(msg: &str) -> ApiError {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
}

fn season_not_found() -> ApiError {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: "Season not found".into(),
        }),
    )
}

/// Resolve `?season=<id>` to a `played_at` window. Omitted / blank → `None`
/// (all time). Unknown id → 404. Shared by leaderboards and every personal
/// stats endpoint so "total or per season" means the same thing everywhere.
pub async fn resolve_season_window(
    state: &AppState,
    season: Option<&str>,
) -> Result<Option<(DateTime<Utc>, DateTime<Utc>)>, ApiError> {
    let Some(sid) = season.map(str::trim).filter(|s| !s.is_empty()) else {
        return Ok(None);
    };
    let season = state
        .db
        .get_season(sid)
        .await
        .map_err(|_e| internal_error())?
        .ok_or_else(season_not_found)?;
    Ok(Some((season.starts_at, season.ends_at)))
}

/// JSON body for `GET /api/public/leaderboards`.
///
/// `rows` keeps the member fields the array body used to return.
/// `cached_at` is when this process started the query that produced `rows`
/// (RFC 3339 UTC). A cache hit repeats that timestamp.
#[derive(Serialize)]
pub struct PublicLeaderboardBody {
    pub rows: Vec<TypesMemberRow>,
    pub cached_at: DateTime<Utc>,
}

enum BoardError {
    SeasonNotFound,
    Db,
}

impl LoadError for BoardError {
    fn on_error(&self) -> OnError {
        match self {
            // A missing season is a 404. Do not substitute an older board.
            BoardError::SeasonNotFound => OnError::Surface,
            BoardError::Db => OnError::ServeStale,
        }
    }
}

/// GET /api/public/leaderboards?metric=winrate|kd|games&limit=25&season=<id>&hero=<name>
///
/// Anonymous and logged-in callers get the same rows. Inactive members are
/// omitted by the query. One grouped scan per season is cached. Metric,
/// hero, and limit are projections of that snapshot, so a different hero
/// does not start another scan. The cache key still carries a public
/// audience tag so a crew-only board cannot be stored in this slot.
pub async fn public_leaderboards(
    State(state): State<AppState>,
    Query(q): Query<LeaderboardQuery>,
) -> Result<Json<PublicLeaderboardBody>, (StatusCode, Json<ErrorResponse>)> {
    let metric = match q.metric.as_str() {
        "kd" | "games" | "winrate" => q.metric.clone(),
        _ => "winrate".to_string(),
    };
    let requested = q.limit.clamp(1, 100);

    // W3 B2: optional ?hero= → canonical HEROES name. Unknown names are 400
    // and are not cached. The hero is applied to the season snapshot.
    let hero = match resolve_leaderboard_hero(q.hero.as_deref()) {
        Ok(h) => h.map(str::to_string),
        Err(()) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Unknown hero".into(),
                }),
            ));
        }
    };
    let season_id = q
        .season
        .as_deref()
        .map(str::trim)
        .filter(|s| !s.is_empty())
        .unwrap_or("")
        .to_string();
    // One cache entry per season. Metric, hero, and limit are projections.
    let key = LeaderboardKey::public_board("scan", 0, &season_id, "");

    let load_state = state.clone();
    let load_season = season_id.clone();
    let board = state
        .leaderboard_cache
        .get_or_load(key, move || {
            let state = load_state;
            let season_id = load_season;
            async move {
                let season_window = resolve_season_window(
                    &state,
                    if season_id.is_empty() {
                        None
                    } else {
                        Some(season_id.as_str())
                    },
                )
                .await
                .map_err(|err| {
                    if err.0 == StatusCode::NOT_FOUND {
                        BoardError::SeasonNotFound
                    } else {
                        BoardError::Db
                    }
                })?;
                let snapshot = state
                    .db
                    .leaderboard_snapshot(season_window)
                    .await
                    .map_err(|_e| BoardError::Db)?;
                Ok(LoadedBoard {
                    rows: Vec::new(),
                    snapshot: Some(Arc::new(snapshot)),
                })
            }
        })
        .await
        .map_err(|err| match err {
            CacheError::Load(BoardError::SeasonNotFound)
            | CacheError::Leader(FailKind::Rejected) => season_not_found(),
            CacheError::Load(BoardError::Db) | CacheError::Leader(FailKind::Unavailable) => {
                internal_error()
            }
        })?;

    let hero_name = hero.as_deref().unwrap_or("");
    let rows = match &board.snapshot {
        Some(snapshot) => snapshot
            .project(&metric, hero_name, requested)
            .into_iter()
            .map(map_lb_row)
            .collect(),
        None => truncate_to_requested(board.rows, requested),
    };

    Ok(Json(PublicLeaderboardBody {
        rows,
        cached_at: board.cached_at,
    }))
}

#[cfg(test)]
mod resolve_hero_tests {
    use super::resolve_leaderboard_hero;

    #[test]
    fn empty_or_missing_is_no_filter() {
        assert_eq!(resolve_leaderboard_hero(None), Ok(None));
        assert_eq!(resolve_leaderboard_hero(Some("")), Ok(None));
        assert_eq!(resolve_leaderboard_hero(Some("   ")), Ok(None));
    }

    #[test]
    fn case_insensitive_canonical() {
        assert_eq!(resolve_leaderboard_hero(Some("ana")), Ok(Some("Ana")));
        assert_eq!(
            resolve_leaderboard_hero(Some("Wrecking Ball")),
            Ok(Some("Wrecking Ball"))
        );
        assert_eq!(resolve_leaderboard_hero(Some("d.va")), Ok(Some("D.Va")));
    }

    #[test]
    fn unknown_is_err() {
        assert!(resolve_leaderboard_hero(Some("NotAHero")).is_err());
    }
}

/// GET /api/public/seasons — list seasons (public).
pub async fn public_list_seasons(
    State(state): State<AppState>,
) -> Result<Json<Vec<Season>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_seasons().await.map(Json).map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })
}

/// POST /api/admin/seasons — create season (admin).
pub async fn admin_create_season(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<CreateSeasonRequest>,
) -> Result<(StatusCode, Json<Season>), (StatusCode, Json<ErrorResponse>)> {
    if body.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "name is required".into(),
            }),
        ));
    }
    if body.ends_at <= body.starts_at {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "ends_at must be after starts_at".into(),
            }),
        ));
    }
    let s = state
        .db
        .create_season(
            body.name.trim(),
            body.starts_at,
            body.ends_at,
            body.is_current,
        )
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::CreatedSeason,
        AuditTargetType::Season,
        &s.id,
        Some(s.name.as_str()),
    )
    .await;

    Ok((StatusCode::CREATED, Json(s)))
}

/// GET /api/admin/seasons — list seasons (admin; same data as public for now).
pub async fn admin_list_seasons(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<Season>>, (StatusCode, Json<ErrorResponse>)> {
    public_list_seasons(State(state)).await
}

#[derive(Serialize)]
pub struct LeaderboardPageMeta {
    pub metric: String,
    pub count: usize,
}

/// PUT /api/admin/seasons/:id — partial update (admin). The merged window
/// must stay non-empty; `is_current = true` demotes any other current season.
pub async fn admin_update_season(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateSeasonRequest>,
) -> Result<Json<Season>, ApiError> {
    let existing = state
        .db
        .get_season(&id)
        .await
        .map_err(|_e| internal_error())?
        .ok_or_else(season_not_found)?;

    let name = match body.name {
        Some(n) => {
            let n = n.trim().to_string();
            if n.is_empty() {
                return Err(bad_request("name is required"));
            }
            n
        }
        None => existing.name.clone(),
    };
    let starts_at = body.starts_at.unwrap_or(existing.starts_at);
    let ends_at = body.ends_at.unwrap_or(existing.ends_at);
    if ends_at <= starts_at {
        return Err(bad_request("ends_at must be after starts_at"));
    }
    let is_current = body.is_current.unwrap_or(existing.is_current);

    let s = state
        .db
        .update_season(&id, &name, starts_at, ends_at, is_current)
        .await
        .map_err(|e| match e {
            scuffed_db::DbError::NotFound(_) => season_not_found(),
            _ => internal_error(),
        })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::UpdatedSeason,
        AuditTargetType::Season,
        &s.id,
        Some(s.name.as_str()),
    )
    .await;

    Ok(Json(s))
}

/// DELETE /api/admin/seasons/:id — remove a season (admin). Matches are
/// untouched; the season was only a window over `played_at`.
pub async fn admin_delete_season(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
) -> Result<StatusCode, ApiError> {
    let existing = state
        .db
        .get_season(&id)
        .await
        .map_err(|_e| internal_error())?
        .ok_or_else(season_not_found)?;
    let deleted = state
        .db
        .delete_season(&id)
        .await
        .map_err(|_e| internal_error())?;
    if !deleted {
        return Err(season_not_found());
    }

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::DeletedSeason,
        AuditTargetType::Season,
        &id,
        Some(existing.name.as_str()),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

#[cfg(test)]
mod cache_http_tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use chrono::{TimeZone, Utc};
    use http_body_util::BodyExt;
    use scuffed_db::OrgRole;
    use scuffed_types::api::{StatsUploadEntry, StatsUploadRequest};
    use serde_json::Value;
    use tower::ServiceExt;

    use crate::create_router;
    use crate::test_support::test_state;

    fn with_peer(builder: axum::http::request::Builder) -> axum::http::request::Builder {
        builder
            .header("x-forwarded-for", "127.0.0.1")
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                9,
            ))))
    }

    async fn body_json(resp: axum::response::Response) -> Value {
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(Value::Null)
    }

    fn upload(session_id: &str, elims: u32, edited: bool) -> StatsUploadRequest {
        StatsUploadRequest {
            matches: vec![StatsUploadEntry {
                session_id: session_id.to_string(),
                hero: "Ana".into(),
                map_name: "Oasis".into(),
                game_mode: "control".into(),
                role: "Support".into(),
                outcome: "victory".into(),
                elims,
                deaths: 1,
                assists: 0,
                damage: 1000,
                healing: 4000,
                mitigation: 0,
                played_at: Utc.with_ymd_and_hms(2026, 7, 1, 20, 0, 0).unwrap(),
                edited,
                recognizer: scuffed_types::RECOGNIZER_OCR_V1.into(),
                suspect_fields: Vec::new(),
            }],
            deleted_sessions: vec![],
        }
    }

    async fn post_upload(app: &axum::Router, token: &str, body: &StatsUploadRequest) {
        let req = Request::builder()
            .method("POST")
            .uri("/api/stats/upload")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        let resp = app.clone().oneshot(req).await.unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
    }

    async fn get_board(app: &axum::Router, token: Option<&str>) -> Value {
        let mut builder = with_peer(
            Request::builder()
                .method("GET")
                .uri("/api/public/leaderboards?metric=games&limit=25"),
        );
        if let Some(token) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let resp = app
            .clone()
            .oneshot(builder.body(Body::empty()).unwrap())
            .await
            .unwrap();
        assert_eq!(resp.status(), StatusCode::OK);
        body_json(resp).await
    }

    async fn member_with_token(state: &crate::state::AppState, name: &str, token: &str) -> String {
        let member = state
            .db
            .create_member(&format!("user-{name}"), name, OrgRole::Member)
            .await
            .unwrap();
        state
            .db
            .create_daemon_token(&member.id, token, "tracker")
            .await
            .unwrap();
        member.id
    }

    #[tokio::test]
    async fn upload_and_edit_do_not_drop_the_cached_board() {
        let state = test_state().await;
        let token = "tracker-token-board";
        member_with_token(&state, "onboard", token).await;
        let app = create_router(state);

        post_upload(&app, token, &upload("sess-a", 1, false)).await;
        let first = get_board(&app, None).await;
        assert_eq!(first["rows"][0]["games"].as_u64(), Some(1));
        assert_eq!(first["rows"][0]["kd"].as_f64(), Some(1.0));
        let cached_at = first["cached_at"].as_str().unwrap();
        assert!(
            cached_at.contains('T') && cached_at.ends_with('Z'),
            "{cached_at}"
        );

        // Same session, corrected elims, then a second game. The cached
        // board stays until the TTL. Freshness does not depend on uploads.
        post_upload(&app, token, &upload("sess-a", 8, true)).await;
        post_upload(&app, token, &upload("sess-b", 2, false)).await;
        let still = get_board(&app, None).await;
        assert_eq!(still, first);
    }

    #[tokio::test]
    async fn anonymous_and_member_see_the_same_public_board() {
        let state = test_state().await;
        let active_id = member_with_token(&state, "onboard", "tok-onboard").await;
        let benched_id = member_with_token(&state, "benched", "tok-benched").await;
        let app = create_router(state.clone());
        post_upload(&app, "tok-onboard", &upload("sess-on", 3, false)).await;
        post_upload(&app, "tok-benched", &upload("sess-bench", 9, false)).await;
        state
            .db
            .update_member(
                &benched_id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let viewer = state
            .db
            .create_local_user("viewer", "unused-hash")
            .await
            .unwrap();
        state
            .db
            .create_session(&viewer.id, "viewer-session", 24)
            .await
            .unwrap();

        // Prime the cache as an anonymous caller, then repeat with a session.
        let anon = get_board(&app, None).await;
        let authed = get_board(&app, Some("viewer-session")).await;
        assert_eq!(
            anon, authed,
            "auth must not change the public board or its cache slot"
        );

        let ids: Vec<&str> = anon["rows"]
            .as_array()
            .unwrap()
            .iter()
            .map(|row| row["member_id"].as_str().unwrap())
            .collect();
        assert_eq!(ids, vec![active_id.as_str()]);
        assert!(!ids.contains(&benched_id.as_str()));
        let cached_at = anon["cached_at"].as_str().unwrap();
        assert!(
            cached_at.contains('T') && cached_at.ends_with('Z'),
            "{cached_at}"
        );
    }
}
