use crate::config::SyncConfig;
use crate::storage::PersonalMatch;

use scuffed_types::api::{
    DaemonConfigResponse, StatsUploadEntry, StatsUploadRequest, StatsUploadResponse,
};

#[derive(Clone)]
pub struct SyncClient {
    config: SyncConfig,
    http: reqwest::Client,
}

impl SyncClient {
    pub fn new(config: SyncConfig) -> Self {
        // A hung connection must never hang the caller: sync runs concurrently
        // with capture, and shutdown does a final inline upload. Fail closed —
        // a client without the timeout re-opens the M4 daemon-stall bug.
        let http = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(30))
            .build()
            .expect("reqwest client with timeout must build");
        Self { http, config }
    }

    /// Fetch daemon configuration from the server (player_name, etc.).
    /// Called on startup when local config has no player_name.
    pub async fn fetch_daemon_config(
        &self,
    ) -> Result<DaemonConfigResponse, Box<dyn std::error::Error + Send + Sync>> {
        let url = format!("{}/api/stats/daemon-config", self.config.server_url);
        let resp = self
            .http
            .get(&url)
            .bearer_auth(&self.config.token)
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("daemon-config fetch failed ({status}): {body}").into());
        }

        Ok(resp.json::<DaemonConfigResponse>().await?)
    }

    pub async fn upload_matches(
        &self,
        matches: &[PersonalMatch],
        deleted_sessions: &[String],
    ) -> Result<StatsUploadResponse, Box<dyn std::error::Error + Send + Sync>> {
        let entries: Vec<StatsUploadEntry> = matches
            .iter()
            // Upload the effective (corrected-if-present, else OCR) values so
            // server aggregates and the leaderboard reflect manual fixes, and
            // flag edited rows for the site badge. The immutable OCR reads stay
            // local — the transparency detail lives in the tracker GUI.
            .map(|m| StatsUploadEntry {
                session_id: m.session_id.clone(),
                hero: m.display_hero().to_string(),
                map_name: m.display_map_name().to_string(),
                game_mode: m.game_mode.clone(),
                role: m.display_role().to_string(),
                outcome: m.display_outcome().to_string(),
                elims: m.display_elims(),
                deaths: m.display_deaths(),
                assists: m.display_assists(),
                damage: m.display_damage(),
                healing: m.display_healing(),
                mitigation: m.display_mitigation(),
                played_at: chrono::DateTime::<chrono::Utc>::from(m.played_at),
                edited: m.is_edited(),
            })
            .collect();

        let url = format!("{}/api/stats/upload", self.config.server_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.token)
            .json(&StatsUploadRequest {
                matches: entries,
                deleted_sessions: deleted_sessions.to_vec(),
            })
            .send()
            .await?;

        if !resp.status().is_success() {
            let status = resp.status();
            let body = resp.text().await.unwrap_or_default();
            return Err(format!("Upload failed ({status}): {body}").into());
        }

        let result: StatsUploadResponse = resp.json().await?;
        tracing::info!(
            inserted = result.inserted,
            skipped = result.skipped,
            deleted = result.deleted,
            "stats upload complete"
        );
        Ok(result)
    }
}
