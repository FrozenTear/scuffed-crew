use axum::{
    extract::{Path, State},
    http::StatusCode,
    Json,
};
use serde::Serialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    Announcement, Event, Game, GameAccount, MatchResult, Member, SiteSettings, Team, TeamRecord,
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
}

/// GET /api/public/overview — aggregated public data for the site
pub async fn overview(
    State(state): State<AppState>,
) -> Result<Json<PublicOverview>, (StatusCode, Json<ErrorResponse>)> {
    let teams = state.db.list_teams().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let mut team_overviews = Vec::with_capacity(teams.len());
    for team in teams {
        let roster = state.db.get_team_roster(&team.id).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
        let record = state.db.get_team_record(&team.id).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
                }),
            )
        })?;
        team_overviews.push(TeamOverview {
            roster_count: roster.len(),
            team,
            record,
        });
    }

    let events = state.db.list_events().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let announcements = state.db.list_announcements().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let settings = state.db.get_settings().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let members = state.db.list_members().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let games = state.db.list_games().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok(Json(PublicOverview {
        teams: team_overviews,
        games,
        events,
        announcements,
        settings,
        member_count: members.len(),
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
    pub joined_at: String,
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
        joined_at: m.joined_at.to_rfc3339(),
    }
}

/// GET /api/public/members — public member list
pub async fn public_members(
    State(state): State<AppState>,
) -> Result<Json<Vec<PublicMember>>, (StatusCode, Json<ErrorResponse>)> {
    let members = state.db.list_members().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    Ok(Json(members.into_iter().map(member_to_public).collect()))
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
        .get_member(&id)
        .await
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
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
    let teams = state.db.list_teams().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    // Find roster entries for this member across all teams
    let mut member_teams = Vec::new();
    for team in &teams {
        let roster = state.db.get_team_roster(&team.id).await.map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
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
    pub recent_matches: Vec<MatchResult>,
    pub upcoming_events: Vec<Event>,
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
        .map_err(|e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: e.to_string(),
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
    let roster_entries = state.db.get_team_roster(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let mut roster = Vec::new();
    for entry in roster_entries {
        if entry.is_active {
            let member = state.db.get_member(&entry.member_id).await.ok().flatten();
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

    let record = state.db.get_team_record(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;

    let recent_matches = state.db.list_team_matches(&id).await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;
    // Limit to 10 most recent
    let recent_matches: Vec<_> = recent_matches.into_iter().take(10).collect();

    // Get events for this team
    let all_events = state.db.list_events().await.map_err(|e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: e.to_string(),
            }),
        )
    })?;
    let upcoming_events: Vec<_> = all_events
        .into_iter()
        .filter(|e| e.team_id.as_deref() == Some(&id))
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
