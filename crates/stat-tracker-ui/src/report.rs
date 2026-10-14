//! Report a problem (`crates/stat-tracker/docs/report-bundle.md` section 7).
//!
//! Log-only reports: the member picks `crash`, `performance`, or `other`,
//! types a short note, and reads a preview built from the exact zip that
//! would leave the PC. A game report (Games tab and the game card) is the
//! same dialog for one session: the category list starts at `wrong_stats`
//! and leaves out `crash` and `performance`. **Send** posts that zip to the
//! linked site with the tracker sync token (create only). **Save report**
//! writes the same zip to the Downloads folder after a private-file warning.
//! The daemon library builds the zip (`stat_tracker::report`). A game report
//! loads its kit with `kit::load`, which checks the blank-table version and
//! the scoreboard hash. This module is the dialog, the upload, the save
//! path, and the crash prompt.

use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};

use chrono::{DateTime, Local, Utc};
use iced::widget::{
    Row, button, checkbox, column, container, opaque, row, scrollable, text, text_input,
};
use iced::{Alignment, Element, Fill, Length, Padding};
use stat_tracker::capture::CaptureBackend;
use stat_tracker::config::Config;
use stat_tracker::report::bundle::REASON_TEXT_MAX_CHARS;
use stat_tracker::report::{
    GameReportInput, LogOnlyCategory, LogReportInput, ReportZip, Resolution,
};

use crate::app::Message;
use crate::theme::{
    self, FONT_BOLD, FONT_EXTRABOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_FEATURED,
    SIZE_LABEL, SIZE_META, SIZE_TITLE, TEXT, TEXT_2, TEXT_3,
};

/// A text edit waits this long before the preview is rebuilt.
pub const TEXT_REBUILD_DELAY: Duration = Duration::from_millis(700);

/// Used when the display size cannot be read. The manifest field is
/// required and must be positive.
pub const FALLBACK_RESOLUTION: Resolution = Resolution {
    width: 1920,
    height: 1080,
};

const PRIVATE_WARNING: &str = "This is a private report. Give it only to an officer, and \
     do not post it in a public channel. Once the file leaves this PC, the app cannot delete \
     the copy you gave away.";

const NAME_HINT: &str = "Do not type other players' names.";

/// Game-report categories. The manifest ids live in the tracker library.
pub use stat_tracker::report::GameCategory;

/// A missed game has no kit. Every other game category needs one.
pub trait GameReportCategory {
    fn allows_missing_kit(self) -> bool;
}

impl GameReportCategory for GameCategory {
    fn allows_missing_kit(self) -> bool {
        matches!(self, GameCategory::MissedGame)
    }
}

/// One row in the game picker. `has_kit` is [`stat_tracker::report::kit::has_kit`]:
/// a scoreboard `kit::load` accepts, not a bare file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GameChoice {
    pub session_id: String,
    pub label: String,
    pub has_kit: bool,
}

/// A preview raster. The zip owns the PNG bytes; this is only the handle
/// iced draws, built once so redraws reuse it.
#[derive(Clone, PartialEq, Eq)]
pub struct KitImage {
    pub path: String,
    pub width: u32,
    pub height: u32,
    pub handle: iced::widget::image::Handle,
}

impl std::fmt::Debug for KitImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("KitImage")
            .field("path", &self.path)
            .field("width", &self.width)
            .field("height", &self.height)
            .finish()
    }
}

/// True when [`stat_tracker::report::kit::load`] returns a scoreboard.
pub use stat_tracker::report::kit::has_kit;

/// Kit session ids, newest directory first. Each id passed [`has_kit`].
pub use stat_tracker::report::kit::sessions as kit_sessions;

/// Preview handles for the images stored in `zip`. The bytes stay in the zip.
pub fn kit_images_from_zip(zip: &ReportZip) -> Vec<KitImage> {
    zip.images
        .iter()
        .map(|image| KitImage {
            path: image.path.clone(),
            width: image.width,
            height: image.height,
            handle: iced::widget::image::Handle::from_bytes(image.png.clone()),
        })
        .collect()
}

/// Messages the dialog sends. Wrapped in [`Message::Report`].
#[derive(Debug, Clone)]
pub enum ReportMessage {
    /// Open the dialog, optionally with a category picked.
    Open(Option<LogOnlyCategory>),
    /// Open on a game. Training is visible and already ticked when `training` is set.
    OpenGame {
        session_id: Option<String>,
        category: GameCategory,
        training: bool,
        choices: Vec<GameChoice>,
    },
    Close,
    Category(LogOnlyCategory),
    GameCategory(GameCategory),
    PickGame(String),
    ToggleImageScale,
    Text(String),
    Training(bool),
    ToggleLog,
    /// A finished build. The `u64` is the build generation; stale ones are dropped.
    Built(u64, Result<Arc<ReportZip>, String>),
    Send,
    Sent(SendOutcome),
    /// First press shows the private-file warning; the second saves.
    Save,
    CancelSave,
    Saved(Result<PathBuf, String>),
    OpenMyReports(String),
    /// Crash prompt answers.
    CrashYes,
    CrashNo,
}

/// What a send produced, already turned into what the dialog shows.
#[derive(Debug, Clone, PartialEq)]
pub enum SendOutcome {
    Sent {
        id: String,
        expires_at: Option<DateTime<Utc>>,
        reports_url: String,
    },
    Failed(String),
}

#[derive(Debug, Clone, PartialEq)]
pub enum SendState {
    Idle,
    Sending,
    Done(SendOutcome),
}

#[derive(Debug, Clone)]
pub struct ReportDialog {
    pub category: LogOnlyCategory,
    /// Set when the report is about a game. Log chips are then unselected.
    pub game_category: Option<GameCategory>,
    pub game_session: Option<String>,
    pub game_choices: Vec<GameChoice>,
    /// Handles for the images in the current preview zip.
    pub kit_images: Vec<KitImage>,
    /// Draw kit pictures at their pixel size. Off fits them to the dialog.
    pub image_actual: bool,
    pub text: String,
    pub training: bool,
    /// Last finished build. Send and Save use exactly these bytes.
    pub built: Option<Result<Arc<ReportZip>, String>>,
    /// Generation of the build in flight, if any.
    pub building: Option<u64>,
    /// Generation the inputs are at. A build for an older one is stale.
    /// Drawn from [`next_generation`], so a build from a closed dialog can
    /// never match a newer dialog.
    pub generation: u64,
    /// Set by a text edit; the tick rebuilds once the typing pauses.
    pub text_edited_at: Option<Instant>,
    pub send: SendState,
    pub confirm_save: bool,
    pub saving: bool,
    pub saved: Option<Result<PathBuf, String>>,
    pub show_log: bool,
    /// The tracker has a sync token. Without one, Send is disabled.
    pub linked: bool,
}

