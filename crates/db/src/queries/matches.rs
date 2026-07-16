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
    #[surreal(default)]
    score_us: Option<i64>,
    #[surreal(default)]
    score_them: Option<i64>,
    map_name: Option<String>,
    game_mode: Option<String>,
    match_type: String,
    #[surreal(default)]
    played_at: Option<SurrealDatetime>,
    #[surreal(default)]
    scheduled_at: Option<SurrealDatetime>,
    #[surreal(default)]
    recorded_by: Option<String>,
    notes: Option<String>,
    #[surreal(default)]
    is_public: bool,
    #[surreal(default)]
    vod_url: Option<String>,
    #[surreal(default)]
    replay_code: Option<String>,
}

fn parse_match_type(s: &str) -> MatchType {
    match s {
        "official" => MatchType::Official,
        "tournament" => MatchType::Tournament,
        _ => MatchType::Scrim,
    }
}

fn opt_i64_to_u32(v: Option<i64>) -> Option<u32> {
    v.and_then(|n| u32::try_from(n).ok())
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
        score_us: opt_i64_to_u32(db.score_us),
        score_them: opt_i64_to_u32(db.score_them),
        map_name: db.map_name,
        game_mode: db.game_mode,
        match_type: parse_match_type(&db.match_type),
        played_at: db.played_at.map(|d| d.into()),
        scheduled_at: db.scheduled_at.map(|d| d.into()),
        recorded_by: db.recorded_by,
        notes: db.notes,
        is_public: db.is_public,
        vod_url: db.vod_url,
        replay_code: db.replay_code,
    }
}

impl Database {
    pub async fn record_match(
        &self,
        team_id: &str,
        opponent: &str,
        score_us: Option<u32>,
        score_them: Option<u32>,
        map_name: Option<&str>,
        game_mode: Option<&str>,
        match_type: MatchType,
        played_at: Option<DateTime<Utc>>,
        scheduled_at: Option<DateTime<Utc>>,
        recorded_by: &str,
        notes: Option<&str>,
        is_public: bool,
        vod_url: Option<&str>,
        replay_code: Option<&str>,
    ) -> DbResult<MatchResult> {
        with_timeout(async {
            let db_match = DbMatchResult {
                id: None,
                team_id: team_id.to_string(),
                opponent: opponent.to_string(),
                score_us: score_us.map(|s| s as i64),
                score_them: score_them.map(|s| s as i64),
                map_name: map_name.map(|s| s.to_string()),
                game_mode: game_mode.map(|s| s.to_string()),
                match_type: match_type.to_string(),
                played_at: played_at.map(SurrealDatetime::from),
                scheduled_at: scheduled_at.map(SurrealDatetime::from),
                recorded_by: Some(recorded_by.to_string()),
                notes: notes.map(|s| s.to_string()),
                is_public,
                vod_url: vod_url.map(|s| s.to_string()),
                replay_code: replay_code.map(|s| s.to_string()),
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
                .query(
                    "SELECT * FROM match_result WHERE team_id = $tid \
                     ORDER BY played_at DESC, scheduled_at DESC",
                )
                .bind(("tid", team_id.to_string()))
                .await?;
            let matches: Vec<DbMatchResult> = result.take(0)?;
            Ok(matches.into_iter().map(db_to_match).collect())
        })
        .await
    }

    /// List team matches with cursor-based pagination.
    ///
    /// When `only_public` is true, filters to public non-scrim rows in SQL so
    /// LIMIT/START count only publishable matches.
    /// When `only_played` is true, requires `played_at` to be set (recent results).
    pub async fn list_team_matches_paginated(
        &self,
        team_id: &str,
        limit: u32,
        offset: u32,
        only_public: bool,
        only_played: bool,
    ) -> DbResult<Vec<MatchResult>> {
        with_timeout(async {
            let fetch = limit + 1;
            // Fixed SQL fragments only — no user input interpolated.
            let sql = match (only_public, only_played) {
                (true, true) => {
                    "SELECT * FROM match_result WHERE team_id = $tid AND is_public = true \
                     AND match_type != 'scrim' AND played_at != NONE \
                     ORDER BY played_at DESC LIMIT $lim START $off"
                }
                (true, false) => {
                    "SELECT * FROM match_result WHERE team_id = $tid AND is_public = true \
                     AND match_type != 'scrim' \
                     ORDER BY played_at DESC, scheduled_at DESC LIMIT $lim START $off"
                }
                (false, true) => {
                    "SELECT * FROM match_result WHERE team_id = $tid AND played_at != NONE \
                     ORDER BY played_at DESC LIMIT $lim START $off"
                }
                (false, false) => {
                    "SELECT * FROM match_result WHERE team_id = $tid \
                     ORDER BY played_at DESC, scheduled_at DESC LIMIT $lim START $off"
                }
            };
            let mut result = self
                .client
                .query(sql)
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
        score_us: Option<Option<u32>>,
        score_them: Option<Option<u32>>,
        map_name: Option<Option<&str>>,
        game_mode: Option<Option<&str>>,
        match_type: Option<MatchType>,
        notes: Option<Option<&str>>,
        is_public: Option<bool>,
        played_at: Option<Option<DateTime<Utc>>>,
        scheduled_at: Option<Option<DateTime<Utc>>>,
        vod_url: Option<Option<&str>>,
        replay_code: Option<Option<&str>>,
    ) -> DbResult<MatchResult> {
        with_timeout(async {
            let existing: Option<DbMatchResult> = self.client.select(("match_result", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Match {id} not found")))?;

            if let Some(o) = opponent {
                db.opponent = o.to_string();
            }
            if let Some(s) = score_us {
                db.score_us = s.map(|v| v as i64);
            }
            if let Some(s) = score_them {
                db.score_them = s.map(|v| v as i64);
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
            if let Some(pa) = played_at {
                db.played_at = pa.map(SurrealDatetime::from);
            }
            if let Some(sa) = scheduled_at {
                db.scheduled_at = sa.map(SurrealDatetime::from);
            }
            if let Some(v) = vod_url {
                db.vod_url = v.map(|s| s.to_string());
            }
            if let Some(r) = replay_code {
                db.replay_code = r.map(|s| s.to_string());
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

            // Only count completed matches (both scores present).
            let mut wins_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid \
                     AND score_us != NONE AND score_them != NONE \
                     AND score_us > score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .await?;
            let wins: Vec<CountResult> = wins_result.take(0)?;

            let mut losses_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid \
                     AND score_us != NONE AND score_them != NONE \
                     AND score_us < score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .await?;
            let losses: Vec<CountResult> = losses_result.take(0)?;

            let mut draws_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid \
                     AND score_us != NONE AND score_them != NONE \
                     AND score_us = score_them GROUP ALL",
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
                    "SELECT count() FROM match_result WHERE team_id = $tid AND match_type = $mt \
                     AND score_us != NONE AND score_them != NONE \
                     AND score_us > score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .bind(("mt", mt.clone()))
                .await?;
            let wins: Vec<CountResult> = wins_result.take(0)?;

            let mut losses_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND match_type = $mt \
                     AND score_us != NONE AND score_them != NONE \
                     AND score_us < score_them GROUP ALL",
                )
                .bind(("tid", team_id.to_string()))
                .bind(("mt", mt.clone()))
                .await?;
            let losses: Vec<CountResult> = losses_result.take(0)?;

            let mut draws_result = self
                .client
                .query(
                    "SELECT count() FROM match_result WHERE team_id = $tid AND match_type = $mt \
                     AND score_us != NONE AND score_them != NONE \
                     AND score_us = score_them GROUP ALL",
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
