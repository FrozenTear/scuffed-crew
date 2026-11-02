//! Private storage and the per-member upload limiter for contributed hero icons.
//!
//! Files live under `HERO_ICONS_DIR` (default `data/hero-icons`) as
//! `<id>.png`, where `id` is 32 lowercase hex characters made by the server.
//! No part of a path on disk comes from the request. The directory is not the
//! upload tree and it is not served as static files: image bytes only leave
//! through the session-checked `GET /api/hero-icons/{id}/image`.
//!
//! The limiter is a per-member token bucket like [`crate::nostr_rate_limit`]:
//! [`HERO_ICON_RATE_BURST`] upload requests, then one more every
//! [`HERO_ICON_RATE_REFILL_SECS`] seconds. State is per process.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Instant;

use scuffed_types::{HERO_ICON_RATE_BURST, HERO_ICON_RATE_REFILL_SECS};

/// `HERO_ICONS_DIR`, or `data/hero-icons` when it is unset or blank.
pub fn hero_icons_dir_from_env() -> PathBuf {
    match std::env::var("HERO_ICONS_DIR") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ => PathBuf::from("data/hero-icons"),
    }
}

/// True for exactly 32 lowercase hex characters, the only id shape the server makes.
///
/// Every id from a URL goes through this before any database or file call, so
/// `..`, slashes and dots never reach a path.
pub fn is_icon_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// True when the icons directory could be served or could overlap uploads or reports.
pub fn hero_icons_dir_conflicts(icons: &Path, upload_dir: &Path, reports_dir: &Path) -> bool {
    crate::stat_reports::reports_dir_conflicts(icons, upload_dir)
        || crate::stat_reports::reports_dir_conflicts(icons, reports_dir)
}

/// Create the directory with mode 0700 on unix.
pub async fn ensure_hero_icons_dir(dir: &Path) -> std::io::Result<()> {
    tokio::fs::create_dir_all(dir).await?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        tokio::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o700)).await?;
    }
    Ok(())
}

/// Open `HERO_ICONS_DIR`. On failure the store is disabled and the site still starts.
pub async fn open_hero_icons_dir(
    configured: &Path,
    upload_dir: &Path,
    reports_dir: &Path,
) -> HeroIconStore {
    match try_open_hero_icons_dir(configured, upload_dir, reports_dir).await {
        Ok(dir) => HeroIconStore::new(dir, true),
        Err(reason) => {
            tracing::error!("hero icons disabled: {reason}");
            HeroIconStore::new(configured.to_path_buf(), false)
        }
    }
}

async fn try_open_hero_icons_dir(
    configured: &Path,
    upload_dir: &Path,
    reports_dir: &Path,
) -> Result<PathBuf, String> {
    ensure_hero_icons_dir(configured).await.map_err(|error| {
        format!(
            "HERO_ICONS_DIR {} is missing or not writable ({error})",
            configured.display()
        )
    })?;
    let probe = configured.join(".write-probe");
    if let Err(error) = tokio::fs::write(&probe, b"ok").await {
        return Err(format!(
            "HERO_ICONS_DIR {} is not writable ({error})",
            configured.display()
        ));
    }
    let _ = tokio::fs::remove_file(&probe).await;
    let icons = std::fs::canonicalize(configured).map_err(|error| {
        format!(
            "HERO_ICONS_DIR {} could not be resolved ({error})",
            configured.display()
        )
    })?;
    if hero_icons_dir_conflicts(&icons, upload_dir, reports_dir) {
        return Err(format!(
            "HERO_ICONS_DIR {} overlaps uploads, reports or the web root",
            icons.display()
        ));
    }
    Ok(icons)
}

struct Bucket {
    tokens: f64,
    last: Instant,
}

fn capacity() -> f64 {
    f64::from(HERO_ICON_RATE_BURST)
}

fn refill_per_sec() -> f64 {
    1.0 / HERO_ICON_RATE_REFILL_SECS as f64
}

impl Bucket {
    fn full(now: Instant) -> Self {
        Bucket {
            tokens: capacity(),
            last: now,
        }
    }

