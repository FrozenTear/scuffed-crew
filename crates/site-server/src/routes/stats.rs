use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    AuditAction, AuditTargetType, DaemonToken, HeroStats, MapStats, PersonalMatch, PersonalStats,
    RoleStats,
};
use scuffed_types::api::{
    CreateDaemonTokenRequest, CreateDaemonTokenResponse, CursorResponse, DaemonConfigResponse,
    MemberSettingsResponse, PaginationParams, RECOGNIZER_ID_ERROR, SUSPECT_FIELDS_ERROR,
    SeasonQuery, StatsUploadBody, StatsUploadResponse, TokenCheckResponse,
    UpdateMemberSettingsRequest, resolve_recognizer, resolve_suspect_fields,
};
use scuffed_types::{hero_key_to_name, role_for_hero_name};

use crate::extractors::{DaemonUser, OpaqueDaemonUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::routes::leaderboards::resolve_season_window;
use crate::state::AppState;

/// Store a known hero under its display name, with the role from the hero
/// table.
///
/// 0.5.0-alpha.1 and alpha.2 trackers on the new reader send the pack key
/// (`wrecking-ball`, `ana`) as the hero and guess the role from it, which
/// gives Damage for hyphenated tanks and supports. Leaderboards and hero
/// stats group on the exact string, so `ana` and `Ana` split. A name that is
/// not a known hero (including `Unknown`) is stored as sent.
fn normalize_hero(hero: String, role: String) -> (String, String) {
    let Some(display) = hero_key_to_name(&hero) else {
        return (hero, role);
    };
    let role = role_for_hero_name(display).map_or(role, |r| r.to_string());
    (display.to_string(), role)
}

/// POST /api/stats/upload — bulk upload personal matches (daemon token auth)
pub async fn upload_stats(
    State(state): State<AppState>,
    daemon: DaemonUser,
    Json(body): Json<StatsUploadBody>,
) -> Result<Json<StatsUploadResponse>, (StatusCode, Json<ErrorResponse>)> {
    if body.matches.is_empty() && body.deleted_sessions.is_empty() {
        return Ok(Json(StatsUploadResponse {
            inserted: 0,
            skipped: 0,
            deleted: 0,
        }));
    }

    // Reject the whole batch before any write when a recognizer id, a
    // suspect_fields list, or a rank-screen field has a JSON type this route
    // cannot decode. A missing required core field fails the same way, during
    // JSON decode. deleted_sessions is not applied either.
    // A rank-screen value of the right JSON type that is out of range or not
    // allowed is dropped (stored as none) so the scoreboard stats are kept.
    // Those drops are logged after this loop, once the batch is accepted.
    let mut recognizers = Vec::with_capacity(body.matches.len());
    let mut suspect_lists = Vec::with_capacity(body.matches.len());
    let mut rank_screens = Vec::with_capacity(body.matches.len());
    for (i, entry) in body.matches.iter().enumerate() {
        match resolve_recognizer(&entry.recognizer) {
            Ok(id) => recognizers.push(id),
            Err(_) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("matches[{i}]: {RECOGNIZER_ID_ERROR}"),
                    }),
                ));
            }
        }
        match resolve_suspect_fields(&entry.suspect_fields) {
            Ok(names) => suspect_lists.push(names),
            Err(_) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("matches[{i}]: {SUSPECT_FIELDS_ERROR}"),
                    }),
                ));
            }
        }
        match &entry.rank_screen {
            Ok(parsed) => rank_screens.push(parsed.clone()),
            Err(message) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("matches[{i}]: {message}"),
                    }),
                ));
            }
        }
    }

    for (entry, parsed) in body.matches.iter().zip(&rank_screens) {
        for dropped in &parsed.drops {
            tracing::warn!(
                match_id = %entry.entry.session_id,
                field = dropped.field,
                reason = dropped.reason,
                "dropped rank-screen field"
            );
        }
    }

    let total = body.matches.len() as u32;

    // Entries whose outcome the schema would reject ('unknown', garbage) are
    // skipped, not 500'd: a single bad entry used to fail the whole batch,
    // and the client retried it forever — wedging sync for everything behind
    // it. Well-behaved clients hold 'unknown' back until the outcome is known.
    let stub_matches: Vec<PersonalMatch> = body
        .matches
        .into_iter()
        .zip(recognizers)
        .zip(suspect_lists)
        .zip(rank_screens)
        .filter(|(((e, _), _), _)| {
            matches!(e.entry.outcome.as_str(), "victory" | "defeat" | "draw")
        })
        .map(|(((e, recognizer), suspect_fields), parsed)| {
            let rank = parsed.fields;
            let e = e.entry;
            let (hero, role) = normalize_hero(e.hero, e.role);
            PersonalMatch {
                id: String::new(),
                member_id: daemon.member.id.clone(),
                session_id: e.session_id,
                hero,
                map_name: e.map_name,
                game_mode: e.game_mode,
                role,
                outcome: e.outcome,
                elims: e.elims,
                deaths: e.deaths,
                assists: e.assists,
                damage: e.damage,
                healing: e.healing,
                mitigation: e.mitigation,
                played_at: e.played_at,
                uploaded_at: chrono::Utc::now(),
                edited: e.edited,
                recognizer,
                suspect_fields,
                rank,
            }
        })
        .collect();
    let dropped = total - stub_matches.len() as u32;
    if dropped > 0 {
        tracing::warn!(
            member_id = %daemon.member.id,
            dropped,
            "stats upload contained entries with unstorable outcomes — skipped"
        );
    }

    let inserted = if stub_matches.is_empty() {
        0
    } else {
        state
            .db
            .upsert_personal_matches(&daemon.member.id, &stub_matches)
            .await
            .map_err(|_e| {
                (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Json(ErrorResponse {
                        error: "Internal error".into(),
                    }),
                )
            })?
    };

    // Tombstones: sessions the user deleted locally. Scoped to this member's
    // rows by the query itself.
    let deleted = state
        .db
        .delete_personal_matches_by_sessions(&daemon.member.id, &body.deleted_sessions)
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
        &daemon.member.id,
        AuditAction::UploadedPersonalStats,
        AuditTargetType::PersonalStats,
        &daemon.member.id,
        Some(&format!("{inserted} matches uploaded, {deleted} deleted")),
    )
    .await;

    Ok(Json(StatsUploadResponse {
        inserted,
        skipped: total - inserted,
        deleted,
    }))
}

