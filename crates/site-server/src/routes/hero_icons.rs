//! Contributed hero icon routes (`/api/hero-icons`).
//!
//! Members contribute PNG hero icons so the stat tracker can build
//! icon-matching templates. Access rules:
//!
//! - Every route needs a session. A daemon sync token is not a session.
//! - Upload needs the caller's own opt-in (`icon_opt_in_required` otherwise).
//!   The caller's own icons are always listed and deletable, opted in or not.
//! - `GET /api/hero-icons/{id}/image` serves the owner, and an officer only
//!   while the owner is opted in. Anyone else, an unknown id and a malformed
//!   id all get the same 404 `icon_not_found`.
//! - `DELETE /api/hero-icons/{id}` is the owner or any officer. Anyone else
//!   gets the same 404.
//! - Officer lists only carry icons of members who are opted in right now.
//!   `GET /api/hero-icons/members/{member_id}` is 403 `icon_opt_in_off` for a
//!   member who is not.
//!
//! Files sit in the private `HERO_ICONS_DIR` as `<id>.png`. The id is made by
//! the server. Ids from a URL must pass [`is_icon_id`] before any database or
//! file call. See `docs/api/hero-icons.md`.

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path;

use axum::Json;
use axum::body::Body;
use axum::extract::multipart::{Field, MultipartError};
use axum::extract::{Multipart, Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use image::ImageFormat;
use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType, DbError, HeroIcon, NewHeroIcon, OrgRole};
use scuffed_types::api::{
    CursorResponse, HERO_ICON_ERR_BAD_DIMENSIONS, HERO_ICON_ERR_DISABLED,
    HERO_ICON_ERR_INVALID_MULTIPART, HERO_ICON_ERR_INVALID_PNG, HERO_ICON_ERR_LIMIT_REACHED,
    HERO_ICON_ERR_NO_ICONS, HERO_ICON_ERR_NOT_FOUND, HERO_ICON_ERR_NOT_PNG,
    HERO_ICON_ERR_OPT_IN_OFF, HERO_ICON_ERR_OPT_IN_REQUIRED, HERO_ICON_ERR_TOO_LARGE,
    HERO_ICON_ERR_TOO_MANY_IN_REQUEST, HERO_ICON_ERR_UNEXPECTED_FIELD, HERO_ICON_ERR_UNKNOWN_HERO,
    HeroIconEntry, HeroIconMemberCount, HeroIconMemberCounts, HeroIconOptIn, HeroIconUploadResponse,
    HeroIconsDeleted, MyHeroIcons, OfficerHeroIcon, OfficerHeroIconList, PaginationParams,
};
use scuffed_types::{
    HERO_ICON_MAX_BYTES, HERO_ICON_MAX_EDGE, HERO_ICON_MAX_FILES_PER_UPLOAD,
    HERO_ICON_MAX_PER_MEMBER, HERO_ICON_MIN_EDGE, hero_key_to_name,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::extractors::{OfficerUser, OrgMember};
use crate::hero_icons::{ensure_hero_icons_dir, is_icon_id};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// The 8-byte PNG file signature.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

type Reject = (StatusCode, Json<ErrorResponse>);

fn reject(status: StatusCode, error_code: &str) -> Reject {
    (
        status,
        Json(ErrorResponse {
            error: error_code.into(),
        }),
    )
}

fn internal() -> Reject {
    reject(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
}

fn not_found() -> Reject {
    reject(StatusCode::NOT_FOUND, HERO_ICON_ERR_NOT_FOUND)
}

fn db_error(error: DbError) -> Reject {
    tracing::error!(%error, "hero icon database call failed");
    internal()
}

/// 503 `hero_icons_disabled` when `HERO_ICONS_DIR` could not be used at startup.
fn configured(state: &AppState) -> Result<(), Reject> {
    if !state.hero_icons.enabled() {
        return Err(reject(StatusCode::SERVICE_UNAVAILABLE, HERO_ICON_ERR_DISABLED));
    }
    Ok(())
}

fn count_u32(count: usize) -> u32 {
    u32::try_from(count).unwrap_or(u32::MAX)
}

/// Upload failures: the usual `{"error": code}` body, or the shared JSON 429
/// from [`crate::rate_limit::rate_limited_response`].
pub enum UploadReject {
    Status(StatusCode, Json<ErrorResponse>),
    Limited(Box<Response>),
}

impl From<Reject> for UploadReject {
    fn from(value: Reject) -> Self {
        Self::Status(value.0, value.1)
    }
}

impl IntoResponse for UploadReject {
    fn into_response(self) -> Response {
        match self {
            Self::Status(status, body) => (status, body).into_response(),
            Self::Limited(response) => *response,
        }
    }
}

fn thumbnail_url(id: &str) -> String {
    format!("/api/hero-icons/{id}/image")
}

fn entry(icon: &HeroIcon) -> HeroIconEntry {
    HeroIconEntry {
        id: icon.id.clone(),
        hero: icon.hero.clone(),
        size_bytes: icon.size_bytes,
        width: icon.width,
        height: icon.height,
        uploaded_at: icon.created_at,
        thumbnail_url: thumbnail_url(&icon.id),
    }
}

fn officer_entry(icon: &HeroIcon, names: &HashMap<String, String>) -> OfficerHeroIcon {
    OfficerHeroIcon {
        id: icon.id.clone(),
        member_id: icon.member_id.clone(),
        display_name: names.get(&icon.member_id).cloned().unwrap_or_default(),
        hero: icon.hero.clone(),
        size_bytes: icon.size_bytes,
        width: icon.width,
        height: icon.height,
        uploaded_at: icon.created_at,
        thumbnail_url: thumbnail_url(&icon.id),
    }
}

/// Display names for officer lists. A failed lookup leaves names empty.
async fn member_names(state: &AppState, member_ids: &[String]) -> HashMap<String, String> {
    match state.db.member_display_names(member_ids).await {
        Ok(names) => names,
        Err(error) => {
            tracing::warn!(%error, "hero icon display name lookup failed");
            HashMap::new()
        }
    }
}

/// Remove one icon file. A file that is already gone is fine.
async fn remove_icon_file(state: &AppState, id: &str) -> std::io::Result<()> {
    if !is_icon_id(id) {
        return Ok(());
    }
    let path = state.hero_icons.icon_path(id);
    match tokio::fs::remove_file(path).await {
        Err(error) if error.kind() != std::io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

/// [`remove_icon_file`] for cleanup paths: a failure is logged, not returned.
async fn discard_icon_file(state: &AppState, id: &str) {
    if let Err(error) = remove_icon_file(state, id).await {
        tracing::error!(%error, "hero icon file cleanup failed");
    }
}

/// Best-effort rollback of rows and files this request stored.
async fn remove_stored(state: &AppState, icons: &[HeroIcon]) {
    for icon in icons {
        if let Err(error) = state.db.delete_hero_icon(&icon.id).await {
            tracing::error!(%error, "hero icon rollback delete failed");
        }
        discard_icon_file(state, &icon.id).await;
    }
}

/// Create the icon file with mode 0600. Never overwrites.
async fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut options = tokio::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        // tokio::fs::OpenOptions::mode is inherent on unix. 0600 keeps the
        // icon unreadable by other users on the host.
        options.mode(0o600);
    }
    let mut file = options.open(path).await?;
    use tokio::io::AsyncWriteExt;
    file.write_all(bytes).await?;
    file.sync_all().await?;
    Ok(())
}

/// One icon after the server decoded and re-encoded it.
struct CleanIcon {
    bytes: Vec<u8>,
    width: u32,
    height: u32,
}

fn edge_ok(edge: u32) -> bool {
    (HERO_ICON_MIN_EDGE..=HERO_ICON_MAX_EDGE).contains(&edge)
}

/// Check the header size, decode, and re-encode one PNG. Runs on a blocking thread.
///
/// The stored file is the re-encoded image, so text chunks and other metadata
/// in the upload are dropped. `Err` is the 400 error code.
fn clean_png(data: &[u8]) -> Result<CleanIcon, &'static str> {
    let reader = image::ImageReader::with_format(Cursor::new(data), ImageFormat::Png);
    let Ok((width, height)) = reader.into_dimensions() else {
        return Err(HERO_ICON_ERR_INVALID_PNG);
    };
    if !edge_ok(width) || !edge_ok(height) {
        return Err(HERO_ICON_ERR_BAD_DIMENSIONS);
    }
    let Ok(decoded) = image::load_from_memory_with_format(data, ImageFormat::Png) else {
        return Err(HERO_ICON_ERR_INVALID_PNG);
    };
    let mut out = Cursor::new(Vec::new());
    if decoded.write_to(&mut out, ImageFormat::Png).is_err() {
        return Err(HERO_ICON_ERR_INVALID_PNG);
    }
    Ok(CleanIcon {
        bytes: out.into_inner(),
        width: decoded.width(),
        height: decoded.height(),
    })
}

fn multipart_reject(error: MultipartError) -> Reject {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return reject(StatusCode::PAYLOAD_TOO_LARGE, HERO_ICON_ERR_TOO_LARGE);
    }
    tracing::debug!(%error, "hero icon multipart rejected");
    reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_INVALID_MULTIPART)
}

