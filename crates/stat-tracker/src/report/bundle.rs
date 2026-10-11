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
use std::path::{Path, PathBuf};

use chrono::{DateTime, Utc};
use serde::Serialize;
use sha2::{Digest, Sha256};

use super::marker::{self, LastExit};
use super::scrub::{self, LogWindow, ScrubContext};
use crate::capture::CaptureBackend;
use crate::config::{Config, ReaderSetting};

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
        match xdg_session_type.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
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
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SystemInfo {
    pub session_type: SessionType,
    pub capture_backend: CaptureBackend,
    /// Machine's available parallelism, at least 1.
    pub cpu_threads: u32,
    /// OCR worker count in use, at least 1.
    pub ocr_threads: u32,
    pub reader: ReaderSetting,
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
        Self {
            session_type: SessionType::from_env(),
            capture_backend,
            cpu_threads: u32::try_from(cpu_threads).unwrap_or(u32::MAX).max(1),
            ocr_threads: u32::try_from(config.ocr_threads_resolved())
                .unwrap_or(1)
                .max(1),
            reader: config.reader,
            shadow_recognizer: config.shadow_recognizer_enabled(),
            prerelease: config.wants_prerelease_updates(),
            daemon_running,
            last_exit: marker::last_exit(&config.data_dir),
        }
    }
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
        let crash_at = match category {
            LogOnlyCategory::Crash => {
                marker::read_last_crash(&config.data_dir).and_then(|c| c.window_anchor())
            }
            _ => None,
        };
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
            window: LogWindow::log_only(built_at, crash_at),
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
    game: Option<()>,
    reads: NullReads,
    corrections: std::collections::BTreeMap<String, String>,
    consent: Consent,
    files: &'a [ReportFile],
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
    shadow_recognizer: bool,
    update_channel: &'static str,
    daemon_running: bool,
    last_exit: &'static str,
}

/// The nine `reads` keys, each `null` in a log-only report.
#[derive(Serialize, Default)]
struct NullReads {
    mode: Option<()>,
    result: Option<()>,
    hero: Option<()>,
    e: Option<()>,
    a: Option<()>,
    d: Option<()>,
    dmg: Option<()>,
    h: Option<()>,
    mit: Option<()>,
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
            matcher: crate::shadow::digits::RECOGNIZER_ID,
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
        reads: NullReads::default(),
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
