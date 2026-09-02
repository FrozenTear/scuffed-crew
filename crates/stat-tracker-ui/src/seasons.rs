use std::path::{Path, PathBuf};
use std::time::SystemTime;

use chrono::{DateTime, Utc};
use scuffed_types::Season;

use crate::aggregate::SeasonWindow;
use crate::model::SeasonSel;

#[derive(Debug, Clone, Default)]
pub struct SeasonCache {
    pub seasons: Vec<Season>,
    pub fetched_at: Option<DateTime<Utc>>,
    pub from_network: bool,
}

pub fn cache_path(data_dir: &Path) -> PathBuf {
    data_dir.join("seasons.json")
}

pub fn load_cache(data_dir: &Path) -> SeasonCache {
    let path = cache_path(data_dir);
    let Ok(bytes) = std::fs::read(&path) else {
        return SeasonCache::default();
    };
    let Ok(seasons) = serde_json::from_slice::<Vec<Season>>(&bytes) else {
        return SeasonCache::default();
    };
    let fetched_at = std::fs::metadata(&path)
        .and_then(|m| m.modified())
        .ok()
        .and_then(system_time_to_utc);
    SeasonCache {
        seasons,
        fetched_at,
        from_network: false,
    }
}

pub fn write_cache(data_dir: &Path, seasons: &[Season]) -> std::io::Result<()> {
    std::fs::create_dir_all(data_dir)?;
    let bytes = serde_json::to_vec_pretty(seasons)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::InvalidData, e))?;
    std::fs::write(cache_path(data_dir), bytes)
}

pub fn default_selection(seasons: &[Season]) -> SeasonSel {
    seasons
        .iter()
        .find(|s| s.is_current)
        .map(|s| SeasonSel::Season(s.id.clone()))
        .unwrap_or(SeasonSel::AllTime)
}

pub fn window_for(sel: &SeasonSel, seasons: &[Season]) -> Option<SeasonWindow> {
    let id = sel.as_id()?;
    seasons.iter().find(|s| s.id == id).map(|s| SeasonWindow {
        starts_at: s.starts_at,
        ends_at: s.ends_at,
    })
}

pub async fn fetch_seasons(url: String) -> Result<Vec<Season>, String> {
    let client = reqwest::Client::builder()
        .timeout(std::time::Duration::from_secs(8))
        .build()
        .map_err(|e| e.to_string())?;
    let resp = client.get(&url).send().await.map_err(|e| e.to_string())?;
    if !resp.status().is_success() {
        return Err(format!("GET {url} → {}", resp.status()));
    }
    resp.json::<Vec<Season>>().await.map_err(|e| e.to_string())
}

pub fn seasons_url_from_server(server_url: &str) -> String {
    format!("{}/api/public/seasons", server_url.trim_end_matches('/'))
}

fn system_time_to_utc(t: SystemTime) -> Option<DateTime<Utc>> {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
}
