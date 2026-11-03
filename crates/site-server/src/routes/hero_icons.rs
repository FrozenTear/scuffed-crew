//! Contributed hero icon routes (`/api/hero-icons`).
//!
//! Members contribute PNG hero icons so the stat tracker can build
//! icon-matching templates. Access rules:
//!
//! - Every route needs a session. A daemon sync token is not a session.
//! - Upload needs the caller's own opt-in (`icon_opt_in_required` otherwise).
//!   The caller's own icons are always listed, readable and deletable, opted
//!   in or not.
//! - Only an admin reads, lists or deletes another member's icons. Officers
//!   get the per-member counts (`GET /api/hero-icons/members`) and nothing
//!   else: no images, no icon lists, no deletes.
//! - Opt-out keeps the icons and hides them. An admin reads or lists an icon
//!   only while its owner is active and opted in, and counts list only those
//!   members. Deletes are different: an admin deletes an active member's
//!   icons opted in or not, so hidden icons can still be removed.
//! - `GET /api/hero-icons/{id}/image` serves the owner, and an admin while the
//!   owner is active and opted in. `DELETE /api/hero-icons/{id}` serves the
//!   owner, and an admin while the owner is active. For any other id, an
//!   officer gets 403 `Admin access required` and everyone else gets 404
//!   `icon_not_found`. Unknown and malformed ids get the same answer, so
//!   neither leaks whether an icon exists.
//! - `GET /api/hero-icons` and `GET` and `DELETE
//!   /api/hero-icons/members/{member_id}` are admin only. The per-member routes
//!   are 404 `member_not_found` for an unknown or inactive member. The `GET`
//!   is 403 `icon_opt_in_off` for a member who is not opted in. The `DELETE`
//!   works either way.
//! - Admin reads of icon lists and admin deletes are written to the audit log.
//!   Single thumbnail fetches are not, since one list page loads many of them.
//! - Opt-in `GET` and `PUT` only touch the flag and work while the store is
//!   disabled. Every other route answers 503 then.
//!
//! Files sit in the private `HERO_ICONS_DIR` as `<id>.png`. The id is made by
//! the server. Ids from a URL must pass [`is_icon_id`] before any database or
//! file call. See `docs/api/hero-icons.md`.

use std::collections::HashMap;
use std::io::Cursor;
use std::path::Path;

use axum::Json;
use axum::body::Body;
use axum::extract::multipart::{Field, MultipartError, MultipartRejection};
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
    HERO_ICON_ERR_MEMBER_NOT_FOUND, HERO_ICON_ERR_NO_ICONS, HERO_ICON_ERR_NOT_FOUND,
    HERO_ICON_ERR_NOT_PNG, HERO_ICON_ERR_OPT_IN_OFF, HERO_ICON_ERR_OPT_IN_REQUIRED,
    HERO_ICON_ERR_TOO_LARGE, HERO_ICON_ERR_TOO_MANY_IN_REQUEST, HERO_ICON_ERR_UNEXPECTED_FIELD,
    HERO_ICON_ERR_UNKNOWN_HERO, HeroIconEntry, HeroIconMemberCount, HeroIconOptIn,
    HeroIconUploadResponse, HeroIconsDeleted, MyHeroIcons, OfficerHeroIcon, OfficerHeroIconList,
    PaginationParams,
};
use scuffed_types::{
    HERO_ICON_MAX_BYTES, HERO_ICON_MAX_EDGE, HERO_ICON_MAX_FILES_PER_UPLOAD,
    HERO_ICON_MAX_PER_MEMBER, HERO_ICON_MIN_EDGE, hero_key_to_name,
};
use serde::Deserialize;
use uuid::Uuid;

use crate::extractors::{AdminUser, OfficerUser, OrgMember};
use crate::hero_icons::{ensure_hero_icons_dir, is_icon_id};
use crate::routes::audit_log::audit;
use crate::state::AppState;

