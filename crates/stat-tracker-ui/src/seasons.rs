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

/// Fixtures never hit the network — production `[]` would wipe the sample list.
pub fn should_fetch_seasons(fixture_active: bool, url: Option<&str>) -> bool {
    !fixture_active && url.is_some()
}

/// An empty 200 must not replace a cache written this run (or any existing list).
pub fn apply_fetched_seasons(existing: Vec<Season>, fetched: Vec<Season>) -> Vec<Season> {
    if fetched.is_empty() && !existing.is_empty() {
        existing
    } else {
        fetched
    }
}

fn system_time_to_utc(t: SystemTime) -> Option<DateTime<Utc>> {
    let d = t.duration_since(SystemTime::UNIX_EPOCH).ok()?;
    DateTime::<Utc>::from_timestamp(d.as_secs() as i64, d.subsec_nanos())
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    fn season(id: &str, current: bool) -> Season {
        Season {
            id: id.into(),
            name: id.into(),
            starts_at: Utc.with_ymd_and_hms(2026, 9, 1, 0, 0, 0).unwrap(),
            ends_at: Utc.with_ymd_and_hms(2026, 12, 1, 0, 0, 0).unwrap(),
            is_current: current,
        }
    }

    #[test]
    fn fixture_never_fetches() {
        assert!(!should_fetch_seasons(
            true,
            Some("https://example.test/api/public/seasons")
        ));
        assert!(should_fetch_seasons(
            false,
            Some("https://example.test/api/public/seasons")
        ));
        assert!(!should_fetch_seasons(false, None));
    }

    #[test]
    fn empty_fetch_does_not_wipe_existing() {
        let existing = vec![season("season-17", true)];
        let kept = apply_fetched_seasons(existing.clone(), Vec::new());
        assert_eq!(kept.len(), 1);
        assert_eq!(kept[0].id, "season-17");
    }

    #[test]
    fn nonempty_fetch_replaces_cache() {
        let existing = vec![season("old", false)];
        let fetched = vec![season("season-17", true)];
        let out = apply_fetched_seasons(existing, fetched);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].id, "season-17");
    }
}
