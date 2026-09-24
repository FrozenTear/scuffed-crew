use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::StatusCode,
    middleware::{self, Next},
    response::Response,
    routing::{get, put},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_auth::server::{AuthUser, HasAuth};
use scuffed_db::{AuditAction, AuditTargetType};
use scuffed_site_server::extractors::OfficerUser;
use scuffed_site_server::routes::audit_log::audit;
use scuffed_site_server::state::AppState;
use scuffed_types::api::ApiSuccess;
use scuffed_types::patch_notes::{CreatePatchNoteRequest, PatchNote, UpdatePatchNoteRequest};
use scuffed_types::strategy::{
    GameMode, Strategy, StrategyElement, StrategySummary, TimelinePhase, Visibility,
};

/// Strategy API routes — merged into the unified server.
///
/// Patch Notes stay ungated. Strategy CRUD/helpers are 404 when
/// `SiteSettings.strategies_enabled` is false.
pub fn strategy_routes(state: AppState) -> Router {
    let gated = Router::new()
        .route(
            "/api/strategy/strategies",
            get(list_strategies).post(create_strategy),
        )
        .route("/api/strategy/strategies/mine", get(list_my_strategies))
        .route(
            "/api/strategy/strategies/{id}",
            get(get_strategy)
                .put(update_strategy)
                .delete(delete_strategy),
        )
        .route("/api/strategy/heroes", get(list_heroes))
        .route("/api/strategy/meta", get(get_meta))
        .route_layer(middleware::from_fn_with_state(
            state.clone(),
            require_strategies_enabled,
        ))
        .with_state(state.clone());

    let patch_notes = Router::new()
        .route(
            "/api/strategy/patch-notes",
            get(list_patch_notes).post(create_patch_note),
        )
        .route(
            "/api/strategy/patch-notes/{version}",
            put(update_patch_note).delete(delete_patch_note),
        )
        .with_state(state);

    gated.merge(patch_notes)
}

async fn require_strategies_enabled(
    State(state): State<AppState>,
    request: Request,
    next: Next,
) -> Result<Response, StatusCode> {
    ensure_strategies_enabled(&state).await?;
    Ok(next.run(request).await)
}

/// Shared by strategy REST middleware and `/api/strategy/ws`.
///
/// Fail closed: a settings read error rejects the request. A disabled flag is
/// 404, matching the REST routes (the feature is absent, not forbidden).
pub(crate) async fn ensure_strategies_enabled(state: &AppState) -> Result<(), StatusCode> {
    let enabled = match state.db.get_settings().await {
        Ok(settings) => Ok(settings.strategies_enabled),
        Err(e) => {
            tracing::error!("strategies gate: failed to load settings: {e}");
            Err(())
        }
    };
    strategies_gate_status(enabled)
}

pub(crate) fn strategies_gate_status(enabled: Result<bool, ()>) -> Result<(), StatusCode> {
    match enabled {
        Ok(true) => Ok(()),
        Ok(false) => Err(StatusCode::NOT_FOUND),
        Err(()) => Err(StatusCode::INTERNAL_SERVER_ERROR),
    }
}

// =============================================================================
// Request/Response types
// =============================================================================

#[derive(Debug, Deserialize)]
struct CreateStrategyRequest {
    name: String,
    #[serde(default)]
    description: Option<String>,
    map_id: String,
    #[serde(default)]
    sub_map_id: Option<String>,
    game_mode: String,
    #[serde(default)]
    team_id: Option<String>,
    #[serde(default = "default_visibility")]
    visibility: String,
}

fn default_visibility() -> String {
    "private".to_string()
}

#[derive(Debug, Deserialize)]
struct UpdateStrategyRequest {
    #[serde(default)]
    name: Option<String>,
    #[serde(default)]
    description: Option<Option<String>>,
    #[serde(default)]
    visibility: Option<String>,
    #[serde(default)]
    elements: Option<Vec<StrategyElement>>,
    #[serde(default)]
    phases: Option<Vec<TimelinePhase>>,
}

#[derive(Debug, Deserialize)]
struct ListStrategiesQuery {
    #[serde(default)]
    search: Option<String>,
    #[serde(default)]
    game_mode: Option<String>,
    #[serde(default = "default_limit")]
    limit: u32,
    #[serde(default)]
    offset: u32,
}

fn default_limit() -> u32 {
    20
}

