use axum::{
    Json, Router,
    extract::{Path, Query, Request, State},
    http::{HeaderMap, StatusCode},
    middleware::{self, Next},
    response::Response,
    routing::{get, put},
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use super::strategy_heroes::list_heroes;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType};
use scuffed_site_server::apply_api_method_not_allowed;
use scuffed_site_server::extractors::{OfficerUser, OrgMember};
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
    let gated = apply_api_method_not_allowed(
        Router::new()
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
            .route("/api/strategy/meta", get(get_meta)),
    )
    .route_layer(middleware::from_fn_with_state(
        state.clone(),
        require_strategies_enabled,
    ))
    .with_state(state.clone());

    let patch_notes = apply_api_method_not_allowed(
        Router::new()
            .route(
                "/api/strategy/patch-notes",
                get(list_patch_notes).post(create_patch_note),
            )
            .route(
                "/api/strategy/patch-notes/{version}",
                put(update_patch_note).delete(delete_patch_note),
            ),
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
    /// Drawing made before the first save. Same caps as update.
    #[serde(default)]
    elements: Option<Vec<StrategyElement>>,
    #[serde(default)]
    phases: Option<Vec<TimelinePhase>>,
}

/// Longest strategy name, in characters.
pub(crate) const MAX_STRATEGY_NAME_CHARS: usize = 100;
/// Longest strategy description, in characters.
pub(crate) const MAX_STRATEGY_DESCRIPTION_CHARS: usize = 2_000;
/// Most elements one strategy may hold (REST saves and WebSocket adds).
pub(crate) const MAX_STRATEGY_ELEMENTS: usize = 1_000;
/// Most timeline phases one strategy may hold (REST saves and WebSocket adds).
pub(crate) const MAX_STRATEGY_PHASES: usize = 50;

fn bad_request(msg: impl Into<String>) -> (StatusCode, Json<Value>) {
    (
        StatusCode::BAD_REQUEST,
        Json(json!({ "error": msg.into() })),
    )
}

/// Trimmed name, or 400 when it is empty or too long.
fn validate_strategy_name(name: &str) -> Result<&str, (StatusCode, Json<Value>)> {
    let name = name.trim();
    if name.is_empty() {
        return Err(bad_request("Name is required"));
    }
    if name.chars().count() > MAX_STRATEGY_NAME_CHARS {
        return Err(bad_request(format!(
            "Name must be at most {MAX_STRATEGY_NAME_CHARS} characters"
        )));
    }
    Ok(name)
}

fn validate_strategy_description(
    description: Option<&str>,
) -> Result<(), (StatusCode, Json<Value>)> {
    if description.is_some_and(|d| d.chars().count() > MAX_STRATEGY_DESCRIPTION_CHARS) {
        return Err(bad_request(format!(
            "Description must be at most {MAX_STRATEGY_DESCRIPTION_CHARS} characters"
        )));
    }
    Ok(())
}

