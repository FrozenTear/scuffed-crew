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
//!   only while its owner is active and opted in. Deletes are different: an
//!   admin deletes an active member's icons opted in or not, so hidden icons
//!   can still be removed. Officer counts list opted-in members only; admin
//!   counts add opted-out members who still have hidden icons.
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

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::Path;

use axum::Json;
use axum::body::Body;
use axum::extract::multipart::{Field, MultipartError, MultipartRejection};
use axum::extract::{Multipart, Path as UrlPath, Query, State};
use axum::http::{StatusCode, header};
use axum::response::{IntoResponse, Response};
use chrono::{DateTime, Utc};
use image::codecs::png::{CompressionType, FilterType, PngEncoder};
use image::{ColorType, DynamicImage, ImageFormat};
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
    CropKind, HERO_ICON_MAX_BYTES, HERO_ICON_MAX_EDGE, HERO_ICON_MAX_FILES_PER_UPLOAD,
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

pub(crate) type Reject = (StatusCode, Json<ErrorResponse>);

/// The 403 body of the `AdminUser` extractor. The per-icon routes give it to
/// officers so Site handles one string for every admin-only refusal.
pub(crate) const ADMIN_REQUIRED: &str = "Admin access required";

pub(crate) fn reject(status: StatusCode, error_code: &str) -> Reject {
    (
        status,
        Json(ErrorResponse {
            error: error_code.into(),
        }),
    )
}

