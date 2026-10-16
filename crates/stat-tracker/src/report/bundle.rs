//! Log-only report bundles (report-bundle spec sections 2 and 7).
//!
//! A `crash`, `performance`, or `other` report from "Report a problem" holds
//! `manifest.json` and a scrubbed `log.txt`, and nothing else. There is no
//! game, no crops, and both name consents are false.
//!
//! [`build_log_report`] builds the zip in memory. The app shows the preview
//! from [`ReportZip`] (or by reading `bytes` back), and Send and Save report
//! use those same bytes.

use std::io::{self, Write};
use std::path::PathBuf;

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::marker::{self, LastExit};
use super::scrub::{self, LogWindow, ScrubContext};
use crate::capture::CaptureBackend;
use crate::config::{Config, OcrSetting, ReaderSetting};

/// `bundle_version` this build writes.
pub const BUNDLE_VERSION: u64 = 2;
/// `recognizers.ocr`.
pub const OCR_ID: &str = "ocr-v1";
/// `app_version`: the tracker version of this build.
pub const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
/// `session_id` of a report that is not about one game.
pub const NO_SESSION: &str = "none";
/// Longest `reason.text`, in characters.
pub const REASON_TEXT_MAX_CHARS: usize = 500;

pub const MANIFEST_PATH: &str = "manifest.json";
pub const LOG_PATH: &str = "log.txt";

/// Report a problem categories. Game categories need a game report.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LogOnlyCategory {
    Crash,
    Performance,
    Other,
}

impl LogOnlyCategory {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Crash => "crash",
            Self::Performance => "performance",
            Self::Other => "other",
        }
    }
}

/// `system.session_type`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SessionType {
    X11,
    Wayland,
    Unknown,
}

impl SessionType {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::X11 => "x11",
            Self::Wayland => "wayland",
            Self::Unknown => "unknown",
        }
    }

    /// `XDG_SESSION_TYPE` when it is `x11` or `wayland`, else
    /// `WAYLAND_DISPLAY`, else `DISPLAY`. Empty values count as unset.
    pub fn detect(
        xdg_session_type: Option<&str>,
        wayland_display: Option<&str>,
        display: Option<&str>,
    ) -> Self {
        let set = |v: Option<&str>| v.is_some_and(|s| !s.trim().is_empty());
        match xdg_session_type
            .map(|s| s.trim().to_ascii_lowercase())
            .as_deref()
        {
            Some("x11") => return Self::X11,
            Some("wayland") => return Self::Wayland,
            _ => {}
        }
        if set(wayland_display) {
            Self::Wayland
        } else if set(display) {
            Self::X11
        } else {
            Self::Unknown
        }
    }

    pub fn from_env() -> Self {
        let var = |k: &str| std::env::var(k).ok();
        Self::detect(
            var("XDG_SESSION_TYPE").as_deref(),
            var("WAYLAND_DISPLAY").as_deref(),
            var("DISPLAY").as_deref(),
        )
    }
}

/// `system.capture_backend`.
pub const fn capture_backend_label(backend: CaptureBackend) -> &'static str {
    match backend {
        CaptureBackend::X11 => "x11",
        CaptureBackend::Wayshot => "wayshot",
        CaptureBackend::Portal => "portal",
        CaptureBackend::None => "none",
    }
}

/// The manifest `system` object. Describes the setup, not the person.
#[derive(Debug, Clone, PartialEq)]
pub struct SystemInfo {
    pub session_type: SessionType,
    pub capture_backend: CaptureBackend,
    /// Machine's available parallelism, at least 1.
    pub cpu_threads: u32,
    /// OCR worker count in use, at least 1.
    pub ocr_threads: u32,
    /// The reader that supplied saved stats: `new` when the daemon ran
    /// with OCR off, whatever the `reader` key says
    /// ([`crate::ocr_switch::effective_reader`]).
    pub reader: ReaderSetting,
    /// What the daemon ran with, from the status file it wrote
    /// ([`crate::ocr_switch::Status`]). On when no daemon wrote one: the
    /// config value alone never says off.
    pub ocr: OcrSetting,
    /// OCR calls the daemon's gate blocked, per call site
    /// ([`crate::ocr::gate::Site::key`]): at most
    /// [`OCR_BLOCKED_SITES_MAX`], the busiest first. Empty with OCR on.
    pub ocr_blocked_calls: Vec<(String, u32)>,
    pub shadow_recognizer: bool,
    /// True for the prerelease channel.
    pub prerelease: bool,
    pub daemon_running: bool,
    pub last_exit: LastExit,
}

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
        Self {
            session_type: SessionType::from_env(),
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
        }
    }
}