#[derive(Debug, Serialize)]
struct StrategyListResponse {
    data: Vec<StrategySummary>,
    total: u64,
}

// =============================================================================
// Handlers
// =============================================================================

/// GET /api/strategy/strategies — list public strategies
async fn list_strategies(
    State(state): State<AppState>,
    Query(params): Query<ListStrategiesQuery>,
) -> Result<Json<Value>, StatusCode> {
    let limit = params.limit.min(100);

    let (data, total) = state
        .db
        .get_public_strategies(
            params.search.as_deref(),
            params.game_mode.as_deref(),
            limit,
            params.offset,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to list strategies: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(
        serde_json::to_value(StrategyListResponse { data, total }).unwrap(),
    ))
}

/// POST /api/strategy/strategies — create a new strategy
async fn create_strategy(
    State(state): State<AppState>,
    user: AuthUser<AppState>,
    Json(body): Json<CreateStrategyRequest>,
) -> Result<(StatusCode, Json<Strategy>), (StatusCode, Json<Value>)> {
    // Validate
    if body.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Name is required" })),
        ));
    }
    if body.map_id.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(json!({ "error": "Map ID is required" })),
        ));
    }

    let game_mode = parse_game_mode_str(&body.game_mode);
    let visibility = parse_visibility_str(&body.visibility);

    let strategy = state
        .db
        .create_strategy(
            body.name.trim(),
            body.description.as_deref(),
            &body.map_id,
            body.sub_map_id.as_deref(),
            game_mode,
            &user.id,
            body.team_id.as_deref(),
            visibility,
        )
        .await
        .map_err(|e| {
            tracing::error!("Failed to create strategy: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to create strategy" })),
            )
        })?;

    Ok((StatusCode::CREATED, Json(strategy)))
}

/// GET /api/strategy/strategies/mine — list current user's strategies
async fn list_my_strategies(
    State(state): State<AppState>,
    user: AuthUser<AppState>,
) -> Result<Json<Value>, StatusCode> {
    let data = state.db.get_user_strategies(&user.id).await.map_err(|e| {
        tracing::error!("Failed to list user strategies: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;

    Ok(Json(json!({ "data": data })))
}

/// GET /api/strategy/strategies/:id — get a single strategy
async fn get_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    jar: CookieJar,
) -> Result<Json<Strategy>, (StatusCode, Json<Value>)> {
    // Try to extract user optionally (don't require auth for public/unlisted)
    let user = try_get_user(&state, &jar).await;
    let user_id = user.as_ref().map(|u| u.id.as_str());

    let can_access = state
        .db
        .can_access_strategy(&id, user_id)
        .await
        .map_err(|e| {
            tracing::error!("Access check failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal error" })),
            )
        })?;

    if !can_access {
        return Err((
            StatusCode::NOT_FOUND,
            Json(json!({ "error": "Strategy not found" })),
        ));
    }

    let strategy = state
        .db
        .get_strategy(&id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to get strategy: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal error" })),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Strategy not found" })),
            )
        })?;

    Ok(Json(strategy))
}

/// PUT /api/strategy/strategies/:id — update a strategy (owner only)
async fn update_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: AuthUser<AppState>,
    Json(body): Json<UpdateStrategyRequest>,
) -> Result<Json<Strategy>, (StatusCode, Json<Value>)> {
    // Check ownership
    let can_edit = state
        .db
        .can_edit_strategy(&id, &user.id)
        .await
        .map_err(|e| {
            tracing::error!("Edit check failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal error" })),
            )
        })?;

    if !can_edit {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Permission denied" })),
        ));
    }

    // Save elements/phases if provided (bulk save from editor)
    if body.elements.is_some() || body.phases.is_some() {
        // Need current data to fill in missing half
        let current = state.db.get_strategy(&id).await.map_err(|e| {
            tracing::error!("Failed to get strategy for update: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal error" })),
            )
        })?;
        let current = current.ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(json!({ "error": "Strategy not found" })),
            )
        })?;

        let elements = body.elements.as_ref().unwrap_or(&current.elements);
        let phases = body.phases.as_ref().unwrap_or(&current.phases);

        state
            .db
            .save_full_strategy(&id, elements, phases)
            .await
            .map_err(|e| {
                tracing::error!("Failed to save strategy content: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "Failed to save strategy content" })),
                )
            })?;
    }

    // Update metadata fields if provided
    let visibility = body.visibility.as_ref().map(|v| parse_visibility_str(v));
    let description = body.description.as_ref().map(|d| d.as_deref());

    let strategy = state
        .db
        .update_strategy(&id, body.name.as_deref(), description, visibility)
        .await
        .map_err(|e| {
            tracing::error!("Failed to update strategy: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to update strategy" })),
            )
        })?;

    Ok(Json(strategy))
}

