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
/// The rank update screen (`kit::rank`). Sent only with the member's opt in.
pub const RANK_PNG: &str = "rank.png";
/// The rank screen's top-right mode banner (`kit::rank`). Sent only with
/// the member's opt in.
pub const RANK_BANNER_PNG: &str = "rank_banner.png";
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

/// Why a session's kit is deleted ([`super::remove`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KitRemoval {
    /// The member deleted the game in the app.
    GameDeleted,
    /// The app sent the game as a report: it keeps no copy of what it sent.
    ReportSent,
}

impl KitRemoval {
    /// The reason as the log line and [`REMOVED_KITS_JSONL`] carry it.
    pub fn as_str(self) -> &'static str {
        match self {
            Self::GameDeleted => "game deleted",
            Self::ReportSent => "game report sent",
        }
    }
}

/// `<data_dir>/debug/removed_kits.jsonl`: one line per kit delete. Local
/// only: nothing uploads it or puts it in a report.
pub const REMOVED_KITS_JSONL: &str = "removed_kits.jsonl";

/// Past this size [`REMOVED_KITS_JSONL`] is cut back to its newest
/// [`REMOVED_KITS_KEEP_LINES`] lines before the next line is added. A line
/// is about 120 bytes, so the file holds a few hundred deletes at most.
pub(super) const REMOVED_KITS_MAX_BYTES: u64 = 64 * 1024;

/// Lines [`REMOVED_KITS_JSONL`] keeps when it is cut back.
pub(super) const REMOVED_KITS_KEEP_LINES: usize = 200;

/// Say that `session_id`'s kit was deleted and why, in the log and as one
/// JSON line in [`REMOVED_KITS_JSONL`] (`removed_at`, `session_id`, `why`,
/// `had_kit`). The file is kept because the desktop app deletes kits too,
/// and its log is often not kept anywhere: with the line, a kit that was
/// deleted can be told from one that was never written (#811). `had_kit`
/// is whether a kit folder was there to delete.
pub(super) fn note_removed(data_dir: &Path, session_id: &str, why: KitRemoval, had_kit: bool) {
    let why = why.as_str();
    tracing::info!(session_id, why, had_kit, "report kit: deleted");
    let row = serde_json::json!({
        "removed_at": chrono::Utc::now().to_rfc3339(),
        "session_id": session_id,
        "why": why,
        "had_kit": had_kit,
    });
    if let Err(e) = append_removed_line(&data_dir.join("debug"), &row.to_string()) {
        tracing::warn!(session_id, error = %e, "report kit: delete not recorded");
    }
}