/// Read one `icon` part. The signature is checked as soon as 8 bytes are in,
/// and reading stops as soon as the part passes [`HERO_ICON_MAX_BYTES`].
async fn read_icon_part(field: &mut Field<'_>) -> Result<Vec<u8>, Reject> {
    let mut data: Vec<u8> = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_reject)? {
        data.extend_from_slice(&chunk);
        if data.len() >= PNG_SIGNATURE.len() && !data.starts_with(&PNG_SIGNATURE) {
            return Err(reject(StatusCode::UNSUPPORTED_MEDIA_TYPE, HERO_ICON_ERR_NOT_PNG));
        }
        if data.len() > HERO_ICON_MAX_BYTES {
            return Err(reject(StatusCode::PAYLOAD_TOO_LARGE, HERO_ICON_ERR_TOO_LARGE));
        }
    }
    if !data.starts_with(&PNG_SIGNATURE) {
        return Err(reject(StatusCode::UNSUPPORTED_MEDIA_TYPE, HERO_ICON_ERR_NOT_PNG));
    }
    Ok(data)
}

/// Write one file, then its row. On a failed insert the file is removed again.
async fn store_icon(
    state: &AppState,
    member_id: &str,
    hero: &str,
    icon: CleanIcon,
    now: DateTime<Utc>,
) -> Result<HeroIcon, Reject> {
    let id = Uuid::new_v4().simple().to_string();
    let path = state.hero_icons.icon_path(&id);
    if let Err(error) = write_private(&path, &icon.bytes).await {
        tracing::error!(%error, "hero icon file write failed");
        // A partial file is ours to remove. An existing one is not.
        if error.kind() != std::io::ErrorKind::AlreadyExists {
            discard_icon_file(state, &id).await;
        }
        return Err(internal());
    }
    let row = NewHeroIcon {
        id: id.clone(),
        member_id: member_id.to_string(),
        hero: hero.to_string(),
        file_name: format!("{id}.png"),
        size_bytes: icon.bytes.len() as u64,
        width: icon.width,
        height: icon.height,
        created_at: now,
    };
    match state.db.insert_hero_icon(&row).await {
        Ok(stored) => Ok(stored),
        Err(error) => {
            tracing::error!(%error, "hero icon insert failed");
            discard_icon_file(state, &id).await;
            Err(internal())
        }
    }
}

