//! Opt-in training crop routes (`/api/training-crops`).
//!
//! The stat tracker sends small PNG crops of what it read so recognizers can
//! be trained on real captures. This is the hero icon store generalized:
//! every crop is a `hero_icon` row with a `kind`, the files share the private
//! `HERO_ICONS_DIR`, and hero icons are the `hero_icon` kind (the
//! `/api/hero-icons` routes keep working and only see that kind).
//!
//! Access rules:
//!
//! - The tracker's sync token is accepted, besides a session, on exactly:
//!   upload, own consent read, own delete-all and own single delete
//!   ([`CropUploader`]). Those routes also share a per-IP limit (see
//!   `create_router_with_dist`). A sync token never acts as staff: on the
//!   single delete it reaches the caller's own crops only. Every other route,
//!   setting consent included, needs a session, so consent is only ever
//!   given on the site.
//! - Consent is per kind and off by default. `hero_icon` consent is the hero
//!   icon opt-in. An upload of a kind the caller does not consent to is 403
//!   `crop_consent_required` and stores nothing.
//! - `full_screen` uploads are refused (422 `full_screen_not_accepted`)
//!   before the body is read. Its consent cannot be stored either: the
//!   consent shapes have no `full_screen` field.
//! - Turning a kind's consent off keeps its crops and hides them from staff,
//!   the same rule as the hero icon opt-out. The owner can still delete them,
//!   and so can an admin while the owner is active.
//! - The admin list covers every stored crop of active members so an admin
//!   can find any id to delete; crops of a kind without consent come back
//!   `hidden`, without hero or metadata, and their image stays 404. Times in
//!   that list are cut to the hour. Officers get per-member counts only, for
//!   consented kinds only.
//! - Consent `GET` and `PUT` work while the store is disabled. Every other
//!   route answers 503 then.
//!
//! See `docs/api/training-crops.md`.

use std::collections::{BTreeMap, HashMap};

use axum::Json;
use axum::body::Body;
use axum::extract::multipart::{Field, MultipartRejection};
use axum::extract::rejection::JsonRejection;
use axum::extract::{Multipart, Path as UrlPath, Query, State};
use axum::http::{HeaderMap, StatusCode, header};
use axum::response::Response;
use chrono::{DateTime, Utc};
use scuffed_db::{AuditAction, AuditTargetType, NewTrainingCrop, OrgRole, TrainingCrop};
use scuffed_types::api::{
    CursorResponse, HERO_ICON_ERR_INVALID_MULTIPART, HERO_ICON_ERR_UNEXPECTED_FIELD,
    HERO_ICON_ERR_UNKNOWN_HERO, MyTrainingCrops, PaginationParams, StaffTrainingCrop,
    TRAINING_CROP_ERR_CONSENT_REQUIRED, TRAINING_CROP_ERR_FULL_SCREEN_REFUSED,
    TRAINING_CROP_ERR_INVALID_CONSENT, TRAINING_CROP_ERR_INVALID_META,
    TRAINING_CROP_ERR_LIMIT_REACHED, TRAINING_CROP_ERR_NO_CROPS, TRAINING_CROP_ERR_NOT_FOUND,
    TRAINING_CROP_ERR_TOO_MANY_IN_REQUEST, TRAINING_CROP_ERR_UNKNOWN_KIND, TrainingCropConsent,
    TrainingCropConsentUpdate, TrainingCropEntry, TrainingCropKindCount, TrainingCropMemberCounts,
    TrainingCropMeta, TrainingCropResolutionCount, TrainingCropUploadMeta,
    TrainingCropUploadResponse, TrainingCropsDeleted,
};
use scuffed_types::{
    CropKind, HERO_ICON_MAX_FILES_PER_UPLOAD, TRAINING_CROP_META_MAX_BYTES,
    TRAINING_CROP_VERSION_MAX_LEN, hero_key,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::extractors::{AdminUser, CropUploader, OfficerUser, OrgMember};
use crate::hero_icons::{ensure_hero_icons_dir, is_icon_id};
use crate::routes::audit_log::audit;
use crate::routes::hero_icons::{
    ADMIN_REQUIRED, CleanIcon, Reject, UploadReject, clean_png_within, configured, db_error,
    discard_icon_file, internal, member_active, multipart_reject, read_png_part, reject,
    remove_icon_file, write_private,
};
use crate::state::AppState;

fn not_found() -> Reject {
    reject(StatusCode::NOT_FOUND, TRAINING_CROP_ERR_NOT_FOUND)
}

fn count_u32(count: u64) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// `?kind=` on upload (required) and on the list and delete-all routes
/// (optional filter).
#[derive(Debug, Default, Deserialize)]
pub struct CropKindQuery {
    pub kind: Option<String>,
}

/// 400 `unknown_crop_kind` for a missing or unknown kind.
fn parse_kind(raw: Option<&str>) -> Result<CropKind, Reject> {
    raw.and_then(CropKind::parse)
        .ok_or_else(|| reject(StatusCode::BAD_REQUEST, TRAINING_CROP_ERR_UNKNOWN_KIND))
}

/// `None` when no kind is given, 400 `unknown_crop_kind` for an unknown one.
fn parse_kind_filter(raw: Option<&str>) -> Result<Option<CropKind>, Reject> {
    match raw {
        None => Ok(None),
        Some(value) => parse_kind(Some(value)).map(Some),
    }
}

fn thumbnail_url(id: &str) -> String {
    format!("/api/training-crops/{id}/image")
}

fn entry(crop: &TrainingCrop) -> TrainingCropEntry {
    TrainingCropEntry {
        id: crop.id.clone(),
        kind: crop.kind,
        hero: crop.hero.clone(),
        size_bytes: crop.size_bytes,
        width: crop.width,
        height: crop.height,
        uploaded_at: crop.created_at,
        tracker_version: crop.tracker_version.clone(),
        meta: crop.meta.clone(),
        thumbnail_url: thumbnail_url(&crop.id),
    }
}

/// The tracker version to store, from a `scuffed-stat-tracker/<semver>`
/// User-Agent. `None` when there is none, or when the version is longer than
/// `TRAINING_CROP_VERSION_MAX_LEN` (64) characters or holds a control
/// character, so a crafted User-Agent cannot fill the column.
fn stored_tracker_version(headers: &HeaderMap) -> Option<String> {
    let version = crate::routes::stats::tracker_version(headers)?.to_string();
    let fits =
        version.len() <= TRAINING_CROP_VERSION_MAX_LEN && !version.chars().any(char::is_control);
    fits.then_some(version)
}

/// Audit action for an admin act on one crop. `hero_icon` keeps the hero
/// icon names, so the hero icon audit trail reads the same whichever route
/// was used. Every other kind gets the training crop names.
fn delete_action(kind: CropKind) -> AuditAction {
    match kind {
        CropKind::HeroIcon => AuditAction::DeletedHeroIcon,
        _ => AuditAction::DeletedTrainingCrop,
    }
}

/// Audit action for an admin read of the list: `viewed_hero_icons` when the
/// list is filtered to `hero_icon`, `viewed_training_crops` otherwise.
fn list_action(filter: Option<CropKind>) -> AuditAction {
    match filter {
        Some(CropKind::HeroIcon) => AuditAction::ViewedHeroIcons,
        _ => AuditAction::ViewedTrainingCrops,
    }
}

/// `at` cut down to the start of its UTC hour. Used in the admin list so an
/// upload time cannot be matched to a game.
fn to_the_hour(at: DateTime<Utc>) -> DateTime<Utc> {
    use chrono::DurationRound;
    at.duration_trunc(chrono::TimeDelta::hours(1)).unwrap_or(at)
}

/// The admin list item for `crop`. A hidden crop (no consent for its kind)
/// carries no hero, tracker version or metadata.
fn staff_item(crop: &TrainingCrop, display_name: String, hidden: bool) -> StaffTrainingCrop {
    let mut item = entry(crop);
    item.uploaded_at = to_the_hour(item.uploaded_at);
    if hidden {
        item.hero = None;
        item.tracker_version = None;
        item.meta = TrainingCropMeta::default();
    }
    StaffTrainingCrop {
        member_id: crop.member_id.clone(),
        display_name,
        hidden,
        crop: item,
    }
}

/// Kinds the server stores, in list order.
fn stored_kinds() -> impl Iterator<Item = CropKind> {
    CropKind::ALL
        .into_iter()
        .filter(|kind| kind.accepts_uploads())
}

/// Best-effort rollback of rows and files this request stored.
async fn remove_stored(state: &AppState, crops: &[TrainingCrop]) {
    for crop in crops {
        if let Err(error) = state.db.delete_training_crop(&crop.id).await {
            tracing::error!(%error, "training crop rollback delete failed");
        }
        discard_icon_file(state, &crop.id).await;
    }
}

/// Write one file, then its row. On a failed insert the file is removed again.
async fn store_crop(
    state: &AppState,
    mut row: NewTrainingCrop,
    image: CleanIcon,
) -> Result<TrainingCrop, Reject> {
    let id = Uuid::new_v4().simple().to_string();
    let path = state.hero_icons.icon_path(&id);
    if let Err(error) = write_private(&path, &image.bytes).await {
        tracing::error!(%error, "training crop file write failed");
        // A partial file is ours to remove. An existing one is not.
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            discard_icon_file(state, &id).await;
        }
        return Err(internal());
    }
    row.file_name = format!("{id}.png");
    row.id = id.clone();
    row.size_bytes = image.bytes.len() as u64;
    row.width = image.width;
    row.height = image.height;
    match state.db.insert_training_crop(&row).await {
        Ok(stored) => Ok(stored),
        Err(error) => {
            tracing::error!(%error, "training crop insert failed");
            discard_icon_file(state, &id).await;
            Err(internal())
        }
    }
}

