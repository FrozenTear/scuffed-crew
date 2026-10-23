//! What a report reads from this machine: the setup behind the manifest
//! `system` object (pid file, the daemon's OCR status file, env, host
//! probe), and the kit, live snapshot and log window behind a report's
//! input. The wire format these feed is in [`super::bundle`].

use chrono::{DateTime, Utc};

use super::bundle::{
    GameCategory, GameReportInput, LogOnlyCategory, LogReportInput, OCR_BLOCKED_SITES_MAX,
    ReportError, Resolution, SessionType, SystemInfo,
};
use super::marker;
use super::scrub::{LogWindow, ScrubContext};
use crate::capture::CaptureBackend;
use crate::config::{Config, OcrSetting};

impl SystemInfo {
    /// Read the setup from `config`, the environment, and the markers in
    /// `config.data_dir`. `capture_backend` is the backend the daemon uses
    /// (or would select), and `daemon_running` whether it answered.
    pub fn collect(config: &Config, capture_backend: CaptureBackend, daemon_running: bool) -> Self {
        let cpu_threads = std::thread::available_parallelism()
            .map(|n| n.get())
            .unwrap_or(1);
        // The gate and its counters live in the daemon, which writes them
        // to the status file. Only a status this tracker's daemon wrote is
        // used: while a daemon runs, the file has to carry its pid, or it
        // is another run's (or an older daemon never wrote one).
        let running_pid = daemon_running
            .then(|| std::fs::read_to_string(config.data_dir.join("daemon.pid")).ok())
            .flatten()
            .and_then(|text| crate::proc_id::parse_pid_record(&text))
            .map(|record| record.pid);
        let switch = switch_for_report(
            crate::ocr_switch::read_status(&config.data_dir),
            daemon_running,
            running_pid,
        );
        let ocr = switch.as_ref().map_or(OcrSetting::On, |s| s.effective);
        let session_type = SessionType::from_env();
        Self {
            session_type,
            capture_backend,
            cpu_threads: u32::try_from(cpu_threads).unwrap_or(u32::MAX).max(1),
            ocr_threads: u32::try_from(config.ocr_threads_resolved())
                .unwrap_or(1)
                .max(1),
            reader: crate::ocr_switch::effective_reader(ocr, config.reader),
            ocr,
            ocr_blocked_calls: switch.map_or_else(Vec::new, |s| blocked_sites(&s)),
            shadow_recognizer: config.shadow_recognizer_enabled(),
            prerelease: config.wants_prerelease_updates(),
            daemon_running,
            last_exit: marker::last_exit(&config.data_dir),
            host: Some(crate::host_probe::host_line_now(session_type.as_str())),
        }
    }
}

/// The status the report may use. With a daemon running, only the file that
/// daemon wrote (its pid). With none running, the last run's file.
pub(super) fn switch_for_report(
    status: Option<crate::ocr_switch::Status>,
    daemon_running: bool,
    running_pid: Option<u32>,
) -> Option<crate::ocr_switch::Status> {
    status.filter(|s| !daemon_running || running_pid == Some(s.pid))
}

/// The blocked call sites of a run with OCR off, as the report sends them.
/// Empty when the run had OCR on.
///
/// The status is a file read from disk, so nothing in it is trusted to fit
/// what the site takes (`docs/report-bundle.md`, section 2):
/// - a key that is not a call site
///   ([`scuffed_types::stat_report::is_ocr_call_site`]: an absolute path,
///   free text, over 96 bytes) is left out, so one key can not make the
///   site refuse the whole report;
/// - the same key twice is one site, counts added;
/// - a count above `u32::MAX` is sent as `u32::MAX`;
/// - only the [`OCR_BLOCKED_SITES_MAX`] sites with the largest counts go,
///   the busiest first.
pub(super) fn blocked_sites(status: &crate::ocr_switch::Status) -> Vec<(String, u32)> {
    if status.effective.is_on() {
        return Vec::new();
    }
    let mut by_key: std::collections::BTreeMap<&str, u64> = Default::default();
    for s in &status.blocked {
        if scuffed_types::stat_report::is_ocr_call_site(&s.site) {
            let count = by_key.entry(s.site.as_str()).or_default();
            *count = count.saturating_add(s.count);
        }
    }
    let mut sites: Vec<(String, u32)> = by_key
        .into_iter()
        .map(|(site, count)| (site.to_string(), u32::try_from(count).unwrap_or(u32::MAX)))
        .collect();
    sites.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
    sites.truncate(OCR_BLOCKED_SITES_MAX);
    sites
}

