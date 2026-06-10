use chrono::Utc;
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::{RecordId, SurrealValue};
use uuid::Uuid;

use scuffed_types::strategy::{
    CoordinateVersion, GameMode, Strategy, StrategyElement, StrategySummary, TimelinePhase,
    Visibility,
};

use crate::{with_timeout, Database, DbError, DbResult};

// =============================================================================
// Internal DB struct
// =============================================================================

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbStrategy {
    #[surreal(default)]
    id: Option<RecordId>,
    name: String,
    description: Option<String>,
    map_id: String,
    sub_map_id: Option<String>,
    game_mode: String,
    owner_id: String,
    team_id: Option<String>,
    visibility: String,
    elements: serde_json::Value,
    phases: serde_json::Value,
    coordinate_version: String,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

/// Lightweight projection for list queries that include computed fields.
#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbStrategySummary {
    #[surreal(default)]
    id: Option<RecordId>,
    name: String,
    map_id: String,
    sub_map_id: Option<String>,
    game_mode: String,
    owner_id: String,
    visibility: String,
    element_count: i64,
    updated_at: SurrealDatetime,
    #[surreal(default)]
    owner_name: Option<String>,
}

// =============================================================================
// Conversion helpers
// =============================================================================

fn parse_game_mode(s: &str) -> GameMode {
    match s {
        "escort" => GameMode::Escort,
        "hybrid" => GameMode::Hybrid,
        "control" => GameMode::Control,
        "push" => GameMode::Push,
        "flashpoint" => GameMode::Flashpoint,
        "clash" => GameMode::Clash,
        "payload_race" => GameMode::PayloadRace,
        "assault" => GameMode::Assault,
        _ => GameMode::Control,
    }
}

fn game_mode_to_string(gm: GameMode) -> String {
    match gm {
        GameMode::Escort => "escort",
        GameMode::Hybrid => "hybrid",
        GameMode::Control => "control",
        GameMode::Push => "push",
        GameMode::Flashpoint => "flashpoint",
        GameMode::Clash => "clash",
        GameMode::PayloadRace => "payload_race",
        GameMode::Assault => "assault",
    }
    .to_string()
}

fn parse_visibility(s: &str) -> Visibility {
    match s {
        "private" => Visibility::Private,
        "unlisted" => Visibility::Unlisted,
        "public" => Visibility::Public,
        _ => Visibility::Private,
    }
}

fn visibility_to_string(v: Visibility) -> String {
    match v {
        Visibility::Private => "private",
        Visibility::Unlisted => "unlisted",
        Visibility::Public => "public",
    }
    .to_string()
}

fn parse_coordinate_version(s: &str) -> CoordinateVersion {
    match s {
        "v1" => CoordinateVersion::V1,
        _ => CoordinateVersion::V2,
    }
}

fn coordinate_version_to_string(v: CoordinateVersion) -> String {
    match v {
        CoordinateVersion::V1 => "v1",
        CoordinateVersion::V2 => "v2",
    }
    .to_string()
}

fn extract_id(rid: Option<RecordId>) -> String {
    rid.map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string())
}

fn db_to_strategy(db: DbStrategy) -> Result<Strategy, DbError> {
    let elements: Vec<StrategyElement> = serde_json::from_value(db.elements)
        .map_err(|e| DbError::Config(format!("Failed to deserialize strategy elements: {e}")))?;
    let phases: Vec<TimelinePhase> = serde_json::from_value(db.phases)
        .map_err(|e| DbError::Config(format!("Failed to deserialize strategy phases: {e}")))?;

    Ok(Strategy {
        id: extract_id(db.id),
        name: db.name,
        description: db.description,
        map_id: db.map_id,
        sub_map_id: db.sub_map_id,
        game_mode: parse_game_mode(&db.game_mode),
        owner_id: db.owner_id,
        team_id: db.team_id,
        visibility: parse_visibility(&db.visibility),
        elements,
        phases,
        coordinate_version: parse_coordinate_version(&db.coordinate_version),
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
    })
}

fn db_summary_to_summary(db: DbStrategySummary) -> StrategySummary {
    StrategySummary {
        id: extract_id(db.id),
        name: db.name,
        map_id: db.map_id,
        sub_map_id: db.sub_map_id,
        game_mode: parse_game_mode(&db.game_mode),
        owner_name: db.owner_name.unwrap_or(db.owner_id),
        visibility: parse_visibility(&db.visibility),
        element_count: db.element_count as usize,
        updated_at: db.updated_at.into(),
    }
}

