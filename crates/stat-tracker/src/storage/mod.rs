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

    /// Delete a session and all its capture snapshots (manual cleanup of a
    /// junk/misdetected game). Already-synced snapshots remain on the server.
    pub async fn delete_session(&self, session_id: &str) -> Result<(), Box<dyn std::error::Error>> {
        self.db
            .query("DELETE match_session WHERE session_id = $sid; DELETE personal_match WHERE session_id = $sid")
            .bind(("sid", session_id.to_string()))
            .await?;
        Ok(())
    }

    /// Apply a queued store command (see [`StoreCommand`]).
    pub async fn apply_command(
        &self,
        cmd: &StoreCommand,
    ) -> Result<(), Box<dyn std::error::Error>> {
        match cmd {
            StoreCommand::SetOutcome {
                session_id,
                outcome,
            } => self.set_session_outcome(session_id, outcome).await,
            StoreCommand::DeleteSession { session_id } => self.delete_session(session_id).await,
        }
    }

    /// Set a session's displayed hero (and role), used to keep the label on
    /// the majority hero across its snapshots rather than whatever the first
    /// capture happened to read.
    pub async fn set_session_hero(
        &self,
        session_id: &str,
        hero: &str,
        role: &str,
    ) -> Result<(), Box<dyn std::error::Error>> {
        self.db
            .query("UPDATE match_session SET hero = $hero, role = $role WHERE session_id = $sid")
            .bind(("hero", hero.to_string()))
            .bind(("role", role.to_string()))
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

/// A store mutation requested by the GUI. SurrealKV is single-process, so
/// while the daemon holds the store the GUI can't write — it queues commands
/// as JSON files in `<data_dir>/commands/`, which the daemon applies within a
/// few seconds (and then refreshes the live snapshot). When no daemon runs,
/// the GUI applies commands directly.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "op", rename_all = "snake_case")]
pub enum StoreCommand {
    /// Manually set a session's outcome ("victory"/"defeat"/"draw"/"unknown"),
    /// back-filled onto all its snapshots and re-queued for sync.
    SetOutcome { session_id: String, outcome: String },
    /// Remove a session and all its snapshots.
    DeleteSession { session_id: String },
}

fn commands_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("commands")
}

/// Queue a command for the daemon to apply (atomic via tmp+rename).
pub fn queue_command(
    data_dir: &Path,
    cmd: &StoreCommand,
) -> Result<(), Box<dyn std::error::Error>> {
    let dir = commands_dir(data_dir);
    std::fs::create_dir_all(&dir)?;
    let name = format!(
        "cmd_{}_{}.json",
        chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default(),
        std::process::id()
    );
    let tmp = dir.join(format!("{name}.tmp"));
    std::fs::write(&tmp, serde_json::to_vec(cmd)?)?;
    std::fs::rename(&tmp, dir.join(name))?;
    Ok(())
}

/// Take all queued commands, oldest first, removing them from disk.
/// Unparseable files are discarded (logged), never retried forever.
pub fn take_commands(data_dir: &Path) -> Vec<StoreCommand> {
    let Ok(entries) = std::fs::read_dir(commands_dir(data_dir)) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .flatten()
        .map(|e| e.path())
        .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("json"))
        .collect();
    files.sort();
    let mut cmds = Vec::new();
    for f in files {
        match std::fs::read(&f)
            .map_err(|e| e.to_string())
            .and_then(|c| serde_json::from_slice::<StoreCommand>(&c).map_err(|e| e.to_string()))
        {
            Ok(cmd) => cmds.push(cmd),
            Err(e) => {
                tracing::warn!(file = %f.display(), error = %e, "discarding bad command file")
            }
        }
        let _ = std::fs::remove_file(&f);
    }
    cmds
}

