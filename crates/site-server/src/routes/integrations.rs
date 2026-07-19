//! Admin integration helpers (Discord webhook test, etc.).

use axum::{Json, extract::State, http::StatusCode};
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType};
use serde::Serialize;

use crate::extractors::AdminUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

#[derive(Serialize)]
pub struct DiscordTestResponse {
    pub ok: bool,
    pub message: String,
}

/// POST /api/admin/integrations/discord/test — admin only.
/// Sends a test ping to the officers Discord webhook (fire-and-forget).
/// Returns 503 if Discord officers webhook is not configured.
pub async fn test_discord_webhook(
    State(state): State<AppState>,
    admin: AdminUser,
) -> Result<Json<DiscordTestResponse>, (StatusCode, Json<ErrorResponse>)> {
    let Some(ref notifier) = state.notifier else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Notifications not configured".into(),
            }),
        ));
    };

    let Some(ref discord) = notifier.discord else {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "Discord webhooks not configured (set DISCORD_WEBHOOK_OFFICERS)".into(),
            }),
        ));
    };

    if !discord.has_officers_webhook() {
        return Err((
            StatusCode::SERVICE_UNAVAILABLE,
            Json(ErrorResponse {
                error: "DISCORD_WEBHOOK_OFFICERS is not set".into(),
            }),
        ));
    }

    discord.notify_officers(
        "🔔 Scuffed Crew test ping — Discord officers webhook is working.".to_string(),
    );

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::TestedDiscordWebhook,
        AuditTargetType::Integration,
        "discord_officers",
        None,
    )
    .await;

    Ok(Json(DiscordTestResponse {
        ok: true,
        message: "Test message queued to Discord officers webhook".into(),
    }))
}
