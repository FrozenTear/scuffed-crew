//! Numeric limits for contributed hero icons (`/api/hero-icons`).
//!
//! One place so the server, the docs and Scuffed Site cannot drift. Icons are
//! PNG files that members contribute for icon-matching templates. They stay on
//! the server volume and are only shown to the owner and to officers.

/// Largest accepted PNG file in one `icon` part, in bytes (64 KiB).
pub const HERO_ICON_MAX_BYTES: usize = 64 * 1024;

/// Most `icon` parts accepted in one upload request.
pub const HERO_ICON_MAX_FILES_PER_UPLOAD: usize = 20;

/// Most icons one member may have stored at the same time.
pub const HERO_ICON_MAX_PER_MEMBER: u32 = 500;

/// Largest accepted width or height, in pixels.
pub const HERO_ICON_MAX_EDGE: u32 = 256;

/// Smallest accepted width or height, in pixels.
pub const HERO_ICON_MIN_EDGE: u32 = 8;

/// Upload requests one member may send before the per-member limiter waits.
pub const HERO_ICON_RATE_BURST: u32 = 10;

/// Seconds until the per-member limiter gives back one upload request.
pub const HERO_ICON_RATE_REFILL_SECS: u64 = 60;

/// Request body cap for one upload request, in bytes (2 MiB).
///
/// Room for [`HERO_ICON_MAX_FILES_PER_UPLOAD`] files of
/// [`HERO_ICON_MAX_BYTES`] each, plus multipart headers.
pub const HERO_ICON_MAX_REQUEST_BYTES: usize = 2 * 1024 * 1024;