#[derive(Debug, Deserialize)]
pub struct HeroIconUploadQuery {
    /// Hero key or display name, for example `ana`, `lucio` or `Soldier: 76`.
    pub hero: Option<String>,
}

/// POST /api/hero-icons?hero=<key>
///
/// Multipart body with one or more parts named `icon`. Order of checks: 503
/// when disabled, the per-member limiter (429), opt-in (403), hero (400), the
/// stored cap (409), then each part (signature 415, size 413, dimensions and
/// decode 400). Nothing is stored unless every part passes.
pub async fn upload_hero_icons(
    State(state): State<AppState>,
    member: OrgMember,
    Query(query): Query<HeroIconUploadQuery>,
    mut multipart: Multipart,
) -> Result<(StatusCode, Json<HeroIconUploadResponse>), UploadReject> {
    configured(&state)?;
    let member_id = member.member.id.clone();
    if let Err(secs) = state.hero_icons.check(&member_id) {
        let limited = crate::rate_limit::rate_limited_response(secs);
        return Err(UploadReject::Limited(Box::new(limited)));
    }
    let opted_in = state
        .db
        .get_hero_icon_opt_in(&member_id)
        .await
        .map_err(db_error)?;
    if !opted_in {
        return Err(reject(StatusCode::FORBIDDEN, HERO_ICON_ERR_OPT_IN_REQUIRED).into());
    }
    let Some(hero) = query.hero.as_deref().and_then(hero_key_to_name) else {
        return Err(reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_UNKNOWN_HERO).into());
    };
    let max_icons = u64::from(HERO_ICON_MAX_PER_MEMBER);
    let stored_before = state
        .db
        .count_hero_icons_for_member(&member_id)
        .await
        .map_err(db_error)?;
    if stored_before >= max_icons {
        return Err(reject(StatusCode::CONFLICT, HERO_ICON_ERR_LIMIT_REACHED).into());
    }

    let mut accepted: Vec<CleanIcon> = Vec::new();
    while let Some(mut field) = multipart.next_field().await.map_err(multipart_reject)? {
        if field.name() != Some("icon") {
            return Err(reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_UNEXPECTED_FIELD).into());
        }
        if accepted.len() >= HERO_ICON_MAX_FILES_PER_UPLOAD {
            let code = HERO_ICON_ERR_TOO_MANY_IN_REQUEST;
            return Err(reject(StatusCode::PAYLOAD_TOO_LARGE, code).into());
        }
        let data = read_icon_part(&mut field).await?;
        let cleaned = tokio::task::spawn_blocking(move || clean_png(&data))
            .await
            .map_err(|error| {
                tracing::error!(%error, "hero icon decode task failed");
                internal()
            })?;
        match cleaned {
            Ok(icon) => accepted.push(icon),
            Err(code) => return Err(reject(StatusCode::BAD_REQUEST, code).into()),
        }
    }
    if accepted.is_empty() {
        return Err(reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_NO_ICONS).into());
    }
    if stored_before + accepted.len() as u64 > max_icons {
        return Err(reject(StatusCode::CONFLICT, HERO_ICON_ERR_LIMIT_REACHED).into());
    }

    let dir = state.hero_icons.dir();
    if tokio::fs::metadata(dir).await.is_err() {
        ensure_hero_icons_dir(dir).await.map_err(|error| {
            tracing::error!(%error, "hero icon directory create failed");
            internal()
        })?;
    }
    let now = Utc::now();
    let mut stored: Vec<HeroIcon> = Vec::with_capacity(accepted.len());
    for icon in accepted {
        match store_icon(&state, &member_id, hero, icon, now).await {
            Ok(row) => stored.push(row),
            Err(error) => {
                remove_stored(&state, &stored).await;
                return Err(error.into());
            }
        }
    }

    // Two uploads can pass the first count together. Recount and roll this
    // request back if the pair went over the cap.
    let stored_after = match state.db.count_hero_icons_for_member(&member_id).await {
        Ok(count) => count,
        Err(error) => {
            remove_stored(&state, &stored).await;
            return Err(db_error(error).into());
        }
    };
    if stored_after > max_icons {
        remove_stored(&state, &stored).await;
        return Err(reject(StatusCode::CONFLICT, HERO_ICON_ERR_LIMIT_REACHED).into());
    }

    let icons: Vec<HeroIconEntry> = stored.iter().map(entry).collect();
    Ok((StatusCode::CREATED, Json(HeroIconUploadResponse { icons })))
}

