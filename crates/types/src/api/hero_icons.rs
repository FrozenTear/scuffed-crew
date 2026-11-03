//! JSON shapes for `/api/hero-icons` (contributed hero icon PNGs).
//!
//! Icons stay on the server volume. Image bytes are only served to the owner,
//! and to officers while the owner is active and opted in. See
//! `docs/api/hero-icons.md`.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Hero icons are switched off on this server (`HERO_ICONS_DIR` unusable). 503.
pub const HERO_ICON_ERR_DISABLED: &str = "hero_icons_disabled";
/// The member has not opted in to contributing icons. 403 on upload.
pub const HERO_ICON_ERR_OPT_IN_REQUIRED: &str = "icon_opt_in_required";
/// The member an officer asked about is not opted in. 403. Their icons are
/// kept but hidden from everyone except the member.
pub const HERO_ICON_ERR_OPT_IN_OFF: &str = "icon_opt_in_off";
/// A part does not start with the PNG signature. 415.
pub const HERO_ICON_ERR_NOT_PNG: &str = "not_png";
/// A part is larger than `HERO_ICON_MAX_BYTES`, or the body is over its cap. 413.
pub const HERO_ICON_ERR_TOO_LARGE: &str = "icon_too_large";
/// Width or height outside `HERO_ICON_MIN_EDGE..=HERO_ICON_MAX_EDGE`. 400.
pub const HERO_ICON_ERR_BAD_DIMENSIONS: &str = "icon_bad_dimensions";
/// The PNG signature is there but the image does not decode. 400.
pub const HERO_ICON_ERR_INVALID_PNG: &str = "invalid_png";
/// `?hero=` is missing or not a known hero. 400.
pub const HERO_ICON_ERR_UNKNOWN_HERO: &str = "unknown_hero";
/// More than `HERO_ICON_MAX_FILES_PER_UPLOAD` parts in one request. 413.
pub const HERO_ICON_ERR_TOO_MANY_IN_REQUEST: &str = "too_many_icons_in_request";
/// The request carried no `icon` part. 400.
pub const HERO_ICON_ERR_NO_ICONS: &str = "no_icons";
/// The upload would take the member over `HERO_ICON_MAX_PER_MEMBER`. 409.
pub const HERO_ICON_ERR_LIMIT_REACHED: &str = "icon_limit_reached";
/// A multipart part is named something other than `icon`. 400.
pub const HERO_ICON_ERR_UNEXPECTED_FIELD: &str = "unexpected_field";
/// The body is not readable multipart. 400.
pub const HERO_ICON_ERR_INVALID_MULTIPART: &str = "invalid_multipart";
/// Unknown id, malformed id, or an icon the caller may not see. 404.
pub const HERO_ICON_ERR_NOT_FOUND: &str = "icon_not_found";
/// Officer per-member route for an unknown or inactive member. 404.
pub const HERO_ICON_ERR_MEMBER_NOT_FOUND: &str = "member_not_found";

/// `GET` and `PUT /api/hero-icons/opt-in`. Off by default.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeroIconOptIn {
    pub icon_contribution_opt_in: bool,
}

/// One of the caller's own icons.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeroIconEntry {
    /// 32 lowercase hex characters, made by the server.
    pub id: String,
    /// Hero display name, for example `"Lúcio"` or `"Soldier: 76"`.
    pub hero: String,
    /// Stored PNG size after the server re-encoded it.
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    /// RFC 3339 UTC.
    pub uploaded_at: DateTime<Utc>,
    /// `/api/hero-icons/{id}/image`. Needs the session; never public.
    pub thumbnail_url: String,
}

/// `GET /api/hero-icons/mine`. Own icons are listed even while opted out.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MyHeroIcons {
    pub icon_contribution_opt_in: bool,
    pub count: u32,
    /// `HERO_ICON_MAX_PER_MEMBER`.
    pub max_icons: u32,
    /// Newest first.
    pub icons: Vec<HeroIconEntry>,
}

/// `201` from `POST /api/hero-icons`. The icons stored by this request.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeroIconUploadResponse {
    pub icons: Vec<HeroIconEntry>,
}

/// `DELETE /api/hero-icons/mine`, `DELETE /api/hero-icons/{id}` and
/// `DELETE /api/hero-icons/members/{member_id}`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeroIconsDeleted {
    pub deleted: u32,
}

/// One icon in an officer list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfficerHeroIcon {
    pub id: String,
    pub member_id: String,
    /// Empty when the member record has no name.
    pub display_name: String,
    pub hero: String,
    pub size_bytes: u64,
    pub width: u32,
    pub height: u32,
    /// RFC 3339 UTC.
    pub uploaded_at: DateTime<Utc>,
    /// `/api/hero-icons/{id}/image`. Needs an officer session.
    pub thumbnail_url: String,
}

/// `GET /api/hero-icons/members/{member_id}` (officer).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct OfficerHeroIconList {
    /// Newest first.
    pub icons: Vec<OfficerHeroIcon>,
}

/// One row of `GET /api/hero-icons/members` (officer), sent as a
/// `CursorResponse` page sorted by display name, then member id.
///
/// Only active members who are opted in are listed, so
/// `icon_contribution_opt_in` is always true. It stays in the shape so a
/// client can show it without a second call.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct HeroIconMemberCount {
    pub member_id: String,
    /// Empty when the member record has no name.
    pub display_name: String,
    pub icon_count: u32,
    pub icon_contribution_opt_in: bool,
}
