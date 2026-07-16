use axum::{
    Json,
    extract::{Path, State},
    http::StatusCode,
};
use chrono::{DateTime, Utc};
use serde::Deserialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, MatchResult, MatchType};
use scuffed_types::api::{CursorResponse, PaginationParams};

use crate::extractors::{OfficerUser, OptionalOrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// Allowed hosts for public VOD links (https only).
const VOD_HOST_ALLOWLIST: &[&str] = &[
    "youtube.com",
    "www.youtube.com",
    "m.youtube.com",
    "youtu.be",
    "twitch.tv",
    "www.twitch.tv",
    "clips.twitch.tv",
    "m.twitch.tv",
];

/// Normalize/validate a VOD URL: https only, host allowlist. Empty → None.
fn normalize_vod_url(raw: &str) -> Result<Option<String>, &'static str> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    let rest = s
        .strip_prefix("https://")
        .ok_or("vod_url must use https")?;
    // host is up to first '/' or '?' or end; reject userinfo/ports oddities simply.
    let host_end = rest
        .find(['/', '?', '#'])
        .unwrap_or(rest.len());
    let host = &rest[..host_end];
    if host.is_empty() || host.contains('@') || host.contains(' ') {
        return Err("vod_url must be a valid URL");
    }
    // Strip optional :port (not expected for these hosts).
    let host = host.split(':').next().unwrap_or(host);
    if !VOD_HOST_ALLOWLIST
        .iter()
        .any(|h| host.eq_ignore_ascii_case(h))
    {
        return Err("vod_url host not allowed (youtube/twitch only)");
    }
    Ok(Some(s.to_string()))
}

/// OW2 replay code: ≤16 alphanumeric. Empty → None.
fn normalize_replay_code(raw: &str) -> Result<Option<String>, &'static str> {
    let s = raw.trim();
    if s.is_empty() {
        return Ok(None);
    }
    if s.len() > 16 {
        return Err("replay_code too long (max 16)");
    }
    if !s.chars().all(|c| c.is_ascii_alphanumeric()) {
        return Err("replay_code must be alphanumeric");
    }
    Ok(Some(s.to_string()))
}

fn bad_request(msg: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::BAD_REQUEST,
        Json(ErrorResponse {
            error: msg.into(),
        }),
    )
}

fn map_media_opt(
    value: &Option<Option<String>>,
    f: fn(&str) -> Result<Option<String>, &'static str>,
) -> Result<Option<Option<String>>, (StatusCode, Json<ErrorResponse>)> {
    match value {
        None => Ok(None),
        Some(None) => Ok(Some(None)),
        Some(Some(raw)) => match f(raw) {
            Ok(v) => Ok(Some(v)),
            Err(msg) => Err(bad_request(msg)),
        },
    }
}

/// GET /api/teams/:id/matches — team match history (cursor-paginated).
/// Anonymous: public non-scrim only (notes/recorded_by stripped).
/// Org members: full rows.
pub async fn list_team_matches(
    State(state): State<AppState>,
    OptionalOrgMember(member): OptionalOrgMember,
    Path(team_id): Path<String>,
    axum::extract::Query(pagination): axum::extract::Query<PaginationParams>,
) -> Result<Json<CursorResponse<MatchResult>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let only_public = member.is_none();
    let items = state
        .db
        .list_team_matches_paginated(&team_id, limit, offset, only_public, false)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    // Public path already filtered in SQL; still strip internal fields.
    let items = if only_public {
        items
            .into_iter()
            .map(|mut m| {
                m.notes = None;
                m.recorded_by = None;
                m
            })
            .collect()
    } else {
        items
    };
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

#[derive(Deserialize)]
pub struct RecordMatchRequest {
    pub team_id: String,
    pub opponent: String,
    #[serde(default)]
    pub score_us: Option<u32>,
    #[serde(default)]
    pub score_them: Option<u32>,
    pub map_name: Option<String>,
    pub game_mode: Option<String>,
    #[serde(default)]
    pub match_type: MatchType,
    #[serde(default)]
    pub played_at: Option<DateTime<Utc>>,
    #[serde(default)]
    pub scheduled_at: Option<DateTime<Utc>>,
    pub notes: Option<String>,
    #[serde(default)]
    pub is_public: bool,
    #[serde(default)]
    pub vod_url: Option<String>,
    #[serde(default)]
    pub replay_code: Option<String>,
}

