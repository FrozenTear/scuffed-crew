use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, Game};

use crate::extractors::AdminUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// GET /api/games — list all games (public)
pub async fn list_games(
    State(state): State<AppState>,
) -> Result<Json<Vec<Game>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_games().await.map(Json).map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
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
