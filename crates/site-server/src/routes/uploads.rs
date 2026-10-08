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
        && avatar_rel_is_safe(rel)
    {
        return Ok(Some(trimmed.to_string()));
    }
    Err("avatar_url must be empty, an https URL, or a file under your own avatars folder")
}

/// Relative path under `avatars/<key>/`. Rejects empty segments, `.`, `..`,
/// and characters that would let a later join escape the folder.
fn avatar_rel_is_safe(rel: &str) -> bool {
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
    if !avatar_rel_is_safe(rel) {
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
///   profile (self, or officer+). Otherwise 403 `{ "error": "..." }`. Unknown
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

    fn png_multipart() -> (Vec<u8>, &'static str) {
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
        let (body, boundary) = png_multipart();
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
}