// =============================================================================
// Database methods
// =============================================================================

impl Database {
    /// Create a new strategy.
    pub async fn create_strategy(
        &self,
        name: &str,
        description: Option<&str>,
        map_id: &str,
        sub_map_id: Option<&str>,
        game_mode: GameMode,
        owner_id: &str,
        team_id: Option<&str>,
        visibility: Visibility,
    ) -> DbResult<Strategy> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db = DbStrategy {
                id: None,
                name: name.to_string(),
                description: description.map(|s| s.to_string()),
                map_id: map_id.to_string(),
                sub_map_id: sub_map_id.map(|s| s.to_string()),
                game_mode: game_mode_to_string(game_mode),
                owner_id: owner_id.to_string(),
                team_id: team_id.map(|s| s.to_string()),
                visibility: visibility_to_string(visibility),
                elements: serde_json::Value::Array(vec![]),
                phases: serde_json::to_value(vec![TimelinePhase::default()])
                    .unwrap_or(serde_json::Value::Array(vec![])),
                coordinate_version: coordinate_version_to_string(CoordinateVersion::V2),
                created_at: now,
                updated_at: now,
            };

            let created: Option<DbStrategy> = self.client.create("strategy").content(db).await?;
            db_to_strategy(
                created.ok_or_else(|| DbError::NotFound("Failed to create strategy".into()))?,
            )
        })
        .await
    }

    /// Get a strategy by ID.
    pub async fn get_strategy(&self, id: &str) -> DbResult<Option<Strategy>> {
        with_timeout(async {
            let db: Option<DbStrategy> = self.client.select(("strategy", id)).await?;
            match db {
                Some(db) => Ok(Some(db_to_strategy(db)?)),
                None => Ok(None),
            }
        })
        .await
    }

    /// Update strategy metadata (not elements/phases).
    pub async fn update_strategy(
        &self,
        id: &str,
        name: Option<&str>,
        description: Option<Option<&str>>,
        visibility: Option<Visibility>,
    ) -> DbResult<Strategy> {
        with_timeout(async {
            let existing: Option<DbStrategy> = self.client.select(("strategy", id)).await?;
            let mut db =
                existing.ok_or_else(|| DbError::NotFound(format!("Strategy {id} not found")))?;

            if let Some(n) = name {
                db.name = n.to_string();
            }
            if let Some(d) = description {
                db.description = d.map(|s| s.to_string());
            }
            if let Some(v) = visibility {
                db.visibility = visibility_to_string(v);
            }
            db.updated_at = SurrealDatetime::from(Utc::now());

            let updated: Option<DbStrategy> =
                self.client.update(("strategy", id)).content(db).await?;
            db_to_strategy(updated.ok_or_else(|| {
                DbError::NotFound(format!("Strategy {id} not found after update"))
            })?)
        })
        .await
    }

    /// Delete a strategy.
    pub async fn delete_strategy(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            let _: Option<DbStrategy> = self.client.delete(("strategy", id)).await?;
            Ok(())
        })
        .await
    }

    /// List strategies owned by a specific user.
    pub async fn get_user_strategies(&self, owner_id: &str) -> DbResult<Vec<StrategySummary>> {
        with_timeout(async {
            let mut result = self
                .client
                .query(
                    "SELECT *, array::len(elements) as element_count \
                     FROM strategy WHERE owner_id = $owner \
                     ORDER BY updated_at DESC",
                )
                .bind(("owner", owner_id.to_string()))
                .await?;
            let rows: Vec<DbStrategySummary> = result.take(0)?;
            Ok(rows.into_iter().map(db_summary_to_summary).collect())
        })
        .await
    }

    /// List public strategies with optional filters.
    pub async fn get_public_strategies(
        &self,
        search: Option<&str>,
        game_mode: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> DbResult<(Vec<StrategySummary>, u64)> {
        with_timeout(async {
            // Build WHERE clause dynamically with bound placeholders (never
            // interpolate user input — see SurrealDB gotchas in CLAUDE.md)
            let mut conditions = vec!["visibility = 'public'".to_string()];
            if game_mode.is_some() {
                conditions.push("game_mode = $game_mode".to_string());
            }
            if search.is_some() {
                // Simple CONTAINS search on name
                conditions
                    .push("string::lowercase(name) CONTAINS string::lowercase($search)".to_string());
            }
            let where_clause = conditions.join(" AND ");

            // Count total
            let count_query =
                format!("SELECT count() as total FROM strategy WHERE {where_clause} GROUP ALL");
            let mut count_q = self.client.query(&count_query);
            if let Some(gm) = game_mode {
                count_q = count_q.bind(("game_mode", gm.to_string()));
            }
            if let Some(q) = search {
                count_q = count_q.bind(("search", q.to_string()));
            }
            let mut count_result = count_q.await?;

            #[derive(Debug, Deserialize, SurrealValue)]
            struct CountRow {
                total: i64,
            }
            let count_rows: Vec<CountRow> = count_result.take(0)?;
            let total = count_rows.first().map(|r| r.total as u64).unwrap_or(0);

            // Fetch page
            let data_query = format!(
                "SELECT *, array::len(elements) as element_count \
                 FROM strategy WHERE {where_clause} \
                 ORDER BY updated_at DESC LIMIT $lim START $off"
            );
            let mut data_q = self
                .client
                .query(&data_query)
                .bind(("lim", limit as i64))
                .bind(("off", offset as i64));
            if let Some(gm) = game_mode {
                data_q = data_q.bind(("game_mode", gm.to_string()));
            }
            if let Some(q) = search {
                data_q = data_q.bind(("search", q.to_string()));
            }
            let mut data_result = data_q.await?;
            let rows: Vec<DbStrategySummary> = data_result.take(0)?;

            Ok((rows.into_iter().map(db_summary_to_summary).collect(), total))
        })
        .await
    }

    /// Save full strategy content (elements + phases). Used by the editor save action.
    pub async fn save_full_strategy(
        &self,
        id: &str,
        elements: &[StrategyElement],
        phases: &[TimelinePhase],
    ) -> DbResult<()> {
        with_timeout(async {
            let elements_json = serde_json::to_value(elements)
                .map_err(|e| DbError::Config(format!("Failed to serialize elements: {e}")))?;
            let phases_json = serde_json::to_value(phases)
                .map_err(|e| DbError::Config(format!("Failed to serialize phases: {e}")))?;

            self.client
                .query(
                    "UPDATE $rid SET elements = $elements, phases = $phases, \
                     updated_at = time::now()",
                )
                .bind(("rid", RecordId::new("strategy", id)))
                .bind(("elements", elements_json))
                .bind(("phases", phases_json))
                .await?
                .check()?;
            Ok(())
        })
        .await
    }

    /// Add an element to a strategy (for WS collab persistence).
    pub async fn add_strategy_element(
        &self,
        strategy_id: &str,
        element: &StrategyElement,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbStrategy> =
                self.client.select(("strategy", strategy_id)).await?;
            let mut db = existing
                .ok_or_else(|| DbError::NotFound(format!("Strategy {strategy_id} not found")))?;

            let mut elements: Vec<serde_json::Value> =
                serde_json::from_value(db.elements.clone()).unwrap_or_default();
            let elem_json = serde_json::to_value(element)
                .map_err(|e| DbError::Config(format!("Failed to serialize element: {e}")))?;
            elements.push(elem_json);

            db.elements = serde_json::Value::Array(elements);
            db.updated_at = SurrealDatetime::from(Utc::now());

            let _: Option<DbStrategy> = self
                .client
                .update(("strategy", strategy_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    /// Update an element within a strategy (for WS collab persistence).
    pub async fn update_strategy_element(
        &self,
        strategy_id: &str,
        element_id: Uuid,
        element: &StrategyElement,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbStrategy> =
                self.client.select(("strategy", strategy_id)).await?;
            let mut db = existing
                .ok_or_else(|| DbError::NotFound(format!("Strategy {strategy_id} not found")))?;

            let mut elements: Vec<StrategyElement> =
                serde_json::from_value(db.elements.clone()).unwrap_or_default();
            if let Some(existing_elem) = elements.iter_mut().find(|e| e.id == element_id) {
                *existing_elem = element.clone();
            }

            db.elements =
                serde_json::to_value(&elements).unwrap_or(serde_json::Value::Array(vec![]));
            db.updated_at = SurrealDatetime::from(Utc::now());

            let _: Option<DbStrategy> = self
                .client
                .update(("strategy", strategy_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    /// Delete an element from a strategy (for WS collab persistence).
    pub async fn delete_strategy_element(
        &self,
        strategy_id: &str,
        element_id: Uuid,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbStrategy> =
                self.client.select(("strategy", strategy_id)).await?;
            let mut db = existing
                .ok_or_else(|| DbError::NotFound(format!("Strategy {strategy_id} not found")))?;

            let mut elements: Vec<StrategyElement> =
                serde_json::from_value(db.elements.clone()).unwrap_or_default();
            elements.retain(|e| e.id != element_id);

            db.elements =
                serde_json::to_value(&elements).unwrap_or(serde_json::Value::Array(vec![]));
            db.updated_at = SurrealDatetime::from(Utc::now());

            let _: Option<DbStrategy> = self
                .client
                .update(("strategy", strategy_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    /// Add a phase to a strategy (for WS collab persistence).
    pub async fn add_strategy_phase(
        &self,
        strategy_id: &str,
        phase: &TimelinePhase,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbStrategy> =
                self.client.select(("strategy", strategy_id)).await?;
            let mut db = existing
                .ok_or_else(|| DbError::NotFound(format!("Strategy {strategy_id} not found")))?;

            let mut phases: Vec<serde_json::Value> =
                serde_json::from_value(db.phases.clone()).unwrap_or_default();
            let phase_json = serde_json::to_value(phase)
                .map_err(|e| DbError::Config(format!("Failed to serialize phase: {e}")))?;
            phases.push(phase_json);

            db.phases = serde_json::Value::Array(phases);
            db.updated_at = SurrealDatetime::from(Utc::now());

            let _: Option<DbStrategy> = self
                .client
                .update(("strategy", strategy_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    /// Update a phase within a strategy (for WS collab persistence).
    pub async fn update_strategy_phase(
        &self,
        strategy_id: &str,
        phase_id: Uuid,
        phase: &TimelinePhase,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbStrategy> =
                self.client.select(("strategy", strategy_id)).await?;
            let mut db = existing
                .ok_or_else(|| DbError::NotFound(format!("Strategy {strategy_id} not found")))?;

            let mut phases: Vec<TimelinePhase> =
                serde_json::from_value(db.phases.clone()).unwrap_or_default();
            if let Some(existing_phase) = phases.iter_mut().find(|p| p.id == phase_id) {
                *existing_phase = phase.clone();
            }

            db.phases = serde_json::to_value(&phases).unwrap_or(serde_json::Value::Array(vec![]));
            db.updated_at = SurrealDatetime::from(Utc::now());

            let _: Option<DbStrategy> = self
                .client
                .update(("strategy", strategy_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    /// Delete a phase from a strategy (for WS collab persistence).
    pub async fn delete_strategy_phase(&self, strategy_id: &str, phase_id: Uuid) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbStrategy> =
                self.client.select(("strategy", strategy_id)).await?;
            let mut db = existing
                .ok_or_else(|| DbError::NotFound(format!("Strategy {strategy_id} not found")))?;

            let mut phases: Vec<TimelinePhase> =
                serde_json::from_value(db.phases.clone()).unwrap_or_default();
            phases.retain(|p| p.id != phase_id);

            db.phases = serde_json::to_value(&phases).unwrap_or(serde_json::Value::Array(vec![]));
            db.updated_at = SurrealDatetime::from(Utc::now());

            let _: Option<DbStrategy> = self
                .client
                .update(("strategy", strategy_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    /// Check if a user can access a strategy (view).
    pub async fn can_access_strategy(&self, id: &str, user_id: Option<&str>) -> DbResult<bool> {
        let strategy = self.get_strategy(id).await?;
        match strategy {
            None => Ok(false),
            Some(s) => match s.visibility {
                Visibility::Public | Visibility::Unlisted => Ok(true),
                Visibility::Private => Ok(user_id == Some(s.owner_id.as_str())),
            },
        }
    }

    /// Check if a user can edit a strategy (must be owner).
    pub async fn can_edit_strategy(&self, id: &str, user_id: &str) -> DbResult<bool> {
        let strategy = self.get_strategy(id).await?;
        match strategy {
            None => Ok(false),
            Some(s) => Ok(s.owner_id == user_id),
        }
    }
}
