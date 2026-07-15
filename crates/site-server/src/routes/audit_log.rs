use axum::{Json, extract::State, http::StatusCode};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditLogEntry, AuditTargetType, Database};

use crate::extractors::AdminUser;
use crate::state::AppState;

#[derive(Deserialize)]
pub struct AuditLogQuery {
    #[serde(default = "default_limit")]
    pub limit: u32,
    #[serde(default)]
    pub offset: u32,
}

fn default_limit() -> u32 {
    50
}

#[derive(Serialize)]
pub struct AuditLogResponse {
    pub entries: Vec<AuditLogEntry>,
    pub total: u64,
}

/// GET /api/audit-log — paginated audit log (admin only)
pub async fn list_audit_log(
    State(state): State<AppState>,
    _admin: AdminUser,
    axum::extract::Query(query): axum::extract::Query<AuditLogQuery>,
) -> Result<Json<AuditLogResponse>, (StatusCode, Json<ErrorResponse>)> {
    let entries = state
        .db
        .list_audit_log(query.limit.min(100), query.offset)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    let total = state.db.count_audit_log().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    Ok(Json(AuditLogResponse { entries, total }))
}

/// Fire-and-forget audit log helper. Logs error but doesn't fail the request.
pub async fn audit(
    db: &Database,
    actor_id: &str,
    action: AuditAction,
    target_type: AuditTargetType,
    target_id: &str,
    details: Option<&str>,
) {
    if let Err(e) = db
        .insert_audit_log(actor_id, action, target_type, target_id, details)
        .await
    {
        tracing::error!("Audit log failed: {e}");
    }
}
