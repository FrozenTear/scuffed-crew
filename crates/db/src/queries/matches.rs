use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{MatchResult, MatchType, TeamRecord};
use crate::{with_timeout, Database, DbResult};

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbMatchResult {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    team_id: String,
    opponent: String,
    score_us: u32,
    score_them: u32,
    map_name: Option<String>,
    game_mode: Option<String>,
    match_type: String,
    played_at: SurrealDatetime,
    recorded_by: String,
    notes: Option<String>,
    #[surreal(default)]
    is_public: bool,
}

fn parse_match_type(s: &str) -> MatchType {
    match s {
        "official" => MatchType::Official,
        "tournament" => MatchType::Tournament,
        _ => MatchType::Scrim,
    }
}

fn db_to_match(db: DbMatchResult) -> MatchResult {
    let id = db
        .id
        .map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string());
    MatchResult {
        id,
        team_id: db.team_id,
        opponent: db.opponent,
        score_us: db.score_us,
        score_them: db.score_them,
        map_name: db.map_name,
        game_mode: db.game_mode,
        match_type: parse_match_type(&db.match_type),
        played_at: db.played_at.into(),
        recorded_by: db.recorded_by,
        notes: db.notes,
        is_public: db.is_public,
    }
}

impl Database {
    pub async fn record_match(
        &self,
        team_id: &str,
        opponent: &str,
        score_us: u32,
        score_them: u32,
        map_name: Option<&str>,
        game_mode: Option<&str>,
        match_type: MatchType,
        played_at: DateTime<Utc>,
        recorded_by: &str,
        notes: Option<&str>,
    ) -> DbResult<MatchResult> {
        with_timeout(async {
            let db_match = DbMatchResult {
                id: None,
                team_id: team_id.to_string(),
                opponent: opponent.to_string(),
                score_us,
                score_them,
                map_name: map_name.map(|s| s.to_string()),
                game_mode: game_mode.map(|s| s.to_string()),
                match_type: match_type.to_string(),
                played_at: SurrealDatetime::from(played_at),
                recorded_by: recorded_by.to_string(),
                notes: notes.map(|s| s.to_string()),
                // Private by default; officers publish via update.
                is_public: false,
            };
            let created: Option<DbMatchResult> =
                self.client.create("match_result").content(db_match).await?;
            Ok(db_to_match(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to record match".into())
            })?))
        })
        .await
    }

    pub async fn list_team_matches(&self, team_id: &str) -> DbResult<Vec<MatchResult>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM match_result WHERE team_id = $tid ORDER BY played_at DESC")
                .bind(("tid", team_id.to_string()))
                .await?;
            let matches: Vec<DbMatchResult> = result.take(0)?;
            Ok(matches.into_iter().map(db_to_match).collect())
        })
        .await
    }

    /// List team matches with cursor-based pagination.
    pub async fn list_team_matches_paginated(
        &self,
        team_id: &str,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<MatchResult>> {
        with_timeout(async {
            let fetch = limit + 1;
            let mut result = self
                .client
                .query(
                    "SELECT * FROM match_result WHERE team_id = $tid ORDER BY played_at DESC LIMIT $lim START $off",
                )
                .bind(("tid", team_id.to_string()))
                .bind(("lim", fetch))
                .bind(("off", offset))
                .await?;
            let matches: Vec<DbMatchResult> = result.take(0)?;
            Ok(matches.into_iter().map(db_to_match).collect())
        })
        .await
    }

    pub async fn update_match(
        &self,
        id: &str,
        opponent: Option<&str>,
        score_us: Option<u32>,
        score_them: Option<u32>,
        map_name: Option<Option<&str>>,
        game_mode: Option<Option<&str>>,
        match_type: Option<MatchType>,
        notes: Option<Option<&str>>,
        is_public: Option<bool>,
    ) -> DbResult<MatchResult> {
        with_timeout(async {
            let existing: Option<DbMatchResult> = self.client.select(("match_result", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Match {id} not found")))?;

            if let Some(o) = opponent {
                db.opponent = o.to_string();
            }
            if let Some(s) = score_us {
                db.score_us = s;
            }
            if let Some(s) = score_them {
                db.score_them = s;
            }
            if let Some(m) = map_name {
                db.map_name = m.map(|s| s.to_string());
            }
            if let Some(g) = game_mode {
                db.game_mode = g.map(|s| s.to_string());
            }
            if let Some(mt) = match_type {
                db.match_type = mt.to_string();
            }
            if let Some(n) = notes {
                db.notes = n.map(|s| s.to_string());
            }
            if let Some(p) = is_public {
                db.is_public = p;
            }

            let updated: Option<DbMatchResult> =
                self.client.update(("match_result", id)).content(db).await?;
            Ok(db_to_match(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Match {id} not found after update"))
            })?))
        })
        .await
    }

    pub async fn get_team_record(&self, team_id: &str) -> DbResult<TeamRecord> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u32,
            }

            let mut wins_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND score_us > score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .await?;
            let wins: Vec<CountResult> = wins_result.take(0)?;

            let mut losses_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND score_us < score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .await?;
            let losses: Vec<CountResult> = losses_result.take(0)?;

            let mut draws_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND score_us = score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .await?;
            let draws: Vec<CountResult> = draws_result.take(0)?;

            Ok(TeamRecord {
                wins: wins.first().map(|c| c.count).unwrap_or(0),
                losses: losses.first().map(|c| c.count).unwrap_or(0),
                draws: draws.first().map(|c| c.count).unwrap_or(0),
            })
        })
        .await
    }

    pub async fn get_team_record_by_type(
        &self,
        team_id: &str,
        match_type: MatchType,
    ) -> DbResult<TeamRecord> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u32,
            }

            let mt = match_type.to_string();

            let mut wins_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND match_type = $mt AND score_us > score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .bind(("mt", mt.clone()))
                .await?;
            let wins: Vec<CountResult> = wins_result.take(0)?;

            let mut losses_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND match_type = $mt AND score_us < score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .bind(("mt", mt.clone()))
                .await?;
            let losses: Vec<CountResult> = losses_result.take(0)?;

            let mut draws_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND match_type = $mt AND score_us = score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .bind(("mt", mt))
                .await?;
            let draws: Vec<CountResult> = draws_result.take(0)?;

            Ok(TeamRecord {
                wins: wins.first().map(|c| c.count).unwrap_or(0),
                losses: losses.first().map(|c| c.count).unwrap_or(0),
                draws: draws.first().map(|c| c.count).unwrap_or(0),
            })
        })
        .await
    }
}