impl LogReportInput {
    /// Input for this machine: scrub values and system object from `config`,
    /// logs from `config.data_dir`, and the section 6 window for a report
    /// built at `built_at` (plus the crash window for `crash`). `names` is
    /// the caller's scrub list of player names (other players, misreads,
    /// short names) it knows were read in the window.
    #[allow(clippy::too_many_arguments)]
    pub fn from_config(
        config: &Config,
        category: LogOnlyCategory,
        text: String,
        resolution: Resolution,
        capture_backend: CaptureBackend,
        daemon_running: bool,
        names: Vec<String>,
        built_at: DateTime<Utc>,
    ) -> Self {
        let window = LogWindow::for_log_only(
            built_at,
            category == LogOnlyCategory::Crash,
            marker::last_crash_seen(&config.data_dir),
        );
        let mut scrub = ScrubContext::from_config(config);
        scrub.names = names;
        Self {
            category,
            text,
            resolution,
            ui_scale: None,
            system: SystemInfo::collect(config, capture_backend, daemon_running),
            training: false,
            scrub,
            window,
            log_dir: config.data_dir.clone(),
        }
    }
}

/// The member's edits on the newest snapshot of `session_id`, by read key.
/// The original read stays in `reads` (section 2).
pub fn session_corrections(
    snapshot: &crate::storage::Snapshot,
    session_id: &str,
) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let Some(m) = snapshot.matches.iter().find(|m| m.session_id == session_id) else {
        return out;
    };
    let mut put = |key: &str, v: Option<String>| {
        if let Some(v) = v.map(|v| v.trim().chars().take(64).collect::<String>())
            && !v.is_empty()
        {
            out.insert(key.to_string(), v);
        }
    };
    put("hero", m.corrected_hero.clone());
    put("result", m.corrected_outcome.clone());
    put("e", m.corrected_elims.map(|v| v.to_string()));
    put("a", m.corrected_assists.map(|v| v.to_string()));
    put("d", m.corrected_deaths.map(|v| v.to_string()));
    put("dmg", m.corrected_damage.map(|v| v.to_string()));
    put("h", m.corrected_healing.map(|v| v.to_string()));
    put("mit", m.corrected_mitigation.map(|v| v.to_string()));
    out
}

impl GameReportInput {
    /// Input for this machine: the session's kit, the member's edits from
    /// the live snapshot, the session log window (section 6), scrub values
    /// and the system object from `config`. `names` is the caller's scrub
    /// list of player names read in the window.
    #[allow(clippy::too_many_arguments)]
    pub fn from_config(
        config: &Config,
        session_id: &str,
        category: GameCategory,
        text: String,
        capture_backend: CaptureBackend,
        daemon_running: bool,
        names: Vec<String>,
        built_at: DateTime<Utc>,
    ) -> Result<Self, ReportError> {
        let mut kit = super::kit::load(&config.data_dir, session_id);
        let snapshot = crate::storage::read_snapshot(&config.data_dir);
        // A game from before alpha.8 (or one whose kit was pruned) has no
        // saved reads. Its stored row still names the game, so the report
        // carries those values and the session id, with no pictures.
        if kit.reads.is_none()
            && let Some(m) = snapshot
                .as_ref()
                .and_then(|s| s.matches.iter().find(|m| m.session_id == session_id))
        {
            let at: DateTime<Utc> = m.played_at.into();
            let mut reads = super::kit::KitReads::from_capture(
                m,
                m,
                0,
                0,
                super::blank::TeamLayout::Unknown,
                at,
            );
            reads.map = Some(m.display_map_name().to_string()).filter(|s| !s.trim().is_empty());
            kit = super::kit::Kit {
                scoreboard_png: None,
                outcome_png: None,
                reads: Some(reads),
            };
        }
        // The kit is written at Tab time, usually before the result screen.
        // The stored row has the result the tracker uploaded.
        if let (Some(reads), Some(m)) = (
            kit.reads.as_mut(),
            snapshot
                .as_ref()
                .and_then(|s| s.matches.iter().find(|m| m.session_id == session_id)),
        ) {
            reads.fill_result(&m.outcome);
        }
        if kit.reads.is_none() && category != GameCategory::MissedGame {
            return Err(ReportError::InvalidInput(
                "this game is not in the tracker's history",
            ));
        }
        let corrections = snapshot
            .as_ref()
            .map(|s| session_corrections(s, session_id))
            .unwrap_or_default();
        let session = snapshot
            .as_ref()
            .and_then(|s| s.sessions.iter().find(|x| x.session_id == session_id));
        let window = match (session, kit.reads.as_ref()) {
            (Some(s), _) => LogWindow::session(s.started_at.into(), s.last_capture_at.into()),
            (None, Some(r)) => LogWindow::session(r.captured_at, r.captured_at),
            (None, None) => LogWindow::log_only(built_at),
        };
        let resolution = kit
            .reads
            .as_ref()
            .filter(|r| r.width > 0 && r.height > 0)
            .map(|r| Resolution {
                width: r.width.max(r.height),
                height: r.width.min(r.height),
            })
            .unwrap_or(Resolution {
                width: 1920,
                height: 1080,
            });
        let mut scrub = ScrubContext::from_config(config);
        scrub.names = names;
        Ok(Self {
            session_id: session_id.to_string(),
            category,
            text,
            resolution,
            ui_scale: None,
            system: SystemInfo::collect(config, capture_backend, daemon_running),
            training: false,
            scrub,
            window,
            log_dir: config.data_dir.clone(),
            kit,
            corrections,
            built_at,
        })
    }
}
