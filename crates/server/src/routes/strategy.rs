use axum::{
    Json, Router,
    extract::{Path, Query, State},
    http::StatusCode,
    routing::get,
};
use axum_extra::extract::cookie::CookieJar;
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

use scuffed_auth::server::{AuthUser, HasAuth};
use scuffed_site_server::state::AppState;
use scuffed_types::api::ApiSuccess;
use scuffed_types::patch_notes::{PatchChange, PatchHeroUpdate, PatchNote, PatchSection};
use scuffed_types::strategy::{
    GameMode, Strategy, StrategyElement, StrategySummary, TimelinePhase, Visibility,
};

/// Strategy API routes — merged into the unified server.
pub fn strategy_routes(state: AppState) -> Router {
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
        .route("/api/strategy/meta", get(get_meta))
        .route("/api/strategy/patch-notes", get(list_patch_notes))
        .with_state(state)
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
/// Envelope is [`ApiSuccess`] (`{ "data": [...] }`). Content is a static catalog
/// (no Surreal table yet) shaped for Site's local `Patch` struct.
async fn list_patch_notes() -> Json<ApiSuccess<Vec<PatchNote>>> {
    Json(ApiSuccess {
        data: patch_notes_catalog(),
    })
}

/// Scuffed-native sample catalog so Site can render cards, filters, and search.
/// Not a live Blizzard scrape and not copied from the older strategy-app.
fn patch_notes_catalog() -> Vec<PatchNote> {
    vec![
        PatchNote {
            version: "2.18.1".into(),
            date: "2026-08-20".into(),
            title: Some("Mid-season balance".into()),
            url: "https://overwatch.blizzard.com/en-us/news/patch-notes/".into(),
            hero_updates: vec![
                PatchHeroUpdate {
                    hero_id: "ana".into(),
                    hero_name: "Ana".into(),
                    change_type: "adjustment".into(),
                    changes: vec![PatchChange {
                        ability: Some("Biotic Grenade".into()),
                        description: "Healing-boost window is a bit shorter.".into(),
                        change_type: "nerf".into(),
                    }],
                    dev_comment: Some(
                        "Keeping her burst sustain in line with other supports.".into(),
                    ),
                },
                PatchHeroUpdate {
                    hero_id: "venture".into(),
                    hero_name: "Venture".into(),
                    change_type: "buff".into(),
                    changes: vec![PatchChange {
                        ability: Some("Drill Dash".into()),
                        description: "Dash distance increased slightly.".into(),
                        change_type: "buff".into(),
                    }],
                    dev_comment: None,
                },
            ],
            sections: vec![
                PatchSection {
                    category: "Competitive".into(),
                    items: vec!["Placement games now show an expected rank band.".into()],
                },
                PatchSection {
                    category: "Bug Fixes".into(),
                    items: vec!["Fixed a rare scoreboard freeze after overtime.".into()],
                },
            ],
        },
        PatchNote {
            version: "2.18.0".into(),
            date: "2026-08-11".into(),
            title: Some("Season 4 launch".into()),
            url: "https://overwatch.blizzard.com/en-us/news/patch-notes/".into(),
            hero_updates: vec![PatchHeroUpdate {
                hero_id: "freja".into(),
                hero_name: "Freja".into(),
                change_type: "bugfix".into(),
                changes: vec![PatchChange {
                    ability: None,
                    description: "Fixed an animation hitch when swapping weapons mid-air.".into(),
                    change_type: "bugfix".into(),
                }],
                dev_comment: None,
            }],
            sections: vec![
                PatchSection {
                    category: "Maps".into(),
                    items: vec![
                        "New Clash rotation includes Aatlis.".into(),
                        "Health pack on New Queen Street mid is easier to contest.".into(),
                    ],
                },
                PatchSection {
                    category: "General".into(),
                    items: vec!["Career profile now lists last season's peak.".into()],
                },
            ],
        },
        PatchNote {
            version: "2.17.3".into(),
            date: "2026-07-22".into(),
            title: Some("Stability hotfix".into()),
            url: "https://overwatch.blizzard.com/en-us/news/patch-notes/".into(),
            hero_updates: vec![],
            sections: vec![
                PatchSection {
                    category: "Bug Fixes".into(),
                    items: vec![
                        "Fixed a disconnect when rejoining a custom game lobby.".into(),
                        "Spectator UI no longer duplicates the ultimate bar.".into(),
                    ],
                },
                PatchSection {
                    category: "Maps".into(),
                    items: vec!["Closed an out-of-bounds perch on Colosseo.".into()],
                },
            ],
        },
    ]
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
    use axum::http::{Method, Request, StatusCode};
    use http_body_util::BodyExt;
    use scuffed_auth::SessionConfig;
    use scuffed_db::Database;
    use scuffed_db::migrations::run_migrations;
    use scuffed_site_server::state::OAuthConfig;
    use std::path::PathBuf;
    use std::sync::Arc;
    use tower::ServiceExt;

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

    async fn get_json(app: Router, path: &str) -> (StatusCode, serde_json::Value) {
        let resp = app
            .oneshot(
                Request::builder()
                    .method(Method::GET)
                    .uri(path)
                    .body(Body::empty())
                    .unwrap(),
            )
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let value: serde_json::Value = serde_json::from_slice(&bytes).expect("json body");
        (status, value)
    }

    #[test]
    fn catalog_has_fields_site_renders() {
        let notes = patch_notes_catalog();
        assert!(
            !notes.is_empty(),
            "empty catalog would show Site empty-state, not cards"
        );
        for note in &notes {
            assert!(!note.version.is_empty());
            assert!(!note.date.is_empty());
            assert!(!note.url.is_empty());
        }
        assert!(
            notes.iter().any(|n| !n.hero_updates.is_empty()),
            "need at least one hero-balance card for the Hero Balance filter"
        );
        assert!(
            notes.iter().any(|n| n
                .sections
                .iter()
                .any(|s| s.category.to_lowercase().contains("bug"))),
            "need a Bug Fixes section for that filter chip"
        );
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
    }

    #[tokio::test]
    async fn patch_notes_root_is_object_not_bare_array() {
        let app = Router::new().route("/api/strategy/patch-notes", get(list_patch_notes));
        let (status, body) = get_json(app, "/api/strategy/patch-notes").await;
        assert_eq!(status, StatusCode::OK);
        assert!(
            body.is_object(),
            "Site unwraps {{ data }}; a bare array would break Browse-style clients"
        );
    }
}
