use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    AuditAction, AuditTargetType, ParticipantStatus, SwissStanding, Tournament, TournamentBracket,
    TournamentFormat, TournamentMatch, TournamentParticipant, TournamentRound, TournamentStatus,
};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::OfficerUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

type ApiResult<T> = Result<T, (StatusCode, Json<ErrorResponse>)>;

fn internal_err(e: impl std::fmt::Display) -> (StatusCode, Json<ErrorResponse>) {
    tracing::error!(error = %e, "internal error");
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal server error".into(),
        }),
    )
}

fn bad_request(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

fn not_found(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::NOT_FOUND,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

fn conflict(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::CONFLICT,
        Json(ErrorResponse {
            error: msg.to_string(),
        }),
    )
}

// ─── List & Detail (Public) ───

#[derive(Deserialize)]
pub struct ListTournamentsQuery {
    pub status: Option<String>,
    pub game_id: Option<String>,
    pub cursor: Option<String>,
    #[serde(default = "default_pagination_limit")]
    pub limit: u32,
}

fn default_pagination_limit() -> u32 {
    25
}

/// GET /api/tournaments (cursor-paginated)
pub async fn list_tournaments(
    State(state): State<AppState>,
    axum::extract::Query(query): axum::extract::Query<ListTournamentsQuery>,
) -> ApiResult<Json<CursorResponse<Tournament>>> {
    let status = query.status.as_deref().and_then(|s| match s {
        "draft" => Some(TournamentStatus::Draft),
        "registration" => Some(TournamentStatus::Registration),
        "in_progress" => Some(TournamentStatus::InProgress),
        "completed" => Some(TournamentStatus::Completed),
        "archived" => Some(TournamentStatus::Archived),
        _ => None,
    });

    let pagination = PaginationParams {
        cursor: query.cursor,
        limit: query.limit,
    };
    let (limit, offset) = pagination.resolve();
    let items = state
        .db
        .list_tournaments_paginated(status, query.game_id.as_deref(), limit, offset)
        .await
        .map_err(internal_err)?;
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

/// GET /api/tournaments/:id
pub async fn get_tournament(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Tournament>> {
    state
        .db
        .get_tournament(&id)
        .await
        .map_err(internal_err)?
        .map(Json)
        .ok_or_else(|| not_found("Tournament not found"))
}

// ─── CRUD (Officer+) ───

#[derive(Deserialize)]
pub struct CreateTournamentRequest {
    pub name: String,
    pub game_id: Option<String>,
    pub format: TournamentFormat,
    pub max_teams: Option<u32>,
    #[serde(default = "default_best_of")]
    pub best_of: u32,
    pub swiss_rounds: Option<u32>,
    #[serde(default)]
    pub is_external: bool,
    #[serde(default)]
    pub is_open: bool,
    pub external_url: Option<String>,
    pub rules: Option<String>,
    pub description: Option<String>,
    pub starts_at: Option<DateTime<Utc>>,
    pub ends_at: Option<DateTime<Utc>>,
}

fn default_best_of() -> u32 {
    1
}

/// POST /api/tournaments
pub async fn create_tournament(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<CreateTournamentRequest>,
) -> ApiResult<(StatusCode, Json<Tournament>)> {
    let tournament = state
        .db
        .create_tournament(
            &body.name,
            body.game_id.as_deref(),
            body.format,
            body.max_teams,
            body.best_of,
            body.swiss_rounds,
            body.is_external,
            body.is_open,
            body.external_url.as_deref(),
            body.rules.as_deref(),
            body.description.as_deref(),
            body.starts_at,
            body.ends_at,
            &officer.member.id,
        )
        .await
        .map_err(internal_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::CreatedTournament,
        AuditTargetType::Tournament,
        &tournament.id,
        Some(&format!("Created tournament: {}", tournament.name)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(tournament)))
}

#[derive(Deserialize)]
pub struct UpdateTournamentRequest {
    pub name: Option<String>,
    pub game_id: Option<Option<String>>,
    pub format: Option<TournamentFormat>,
    pub max_teams: Option<Option<u32>>,
    pub best_of: Option<u32>,
    pub swiss_rounds: Option<Option<u32>>,
    pub is_external: Option<bool>,
    pub is_open: Option<bool>,
    pub external_url: Option<Option<String>>,
    pub rules: Option<Option<String>>,
    pub description: Option<Option<String>>,
    pub starts_at: Option<Option<DateTime<Utc>>>,
    pub ends_at: Option<Option<DateTime<Utc>>>,
}

/// PUT /api/tournaments/:id
pub async fn update_tournament(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateTournamentRequest>,
) -> ApiResult<Json<Tournament>> {
    let tournament = state
        .db
        .update_tournament(
            &id,
            body.name.as_deref(),
            body.game_id.as_ref().map(|g| g.as_deref()),
            body.format,
            body.max_teams,
            body.best_of,
            body.swiss_rounds,
            body.is_external,
            body.is_open,
            body.external_url.as_ref().map(|u| u.as_deref()),
            body.rules.as_ref().map(|r| r.as_deref()),
            body.description.as_ref().map(|d| d.as_deref()),
            body.starts_at,
            body.ends_at,
        )
        .await
        .map_err(internal_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedTournament,
        AuditTargetType::Tournament,
        &id,
        None,
    )
    .await;

    Ok(Json(tournament))
}

// ─── Status Transitions ───

#[derive(Deserialize)]
pub struct StatusTransitionRequest {
    pub status: String,
}

/// PATCH /api/tournaments/:id/status
pub async fn transition_status(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<StatusTransitionRequest>,
) -> ApiResult<Json<Tournament>> {
    let tournament = state
        .db
        .get_tournament(&id)
        .await
        .map_err(internal_err)?
        .ok_or_else(|| not_found("Tournament not found"))?;

    let new_status = match body.status.as_str() {
        "registration" => TournamentStatus::Registration,
        "in_progress" => TournamentStatus::InProgress,
        "completed" => TournamentStatus::Completed,
        "archived" => TournamentStatus::Archived,
        _ => return Err(bad_request("Invalid status")),
    };

    // Validate transitions
    match (tournament.status, new_status) {
        (TournamentStatus::Draft, TournamentStatus::Registration) => {
            if tournament.name.is_empty() {
                return Err(bad_request("Tournament needs a name"));
            }
        }
        (TournamentStatus::Registration, TournamentStatus::InProgress) => {
            let count = state
                .db
                .count_tournament_participants(&id)
                .await
                .map_err(internal_err)?;
            if count < 2 {
                return Err(bad_request("Need at least 2 participants"));
            }
            // Check bracket exists
            let matches = state
                .db
                .list_tournament_matches(&id)
                .await
                .map_err(internal_err)?;
            if matches.is_empty() {
                return Err(bad_request("Generate bracket first"));
            }
        }
        (TournamentStatus::InProgress, TournamentStatus::Completed) => {}
        (TournamentStatus::Completed, TournamentStatus::Archived) => {}
        // Cancel / soft-delete before or after a run
        (
            TournamentStatus::Draft | TournamentStatus::Registration | TournamentStatus::InProgress,
            TournamentStatus::Archived,
        ) => {}
        _ => {
            return Err(bad_request(&format!(
                "Invalid transition: {} → {}",
                tournament.status, new_status
            )));
        }
    }

    let updated = state
        .db
        .update_tournament_status(&id, new_status)
        .await
        .map_err(internal_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::ChangedTournamentStatus,
        AuditTargetType::Tournament,
        &id,
        Some(&format!("{} → {}", tournament.status, new_status)),
    )
    .await;

    Ok(Json(updated))
}

// ─── Bracket ───

/// GET /api/tournaments/:id/bracket
pub async fn get_bracket(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<TournamentBracket>> {
    state
        .db
        .get_tournament_bracket(&id)
        .await
        .map(Json)
        .map_err(internal_err)
}

/// POST /api/tournaments/:id/generate-bracket
pub async fn generate_bracket(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
) -> ApiResult<Json<TournamentBracket>> {
    let tournament = state
        .db
        .get_tournament(&id)
        .await
        .map_err(internal_err)?
        .ok_or_else(|| not_found("Tournament not found"))?;

    match tournament.format {
        TournamentFormat::SingleElim => {
            state
                .db
                .generate_single_elim_bracket(&id)
                .await
                .map_err(internal_err)?;
        }
        TournamentFormat::DoubleElim => {
            state
                .db
                .generate_double_elim_bracket(&id)
                .await
                .map_err(internal_err)?;
        }
        TournamentFormat::RoundRobin => {
            state
                .db
                .generate_round_robin_pairings(&id)
                .await
                .map_err(internal_err)?;
        }
        TournamentFormat::Swiss => {
            // Swiss generates one round at a time; for initial bracket, generate round 1
            state
                .db
                .generate_swiss_round(&id)
                .await
                .map_err(internal_err)?;
        }
    }

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::GeneratedBracket,
        AuditTargetType::Tournament,
        &id,
        Some(&format!("Generated {} bracket", tournament.format)),
    )
    .await;

    state
        .db
        .get_tournament_bracket(&id)
        .await
        .map(Json)
        .map_err(internal_err)
}

// ─── Participants ───

/// GET /api/tournaments/:id/participants
pub async fn list_participants(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<TournamentParticipant>>> {
    state
        .db
        .list_tournament_participants(&id)
        .await
        .map(Json)
        .map_err(internal_err)
}

#[derive(Deserialize)]
pub struct AddParticipantRequest {
    pub team_id: Option<String>,
    pub external_name: Option<String>,
    pub seed: Option<u32>,
}

/// POST /api/tournaments/:id/participants
pub async fn add_participant(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<AddParticipantRequest>,
) -> ApiResult<(StatusCode, Json<TournamentParticipant>)> {
    if body.team_id.is_none() && body.external_name.is_none() {
        return Err(bad_request("Provide team_id or external_name"));
    }

    // Check max_teams
    let tournament = state
        .db
        .get_tournament(&id)
        .await
        .map_err(internal_err)?
        .ok_or_else(|| not_found("Tournament not found"))?;

    if let Some(max) = tournament.max_teams {
        let count = state
            .db
            .count_tournament_participants(&id)
            .await
            .map_err(internal_err)?;
        if count >= max as u64 {
            return Err(bad_request("Tournament is full"));
        }
    }

    let participant = state
        .db
        .add_tournament_participant(
            &id,
            body.team_id.as_deref(),
            body.external_name.as_deref(),
            body.seed,
        )
        .await
        .map_err(internal_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::AddedTournamentParticipant,
        AuditTargetType::TournamentParticipant,
        &participant.id,
        Some(&format!(
            "Added to tournament {id}: {}",
            body.external_name
                .as_deref()
                .or(body.team_id.as_deref())
                .unwrap_or("unknown")
        )),
    )
    .await;

    Ok((StatusCode::CREATED, Json(participant)))
}

#[derive(Deserialize)]
pub struct UpdateParticipantRequest {
    pub seed: Option<Option<u32>>,
    pub status: Option<String>,
    pub group_label: Option<Option<String>>,
}

/// PUT /api/tournaments/:id/participants/:pid
pub async fn update_participant(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path((id, pid)): Path<(String, String)>,
    Json(body): Json<UpdateParticipantRequest>,
) -> ApiResult<Json<TournamentParticipant>> {
    let status = body.status.as_deref().map(|s| match s {
        "checked_in" => ParticipantStatus::CheckedIn,
        "active" => ParticipantStatus::Active,
        "eliminated" => ParticipantStatus::Eliminated,
        "withdrawn" => ParticipantStatus::Withdrawn,
        "disqualified" => ParticipantStatus::Disqualified,
        _ => ParticipantStatus::Registered,
    });

    let result = state
        .db
        .update_tournament_participant(
            &pid,
            body.seed,
            status,
            body.group_label.as_ref().map(|g| g.as_deref()),
        )
        .await
        .map_err(internal_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UpdatedTournament,
        AuditTargetType::TournamentParticipant,
        &pid,
        Some(&format!("Updated participant in tournament {id}")),
    )
    .await;

    Ok(Json(result))
}

/// DELETE /api/tournaments/:id/participants/:pid
pub async fn remove_participant(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path((id, pid)): Path<(String, String)>,
) -> ApiResult<StatusCode> {
    state
        .db
        .remove_tournament_participant(&pid)
        .await
        .map_err(internal_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::RemovedTournamentParticipant,
        AuditTargetType::TournamentParticipant,
        &pid,
        Some(&format!("Removed from tournament {id}")),
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

// ─── Match Reporting ───

/// GET /api/tournaments/:id/matches
pub async fn list_matches(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<TournamentMatch>>> {
    state
        .db
        .list_tournament_matches(&id)
        .await
        .map(Json)
        .map_err(internal_err)
}

#[derive(Deserialize)]
pub struct ReportMatchRequest {
    pub score_a: u32,
    pub score_b: u32,
    pub winner_id: String,
    pub notes: Option<String>,
    pub replay_codes: Option<Vec<String>>,
}

/// PATCH /api/tournaments/:id/matches/:mid/report
pub async fn report_match(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path((id, mid)): Path<(String, String)>,
    Json(body): Json<ReportMatchRequest>,
) -> ApiResult<Json<TournamentMatch>> {
    let winner: &str = body.winner_id.trim();

    // DR1-DB-002 + DB-001: pre-validate against the loaded match so the common
    // bad-input cases return a clean 400/404 (the DB layer re-checks all of
    // these and does the CAS, so it stays the authoritative fail-closed guard).
    let existing = state
        .db
        .get_tournament_match(&mid)
        .await
        .map_err(internal_err)?
        .ok_or_else(|| not_found("Match not found"))?;
    if existing.tournament_id != id {
        return Err(not_found("Match not found in this tournament"));
    }
    if winner.is_empty()
        || (existing.participant_a_id.as_deref() != Some(winner)
            && existing.participant_b_id.as_deref() != Some(winner))
    {
        return Err(bad_request(
            "winner_id must be one of the match participants",
        ));
    }

    // Report the match result. The DB does an atomic CAS `WHERE status =
    // 'pending'`, so a re-report or a concurrent double-report yields Conflict
    // (409) and the advancement below runs at most once per match.
    let reported = state
        .db
        .report_tournament_match(
            &mid,
            body.score_a,
            body.score_b,
            winner,
            body.notes.as_deref(),
            body.replay_codes.unwrap_or_default(),
        )
        .await
        .map_err(|e| match e {
            scuffed_db::DbError::NotFound(msg) => not_found(&msg),
            scuffed_db::DbError::Conflict(msg) => conflict(&msg),
            other => internal_err(other),
        })?;

    // Auto-advance winner to next match
    if let (Some(next_id), Some(next_slot)) = (&reported.next_match_id, &reported.next_match_slot) {
        state
            .db
            .set_match_participant(next_id, next_slot, winner)
            .await
            .map_err(internal_err)?;
    }

    // Auto-advance loser to losers bracket (double elim)
    if let (Some(loser_next_id), Some(loser_slot)) = (
        &reported.loser_next_match_id,
        &reported.loser_next_match_slot,
    ) {
        let loser_id = if reported.participant_a_id.as_deref() == Some(winner) {
            &reported.participant_b_id
        } else {
            &reported.participant_a_id
        };
        if let Some(loser) = loser_id {
            state
                .db
                .set_match_participant(loser_next_id, loser_slot, loser)
                .await
                .map_err(internal_err)?;
        }
    }

    // Mark loser as eliminated (single elim only, when no loser bracket)
    if reported.loser_next_match_id.is_none() {
        let loser_id = if reported.participant_a_id.as_deref() == Some(winner) {
            &reported.participant_b_id
        } else {
            &reported.participant_a_id
        };
        if let Some(loser) = loser_id {
            let tournament = state.db.get_tournament(&id).await.map_err(internal_err)?;
            if let Some(t) = &tournament
                && t.format == TournamentFormat::SingleElim
            {
                let _ = state
                    .db
                    .update_tournament_participant(
                        loser,
                        None,
                        Some(ParticipantStatus::Eliminated),
                        None,
                    )
                    .await;
            }
        }
    }

    // Check if round is complete → auto-complete round
    let all_matches = state
        .db
        .list_tournament_matches(&id)
        .await
        .map_err(internal_err)?;

    let round_matches: Vec<&TournamentMatch> = all_matches
        .iter()
        .filter(|m| m.round_id == reported.round_id)
        .collect();

    let round_complete = round_matches.iter().all(|m| {
        m.status == scuffed_db::TournamentMatchStatus::Completed
            || m.status == scuffed_db::TournamentMatchStatus::Bye
    });

    if round_complete {
        let _ = state
            .db
            .update_round_status(&reported.round_id, scuffed_db::RoundStatus::Completed)
            .await;
    }

    // Check if tournament is complete (all matches done)
    let all_complete = all_matches.iter().all(|m| {
        m.status == scuffed_db::TournamentMatchStatus::Completed
            || m.status == scuffed_db::TournamentMatchStatus::Bye
    });

    if all_complete {
        let _ = state
            .db
            .update_tournament_status(&id, TournamentStatus::Completed)
            .await;
    }

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::ReportedTournamentMatch,
        AuditTargetType::TournamentMatch,
        &mid,
        Some(&format!("{}-{}", body.score_a, body.score_b)),
    )
    .await;

    Ok(Json(reported))
}

// ─── Swiss-specific ───

/// GET /api/tournaments/:id/standings
pub async fn get_standings(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> ApiResult<Json<Vec<SwissStanding>>> {
    state
        .db
        .get_swiss_standings(&id)
        .await
        .map(Json)
        .map_err(internal_err)
}

/// POST /api/tournaments/:id/next-round
pub async fn generate_next_round(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
) -> ApiResult<(StatusCode, Json<TournamentRound>)> {
    let round = state
        .db
        .generate_swiss_round(&id)
        .await
        .map_err(internal_err)?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::GeneratedBracket,
        AuditTargetType::Tournament,
        &id,
        Some(&format!("Generated Swiss round {}", round.round_number)),
    )
    .await;

    Ok((StatusCode::CREATED, Json(round)))
}
