use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    AuditAction, AuditTargetType, DaemonToken, HeroStats, MapStats, PersonalMatch, PersonalStats,
};
use scuffed_types::api::{
    CreateDaemonTokenRequest, CreateDaemonTokenResponse, CursorResponse, PaginationParams,
    StatsUploadRequest, StatsUploadResponse,
};

use crate::extractors::{DaemonUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// POST /api/stats/upload — bulk upload personal matches (daemon token auth)
pub async fn upload_stats(
    State(state): State<AppState>,
    daemon: DaemonUser,
    Json(body): Json<StatsUploadRequest>,
) -> Result<Json<StatsUploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    if body.matches.is_empty() {
        return Ok(Json(StatsUploadResponse {
            inserted: 0,
            skipped: 0,
        }));
    }

    let total = body.matches.len() as u32;

    let stub_matches: Vec<PersonalMatch> = body
        .matches
        .into_iter()
        .map(|e| PersonalMatch {
            id: String::new(),
            member_id: daemon.member.id.clone(),
            hero: e.hero,
            map_name: e.map_name,
            game_mode: e.game_mode,
            role: e.role,
            outcome: e.outcome,
            elims: e.elims,
            deaths: e.deaths,
            assists: e.assists,
            damage: e.damage,
            healing: e.healing,
            mitigation: e.mitigation,
            played_at: e.played_at,
            uploaded_at: chrono::Utc::now(),
        })
        .collect();

    let inserted = state
        .db
        .bulk_insert_personal_matches(&daemon.member.id, &stub_matches)
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
        &daemon.member.id,
        AuditAction::UploadedPersonalStats,
        AuditTargetType::PersonalStats,
        &daemon.member.id,
        Some(&format!("{inserted} matches uploaded")),
    )
    .await;

    Ok(Json(StatsUploadResponse {
        inserted,
        skipped: total - inserted,
    }))
}

/// GET /api/stats/me — personal stats overview (session auth)
pub async fn my_stats(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<PersonalStats>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_personal_stats(&member.member.id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// GET /api/stats/me/matches — personal match history (session auth, paginated)
pub async fn my_matches(
    State(state): State<AppState>,
    member: OrgMember,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<PersonalMatch>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_personal_matches(&member.member.id, limit, offset)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

/// GET /api/stats/me/heroes — per-hero stats (session auth)
pub async fn my_hero_stats(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<Vec<HeroStats>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_hero_stats(&member.member.id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// GET /api/stats/me/maps — per-map stats (session auth)
pub async fn my_map_stats(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<Vec<MapStats>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_map_stats(&member.member.id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// GET /api/stats/member/:id — view another member's stats (session auth)
pub async fn member_stats(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
) -> Result<Json<PersonalStats>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_personal_stats(&member_id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// GET /api/stats/member/:id/heroes — view another member's hero stats
pub async fn member_hero_stats(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
) -> Result<Json<Vec<HeroStats>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_hero_stats(&member_id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// GET /api/stats/member/:id/maps — view another member's map stats
pub async fn member_map_stats(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
) -> Result<Json<Vec<MapStats>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .get_map_stats(&member_id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// POST /api/stats/tokens — create a daemon token (session auth, for self)
pub async fn create_daemon_token(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<CreateDaemonTokenRequest>,
) -> Result<(StatusCode, Json<CreateDaemonTokenResponse>), (StatusCode, Json<ErrorResponse>)> {
    let raw_token = generate_token();

    let token = state
        .db
        .create_daemon_token(&member.member.id, &raw_token, &body.label)
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
        &member.member.id,
        AuditAction::CreatedDaemonToken,
        AuditTargetType::DaemonToken,
        &token.id,
        Some(&format!("label: {}", body.label)),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CreateDaemonTokenResponse {
            id: token.id,
            token: raw_token,
            label: token.label,
        }),
    ))
}

/// GET /api/stats/tokens — list own daemon tokens (session auth)
pub async fn list_daemon_tokens(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<Vec<DaemonToken>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_daemon_tokens(&member.member.id)
        .await
        .map(Json)
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })
}

/// DELETE /api/stats/tokens/:id — revoke a daemon token (session auth)
pub async fn revoke_daemon_token(
    State(state): State<AppState>,
    member: OrgMember,
    Path(token_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .revoke_daemon_token(&token_id, &member.member.id)
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
        &member.member.id,
        AuditAction::RevokedDaemonToken,
        AuditTargetType::DaemonToken,
        &token_id,
        None,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

fn generate_token() -> String {
    use rand::Rng;
    use std::fmt::Write;
    let bytes: [u8; 32] = rand::thread_rng().r#gen();
    let mut s = String::with_capacity(64);
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}
