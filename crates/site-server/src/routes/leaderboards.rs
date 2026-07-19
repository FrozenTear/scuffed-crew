use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, HeroStats, MemberLeaderboardRow, Season};
use scuffed_types::{HEROES, HeroAgg, MemberLeaderboardRow as TypesMemberRow};

use crate::extractors::AdminUser;
use crate::routes::audit_log::audit;
use crate::state::AppState;

fn hero_stats_to_agg(h: HeroStats) -> HeroAgg {
    let winrate = if h.matches > 0 {
        h.wins as f32 / h.matches as f32
    } else {
        0.0
    };
    HeroAgg {
        hero: h.hero,
        games: h.matches,
        wins: h.wins,
        losses: h.losses,
        draws: h.draws,
        winrate,
        avg_elims: h.avg_elims,
        avg_deaths: h.avg_deaths,
    }
}

fn map_lb_row(r: MemberLeaderboardRow) -> TypesMemberRow {
    TypesMemberRow {
        member_id: r.member_id,
        display_name: r.display_name,
        games: r.games,
        winrate: r.winrate,
        kd: r.kd,
    }
}

#[derive(Deserialize)]
pub struct HeroesQuery {
    #[serde(default = "default_top")]
    pub top: u32,
}

fn default_top() -> u32 {
    3
}

/// GET /api/public/members/:id/heroes?top=3 — public top heroes (no auth).
///
/// `top=0` returns **all** heroes (hero-stats W2 B4). Default remains 3.
/// Non-zero values are clamped to 1..=50.
pub async fn public_member_heroes(
    State(state): State<AppState>,
    Path(id): Path<String>,
    Query(q): Query<HeroesQuery>,
) -> Result<Json<Vec<HeroAgg>>, (StatusCode, Json<ErrorResponse>)> {
    let top = if q.top == 0 { 0 } else { q.top.clamp(1, 50) };
    // Ensure member exists (safe projection).
    let exists = state
        .db
        .get_member_safe(&id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?
        .is_some();
    if !exists {
        return Err((
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Member not found".into(),
            }),
        ));
    }
    let heroes = state.db.top_heroes(&id, top).await.map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })?;
    Ok(Json(heroes.into_iter().map(hero_stats_to_agg).collect()))
}

#[derive(Deserialize)]
pub struct LeaderboardQuery {
    #[serde(default = "default_metric")]
    pub metric: String,
    #[serde(default = "default_limit")]
    pub limit: u32,
    /// Optional season id — filters aggregates to `played_at` in [starts_at, ends_at).
    pub season: Option<String>,
    /// Optional hero filter (hero-stats W3 B2). Empty/omitted = all heroes.
    /// Must match a canonical [`HEROES`] entry (case-insensitive); unknown → 400.
    pub hero: Option<String>,
}

fn default_metric() -> String {
    "winrate".into()
}

fn default_limit() -> u32 {
    25
}

/// Resolve query `hero=` to a canonical HEROES display name.
/// Empty / whitespace-only → no filter. Unknown name → None (caller returns 400).
fn resolve_leaderboard_hero(raw: Option<&str>) -> Result<Option<&'static str>, ()> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    let lower = trimmed.to_lowercase();
    for &hero in HEROES {
        if hero.to_lowercase() == lower {
            return Ok(Some(hero));
        }
    }
    Err(())
}