impl ReportDialog {
    pub fn new(category: LogOnlyCategory, linked: bool) -> Self {
        Self {
            category,
            game_category: None,
            game_session: None,
            game_choices: Vec::new(),
            kit_images: Vec::new(),
            image_actual: false,
            text: String::new(),
            training: false,
            built: None,
            building: None,
            generation: 0,
            text_edited_at: None,
            send: SendState::Idle,
            confirm_save: false,
            saving: false,
            saved: None,
            show_log: false,
            linked,
        }
    }

    /// The preview matches the current inputs, so Send and Save may use it.
    pub fn preview_current(&self) -> bool {
        self.building.is_none()
            && self.text_edited_at.is_none()
            && matches!(self.built, Some(Ok(_)))
    }

    pub fn zip(&self) -> Option<Arc<ReportZip>> {
        match &self.built {
            Some(Ok(zip)) if self.preview_current() => Some(zip.clone()),
            _ => None,
        }
    }

    pub fn is_game(&self) -> bool {
        self.game_category.is_some()
    }

    /// Send posts the preview zip. A log zip is never sent from a game
    /// dialog, and a game zip is never sent from a log dialog.
    pub fn can_send(&self) -> bool {
        self.linked
            && self.preview_current()
            && self.preview_matches_kind()
            && !matches!(self.send, SendState::Sending)
            && !self.sent()
    }

    pub fn sent(&self) -> bool {
        matches!(self.send, SendState::Done(SendOutcome::Sent { .. }))
    }

    pub fn can_save(&self) -> bool {
        self.preview_current() && self.preview_matches_kind() && !self.saving && !self.sent()
    }

    /// The preview zip is a game report exactly when the dialog is one.
    fn preview_matches_kind(&self) -> bool {
        let Some(Ok(zip)) = &self.built else {
            return false;
        };
        manifest_has_game(&zip.manifest_json) == self.is_game()
    }

    pub fn busy(&self) -> bool {
        matches!(self.send, SendState::Sending) || self.saving
    }

    /// Inputs changed: start a new generation and forget the old outcome,
    /// which described a different zip.
    pub fn bump(&mut self) -> u64 {
        self.generation = next_generation();
        self.confirm_save = false;
        if !matches!(self.send, SendState::Sending) && !self.sent() {
            self.send = SendState::Idle;
        }
        self.generation
    }
}

/// `manifest.game` is an object on a game report and null on a log report.
fn manifest_has_game(manifest_json: &str) -> bool {
    serde_json::from_str::<serde_json::Value>(manifest_json)
        .ok()
        .and_then(|value| value.get("game").cloned())
        .is_some_and(|game| !game.is_null())
}

/// Process-wide build counter. Starts at 1; 0 means no build yet.
pub fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Open-dialog state: the crash prompt, or the dialog itself.
#[derive(Debug, Clone)]
pub enum ReportUi {
    CrashPrompt { last_seen: Option<DateTime<Utc>> },
    Dialog(Box<ReportDialog>),
}

/// Crash prompt for app start: `Some` when `last_crash.marker` exists.
/// Sending or answering No removes the marker, so its existence means
/// the member has not answered for this crash yet.
pub fn crash_prompt(data_dir: &Path) -> Option<ReportUi> {
    let marker = stat_tracker::report::marker::read_last_crash(data_dir)?;
    Some(ReportUi::CrashPrompt {
        last_seen: marker.map(|m| m.last_seen),
    })
}

/// Everything the build needs, copied out of the app.
pub struct BuildRequest {
    pub config: Config,
    pub category: LogOnlyCategory,
    pub text: String,
    pub training: bool,
    pub resolution: Resolution,
    pub capture_backend: CaptureBackend,
}

/// Build the zip off the UI thread. The scrub list starts from the
/// configured name and the log's name fields are dropped by the scrubber.
pub async fn build(req: BuildRequest) -> Result<Arc<ReportZip>, String> {
    tokio::task::spawn_blocking(move || {
        let daemon_running = stat_tracker::report::daemon_running(&req.config.data_dir);
        let mut input = LogReportInput::from_config(
            &req.config,
            req.category,
            req.text,
            req.resolution,
            req.capture_backend,
            daemon_running,
            Vec::new(),
            Utc::now(),
        );
        input.training = req.training;
        stat_tracker::report::build_log_report(&input)
            .map(Arc::new)
            .map_err(|e| format!("Could not build the report: {e}"))
    })
    .await
    .map_err(|_| "Could not build the report.".to_string())?
}

/// Everything a game-report build needs. The kit is not here: [`build_game`]
/// loads it with `kit::load` inside [`GameReportInput::from_config`].
pub struct GameBuildRequest {
    pub config: Config,
    pub session_id: String,
    pub category: GameCategory,
    pub text: String,
    pub training: bool,
    pub capture_backend: CaptureBackend,
}

/// Build a game-report zip off the UI thread. The scoreboard bytes are the
/// ones `kit::load` hash-checked, never a picture the dialog read itself.
pub async fn build_game(req: GameBuildRequest) -> Result<Arc<ReportZip>, String> {
    tokio::task::spawn_blocking(move || {
        let daemon_running = stat_tracker::report::daemon_running(&req.config.data_dir);
        let mut input = GameReportInput::from_config(
            &req.config,
            &req.session_id,
            req.category,
            req.text,
            req.capture_backend,
            daemon_running,
            Vec::new(),
            Utc::now(),
        )
        .map_err(|e| format!("Could not build the report: {e}"))?;
        input.training = req.training;
        stat_tracker::report::build_game_report(&input)
            .map(Arc::new)
            .map_err(|e| format!("Could not build the report: {e}"))
    })
    .await
    .map_err(|_| "Could not build the report.".to_string())?
}

