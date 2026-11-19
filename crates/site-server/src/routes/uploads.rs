use std::io::Cursor;
use std::path::Path;

use axum::{
    Json,
    extract::{Multipart, Query, State},
    http::StatusCode,
};
use serde::{Deserialize, Serialize};

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType};

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::routes::hero_icons::{PNG_SIGNATURE, decode_limits};
use crate::state::AppState;
use crate::uploads::{UploadError, save_upload, sniff_image_ext};

const AVATAR_MAX_BYTES: usize = 2 * 1024 * 1024; // 2 MB
const IMAGE_MAX_BYTES: usize = 5 * 1024 * 1024; // 5 MB

// ─── Per-member upload quota (DR1-ADMIN-001) ────────────────────────────────
// Uploads are scoped to a per-member sub-directory (`{category}/{member_key}`)
// so usage is attributable and boundable. These caps apply *per member, per
// category*; combined with delete-on-replace for avatars this bounds disk
// growth from any single (officer-approved, attributable) member.
const MEMBER_MAX_UPLOAD_FILES: usize = 60;
const MEMBER_MAX_UPLOAD_BYTES: u64 = 25 * 1024 * 1024; // 25 MB

// ─── Decompression-bomb / pixel-flood guard (DR1-ADMIN-003) ─────────────────
// Read only the image *header* to learn the declared dimensions (cheap — no
// full decode / no gigapixel allocation) and reject anything that could force a
// client renderer to allocate a huge bitmap.
const MAX_IMAGE_EDGE: u32 = 10_000; // max width or height, px
const MAX_IMAGE_PIXELS: u64 = 40_000_000; // ~40 MP total canvas

type Reject = (StatusCode, Json<ErrorResponse>);

fn reject(code: StatusCode, msg: impl Into<String>) -> Reject {
    (code, Json(ErrorResponse { error: msg.into() }))
}

#[derive(Serialize)]
pub struct UploadResponse {
    pub url: String,
}

/// Map the (server-controlled) member id to a filesystem-safe directory key.
///
/// Member ids are server-generated record ids (e.g. `member:abc123`); we still
/// sanitize defensively so a key can never contain path separators or `..`.
pub(crate) fn member_dir_key(member_id: &str) -> String {
    let key: String = member_id
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '_'
            }
        })
        .collect();
    if key.is_empty() {
        "unknown".to_string()
    } else {
        key
    }
}

/// Header-only dimension guard. Decodes just enough to read the declared
/// canvas size (no pixel data), then rejects oversized / decompression-bomb
/// images before anything is persisted. The magic-byte sniff in `save_upload`
/// still runs afterwards.
///
/// A PNG first goes through [`check_png_chunks`] (colour profile bound,
/// critical chunks, cut-short files). The header read then runs under
/// [`decode_limits`]; for a PNG header read that cap bounds only how far png
/// inflates an `iCCP` profile, and the chunk check has already refused any
/// profile over [`PNG_ICCP_MAX_BYTES`].
///
/// AVIF is read with `imagesize` (container header only). The `image` crate
/// is built without AVIF, so it cannot size one.
fn check_image_dimensions(data: &[u8]) -> Result<(), Reject> {
    let (w, h) = if sniff_image_ext(data) == Some("avif") {
        avif_dimensions(data)?
    } else {
        check_png_chunks(data)?;
        let mut reader = image::ImageReader::new(Cursor::new(data))
            .with_guessed_format()
            .map_err(|_| reject(StatusCode::BAD_REQUEST, "Could not read image"))?;
        // Same 32 MiB allocation cap as training crops (#691), as a
        // backstop: without it png may inflate a colour profile up to the
        // image crate's 512 MiB default during the header read.
        reader.limits(decode_limits());

        reader
            .into_dimensions()
            .map_err(|_| unreadable_dimensions())?
    };

    if w > MAX_IMAGE_EDGE || h > MAX_IMAGE_EDGE || (w as u64) * (h as u64) > MAX_IMAGE_PIXELS {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            format!("Image dimensions too large (max {MAX_IMAGE_EDGE}px per side)"),
        ));
    }

    Ok(())
}

fn unreadable_dimensions() -> Reject {
    reject(
        StatusCode::BAD_REQUEST,
        "Invalid or unreadable image dimensions",
    )
}

/// Most that the `iCCP` colour profile of a PNG may inflate to: 4 MiB.
/// Real profiles are a few KiB.
const PNG_ICCP_MAX_BYTES: u64 = 4 * 1024 * 1024;

/// Chunk checks for a PNG that is stored as uploaded (avatars and article
/// images are not re-encoded, so what is checked here is what `/uploads`
/// serves; #691). Walks the chunk headers like
/// [`crate::routes::hero_icons`]'s `png_pixel_chunks`, keeping nothing, and
/// refuses with 400:
///
/// - an `iCCP` profile that inflates past [`PNG_ICCP_MAX_BYTES`] ("colour
///   profile too large"). It is inflated into a fixed-size buffer, so the
///   memory cost does not grow with the profile;
/// - an `iCCP` profile whose zlib stream is corrupt or cut short: the
///   2-byte zlib header must be valid and the deflate data must reach its
///   end. The Adler-32 trailer is not required, since some exporters omit
///   it (Figma Display P3, lovell/sharp#4128) and libpng still reads such a
///   profile; bytes after the deflate end are ignored;
/// - a second `iCCP` (PNG allows one, and each costs a decoder). It is
///   refused before it is inflated;
/// - a critical chunk (uppercase first letter) whose CRC is wrong, or whose
///   type is not `IHDR`, `PLTE`, `IDAT` or `IEND`. Bad ancillary CRCs are
///   left to the browser, which ignores such a chunk;
/// - a file cut short: a chunk that runs past the end, or no `IEND`.
///
/// Bytes after the first `IEND` are ignored. Not a PNG: nothing to check
/// here, the image read decides.
fn check_png_chunks(data: &[u8]) -> Result<(), Reject> {
    use flate2::{Decompress, FlushDecompress, Status};
    if !data.starts_with(&PNG_SIGNATURE) {
        return Ok(());
    }
    let unreadable = || reject(StatusCode::BAD_REQUEST, "Could not read image");
    let mut seen_iccp = false;
    let mut at = PNG_SIGNATURE.len();
    loop {
        let header = data.get(at..at + 8).ok_or_else(unreadable)?;
        let len = u32::from_be_bytes([header[0], header[1], header[2], header[3]]) as usize;
        let chunk = at
            .checked_add(12)
            .and_then(|end| end.checked_add(len))
            .and_then(|end| data.get(at..end))
            .ok_or_else(unreadable)?;
        let kind = &header[4..8];
        let body = &chunk[8..8 + len];
        if kind[0] & 0x20 == 0 {
            if !matches!(kind, b"IHDR" | b"PLTE" | b"IDAT" | b"IEND") {
                return Err(unreadable());
            }
            let mut crc = flate2::Crc::new();
            crc.update(&chunk[4..8 + len]);
            if crc.sum().to_be_bytes() != chunk[8 + len..] {
                return Err(unreadable());
            }
        }
        if kind == b"iCCP" {
            if std::mem::replace(&mut seen_iccp, true) {
                return Err(unreadable());
            }
            // Profile name (1-79 bytes), NUL, compression method 0, zlib.
            let zlib = body
                .iter()
                .take(80)
                .position(|&b| b == 0)
                .filter(|&nul| nul > 0 && body.get(nul + 1) == Some(&0))
                .map(|nul| &body[nul + 2..])
                .ok_or_else(unreadable)?;
            // zlib header (RFC 1950): deflate with a window of at most
            // 32 KiB, a valid check value, no preset dictionary. The raw
            // deflate data follows; the Adler-32 after it is not required.
            let stream = match zlib {
                [cmf, flg, rest @ ..]
                    if cmf & 0x0f == 8
                        && cmf >> 4 <= 7
                        && flg & 0x20 == 0
                        && (u16::from(*cmf) << 8 | u16::from(*flg)) % 31 == 0 =>
                {
                    rest
                }
                _ => return Err(unreadable()),
            };
            let mut z = Decompress::new(false);
            let mut sink = [0u8; 8 * 1024];
            loop {
                let (fed, made) = (z.total_in(), z.total_out());
                let status = z
                    .decompress(&stream[fed as usize..], &mut sink, FlushDecompress::None)
                    .map_err(|_| unreadable())?;
                if z.total_out() > PNG_ICCP_MAX_BYTES {
                    return Err(reject(
                        StatusCode::BAD_REQUEST,
                        format!(
                            "Image colour profile too large (max {} MB)",
                            PNG_ICCP_MAX_BYTES / (1024 * 1024)
                        ),
                    ));
                }
                match status {
                    Status::StreamEnd => break,
                    // No progress: the input ran out before the deflate end.
                    _ if z.total_in() == fed && z.total_out() == made => {
                        return Err(unreadable());
                    }
                    _ => {}
                }
            }
        }
        at += chunk.len();
        if kind == b"IEND" {
            return Ok(());
        }
    }
}