/// The 8-byte PNG file signature.
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0D, 0x0A, 0x1A, 0x0A];

type Reject = (StatusCode, Json<ErrorResponse>);

/// The 403 body of the `AdminUser` extractor. The per-icon routes give it to
/// officers so Site handles one string for every admin-only refusal.
const ADMIN_REQUIRED: &str = "Admin access required";

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
        return Err(reject(
            StatusCode::SERVICE_UNAVAILABLE,
            HERO_ICON_ERR_DISABLED,
        ));
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

/// Members whose icons staff may see: active and opted in right now.
///
/// Admins list and read icons of these members only. Officer counts list
/// these members only.
///
/// Maps member id to display name. This is a visibility filter, so a failed
/// lookup is a 500, never an empty or unfiltered answer.
async fn visible_members(state: &AppState) -> Result<HashMap<String, String>, Reject> {
    let opted_in = state
        .db
        .hero_icon_opted_in_member_ids()
        .await
        .map_err(db_error)?;
    state
        .db
        .active_member_display_names(&opted_in)
        .await
        .map_err(db_error)
}

/// What an admin wants to do with another member's icons.
#[derive(Clone, Copy, PartialEq, Eq)]
enum AdminAccess {
    /// List or read: the member must be active and opted in.
    Read,
    /// Delete: the member must be active. Opt-in does not matter, so an admin
    /// can remove the hidden icons of an opted-out member.
    Delete,
}

/// True when `member_id` is an active member.
async fn member_active(state: &AppState, member_id: &str) -> Result<bool, Reject> {
    let active = state
        .db
        .active_member_display_names(&[member_id.to_string()])
        .await
        .map_err(db_error)?;
    Ok(active.contains_key(member_id))
}

/// True when an admin may do `access` on `member_id`'s icons.
async fn admin_may(state: &AppState, member_id: &str, access: AdminAccess) -> Result<bool, Reject> {
    if access == AdminAccess::Read {
        let opted_in = state
            .db
            .get_hero_icon_opt_in(member_id)
            .await
            .map_err(db_error)?;
        if !opted_in {
            return Ok(false);
        }
    }
    member_active(state, member_id).await
}