/// GET /api/stats/me — personal stats overview (session auth)
pub async fn my_stats(
    State(state): State<AppState>,
    member: OrgMember,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<PersonalStats>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_personal_stats_in(&member.member.id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/stats/me/matches — personal match history (session auth, paginated)
pub async fn my_matches(
    State(state): State<AppState>,
    member: OrgMember,
    Query(pagination): Query<PaginationParams>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<CursorResponse<PersonalMatch>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    let items = state
        .db
        .list_personal_matches_in(&member.member.id, limit, offset, season)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

/// GET /api/stats/me/heroes — per-hero stats (session auth)
pub async fn my_hero_stats(
    State(state): State<AppState>,
    member: OrgMember,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<HeroStats>>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_hero_stats_in(&member.member.id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/stats/me/roles — per-role stats from the stored match role (session auth).
///
/// Query: `?season=<id>`, the same window as [`my_hero_stats`] (omitted or blank
/// is all time; an unknown id is 404). No other filters.
///
/// Response: a JSON array of [`RoleStats`] (`role`, `matches`, `wins`, `losses`,
/// `draws`, `avg_elims`, `avg_deaths`, `avg_damage`, `avg_healing`). Grouped by
/// the `personal_match.role` column only — never derived from the hero name.
/// An empty stored role is its own row (`"role": ""`). Ordered by `matches`
/// descending, then `role` ascending.
///
/// Auth matches [`my_hero_stats`]: a session is required (401 when anonymous or
/// the bearer is not a session). Inactive or suspended members are 403.
pub async fn my_role_stats(
    State(state): State<AppState>,
    member: OrgMember,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<RoleStats>>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_role_stats_in(&member.member.id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/stats/me/maps — per-map stats (session auth)
pub async fn my_map_stats(
    State(state): State<AppState>,
    member: OrgMember,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<MapStats>>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_map_stats_in(&member.member.id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/stats/member/:id — view another member's stats (session auth)
pub async fn member_stats(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<PersonalStats>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_personal_stats_in(&member_id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/stats/member/:id/heroes — view another member's hero stats
pub async fn member_hero_stats(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<HeroStats>>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_hero_stats_in(&member_id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/stats/member/:id/roles — another member's per-role stats (session auth).
///
/// Same query, [`RoleStats`] shape, and ordering as [`my_role_stats`].
/// Visibility matches [`member_hero_stats`]: any active org member may read any
/// member id. There is no per-member privacy flag. A missing id returns `[]`.
/// Anonymous requests are 401; inactive or suspended callers are 403.
pub async fn member_role_stats(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<RoleStats>>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_role_stats_in(&member_id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// GET /api/stats/member/:id/maps — view another member's map stats
pub async fn member_map_stats(
    State(state): State<AppState>,
    _member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<MapStats>>, (StatusCode, Json<ErrorResponse>)> {
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_map_stats_in(&member_id, season)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// POST /api/stats/tokens — create a daemon token (session auth, for self)
pub async fn create_daemon_token(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<CreateDaemonTokenRequest>,
) -> Result<(StatusCode, Json<CreateDaemonTokenResponse>), (StatusCode, Json<ErrorResponse>)> {
    let raw_token = generate_token();

    let token = state
        .db
        .create_daemon_token(&member.member.id, &raw_token, &body.label)
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
        &member.member.id,
        AuditAction::CreatedDaemonToken,
        AuditTargetType::DaemonToken,
        &token.id,
        Some(&format!("label: {}", body.label)),
    )
    .await;

    Ok((
        StatusCode::CREATED,
        Json(CreateDaemonTokenResponse {
            id: token.id,
            token: raw_token,
            label: token.label,
        }),
    ))
}

/// GET /api/stats/tokens — list own daemon tokens (session auth)
pub async fn list_daemon_tokens(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<Vec<DaemonToken>>, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .list_daemon_tokens(&member.member.id)
        .await
        .map(Json)
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })
}

/// DELETE /api/stats/tokens/:id — revoke a daemon token (session auth)
pub async fn revoke_daemon_token(
    State(state): State<AppState>,
    member: OrgMember,
    Path(token_id): Path<String>,
) -> Result<StatusCode, (StatusCode, Json<ErrorResponse>)> {
    state
        .db
        .revoke_daemon_token(&token_id, &member.member.id)
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
        &member.member.id,
        AuditAction::RevokedDaemonToken,
        AuditTargetType::DaemonToken,
        &token_id,
        None,
    )
    .await;

    Ok(StatusCode::NO_CONTENT)
}

/// GET /api/stats/settings — load player's daemon settings (session auth)
pub async fn get_member_settings(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<MemberSettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let settings = state
        .db
        .get_member_settings(&member.member.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(MemberSettingsResponse {
        player_name: settings.and_then(|s| s.player_name),
    }))
}

/// PUT /api/stats/settings — save player's daemon settings (session auth)
pub async fn update_member_settings(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<UpdateMemberSettingsRequest>,
) -> Result<Json<MemberSettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    // Trim and treat empty string as None
    let player_name = body
        .player_name
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let settings = state
        .db
        .upsert_member_settings(&member.member.id, player_name)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(MemberSettingsResponse {
        player_name: settings.player_name,
    }))
}

/// GET /api/stats/token-check
///
/// Same daemon token auth as `POST /api/stats/upload`. Returns the member
/// display name only. Missing, bad, revoked, and expired tokens share one
/// 401 body.
///
/// `validate_daemon_token` already sets `last_used_at` on success (the upload
/// path does this). This handler does not write. The 200 is `no-store`, same
/// as the daemon-token 401, so a shared cache cannot keep the display name.
pub async fn token_check(
    daemon: OpaqueDaemonUser,
) -> (
    [(header::HeaderName, &'static str); 1],
    Json<TokenCheckResponse>,
) {
    (
        [(header::CACHE_CONTROL, "no-store")],
        Json(TokenCheckResponse {
            display_name: daemon.member.display_name,
        }),
    )
}

/// GET /api/stats/daemon-config — fetch config for daemon (token auth)
pub async fn daemon_config(
    State(state): State<AppState>,
    daemon: DaemonUser,
) -> Result<Json<DaemonConfigResponse>, (StatusCode, Json<ErrorResponse>)> {
    let settings = state
        .db
        .get_member_settings(&daemon.member.id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    Ok(Json(DaemonConfigResponse {
        player_name: settings.and_then(|s| s.player_name),
    }))
}

fn generate_token() -> String {
    use rand::RngCore;
    use std::fmt::Write;
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    let mut s = String::with_capacity(64);
    for b in bytes {
        write!(s, "{b:02x}").unwrap();
    }
    s
}

#[cfg(test)]
mod tests {
    use axum::body::Body;
    use axum::http::{Request, StatusCode, header};
    use chrono::{TimeZone, Utc};
    use http_body_util::BodyExt;
    use scuffed_db::OrgRole;
    use scuffed_types::api::{StatsUploadEntry, StatsUploadRequest, StatsUploadResponse};
    use tower::ServiceExt;

    use crate::create_router;
    use crate::test_support::test_state;

    fn upload_body(session_id: &str, elims: u32, outcome: &str) -> StatsUploadRequest {
        StatsUploadRequest {
            matches: vec![StatsUploadEntry {
                session_id: session_id.to_string(),
                hero: "Ana".into(),
                map_name: "Oasis".into(),
                game_mode: "control".into(),
                role: "Support".into(),
                outcome: outcome.to_string(),
                elims,
                deaths: 1,
                assists: 2,
                damage: 1000,
                healing: 4000,
                mitigation: 0,
                played_at: Utc.with_ymd_and_hms(2026, 7, 1, 20, 0, 0).unwrap(),
                edited: false,
                recognizer: Some(scuffed_types::RECOGNIZER_OCR_V1.into()),
                suspect_fields: Vec::new(),
            }],
            deleted_sessions: vec![],
        }
    }

    async fn post_upload(
        app: axum::Router,
        token: &str,
        body: &StatsUploadRequest,
    ) -> (StatusCode, StatsUploadResponse) {
        let req = Request::builder()
            .method("POST")
            .uri("/api/stats/upload")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        if !status.is_success() {
            panic!(
                "upload failed ({status}): {}",
                String::from_utf8_lossy(&bytes)
            );
        }
        let parsed = serde_json::from_slice(&bytes)
            .unwrap_or_else(|e| panic!("upload body was not StatsUploadResponse ({e}): {bytes:?}"));
        (status, parsed)
    }

    #[tokio::test]
    async fn retried_and_concurrent_upload_returns_success_and_one_row() {
        let state = test_state().await;
        let member = state
            .db
            .create_member("u-m11", "m11player", OrgRole::Member)
            .await
            .unwrap();
        let token = "m11-daemon-token";
        state
            .db
            .create_daemon_token(&member.id, token, "tracker")
            .await
            .unwrap();

        let app = create_router(state.clone());
        let (status, body) =
            post_upload(app.clone(), token, &upload_body("sess-m11", 4, "victory")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(
            (body.inserted, body.skipped, body.deleted),
            (1, 0, 0),
            "daemon contract is inserted/skipped/deleted"
        );

        let (status, body) =
            post_upload(app.clone(), token, &upload_body("sess-m11", 11, "defeat")).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!((body.inserted, body.skipped, body.deleted), (1, 0, 0));

        let mut handles = Vec::new();
        for i in 0..6u32 {
            let app = app.clone();
            let token = token.to_string();
            handles.push(tokio::spawn(async move {
                post_upload(app, &token, &upload_body("sess-m11", 20 + i, "victory")).await
            }));
        }
        for handle in handles {
            let (status, body) = handle.await.unwrap();
            assert_eq!(status, StatusCode::OK);
            assert_eq!((body.inserted, body.skipped, body.deleted), (1, 0, 0));
        }

        let rows = state
            .db
            .list_personal_matches(&member.id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "retried uploads of one session stay one row");
        assert_eq!(rows[0].session_id, "sess-m11");
        assert_eq!(
            rows[0].recognizer, "ocr-v1",
            "a body that omits recognizer is stored as ocr-v1"
        );
    }

    fn match_object(session_id: &str, elims: u32) -> serde_json::Value {
        serde_json::json!({
            "session_id": session_id,
            "hero": "Ana",
            "map_name": "Oasis",
            "game_mode": "control",
            "role": "Support",
            "outcome": "victory",
            "elims": elims,
            "deaths": 1,
            "assists": 2,
            "damage": 1000,
            "healing": 4000,
            "mitigation": 0,
            "played_at": "2026-07-01T20:00:00Z",
            "edited": false
        })
    }

    async fn post_raw(
        app: axum::Router,
        token: &str,
        body: serde_json::Value,
    ) -> (StatusCode, serde_json::Value) {
        let req = Request::builder()
            .method("POST")
            .uri("/api/stats/upload")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .body(Body::from(serde_json::to_vec(&body).unwrap()))
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, parsed)
    }

    async fn daemon() -> (crate::state::AppState, String, String) {
        let state = test_state().await;
        let member = state
            .db
            .create_member("u-rec", "recplayer", OrgRole::Member)
            .await
            .unwrap();
        let token = "rec-daemon-token".to_string();
        state
            .db
            .create_daemon_token(&member.id, &token, "tracker")
            .await
            .unwrap();
        (state, member.id, token)
    }

    #[test]
    fn normalize_hero_maps_pack_keys_to_display_name_and_table_role() {
        use super::normalize_hero;
        let n = |hero: &str, role: &str| normalize_hero(hero.into(), role.into());
        assert_eq!(
            n("wrecking-ball", "Damage"),
            ("Wrecking Ball".into(), "Tank".into())
        );
        assert_eq!(
            n("junker-queen", "Damage"),
            ("Junker Queen".into(), "Tank".into())
        );
        assert_eq!(n("ana", "Support"), ("Ana".into(), "Support".into()));
        assert_eq!(
            n("soldier-76", "Damage"),
            ("Soldier: 76".into(), "Damage".into())
        );
        assert_eq!(n("Ana", "Support"), ("Ana".into(), "Support".into()));
    }

    #[test]
    fn normalize_hero_keeps_unknown_heroes_as_sent() {
        use super::normalize_hero;
        let n = |hero: &str, role: &str| normalize_hero(hero.into(), role.into());
        assert_eq!(n("Unknown", "Damage"), ("Unknown".into(), "Damage".into()));
        assert_eq!(
            n("not-a-hero", "Support"),
            ("not-a-hero".into(), "Support".into())
        );
        assert_eq!(n("", "Tank"), ("".into(), "Tank".into()));
    }

    #[tokio::test]
    async fn upload_stores_pack_key_hero_as_display_name_and_table_role() {
        let (state, member_id, token) = daemon().await;
        let app = create_router(state.clone());
        let mut entry = match_object("sess-pack-key", 5);
        entry["hero"] = "wrecking-ball".into();
        entry["role"] = "Damage".into();
        let (status, _) = post_raw(app, &token, serde_json::json!({ "matches": [entry] })).await;
        assert_eq!(status, StatusCode::OK);

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hero, "Wrecking Ball");
        assert_eq!(rows[0].role, "Tank");
    }

    #[tokio::test]
    async fn upload_without_recognizer_stores_ocr_v1_and_keeps_response_shape() {
        let (state, member_id, token) = daemon().await;
        let app = create_router(state.clone());
        let body = serde_json::json!({
            "matches": [match_object("sess-omit", 4)]
        });
        let (status, parsed) = post_raw(app, &token, body).await;
        assert_eq!(status, StatusCode::OK);
        let mut keys: Vec<&str> = parsed
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect();
        keys.sort_unstable();
        assert_eq!(
            keys,
            ["deleted", "inserted", "skipped"],
            "upload response stays inserted/skipped/deleted"
        );
        assert_eq!(parsed["inserted"], 1);
        assert_eq!(parsed["skipped"], 0);
        assert_eq!(parsed["deleted"], 0);

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].recognizer, "ocr-v1");
        assert!(rows[0].suspect_fields.is_empty());
        assert_eq!(rows[0].elims, 4);
        let stats = state.db.get_personal_stats(&member_id).await.unwrap();
        assert_eq!(stats.wins, 1);
    }

    #[tokio::test]
    async fn upload_cv_v1_stores_cv_v1() {
        let (state, member_id, token) = daemon().await;
        let mut entry = match_object("sess-cv", 8);
        entry["recognizer"] = serde_json::json!("cv-v1");
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [entry] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 1);

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows[0].recognizer, "cv-v1");
        assert_eq!(rows[0].elims, 8);
        let stats = state.db.get_personal_stats(&member_id).await.unwrap();
        assert_eq!((stats.total_matches, stats.wins), (1, 1));
    }

    #[tokio::test]
    async fn upload_null_recognizer_stores_ocr_v1() {
        let (state, member_id, token) = daemon().await;
        let mut entry = match_object("sess-null", 3);
        entry["recognizer"] = serde_json::Value::Null;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [entry] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows[0].recognizer, "ocr-v1");
    }

    #[tokio::test]
    async fn upload_invalid_recognizer_is_400_and_stores_nothing() {
        let (state, member_id, token) = daemon().await;
        // A good row is stored first. A later batch that mixes a valid id
        // with garbage must not update that row or insert the sibling.
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-keep", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let mut bad = match_object("sess-keep", 99);
        bad["recognizer"] = serde_json::json!("NOPE");
        let mut sibling = match_object("sess-new", 1);
        sibling["recognizer"] = serde_json::json!("cv-v1");
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [sibling, bad] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            parsed["error"],
            "matches[1]: recognizer must be a string of 1-32 characters in [a-z0-9.-] including a letter or digit"
        );

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "rejected batch must not insert or update");
        assert_eq!(rows[0].session_id, "sess-keep");
        assert_eq!(rows[0].elims, 4);
        assert_eq!(rows[0].recognizer, "ocr-v1");

        let mut empty = match_object("sess-empty", 1);
        empty["recognizer"] = serde_json::json!("");
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [empty] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let mut number = match_object("sess-num", 1);
        number["recognizer"] = serde_json::json!(1);
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [number] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
    }

    #[tokio::test]
    async fn historical_upload_body_without_optional_fields_stores_ocr_v1() {
        let (state, member_id, token) = daemon().await;
        let body = serde_json::json!({
            "matches": [{
                "hero": "Ana",
                "map_name": "Oasis",
                "game_mode": "control",
                "role": "Support",
                "outcome": "victory",
                "played_at": "2026-07-01T20:00:00Z"
            }]
        });
        let (status, parsed) = post_raw(create_router(state.clone()), &token, body).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 1);
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].recognizer, "ocr-v1");
        assert!(rows[0].suspect_fields.is_empty());
        assert_eq!(rows[0].elims, 0);
        assert!(!rows[0].edited);
    }

    #[tokio::test]
    async fn upload_non_ascii_recognizer_returns_400() {
        let (state, member_id, token) = daemon().await;
        // `é` and U+2011 (a Unicode hyphen, not ASCII `-`) are outside [a-z0-9.-].
        for id in ["cv-v1\u{00e9}", "ocr\u{2011}v1"] {
            let mut entry = match_object("sess-non-ascii", 2);
            entry["recognizer"] = serde_json::json!(id);
            let (status, parsed) = post_raw(
                create_router(state.clone()),
                &token,
                serde_json::json!({ "matches": [entry] }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{id}");
            assert!(
                parsed["error"]
                    .as_str()
                    .unwrap_or("")
                    .contains("recognizer"),
                "{id}: {parsed}"
            );
        }
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert!(rows.is_empty(), "non-ascii ids must not be stored");
    }

    #[tokio::test]
    async fn upload_recognizer_rejects_33_bytes_and_accepts_32() {
        let (state, member_id, token) = daemon().await;
        let too_long = "a".repeat(33);
        assert_eq!(too_long.len(), 33);
        let mut entry = match_object("sess-len", 2);
        entry["recognizer"] = serde_json::json!(too_long);
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [entry] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            parsed["error"]
                .as_str()
                .unwrap_or("")
                .contains("recognizer"),
            "{parsed}"
        );
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert!(rows.is_empty(), "a 33-byte id must not be stored");

        let accepted = "a".repeat(32);
        assert_eq!(accepted.len(), 32);
        let mut entry = match_object("sess-len", 6);
        entry["recognizer"] = serde_json::json!(accepted);
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [entry] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 1);
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].recognizer, accepted);
        assert_eq!(rows[0].elims, 6);
    }

    #[tokio::test]
    async fn upload_bad_recognizer_with_deleted_sessions_returns_400_and_keeps_rows() {
        let (state, member_id, token) = daemon().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-tomb", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-edit", 5)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        // Tombstone an existing row and try to overwrite another. The bad id
        // must fail the batch before either write.
        let mut bad = match_object("sess-edit", 99);
        bad["recognizer"] = serde_json::json!("NOPE");
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({
                "matches": [bad],
                "deleted_sessions": ["sess-tomb"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert!(
            parsed["error"]
                .as_str()
                .unwrap_or("")
                .contains("recognizer"),
            "{parsed}"
        );

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "rejected batch must not insert a row");
        let tomb = rows
            .iter()
            .find(|r| r.session_id == "sess-tomb")
            .expect("tombstoned row still exists");
        assert_eq!(tomb.elims, 4);
        assert_eq!(tomb.recognizer, "ocr-v1");
        let edited = rows
            .iter()
            .find(|r| r.session_id == "sess-edit")
            .expect("match from the batch was not removed");
        assert_eq!(edited.elims, 5, "match from the batch was not updated");
        assert_eq!(edited.recognizer, "ocr-v1");
    }

    async fn member_reader() -> (crate::state::AppState, String, String, String) {
        let state = test_state().await;
        crate::test_support::seed_user(&state, "susplayer", "susplayer").await;
        let member = state
            .db
            .create_member("susplayer", "susplayer", OrgRole::Member)
            .await
            .unwrap();
        let daemon_token = "sus-daemon-token".to_string();
        state
            .db
            .create_daemon_token(&member.id, &daemon_token, "tracker")
            .await
            .unwrap();
        let session_token = "sus-session-token".to_string();
        state
            .db
            .create_session("susplayer", &session_token, 24)
            .await
            .unwrap();
        (state, member.id, daemon_token, session_token)
    }

    async fn get_my_matches(app: axum::Router, token: &str) -> serde_json::Value {
        let req = Request::builder()
            .method("GET")
            .uri("/api/stats/me/matches?limit=20")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .body(Body::empty())
            .unwrap();
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        serde_json::from_slice(&bytes).unwrap()
    }

    fn row_by_session<'a>(page: &'a serde_json::Value, session_id: &str) -> &'a serde_json::Value {
        page["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["session_id"] == session_id)
            .unwrap_or_else(|| panic!("missing session {session_id} in {page}"))
    }

    #[tokio::test]
    async fn upload_suspect_fields_valid_lists_are_returned_on_me_matches() {
        let (state, member_id, daemon_token, session_token) = member_reader().await;
        let all = [
            "map", "mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit",
        ];
        let mut full = match_object("sess-all", 4);
        full["suspect_fields"] = serde_json::json!(all);
        full["recognizer"] = serde_json::json!("cv-v1");
        let mut ordered = match_object("sess-order", 5);
        ordered["suspect_fields"] = serde_json::json!(["mit", "map"]);
        let mut empty = match_object("sess-empty-list", 6);
        empty["suspect_fields"] = serde_json::json!([]);

        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({ "matches": [full, ordered, empty] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 3);
        assert_eq!(parsed["deleted"], 0);

        let page = get_my_matches(create_router(state.clone()), &session_token).await;
        let full_row = row_by_session(&page, "sess-all");
        assert_eq!(full_row["recognizer"], "cv-v1");
        assert_eq!(full_row["suspect_fields"], serde_json::json!(all));
        let ordered_row = row_by_session(&page, "sess-order");
        assert_eq!(ordered_row["recognizer"], "ocr-v1");
        assert_eq!(
            ordered_row["suspect_fields"],
            serde_json::json!(["mit", "map"])
        );
        let empty_row = row_by_session(&page, "sess-empty-list");
        assert_eq!(empty_row["suspect_fields"], serde_json::json!([]));

        let stats = state.db.get_personal_stats(&member_id).await.unwrap();
        assert_eq!(stats.wins, 3);
    }

    #[tokio::test]
    async fn upload_missing_and_null_suspect_fields_store_empty() {
        let (state, member_id, token) = daemon().await;
        let missing = match_object("sess-miss", 2);
        let mut null_fields = match_object("sess-null-fields", 3);
        null_fields["suspect_fields"] = serde_json::Value::Null;
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [missing, null_fields] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 2);
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        for row in &rows {
            assert!(
                row.suspect_fields.is_empty(),
                "{} stored {:?}",
                row.session_id,
                row.suspect_fields
            );
            assert_eq!(row.recognizer, "ocr-v1");
        }
    }

    #[tokio::test]
    async fn reupload_without_suspect_fields_resets_to_empty_like_recognizer() {
        let (state, member_id, token) = daemon().await;
        let mut first = match_object("sess-reset", 4);
        first["recognizer"] = serde_json::json!("cv-v1");
        first["suspect_fields"] = serde_json::json!(["hero", "dmg"]);
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [first] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 1);

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].recognizer, "cv-v1");
        assert_eq!(
            rows[0].suspect_fields,
            vec!["hero".to_string(), "dmg".to_string()]
        );

        // Same session, neither optional field present. Both fall back.
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-reset", 9)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 1);

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "re-upload updates the same session");
        assert_eq!(rows[0].elims, 9);
        assert_eq!(rows[0].recognizer, "ocr-v1");
        assert!(
            rows[0].suspect_fields.is_empty(),
            "omitting suspect_fields clears the stored list, got {:?}",
            rows[0].suspect_fields
        );
    }

    async fn assert_suspect_fields_rejected(
        state: &crate::state::AppState,
        token: &str,
        matches: serde_json::Value,
        index: usize,
    ) {
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            token,
            serde_json::json!({ "matches": matches }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{parsed}");
        assert_eq!(
            parsed["error"],
            format!(
                "matches[{index}]: {}",
                scuffed_types::api::SUSPECT_FIELDS_ERROR
            ),
            "{parsed}"
        );
    }

    #[tokio::test]
    async fn upload_invalid_suspect_fields_is_400_and_stores_nothing() {
        let (state, member_id, token) = daemon().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-keep", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let cases: Vec<serde_json::Value> = vec![
            serde_json::json!(["nope"]),
            serde_json::json!(["r3.dmg"]),
            serde_json::json!(["r01.e"]),
            serde_json::json!(["r12.mit"]),
            serde_json::json!(["e", "e"]),
            // The allowlist has 10 names, so an 11-entry list can only reach
            // the length cap by repeating one of them.
            serde_json::json!([
                "map", "mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit", "map"
            ]),
            serde_json::json!(["Map"]),
            serde_json::json!([" map"]),
            serde_json::json!([""]),
            serde_json::json!(["map", 1]),
            serde_json::json!([null]),
            serde_json::json!("map"),
            serde_json::json!(5),
            serde_json::json!({ "hero": true }),
        ];
        for fields in cases {
            let mut bad = match_object("sess-keep", 99);
            bad["suspect_fields"] = fields.clone();
            let mut sibling = match_object("sess-new", 1);
            sibling["suspect_fields"] = serde_json::json!(["hero"]);
            assert_suspect_fields_rejected(&state, &token, serde_json::json!([sibling, bad]), 1)
                .await;

            let rows = state
                .db
                .list_personal_matches(&member_id, 10, 0)
                .await
                .unwrap();
            assert_eq!(
                rows.len(),
                1,
                "rejected batch must not insert or update: {fields}"
            );
            assert_eq!(rows[0].session_id, "sess-keep");
            assert_eq!(rows[0].elims, 4);
            assert!(rows[0].suspect_fields.is_empty());
            assert_eq!(rows[0].recognizer, "ocr-v1");
        }
    }

    #[tokio::test]
    async fn upload_bad_suspect_fields_with_deleted_sessions_returns_400_and_keeps_rows() {
        let (state, member_id, token) = daemon().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-tomb", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-edit", 5)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let mut bad = match_object("sess-edit", 99);
        bad["suspect_fields"] = serde_json::json!(["r3.dmg"]);
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({
                "matches": [bad],
                "deleted_sessions": ["sess-tomb"]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            parsed["error"],
            format!("matches[0]: {}", scuffed_types::api::SUSPECT_FIELDS_ERROR),
            "{parsed}"
        );

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2, "rejected batch must not delete or update");
        let tomb = rows
            .iter()
            .find(|r| r.session_id == "sess-tomb")
            .expect("tombstoned row still exists");
        assert_eq!(tomb.elims, 4);
        assert!(tomb.suspect_fields.is_empty());
        let edited = rows
            .iter()
            .find(|r| r.session_id == "sess-edit")
            .expect("match from the batch was not removed");
        assert_eq!(edited.elims, 5, "match from the batch was not updated");
        assert!(edited.suspect_fields.is_empty());
    }

    #[tokio::test]
    async fn old_personal_match_reads_recognizer_ocr_v1_and_empty_suspect_fields() {
        let (state, member_id, _daemon_token, session_token) = member_reader().await;
        state
            .db
            .client
            .query(
                r#"REMOVE FIELD IF EXISTS recognizer ON personal_match;
                   REMOVE FIELD IF EXISTS suspect_fields ON personal_match;
                   CREATE personal_match SET
                       member_id = $mid,
                       session_id = 'legacy-row',
                       hero = 'Ana',
                       map_name = 'Oasis',
                       game_mode = 'control',
                       role = 'Support',
                       outcome = 'victory',
                       elims = 7,
                       deaths = 1,
                       assists = 1,
                       damage = 1,
                       healing = 1,
                       mitigation = 0,
                       edited = false,
                       played_at = d'2026-07-01T20:00:00Z',
                       uploaded_at = d'2026-07-01T21:00:00Z';
                   DEFINE FIELD OVERWRITE recognizer ON personal_match TYPE string DEFAULT 'ocr-v1';
                   DEFINE FIELD OVERWRITE suspect_fields ON personal_match TYPE array<string> DEFAULT [];"#,
            )
            .bind(("mid", member_id))
            .await
            .unwrap()
            .check()
            .unwrap();

        let page = get_my_matches(create_router(state), &session_token).await;
        let row = row_by_session(&page, "legacy-row");
        assert_eq!(row["recognizer"], "ocr-v1");
        assert_eq!(row["suspect_fields"], serde_json::json!([]));
        assert_eq!(row["elims"], 7);
        assert_rank_keys_absent(row);
    }

    const RANK_JSON_KEYS: &[&str] = &[
        "rank_role",
        "rank_tier",
        "rank_division",
        "rank_progress_pct",
        "rank_delta_pct",
        "rank_modifiers",
        "hero_sr",
        "hero_sr_delta",
        "rank_recognizer",
        "rank_confidence",
        "rank_dropped",
    ];

    fn json_has_key(value: &serde_json::Value, key: &str) -> bool {
        match value {
            serde_json::Value::Object(map) => {
                map.contains_key(key) || map.values().any(|child| json_has_key(child, key))
            }
            serde_json::Value::Array(items) => items.iter().any(|child| json_has_key(child, key)),
            _ => false,
        }
    }

    fn assert_rank_keys_absent(value: &serde_json::Value) {
        for key in RANK_JSON_KEYS {
            assert!(!json_has_key(value, key), "{key} leaked into {value}");
        }
    }

    fn rank_payload() -> serde_json::Value {
        serde_json::json!({
            "rank_role": "support",
            "rank_tier": "diamond",
            "rank_division": 2,
            "rank_progress_pct": 40,
            "rank_delta_pct": -15,
            "rank_modifiers": ["Victory", "win streak", "future_mod"],
            "hero_sr": 3200,
            "hero_sr_delta": -25,
            "rank_recognizer": "cv-v1",
            "rank_confidence": {
                "rank_role": 0.9,
                "rank_tier": 0.5,
                "rank_division": 0.0,
                "hero_sr": 1.0
            }
        })
    }

    fn with_rank(session_id: &str, elims: u32) -> serde_json::Value {
        let mut entry = match_object(session_id, elims);
        let extra = rank_payload();
        let obj = entry.as_object_mut().unwrap();
        for (key, value) in extra.as_object().unwrap() {
            obj.insert(key.clone(), value.clone());
        }
        entry
    }

    async fn read_json(app: axum::Router, req: Request<Body>) -> (StatusCode, serde_json::Value) {
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, parsed)
    }

    fn public_get(uri: &str) -> Request<Body> {
        Request::builder()
            .method("GET")
            .uri(uri)
            .header("x-forwarded-for", "127.0.0.1")
            .extension(axum::extract::ConnectInfo(std::net::SocketAddr::from((
                [127, 0, 0, 1],
                9,
            ))))
            .body(Body::empty())
            .unwrap()
    }

    #[tokio::test]
    async fn upload_rank_screen_round_trips_for_the_member_only() {
        let (state, member_id, daemon_token, session_token) = member_reader().await;
        let mut bounds = match_object("sess-bounds", 3);
        bounds["rank_role"] = serde_json::json!("tank");
        bounds["rank_tier"] = serde_json::json!("champion");
        bounds["rank_division"] = serde_json::json!(5);
        bounds["rank_progress_pct"] = serde_json::json!(100);
        bounds["rank_delta_pct"] = serde_json::json!(-100);
        bounds["hero_sr"] = serde_json::json!(6000);
        bounds["hero_sr_delta"] = serde_json::json!(1000);
        bounds["rank_recognizer"] = serde_json::json!("a".repeat(32));
        bounds["rank_modifiers"] = serde_json::json!([
            "variance",
            "victory",
            "win_streak",
            "loss_streak",
            "uphill_battle",
            "reversal",
            "consolation",
            "calibration"
        ]);
        bounds["rank_confidence"] = serde_json::json!({
            "rank_modifiers": 1,
            "hero_sr_delta": 0
        });

        let mut partial = match_object("sess-partial", 2);
        partial["rank_tier"] = serde_json::json!("gold");
        partial["rank_division"] = serde_json::json!(1);
        partial["rank_progress_pct"] = serde_json::json!(0);
        partial["hero_sr"] = serde_json::json!(0);
        partial["hero_sr_delta"] = serde_json::json!(-1000);

        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({
                "matches": [with_rank("sess-rank", 4), bounds, partial, match_object("sess-old", 1)]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 4);

        let page = get_my_matches(create_router(state.clone()), &session_token).await;
        let row = row_by_session(&page, "sess-rank");
        assert_eq!(row["role"], "Support");
        assert_eq!(row["rank_role"], "support");
        assert_eq!(row["rank_tier"], "diamond");
        assert_eq!(row["rank_division"], 2);
        assert_eq!(row["rank_progress_pct"], 40);
        assert_eq!(row["rank_delta_pct"], -15);
        assert_eq!(
            row["rank_modifiers"],
            serde_json::json!(["victory", "win_streak", "future_mod"])
        );
        assert_eq!(row["hero_sr"], 3200);
        assert_eq!(row["hero_sr_delta"], -25);
        assert_eq!(row["rank_recognizer"], "cv-v1");
        assert_eq!(row["rank_confidence"]["rank_role"], 0.9);
        assert_eq!(row["rank_confidence"]["rank_tier"], 0.5);
        assert_eq!(row["rank_confidence"]["rank_division"], 0.0);
        assert_eq!(row["rank_confidence"]["hero_sr"], 1.0);
        assert!(row.get("rank_dropped").is_none(), "{row}");

        let bounds_row = row_by_session(&page, "sess-bounds");
        assert_eq!(bounds_row["rank_division"], 5);
        assert_eq!(bounds_row["rank_progress_pct"], 100);
        assert_eq!(bounds_row["rank_delta_pct"], -100);
        assert_eq!(bounds_row["hero_sr"], 6000);
        assert_eq!(bounds_row["hero_sr_delta"], 1000);
        assert_eq!(bounds_row["rank_modifiers"].as_array().unwrap().len(), 8);
        assert_eq!(bounds_row["rank_recognizer"].as_str().unwrap().len(), 32);

        let partial_row = row_by_session(&page, "sess-partial");
        assert_eq!(partial_row["rank_tier"], "gold");
        assert_eq!(partial_row["rank_division"], 1);
        assert!(partial_row.get("rank_role").is_none());
        assert!(partial_row.get("rank_modifiers").is_none());
        assert!(partial_row.get("rank_recognizer").is_none());
        assert!(partial_row.get("rank_confidence").is_none());

        assert_rank_keys_absent(row_by_session(&page, "sess-old"));

        let (status, anon) = read_json(
            create_router(state.clone()),
            Request::builder()
                .method("GET")
                .uri("/api/stats/me/matches?limit=20")
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED);
        assert_rank_keys_absent(&anon);

        let heroes_uri = format!("/api/public/members/{member_id}/heroes?top=0");
        let (status, heroes) =
            read_json(create_router(state.clone()), public_get(&heroes_uri)).await;
        assert_eq!(status, StatusCode::OK, "{heroes}");
        assert!(
            heroes
                .as_array()
                .unwrap()
                .iter()
                .any(|row| row["games"].as_u64().unwrap() >= 1),
            "public heroes should count the uploaded games: {heroes}"
        );
        assert_rank_keys_absent(&heroes);

        let (status, board) = read_json(
            create_router(state.clone()),
            public_get("/api/public/leaderboards?metric=games&limit=10"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{board}");
        assert_rank_keys_absent(&board);

        let (status, mine) = read_json(
            create_router(state.clone()),
            Request::builder()
                .method("GET")
                .uri("/api/stats/me")
                .header(header::AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_rank_keys_absent(&mine);
        assert!(mine["total_matches"].as_u64().unwrap() >= 1);
    }

    #[tokio::test]
    async fn upload_without_rank_fields_keeps_old_client_and_clears_on_replace() {
        let (state, member_id, daemon_token, session_token) = member_reader().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({ "matches": [with_rank("sess-clear", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let old = serde_json::json!({
            "matches": [{
                "session_id": "sess-clear",
                "hero": "Ana",
                "map_name": "Oasis",
                "game_mode": "control",
                "role": "Support",
                "outcome": "victory",
                "elims": 8,
                "played_at": "2026-07-01T20:00:00Z"
            }, {
                "hero": "Ana",
                "map_name": "Oasis",
                "game_mode": "control",
                "role": "Support",
                "outcome": "defeat",
                "played_at": "2026-07-02T20:00:00Z"
            }]
        });
        let (status, parsed) = post_raw(create_router(state.clone()), &daemon_token, old).await;
        assert_eq!(status, StatusCode::OK);
        assert_eq!(parsed["inserted"], 2);

        let page = get_my_matches(create_router(state.clone()), &session_token).await;
        let cleared = row_by_session(&page, "sess-clear");
        assert_eq!(cleared["elims"], 8);
        assert_rank_keys_absent(cleared);
        let bare = page["data"]
            .as_array()
            .unwrap()
            .iter()
            .find(|row| row["outcome"] == "defeat")
            .expect("old client row");
        assert_rank_keys_absent(bare);
        assert_eq!(bare["role"], "Support");

        let stored = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert!(
            stored
                .iter()
                .all(|row| row.rank == scuffed_types::RankScreenFields::default())
        );
    }

    #[derive(Clone, Default)]
    struct RankLogBuf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);

    impl std::io::Write for RankLogBuf {
        fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
            self.0.lock().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        }
        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for RankLogBuf {
        type Writer = RankLogBuf;
        fn make_writer(&'a self) -> RankLogBuf {
            self.clone()
        }
    }

    fn assert_scoreboard(row: &serde_json::Value, elims: u64) {
        assert_eq!(row["hero"], "Ana");
        assert_eq!(row["map_name"], "Oasis");
        assert_eq!(row["game_mode"], "control");
        assert_eq!(row["role"], "Support");
        assert_eq!(row["outcome"], "victory");
        assert_eq!(row["elims"], elims);
        assert_eq!(row["deaths"], 1);
        assert_eq!(row["assists"], 2);
        assert_eq!(row["damage"], 1000);
        assert_eq!(row["healing"], 4000);
        assert_eq!(row["mitigation"], 0);
    }

    #[tokio::test(flavor = "current_thread")]
    async fn upload_rank_misread_drops_field_and_keeps_scoreboard() {
        let (state, member_id, daemon_token, session_token) = member_reader().await;
        let buf = RankLogBuf::default();
        let subscriber = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        let _guard = tracing::subscriber::set_default(subscriber);

        let mut delta = with_rank("sess-delta", 11);
        delta["rank_delta_pct"] = serde_json::json!(150);
        delta["rank_dropped"] = serde_json::json!(["client_sent", "rank_role"]);
        let mut hero_sr = with_rank("sess-sr", 12);
        hero_sr["hero_sr"] = serde_json::json!(6001);
        let mut confidence = with_rank("sess-conf", 13);
        confidence["rank_confidence"] = serde_json::json!({
            "rank_role": 0.4,
            "rank_tier": 1.5,
            "ocr_junk_token": 0.2
        });

        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({
                "matches": [delta, hero_sr, confidence, with_rank("sess-ok", 14)]
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["inserted"], 4);

        let page = get_my_matches(create_router(state.clone()), &session_token).await;

        let delta_row = row_by_session(&page, "sess-delta");
        assert_scoreboard(delta_row, 11);
        assert!(delta_row.get("rank_delta_pct").is_none(), "{delta_row}");
        assert_eq!(delta_row["rank_role"], "support");
        assert_eq!(delta_row["rank_tier"], "diamond");
        assert_eq!(delta_row["rank_division"], 2);
        assert_eq!(delta_row["rank_progress_pct"], 40);
        assert_eq!(delta_row["hero_sr"], 3200);
        assert_eq!(delta_row["hero_sr_delta"], -25);
        assert_eq!(delta_row["rank_recognizer"], "cv-v1");
        assert_eq!(
            delta_row["rank_modifiers"],
            serde_json::json!(["victory", "win_streak", "future_mod"])
        );
        assert_eq!(
            delta_row["rank_dropped"],
            serde_json::json!(["rank_delta_pct"])
        );

        let sr_row = row_by_session(&page, "sess-sr");
        assert_scoreboard(sr_row, 12);
        assert!(sr_row.get("hero_sr").is_none(), "{sr_row}");
        assert_eq!(sr_row["hero_sr_delta"], -25);
        assert_eq!(sr_row["rank_tier"], "diamond");
        assert_eq!(sr_row["rank_delta_pct"], -15);
        assert_eq!(sr_row["rank_dropped"], serde_json::json!(["hero_sr"]));

        let conf_row = row_by_session(&page, "sess-conf");
        assert_scoreboard(conf_row, 13);
        assert_eq!(conf_row["rank_confidence"]["rank_role"], 0.4);
        assert!(conf_row["rank_confidence"].get("rank_tier").is_none());
        assert!(conf_row["rank_confidence"].get("ocr_junk_token").is_none());
        assert_eq!(conf_row["rank_tier"], "diamond");
        assert_eq!(conf_row["hero_sr"], 3200);
        assert_eq!(
            conf_row["rank_dropped"],
            serde_json::json!(["rank_confidence", "rank_confidence.rank_tier"])
        );

        let ok_row = row_by_session(&page, "sess-ok");
        assert_scoreboard(ok_row, 14);
        assert_eq!(ok_row["rank_delta_pct"], -15);
        assert_eq!(ok_row["hero_sr"], 3200);
        assert!(ok_row.get("rank_dropped").is_none(), "{ok_row}");

        let heroes_uri = format!("/api/public/members/{member_id}/heroes?top=0");
        let (status, heroes) =
            read_json(create_router(state.clone()), public_get(&heroes_uri)).await;
        assert_eq!(status, StatusCode::OK, "{heroes}");
        assert_rank_keys_absent(&heroes);
        let (status, board) = read_json(
            create_router(state.clone()),
            public_get("/api/public/leaderboards?metric=games&limit=10"),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{board}");
        assert_rank_keys_absent(&board);
        let (status, mine) = read_json(
            create_router(state),
            Request::builder()
                .method("GET")
                .uri("/api/stats/me")
                .header(header::AUTHORIZATION, format!("Bearer {session_token}"))
                .body(Body::empty())
                .unwrap(),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{mine}");
        assert_rank_keys_absent(&mine);

        let log = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
        let drops: Vec<&str> = log
            .lines()
            .filter(|line| line.contains("dropped rank-screen field"))
            .collect();
        let drops_text = drops.join("\n");
        for (session, field, reason) in [
            ("sess-delta", "rank_delta_pct", "out of range"),
            ("sess-sr", "hero_sr", "out of range"),
            ("sess-conf", "rank_confidence", "confidence entry dropped"),
        ] {
            assert!(
                drops.iter().any(|line| {
                    line.contains(session) && line.contains(field) && line.contains(reason)
                }),
                "missing drop log for {session} {field} {reason} in {drops_text}"
            );
        }
        assert_eq!(drops_text.matches("confidence entry dropped").count(), 2);
        for secret in [
            "150",
            "6001",
            "1.5",
            "ocr_junk_token",
            "susplayer",
            "client_sent",
        ] {
            assert!(
                !drops_text.contains(secret),
                "rank drop log leaked {secret}: {drops_text}"
            );
        }
    }

    #[tokio::test]
    async fn upload_top500_saves_and_drops_division() {
        let (state, _member_id, daemon_token, session_token) = member_reader().await;
        let mut with_division = with_rank("sess-top", 7);
        with_division["rank_tier"] = serde_json::json!("top500");
        with_division["rank_division"] = serde_json::json!(4);
        let mut bare = match_object("sess-top-bare", 8);
        bare["rank_tier"] = serde_json::json!("top500");

        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({ "matches": [with_division, bare] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["inserted"], 2);

        let page = get_my_matches(create_router(state), &session_token).await;
        let row = row_by_session(&page, "sess-top");
        assert_scoreboard(row, 7);
        assert_eq!(row["rank_tier"], "top500");
        assert!(row.get("rank_division").is_none(), "{row}");
        assert_eq!(row["rank_role"], "support");
        assert_eq!(row["hero_sr"], 3200);
        assert_eq!(row["rank_delta_pct"], -15);
        assert_eq!(row["rank_dropped"], serde_json::json!(["rank_division"]));

        let bare_row = row_by_session(&page, "sess-top-bare");
        assert_scoreboard(bare_row, 8);
        assert_eq!(bare_row["rank_tier"], "top500");
        assert!(bare_row.get("rank_division").is_none(), "{bare_row}");
        assert!(bare_row.get("rank_dropped").is_none(), "{bare_row}");
    }

    #[tokio::test]
    async fn upload_rank_screen_wrong_type_is_400_and_stores_nothing() {
        let (state, member_id, token) = daemon().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-keep", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let cases = [
            ("rank_role", serde_json::json!(1), "rank_role"),
            ("rank_division", serde_json::json!("3"), "rank_division"),
            ("rank_division", serde_json::json!(1.5), "rank_division"),
            ("rank_delta_pct", serde_json::json!(true), "rank_delta_pct"),
            ("rank_delta_pct", serde_json::json!("150"), "rank_delta_pct"),
            ("hero_sr", serde_json::json!("6000"), "hero_sr"),
            ("rank_recognizer", serde_json::json!(1), "rank_recognizer"),
            (
                "rank_modifiers",
                serde_json::json!("victory"),
                "rank_modifiers",
            ),
            (
                "rank_confidence",
                serde_json::json!(["rank_tier"]),
                "rank_confidence",
            ),
        ];

        for (key, bad, needle) in cases {
            let mut bad_row = with_rank("sess-keep", 99);
            bad_row[key] = bad.clone();
            let sibling = with_rank("sess-new", 1);
            let (status, parsed) = post_raw(
                create_router(state.clone()),
                &token,
                serde_json::json!({
                    "matches": [sibling, bad_row],
                    "deleted_sessions": ["sess-keep"]
                }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{key}={bad} -> {parsed}");
            let error = parsed["error"].as_str().unwrap_or("");
            assert!(
                error.starts_with("matches[1]: ") && error.contains(needle),
                "{key}={bad} -> {error}"
            );

            let rows = state
                .db
                .list_personal_matches(&member_id, 10, 0)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1, "{key}={bad} must not insert");
            assert_eq!(rows[0].session_id, "sess-keep");
            assert_eq!(rows[0].elims, 4, "{key}={bad} must not update");
            assert_eq!(rows[0].rank, scuffed_types::RankScreenFields::default());
        }

        let mut missing_hero = match_object("sess-new", 5);
        missing_hero.as_object_mut().unwrap().remove("hero");
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({
                "matches": [missing_hero],
                "deleted_sessions": ["sess-keep"]
            }),
        )
        .await;
        assert!(
            status.is_client_error(),
            "missing hero must not be stored, got {status}"
        );
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].session_id, "sess-keep");
        assert_eq!(rows[0].elims, 4);
    }
}
