//! Private storage and the per-member upload limiter for contributed hero icons.
//!
//! Files live under `HERO_ICONS_DIR` (see [`hero_icons_dir_from_env`]) as
//! `<id>.png`, where `id` is 32 lowercase hex characters made by the server.
//! No part of a path on disk comes from the request. The directory is not the
//! upload tree and it is not served as static files: image bytes only leave
//! through the session-checked `GET /api/hero-icons/{id}/image`.
//!
//! The limiter is a per-member token bucket like [`crate::nostr_rate_limit`]:
//! [`HERO_ICON_RATE_BURST`] upload requests, then one more every
//! [`HERO_ICON_RATE_REFILL_SECS`] seconds. State is per process. Buckets that
//! have refilled are dropped once the map holds more than
//! [`LIMITER_PRUNE_ABOVE`] entries, since a full bucket acts like a new one.
//!
//! [`sweep_orphan_files`] removes icon files that have no row, once at startup.

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant, SystemTime};

use scuffed_db::Database;
use scuffed_types::{HERO_ICON_RATE_BURST, HERO_ICON_RATE_REFILL_SECS};

/// The limiter map is pruned when it holds more buckets than this.
pub const LIMITER_PRUNE_ABOVE: usize = 1024;

/// The sweep leaves files younger than this alone, so an upload that wrote its
/// file but has not inserted the row yet is never touched.
pub const ORPHAN_MIN_AGE: Duration = Duration::from_secs(3600);

/// Ids looked up per database query during the sweep.
const SWEEP_BATCH: usize = 500;

/// `HERO_ICONS_DIR`, or a default when it is unset or blank.
///
/// The production default is `data/hero-icons` (`/app/data/hero-icons` in the
/// image). Outside production the default is `scuffed-hero-icons` under the
/// system temp dir, so a dev server started from the repo root never writes
/// icons into the repo tree.
pub fn hero_icons_dir_from_env() -> PathBuf {
    match std::env::var("HERO_ICONS_DIR") {
        Ok(value) if !value.trim().is_empty() => PathBuf::from(value.trim()),
        _ if scuffed_auth::is_production_env() => PathBuf::from("data/hero-icons"),
        _ => std::env::temp_dir().join("scuffed-hero-icons"),
    }
}

/// True for exactly 32 lowercase hex characters, the only id shape the server makes.
///
/// Every id from a URL goes through this before any database or file call, so
/// `..`, slashes and dots never reach a path.
pub fn is_icon_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| matches!(b, b'0'..=b'9' | b'a'..=b'f'))
}