/// Checks that run after an upload stored its crops, as for hero icons.
///
/// Two uploads can pass the first count together, so the cap of the kind is
/// checked again (409). The member may also have withdrawn consent for the
/// kind while the body was read: this request's crops are removed and the
/// answer is 403. Crops stored by earlier requests are not touched. On any
/// failure, including a database error, the rows and files in `stored` go.
pub(crate) async fn recheck_crops_after_store(
    state: &AppState,
    member_id: &str,
    kind: CropKind,
    stored: &[TrainingCrop],
) -> Result<(), Reject> {
    let cap = u64::from(kind.max_per_member());
    let stored_after = match state
        .db
        .count_training_crops_for_member(member_id, kind)
        .await
    {
        Ok(count) => count,
        Err(error) => {
            remove_stored(state, stored).await;
            return Err(db_error(error));
        }
    };
    if stored_after > cap {
        remove_stored(state, stored).await;
        return Err(reject(
            StatusCode::CONFLICT,
            TRAINING_CROP_ERR_LIMIT_REACHED,
        ));
    }
    let consent = match state.db.get_training_crop_consent(member_id).await {
        Ok(value) => value,
        Err(error) => {
            remove_stored(state, stored).await;
            return Err(db_error(error));
        }
    };
    if !consent.get(kind) {
        remove_stored(state, stored).await;
        return Err(reject(
            StatusCode::FORBIDDEN,
            TRAINING_CROP_ERR_CONSENT_REQUIRED,
        ));
    }
    Ok(())
}

/// Read one `meta` part, at most [`TRAINING_CROP_META_MAX_BYTES`].
async fn read_meta_part(field: &mut Field<'_>) -> Result<Vec<u8>, Reject> {
    let mut data: Vec<u8> = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_reject)? {
        data.extend_from_slice(&chunk);
        if data.len() > TRAINING_CROP_META_MAX_BYTES {
            tracing::debug!("training crop meta part too large");
            return Err(reject(
                StatusCode::BAD_REQUEST,
                TRAINING_CROP_ERR_INVALID_META,
            ));
        }
    }
    Ok(data)
}