/// Most call sites `system.ocr_blocked_calls` carries: the validator's cap.
pub const OCR_BLOCKED_SITES_MAX: usize = scuffed_types::stat_report::OCR_BLOCKED_SITES_MAX;

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

/// Capture size in pixels. Both edges must be positive.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Resolution {
    pub width: u32,
    pub height: u32,
}

/// Everything [`build_log_report`] needs. Build it with
/// [`LogReportInput::from_config`] in the app; tests fill it by hand.
#[derive(Debug, Clone)]
pub struct LogReportInput {
    pub category: LogOnlyCategory,
    /// Member's free text. Scrubbed, then cut to 500 characters.
    pub text: String,
    /// Capture size of the selected output. The manifest field is required
    /// and positive even when the report has no frame.
    pub resolution: Resolution,
    /// What the member typed, or `None`.
    pub ui_scale: Option<f64>,
    pub system: SystemInfo,
    /// Training box. Off unless the member ticked it.
    pub training: bool,
    pub scrub: ScrubContext,
    pub window: LogWindow,
    /// Directory holding `daemon.log` and `daemon.log.1`.
    pub log_dir: PathBuf,
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

/// One `files` entry, as written into the manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ReportFile {
    pub path: String,
    pub sha256: String,
    pub bytes: u64,
    pub role: &'static str,
    pub screen_class: Option<&'static str>,
}

/// A sealed report zip and what the preview lists.
#[derive(Debug, Clone)]
pub struct ReportZip {
    /// The zip that Send uploads and Save report writes.
    pub bytes: Vec<u8>,
    /// `manifest.json` exactly as stored in the zip.
    pub manifest_json: String,
    /// The manifest `files` list (every file except `manifest.json`).
    pub files: Vec<ReportFile>,
    /// `log.txt` exactly as stored in the zip, for the preview.
    pub log_text: String,
    /// Every image in the zip, the same bytes, for the preview.
    pub images: Vec<ReportImage>,
}

/// One image of a [`ReportZip`], as stored in the zip.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReportImage {
    pub path: String,
    pub png: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

#[derive(Debug)]
pub enum ReportError {
    InvalidInput(&'static str),
    Zip(String),
    Io(io::Error),
}

impl std::fmt::Display for ReportError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::InvalidInput(m) => write!(f, "invalid report input: {m}"),
            Self::Zip(m) => write!(f, "could not write the report zip: {m}"),
            Self::Io(e) => write!(f, "report i/o error: {e}"),
        }
    }
}

impl std::error::Error for ReportError {}

impl From<io::Error> for ReportError {
    fn from(e: io::Error) -> Self {
        Self::Io(e)
    }
}

impl From<zip::result::ZipError> for ReportError {
    fn from(e: zip::result::ZipError) -> Self {
        Self::Zip(e.to_string())
    }
}

#[derive(Serialize)]
struct Manifest<'a> {
    bundle_version: u64,
    app_version: &'a str,
    recognizers: Recognizers,
    resolution: Resolution,
    ui_scale: Option<f64>,
    reason: Reason<'a>,
    system: SystemJson,
    session_id: &'a str,
    game: Option<GameJson<'a>>,
    reads: serde_json::Value,
    corrections: std::collections::BTreeMap<String, String>,
    consent: Consent,
    files: &'a [ReportFile],
}

#[derive(Serialize)]
struct GameJson<'a> {
    map: Option<&'a str>,
    mode: Option<&'a str>,
    result: Option<&'a str>,
    team_size: Option<u32>,
    captured_at: String,
}