/// POST /api/matches — record match result or schedule a fixture (officer+)
pub async fn record_match(
    State(state): State<AppState>,
    officer: OfficerUser,
    Json(body): Json<RecordMatchRequest>,
) -> Result<(StatusCode, Json<MatchResult>), (StatusCode, Json<ErrorResponse>)> {
    if body.opponent.trim().is_empty() {
        return Err(bad_request("opponent is required"));
    }
    // Need a time anchor: played, scheduled, or scores (implies played).
    let has_scores = body.score_us.is_some() || body.score_them.is_some();
    if body.played_at.is_none() && body.scheduled_at.is_none() && !has_scores {
        return Err(bad_request(
            "provide played_at, scheduled_at, or scores for the match",
        ));
    }
    // Partial scores are invalid.
    match (body.score_us, body.score_them) {
        (Some(_), None) | (None, Some(_)) => {
            return Err(bad_request("score_us and score_them must both be set or both omitted"));
        }
        _ => {}
    }

    let vod_url = match body.vod_url.as_deref() {
        None => None,
        Some(raw) => normalize_vod_url(raw).map_err(bad_request)?,
    };
    let replay_code = match body.replay_code.as_deref() {
        None => None,
        Some(raw) => normalize_replay_code(raw).map_err(bad_request)?,
    };

    // If scores are present and played_at is missing, stamp now so recent filters work.
    let played_at = if body.played_at.is_none() && has_scores {
        Some(Utc::now())
    } else {
        body.played_at
    };

    let result = state
        .db
        .record_match(
            &body.team_id,
            body.opponent.trim(),
            body.score_us,
            body.score_them,
            body.map_name.as_deref(),
            body.game_mode.as_deref(),
            body.match_type,
            played_at,
            body.scheduled_at,
            &officer.member.id,
            body.notes.as_deref(),
            body.is_public,
            vod_url.as_deref(),
            replay_code.as_deref(),
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
    let score_label = match (result.score_us, result.score_them) {
        (Some(u), Some(t)) => format!("{u}-{t}"),
        _ => "scheduled".into(),
    };
    audit(
        &state.db,
        &officer.member.id,
        AuditAction::RecordedMatch,
        AuditTargetType::Match,
        &result.id,
        Some(&format!(
            "{} vs {} ({})",
            result.match_type, result.opponent, score_label
        )),
    )
    .await;

    Ok((StatusCode::CREATED, Json(result)))
}

#[derive(Deserialize)]
pub struct UpdateMatchRequest {
    pub opponent: Option<String>,
    /// Double-option: omit=leave, null=clear, value=set.
    pub score_us: Option<Option<u32>>,
    pub score_them: Option<Option<u32>>,
    pub map_name: Option<Option<String>>,
    pub game_mode: Option<Option<String>>,
    pub match_type: Option<MatchType>,
    pub notes: Option<Option<String>>,
    pub is_public: Option<bool>,
    pub played_at: Option<Option<DateTime<Utc>>>,
    pub scheduled_at: Option<Option<DateTime<Utc>>>,
    pub vod_url: Option<Option<String>>,
    pub replay_code: Option<Option<String>>,
}

/// PUT /api/matches/:id — update match (officer+)
pub async fn update_match(
    State(state): State<AppState>,
    officer: OfficerUser,
    Path(id): Path<String>,
    Json(body): Json<UpdateMatchRequest>,
) -> Result<Json<MatchResult>, (StatusCode, Json<ErrorResponse>)> {
    // Validate partial score pairs when either side is provided as a concrete value.
    match (&body.score_us, &body.score_them) {
        (Some(Some(_)), Some(None)) | (Some(None), Some(Some(_))) => {
            return Err(bad_request(
                "score_us and score_them must both be set or both cleared",
            ));
        }
        (Some(Some(_)), None) | (None, Some(Some(_))) => {
            return Err(bad_request(
                "score_us and score_them must both be provided together",
            ));
        }
        _ => {}
    }

    let vod_url = map_media_opt(&body.vod_url, normalize_vod_url)?;
    let replay_code = map_media_opt(&body.replay_code, normalize_replay_code)?;

    let result = state
        .db
        .update_match(
            &id,
            body.opponent.as_deref(),
            body.score_us,
            body.score_them,
            body.map_name.as_ref().map(|m| m.as_deref()),
            body.game_mode.as_ref().map(|g| g.as_deref()),
            body.match_type,
            body.notes.as_ref().map(|n| n.as_deref()),
            body.is_public,
            body.played_at,
            body.scheduled_at,
            vod_url.as_ref().map(|v| v.as_deref()),
            replay_code.as_ref().map(|r| r.as_deref()),
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
        &officer.member.id,
        AuditAction::UpdatedMatch,
        AuditTargetType::Match,
        &id,
        None,
    )
    .await;

    Ok(Json(result))
}

#[cfg(test)]
mod media_validation_tests {
    use super::*;

    #[test]
    fn vod_accepts_youtube_https() {
        let v = normalize_vod_url("https://www.youtube.com/watch?v=abc123").unwrap();
        assert!(v.is_some());
    }

    #[test]
    fn vod_rejects_http() {
        assert!(normalize_vod_url("http://www.youtube.com/watch?v=abc").is_err());
    }

    #[test]
    fn vod_rejects_unknown_host() {
        assert!(normalize_vod_url("https://example.com/v/1").is_err());
    }

    #[test]
    fn replay_accepts_alnum_16() {
        assert_eq!(
            normalize_replay_code("Ab12Cd34Ef56Gh78").unwrap().as_deref(),
            Some("Ab12Cd34Ef56Gh78")
        );
    }

    #[test]
    fn replay_rejects_long_or_symbols() {
        assert!(normalize_replay_code("ABCDEFGHIJKLMNOPQ").is_err());
        assert!(normalize_replay_code("AB-12").is_err());
    }
}