/// The majority hero across a session's snapshots. Individual captures can
/// mislabel — the career panel shows the SPECTATED hero while the player is
/// dead, and portrait matching can misfire — but across 20+ captures the
/// player's real hero dominates. "Unknown" rows don't vote; ties break
/// alphabetically for determinism.
pub fn majority_hero(snapshots: &[PersonalMatch]) -> Option<String> {
    let mut counts: std::collections::HashMap<&str, usize> = std::collections::HashMap::new();
    for m in snapshots {
        if m.hero != "Unknown" && !m.hero.is_empty() {
            *counts.entry(m.hero.as_str()).or_default() += 1;
        }
    }
    counts
        .into_iter()
        .max_by(|a, b| a.1.cmp(&b.1).then(b.0.cmp(a.0)))
        .map(|(hero, _)| hero.to_string())
}

/// Collapse capture snapshots (multiple Tab presses during one match) to one
/// row per game. Snapshots of the same game share a `session_id`, and the
/// newest snapshot carries the final scoreboard — so with newest-first input
/// (the order of `get_all_matches`, the live snapshot, and `read_match_log`)
/// the first row seen per session wins. Rows without a session_id (legacy
/// data) pass through individually.
pub fn latest_per_game(matches: Vec<PersonalMatch>) -> Vec<PersonalMatch> {
    let mut seen = std::collections::HashSet::new();
    let mut out = matches;
    out.retain(|m| m.session_id.is_empty() || seen.insert(m.session_id.clone()));
    out
}

/// Remove the on-disk exports (append-only log + live snapshot). Called by
/// every clear path — a stale snapshot would resurrect cleared data in the
/// GUI's locked-store view.
pub fn clear_match_log(data_dir: &Path) {
    for path in [match_log_path(data_dir), snapshot_path(data_dir)] {
        if path.exists() {
            let _ = std::fs::remove_file(&path);
        }
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

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Utc;

    fn snap(session_id: &str, elims: u32) -> PersonalMatch {
        PersonalMatch {
            hero: "Test".into(),
            map_name: String::new(),
            game_mode: String::new(),
            role: "Tank".into(),
            outcome: "victory".into(),
            elims,
            deaths: 0,
            assists: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
            played_at: SurrealDatetime::from(Utc::now()),
            synced: false,
            session_id: session_id.into(),
        }
    }

    #[test]
    fn latest_per_game_keeps_final_snapshot_per_session() {
        // Newest-first input: the first snapshot per session is the final one.
        let rows = vec![snap("b", 30), snap("a", 20), snap("a", 10), snap("a", 5)];
        let games = latest_per_game(rows);
        assert_eq!(games.len(), 2);
        assert_eq!(games[0].session_id, "b");
        assert_eq!(games[1].elims, 20);
    }

    #[test]
    fn command_queue_roundtrip() {
        let dir = tempfile::tempdir().expect("tempdir");
        let a = StoreCommand::SetOutcome {
            session_id: "s1".into(),
            outcome: "defeat".into(),
        };
        let b = StoreCommand::DeleteSession {
            session_id: "s2".into(),
        };
        queue_command(dir.path(), &a).unwrap();
        queue_command(dir.path(), &b).unwrap();
        let cmds = take_commands(dir.path());
        assert_eq!(cmds.len(), 2);
        assert!(
            matches!(&cmds[0], StoreCommand::SetOutcome { session_id, outcome }
            if session_id == "s1" && outcome == "defeat")
        );
        assert!(
            matches!(&cmds[1], StoreCommand::DeleteSession { session_id }
            if session_id == "s2")
        );
        // Consumed — second take is empty.
        assert!(take_commands(dir.path()).is_empty());
    }

    #[test]
    fn majority_hero_ignores_unknown_and_picks_dominant() {
        let mut rows: Vec<PersonalMatch> = (0..5).map(|_| snap("a", 1)).collect();
        rows[0].hero = "illari".into();
        rows[1].hero = "Unknown".into();
        for r in rows.iter_mut().skip(2) {
            r.hero = "Wrecking Ball".into();
        }
        assert_eq!(majority_hero(&rows).as_deref(), Some("Wrecking Ball"));
        assert_eq!(majority_hero(&[]), None);
    }

    #[test]
    fn latest_per_game_passes_legacy_rows_through() {
        let rows = vec![snap("", 1), snap("", 2), snap("a", 3), snap("a", 4)];
        assert_eq!(latest_per_game(rows).len(), 3);
    }
}
