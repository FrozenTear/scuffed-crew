//! Training crop export for Scuffed Vision.
//!
//! - `POST|GET /api/training-crops/export-tokens` and
//!   `DELETE /api/training-crops/export-tokens/{id}`: admin session only
//!   ([`AdminUser`]). A sync token, an officer and an export token are
//!   refused like on any admin route. Mint and revoke are audited.
//! - `GET /api/training-crops/export` and
//!   `GET /api/training-crops/export/{id}/image`: export token only
//!   ([`ExportTokenAuth`]), behind their own per-IP limit. The token is
//!   read-only and is not a member: every other route answers it 401.
//!
//! What is exported: crops of a stored kind whose owner is an active
//! member, not suspended or banned, who consents to that kind now. Never
//! the member id: `member_key` is an HMAC-SHA256 of it under the token's own
//! random salt.
//!
//! The feed: each list call first syncs the feed tables (see
//! `scuffed_db::queries::training_export`), so a crop that stopped being
//! eligible by any path (any delete route, consent withdrawal, a member
//! leaving, deactivated, suspended or banned) leaves it with a tombstone,
//! and one that became eligible again comes back at a new position. The
//! cursor is a position `(time, items before removals, crop id)` in that
//! feed. Tombstones older than [`TRAINING_TOMBSTONE_RETENTION_DAYS`] are
//! pruned on each call, and a cursor older than that is 410.
//!
//! See "Export for training" in `docs/api/training-crops.md`.

use std::collections::HashMap;

use axum::Json;
use axum::body::Body;
use axum::extract::rejection::JsonRejection;
use axum::extract::{Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use hmac::{Hmac, Mac};
use rand::RngCore;
use scuffed_db::{
    AuditAction, AuditTargetType, TrainingCrop, TrainingExportPosition, TrainingExportToken,
};
use scuffed_types::CropKind;
use scuffed_types::api::{
    TRAINING_CROP_ERR_FULL_SCREEN_REFUSED, TRAINING_CROP_ERR_UNKNOWN_KIND,
    TRAINING_EXPORT_ERR_CURSOR_EXPIRED, TRAINING_EXPORT_ERR_INVALID_CURSOR,
    TRAINING_EXPORT_ERR_INVALID_TOKEN_REQUEST, TRAINING_EXPORT_ERR_TOKEN_NOT_FOUND,
    TRAINING_EXPORT_PAGE_DEFAULT, TRAINING_EXPORT_PAGE_MAX, TRAINING_EXPORT_TOKEN_DEFAULT_DAYS,
    TRAINING_EXPORT_TOKEN_LABEL_MAX_LEN, TRAINING_EXPORT_TOKEN_MAX_DAYS,
    TRAINING_TOMBSTONE_RETENTION_DAYS, TrainingExportItem, TrainingExportPage, TrainingExportQuery,
    TrainingExportRemoved, TrainingExportTokenCreate, TrainingExportTokenCreated,
    TrainingExportTokenInfo,
};
use sha2::{Digest, Sha256};

use crate::extractors::{AdminUser, ExportTokenAuth};
use crate::hero_icons::is_icon_id;
use crate::routes::audit_log::audit;
use crate::routes::hero_icons::{Reject, configured, db_error, internal, member_active, reject};
use crate::state::AppState;

/// Prefix of a plaintext export token, so a leaked one is easy to spot.
const TOKEN_PREFIX: &str = "sctx_";

/// Serialises feed syncs and holds the last stamp handed out, so stamps are
/// strictly increasing and no cursor is ever handed out ahead of a row
/// written later.
static FEED_LOCK: tokio::sync::Mutex<Option<DateTime<Utc>>> = tokio::sync::Mutex::const_new(None);

fn hex(bytes: &[u8]) -> String {
    use std::fmt::Write;
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        let _ = write!(out, "{byte:02x}");
    }
    out
}

fn unhex(text: &str) -> Option<Vec<u8>> {
    if !text.len().is_multiple_of(2) {
        return None;
    }
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(text.get(i..i + 2)?, 16).ok())
        .collect()
}