/// Parse and check one `meta` part for `kind`.
///
/// 400 `invalid_crop_meta` for bad JSON, an unknown field or a value out of
/// bounds. 400 `unknown_hero` for a hero that is not on the list, or a
/// missing hero on a `hero_icon` crop. Returns the hero key (`soldier-76`,
/// see [`hero_key`]) and the metadata to store.
pub(crate) fn parse_meta(
    kind: CropKind,
    bytes: &[u8],
) -> Result<(Option<&'static str>, TrainingCropMeta), Reject> {
    let invalid = || reject(StatusCode::BAD_REQUEST, TRAINING_CROP_ERR_INVALID_META);
    let meta: TrainingCropUploadMeta = serde_json::from_slice(bytes).map_err(|error| {
        tracing::debug!(%error, "training crop meta is not valid");
        invalid()
    })?;
    if let Err(field) = meta.validate() {
        tracing::debug!(field, "training crop meta out of bounds");
        return Err(invalid());
    }
    let (hero_raw, stored) = meta.split();
    let hero = match hero_raw.as_deref() {
        Some(raw) => match hero_key(raw) {
            Some(key) => Some(key),
            None => return Err(reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_UNKNOWN_HERO)),
        },
        None => None,
    };
    if kind == CropKind::HeroIcon && hero.is_none() {
        return Err(reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_UNKNOWN_HERO));
    }
    Ok((hero, stored))
}

/// One crop that passed every check and waits to be stored.
struct PendingCrop {
    hero: Option<&'static str>,
    meta: TrainingCropMeta,
    image: CleanIcon,
}

/// POST /api/training-crops?kind=<kind>
///
/// Session or sync token ([`CropUploader`]). Multipart body of up to
/// `HERO_ICON_MAX_FILES_PER_UPLOAD` pairs: a `meta` part (JSON
/// [`TrainingCropUploadMeta`]) followed by its `crop` part (PNG).
///
/// Order of checks: 503 when disabled, kind (400), `full_screen` (422), the
/// per-member per-kind limiter (429), consent (403), the stored cap (409),
/// then the body: part names and order (400 `unexpected_field`), count (413),
/// each `meta` (400), each `crop` (signature 415, size 413, dimensions and
/// decode 400; bounds per kind, see `CropKind::max_edges` and
/// `CropKind::max_crop_bytes`), at least one pair (400 `no_crops`), the cap
/// again (409). Nothing is stored unless every part passes. The tracker
/// version comes from a `scuffed-stat-tracker/<semver>` User-Agent, like
/// stats sync, and is dropped when too long ([`stored_tracker_version`]).
pub async fn upload_training_crops(
    State(state): State<AppState>,
    uploader: CropUploader,
    headers: HeaderMap,
    Query(query): Query<CropKindQuery>,
    multipart: Result<Multipart, MultipartRejection>,
) -> Result<(StatusCode, Json<TrainingCropUploadResponse>), UploadReject> {
    configured(&state)?;
    let kind = parse_kind(query.kind.as_deref())?;
    if !kind.accepts_uploads() {
        let code = TRAINING_CROP_ERR_FULL_SCREEN_REFUSED;
        return Err(reject(StatusCode::UNPROCESSABLE_ENTITY, code).into());
    }
    let member_id = uploader.member.id.clone();
    if let Err(secs) = state.hero_icons.check_kind(&member_id, kind) {
        let limited = crate::rate_limit::rate_limited_response(secs);
        return Err(UploadReject::Limited(Box::new(limited)));
    }
    let consent = state
        .db
        .get_training_crop_consent(&member_id)
        .await
        .map_err(db_error)?;
    if !consent.get(kind) {
        let code = TRAINING_CROP_ERR_CONSENT_REQUIRED;
        return Err(reject(StatusCode::FORBIDDEN, code).into());
    }
    let cap = u64::from(kind.max_per_member());
    let stored_before = state
        .db
        .count_training_crops_for_member(&member_id, kind)
        .await
        .map_err(db_error)?;
    if stored_before >= cap {
        return Err(reject(StatusCode::CONFLICT, TRAINING_CROP_ERR_LIMIT_REACHED).into());
    }
    let Ok(mut multipart) = multipart else {
        return Err(reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_INVALID_MULTIPART).into());
    };

    let unexpected = || reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_UNEXPECTED_FIELD);
    let (max_width, max_height) = kind.max_edges();
    let mut accepted: Vec<PendingCrop> = Vec::new();
    let mut pending: Option<(Option<&'static str>, TrainingCropMeta)> = None;
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_reject)? {
        match field.name() {
            Some("meta") => {
                if pending.is_some() {
                    return Err(unexpected().into());
                }
                if accepted.len() >= HERO_ICON_MAX_FILES_PER_UPLOAD {
                    let code = TRAINING_CROP_ERR_TOO_MANY_IN_REQUEST;
                    return Err(reject(StatusCode::PAYLOAD_TOO_LARGE, code).into());
                }
                let bytes = read_meta_part(&mut field).await?;
                pending = Some(parse_meta(kind, &bytes)?);
            }
            Some("crop") => {
                let Some((hero, meta)) = pending.take() else {
                    return Err(unexpected().into());
                };
                let data = read_png_part(&mut field, kind.max_crop_bytes()).await?;
                let cleaned = tokio::task::spawn_blocking(move || {
                    clean_png_within(&data, max_width, max_height)
                })
                .await
                .map_err(|error| {
                    tracing::error!(%error, "training crop decode task failed");
                    internal()
                })?;
                match cleaned {
                    Ok(image) => accepted.push(PendingCrop { hero, meta, image }),
                    Err(code) => return Err(reject(StatusCode::BAD_REQUEST, code).into()),
                }
            }
            _ => return Err(unexpected().into()),
        }
    }
    if pending.is_some() {
        return Err(unexpected().into());
    }
    if accepted.is_empty() {
        return Err(reject(StatusCode::BAD_REQUEST, TRAINING_CROP_ERR_NO_CROPS).into());
    }
    if stored_before + accepted.len() as u64 > cap {
        return Err(reject(StatusCode::CONFLICT, TRAINING_CROP_ERR_LIMIT_REACHED).into());
    }

    let dir = state.hero_icons.dir();
    if tokio::fs::metadata(dir).await.is_err() {
        ensure_hero_icons_dir(dir).await.map_err(|error| {
            tracing::error!(%error, "hero icon directory create failed");
            internal()
        })?;
    }
    let tracker_version = stored_tracker_version(&headers);
    // One microsecond apart, as for hero icons, so lists are stable.
    let base = Utc::now();
    let mut stored: Vec<TrainingCrop> = Vec::with_capacity(accepted.len());
    for (index, crop) in accepted.into_iter().enumerate() {
        let offset = i64::try_from(index).unwrap_or(i64::MAX);
        let created_at: DateTime<Utc> = base + chrono::Duration::microseconds(offset);
        let row = NewTrainingCrop {
            id: String::new(),
            member_id: member_id.clone(),
            kind,
            hero: crop.hero.map(str::to_string),
            file_name: String::new(),
            size_bytes: 0,
            width: 0,
            height: 0,
            created_at,
            tracker_version: tracker_version.clone(),
            meta: crop.meta,
        };
        match store_crop(&state, row, crop.image).await {
            Ok(row) => stored.push(row),
            Err(error) => {
                remove_stored(&state, &stored).await;
                return Err(error.into());
            }
        }
    }
    recheck_crops_after_store(&state, &member_id, kind, &stored).await?;

    let crops: Vec<TrainingCropEntry> = stored.iter().map(entry).collect();
    Ok((
        StatusCode::CREATED,
        Json(TrainingCropUploadResponse { kind, crops }),
    ))
}

