use axum::{
    Json,
    extract::{Path, Query, State},
    http::StatusCode,
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

use crate::extractors::{DaemonUser, OpaqueDaemonUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::routes::leaderboards::resolve_season_window;
use crate::state::AppState;

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

    // Reject the whole batch before any write. A bad recognizer id or a bad
    // suspect_fields list must not be stored, and a later entry must not leave
    // the earlier ones committed. deleted_sessions is not applied either.
    let mut recognizers = Vec::with_capacity(body.matches.len());
    let mut suspect_lists = Vec::with_capacity(body.matches.len());
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
    }

    let total = body.matches.len() as u32;

    // Entries whose outcome the schema would reject ('unknown', garbage) are
    // skipped, not 500'd: a single bad entry used to fail the whole batch,
    // and the client retried it forever — wedging sync for everything behind
    // it. Well-behaved clients hold 'unknown' back until the outcome is known.
    let stub_matches: Vec<PersonalMatch> = body
        .matches
        .into_iter()
        .zip(recognizers.into_iter().zip(suspect_lists))
        .filter(|(e, _)| matches!(e.entry.outcome.as_str(), "victory" | "defeat" | "draw"))
        .map(|(e, (recognizer, suspect_fields))| {
            let e = e.entry;
            PersonalMatch {
                id: String::new(),
                member_id: daemon.member.id.clone(),
                session_id: e.session_id,
                hero: e.hero,
                map_name: e.map_name,
                game_mode: e.game_mode,
                role: e.role,
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
/// path does this). This handler does not write.
pub async fn token_check(daemon: OpaqueDaemonUser) -> Json<TokenCheckResponse> {
    Json(TokenCheckResponse {
        display_name: daemon.member.display_name,
    })
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
    }
}
