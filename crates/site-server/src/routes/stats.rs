use axum::{
    Json,
    extract::{Path, Query, State},
    http::{StatusCode, header},
    response::{IntoResponse, Response},
};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{
    AuditAction, AuditTargetType, DaemonToken, HeroStats, MapStats, OrgRole, PersonalMatch,
    PersonalStats, RoleStats,
};
use scuffed_types::api::{
    CreateDaemonTokenRequest, CreateDaemonTokenResponse, CursorResponse, DaemonConfigResponse,
    MemberSettingsResponse, NAME_RECOGNIZER_ID_ERROR, PaginationParams, RECOGNIZER_ID_ERROR,
    SUSPECT_FIELDS_ERROR, SeasonQuery, StatsUploadBody, StatsUploadResponse, StatsVisibility,
    TokenCheckResponse, UPLOAD_HERO_UNKNOWN, is_upload_hero_unknown, resolve_name_recognizer,
    resolve_recognizer, resolve_suspect_fields, validate_deleted_session, validate_upload_entry,
};
use scuffed_types::{
    STATS_UPLOAD_MAX_DELETED_SESSIONS, STATS_UPLOAD_MAX_MATCHES, hero_key_to_name,
    role_for_hero_name,
};

use crate::extractors::{DaemonUser, LimitedUploadDaemon, OpaqueDaemonUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::routes::leaderboards::resolve_season_window;
use crate::state::AppState;

/// Store a known hero under its display name, with the role from the hero
/// table.
///
/// 0.5.0-alpha.1 and alpha.2 trackers on the new reader send the pack key
/// (`wrecking-ball`, `ana`) as the hero and guess the role from it, which
/// gives Damage for hyphenated tanks and supports. Leaderboards and hero
/// stats group on the exact string, so `ana` and `Ana` split. `Unknown` in
/// any case is stored as `Unknown`, matching the rows already in live data.
fn normalize_hero(hero: String, role: String) -> (String, String) {
    if is_upload_hero_unknown(&hero) {
        return (UPLOAD_HERO_UNKNOWN.to_string(), role);
    }
    let Some(display) = hero_key_to_name(&hero) else {
        return (hero, role);
    };
    let role = role_for_hero_name(display).map_or(role, |r| r.to_string());
    (display.to_string(), role)
}

/// Rejection from [`upload_stats`]. Existing `{error}` bodies and the batch
/// cap's `{code}` body both become responses.
pub(crate) struct UploadReject(Box<Response>);

impl From<(StatusCode, Json<ErrorResponse>)> for UploadReject {
    fn from(err: (StatusCode, Json<ErrorResponse>)) -> Self {
        Self(Box::new(err.into_response()))
    }
}

impl IntoResponse for UploadReject {
    fn into_response(self) -> Response {
        *self.0
    }
}

fn bad_request(error: String) -> UploadReject {
    (StatusCode::BAD_REQUEST, Json(ErrorResponse { error })).into()
}

fn batch_over_cap(body: &StatsUploadBody) -> bool {
    body.matches.len() > STATS_UPLOAD_MAX_MATCHES
        || body.deleted_sessions.len() > STATS_UPLOAD_MAX_DELETED_SESSIONS
}

fn batch_too_large() -> UploadReject {
    let body = format!(
        r#"{{"code":"batch_too_large","max_matches":{STATS_UPLOAD_MAX_MATCHES},"max_deleted_sessions":{STATS_UPLOAD_MAX_DELETED_SESSIONS}}}"#
    );
    UploadReject(Box::new(
        Response::builder()
            .status(StatusCode::PAYLOAD_TOO_LARGE)
            .header(header::CONTENT_TYPE, "application/json")
            .header(header::CACHE_CONTROL, "no-store")
            .body(axum::body::Body::from(body))
            .expect("batch_too_large response"),
    ))
}

/// POST /api/stats/upload — bulk upload personal matches (daemon token auth)
pub(crate) async fn upload_stats(
    State(state): State<AppState>,
    daemon: LimitedUploadDaemon,
    Json(body): Json<StatsUploadBody>,
) -> Result<Json<StatsUploadResponse>, UploadReject> {
    if crate::stats_upload_limits::stats_upload_batch_cap_enabled() && batch_over_cap(&body) {
        return Err(batch_too_large());
    }

    if body.matches.is_empty() && body.deleted_sessions.is_empty() {
        return Ok(Json(StatsUploadResponse {
            inserted: 0,
            skipped: 0,
            deleted: 0,
        }));
    }

    // Reject the whole batch before any write. A bad recognizer id, a bad
    // name_recognizer id, or a bad suspect_fields list must not be stored,
    // and a later entry must not leave the earlier ones committed.
    // deleted_sessions is not applied either.
    // `recognizer` is checked first. A match that fails both reports that error.
    //
    // After those, each match must pass `validate_upload_entry` (API-3,
    // DB19): 64-character strings, a known hero or `Unknown`, a played_at
    // window, and
    // numeric ceilings. The message keeps the `matches[n]: ` prefix so the
    // tracker can isolate the one bad row. A long deleted_sessions entry is
    // a 400 for the whole request.
    for (i, session_id) in body.deleted_sessions.iter().enumerate() {
        if let Err(message) = validate_deleted_session(session_id) {
            return Err(bad_request(format!("deleted_sessions[{i}]: {message}")));
        }
    }
    let now = chrono::Utc::now();
    let mut recognizers = Vec::with_capacity(body.matches.len());
    let mut name_recognizers = Vec::with_capacity(body.matches.len());
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
                )
                    .into());
            }
        }
        match resolve_name_recognizer(&entry.name_recognizer) {
            Ok(id) => name_recognizers.push(id),
            Err(_) => {
                return Err((
                    StatusCode::BAD_REQUEST,
                    Json(ErrorResponse {
                        error: format!("matches[{i}]: {NAME_RECOGNIZER_ID_ERROR}"),
                    }),
                )
                    .into());
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
                )
                    .into());
            }
        }
        if let Err(message) = validate_upload_entry(&entry.entry, now) {
            return Err(bad_request(format!("matches[{i}]: {message}")));
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
        .zip(name_recognizers)
        .zip(suspect_lists)
        .map(|(((entry, recognizer), name_recognizer), suspect_fields)| {
            (entry, recognizer, name_recognizer, suspect_fields)
        })
        .filter(|(e, _, _, _)| matches!(e.entry.outcome.as_str(), "victory" | "defeat" | "draw"))
        .map(|(e, recognizer, name_recognizer, suspect_fields)| {
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
                name_recognizer,
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
///
/// Each item's `role` is filled at read time with
/// [`scuffed_types::filled_role_label`]: a recognized stored role is kept, an
/// empty or unrecognized one takes the hero's current role, and a hero not on
/// the list keeps `""`. The stored row is not changed.
pub async fn my_matches(
    State(state): State<AppState>,
    member: OrgMember,
    Query(pagination): Query<PaginationParams>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<CursorResponse<PersonalMatch>>, (StatusCode, Json<ErrorResponse>)> {
    let (limit, offset) = pagination.resolve();
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    let mut items = state
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
    // Read-time role fill only: the stored row keeps its role.
    for m in &mut items {
        m.role = scuffed_types::filled_role_label(&m.role, &m.hero);
    }
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

/// GET /api/stats/me/matches/{id}: one of the caller's own games (session auth).
///
/// `id` is the `id` of a row on [`my_matches`]. The body is that same row
/// (same fields, including `edited`, `recognizer` and `suspect_fields`), with
/// `role` filled at read time the same way. A key that does not exist, a
/// malformed key and another member's game are all the same 404, so the
/// route does not reveal whether someone else's game exists. Auth matches
/// [`my_matches`]: 401 without a session, 403 for inactive members.
pub async fn my_match(
    State(state): State<AppState>,
    member: OrgMember,
    Path(id): Path<String>,
) -> Result<Json<PersonalMatch>, (StatusCode, Json<ErrorResponse>)> {
    let found = state
        .db
        .get_personal_match_for_member(&member.member.id, &id)
        .await
        .map_err(|_e| {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ErrorResponse {
                    error: "Internal error".into(),
                }),
            )
        })?;
    let mut m = found.ok_or_else(|| {
        (
            StatusCode::NOT_FOUND,
            Json(ErrorResponse {
                error: "Match not found".into(),
            }),
        )
    })?;
    // Read-time role fill only, as on the list: the stored row keeps its role.
    m.role = scuffed_types::filled_role_label(&m.role, &m.hero);
    Ok(Json(m))
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

/// GET /api/stats/me/roles: per-role stats from the stored match role, filled at read time (session auth).
///
/// Query: `?season=<id>`, the same window as [`my_hero_stats`] (omitted or blank
/// is all time; an unknown id is 404). No other filters.
///
/// Response: a JSON array of [`RoleStats`] (`role`, `matches`, `wins`, `losses`,
/// `draws`, `avg_elims`, `avg_deaths`, `avg_damage`, `avg_healing`). Grouped by
/// the `personal_match.role` column, filled at read time: a recognized stored
/// role is kept, an empty or unrecognized one takes the hero's current role
/// (`scuffed_types::filled_role_label`). Only games with a hero not on the list
/// stay in the `""` row. Stored roles are never rewritten. Ordered by `matches`
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
        .get_filled_role_stats_in(&member.member.id, season)
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

fn stats_forbidden(error: &str) -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::FORBIDDEN,
        Json(ErrorResponse {
            error: error.to_string(),
        }),
    )
}

fn internal_error() -> (StatusCode, Json<ErrorResponse>) {
    (
        StatusCode::INTERNAL_SERVER_ERROR,
        Json(ErrorResponse {
            error: "Internal error".into(),
        }),
    )
}

async fn ensure_member_stats_visible(
    state: &AppState,
    viewer: &OrgMember,
    target_id: &str,
) -> Result<(), (StatusCode, Json<ErrorResponse>)> {
    let visibility = state
        .db
        .get_stats_visibility(target_id)
        .await
        .map_err(|_e| internal_error())?;
    if let Some(error) = visibility.denial(
        true,
        viewer.member.id == target_id,
        viewer.member.org_role.is_at_least(OrgRole::Officer),
    ) {
        return Err(stats_forbidden(error));
    }
    Ok(())
}

/// GET /api/stats/member/:id — view another member's stats (session auth).
///
/// `public` and `members` are visible to any active member. `private` is the
/// owner or an officer. Anyone else gets 403 `stats_private`.
pub async fn member_stats(
    State(state): State<AppState>,
    member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<PersonalStats>, (StatusCode, Json<ErrorResponse>)> {
    ensure_member_stats_visible(&state, &member, &member_id).await?;
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
    member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<HeroStats>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_member_stats_visible(&state, &member, &member_id).await?;
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
/// Visibility matches [`member_stats`]: `public` and `members` for any active
/// member, `private` for the owner or an officer. A missing id with public
/// visibility returns `[]`. Anonymous requests are 401; inactive or suspended
/// callers are 403 `Membership inactive` before this check.
pub async fn member_role_stats(
    State(state): State<AppState>,
    member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<RoleStats>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_member_stats_visible(&state, &member, &member_id).await?;
    let season = resolve_season_window(&state, sq.season.as_deref()).await?;
    state
        .db
        .get_filled_role_stats_in(&member_id, season)
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
    member: OrgMember,
    Path(member_id): Path<String>,
    Query(sq): Query<SeasonQuery>,
) -> Result<Json<Vec<MapStats>>, (StatusCode, Json<ErrorResponse>)> {
    ensure_member_stats_visible(&state, &member, &member_id).await?;
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
    let (player_name, stats_visibility) = match settings {
        Some(s) => (s.player_name, s.stats_visibility),
        None => (None, StatsVisibility::Public),
    };
    Ok(Json(MemberSettingsResponse {
        player_name,
        stats_visibility,
    }))
}

enum SettingsPutReject {
    Malformed,
    BadVisibility,
}

/// Same JSON shape as [`scuffed_types::UpdateMemberSettingsRequest`].
///
/// Parsed by hand so an unknown `stats_visibility` is 400. A missing
/// `player_name` is still `None`, which clears the stored name.
fn parse_settings_put(
    value: serde_json::Value,
) -> Result<(Option<String>, Option<StatsVisibility>), SettingsPutReject> {
    let Some(obj) = value.as_object() else {
        return Err(SettingsPutReject::Malformed);
    };
    let player_name = match obj.get("player_name") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => Some(s.clone()),
        Some(_) => return Err(SettingsPutReject::Malformed),
    };
    let stats_visibility = match obj.get("stats_visibility") {
        None | Some(serde_json::Value::Null) => None,
        Some(serde_json::Value::String(s)) => match s.as_str() {
            "public" => Some(StatsVisibility::Public),
            "members" => Some(StatsVisibility::Members),
            "private" => Some(StatsVisibility::Private),
            _ => return Err(SettingsPutReject::BadVisibility),
        },
        Some(_) => return Err(SettingsPutReject::BadVisibility),
    };
    Ok((player_name, stats_visibility))
}

/// PUT /api/stats/settings. Save the caller's daemon settings (session auth).
///
/// Omitting `player_name` still clears the stored name. Omitting
/// `stats_visibility` keeps the stored value. A visibility change drops the
/// leaderboard cache so the next board is scanned under the new rule.
pub async fn update_member_settings(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<serde_json::Value>,
) -> Result<Json<MemberSettingsResponse>, (StatusCode, Json<ErrorResponse>)> {
    let (player_name_raw, stats_visibility) =
        parse_settings_put(body).map_err(|reject| match reject {
            SettingsPutReject::Malformed => (
                StatusCode::UNPROCESSABLE_ENTITY,
                Json(ErrorResponse {
                    error: "invalid_settings".into(),
                }),
            ),
            SettingsPutReject::BadVisibility => (
                StatusCode::BAD_REQUEST,
                Json(ErrorResponse {
                    error: "invalid_stats_visibility".into(),
                }),
            ),
        })?;
    // Trim and treat empty string as None
    let player_name = player_name_raw
        .as_deref()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty());

    let before = state
        .db
        .get_stats_visibility(&member.member.id)
        .await
        .map_err(|_e| internal_error())?;
    let settings = state
        .db
        .upsert_member_settings(&member.member.id, player_name, stats_visibility)
        .await
        .map_err(|_e| internal_error())?;
    if settings.stats_visibility != before {
        state.leaderboard_cache.invalidate();
    }
    Ok(Json(MemberSettingsResponse {
        player_name: settings.player_name,
        stats_visibility: settings.stats_visibility,
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

/// GET /api/stats/daemon-config. Daemon-token auth, read only.
///
/// `stats_visibility` comes from the same `member_settings` row as
/// `player_name`. No row uses public, which is what GET /api/stats/settings
/// reports for a member with no row. This route does not write settings, and
/// the daemon token is not accepted on /api/stats/settings.
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
    // Same default as `get_member_settings`: a missing row is public.
    let (player_name, stats_visibility) = match settings {
        Some(s) => (s.player_name, s.stats_visibility),
        None => (None, StatsVisibility::Public),
    };
    Ok(Json(DaemonConfigResponse {
        player_name,
        stats_visibility: Some(stats_visibility),
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
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};

    use axum::body::Body;
    use axum::extract::ConnectInfo;
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
                name_recognizer: None,
            }],
            deleted_sessions: vec![],
        }
    }

    fn upload_request(token: &str, body: Vec<u8>, peer: Ipv4Addr) -> Request<Body> {
        Request::builder()
            .method("POST")
            .uri("/api/stats/upload")
            .header(header::AUTHORIZATION, format!("Bearer {token}"))
            .header(header::CONTENT_TYPE, "application/json")
            .extension(ConnectInfo(SocketAddr::new(IpAddr::V4(peer), 9)))
            .body(Body::from(body))
            .unwrap()
    }

    async fn post_upload(
        app: axum::Router,
        token: &str,
        body: &StatsUploadRequest,
    ) -> (StatusCode, StatsUploadResponse) {
        let req = upload_request(
            token,
            serde_json::to_vec(body).unwrap(),
            Ipv4Addr::LOCALHOST,
        );
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
        let req = upload_request(
            token,
            serde_json::to_vec(&body).unwrap(),
            Ipv4Addr::LOCALHOST,
        );
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

    /// Post `bad` after a valid sibling and assert a 400 naming `matches[1]`
    /// with `message`, and that neither row was stored.
    async fn assert_entry_rejected(bad: serde_json::Value, message: &str) {
        let (state, member_id, token) = daemon().await;
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-ok", 3), bad] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{parsed}");
        assert_eq!(parsed["error"], format!("matches[1]: {message}"));
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert!(rows.is_empty(), "rejected batch must store nothing");
    }

    const PLAYED_AT_ERROR: &str = "played_at must be on or after 2022-10-04T00:00:00Z and no more than 24 hours after server time";

    #[tokio::test]
    async fn upload_future_played_at_is_400_and_stores_nothing() {
        let mut bad = match_object("sess-future", 1);
        bad["played_at"] = serde_json::json!("2099-01-01T00:00:00Z");
        assert_entry_rejected(bad, PLAYED_AT_ERROR).await;

        let mut tomorrow_plus = match_object("sess-future2", 1);
        tomorrow_plus["played_at"] =
            serde_json::json!((Utc::now() + chrono::Duration::hours(25)).to_rfc3339());
        assert_entry_rejected(tomorrow_plus, PLAYED_AT_ERROR).await;
    }

    #[tokio::test]
    async fn upload_played_at_before_overwatch_2_is_400() {
        let mut bad = match_object("sess-old", 1);
        bad["played_at"] = serde_json::json!("2022-10-03T23:59:59Z");
        assert_entry_rejected(bad, PLAYED_AT_ERROR).await;
    }

    #[tokio::test]
    async fn upload_non_hero_is_400() {
        for hero in ["Totally Real Hero", "Unknown Hero", ""] {
            let mut bad = match_object("sess-hero", 1);
            bad["hero"] = serde_json::json!(hero);
            assert_entry_rejected(bad, "hero must be a known hero or Unknown").await;
        }
    }

    /// Regression: 0.4.x and 0.5.0 trackers send `Unknown` when no hero was
    /// read. Those rows are stored (as `Unknown`, whatever the case), not
    /// rejected.
    #[tokio::test]
    async fn upload_with_hero_unknown_is_stored() {
        let (state, member_id, token) = daemon().await;
        let mut upper = match_object("sess-unknown", 2);
        upper["hero"] = serde_json::json!("Unknown");
        let mut lower = match_object("sess-unknown-lower", 2);
        lower["hero"] = serde_json::json!("unknown");
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [upper, lower] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["inserted"], 2, "{parsed}");
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        assert!(rows.iter().all(|r| r.hero == "Unknown"), "{rows:?}");
    }

    #[tokio::test]
    async fn upload_with_hero_doctrine_is_stored() {
        let (state, member_id, token) = daemon().await;
        let mut row = match_object("sess-doctrine", 2);
        row["hero"] = serde_json::json!("doctrine");
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [row] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["inserted"], 1, "{parsed}");
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1);
        assert_eq!(rows[0].hero, "Doctrine");
    }

    #[tokio::test]
    async fn upload_huge_damage_and_healing_are_400() {
        for (field, max) in [
            ("damage", 200_000u32),
            ("healing", 200_000),
            ("mitigation", 200_000),
            ("elims", 999),
            ("assists", 999),
            ("deaths", 50),
        ] {
            let mut bad = match_object("sess-num", 1);
            bad[field] = serde_json::json!(u32::MAX);
            assert_entry_rejected(bad, &format!("{field} must be at most {max}")).await;
            let mut over = match_object("sess-num", 1);
            over[field] = serde_json::json!(max + 1);
            assert_entry_rejected(over, &format!("{field} must be at most {max}")).await;
        }
    }

    #[tokio::test]
    async fn upload_strings_over_64_characters_are_400() {
        let long = "x".repeat(65);
        for field in ["session_id", "map_name", "game_mode", "role", "outcome"] {
            let mut bad = match_object("sess-long", 1);
            bad[field] = serde_json::json!(long);
            assert_entry_rejected(bad, &format!("{field} must be at most 64 characters")).await;
        }
    }

    #[tokio::test]
    async fn upload_long_deleted_session_is_400_and_keeps_rows() {
        let (state, member_id, token) = daemon().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-keep", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({
                "matches": [match_object("sess-new", 1)],
                "deleted_sessions": ["sess-keep", "y".repeat(65)],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(
            parsed["error"],
            "deleted_sessions[1]: session id must be at most 64 characters"
        );
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 1, "nothing deleted or inserted");
        assert_eq!(rows[0].session_id, "sess-keep");
    }

    #[tokio::test]
    async fn upload_values_at_the_limits_are_stored() {
        let (state, member_id, token) = daemon().await;
        let mut edge = match_object(&"s".repeat(64), 999);
        edge["map_name"] = serde_json::json!("\u{e9}".repeat(64));
        edge["hero"] = serde_json::json!("wrecking-ball");
        edge["assists"] = serde_json::json!(999);
        edge["deaths"] = serde_json::json!(50);
        edge["damage"] = serde_json::json!(200_000);
        edge["healing"] = serde_json::json!(200_000);
        edge["mitigation"] = serde_json::json!(200_000);
        edge["played_at"] =
            serde_json::json!((Utc::now() + chrono::Duration::hours(1)).to_rfc3339());
        let mut floor = match_object("sess-floor", 1);
        floor["played_at"] = serde_json::json!("2022-10-04T00:00:00Z");
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({
                "matches": [edge, floor],
                "deleted_sessions": ["d".repeat(64)],
            }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["inserted"], 2);
        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(rows.len(), 2);
        let edge_row = rows
            .iter()
            .find(|r| r.session_id == "s".repeat(64))
            .unwrap();
        assert_eq!(edge_row.damage, 200_000);
        assert_eq!(edge_row.hero, "Wrecking Ball");
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
        assert_eq!(rows[0].name_recognizer, None);
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

    async fn json_call(
        app: axum::Router,
        method: &str,
        uri: &str,
        bearer: Option<&str>,
        body: Option<serde_json::Value>,
    ) -> (StatusCode, serde_json::Value) {
        let mut builder = Request::builder().method(method).uri(uri);
        if let Some(token) = bearer {
            builder = builder.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let req = if let Some(body) = body {
            builder
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(serde_json::to_vec(&body).unwrap()))
                .unwrap()
        } else {
            builder.body(Body::empty()).unwrap()
        };
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let parsed = serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null);
        (status, parsed)
    }

    /// Daemon-config returns the same visibility GET /api/stats/settings does.
    /// A missing settings row is public. The daemon token cannot write settings.
    #[tokio::test]
    async fn daemon_config_returns_stats_visibility() {
        let (state, _member_id, daemon_token, session_token) = member_reader().await;
        let app = create_router(state);

        let (status, cfg) = json_call(
            app.clone(),
            "GET",
            "/api/stats/daemon-config",
            Some(&daemon_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{cfg}");
        assert!(cfg["player_name"].is_null(), "{cfg}");
        assert_eq!(cfg["stats_visibility"], "public");

        let (status, settings) = json_call(
            app.clone(),
            "GET",
            "/api/stats/settings",
            Some(&session_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{settings}");
        assert_eq!(settings["stats_visibility"], cfg["stats_visibility"]);

        for vis in ["public", "members", "private"] {
            let (status, put) = json_call(
                app.clone(),
                "PUT",
                "/api/stats/settings",
                Some(&session_token),
                Some(serde_json::json!({
                    "player_name": "Ana",
                    "stats_visibility": vis
                })),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{put}");
            assert_eq!(put["stats_visibility"], vis);
            assert_eq!(put["player_name"], "Ana");

            let (status, cfg) = json_call(
                app.clone(),
                "GET",
                "/api/stats/daemon-config",
                Some(&daemon_token),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{cfg}");
            assert_eq!(cfg["stats_visibility"], vis, "{cfg}");
            assert_eq!(cfg["player_name"], "Ana");

            let (status, settings) = json_call(
                app.clone(),
                "GET",
                "/api/stats/settings",
                Some(&session_token),
                None,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{settings}");
            assert_eq!(settings["stats_visibility"], cfg["stats_visibility"]);
        }

        let (missing_status, missing_body) =
            json_call(app.clone(), "GET", "/api/stats/daemon-config", None, None).await;
        assert_eq!(missing_status, StatusCode::UNAUTHORIZED, "{missing_body}");
        assert_eq!(missing_body["error"], "Missing Bearer token");

        let (bad_status, bad_body) = json_call(
            app.clone(),
            "GET",
            "/api/stats/daemon-config",
            Some("not-a-daemon-token"),
            None,
        )
        .await;
        assert_eq!(bad_status, StatusCode::UNAUTHORIZED, "{bad_body}");
        assert_eq!(bad_body["error"], "Invalid or revoked daemon token");

        // The daemon token stays off the session settings routes.
        let (status, body) = json_call(
            app.clone(),
            "GET",
            "/api/stats/settings",
            Some(&daemon_token),
            None,
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
        let (status, body) = json_call(
            app.clone(),
            "PUT",
            "/api/stats/settings",
            Some(&daemon_token),
            Some(serde_json::json!({
                "player_name": "Ana",
                "stats_visibility": "public"
            })),
        )
        .await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{body}");
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

    /// `/me/matches` fills an empty or unrecognized role from the hero at read
    /// time, keeps a recognized one, and never changes the stored row.
    #[tokio::test]
    async fn me_matches_fill_role_at_read_time_only() {
        let (state, member_id, _daemon_token, session_token) = member_reader().await;
        let game = |sid: &str, hero: &str, role: &str| scuffed_db::PersonalMatch {
            id: String::new(),
            member_id: member_id.clone(),
            session_id: sid.into(),
            hero: hero.into(),
            map_name: "Oasis".into(),
            game_mode: "control".into(),
            role: role.into(),
            outcome: "victory".into(),
            elims: 1,
            deaths: 1,
            assists: 0,
            damage: 0,
            healing: 0,
            mitigation: 0,
            played_at: Utc.with_ymd_and_hms(2026, 7, 1, 20, 0, 0).unwrap(),
            uploaded_at: Utc::now(),
            edited: false,
            recognizer: scuffed_types::RECOGNIZER_OCR_V1.into(),
            name_recognizer: None,
            suspect_fields: Vec::new(),
        };
        let seeded = [
            ("kept", "Ana", "Damage", "Damage"),
            ("empty", "Reaper", "", "Damage"),
            ("garbage", "Reinhardt", "dps", "Tank"),
            ("unknown", "Unknown", "", ""),
            ("sombra", "Sombra", "Support", "Support"),
        ];
        let rows: Vec<_> = seeded
            .iter()
            .map(|(sid, hero, role, _)| game(sid, hero, role))
            .collect();
        state
            .db
            .upsert_personal_matches(&member_id, &rows)
            .await
            .unwrap();

        let page = get_my_matches(create_router(state.clone()), &session_token).await;
        for (sid, _, _, filled) in seeded {
            assert_eq!(row_by_session(&page, sid)["role"], filled, "{sid}");
        }
        let stored = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        for (sid, _, role, _) in seeded {
            let row = stored.iter().find(|m| m.session_id == sid).unwrap();
            assert_eq!(row.role, role, "stored role for {sid} must not change");
        }
    }

    async fn get_my_match(
        app: axum::Router,
        token: Option<&str>,
        id: &str,
    ) -> (StatusCode, String) {
        let mut req = Request::builder()
            .method("GET")
            .uri(format!("/api/stats/me/matches/{id}"));
        if let Some(token) = token {
            req = req.header(header::AUTHORIZATION, format!("Bearer {token}"));
        }
        let resp = app.oneshot(req.body(Body::empty()).unwrap()).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        (status, String::from_utf8(bytes.to_vec()).unwrap())
    }

    fn single_game(member_id: &str, sid: &str) -> scuffed_db::PersonalMatch {
        scuffed_db::PersonalMatch {
            id: String::new(),
            member_id: member_id.into(),
            session_id: sid.into(),
            hero: "Reaper".into(),
            map_name: "Oasis".into(),
            game_mode: "control".into(),
            role: String::new(),
            outcome: "victory".into(),
            elims: 12,
            deaths: 3,
            assists: 4,
            damage: 9000,
            healing: 500,
            mitigation: 0,
            played_at: Utc.with_ymd_and_hms(2026, 7, 2, 20, 0, 0).unwrap(),
            uploaded_at: Utc::now(),
            edited: true,
            recognizer: "cv-v1".into(),
            name_recognizer: Some("names-v1".into()),
            suspect_fields: vec!["dmg".into(), "map".into()],
        }
    }

    /// Two members, each with one game. Returns the state, the first
    /// member's session, and both games' ids as listed on `/me/matches`.
    async fn two_members_one_game_each() -> (crate::state::AppState, String, String, String) {
        let (state, member_id, _daemon_token, session_token) = member_reader().await;
        crate::test_support::seed_user(&state, "otherplayer", "otherplayer").await;
        let other = state
            .db
            .create_member("otherplayer", "otherplayer", OrgRole::Member)
            .await
            .unwrap();
        state
            .db
            .upsert_personal_matches(&member_id, &[single_game(&member_id, "mine-1")])
            .await
            .unwrap();
        state
            .db
            .upsert_personal_matches(&other.id, &[single_game(&other.id, "theirs-1")])
            .await
            .unwrap();
        let mine = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        let theirs = state
            .db
            .list_personal_matches(&other.id, 10, 0)
            .await
            .unwrap();
        assert_eq!(mine.len(), 1);
        assert_eq!(theirs.len(), 1);
        (
            state,
            session_token,
            mine[0].id.clone(),
            theirs[0].id.clone(),
        )
    }

    #[tokio::test]
    async fn me_match_returns_own_game_same_shape_as_list_row() {
        let (state, session_token, mine, _theirs) = two_members_one_game_each().await;
        let page = get_my_matches(create_router(state.clone()), &session_token).await;
        let listed = row_by_session(&page, "mine-1").clone();
        assert_eq!(listed["id"], mine.as_str());

        let (status, text) = get_my_match(create_router(state), Some(&session_token), &mine).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        let single: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(single, listed, "single game must match its list row");
        assert_eq!(single["edited"], true);
        assert_eq!(single["recognizer"], "cv-v1");
        assert_eq!(single["suspect_fields"], serde_json::json!(["dmg", "map"]));
        assert_eq!(single["role"], "Damage", "role is filled at read time");
    }

    #[tokio::test]
    async fn me_match_other_members_game_is_the_same_404_as_missing() {
        let (state, session_token, _mine, theirs) = two_members_one_game_each().await;
        let (other_status, other_text) =
            get_my_match(create_router(state.clone()), Some(&session_token), &theirs).await;
        assert_eq!(other_status, StatusCode::NOT_FOUND, "{other_text}");

        let (missing_status, missing_text) = get_my_match(
            create_router(state),
            Some(&session_token),
            "pm00000000deadbeef",
        )
        .await;
        assert_eq!(missing_status, StatusCode::NOT_FOUND, "{missing_text}");
        assert_eq!(
            other_text, missing_text,
            "another member's game must not be told apart from a missing one"
        );
    }

    #[tokio::test]
    async fn me_match_missing_and_malformed_ids_are_404() {
        let (state, session_token, _mine, _theirs) = two_members_one_game_each().await;
        let (missing_status, missing_text) = get_my_match(
            create_router(state.clone()),
            Some(&session_token),
            "pm00000000deadbeef",
        )
        .await;
        assert_eq!(missing_status, StatusCode::NOT_FOUND, "{missing_text}");
        for id in ["not-a-key", "12345", "a%3Ab"] {
            let (status, text) =
                get_my_match(create_router(state.clone()), Some(&session_token), id).await;
            assert_eq!(status, StatusCode::NOT_FOUND, "{id}: {text}");
            assert_eq!(
                text, missing_text,
                "{id}: a malformed id must get the same 404 body as a missing one"
            );
        }
    }

    #[tokio::test]
    async fn me_match_without_a_session_is_401() {
        let (state, _session_token, mine, _theirs) = two_members_one_game_each().await;
        let (status, text) = get_my_match(create_router(state.clone()), None, &mine).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
        let (status, text) = get_my_match(create_router(state), Some("not-a-session"), &mine).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
    }

    #[tokio::test]
    async fn me_match_inactive_member_is_403() {
        let (state, member_id, _daemon_token, session_token) = member_reader().await;
        state
            .db
            .upsert_personal_matches(&member_id, &[single_game(&member_id, "mine-1")])
            .await
            .unwrap();
        let mine = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(mine.len(), 1);
        let id = mine[0].id.clone();
        let (status, text) =
            get_my_match(create_router(state.clone()), Some(&session_token), &id).await;
        assert_eq!(status, StatusCode::OK, "{text}");

        state
            .db
            .update_member(
                &member_id,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                Some(false),
                None,
                None,
                None,
            )
            .await
            .expect("deactivate member");
        let (status, text) = get_my_match(create_router(state), Some(&session_token), &id).await;
        assert_eq!(status, StatusCode::FORBIDDEN, "{text}");
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["error"], "Membership inactive");
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
        assert!(
            row["name_recognizer"].is_null(),
            "a row written before name_recognizer existed is unknown: {row}"
        );
        assert_eq!(row["suspect_fields"], serde_json::json!([]));
        assert_eq!(row["elims"], 7);
    }

    #[tokio::test]
    async fn upload_absent_name_recognizer_is_null_on_my_matches() {
        let (state, _member_id, daemon_token, session_token) = member_reader().await;
        let omitted = match_object("sess-omit-name", 4);
        let mut null_field = match_object("sess-null-name", 5);
        null_field["name_recognizer"] = serde_json::Value::Null;
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({ "matches": [omitted, null_field] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["inserted"], 2);

        let page = get_my_matches(create_router(state), &session_token).await;
        for session_id in ["sess-omit-name", "sess-null-name"] {
            let row = row_by_session(&page, session_id);
            assert!(
                row.get("name_recognizer").is_some(),
                "own match reads include the field: {row}"
            );
            assert!(row["name_recognizer"].is_null(), "{row}");
            assert_eq!(row["recognizer"], "ocr-v1");
        }
    }

    #[tokio::test]
    async fn upload_valid_name_recognizer_round_trips_on_my_matches() {
        let (state, member_id, daemon_token, session_token) = member_reader().await;
        let mut names = match_object("sess-names", 4);
        names["recognizer"] = serde_json::json!("cv-v1");
        names["name_recognizer"] = serde_json::json!("names-v1");
        let mut ocr = match_object("sess-ocr-name", 5);
        ocr["name_recognizer"] = serde_json::json!("ocr-v1");
        let mut later = match_object("sess-later-name", 6);
        later["name_recognizer"] = serde_json::json!("reader-v2");
        let max_id = "n".repeat(scuffed_types::RECOGNIZER_ID_MAX_LEN);
        let mut maxed = match_object("sess-max-name", 7);
        maxed["name_recognizer"] = serde_json::json!(max_id);

        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({ "matches": [names, ocr, later, maxed] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        assert_eq!(parsed["inserted"], 4);

        let page = get_my_matches(create_router(state.clone()), &session_token).await;
        assert_eq!(
            row_by_session(&page, "sess-names")["name_recognizer"],
            "names-v1"
        );
        assert_eq!(row_by_session(&page, "sess-names")["recognizer"], "cv-v1");
        assert_eq!(
            row_by_session(&page, "sess-ocr-name")["name_recognizer"],
            "ocr-v1"
        );
        assert_eq!(
            row_by_session(&page, "sess-later-name")["name_recognizer"],
            "reader-v2"
        );
        assert_eq!(
            row_by_session(&page, "sess-max-name")["name_recognizer"],
            max_id
        );

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        let stored = |sid: &str| {
            rows.iter()
                .find(|row| row.session_id == sid)
                .unwrap()
                .name_recognizer
                .clone()
        };
        assert_eq!(stored("sess-names").as_deref(), Some("names-v1"));
        assert_eq!(stored("sess-ocr-name").as_deref(), Some("ocr-v1"));
        assert_eq!(stored("sess-later-name").as_deref(), Some("reader-v2"));
        assert_eq!(stored("sess-max-name").as_deref(), Some(max_id.as_str()));

        // A later upload of the same session that leaves the field out stores
        // NONE. Old clients do not send it, and that upload still succeeds.
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &daemon_token,
            serde_json::json!({ "matches": [match_object("sess-names", 8)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{parsed}");
        let page = get_my_matches(create_router(state.clone()), &session_token).await;
        let cleared = row_by_session(&page, "sess-names");
        assert_eq!(cleared["elims"], 8);
        assert!(cleared["name_recognizer"].is_null(), "{cleared}");
        assert_eq!(
            row_by_session(&page, "sess-ocr-name")["name_recognizer"],
            "ocr-v1"
        );

        let overview = Request::builder()
            .method("GET")
            .uri(format!("/api/stats/member/{member_id}"))
            .header(header::AUTHORIZATION, format!("Bearer {session_token}"))
            .body(Body::empty())
            .unwrap();
        let resp = create_router(state).oneshot(overview).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert!(
            !text.contains("name_recognizer"),
            "member aggregate reads do not include the reader id: {text}"
        );
    }

    #[tokio::test]
    async fn upload_invalid_name_recognizer_is_400_and_stores_nothing() {
        let (state, member_id, token) = daemon().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-keep", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let too_long = "a".repeat(scuffed_types::RECOGNIZER_ID_MAX_LEN + 1);
        let cases = [
            serde_json::json!("NOPE"),
            serde_json::json!(""),
            serde_json::json!(too_long),
            serde_json::json!(1),
            serde_json::json!("has space"),
            serde_json::json!("Names-V1"),
        ];
        for value in cases {
            let mut bad = match_object("sess-keep", 99);
            bad["name_recognizer"] = value.clone();
            let mut sibling = match_object("sess-new", 1);
            sibling["name_recognizer"] = serde_json::json!("names-v1");
            let (status, parsed) = post_raw(
                create_router(state.clone()),
                &token,
                serde_json::json!({ "matches": [sibling, bad] }),
            )
            .await;
            assert_eq!(status, StatusCode::BAD_REQUEST, "{value}: {parsed}");
            assert_eq!(
                parsed["error"],
                format!(
                    "matches[1]: {}",
                    scuffed_types::api::NAME_RECOGNIZER_ID_ERROR
                ),
                "{value}: {parsed}"
            );

            let rows = state
                .db
                .list_personal_matches(&member_id, 10, 0)
                .await
                .unwrap();
            assert_eq!(
                rows.len(),
                1,
                "rejected batch must not insert or update: {value}"
            );
            assert_eq!(rows[0].session_id, "sess-keep");
            assert_eq!(rows[0].elims, 4);
            assert_eq!(rows[0].name_recognizer, None);
            assert_eq!(rows[0].recognizer, "ocr-v1");
        }
    }

    /// Both ids are unusable. The response names `recognizer`, which is
    /// checked before `name_recognizer`.
    #[tokio::test]
    async fn upload_bad_recognizer_and_name_recognizer_reports_recognizer_first() {
        let (state, member_id, token) = daemon().await;
        let (status, _) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [match_object("sess-keep", 4)] }),
        )
        .await;
        assert_eq!(status, StatusCode::OK);

        let mut bad = match_object("sess-keep", 99);
        bad["recognizer"] = serde_json::json!("NOPE");
        bad["name_recognizer"] = serde_json::json!("ALSO-BAD");
        let (status, parsed) = post_raw(
            create_router(state.clone()),
            &token,
            serde_json::json!({ "matches": [bad] }),
        )
        .await;
        assert_eq!(status, StatusCode::BAD_REQUEST, "{parsed}");
        assert_eq!(
            parsed["error"],
            format!("matches[0]: {}", scuffed_types::api::RECOGNIZER_ID_ERROR),
            "{parsed}"
        );

        let rows = state
            .db
            .list_personal_matches(&member_id, 10, 0)
            .await
            .unwrap();
        assert_eq!(
            rows.len(),
            1,
            "rejected batch must not update the stored row"
        );
        assert_eq!(rows[0].session_id, "sess-keep");
        assert_eq!(rows[0].elims, 4);
        assert_eq!(rows[0].recognizer, "ocr-v1");
        assert_eq!(rows[0].name_recognizer, None);
    }

    const BATCH_TOO_LARGE: &str =
        r#"{"code":"batch_too_large","max_matches":200,"max_deleted_sessions":500}"#;

    fn block_on<F: std::future::Future>(fut: F) -> F::Output {
        let rt = tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .expect("runtime");
        rt.block_on(fut)
    }

    fn match_entry(session_id: &str) -> serde_json::Value {
        serde_json::json!({
            "session_id": session_id,
            "hero": "Ana",
            "map_name": "Oasis",
            "game_mode": "control",
            "role": "Support",
            "outcome": "victory",
            "elims": 1,
            "deaths": 1,
            "assists": 1,
            "damage": 1,
            "healing": 1,
            "mitigation": 0,
            "played_at": "2026-07-01T20:00:00Z",
            "edited": false
        })
    }

    fn matches_body(prefix: &str, count: usize) -> serde_json::Value {
        let matches: Vec<_> = (0..count)
            .map(|i| match_entry(&format!("{prefix}-{i}")))
            .collect();
        serde_json::json!({ "matches": matches, "deleted_sessions": [] })
    }

    async fn post_text(
        app: axum::Router,
        token: &str,
        body: serde_json::Value,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        post_text_from(app, token, body, Ipv4Addr::LOCALHOST).await
    }

    async fn post_text_from(
        app: axum::Router,
        token: &str,
        body: serde_json::Value,
        peer: Ipv4Addr,
    ) -> (StatusCode, axum::http::HeaderMap, String) {
        let req = upload_request(token, serde_json::to_vec(&body).unwrap(), peer);
        let resp = app.oneshot(req).await.unwrap();
        let status = resp.status();
        let headers = resp.headers().clone();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        (status, headers, text)
    }

    async fn capped_member(name: &str) -> (crate::state::AppState, String, String) {
        let state = test_state().await;
        let member = state
            .db
            .create_member(&format!("u-{name}"), name, OrgRole::Member)
            .await
            .unwrap();
        let token = format!("tok-{name}");
        state
            .db
            .create_daemon_token(&member.id, &token, name)
            .await
            .unwrap();
        (state, member.id, token)
    }

    async fn insert_expiry_token(
        state: &crate::state::AppState,
        member_id: &str,
        raw: &str,
        label: &str,
        expires_sql: &str,
    ) {
        let token_hash = scuffed_auth::crypto::hash_session_token(raw);
        state
            .db
            .client
            .query(format!(
                "CREATE daemon_token SET \
                    member_id = $mid, \
                    token_hash = $tok, \
                    label = $label, \
                    is_active = true, \
                    created_at = time::now(), \
                    expires_at = {expires_sql}"
            ))
            .bind(("mid", member_id.to_string()))
            .bind(("tok", token_hash))
            .bind(("label", label.to_string()))
            .await
            .unwrap()
            .check()
            .unwrap();
    }

    #[test]
    fn batch_cap_off_accepts_201_matches() {
        let _force = crate::stats_upload_limits::force_batch_cap(false);
        block_on(async {
            let (state, _member_id, token) = capped_member("capoff").await;
            let app = create_router(state.clone());
            let (status, _, text) = post_text(app, &token, matches_body("off", 201)).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            let parsed: StatsUploadResponse = serde_json::from_str(&text).unwrap();
            assert_eq!(parsed.inserted, 201);
            assert_eq!(state.db.count_table("personal_match").await.unwrap(), 201);
        });
    }

    #[test]
    fn batch_cap_on_rejects_201_matches_and_501_deletes_before_any_write() {
        let _force = crate::stats_upload_limits::force_batch_cap(true);
        block_on(async {
            let (state, member_id, token) = capped_member("capon").await;
            let app = create_router(state.clone());

            let (status, _, text) = post_text(app.clone(), &token, matches_body("keep", 1)).await;
            assert_eq!(status, StatusCode::OK, "{text}");
            assert_eq!(state.db.count_table("personal_match").await.unwrap(), 1);

            let (status, headers, text) =
                post_text(app.clone(), &token, matches_body("over", 201)).await;
            assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{text}");
            assert_eq!(text, BATCH_TOO_LARGE);
            assert_eq!(
                headers
                    .get(header::CACHE_CONTROL)
                    .and_then(|v| v.to_str().ok()),
                Some("no-store")
            );
            assert_eq!(
                state.db.count_table("personal_match").await.unwrap(),
                1,
                "oversized matches must not be written"
            );

            let mut deleted: Vec<String> = (0..500).map(|i| format!("gone-{i}")).collect();
            deleted.push("keep-0".into());
            assert_eq!(deleted.len(), 501);
            let (status, headers, text) = post_text(
                app,
                &token,
                serde_json::json!({ "matches": [], "deleted_sessions": deleted }),
            )
            .await;
            assert_eq!(status, StatusCode::PAYLOAD_TOO_LARGE, "{text}");
            assert_eq!(text, BATCH_TOO_LARGE);
            assert_eq!(
                headers
                    .get(header::CACHE_CONTROL)
                    .and_then(|v| v.to_str().ok()),
                Some("no-store")
            );
            let rows = state
                .db
                .list_personal_matches(&member_id, 10, 0)
                .await
                .unwrap();
            assert_eq!(rows.len(), 1);
            assert_eq!(rows[0].session_id, "keep-0");
        });
    }

    #[tokio::test]
    async fn upload_rate_limit_is_per_token_hash() {
        let state = test_state().await;
        let member = state
            .db
            .create_member("u-burst", "burst", OrgRole::Member)
            .await
            .unwrap();
        let token = "burst-token";
        let other = "burst-other";
        state
            .db
            .create_daemon_token(&member.id, token, "burst")
            .await
            .unwrap();
        state
            .db
            .create_daemon_token(&member.id, other, "other")
            .await
            .unwrap();
        let app = create_router(state.clone());

        for i in 0..10 {
            let (status, _, text) = post_text_from(
                app.clone(),
                token,
                matches_body(&format!("b{i}"), 1),
                Ipv4Addr::new(10, 2, 0, i + 1),
            )
            .await;
            assert_eq!(status, StatusCode::OK, "request {i}: {text}");
        }

        let (status, headers, text) = post_text_from(
            app.clone(),
            token,
            matches_body("blocked", 1),
            Ipv4Addr::new(10, 2, 0, 11),
        )
        .await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{text}");
        assert_eq!(text, r#"{"code":"rate_limited"}"#);
        assert_eq!(
            headers
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        let retry = headers
            .get(header::RETRY_AFTER)
            .expect("Retry-After")
            .to_str()
            .unwrap();
        let secs: u64 = retry.parse().expect("Retry-After seconds");
        assert!(secs >= 1, "Retry-After={secs}");
        assert_eq!(
            state.db.count_table("personal_match").await.unwrap(),
            10,
            "the limited request must not write"
        );

        let (status, _, text) = post_text_from(
            app,
            other,
            matches_body("other", 1),
            Ipv4Addr::new(10, 2, 0, 12),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "other token: {text}");
        assert_eq!(state.db.count_table("personal_match").await.unwrap(), 11);
    }

    #[tokio::test]
    async fn over_limit_token_malformed_json_is_429_not_400() {
        let (state, _member_id, token) = daemon().await;
        let app = create_router(state);
        let peer = Ipv4Addr::new(203, 0, 113, 60);
        for i in 0..scuffed_types::STATS_UPLOAD_TOKEN_BURST {
            let (status, _, text) = post_text_from(
                app.clone(),
                &token,
                matches_body(&format!("ok{i}"), 1),
                peer,
            )
            .await;
            assert_eq!(status, StatusCode::OK, "{text}");
        }

        let resp = app
            .oneshot(upload_request(&token, b"{".to_vec(), peer))
            .await
            .unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        let text = String::from_utf8(bytes.to_vec()).unwrap();
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{text}");
        assert_eq!(text, r#"{"code":"rate_limited"}"#);
    }

    #[tokio::test]
    async fn fake_bearers_leave_the_token_limiter_empty() {
        let (state, _member_id, live, _session) = member_reader().await;
        let governor = crate::stats_upload_limits::StatsUploadGovernor::new();
        crate::stats_upload_limits::override_stats_upload_governor(governor.clone());
        let app = create_router(state);

        for i in 1..=30u8 {
            let (status, _, text) = post_text_from(
                app.clone(),
                &format!("made-up-{i}"),
                matches_body("nope", 1),
                Ipv4Addr::new(198, 51, 100, i),
            )
            .await;
            assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
        }
        assert_eq!(
            governor.len(),
            0,
            "unknown bearers must not create limiter entries"
        );

        let (status, _, text) = post_text_from(
            app,
            &live,
            matches_body("live", 1),
            Ipv4Addr::new(203, 0, 113, 20),
        )
        .await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(governor.len(), 1, "one validated token id is one entry");
    }

    #[tokio::test]
    async fn two_tokens_on_one_ip_send_twenty_uploads_without_ip_limit() {
        let (state, member_id, token_a) = daemon().await;
        let token_b = "rec-daemon-token-b";
        state
            .db
            .create_daemon_token(&member_id, token_b, "second")
            .await
            .unwrap();
        let app = create_router(state);
        let peer = Ipv4Addr::new(203, 0, 113, 41);

        for (token, prefix) in [(token_a.as_str(), "a"), (token_b, "b")] {
            for i in 0..scuffed_types::STATS_UPLOAD_TOKEN_BURST {
                let (status, _, text) = post_text_from(
                    app.clone(),
                    token,
                    matches_body(&format!("{prefix}{i}"), 1),
                    peer,
                )
                .await;
                assert_eq!(status, StatusCode::OK, "{prefix}{i}: {text}");
            }
        }

        let (status, _, text) =
            post_text_from(app, &token_a, matches_body("a-over", 1), peer).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{text}");
        assert_eq!(text, r#"{"code":"rate_limited"}"#);
    }

    #[tokio::test]
    async fn upload_ip_limit_returns_governor_429() {
        let state = test_state().await;
        let member = state
            .db
            .create_member("u-ip", "ipplayer", OrgRole::Member)
            .await
            .unwrap();
        let tokens = ["ip-a", "ip-b", "ip-c", "ip-d", "ip-e"];
        for token in tokens {
            state
                .db
                .create_daemon_token(&member.id, token, token)
                .await
                .unwrap();
        }
        let governor = crate::stats_upload_limits::StatsUploadGovernor::new();
        crate::stats_upload_limits::override_stats_upload_governor(governor.clone());
        let app = create_router(state);
        let peer = Ipv4Addr::new(203, 0, 113, 40);
        let burst = scuffed_types::STATS_UPLOAD_IP_BURST;
        assert!(
            burst / (tokens.len() as u32) < scuffed_types::STATS_UPLOAD_TOKEN_BURST,
            "each token must stay under its own burst while the IP bucket fills"
        );

        for i in 0..burst {
            let token = tokens[(i as usize) % tokens.len()];
            let (status, _, text) =
                post_text_from(app.clone(), token, matches_body(&format!("ip{i}"), 1), peer).await;
            assert_eq!(status, StatusCode::OK, "request {i}: {text}");
        }

        let (status, headers, text) =
            post_text_from(app, tokens[0], matches_body("ip-blocked", 1), peer).await;
        assert_eq!(status, StatusCode::TOO_MANY_REQUESTS, "{text}");
        let body: serde_json::Value = serde_json::from_str(&text).unwrap();
        assert_eq!(body["code"], "rate_limited");
        assert_eq!(body["error"], "rate_limited");
        let retry = body["retry_after"].as_u64().expect("retry_after");
        assert!(retry >= 1, "retry_after={retry}");
        assert_eq!(
            headers
                .get(header::RETRY_AFTER)
                .and_then(|v| v.to_str().ok()),
            Some(retry.to_string().as_str())
        );
        assert_eq!(
            headers
                .get(header::CACHE_CONTROL)
                .and_then(|v| v.to_str().ok()),
            Some("no-store")
        );
        assert_eq!(governor.len(), tokens.len());
    }

    #[tokio::test]
    async fn none_expiry_token_uploads_and_expired_token_is_401() {
        let state = test_state().await;
        let member = state
            .db
            .create_member("u-exp", "expiry", OrgRole::Member)
            .await
            .unwrap();
        insert_expiry_token(&state, &member.id, "none-token", "none", "NONE").await;
        insert_expiry_token(
            &state,
            &member.id,
            "expired-token",
            "expired",
            "time::now() - 1d",
        )
        .await;
        let app = create_router(state.clone());

        let (status, _, text) = post_text(app.clone(), "none-token", matches_body("none", 1)).await;
        assert_eq!(status, StatusCode::OK, "{text}");
        assert_eq!(state.db.count_table("personal_match").await.unwrap(), 1);

        let before = state.db.count_table("personal_match").await.unwrap();
        let (status, _, text) = post_text(app, "expired-token", matches_body("expired", 1)).await;
        assert_eq!(status, StatusCode::UNAUTHORIZED, "{text}");
        assert_eq!(text, r#"{"code":"token_expired"}"#);
        assert_eq!(
            state.db.count_table("personal_match").await.unwrap(),
            before
        );
    }

    /// Fields the current site page deserializes. Extra keys must stay ignorable.
    #[derive(Debug, serde::Deserialize)]
    struct CurrentSiteToken {
        id: String,
        member_id: String,
        label: String,
        is_active: bool,
        created_at: chrono::DateTime<Utc>,
        last_used_at: Option<chrono::DateTime<Utc>>,
    }

    #[tokio::test]
    async fn token_list_reports_live_and_expired() {
        let (state, member_id, _daemon, session) = member_reader().await;
        insert_expiry_token(
            &state,
            &member_id,
            "expired-list-token",
            "expired",
            "time::now() - 1d",
        )
        .await;
        insert_expiry_token(&state, &member_id, "none-list-token", "legacy", "NONE").await;

        let req = Request::builder()
            .method("GET")
            .uri("/api/stats/tokens")
            .header(header::AUTHORIZATION, format!("Bearer {session}"))
            .body(Body::empty())
            .unwrap();
        let resp = create_router(state).oneshot(req).await.unwrap();
        let status = resp.status();
        let bytes = resp.into_body().collect().await.unwrap().to_bytes();
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );

        let site: Vec<CurrentSiteToken> =
            serde_json::from_slice(&bytes).expect("current site fields still parse");
        assert_eq!(site.len(), 3);

        let listed: Vec<serde_json::Value> = serde_json::from_slice(&bytes).unwrap();
        let by_label = |label: &str| {
            listed
                .iter()
                .find(|row| row["label"] == label)
                .unwrap_or_else(|| panic!("missing {label}"))
                .clone()
        };
        let live = by_label("tracker");
        let expired = by_label("expired");
        let legacy = by_label("legacy");

        let live_at = live["expires_at"]
            .as_str()
            .expect("live expires_at is an RFC 3339 string");
        let live_exp = chrono::DateTime::parse_from_rfc3339(live_at).expect("RFC 3339");
        assert!(live_exp > Utc::now());
        assert_eq!(live["expired"], false);
        assert_eq!(live["is_active"], true);

        let expired_at = expired["expires_at"]
            .as_str()
            .expect("expired expires_at is an RFC 3339 string");
        let expired_exp = chrono::DateTime::parse_from_rfc3339(expired_at).expect("RFC 3339");
        assert!(expired_exp <= Utc::now());
        assert_eq!(expired["expired"], true);
        assert_eq!(expired["is_active"], true);

        assert!(legacy["expires_at"].is_null());
        assert_eq!(legacy["expired"], false);

        let tracker = site
            .iter()
            .find(|row| row.label == "tracker")
            .expect("live token");
        assert!(tracker.is_active);
        assert_eq!(tracker.id, live["id"]);
        assert_eq!(tracker.member_id, member_id);
        assert!(tracker.created_at <= Utc::now());
        assert!(tracker.last_used_at.is_none());
    }
}