/// `POST {base}/api/stat-reports` with the sync token.
pub fn create_url(server_url: &str) -> Result<String, String> {
    let base = crate::setup_guide::normalize_base(server_url)?;
    Ok(format!("{base}/api/stat-reports"))
}

pub fn my_reports_url(server_url: &str) -> Option<String> {
    crate::setup_guide::normalize_base(server_url)
        .ok()
        .map(|base| format!("{base}/reports"))
}

/// Upload the preview zip. Never retries on its own.
pub async fn send(server_url: String, token: String, bytes: Arc<ReportZip>) -> SendOutcome {
    let url = match create_url(&server_url) {
        Ok(url) => url,
        Err(e) => return SendOutcome::Failed(format!("The report was not sent. {e}")),
    };
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .build()
    {
        Ok(client) => client,
        Err(_) => return SendOutcome::Failed(not_sent_unreachable()),
    };
    let response = client
        .post(&url)
        .bearer_auth(&token)
        .header(reqwest::header::CONTENT_TYPE, "application/zip")
        .body(bytes.bytes.clone())
        .send()
        .await;
    let response = match response {
        Ok(response) => response,
        Err(_) => return SendOutcome::Failed(not_sent_unreachable()),
    };
    let status = response.status().as_u16();
    let retry_after = retry_after_secs(response.headers());
    let body = response.text().await.unwrap_or_default();
    outcome_for(status, retry_after, &body, my_reports_url(&server_url))
}

fn not_sent_unreachable() -> String {
    "The report was not sent. Could not reach the site. Try again, or use Save report.".into()
}

fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse().ok())
}

#[derive(serde::Deserialize)]
struct Created {
    id: String,
    expires_at: Option<DateTime<Utc>>,
}

/// Map a create response to what the dialog says (section 7).
pub fn outcome_for(
    status: u16,
    retry_after: Option<u64>,
    body: &str,
    reports_url: Option<String>,
) -> SendOutcome {
    match status {
        201 | 200 => match serde_json::from_str::<Created>(body) {
            Ok(created) => SendOutcome::Sent {
                id: created.id,
                expires_at: created.expires_at,
                reports_url: reports_url.unwrap_or_default(),
            },
            Err(_) => SendOutcome::Failed(
                "The site answered, but the reply could not be read. Check My reports on the \
                 site before sending again."
                    .into(),
            ),
        },
        401 => SendOutcome::Failed(
            "The tracker link is no longer valid. Link the tracker again in Settings, or use \
             Save report."
                .into(),
        ),
        403 => SendOutcome::Failed(
            "Your account cannot send reports right now. Ask an officer, or use Save report."
                .into(),
        ),
        // The site refused the report's contents. Sending it again gives
        // the same answer. It happens when the site is older than the
        // tracker and does not know a field the report carries, and also
        // when the report itself is not valid: the text is true for both.
        400 => SendOutcome::Failed("The site did not accept this report. Use Save report.".into()),
        413 => SendOutcome::Failed("The report is too large to send. Use Save report.".into()),
        429 => {
            let wait = retry_after
                .filter(|secs| *secs > 0)
                .map(|secs| format!(" Try again in {}.", wait_text(secs)))
                .unwrap_or_default();
            SendOutcome::Failed(format!(
                "You have reached the daily report limit.{wait} Save report still works."
            ))
        }
        503 if body.contains("reports_disabled") => SendOutcome::Failed(
            "Reports are off right now. Try Send again later, or use Save report.".into(),
        ),
        _ => SendOutcome::Failed(format!(
            "The report was not sent (HTTP {status}). Try again, or use Save report."
        )),
    }
}

fn wait_text(secs: u64) -> String {
    if secs < 90 {
        format!("{secs} seconds")
    } else if secs < 90 * 60 {
        format!("{} minutes", secs.div_ceil(60))
    } else {
        format!("{} hours", secs.div_ceil(3600))
    }
}

/// `XDG_DOWNLOAD_DIR` from `user-dirs.dirs`, else `$HOME/Downloads`.
pub fn download_dir(home: &Path, xdg_config_home: Option<&Path>) -> PathBuf {
    let config_home = xdg_config_home
        .filter(|p| p.is_absolute())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| home.join(".config"));
    fs::read_to_string(config_home.join("user-dirs.dirs"))
        .ok()
        .and_then(|text| parse_download_dir(&text, home))
        .unwrap_or_else(|| home.join("Downloads"))
}

/// The `XDG_DOWNLOAD_DIR="..."` line, with `$HOME` expanded. Only an
/// absolute path or one under `$HOME` is accepted, as xdg-user-dirs writes.
pub fn parse_download_dir(text: &str, home: &Path) -> Option<PathBuf> {
    let value = text.lines().rev().find_map(|line| {
        line.trim()
            .strip_prefix("XDG_DOWNLOAD_DIR=")
            .map(|v| v.trim().trim_matches('"').to_string())
    })?;
    let path = if let Some(rest) = value.strip_prefix("$HOME") {
        let rest = rest.trim_start_matches('/');
        if rest.is_empty() {
            return None;
        }
        home.join(rest)
    } else if value.starts_with('/') {
        PathBuf::from(value)
    } else {
        return None;
    };
    (path != home).then_some(path)
}

/// `scuffed-report-YYYYMMDDTHHMMSSZ.zip`.
pub fn report_file_name(now: DateTime<Utc>) -> String {
    format!("scuffed-report-{}.zip", now.format("%Y%m%dT%H%M%SZ"))
}

