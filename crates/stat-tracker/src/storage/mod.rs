use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};
use surrealdb::Surreal;
use surrealdb::engine::local::SurrealKv;
use surrealdb_types::Datetime as SurrealDatetime;
use surrealdb_types::SurrealValue;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, SurrealValue)]
pub struct PersonalMatch {
    pub hero: String,
    pub map_name: String,
    pub game_mode: String,
    pub role: String,
    pub outcome: String,
    pub elims: u32,
    pub deaths: u32,
    pub assists: u32,
    pub damage: u32,
    pub healing: u32,
    pub mitigation: u32,
    pub played_at: SurrealDatetime,
    #[serde(default)]
    pub synced: bool,
    #[serde(default)]
    pub session_id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
pub struct MatchSession {
    pub session_id: String,
    pub hero: String,
    pub map_name: String,
    pub role: String,
    pub started_at: SurrealDatetime,
    pub last_capture_at: SurrealDatetime,
    pub capture_count: u32,
    pub final_outcome: String,
}

#[derive(Clone)]
pub struct LocalStore {
    db: Surreal<surrealdb::engine::local::Db>,
}

impl LocalStore {
    pub async fn open(data_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let db_path = data_dir.join("stats.surrealkv");
        std::fs::create_dir_all(&db_path)?;

        let db =
            Surreal::new::<SurrealKv>(db_path.to_str().ok_or("data_dir path is not valid UTF-8")?)
                .await?;
        db.use_ns("stat_tracker").use_db("local").await?;

        db.query(
            "
            DEFINE TABLE IF NOT EXISTS personal_match SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS idx_synced ON personal_match FIELDS synced;
            DEFINE INDEX IF NOT EXISTS idx_played_at ON personal_match FIELDS played_at;
            DEFINE INDEX IF NOT EXISTS idx_session_id ON personal_match FIELDS session_id;
            DEFINE TABLE IF NOT EXISTS match_session SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS idx_session_id ON match_session FIELDS session_id;
            DEFINE INDEX IF NOT EXISTS idx_last_capture ON match_session FIELDS last_capture_at;
        ",
        )
        .await?;

        tracing::info!(path = %db_path.display(), "local store opened");
        Ok(Self { db })
    }

    pub async fn insert_match(
        &self,
        match_data: PersonalMatch,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _: Option<PersonalMatch> = self.db.create("personal_match").content(match_data).await?;
        tracing::debug!("match inserted into local store");
        Ok(())
    }

    pub async fn get_unsynced(&self) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error>> {
        let mut result = self
            .db
            .query("SELECT * FROM personal_match WHERE synced = false ORDER BY played_at ASC")
            .await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        Ok(matches)
    }

    pub async fn mark_synced(&self, count: usize) -> Result<(), Box<dyn std::error::Error>> {
        self.db
            .query("UPDATE (SELECT id FROM personal_match WHERE synced = false ORDER BY played_at ASC LIMIT $limit) SET synced = true")
            .bind(("limit", count))
            .await?;
        Ok(())
    }

    pub async fn match_count(&self) -> Result<usize, Box<dyn std::error::Error>> {
        let mut result = self
            .db
            .query("SELECT count() AS total FROM personal_match GROUP ALL")
            .await?;
        let row: Option<CountRow> = result.take(0)?;
        Ok(row.map(|r| r.total).unwrap_or(0))
    }

    pub async fn get_all_matches(&self) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error>> {
        let mut result = self
            .db
            .query("SELECT * FROM personal_match ORDER BY played_at DESC")
            .await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        Ok(matches)
    }

    pub async fn find_active_session(
        &self,
        hero: &str,
        window_secs: u64,
    ) -> Result<Option<MatchSession>, Box<dyn std::error::Error>> {
        let cutoff = chrono::Utc::now() - chrono::Duration::seconds(window_secs as i64);
        let cutoff_dt = SurrealDatetime::from(cutoff);
        let mut result = self
            .db
            .query("SELECT * FROM match_session WHERE hero = $hero AND last_capture_at > $cutoff ORDER BY last_capture_at DESC LIMIT 1")
            .bind(("hero", hero.to_string()))
            .bind(("cutoff", cutoff_dt))
            .await?;
        let session: Option<MatchSession> = result.take(0)?;
        Ok(session)
    }

    pub async fn create_session(
        &self,
        session: &MatchSession,
    ) -> Result<(), Box<dyn std::error::Error>> {
        let _: Option<MatchSession> = self
            .db
            .create("match_session")
            .content(session.clone())
            .await?;
        Ok(())
    }

    pub async fn update_session(
        &self,
        session_id: &str,
        capture_time: SurrealDatetime,
        capture_count: u32,
        outcome: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.db
            .query("UPDATE match_session SET last_capture_at = $time, capture_count = $count, final_outcome = $outcome WHERE session_id = $sid")
            .bind(("time", capture_time))
            .bind(("count", capture_count))
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }

    /// Append a capture to an existing session: bump the capture count and time,
    /// and refresh the final outcome (the active game owns the authoritative value).
    pub async fn append_capture(
        &self,
        session_id: &str,
        capture_time: SurrealDatetime,
        outcome: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.db
            .query("UPDATE match_session SET last_capture_at = $time, capture_count += 1, final_outcome = $outcome WHERE session_id = $sid")
            .bind(("time", capture_time))
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }

    /// Back-fill a detected outcome onto a session and every capture snapshot it
    /// contains, re-queuing those snapshots for sync so the corrected result
    /// (e.g. a VICTORY read off the accolade screen after the stats were already
    /// captured with `unknown`) propagates to the server.
    pub async fn set_session_outcome(
        &self,
        session_id: &str,
        outcome: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.db
            .query("UPDATE match_session SET final_outcome = $outcome WHERE session_id = $sid; UPDATE personal_match SET outcome = $outcome, synced = false WHERE session_id = $sid")
            .bind(("outcome", outcome.to_string()))
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }

