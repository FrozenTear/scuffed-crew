use std::collections::HashMap;

use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    Announcement, Event, Game, GameAccount, MatchResult, MatchType, Member, NamedRosterEntry,
    SiteSettings, Team, TeamRecord,
};
use scuffed_types::api::{CursorResponse, PaginationParams};
use scuffed_types::{
    MatchResult as TypesMatchResult, MatchType as TypesMatchType, PublicMatch, RecentResult,
    UpcomingMatch, resolve_hero_query,
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

/// Nested per-hero stats when the roster is fetched with `?hero=` (hero-stats W3 B3 / Q2).
/// `winrate` is a 0.0–1.0 fraction. Absent when no hero filter or the member has
/// no matches on that hero.
#[derive(Debug, Clone, Serialize)]
pub struct HeroScoped {
    pub games: u32,
    pub winrate: f32,
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
    /// Present only when list was requested with a valid `?hero=` filter and
    /// this member has ≥1 game on that hero (hero-stats W3 B3).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub hero_scoped: Option<HeroScoped>,
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
        hero_scoped: None,
    }
}

/// Query for `GET /api/public/members` — pagination + optional hero filter (W3 B3).
///
/// Fields are inlined (not `#[serde(flatten)]` of `PaginationParams`) because
/// axum's `Query` uses `serde_urlencoded`, which does not support `flatten` —
/// a flattened struct fails to deserialize once a sibling field like `hero` is
/// present, yielding a spurious 400.
#[derive(Debug, Deserialize)]
pub struct PublicMembersQuery {
    /// Opaque pagination cursor (see [`PaginationParams`]).
    pub cursor: Option<String>,
    /// Items per page; falls back to the `PaginationParams` default (25) when omitted.
    pub limit: Option<u32>,
    /// Optional hero filter. Empty/omitted = no `hero_scoped`. Unknown → 400.
    /// Case-insensitive match against canonical [`HEROES`] names.
    pub hero: Option<String>,
}

/// Resolve query `hero=` to a canonical HEROES display name (same contract as W3 B2).
fn resolve_members_hero(raw: Option<&str>) -> Result<Option<&'static str>, ()> {
    resolve_hero_query(raw)
}

/// GET /api/public/members — public member list (cursor-paginated).
///
/// Optional `?hero=<name>` (hero-stats W3 B3): attaches `hero_scoped{games,winrate}`
/// for members who have played that hero. Unknown hero → 400.
///
/// HS-DR P1: hero attach is **page-scoped** (`hero_scoped_for_members` over the
/// current page's ids only) — not a full-table `member_leaderboard(500)`.
pub async fn public_members(
    State(state): State<AppState>,
    Query(q): Query<PublicMembersQuery>,
) -> Result<Json<CursorResponse<PublicMember>>, (StatusCode, Json<ErrorResponse>)> {
    let pagination = PaginationParams {
        cursor: q.cursor.clone(),
        limit: q.limit.unwrap_or(25),
    };
    let (limit, offset) = pagination.resolve();
    let hero = match resolve_members_hero(q.hero.as_deref()) {
        Ok(h) => h,
        Err(()) => {
            return Err((
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "Unknown hero".into(),
                }),
            ));
        }
    };

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

    let scoped: HashMap<String, HeroScoped> = if let Some(hero_name) = hero {
        let ids: Vec<String> = members.iter().map(|m| m.id.clone()).collect();
        let rows = state
            .db
            .hero_scoped_for_members(&ids, hero_name)
            .await
            .map_err(|_e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            })?;
        rows.into_iter()
            .map(|r| {
                (
                    r.member_id,
                    HeroScoped {
                        games: r.games,
                        winrate: r.winrate,
                    },
                )
            })
            .collect()
    } else {
        HashMap::new()
    };

    let public: Vec<PublicMember> = members
        .into_iter()
        .map(|m| {
            let mut p = member_to_public(m);
            if hero.is_some() {
                p.hero_scoped = scoped.get(&p.id).cloned();
            }
            p
        })
        .collect();
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

    // Get roster with member names — single db-level join (no per-member N+1).
    let roster_entries = state.db.get_team_roster_named(&id).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;

    let roster: Vec<TeamRosterMember> = roster_entries
        .into_iter()
        .map(|entry| {
            let NamedRosterEntry {
                member_id,
                member_name,
                avatar_url,
                team_role,
                ..
            } = entry;
            let display_name = member_name.unwrap_or_else(|| {
                tracing::warn!(
                    member_id = %member_id,
                    team_id = %id,
                    "roster references a member with no row (dangling plays_on edge)"
                );
                "Unknown".to_string()
            });
            TeamRosterMember {
                member_id,
                display_name,
                avatar_url,
                team_role: team_role.to_string(),
            }
        })
        .collect();

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

#[cfg(test)]
mod resolve_members_hero_tests {
    use super::resolve_members_hero;

    #[test]
    fn empty_is_no_filter() {
        assert_eq!(resolve_members_hero(None), Ok(None));
        assert_eq!(resolve_members_hero(Some("")), Ok(None));
        assert_eq!(resolve_members_hero(Some("   ")), Ok(None));
    }

    #[test]
    fn case_insensitive_canonical() {
        assert_eq!(resolve_members_hero(Some("ana")), Ok(Some("Ana")));
        assert_eq!(
            resolve_members_hero(Some("Wrecking Ball")),
            Ok(Some("Wrecking Ball"))
        );
    }

    #[test]
    fn unknown_errors() {
        assert!(resolve_members_hero(Some("NotAHero")).is_err());
    }
}