/// Write `bytes` into `dir` under `name`, or `name-2`, `name-3`, ... when
/// taken. Never overwrites. The file is private to this account.
pub fn save_zip(dir: &Path, name: &str, bytes: &[u8]) -> Result<PathBuf, String> {
    fs::create_dir_all(dir).map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    let stem = name.strip_suffix(".zip").unwrap_or(name);
    for n in 1..1000u32 {
        let file = if n == 1 {
            format!("{stem}.zip")
        } else {
            format!("{stem}-{n}.zip")
        };
        let path = dir.join(file);
        let mut options = fs::OpenOptions::new();
        options.write(true).create_new(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        match options.open(&path) {
            Ok(mut out) => {
                return out
                    .write_all(bytes)
                    .and_then(|()| out.sync_all())
                    .map(|()| path.clone())
                    .map_err(|e| {
                        let _ = fs::remove_file(&path);
                        format!("Could not write {}: {e}", path.display())
                    });
            }
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => return Err(format!("Could not write {}: {e}", path.display())),
        }
    }
    Err(format!("Too many saved reports in {}.", dir.display()))
}

/// Save into the Downloads folder, falling back to `$HOME/Downloads` when
/// the XDG folder cannot be created.
pub async fn save(bytes: Arc<ReportZip>) -> Result<PathBuf, String> {
    tokio::task::spawn_blocking(move || {
        let home = dirs::home_dir().ok_or("Could not find your home folder.")?;
        let xdg = std::env::var_os("XDG_CONFIG_HOME").map(PathBuf::from);
        let name = report_file_name(Utc::now());
        let dir = download_dir(&home, xdg.as_deref());
        match save_zip(&dir, &name, &bytes.bytes) {
            Ok(path) => Ok(path),
            Err(first) => {
                let fallback = home.join("Downloads");
                if fallback == dir {
                    return Err(first);
                }
                save_zip(&fallback, &name, &bytes.bytes)
            }
        }
    })
    .await
    .map_err(|_| "Could not save the report.".to_string())?
}

/// Physical size of the primary display, in pixels.
pub fn display_resolution() -> Resolution {
    use gtk::prelude::MonitorExt as _;
    let Some(display) = gtk::gdk::Display::default() else {
        return FALLBACK_RESOLUTION;
    };
    let Some(monitor) = display.primary_monitor().or_else(|| display.monitor(0)) else {
        return FALLBACK_RESOLUTION;
    };
    let rect = monitor.geometry();
    let scale = monitor.scale_factor().max(1);
    let (Ok(width), Ok(height)) = (
        u32::try_from(rect.width() * scale),
        u32::try_from(rect.height() * scale),
    ) else {
        return FALLBACK_RESOLUTION;
    };
    if width == 0 || height == 0 {
        return FALLBACK_RESOLUTION;
    }
    // A rotated or portrait monitor reports its edges the other way round.
    // Overwatch runs landscape, so the long edge is the width.
    Resolution {
        width: width.max(height),
        height: width.min(height),
    }
}

pub fn category_label(category: LogOnlyCategory) -> &'static str {
    match category {
        LogOnlyCategory::Crash => "Crash",
        LogOnlyCategory::Performance => "Performance",
        LogOnlyCategory::Other => "Other",
    }
}

fn category_hint(category: LogOnlyCategory) -> &'static str {
    match category {
        LogOnlyCategory::Crash => {
            "The tracker stopped or closed by itself. The log covers the last 30 minutes. \
             After a crash the app noticed, it also covers the 30 minutes before the stop and \
             2 minutes after."
        }
        LogOnlyCategory::Performance => {
            "Low FPS, stutter, or the tracker using a lot of CPU. The log covers the last \
             30 minutes."
        }
        LogOnlyCategory::Other => "Anything else. The log covers the last 30 minutes.",
    }
}

fn btn<'a>(label: &'a str, color: iced::Color) -> iced::widget::Button<'a, Message> {
    button(text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(color))
        .padding(Padding::from([8, 16]))
}

fn msg(m: ReportMessage) -> Message {
    Message::Report(m)
}

pub fn view<'a>(ui: &'a ReportUi) -> Element<'a, Message> {
    let card = match ui {
        ReportUi::CrashPrompt { last_seen } => crash_card(*last_seen),
        ReportUi::Dialog(dialog) => dialog_card(dialog),
    };
    let mut backdrop = theme::BG;
    backdrop.a = 0.88;
    opaque(
        container(card)
            .padding(28)
            .center(Fill)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(backdrop)),
                ..container::Style::default()
            }),
    )
}

fn crash_card(last_seen: Option<DateTime<Utc>>) -> Element<'static, Message> {
    let when = last_seen
        .map(|t| {
            format!(
                "It stopped around {}.",
                t.with_timezone(&Local).format("%Y-%m-%d %H:%M")
            )
        })
        .unwrap_or_default();
    let body = column![
        text("The tracker stopped unexpectedly. Send a report?")
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
        text(when).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_2),
        text("You see the report before anything is sent.")
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        row![
            btn("No", TEXT)
                .style(theme::ghost_btn())
                .on_press(msg(ReportMessage::CrashNo)),
            btn("Yes", TEXT)
                .style(theme::chip(true))
                .on_press(msg(ReportMessage::CrashYes)),
        ]
        .spacing(8),
    ]
    .spacing(10)
    .width(Fill);
    shell(body.into(), 300.0)
}

fn dialog_card(d: &ReportDialog) -> Element<'_, Message> {
    let mut body = column![
        text("Report a problem")
            .size(SIZE_FEATURED)
            .font(FONT_EXTRABOLD)
            .color(TEXT),
    ]
    .spacing(10)
    .width(Fill);

    let mut chips = Row::new().spacing(6);
    for category in [
        LogOnlyCategory::Crash,
        LogOnlyCategory::Performance,
        LogOnlyCategory::Other,
    ] {
        let selected = d.category == category;
        let mut chip = btn(
            category_label(category),
            if selected { TEXT } else { TEXT_2 },
        )
        .style(theme::chip(selected));
        if !d.busy() && !d.sent() {
            chip = chip.on_press(msg(ReportMessage::Category(category)));
        }
        chips = chips.push(chip);
    }
    body = body.push(chips);
    body = body.push(game_row(d));
    body = body.push(
        text(if let Some(category) = d.game_category {
            game_hint(category)
        } else {
            category_hint(d.category)
        })
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
    );

    let mut input = text_input("What happened? (optional)", &d.text)
        .padding(Padding::from([8, 10]))
        .size(SIZE_BODY)
        .style(theme::text_input_style)
        .width(Fill);
    if !d.busy() && !d.sent() {
        input = input.on_input(|t| msg(ReportMessage::Text(t)));
    }
    body = body.push(input);
    body = body.push(
        text(format!(
            "{NAME_HINT} Names are scrubbed anyway. Up to {REASON_TEXT_MAX_CHARS} characters."
        ))
        .size(SIZE_LABEL)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
    );

    let mut training = checkbox(d.training)
        .label(
            "Keep this report for training. Without this, it is deleted 30 days after it is \
             sent.",
        )
        .size(18)
        .text_size(SIZE_META)
        .font(FONT_SEMIBOLD)
        .style(crate::uninstall::checkbox_style);
    if !d.busy() && !d.sent() {
        training = training.on_toggle(|on| msg(ReportMessage::Training(on)));
    }
    body = body.push(training);

    body = body.push(preview_block(d));
    body = body.push(status_block(d));
    body = body.push(actions(d));

    shell(
        scrollable(body).height(Length::Shrink).width(Fill).into(),
        620.0,
    )
}

