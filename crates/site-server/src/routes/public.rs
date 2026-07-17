use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use serde::Serialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    Announcement, Event, Game, GameAccount, MatchResult, MatchType, Member, SiteSettings, Team,
    TeamRecord,
};
use scuffed_types::api::{CursorResponse, PaginationParams};
use scuffed_types::{
    MatchResult as TypesMatchResult, MatchType as TypesMatchType, PublicMatch, RecentResult,
    UpcomingMatch,
};

use crate::state::AppState;

#[derive(Serialize)]
pub struct TeamOverview {
    #[serde(flatten)]
    pub team: Team,
    pub roster_count: usize,
    pub record: TeamRecord,
}

#[derive(Serialize)]
pub struct PublicOverview {
    pub teams: Vec<TeamOverview>,
    pub games: Vec<Game>,
    pub events: Vec<Event>,
    pub announcements: Vec<Announcement>,
    pub settings: SiteSettings,
    pub member_count: usize,
    /// Next public fixtures (scheduled, not yet played) — home next-match widget.
    #[serde(default)]
    pub upcoming_matches: Vec<UpcomingMatch>,
    /// Recent public results — home results ticker.
    #[serde(default)]
    pub recent_results: Vec<RecentResult>,
}

/// GET /api/public/overview — aggregated public data for the site
pub async fn overview(
    State(state): State<AppState>,
) -> Result<Json<PublicOverview>, (StatusCode, Json<ErrorResponse>)> {
    let teams = state.db.list_teams().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    let mut team_overviews = Vec::with_capacity(teams.len());
    for team in teams {
        let roster = state.db.get_team_roster(&team.id).await.map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
        let record = state.db.get_team_record(&team.id).await.map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
        team_overviews.push(TeamOverview {
            roster_count: roster.len(),
            team,
            record,
        });
    }

    let events = state.db.list_events().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    let events: Vec<_> = events.into_iter().filter(|e| e.is_public).collect();

    let announcements = state.db.list_announcements().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    let settings = state.db.get_settings().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    let member_count = state.db.count_active_members().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    let games = state.db.list_games().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    // Team/game name maps for home live widgets (#2).
    let team_name: std::collections::HashMap<String, String> = team_overviews
        .iter()
        .map(|t| (t.team.id.clone(), t.team.name.clone()))
        .collect();
    let game_by_team: std::collections::HashMap<String, String> = team_overviews
        .iter()
        .map(|t| (t.team.id.clone(), t.team.game_id.clone()))
        .collect();
    let game_name: std::collections::HashMap<String, String> = games
        .iter()
        .map(|g| (g.id.clone(), g.name.clone()))
        .collect();

    let upcoming_raw = state
        .db
        .list_public_upcoming_matches(5)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    let upcoming_matches: Vec<UpcomingMatch> = upcoming_raw
        .into_iter()
        .filter_map(|m| {
            let scheduled_at = m.scheduled_at?;
            let team_id = m.team_id.clone();
            let tname = team_name
                .get(&team_id)
                .cloned()
                .unwrap_or_else(|| "Team".into());
            let gname = game_by_team
                .get(&team_id)
                .and_then(|gid| game_name.get(gid).cloned());
            let match_type = match m.match_type {
                MatchType::Official => TypesMatchType::Official,
                MatchType::Tournament => TypesMatchType::Tournament,
                MatchType::Scrim => return None,
            };
            Some(UpcomingMatch {
                id: m.id,
                team_id,
                team_name: tname,
                game_name: gname,
                opponent: m.opponent,
                match_type,
                scheduled_at,
            })
        })
        .collect();

    let recent_raw = state.db.list_public_recent_matches(8).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    let recent_results: Vec<RecentResult> = recent_raw
        .into_iter()
        .filter_map(|m| {
            let played_at = m.played_at?;
            let team_id = m.team_id.clone();
            let tname = team_name
                .get(&team_id)
                .cloned()
                .unwrap_or_else(|| "Team".into());
            let match_type = match m.match_type {
                MatchType::Official => TypesMatchType::Official,
                MatchType::Tournament => TypesMatchType::Tournament,
                MatchType::Scrim => return None,
            };
            Some(RecentResult {
                id: m.id,
                team_id,
                team_name: tname,
                opponent: m.opponent,
                score_us: m.score_us,
                score_them: m.score_them,
                outcome: RecentResult::outcome_from_scores(m.score_us, m.score_them).into(),
                match_type,
                played_at,
            })
        })
        .collect();

    Ok(Json(PublicOverview {
        teams: team_overviews,
        games,
        events,
        announcements,
        settings,
        member_count: member_count as usize,
        upcoming_matches,
        recent_results,
    }))
}