/// DELETE /api/strategy/strategies/:id — delete a strategy (owner only)
async fn delete_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    user: AuthUser<AppState>,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let can_edit = state
        .db
        .can_edit_strategy(&id, &user.id)
        .await
        .map_err(|e| {
            tracing::error!("Edit check failed: {e}");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Internal error" })),
            )
        })?;

    if !can_edit {
        return Err((
            StatusCode::FORBIDDEN,
            Json(json!({ "error": "Permission denied" })),
        ));
    }

    state.db.delete_strategy(&id).await.map_err(|e| {
        tracing::error!("Failed to delete strategy: {e}");
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(json!({ "error": "Failed to delete strategy" })),
        )
    })?;

    Ok(StatusCode::NO_CONTENT)
}

// =============================================================================
// Hero / meta endpoint
// =============================================================================

#[derive(Debug, Serialize)]
struct MetaResponse {
    updated: String,
    source: String,
    heroes: Vec<HeroMeta>,
    /// Personal stats for the authenticated org member, when available.
    /// `None` for anonymous requests or non-members.
    #[serde(skip_serializing_if = "Option::is_none")]
    personal: Option<PersonalMeta>,
}

#[derive(Debug, Serialize)]
struct HeroMeta {
    id: String,
    name: String,
    role: String,
    portrait_url: String,
    pickrate: f64,
    winrate: f64,
}

#[derive(Debug, Serialize)]
struct PersonalMeta {
    member_id: String,
    heroes: Vec<HeroPersonalEntry>,
    maps: Vec<MapPersonalEntry>,
}

#[derive(Debug, Serialize)]
struct HeroPersonalEntry {
    hero: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
    winrate: f64,
}

#[derive(Debug, Serialize)]
struct MapPersonalEntry {
    map_name: String,
    matches: u32,
    wins: u32,
    losses: u32,
    draws: u32,
    winrate: f64,
}

fn winrate_pct(wins: u32, matches: u32) -> f64 {
    if matches == 0 {
        0.0
    } else {
        (wins as f64 / matches as f64) * 100.0
    }
}

async fn list_heroes() -> Json<Value> {
    Json(json!({ "data": [] }))
}

/// GET /api/strategy/patch-notes — public list for the Site Patch Notes page.
///
/// Auth matches other public strategy GETs (`/heroes`, `/strategies`): no login.
/// Envelope is [`ApiSuccess`] (`{ "data": [...] }`). Field names are frozen in
/// `scuffed_types::patch_notes` so Site stays valid.
async fn list_patch_notes(
    State(state): State<AppState>,
) -> Result<Json<ApiSuccess<Vec<PatchNote>>>, StatusCode> {
    let data = state.db.list_patch_notes().await.map_err(|e| {
        tracing::error!("Failed to list patch notes: {e}");
        StatusCode::INTERNAL_SERVER_ERROR
    })?;
    Ok(Json(ApiSuccess { data }))
}

fn patch_note_db_err(e: scuffed_db::DbError) -> (StatusCode, Json<ErrorResponse>) {
    let status = match &e {
        scuffed_db::DbError::NotFound(_) => StatusCode::NOT_FOUND,
        scuffed_db::DbError::Conflict(_) => StatusCode::CONFLICT,
        _ => StatusCode::INTERNAL_SERVER_ERROR,
    };
    let error = match &e {
        scuffed_db::DbError::Conflict(msg) => msg.clone(),
        scuffed_db::DbError::NotFound(_) => "Patch note not found".into(),
        _ => "Internal error".into(),
    };
    (status, Json(ErrorResponse { error }))
}

fn require_patch_note_fields(
    version: &str,
    date: &str,
    url: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if version.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "version is required".into(),
            }),
        ));
    }
    if date.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "date is required".into(),
            }),
        ));
    }
    if url.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "url is required".into(),
            }),
        ));
    }
    Ok(())
}

