use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Game};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::AdminUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/games — list all games (public, cursor-paginated envelope).
///
/// Returns [`CursorResponse`] so admin + public clients can share `use_api_list`.
/// Games tables are small; we still honor `limit`/`cursor` for consistency.
pub async fn list_games(
    State(state): State<AppState>,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<Game>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let items = state.db.list_games().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    // In-memory page over the full list (game catalog is tiny).
    let page: Vec<Game> = items.into_iter().skip(offset as usize).collect();
    Ok(Json(CursorResponse::from_oversized(page, limit, offset)))
}

/// GET /api/games/:id — get game detail (public)
pub async fn get_game(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<Game>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_game(&id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        .map(Json)
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Game not found".into(),
                }),
            )
        })
}

#[derive(Deserialize)]
pub struct CreateGameRequest {
    pub name: String,
    pub abbreviation: Option<String>,
}

/// POST /api/games — create game (admin only)
pub async fn create_game(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<CreateGameRequest>,
) -> Result<(StatusCode, Json<Game>), (StatusCode, Json<ErrorResponse>)> {
    let game = state
        .db
        .create_game(&body.name, body.abbreviation.as_deref())
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
        AuditAction::CreatedGame,
        AuditTargetType::Game,
        &game.id,
        Some(&format!("Created game: {}", game.name)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(game)))
}

#[derive(Deserialize)]
pub struct UpdateGameRequest {
    pub name: Option<String>,
    pub abbreviation: Option<Option<String>>,
}

/// PUT /api/games/:id — update game (admin only)
pub async fn update_game(
    State(state): State<AppState>,
    admin: AdminUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateGameRequest>,
) -> Result<Json<Game>, (StatusCode, Json<ErrorResponse>)> {
    let game = state
        .db
        .update_game(
            &id,
            body.name.as_deref(),
            body.abbreviation.as_ref().map(|a| a.as_deref()),
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
        AuditAction::UpdatedGame,
        AuditTargetType::Game,
        &id,
        None,
    )
    .await;

    Ok(Json(game))
}