/// GET /api/hero-icons/mine
pub async fn list_mine(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<MyHeroIcons>, Reject> {
    configured(&state)?;
    let member_id = &member.member.id;
    let opted_in = state
        .db
        .get_hero_icon_opt_in(member_id)
        .await
        .map_err(db_error)?;
    let rows = state
        .db
        .list_hero_icons_for_member(member_id)
        .await
        .map_err(db_error)?;
    let icons: Vec<HeroIconEntry> = rows.iter().map(entry).collect();
    Ok(Json(MyHeroIcons {
        icon_contribution_opt_in: opted_in,
        count: count_u32(icons.len()),
        max_icons: HERO_ICON_MAX_PER_MEMBER,
        icons,
    }))
}

/// DELETE /api/hero-icons/mine
///
/// Removes every row and file of the caller. Opt-in is left as it is.
pub async fn delete_mine(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<HeroIconsDeleted>, Reject> {
    configured(&state)?;
    let removed = state
        .db
        .delete_hero_icons_for_member(&member.member.id)
        .await
        .map_err(db_error)?;
    for icon in &removed {
        discard_icon_file(&state, &icon.id).await;
    }
    Ok(Json(HeroIconsDeleted {
        deleted: count_u32(removed.len()),
    }))
}

/// GET /api/hero-icons/opt-in
pub async fn get_opt_in(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<HeroIconOptIn>, Reject> {
    configured(&state)?;
    let opted_in = state
        .db
        .get_hero_icon_opt_in(&member.member.id)
        .await
        .map_err(db_error)?;
    Ok(Json(HeroIconOptIn {
        icon_contribution_opt_in: opted_in,
    }))
}

/// PUT /api/hero-icons/opt-in
///
/// Turning it off keeps the stored icons. Officer reads stop at once.
pub async fn put_opt_in(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<HeroIconOptIn>,
) -> Result<Json<HeroIconOptIn>, Reject> {
    configured(&state)?;
    let opted_in = state
        .db
        .set_hero_icon_opt_in(&member.member.id, body.icon_contribution_opt_in)
        .await
        .map_err(db_error)?;
    Ok(Json(HeroIconOptIn {
        icon_contribution_opt_in: opted_in,
    }))
}

/// The icon when `viewer` may see it: the owner, or an officer while the owner
/// is opted in. Everything else is the same 404.
async fn visible_icon(state: &AppState, viewer: &OrgMember, id: &str) -> Result<HeroIcon, Reject> {
    if !is_icon_id(id) {
        return Err(not_found());
    }
    let Some(icon) = state.db.get_hero_icon(id).await.map_err(db_error)? else {
        return Err(not_found());
    };
    if icon.member_id == viewer.member.id {
        return Ok(icon);
    }
    if !viewer.member.org_role.is_at_least(OrgRole::Officer) {
        return Err(not_found());
    }
    let owner_opted_in = state
        .db
        .get_hero_icon_opt_in(&icon.member_id)
        .await
        .map_err(db_error)?;
    if owner_opted_in {
        Ok(icon)
    } else {
        Err(not_found())
    }
}

/// GET /api/hero-icons/{id}/image
pub async fn hero_icon_image(
    State(state): State<AppState>,
    member: OrgMember,
    UrlPath(id): UrlPath<String>,
) -> Result<Response, Reject> {
    configured(&state)?;
    let icon = visible_icon(&state, &member, &id).await?;
    let path = state.hero_icons.icon_path(&icon.id);
    let bytes = match tokio::fs::read(path).await {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            tracing::warn!("hero icon row has no file");
            return Err(not_found());
        }
        Err(error) => {
            tracing::error!(%error, "hero icon file read failed");
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

/// DELETE /api/hero-icons/{id}
///
/// The owner, or any officer even when the owner is opted out. The file goes
/// first, so a failed file delete keeps the row for a retry.
pub async fn delete_hero_icon(
    State(state): State<AppState>,
    member: OrgMember,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<HeroIconsDeleted>, Reject> {
    configured(&state)?;
    if !is_icon_id(&id) {
        return Err(not_found());
    }
    let Some(icon) = state.db.get_hero_icon(&id).await.map_err(db_error)? else {
        return Err(not_found());
    };
    let own = icon.member_id == member.member.id;
    if !own && !member.member.org_role.is_at_least(OrgRole::Officer) {
        return Err(not_found());
    }
    if let Err(error) = remove_icon_file(&state, &icon.id).await {
        tracing::error!(%error, "hero icon file delete failed");
        return Err(internal());
    }
    let removed = state.db.delete_hero_icon(&id).await.map_err(db_error)?;
    if !own {
        audit(
            &state.db,
            &member.member.id,
            AuditAction::DeletedHeroIcon,
            AuditTargetType::HeroIcon,
            &icon.id,
            Some(icon.member_id.as_str()),
        )
        .await;
    }
    Ok(Json(HeroIconsDeleted {
        deleted: u32::from(removed.is_some()),
    }))
}

/// GET /api/hero-icons (officer)
///
/// Icons of members who are opted in right now, newest first.
pub async fn list_hero_icons(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<CursorResponse<OfficerHeroIcon>>, Reject> {
    configured(&state)?;
    let (limit, offset) = pagination.resolve();
    let opted_in = state
        .db
        .hero_icon_opted_in_member_ids()
        .await
        .map_err(db_error)?;
    let rows = state
        .db
        .list_hero_icons_for_members(&opted_in, limit, offset)
        .await
        .map_err(db_error)?;
    let ids: Vec<String> = rows.iter().map(|icon| icon.member_id.clone()).collect();
    let display_names = member_names(&state, &ids).await;
    let items: Vec<OfficerHeroIcon> = rows
        .iter()
        .map(|icon| officer_entry(icon, &display_names))
        .collect();
    Ok(Json(CursorResponse::from_oversized(items, limit, offset)))
}

/// GET /api/hero-icons/members (officer)
///
/// Every member with stored icons or an opt-in. Counts include icons of
/// members who opted out, but those icons are not readable.
pub async fn hero_icon_member_counts(
    State(state): State<AppState>,
    _officer: OfficerUser,
) -> Result<Json<HeroIconMemberCounts>, Reject> {
    configured(&state)?;
    let counts = state.db.hero_icon_counts().await.map_err(db_error)?;
    let opted_in: HashSet<String> = state
        .db
        .hero_icon_opted_in_member_ids()
        .await
        .map_err(db_error)?
        .into_iter()
        .collect();
    let mut ids: Vec<String> = counts.keys().cloned().collect();
    for id in &opted_in {
        if !counts.contains_key(id) {
            ids.push(id.clone());
        }
    }
    let names = member_names(&state, &ids).await;
    let mut members: Vec<HeroIconMemberCount> = Vec::with_capacity(ids.len());
    for member_id in ids {
        let icon_count = counts.get(&member_id).copied().unwrap_or(0);
        members.push(HeroIconMemberCount {
            display_name: names.get(&member_id).cloned().unwrap_or_default(),
            icon_count: u32::try_from(icon_count).unwrap_or(u32::MAX),
            icon_contribution_opt_in: opted_in.contains(&member_id),
            member_id,
        });
    }
    members.sort_by_key(|row| (row.display_name.to_lowercase(), row.member_id.clone()));
    Ok(Json(HeroIconMemberCounts { members }))
}

/// GET /api/hero-icons/members/{member_id} (officer)
///
/// 403 `icon_opt_in_off` while that member is not opted in.
pub async fn member_hero_icons(
    State(state): State<AppState>,
    _officer: OfficerUser,
    UrlPath(member_id): UrlPath<String>,
) -> Result<Json<OfficerHeroIconList>, Reject> {
    configured(&state)?;
    let opted_in = state
        .db
        .get_hero_icon_opt_in(&member_id)
        .await
        .map_err(db_error)?;
    if !opted_in {
        return Err(reject(StatusCode::FORBIDDEN, HERO_ICON_ERR_OPT_IN_OFF));
    }
    let rows = state
        .db
        .list_hero_icons_for_member(&member_id)
        .await
        .map_err(db_error)?;
    let display_names = member_names(&state, std::slice::from_ref(&member_id)).await;
    let icons: Vec<OfficerHeroIcon> = rows
        .iter()
        .map(|icon| officer_entry(icon, &display_names))
        .collect();
    Ok(Json(OfficerHeroIconList { icons }))
}