#[derive(Serialize)]
struct Recognizers {
    matcher: &'static str,
    ocr: &'static str,
}

#[derive(Serialize)]
struct Reason<'a> {
    category: &'static str,
    text: &'a str,
}

#[derive(Serialize)]
struct SystemJson {
    session_type: &'static str,
    capture_backend: &'static str,
    cpu_threads: u32,
    ocr_threads: u32,
    reader: &'static str,
    /// `off`, and only then. A tracker with classic OCR on writes the
    /// `system` object it always wrote, so a site from before the switch
    /// still takes its reports; the site reads a missing key as `on`.
    #[serde(skip_serializing_if = "Option::is_none")]
    ocr: Option<&'static str>,
    /// Written with `ocr`: call site to blocked calls.
    #[serde(skip_serializing_if = "Option::is_none")]
    ocr_blocked_calls: Option<std::collections::BTreeMap<String, u32>>,
    shadow_recognizer: bool,
    update_channel: &'static str,
    daemon_running: bool,
    last_exit: &'static str,
}

/// A read with no value: `value`, `confidence`, and `ocr_v1` null, and
/// `suspect` true (a null value is always suspect).
#[derive(Serialize)]
struct EmptyRead {
    value: Option<()>,
    confidence: Option<()>,
    suspect: bool,
    ocr_v1: Option<()>,
}

impl Default for EmptyRead {
    fn default() -> Self {
        Self {
            value: None,
            confidence: None,
            suspect: true,
            ocr_v1: None,
        }
    }
}

/// The nine `reads` keys of a log-only report, each an [`EmptyRead`].
#[derive(Serialize, Default)]
struct NullReads {
    mode: EmptyRead,
    result: EmptyRead,
    hero: EmptyRead,
    e: EmptyRead,
    a: EmptyRead,
    d: EmptyRead,
    dmg: EmptyRead,
    h: EmptyRead,
    mit: EmptyRead,
}

#[derive(Serialize)]
struct Consent {
    training: bool,
    own_name_included: bool,
    glyphs_included: bool,
}

impl From<&SystemInfo> for SystemJson {
    fn from(s: &SystemInfo) -> Self {
        Self {
            session_type: s.session_type.as_str(),
            capture_backend: capture_backend_label(s.capture_backend),
            cpu_threads: s.cpu_threads.max(1),
            ocr_threads: s.ocr_threads.max(1),
            reader: s.reader.as_str(),
            ocr: s.ocr.is_off().then_some(OcrSetting::OFF),
            ocr_blocked_calls: s
                .ocr
                .is_off()
                .then(|| s.ocr_blocked_calls.iter().cloned().collect()),
            shadow_recognizer: s.shadow_recognizer,
            update_channel: if s.prerelease { "prerelease" } else { "stable" },
            daemon_running: s.daemon_running,
            last_exit: s.last_exit.as_str(),
        }
    }
}

pub fn sha256_hex(bytes: &[u8]) -> String {
    use std::fmt::Write as _;
    let digest = Sha256::digest(bytes);
    let mut out = String::with_capacity(64);
    for b in digest {
        let _ = write!(out, "{b:02x}");
    }
    out
}

/// Scrubbed `reason.text`: section 6 rules, no NUL, at most 500 characters.
pub fn scrub_reason_text(ctx: &ScrubContext, text: &str) -> String {
    let cleaned: String = text.chars().filter(|c| *c != '\0').collect();
    scrub::scrub_text(ctx, &cleaned)
        .chars()
        .take(REASON_TEXT_MAX_CHARS)
        .collect()
}