fn game_hint(category: GameCategory) -> &'static str {
    match category {
        GameCategory::MissedGame => "A game the tracker never recorded. No pictures are required.",
        _ => "The pictures are the blanked scoreboard and the result banner stored for this game.",
    }
}

fn game_row(d: &ReportDialog) -> Element<'_, Message> {
    let mut col = column![
        text("About a game")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2)
    ]
    .spacing(6)
    .width(Fill);
    // The dialog is 640px. Seven chips on one row run past the card.
    for row in GameCategory::ALL.chunks(4) {
        let mut chips = Row::new().spacing(6);
        for category in row.iter().copied() {
            let selected = d.game_category == Some(category);
            let mut chip = btn(category.label(), if selected { TEXT } else { TEXT_2 })
                .style(theme::chip(selected));
            if !d.busy() && !d.sent() {
                chip = chip.on_press(msg(ReportMessage::GameCategory(category)));
            }
            chips = chips.push(chip);
        }
        col = col.push(chips);
    }
    if d.game_category
        .is_some_and(|category| !category.allows_missing_kit())
    {
        col = col.push(game_picker(d));
    }
    if !d.kit_images.is_empty() {
        col = col.push(kit_pictures(d));
    } else if d.is_game()
        && d.building.is_none()
        && d.text_edited_at.is_none()
        && !d.game_category.is_some_and(|c| c.allows_missing_kit())
    {
        col = col.push(
            text("No pictures for this game yet.")
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
    }
    col.into()
}

fn game_picker(d: &ReportDialog) -> Element<'_, Message> {
    let mut col = column![].spacing(4).width(Fill);
    if d.game_choices.is_empty() {
        return text("No games on this PC yet.")
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3)
            .into();
    }
    for choice in &d.game_choices {
        let selected = d.game_session.as_deref() == Some(choice.session_id.as_str());
        let enabled = choice.has_kit;
        let mut chip = btn(
            &choice.label,
            if selected {
                TEXT
            } else if enabled {
                TEXT_2
            } else {
                TEXT_3
            },
        )
        .style(theme::chip(selected));
        if enabled && !d.busy() && !d.sent() {
            chip = chip.on_press(msg(ReportMessage::PickGame(choice.session_id.clone())));
        }
        col = col.push(chip);
    }
    col.into()
}

fn kit_pictures(d: &ReportDialog) -> Element<'_, Message> {
    let mut col = column![
        text("Pictures in this report. They are the blanked scoreboard and the result banner.")
            .size(SIZE_LABEL)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        btn(
            if d.image_actual {
                "Fit to the dialog"
            } else {
                "Actual size"
            },
            TEXT,
        )
        .style(theme::ghost_btn())
        .on_press(msg(ReportMessage::ToggleImageScale)),
    ]
    .spacing(6)
    .width(Fill);
    for image in &d.kit_images {
        let widget = iced::widget::image(image.handle.clone());
        let shown: Element<'_, Message> = if d.image_actual {
            scrollable(
                widget
                    .width(Length::Fixed(image.width as f32))
                    .height(Length::Fixed(image.height as f32)),
            )
            .direction(iced::widget::scrollable::Direction::Both {
                vertical: iced::widget::scrollable::Scrollbar::default(),
                horizontal: iced::widget::scrollable::Scrollbar::default(),
            })
            .width(Fill)
            .height(Length::Fixed(320.0))
            .into()
        } else {
            widget.width(Fill).into()
        };
        col = col.push(
            text(format!("{} {}×{}", image.path, image.width, image.height))
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
        );
        col = col.push(shown);
    }
    col.into()
}

fn preview_block(d: &ReportDialog) -> Element<'_, Message> {
    let mut col = column![
        text(if d.sent() { "Sent" } else { "What will be sent" })
            .size(SIZE_BODY)
            .font(FONT_BOLD)
            .color(TEXT)
    ]
    .spacing(6)
    .width(Fill);
    match &d.built {
        _ if d.building.is_some() || d.text_edited_at.is_some() => {
            col = col.push(
                text("Updating the preview...")
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_3),
            );
        }
        None => {}
        Some(Err(e)) => {
            col = col.push(
                text(e.as_str())
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(theme::DANGER),
            );
        }
        Some(Ok(zip)) => {
            let pictures = if d.is_game() {
                "The pictures above are the ones in the zip."
            } else {
                "No screenshots."
            };
            col = col.push(
                text(format!(
                    "One zip, {} compressed. {pictures} The log is scrubbed: your name, \
                     the sync token, the site address, your home folder, account name, and \
                     hostname are replaced.",
                    size_text(zip.bytes.len() as u64)
                ))
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
            );
            col = col.push(file_line(
                "manifest.json",
                zip.manifest_json.len() as u64,
                &stat_tracker::report::bundle::sha256_hex(zip.manifest_json.as_bytes()),
            ));
            for file in &zip.files {
                col = col.push(file_line(&file.path, file.bytes, &file.sha256));
            }
            col = col.push(
                btn(
                    if d.show_log {
                        "Hide the log"
                    } else {
                        "Show the log"
                    },
                    TEXT,
                )
                .style(theme::ghost_btn())
                .on_press(msg(ReportMessage::ToggleLog)),
            );
            if d.show_log {
                let log = if zip.log_text.trim().is_empty() {
                    "(no log lines in the window)"
                } else {
                    zip.log_text.as_str()
                };
                col = col.push(
                    container(
                        scrollable(
                            text(log)
                                .size(SIZE_LABEL)
                                .font(iced::Font::MONOSPACE)
                                .color(TEXT_2),
                        )
                        .height(Length::Fixed(220.0))
                        .width(Fill),
                    )
                    .padding(8)
                    .style(theme::stat_box),
                );
            }
        }
    }
    col.into()
}