pub(crate) fn internal() -> Reject {
    reject(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
}

fn not_found() -> Reject {
    reject(StatusCode::NOT_FOUND, HERO_ICON_ERR_NOT_FOUND)
}

pub(crate) fn db_error(error: DbError) -> Reject {
    tracing::error!(%error, "hero icon database call failed");
    internal()
}

/// 503 `hero_icons_disabled` when `HERO_ICONS_DIR` could not be used at startup.
pub(crate) fn configured(state: &AppState) -> Result<(), Reject> {
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
/// Admin icon lists and reads cover these members only.
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
pub(crate) async fn member_active(state: &AppState, member_id: &str) -> Result<bool, Reject> {
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
pub(crate) async fn remove_icon_file(state: &AppState, id: &str) -> std::io::Result<()> {
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
pub(crate) async fn discard_icon_file(state: &AppState, id: &str) {
    if let Err(error) = remove_icon_file(state, id).await {
        tracing::error!(%error, "hero icon file cleanup failed");
    }
}

/// What [`delete_all_icons`] did.
///
/// `deleted` counts the rows removed even when `failure` is set, so a caller
/// can audit a partial delete before it answers with the failure.
struct DeleteAllOutcome {
    deleted: u32,
    failure: Option<Reject>,
}

impl DeleteAllOutcome {
    /// The count, or the failure when one happened.
    fn into_result(self) -> Result<u32, Reject> {
        match self.failure {
            Some(failure) => Err(failure),
            None => Ok(self.deleted),
        }
    }
}

/// Remove every icon of `member_id`.
///
/// Each file goes before its row, as in a single delete. A file that cannot be
/// removed keeps its row, so the icon stays listed and the call can be
/// retried. The other icons are still removed, and the outcome carries a 500.
/// A database error stops the loop and is carried the same way. Opt-in is not
/// checked: the owner and admins delete hidden icons too.
async fn delete_all_icons(state: &AppState, member_id: &str) -> Result<DeleteAllOutcome, Reject> {
    // Ids of hero_icon rows only, so a row that fails to parse is deleted
    // too and other kinds are left alone.
    let ids = state
        .db
        .training_crop_ids_for_member(member_id, Some(CropKind::HeroIcon))
        .await
        .map_err(db_error)?;
    let mut deleted: u32 = 0;
    let mut file_failed = false;
    let mut failure = None;
    for id in &ids {
        if let Err(error) = remove_icon_file(state, id).await {
            tracing::error!(%error, "hero icon file delete failed");
            file_failed = true;
            continue;
        }
        match state.db.delete_training_crop(id).await {
            Ok(true) => deleted += 1,
            Ok(false) => {}
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
pub(crate) async fn write_private(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
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
pub(crate) struct CleanIcon {
    pub(crate) bytes: Vec<u8>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}

/// Check the header size, decode, and re-encode one PNG. Runs on a blocking thread.
///
/// The stored file is the re-encoded image, so text chunks and other metadata
/// in the upload are dropped. `Err` is the error code; answer it with
/// [`clean_reject`].
pub(crate) fn clean_png(data: &[u8]) -> Result<CleanIcon, &'static str> {
    clean_png_within(
        data,
        HERO_ICON_MAX_EDGE,
        HERO_ICON_MAX_EDGE,
        HERO_ICON_MAX_BYTES,
    )
}

/// [`clean_png`] with its own largest width, height and stored size. Each
/// edge is still at least `HERO_ICON_MIN_EDGE`. Training crops use it for
/// the bounds of each kind.
///
/// The stored file is never larger than `max_bytes`, the same limit as the
/// upload part: the image is re-encoded at 8 bits per sample (gray, gray
/// with alpha, RGB or RGBA, as the upload has them; 16-bit samples are
/// rounded) at the default PNG compression level, again at the best level
/// only when the default result is over `max_bytes`, and a re-encode still
/// over it is refused with `icon_too_large` (413, like an oversized
/// upload). Without this a small 16-bit or palette upload could be stored
/// many times larger than its kind's limit.
///
/// Before anything is decoded, the width and height are read from the
/// `IHDR` bytes and checked, and only `IHDR`, `PLTE`, `tRNS`, `IDAT` and
/// `IEND` are passed to the decoder ([`png_pixel_chunks`]). Every other
/// chunk (colour profiles, text, EXIF, animation) is dropped without being
/// inflated, so a compressed `iCCP` or `zTXt` costs nothing however large it
/// inflates. What a decode can allocate is then the pixel buffer plus small
/// working buffers; [`PNG_DECODE_MAX_ALLOC`] is a backstop on top. Each
/// re-encode attempt writes into a writer capped at `max_bytes` and a
/// failed default-level attempt is dropped before Best runs, so a refused
/// crop costs about what an accepted one does (measured figures on
/// [`PNG_DECODE_MAX_ALLOC`]). The PNG encoder's own compression buffer is
/// not capped (png 0.18 compresses the whole image before writing it out);
/// the measured peaks include it.
pub(crate) fn clean_png_within(
    data: &[u8],
    max_width: u32,
    max_height: u32,
    max_bytes: usize,
) -> Result<CleanIcon, &'static str> {
    let Some((width, height, pixels_only)) = png_pixel_chunks(data) else {
        return Err(HERO_ICON_ERR_INVALID_PNG);
    };
    let edge_ok = |edge: u32, max: u32| (HERO_ICON_MIN_EDGE..=max).contains(&edge);
    if !edge_ok(width, max_width) || !edge_ok(height, max_height) {
        return Err(HERO_ICON_ERR_BAD_DIMENSIONS);
    }
    let Ok(decoded) = png_reader(&pixels_only).decode() else {
        return Err(HERO_ICON_ERR_INVALID_PNG);
    };
    let (width, height) = (decoded.width(), decoded.height());
    let eight_bit = to_eight_bit(decoded);
    // The default level first: fast, and enough for nearly every crop. Best
    // only when that is over the limit, since it costs far more CPU. Each
    // attempt writes into a writer capped at `max_bytes`, so an attempt that
    // goes over stops there and keeps nothing; the default attempt's output
    // is gone before Best runs.
    let bytes = match encode_png(&eight_bit, CompressionType::Default, max_bytes) {
        Encoded::Fits(bytes) => bytes,
        Encoded::OverLimit => match encode_png(&eight_bit, CompressionType::Best, max_bytes) {
            Encoded::Fits(bytes) => bytes,
            Encoded::OverLimit => return Err(HERO_ICON_ERR_TOO_LARGE),
            Encoded::Failed => return Err(HERO_ICON_ERR_INVALID_PNG),
        },
        Encoded::Failed => return Err(HERO_ICON_ERR_INVALID_PNG),
    };
    Ok(CleanIcon {
        bytes,
        width,
        height,
    })
}

/// Backstop on what the image decoder may allocate: 32 MiB (the image
/// crate's default is 512 MiB). The real bound comes from
/// [`png_pixel_chunks`] and the edge check: only pixel data reaches the
/// decoder, and the largest accepted crop, 1536x720 16-bit RGBA, is about
/// 8.4 MiB decoded. Measured peak heap for a whole [`clean_png_within`]
/// call (test `one_png_read_stays_within_its_measured_peak`, debug build):
/// 12.7 MiB for the largest accepted rank crop (1536x720 16-bit RGBA),
/// 12.9 MiB for a refused rank crop whose re-encode is about 4.4 MiB at
/// both levels (413), 3.6 MiB for a colour profile bomb, 0.8 MiB for the
/// largest hero icon and 0.9 MiB for a refused one. The test asserts the
/// rank cases stay under 20 MiB.
pub(crate) const PNG_DECODE_MAX_ALLOC: u64 = 32 * 1024 * 1024;

/// The chunks the decoder needs for the pixels. Everything else is dropped.
const PNG_PIXEL_CHUNKS: [&[u8; 4]; 5] = [b"IHDR", b"PLTE", b"tRNS", b"IDAT", b"IEND"];

/// Width, height and a copy of `data` (a PNG) that keeps only
/// [`PNG_PIXEL_CHUNKS`], each copied byte for byte (CRC included). Walks the
/// chunk headers; nothing is inflated. Ancillary chunks (lowercase first
/// letter: colour profiles, text, EXIF, animation) are dropped. `None` when
/// the signature is missing, the first chunk is not a 13-byte `IHDR`, a
/// chunk runs past the end of `data`, a `PLTE` length is not a multiple of
/// 3, or there is a critical chunk (uppercase first letter) that is not in
/// [`PNG_PIXEL_CHUNKS`]. Bytes after `IEND` are ignored.
fn png_pixel_chunks(data: &[u8]) -> Option<(u32, u32, Vec<u8>)> {
    if !data.starts_with(&PNG_SIGNATURE) {
        return None;
    }
    let mut out = Vec::with_capacity(data.len());
    out.extend_from_slice(&PNG_SIGNATURE);
    let mut size = None;
    let mut at = PNG_SIGNATURE.len();
    while at < data.len() {
        let header = data.get(at..at + 8)?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let end = at.checked_add(12)?.checked_add(len)?;
        let chunk = data.get(at..end)?;
        let kind = &header[4..8];
        if size.is_none() {
            // The length check comes first; the fields are then read with
            // `get`, so even a short IHDR can never index out of bounds.
            if kind != b"IHDR" || len != 13 {
                return None;
            }
            let fields = chunk.get(8..16)?;
            let field = |i: usize| {
                u32::from_be_bytes([fields[i], fields[i + 1], fields[i + 2], fields[i + 3]])
            };
            size = Some((field(0), field(4)));
        }
        // A PLTE that is not whole RGB triples makes png 0.18.1 panic
        // (decoder/transform/palette.rs), which would be a 500.
        if kind == b"PLTE" && !len.is_multiple_of(3) {
            return None;
        }
        if PNG_PIXEL_CHUNKS.iter().any(|keep| kind == &keep[..]) {
            out.extend_from_slice(chunk);
        } else if kind[0] & 0x20 == 0 {
            // An unknown critical chunk (uppercase first letter). A decoder
            // must not ignore one (PNG spec), so the file is refused.
            return None;
        }
        at = end;
        if kind == b"IEND" {
            break;
        }
    }
    let (width, height) = size?;
    Some((width, height, out))
}

/// A PNG reader over `data` with [`PNG_DECODE_MAX_ALLOC`] as its memory
/// limit. Give it the output of [`png_pixel_chunks`].
fn png_reader(data: &[u8]) -> image::ImageReader<Cursor<&[u8]>> {
    let mut reader = image::ImageReader::with_format(Cursor::new(data), ImageFormat::Png);
    let mut limits = image::Limits::default();
    limits.max_alloc = Some(PNG_DECODE_MAX_ALLOC);
    reader.limits(limits);
    reader
}

/// The result of one capped PNG encode.
enum Encoded {
    /// The whole PNG, at most the cap.
    Fits(Vec<u8>),
    /// The output passed the cap; nothing is kept.
    OverLimit,
    /// The encoder failed for another reason.
    Failed,
}

/// A `Vec` writer that refuses any write that would take it past `cap`
/// bytes, so the output of one encode never grows past the limit.
struct CappedWriter {
    bytes: Vec<u8>,
    cap: usize,
    over: bool,
}

impl std::io::Write for CappedWriter {
    fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
        if self.bytes.len().saturating_add(data.len()) > self.cap {
            self.over = true;
            return Err(std::io::Error::other("PNG re-encode over the byte limit"));
        }
        self.bytes.extend_from_slice(data);
        Ok(data.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

/// `image` as a PNG at `level` with adaptive filtering, into a writer
/// capped at `max_bytes`. The PNG encoder (png 0.18) still compresses the
/// whole image into its own buffer before writing it out, so that buffer
/// is not capped; the measured peak covers it (see
/// `one_png_read_stays_within_its_measured_peak`).
fn encode_png(image: &DynamicImage, level: CompressionType, max_bytes: usize) -> Encoded {
    let mut writer = CappedWriter {
        bytes: Vec::new(),
        cap: max_bytes,
        over: false,
    };
    let encoder = PngEncoder::new_with_quality(&mut writer, level, FilterType::Adaptive);
    let result = image.write_with_encoder(encoder);
    // Check `over` first, even on `Ok`: png 0.18 writes the closing `IEND`
    // when its writer is dropped and ignores an error there, so a cap that
    // falls inside the last 12 bytes would otherwise come back as `Ok` with
    // a PNG missing its `IEND`.
    if writer.over {
        return Encoded::OverLimit;
    }
    match result {
        Ok(()) => Encoded::Fits(writer.bytes),
        Err(_) => Encoded::Failed,
    }
}

/// `image` at 8 bits per sample, keeping gray and alpha where it has them.
fn to_eight_bit(image: DynamicImage) -> DynamicImage {
    match image.color() {
        ColorType::L8 | ColorType::La8 | ColorType::Rgb8 | ColorType::Rgba8 => image,
        ColorType::L16 => DynamicImage::ImageLuma8(image.to_luma8()),
        ColorType::La16 => DynamicImage::ImageLumaA8(image.to_luma_alpha8()),
        color if color.has_alpha() => DynamicImage::ImageRgba8(image.to_rgba8()),
        _ => DynamicImage::ImageRgb8(image.to_rgb8()),
    }
}

/// The answer for a [`clean_png_within`] error code: 413 for
/// `icon_too_large` (a re-encode over the kind's byte limit), 400 for the
/// others.
pub(crate) fn clean_reject(code: &'static str) -> Reject {
    let status = if code == HERO_ICON_ERR_TOO_LARGE {
        StatusCode::PAYLOAD_TOO_LARGE
    } else {
        StatusCode::BAD_REQUEST
    };
    reject(status, code)
}

pub(crate) fn multipart_reject(error: MultipartError) -> Reject {
    if error.status() == StatusCode::PAYLOAD_TOO_LARGE {
        return reject(StatusCode::PAYLOAD_TOO_LARGE, HERO_ICON_ERR_TOO_LARGE);
    }
    tracing::debug!(%error, "hero icon multipart rejected");
    reject(StatusCode::BAD_REQUEST, HERO_ICON_ERR_INVALID_MULTIPART)
}

/// Read one `icon` part. The signature is checked as soon as 8 bytes are in,
/// and reading stops as soon as the part passes [`HERO_ICON_MAX_BYTES`].
pub(crate) async fn read_icon_part(field: &mut Field<'_>) -> Result<Vec<u8>, Reject> {
    read_png_part(field, HERO_ICON_MAX_BYTES).await
}

/// [`read_icon_part`] with its own byte limit (413 `icon_too_large` past
/// `max_bytes`). Training crops use it for each kind's limit
/// (`CropKind::max_crop_bytes`).
pub(crate) async fn read_png_part(
    field: &mut Field<'_>,
    max_bytes: usize,
) -> Result<Vec<u8>, Reject> {
    let mut data: Vec<u8> = Vec::new();
    while let Some(chunk) = field.chunk().await.map_err(multipart_reject)? {
        data.extend_from_slice(&chunk);
        if data.len() >= PNG_SIGNATURE.len() && !data.starts_with(&PNG_SIGNATURE) {
            return Err(reject(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                HERO_ICON_ERR_NOT_PNG,
            ));
        }
        if data.len() > max_bytes {
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
/// decode 400, re-encode over `HERO_ICON_MAX_BYTES` 413). Nothing is stored unless every part passes. A body that is not
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
            Err(code) => return Err(clean_reject(code).into()),
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
    let deleted = delete_all_icons(&state, &member.member.id)
        .await?
        .into_result()?;
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
/// Active members with their stored icon count, sorted by display name (case
/// folded), then member id. Paged in memory with the usual cursor: the list is
/// at most one row per member. Inactive members are never listed.
///
/// - Officers get opted-in members only, so no hidden icon is revealed.
/// - Admins also get opted-out members who still have at least one hidden
///   icon, with `opted_in` false, since admins may delete those icons.
pub async fn hero_icon_member_counts(
    State(state): State<AppState>,
    staff: OfficerUser,
    Query(pagination): Query<PaginationParams>,
) -> Result<Json<CursorResponse<HeroIconMemberCount>>, Reject> {
    configured(&state)?;
    let (limit, offset) = pagination.resolve();
    let is_admin = staff.member.org_role == OrgRole::Admin;
    let counts = state.db.hero_icon_counts().await.map_err(db_error)?;
    let opted_in: HashSet<String> = state
        .db
        .hero_icon_opted_in_member_ids()
        .await
        .map_err(db_error)?
        .into_iter()
        .collect();
    let mut candidates: Vec<String> = opted_in.iter().cloned().collect();
    if is_admin {
        let hidden = counts
            .iter()
            .filter(|(member_id, count)| **count > 0 && !opted_in.contains(*member_id))
            .map(|(member_id, _)| member_id.clone());
        candidates.extend(hidden);
    }
    // A visibility filter: a failed lookup is a 500, never an unfiltered list.
    let active = state
        .db
        .active_member_display_names(&candidates)
        .await
        .map_err(db_error)?;
    let mut members: Vec<HeroIconMemberCount> = active
        .into_iter()
        .map(|(member_id, display_name)| {
            let icon_count = counts.get(&member_id).copied().unwrap_or(0);
            let member_opted_in = opted_in.contains(&member_id);
            HeroIconMemberCount {
                display_name,
                icon_count: u32::try_from(icon_count).unwrap_or(u32::MAX),
                opted_in: member_opted_in,
                icon_contribution_opt_in: member_opted_in,
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
/// `deleted_all_hero_icons` when the caller is not that member and at least
/// one icon went, also when a later icon failed and the answer is a 500. The
/// details then say `partial`.
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
    let outcome = delete_all_icons(&state, &member_id).await?;
    let deleted = outcome.deleted;
    if deleted > 0 && !own {
        let details = if outcome.failure.is_some() {
            format!("{deleted} icons, partial")
        } else {
            format!("{deleted} icons")
        };
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
    let deleted = outcome.into_result()?;
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

    /// A `width` x `height` RGB PNG whose `iCCP` chunk inflates to
    /// `profile_bytes` zeros (a few KiB compressed per MiB).
    fn iccp_png(width: u32, height: u32, profile_bytes: usize) -> Vec<u8> {
        use std::io::Write;
        let plain = encode(DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            width,
            height,
            image::Rgb([20, 120, 220]),
        )));
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
        let zeros = vec![0u8; 1024 * 1024];
        let mut left = profile_bytes;
        while left > 0 {
            let n = left.min(zeros.len());
            z.write_all(&zeros[..n]).unwrap();
            left -= n;
        }
        let mut data = b"bomb\0\0".to_vec(); // profile name, NUL, compression method 0
        data.extend(z.finish().unwrap());
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(b"iCCP");
        chunk.extend_from_slice(&data);
        let mut crc = flate2::Crc::new();
        crc.update(&chunk[4..]);
        chunk.extend_from_slice(&crc.sum().to_be_bytes());
        // After the signature (8) and IHDR (12 + 13).
        let at = 8 + 12 + 13;
        let mut out = plain[..at].to_vec();
        out.extend(chunk);
        out.extend_from_slice(&plain[at..]);
        out
    }

    /// `image` encoded at `level` with no cap, for comparisons.
    fn encode_at(image: &DynamicImage, level: CompressionType) -> Vec<u8> {
        match encode_png(image, level, usize::MAX) {
            Encoded::Fits(bytes) => bytes,
            _ => panic!("an uncapped encode fits"),
        }
    }

    /// Encode `image` as a PNG with the image crate's defaults (fast
    /// compression, the image's own bit depth), as a client might.
    fn encode(image: DynamicImage) -> Vec<u8> {
        let mut out = Cursor::new(Vec::new());
        image.write_to(&mut out, ImageFormat::Png).unwrap();
        out.into_inner()
    }

    /// A 1536x720 16-bit image in 64 px blocks: small as an upload, large
    /// once decoded (8 bytes per pixel with alpha).
    fn blocks_16bit(alpha: bool) -> DynamicImage {
        let value =
            |x: u32, y: u32, k: u32| (((x / 64) * 7 + (y / 64) * 13 + k * 29) % 256) as u16 * 257;
        if alpha {
            DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(1536, 720, |x, y| {
                image::Rgba([value(x, y, 0), value(x, y, 1), value(x, y, 2), 65535])
            }))
        } else {
            DynamicImage::ImageRgb16(image::ImageBuffer::from_fn(1536, 720, |x, y| {
                image::Rgb([value(x, y, 0), value(x, y, 1), value(x, y, 2)])
            }))
        }
    }

    #[test]
    fn sixteen_bit_uploads_are_stored_as_eight_bit_within_the_limit() {
        const LIMIT: usize = 384 * 1024;
        for (alpha, stored_color) in [(true, ColorType::Rgba8), (false, ColorType::Rgb8)] {
            let upload = encode(blocks_16bit(alpha));
            assert!(upload.len() <= LIMIT, "upload {}", upload.len());
            let clean = clean_png_within(&upload, 1536, 720, LIMIT).expect("accepted");
            assert!(clean.bytes.len() <= LIMIT, "stored {}", clean.bytes.len());
            assert_eq!((clean.width, clean.height), (1536, 720));
            let stored =
                image::load_from_memory_with_format(&clean.bytes, ImageFormat::Png).unwrap();
            assert_eq!(stored.color(), stored_color);
            // Same picture at 8 bits: every 16-bit sample was v * 257.
            let original = blocks_16bit(alpha).to_rgba8();
            assert_eq!(stored.to_rgba8(), original);
        }
        // Gray stays gray, and gray with alpha keeps its alpha, at 8 bits.
        // 16-bit samples are rounded to 8 bits ((v + 128) / 257), not cut to
        // the high byte (v >> 8). The values are ones where the two differ:
        // 511 rounds to 2 (cut: 1), 200 rounds to 1 (cut: 0), 65_400
        // rounds to 254 (cut: 255).
        let gray = DynamicImage::ImageLuma16(image::ImageBuffer::from_pixel(
            64,
            64,
            image::Luma([511u16]),
        ));
        let clean = clean_png_within(&encode(gray), 256, 256, LIMIT).unwrap();
        let stored = image::load_from_memory_with_format(&clean.bytes, ImageFormat::Png).unwrap();
        assert_eq!(stored.color(), ColorType::L8);
        assert_eq!(stored.to_luma8().get_pixel(0, 0).0, [2]);
        let gray_alpha = DynamicImage::ImageLumaA16(image::ImageBuffer::from_pixel(
            64,
            64,
            image::LumaA([200u16, 65_400]),
        ));
        let clean = clean_png_within(&encode(gray_alpha), 256, 256, LIMIT).unwrap();
        let stored = image::load_from_memory_with_format(&clean.bytes, ImageFormat::Png).unwrap();
        assert_eq!(stored.color(), ColorType::La8);
        assert_eq!(stored.to_luma_alpha8().get_pixel(0, 0).0, [1, 254]);
    }

    #[test]
    fn a_re_encode_over_the_byte_limit_is_icon_too_large() {
        // Noise does not compress, so its re-encode is about 4 bytes per
        // pixel whatever the upload looked like.
        let mut state = 0x9e37_79b9u32;
        let noise = image::RgbaImage::from_fn(128, 128, |_, _| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            image::Rgba(state.to_le_bytes())
        });
        let noise = DynamicImage::ImageRgba8(noise);
        let upload = encode(noise.clone());
        // The smallest the server can make it: the better of both levels.
        let smallest = [CompressionType::Default, CompressionType::Best]
            .map(|level| encode_at(&noise, level).len())
            .into_iter()
            .min()
            .unwrap();
        assert!(
            clean_png_within(&upload, 256, 256, smallest).is_ok(),
            "exactly at the limit"
        );
        assert_eq!(
            clean_png_within(&upload, 256, 256, smallest - 1).err(),
            Some(HERO_ICON_ERR_TOO_LARGE)
        );
        assert_eq!(
            clean_reject(HERO_ICON_ERR_TOO_LARGE).0,
            StatusCode::PAYLOAD_TOO_LARGE
        );
        assert_eq!(
            clean_reject(HERO_ICON_ERR_BAD_DIMENSIONS).0,
            StatusCode::BAD_REQUEST
        );
        // The hero icon route's own limit applies to its re-encode too.
        assert_eq!(clean_png(&upload).err(), Some(HERO_ICON_ERR_TOO_LARGE));
    }

    #[test]
    fn a_normal_crop_is_stored_at_the_default_level() {
        let image = blocks_16bit(true);
        let clean = clean_png_within(&encode(image.clone()), 1536, 720, 384 * 1024).unwrap();
        let eight = to_eight_bit(image);
        let default = encode_at(&eight, CompressionType::Default);
        let best = encode_at(&eight, CompressionType::Best);
        assert_ne!(default, best, "the two levels give different files here");
        assert_eq!(
            clean.bytes, default,
            "stored at the default level, not re-encoded at best"
        );
    }

    #[test]
    fn a_crop_over_the_limit_at_default_but_not_at_best_is_stored_at_best() {
        // Half noise, half flat: the levels differ by enough to put a limit
        // between them.
        let mut state = 0x2545_f491u32;
        let image = DynamicImage::ImageRgb8(image::RgbImage::from_fn(256, 256, |x, y| {
            state ^= state << 13;
            state ^= state >> 17;
            state ^= state << 5;
            if (x / 16 + y / 16) % 2 == 0 {
                image::Rgb([(state & 0x0f) as u8, 90, 180])
            } else {
                image::Rgb([30, 60, (x % 7) as u8 * 30])
            }
        }));
        let default = encode_at(&image, CompressionType::Default);
        let best = encode_at(&image, CompressionType::Best);
        assert!(
            best.len() < default.len(),
            "best {} default {}",
            best.len(),
            default.len()
        );
        let upload = encode(image);
        // Limit between the two: accepted, stored at best.
        let clean = clean_png_within(&upload, 256, 256, best.len()).expect("fits at best");
        assert_eq!(clean.bytes, best);
        // Limit under best: 413.
        assert_eq!(
            clean_png_within(&upload, 256, 256, best.len() - 1).err(),
            Some(HERO_ICON_ERR_TOO_LARGE)
        );
        // Limit at the default size: stored at default.
        let clean = clean_png_within(&upload, 256, 256, default.len()).unwrap();
        assert_eq!(clean.bytes, default);
    }

    /// The chunk types of `png`, in order.
    fn chunk_kinds(png: &[u8]) -> Vec<[u8; 4]> {
        let mut kinds = Vec::new();
        let mut at = 8;
        while let Some(header) = png.get(at..at + 8) {
            kinds.push([header[4], header[5], header[6], header[7]]);
            at += 12 + u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        }
        kinds
    }

    /// One PNG chunk: length, type, data, CRC.
    fn png_chunk(kind: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = u32::try_from(data.len()).unwrap().to_be_bytes().to_vec();
        chunk.extend_from_slice(kind);
        chunk.extend_from_slice(data);
        let mut crc = flate2::Crc::new();
        crc.update(&chunk[4..]);
        chunk.extend_from_slice(&crc.sum().to_be_bytes());
        chunk
    }

    /// An 8-bit palette PNG, `width` x `height`, pixel x using index
    /// `x % 2`. `extra` chunks go between `tRNS` and `IDAT`; the image data
    /// is split over two `IDAT` chunks.
    fn palette_png(
        width: u32,
        height: u32,
        plte: &[u8],
        trns: Option<&[u8]>,
        extra: &[Vec<u8>],
    ) -> Vec<u8> {
        use std::io::Write;
        let mut raw = Vec::new();
        for _ in 0..height {
            raw.push(0);
            raw.extend((0..width).map(|x| (x % 2) as u8));
        }
        let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        z.write_all(&raw).unwrap();
        let idat = z.finish().unwrap();
        let mut ihdr = width.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&height.to_be_bytes());
        ihdr.extend_from_slice(&[8, 3, 0, 0, 0]);
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend(png_chunk(b"IHDR", &ihdr));
        png.extend(png_chunk(b"PLTE", plte));
        if let Some(trns) = trns {
            png.extend(png_chunk(b"tRNS", trns));
        }
        for chunk in extra {
            png.extend_from_slice(chunk);
        }
        let half = idat.len() / 2;
        png.extend(png_chunk(b"IDAT", &idat[..half]));
        png.extend(png_chunk(b"IDAT", &idat[half..]));
        png.extend(png_chunk(b"IEND", &[]));
        png
    }

    const RED_AND_BLACK: [u8; 6] = [0, 0, 0, 255, 0, 0];

    #[test]
    fn a_plte_that_is_not_whole_triples_is_invalid_png_not_a_panic() {
        let bad = palette_png(8, 8, &[0, 0, 0, 255], None, &[]);
        // Without the walker's check png 0.18.1 panics on this file.
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = png_reader(&bad).decode();
        }))
        .is_err();
        assert!(panicked, "png 0.18.1 panics on a PLTE of 4 bytes");
        assert!(png_pixel_chunks(&bad).is_none());
        assert_eq!(
            clean_png_within(&bad, 1536, 720, 384 * 1024).err(),
            Some(HERO_ICON_ERR_INVALID_PNG)
        );
        assert_eq!(clean_png(&bad).err(), Some(HERO_ICON_ERR_INVALID_PNG));
        // Whole triples are fine.
        let good = palette_png(8, 8, &RED_AND_BLACK, None, &[]);
        assert!(clean_png(&good).is_ok());
    }

    #[test]
    fn trns_is_kept_so_palette_transparency_survives() {
        // Index 0 fully transparent, index 1 opaque red.
        let png = palette_png(8, 8, &RED_AND_BLACK, Some(&[0, 255]), &[]);
        let clean = clean_png_within(&png, 256, 256, 64 * 1024).expect("stored");
        let stored = image::load_from_memory_with_format(&clean.bytes, ImageFormat::Png).unwrap();
        assert_eq!(
            stored.color(),
            ColorType::Rgba8,
            "tRNS kept: stored with alpha"
        );
        let rgba = stored.to_rgba8();
        assert_eq!(rgba.get_pixel(0, 0).0, [0, 0, 0, 0]);
        assert_eq!(rgba.get_pixel(1, 0).0, [255, 0, 0, 255]);
    }

    #[test]
    fn the_keep_list_is_exactly_ihdr_plte_trns_idat_iend() {
        let extra = [
            png_chunk(b"tEXt", b"Comment\0hello"),
            png_chunk(b"zTXt", b"Comment\0\0x"),
            png_chunk(b"gAMA", &45_455u32.to_be_bytes()),
            png_chunk(b"juNK", b"private ancillary"),
        ];
        let mut png = palette_png(8, 8, &RED_AND_BLACK, Some(&[0, 255]), &extra);
        png.extend_from_slice(b"bytes after IEND are ignored");
        let (_, _, kept) = png_pixel_chunks(&png).expect("valid");
        assert_eq!(
            chunk_kinds(&kept),
            vec![*b"IHDR", *b"PLTE", *b"tRNS", *b"IDAT", *b"IDAT", *b"IEND"]
        );
        // Kept chunks are byte for byte, so the pixels are the same.
        let kept_image = image::load_from_memory_with_format(&kept, ImageFormat::Png).unwrap();
        let clean = palette_png(8, 8, &RED_AND_BLACK, Some(&[0, 255]), &[]);
        let plain_image = image::load_from_memory_with_format(&clean, ImageFormat::Png).unwrap();
        assert_eq!(kept_image.to_rgba8(), plain_image.to_rgba8());
        // IHDR longer than 13 bytes is refused.
        let mut long_ihdr = PNG_SIGNATURE.to_vec();
        long_ihdr.extend(png_chunk(b"IHDR", &[0; 14]));
        long_ihdr.extend_from_slice(&png[8 + 25..]);
        assert!(png_pixel_chunks(&long_ihdr).is_none());
    }

    #[test]
    fn an_unknown_critical_chunk_is_refused() {
        // Uppercase first letter: critical. A decoder must not skip it.
        let png = palette_png(8, 8, &RED_AND_BLACK, None, &[png_chunk(b"JUNK", b"x")]);
        assert!(png_pixel_chunks(&png).is_none());
        assert_eq!(clean_png(&png).err(), Some(HERO_ICON_ERR_INVALID_PNG));
        // The same chunk as ancillary (lowercase first letter) is dropped.
        let png = palette_png(8, 8, &RED_AND_BLACK, None, &[png_chunk(b"jUNK", b"x")]);
        assert!(clean_png(&png).is_ok());
    }

    #[test]
    fn the_decode_backstop_refuses_a_pixel_buffer_over_32_mib() {
        // 4000x4000 16-bit RGBA is 122 MiB of pixels. The edge check does
        // not stop it here (bounds of 4000 on purpose), so only the decoder
        // limit does, before any pixel is read.
        let mut ihdr = 4000u32.to_be_bytes().to_vec();
        ihdr.extend_from_slice(&4000u32.to_be_bytes());
        ihdr.extend_from_slice(&[16, 6, 0, 0, 0]);
        let mut png = PNG_SIGNATURE.to_vec();
        png.extend(png_chunk(b"IHDR", &ihdr));
        png.extend(png_chunk(
            b"IDAT",
            &[0x78, 0x01, 0x03, 0x00, 0x00, 0x00, 0x00, 0x01],
        ));
        png.extend(png_chunk(b"IEND", &[]));
        let (_, _, kept) = png_pixel_chunks(&png).unwrap();
        assert!(
            matches!(
                png_reader(&kept).decode(),
                Err(image::ImageError::Limits(_))
            ),
            "refused by the 32 MiB limit"
        );
        assert_eq!(
            clean_png_within(&png, 4000, 4000, usize::MAX).err(),
            Some(HERO_ICON_ERR_INVALID_PNG)
        );
        assert_eq!(PNG_DECODE_MAX_ALLOC, 32 * 1024 * 1024);
    }

    #[test]
    fn only_pixel_chunks_reach_the_decoder() {
        let png = iccp_png(64, 32, 4096);
        assert!(chunk_kinds(&png).contains(b"iCCP"));
        let (width, height, kept) = png_pixel_chunks(&png).expect("valid");
        assert_eq!((width, height), (64, 32));
        assert_eq!(chunk_kinds(&kept), vec![*b"IHDR", *b"IDAT", *b"IEND"]);
        // Kept chunks are copied as they are; the image is unchanged.
        let a = image::load_from_memory_with_format(&png, ImageFormat::Png).unwrap();
        let b = image::load_from_memory_with_format(&kept, ImageFormat::Png).unwrap();
        assert_eq!(a.to_rgba8(), b.to_rgba8());
        // Malformed shapes are None (400 invalid_png at the route).
        assert!(png_pixel_chunks(b"not a png").is_none());
        assert!(
            png_pixel_chunks(&png[..png.len() - 5]).is_none(),
            "chunk past the end"
        );
        let mut no_ihdr = png[..8].to_vec();
        no_ihdr.extend_from_slice(&png[8 + 25..]);
        assert!(
            png_pixel_chunks(&no_ihdr).is_none(),
            "first chunk must be IHDR"
        );
    }

    #[test]
    fn an_absurd_header_is_bad_dimensions_before_any_decode() {
        // IHDR claims 100000 x 100000; nothing is decoded.
        let mut png = encode(DynamicImage::ImageRgb8(image::RgbImage::new(8, 8)));
        png[16..20].copy_from_slice(&100_000u32.to_be_bytes());
        png[20..24].copy_from_slice(&100_000u32.to_be_bytes());
        assert_eq!(
            clean_png_within(&png, 1536, 720, 384 * 1024).err(),
            Some(HERO_ICON_ERR_BAD_DIMENSIONS)
        );
    }

    #[test]
    fn a_huge_iccp_profile_is_dropped_unread_and_the_crop_is_stored() {
        // 40 MiB of profile in a 58 KB upload, and an unreadable profile
        // (bad CRC): both dropped before decoding, the pixels stored.
        let bomb = iccp_png(1536, 720, 40 * 1024 * 1024);
        assert!(bomb.len() < 64 * 1024, "{}", bomb.len());
        let mut bad_crc = iccp_png(1536, 720, 4096);
        let iccp_crc_end = 8
            + 25
            + 12
            + u32::from_be_bytes([bad_crc[33], bad_crc[34], bad_crc[35], bad_crc[36]]) as usize;
        bad_crc[iccp_crc_end - 1] ^= 0xff;
        for upload in [&bomb, &bad_crc] {
            let clean = clean_png_within(upload, 1536, 720, 384 * 1024).expect("stored");
            assert_eq!((clean.width, clean.height), (1536, 720));
            assert!(!chunk_kinds(&clean.bytes).contains(b"iCCP"));
        }
        let small = iccp_png(64, 64, 40 * 1024 * 1024);
        assert!(clean_png(&small).is_ok());
    }

    /// Heap bytes allocated by this thread: now and the peak since
    /// [`reset_peak`]. Counted by [`PeakAlloc`] in this test binary only.
    mod heap {
        use std::alloc::{GlobalAlloc, Layout, System};
        use std::cell::Cell;

        thread_local! {
            static NOW: Cell<usize> = const { Cell::new(0) };
            static PEAK: Cell<usize> = const { Cell::new(0) };
        }

        fn track(grow: usize, shrink: usize) {
            let _ = NOW.try_with(|now| {
                let value = now.get().saturating_add(grow).saturating_sub(shrink);
                now.set(value);
                let _ = PEAK.try_with(|peak| peak.set(peak.get().max(value)));
            });
        }

        pub(super) struct PeakAlloc;

        // SAFETY: every call goes to `System`; this only counts sizes.
        unsafe impl GlobalAlloc for PeakAlloc {
            unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
                let ptr = unsafe { System.alloc(layout) };
                if !ptr.is_null() {
                    track(layout.size(), 0);
                }
                ptr
            }
            unsafe fn alloc_zeroed(&self, layout: Layout) -> *mut u8 {
                let ptr = unsafe { System.alloc_zeroed(layout) };
                if !ptr.is_null() {
                    track(layout.size(), 0);
                }
                ptr
            }
            unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
                unsafe { System.dealloc(ptr, layout) };
                track(0, layout.size());
            }
            unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
                let new = unsafe { System.realloc(ptr, layout, new_size) };
                if !new.is_null() {
                    track(new_size, layout.size());
                }
                new
            }
        }

        #[global_allocator]
        static ALLOC: PeakAlloc = PeakAlloc;

        /// Peak heap growth on this thread while `run` runs, in bytes.
        pub(super) fn peak_during<T>(run: impl FnOnce() -> T) -> (T, usize) {
            let start = NOW.with(Cell::get);
            PEAK.with(|peak| peak.set(start));
            let out = run();
            let peak = PEAK.with(Cell::get);
            (out, peak.saturating_sub(start))
        }
    }

    const MIB: f64 = 1024.0 * 1024.0;

    /// QA's refused crop (port of their generator): an Adam7 interlaced
    /// RGBA PNG whose every interlace pass repeats one random `period`-byte
    /// block. The upload compresses well, but the de-interlaced pixels have
    /// no repeats inside deflate's window, so the 8-bit re-encode is about
    /// raw size and over the limit at both levels. `period` shrinks until
    /// the upload fits `upload_limit`.
    fn adam7_amplifier(width: u32, height: u32, depth: u8, upload_limit: usize) -> Vec<u8> {
        use std::io::Write;
        const PASSES: [(u32, u32, u32, u32); 7] = [
            (0, 0, 8, 8),
            (4, 0, 8, 8),
            (0, 4, 4, 8),
            (2, 0, 4, 4),
            (0, 2, 2, 4),
            (1, 0, 2, 2),
            (0, 1, 1, 2),
        ];
        let bytes_per_pixel = 4 * usize::from(depth) / 8;
        let mut period = 32_000usize;
        loop {
            let mut state = 0x1234_5678u32;
            let mut stream = Vec::new();
            for (x0, y0, dx, dy) in PASSES {
                let pass_w = (x0..width).step_by(dx as usize).count();
                let pass_h = (y0..height).step_by(dy as usize).count();
                if pass_w == 0 || pass_h == 0 {
                    continue;
                }
                let block: Vec<u8> = (0..period)
                    .map(|_| {
                        state ^= state << 13;
                        state ^= state >> 17;
                        state ^= state << 5;
                        state as u8
                    })
                    .collect();
                let row = pass_w * bytes_per_pixel + 1;
                let start = stream.len();
                stream.extend(block.iter().cycle().take(pass_h * row));
                for r in 0..pass_h {
                    stream[start + r * row] = 0; // filter: none
                }
            }
            let mut z = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::best());
            z.write_all(&stream).unwrap();
            let mut ihdr = width.to_be_bytes().to_vec();
            ihdr.extend_from_slice(&height.to_be_bytes());
            ihdr.extend_from_slice(&[depth, 6, 0, 0, 1]); // RGBA, Adam7
            let mut png = PNG_SIGNATURE.to_vec();
            png.extend(png_chunk(b"IHDR", &ihdr));
            png.extend(png_chunk(b"IDAT", &z.finish().unwrap()));
            png.extend(png_chunk(b"IEND", &[]));
            if png.len() <= upload_limit {
                return png;
            }
            period -= 4_000;
        }
    }

    #[test]
    fn a_cap_inside_the_closing_iend_is_over_the_limit() {
        // png writes IEND (12 bytes) when its writer drops and ignores an
        // error there; every cap short of the full size must be OverLimit,
        // never a PNG without IEND.
        let image = blocks_16bit(false);
        let eight = to_eight_bit(image);
        let full = encode_at(&eight, CompressionType::Default);
        assert!(full.ends_with(&png_chunk(b"IEND", &[])));
        for short in 1..=13 {
            assert!(
                matches!(
                    encode_png(&eight, CompressionType::Default, full.len() - short),
                    Encoded::OverLimit
                ),
                "cap {short} bytes short"
            );
        }
        assert!(matches!(
            encode_png(&eight, CompressionType::Default, full.len()),
            Encoded::Fits(bytes) if bytes == full
        ));
    }

    #[test]
    fn the_chunk_walk_refuses_a_chunk_past_the_end_and_a_late_ihdr() {
        let png = palette_png(8, 8, &RED_AND_BLACK, None, &[]);
        // Cut inside the first IDAT's data: its 8-byte header is whole, its
        // data and CRC are not.
        let first_idat = 8 + 25 + 12 + RED_AND_BLACK.len();
        assert_eq!(&png[first_idat + 4..first_idat + 8], b"IDAT");
        assert!(
            png_pixel_chunks(&png[..first_idat + 8 + 2]).is_none(),
            "an IDAT that runs past the end"
        );
        // IHDR present, but not first: a tEXt chunk before it.
        let mut late_ihdr = PNG_SIGNATURE.to_vec();
        late_ihdr.extend(png_chunk(b"tEXt", b"Comment\0first"));
        late_ihdr.extend_from_slice(&png[8..]);
        assert!(
            png_pixel_chunks(&late_ihdr).is_none(),
            "IHDR must be the first chunk"
        );
        assert_eq!(clean_png(&late_ihdr).err(), Some(HERO_ICON_ERR_INVALID_PNG));
    }

    #[test]
    fn a_short_ihdr_is_refused_without_reading_past_it() {
        for len in [0usize, 4, 12] {
            let mut png = PNG_SIGNATURE.to_vec();
            png.extend(png_chunk(b"IHDR", &vec![0; len]));
            png.extend(png_chunk(b"IEND", &[]));
            assert!(png_pixel_chunks(&png).is_none(), "IHDR of {len} bytes");
            assert_eq!(clean_png(&png).err(), Some(HERO_ICON_ERR_INVALID_PNG));
        }
    }

    /// The re-encode as it was before the capped writer: the default-level
    /// output kept while Best ran. Only used to compare peaks.
    fn reencode_uncapped_like_before(image: &DynamicImage, max_bytes: usize) -> usize {
        let default = encode_at(image, CompressionType::Default);
        if default.len() <= max_bytes {
            return default.len();
        }
        let best = encode_at(image, CompressionType::Best);
        best.len() + default.len()
    }

    /// A stand-in for how the previous version of this PR read a PNG: one
    /// decoder over the whole upload (profile included), its
    /// `icc_profile()` copy, then the decode. Only used to compare peaks.
    fn read_whole_upload_like_before(data: &[u8]) {
        use image::ImageDecoder;
        let mut decoder = png_reader(data).into_decoder().unwrap();
        let _profile = decoder.icc_profile();
        let _ = DynamicImage::from_decoder(decoder);
    }

    #[test]
    fn one_png_read_stays_within_its_measured_peak() {
        // QA's two profile cases at the rank bound, the largest rank crop
        // (1536x720 16-bit RGBA, the most pixel memory any kind accepts),
        // and the largest hero icon (256x256 16-bit RGBA).
        let bomb_40 = iccp_png(1536, 720, 40 * 1024 * 1024);
        let near_cap_31 = iccp_png(1536, 720, 31 * 1024 * 1024);
        let largest_rank = encode(blocks_16bit(true));
        let largest_icon = encode(DynamicImage::ImageRgba16(image::ImageBuffer::from_fn(
            256,
            256,
            |x, y| image::Rgba([(x * 257) as u16, (y * 257) as u16, 9000, 65535]),
        )));
        let mut report = Vec::new();
        for (name, upload, (w, h, bytes)) in [
            ("40 MiB profile", &bomb_40, (1536, 720, 384 * 1024)),
            ("31 MiB profile", &near_cap_31, (1536, 720, 384 * 1024)),
            (
                "rank 1536x720 RGBA16",
                &largest_rank,
                (1536, 720, 384 * 1024),
            ),
            (
                "hero icon 256x256 RGBA16",
                &largest_icon,
                (256, 256, usize::MAX),
            ),
        ] {
            let (result, now) = heap::peak_during(|| clean_png_within(upload, w, h, bytes));
            assert!(result.is_ok(), "{name} is stored");
            let ((), before) = heap::peak_during(|| read_whole_upload_like_before(upload));
            report.push(format!(
                "{name} ({} B upload): peak heap {:.1} MiB now, {:.1} MiB with the earlier whole-upload read",
                upload.len(),
                now as f64 / MIB,
                before as f64 / MIB
            ));
            // The bound the docs state: one whole clean_png_within call,
            // decode and re-encode, stays under 20 MiB for each of these
            // four, the largest rank crop included.
            assert!(now < 20 * 1024 * 1024, "{name}: {now} bytes");
        }
        // QA's refused crop: 1536x720 Adam7 RGBA, within the rank upload
        // limit, re-encodes to about 4.4 MiB at both levels, so it is 413.
        let refused = adam7_amplifier(1536, 720, 8, 384 * 1024);
        let (result, now) = heap::peak_during(|| clean_png_within(&refused, 1536, 720, 384 * 1024));
        assert_eq!(
            result.err(),
            Some(HERO_ICON_ERR_TOO_LARGE),
            "refused at both levels"
        );
        let decoded = png_reader(&refused).decode().unwrap();
        let eight = to_eight_bit(decoded);
        let ((), before) = heap::peak_during(|| {
            let _ = reencode_uncapped_like_before(&eight, 384 * 1024);
        });
        let eight_bytes = eight.as_bytes().len();
        drop(eight);
        report.push(format!(
            "refused rank 1536x720 Adam7 RGBA ({} B upload): peak heap {:.1} MiB now; \
             the two encodes alone, uncapped as before, {:.1} MiB on top of the {:.1} MiB image",
            refused.len(),
            now as f64 / MIB,
            before as f64 / MIB,
            eight_bytes as f64 / MIB
        ));
        // Asserted bound for a refused crop, the largest case measured.
        assert!(now < 20 * 1024 * 1024, "refused crop: {now} bytes");
        // The same attack at the hero icon bound (256x256, 64 KiB).
        let refused_icon = adam7_amplifier(256, 256, 8, 64 * 1024);
        let (result, now) =
            heap::peak_during(|| clean_png_within(&refused_icon, 256, 256, 64 * 1024));
        assert_eq!(result.err(), Some(HERO_ICON_ERR_TOO_LARGE));
        report.push(format!(
            "refused hero icon 256x256 Adam7 RGBA ({} B upload): peak heap {:.1} MiB now",
            refused_icon.len(),
            now as f64 / MIB
        ));
        assert!(now < 4 * 1024 * 1024, "refused hero icon: {now} bytes");
        println!("{}", report.join("\n"));
        // The profile no longer counts: both profile cases cost what the
        // same pixels cost without one.
        let plain = encode(DynamicImage::ImageRgb8(image::RgbImage::from_pixel(
            1536,
            720,
            image::Rgb([20, 120, 220]),
        )));
        let (_, plain_peak) = heap::peak_during(|| clean_png_within(&plain, 1536, 720, 384 * 1024));
        let (_, bomb_peak) =
            heap::peak_during(|| clean_png_within(&bomb_40, 1536, 720, 384 * 1024));
        assert!(
            bomb_peak < plain_peak + 1024 * 1024,
            "{bomb_peak} vs {plain_peak}"
        );
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