/// POST /api/strategy/patch-notes — create a patch note (officer+)
async fn create_patch_note(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(mut body): Json<CreatePatchNoteRequest>,
) -> Result<(StatusCode, Json<PatchNote>), (StatusCode, Json<ErrorResponse>)> {
    body.version = body.version.trim().to_string();
    body.date = body.date.trim().to_string();
    body.url = body.url.trim().to_string();
    require_patch_note_fields(&body.version, &body.date, &body.url)?;

    let note = state
        .db
        .create_patch_note(&body)
        .await
        .map_err(patch_note_db_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::CreatedPatchNote,
        AuditTargetType::PatchNote,
        &note.version,
        Some(&format!("Created patch note {}", note.version)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(note)))
}

/// PUT /api/strategy/patch-notes/:version — update a patch note (officer+)
async fn update_patch_note(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(version): Path<String>,
    Json(body): Json<UpdatePatchNoteRequest>,
) -> Result<Json<PatchNote>, (StatusCode, Json<ErrorResponse>)> {
    if let Some(date) = body.date.as_deref()
        && date.trim().is_empty()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "date is required".into(),
            }),
        ));
    }
    if let Some(url) = body.url.as_deref()
        && url.trim().is_empty()
    {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "url is required".into(),
            }),
        ));
    }

    let note = state
        .db
        .update_patch_note(&version, &body)
        .await
        .map_err(patch_note_db_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedPatchNote,
        AuditTargetType::PatchNote,
        &note.version,
        None,
    )
    .await;

    Ok(Json(note))
}

/// DELETE /api/strategy/patch-notes/:version — delete a patch note (officer+)
async fn delete_patch_note(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(version): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .delete_patch_note(&version)
        .await
        .map_err(patch_note_db_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::DeletedPatchNote,
        AuditTargetType::PatchNote,
        &version,
        Some(&format!("Deleted patch note {version}")),
    )
    .await;

    Ok(StatusCode::OK)
}

/// GET /api/strategy/meta — global meta data + personal winrates per hero/map.
///
/// Anonymous: returns the global stub only. Authed org members: also returns
/// `personal.heroes` and `personal.maps` from their stat-tracker uploads.
async fn get_meta(State(state): State<AppState>, jar: CookieJar) -> Json<MetaResponse> {
    let mut response = MetaResponse {
        updated: chrono::Utc::now().to_rfc3339(),
        source: "Scuffed Crew".into(),
        heroes: Vec::new(),
        personal: None,
    };

    let Some(user) = try_get_user(&state, &jar).await else {
        return Json(response);
    };

    let Ok(Some(member)) = state.db.get_member_by_user(&user.id).await else {
        return Json(response);
    };

    if !member.is_active {
        return Json(response);
    }

    let hero_rows = state
        .db
        .get_hero_stats(&member.id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to load personal hero stats: {e}");
            Vec::new()
        });

    let map_rows = state
        .db
        .get_map_stats(&member.id)
        .await
        .unwrap_or_else(|e| {
            tracing::error!("Failed to load personal map stats: {e}");
            Vec::new()
        });

    let heroes = hero_rows
        .into_iter()
        .map(|h| HeroPersonalEntry {
            winrate: winrate_pct(h.wins, h.matches),
            hero: h.hero,
            matches: h.matches,
            wins: h.wins,
            losses: h.losses,
            draws: h.draws,
        })
        .collect();

    let maps = map_rows
        .into_iter()
        .map(|m| MapPersonalEntry {
            winrate: winrate_pct(m.wins, m.matches),
            map_name: m.map_name,
            matches: m.matches,
            wins: m.wins,
            losses: m.losses,
            draws: m.draws,
        })
        .collect();

    response.personal = Some(PersonalMeta {
        member_id: member.id,
        heroes,
        maps,
    });

    Json(response)
}

// =============================================================================
// Helpers
// =============================================================================

fn parse_game_mode_str(s: &str) -> GameMode {
    match s {
        "escort" => GameMode::Escort,
        "hybrid" => GameMode::Hybrid,
        "control" => GameMode::Control,
        "push" => GameMode::Push,
        "flashpoint" => GameMode::Flashpoint,
        "clash" => GameMode::Clash,
        "payload_race" => GameMode::PayloadRace,
        "assault" => GameMode::Assault,
        // Also accept the display/competitive format
        "competitive" => GameMode::Control,
        _ => GameMode::Control,
    }
}

fn parse_visibility_str(s: &str) -> Visibility {
    match s {
        "private" => Visibility::Private,
        "unlisted" => Visibility::Unlisted,
        "public" => Visibility::Public,
        _ => Visibility::Private,
    }
}

