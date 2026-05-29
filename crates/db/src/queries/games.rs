use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::Game;
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbGame {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    name: String,
    abbreviation: Option<String>,
    is_active: bool,
    created_at: SurrealDatetime,
}

fn db_to_game(db: DbGame) -> Game {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    Game {
        id,
        name: db.name,
        abbreviation: db.abbreviation,
        is_active: db.is_active,
        created_at: db.created_at.into(),
    }
}

impl Database {
    /// Create a new game.
    pub async fn create_game(&self, name: &str, abbreviation: Option<&str>) -> DbResult<Game> {
        with_timeout(async {
            let db_game = DbGame {
                id: None,
                name: name.to_string(),
                abbreviation: abbreviation.map(|s| s.to_string()),
                is_active: true,
                created_at: SurrealDatetime::from(Utc::now()),
            };
            let created: Option<DbGame> = self.client.create("game").content(db_game).await?;
            Ok(db_to_game(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create game".into())
            })?))
        })
        .await
    }

    /// List all active games.
    pub async fn list_games(&self) -> DbResult<Vec<Game>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM game WHERE is_active = true ORDER BY name ASC")
                .await?;
            let games: Vec<DbGame> = result.take(0)?;
            Ok(games.into_iter().map(db_to_game).collect())
        })
        .await
    }

    /// Get a game by ID.
    pub async fn get_game(&self, id: &str) -> DbResult<Option<Game>> {
        with_timeout(async {
            let db_game: Option<DbGame> = self.client.select(("game", id)).await?;
            Ok(db_game.map(db_to_game))
        })
        .await
    }

    /// Update a game's fields.
    pub async fn update_game(
        &self,
        id: &str,
        name: Option<&str>,
        abbreviation: Option<Option<&str>>,
    ) -> DbResult<Game> {
        with_timeout(async {
            let existing: Option<DbGame> = self.client.select(("game", id)).await?;
            let mut db =
                existing.ok_or_else(|| crate::DbError::NotFound(format!("Game {id} not found")))?;

            if let Some(n) = name {
                db.name = n.to_string();
            }
            if let Some(a) = abbreviation {
                db.abbreviation = a.map(|s| s.to_string());
            }

            let updated: Option<DbGame> = self.client.update(("game", id)).content(db).await?;
            Ok(db_to_game(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Game {id} not found after update"))
            })?))
        })
        .await
    }
}