    /// Refill by elapsed time, then try to spend one token.
    ///
    /// `Err(secs)` is the whole seconds until one token is back, at least 1.
    fn try_spend(&mut self, now: Instant) -> Result<(), u64> {
        let elapsed = now.duration_since(self.last).as_secs_f64();
        self.tokens = (self.tokens + elapsed * refill_per_sec()).min(capacity());
        self.last = now;
        if self.tokens >= 1.0 {
            self.tokens -= 1.0;
            Ok(())
        } else {
            Err(secs_until_token(self.tokens))
        }
    }
}

/// Whole seconds until `tokens` reaches 1. Always at least 1.
fn secs_until_token(tokens: f64) -> u64 {
    let deficit = (1.0 - tokens).max(0.0);
    let raw = deficit / refill_per_sec();
    if !raw.is_finite() || raw <= 1.0 {
        return 1;
    }
    let ceil = raw.ceil();
    if ceil >= u64::MAX as f64 {
        u64::MAX
    } else {
        ceil as u64
    }
}

/// Where icons are stored, whether the feature is on, and the upload limiter.
///
/// Cheap to clone. Clones share one limiter.
#[derive(Clone)]
pub struct HeroIconStore {
    dir: PathBuf,
    enabled: bool,
    limiter: Arc<Mutex<HashMap<String, Bucket>>>,
}

impl HeroIconStore {
    pub fn new(dir: PathBuf, enabled: bool) -> Self {
        HeroIconStore {
            dir,
            enabled,
            limiter: Arc::new(Mutex::new(HashMap::new())),
        }
    }

    /// Enabled store in a fresh directory under the system temp dir, for tests.
    ///
    /// The directory is created with mode 0700 on the first upload. The caller
    /// removes it with [`HeroIconStore::dir`] when the test ends.
    pub fn in_temp_dir() -> Self {
        let name = format!("scuffed-hero-icons-{}", uuid::Uuid::new_v4().simple());
        HeroIconStore::new(std::env::temp_dir().join(name), true)
    }

    /// The icons directory. Only meaningful while [`HeroIconStore::enabled`].
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    /// False when `HERO_ICONS_DIR` could not be used. Routes answer 503.
    pub fn enabled(&self) -> bool {
        self.enabled
    }

    /// Path of one icon file. Callers check [`is_icon_id`] first.
    pub(crate) fn icon_path(&self, id: &str) -> PathBuf {
        self.dir.join(format!("{id}.png"))
    }

    /// Spend one upload request from `member_id`'s bucket.
    ///
    /// `Err(secs)` means the member is throttled for `secs` seconds (at least 1).
    /// A member seen for the first time starts with a full bucket.
    pub fn check(&self, member_id: &str) -> Result<(), u64> {
        let now = Instant::now();
        // A poisoned lock is recovered: the limiter must not take down uploads.
        let mut map = self.limiter.lock().unwrap_or_else(|e| e.into_inner());
        let bucket = map
            .entry(member_id.to_string())
            .or_insert_with(|| Bucket::full(now));
        bucket.try_spend(now)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn icon_ids_are_32_lowercase_hex() {
        assert!(is_icon_id("0123456789abcdef0123456789abcdef"));
        assert!(!is_icon_id("0123456789ABCDEF0123456789abcdef"));
        assert!(!is_icon_id("0123456789abcdef0123456789abcde"));
        assert!(!is_icon_id("../../etc/passwd"));
        assert!(!is_icon_id("0123456789abcdef0123456789abc.ef"));
        assert!(!is_icon_id(""));
    }

    #[test]
    fn bucket_allows_the_burst_then_waits() {
        let store = HeroIconStore::new(PathBuf::from("unused"), true);
        for _ in 0..HERO_ICON_RATE_BURST {
            assert!(store.check("m1").is_ok());
        }
        let wait = store.check("m1").unwrap_err();
        assert!(wait >= 1);
        assert!(wait <= HERO_ICON_RATE_REFILL_SECS);
        assert!(store.check("m2").is_ok(), "buckets are per member");
    }
}