/// GET /api/public/leaderboards?metric=winrate|kd|games&limit=25&season=<id>&hero=<name>
pub async fn public_leaderboards(
    State(state): State<AppState>,
    Query(q): Query<LeaderboardQuery>,
) -> Result<Json<Vec<TypesMemberRow>>, (StatusCode, Json<ErrorResponse>)> {
    let metric = match q.metric.as_str() {
        "kd" | "games" | "winrate" => q.metric.as_str(),
        _ => "winrate",
    };
    let limit = q.limit.clamp(1, 100);

    let season_window = if let Some(ref sid) = q.season {
        let sid = sid.trim();
        if sid.is_empty() {
            None
        } else {
            let season = state.db.get_season(sid).await.map_err(|_e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            })?;
            let Some(season) = season else {
                return Err((
                    StatusCode::NOT_FOUND,
                    Json(ErrorResponse {
                        error: "Season not found".into(),
                    }),
                ));
            };
            Some((season.starts_at, season.ends_at))
        }
    } else {
        None
    };

    // W3 B2: optional ?hero= → canonical HEROES name, then DB bound filter.
    let hero = match resolve_leaderboard_hero(q.hero.as_deref()) {
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

    let rows = state
        .db
        .member_leaderboard(metric, limit, season_window, hero)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(rows.into_iter().map(map_lb_row).collect()))
}

#[cfg(test)]
mod resolve_hero_tests {
    use super::resolve_leaderboard_hero;

    #[test]
    fn empty_or_missing_is_no_filter() {
        assert_eq!(resolve_leaderboard_hero(None), Ok(None));
        assert_eq!(resolve_leaderboard_hero(Some("")), Ok(None));
        assert_eq!(resolve_leaderboard_hero(Some("   ")), Ok(None));
    }

    #[test]
    fn case_insensitive_canonical() {
        assert_eq!(resolve_leaderboard_hero(Some("ana")), Ok(Some("Ana")));
        assert_eq!(
            resolve_leaderboard_hero(Some("Wrecking Ball")),
            Ok(Some("Wrecking Ball"))
        );
        assert_eq!(resolve_leaderboard_hero(Some("d.va")), Ok(Some("D.Va")));
    }

    #[test]
    fn unknown_is_err() {
        assert!(resolve_leaderboard_hero(Some("NotAHero")).is_err());
    }
}

/// GET /api/public/seasons — list seasons (public).
pub async fn public_list_seasons(
    State(state): State<AppState>,
) -> Result<Json<Vec<Season>>, (StatusCode, Json<ErrorResponse>)> {
    state.db.list_seasons().await.map(Json).map_err(|_e| {
        (
            StatusCode::INTERNAL_SERVER_ERROR,
            Json(ErrorResponse {
                error: "Internal error".into(),
            }),
        )
    })
}

#[derive(Deserialize)]
pub struct CreateSeasonRequest {
    pub name: String,
    pub starts_at: DateTime<Utc>,
    pub ends_at: DateTime<Utc>,
    #[serde(default)]
    pub is_current: bool,
}

/// POST /api/admin/seasons — create season (admin).
pub async fn admin_create_season(
    State(state): State<AppState>,
    admin: AdminUser,
    Json(body): Json<CreateSeasonRequest>,
) -> Result<(StatusCode, Json<Season>), (StatusCode, Json<ErrorResponse>)> {
    if body.name.trim().is_empty() {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "name is required".into(),
            }),
        ));
    }
    if body.ends_at <= body.starts_at {
        return Err((
            StatusCode::BAD_REQUEST,
            Json(ErrorResponse {
                error: "ends_at must be after starts_at".into(),
            }),
        ));
    }
    let s = state
        .db
        .create_season(
            body.name.trim(),
            body.starts_at,
            body.ends_at,
            body.is_current,
        )
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;

    audit(
        &state.db,
        &admin.member.id,
        AuditAction::CreatedSeason,
        AuditTargetType::Season,
        &s.id,
        Some(s.name.as_str()),
    )
    .await;

    Ok((StatusCode::CREATED, Json(s)))
}

/// GET /api/admin/seasons — list seasons (admin; same data as public for now).
pub async fn admin_list_seasons(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<Season>>, (StatusCode, Json<ErrorResponse>)> {
    public_list_seasons(State(state)).await
}

#[derive(Serialize)]
pub struct LeaderboardPageMeta {
    pub metric: String,
    pub count: usize,
}