/// True when the icons directory could be served or could overlap uploads,
/// reports or recognizer packs.
pub fn hero_icons_dir_conflicts(
    icons: &Path,
    upload_dir: &Path,
    reports_dir: &Path,
    packs_dir: Option<&Path>,
) -> bool {
    crate::stat_reports::reports_dir_conflicts(icons, upload_dir)
        || crate::stat_reports::reports_dir_conflicts(icons, reports_dir)
        || packs_dir.is_some_and(|packs| crate::stat_reports::reports_dir_conflicts(icons, packs))
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
    packs_dir: Option<&Path>,
) -> HeroIconStore {
    match try_open_hero_icons_dir(configured, upload_dir, reports_dir, packs_dir).await {
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
    packs_dir: Option<&Path>,
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
    if hero_icons_dir_conflicts(&icons, upload_dir, reports_dir, packs_dir) {
        return Err(format!(
            "HERO_ICONS_DIR {} overlaps uploads, reports, packs or the web root",
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

    /// True when the bucket would be full at `now`, so it acts like a new one.
    fn is_full_at(&self, now: Instant) -> bool {
        let elapsed = now.saturating_duration_since(self.last).as_secs_f64();
        self.tokens + elapsed * refill_per_sec() >= capacity()
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
        self.check_at(member_id, Instant::now())
    }

    fn check_at(&self, member_id: &str, now: Instant) -> Result<(), u64> {
        // A poisoned lock is recovered: the limiter must not take down uploads.
        let mut map = self.limiter.lock().unwrap_or_else(|e| e.into_inner());
        if map.len() > LIMITER_PRUNE_ABOVE {
            // Lossless: a dropped bucket comes back full, which is what it was.
            // Keys are member ids, so what is left is bounded by the members
            // who uploaded within the last refill window.
            map.retain(|_, bucket| !bucket.is_full_at(now));
        }
        let bucket = map
            .entry(member_id.to_string())
            .or_insert_with(|| Bucket::full(now));
        bucket.try_spend(now)
    }

    #[cfg(test)]
    fn limiter_len(&self) -> usize {
        self.limiter.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

/// Run [`sweep_orphan_files`] once in the background when the store is enabled.
///
/// The count goes to the info log and a failure to the warn log. Neither stops
/// the server.
pub fn spawn_orphan_sweep(db: Arc<Database>, store: HeroIconStore) {
    if !store.enabled() {
        return;
    }
    tokio::spawn(async move {
        match sweep_orphan_files(&db, &store).await {
            Ok(removed) => tracing::info!(removed, "hero icon orphan sweep done"),
            Err(error) => tracing::warn!(%error, "hero icon orphan sweep failed"),
        }
    });
}

/// Remove icon files older than [`ORPHAN_MIN_AGE`] that have no `hero_icon` row.
///
/// Returns how many files went. See [`sweep_orphan_files_older_than`].
pub async fn sweep_orphan_files(db: &Database, store: &HeroIconStore) -> Result<usize, String> {
    sweep_orphan_files_older_than(db, store, ORPHAN_MIN_AGE).await
}

/// Remove icon files at least `min_age` old that have no `hero_icon` row.
///
/// Only plain files directly in the icons directory whose name is
/// `<32 lowercase hex>.png` are candidates. Symlinks, directories and every
/// other name are left alone and never followed. Uploads write straight to
/// `<id>.png` with no temp file, and a failed write removes its file, so no
/// other names are made by the server. A missing directory is an empty sweep.
pub async fn sweep_orphan_files_older_than(
    db: &Database,
    store: &HeroIconStore,
    min_age: Duration,
) -> Result<usize, String> {
    let dir = store.dir();
    let mut entries = match tokio::fs::read_dir(dir).await {
        Ok(entries) => entries,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(error) => return Err(error.to_string()),
    };
    let now = SystemTime::now();
    let mut candidates: Vec<String> = Vec::new();
    while let Some(entry) = entries.next_entry().await.map_err(|e| e.to_string())? {
        let name = entry.file_name();
        let Some(id) = name.to_str().and_then(|name| name.strip_suffix(".png")) else {
            continue;
        };
        if !is_icon_id(id) {
            continue;
        }
        // DirEntry::file_type does not follow symlinks.
        let Ok(file_type) = entry.file_type().await else {
            continue;
        };
        if !file_type.is_file() {
            continue;
        }
        let Ok(modified) = entry.metadata().await.and_then(|meta| meta.modified()) else {
            continue;
        };
        let age = now.duration_since(modified).unwrap_or(Duration::ZERO);
        if age >= min_age {
            candidates.push(id.to_string());
        }
    }

    let mut removed = 0usize;
    for batch in candidates.chunks(SWEEP_BATCH) {
        let existing = db
            .existing_hero_icon_ids(batch)
            .await
            .map_err(|e| e.to_string())?;
        for id in batch.iter().filter(|id| !existing.contains(*id)) {
            match tokio::fs::remove_file(store.icon_path(id)).await {
                Ok(()) => removed += 1,
                Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
                Err(error) => tracing::warn!(%error, "hero icon orphan delete failed"),
            }
        }
    }
    Ok(removed)
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

    #[test]
    fn limiter_drops_refilled_buckets_when_large() {
        let store = HeroIconStore::new(PathBuf::from("unused"), true);
        let start = Instant::now();
        let count = LIMITER_PRUNE_ABOVE + 100;
        for i in 0..count {
            assert!(store.check_at(&format!("m{i}"), start).is_ok());
        }
        // Each bucket spent a token, so none is full and nothing was dropped.
        assert_eq!(store.limiter_len(), count);

        // After a full refill every old bucket acts like a new one and goes.
        let refill = Duration::from_secs(HERO_ICON_REFILL_SECS_ALL);
        assert!(store.check_at("late", start + refill).is_ok());
        assert_eq!(store.limiter_len(), 1);
    }

    /// Seconds for an empty bucket to refill completely.
    const HERO_ICON_REFILL_SECS_ALL: u64 = HERO_ICON_RATE_REFILL_SECS * HERO_ICON_RATE_BURST as u64;

    fn store_in_temp_dir() -> HeroIconStore {
        let store = HeroIconStore::in_temp_dir();
        std::fs::create_dir_all(store.dir()).unwrap();
        store
    }

    fn write_file(store: &HeroIconStore, name: &str, age: Duration) -> PathBuf {
        let path = store.dir().join(name);
        std::fs::write(&path, b"png").unwrap();
        let file = std::fs::File::options().write(true).open(&path).unwrap();
        file.set_modified(SystemTime::now() - age).unwrap();
        path
    }

    #[tokio::test]
    async fn sweep_removes_only_old_orphan_icon_files() {
        let state = crate::test_support::test_state().await;
        let store = store_in_temp_dir();
        let old = Duration::from_secs(2 * 3600);

        let orphan_id = "a".repeat(32);
        let kept_id = "b".repeat(32);
        let young_id = "c".repeat(32);
        let orphan = write_file(&store, &format!("{orphan_id}.png"), old);
        let kept = write_file(&store, &format!("{kept_id}.png"), old);
        let young = write_file(&store, &format!("{young_id}.png"), Duration::ZERO);
        let other = write_file(&store, "notes.txt", old);
        let upper = write_file(&store, &format!("{}.png", "D".repeat(32)), old);

        let row = scuffed_db::NewHeroIcon {
            id: kept_id.clone(),
            member_id: "m1".to_string(),
            hero: "Ana".to_string(),
            file_name: format!("{kept_id}.png"),
            size_bytes: 3,
            width: 16,
            height: 16,
            created_at: chrono::Utc::now(),
        };
        state.db.insert_hero_icon(&row).await.unwrap();

        let removed = sweep_orphan_files(&state.db, &store).await.unwrap();
        assert_eq!(removed, 1);
        assert!(!orphan.exists());
        assert!(kept.exists());
        assert!(young.exists(), "younger than the minimum age");
        assert!(other.exists(), "not an icon name");
        assert!(upper.exists(), "not an icon name");

        // With no minimum age the young orphan goes too.
        let removed = sweep_orphan_files_older_than(&state.db, &store, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(removed, 1);
        assert!(!young.exists());
        assert!(kept.exists());

        let _ = std::fs::remove_dir_all(store.dir());
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn sweep_never_follows_symlinks() {
        let state = crate::test_support::test_state().await;
        let store = store_in_temp_dir();
        let outside = store_in_temp_dir();
        let target = write_file(&outside, "target.png", Duration::from_secs(7200));
        let link = store.dir().join(format!("{}.png", "e".repeat(32)));
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let removed = sweep_orphan_files_older_than(&state.db, &store, Duration::ZERO)
            .await
            .unwrap();
        assert_eq!(removed, 0);
        assert!(target.exists());
        assert!(link.symlink_metadata().is_ok());

        let _ = std::fs::remove_dir_all(store.dir());
        let _ = std::fs::remove_dir_all(outside.dir());
    }

    #[tokio::test]
    async fn sweep_of_a_missing_directory_is_empty() {
        let state = crate::test_support::test_state().await;
        let store = HeroIconStore::in_temp_dir();
        let removed = sweep_orphan_files(&state.db, &store).await.unwrap();
        assert_eq!(removed, 0);
    }
}