/// Public member info (no user_id exposed).
#[derive(Serialize)]
pub struct PublicMember {
    pub id: String,
    pub display_name: String,
    pub org_role: String,
    pub bio: Option<String>,
    pub avatar_url: Option<String>,
    pub timezone: Option<String>,
    pub pronouns: Option<String>,
    pub availability_status: Option<String>,
    pub nostr_pubkey: Option<String>,
    pub joined_at: String,
    /// Preferred competitive role (public-but-minimal).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub main_role: Option<String>,
    /// Twitch handle (not URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitch: Option<String>,
    /// X/Twitter handle (not URL).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub twitter: Option<String>,
}

fn member_to_public(m: Member) -> PublicMember {
    PublicMember {
        id: m.id,
        display_name: m.display_name,
        org_role: m.org_role.to_string(),
        bio: m.bio,
        avatar_url: m.avatar_url,
        timezone: m.timezone,
        pronouns: m.pronouns,
        availability_status: m.availability_status,
        nostr_pubkey: m.nostr_pubkey,
        joined_at: m.joined_at.to_rfc3339(),
        main_role: m.main_role,
        twitch: m.twitch,
        twitter: m.twitter,
    }
}

/// GET /api/public/members — public member list (cursor-paginated)
pub async fn public_members(
    State(state): State<AppState>,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<PublicMember>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let members = state
        .db
        .list_members_paginated(limit, offset)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    let public: Vec<PublicMember> = members.into_iter().map(member_to_public).collect();
    Ok(Json(CursorResponse::from_oversized(public, limit, offset)))
}

/// Public member profile with team memberships and game accounts.
#[derive(Serialize)]
pub struct PublicMemberProfile {
    #[serde(flatten)]
    pub member: PublicMember,
    pub teams: Vec<MemberTeamInfo>,
    pub game_accounts: Vec<GameAccount>,
}

#[derive(Serialize)]
pub struct MemberTeamInfo {
    pub team_id: String,
    pub team_name: String,
    pub team_role: String,
}

/// GET /api/public/members/:id — member profile with teams and game accounts
pub async fn public_member_profile(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PublicMemberProfile>, (StatusCode, Json<ErrorResponse>)> {
    let member = state
        .db
        .get_member_safe(&id)
        .await
        .map_err(|e| {
            tracing::error!(error = %e, "public get_member_safe failed");
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Member not found".into(),
                }),
            )
        })?;

    // Get all teams to resolve names
    let teams = state.db.list_teams().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    // Find roster entries for this member across all teams
    let mut member_teams = Vec::new();
    for team in &teams {
        let roster = state.db.get_team_roster(&team.id).await.map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
        for entry in roster {
            if entry.member_id == id && entry.is_active {
                member_teams.push(MemberTeamInfo {
                    team_id: team.id.clone(),
                    team_name: team.name.clone(),
                    team_role: entry.team_role.to_string(),
                });
            }
        }
    }

    // Get game accounts
    let game_accounts = state
        .db
        .list_member_game_accounts(&id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    Ok(Json(PublicMemberProfile {
        member: member_to_public(member),
        teams: member_teams,
        game_accounts,
    }))
}

/// Full team detail for public team page.
#[derive(Serialize)]
pub struct PublicTeamDetail {
    #[serde(flatten)]
    pub team: Team,
    pub game_name: Option<String>,
    pub roster: Vec<TeamRosterMember>,
    pub record: TeamRecord,
    /// Public-safe matches only (no notes/scrims).
    pub recent_matches: Vec<PublicMatch>,
    /// Public events only.
    pub upcoming_events: Vec<Event>,
}