/// Declared AVIF canvas from the HEIF property boxes (`meta` / `iprp` /
/// `ipco` / `ispe`). No AV1 data is decoded. When there are several `ispe`
/// boxes, `imagesize` reports the largest by area, which is the one the
/// pixel cap needs. A header it cannot parse, or one that declares a zero
/// side, is refused with the same error as any other unreadable image.
fn avif_dimensions(data: &[u8]) -> Result<(u32, u32), Reject> {
    let size = imagesize::ImageType::Heif(imagesize::Compression::Av1)
        .reader_size(&mut Cursor::new(data))
        .map_err(|_| unreadable_dimensions())?;
    if size.width == 0 || size.height == 0 {
        return Err(unreadable_dimensions());
    }
    // `ispe` fields are u32, so these only saturate on a platform where usize
    // is narrower, and a saturated side is still over the edge cap.
    let w = u32::try_from(size.width).unwrap_or(u32::MAX);
    let h = u32::try_from(size.height).unwrap_or(u32::MAX);
    Ok((w, h))
}

/// Count files and total bytes already stored in a member's category dir.
/// A missing directory (first upload) reports `(0, 0)`.
async fn dir_usage(dir: &Path) -> (usize, u64) {
    let mut count = 0usize;
    let mut bytes = 0u64;
    if let Ok(mut rd) = tokio::fs::read_dir(dir).await {
        while let Ok(Some(entry)) = rd.next_entry().await {
            if let Ok(md) = entry.metadata().await
                && md.is_file()
            {
                count += 1;
                bytes += md.len();
            }
        }
    }
    (count, bytes)
}

/// Reject with 413 if adding `new_bytes` would push the member over their
/// per-category file-count or storage quota.
async fn enforce_member_quota(category_dir: &Path, new_bytes: u64) -> Result<(), Reject> {
    let (count, bytes) = dir_usage(category_dir).await;
    if count + 1 > MEMBER_MAX_UPLOAD_FILES {
        return Err(reject(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!("Upload quota exceeded (max {MEMBER_MAX_UPLOAD_FILES} files)"),
        ));
    }
    if bytes.saturating_add(new_bytes) > MEMBER_MAX_UPLOAD_BYTES {
        return Err(reject(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "Upload quota exceeded (max {} MB stored)",
                MEMBER_MAX_UPLOAD_BYTES / (1024 * 1024)
            ),
        ));
    }
    Ok(())
}

/// Public URL prefix for one member's avatar files: `/uploads/avatars/<key>/`.
pub(crate) fn member_avatar_prefix(member_id: &str) -> String {
    format!("/uploads/avatars/{}/", member_dir_key(member_id))
}

/// Accept an avatar URL on profile update.
///
/// `Ok(None)` clears the field (empty / whitespace). `Ok(Some)` is either an
/// external `https` URL or a path under **this** member's `avatars/<key>/`
/// folder. Anything else — another member's upload, an officer image, a
/// traversal, `http` — is rejected.
pub(crate) fn normalize_avatar_url(
    member_id: &str,
    raw: &str,
) -> Result<Option<String>, &'static str> {
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    if trimmed.chars().any(|c| c.is_control() || c.is_whitespace()) {
        return Err(
            "avatar_url must be empty, an https URL, or a file under your own avatars folder",
        );
    }
    if trimmed
        .get(..8)
        .is_some_and(|prefix| prefix.eq_ignore_ascii_case("https://"))
    {
        let rest = &trimmed[8..];
        if rest.is_empty() || rest.contains('\\') {
            return Err(
                "avatar_url must be empty, an https URL, or a file under your own avatars folder",
            );
        }
        return Ok(Some(trimmed.to_string()));
    }
    let prefix = member_avatar_prefix(member_id);
    if let Some(rel) = trimmed.strip_prefix(&prefix)
        && upload_rel_is_safe(rel)
    {
        return Ok(Some(trimmed.to_string()));
    }
    Err("avatar_url must be empty, an https URL, or a file under your own avatars folder")
}

/// Relative path under an uploads folder (`avatars/<key>/`, `images/`).
/// Rejects empty segments, `.`, `..`, and characters that would let a later
/// join escape the folder.
pub(crate) fn upload_rel_is_safe(rel: &str) -> bool {
    if rel.is_empty()
        || rel.contains('\\')
        || rel.contains('\0')
        || rel.contains('?')
        || rel.contains('#')
        || rel.contains('%')
    {
        return false;
    }
    let mut any = false;
    for seg in rel.split('/') {
        if seg.is_empty() || seg == "." || seg == ".." {
            return false;
        }
        if !seg
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.')
        {
            return false;
        }
        any = true;
    }
    any
}

/// Delete `member_id`'s previous avatar file, and nothing else.
///
/// The stored URL must sit under `avatars/<this member's key>/`. The path is
/// canonicalized; `..`, a symlink anywhere along the path, or a resolved
/// location outside that folder is left untouched. Callers pass the folder
/// owner (the upload target), never some other member's id.
async fn delete_replaced_avatar(upload_dir: &Path, member_id: &str, url: &str) {
    let Some(path) = owned_avatar_file(upload_dir, member_id, url).await else {
        if url.starts_with("/uploads/") {
            tracing::warn!(
                url = %url,
                member_id = %member_id,
                "refusing to delete avatar outside this member's own folder"
            );
        }
        return;
    };
    match tokio::fs::remove_file(&path).await {
        Ok(()) => tracing::debug!(path = %path.display(), "deleted replaced avatar"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(path = %path.display(), "replaced avatar already gone");
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to delete replaced avatar")
        }
    }
}

async fn owned_avatar_file(
    upload_dir: &Path,
    member_id: &str,
    url: &str,
) -> Option<std::path::PathBuf> {
    let prefix = member_avatar_prefix(member_id);
    let rel = url.strip_prefix(&prefix)?;
    if !upload_rel_is_safe(rel) {
        return None;
    }
    let base = upload_dir.join("avatars").join(member_dir_key(member_id));
    let candidate = base.join(rel);
    if path_contains_symlink(&candidate).await {
        return None;
    }
    let canon_base = tokio::fs::canonicalize(&base).await.ok()?;
    let canon_file = tokio::fs::canonicalize(&candidate).await.ok()?;
    if !canon_file.starts_with(&canon_base) || canon_file == canon_base {
        return None;
    }
    let meta = tokio::fs::symlink_metadata(&canon_file).await.ok()?;
    if !meta.is_file() {
        return None;
    }
    Some(canon_file)
}

