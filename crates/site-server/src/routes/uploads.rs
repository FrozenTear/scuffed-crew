use std::io::Cursor;
use std::path::Path;

use axum::{
    Json,
    extract::{Multipart, State},
    http::StatusCode,
};
use serde::Serialize;

use scuffed_auth::server::session::ErrorResponse;
use scuffed_db::{AuditAction, AuditTargetType};

use crate::extractors::{OfficerUser, OrgMember};
use crate::routes::audit_log::audit;
use crate::state::AppState;
use crate::uploads::{UploadError, save_upload};

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
fn member_dir_key(member_id: &str) -> String {
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
fn check_image_dimensions(data: &[u8]) -> Result<(), Reject> {
    let reader = image::ImageReader::new(Cursor::new(data))
        .with_guessed_format()
        .map_err(|_| reject(StatusCode::BAD_REQUEST, "Could not read image"))?;

    let (w, h) = reader.into_dimensions().map_err(|_| {
        reject(
            StatusCode::BAD_REQUEST,
            "Invalid or unreadable image dimensions",
        )
    })?;

    if w > MAX_IMAGE_EDGE || h > MAX_IMAGE_EDGE || (w as u64) * (h as u64) > MAX_IMAGE_PIXELS {
        return Err(reject(
            StatusCode::BAD_REQUEST,
            format!("Image dimensions too large (max {MAX_IMAGE_EDGE}px per side)"),
        ));
    }

    Ok(())
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

/// Best-effort delete of a previously-stored **local** upload. External URLs
/// (e.g. a Discord/Google OAuth avatar CDN link) and anything that isn't a
/// clean `/uploads/…` path are ignored. Failures are logged, never fatal.
async fn delete_local_upload(upload_dir: &Path, url: &str) {
    let Some(rel) = url.strip_prefix("/uploads/") else {
        return; // not a locally-hosted upload
    };
    // Reject traversal / empty components defensively.
    if rel.is_empty()
        || rel
            .split('/')
            .any(|c| c.is_empty() || c == "." || c == "..")
    {
        tracing::warn!(url = %url, "refusing to delete suspicious upload path");
        return;
    }
    let path = upload_dir.join(rel);
    match tokio::fs::remove_file(&path).await {
        Ok(()) => tracing::debug!(path = %path.display(), "deleted replaced upload"),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            tracing::debug!(path = %path.display(), "replaced upload already gone");
        }
        Err(e) => {
            tracing::warn!(path = %path.display(), error = %e, "failed to delete replaced upload")
        }
    }
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

/// POST /api/upload/avatar — upload member avatar (org member)
pub async fn upload_avatar(
    State(state): State<AppState>,
    member: OrgMember,
    mut multipart: Multipart,
) -> Result<Json<UploadResponse>, Reject> {
    let (content_type, data) = read_upload_field(&mut multipart).await?;

    let url = store_member_upload(
        &state,
        &member.member.id,
        "avatars",
        &content_type,
        &data,
        AVATAR_MAX_BYTES,
    )
    .await?;

    // Delete-on-replace: remove the member's PREVIOUS local avatar so it is not
    // orphaned forever (biggest disk-growth fix). Best-effort; never fatal.
    if let Some(prev) = member.member.avatar_url.as_deref()
        && prev != url
    {
        delete_local_upload(&state.upload_dir, prev).await;
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
    fn dimension_guard_rejects_non_image() {
        assert!(check_image_dimensions(b"not an image at all").is_err());
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
    async fn delete_local_upload_removes_local_ignores_external_and_traversal() {
        let root = unique_tmp("delete");
        let avatars = root.join("avatars");
        tokio::fs::create_dir_all(&avatars).await.unwrap();
        let file = avatars.join("old.png");
        tokio::fs::write(&file, b"old").await.unwrap();
        assert!(file.exists());

        // Local upload path → deleted.
        delete_local_upload(&root, "/uploads/avatars/old.png").await;
        assert!(!file.exists());

        // External CDN URL → no-op, no panic.
        delete_local_upload(&root, "https://cdn.discordapp.com/avatars/x/y.png").await;

        // Traversal attempt → refused, sentinel outside the uploads path survives.
        let sentinel = root.join("sentinel");
        tokio::fs::write(&sentinel, b"keep").await.unwrap();
        delete_local_upload(&root, "/uploads/../sentinel").await;
        assert!(sentinel.exists());

        tokio::fs::remove_dir_all(&root).await.ok();
    }

    #[test]
    fn member_dir_key_is_filesystem_safe() {
        assert_eq!(member_dir_key("member:abc123"), "member_abc123");
        assert_eq!(member_dir_key("a/../b"), "a____b");
        assert_eq!(member_dir_key(""), "unknown");
    }
}