fn file_line<'a>(path: &str, bytes: u64, sha256: &str) -> Element<'a, Message> {
    column![
        text(format!("{path}, {}", size_text(bytes)))
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
        text(format!("sha256 {sha256}"))
            .size(SIZE_LABEL)
            .font(iced::Font::MONOSPACE)
            .color(TEXT_3),
    ]
    .spacing(2)
    .into()
}

fn size_text(bytes: u64) -> String {
    if bytes < 1024 {
        format!("{bytes} bytes")
    } else if bytes < 1024 * 1024 {
        format!("{bytes} bytes, {:.1} KB", bytes as f64 / 1024.0)
    } else {
        format!("{bytes} bytes, {:.1} MB", bytes as f64 / (1024.0 * 1024.0))
    }
}

fn status_block(d: &ReportDialog) -> Element<'_, Message> {
    let mut col = column![].spacing(6).width(Fill);
    if !d.linked && !d.sent() {
        col = col.push(
            text(
                "The tracker is not linked, so Send is off. Link it in Settings, or use Save \
                 report and give the file to an officer.",
            )
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(theme::WARN),
        );
    }
    match &d.send {
        SendState::Idle => {}
        SendState::Sending => {
            col = col.push(
                text("Sending...")
                    .size(SIZE_META)
                    .font(FONT_MEDIUM)
                    .color(TEXT_2),
            );
        }
        SendState::Done(SendOutcome::Sent {
            id,
            expires_at,
            reports_url,
        }) => {
            let expiry = match expires_at {
                Some(t) => format!(
                    "It is deleted on {} (your local time, UTC{}).",
                    t.with_timezone(&Local).format("%Y-%m-%d %H:%M"),
                    t.with_timezone(&Local).format("%:z")
                ),
                None => "It is kept for training until you withdraw it.".into(),
            };
            col = col.push(
                text(format!("Report sent. Id {id}. {expiry}"))
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(theme::OK),
            );
            if !reports_url.is_empty() {
                col = col.push(
                    row![
                        text("Delete it or withdraw training on My reports.")
                            .size(SIZE_META)
                            .font(FONT_MEDIUM)
                            .color(TEXT_2),
                        btn("Open My reports", TEXT)
                            .style(theme::ghost_btn())
                            .on_press(msg(ReportMessage::OpenMyReports(reports_url.clone()))),
                    ]
                    .spacing(8)
                    .align_y(Alignment::Center),
                );
            }
        }
        SendState::Done(SendOutcome::Failed(reason)) => {
            col = col.push(
                text(reason.as_str())
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(theme::DANGER),
            );
        }
    }
    if d.confirm_save {
        col = col.push(
            text(PRIVATE_WARNING)
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(theme::WARN),
        );
    }
    match &d.saved {
        Some(Ok(path)) => {
            col = col.push(
                text(format!("Saved to {}", path.display()))
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(theme::OK),
            );
        }
        Some(Err(e)) => {
            col = col.push(
                text(e.as_str())
                    .size(SIZE_META)
                    .font(FONT_SEMIBOLD)
                    .color(theme::DANGER),
            );
        }
        None => {}
    }
    col.into()
}

fn actions(d: &ReportDialog) -> Element<'_, Message> {
    let mut close = btn(if d.sent() { "Done" } else { "Cancel" }, TEXT).style(theme::ghost_btn());
    if !d.busy() {
        close = close.on_press(msg(ReportMessage::Close));
    }
    let mut out = row![close].spacing(8).align_y(Alignment::Center);
    if d.confirm_save {
        out = out.push(
            btn("Don't save", TEXT)
                .style(theme::ghost_btn())
                .on_press(msg(ReportMessage::CancelSave)),
        );
        let label = if d.saving { "Saving..." } else { "Save anyway" };
        return out
            .push(action_btn(label, true, d.can_save(), ReportMessage::Save))
            .into();
    }
    if !d.sent() {
        let save = action_btn("Save report", false, d.can_save(), ReportMessage::Save);
        let failed = matches!(d.send, SendState::Done(SendOutcome::Failed(_)));
        let label = match d.send {
            SendState::Sending => "Sending...",
            _ if failed => "Send again",
            _ => "Send",
        };
        let send = action_btn(label, true, d.can_send(), ReportMessage::Send);
        out = out.push(save).push(send);
    }
    out.into()
}

/// A disabled button is drawn flat with dim text, so it does not look
/// pressable. Only an enabled primary button gets the accent fill.
fn action_btn(
    label: &str,
    primary: bool,
    enabled: bool,
    on_press: ReportMessage,
) -> iced::widget::Button<'_, Message> {
    if !enabled {
        return btn(label, TEXT_3).style(theme::chip(false));
    }
    let style = if primary {
        theme::chip(true)
    } else {
        theme::chip(false)
    };
    btn(label, TEXT).style(style).on_press(msg(on_press))
}