fn random_hex_32() -> String {
    let mut bytes = [0u8; 32];
    rand::rngs::OsRng.fill_bytes(&mut bytes);
    hex(&bytes)
}

/// Hex HMAC-SHA256 of `member_id` keyed with the token's salt bytes.
pub(crate) fn member_key(salt_hex: &str, member_id: &str) -> String {
    let key = unhex(salt_hex).unwrap_or_else(|| salt_hex.as_bytes().to_vec());
    let mut mac = <Hmac<Sha256> as Mac>::new_from_slice(&key).expect("HMAC takes any key length");
    mac.update(member_id.as_bytes());
    hex(&mac.finalize().into_bytes())
}

fn info(token: &TrainingExportToken) -> TrainingExportTokenInfo {
    TrainingExportTokenInfo {
        id: token.id.clone(),
        label: token.label.clone(),
        created_by: token.created_by.clone(),
        created_at: token.created_at,
        expires_at: token.expires_at,
        revoked_at: token.revoked_at,
        expired: token.expires_at <= Utc::now(),
    }
}

fn invalid_request() -> Reject {
    reject(
        StatusCode::BAD_REQUEST,
        TRAINING_EXPORT_ERR_INVALID_TOKEN_REQUEST,
    )
}

/// POST /api/training-crops/export-tokens (admin session)
///
/// Body `{label, expires_in_days?}`. The plaintext token is in this answer
/// only; the server keeps its hash.
pub async fn mint_token(
    State(state): State<AppState>,
    admin: AdminUser,
    body: Result<Json<TrainingExportTokenCreate>, JsonRejection>,
) -> Result<Response, Reject> {
    let Ok(Json(body)) = body else {
        return Err(invalid_request());
    };
    let label = body.label.trim();
    let label_ok = !label.is_empty()
        && label.chars().count() <= TRAINING_EXPORT_TOKEN_LABEL_MAX_LEN
        && !label.chars().any(char::is_control);
    let days = body
        .expires_in_days
        .unwrap_or(TRAINING_EXPORT_TOKEN_DEFAULT_DAYS);
    if !label_ok || days == 0 || days > TRAINING_EXPORT_TOKEN_MAX_DAYS {
        return Err(invalid_request());
    }
    let raw = format!("{TOKEN_PREFIX}{}", random_hex_32());
    let expires_at = Utc::now() + chrono::Duration::days(i64::from(days));
    let token = state
        .db
        .create_training_export_token(&admin.member.id, &raw, label, expires_at, &random_hex_32())
        .await
        .map_err(db_error)?;
    let details = format!("label {label}, expires {}", expires_at.date_naive());
    audit(
        &state.db,
        &admin.member.id,
        AuditAction::CreatedTrainingExportToken,
        AuditTargetType::TrainingExportToken,
        &token.id,
        Some(details.as_str()),
    )
    .await;
    let created = TrainingExportTokenCreated {
        token: raw,
        info: info(&token),
    };
    Ok((
        StatusCode::CREATED,
        [(header::CACHE_CONTROL, "no-store")],
        Json(created),
    )
        .into_response())
}

/// GET /api/training-crops/export-tokens (admin session). No secrets.
pub async fn list_tokens(
    State(state): State<AppState>,
    _admin: AdminUser,
) -> Result<Json<Vec<TrainingExportTokenInfo>>, Reject> {
    let tokens = state
        .db
        .list_training_export_tokens()
        .await
        .map_err(db_error)?;
    Ok(Json(tokens.iter().map(info).collect()))
}