/// True when any existing component of `path` is a symlink.
async fn path_contains_symlink(path: &Path) -> bool {
    let mut cur = std::path::PathBuf::new();
    for comp in path.components() {
        use std::path::Component;
        match comp {
            Component::Prefix(_) | Component::RootDir => cur.push(comp),
            Component::CurDir => {}
            Component::ParentDir => {
                cur.pop();
            }
            Component::Normal(seg) => {
                cur.push(seg);
                if let Ok(meta) = tokio::fs::symlink_metadata(&cur).await
                    && meta.file_type().is_symlink()
                {
                    return true;
                }
            }
        }
    }
    false
}

/// Map a `save_upload` failure onto an HTTP response, preserving the specific
/// (already-authored) message and coding an IO fault as 500.
fn map_save_error(e: UploadError) -> Reject {
    match e {
        UploadError::FileTooLarge { .. } => reject(StatusCode::PAYLOAD_TOO_LARGE, e.to_string()),
        UploadError::InvalidContentType => reject(StatusCode::BAD_REQUEST, e.to_string()),
        UploadError::IoError(_) => {
            tracing::error!(error = %e, "upload IO error");
            reject(StatusCode::INTERNAL_SERVER_ERROR, "Internal error")
        }
    }
}

/// Read the first multipart field's declared content-type and raw bytes.
async fn read_upload_field(multipart: &mut Multipart) -> Result<(String, Vec<u8>), Reject> {
    let field = multipart
        .next_field()
        .await
        .map_err(|e| reject(StatusCode::BAD_REQUEST, format!("Invalid multipart: {e}")))?
        .ok_or_else(|| reject(StatusCode::BAD_REQUEST, "No file provided"))?;

    let content_type = field
        .content_type()
        .unwrap_or("application/octet-stream")
        .to_string();

    let data = field
        .bytes()
        .await
        .map_err(|e| reject(StatusCode::BAD_REQUEST, format!("Failed to read file: {e}")))?;

    Ok((content_type, data.to_vec()))
}

/// Shared upload flow: validate dimensions, enforce the per-member quota, then
/// persist into the member's category sub-directory. Returns the public URL.
async fn store_member_upload(
    state: &AppState,
    member_id: &str,
    category: &str,
    content_type: &str,
    data: &[u8],
    max_bytes: usize,
) -> Result<String, Reject> {
    // Cheap early byte-cap (also enforced authoritatively in save_upload).
    if data.len() > max_bytes {
        return Err(reject(
            StatusCode::PAYLOAD_TOO_LARGE,
            format!(
                "File too large. Maximum size is {} MB",
                max_bytes / (1024 * 1024)
            ),
        ));
    }

    // Decompression-bomb / pixel-flood guard (before persisting).
    check_image_dimensions(data)?;

    let key = member_dir_key(member_id);
    let scoped_category = format!("{category}/{key}");
    let category_dir = state.upload_dir.join(&scoped_category);

    // Per-member quota check (before writing).
    enforce_member_quota(&category_dir, data.len() as u64).await?;

    save_upload(
        &state.upload_dir,
        &scoped_category,
        data,
        content_type,
        max_bytes,
    )
    .await
    .map_err(map_save_error)
}

#[derive(Deserialize)]
pub struct AvatarUploadQuery {
    /// Member record key, same form as `PUT /api/members/{mid}`.
    ///
    /// Absent, blank, or equal to the caller: the upload is the caller's own
    /// avatar. Any other id stores the file for that member, which requires
    /// the same permission as editing their profile.
    pub member_id: Option<String>,
}

struct AvatarFolder {
    id: String,
    avatar_url: Option<String>,
}

/// Who owns the avatar folder for this upload.
///
/// Absent or the caller's own record key keeps today's behaviour: the caller's
/// folder and the caller's current `avatar_url`. Another member is loaded with
/// [`crate::routes::members::require_profile_edit`] (404 unknown, 403 when the
/// caller cannot edit that profile) and the file is stored under that member.
async fn avatar_folder_for_upload(
    state: &AppState,
    caller: &OrgMember,
    member_id: Option<&str>,
) -> Result<AvatarFolder, Reject> {
    let Some(raw) = member_id.map(str::trim).filter(|id| !id.is_empty()) else {
        return Ok(AvatarFolder {
            id: caller.member.id.clone(),
            avatar_url: caller.member.avatar_url.clone(),
        });
    };
    if raw == caller.member.id {
        return Ok(AvatarFolder {
            id: caller.member.id.clone(),
            avatar_url: caller.member.avatar_url.clone(),
        });
    }
    let target = crate::routes::members::require_profile_edit(state, caller, raw).await?;
    if target.user_id == caller.user.id {
        return Ok(AvatarFolder {
            id: caller.member.id.clone(),
            avatar_url: caller.member.avatar_url.clone(),
        });
    }
    Ok(AvatarFolder {
        id: target.id,
        avatar_url: target.avatar_url,
    })
}

