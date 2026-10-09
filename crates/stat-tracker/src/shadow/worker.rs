//! The shadow worker: one background thread that runs the digit matcher on
//! accepted scoreboards and logs how it compares with ocr-v1.
//!
//! Guardrails:
//! - Fed by a bounded channel of capacity 1 through `try_send`. When the
//!   worker is busy and a job is already queued, the new job is dropped and
//!   counted. The capture path never blocks or awaits on it.
//! - A job holds an `Arc` of the scoreboard crop the capture already made, so
//!   queueing or dropping one never copies pixels.
//! - Each board gets a [`FRAME_BUDGET`] passed to `read_board`.
//! - Shutdown never joins the thread. Dropping [`ShadowWorker`] drops the
//!   sender; the thread ends after its current job, or with the process. A
//!   slow frame can not delay the final session save.
//! - Thread priority is left alone: `libc` is not a dependency of this crate.

use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError};
use std::time::Duration;

use chrono::{DateTime, SecondsFormat, Utc};
use image::DynamicImage;

use super::digits::{self, BoardRead, FIELDS, ShadowError};
use super::log::{CellDiff, ShadowLog, ShadowRecord};
use crate::ocr::RowOcrResult;

/// Matcher time allowed per board.
pub const FRAME_BUDGET: Duration = Duration::from_millis(300);

/// What frame analysis hands to the accept path when shadow mode is on.
#[derive(Debug, Clone)]
pub struct ShadowInput {
    /// The same crop ocr-v1 read, shared, not copied.
    pub scoreboard: Arc<DynamicImage>,
    pub team_size: usize,
    /// Full frame height (the log's `resolution`).
    pub frame_height: u32,
}

impl ShadowInput {
    /// `Some` only when shadow mode is on. Off, it costs nothing.
    pub fn when(
        enabled: bool,
        scoreboard: &Arc<DynamicImage>,
        team_size: usize,
        frame_height: u32,
    ) -> Option<Self> {
        enabled.then(|| Self {
            scoreboard: Arc::clone(scoreboard),
            team_size,
            frame_height,
        })
    }
}

/// One accepted board for the worker.
#[derive(Debug, Clone)]
pub struct ShadowJob {
    pub scoreboard: Arc<DynamicImage>,
    pub team_size: usize,
    pub frame_height: u32,
    pub owner_row: Option<usize>,
    /// ocr-v1 values per row in [`FIELDS`] order.
    pub ocr_values: Vec<[Option<u32>; 6]>,
    /// ocr-v1 Tesseract confidence per cell, `None` where no cell was read.
    pub ocr_conf: Vec<[Option<i32>; 6]>,
    pub session: String,
    pub captured_at: DateTime<Utc>,
}

impl ShadowJob {
    pub fn new(
        input: &ShadowInput,
        rows: &[RowOcrResult],
        owner_row: Option<usize>,
        session: &str,
        captured_at: DateTime<Utc>,
    ) -> Self {
        let ocr_values = rows
            .iter()
            .map(|r| std::array::from_fn(|i| r.stats.get(i).and_then(|c| cell_number(&c.value))))
            .collect();
        let ocr_conf = rows
            .iter()
            .map(|r| std::array::from_fn(|i| r.stats.get(i).map(|c| c.confidence)))
            .collect();
        Self {
            scoreboard: Arc::clone(&input.scoreboard),
            team_size: input.team_size,
            frame_height: input.frame_height,
            owner_row,
            ocr_values,
            ocr_conf,
            session: session.to_string(),
            captured_at,
        }
    }
}

/// Same rule as the tracker's stat-cell parse: keep the digits, read a u32.
fn cell_number(s: &str) -> Option<u32> {
    let digits: String = s.chars().filter(|c| c.is_ascii_digit()).collect();
    if digits.is_empty() {
        None
    } else {
        digits.parse().ok()
    }
}

type ReadFn = dyn Fn(&DynamicImage, usize, Duration) -> Result<BoardRead, ShadowError> + Send;
type SinkFn = dyn FnMut(ShadowRecord) + Send;