/// DELETE /api/training-crops/export-tokens/{id} (admin session)
///
/// 404 `export_token_not_found` when no unrevoked token has this id.
pub async fn revoke_token(
    State(state): State<AppState>,
    admin: AdminUser,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<TrainingExportTokenInfo>, Reject> {
    let not_found = || reject(StatusCode::NOT_FOUND, TRAINING_EXPORT_ERR_TOKEN_NOT_FOUND);
    let id_ok = !id.is_empty() && id.len() <= 64 && id.chars().all(|c| c.is_ascii_alphanumeric());
    if !id_ok {
        return Err(not_found());
    }
    let token = state
        .db
        .revoke_training_export_token(&id)
        .await
        .map_err(db_error)?
        .ok_or_else(not_found)?;
    audit(
        &state.db,
        &admin.member.id,
        AuditAction::RevokedTrainingExportToken,
        AuditTargetType::TrainingExportToken,
        &token.id,
        Some(format!("label {}", token.label).as_str()),
    )
    .await;
    Ok(Json(info(&token)))
}

/// `v1.<unix micros>.<tag>.<crop id>`. Opaque to the puller.
fn encode_cursor(pos: &TrainingExportPosition) -> String {
    format!(
        "v1.{}.{}.{}",
        pos.at.timestamp_micros(),
        pos.tag,
        pos.crop_id
    )
}

fn decode_cursor(raw: &str) -> Option<TrainingExportPosition> {
    let mut parts = raw.split('.');
    let (Some("v1"), Some(micros), Some(tag), Some(crop_id), None) = (
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
        parts.next(),
    ) else {
        return None;
    };
    let at = DateTime::from_timestamp_micros(micros.parse().ok()?)?;
    let tag: u8 = tag.parse().ok()?;
    let id_ok = match tag {
        0 | 1 => is_icon_id(crop_id),
        2 => crop_id.is_empty(),
        _ => false,
    };
    id_ok.then(|| TrainingExportPosition {
        at,
        tag,
        crop_id: crop_id.to_string(),
    })
}

/// Now at microsecond precision (what the database keeps and the cursor
/// carries), and after `last`.
fn next_stamp(last: Option<DateTime<Utc>>) -> DateTime<Utc> {
    let now = Utc::now();
    let now = DateTime::from_timestamp_micros(now.timestamp_micros()).unwrap_or(now);
    match last {
        Some(last) if last >= now => last + chrono::Duration::microseconds(1),
        _ => now,
    }
}

fn retention() -> chrono::Duration {
    chrono::Duration::days(i64::from(TRAINING_TOMBSTONE_RETENTION_DAYS))
}

/// `?kind=` on the export: absent for every kind, 400 for an unknown one,
/// 422 for `full_screen` (never stored).
fn export_kind(raw: Option<&str>) -> Result<Option<CropKind>, Reject> {
    let Some(raw) = raw else {
        return Ok(None);
    };
    let kind = CropKind::parse(raw)
        .ok_or_else(|| reject(StatusCode::BAD_REQUEST, TRAINING_CROP_ERR_UNKNOWN_KIND))?;
    if !kind.accepts_uploads() {
        return Err(reject(
            StatusCode::UNPROCESSABLE_ENTITY,
            TRAINING_CROP_ERR_FULL_SCREEN_REFUSED,
        ));
    }
    Ok(Some(kind))
}

fn to_the_hour(at: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::DurationRound;
    at.duration_trunc(chrono::TimeDelta::hours(1)).unwrap_or(at)
}

/// The stored file's bytes, `None` when it is gone.
async fn read_crop_file(state: &AppState, id: &str) -> Result<Option<Vec<u8>>, Reject> {
    match tokio::fs::read(state.hero_icons.icon_path(id)).await {
        Ok(bytes) => Ok(Some(bytes)),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("training crop row has no file");
            Ok(None)
        }
        Err(error) => {
            tracing::error!(%error, "training crop file read failed");
            Err(internal())
        }
    }
}