/// Checks for the admin per-member routes when the target is not the caller.
///
/// 404 `member_not_found` for an unknown or inactive member. For
/// [`AdminAccess::Read`], then 403 `icon_opt_in_off` while that member is not
/// opted in. Returns the display name.
async fn admin_target(
    state: &AppState,
    member_id: &str,
    access: AdminAccess,
) -> Result<String, Reject> {
    let names = state
        .db
        .active_member_display_names(&[member_id.to_string()])
        .await
        .map_err(db_error)?;
    let Some(name) = names.get(member_id) else {
        return Err(reject(
            StatusCode::NOT_FOUND,
            HERO_ICON_ERR_MEMBER_NOT_FOUND,
        ));
    };
    if access == AdminAccess::Read {
        let opted_in = state
            .db
            .get_hero_icon_opt_in(member_id)
            .await
            .map_err(db_error)?;
        if !opted_in {
            return Err(reject(StatusCode::FORBIDDEN, HERO_ICON_ERR_OPT_IN_OFF));
        }
    }
    Ok(name.clone())
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

/// Remove every icon of `member_id` and return how many rows went.
///
/// Each file goes before its row, as in a single delete. A file that cannot be
/// removed keeps its row, so the icon stays listed and the call can be
/// retried. The other icons are still removed, and the result is a 500.
/// Opt-in is not checked: the owner and admins delete hidden icons too.
async fn delete_all_icons(state: &AppState, member_id: &str) -> Result<u32, Reject> {
    let rows = state
        .db
        .list_hero_icons_for_member(member_id)
        .await
        .map_err(db_error)?;
    let mut deleted: u32 = 0;
    let mut file_failed = false;
    for icon in &rows {
        if let Err(error) = remove_icon_file(state, &icon.id).await {
            tracing::error!(%error, "hero icon file delete failed");
            file_failed = true;
            continue;
        }
        match state.db.delete_hero_icon(&icon.id).await {
            Ok(Some(_)) => deleted += 1,
            Ok(None) => {}
            Err(error) => return Err(db_error(error)),
        }
    }
    if file_failed {
        return Err(internal());
    }
    Ok(deleted)
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
            return Err(reject(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                HERO_ICON_ERR_NOT_PNG,
            ));
        }
        if data.len() > HERO_ICON_MAX_BYTES {
            return Err(reject(
                StatusCode::PAYLOAD_TOO_LARGE,
                HERO_ICON_ERR_TOO_LARGE,
            ));
        }
    }
    if !data.starts_with(&PNG_SIGNATURE) {
        return Err(reject(
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
            HERO_ICON_ERR_NOT_PNG,
        ));
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

/// Checks that run after an upload stored its icons.
///
/// Two uploads can pass the first count together, so the cap is checked
/// again (409). The member may also have opted out while the body was read:
/// a new upload is accepted only while opted in, so this request's icons are
/// removed and the answer is 403 `icon_opt_in_required`. Icons stored by
/// earlier requests are not touched. On any failure, including a database
/// error, the rows and files in `stored` are removed.
pub(crate) async fn recheck_after_store(
    state: &AppState,
    member_id: &str,
    stored: &[HeroIcon],
) -> Result<(), Reject> {
    let max_icons = u64::from(HERO_ICON_MAX_PER_MEMBER);
    let stored_after = match state.db.count_hero_icons_for_member(member_id).await {
        Ok(count) => count,
        Err(error) => {
            remove_stored(state, stored).await;
            return Err(db_error(error));
        }
    };
    if stored_after > max_icons {
        remove_stored(state, stored).await;
        return Err(reject(StatusCode::CONFLICT, HERO_ICON_ERR_LIMIT_REACHED));
    }
    let still_opted_in = match state.db.get_hero_icon_opt_in(member_id).await {
        Ok(value) => value,
        Err(error) => {
            remove_stored(state, stored).await;
            return Err(db_error(error));
        }
    };
    if !still_opted_in {
        remove_stored(state, stored).await;
        return Err(reject(StatusCode::FORBIDDEN, HERO_ICON_ERR_OPT_IN_REQUIRED));
    }
    Ok(())
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
/// decode 400). Nothing is stored unless every part passes. A body that is not
/// multipart is 400 `invalid_multipart` after the stored cap check.
pub async fn upload_hero_icons(
    State(state): State<AppState>,
    member: OrgMember,
    Query(query): Query<HeroIconUploadQuery>,
    multipart: Result<Multipart, MultipartRejection>,
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
    let Ok(mut multipart) = multipart else {
        return Err(reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_INVALID_MULTIPART).into());
    };

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
    // Each part gets its own time, one microsecond after the part before, so
    // lists (newest first) are stable and show the last part first.
    let base = Utc::now();
    let mut stored: Vec<HeroIcon> = Vec::with_capacity(accepted.len());
    for (index, icon) in accepted.into_iter().enumerate() {
        let offset = i64::try_from(index).unwrap_or(i64::MAX);
        let created_at = base + chrono::Duration::microseconds(offset);
        match store_icon(&state, &member_id, hero, icon, created_at).await {
            Ok(row) => stored.push(row),
            Err(error) => {
                remove_stored(&state, &stored).await;
                return Err(error.into());
            }
        }
    }
    recheck_after_store(&state, &member_id, &stored).await?;

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
    let deleted = delete_all_icons(&state, &member.member.id).await?;
    Ok(Json(HeroIconsDeleted { deleted }))
}

/// GET /api/hero-icons/opt-in
///
/// Works while the store is disabled: it only reads the flag.
pub async fn get_opt_in(
    State(state): State<AppState>,
    member: OrgMember,
) -> Result<Json<HeroIconOptIn>, Reject> {
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
/// Turning it off keeps the stored icons and hides them: admins can no
/// longer list, read or delete them, and officer counts leave the member
/// out. Only the member deletes them. Works while the store is disabled: it
/// only flips the flag.
pub async fn put_opt_in(
    State(state): State<AppState>,
    member: OrgMember,
    Json(body): Json<HeroIconOptIn>,
) -> Result<Json<HeroIconOptIn>, Reject> {
    let opted_in = state
        .db
        .set_hero_icon_opt_in(&member.member.id, body.icon_contribution_opt_in)
        .await
        .map_err(db_error)?;
    Ok(Json(HeroIconOptIn {
        icon_contribution_opt_in: opted_in,
    }))
}

/// The icon when `viewer` may do `access` on it: the owner, or an admin as
/// [`admin_may`] allows (read: owner active and opted in; delete: owner
/// active).
///
/// Every other case gets one answer that does not depend on the id: 403
/// `Admin access required` for an officer who is not an admin, 404
/// `icon_not_found` for everyone else. So an officer cannot tell another
/// member's icon from an unknown id.
async fn visible_icon(
    state: &AppState,
    viewer: &OrgMember,
    id: &str,
    access: AdminAccess,
) -> Result<HeroIcon, Reject> {
    let role = viewer.member.org_role;
    let is_admin = role == OrgRole::Admin;
    let refuse = || {
        if !is_admin && role.is_at_least(OrgRole::Officer) {
            reject(StatusCode::FORBIDDEN, ADMIN_REQUIRED)
        } else {
            not_found()
        }
    };
    if !is_icon_id(id) {
        return Err(refuse());
    }
    let Some(icon) = state.db.get_hero_icon(id).await.map_err(db_error)? else {
        return Err(refuse());
    };
    if icon.member_id == viewer.member.id {
        return Ok(icon);
    }
    if is_admin && admin_may(state, &icon.member_id, access).await? {
        Ok(icon)
    } else {
        Err(refuse())
    }
}

/// GET /api/hero-icons/{id}/image
pub async fn hero_icon_image(
    State(state): State<AppState>,
    member: OrgMember,
    UrlPath(id): UrlPath<String>,
) -> Result<Response, Reject> {
    configured(&state)?;
    let icon = visible_icon(&state, &member, &id, AdminAccess::Read).await?;
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
/// The owner, or an admin while the owner is active, opted in or not: an admin
/// can remove an opted-out member's hidden icon, and the delete is audited.
/// Officers get 403 for any icon that is not their own. The file goes first,
/// so a failed file delete keeps the row for a retry.
pub async fn delete_hero_icon(
    State(state): State<AppState>,
    member: OrgMember,
    UrlPath(id): UrlPath<String>,
) -> Result<Json<HeroIconsDeleted>, Reject> {
    configured(&state)?;
    let icon = visible_icon(&state, &member, &id, AdminAccess::Delete).await?;
    let own = icon.member_id == member.member.id;
    if let Err(error) = remove_icon_file(&state, &icon.id).await {
        tracing::error!(%error, "hero icon file delete failed");
        return Err(internal());
    }
    let removed = state
        .db
        .delete_hero_icon(&icon.id)
        .await
        .map_err(db_error)?;
    if !own && removed.is_some() {
        let details = format!("icon {}", icon.id);
        audit(
            &state.db,
            &member.member.id,
            AuditAction::DeletedHeroIcon,
            AuditTargetType::Member,
            &icon.member_id,
            Some(details.as_str()),
        )
        .await;
    }
    Ok(Json(HeroIconsDeleted {
        deleted: u32::from(removed.is_some()),
    }))
}

/// GET /api/hero-icons (admin)
///
/// Icons of members who are active and opted in right now, newest first. One
/// audit entry `viewed_hero_icons` per request, target member `all`.
pub async fn list_hero_icons(
    State(state): State<AppState>,
    admin: AdminUser,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<CursorResponse<OfficerHeroIcon>>, Reject> {
    configured(&state)?;
    let (limit, offset) = pagination.resolve();
    let visible = visible_members(&state).await?;
    let member_ids: Vec<String> = visible.keys().cloned().collect();
    let rows = state
        .db
        .list_hero_icons_for_members(&member_ids, limit, offset)
        .await
        .map_err(db_error)?;
    let items: Vec<OfficerHeroIcon> = rows
        .iter()
        .map(|icon| officer_entry(icon, &visible))
        .collect();
    let page = CursorResponse::from_oversized(items, limit, offset);
    let details = format!("{} icons", page.data.len());
    audit(
        &state.db,
        &admin.member.id,
        AuditAction::ViewedHeroIcons,
        AuditTargetType::Member,
        "all",
        Some(details.as_str()),
    )
    .await;
    Ok(Json(page))
}

/// GET /api/hero-icons/members (officer or admin)
///
/// The only hero icon route open to officers. Counts only: no ids, images or
/// thumbnails.
///
/// Active members who are opted in, with their stored icon count, sorted by
/// display name (case folded), then member id. Paged in memory with the usual
/// cursor: the list is at most one row per member. Opted-out and inactive
/// members are not listed.
pub async fn hero_icon_member_counts(
    State(state): State<AppState>,
    _officer: OfficerUser,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<CursorResponse<HeroIconMemberCount>>, Reject> {
    configured(&state)?;
    let (limit, offset) = pagination.resolve();
    let visible = visible_members(&state).await?;
    let counts = state.db.hero_icon_counts().await.map_err(db_error)?;
    let mut members: Vec<HeroIconMemberCount> = visible
        .into_iter()
        .map(|(member_id, display_name)| {
            let icon_count = counts.get(&member_id).copied().unwrap_or(0);
            HeroIconMemberCount {
                display_name,
                icon_count: u32::try_from(icon_count).unwrap_or(u32::MAX),
                icon_contribution_opt_in: true,
                member_id,
            }
        })
        .collect();
    members.sort_by_key(|row| (row.display_name.to_lowercase(), row.member_id.clone()));
    let page: Vec<HeroIconMemberCount> = members
        .into_iter()
        .skip(offset as usize)
        .take(limit as usize + 1)
        .collect();
    Ok(Json(CursorResponse::from_oversized(page, limit, offset)))
}

/// GET /api/hero-icons/members/{member_id} (admin)
///
/// 404 `member_not_found` for an unknown or inactive member, 403
/// `icon_opt_in_off` while that member is not opted in. An admin asking for
/// their own icons gets them opted in or not. Reads of another member's list
/// are written to the audit log as `viewed_hero_icons`.
pub async fn member_hero_icons(
    State(state): State<AppState>,
    admin: AdminUser,
    UrlPath(member_id): UrlPath<String>,
) -> Result<Json<OfficerHeroIconList>, Reject> {
    configured(&state)?;
    let own = member_id == admin.member.id;
    let display_name = if own {
        admin.member.display_name.clone()
    } else {
        admin_target(&state, &member_id, AdminAccess::Read).await?
    };
    let rows = state
        .db
        .list_hero_icons_for_member(&member_id)
        .await
        .map_err(db_error)?;
    let names = HashMap::from([(member_id.clone(), display_name)]);
    let icons: Vec<OfficerHeroIcon> = rows
        .iter()
        .map(|icon| officer_entry(icon, &names))
        .collect();
    if !own {
        let details = format!("{} icons", icons.len());
        audit(
            &state.db,
            &admin.member.id,
            AuditAction::ViewedHeroIcons,
            AuditTargetType::Member,
            &member_id,
            Some(details.as_str()),
        )
        .await;
    }
    Ok(Json(OfficerHeroIconList { icons }))
}

/// DELETE /api/hero-icons/members/{member_id} (admin)
///
/// Removes every icon of one member, rows and files, while that member is
/// active, opted in or not: this is how an admin removes an opted-out
/// member's hidden icons. 404 `member_not_found` for an unknown or inactive
/// member, and nothing is deleted then. Written to the audit log as
/// `deleted_all_hero_icons` when the caller is not that member.
pub async fn delete_member_hero_icons(
    State(state): State<AppState>,
    admin: AdminUser,
    UrlPath(member_id): UrlPath<String>,
) -> Result<Json<HeroIconsDeleted>, Reject> {
    configured(&state)?;
    let own = member_id == admin.member.id;
    if !own {
        admin_target(&state, &member_id, AdminAccess::Delete).await?;
    }
    let deleted = delete_all_icons(&state, &member_id).await?;
    if deleted > 0 && !own {
        let details = format!("{deleted} icons");
        audit(
            &state.db,
            &admin.member.id,
            AuditAction::DeletedAllHeroIcons,
            AuditTargetType::Member,
            &member_id,
            Some(details.as_str()),
        )
        .await;
    }
    Ok(Json(HeroIconsDeleted { deleted }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::{must_err, must_ok, test_state};

    fn png_bytes(edge: u32) -> Vec<u8> {
        let canvas = image::RgbaImage::from_pixel(edge, edge, image::Rgba([10, 200, 90, 255]));
        let mut out = Cursor::new(Vec::new());
        canvas.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// Store two icons for `member_id` the way the upload route does.
    async fn store_two(state: &AppState, member_id: &str) -> Vec<HeroIcon> {
        ensure_hero_icons_dir(state.hero_icons.dir()).await.unwrap();
        let mut stored = Vec::new();
        for edge in [16, 24] {
            let icon = clean_png(&png_bytes(edge)).expect("clean png");
            let row = must_ok(
                store_icon(state, member_id, "Ana", icon, Utc::now()).await,
                "store icon",
            );
            stored.push(row);
        }
        stored
    }

    #[tokio::test]
    async fn recheck_keeps_icons_while_opted_in() {
        let state = test_state().await;
        state.db.set_hero_icon_opt_in("m1", true).await.unwrap();
        let stored = store_two(&state, "m1").await;

        must_ok(
            recheck_after_store(&state, "m1", &stored).await,
            "still opted in",
        );
        for icon in &stored {
            assert!(state.db.get_hero_icon(&icon.id).await.unwrap().is_some());
            assert!(state.hero_icons.icon_path(&icon.id).exists());
        }
        let _ = std::fs::remove_dir_all(state.hero_icons.dir());
    }

    #[tokio::test]
    async fn opt_out_during_upload_removes_this_requests_icons() {
        let state = test_state().await;
        state.db.set_hero_icon_opt_in("m1", true).await.unwrap();
        // An icon from an earlier request stays: opt-out keeps stored icons.
        let earlier = store_two(&state, "m1").await;
        let stored = store_two(&state, "m1").await;
        // The member opts out while this request's body is being read.
        state.db.set_hero_icon_opt_in("m1", false).await.unwrap();

        let (status, body) = must_err(
            recheck_after_store(&state, "m1", &stored).await,
            "opted out mid upload",
        );
        assert_eq!(status, StatusCode::FORBIDDEN);
        assert_eq!(body.error, HERO_ICON_ERR_OPT_IN_REQUIRED);
        for icon in &stored {
            assert!(state.db.get_hero_icon(&icon.id).await.unwrap().is_none());
            assert!(!state.hero_icons.icon_path(&icon.id).exists());
        }
        for icon in &earlier {
            assert!(state.db.get_hero_icon(&icon.id).await.unwrap().is_some());
            assert!(state.hero_icons.icon_path(&icon.id).exists());
        }
        let _ = std::fs::remove_dir_all(state.hero_icons.dir());
    }
}