fn validate_strategy_content(
    elements: Option<&[StrategyElement]>,
    phases: Option<&[TimelinePhase]>,
) -> Result<(), (StatusCode, Json<Value>)> {
    if elements.is_some_and(|e| e.len() > MAX_STRATEGY_ELEMENTS) {
        return Err(bad_request(format!(
            "A strategy can hold at most {MAX_STRATEGY_ELEMENTS} elements"
        )));
    }
    if phases.is_some_and(|p| p.len() > MAX_STRATEGY_PHASES) {
        return Err(bad_request(format!(
            "A strategy can hold at most {MAX_STRATEGY_PHASES} phases"
        )));
    }
    Ok(())
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

/// POST /api/strategy/strategies: create a new strategy (org members only).
///
/// The editor sends its drawing (`elements`, `phases`) with the first save, so
/// a new strategy is not created empty.
async fn create_strategy(
    State(state): State<AppState>,
    caller: OrgMember,
    Json(body): Json<CreateStrategyRequest>,
) -> Result<(StatusCode, Json<Strategy>), (StatusCode, Json<Value>)> {
    let name = validate_strategy_name(&body.name)?;
    if body.map_id.trim().is_empty() {
        return Err(bad_request("Map ID is required"));
    }
    validate_strategy_description(body.description.as_deref())?;
    validate_strategy_content(body.elements.as_deref(), body.phases.as_deref())?;

    let team_id = body
        .team_id
        .as_deref()
        .map(str::trim)
        .filter(|t| !t.is_empty());
    if let Some(team_id) = team_id {
        let on_team = state
            .db
            .is_on_team_roster(&caller.member.id, team_id)
            .await
            .map_err(|e| {
                tracing::error!("Team roster check failed: {e}");
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(json!({ "error": "Internal error" })),
                )
            })?;
        if !on_team {
            return Err((
                StatusCode::FORBIDDEN,
                Json(json!({ "error": "You are not on that team" })),
            ));
        }
    }

    let game_mode = parse_game_mode_str(&body.game_mode);
    let visibility = parse_visibility_str(&body.visibility);

    let mut strategy = state
        .db
        .create_strategy(
            name,
            body.description.as_deref(),
            &body.map_id,
            body.sub_map_id.as_deref(),
            game_mode,
            &caller.user.id,
            team_id,
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

    let elements = body.elements.filter(|e| !e.is_empty());
    let phases = body.phases.filter(|p| !p.is_empty());
    if elements.is_some() || phases.is_some() {
        let elements = elements.unwrap_or_else(|| strategy.elements.clone());
        let phases = phases.unwrap_or_else(|| strategy.phases.clone());
        if let Err(e) = state
            .db
            .save_full_strategy(&strategy.id, &elements, &phases)
            .await
        {
            tracing::error!("Failed to save new strategy content: {e}");
            // Do not leave an empty strategy behind after reporting a failure.
            if let Err(e) = state.db.delete_strategy(&strategy.id).await {
                tracing::error!("Failed to remove half-created strategy: {e}");
            }
            return Err((
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(json!({ "error": "Failed to save strategy content" })),
            ));
        }
        strategy.elements = elements;
        strategy.phases = phases;
    }

    Ok((StatusCode::CREATED, Json(strategy)))
}

/// GET /api/strategy/strategies/mine: list the caller's strategies (org members only).
async fn list_my_strategies(
    State(state): State<AppState>,
    caller: OrgMember,
) -> Result<Json<Value>, StatusCode> {
    let data = state
        .db
        .get_user_strategies(&caller.user.id)
        .await
        .map_err(|e| {
            tracing::error!("Failed to list user strategies: {e}");
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

    Ok(Json(json!({ "data": data })))
}

/// GET /api/strategy/strategies/:id: get a single strategy.
///
/// Public and unlisted strategies are readable without a session. A private
/// strategy is readable by its owner. The owner is resolved the same way as
/// the rest of the API: `Authorization: Bearer` first, then the session cookie.
async fn get_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Result<Json<Strategy>, (StatusCode, Json<Value>)> {
    // Optional: public and unlisted strategies do not require a session.
    let user = try_get_user(&state, &headers, &jar).await;
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

/// PUT /api/strategy/strategies/:id: update a strategy (owner, who must be an org member)
async fn update_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: OrgMember,
    Json(body): Json<UpdateStrategyRequest>,
) -> Result<Json<Strategy>, (StatusCode, Json<Value>)> {
    let name = body
        .name
        .as_deref()
        .map(validate_strategy_name)
        .transpose()?;
    validate_strategy_description(body.description.as_ref().and_then(|d| d.as_deref()))?;
    validate_strategy_content(body.elements.as_deref(), body.phases.as_deref())?;

    // Check ownership
    let can_edit = state
        .db
        .can_edit_strategy(&id, &caller.user.id)
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
        .update_strategy(&id, name, description, visibility)
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

/// DELETE /api/strategy/strategies/:id: delete a strategy (owner, who must be an org member)
async fn delete_strategy(
    State(state): State<AppState>,
    Path(id): Path<String>,
    caller: OrgMember,
) -> Result<StatusCode, (StatusCode, Json<Value>)> {
    let can_edit = state
        .db
        .can_edit_strategy(&id, &caller.user.id)
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

const PATCH_NOTE_URL_ERROR: &str = "url must be an absolute http or https URL";

/// True when `url` is an absolute `http` or `https` URL with a host.
///
/// Callers trim first. `javascript:`, `data:`, protocol-relative, and
/// scheme-without-authority forms (`http:javascript:...`) are rejected.
/// Seeded Blizzard notes use `https://overwatch.blizzard.com/...`.
fn absolute_http_url(url: &str) -> bool {
    if url.is_empty()
        || url
            .chars()
            .any(|c| c.is_whitespace() || c.is_control() || c == '\\')
    {
        return false;
    }
    let has_authority = starts_with_ignore_ascii_case(url, "https://")
        || starts_with_ignore_ascii_case(url, "http://");
    if !has_authority {
        return false;
    }
    let Ok(parsed) = url::Url::parse(url) else {
        return false;
    };
    matches!(parsed.scheme(), "http" | "https")
        && parsed.host_str().is_some_and(|host| !host.is_empty())
}

fn starts_with_ignore_ascii_case(value: &str, prefix: &str) -> bool {
    let bytes = value.as_bytes();
    let prefix = prefix.as_bytes();
    bytes.len() >= prefix.len() && bytes[..prefix.len()].eq_ignore_ascii_case(prefix)
}

fn patch_note_bad_request(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse { error: msg.into() }),
    )
}

fn require_patch_note_fields(
    version: &str,
    date: &str,
    url: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    if version.trim().is_empty() {
        return Err(patch_note_bad_request("version is required"));
    }
    if date.trim().is_empty() {
        return Err(patch_note_bad_request("date is required"));
    }
    if url.trim().is_empty() {
        return Err(patch_note_bad_request("url is required"));
    }
    if !absolute_http_url(url) {
        return Err(patch_note_bad_request(PATCH_NOTE_URL_ERROR));
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
    Json(mut body): Json<UpdatePatchNoteRequest>,
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
    if let Some(url) = body.url.as_mut() {
        *url = url.trim().to_string();
        if url.is_empty() {
            return Err(patch_note_bad_request("url is required"));
        }
        if !absolute_http_url(url) {
            return Err(patch_note_bad_request(PATCH_NOTE_URL_ERROR));
        }
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
/// Anonymous: returns the global stub only. Authed org members (session cookie
/// or `Authorization: Bearer`): also returns `personal.heroes` and
/// `personal.maps` from their stat-tracker uploads.
async fn get_meta(
    State(state): State<AppState>,
    headers: HeaderMap,
    jar: CookieJar,
) -> Json<MetaResponse> {
    let mut response = MetaResponse {
        updated: chrono::Utc::now().to_rfc3339(),
        source: "Scuffed Crew".into(),
        heroes: Vec::new(),
        personal: None,
    };

    let Some(user) = try_get_user(&state, &headers, &jar).await else {
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

/// Optional session, using the same bearer-then-cookie order as `AuthUser`.
async fn try_get_user(
    state: &AppState,
    headers: &HeaderMap,
    jar: &CookieJar,
) -> Option<scuffed_auth::User> {
    scuffed_site_server::extractors::optional_session_user(state, headers, jar).await
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
            auth_redirect_base: None,
            upload_dir: PathBuf::from("/tmp/scuffed-test-uploads"),
            reports_dir: PathBuf::from("/tmp/scuffed-test-reports"),
            reports_enabled: true,
            packs_dir: None,
            notifier: None,
            nostr_challenge_key: [0u8; 32],
            consumed_challenges: scuffed_site_server::challenge_store::ConsumedChallengeStore::new(
            ),
            nostr_rate_limiter: scuffed_site_server::nostr_rate_limit::NostrRateLimiter::new(),
            login_lockout: scuffed_site_server::login_lockout::LoginLockout::new(),
            link_code_attempts: scuffed_site_server::link_attempts::LinkCodeAttempts::new(),
            link_poll: scuffed_site_server::link_poll::LinkPollGate::system(),
            crypto: None,
            relay_url: None,
            dm_events: None,
            nip05_domain: None,
            nip05_republish_enabled: false,
            public_settings: scuffed_site_server::state::PublicSettingsCache::new(),
            leaderboard_cache: scuffed_site_server::leaderboard_cache::LeaderboardCache::from_env(),
        }
    }

    async fn seed_role(state: &AppState, username: &str, role: OrgRole, token: &str) -> String {
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
        user.id
    }

    async fn call_json(
        app: Router,
        method: Method,
        path: &str,
        token: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        call_auth(app, method, path, token, None, body).await
    }

    /// `bearer` is sent as `Authorization: Bearer`. `cookie` is the raw session
    /// token sent as the session cookie. Both may be set so tests can check
    /// that a bearer credential is not ignored in favor of the cookie.
    async fn call_auth(
        app: Router,
        method: Method,
        path: &str,
        bearer: Option<&str>,
        cookie: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(path);
        if let Some(tok) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {tok}"));
        }
        if let Some(tok) = cookie {
            let name = SessionConfig::default().cookie_name;
            builder = builder.header(header::COOKIE, format!("{name}={tok}"));
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

    /// Local account plus a session, with no org membership row.
    async fn seed_session(state: &AppState, username: &str, token: &str) -> String {
        let user = state
            .db
            .create_local_user(username, "unused-hash")
            .await
            .expect("user");
        state
            .db
            .create_session(&user.id, token, 24)
            .await
            .expect("session");
        user.id
    }

    async fn create_private_strategy(state: &AppState, token: &str) -> String {
        let (status, created) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some(token),
            Some(json!({
                "name": "Private plan",
                "map_id": "kings-row",
                "game_mode": "hybrid",
                "visibility": "private"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        created["id"]
            .as_str()
            .expect("created strategy id")
            .to_string()
    }

    async fn read_strategy(
        state: &AppState,
        id: &str,
        bearer: Option<&str>,
        cookie: Option<&str>,
    ) -> (StatusCode, serde_json::Value) {
        call_auth(
            strategy_routes(state.clone()),
            Method::GET,
            &format!("/api/strategy/strategies/{id}"),
            bearer,
            cookie,
            None,
        )
        .await
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

    #[test]
    fn absolute_http_url_accepts_blizzard_and_rejects_script_schemes() {
        let blizzard = "https://overwatch.blizzard.com/en-us/news/patch-notes/";
        assert!(absolute_http_url(blizzard));
        assert!(absolute_http_url(
            "http://overwatch.blizzard.com/en-us/news/patch-notes/"
        ));
        assert!(absolute_http_url(
            "HTTPS://overwatch.blizzard.com/en-us/news/patch-notes/"
        ));
        for note in scuffed_db::queries::patch_notes::initial_patch_note_catalog() {
            assert!(
                absolute_http_url(&note.url),
                "seeded {} url {} must stay valid",
                note.version,
                note.url
            );
        }
        for bad in [
            "javascript:alert(1)",
            "JavaScript:alert(1)",
            "data:text/html,hi",
            "vbscript:msgbox(1)",
            "//overwatch.blizzard.com/en-us/news/patch-notes/",
            "/en-us/news/patch-notes/",
            "https:overwatch.blizzard.com/en-us/news/patch-notes/",
            "http:javascript:alert(1)",
            "http://",
            "https://",
            "https://exa mple.com",
            "https://example.com\\@evil.com",
            "",
        ] {
            assert!(!absolute_http_url(bad), "{bad:?} must be rejected");
        }
    }

    #[tokio::test]
    async fn patch_note_url_rejects_javascript_and_accepts_https() {
        let state = test_state().await;
        seed_role(&state, "officer", OrgRole::Officer, OFFICER_TOKEN).await;

        let mut javascript = sample_create("9.9.0");
        javascript["url"] = json!("javascript:alert(1)");
        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(javascript),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], PATCH_NOTE_URL_ERROR);

        let mut data_url = sample_create("9.9.1");
        data_url["url"] = json!("data:text/html,hi");
        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(data_url),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], PATCH_NOTE_URL_ERROR);

        let blizzard = "https://overwatch.blizzard.com/en-us/news/patch-notes/";
        let mut ok = sample_create("9.9.2");
        ok["url"] = json!(blizzard);
        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(ok),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["url"], blizzard);

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::PUT,
            "/api/strategy/patch-notes/9.9.2",
            Some(OFFICER_TOKEN),
            Some(json!({ "url": "javascript:alert(document.domain)" })),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{body}");
        assert_eq!(body["error"], PATCH_NOTE_URL_ERROR);

        let (status, body) =
            get_json(strategy_routes(state.clone()), "/api/strategy/patch-notes").await;
        assert_eq!(status, StatusCode::OK, "{body}");
        let stored = body["data"]
            .as_array()
            .expect("data")
            .iter()
            .find(|n| n["version"] == "9.9.2")
            .expect("created note still listed");
        assert_eq!(stored["url"], blizzard, "rejected update must not persist");

        let updated = "https://overwatch.blizzard.com/en-us/news/";
        let (status, body) = call_json(
            strategy_routes(state),
            Method::PUT,
            "/api/strategy/patch-notes/9.9.2",
            Some(OFFICER_TOKEN),
            Some(json!({ "url": format!("  {updated}  ") })),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["url"], updated);
    }

    #[tokio::test]
    async fn patch_note_create_trims_surrounding_whitespace() {
        let state = test_state().await;
        seed_role(&state, "officer", OrgRole::Officer, OFFICER_TOKEN).await;

        let trimmed = "https://overwatch.blizzard.com/en-us/news/patch-notes/";
        let mut body = sample_create("9.9.4");
        body["url"] = json!(format!(" \n{trimmed}\r\n "));
        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/patch-notes",
            Some(OFFICER_TOKEN),
            Some(body),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["url"], trimmed);

        let (status, listed) = get_json(strategy_routes(state), "/api/strategy/patch-notes").await;
        assert_eq!(status, StatusCode::OK, "{listed}");
        let stored = listed["data"]
            .as_array()
            .expect("data")
            .iter()
            .find(|n| n["version"] == "9.9.4")
            .expect("created note");
        assert_eq!(stored["url"], trimmed);
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
        let heroes = body["data"].as_array().expect("{ data: [...] } envelope");
        assert_eq!(heroes.len(), scuffed_types::HERO_NAMES.len());
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

    /// Issue #336: `POST` accepts the session as `Authorization: Bearer`, but
    /// `GET /api/strategy/strategies/{id}` used to read only the session cookie,
    /// so the owner who just created a private strategy got 404.
    #[tokio::test]
    async fn owner_bearer_can_read_private_strategy() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        let id = create_private_strategy(&state, "owner-token").await;

        let (status, body) = read_strategy(&state, &id, Some("owner-token"), None).await;
        assert_eq!(
            status,
            StatusCode::OK,
            "owner bearer must read the private strategy they just created, got {status} {body}"
        );
        assert_eq!(body["name"], "Private plan");
        assert_eq!(body["visibility"], "private");
    }

    #[tokio::test]
    async fn owner_cookie_can_read_private_strategy() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        let id = create_private_strategy(&state, "owner-token").await;

        let (status, body) = read_strategy(&state, &id, None, Some("owner-token")).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["name"], "Private plan");
    }

    #[tokio::test]
    async fn anonymous_cannot_read_private_strategy() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        let id = create_private_strategy(&state, "owner-token").await;

        let (status, body) = read_strategy(&state, &id, None, None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "Strategy not found");
    }

    #[tokio::test]
    async fn non_member_cannot_read_private_strategy() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        seed_session(&state, "outsider", "outsider-token").await;
        let id = create_private_strategy(&state, "owner-token").await;

        let (status, body) = read_strategy(&state, &id, Some("outsider-token"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "Strategy not found");
    }

    #[tokio::test]
    async fn other_member_cannot_read_private_strategy() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        seed_role(&state, "other", OrgRole::Member, "other-token").await;
        let id = create_private_strategy(&state, "owner-token").await;

        let (status, body) = read_strategy(&state, &id, Some("other-token"), None).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "Strategy not found");
    }

    /// A present `Authorization` header is the credential, same as `AuthUser`.
    /// An outsider bearer must not fall through to the owner's session cookie.
    #[tokio::test]
    async fn outsider_bearer_does_not_use_owner_cookie() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        seed_session(&state, "outsider", "outsider-token").await;
        let id = create_private_strategy(&state, "owner-token").await;

        let (status, body) =
            read_strategy(&state, &id, Some("outsider-token"), Some("owner-token")).await;
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "Strategy not found");
    }

    fn assert_private_strategy_hidden(status: StatusCode, body: &serde_json::Value) {
        assert_eq!(status, StatusCode::NOT_FOUND, "{body}");
        assert_eq!(body["error"], "Strategy not found");
        assert!(
            body.get("name").is_none() && body.get("owner_id").is_none(),
            "anonymous read must not return the strategy: {body}"
        );
    }

    /// An unknown bearer is not a session. With or without the owner's cookie
    /// it stays anonymous and must not read the private strategy.
    #[tokio::test]
    async fn unknown_bearer_is_anonymous_on_private_strategy() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        let id = create_private_strategy(&state, "owner-token").await;

        let (status, body) = read_strategy(&state, &id, Some("no-such-session"), None).await;
        assert_private_strategy_hidden(status, &body);

        let (status, body) =
            read_strategy(&state, &id, Some("no-such-session"), Some("owner-token")).await;
        assert_private_strategy_hidden(status, &body);
    }

    /// A garbage bearer value is not a session either, and does not fall
    /// through to the owner's cookie.
    #[tokio::test]
    async fn garbage_bearer_is_anonymous_on_private_strategy() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        let id = create_private_strategy(&state, "owner-token").await;
        let garbage = "!!!not a token!!!";

        let (status, body) = read_strategy(&state, &id, Some(garbage), None).await;
        assert_private_strategy_hidden(status, &body);

        let (status, body) = read_strategy(&state, &id, Some(garbage), Some("owner-token")).await;
        assert_private_strategy_hidden(status, &body);
    }

    #[tokio::test]
    async fn public_strategy_stays_readable_without_auth() {
        let state = test_state().await;
        seed_role(&state, "stratowner", OrgRole::Member, "owner-token").await;
        let (status, created) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some("owner-token"),
            Some(json!({
                "name": "Public plan",
                "map_id": "kings-row",
                "game_mode": "hybrid",
                "visibility": "public"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().expect("id");

        let (status, body) = read_strategy(&state, id, None, None).await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert_eq!(body["name"], "Public plan");
    }

    /// Personal meta is member-only. Bearer and cookie both count; anonymous
    /// and non-member sessions still get the public stub.
    #[tokio::test]
    async fn member_bearer_gets_personal_meta_and_non_members_do_not() {
        let state = test_state().await;
        seed_role(&state, "member", OrgRole::Member, "member-token").await;
        seed_session(&state, "outsider", "outsider-token").await;

        let (status, body) = call_auth(
            strategy_routes(state.clone()),
            Method::GET,
            "/api/strategy/meta",
            Some("member-token"),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{body}");
        assert!(
            body.get("personal").is_some(),
            "member bearer must receive personal meta, got {body}"
        );

        let (status, cookie_body) = call_auth(
            strategy_routes(state.clone()),
            Method::GET,
            "/api/strategy/meta",
            None,
            Some("member-token"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cookie_body}");
        assert!(cookie_body.get("personal").is_some(), "{cookie_body}");

        let (status, anon) = call_auth(
            strategy_routes(state.clone()),
            Method::GET,
            "/api/strategy/meta",
            None,
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{anon}");
        assert!(anon.get("personal").is_none(), "{anon}");

        let (status, outsider) = call_auth(
            strategy_routes(state),
            Method::GET,
            "/api/strategy/meta",
            Some("outsider-token"),
            None,
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{outsider}");
        assert!(
            outsider.get("personal").is_none(),
            "non-member bearer must not receive personal meta, got {outsider}"
        );
    }

    fn sample_element() -> StrategyElement {
        StrategyElement::new(
            scuffed_types::strategy::ElementType::PlayerMarker,
            scuffed_types::strategy::Position::new(12.0, 34.0),
        )
        .with_label("Tank here".into())
    }

    /// APP2: the first save of a new strategy carries the drawing. It used to
    /// be dropped because create ignored elements and phases.
    #[tokio::test]
    async fn create_with_elements_and_phases_returns_them_on_read() {
        let state = test_state().await;
        seed_role(&state, "drawer", OrgRole::Member, MEMBER_TOKEN).await;
        let element = sample_element();
        let phases = vec![
            TimelinePhase::new("Setup".into(), 0),
            TimelinePhase::new("Push".into(), 1),
        ];

        let (status, created) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some(MEMBER_TOKEN),
            Some(json!({
                "name": "Drawn before save",
                "map_id": "kings-row",
                "game_mode": "hybrid",
                "visibility": "private",
                "elements": [element],
                "phases": phases,
            })),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().expect("id").to_string();
        assert_eq!(created["elements"].as_array().map(Vec::len), Some(1));

        let (status, read) = read_strategy(&state, &id, Some(MEMBER_TOKEN), None).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        let stored: Strategy = serde_json::from_value(read).expect("strategy");
        assert_eq!(stored.elements, vec![element]);
        assert_eq!(stored.phases, phases);
    }

    /// Without content the server still seeds its default phase.
    #[tokio::test]
    async fn create_without_content_keeps_default_phase() {
        let state = test_state().await;
        seed_role(&state, "drawer", OrgRole::Member, MEMBER_TOKEN).await;
        let id = create_private_strategy(&state, MEMBER_TOKEN).await;
        let (status, read) = read_strategy(&state, &id, Some(MEMBER_TOKEN), None).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        assert_eq!(read["elements"].as_array().map(Vec::len), Some(0));
        assert_eq!(read["phases"].as_array().map(Vec::len), Some(1));
    }

    /// SEC-4: a session without an org membership cannot create, list "mine",
    /// update or delete strategies, even one it owns from before the fix.
    #[tokio::test]
    async fn non_member_session_cannot_write_strategies() {
        let state = test_state().await;
        let outsider_id = seed_session(&state, "outsider", "outsider-token").await;
        let owned = state
            .db
            .create_strategy(
                "Legacy plan",
                None,
                "kings-row",
                None,
                GameMode::Hybrid,
                &outsider_id,
                None,
                Visibility::Public,
            )
            .await
            .expect("legacy strategy")
            .id;

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some("outsider-token"),
            Some(sample_strategy()),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::GET,
            "/api/strategy/strategies/mine",
            Some("outsider-token"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::PUT,
            &format!("/api/strategy/strategies/{owned}"),
            Some("outsider-token"),
            Some(json!({ "name": "Defaced" })),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::DELETE,
            &format!("/api/strategy/strategies/{owned}"),
            Some("outsider-token"),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            None,
            Some(sample_strategy()),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");

        let stored = state
            .db
            .get_strategy(&owned)
            .await
            .expect("read")
            .expect("legacy strategy still exists");
        assert_eq!(stored.name, "Legacy plan");
    }

    #[tokio::test]
    async fn oversized_strategy_input_is_rejected_with_400() {
        let state = test_state().await;
        seed_role(&state, "member", OrgRole::Member, MEMBER_TOKEN).await;

        let mut long_name = sample_strategy();
        long_name["name"] = json!("n".repeat(MAX_STRATEGY_NAME_CHARS + 1));
        let mut long_description = sample_strategy();
        long_description["description"] = json!("d".repeat(MAX_STRATEGY_DESCRIPTION_CHARS + 1));
        let mut too_many_elements = sample_strategy();
        too_many_elements["elements"] = json!(
            (0..=MAX_STRATEGY_ELEMENTS)
                .map(|_| sample_element())
                .collect::<Vec<_>>()
        );
        let mut too_many_phases = sample_strategy();
        too_many_phases["phases"] = json!(
            (0..=MAX_STRATEGY_PHASES as u32)
                .map(|i| TimelinePhase::new(format!("P{i}"), i))
                .collect::<Vec<_>>()
        );

        for body in [
            long_name,
            long_description,
            too_many_elements,
            too_many_phases,
        ] {
            let (status, resp) = call_json(
                strategy_routes(state.clone()),
                Method::POST,
                "/api/strategy/strategies",
                Some(MEMBER_TOKEN),
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
        }

        // A name at the limit is fine.
        let mut at_limit = sample_strategy();
        at_limit["name"] = json!("n".repeat(MAX_STRATEGY_NAME_CHARS));
        let (status, created) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some(MEMBER_TOKEN),
            Some(at_limit),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{created}");
        let id = created["id"].as_str().expect("id").to_string();
        let path = format!("/api/strategy/strategies/{id}");

        for body in [
            json!({ "name": "n".repeat(MAX_STRATEGY_NAME_CHARS + 1) }),
            json!({ "name": "   " }),
            json!({ "description": "d".repeat(MAX_STRATEGY_DESCRIPTION_CHARS + 1) }),
            json!({ "elements": (0..=MAX_STRATEGY_ELEMENTS).map(|_| sample_element()).collect::<Vec<_>>() }),
            json!({ "phases": (0..=MAX_STRATEGY_PHASES as u32).map(|i| TimelinePhase::new(format!("P{i}"), i)).collect::<Vec<_>>() }),
        ] {
            let (status, resp) = call_json(
                strategy_routes(state.clone()),
                Method::PUT,
                &path,
                Some(MEMBER_TOKEN),
                Some(body),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{resp}");
        }

        let (status, read) = read_strategy(&state, &id, Some(MEMBER_TOKEN), None).await;
        assert_eq!(status, StatusCode::OK, "{read}");
        assert_eq!(read["name"], "n".repeat(MAX_STRATEGY_NAME_CHARS));
        assert_eq!(read["elements"].as_array().map(Vec::len), Some(0));
    }

    #[tokio::test]
    async fn create_team_id_must_be_one_of_the_callers_teams() {
        let state = test_state().await;
        let member = seed_role(&state, "member", OrgRole::Member, MEMBER_TOKEN).await;
        let member = state
            .db
            .get_member_by_user(&member)
            .await
            .expect("member lookup")
            .expect("member row");
        let mine = state
            .db
            .create_team("Mine", "overwatch", None, None, None)
            .await
            .expect("team");
        let other = state
            .db
            .create_team("Other", "overwatch", None, None, None)
            .await
            .expect("team");
        state
            .db
            .add_to_roster(&member.id, &mine.id, scuffed_db::types::TeamRole::Player)
            .await
            .expect("roster");

        let mut foreign = sample_strategy();
        foreign["team_id"] = json!(other.id);
        let (status, body) = call_json(
            strategy_routes(state.clone()),
            Method::POST,
            "/api/strategy/strategies",
            Some(MEMBER_TOKEN),
            Some(foreign),
        )
        .await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{body}");

        let mut own = sample_strategy();
        own["team_id"] = json!(mine.id);
        let (status, body) = call_json(
            strategy_routes(state),
            Method::POST,
            "/api/strategy/strategies",
            Some(MEMBER_TOKEN),
            Some(own),
        )
        .await;
        assert_eq!(status, StatusCode::CREATED, "{body}");
        assert_eq!(body["team_id"], json!(mine.id));
    }
}