/// GET /api/training-crops/consent
///
/// Session or sync token. Works while the store is disabled.
pub async fn get_consent(
    State(state): State<AppState>,
    uploader: CropUploader,
) -> Result<Json<TrainingCropConsent>, Reject> {
    let consent = state
        .db
        .get_training_crop_consent(&uploader.member.id)
        .await
        .map_err(db_error)?;
    Ok(Json(consent))
}

/// PUT /api/training-crops/consent
///
/// Session only ([`OrgMember`]): a sync token is 401, so consent is only
/// given on the site and the tracker shows it read-only. Only the kinds in
/// the body change. Turning a kind off keeps its crops and hides them from
/// staff. Works while the store is disabled. 400 `invalid_consent` for a
/// body that is not JSON, names an unknown kind (`full_screen` included, its
/// consent cannot be stored) or has a value that is not a bool.
pub async fn put_consent(
    State(state): State<AppState>,
    member: OrgMember,
    body: Result<Json<TrainingCropConsentUpdate>, JsonRejection>,
) -> Result<Json<TrainingCropConsent>, Reject> {
    let Ok(Json(update)) = body else {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            TRAINING_CROP_ERR_INVALID_CONSENT,
        ));
    };
    let consent = state
        .db
        .set_training_crop_consent(&member.member.id, update)
        .await
        .map_err(db_error)?;
    Ok(Json(consent))
}

/// Per-kind counts of one member's stored crops.
async fn kind_counts(
    state: &AppState,
    member_id: &str,
    consent: &TrainingCropConsent,
) -> Result<Vec<TrainingCropKindCount>, Reject> {
    let mut kinds = Vec::new();
    for kind in stored_kinds() {
        let count = state
            .db
            .count_training_crops_for_member(member_id, kind)
            .await
            .map_err(db_error)?;
        kinds.push(TrainingCropKindCount {
            kind,
            count: count_u32(count),
            consented: consent.get(kind),
            max: kind.max_per_member(),
        });
    }
    Ok(kinds)
}

/// GET /api/training-crops/mine?kind=&cursor=&limit= (session only)
///
/// Own crops of every kind, consented or not, newest first.
pub async fn list_mine(
    State(state): State<AppState>,
    member: OrgMember,
    Query(filter): Query<CropKindQuery>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<MyTrainingCrops>, Reject> {
    configured(&state)?;
    let kind = parse_kind_filter(filter.kind.as_deref())?;
    let (limit, offset) = pagination.resolve();
    let member_id = &member.member.id;
    let consent = state
        .db
        .get_training_crop_consent(member_id)
        .await
        .map_err(db_error)?;
    let kinds = kind_counts(&state, member_id, &consent).await?;
    let rows = state
        .db
        .list_training_crops_for_member(member_id, kind, Some((limit, offset)))
        .await
        .map_err(db_error)?;
    let page = CursorResponse::from_oversized(rows.iter().map(entry).collect(), limit, offset);
    Ok(Json(MyTrainingCrops {
        consent,
        kinds,
        crops: page.data,
        next_cursor: page.next_cursor,
    }))
}

/// What happened in [`delete_all_crops`]. `deleted` counts the rows removed
/// even when `failure` is set.
struct DeleteAllOutcome {
    deleted: u32,
    failure: Option<Reject>,
}

/// Remove every crop of `member_id`, or every crop of one kind.
///
/// Same rules as the hero icon delete-all: each file goes before its row, a
/// file that cannot be removed keeps its row (and the answer is a 500 after
/// the other crops went), and a database error stops the loop.
async fn delete_all_crops(
    state: &AppState,
    member_id: &str,
    kind: Option<CropKind>,
) -> Result<DeleteAllOutcome, Reject> {
    let rows = state
        .db
        .list_training_crops_for_member(member_id, kind, None)
        .await
        .map_err(db_error)?;
    let mut deleted: u32 = 0;
    let mut file_failed = false;
    let mut failure = None;
    for crop in &rows {
        if let Err(error) = remove_icon_file(state, &crop.id).await {
            tracing::error!(%error, "training crop file delete failed");
            file_failed = true;
            continue;
        }
        match state.db.delete_training_crop(&crop.id).await {
            Ok(Some(_)) => deleted += 1,
            Ok(None) => {}
            Err(error) => {
                failure = Some(db_error(error));
                break;
            }
        }
    }
    if failure.is_none() && file_failed {
        failure = Some(internal());
    }
    Ok(DeleteAllOutcome { deleted, failure })
}

/// DELETE /api/training-crops/mine?kind=
///
/// Session or sync token. Removes every row and file of the caller, or of one
/// kind with `?kind=`. Consent is left as it is.
pub async fn delete_mine(
    State(state): State<AppState>,
    uploader: CropUploader,
    Query(filter): Query<CropKindQuery>,
) -> Result<Json<TrainingCropsDeleted>, Reject> {
    configured(&state)?;
    let kind = parse_kind_filter(filter.kind.as_deref())?;
    let outcome = delete_all_crops(&state, &uploader.member.id, kind).await?;
    match outcome.failure {
        Some(failure) => Err(failure),
        None => Ok(Json(TrainingCropsDeleted {
            deleted: outcome.deleted,
        })),
    }
}

/// What an admin wants to do with another member's crop.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AdminAccess {
    /// Read: owner active and consenting to the crop's kind.
    Read,
    /// Delete: owner active, consent or not.
    Delete,
}