/// POST /api/upload/avatar — upload an avatar (org member).
///
/// Query `member_id` (optional) is the member record key, the same form as
/// `PUT /api/members/{mid}`. The admin Members modal sends it so an officer
/// can set another member's avatar.
///
/// - Absent, blank, or equal to the caller: save into `avatars/<caller key>/`
///   and delete-on-replace the caller's current avatar file.
/// - Another member: same permission as `PUT /api/members/{mid}` for that
///   profile (self, an admin, or an officer editing a member or recruit).
///   Otherwise 403 `{ "error": "..." }`. Unknown
///   member 404. The file is saved into `avatars/<target key>/` and
///   delete-on-replace runs against the target's current `avatar_url`, only
///   inside the target's folder.
pub async fn upload_avatar(
    State(state): State<AppState>,
    member: OrgMember,
    Query(query): Query<AvatarUploadQuery>,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, Reject> {
    let folder = avatar_folder_for_upload(&state, &member, query.member_id.as_deref()).await?;

    let (content_type, data) = read_upload_field(&mut multipart).await?;

    let url = store_member_upload(
        &state,
        &folder.id,
        "avatars",
        &content_type,
        &data,
        AVATAR_MAX_BYTES,
    )
    .await?;

    // Delete-on-replace: only the folder owner's previous avatar file.
    // A stored URL that points anywhere else (another member, an officer
    // image, a symlink out of the folder) is left on disk.
    if let Some(prev) = folder.avatar_url.as_deref()
        && prev != url
    {
        delete_replaced_avatar(&state.upload_dir, &folder.id, prev).await;
    }

    audit(
        &state.db,
        &member.member.id,
        AuditAction::UploadedAvatar,
        AuditTargetType::Upload,
        &url,
        None,
    )
    .await;

    Ok(Json(UploadResponse { url }))
}

/// POST /api/upload/image — upload general image (officer+)
pub async fn upload_image(
    State(state): State<AppState>,
    officer: OfficerUser,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, Reject> {
    let (content_type, data) = read_upload_field(&mut multipart).await?;

    let url = store_member_upload(
        &state,
        &officer.member.id,
        "images",
        &content_type,
        &data,
        IMAGE_MAX_BYTES,
    )
    .await?;

    audit(
        &state.db,
        &officer.member.id,
        AuditAction::UploadedImage,
        AuditTargetType::Upload,
        &url,
        None,
    )
    .await;

    Ok(Json(UploadResponse { url }))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::SocketAddr;

    use axum::body::Body;
    use axum::extract::ConnectInfo;
    use axum::http::{Request, StatusCode};
    use tower::ServiceExt;

    /// Standard CRC-32 (IEEE, poly 0xEDB88320) for crafting valid PNG chunks.
    fn crc32(bytes: &[u8]) -> u32 {
        let mut crc: u32 = 0xFFFF_FFFF;
        for &b in bytes {
            crc ^= b as u32;
            for _ in 0..8 {
                let mask = (crc & 1).wrapping_neg();
                crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
            }
        }
        !crc
    }

    /// Build a minimal, structurally-valid PNG (signature + IHDR + IEND) that
    /// *declares* the given dimensions. Enough for a header-only dimension read.
    fn png_with_dims(w: u32, h: u32) -> Vec<u8> {
        let mut out = vec![0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n'];

        // IHDR chunk data: width, height, bit depth 8, color type 2 (RGB), rest 0.
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(b"IHDR");
        ihdr.extend_from_slice(&w.to_be_bytes());
        ihdr.extend_from_slice(&h.to_be_bytes());
        ihdr.extend_from_slice(&[8, 2, 0, 0, 0]);
        out.extend_from_slice(&(13u32).to_be_bytes());
        out.extend_from_slice(&ihdr);
        out.extend_from_slice(&crc32(&ihdr).to_be_bytes());

        // Minimal IDAT chunk. `into_dimensions` requires an IDAT to be present
        // but does NOT inflate it, so a tiny zlib stream suffices even for a
        // PNG that *declares* gigapixel dimensions.
        let mut idat = Vec::new();
        idat.extend_from_slice(b"IDAT");
        idat.extend_from_slice(&[0x78, 0x9c, 0x63, 0x00, 0x00, 0x00, 0x01, 0x00, 0x01]);
        out.extend_from_slice(&((idat.len() - 4) as u32).to_be_bytes());
        out.extend_from_slice(&idat);
        out.extend_from_slice(&crc32(&idat).to_be_bytes());

        // IEND chunk (empty).
        out.extend_from_slice(&(0u32).to_be_bytes());
        let iend = b"IEND";
        out.extend_from_slice(iend);
        out.extend_from_slice(&crc32(iend).to_be_bytes());

        out
    }

    fn unique_tmp(tag: &str) -> std::path::PathBuf {
        let n = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("scuffed-upl-test-{tag}-{n}"))
    }

    #[test]
    fn dimension_guard_accepts_small_and_rejects_bomb() {
        // 1x1 is fine.
        assert!(check_image_dimensions(&png_with_dims(1, 1)).is_ok());
        // A gigapixel-declaring PNG (well over the edge + pixel caps) is rejected.
        let bomb = png_with_dims(50_000, 50_000);
        let err = check_image_dimensions(&bomb).unwrap_err();
        assert_eq!(err.0, StatusCode::BAD_REQUEST);
        // Just over the edge cap on a single side is rejected too.
        let wide = png_with_dims(MAX_IMAGE_EDGE + 1, 1);
        assert_eq!(
            check_image_dimensions(&wide).unwrap_err().0,
            StatusCode::BAD_REQUEST
        );
    }

    #[test]
    fn avif_dimension_guard_reads_header_and_applies_caps() {
        use crate::uploads::test_avif::{TINY_AVIF, avif_with_dims};

        // The real 2x1 fixture passes.
        assert!(check_image_dimensions(TINY_AVIF).is_ok());
        assert!(check_image_dimensions(&avif_with_dims(MAX_IMAGE_EDGE, 1)).is_ok());

        // A header declaring a gigapixel canvas is rejected.
        let bomb = check_image_dimensions(&avif_with_dims(50_000, 50_000)).unwrap_err();
        assert_eq!(bomb.0, StatusCode::BAD_REQUEST);
        assert!(bomb.1.error.contains("too large"), "{}", bomb.1.error);
        // One side over the edge cap.
        let wide = check_image_dimensions(&avif_with_dims(MAX_IMAGE_EDGE + 1, 1)).unwrap_err();
        assert_eq!(wide.0, StatusCode::BAD_REQUEST);
        assert!(wide.1.error.contains("too large"), "{}", wide.1.error);
        // Both sides under the edge cap but over the pixel cap.
        let flood =
            check_image_dimensions(&avif_with_dims(MAX_IMAGE_EDGE, MAX_IMAGE_EDGE)).unwrap_err();
        assert_eq!(flood.0, StatusCode::BAD_REQUEST);
        assert!(flood.1.error.contains("too large"), "{}", flood.1.error);
    }

    #[test]
    fn avif_with_unparseable_header_is_refused_like_other_bad_images() {
        use crate::uploads::test_avif::{TINY_AVIF, avif_with_dims, ftyp};

        let bad_png = check_image_dimensions(&png_with_dims(0, 0)).unwrap_err();
        assert_eq!(bad_png.0, StatusCode::BAD_REQUEST);

        // Sniffs as AVIF but has no `meta` box at all.
        let bare = ftyp(b"avif", &[b"mif1", b"miaf"]);
        // Sniffs as AVIF, then garbage where the boxes should be.
        let mut garbage = ftyp(b"avif", &[b"mif1"]);
        garbage.extend_from_slice(&[0xff; 64]);
        // The real fixture cut off inside its `meta` box.
        let truncated = TINY_AVIF[..80].to_vec();
        // A declared zero side.
        let zero = avif_with_dims(0, 1);

        for (name, data) in [
            ("bare", bare),
            ("garbage", garbage),
            ("truncated", truncated),
            ("zero", zero),
        ] {
            assert_eq!(sniff_image_ext(&data), Some("avif"), "{name}");
            let err = check_image_dimensions(&data).unwrap_err();
            assert_eq!(err.0, bad_png.0, "{name}");
            assert_eq!(err.1.error, bad_png.1.error, "{name}");
        }
    }

    #[test]
    fn dimension_guard_rejects_non_image() {
        assert!(check_image_dimensions(b"not an image at all").is_err());
    }

    /// A 64x64 PNG whose `iCCP` inflates to 64 MiB, in about 64 KB.
    fn iccp_bomb() -> Vec<u8> {
        let bomb = crate::routes::hero_icons::tests::iccp_png(64, 64, 64 * 1024 * 1024);
        assert!(bomb.len() < 128 * 1024, "{}", bomb.len());
        bomb
    }

    #[test]
    fn a_colour_profile_bomb_is_refused_before_the_image_read() {
        use crate::routes::hero_icons::tests::heap::peak_during;
        const MIB: usize = 1024 * 1024;
        let bomb = iccp_bomb();
        // The image crate's default limits (512 MiB) inflate the whole
        // 64 MiB profile during the header read: the #691 bug.
        let (unbounded, unbounded_peak) = peak_during(|| {
            image::ImageReader::new(Cursor::new(&bomb[..]))
                .with_guessed_format()
                .unwrap()
                .into_dimensions()
        });
        assert_eq!(unbounded.unwrap(), (64, 64));
        assert!(
            unbounded_peak >= 64 * MIB,
            "default limits peak {unbounded_peak} B"
        );
        // The backstop alone: under `decode_limits` png stops inflating at
        // 32 MiB and skips the profile (png 0.18 does not fail on it).
        let (capped, capped_peak) = peak_during(|| {
            let mut reader = image::ImageReader::new(Cursor::new(&bomb[..]))
                .with_guessed_format()
                .unwrap();
            reader.limits(decode_limits());
            reader.into_dimensions()
        });
        assert_eq!(capped.unwrap(), (64, 64));
        assert!(capped_peak <= 36 * MIB, "capped peak {capped_peak} B");
        // The chunk check refuses the bomb before the image read, inflating
        // into a sink: the cost does not grow with the profile.
        let (guarded, guarded_peak) = peak_during(|| check_image_dimensions(&bomb));
        let (code, body) = guarded.unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(
            body.error.contains("colour profile too large"),
            "{}",
            body.error
        );
        assert!(
            guarded_peak <= MIB,
            "guarded peak {guarded_peak} B is over 1 MiB"
        );
    }

    #[test]
    fn a_normal_colour_profile_is_accepted() {
        use crate::routes::hero_icons::tests::iccp_png;
        // A 4 KiB profile, the size of a real sRGB one.
        assert!(check_image_dimensions(&iccp_png(64, 64, 4096)).is_ok());
    }

    /// Byte offset of chunk number `index` (0 = IHDR) in `png`.
    fn chunk_at(png: &[u8], index: usize) -> usize {
        let mut at = 8;
        for _ in 0..index {
            at += 12 + u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        }
        at
    }

    /// A chunk with a valid CRC.
    fn chunk(kind: &[u8; 4], body: &[u8]) -> Vec<u8> {
        let mut out = u32::try_from(body.len()).unwrap().to_be_bytes().to_vec();
        out.extend_from_slice(kind);
        out.extend_from_slice(body);
        let mut crc = flate2::Crc::new();
        crc.update(&out[4..]);
        out.extend_from_slice(&crc.sum().to_be_bytes());
        out
    }

    /// `png` with `chunk` inserted before its chunk number `index`.
    fn with_chunk(png: &[u8], index: usize, chunk: &[u8]) -> Vec<u8> {
        let at = chunk_at(png, index);
        let mut out = png[..at].to_vec();
        out.extend_from_slice(chunk);
        out.extend_from_slice(&png[at..]);
        out
    }

    /// `png` with the body of its chunk number `index` passed through `edit`
    /// (length and CRC fixed up).
    fn with_body(png: &[u8], index: usize, edit: impl FnOnce(&[u8]) -> Vec<u8>) -> Vec<u8> {
        let at = chunk_at(png, index);
        let len = u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        let kind: [u8; 4] = png[at + 4..at + 8].try_into().unwrap();
        let mut out = png[..at].to_vec();
        out.extend(chunk(&kind, &edit(&png[at + 8..at + 8 + len])));
        out.extend_from_slice(&png[at + 12 + len..]);
        out
    }

    fn assert_refused(png: &[u8], message: &str, case: &str) {
        let (code, body) = check_image_dimensions(png).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST, "{case}");
        assert!(body.error.contains(message), "{case}: {}", body.error);
    }

    #[test]
    fn the_colour_profile_bound_is_4_mib() {
        use crate::routes::hero_icons::tests::iccp_png;
        let at = PNG_ICCP_MAX_BYTES as usize;
        // The bound is inclusive.
        assert!(check_image_dimensions(&iccp_png(64, 64, at)).is_ok());
        assert_refused(
            &iccp_png(64, 64, at + 1),
            "colour profile too large",
            "4 MiB + 1",
        );
    }

    #[test]
    fn a_second_colour_profile_is_refused() {
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        let profile = png[chunk_at(&png, 1)..chunk_at(&png, 2)].to_vec();
        // Chunks: IHDR, iCCP, IDAT, IEND. A second copy right after the
        // first, and one after the IDAT (which the header read never sees).
        for index in [2, 3] {
            let two = with_chunk(&png, index, &profile);
            assert_refused(&two, "Could not read image", &format!("at {index}"));
        }
    }

    #[test]
    fn a_second_colour_profile_is_refused_before_it_is_inflated() {
        use crate::routes::hero_icons::tests::heap::peak_during;
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        let bomb = iccp_bomb();
        let second = &bomb[chunk_at(&bomb, 1)..chunk_at(&bomb, 2)];
        assert_eq!(&second[4..8], b"iCCP");
        let two = with_chunk(&png, 2, second);
        // Inflating the 64 MiB profile would answer "colour profile too
        // large"; the second-profile rule answers first, in bounded memory.
        let (out, peak) = peak_during(|| check_image_dimensions(&two));
        let (code, body) = out.unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
        assert!(
            body.error.contains("Could not read image"),
            "{}",
            body.error
        );
        assert!(peak <= 1024 * 1024, "peak {peak} B is over 1 MiB");
    }

    #[test]
    fn a_cut_short_profile_stream_is_refused() {
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        // "bomb\0\0", then the zlib stream: header, deflate, Adler-32.
        let whole = with_body(&png, 1, |body| body.to_vec());
        assert_eq!(whole, png);
        assert!(check_image_dimensions(&whole).is_ok());
        // No Adler-32: the deflate data is whole, so it is accepted.
        let no_adler = with_body(&png, 1, |body| body[..body.len() - 4].to_vec());
        assert!(check_image_dimensions(&no_adler).is_ok(), "no Adler-32");
        let half = with_body(&png, 1, |body| body[..6 + (body.len() - 6) / 2].to_vec());
        assert_refused(&half, "Could not read image", "half the stream");
        // A stream that stops right after the zlib header.
        let bare = with_body(&png, 1, |body| body[..8].to_vec());
        assert_refused(&bare, "Could not read image", "header only");
        // A stream that stops inside the zlib header.
        let one = with_body(&png, 1, |body| body[..7].to_vec());
        assert_refused(&one, "Could not read image", "half a header");
    }

    #[test]
    fn a_profile_stream_without_adler32_is_accepted() {
        use std::io::Write;
        // The Figma Display P3 shape (lovell/sharp#4128): a valid zlib
        // header, a complete deflate stream, no Adler-32 trailer.
        let mut deflate =
            flate2::write::DeflateEncoder::new(Vec::new(), flate2::Compression::default());
        deflate.write_all(&[7u8; 3144]).unwrap();
        let mut body = b"Display P3\0\0\x78\x9c".to_vec();
        body.extend(deflate.finish().unwrap());
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        let figma = with_body(&png, 1, |_| body.clone());
        assert!(check_image_dimensions(&figma).is_ok());
        // The same stream under a bad zlib header (check value off by one,
        // or a preset dictionary) is refused.
        // Each header below fails one condition only: 0x7709 is CM 7 and
        // 0x881c is CINFO 8, both with a valid check value and no FDICT.
        for (case, header) in [
            ("check value", [0x78, 0x9d]),
            ("dictionary", [0x78, 0xbb]),
            ("method 7", [0x77, 0x09]),
            ("window 64 KiB", [0x88, 0x1c]),
        ] {
            assert_eq!(u16::from_be_bytes(header) % 31 == 0, case != "check value");
            let mut bad = body.clone();
            bad[12..14].copy_from_slice(&header);
            let bad = with_body(&png, 1, |_| bad);
            assert_refused(&bad, "Could not read image", case);
        }
    }

    #[test]
    fn bytes_after_the_profile_deflate_end_are_ignored() {
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        // "bomb\0\0", then the zlib stream: header, deflate, Adler-32.
        // A wrong Adler-32 is not checked.
        let bad_adler = with_body(&png, 1, |body| {
            let mut body = body.to_vec();
            let at = body.len() - 4;
            body[at] ^= 0xff;
            body
        });
        assert_ne!(bad_adler, png);
        assert!(check_image_dimensions(&bad_adler).is_ok(), "wrong Adler-32");
        // Junk after the Adler-32, and junk in place of it.
        let junk = b"not deflate data at all";
        let after = with_body(&png, 1, |body| [body, &junk[..]].concat());
        assert!(
            check_image_dimensions(&after).is_ok(),
            "junk after Adler-32"
        );
        let instead = with_body(&png, 1, |body| {
            [&body[..body.len() - 4], &junk[..]].concat()
        });
        assert!(
            check_image_dimensions(&instead).is_ok(),
            "junk for Adler-32"
        );
    }

    #[test]
    fn a_cut_short_file_is_refused() {
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        let idat = chunk_at(&png, 2);
        let iend = chunk_at(&png, 3);
        assert_eq!(&png[idat + 4..idat + 8], b"IDAT");
        assert_eq!(png.len(), iend + 12);
        for (case, cut) in [
            ("inside the IDAT body", idat + 8 + (iend - idat - 12) / 2),
            ("inside the IDAT CRC", iend - 2),
            ("no IEND", iend),
            ("inside the IEND header", iend + 5),
            ("inside the IEND CRC", iend + 10),
        ] {
            assert_refused(&png[..cut], "Could not read image", case);
        }
        // Bytes after the IEND are still ignored.
        let mut trailing = png.clone();
        trailing.extend_from_slice(b"trailing bytes");
        assert!(check_image_dimensions(&trailing).is_ok());
    }

    #[test]
    fn an_unknown_critical_chunk_is_refused_and_an_unknown_ancillary_one_is_not() {
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        // Chunks: IHDR, iCCP, IDAT, IEND. Index 3 is after the first IDAT,
        // where the header read never looks.
        for index in [2, 3] {
            let bad = with_chunk(&png, index, &chunk(b"IDOT", b"x"));
            assert_refused(&bad, "Could not read image", &format!("IDOT at {index}"));
            let ok = with_chunk(&png, index, &chunk(b"teSt", b"x"));
            assert!(check_image_dimensions(&ok).is_ok(), "teSt at {index}");
        }
    }

    /// `png` with the CRC of its chunk number `index` (0 = IHDR) flipped.
    fn with_bad_crc(mut png: Vec<u8>, index: usize) -> (Vec<u8>, [u8; 4]) {
        let at = chunk_at(&png, index);
        let kind: [u8; 4] = png[at + 4..at + 8].try_into().unwrap();
        let crc_at = at + 8 + u32::from_be_bytes(png[at..at + 4].try_into().unwrap()) as usize;
        png[crc_at] ^= 0xff;
        (png, kind)
    }

    #[test]
    fn a_bad_critical_chunk_crc_is_refused_and_a_bad_profile_crc_is_not() {
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        assert!(check_image_dimensions(&png).is_ok());
        // Chunks: IHDR, iCCP, IDAT, IEND (the encoder writes one IDAT).
        for index in [0, 2, 3] {
            let (bad, kind) = with_bad_crc(png.clone(), index);
            assert!(kind[0].is_ascii_uppercase(), "{kind:?}");
            let (code, _) = check_image_dimensions(&bad).unwrap_err();
            assert_eq!(code, StatusCode::BAD_REQUEST, "{kind:?}");
        }
        let (bad, kind) = with_bad_crc(png.clone(), 1);
        assert_eq!(&kind, b"iCCP");
        assert!(check_image_dimensions(&bad).is_ok());
    }

    #[test]
    fn a_profile_that_does_not_inflate_is_refused() {
        let png = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        // Corrupt the zlib header of the profile and fix the chunk CRC, so
        // only the inflate can catch it. "bomb\0\0" comes first.
        let len = u32::from_be_bytes(png[33..37].try_into().unwrap()) as usize;
        let mut bad = png.clone();
        bad[41 + 6] ^= 0xff;
        let mut crc = flate2::Crc::new();
        crc.update(&bad[37..41 + len]);
        bad[41 + len..45 + len].copy_from_slice(&crc.sum().to_be_bytes());
        let (code, _) = check_image_dimensions(&bad).unwrap_err();
        assert_eq!(code, StatusCode::BAD_REQUEST);
    }

    #[tokio::test]
    async fn quota_blocks_when_over_byte_cap() {
        let dir = unique_tmp("quota-bytes");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        // Sparse file reporting the full byte cap without writing that many bytes.
        let f = std::fs::File::create(dir.join("big.bin")).unwrap();
        f.set_len(MEMBER_MAX_UPLOAD_BYTES).unwrap();

        // One more byte pushes over the storage cap → 413.
        let err = enforce_member_quota(&dir, 1).await.unwrap_err();
        assert_eq!(err.0, StatusCode::PAYLOAD_TOO_LARGE);

        // A brand-new (empty) member dir is under quota.
        let fresh = unique_tmp("quota-fresh");
        assert!(enforce_member_quota(&fresh, 1024).await.is_ok());

        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn quota_blocks_when_over_file_count() {
        let dir = unique_tmp("quota-files");
        tokio::fs::create_dir_all(&dir).await.unwrap();
        for i in 0..MEMBER_MAX_UPLOAD_FILES {
            tokio::fs::write(dir.join(format!("f{i}.png")), b"x")
                .await
                .unwrap();
        }
        let err = enforce_member_quota(&dir, 1).await.unwrap_err();
        assert_eq!(err.0, StatusCode::PAYLOAD_TOO_LARGE);
        tokio::fs::remove_dir_all(&dir).await.ok();
    }

    #[tokio::test]
    async fn delete_replaced_avatar_only_removes_own_folder_file() {
        let root = unique_tmp("delete");
        let own_id = "member:attacker";
        let own_dir = root.join("avatars").join(member_dir_key(own_id));
        let victim_dir = root.join("images").join("victim");
        tokio::fs::create_dir_all(&own_dir).await.unwrap();
        tokio::fs::create_dir_all(&victim_dir).await.unwrap();
        let own = own_dir.join("old.png");
        let victim = victim_dir.join("secret.png");
        tokio::fs::write(&own, b"old").await.unwrap();
        tokio::fs::write(&victim, b"keep").await.unwrap();

        delete_replaced_avatar(
            &root,
            own_id,
            &format!("/uploads/avatars/{}/old.png", member_dir_key(own_id)),
        )
        .await;
        assert!(!own.exists(), "own previous avatar is removed");
        assert!(victim.exists());

        // Another member's /uploads path, an officer image, and traversal stay.
        delete_replaced_avatar(&root, own_id, "/uploads/images/victim/secret.png").await;
        delete_replaced_avatar(
            &root,
            own_id,
            &format!(
                "/uploads/avatars/{}/../../images/victim/secret.png",
                member_dir_key(own_id)
            ),
        )
        .await;
        delete_replaced_avatar(&root, own_id, "https://cdn.discordapp.com/avatars/x/y.png").await;
        assert!(victim.exists(), "other uploads must survive avatar replace");

        // Symlink inside the member folder pointing at the victim file.
        let link = own_dir.join("link.png");
        std::os::unix::fs::symlink(&victim, &link).unwrap();
        delete_replaced_avatar(
            &root,
            own_id,
            &format!("/uploads/avatars/{}/link.png", member_dir_key(own_id)),
        )
        .await;
        assert!(victim.exists(), "symlink must not delete the target");
        assert!(link.exists(), "symlink itself is not removed");

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[test]
    fn avatar_url_must_be_empty_https_or_own_folder() {
        let id = "member:abc";
        let prefix = member_avatar_prefix(id);
        assert_eq!(normalize_avatar_url(id, "  ").unwrap(), None);
        assert_eq!(
            normalize_avatar_url(id, "https://cdn.example/a.png")
                .unwrap()
                .as_deref(),
            Some("https://cdn.example/a.png")
        );
        let own = format!("{prefix}pic.png");
        assert_eq!(
            normalize_avatar_url(id, &own).unwrap().as_deref(),
            Some(own.as_str())
        );
        assert!(normalize_avatar_url(id, "/uploads/images/officer/x.png").is_err());
        assert!(normalize_avatar_url(id, "/uploads/avatars/member_other/x.png").is_err());
        assert!(normalize_avatar_url(id, &format!("{prefix}../x.png")).is_err());
        assert!(normalize_avatar_url(id, "http://cdn.example/a.png").is_err());
    }

    fn with_peer(builder: axum::http::request::Builder) -> axum::http::request::Builder {
        builder
            .header("x-forwarded-for", "127.0.0.1")
            .extension(ConnectInfo(SocketAddr::from(([127, 0, 0, 1], 9))))
    }

    #[tokio::test]
    async fn foreign_avatar_url_is_rejected_and_upload_does_not_delete_it() {
        let mut state = crate::test_support::test_state().await;
        let root = unique_tmp("avatar-http");
        state.upload_dir = root.clone();
        crate::test_support::seed_user(&state, "attacker", "attacker").await;
        let member = state
            .db
            .create_member("attacker", "Attacker", scuffed_db::OrgRole::Member)
            .await
            .unwrap();
        let token = "avatar-test-token";
        state
            .db
            .create_session("attacker", token, 24)
            .await
            .unwrap();

        let victim = root.join("images/victim/secret.png");
        tokio::fs::create_dir_all(victim.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&victim, b"keep-me").await.unwrap();

        let app = crate::create_router(state.clone());
        let foreign = "/uploads/images/victim/secret.png";
        let put = with_peer(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/members/{}", urlencoding::encode(&member.id)))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json"),
        )
        .body(Body::from(format!(r#"{{"avatar_url":"{foreign}"}}"#)))
        .unwrap();
        let res = app.clone().oneshot(put).await.unwrap();
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        assert!(victim.exists());

        let own = format!("{}ok.png", member_avatar_prefix(&member.id));
        let put_own = with_peer(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/members/{}", urlencoding::encode(&member.id)))
                .header("authorization", format!("Bearer {token}"))
                .header("content-type", "application/json"),
        )
        .body(Body::from(format!(r#"{{"avatar_url":"{own}"}}"#)))
        .unwrap();
        let res = app.clone().oneshot(put_own).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "own avatar path is allowed");

        // A row written before the setter check (or by another bug) must still
        // not be able to aim delete-on-replace at someone else's file.
        state
            .db
            .update_member(
                &member.id,
                None,
                None,
                Some(Some(foreign)),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();

        let png = png_with_dims(1, 1);
        let boundary = "----scuffedboundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(&png);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        let upload = with_peer(
            Request::builder()
                .method("POST")
                .uri("/api/upload/avatar")
                .header("authorization", format!("Bearer {token}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                ),
        )
        .body(Body::from(body))
        .unwrap();
        let res = app.oneshot(upload).await.unwrap();
        assert_eq!(res.status(), StatusCode::OK, "avatar upload should succeed");
        assert!(
            victim.exists(),
            "avatar upload deleted another member's file"
        );
        tokio::fs::remove_dir_all(&root).await.ok();
    }

    fn multipart_of(png: &[u8]) -> (Vec<u8>, &'static str) {
        let boundary = "----scuffedboundary";
        let mut body = Vec::new();
        body.extend_from_slice(
            format!(
                "--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"a.png\"\r\nContent-Type: image/png\r\n\r\n"
            )
            .as_bytes(),
        );
        body.extend_from_slice(png);
        body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
        (body, boundary)
    }

    async fn session_member(
        state: &crate::state::AppState,
        user_id: &str,
        role: scuffed_db::OrgRole,
    ) -> (scuffed_db::Member, String) {
        crate::test_support::seed_user(state, user_id, user_id).await;
        let member = state
            .db
            .create_member(user_id, user_id, role)
            .await
            .unwrap();
        let token = format!("{user_id}-token");
        state.db.create_session(user_id, &token, 24).await.unwrap();
        (member, token)
    }

    async fn plant_avatar(
        root: &Path,
        member_id: &str,
        name: &str,
    ) -> (std::path::PathBuf, String) {
        let dir = root.join("avatars").join(member_dir_key(member_id));
        tokio::fs::create_dir_all(&dir).await.unwrap();
        let path = dir.join(name);
        tokio::fs::write(&path, b"previous-avatar").await.unwrap();
        let url = format!("{}{name}", member_avatar_prefix(member_id));
        (path, url)
    }

    async fn set_avatar_url(state: &crate::state::AppState, member_id: &str, url: &str) {
        state
            .db
            .update_member(
                member_id,
                None,
                None,
                Some(Some(url)),
                None,
                None,
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await
            .unwrap();
    }

    fn files_in(dir: &Path) -> Vec<String> {
        let mut names = Vec::new();
        if let Ok(rd) = std::fs::read_dir(dir) {
            for entry in rd.flatten() {
                if entry.path().is_file() {
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
        }
        names.sort();
        names
    }

    async fn post_avatar(
        app: &axum::Router,
        token: &str,
        member_id: Option<&str>,
    ) -> axum::response::Response {
        post_avatar_png(app, token, member_id, &png_with_dims(1, 1)).await
    }

    async fn post_avatar_png(
        app: &axum::Router,
        token: &str,
        member_id: Option<&str>,
        png: &[u8],
    ) -> axum::response::Response {
        let (body, boundary) = multipart_of(png);
        let uri = match member_id {
            None => "/api/upload/avatar".to_string(),
            Some(id) => format!("/api/upload/avatar?member_id={}", urlencoding::encode(id)),
        };
        let req = with_peer(
            Request::builder()
                .method("POST")
                .uri(uri)
                .header("authorization", format!("Bearer {token}"))
                .header(
                    "content-type",
                    format!("multipart/form-data; boundary={boundary}"),
                ),
        )
        .body(Body::from(body))
        .unwrap();
        app.clone().oneshot(req).await.unwrap()
    }

    async fn response_json(res: axum::response::Response) -> serde_json::Value {
        use http_body_util::BodyExt;
        let bytes = res.into_body().collect().await.unwrap().to_bytes();
        serde_json::from_slice(&bytes).unwrap_or(serde_json::Value::Null)
    }

    #[tokio::test]
    async fn officer_upload_for_member_then_put_keeps_officer_file() {
        let mut state = crate::test_support::test_state().await;
        let root = unique_tmp("officer-for-member");
        state.upload_dir = root.clone();
        let (officer, officer_token) =
            session_member(&state, "officerava", scuffed_db::OrgRole::Officer).await;
        let (member, _) = session_member(&state, "playerava", scuffed_db::OrgRole::Member).await;

        let (officer_file, officer_url) = plant_avatar(&root, &officer.id, "officer.png").await;
        let (member_file, member_url) = plant_avatar(&root, &member.id, "old.png").await;
        set_avatar_url(&state, &officer.id, &officer_url).await;
        set_avatar_url(&state, &member.id, &member_url).await;

        let app = crate::create_router(state.clone());
        let res = post_avatar(&app, &officer_token, Some(&member.id)).await;
        assert_eq!(res.status(), StatusCode::OK);
        let uploaded = response_json(res).await;
        let url = uploaded["url"].as_str().unwrap().to_string();
        let member_prefix = member_avatar_prefix(&member.id);
        assert!(
            url.starts_with(&member_prefix),
            "upload must land in the target folder: {url}"
        );
        assert!(
            !url.starts_with(&member_avatar_prefix(&officer.id)),
            "upload must not land in the officer folder"
        );
        assert!(
            !member_file.exists(),
            "target's previous avatar is replaced"
        );
        assert!(officer_file.exists(), "officer's own avatar file must stay");
        let stored_officer = state
            .db
            .get_member_safe(&officer.id)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            stored_officer.avatar_url.as_deref(),
            Some(officer_url.as_str())
        );

        let put = with_peer(
            Request::builder()
                .method("PUT")
                .uri(format!("/api/members/{}", urlencoding::encode(&member.id)))
                .header("authorization", format!("Bearer {officer_token}"))
                .header("content-type", "application/json"),
        )
        .body(Body::from(
            serde_json::json!({ "avatar_url": url }).to_string(),
        ))
        .unwrap();
        let put_res = app.oneshot(put).await.unwrap();
        assert_eq!(
            put_res.status(),
            StatusCode::OK,
            "PUT of the target's own folder URL"
        );
        let stored = state.db.get_member_safe(&member.id).await.unwrap().unwrap();
        assert_eq!(stored.avatar_url.as_deref(), Some(url.as_str()));
        assert!(
            officer_file.exists(),
            "PUT must not remove the officer file"
        );

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn non_officer_member_id_is_forbidden_and_writes_nothing() {
        let mut state = crate::test_support::test_state().await;
        let root = unique_tmp("member-forbidden");
        state.upload_dir = root.clone();
        let (caller, token) =
            session_member(&state, "plainmember", scuffed_db::OrgRole::Member).await;
        let (other, _) = session_member(&state, "othermember", scuffed_db::OrgRole::Member).await;
        let (caller_file, caller_url) = plant_avatar(&root, &caller.id, "mine.png").await;
        let (other_file, other_url) = plant_avatar(&root, &other.id, "theirs.png").await;
        set_avatar_url(&state, &caller.id, &caller_url).await;
        set_avatar_url(&state, &other.id, &other_url).await;
        let caller_dir = root.join("avatars").join(member_dir_key(&caller.id));
        let other_dir = root.join("avatars").join(member_dir_key(&other.id));
        let caller_before = files_in(&caller_dir);
        let other_before = files_in(&other_dir);

        let app = crate::create_router(state.clone());
        let res = post_avatar(&app, &token, Some(&other.id)).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = response_json(res).await;
        assert_eq!(body["error"], "Can only edit your own profile");
        assert_eq!(files_in(&caller_dir), caller_before);
        assert_eq!(files_in(&other_dir), other_before);
        assert!(caller_file.exists());
        assert!(other_file.exists());

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    /// An officer may not upload an avatar for an admin. The stored URL and
    /// the file on disk stay as they were.
    #[tokio::test]
    async fn officer_upload_for_admin_is_forbidden_and_leaves_avatar() {
        let mut state = crate::test_support::test_state().await;
        let root = unique_tmp("officer-admin-avatar");
        state.upload_dir = root.clone();
        let (officer, officer_token) =
            session_member(&state, "officeradminava", scuffed_db::OrgRole::Officer).await;
        let (admin, _) = session_member(&state, "adminava", scuffed_db::OrgRole::Admin).await;
        let (admin_file, admin_url) = plant_avatar(&root, &admin.id, "admin.png").await;
        set_avatar_url(&state, &admin.id, &admin_url).await;
        let admin_dir = root.join("avatars").join(member_dir_key(&admin.id));
        let officer_dir = root.join("avatars").join(member_dir_key(&officer.id));
        let admin_before = files_in(&admin_dir);

        let app = crate::create_router(state.clone());
        let res = post_avatar(&app, &officer_token, Some(&admin.id)).await;
        assert_eq!(res.status(), StatusCode::FORBIDDEN);
        let body = response_json(res).await;
        assert_eq!(
            body["error"],
            "Officers cannot edit the profile of admins or other officers"
        );
        let stored = state.db.get_member_safe(&admin.id).await.unwrap().unwrap();
        assert_eq!(stored.avatar_url.as_deref(), Some(admin_url.as_str()));
        assert!(admin_file.exists());
        assert_eq!(files_in(&admin_dir), admin_before);
        assert!(
            !officer_dir.exists() || files_in(&officer_dir).is_empty(),
            "a refused upload must not write the officer folder"
        );

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn unknown_avatar_member_id_is_not_found() {
        let mut state = crate::test_support::test_state().await;
        let root = unique_tmp("avatar-missing");
        state.upload_dir = root.clone();
        let (_, token) = session_member(&state, "officer404", scuffed_db::OrgRole::Officer).await;
        let app = crate::create_router(state);
        let res = post_avatar(&app, &token, Some("missingmember")).await;
        assert_eq!(res.status(), StatusCode::NOT_FOUND);
        let body = response_json(res).await;
        assert_eq!(body["error"], "Member not found");
        assert!(
            !root
                .join("avatars")
                .join(member_dir_key("missingmember"))
                .exists()
        );
        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[tokio::test]
    async fn member_id_equal_to_self_matches_omitted_param() {
        let mut state = crate::test_support::test_state().await;
        let root = unique_tmp("avatar-self");
        state.upload_dir = root.clone();
        let (explicit, explicit_token) =
            session_member(&state, "selfexplicit", scuffed_db::OrgRole::Member).await;
        let (omitted, omitted_token) =
            session_member(&state, "selfomitted", scuffed_db::OrgRole::Member).await;
        let (explicit_old, explicit_url) = plant_avatar(&root, &explicit.id, "old.png").await;
        let (omitted_old, omitted_url) = plant_avatar(&root, &omitted.id, "old.png").await;
        set_avatar_url(&state, &explicit.id, &explicit_url).await;
        set_avatar_url(&state, &omitted.id, &omitted_url).await;
        let decoy = root.join("images/victim/secret.png");
        tokio::fs::create_dir_all(decoy.parent().unwrap())
            .await
            .unwrap();
        tokio::fs::write(&decoy, b"keep").await.unwrap();

        let app = crate::create_router(state);
        let explicit_res = post_avatar(&app, &explicit_token, Some(&explicit.id)).await;
        assert_eq!(explicit_res.status(), StatusCode::OK);
        let explicit_url_new = response_json(explicit_res).await["url"]
            .as_str()
            .unwrap()
            .to_string();
        let omitted_res = post_avatar(&app, &omitted_token, None).await;
        assert_eq!(omitted_res.status(), StatusCode::OK);
        let omitted_url_new = response_json(omitted_res).await["url"]
            .as_str()
            .unwrap()
            .to_string();

        assert!(explicit_url_new.starts_with(&member_avatar_prefix(&explicit.id)));
        assert!(omitted_url_new.starts_with(&member_avatar_prefix(&omitted.id)));
        assert!(
            !explicit_old.exists(),
            "self param replaces the caller's file"
        );
        assert!(
            !omitted_old.exists(),
            "omitted param replaces the caller's file"
        );
        assert!(
            !explicit_url_new.starts_with(&member_avatar_prefix(&omitted.id)),
            "self upload stays out of the other member's folder"
        );
        assert!(decoy.exists());

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[test]
    fn member_dir_key_is_filesystem_safe() {
        assert_eq!(member_dir_key("member:abc123"), "member_abc123");
        assert_eq!(member_dir_key("a/../b"), "a____b");
        assert_eq!(member_dir_key(""), "unknown");
    }

    #[tokio::test]
    async fn avatar_upload_refuses_image_bombs_and_stores_a_normal_png() {
        let mut state = crate::test_support::test_state().await;
        let root = unique_tmp("avatar-bomb");
        state.upload_dir = root.clone();
        let (_member, token) = session_member(&state, "bomber", scuffed_db::OrgRole::Member).await;
        let app = crate::create_router(state.clone());

        // Declared gigapixel canvas: refused before anything is stored.
        let res = post_avatar_png(&app, &token, None, &png_with_dims(100_000, 100_000)).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = response_json(res).await;
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .starts_with("Image dimensions too large"),
            "{body}"
        );

        // A 64 MiB colour profile: refused before anything is stored, since
        // the file would be served as uploaded, profile included.
        let res = post_avatar_png(&app, &token, None, &iccp_bomb()).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = response_json(res).await;
        assert!(
            body["error"]
                .as_str()
                .unwrap()
                .starts_with("Image colour profile too large"),
            "{body}"
        );
        assert!(!root.join("avatars").exists(), "nothing stored");

        // A normal avatar cut short inside its IDAT: the header read alone
        // would size it, but it is refused and nothing is stored.
        let normal = crate::routes::hero_icons::tests::iccp_png(64, 64, 4096);
        let res = post_avatar_png(&app, &token, None, &normal[..normal.len() - 20]).await;
        assert_eq!(res.status(), StatusCode::BAD_REQUEST);
        let body = response_json(res).await;
        assert_eq!(body["error"], "Could not read image", "{body}");
        assert!(!root.join("avatars").exists(), "nothing stored");

        // A normal avatar with a real-size profile is stored byte for byte.
        let res = post_avatar_png(&app, &token, None, &normal).await;
        assert_eq!(res.status(), StatusCode::OK, "a normal avatar is stored");
        let body = response_json(res).await;
        let url = body["url"].as_str().unwrap();
        let stored = tokio::fs::read(root.join(url.strip_prefix("/uploads/").unwrap()))
            .await
            .unwrap();
        assert_eq!(stored, normal);
        tokio::fs::remove_dir_all(&root).await.ok();
    }
}