/// GET /api/training-crops/export?kind=&since=&limit= (export token)
///
/// Oldest first. `items` are crops placed in the feed after `since`,
/// `removed` are crops that left it after `since`, in one order, cut at
/// `limit` (default 100, at most 500) events together. One audit entry per
/// call.
pub async fn export_list(
    State(state): State<AppState>,
    auth: ExportTokenAuth,
    Query(query): Query<TrainingExportQuery>,
) -> Result<Json<TrainingExportPage>, Reject> {
    configured(&state)?;
    let kind = export_kind(query.kind.as_deref())?;
    let limit = query
        .limit
        .unwrap_or(TRAINING_EXPORT_PAGE_DEFAULT)
        .clamp(1, TRAINING_EXPORT_PAGE_MAX);
    let since =
        match query.since.as_deref() {
            None => None,
            Some(raw) => Some(decode_cursor(raw).ok_or_else(|| {
                reject(StatusCode::BAD_REQUEST, TRAINING_EXPORT_ERR_INVALID_CURSOR)
            })?),
        };
    if since
        .as_ref()
        .is_some_and(|pos| pos.at < Utc::now() - retention())
    {
        return Err(reject(StatusCode::GONE, TRAINING_EXPORT_ERR_CURSOR_EXPIRED));
    }

    let (stamp, mut events) = {
        let mut last = FEED_LOCK.lock().await;
        let stamp = next_stamp(*last);
        *last = Some(stamp);
        if let Err(error) = state
            .db
            .prune_training_tombstones(stamp - retention())
            .await
        {
            tracing::warn!(%error, "training tombstone prune failed");
        }
        state
            .db
            .sync_training_export(stamp)
            .await
            .map_err(db_error)?;
        let events = state
            .db
            .training_export_events_after(since.as_ref(), kind, limit + 1)
            .await
            .map_err(db_error)?;
        (stamp, events)
    };
    let has_more = events.len() > limit as usize;
    events.truncate(limit as usize);
    let next = match (has_more, events.last()) {
        (true, Some(last)) => last.position.clone(),
        _ => TrainingExportPosition {
            at: stamp,
            tag: 2,
            crop_id: String::new(),
        },
    };

    let item_ids: Vec<String> = events
        .iter()
        .filter(|e| !e.removed)
        .map(|e| e.position.crop_id.clone())
        .collect();
    let rows: HashMap<String, TrainingCrop> = state
        .db
        .training_crops_by_ids(&item_ids)
        .await
        .map_err(db_error)?
        .into_iter()
        .map(|crop| (crop.id.clone(), crop))
        .collect();
    let salt = &auth.token.pseudonym_salt;
    let mut items = Vec::new();
    let mut removed = Vec::new();
    for event in &events {
        let id = &event.position.crop_id;
        if event.removed {
            removed.push(TrainingExportRemoved {
                id: id.clone(),
                kind: event.kind,
            });
            continue;
        }
        // Gone since the sync, or unreadable: the next sync tombstones a
        // deleted row; an unreadable one is skipped.
        let Some(crop) = rows.get(id) else {
            continue;
        };
        let Some(bytes) = read_crop_file(&state, id).await? else {
            continue;
        };
        items.push(TrainingExportItem {
            id: crop.id.clone(),
            kind: crop.kind,
            hero: crop.hero.clone(),
            meta: crop.meta.clone(),
            created_at: to_the_hour(crop.created_at),
            sha256: hex(&Sha256::digest(&bytes)),
            size_bytes: bytes.len() as u64,
            width: crop.width,
            height: crop.height,
            member_key: member_key(salt, &crop.member_id),
        });
    }

    let scope = kind.map_or("all kinds", CropKind::as_str);
    let details = format!(
        "export token {}: {} items, {} removed, {scope}",
        auth.token.id,
        items.len(),
        removed.len()
    );
    audit(
        &state.db,
        &auth.token.created_by,
        AuditAction::ExportedTrainingCrops,
        AuditTargetType::TrainingExportToken,
        &auth.token.id,
        Some(details.as_str()),
    )
    .await;
    Ok(Json(TrainingExportPage {
        items,
        removed,
        next_cursor: encode_cursor(&next),
        has_more,
        tombstone_retention_days: TRAINING_TOMBSTONE_RETENTION_DAYS,
    }))
}

