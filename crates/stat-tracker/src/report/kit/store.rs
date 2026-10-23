//! The on-disk kit store: the `report-kit/<session>/` layout, private
//! (0700/0600) atomic writes under one process-wide write lock, the
//! deleted-session set, and retention and pruning.

use std::fs;
use std::io::{self, Write};
use std::path::{Path, PathBuf};

use image::{ImageEncoder, RgbImage};

use super::KitReads;
use crate::config::{DEFAULT_MAX_GAMES, DEFAULT_MAX_MB, Retention};

pub const KIT_DIR: &str = "report-kit";

static RETENTION_GAMES: std::sync::atomic::AtomicUsize =
    std::sync::atomic::AtomicUsize::new(DEFAULT_MAX_GAMES);
static RETENTION_BYTES: std::sync::atomic::AtomicU64 =
    std::sync::atomic::AtomicU64::new(DEFAULT_MAX_MB * 1024 * 1024);

/// Set the retention the daemon prunes with after each kit write.
pub fn set_retention(r: Retention) {
    use std::sync::atomic::Ordering::Relaxed;
    RETENTION_GAMES.store(r.max_games.max(1), Relaxed);
    RETENTION_BYTES.store(r.max_bytes, Relaxed);
}

pub fn retention() -> Retention {
    use std::sync::atomic::Ordering::Relaxed;
    Retention {
        max_games: RETENTION_GAMES.load(Relaxed),
        max_bytes: RETENTION_BYTES.load(Relaxed),
    }
}

/// A kit older than this (14 days) is deleted on the next prune.
pub const MAX_AGE: std::time::Duration = std::time::Duration::from_secs(14 * 24 * 60 * 60);

pub const SCOREBOARD_PNG: &str = "scoreboard.png";
pub const OUTCOME_PNG: &str = "outcome.png";
pub const READS_JSON: &str = "reads.json";
/// Player names read on this session's boards. Local only: never put in a
/// report zip, only handed to the log scrubber so those names are removed.
pub const NAMES_JSON: &str = "names.json";

/// A session id is used as a directory name only when it is plain: ASCII
/// letters, digits, `-`, `_`, 1 to 64 characters. Anything else has no kit.
pub(super) fn safe_session(session_id: &str) -> Option<&str> {
    let ok = !session_id.is_empty()
        && session_id.len() <= 64
        && session_id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_');
    ok.then_some(session_id)
}

pub fn kit_root(data_dir: &Path) -> PathBuf {
    data_dir.join(KIT_DIR)
}

pub fn session_dir(data_dir: &Path, session_id: &str) -> Option<PathBuf> {
    safe_session(session_id).map(|s| kit_root(data_dir).join(s))
}

/// Serialises kit writes in this process: accepted captures hand their kit
/// to blocking tasks, and those can finish out of order.
static WRITE_LOCK: std::sync::Mutex<()> = std::sync::Mutex::new(());

pub(super) fn write_lock() -> std::sync::MutexGuard<'static, ()> {
    WRITE_LOCK.lock().unwrap_or_else(|e| e.into_inner())
}

pub(super) fn read_reads(dir: &Path) -> Option<KitReads> {
    fs::read(dir.join(READS_JSON))
        .ok()
        .and_then(|b| serde_json::from_slice(&b).ok())
}

pub(super) fn private_dir(dir: &Path) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Write `bytes` to `dir/name` through a private temp file and a rename.
pub(super) fn write_private(dir: &Path, name: &str, bytes: &[u8]) -> io::Result<()> {
    static SEQ: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let seq = SEQ.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    let tmp = dir.join(format!(".{name}.{}.{seq}.tmp", std::process::id()));
    {
        let mut opts = fs::OpenOptions::new();
        opts.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt;
            opts.mode(0o600);
        }
        let written = opts.open(&tmp).and_then(|mut f| {
            f.write_all(bytes)?;
            f.sync_all()
        });
        if let Err(e) = written {
            let _ = fs::remove_file(&tmp);
            return Err(e);
        }
    }
    if let Err(e) = fs::rename(&tmp, dir.join(name)) {
        let _ = fs::remove_file(&tmp);
        return Err(e);
    }
    // the rename itself survives a crash only once the directory is synced
    #[cfg(unix)]
    fs::File::open(dir)?.sync_all()?;
    Ok(())
}

/// Lossless 8-bit RGB PNG with no ancillary chunks.
pub fn encode_png(img: &RgbImage) -> io::Result<Vec<u8>> {
    let mut out = Vec::new();
    image::codecs::png::PngEncoder::new(&mut out)
        .write_image(
            img.as_raw(),
            img.width(),
            img.height(),
            image::ExtendedColorType::Rgb8,
        )
        .map_err(io::Error::other)?;
    Ok(out)
}

/// Session directories deleted while the daemon runs: a capture still in
/// flight when the member deleted the game never writes its kit again.
pub(super) fn removed() -> &'static std::sync::Mutex<std::collections::HashSet<PathBuf>> {
    static R: std::sync::OnceLock<std::sync::Mutex<std::collections::HashSet<PathBuf>>> =
        std::sync::OnceLock::new();
    R.get_or_init(Default::default)
}

pub(super) fn was_removed(dir: &Path) -> bool {
    removed()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .contains(dir)
}

pub(super) fn modified(path: &Path) -> std::time::SystemTime {
    fs::metadata(path)
        .and_then(|m| m.modified())
        .unwrap_or(std::time::UNIX_EPOCH)
}

pub(super) fn prune_locked(data_dir: &Path, keep: Retention) {
    let root = kit_root(data_dir);
    let Ok(rd) = fs::read_dir(&root) else {
        return;
    };
    let mut dirs: Vec<(std::time::SystemTime, PathBuf)> = rd
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| p.is_dir())
        .map(|p| (modified(&p), p))
        .collect();
    dirs.sort_by_key(|d| std::cmp::Reverse(d.0));
    let now = std::time::SystemTime::now();
    let mut total: u64 = 0;
    // Strictly oldest first: once a game is dropped, every older one goes
    // too, so an older game is never kept while a newer one is deleted.
    let mut dropping = false;
    for (i, (at, p)) in dirs.into_iter().enumerate() {
        // The newest kit is always kept, by every limit.
        if i > 0 && !dropping {
            let size = dir_bytes(&p);
            let too_old = now.duration_since(at).is_ok_and(|age| age > MAX_AGE);
            let too_big = total.saturating_add(size) > keep.max_bytes;
            dropping = i >= keep.max_games || too_old || too_big;
            if !dropping {
                total += size;
            }
        } else if i == 0 {
            total = dir_bytes(&p);
        }
        if dropping {
            let _ = fs::remove_dir_all(p);
        }
    }
}

fn dir_bytes(dir: &Path) -> u64 {
    fs::read_dir(dir)
        .map(|rd| {
            rd.filter_map(Result::ok)
                .filter_map(|e| e.metadata().ok())
                .filter(|m| m.is_file())
                .map(|m| m.len())
                .sum()
        })
        .unwrap_or(0)
}
