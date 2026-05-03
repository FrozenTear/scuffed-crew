use std::path::Path;

use serde::{Deserialize, Serialize};
use surrealdb::engine::local::SurrealKv;
use surrealdb_types::Datetime as SurrealDatetime;
use surrealdb::Surreal;
use surrealdb_types::SurrealValue;

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
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
}

pub struct LocalStore {
    db: Surreal<surrealdb::engine::local::Db>,
}

impl LocalStore {
    pub async fn open(data_dir: &Path) -> Result<Self, Box<dyn std::error::Error>> {
        let db_path = data_dir.join("stats.surrealkv");
        std::fs::create_dir_all(&db_path)?;

        let db = Surreal::new::<SurrealKv>(
            db_path
                .to_str()
                .ok_or("data_dir path is not valid UTF-8")?,
        )
        .await?;
        db.use_ns("stat_tracker").use_db("local").await?;

        db.query(
            "
            DEFINE TABLE IF NOT EXISTS personal_match SCHEMALESS;
            DEFINE INDEX IF NOT EXISTS idx_synced ON personal_match FIELDS synced;
            DEFINE INDEX IF NOT EXISTS idx_played_at ON personal_match FIELDS played_at;
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
        let _: Option<PersonalMatch> = self
            .db
            .create("personal_match")
            .content(match_data)
            .await?;
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
}

#[derive(Deserialize, SurrealValue)]
struct CountRow {
    total: usize,
}