fn match_to_public(m: MatchResult) -> Option<PublicMatch> {
    let typed = TypesMatchResult {
        id: m.id,
        team_id: m.team_id,
        opponent: m.opponent,
        score_us: m.score_us,
        score_them: m.score_them,
        map_name: m.map_name,
        game_mode: m.game_mode,
        match_type: match m.match_type {
            MatchType::Official => TypesMatchType::Official,
            MatchType::Tournament => TypesMatchType::Tournament,
            MatchType::Scrim => TypesMatchType::Scrim,
        },
        played_at: m.played_at,
        scheduled_at: m.scheduled_at,
        recorded_by: m.recorded_by,
        notes: m.notes,
        is_public: m.is_public,
        vod_url: m.vod_url,
        replay_code: m.replay_code,
    };
    PublicMatch::try_from_match(&typed)
}

/// Public match detail with team/game display names for the match page.
#[derive(Serialize)]
pub struct PublicMatchDetail {
    #[serde(flatten)]
    pub match_row: PublicMatch,
    pub team_name: String,
    pub game_name: Option<String>,
}

/// GET /api/public/matches/:id — public non-scrim match detail (404 if private).
pub async fn public_match_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PublicMatchDetail>, (StatusCode, Json<ErrorResponse>)> {
    let m = state.db.get_match(&id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    let m = m.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Match not found".into(),
            }),
        )
    })?;
    let pub_m = match_to_public(m).ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Match not found".into(),
            }),
        )
    })?;

    let team = state.db.get_team(&pub_m.team_id).await.ok().flatten();
    let team_name = team
        .as_ref()
        .map(|t| t.name.clone())
        .unwrap_or_else(|| "Team".into());
    let game_name = if let Some(t) = team.as_ref() {
        state
            .db
            .get_game(&t.game_id)
            .await
            .ok()
            .flatten()
            .map(|g| g.name)
    } else {
        None
    };

    Ok(Json(PublicMatchDetail {
        match_row: pub_m,
        team_name,
        game_name,
    }))
}

#[derive(Serialize)]
pub struct TeamRosterMember {
    pub member_id: String,
    pub display_name: String,
    pub avatar_url: Option<String>,
    pub team_role: String,
}

/// GET /api/public/teams/:id — full team detail
pub async fn public_team_detail(
    State(state): State<AppState>,
    Path(id): Path<String>,
) -> Result<Json<PublicTeamDetail>, (StatusCode, Json<ErrorResponse>)> {
    let team = state
        .db
        .get_team(&id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        .ok_or_else(|| {
            (
                StatusCode::NOT_FOUND,
                Json(ErrorResponse {
                    error: "Team not found".into(),
                }),
            )
        })?;

    // Get game name
    let game_name = state
        .db
        .get_game(&team.game_id)
        .await
        .ok()
        .flatten()
        .map(|g| g.name);

    // Get roster with member names
    let roster_entries = state.db.get_team_roster(&id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    let mut roster = Vec::new();
    for entry in roster_entries {
        if entry.is_active {
            let member = state
                .db
                .get_member_safe(&entry.member_id)
                .await
                .ok()
                .flatten();
            roster.push(TeamRosterMember {
                member_id: entry.member_id,
                display_name: member
                    .as_ref()
                    .map(|m| m.display_name.clone())
                    .unwrap_or_else(|| "Unknown".to_string()),
                avatar_url: member.and_then(|m| m.avatar_url),
                team_role: entry.team_role.to_string(),
            });
        }
    }

    let record = state.db.get_team_record(&id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    // Cap + filter in SQL: public non-scrim played matches only (PR2 nit).
    // Avoids loading full history and keeps scheduled fixtures out of recent results.
    let recent_matches = state
        .db
        .list_team_matches_paginated(&id, 10, 0, true, true)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    let recent_matches: Vec<_> = recent_matches
        .into_iter()
        .filter_map(match_to_public)
        .take(10)
        .collect();

    let all_events = state.db.list_events().await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    let upcoming_events: Vec<_> = all_events
        .into_iter()
        .filter(|e| e.is_public && e.team_id.as_deref() == Some(&id))
        .collect();

    Ok(Json(PublicTeamDetail {
        team,
        game_name,
        roster,
        record,
        recent_matches,
        upcoming_events,
    }))
}
