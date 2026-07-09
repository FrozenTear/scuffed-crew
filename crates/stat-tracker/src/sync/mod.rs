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
    ) -> Result<StatsUploadResponse, Box<dyn std::error::Error + Send + Sync>> {
        let entries: Vec<StatsUploadEntry> = matches
            .iter()
            .map(|m| StatsUploadEntry {
                session_id: m.session_id.clone(),
                hero: m.hero.clone(),
                map_name: m.map_name.clone(),
                game_mode: m.game_mode.clone(),
                role: m.role.clone(),
                outcome: m.outcome.clone(),
                elims: m.elims,
                deaths: m.deaths,
                assists: m.assists,
                damage: m.damage,
                healing: m.healing,
                mitigation: m.mitigation,
                played_at: chrono::DateTime::<chrono::Utc>::from(m.played_at),
            })
            .collect();

        let url = format!("{}/api/stats/upload", self.config.server_url);
        let resp = self
            .http
            .post(&url)
            .bearer_auth(&self.config.token)
            .json(&StatsUploadRequest { matches: entries })
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
            "stats upload complete"
        );
        Ok(result)
    }
}