/// Try to extract a user from the request without requiring auth.
async fn try_get_user(state: &AppState, jar: &CookieJar) -> Option<scuffed_auth::User> {
    let config = state.session_config();
    let token = jar.get(&config.cookie_name)?.value().to_string();
    state.get_session_user(&token).await.ok().flatten()
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::Router;
    use axum::body::Body;
    use axum::http::{Method, Request, StatusCode, header};
    use http_body_util::BodyExt;
    use scuffed_auth::SessionConfig;
    use scuffed_db::Database;
    use scuffed_db::migrations::run_migrations;
    use scuffed_db::types::OrgRole;
    use scuffed_site_server::state::OAuthConfig;
    use serde_json::json;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

    const OFFICER_TOKEN: &str = "test-officer-token";
    const MEMBER_TOKEN: &str = "test-member-token";

    async fn test_state() -> AppState {
        let db = Database::connect_memory().await.expect("mem db");
        run_migrations(&db.client).await.expect("migrations");
        AppState {
            db: Arc::new(db),
            session_config: SessionConfig::default(),
            oauth_config: OAuthConfig {
                discord_client_id: String::new(),
                discord_client_secret: String::new(),
                google_client_id: String::new(),
                google_client_secret: String::new(),
                redirect_base_url: "http://localhost:3000".into(),
                allowed_origins: vec!["http://localhost:3000".into()],
            },
            upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
            notifier: None,
            nostr_challenge_key: [0u8; 32],
            consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(
            ),
            nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
            crypto: None,
            relay_url: None,
            dm_events: None,
            nip05_domain: None,
            nip05_republish_enabled: false,
        }
    }

    async fn seed_role(state: &AppState, username: &str, role: OrgRole, token: &str) {
        let user = state
            .db
            .create_local_user(username, "unused-hash")
            .await
            .expect("user");
        state
            .db
            .create_member(&user.id, username, role)
            .await
            .expect("member");
        state
            .db
            .create_session(&user.id, token, 24)
            .await
            .expect("session");
    }

    async fn call_json(
        app: Router,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(tok) = token {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {tok}"));
        }
        if body.is_some() {
            builder = builder.header(header::CONTENT_TYPE, "application/json");
        }
        let req_body = body
            .map(|v| Body::from(serde_json::to_vec(&v).unwrap()))
            .unwrap_or_else(Body::empty);
        let resp = app.oneshot(builder.body(req_body).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value = if bytes.is_empty() {
            serde_json::Value::Null
        } else {
            serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
        };
        (status, value)
    }

    async fn get_json(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
        call_json(app, Method::GET, path, None, None).await
    }

    fn sample_create(version: &str) -> serde_json::Value {
        json!({
            "version": version,
            "date": "2026-09-14",
            "title": "Officer write",
            "url": "https://example.test/notes",
            "hero_updates": [],
            "sections": [{ "category": "General", "items": ["Seeded by test"] }]
        })
    }

    #[tokio::test]
    async fn patch_notes_is_public_data_envelope() {
        let app = strategy_routes(test_state().await);
        let (status, body) = get_json(app, "/api/strategy/patch-notes").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let data = body
            .get("data")
            .and_then(|v| v.as_array())
            .expect("{ data: [...] } envelope");
        assert!(!data.is_empty());
        let first = &data[0];
        assert!(first.get("version").and_then(|v| v.as_str()).is_some());
        assert!(first.get("date").and_then(|v| v.as_str()).is_some());
        assert!(first.get("url").and_then(|v| v.as_str()).is_some());
        assert!(
            first
                .get("hero_updates")
                .and_then(|v| v.as_array())
                .is_some()
        );
        assert!(first.get("sections").and_then(|v| v.as_array()).is_some());
        assert!(
            body.get("strategies").is_none(),
            "must not use a Browse-style strategies key"
        );
        assert!(
            !first
                .as_object()
                .expect("object")
                .contains_key("created_at"),
            "public PatchNote shape must not leak DB timestamps"
        );
    }

    #[tokio::test]
    async fn patch_notes_root_is_object_not_bare_array() {
        let app = strategy_routes(test_state().await);
        let (status, body) = get_json(app, "/api/strategy/patch-notes").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.is_object(),
            "Site unwraps {{ data }}; a bare array would break Browse-style clients"
        );
    }

    #[tokio::test]
    async fn patch_notes_write_requires_officer() {
        let state = test_state().await;
        seed_role(&state, "officer", OrgRole::Officer, OFFICER_TOKEN).await;
        seed_role(&state, "member", OrgRole::Member, MEMBER_TOKEN).await;

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/patch-notes",
            None,
            Some(sample_create("3.0.0")),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(MEMBER_TOKEN),
            Some(sample_create("3.0.0")),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::PUT,
            "/api/strategy/patch-notes/2.18.1",
            Some(MEMBER_TOKEN),
            Some(json!({ "title": "nope" })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::DELETE,
            "/api/strategy/patch-notes/2.18.1",
            Some(MEMBER_TOKEN),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = call_json(
            strategy_routes(state),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(sample_create("3.0.0")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["version"], "3.0.0");
    }

    #[tokio::test]
    async fn patch_notes_duplicate_version_is_conflict() {
        let state = test_state().await;
        seed_role(&state, "officer", OrgRole::Officer, OFFICER_TOKEN).await;

        let (status, body) = call_json(
            strategy_routes(state),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(sample_create("2.18.1")),
        )
        .await;
        assert_eq!(status, StatusCode::CONFLICT, "{body}");
    }

    #[tokio::test]
    async fn officer_can_update_and_delete_patch_note() {
        let state = test_state().await;
        seed_role(&state, "officer", OrgRole::Officer, OFFICER_TOKEN).await;

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(sample_create("4.0.0")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::PUT,
            "/api/strategy/patch-notes/4.0.0",
            Some(OFFICER_TOKEN),
            Some(json!({ "title": "Renamed by officer" })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["title"], "Renamed by officer");

        let (status, _body) = call_json(
            strategy_routes(state.clone()),
            Method::DELETE,
            "/api/strategy/patch-notes/4.0.0",
            Some(OFFICER_TOKEN),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, body) = get_json(strategy_routes(state), "/api/strategy/patch-notes").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let data = body["data"].as_array().expect("data");
        assert!(data.iter().all(|n| n["version"] != "4.0.0"));
    }

    async fn set_strategies_enabled(state: &AppState, enabled: bool) {
        state
            .db
            .update_settings(
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(enabled),
                None,
            )
            .await
            .expect("update strategies_enabled");
    }

    fn sample_strategy() -> serde_json::Value {
        json!({
            "name": "Test strat",
            "map_id": "kings-row",
            "game_mode": "hybrid",
            "visibility": "public"
        })
    }

    #[tokio::test]
    async fn strategies_enabled_default_true_keeps_strategy_routes() {
        let state = test_state().await;
        seed_role(&state, "member", OrgRole::Member, MEMBER_TOKEN).await;
        assert!(
            state
                .db
                .get_settings()
                .await
                .expect("settings")
                .strategies_enabled
        );

        let (status, body) =
            get_json(strategy_routes(state.clone()), "/api/strategy/strategies").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(body.get("data").and_then(|v| v.as_array()).is_some());

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some(MEMBER_TOKEN),
            Some(sample_strategy()),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");

        let (status, body) = get_json(strategy_routes(state), "/api/strategy/heroes").await;
        assert_eq!(status, StatusCode::OK, "{body}");
    }

    #[tokio::test]
    async fn strategies_disabled_returns_404_but_patch_notes_stay_public() {
        let state = test_state().await;
        seed_role(&state, "member", OrgRole::Member, MEMBER_TOKEN).await;
        seed_role(&state, "officer", OrgRole::Officer, OFFICER_TOKEN).await;
        set_strategies_enabled(&state, false).await;

        let (status, body) =
            get_json(strategy_routes(state.clone()), "/api/strategy/strategies").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some(MEMBER_TOKEN),
            Some(sample_strategy()),
        )
        .await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        let (status, body) = get_json(strategy_routes(state.clone()), "/api/strategy/heroes").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        let (status, body) = get_json(strategy_routes(state.clone()), "/api/strategy/meta").await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");

        let (status, body) =
            get_json(strategy_routes(state.clone()), "/api/strategy/patch-notes").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let data = body
            .get("data")
            .and_then(|v| v.as_array())
            .expect("{ data: [...] } envelope");
        assert!(!data.is_empty());

        let (status, body) = call_json(
            strategy_routes(state),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(sample_create("5.0.0")),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["version"], "5.0.0");
    }
}