fn shell<'a>(body: Element<'a, Message>, max_height: f32) -> Element<'a, Message> {
    container(body)
        .padding(24)
        .width(Length::Fixed(640.0))
        .max_height(max_height)
        .style(theme::surface_panel)
        .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::TimeZone;

    #[test]
    fn send_outcomes_follow_section_7() {
        let ok = outcome_for(
            201,
            None,
            r#"{"id":"abc","training":false,"expires_at":"2026-11-11T20:00:00Z"}"#,
            Some("https://site/reports".into()),
        );
        assert_eq!(
            ok,
            SendOutcome::Sent {
                id: "abc".into(),
                expires_at: Some(Utc.with_ymd_and_hms(2026, 11, 11, 20, 0, 0).unwrap()),
                reports_url: "https://site/reports".into(),
            }
        );
        let training = outcome_for(
            201,
            None,
            r#"{"id":"x","training":true,"expires_at":null}"#,
            None,
        );
        assert!(matches!(
            training,
            SendOutcome::Sent {
                expires_at: None,
                ..
            }
        ));

        let SendOutcome::Failed(m) = outcome_for(401, None, "", None) else {
            panic!()
        };
        assert!(m.contains("no longer valid") && m.contains("Link"));
        let SendOutcome::Failed(m) = outcome_for(429, Some(3600), "", None) else {
            panic!()
        };
        assert!(
            m.contains("daily report limit") && m.contains("60 minutes"),
            "{m}"
        );
        let SendOutcome::Failed(m) = outcome_for(429, None, "", None) else {
            panic!()
        };
        assert!(!m.contains("Try again in"), "{m}");
        let SendOutcome::Failed(m) =
            outcome_for(503, None, r#"{"error":"reports_disabled"}"#, None)
        else {
            panic!()
        };
        assert!(m.starts_with("Reports are off right now"), "{m}");
        let SendOutcome::Failed(m) = outcome_for(500, None, "", None) else {
            panic!()
        };
        assert!(m.contains("was not sent"), "{m}");
        // A 400 is the site not taking the contents: no "Try again".
        let SendOutcome::Failed(m) = outcome_for(400, None, r#"{"error":"bad"}"#, None) else {
            panic!()
        };
        assert_eq!(m, "The site did not accept this report. Use Save report.");
        assert!(!m.contains("Try again"), "{m}");
        assert!(
            !m.contains("version"),
            "not every 400 is the site's age: {m}"
        );
        // A 201 without a readable body is not shown as sent.
        assert!(matches!(
            outcome_for(201, None, "nope", None),
            SendOutcome::Failed(_)
        ));
    }

    #[test]
    fn create_url_uses_the_linked_site_only() {
        assert_eq!(
            create_url("https://scuffed.example/").unwrap(),
            "https://scuffed.example/api/stat-reports"
        );
        assert!(create_url("not a url").is_err());
    }

    #[test]
    fn download_dir_reads_user_dirs_and_falls_back() {
        let home = Path::new("/home/qx");
        assert_eq!(
            parse_download_dir("# c\nXDG_DOWNLOAD_DIR=\"$HOME/Hent\"\n", home),
            Some(PathBuf::from("/home/qx/Hent"))
        );
        assert_eq!(
            parse_download_dir("XDG_DOWNLOAD_DIR=\"/data/dl\"", home),
            Some(PathBuf::from("/data/dl"))
        );
        // xdg-user-dirs writes "$HOME/" when the folder is disabled.
        assert_eq!(
            parse_download_dir("XDG_DOWNLOAD_DIR=\"$HOME/\"", home),
            None
        );
        assert_eq!(parse_download_dir("XDG_DOWNLOAD_DIR=\"rel\"", home), None);
        assert_eq!(parse_download_dir("XDG_MUSIC_DIR=\"$HOME/M\"", home), None);

        let tmp = tempfile::tempdir().unwrap();
        let home = tmp.path().join("home");
        fs::create_dir_all(home.join(".config")).unwrap();
        assert_eq!(download_dir(&home, None), home.join("Downloads"));
        fs::write(
            home.join(".config/user-dirs.dirs"),
            "XDG_DOWNLOAD_DIR=\"$HOME/Nedlastinger\"\n",
        )
        .unwrap();
        assert_eq!(download_dir(&home, None), home.join("Nedlastinger"));
        let xdg = tmp.path().join("xdg");
        fs::create_dir_all(&xdg).unwrap();
        fs::write(xdg.join("user-dirs.dirs"), "XDG_DOWNLOAD_DIR=\"/srv/dl\"\n").unwrap();
        assert_eq!(download_dir(&home, Some(&xdg)), PathBuf::from("/srv/dl"));
    }

    #[test]
    fn save_never_overwrites_and_is_private() {
        let now = Utc.with_ymd_and_hms(2026, 10, 12, 18, 5, 9).unwrap();
        let name = report_file_name(now);
        assert_eq!(name, "scuffed-report-20261012T180509Z.zip");
        let tmp = tempfile::tempdir().unwrap();
        let dir = tmp.path().join("Downloads");
        let a = save_zip(&dir, &name, b"one").unwrap();
        let b = save_zip(&dir, &name, b"two").unwrap();
        let c = save_zip(&dir, &name, b"three").unwrap();
        assert_eq!(a, dir.join("scuffed-report-20261012T180509Z.zip"));
        assert_eq!(b, dir.join("scuffed-report-20261012T180509Z-2.zip"));
        assert_eq!(c, dir.join("scuffed-report-20261012T180509Z-3.zip"));
        assert_eq!(fs::read(&a).unwrap(), b"one");
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt as _;
            assert_eq!(
                fs::metadata(&a).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn send_and_save_need_a_current_preview() {
        let zip = Arc::new(ReportZip {
            bytes: vec![1, 2, 3],
            manifest_json: "{}".into(),
            files: Vec::new(),
            log_text: String::new(),
            images: Vec::new(),
        });
        let mut d = ReportDialog::new(LogOnlyCategory::Other, true);
        assert!(!d.can_send() && !d.can_save());
        d.built = Some(Ok(zip.clone()));
        assert!(d.can_send() && d.can_save());
        d.text_edited_at = Some(Instant::now());
        assert!(!d.can_send() && !d.can_save() && d.zip().is_none());
        d.text_edited_at = None;
        d.building = Some(1);
        assert!(!d.can_send() && !d.can_save());
        d.building = None;
        d.linked = false;
        assert!(!d.can_send() && d.can_save());
        d.linked = true;
        d.send = SendState::Done(SendOutcome::Sent {
            id: "a".into(),
            expires_at: None,
            reports_url: String::new(),
        });
        assert!(!d.can_send() && !d.can_save());
        // A failed send can be retried by hand.
        d.send = SendState::Done(SendOutcome::Failed("x".into()));
        assert!(d.can_send());
        // A new input generation forgets the failure.
        let before = d.generation;
        d.bump();
        assert!(d.generation > before);
        // Generations are process-wide: a reopened dialog's first build never
        // shares a number with a build still in flight from the closed one.
        let closed = ReportDialog::new(LogOnlyCategory::Crash, true).bump();
        let reopened = ReportDialog::new(LogOnlyCategory::Crash, true).bump();
        assert_ne!(closed, reopened);
        assert_eq!(d.send, SendState::Idle);
    }

    #[test]
    fn crash_prompt_follows_the_marker() {
        let tmp = tempfile::tempdir().unwrap();
        assert!(crash_prompt(tmp.path()).is_none());
        let marker = stat_tracker::report::RunMarker {
            pid: 4242,
            started_at: Utc.with_ymd_and_hms(2026, 10, 12, 17, 0, 0).unwrap(),
            last_seen: Utc.with_ymd_and_hms(2026, 10, 12, 18, 0, 0).unwrap(),
        };
        fs::write(
            stat_tracker::report::marker::last_crash_marker_path(tmp.path()),
            marker.format(),
        )
        .unwrap();
        let Some(ReportUi::CrashPrompt { last_seen }) = crash_prompt(tmp.path()) else {
            panic!("expected the crash prompt");
        };
        assert_eq!(last_seen, Some(marker.last_seen));
        stat_tracker::report::marker::dismiss_last_crash(tmp.path()).unwrap();
        assert!(crash_prompt(tmp.path()).is_none());
    }

    /// 1x1 PNG. `kit::load` accepts it only with a matching `reads.json`.
    fn one_pixel_png() -> Vec<u8> {
        let image = image::RgbImage::from_pixel(1, 1, image::Rgb([8, 8, 8]));
        let mut png = Vec::new();
        image::DynamicImage::ImageRgb8(image)
            .write_to(&mut std::io::Cursor::new(&mut png), image::ImageFormat::Png)
            .unwrap();
        png
    }

    fn write_checked_kit(dir: &Path, png: &[u8]) {
        use stat_tracker::report::blank::BLANK_TABLE_VERSION;
        use stat_tracker::report::bundle::sha256_hex;
        use stat_tracker::report::kit::{KitRead, KitReads, READS_JSON, SCOREBOARD_PNG};
        fs::create_dir_all(dir).unwrap();
        fs::write(dir.join(SCOREBOARD_PNG), png).unwrap();
        let mut reads = std::collections::BTreeMap::new();
        reads.insert(
            "hero".into(),
            KitRead {
                value: Some("ana".into()),
                confidence: None,
                ocr_v1: None,
                suspect: false,
            },
        );
        let reads = KitReads {
            captured_at: Utc.with_ymd_and_hms(2026, 10, 13, 1, 0, 0).unwrap(),
            width: 1,
            height: 1,
            team_size: Some(5),
            map: None,
            mode: None,
            result: None,
            recognizer: "cv-v5".into(),
            blank_table: BLANK_TABLE_VERSION,
            scoreboard_sha256: sha256_hex(png),
            reads,
        };
        fs::write(dir.join(READS_JSON), serde_json::to_vec(&reads).unwrap()).unwrap();
    }

    #[test]
    fn a_log_zip_is_never_sent_as_a_game_report() {
        assert_eq!(GameCategory::WrongStats.as_str(), "wrong_stats");
        assert!(GameCategory::MissedGame.allows_missing_kit());
        assert!(!GameCategory::WrongStats.allows_missing_kit());

        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            data_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let built_at = Utc.with_ymd_and_hms(2026, 10, 13, 1, 0, 0).unwrap();
        let log_input = LogReportInput::from_config(
            &config,
            LogOnlyCategory::Other,
            "the log".into(),
            FALLBACK_RESOLUTION,
            CaptureBackend::None,
            false,
            Vec::new(),
            built_at,
        );
        let log_zip = stat_tracker::report::build_log_report(&log_input).unwrap();
        assert!(!manifest_has_game(&log_zip.manifest_json));

        let mut dialog = ReportDialog::new(LogOnlyCategory::Other, true);
        dialog.built = Some(Ok(Arc::new(log_zip)));
        assert!(dialog.can_send() && dialog.can_save());
        dialog.game_category = Some(GameCategory::WrongStats);
        assert!(!dialog.can_send() && !dialog.can_save());

        // A bare scoreboard file is not a kit. `kit::load` wants the hash.
        let bare = stat_tracker::report::kit::session_dir(tmp.path(), "bare").unwrap();
        fs::create_dir_all(&bare).unwrap();
        fs::write(bare.join("scoreboard.png"), one_pixel_png()).unwrap();
        assert!(!has_kit(tmp.path(), "bare"));
        assert!(!has_kit(tmp.path(), "../bare"));
        assert!(kit_sessions(tmp.path()).is_empty());

        let png = one_pixel_png();
        let session = stat_tracker::report::kit::session_dir(tmp.path(), "s1").unwrap();
        write_checked_kit(&session, &png);
        assert!(has_kit(tmp.path(), "s1"));
        assert_eq!(kit_sessions(tmp.path()), vec!["s1".to_string()]);

        let game_input = GameReportInput::from_config(
            &config,
            "s1",
            GameCategory::WrongStats,
            "the stats".into(),
            CaptureBackend::None,
            false,
            Vec::new(),
            built_at,
        )
        .unwrap();
        assert_eq!(
            game_input.kit.scoreboard_png.as_deref(),
            Some(png.as_slice())
        );
        let game_zip = stat_tracker::report::build_game_report(&game_input).unwrap();
        assert!(manifest_has_game(&game_zip.manifest_json));
        let preview = kit_images_from_zip(&game_zip);
        assert_eq!(preview.len(), 1);
        assert_eq!(preview[0].path, "crops/scoreboard.png");
        assert_eq!((preview[0].width, preview[0].height), (1, 1));
        let file = game_zip
            .files
            .iter()
            .find(|entry| entry.path == "crops/scoreboard.png")
            .unwrap();
        let image = game_zip
            .images
            .iter()
            .find(|entry| entry.path == "crops/scoreboard.png")
            .unwrap();
        assert_eq!(
            file.sha256,
            stat_tracker::report::bundle::sha256_hex(&image.png)
        );
        assert_eq!(file.sha256, stat_tracker::report::bundle::sha256_hex(&png));

        dialog.built = Some(Ok(Arc::new(game_zip)));
        assert!(dialog.can_send() && dialog.can_save());
        dialog.game_category = None;
        assert!(!dialog.can_send() && !dialog.can_save());
    }
}