/// True when `crop` may be exported right now: a stored kind, owner active,
/// not suspended or banned, consenting to the kind.
async fn exportable_now(state: &AppState, id: &str) -> Result<bool, Reject> {
    let Some(owner) = state.db.training_crop_owner(id).await.map_err(db_error)? else {
        return Ok(false);
    };
    let Some(kind) = owner.kind.filter(|kind| kind.accepts_uploads()) else {
        return Ok(false);
    };
    if !member_active(state, &owner.member_id).await? {
        return Ok(false);
    }
    let blocked = state
        .db
        .is_member_suspended_or_banned(&owner.member_id)
        .await
        .map_err(db_error)?;
    if blocked {
        return Ok(false);
    }
    let consent = state
        .db
        .get_training_crop_consent(&owner.member_id)
        .await
        .map_err(db_error)?;
    Ok(consent.get(kind))
}

/// GET /api/training-crops/export/{id}/image (export token)
///
/// The same eligibility check as the list, at request time. Anything else
/// is 404 `crop_not_found`.
pub async fn export_image(
    State(state): State<AppState>,
    _auth: ExportTokenAuth,
    UrlPath(id): UrlPath<String>,
) -> Result<Response, Reject> {
    configured(&state)?;
    let not_found = || {
        reject(
            StatusCode::NOT_FOUND,
            scuffed_types::api::TRAINING_CROP_ERR_NOT_FOUND,
        )
    };
    if !is_icon_id(&id) || !exportable_now(&state, &id).await? {
        return Err(not_found());
    }
    let Some(bytes) = read_crop_file(&state, &id).await? else {
        return Err(not_found());
    };
    Response::builder()
        .status(StatusCode::OK)
        .header(header::CONTENT_TYPE, "image/png")
        .header(header::CONTENT_DISPOSITION, "inline")
        .header(header::X_CONTENT_TYPE_OPTIONS, "nosniff")
        .header(header::CACHE_CONTROL, "private, no-store")
        .body(Body::from(bytes))
        .map_err(|_| internal())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cursor_round_trips_and_rejects_junk() {
        let at = DateTime::from_timestamp_micros(1_790_000_000_123_456).unwrap();
        let pos = TrainingExportPosition {
            at,
            tag: 0,
            crop_id: "0123456789abcdef0123456789abcdef".into(),
        };
        assert_eq!(decode_cursor(&encode_cursor(&pos)), Some(pos));
        let end = TrainingExportPosition {
            at,
            tag: 2,
            crop_id: String::new(),
        };
        assert_eq!(decode_cursor(&encode_cursor(&end)), Some(end));
        for junk in [
            "",
            "v1",
            "v2.1.0.0123456789abcdef0123456789abcdef",
            "v1.x.0.0123456789abcdef0123456789abcdef",
            "v1.1.3.",
            "v1.1.2.abc",
            "v1.1.0.not-hex",
            "v1.1.0.0123456789abcdef0123456789abcdef.extra",
        ] {
            assert_eq!(decode_cursor(junk), None, "{junk}");
        }
    }

    #[test]
    fn stamps_increase_even_when_the_clock_does_not() {
        let ahead = Utc::now() + chrono::Duration::seconds(5);
        let next = next_stamp(Some(ahead));
        assert_eq!(next, ahead + chrono::Duration::microseconds(1));
        let fresh = next_stamp(None);
        assert_eq!(fresh.timestamp_subsec_nanos() % 1000, 0);
    }

    #[test]
    fn member_key_is_keyed_by_the_salt() {
        let a = member_key(&"11".repeat(32), "member-1");
        assert_eq!(a.len(), 64);
        assert_eq!(a, member_key(&"11".repeat(32), "member-1"));
        assert_ne!(a, member_key(&"22".repeat(32), "member-1"));
        assert_ne!(a, member_key(&"11".repeat(32), "member-2"));
        // RFC 4231 test case 2: key "Jefe".
        assert_eq!(
            member_key(&hex(b"Jefe"), "what do ya want for nothing?"),
            "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843"
        );
    }
}