/// Build a `crash`, `performance`, or `other` report zip.
pub fn build_log_report(input: &LogReportInput) -> Result<ReportZip, ReportError> {
    if input.resolution.width == 0 || input.resolution.height == 0 {
        return Err(ReportError::InvalidInput("resolution must be positive"));
    }
    if let Some(scale) = input.ui_scale
        && (!scale.is_finite() || scale <= 0.0 || scale > 200.0)
    {
        return Err(ReportError::InvalidInput("ui_scale must be in (0, 200]"));
    }

    let raw_log = scrub::read_daemon_logs(&input.log_dir);
    let log_text = scrub::scrub_log(&input.scrub, &input.window, &raw_log);
    let reason_text = scrub_reason_text(&input.scrub, &input.text);

    let files = vec![ReportFile {
        path: LOG_PATH.to_string(),
        sha256: sha256_hex(log_text.as_bytes()),
        bytes: log_text.len() as u64,
        role: "log",
        screen_class: None,
    }];
    let manifest = Manifest {
        bundle_version: BUNDLE_VERSION,
        app_version: APP_VERSION,
        recognizers: Recognizers {
            // OCR on or off is also in `system.ocr`; `ocr` stays ocr-v1.
            matcher: crate::shadow::digits::recognizer_id(input.system.ocr != OcrSetting::Off),
            ocr: OCR_ID,
        },
        resolution: input.resolution,
        ui_scale: input.ui_scale,
        reason: Reason {
            category: input.category.as_str(),
            text: &reason_text,
        },
        system: SystemJson::from(&input.system),
        session_id: NO_SESSION,
        game: None,
        reads: serde_json::to_value(NullReads::default())
            .map_err(|_| ReportError::InvalidInput("reads did not serialize"))?,
        corrections: Default::default(),
        consent: Consent {
            training: input.training,
            own_name_included: false,
            glyphs_included: false,
        },
        files: &files,
    };
    let mut manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|_| ReportError::InvalidInput("manifest did not serialize"))?;
    manifest_json.push('\n');

    let bytes = write_zip(&[
        (MANIFEST_PATH, manifest_json.as_bytes()),
        (LOG_PATH, log_text.as_bytes()),
    ])?;
    Ok(ReportZip {
        bytes,
        manifest_json,
        files,
        log_text,
        images: Vec::new(),
    })
}

/// Deflated zip, every entry dated 1980-01-01 00:00:00, no comment, no
/// extra fields.
fn write_zip(entries: &[(&str, &[u8])]) -> Result<Vec<u8>, ReportError> {
    let opts = zip::write::SimpleFileOptions::default()
        .compression_method(zip::CompressionMethod::Deflated)
        .last_modified_time(zip::DateTime::default());
    let mut zw = zip::ZipWriter::new(io::Cursor::new(Vec::new()));
    for (path, data) in entries {
        zw.start_file(*path, opts)?;
        zw.write_all(data)?;
    }
    Ok(zw.finish()?.into_inner())
}

// --- Game reports (sections 2, 3, 7) ---

/// `reason.category` of a report about one game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GameCategory {
    WrongStats,
    WrongHero,
    WrongMap,
    WrongMode,
    WrongResult,
    MissedGame,
    Other,
}

impl GameCategory {
    pub const ALL: [Self; 7] = [
        Self::WrongStats,
        Self::WrongHero,
        Self::WrongMap,
        Self::WrongMode,
        Self::WrongResult,
        Self::MissedGame,
        Self::Other,
    ];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::WrongStats => "wrong_stats",
            Self::WrongHero => "wrong_hero",
            Self::WrongMap => "wrong_map",
            Self::WrongMode => "wrong_mode",
            Self::WrongResult => "wrong_result",
            Self::MissedGame => "missed_game",
            Self::Other => "other",
        }
    }

    /// Text for the category chips.
    pub const fn label(self) -> &'static str {
        match self {
            Self::WrongStats => "Wrong stats",
            Self::WrongHero => "Wrong hero",
            Self::WrongMap => "Wrong map",
            Self::WrongMode => "Wrong mode",
            Self::WrongResult => "Wrong result",
            Self::MissedGame => "Missed game",
            Self::Other => "Other",
        }
    }
}