/// Add `line` to `dir`'s [`REMOVED_KITS_JSONL`], first cutting the file
/// back to its newest lines when it is over [`REMOVED_KITS_MAX_BYTES`].
fn append_removed_line(dir: &Path, line: &str) -> io::Result<()> {
    fs::create_dir_all(dir)?;
    let path = dir.join(REMOVED_KITS_JSONL);
    if fs::metadata(&path).is_ok_and(|m| m.len() > REMOVED_KITS_MAX_BYTES) {
        let bytes = fs::read(&path)?;
        let text = String::from_utf8_lossy(&bytes);
        let lines: Vec<&str> = text.lines().collect();
        let newest = &lines[lines.len().saturating_sub(REMOVED_KITS_KEEP_LINES)..];
        fs::write(&path, format!("{}\n", newest.join("\n")))?;
    }
    let mut file = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(path)?;
    writeln!(file, "{line}")
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

#[cfg(test)]
mod removal_tests {
    use super::*;

    /// The lines of `debug/removed_kits.jsonl`, parsed.
    fn removed_lines(data_dir: &Path) -> Vec<serde_json::Value> {
        let path = data_dir.join("debug").join(REMOVED_KITS_JSONL);
        fs::read_to_string(path)
            .unwrap_or_default()
            .lines()
            .map(|line| serde_json::from_str(line).expect("one JSON object per line"))
            .collect()
    }

    /// #811: a kit delete left nothing behind, so a game with rows and no
    /// kit could not be told from one whose kit was never written. Both
    /// callers of `remove` (the daemon for a deleted game, the app after a
    /// sent report) now leave which session and why.
    #[test]
    fn a_deleted_kit_leaves_a_line_saying_which_session_and_why() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let dir = session_dir(data, "sent-1").unwrap();
        private_dir(&dir).unwrap();
        write_private(&dir, READS_JSON, b"{}").unwrap();
        assert!(removed_lines(data).is_empty());

        super::super::remove(data, "sent-1", KitRemoval::ReportSent);
        assert!(!dir.exists(), "the kit is deleted");
        let lines = removed_lines(data);
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["session_id"], "sent-1");
        assert_eq!(lines[0]["why"], "game report sent");
        assert_eq!(lines[0]["had_kit"], true);
        let at = lines[0]["removed_at"].as_str().expect("a time");
        assert!(chrono::DateTime::parse_from_rfc3339(at).is_ok(), "{at}");

        // A delete with no kit folder is kept too: it still stops any kit
        // write in flight for that session.
        super::super::remove(data, "never-had-one", KitRemoval::GameDeleted);
        let lines = removed_lines(data);
        assert_eq!(lines.len(), 2, "{lines:?}");
        assert_eq!(lines[1]["session_id"], "never-had-one");
        assert_eq!(lines[1]["why"], "game deleted");
        assert_eq!(lines[1]["had_kit"], false);
    }

    /// The record is bounded: past [`REMOVED_KITS_MAX_BYTES`] only the
    /// newest [`REMOVED_KITS_KEEP_LINES`] lines stay, and the new line
    /// follows them.
    #[test]
    fn the_record_of_deleted_kits_keeps_only_its_newest_lines() {
        let tmp = tempfile::tempdir().unwrap();
        let data = tmp.path();
        let path = data.join("debug").join(REMOVED_KITS_JSONL);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        let old: String = (0..2000)
            .map(|i| {
                format!(
                    "{{\"session_id\":\"old-{i:04}\",\"why\":\"game deleted\",\"had_kit\":true}}\n"
                )
            })
            .collect();
        assert!(old.len() as u64 > REMOVED_KITS_MAX_BYTES);
        fs::write(&path, old).unwrap();

        super::super::remove(data, "newest", KitRemoval::GameDeleted);
        let lines = removed_lines(data);
        assert_eq!(lines.len(), REMOVED_KITS_KEEP_LINES + 1);
        assert_eq!(lines[0]["session_id"], "old-1800", "the oldest lines went");
        assert_eq!(lines[REMOVED_KITS_KEEP_LINES - 1]["session_id"], "old-1999");
        assert_eq!(lines[REMOVED_KITS_KEEP_LINES]["session_id"], "newest");
        assert!(fs::metadata(&path).unwrap().len() < REMOVED_KITS_MAX_BYTES);

        // Under the limit nothing is cut.
        super::super::remove(data, "next", KitRemoval::ReportSent);
        assert_eq!(removed_lines(data).len(), REMOVED_KITS_KEEP_LINES + 2);
    }

    /// What this thread logged, every level, one line per event, while
    /// `attempt` ran for tries `0, 1, ...` until `done` holds for the lines
    /// so far (at most 50 tries).
    ///
    /// The tries are for tracing's interest cache: while one subscriber
    /// exists, a parallel test with none can register a callsite first and
    /// cache it as off. The cache is rebuilt before each try, so a later try
    /// sees the line. `done` should not look at the level, so a line logged
    /// at the wrong level still ends the tries and fails the test.
    fn logged_until(
        mut attempt: impl FnMut(usize),
        done: impl Fn(&[String]) -> bool,
    ) -> Vec<String> {
        #[derive(Clone, Default)]
        struct Buf(std::sync::Arc<std::sync::Mutex<Vec<u8>>>);
        impl std::io::Write for Buf {
            fn write(&mut self, b: &[u8]) -> std::io::Result<usize> {
                self.0.lock().unwrap().extend_from_slice(b);
                Ok(b.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        impl<'a> tracing_subscriber::fmt::MakeWriter<'a> for Buf {
            type Writer = Buf;
            fn make_writer(&'a self) -> Buf {
                self.clone()
            }
        }
        let buf = Buf::default();
        let lines = || -> Vec<String> {
            let text = String::from_utf8(buf.0.lock().unwrap().clone()).unwrap();
            text.lines().map(str::to_string).collect()
        };
        let sub = tracing_subscriber::fmt()
            .with_writer(buf.clone())
            .with_ansi(false)
            .with_max_level(tracing::Level::TRACE)
            .finish();
        tracing::subscriber::with_default(sub, || {
            for i in 0..50 {
                tracing::callsite::rebuild_interest_cache();
                attempt(i);
                if done(&lines()) {
                    break;
                }
            }
        });
        lines()
    }

    /// Each delete is an INFO line with the session and the reason: the
    /// default log level (`stat_tracker=info`) keeps it, a debug line
    /// would be dropped (#811).
    #[test]
    fn a_kit_delete_is_an_info_line_with_the_session_and_why() {
        const DELETED: &str = "report kit: deleted";
        let tmp = tempfile::tempdir().unwrap();
        let lines = logged_until(
            |i| super::super::remove(tmp.path(), &format!("gone-{i}"), KitRemoval::GameDeleted),
            |lines| lines.iter().any(|l| l.contains(DELETED)),
        );
        let deleted: Vec<&String> = lines.iter().filter(|l| l.contains(DELETED)).collect();
        assert_eq!(deleted.len(), 1, "{lines:?}");
        assert!(deleted[0].contains(" INFO "), "{}", deleted[0]);
        assert!(deleted[0].contains("gone-"), "{}", deleted[0]);
        assert!(deleted[0].contains("game deleted"), "{}", deleted[0]);
    }

    /// The app's one-line form gives the sent-report reason.
    #[test]
    fn remove_sent_records_a_sent_report() {
        let tmp = tempfile::tempdir().unwrap();
        super::super::remove_sent(tmp.path(), "reported");
        let lines = removed_lines(tmp.path());
        assert_eq!(lines.len(), 1, "{lines:?}");
        assert_eq!(lines[0]["why"], "game report sent");
    }

    /// A session id that is no folder name has no kit and leaves no line.
    #[test]
    fn an_unsafe_session_id_deletes_and_records_nothing() {
        let tmp = tempfile::tempdir().unwrap();
        super::super::remove(tmp.path(), "../escape", KitRemoval::GameDeleted);
        assert!(removed_lines(tmp.path()).is_empty());
        assert!(!tmp.path().join("debug").exists());
    }
}
