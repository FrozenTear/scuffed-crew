use axum::{
    Json,
    extract::{Multipart, State},
    http::StatusCode,
};
use serde::Serialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType};

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;
use crate::uploads::save_upload;

const AVATAR_MAX_BYTES: usize = 2 * 1024 * 1024; // 2 MB
const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024; // 5 MB

#[derive(Serialize)]
pub struct UploadResponse {
    pub url: String,
}

/// POST /api/upload/avatar — upload member avatar (org member)
pub async fn upload_avatar(
    State(state): State<AppState>,
    member: OrgMember,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid multipart: {e}"),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "No file provided".into(),
                }),
            )
        })?;

    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    let data = field.bytes().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Failed to read file: {e}"),
            }),
        )
    })?;

    let url = save_upload(
        &state.upload_dir,
        "avatars",
        &data,
        &content_type,
        AVATAR_MAX_BYTES,
    )
    .await
    .map_err(|_e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    audit(
        &state.db,
        &member.member.id,
        AuditAction::UploadedAvatar,
        AuditTargetType::Upload,
        &url,
        None,
    )
    .await;

    Ok(Json(UploadResponse { url }))
}

/// POST /api/upload/image — upload general image (officer+)
pub async fn upload_image(
    State(state): State<AppState>,
    officer: OfficerUser,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: format!("Invalid multipart: {e}"),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "No file provided".into(),
                }),
            )
        })?;

    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    let data = field.bytes().await.map_err(|e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: format!("Failed to read file: {e}"),
            }),
        )
    })?;

    let url = save_upload(
        &state.upload_dir,
        "images",
        &data,
        &content_type,
        IMAGE_MAX_BYTES,
    )
    .await
    .map_err(|_e| {
        (
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UploadedImage,
        AuditTargetType::Upload,
        &url,
        None,
    )
    .await;

    Ok(Json(UploadResponse { url }))
}