/// The crop when the caller may do `access` on it.
///
/// The owner always may. A session admin may as [`AdminAccess`] says. A
/// sync token never acts as staff. Every other case gets one answer that
/// does not depend on the id: 403 `Admin access required` for a session
/// officer who is not an admin, 404 `crop_not_found` for everyone else.
async fn visible_crop(
    state: &AppState,
    member: &scuffed_db::Member,
    staff_allowed: bool,
    id: &str,
    access: AdminAccess,
) -> Result<TrainingCrop, Reject> {
    let role = member.org_role;
    let is_admin = staff_allowed && role == OrgRole::Admin;
    let refuse = || {
        if staff_allowed && !is_admin && role.is_at_least(OrgRole::Officer) {
            reject(StatusCode::FORBIDDEN, ADMIN_REQUIRED)
        } else {
            not_found()
        }
    };
    if !is_icon_id(id) {
        return Err(refuse());
    }
    let Some(crop) = state.db.get_training_crop(id).await.map_err(db_error)? else {
        return Err(refuse());
    };
    if crop.member_id == member.id {
        return Ok(crop);
    }
    if !is_admin {
        return Err(refuse());
    }
    if access == AdminAccess::Read {
        let consent = state
            .db
            .get_training_crop_consent(&crop.member_id)
            .await
            .map_err(db_error)?;
        if !consent.get(crop.kind) {
            return Err(refuse());
        }
    }
    if member_active(state, &crop.member_id).await? {
        Ok(crop)
    } else {
        Err(refuse())
    }
}

/// GET /api/training-crops/{id}/image (session only)
pub async fn crop_image(
    State(state): State<AppState>,
    member: OrgMember,
    UrlPath(id): UrlPath<String>,
) -> Result<Response, Reject> {
    configured(&state)?;
    let crop = visible_crop(&state, &member.member, true, &id, AdminAccess::Read).await?;
    let path = state.hero_icons.icon_path(&crop.id);
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("training crop row has no file");
            return Err(not_found());
        }
        Err(error) => {
            tracing::error!(%error, "training crop file read failed");
            return Err(internal());
        }
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

