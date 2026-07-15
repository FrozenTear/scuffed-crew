use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use surrealdb::types::Datetime as SurrealDatetime;
use surrealdb_types::RecordId;
use surrealdb_types::SurrealValue;

use crate::types::{
    BracketStage, ParticipantStatus, RoundStatus, SwissStanding, Tournament, TournamentBracket,
    TournamentFormat, TournamentMatch, TournamentMatchStatus, TournamentParticipant,
    TournamentRound, TournamentStatus,
};
use crate::{with_timeout, Database, DbResult};

// ─── Internal DB Structs ───

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbTournament {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    name: String,
    game_id: Option<String>,
    format: String,
    status: String,
    max_teams: Option<u32>,
    best_of: u32,
    swiss_rounds: Option<u32>,
    is_external: bool,
    is_open: bool,
    external_url: Option<String>,
    rules: Option<String>,
    description: Option<String>,
    starts_at: Option<SurrealDatetime>,
    ends_at: Option<SurrealDatetime>,
    created_by: String,
    created_at: SurrealDatetime,
    updated_at: SurrealDatetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbTournamentParticipant {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    tournament_id: String,
    team_id: Option<String>,
    external_name: Option<String>,
    seed: Option<u32>,
    group_label: Option<String>,
    status: String,
    created_at: SurrealDatetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbTournamentRound {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    tournament_id: String,
    round_number: u32,
    stage: String,
    status: String,
    created_at: SurrealDatetime,
}

#[derive(Debug, Clone, Serialize, Deserialize, SurrealValue)]
struct DbTournamentMatch {
    #[surreal(default)]
    #[allow(dead_code)]
    id: Option<RecordId>,
    tournament_id: String,
    round_id: String,
    bracket_position: u32,
    participant_a_id: Option<String>,
    participant_b_id: Option<String>,
    score_a: Option<u32>,
    score_b: Option<u32>,
    winner_id: Option<String>,
    status: String,
    scheduled_at: Option<SurrealDatetime>,
    completed_at: Option<SurrealDatetime>,
    match_result_id: Option<String>,
    next_match_id: Option<String>,
    next_match_slot: Option<String>,
    loser_next_match_id: Option<String>,
    loser_next_match_slot: Option<String>,
    notes: Option<String>,
    #[serde(default)]
    replay_codes: Vec<String>,
}

// ─── Conversion Helpers ───

fn parse_format(s: &str) -> TournamentFormat {
    match s {
        "double_elim" => TournamentFormat::DoubleElim,
        "round_robin" => TournamentFormat::RoundRobin,
        "swiss" => TournamentFormat::Swiss,
        _ => TournamentFormat::SingleElim,
    }
}

fn parse_status(s: &str) -> TournamentStatus {
    match s {
        "registration" => TournamentStatus::Registration,
        "in_progress" => TournamentStatus::InProgress,
        "completed" => TournamentStatus::Completed,
        "archived" => TournamentStatus::Archived,
        _ => TournamentStatus::Draft,
    }
}

fn parse_participant_status(s: &str) -> ParticipantStatus {
    match s {
        "checked_in" => ParticipantStatus::CheckedIn,
        "active" => ParticipantStatus::Active,
        "eliminated" => ParticipantStatus::Eliminated,
        "withdrawn" => ParticipantStatus::Withdrawn,
        "disqualified" => ParticipantStatus::Disqualified,
        _ => ParticipantStatus::Registered,
    }
}

fn parse_stage(s: &str) -> BracketStage {
    match s {
        "winners" => BracketStage::Winners,
        "losers" => BracketStage::Losers,
        "grand_final" => BracketStage::GrandFinal,
        "group" => BracketStage::Group,
        _ => BracketStage::Main,
    }
}

fn parse_round_status(s: &str) -> RoundStatus {
    match s {
        "in_progress" => RoundStatus::InProgress,
        "completed" => RoundStatus::Completed,
        _ => RoundStatus::Pending,
    }
}

fn parse_match_status(s: &str) -> TournamentMatchStatus {
    match s {
        "scheduled" => TournamentMatchStatus::Scheduled,
        "in_progress" => TournamentMatchStatus::InProgress,
        "completed" => TournamentMatchStatus::Completed,
        "bye" => TournamentMatchStatus::Bye,
        _ => TournamentMatchStatus::Pending,
    }
}

fn thing_to_id(t: Option<RecordId>) -> String {
    t.map(|r| crate::record_id_key_to_string(r.key))
        .unwrap_or_else(|| "unknown".to_string())
}

fn db_to_tournament(db: DbTournament) -> Tournament {
    Tournament {
        id: thing_to_id(db.id),
        name: db.name,
        game_id: db.game_id,
        format: parse_format(&db.format),
        status: parse_status(&db.status),
        max_teams: db.max_teams,
        best_of: db.best_of,
        swiss_rounds: db.swiss_rounds,
        is_external: db.is_external,
        is_open: db.is_open,
        external_url: db.external_url,
        rules: db.rules,
        description: db.description,
        starts_at: db.starts_at.map(|d| d.into()),
        ends_at: db.ends_at.map(|d| d.into()),
        created_by: db.created_by,
        created_at: db.created_at.into(),
        updated_at: db.updated_at.into(),
    }
}

fn db_to_participant(db: DbTournamentParticipant) -> TournamentParticipant {
    TournamentParticipant {
        id: thing_to_id(db.id),
        tournament_id: db.tournament_id,
        team_id: db.team_id,
        external_name: db.external_name,
        seed: db.seed,
        group_label: db.group_label,
        status: parse_participant_status(&db.status),
        created_at: db.created_at.into(),
    }
}

fn db_to_round(db: DbTournamentRound) -> TournamentRound {
    TournamentRound {
        id: thing_to_id(db.id),
        tournament_id: db.tournament_id,
        round_number: db.round_number,
        stage: parse_stage(&db.stage),
        status: parse_round_status(&db.status),
        created_at: db.created_at.into(),
    }
}

fn db_to_match(db: DbTournamentMatch) -> TournamentMatch {
    TournamentMatch {
        id: thing_to_id(db.id),
        tournament_id: db.tournament_id,
        round_id: db.round_id,
        bracket_position: db.bracket_position,
        participant_a_id: db.participant_a_id,
        participant_b_id: db.participant_b_id,
        score_a: db.score_a,
        score_b: db.score_b,
        winner_id: db.winner_id,
        status: parse_match_status(&db.status),
        scheduled_at: db.scheduled_at.map(|d| d.into()),
        completed_at: db.completed_at.map(|d| d.into()),
        match_result_id: db.match_result_id,
        next_match_id: db.next_match_id,
        next_match_slot: db.next_match_slot,
        loser_next_match_id: db.loser_next_match_id,
        loser_next_match_slot: db.loser_next_match_slot,
        notes: db.notes,
        replay_codes: db.replay_codes,
    }
}

// ─── Database Methods ───

impl Database {
    // ── Tournament CRUD ──

    pub async fn create_tournament(
        &self,
        name: &str,
        game_id: Option<&str>,
        format: TournamentFormat,
        max_teams: Option<u32>,
        best_of: u32,
        swiss_rounds: Option<u32>,
        is_external: bool,
        is_open: bool,
        external_url: Option<&str>,
        rules: Option<&str>,
        description: Option<&str>,
        starts_at: Option<DateTime<Utc>>,
        ends_at: Option<DateTime<Utc>>,
        created_by: &str,
    ) -> DbResult<Tournament> {
        with_timeout(async {
            let now = SurrealDatetime::from(Utc::now());
            let db = DbTournament {
                id: None,
                name: name.to_string(),
                game_id: game_id.map(|s| s.to_string()),
                format: format.to_string(),
                status: "draft".to_string(),
                max_teams,
                best_of,
                swiss_rounds,
                is_external,
                is_open,
                external_url: external_url.map(|s| s.to_string()),
                rules: rules.map(|s| s.to_string()),
                description: description.map(|s| s.to_string()),
                starts_at: starts_at.map(SurrealDatetime::from),
                ends_at: ends_at.map(SurrealDatetime::from),
                created_by: created_by.to_string(),
                created_at: now,
                updated_at: now,
            };
            let created: Option<DbTournament> =
                self.client.create("tournament").content(db).await?;
            Ok(db_to_tournament(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create tournament".into())
            })?))
        })
        .await
    }

    pub async fn get_tournament(&self, id: &str) -> DbResult<Option<Tournament>> {
        with_timeout(async {
            let db: Option<DbTournament> = self.client.select(("tournament", id)).await?;
            Ok(db.map(db_to_tournament))
        })
        .await
    }

    pub async fn list_tournaments(
        &self,
        status: Option<TournamentStatus>,
        game_id: Option<&str>,
    ) -> DbResult<Vec<Tournament>> {
        with_timeout(async {
            let query = match (status, game_id) {
                (Some(s), Some(g)) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament WHERE status = $st AND game_id = $gid ORDER BY created_at DESC")
                        .bind(("st", s.to_string()))
                        .bind(("gid", g.to_string()))
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
                (Some(s), None) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament WHERE status = $st ORDER BY created_at DESC")
                        .bind(("st", s.to_string()))
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
                (None, Some(g)) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament WHERE game_id = $gid ORDER BY created_at DESC")
                        .bind(("gid", g.to_string()))
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
                (None, None) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament ORDER BY created_at DESC")
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
            };
            Ok(query.into_iter().map(db_to_tournament).collect())
        })
        .await
    }

    /// List tournaments with filters and cursor-based pagination.
    pub async fn list_tournaments_paginated(
        &self,
        status: Option<TournamentStatus>,
        game_id: Option<&str>,
        limit: u32,
        offset: u32,
    ) -> DbResult<Vec<Tournament>> {
        with_timeout(async {
            let fetch = limit + 1;
            let query = match (status, game_id) {
                (Some(s), Some(g)) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament WHERE status = $st AND game_id = $gid ORDER BY created_at DESC LIMIT $lim START $off")
                        .bind(("st", s.to_string()))
                        .bind(("gid", g.to_string()))
                        .bind(("lim", fetch))
                        .bind(("off", offset))
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
                (Some(s), None) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament WHERE status = $st ORDER BY created_at DESC LIMIT $lim START $off")
                        .bind(("st", s.to_string()))
                        .bind(("lim", fetch))
                        .bind(("off", offset))
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
                (None, Some(g)) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament WHERE game_id = $gid ORDER BY created_at DESC LIMIT $lim START $off")
                        .bind(("gid", g.to_string()))
                        .bind(("lim", fetch))
                        .bind(("off", offset))
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
                (None, None) => {
                    let mut r = self
                        .client
                        .query("SELECT * FROM tournament ORDER BY created_at DESC LIMIT $lim START $off")
                        .bind(("lim", fetch))
                        .bind(("off", offset))
                        .await?;
                    let items: Vec<DbTournament> = r.take(0)?;
                    items
                }
            };
            Ok(query.into_iter().map(db_to_tournament).collect())
        })
        .await
    }

    pub async fn update_tournament(
        &self,
        id: &str,
        name: Option<&str>,
        game_id: Option<Option<&str>>,
        format: Option<TournamentFormat>,
        max_teams: Option<Option<u32>>,
        best_of: Option<u32>,
        swiss_rounds: Option<Option<u32>>,
        is_external: Option<bool>,
        is_open: Option<bool>,
        external_url: Option<Option<&str>>,
        rules: Option<Option<&str>>,
        description: Option<Option<&str>>,
        starts_at: Option<Option<DateTime<Utc>>>,
        ends_at: Option<Option<DateTime<Utc>>>,
    ) -> DbResult<Tournament> {
        with_timeout(async {
            let existing: Option<DbTournament> = self.client.select(("tournament", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Tournament {id} not found")))?;

            if let Some(n) = name {
                db.name = n.to_string();
            }
            if let Some(g) = game_id {
                db.game_id = g.map(|s| s.to_string());
            }
            if let Some(f) = format {
                db.format = f.to_string();
            }
            if let Some(m) = max_teams {
                db.max_teams = m;
            }
            if let Some(b) = best_of {
                db.best_of = b;
            }
            if let Some(s) = swiss_rounds {
                db.swiss_rounds = s;
            }
            if let Some(e) = is_external {
                db.is_external = e;
            }
            if let Some(o) = is_open {
                db.is_open = o;
            }
            if let Some(u) = external_url {
                db.external_url = u.map(|s| s.to_string());
            }
            if let Some(r) = rules {
                db.rules = r.map(|s| s.to_string());
            }
            if let Some(d) = description {
                db.description = d.map(|s| s.to_string());
            }
            if let Some(s) = starts_at {
                db.starts_at = s.map(SurrealDatetime::from);
            }
            if let Some(e) = ends_at {
                db.ends_at = e.map(SurrealDatetime::from);
            }
            db.updated_at = SurrealDatetime::from(Utc::now());

            let updated: Option<DbTournament> =
                self.client.update(("tournament", id)).content(db).await?;
            Ok(db_to_tournament(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Tournament {id} not found after update"))
            })?))
        })
        .await
    }

    pub async fn update_tournament_status(
        &self,
        id: &str,
        status: TournamentStatus,
    ) -> DbResult<Tournament> {
        with_timeout(async {
            let existing: Option<DbTournament> = self.client.select(("tournament", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Tournament {id} not found")))?;
            db.status = status.to_string();
            db.updated_at = SurrealDatetime::from(Utc::now());

            let updated: Option<DbTournament> =
                self.client.update(("tournament", id)).content(db).await?;
            Ok(db_to_tournament(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Tournament {id} not found after update"))
            })?))
        })
        .await
    }

    // ── Participants ──

    pub async fn add_tournament_participant(
        &self,
        tournament_id: &str,
        team_id: Option<&str>,
        external_name: Option<&str>,
        seed: Option<u32>,
    ) -> DbResult<TournamentParticipant> {
        with_timeout(async {
            let db = DbTournamentParticipant {
                id: None,
                tournament_id: tournament_id.to_string(),
                team_id: team_id.map(|s| s.to_string()),
                external_name: external_name.map(|s| s.to_string()),
                seed,
                group_label: None,
                status: "registered".to_string(),
                created_at: SurrealDatetime::from(Utc::now()),
            };
            let created: Option<DbTournamentParticipant> = self
                .client
                .create("tournament_participant")
                .content(db)
                .await?;
            Ok(db_to_participant(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to add participant".into())
            })?))
        })
        .await
    }

    pub async fn list_tournament_participants(
        &self,
        tournament_id: &str,
    ) -> DbResult<Vec<TournamentParticipant>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM tournament_participant WHERE tournament_id = $tid ORDER BY seed ASC, created_at ASC")
                .bind(("tid", tournament_id.to_string()))
                .await?;
            let items: Vec<DbTournamentParticipant> = result.take(0)?;
            Ok(items.into_iter().map(db_to_participant).collect())
        })
        .await
    }

    pub async fn update_tournament_participant(
        &self,
        id: &str,
        seed: Option<Option<u32>>,
        status: Option<ParticipantStatus>,
        group_label: Option<Option<&str>>,
    ) -> DbResult<TournamentParticipant> {
        with_timeout(async {
            let existing: Option<DbTournamentParticipant> =
                self.client.select(("tournament_participant", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Participant {id} not found")))?;

            if let Some(s) = seed {
                db.seed = s;
            }
            if let Some(st) = status {
                db.status = st.to_string();
            }
            if let Some(g) = group_label {
                db.group_label = g.map(|s| s.to_string());
            }

            let updated: Option<DbTournamentParticipant> = self
                .client
                .update(("tournament_participant", id))
                .content(db)
                .await?;
            Ok(db_to_participant(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Participant {id} not found after update"))
            })?))
        })
        .await
    }

    pub async fn remove_tournament_participant(&self, id: &str) -> DbResult<()> {
        with_timeout(async {
            let _: Option<DbTournamentParticipant> =
                self.client.delete(("tournament_participant", id)).await?;
            Ok(())
        })
        .await
    }

    // ── Rounds ──

    pub async fn create_tournament_round(
        &self,
        tournament_id: &str,
        round_number: u32,
        stage: BracketStage,
    ) -> DbResult<TournamentRound> {
        with_timeout(async {
            let db = DbTournamentRound {
                id: None,
                tournament_id: tournament_id.to_string(),
                round_number,
                stage: stage.to_string(),
                status: "pending".to_string(),
                created_at: SurrealDatetime::from(Utc::now()),
            };
            let created: Option<DbTournamentRound> =
                self.client.create("tournament_round").content(db).await?;
            Ok(db_to_round(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create round".into())
            })?))
        })
        .await
    }

    pub async fn list_tournament_rounds(
        &self,
        tournament_id: &str,
    ) -> DbResult<Vec<TournamentRound>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM tournament_round WHERE tournament_id = $tid ORDER BY stage ASC, round_number ASC")
                .bind(("tid", tournament_id.to_string()))
                .await?;
            let items: Vec<DbTournamentRound> = result.take(0)?;
            Ok(items.into_iter().map(db_to_round).collect())
        })
        .await
    }

    pub async fn update_round_status(
        &self,
        id: &str,
        status: RoundStatus,
    ) -> DbResult<TournamentRound> {
        with_timeout(async {
            let existing: Option<DbTournamentRound> =
                self.client.select(("tournament_round", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Round {id} not found")))?;
            db.status = status.to_string();

            let updated: Option<DbTournamentRound> = self
                .client
                .update(("tournament_round", id))
                .content(db)
                .await?;
            Ok(db_to_round(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Round {id} not found after update"))
            })?))
        })
        .await
    }

    // ── Matches ──

    pub async fn create_tournament_match(
        &self,
        tournament_id: &str,
        round_id: &str,
        bracket_position: u32,
        participant_a_id: Option<&str>,
        participant_b_id: Option<&str>,
        status: TournamentMatchStatus,
        next_match_id: Option<&str>,
        next_match_slot: Option<&str>,
        loser_next_match_id: Option<&str>,
        loser_next_match_slot: Option<&str>,
    ) -> DbResult<TournamentMatch> {
        with_timeout(async {
            let db = DbTournamentMatch {
                id: None,
                tournament_id: tournament_id.to_string(),
                round_id: round_id.to_string(),
                bracket_position,
                participant_a_id: participant_a_id.map(|s| s.to_string()),
                participant_b_id: participant_b_id.map(|s| s.to_string()),
                score_a: None,
                score_b: None,
                winner_id: None,
                status: status.to_string(),
                scheduled_at: None,
                completed_at: None,
                match_result_id: None,
                next_match_id: next_match_id.map(|s| s.to_string()),
                next_match_slot: next_match_slot.map(|s| s.to_string()),
                loser_next_match_id: loser_next_match_id.map(|s| s.to_string()),
                loser_next_match_slot: loser_next_match_slot.map(|s| s.to_string()),
                notes: None,
                replay_codes: vec![],
            };
            let created: Option<DbTournamentMatch> =
                self.client.create("tournament_match").content(db).await?;
            Ok(db_to_match(created.ok_or_else(|| {
                crate::DbError::NotFound("Failed to create match".into())
            })?))
        })
        .await
    }

    pub async fn list_tournament_matches(
        &self,
        tournament_id: &str,
    ) -> DbResult<Vec<TournamentMatch>> {
        with_timeout(async {
            let mut result = self
                .client
                .query("SELECT * FROM tournament_match WHERE tournament_id = $tid ORDER BY bracket_position ASC")
                .bind(("tid", tournament_id.to_string()))
                .await?;
            let items: Vec<DbTournamentMatch> = result.take(0)?;
            Ok(items.into_iter().map(db_to_match).collect())
        })
        .await
    }

    pub async fn get_tournament_match(&self, id: &str) -> DbResult<Option<TournamentMatch>> {
        with_timeout(async {
            let db: Option<DbTournamentMatch> =
                self.client.select(("tournament_match", id)).await?;
            Ok(db.map(db_to_match))
        })
        .await
    }

    pub async fn report_tournament_match(
        &self,
        id: &str,
        score_a: u32,
        score_b: u32,
        winner_id: &str,
        notes: Option<&str>,
        replay_codes: Vec<String>,
    ) -> DbResult<TournamentMatch> {
        with_timeout(async {
            let existing: Option<DbTournamentMatch> =
                self.client.select(("tournament_match", id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Match {id} not found")))?;

            db.score_a = Some(score_a);
            db.score_b = Some(score_b);
            db.winner_id = Some(winner_id.to_string());
            db.status = "completed".to_string();
            db.completed_at = Some(SurrealDatetime::from(Utc::now()));
            db.replay_codes = replay_codes;
            if let Some(n) = notes {
                db.notes = Some(n.to_string());
            }

            let updated: Option<DbTournamentMatch> = self
                .client
                .update(("tournament_match", id))
                .content(db)
                .await?;
            Ok(db_to_match(updated.ok_or_else(|| {
                crate::DbError::NotFound(format!("Match {id} not found after update"))
            })?))
        })
        .await
    }

    /// Update the participant slot on a match (for bracket advancement).
    pub async fn set_match_participant(
        &self,
        match_id: &str,
        slot: &str, // "a" or "b"
        participant_id: &str,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbTournamentMatch> =
                self.client.select(("tournament_match", match_id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Match {match_id} not found")))?;

            match slot {
                "a" => db.participant_a_id = Some(participant_id.to_string()),
                "b" => db.participant_b_id = Some(participant_id.to_string()),
                _ => return Err(crate::DbError::NotFound(format!("Invalid slot: {slot}"))),
            }

            let _: Option<DbTournamentMatch> = self
                .client
                .update(("tournament_match", match_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    // ── Aggregates ──

    pub async fn get_tournament_bracket(&self, id: &str) -> DbResult<TournamentBracket> {
        let tournament = self
            .get_tournament(id)
            .await?
            .ok_or_else(|| crate::DbError::NotFound(format!("Tournament {id} not found")))?;
        let participants = self.list_tournament_participants(id).await?;
        let rounds = self.list_tournament_rounds(id).await?;
        let matches = self.list_tournament_matches(id).await?;

        Ok(TournamentBracket {
            tournament,
            participants,
            rounds,
            matches,
        })
    }

    pub async fn get_swiss_standings(&self, tournament_id: &str) -> DbResult<Vec<SwissStanding>> {
        let participants = self.list_tournament_participants(tournament_id).await?;
        let matches = self.list_tournament_matches(tournament_id).await?;

        let completed: Vec<&TournamentMatch> = matches
            .iter()
            .filter(|m| m.status == TournamentMatchStatus::Completed)
            .collect();

        // Build standings per participant
        let mut standings: Vec<SwissStanding> = participants
            .iter()
            .map(|p| {
                let mut wins = 0u32;
                let mut losses = 0u32;
                let mut draws = 0u32;
                let mut game_wins = 0u32;
                let mut game_losses = 0u32;

                for m in &completed {
                    let is_a = m.participant_a_id.as_deref() == Some(&p.id);
                    let is_b = m.participant_b_id.as_deref() == Some(&p.id);
                    if !is_a && !is_b {
                        continue;
                    }

                    let (my_score, their_score) = if is_a {
                        (m.score_a.unwrap_or(0), m.score_b.unwrap_or(0))
                    } else {
                        (m.score_b.unwrap_or(0), m.score_a.unwrap_or(0))
                    };

                    game_wins += my_score;
                    game_losses += their_score;

                    if m.winner_id.as_deref() == Some(&p.id) {
                        wins += 1;
                    } else if m.winner_id.is_some() {
                        losses += 1;
                    } else {
                        draws += 1;
                    }
                }

                SwissStanding {
                    participant_id: p.id.clone(),
                    participant_name: p
                        .external_name
                        .clone()
                        .or_else(|| p.team_id.clone())
                        .unwrap_or_else(|| "TBD".to_string()),
                    wins,
                    losses,
                    draws,
                    game_wins,
                    game_losses,
                    buchholz: 0.0,
                    rank: 0,
                }
            })
            .collect();

        // Compute Buchholz tiebreaker (sum of opponents' wins)
        for i in 0..standings.len() {
            let pid = &standings[i].participant_id;
            let mut buchholz = 0.0;
            for m in &completed {
                let opponent_id = if m.participant_a_id.as_deref() == Some(pid) {
                    m.participant_b_id.as_deref()
                } else if m.participant_b_id.as_deref() == Some(pid) {
                    m.participant_a_id.as_deref()
                } else {
                    None
                };
                if let Some(oid) = opponent_id {
                    if let Some(opp) = standings.iter().find(|s| s.participant_id == oid) {
                        buchholz += opp.wins as f64;
                    }
                }
            }
            standings[i].buchholz = buchholz;
        }

        // Sort by wins desc, then buchholz desc, then game diff desc
        standings.sort_by(|a, b| {
            b.wins
                .cmp(&a.wins)
                .then_with(|| {
                    b.buchholz
                        .partial_cmp(&a.buchholz)
                        .unwrap_or(std::cmp::Ordering::Equal)
                })
                .then_with(|| {
                    let a_diff = a.game_wins as i64 - a.game_losses as i64;
                    let b_diff = b.game_wins as i64 - b.game_losses as i64;
                    b_diff.cmp(&a_diff)
                })
        });

        // Assign ranks
        for (i, s) in standings.iter_mut().enumerate() {
            s.rank = (i + 1) as u32;
        }

        Ok(standings)
    }

    // ── Bracket Generation ──

    pub async fn delete_tournament_bracket(&self, tournament_id: &str) -> DbResult<()> {
        with_timeout(async {
            self.client
                .query("DELETE tournament_match WHERE tournament_id = $tid")
                .bind(("tid", tournament_id.to_string()))
                .await?;
            self.client
                .query("DELETE tournament_round WHERE tournament_id = $tid")
                .bind(("tid", tournament_id.to_string()))
                .await?;
            Ok(())
        })
        .await
    }

    pub async fn generate_single_elim_bracket(&self, tournament_id: &str) -> DbResult<()> {
        let participants = self.list_tournament_participants(tournament_id).await?;
        let n = participants.len();
        if n < 2 {
            return Err(crate::DbError::NotFound(
                "Need at least 2 participants".into(),
            ));
        }

        // Delete existing bracket
        self.delete_tournament_bracket(tournament_id).await?;

        // Pad to power of 2
        let size = n.next_power_of_two();
        let num_rounds = (size as f64).log2() as u32;

        // Create rounds
        let mut round_ids = Vec::new();
        for r in 1..=num_rounds {
            let round = self
                .create_tournament_round(tournament_id, r, BracketStage::Main)
                .await?;
            round_ids.push(round.id);
        }

        // Seed participants (sorted by seed, then order)
        let mut seeded: Vec<Option<String>> = Vec::with_capacity(size);
        for p in &participants {
            seeded.push(Some(p.id.clone()));
        }
        // Fill with byes
        while seeded.len() < size {
            seeded.push(None);
        }

        // Standard seeding order for power-of-2 bracket
        let seed_order = standard_seed_order(size);
        let mut ordered: Vec<Option<String>> = vec![None; size];
        for (i, &pos) in seed_order.iter().enumerate() {
            if i < seeded.len() {
                ordered[pos] = seeded[i].clone();
            }
        }

        // Create first round matches
        let matches_in_first_round = size / 2;
        let mut match_ids: Vec<Vec<String>> = vec![Vec::new(); num_rounds as usize];

        for i in 0..matches_in_first_round {
            let a = ordered[i * 2].as_deref();
            let b = ordered[i * 2 + 1].as_deref();

            let is_bye = a.is_none() || b.is_none();
            let status = if is_bye {
                TournamentMatchStatus::Bye
            } else {
                TournamentMatchStatus::Pending
            };

            let m = self
                .create_tournament_match(
                    tournament_id,
                    &round_ids[0],
                    i as u32,
                    a,
                    b,
                    status,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
            match_ids[0].push(m.id);
        }

        // Create subsequent round matches (empty)
        for r in 1..num_rounds as usize {
            let num_matches = matches_in_first_round / (1 << r);
            for i in 0..num_matches {
                let m = self
                    .create_tournament_match(
                        tournament_id,
                        &round_ids[r],
                        i as u32,
                        None,
                        None,
                        TournamentMatchStatus::Pending,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                match_ids[r].push(m.id);
            }
        }

        // Wire up next_match_id and slots
        for r in 0..num_rounds as usize - 1 {
            for (i, mid) in match_ids[r].iter().enumerate() {
                let next_match_idx = i / 2;
                let slot = if i % 2 == 0 { "a" } else { "b" };
                if next_match_idx < match_ids[r + 1].len() {
                    let next_id = &match_ids[r + 1][next_match_idx];
                    self.update_match_next(mid, next_id, slot).await?;
                }
            }
        }

        // Auto-advance byes in first round
        for mid in &match_ids[0] {
            let m = self.get_tournament_match(mid).await?.unwrap();
            if m.status == TournamentMatchStatus::Bye {
                let winner = m.participant_a_id.as_ref().or(m.participant_b_id.as_ref());
                if let Some(winner_id) = winner {
                    if let (Some(next_id), Some(next_slot)) = (&m.next_match_id, &m.next_match_slot)
                    {
                        self.set_match_participant(next_id, next_slot, winner_id)
                            .await?;
                    }
                }
            }
        }

        Ok(())
    }

    /// Double-elimination bracket (power-of-2 padded with byes).
    ///
    /// Structure (8-team example):
    /// - Winners: WR1(4) → WR2(2) → WF(1)
    /// - Losers: LR1(2) ← WR1 losers; LR2(2) ← LR1 winners + WR2 losers;
    ///   LR3(1) ← LR2; LR4(1) ← LR3 + WF loser
    /// - Grand final: WF winner (slot a) vs LR final winner (slot b)
    pub async fn generate_double_elim_bracket(&self, tournament_id: &str) -> DbResult<()> {
        let participants = self.list_tournament_participants(tournament_id).await?;
        let n = participants.len();
        if n < 2 {
            return Err(crate::DbError::NotFound(
                "Need at least 2 participants".into(),
            ));
        }

        self.delete_tournament_bracket(tournament_id).await?;

        let DoubleElimShape {
            size,
            w_rounds,
            l_round_count,
            first_round_matches,
        } = double_elim_shape(n);

        let mut w_round_ids = Vec::new();
        for r in 1..=w_rounds {
            let round = self
                .create_tournament_round(tournament_id, r, BracketStage::Winners)
                .await?;
            w_round_ids.push(round.id);
        }

        let mut l_round_ids = Vec::new();
        for r in 1..=l_round_count {
            let round = self
                .create_tournament_round(tournament_id, r, BracketStage::Losers)
                .await?;
            l_round_ids.push(round.id);
        }

        let gf_round = self
            .create_tournament_round(tournament_id, 1, BracketStage::GrandFinal)
            .await?;

        let participant_ids: Vec<String> = participants.iter().map(|p| p.id.clone()).collect();
        let ordered = double_elim_seeded_slots(&participant_ids, size);
        let mut w_match_ids: Vec<Vec<String>> = vec![Vec::new(); w_rounds as usize];

        for i in 0..first_round_matches {
            let a = ordered[i * 2].as_deref();
            let b = ordered[i * 2 + 1].as_deref();
            let is_bye = a.is_none() || b.is_none();
            let status = if is_bye {
                TournamentMatchStatus::Bye
            } else {
                TournamentMatchStatus::Pending
            };

            let m = self
                .create_tournament_match(
                    tournament_id,
                    &w_round_ids[0],
                    i as u32,
                    a,
                    b,
                    status,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
            w_match_ids[0].push(m.id);
        }

        for r in 1..w_rounds as usize {
            let num_matches = first_round_matches / (1 << r);
            for i in 0..num_matches {
                let m = self
                    .create_tournament_match(
                        tournament_id,
                        &w_round_ids[r],
                        i as u32,
                        None,
                        None,
                        TournamentMatchStatus::Pending,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                w_match_ids[r].push(m.id);
            }
        }

        // LB match counts: start at WR1_matches/2, keep on even LB rounds, halve after odd
        let mut l_match_ids: Vec<Vec<String>> = Vec::new();
        let mut l_count = (first_round_matches / 2).max(1);
        // Index loop: mutates l_count mid-pass and indexes parallel id vectors.
        #[allow(clippy::needless_range_loop)]
        for r in 0..l_round_count as usize {
            let mut round_matches = Vec::new();
            for i in 0..l_count {
                let m = self
                    .create_tournament_match(
                        tournament_id,
                        &l_round_ids[r],
                        i as u32,
                        None,
                        None,
                        TournamentMatchStatus::Pending,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                round_matches.push(m.id);
            }
            l_match_ids.push(round_matches);
            if r % 2 == 1 {
                l_count = l_count.div_ceil(2).max(1);
            }
        }

        let gf = self
            .create_tournament_match(
                tournament_id,
                &gf_round.id,
                0,
                None,
                None,
                TournamentMatchStatus::Pending,
                None,
                None,
                None,
                None,
            )
            .await?;

        // Winners advancement: WR[r] → WR[r+1]
        if w_rounds > 1 {
            for r in 0..w_rounds as usize - 1 {
                for (i, mid) in w_match_ids[r].iter().enumerate() {
                    let next_idx = i / 2;
                    let slot = if i % 2 == 0 { "a" } else { "b" };
                    if next_idx < w_match_ids[r + 1].len() {
                        self.update_match_next(mid, &w_match_ids[r + 1][next_idx], slot)
                            .await?;
                    }
                }
            }
        }

        // Loser drops from every winners round (including winners final)
        if l_match_ids.is_empty() {
            // 2-team: single WR match — loser goes to grand final slot b
            if let Some(mid) = w_match_ids.first().and_then(|v| v.first()) {
                self.update_match_loser_next(mid, &gf.id, "b").await?;
            }
        } else {
            // Index loop: indexes w_match_ids by winners-round while mapping into l_match_ids.
            #[allow(clippy::needless_range_loop)]
            for r in 0..w_rounds as usize {
                for (i, mid) in w_match_ids[r].iter().enumerate() {
                    // WR1 → LR0 (pair into half matches); later WR → odd LB rounds (drop-in)
                    // Winners final → last LB match slot a
                    let (loser_round, loser_idx, loser_slot) = if r == 0 {
                        (0usize, i / 2, if i % 2 == 0 { "a" } else { "b" })
                    } else if r + 1 == w_rounds as usize {
                        let last = l_match_ids.len() - 1;
                        (last, 0, "a")
                    } else {
                        // WR2 → LR1, WR3 → LR3, … (drop-in rounds at odd indices)
                        let lr = 2 * r - 1;
                        (lr, i, "a")
                    };
                    if loser_round < l_match_ids.len() && loser_idx < l_match_ids[loser_round].len()
                    {
                        self.update_match_loser_next(
                            mid,
                            &l_match_ids[loser_round][loser_idx],
                            loser_slot,
                        )
                        .await?;
                    }
                }
            }
        }

        // Losers bracket advancement
        // Even LR index (0,2,…): winners go to next round same index as slot "b"
        //   (slot "a" is reserved for drop-ins from winners)
        // Odd LR index: winners pair into next (halving) round
        for r in 0..l_match_ids.len().saturating_sub(1) {
            for (i, mid) in l_match_ids[r].iter().enumerate() {
                let next_round = r + 1;
                let (next_idx, slot) = if r % 2 == 0 {
                    (i, "b")
                } else {
                    (i / 2, if i % 2 == 0 { "a" } else { "b" })
                };
                if next_round < l_match_ids.len() && next_idx < l_match_ids[next_round].len() {
                    self.update_match_next(mid, &l_match_ids[next_round][next_idx], slot)
                        .await?;
                }
            }
        }

        // Winners final → Grand final slot a; losers final → Grand final slot b
        if let Some(last_w) = w_match_ids.last().and_then(|v| v.first()) {
            self.update_match_next(last_w, &gf.id, "a").await?;
        }
        if let Some(last_l) = l_match_ids.last().and_then(|v| v.first()) {
            self.update_match_next(last_l, &gf.id, "b").await?;
        }

        // Auto-advance first-round byes in winners (no loser to drop)
        for mid in &w_match_ids[0] {
            let m = self.get_tournament_match(mid).await?.unwrap();
            if m.status == TournamentMatchStatus::Bye {
                let winner = m.participant_a_id.as_ref().or(m.participant_b_id.as_ref());
                if let Some(winner_id) = winner {
                    if let (Some(next_id), Some(next_slot)) = (&m.next_match_id, &m.next_match_slot)
                    {
                        self.set_match_participant(next_id, next_slot, winner_id)
                            .await?;
                    }
                }
            }
        }

        Ok(())
    }

    pub async fn generate_round_robin_pairings(&self, tournament_id: &str) -> DbResult<()> {
        let participants = self.list_tournament_participants(tournament_id).await?;
        let n = participants.len();
        if n < 2 {
            return Err(crate::DbError::NotFound(
                "Need at least 2 participants".into(),
            ));
        }

        self.delete_tournament_bracket(tournament_id).await?;

        // Circle method: if odd number, add a "bye" placeholder
        let ids: Vec<String> = participants.iter().map(|p| p.id.clone()).collect();
        let even_n = if n % 2 == 0 { n } else { n + 1 };
        let mut slots: Vec<Option<String>> = ids.into_iter().map(Some).collect();
        if n % 2 != 0 {
            slots.push(None); // bye slot
        }

        let num_rounds = even_n - 1;

        for r in 0..num_rounds {
            let round = self
                .create_tournament_round(tournament_id, (r + 1) as u32, BracketStage::Main)
                .await?;

            let half = even_n / 2;
            for i in 0..half {
                let a_idx = i;
                let b_idx = even_n - 1 - i;
                let a = slots[a_idx].as_deref();
                let b = slots[b_idx].as_deref();

                if a.is_none() || b.is_none() {
                    // Bye match
                    self.create_tournament_match(
                        tournament_id,
                        &round.id,
                        i as u32,
                        a,
                        b,
                        TournamentMatchStatus::Bye,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                } else {
                    self.create_tournament_match(
                        tournament_id,
                        &round.id,
                        i as u32,
                        a,
                        b,
                        TournamentMatchStatus::Pending,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                }
            }

            // Rotate: fix first element, rotate rest
            let last = slots.pop().unwrap();
            slots.insert(1, last);
        }

        Ok(())
    }

    pub async fn generate_swiss_round(&self, tournament_id: &str) -> DbResult<TournamentRound> {
        let rounds = self.list_tournament_rounds(tournament_id).await?;
        let round_number = rounds.len() as u32 + 1;

        let standings = self.get_swiss_standings(tournament_id).await?;
        let matches = self.list_tournament_matches(tournament_id).await?;

        // Build set of previous pairings
        let mut previous_pairings: std::collections::HashSet<(String, String)> =
            std::collections::HashSet::new();
        for m in &matches {
            if let (Some(a), Some(b)) = (&m.participant_a_id, &m.participant_b_id) {
                let mut pair = [a.clone(), b.clone()];
                pair.sort();
                previous_pairings.insert((pair[0].clone(), pair[1].clone()));
            }
        }

        let round = self
            .create_tournament_round(tournament_id, round_number, BracketStage::Main)
            .await?;

        // Pair by standings order, avoiding rematches
        let mut available: Vec<String> =
            standings.iter().map(|s| s.participant_id.clone()).collect();
        let mut position = 0u32;

        while available.len() >= 2 {
            let a = available.remove(0);
            let mut paired = false;
            for i in 0..available.len() {
                let b = &available[i];
                let mut pair = [a.clone(), b.clone()];
                pair.sort();
                if !previous_pairings.contains(&(pair[0].clone(), pair[1].clone())) {
                    let b = available.remove(i);
                    self.create_tournament_match(
                        tournament_id,
                        &round.id,
                        position,
                        Some(&a),
                        Some(&b),
                        TournamentMatchStatus::Pending,
                        None,
                        None,
                        None,
                        None,
                    )
                    .await?;
                    position += 1;
                    paired = true;
                    break;
                }
            }
            if !paired {
                // No non-rematch available, pair with first available
                let b = available.remove(0);
                self.create_tournament_match(
                    tournament_id,
                    &round.id,
                    position,
                    Some(&a),
                    Some(&b),
                    TournamentMatchStatus::Pending,
                    None,
                    None,
                    None,
                    None,
                )
                .await?;
                position += 1;
            }
        }

        // Odd player gets a bye
        if available.len() == 1 {
            let a = &available[0];
            self.create_tournament_match(
                tournament_id,
                &round.id,
                position,
                Some(a),
                None,
                TournamentMatchStatus::Bye,
                None,
                None,
                None,
                None,
            )
            .await?;
        }

        Ok(round)
    }

    // ── Internal helpers ──

    async fn update_match_next(
        &self,
        match_id: &str,
        next_match_id: &str,
        slot: &str,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbTournamentMatch> =
                self.client.select(("tournament_match", match_id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Match {match_id} not found")))?;
            db.next_match_id = Some(next_match_id.to_string());
            db.next_match_slot = Some(slot.to_string());
            let _: Option<DbTournamentMatch> = self
                .client
                .update(("tournament_match", match_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    async fn update_match_loser_next(
        &self,
        match_id: &str,
        loser_next_match_id: &str,
        slot: &str,
    ) -> DbResult<()> {
        with_timeout(async {
            let existing: Option<DbTournamentMatch> =
                self.client.select(("tournament_match", match_id)).await?;
            let mut db = existing
                .ok_or_else(|| crate::DbError::NotFound(format!("Match {match_id} not found")))?;
            db.loser_next_match_id = Some(loser_next_match_id.to_string());
            db.loser_next_match_slot = Some(slot.to_string());
            let _: Option<DbTournamentMatch> = self
                .client
                .update(("tournament_match", match_id))
                .content(db)
                .await?;
            Ok(())
        })
        .await
    }

    /// Count participants in a tournament.
    pub async fn count_tournament_participants(&self, tournament_id: &str) -> DbResult<u64> {
        with_timeout(async {
            #[derive(Deserialize, SurrealValue)]
            struct CountResult {
                count: u64,
            }
            let mut result = self
                .client
                .query("SELECT count() FROM tournament_participant WHERE tournament_id = $tid GROUP ALL")
                .bind(("tid", tournament_id.to_string()))
                .await?;
            let counts: Vec<CountResult> = result.take(0)?;
            Ok(counts.first().map(|c| c.count).unwrap_or(0))
        })
        .await
    }
}

// ─── Seeding Helpers ───

/// Pure shape of a double-elim bracket for `n` participants (power-of-two pad).
#[derive(Debug, Clone, Copy)]
struct DoubleElimShape {
    size: usize,
    w_rounds: u32,
    l_round_count: u32,
    first_round_matches: usize,
}

fn double_elim_shape(n: usize) -> DoubleElimShape {
    let size = n.next_power_of_two();
    let w_rounds = (size as f64).log2() as u32;
    // 2 teams (w_rounds=1): no intermediate LB — WR loser drops straight to GF slot b.
    // Larger: classic 2*(w_rounds-1) losers rounds.
    let l_round_count = if w_rounds <= 1 {
        0
    } else {
        2 * (w_rounds - 1)
    };
    DoubleElimShape {
        size,
        w_rounds,
        l_round_count,
        first_round_matches: size / 2,
    }
}

/// Place participant ids into bracket slots using standard seeding; pads with byes (`None`).
fn double_elim_seeded_slots(participant_ids: &[String], size: usize) -> Vec<Option<String>> {
    let mut seeded: Vec<Option<String>> = participant_ids.iter().cloned().map(Some).collect();
    while seeded.len() < size {
        seeded.push(None);
    }
    let seed_order = standard_seed_order(size);
    let mut ordered: Vec<Option<String>> = vec![None; size];
    for (i, &pos) in seed_order.iter().enumerate() {
        if i < seeded.len() {
            ordered[pos] = seeded[i].clone();
        }
    }
    ordered
}

/// Generate standard tournament seeding order for a bracket of given size.
/// e.g., for 8: [0, 7, 3, 4, 1, 6, 2, 5] — ensures top seeds are spread out.
fn standard_seed_order(size: usize) -> Vec<usize> {
    if size <= 1 {
        return vec![0];
    }
    let mut order = vec![0usize; size];
    order[0] = 0;
    order[1] = 1;
    let mut chunk = 2;
    while chunk < size {
        let mut new_order = vec![0; chunk * 2];
        for (i, &pos) in order[..chunk].iter().enumerate() {
            new_order[i * 2] = pos;
            new_order[i * 2 + 1] = chunk * 2 - 1 - pos;
        }
        order = new_order;
        chunk *= 2;
    }
    order
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_seed_order_4() {
        let order = standard_seed_order(4);
        // 1 vs 4, 2 vs 3 → positions [0, 3, 1, 2]
        assert_eq!(order, vec![0, 3, 1, 2]);
    }

    #[test]
    fn test_seed_order_8() {
        let order = standard_seed_order(8);
        assert_eq!(order.len(), 8);
        // Seed 1 (idx 0) at pos 0, seed 2 (idx 1) at different half
        assert_eq!(order[0], 0);
    }

    #[test]
    fn test_double_elim_shape_2() {
        let s = double_elim_shape(2);
        assert_eq!(s.size, 2);
        assert_eq!(s.w_rounds, 1);
        assert_eq!(s.l_round_count, 0);
        assert_eq!(s.first_round_matches, 1);
    }

    #[test]
    fn test_double_elim_shape_5() {
        let s = double_elim_shape(5);
        assert_eq!(s.size, 8);
        assert_eq!(s.w_rounds, 3);
        assert_eq!(s.l_round_count, 4);
    }

    #[test]
    fn test_double_elim_seeded_slots_pads_byes() {
        let ids = vec!["a".into(), "b".into(), "c".into()];
        let slots = double_elim_seeded_slots(&ids, 4);
        assert_eq!(slots.len(), 4);
        assert_eq!(slots.iter().filter(|s| s.is_some()).count(), 3);
        assert_eq!(slots.iter().filter(|s| s.is_none()).count(), 1);
    }
}
