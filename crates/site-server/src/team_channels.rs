//! Best-effort team chat channel provisioning (F-API-003).
//!
//! `send_encrypted` looks up `team_channel` by `group_id`. Nothing used to
//! write those rows. Team create/update and a startup/admin backfill call
//! [`scuffed_chat::ensure_team_channel_rows`] so the encrypt path can find a
//! channel. Relay NIP-29 publish is optional and not required here.

use scuffed_db::Database;

use crate::state::AppState;

/// Provision public + officer channel rows for one team. Logs and continues
/// on failure so team create/update still succeed.
pub async fn provision_for_team(state: &AppState, team_id: &str) {
    let relay_url = state.relay_url.as_deref().unwrap_or("");
    match scuffed_chat::ensure_team_channel_rows(&state.db, team_id, relay_url).await {
        Ok(p) => tracing::info!(
            team_id,
            public = %p.public_group_id,
            officer = ?p.officer_group_id,
            "Team chat channels provisioned"
        ),
        Err(e) => tracing::error!(team_id, error = %e, "Failed to provision team chat channels"),
    }
}

/// Backfill every active team. Used at process start and by the admin POST.
pub async fn backfill_all_teams(db: &Database, relay_url: Option<&str>) -> usize {
    let url = relay_url.unwrap_or("");
    match scuffed_chat::provision_all_team_channels(db, url).await {
        Ok(n) => {
            tracing::info!(teams = n, "Team chat channel backfill complete");
            n
        }
        Err(e) => {
            tracing::error!(error = %e, "Team chat channel backfill failed");
            0
        }
    }
}

/// Startup hook: never fail boot if backfill errors.
pub async fn backfill_on_startup(state: &AppState) {
    let _ = backfill_all_teams(&state.db, state.relay_url.as_deref()).await;
}