/// Handle to the running worker. Dropping it stops the feed; nothing waits.
#[derive(Debug)]
pub struct ShadowWorker {
    tx: SyncSender<ShadowJob>,
    dropped: Arc<AtomicU64>,
}

impl ShadowWorker {
    /// Spawn the worker only when the config flag is on.
    pub fn start_if_enabled(config: &crate::config::Config) -> Option<Self> {
        if !config.shadow_recognizer_enabled() {
            return None;
        }
        let log = ShadowLog::new(&config.data_dir);
        tracing::info!(log = %log.path().display(), "shadow digit recognizer on (log only)");
        Self::spawn(
            Box::new(digits::read_board),
            Box::new(move |r| log.append(&r)),
        )
    }

    /// Spawn with a custom matcher and sink. Tests use this.
    fn spawn(read: Box<ReadFn>, sink: Box<SinkFn>) -> Option<Self> {
        let (tx, rx) = std::sync::mpsc::sync_channel::<ShadowJob>(1);
        let dropped = Arc::new(AtomicU64::new(0));
        let worker_dropped = Arc::clone(&dropped);
        let spawned = std::thread::Builder::new()
            .name("shadow-digits".into())
            .spawn(move || run(rx, read, sink, worker_dropped));
        match spawned {
            Ok(_detached) => Some(Self { tx, dropped }),
            Err(e) => {
                tracing::debug!(error = %e, "shadow worker failed to start");
                None
            }
        }
    }

    /// Queue a job without blocking. Returns false when it was dropped.
    pub fn try_submit(&self, job: ShadowJob) -> bool {
        match self.tx.try_send(job) {
            Ok(()) => true,
            Err(TrySendError::Full(_)) => {
                self.dropped.fetch_add(1, Ordering::Relaxed);
                tracing::debug!("shadow worker busy, board dropped");
                false
            }
            Err(TrySendError::Disconnected(_)) => {
                tracing::debug!("shadow worker gone, board dropped");
                false
            }
        }
    }
}

fn run(rx: Receiver<ShadowJob>, read: Box<ReadFn>, mut sink: Box<SinkFn>, dropped: Arc<AtomicU64>) {
    while let Ok(job) = rx.recv() {
        let dropped_since_last = dropped.swap(0, Ordering::Relaxed);
        let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            read(&job.scoreboard, job.team_size, FRAME_BUDGET)
        }))
        .unwrap_or(Err(ShadowError::Layout("matcher panicked")));
        sink(compare(&job, result, dropped_since_last));
    }
}