/// Everything [`build_game_report`] needs. Build it with
/// [`GameReportInput::from_config`] in the app; tests fill it by hand.
#[derive(Debug, Clone)]
pub struct GameReportInput {
    pub session_id: String,
    pub category: GameCategory,
    /// Member's free text. Scrubbed, then cut to 500 characters.
    pub text: String,
    pub resolution: Resolution,
    pub ui_scale: Option<f64>,
    pub system: SystemInfo,
    /// Training box. Off unless the member ticked it.
    pub training: bool,
    pub scrub: ScrubContext,
    pub window: LogWindow,
    pub log_dir: PathBuf,
    /// The session's report kit (blanked crops and reads).
    pub kit: super::kit::Kit,
    /// The member's edits, by read key (`hero`, `result`, `e`, ...).
    pub corrections: std::collections::BTreeMap<String, String>,
    /// When the report was built. A missed game with no kit uses it as
    /// `game.captured_at`.
    pub built_at: DateTime<Utc>,
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

/// The zip may not be bigger than this (section 8).
pub const MAX_ZIP_BYTES: usize = 10 * 1024 * 1024;

/// Files in the zip besides `manifest.json` (the site's limit).
pub const MAX_FILES: usize = 30;

/// Widest and tallest picture the site accepts, in pixels.
pub const MAX_IMAGE_SIDE: u32 = 7680;

/// The pictures a game report for `kit` would carry, for a preview before
/// a report is built. Same rule as the report: only a trusted kit with
/// reads gives pictures, and an unreadable or oversized one is left out.
pub fn kit_preview_images(kit: &super::kit::Kit) -> Vec<ReportImage> {
    if kit.reads.is_none() || !super::kit::is_trusted(kit) {
        return Vec::new();
    }
    [
        ("crops/scoreboard.png", kit.scoreboard_png.as_ref()),
        ("crops/outcome.png", kit.outcome_png.as_ref()),
    ]
    .into_iter()
    .filter_map(|(path, png)| {
        let png = png?;
        let (width, height) = png_size(png).ok()?;
        (width <= MAX_IMAGE_SIDE && height <= MAX_IMAGE_SIDE).then(|| ReportImage {
            path: path.to_string(),
            png: png.clone(),
            width,
            height,
        })
    })
    .collect()
}

fn png_size(png: &[u8]) -> Result<(u32, u32), ReportError> {
    let img = image::load_from_memory_with_format(png, image::ImageFormat::Png)
        .map_err(|_| ReportError::InvalidInput("a saved crop is not a PNG"))?;
    Ok((img.width(), img.height()))
}

/// Build a report about one game: the blanked scoreboard, the rank screen
/// result word when the tracker saw it, the scrubbed session log, the reads,
/// and the member's edits.
pub fn build_game_report(input: &GameReportInput) -> Result<ReportZip, ReportError> {
    if input.resolution.width == 0 || input.resolution.height == 0 {
        return Err(ReportError::InvalidInput("resolution must be positive"));
    }
    if let Some(scale) = input.ui_scale
        && (!scale.is_finite() || scale <= 0.0 || scale > 200.0)
    {
        return Err(ReportError::InvalidInput("ui_scale must be in (0, 200]"));
    }
    // R2-4 on #236: the same checks as `kit::load`, again at the sink. A kit
    // from another blank table, or a scoreboard that is not the one its
    // reads were written with, counts as no kit: none of its images or
    // reads go into the zip.
    let trusted = super::kit::is_trusted(&input.kit);
    if input.kit.reads.is_some() && !trusted && input.category != GameCategory::MissedGame {
        return Err(ReportError::InvalidInput(
            "the saved scoreboard does not match its reads or this blanker",
        ));
    }
    let reads = input.kit.reads.as_ref().filter(|_| trusted);
    if reads.is_none() && input.category != GameCategory::MissedGame {
        return Err(ReportError::InvalidInput("this game has no saved reads"));
    }

    // The journal is read from the game's own window start (section 6).
    let raw_log =
        scrub::read_daemon_logs_since(&input.log_dir, input.window.start(), input.window.end());
    let log_text = scrub::scrub_log(&input.scrub, &input.window, &raw_log);
    let reason_text = scrub_reason_text(&input.scrub, &input.text);

    let mut images = Vec::new();
    for (path, png) in [
        ("crops/scoreboard.png", input.kit.scoreboard_png.as_ref()),
        ("crops/outcome.png", input.kit.outcome_png.as_ref()),
    ] {
        if let (Some(png), Some(_)) = (png, reads) {
            let (width, height) = png_size(png)?;
            if width > MAX_IMAGE_SIDE || height > MAX_IMAGE_SIDE {
                return Err(ReportError::InvalidInput(
                    "a picture is larger than 7680 px on a side",
                ));
            }
            images.push(ReportImage {
                path: path.to_string(),
                png: png.clone(),
                width,
                height,
            });
        }
    }

    let mut files = vec![ReportFile {
        path: LOG_PATH.to_string(),
        sha256: sha256_hex(log_text.as_bytes()),
        bytes: log_text.len() as u64,
        role: "log",
        screen_class: None,
    }];
    for img in &images {
        files.push(ReportFile {
            path: img.path.clone(),
            sha256: sha256_hex(&img.png),
            bytes: img.png.len() as u64,
            role: "crop",
            screen_class: Some(if img.path == "crops/outcome.png" {
                "outcome"
            } else {
                "scoreboard"
            }),
        });
    }

    if files.len() > MAX_FILES {
        return Err(ReportError::InvalidInput("a report holds at most 30 files"));
    }

    // Every game category carries a game object. A missed game with no kit
    // has nothing read: empty fields, the build time, and empty reads.
    let secs = |t: DateTime<Utc>| t.to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
    let game = Some(match reads {
        Some(r) => GameJson {
            map: r.map.as_deref(),
            mode: r.mode.as_deref(),
            result: r.result.as_deref(),
            team_size: r.team_size,
            captured_at: secs(r.captured_at),
        },
        None => GameJson {
            map: None,
            mode: None,
            result: None,
            team_size: None,
            captured_at: secs(input.built_at),
        },
    });
    let reads_json = match reads {
        Some(r) => {
            let mut map = serde_json::Map::new();
            for key in ["mode", "result", "hero", "e", "a", "d", "dmg", "h", "mit"] {
                let read = r.reads.get(key).cloned().unwrap_or(super::kit::KitRead {
                    value: None,
                    confidence: None,
                    ocr_v1: None,
                    suspect: true,
                });
                map.insert(
                    key.to_string(),
                    serde_json::to_value(read)
                        .map_err(|_| ReportError::InvalidInput("reads did not serialize"))?,
                );
            }
            serde_json::Value::Object(map)
        }
        None => serde_json::to_value(NullReads::default())
            .map_err(|_| ReportError::InvalidInput("reads did not serialize"))?,
    };
    let session_id = if reads.is_some() {
        input.session_id.as_str()
    } else {
        NO_SESSION
    };
    let corrections = if reads.is_some() {
        input.corrections.clone()
    } else {
        Default::default()
    };
    let manifest = Manifest {
        bundle_version: BUNDLE_VERSION,
        app_version: APP_VERSION,
        recognizers: Recognizers {
            // OCR on or off is also in `system.ocr`; `ocr` stays ocr-v1.
            matcher: crate::shadow::digits::recognizer_id(input.system.ocr != OcrSetting::Off),
            ocr: OCR_ID,
        },
        resolution: input.resolution,
        ui_scale: input.ui_scale,
        reason: Reason {
            category: input.category.as_str(),
            text: &reason_text,
        },
        system: SystemJson::from(&input.system),
        session_id,
        game,
        reads: reads_json,
        corrections,
        consent: Consent {
            training: input.training,
            own_name_included: false,
            glyphs_included: false,
        },
        files: &files,
    };
    let mut manifest_json = serde_json::to_string_pretty(&manifest)
        .map_err(|_| ReportError::InvalidInput("manifest did not serialize"))?;
    manifest_json.push('\n');

    let mut entries: Vec<(&str, &[u8])> = vec![
        (MANIFEST_PATH, manifest_json.as_bytes()),
        (LOG_PATH, log_text.as_bytes()),
    ];
    for img in &images {
        entries.push((img.path.as_str(), img.png.as_slice()));
    }
    let bytes = write_zip(&entries)?;
    if bytes.len() > MAX_ZIP_BYTES {
        return Err(ReportError::InvalidInput("the report is bigger than 10 MB"));
    }
    Ok(ReportZip {
        bytes,
        manifest_json,
        files,
        log_text,
        images,
    })
}