/// DELETE /api/training-crops/{id}
///
/// Session or sync token. The owner, or with a session an admin while the
/// owner is active (consent or not; audited as `deleted_hero_icon` for a hero
/// icon, `deleted_training_crop` for any other kind). With a sync token another
/// member's crop is 404 `crop_not_found` whatever the role. The file goes
/// first, so a failed file delete keeps the row for a retry.
pub async fn delete_crop(
    State(state): State<AppState>,
    uploader: CropUploader,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<TrainingCropsDeleted>, Reject> {
    configured(&state)?;
    let staff_allowed = uploader.is_session();
    let crop = visible_crop(
        &state,
        &uploader.member,
        staff_allowed,
        &id,
        AdminAccess::Delete,
    )
    .await?;
    let own = crop.member_id == uploader.member.id;
    if let Err(error) = remove_icon_file(&state, &crop.id).await {
        tracing::error!(%error, "training crop file delete failed");
        return Err(internal());
    }
    let removed = state
        .db
        .delete_training_crop(&crop.id)
        .await
        .map_err(db_error)?;
    if !own && removed.is_some() {
        let details = format!("crop {} kind {}", crop.id, crop.kind.as_str());
        audit(
            &state.db,
            &uploader.member.id,
            delete_action(crop.kind),
            AuditTargetType::Member,
            &crop.member_id,
            Some(details.as_str()),
        )
        .await;
    }
    Ok(Json(TrainingCropsDeleted {
        deleted: u32::from(removed.is_some()),
    }))
}

/// GET /api/training-crops?kind=&cursor=&limit= (admin)
///
/// Every stored crop of active members, of every kind or of one kind with
/// `?kind=`, newest first, so an admin can find any crop's id and delete it
/// with `DELETE /api/training-crops/{id}`. A crop of a kind the owner does
/// not consent to right now is `hidden`: no hero, tracker version or
/// metadata, and its image stays 404. `uploaded_at` is cut to the hour.
///
/// One audit entry per request, target `all`: `viewed_hero_icons` when the
/// list is filtered to `hero_icon`, `viewed_training_crops` otherwise, with
/// the count and the filter in the details.
pub async fn list_training_crops(
    State(state): State<AppState>,
    admin: AdminUser,
    Query(filter): Query<CropKindQuery>,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<CursorResponse<StaffTrainingCrop>>, Reject> {
    configured(&state)?;
    let only = parse_kind_filter(filter.kind.as_deref())?;
    let (limit, offset) = pagination.resolve();
    // Members with stored crops, cut down to active ones. A visibility
    // filter: a failed lookup is a 500, never an unfiltered list.
    let counts = state.db.training_crop_counts().await.map_err(db_error)?;
    let mut with_crops: Vec<String> = counts.into_iter().map(|row| row.member_id).collect();
    with_crops.sort();
    with_crops.dedup();
    let names = state
        .db
        .active_member_display_names(&with_crops)
        .await
        .map_err(db_error)?;
    let consents = state.db.training_crop_consents().await.map_err(db_error)?;
    let active: Vec<String> = names.keys().cloned().collect();
    let groups: Vec<(CropKind, Vec<String>)> = stored_kinds()
        .filter(|kind| only.is_none_or(|only| only == *kind))
        .map(|kind| (kind, active.clone()))
        .collect();
    let rows = state
        .db
        .list_training_crops_visible(&groups, limit, offset)
        .await
        .map_err(db_error)?;
    let items: Vec<StaffTrainingCrop> = rows
        .iter()
        .map(|crop| {
            let consented = consents
                .get(&crop.member_id)
                .is_some_and(|consent| consent.get(crop.kind));
            let name = names.get(&crop.member_id).cloned().unwrap_or_default();
            staff_item(crop, name, !consented)
        })
        .collect();
    let page = CursorResponse::from_oversized(items, limit, offset);
    let scope = only.map_or("all kinds", CropKind::as_str);
    let details = format!("{} training crops, {scope}", page.data.len());
    audit(
        &state.db,
        &admin.member.id,
        list_action(only),
        AuditTargetType::Member,
        "all",
        Some(details.as_str()),
    )
    .await;
    Ok(Json(page))
}

/// GET /api/training-crops/members (officer or admin)
///
/// Counts only: no ids, images or metadata. Active members, sorted by display
/// name (case folded), then member id, paged in memory.
///
/// - Officers get, per member, the kinds that member consents to (with a
///   zero count when nothing is stored), and only members who consent to at
///   least one kind.
/// - Admins also get kinds with consent off that still have stored crops,
///   with `consented` false, since admins may delete those.
///
/// `resolutions` breaks the listed kinds down by game resolution.
pub async fn member_counts(
    State(state): State<AppState>,
    staff: OfficerUser,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<CursorResponse<TrainingCropMemberCounts>>, Reject> {
    configured(&state)?;
    let (limit, offset) = pagination.resolve();
    let is_admin = staff.member.org_role == OrgRole::Admin;
    let rows = state.db.training_crop_counts().await.map_err(db_error)?;
    let consents = state.db.training_crop_consents().await.map_err(db_error)?;

    // member -> kind -> (width, height) -> count
    type ByResolution = BTreeMap<(Option<u32>, Option<u32>), u64>;
    let mut stored: HashMap<String, BTreeMap<CropKind, ByResolution>> = HashMap::new();
    for row in rows {
        *stored
            .entry(row.member_id)
            .or_default()
            .entry(row.kind)
            .or_default()
            .entry((row.screen_width, row.screen_height))
            .or_default() += row.count;
    }
    let mut candidates: Vec<String> = consents.keys().cloned().collect();
    if is_admin {
        candidates.extend(
            stored
                .keys()
                .filter(|id| !consents.contains_key(*id))
                .cloned(),
        );
    }
    // A visibility filter: a failed lookup is a 500, never an unfiltered list.
    let active = state
        .db
        .active_member_display_names(&candidates)
        .await
        .map_err(db_error)?;

    let empty = BTreeMap::new();
    let mut members: Vec<TrainingCropMemberCounts> = Vec::new();
    for (member_id, display_name) in active {
        let consent = consents.get(&member_id).copied().unwrap_or_default();
        let by_kind = stored.get(&member_id).unwrap_or(&empty);
        let mut kinds = Vec::new();
        let mut resolutions = Vec::new();
        for kind in stored_kinds() {
            let consented = consent.get(kind);
            let per_resolution = by_kind.get(&kind);
            let count: u64 = per_resolution.map(|r| r.values().sum()).unwrap_or(0);
            let listed = consented || (is_admin && count > 0);
            if !listed {
                continue;
            }
            kinds.push(TrainingCropKindCount {
                kind,
                count: count_u32(count),
                consented,
                max: kind.max_per_member(),
            });
            for ((width, height), n) in per_resolution.into_iter().flatten() {
                resolutions.push(TrainingCropResolutionCount {
                    kind,
                    screen_width: *width,
                    screen_height: *height,
                    count: count_u32(*n),
                });
            }
        }
        if kinds.is_empty() {
            continue;
        }
        resolutions.sort_by(|a, b| {
            b.count.cmp(&a.count).then_with(|| {
                (a.kind, a.screen_width, a.screen_height).cmp(&(
                    b.kind,
                    b.screen_width,
                    b.screen_height,
                ))
            })
        });
        let total = kinds.iter().map(|k| u64::from(k.count)).sum();
        members.push(TrainingCropMemberCounts {
            member_id,
            display_name,
            total: count_u32(total),
            kinds,
            resolutions,
        });
    }
    members.sort_by_key(|row| (row.display_name.to_lowercase(), row.member_id.clone()));
    let page: Vec<TrainingCropMemberCounts> = members
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize + 1)
        .collect();
    Ok(Json(CursorResponse::from_oversized(page, limit, offset)))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{must_err, must_ok};

    fn code(result: Result<(Option<&'static str>, TrainingCropMeta), Reject>) -> String {
        match result {
            Ok(_) => "ok".to_string(),
            Err((status, body)) => format!("{} {}", status.as_u16(), body.error),
        }
    }

    #[test]
    fn kind_parsing() {
        assert_eq!(
            must_ok(parse_kind(Some("map_name")), "kind"),
            CropKind::MapName
        );
        let (status, body) = must_err(parse_kind(None), "no kind");
        assert_eq!(status, StatusCode::BAD_REQUEST);
        assert_eq!(body.error, TRAINING_CROP_ERR_UNKNOWN_KIND);
        assert!(parse_kind(Some("replay")).is_err());
        assert_eq!(must_ok(parse_kind_filter(None), "no filter"), None);
        assert!(parse_kind_filter(Some("")).is_err());
    }

    #[test]
    fn meta_needs_a_known_hero_for_hero_icons_only() {
        assert_eq!(
            code(parse_meta(CropKind::HeroIcon, b"{}")),
            "400 unknown_hero"
        );
        let ok = must_ok(
            parse_meta(CropKind::HeroIcon, br#"{"hero":"soldier-76"}"#),
            "hero",
        );
        assert_eq!(ok.0, Some("soldier-76"), "stored as the hero key");
        let ok = must_ok(
            parse_meta(CropKind::HeroIcon, br#"{"hero":"Soldier: 76"}"#),
            "display name",
        );
        assert_eq!(ok.0, Some("soldier-76"));
        let ok = must_ok(
            parse_meta(CropKind::StatDigits, r#"{"hero":"Lúcio"}"#.as_bytes()),
            "accented name",
        );
        assert_eq!(ok.0, Some("lucio"));
        assert_eq!(code(parse_meta(CropKind::StatDigits, b"{}")), "ok");
        assert_eq!(
            code(parse_meta(CropKind::MapName, br#"{"hero":"nobody"}"#)),
            "400 unknown_hero"
        );
    }

    #[test]
    fn bad_meta_is_one_stable_code() {
        for body in [
            &b"not json"[..],
            br#"{"kind":"map_name"}"#,
            br#"{"slot_index":99}"#,
            br#"{"screen_width":1920}"#,
            br#"{"found_confidence":0.5}"#,
            br#"{"score":"high"}"#,
        ] {
            assert_eq!(
                code(parse_meta(CropKind::StatDigits, body)),
                "400 invalid_crop_meta",
                "{}",
                String::from_utf8_lossy(body)
            );
        }
    }

    #[test]
    fn meta_fields_are_kept() {
        let body = br#"{"screen":"scoreboard","slot_type":"enemy","slot_index":2,
            "game_token":"0123456789abcdef0123456789abcdef","score":0.9,"runner_up":0.3,"margin":0.6,
            "icon_set_version":"7","spot_version":"s1","vote_card_filled":false,
            "locked_in":true,"screen_width":1920,"screen_height":1080,"ui_scale":90,
            "colorblind_filter":"off","found_spot":{"x":1,"y":2,"w":3,"h":4},
            "found_confidence":0.4}"#;
        let (_, meta) = must_ok(parse_meta(CropKind::StatDigits, body), "full meta");
        assert_eq!(meta.slot_index, Some(2));
        assert_eq!(meta.ui_scale, Some(90.0));
        assert_eq!(meta.locked_in, Some(true));
        assert_eq!(meta.found_spot.map(|s| s.h), Some(4));
        assert_eq!(meta.colorblind_filter.as_deref(), Some("off"));
    }

    #[test]
    fn bad_game_tokens_are_invalid_meta() {
        for token in [
            "abcdefgh12",
            "3f2a9c0e-6b1d-4e8f-9a7c-5b3d1e0f2a4c",
            "2026-11-10T18:04:05Z",
            "0123456789ABCDEF0123456789ABCDEF",
        ] {
            let body = format!(r#"{{"game_token":"{token}"}}"#);
            assert_eq!(
                code(parse_meta(CropKind::StatDigits, body.as_bytes())),
                "400 invalid_crop_meta",
                "{token}"
            );
        }
    }

    fn ua(value: &str) -> HeaderMap {
        let mut headers = HeaderMap::new();
        headers.insert(header::USER_AGENT, value.parse().unwrap());
        headers
    }

    #[test]
    fn tracker_version_is_capped_for_storage() {
        assert_eq!(
            stored_tracker_version(&ua("scuffed-stat-tracker/0.6.0-alpha.3")).as_deref(),
            Some("0.6.0-alpha.3")
        );
        assert_eq!(stored_tracker_version(&HeaderMap::new()), None);
        assert_eq!(stored_tracker_version(&ua("curl/8.0")), None);
        // Exactly 64 characters is kept, 65 is dropped.
        let pre = "a".repeat(64 - "1.0.0-".len());
        let at_limit = format!("1.0.0-{pre}");
        assert_eq!(at_limit.len(), 64);
        assert_eq!(
            stored_tracker_version(&ua(&format!("scuffed-stat-tracker/{at_limit}"))),
            Some(at_limit.clone())
        );
        let over = format!("{at_limit}a");
        assert_eq!(
            stored_tracker_version(&ua(&format!("scuffed-stat-tracker/{over}"))),
            None
        );
        let long_build = format!("scuffed-stat-tracker/1.0.0+{}", "b".repeat(500));
        assert_eq!(stored_tracker_version(&ua(&long_build)), None);
        // A tab ends the version token; header values cannot hold other
        // control characters, and the check refuses them anyway.
        assert_eq!(
            stored_tracker_version(&ua("scuffed-stat-tracker/1.2.3\textra")).as_deref(),
            Some("1.2.3")
        );
    }

    #[test]
    fn admin_list_times_are_cut_to_the_hour() {
        let at = DateTime::parse_from_rfc3339("2026-11-10T18:04:05.123456Z")
            .unwrap()
            .with_timezone(&Utc);
        let cut = to_the_hour(at);
        assert_eq!(cut.to_rfc3339(), "2026-11-10T18:00:00+00:00");
        assert_eq!(to_the_hour(cut), cut);
    }

    #[test]
    fn hero_icon_audit_names_are_kept_and_other_kinds_get_their_own() {
        assert_eq!(
            delete_action(CropKind::HeroIcon).to_string(),
            "deleted_hero_icon"
        );
        for kind in [CropKind::StatDigits, CropKind::MapName] {
            assert_eq!(delete_action(kind).to_string(), "deleted_training_crop");
            assert_eq!(list_action(Some(kind)).to_string(), "viewed_training_crops");
        }
        assert_eq!(
            list_action(Some(CropKind::HeroIcon)).to_string(),
            "viewed_hero_icons"
        );
        assert_eq!(list_action(None).to_string(), "viewed_training_crops");
    }

    fn clean(width: u32, height: u32) -> CleanIcon {
        let canvas = image::RgbaImage::from_pixel(width, height, image::Rgba([9, 9, 9, 255]));
        let mut out = std::io::Cursor::new(Vec::new());
        canvas
            .write_to(&mut out, image::ImageFormat::Png)
            .expect("png");
        clean_png_within(&out.into_inner(), 1024, 256).expect("clean")
    }

    fn new_row(member: &str, kind: CropKind) -> NewTrainingCrop {
        NewTrainingCrop {
            id: String::new(),
            member_id: member.to_string(),
            kind,
            hero: None,
            file_name: String::new(),
            size_bytes: 0,
            width: 0,
            height: 0,
            created_at: Utc::now(),
            tracker_version: None,
            meta: TrainingCropMeta::default(),
        }
    }

    /// Store `count` crops for one simulated request: file and row each.
    async fn store_request(
        state: &AppState,
        member: &str,
        kind: CropKind,
        count: usize,
    ) -> Vec<TrainingCrop> {
        let mut stored = Vec::new();
        for _ in 0..count {
            let row = new_row(member, kind);
            stored.push(must_ok(
                store_crop(state, row, clean(40, 16)).await,
                "store",
            ));
        }
        stored
    }

    async fn rows(state: &AppState, member: &str) -> Vec<String> {
        let mut ids: Vec<String> = state
            .db
            .list_training_crops_for_member(member, None, None)
            .await
            .unwrap()
            .into_iter()
            .map(|crop| crop.id)
            .collect();
        ids.sort();
        ids
    }

    fn file_exists(state: &AppState, id: &str) -> bool {
        state.hero_icons.icon_path(id).exists()
    }

    /// Two uploads both passed the first count at cap - 1 and both stored a
    /// crop, so the member is one over the cap. The first to recheck loses:
    /// its row and file go, the other request keeps its crop, and the
    /// member ends exactly at the cap.
    #[tokio::test]
    async fn racing_uploads_past_the_cap_clean_up_the_loser() {
        let state = crate::test_support::test_state().await;
        ensure_hero_icons_dir(state.hero_icons.dir()).await.unwrap();
        let member = "racer";
        let kind = CropKind::MapName;
        state
            .db
            .set_training_crop_consent(
                member,
                TrainingCropConsentUpdate {
                    map_name: Some(true),
                    ..Default::default()
                },
            )
            .await
            .unwrap();
        let earlier =
            store_request(&state, member, kind, (kind.max_per_member() - 1) as usize).await;
        let first = store_request(&state, member, kind, 1).await;
        let second = store_request(&state, member, kind, 1).await;
        let cap = u64::from(kind.max_per_member());
        let count = state
            .db
            .count_training_crops_for_member(member, kind)
            .await
            .unwrap();
        assert_eq!(count, cap + 1, "both requests got past the first count");

        let (status, body) = must_err(
            recheck_crops_after_store(&state, member, kind, &first).await,
            "loser",
        );
        assert_eq!(status, StatusCode::CONFLICT);
        assert_eq!(body.error, TRAINING_CROP_ERR_LIMIT_REACHED);
        assert!(
            state
                .db
                .get_training_crop(&first[0].id)
                .await
                .unwrap()
                .is_none()
        );
        assert!(!file_exists(&state, &first[0].id));

        must_ok(
            recheck_crops_after_store(&state, member, kind, &second).await,
            "winner",
        );
        assert!(file_exists(&state, &second[0].id));
        let stored = rows(&state, member).await;
        assert_eq!(stored.len() as u64, cap);
        assert!(stored.contains(&second[0].id));
        for crop in &earlier {
            assert!(file_exists(&state, &crop.id), "earlier crops are kept");
        }
        let _ = std::fs::remove_dir_all(state.hero_icons.dir());
    }

    /// Consent goes off while a request is storing its crops: the recheck
    /// removes this request's rows and files and answers 403. Crops stored
    /// by an earlier request stay (hidden, like any opt-out).
    #[tokio::test]
    async fn consent_withdrawn_mid_upload_removes_this_requests_crops() {
        let state = crate::test_support::test_state().await;
        ensure_hero_icons_dir(state.hero_icons.dir()).await.unwrap();
        let member = "withdrawer";
        let kind = CropKind::StatDigits;
        let on = TrainingCropConsentUpdate {
            stat_digits: Some(true),
            ..Default::default()
        };
        state
            .db
            .set_training_crop_consent(member, on)
            .await
            .unwrap();
        let earlier = store_request(&state, member, kind, 1).await;
        must_ok(
            recheck_crops_after_store(&state, member, kind, &earlier).await,
            "consented",
        );

        let in_flight = store_request(&state, member, kind, 3).await;
        let off = TrainingCropConsentUpdate {
            stat_digits: Some(false),
            ..Default::default()
        };
        state
            .db
            .set_training_crop_consent(member, off)
            .await
            .unwrap();
        let (status, body) = must_err(
            recheck_crops_after_store(&state, member, kind, &in_flight).await,
            "withdrawn",
        );
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.error, TRAINING_CROP_ERR_CONSENT_REQUIRED);
        for crop in &in_flight {
            assert!(
                state
                    .db
                    .get_training_crop(&crop.id)
                    .await
                    .unwrap()
                    .is_none()
            );
            assert!(!file_exists(&state, &crop.id));
        }
        assert_eq!(rows(&state, member).await, vec![earlier[0].id.clone()]);
        assert!(file_exists(&state, &earlier[0].id));
        let _ = std::fs::remove_dir_all(state.hero_icons.dir());
    }
}