    pub async fn get_all_sessions(&self) -> Result<Vec<MatchSession>, Box<dyn std::error::Error>> {
        let mut result = self
            .db
            .query("SELECT * FROM match_session ORDER BY last_capture_at DESC")
            .await?;
        let sessions: Vec<MatchSession> = result.take(0)?;
        Ok(sessions)
    }

    pub async fn get_session_snapshots(
        &self,
        session_id: &str,
    ) -> Result<Vec<PersonalMatch>, Box<dyn std::error::Error>> {
        let mut result = self
            .db
            .query("SELECT * FROM personal_match WHERE session_id = $sid ORDER BY played_at ASC")
            .bind(("sid", session_id.to_string()))
            .await?;
        let matches: Vec<PersonalMatch> = result.take(0)?;
        Ok(matches)
    }

    pub async fn get_multi_capture_sessions(
        &self,
    ) -> Result<Vec<MatchSession>, Box<dyn std::error::Error>> {
        let mut result = self
            .db
            .query(
                "SELECT * FROM match_session WHERE capture_count > 1 ORDER BY last_capture_at DESC",
            )
            .await?;
        let sessions: Vec<MatchSession> = result.take(0)?;
        Ok(sessions)
    }

    pub async fn clear_all_data(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.db.query("DELETE personal_match").await?;
        self.db.query("DELETE match_session").await?;
        tracing::info!("cleared all match data from local store");
        Ok(())
    }

    /// Export all matches and sessions to `live_snapshot.json` (atomic via
    /// tmp+rename). SurrealKV is single-process, so while the daemon holds the
    /// store open the GUI cannot read it — this snapshot is the GUI's live
    /// data source, refreshed by the daemon after every mutation. Unlike
    /// `matches.jsonl` (append-only insert log), it reflects back-filled
    /// outcomes and sync-state changes.
    pub async fn export_snapshot(&self, data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
        let snapshot = Snapshot {
            matches: self.get_all_matches().await?,
            sessions: self.get_all_sessions().await?,
        };
        let json = serde_json::to_vec(&snapshot)?;
        let tmp = data_dir.join("live_snapshot.json.tmp");
        std::fs::write(&tmp, &json)?;
        std::fs::rename(&tmp, snapshot_path(data_dir))?;
        Ok(())
    }

    pub async fn last_capture_time(&self) -> Option<String> {
        let mut result = self
            .db
            .query("SELECT played_at FROM personal_match ORDER BY played_at DESC LIMIT 1")
            .await
            .ok()?;
        let row: Option<LastCaptureRow> = result.take(0).ok()?;
        row.map(|r| {
            let dt: chrono::DateTime<chrono::Utc> = r.played_at.into();
            dt.with_timezone(&chrono::Local)
                .format("%Y-%m-%d %H:%M:%S")
                .to_string()
        })
    }
}

#[derive(Deserialize, SurrealValue)]
struct CountRow {
    total: usize,
}

#[derive(Deserialize, SurrealValue)]
struct LastCaptureRow {
    played_at: SurrealDatetime,
}

/// The daemon's live export of the full store, for readers (the GUI) that
/// can't open SurrealKV while the daemon has it locked.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Snapshot {
    /// All matches, newest first (mirrors `get_all_matches`).
    pub matches: Vec<PersonalMatch>,
    /// All sessions, most recently captured first (mirrors `get_all_sessions`).
    pub sessions: Vec<MatchSession>,
}

fn snapshot_path(data_dir: &Path) -> PathBuf {
    data_dir.join("live_snapshot.json")
}

/// Read the daemon's live snapshot. `None` if it doesn't exist yet or is
/// unreadable — callers fall back to the append-only match log.
pub fn read_snapshot(data_dir: &Path) -> Option<Snapshot> {
    let content = std::fs::read(snapshot_path(data_dir)).ok()?;
    serde_json::from_slice(&content).ok()
}

fn match_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("matches.jsonl")
}

pub fn append_match_log(data_dir: &Path, m: &PersonalMatch) {
    use std::io::Write;
    let path = match_log_path(data_dir);
    let Ok(mut f) = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
    else {
        return;
    };
    if let Ok(json) = serde_json::to_string(m) {
        let _ = writeln!(f, "{json}");
    }
}

pub fn clear_match_log(data_dir: &Path) {
    let path = match_log_path(data_dir);
    if path.exists() {
        let _ = std::fs::remove_file(&path);
    }
}

pub fn force_clear_data_dir(data_dir: &Path) -> Result<(), Box<dyn std::error::Error>> {
    let db_path = data_dir.join("stats.surrealkv");
    if db_path.exists() {
        std::fs::remove_dir_all(&db_path)?;
    }
    clear_match_log(data_dir);
    tracing::info!("force-cleared data directory");
    Ok(())
}

pub fn read_match_log(data_dir: &Path) -> Vec<PersonalMatch> {
    let path = match_log_path(data_dir);
    let Ok(content) = std::fs::read_to_string(&path) else {
        return Vec::new();
    };
    let mut matches: Vec<PersonalMatch> = content
        .lines()
        .filter_map(|line| serde_json::from_str(line).ok())
        .collect();
    matches.sort_by(|a, b| {
        let da: chrono::DateTime<chrono::Utc> = b.played_at.into();
        let db_time: chrono::DateTime<chrono::Utc> = a.played_at.into();
        da.cmp(&db_time)
    });
    matches
}
