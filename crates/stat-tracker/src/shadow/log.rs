//! Append-only JSONL log of shadow matcher results at
//! `<data_dir>/shadow/digits.jsonl`. One line per job. Values and confidences
//! only: no player names, no images.
//!
//! Size cap: when the next line would push the file past [`MAX_BYTES`], the
//! file is renamed to `digits.jsonl.1` (replacing any older `.1`) and a fresh
//! file is started. At most two files exist, so the log never holds more than
//! twice [`MAX_BYTES`] on disk.
//!
//! Every error is logged at debug and swallowed. The shadow log must never
//! fail or slow a capture.

use std::io::Write as _;
use std::path::{Path, PathBuf};

use serde::Serialize;

/// Rotate the live file at this size.
pub const MAX_BYTES: u64 = 2 * 1024 * 1024;

/// One disagreeing or suspect cell.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct CellDiff {
    pub row: usize,
    pub field: &'static str,
    /// ocr-v1 value, parsed the same way the tracker parses a stat cell.
    pub ocr: Option<u32>,
    /// ocr-v1 Tesseract confidence for the cell, when it was read.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ocr_conf: Option<i32>,
    pub matcher: Option<u32>,
    pub conf: f32,
    pub suspect: bool,
}

/// One log line: the result of one shadow job.
#[derive(Debug, Clone, PartialEq, Serialize)]
pub struct ShadowRecord {
    /// Capture time of the frame (RFC 3339, UTC).
    pub ts: String,
    pub session: String,
    /// Full frame height in pixels (1080, 1440, ...).
    pub resolution: u32,
    pub team_size: usize,
    pub owner_row: Option<usize>,
    /// Matcher time for this board. `None` when the matcher returned an error.
    pub elapsed_ms: Option<u32>,
    /// Jobs dropped because the worker was busy since the previous line.
    pub dropped_since_last: u64,
    /// Cells compared (rows read by both recognizers times six).
    pub cells: usize,
    /// Cells where matcher and ocr-v1 read the same value.
    pub agree: usize,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    pub diffs: Vec<CellDiff>,
}

#[derive(Debug, Clone)]
pub struct ShadowLog {
    path: PathBuf,
    max_bytes: u64,
}

impl ShadowLog {
    /// The log under `<data_dir>/shadow/digits.jsonl` with the default cap.
    pub fn new(data_dir: &Path) -> Self {
        Self::with_limit(data_dir.join("shadow").join("digits.jsonl"), MAX_BYTES)
    }

    pub fn with_limit(path: PathBuf, max_bytes: u64) -> Self {
        Self { path, max_bytes }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Path of the single rotated file.
    pub fn rotated_path(&self) -> PathBuf {
        let mut os = self.path.clone().into_os_string();
        os.push(".1");
        PathBuf::from(os)
    }

    /// Append one record. Never returns an error.
    pub fn append(&self, record: &ShadowRecord) {
        if let Err(e) = self.try_append(record) {
            tracing::debug!(error = %e, path = %self.path.display(), "shadow log write failed");
        }
    }

    fn try_append(&self, record: &ShadowRecord) -> std::io::Result<()> {
        let mut line = serde_json::to_vec(record).map_err(std::io::Error::other)?;
        line.push(b'\n');
        if let Some(dir) = self.path.parent() {
            std::fs::create_dir_all(dir)?;
        }
        let size = std::fs::metadata(&self.path).map(|m| m.len()).unwrap_or(0);
        if size > 0 && size + line.len() as u64 > self.max_bytes {
            // rename replaces an existing `.1`, so only one old file is kept.
            std::fs::rename(&self.path, self.rotated_path())?;
        }
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)?;
        f.write_all(&line)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(i: usize) -> ShadowRecord {
        ShadowRecord {
            ts: "2026-10-08T18:00:00Z".into(),
            session: format!("session-{i}"),
            resolution: 1440,
            team_size: 5,
            owner_row: Some(2),
            elapsed_ms: Some(40),
            dropped_since_last: 0,
            cells: 60,
            agree: 59,
            error: None,
            diffs: vec![CellDiff {
                row: 2,
                field: "DMG",
                ocr: Some(1234),
                ocr_conf: Some(71),
                matcher: Some(1284),
                conf: 0.91,
                suspect: false,
            }],
        }
    }

    #[test]
    fn appends_one_json_line_per_record() {
        let dir = tempfile::tempdir().unwrap();
        let log = ShadowLog::new(dir.path());
        log.append(&record(1));
        log.append(&record(2));
        let text = std::fs::read_to_string(log.path()).unwrap();
        let lines: Vec<&str> = text.lines().collect();
        assert_eq!(lines.len(), 2);
        let v: serde_json::Value = serde_json::from_str(lines[1]).unwrap();
        assert_eq!(v["session"], "session-2");
        assert_eq!(v["diffs"][0]["field"], "DMG");
        assert_eq!(v["diffs"][0]["matcher"], 1284);
        assert!(v.get("error").is_none());
    }

    #[test]
    fn rotation_keeps_at_most_one_old_file_and_caps_total_size() {
        let dir = tempfile::tempdir().unwrap();
        let limit = 4 * 1024;
        let log = ShadowLog::with_limit(dir.path().join("shadow").join("digits.jsonl"), limit);
        for i in 0..2_000 {
            log.append(&record(i));
        }
        let names: Vec<String> = std::fs::read_dir(dir.path().join("shadow"))
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        assert_eq!(names.len(), 2, "only the live file and one .1: {names:?}");
        let live = std::fs::metadata(log.path()).unwrap().len();
        let old = std::fs::metadata(log.rotated_path()).unwrap().len();
        assert!(live <= limit && old <= limit, "live {live}, old {old}");
        assert!(live + old <= 2 * limit);
        // The newest record is in the live file, and no line was split.
        let text = std::fs::read_to_string(log.path()).unwrap();
        assert!(text.lines().last().unwrap().contains("session-1999"));
        for line in text.lines() {
            serde_json::from_str::<serde_json::Value>(line).unwrap();
        }
    }

    #[test]
    fn default_cap_is_two_megabytes() {
        assert_eq!(MAX_BYTES, 2 * 1024 * 1024);
        let log = ShadowLog::new(Path::new("/data"));
        assert_eq!(log.path(), Path::new("/data/shadow/digits.jsonl"));
        assert_eq!(log.rotated_path(), Path::new("/data/shadow/digits.jsonl.1"));
    }

    #[test]
    fn write_errors_are_swallowed() {
        let dir = tempfile::tempdir().unwrap();
        // A regular file where the shadow directory should be.
        std::fs::write(dir.path().join("shadow"), b"x").unwrap();
        let log = ShadowLog::new(dir.path());
        log.append(&record(1)); // must not panic
        assert!(!log.path().exists());
    }
}
