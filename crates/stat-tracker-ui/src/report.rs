//! Report a problem (`crates/stat-tracker/docs/report-bundle.md` section 7).
//!
//! Log-only reports: the member picks `crash`, `performance`, or `other`,
//! types a short note, and reads a preview built from the exact zip that
//! would leave the PC. **Send** posts that zip to the linked site with the
//! tracker sync token (create only). **Save report** writes the same zip to
//! the Downloads folder after a private-file warning. The daemon library
//! builds the zip (`stat_tracker::report`); this module is the dialog, the
//! upload, the save path, and the crash prompt.

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
use stat_tracker::report::{LogOnlyCategory, LogReportInput, ReportZip, Resolution};

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

/// Messages the dialog sends. Wrapped in [`Message::Report`].
#[derive(Debug, Clone)]
pub enum ReportMessage {
    /// Open the dialog, optionally with a category picked.
    Open(Option<LogOnlyCategory>),
    Close,
    Category(LogOnlyCategory),
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

    pub fn can_send(&self) -> bool {
        self.linked
            && self.preview_current()
            && !matches!(self.send, SendState::Sending)
            && !self.sent()
    }

    pub fn sent(&self) -> bool {
        matches!(self.send, SendState::Done(SendOutcome::Sent { .. }))
    }

    pub fn can_save(&self) -> bool {
        self.preview_current() && !self.saving && !self.sent()
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

/// Process-wide build counter. Starts at 1; 0 means no build yet.
pub fn next_generation() -> u64 {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
    NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
}

/// Open-dialog state: the crash prompt, or the dialog itself.
#[derive(Debug, Clone)]
pub enum ReportUi {
    CrashPrompt { last_seen: Option<DateTime<Utc>> },
    Dialog(ReportDialog),
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
    Resolution { width, height }
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
            "The tracker stopped or closed by itself. The log covers the 30 minutes before \
             it stopped and the last 30 minutes."
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
    body = body.push(
        text(category_hint(d.category))
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

    shell(scrollable(body).height(Fill).width(Fill).into(), 620.0)
}

fn preview_block(d: &ReportDialog) -> Element<'_, Message> {
    let mut col = column![
        text("What will be sent")
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
            col = col.push(
                text(format!(
                    "One zip, {} in total. No screenshots. The log is scrubbed: your name, \
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
        text(format!("{path}  ({})", size_text(bytes)))
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
        format!("{:.1} KB ({bytes} bytes)", bytes as f64 / 1024.0)
    } else {
        format!("{:.1} MB ({bytes} bytes)", bytes as f64 / (1024.0 * 1024.0))
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
                    "It is deleted on {}.",
                    t.with_timezone(&Local).format("%Y-%m-%d %H:%M")
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
        let mut save =
            btn(if d.saving { "Saving..." } else { "Save anyway" }, TEXT).style(theme::chip(true));
        if d.can_save() {
            save = save.on_press(msg(ReportMessage::Save));
        }
        return out.push(save).into();
    }
    if !d.sent() {
        let mut save = btn("Save report", TEXT).style(theme::ghost_btn());
        if d.can_save() {
            save = save.on_press(msg(ReportMessage::Save));
        }
        let failed = matches!(d.send, SendState::Done(SendOutcome::Failed(_)));
        let label = match d.send {
            SendState::Sending => "Sending...",
            _ if failed => "Send again",
            _ => "Send",
        };
        let mut send = btn(label, TEXT).style(theme::chip(true));
        if d.can_send() {
            send = send.on_press(msg(ReportMessage::Send));
        }
        out = out.push(save).push(send);
    }
    out.into()
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
}