/// Build the log record for one job: per-cell agreement with ocr-v1, plus
/// every disagreeing or suspect cell.
pub fn compare(
    job: &ShadowJob,
    result: Result<BoardRead, ShadowError>,
    dropped_since_last: u64,
) -> ShadowRecord {
    let mut record = ShadowRecord {
        ts: job.captured_at.to_rfc3339_opts(SecondsFormat::Millis, true),
        recognizer: digits::RECOGNIZER_ID,
        session: job.session.clone(),
        resolution: job.frame_height,
        team_size: job.team_size,
        owner_row: job.owner_row,
        elapsed_ms: None,
        dropped_since_last,
        cells: 0,
        agree: 0,
        error: None,
        diffs: Vec::new(),
    };
    let board = match result {
        Ok(b) => b,
        Err(e) => {
            record.error = Some(e.to_string());
            return record;
        }
    };
    record.elapsed_ms = Some(board.elapsed_ms);
    for (row, (read, ocr)) in board.rows.iter().zip(&job.ocr_values).enumerate() {
        for (i, cell) in read.cells.iter().enumerate() {
            record.cells += 1;
            let same = cell.value == ocr[i];
            if same {
                record.agree += 1;
            }
            if !same || cell.suspect {
                record.diffs.push(CellDiff {
                    row,
                    field: FIELDS[i],
                    ocr: ocr[i],
                    ocr_conf: job.ocr_conf.get(row).and_then(|c| c[i]),
                    matcher: cell.value,
                    conf: cell.confidence,
                    suspect: cell.suspect,
                });
            }
        }
    }
    record
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::ocr::CellOcrResult;
    use crate::shadow::digits::{CellRead, RowRead};
    use std::sync::Mutex;
    use std::sync::mpsc;

    fn input() -> ShadowInput {
        ShadowInput {
            scoreboard: Arc::new(DynamicImage::new_rgb8(4, 4)),
            team_size: 5,
            frame_height: 1440,
        }
    }

    fn job(session: &str) -> ShadowJob {
        ShadowJob {
            scoreboard: Arc::clone(&input().scoreboard),
            team_size: 5,
            frame_height: 1440,
            owner_row: Some(0),
            ocr_values: vec![[Some(1), Some(2), Some(3), Some(4000), Some(0), Some(77)]],
            ocr_conf: vec![[Some(90); 6]],
            session: session.into(),
            captured_at: Utc::now(),
        }
    }

    fn cell(value: Option<u32>, suspect: bool) -> CellRead {
        CellRead {
            value,
            confidence: 0.9,
            suspect,
        }
    }

    #[test]
    fn flag_off_never_spawns_and_never_builds_input() {
        let config = crate::config::Config::default();
        assert!(!config.shadow_recognizer);
        assert!(ShadowWorker::start_if_enabled(&config).is_none());
        let board = Arc::new(DynamicImage::new_rgb8(4, 4));
        assert!(ShadowInput::when(false, &board, 5, 1440).is_none());
        assert_eq!(Arc::strong_count(&board), 1, "no extra reference when off");
        let on = ShadowInput::when(true, &board, 6, 1080).unwrap();
        assert!(Arc::ptr_eq(&on.scoreboard, &board), "shared, not copied");
    }

    #[test]
    fn flag_on_runs_the_real_matcher_and_writes_one_log_line() {
        let dir = tempfile::tempdir().unwrap();
        let config = crate::config::Config {
            data_dir: dir.path().to_path_buf(),
            shadow_recognizer: true,
            ..crate::config::Config::default()
        };
        let worker = ShadowWorker::start_if_enabled(&config).expect("flag on spawns");
        // A blank board: the matcher must answer (error or read), never hang.
        assert!(worker.try_submit(job("e2e")));
        let log = ShadowLog::new(dir.path());
        let deadline = std::time::Instant::now() + Duration::from_secs(10);
        let line_written = || std::fs::read_to_string(log.path()).is_ok_and(|t| t.ends_with('\n'));
        while !line_written() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(10));
        }
        drop(worker);
        let text = std::fs::read_to_string(log.path()).expect("one line written");
        let v: serde_json::Value = serde_json::from_str(text.lines().next().unwrap()).unwrap();
        assert_eq!(v["session"], "e2e");
        assert_eq!(v["resolution"], 1440);
    }

    #[test]
    fn job_reads_ocr_values_like_the_tracker_parse() {
        let c = |v: &str, conf| CellOcrResult {
            value: v.into(),
            confidence: conf,
            suspect: false,
        };
        let rows = vec![RowOcrResult {
            name: None,
            stats: vec![
                c("12", 90),
                c("0", 88),
                c("", 0),
                c("12,345", 70),
                c("9", 80),
            ],
            mean_confidence: 80,
        }];
        let j = ShadowJob::new(&input(), &rows, Some(0), "s", Utc::now());
        assert_eq!(
            j.ocr_values,
            vec![[Some(12), Some(0), None, Some(12345), Some(9), None]]
        );
        assert_eq!(j.ocr_conf[0][3], Some(70));
        assert_eq!(j.ocr_conf[0][5], None);
    }

    #[test]
    fn compare_counts_agreement_and_lists_disagreeing_and_suspect_cells() {
        let board = BoardRead {
            rows: vec![RowRead {
                cells: [
                    cell(Some(1), false),
                    cell(Some(2), true),
                    cell(Some(8), false),
                    cell(Some(4000), false),
                    cell(None, true),
                    cell(Some(77), false),
                ],
            }],
            elapsed_ms: 12,
        };
        let r = compare(&job("s1"), Ok(board), 3);
        assert_eq!(r.recognizer, digits::RECOGNIZER_ID);
        assert_eq!((r.cells, r.agree), (6, 4));
        assert_eq!(r.dropped_since_last, 3);
        assert_eq!(r.elapsed_ms, Some(12));
        let fields: Vec<_> = r.diffs.iter().map(|d| d.field).collect();
        assert_eq!(fields, ["A", "D", "H"]);
        assert_eq!(r.diffs[1].ocr, Some(3));
        assert_eq!(r.diffs[1].matcher, Some(8));

        let err = compare(&job("s2"), Err(ShadowError::OverBudget), 0);
        assert_eq!(err.error.as_deref(), Some("over budget"));
        assert_eq!(err.recognizer, digits::RECOGNIZER_ID);
        assert_eq!((err.cells, err.agree), (0, 0));
    }

    type Gated = (
        ShadowWorker,
        mpsc::Receiver<()>,
        mpsc::Sender<()>,
        Arc<Mutex<Vec<ShadowRecord>>>,
    );

    /// A matcher that blocks until released, so the queue state is exact.
    fn gated_worker() -> Gated {
        let (started_tx, started_rx) = mpsc::channel::<()>();
        let (release_tx, release_rx) = mpsc::channel::<()>();
        let release_rx = Mutex::new(release_rx);
        let records = Arc::new(Mutex::new(Vec::new()));
        let sink_records = Arc::clone(&records);
        let worker = ShadowWorker::spawn(
            Box::new(move |_, _, _| {
                let _ = started_tx.send(());
                let _ = release_rx.lock().unwrap().recv();
                Err(ShadowError::Unsupported("test"))
            }),
            Box::new(move |r| sink_records.lock().unwrap().push(r)),
        )
        .unwrap();
        (worker, started_rx, release_tx, records)
    }

    #[test]
    fn drops_new_jobs_when_full_and_reports_the_count() {
        let (worker, started, release, records) = gated_worker();
        assert!(worker.try_submit(job("a")));
        started.recv().unwrap(); // worker is inside the matcher on "a"
        assert!(worker.try_submit(job("b")), "one job may wait");
        assert!(!worker.try_submit(job("c")), "full: dropped");
        assert!(!worker.try_submit(job("d")), "full: dropped");
        release.send(()).unwrap();
        started.recv().unwrap(); // now on "b"
        release.send(()).unwrap();
        drop(worker);
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while records.lock().unwrap().len() < 2 && std::time::Instant::now() < deadline {
            std::thread::yield_now();
        }
        let got = records.lock().unwrap();
        let sessions: Vec<_> = got.iter().map(|r| r.session.as_str()).collect();
        assert_eq!(sessions, ["a", "b"], "dropped jobs never run");
        assert_eq!(got[0].dropped_since_last, 0);
        assert_eq!(got[1].dropped_since_last, 2);
    }

    #[test]
    fn dropping_the_handle_never_waits_for_a_busy_worker() {
        let (worker, started, release, _records) = gated_worker();
        assert!(worker.try_submit(job("slow")));
        started.recv().unwrap();
        let t = std::time::Instant::now();
        drop(worker); // the shutdown path: matcher still blocked
        assert!(t.elapsed() < Duration::from_millis(50));
        drop(release);
    }

    #[test]
    fn a_panicking_matcher_is_logged_not_fatal() {
        let records = Arc::new(Mutex::new(Vec::new()));
        let sink_records = Arc::clone(&records);
        let worker = ShadowWorker::spawn(
            Box::new(|_, _, _| panic!("boom")),
            Box::new(move |r| sink_records.lock().unwrap().push(r)),
        )
        .unwrap();
        let wait_for = |n: usize| {
            let deadline = std::time::Instant::now() + Duration::from_secs(5);
            while records.lock().unwrap().len() < n && std::time::Instant::now() < deadline {
                std::thread::yield_now();
            }
        };
        assert!(worker.try_submit(job("p1")));
        wait_for(1);
        assert!(worker.try_submit(job("p2")), "worker survives the panic");
        wait_for(2);
        let got = records.lock().unwrap();
        assert_eq!(got.len(), 2);
        assert!(got.iter().all(|r| r.error.is_some()));
    }
}
