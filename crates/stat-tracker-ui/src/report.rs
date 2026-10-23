//! Report a problem (`crates/stat-tracker/docs/report-bundle.md` section 7).
//!
//! Log-only reports: the member picks `crash`, `performance`, or `other`,
//! types a short note, and reads a preview built from the exact zip that
//! would leave the PC. The dialog opens with nothing picked. Exactly one
//! kind is picked at a time: a log chip clears the game row (and the game
//! and its pictures), and a game chip clears the log row. `crash` and
//! `performance` never carry a game or pictures. The Send button names the
//! picked kind and says whether pictures go with it. A game report (Games tab and the game card) is the
//! same dialog for one session: no category is picked when it opens, the
//! member picks one before Send or Save, and the list leaves out `crash`
//! and `performance`. **Send** posts that zip to the
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

/// Placeholder of the note box on a Crash or Performance report. The note
/// is optional and goes into the manifest's `reason.text`.
pub const LOG_NOTE_HINT: &str = "What did you notice? Stutter, FPS drops, high CPU, which hero?";

/// Placeholder of the note box on every other kind, and before a pick.
pub const NOTE_HINT: &str = "What happened? (optional)";

/// The note as it is kept and sent: NUL characters dropped, then cut to
/// [`REASON_TEXT_MAX_CHARS`] characters. The library scrubs it again when
/// the zip is built (`scrub_reason_text`).
pub fn clamp_note(value: &str) -> String {
    value
        .chars()
        .filter(|c| *c != '\0')
        .take(REASON_TEXT_MAX_CHARS)
        .collect()
}

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

/// Preview handles for `images` loaded straight from a game's kit.
pub fn kit_images_from_report_images(
    images: Vec<stat_tracker::report::bundle::ReportImage>,
) -> Vec<KitImage> {
    images
        .into_iter()
        .map(|image| KitImage {
            path: image.path,
            width: image.width,
            height: image.height,
            handle: iced::widget::image::Handle::from_bytes(image.png),
        })
        .collect()
}

/// Load a game's saved pictures without building a report, so a game
/// opened from its card shows them before a category is picked.
pub async fn load_kit_preview(
    data_dir: PathBuf,
    session_id: String,
) -> Vec<stat_tracker::report::bundle::ReportImage> {
    tokio::task::spawn_blocking(move || {
        let kit = stat_tracker::report::kit::load(&data_dir, &session_id);
        stat_tracker::report::bundle::kit_preview_images(&kit)
    })
    .await
    .unwrap_or_default()
}

/// Messages the dialog sends. Wrapped in [`Message::Report`].
#[derive(Debug, Clone)]
pub enum ReportMessage {
    /// Open the dialog, optionally with a category picked.
    Open(Option<LogOnlyCategory>),
    /// Open on a game. Training is visible and already ticked when `training` is set.
    OpenGame {
        session_id: Option<String>,
        /// `None` opens with no category picked; Send and Save wait for one.
        category: Option<GameCategory>,
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
    /// A game's pictures loaded before any build, for that generation.
    KitLoaded(u64, Vec<stat_tracker::report::bundle::ReportImage>),
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

/// The one kind of report the member picked: a chip in the top row
/// (log only, never a game or pictures) or a chip under About a game.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PickedKind {
    Log(LogOnlyCategory),
    Game(GameCategory),
}

impl PickedKind {
    /// Every kind a member can pick, top row first.
    pub const ALL: [Self; 10] = [
        Self::Log(LogOnlyCategory::Crash),
        Self::Log(LogOnlyCategory::Performance),
        Self::Log(LogOnlyCategory::Other),
        Self::Game(GameCategory::WrongStats),
        Self::Game(GameCategory::WrongHero),
        Self::Game(GameCategory::WrongMap),
        Self::Game(GameCategory::WrongMode),
        Self::Game(GameCategory::WrongResult),
        Self::Game(GameCategory::MissedGame),
        Self::Game(GameCategory::Other),
    ];

    /// `reason.category` the uploaded manifest carries for this kind.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Log(c) => c.as_str(),
            Self::Game(c) => c.as_str(),
        }
    }

    pub const fn is_game(self) -> bool {
        matches!(self, Self::Game(_))
    }
}

/// What [`ReportDialog::build_plan`] says to do for the current inputs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BuildPlan {
    /// Nothing is picked and there is no game to show: no zip.
    Nothing,
    /// A game opened from its card with no kind picked yet: show its
    /// pictures, build nothing.
    LoadKit(String),
    /// A log-only zip. Never a game, never pictures.
    Log(LogOnlyCategory),
    /// A game zip for `session_id`.
    Game {
        session_id: String,
        category: GameCategory,
    },
}

#[derive(Debug, Clone)]
pub struct ReportDialog {
    /// The kind picked in the top row (log only). `None` when nothing is
    /// picked there. Set only together with `game_report == false` and no
    /// game category, so two rows are never picked at once.
    pub category: Option<LogOnlyCategory>,
    /// The report is about a game. Log chips are then unselected.
    pub game_report: bool,
    /// The game category the member picked. `None` on a game report until
    /// one is picked, and always on a log report.
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
    /// Generation of a picture load in flight (no category picked yet).
    pub kit_loading: Option<u64>,
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
    /// A log dialog with `category` picked, or nothing picked.
    pub fn new(category: Option<LogOnlyCategory>, linked: bool) -> Self {
        Self {
            category,
            game_report: false,
            game_category: None,
            game_session: None,
            game_choices: Vec::new(),
            kit_images: Vec::new(),
            image_actual: false,
            text: String::new(),
            training: false,
            built: None,
            building: None,
            kit_loading: None,
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

    /// A dialog about one game, opened with `category` picked or none.
    /// The top row starts with nothing picked.
    pub fn for_game(category: Option<GameCategory>, linked: bool) -> Self {
        let mut dialog = Self::new(None, linked);
        dialog.game_report = true;
        dialog.game_category = category;
        dialog
    }

    /// The report is about a game. A picked log kind always wins, so a
    /// stale game flag can never turn a log report into a game one.
    pub fn is_game(&self) -> bool {
        self.game_report && self.category.is_none()
    }

    /// Placeholder of the note box: [`LOG_NOTE_HINT`] while Crash or
    /// Performance is picked, [`NOTE_HINT`] otherwise.
    pub fn note_hint(&self) -> &'static str {
        match self.picked() {
            Some(PickedKind::Log(LogOnlyCategory::Crash | LogOnlyCategory::Performance)) => {
                LOG_NOTE_HINT
            }
            _ => NOTE_HINT,
        }
    }

    /// The one kind that would be sent, if any.
    pub fn picked(&self) -> Option<PickedKind> {
        if let Some(category) = self.category {
            return Some(PickedKind::Log(category));
        }
        if self.game_report {
            return self.game_category.map(PickedKind::Game);
        }
        None
    }

    /// Whether `kind`'s chip is drawn selected. At most one chip is.
    pub fn is_picked(&self, kind: PickedKind) -> bool {
        self.picked() == Some(kind)
    }

    /// Edits are off while sending or saving and after a send.
    fn locked(&self) -> bool {
        self.busy() || self.sent()
    }

    /// Pick a top-row kind. Clears the game row, the game, and its
    /// pictures: a log report never carries them. Returns true when the
    /// inputs changed and the preview must be rebuilt.
    pub fn pick_log(&mut self, category: LogOnlyCategory) -> bool {
        if self.locked() || self.picked() == Some(PickedKind::Log(category)) {
            return false;
        }
        self.category = Some(category);
        self.game_report = false;
        self.game_category = None;
        self.game_session = None;
        self.kit_images.clear();
        self.kit_loading = None;
        self.built = None;
        true
    }

    /// Pick a kind under About a game. Clears the top row. `choices` fills
    /// the game list when the dialog has none yet. Returns true when the
    /// inputs changed.
    pub fn pick_game(&mut self, category: GameCategory, choices: Vec<GameChoice>) -> bool {
        if self.locked() || self.picked() == Some(PickedKind::Game(category)) {
            return false;
        }
        self.category = None;
        self.game_report = true;
        self.game_category = Some(category);
        if self.game_choices.is_empty() {
            self.game_choices = choices;
        }
        if category.allows_missing_kit() {
            self.game_session = None;
        }
        self.kit_images.clear();
        self.built = None;
        true
    }

    /// Pick the game a game report is about. Ignored on a log report, so a
    /// game can never ride along with Crash or Performance.
    pub fn pick_session(&mut self, session_id: String) -> bool {
        if self.locked() || !self.is_game() {
            return false;
        }
        if self.game_session.as_deref() == Some(session_id.as_str()) {
            return false;
        }
        self.game_session = Some(session_id);
        self.kit_images.clear();
        self.built = None;
        true
    }

    /// What to build for the current inputs. The zip kind always follows
    /// [`Self::picked`].
    pub fn build_plan(&self) -> BuildPlan {
        match self.picked() {
            Some(PickedKind::Log(category)) => BuildPlan::Log(category),
            Some(PickedKind::Game(category)) => BuildPlan::Game {
                session_id: self.game_session.clone().unwrap_or_default(),
                category,
            },
            None => match (&self.game_session, self.is_game()) {
                (Some(session_id), true) => BuildPlan::LoadKit(session_id.clone()),
                _ => BuildPlan::Nothing,
            },
        }
    }

    /// Pictures go with the report. From the built zip when there is a
    /// current one, else what the picked kind would carry.
    pub fn sends_pictures(&self) -> bool {
        match self.picked() {
            None | Some(PickedKind::Log(_)) => false,
            Some(PickedKind::Game(category)) => match &self.built {
                Some(Ok(zip)) if self.preview_current() => !zip.images.is_empty(),
                _ => !category.allows_missing_kit(),
            },
        }
    }

    /// The Send button's text: the picked kind and whether pictures go.
    pub fn send_label(&self) -> String {
        if matches!(self.send, SendState::Sending) {
            return "Sending...".into();
        }
        let again = matches!(self.send, SendState::Done(SendOutcome::Failed(_)));
        let Some(kind) = self.picked() else {
            return if again { "Send again" } else { "Send" }.into();
        };
        let what = match kind {
            PickedKind::Log(category) => format!("{} report", category_label(category)),
            PickedKind::Game(category) => format!("game report: {}", category.label()),
        };
        let pictures = if self.sends_pictures() {
            "with pictures"
        } else {
            "no pictures"
        };
        if again {
            format!("Send again: {what} ({pictures})")
        } else {
            format!("Send {what} ({pictures})")
        }
    }

    /// A game report needs a picked category before it is built or sent.
    pub fn needs_game_category(&self) -> bool {
        self.game_report && self.game_category.is_none()
    }

    /// The line shown where the pictures go when none are loaded. Only once
    /// the game's pictures were looked for (by a build, or by the picture
    /// load for a game opened from its card) and none came back.
    pub fn pictures_note(&self) -> Option<&'static str> {
        if !self.kit_images.is_empty()
            || !self.is_game()
            || self.building.is_some()
            || self.kit_loading.is_some()
            || self.text_edited_at.is_some()
            || self.game_category.is_some_and(|c| c.allows_missing_kit())
        {
            return None;
        }
        if self.needs_game_category() && self.game_session.is_none() {
            // Nothing picked and no game: there is nothing to look for yet.
            return None;
        }
        Some("No pictures for this game yet.")
    }

    /// Send posts the preview zip. A log zip is never sent from a game
    /// dialog, and a game zip is never sent from a log dialog.
    pub fn can_send(&self) -> bool {
        self.linked
            && self.picked().is_some()
            && self.preview_current()
            && self.preview_matches_kind()
            && !matches!(self.send, SendState::Sending)
            && !self.sent()
    }

    pub fn sent(&self) -> bool {
        matches!(self.send, SendState::Done(SendOutcome::Sent { .. }))
    }

    pub fn can_save(&self) -> bool {
        self.picked().is_some()
            && self.preview_current()
            && self.preview_matches_kind()
            && !self.saving
            && !self.sent()
    }

    /// The preview zip is the picked kind: its `reason.category` is the
    /// picked one, it has a game exactly when a game kind is picked, and a
    /// log kind carries no pictures. A zip left from an earlier pick is
    /// never sent.
    fn preview_matches_kind(&self) -> bool {
        let (Some(Ok(zip)), Some(kind)) = (&self.built, self.picked()) else {
            return false;
        };
        zip_matches_kind(zip, kind)
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

/// `zip` is a report of `kind`: same `reason.category`, a game exactly
/// for a game kind, and no pictures for a log kind.
pub fn zip_matches_kind(zip: &ReportZip, kind: PickedKind) -> bool {
    if manifest_has_game(&zip.manifest_json) != kind.is_game() {
        return false;
    }
    if manifest_reason_category(&zip.manifest_json).as_deref() != Some(kind.as_str()) {
        return false;
    }
    kind.is_game()
        || (zip.images.is_empty() && !zip.files.iter().any(|f| f.path.starts_with("crops/")))
}

/// `manifest.reason.category`, if the manifest has one.
fn manifest_reason_category(manifest_json: &str) -> Option<String> {
    serde_json::from_str::<serde_json::Value>(manifest_json)
        .ok()?
        .get("reason")?
        .get("category")?
        .as_str()
        .map(str::to_string)
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
            log_scrub_names(&req.config.data_dir),
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
            game_scrub_names(&req.config.data_dir, &req.session_id),
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

/// Scrub list for a log report: the player names stored with every recent
/// game's kit, since the log window can cover any of them.
pub fn log_scrub_names(data_dir: &Path) -> Vec<String> {
    stat_tracker::report::kit::all_names(data_dir)
}

/// Scrub list for a game report: the player names read on that game's
/// stored boards.
pub fn game_scrub_names(data_dir: &Path, session_id: &str) -> Vec<String> {
    stat_tracker::report::kit::names(data_dir, session_id)
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
    // The upload carries the sync token and the zip: a redirect is followed
    // only on the site's own scheme, host and port.
    let client = match reqwest::Client::builder()
        .timeout(Duration::from_secs(60))
        .redirect(stat_tracker::sync::token_safe_redirects())
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
            // The site's own words, e.g. "Daily report limit reached". The
            // game's saved pictures are kept (only a sent report removes
            // them), so it can be sent again tomorrow.
            let said = serde_json::from_str::<serde_json::Value>(body)
                .ok()
                .and_then(|v| v.get("error").and_then(|e| e.as_str()).map(str::to_string))
                .map(|e| e.trim().chars().take(120).collect::<String>())
                .filter(|e| !e.is_empty())
                .unwrap_or_else(|| "You have reached the daily report limit".into());
            let said = said.trim_end_matches('.');
            SendOutcome::Failed(format!(
                "{said}.{wait} This game stays saved, so you can send it tomorrow. Save report still works."
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
            "Low FPS, stutter, or the tracker using a lot of CPU. Sends only the performance \
             log (CPU, memory, capture and OCR timing, and your setup). No pictures."
        }
        LogOnlyCategory::Other => "Anything else. The log covers the last 30 minutes.",
    }
}

fn btn<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    color: iced::Color,
) -> iced::widget::Button<'a, Message> {
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
        let selected = d.is_picked(PickedKind::Log(category));
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
        text(match d.picked() {
            Some(PickedKind::Log(category)) => category_hint(category),
            Some(PickedKind::Game(category)) => game_hint(category),
            None if d.is_game() => "Pick what was wrong with this game before you send the report.",
            None => "Pick what kind of problem this is before you send the report.",
        })
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(TEXT_3),
    );

    let mut input = text_input(d.note_hint(), &d.text)
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
            let selected = d.is_picked(PickedKind::Game(category));
            let mut chip = btn(category.label(), if selected { TEXT } else { TEXT_2 })
                .style(theme::chip(selected));
            if !d.busy() && !d.sent() {
                chip = chip.on_press(msg(ReportMessage::GameCategory(category)));
            }
            chips = chips.push(chip);
        }
        col = col.push(chips);
    }
    if d.is_game() && !d.game_category.is_some_and(|c| c.allows_missing_kit()) {
        col = col.push(game_picker(d));
    }
    if d.is_game() && !d.kit_images.is_empty() {
        col = col.push(kit_pictures(d));
    } else if let Some(note) = d.pictures_note() {
        col = col.push(text(note).size(SIZE_META).font(FONT_MEDIUM).color(TEXT_3));
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
        text(if d.sent() {
            "Sent"
        } else {
            "What will be sent"
        })
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
            let pictures = if zip.images.is_empty() {
                "No pictures."
            } else {
                "The pictures above are the ones in the zip."
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
        let send = action_btn(d.send_label(), true, d.can_send(), ReportMessage::Send);
        out = out.push(save).push(send);
    }
    out.into()
}

/// A disabled button is drawn flat with dim text, so it does not look
/// pressable. Only an enabled primary button gets the accent fill.
fn action_btn<'a>(
    label: impl iced::widget::text::IntoFragment<'a>,
    primary: bool,
    enabled: bool,
    on_press: ReportMessage,
) -> iced::widget::Button<'a, Message> {
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
        let SendOutcome::Failed(m) =
            outcome_for(429, None, r#"{"error":"Daily report limit reached"}"#, None)
        else {
            panic!()
        };
        assert!(
            m.starts_with("Daily report limit reached.") && m.contains("send it tomorrow"),
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
            manifest_json: r#"{"game":null,"reason":{"category":"other"}}"#.into(),
            files: Vec::new(),
            log_text: String::new(),
            images: Vec::new(),
        });
        let mut d = ReportDialog::new(Some(LogOnlyCategory::Other), true);
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
        let closed = ReportDialog::new(Some(LogOnlyCategory::Crash), true).bump();
        let reopened = ReportDialog::new(Some(LogOnlyCategory::Crash), true).bump();
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

        let log_zip = Arc::new(log_zip);
        let mut dialog = ReportDialog::new(Some(LogOnlyCategory::Other), true);
        dialog.built = Some(Ok(log_zip.clone()));
        assert!(dialog.can_send() && dialog.can_save());
        assert!(dialog.pick_game(GameCategory::WrongStats, Vec::new()));
        dialog.built = Some(Ok(log_zip));
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
        assert!(
            !dialog.can_send() && !dialog.can_save(),
            "a game report with no category picked cannot be sent or saved"
        );
    }

    /// alpha.10: the stored board's names reach the scrubber of both report
    /// kinds.
    #[test]
    fn report_builds_pass_the_stored_board_names_to_the_scrubber() {
        let tmp = tempfile::tempdir().unwrap();
        let png = one_pixel_png();
        for (sid, names) in [
            ("g1", vec!["QXOther".to_string()]),
            ("g2", vec!["Zed".to_string()]),
        ] {
            let dir = stat_tracker::report::kit::session_dir(tmp.path(), sid).unwrap();
            fs::create_dir_all(&dir).unwrap();
            write_checked_kit(&dir, &png);
            stat_tracker::report::kit::record_names(tmp.path(), sid, &names).unwrap();
        }
        assert_eq!(
            game_scrub_names(tmp.path(), "g1"),
            vec!["QXOther".to_string()]
        );
        let mut all = log_scrub_names(tmp.path());
        all.sort();
        assert_eq!(all, vec!["QXOther".to_string(), "Zed".to_string()]);
    }

    /// alpha.10: a game report opens with no category picked, and Send and
    /// Save stay off until the member picks one.
    #[test]
    fn a_game_report_opens_with_no_category_and_waits_for_one() {
        let d = ReportDialog::for_game(None, true);
        assert!(d.is_game());
        assert_eq!(d.game_category, None);
        assert!(d.needs_game_category());
        assert!(!d.can_send() && !d.can_save());
        let picked = ReportDialog::for_game(Some(GameCategory::WrongHero), true);
        assert!(!picked.needs_game_category());
        assert!(!ReportDialog::new(Some(LogOnlyCategory::Other), true).needs_game_category());
    }

    /// A game opened from its card has no category, so its pictures are not
    /// loaded yet. The dialog must not say the game has no pictures.
    #[test]
    fn a_game_opened_from_its_card_does_not_claim_no_pictures() {
        let mut d = ReportDialog::for_game(None, true);
        d.game_session = Some("827e7d10be7f0232".into());
        d.kit_loading = Some(1);
        assert_eq!(d.pictures_note(), None, "still loading");
        d.kit_loading = None;
        assert_eq!(
            d.pictures_note(),
            Some("No pictures for this game yet."),
            "looked and found none"
        );
        let built = ReportDialog::for_game(Some(GameCategory::WrongHero), true);
        assert_eq!(
            built.pictures_note(),
            Some("No pictures for this game yet.")
        );
        assert_eq!(
            ReportDialog::new(Some(LogOnlyCategory::Other), true).pictures_note(),
            None
        );
    }
    /// Data dir with one checked kit (`s1`) for the pick tests.
    fn pick_fixture() -> (tempfile::TempDir, Config) {
        let tmp = tempfile::tempdir().unwrap();
        let session = stat_tracker::report::kit::session_dir(tmp.path(), "s1").unwrap();
        write_checked_kit(&session, &one_pixel_png());
        let config = Config {
            data_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        (tmp, config)
    }

    /// Build the zip `plan` asks for with the real report builders, the
    /// same ones `rebuild_report` runs. `None` when the plan builds nothing.
    fn build_for_plan(config: &Config, plan: &BuildPlan) -> Option<Arc<ReportZip>> {
        let built_at = Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap();
        match plan {
            BuildPlan::Nothing | BuildPlan::LoadKit(_) => None,
            BuildPlan::Log(category) => {
                let input = LogReportInput::from_config(
                    config,
                    *category,
                    String::new(),
                    FALLBACK_RESOLUTION,
                    CaptureBackend::None,
                    false,
                    Vec::new(),
                    built_at,
                );
                Some(Arc::new(
                    stat_tracker::report::build_log_report(&input).unwrap(),
                ))
            }
            BuildPlan::Game {
                session_id,
                category,
            } => {
                let input = GameReportInput::from_config(
                    config,
                    session_id,
                    *category,
                    String::new(),
                    CaptureBackend::None,
                    false,
                    Vec::new(),
                    built_at,
                )
                .unwrap();
                Some(Arc::new(
                    stat_tracker::report::build_game_report(&input).unwrap(),
                ))
            }
        }
    }

    fn choices() -> Vec<GameChoice> {
        vec![GameChoice {
            session_id: "s1".into(),
            label: "Dorado".into(),
            has_kit: true,
        }]
    }

    /// Click `kind` the way the dialog does: a game kind also picks the
    /// game from the list, except Missed game, which has none.
    fn click(d: &mut ReportDialog, kind: PickedKind) {
        match kind {
            PickedKind::Log(category) => {
                d.pick_log(category);
            }
            PickedKind::Game(category) => {
                d.pick_game(category, choices());
                if !category.allows_missing_kit() {
                    d.pick_session("s1".into());
                }
            }
        }
    }

    /// Never two rows at once, and a log kind never carries a game or
    /// pictures.
    fn assert_one_row(d: &ReportDialog, ctx: &str) {
        let top = PickedKind::ALL[..3]
            .iter()
            .filter(|k| d.is_picked(**k))
            .count();
        let game = PickedKind::ALL[3..]
            .iter()
            .filter(|k| d.is_picked(**k))
            .count();
        assert!(
            top + game <= 1,
            "{ctx}: {top} top and {game} game chips picked"
        );
        if d.category.is_some() {
            assert!(!d.is_game(), "{ctx}: log kind is a game report");
            assert_eq!(d.game_category, None, "{ctx}: log kind kept a game kind");
            assert_eq!(d.game_session, None, "{ctx}: log kind kept a game");
            assert!(d.kit_images.is_empty(), "{ctx}: log kind kept pictures");
        }
    }

    /// Build what the dialog would build, then check the zip Send would
    /// post is exactly the picked kind, with pictures only for a game.
    fn assert_sent_kind_is_picked(
        d: &mut ReportDialog,
        config: &Config,
        expected: PickedKind,
        ctx: &str,
    ) {
        assert_eq!(d.picked(), Some(expected), "{ctx}");
        let zip = build_for_plan(config, &d.build_plan())
            .unwrap_or_else(|| panic!("{ctx}: a picked kind builds a zip"));
        d.built = Some(Ok(zip.clone()));
        assert!(d.can_send() && d.can_save(), "{ctx}: cannot send");
        let sent = d.zip().unwrap();
        assert_eq!(
            manifest_reason_category(&sent.manifest_json).as_deref(),
            Some(expected.as_str()),
            "{ctx}: uploaded kind"
        );
        assert_eq!(
            manifest_has_game(&sent.manifest_json),
            expected.is_game(),
            "{ctx}: game in the upload"
        );
        let label = d.send_label();
        match expected {
            PickedKind::Log(category) => {
                assert!(sent.images.is_empty(), "{ctx}: log kind sent pictures");
                assert!(
                    !sent.files.iter().any(|f| f.path.starts_with("crops/")),
                    "{ctx}: log kind sent crops"
                );
                assert_eq!(
                    label,
                    format!("Send {} report (no pictures)", category_label(category)),
                    "{ctx}"
                );
            }
            PickedKind::Game(category) => {
                assert_eq!(
                    sent.images.is_empty(),
                    category.allows_missing_kit(),
                    "{ctx}: game pictures"
                );
                let pictures = if category.allows_missing_kit() {
                    "no pictures"
                } else {
                    "with pictures"
                };
                assert_eq!(
                    label,
                    format!("Send game report: {} ({pictures})", category.label()),
                    "{ctx}"
                );
            }
        }
        assert!(!label.contains('\u{2014}'), "{ctx}: no em dash in {label}");
    }

    /// The alpha.11 bug: Performance in the top row, then Other under
    /// About a game with a game picked. Both chips looked picked and only
    /// the game report (with its scoreboard) was sent. Now one row wins,
    /// and going back to Performance sends Performance with no pictures.
    #[test]
    fn performance_then_other_game_sends_only_the_last_pick() {
        let (_tmp, config) = pick_fixture();
        let mut d = ReportDialog::new(None, true);
        click(&mut d, PickedKind::Log(LogOnlyCategory::Performance));
        click(&mut d, PickedKind::Game(GameCategory::Other));
        assert_one_row(&d, "performance then other");
        assert!(!d.is_picked(PickedKind::Log(LogOnlyCategory::Performance)));
        assert!(!d.is_picked(PickedKind::Log(LogOnlyCategory::Other)));
        assert_eq!(d.game_session.as_deref(), Some("s1"));
        assert_sent_kind_is_picked(
            &mut d,
            &config,
            PickedKind::Game(GameCategory::Other),
            "performance then other",
        );
        let game_zip = d.built.clone();

        // Back to Performance: the game, its pictures, and the game zip go.
        d.kit_images = kit_images_from_zip(d.zip().as_ref().unwrap());
        assert!(!d.kit_images.is_empty());
        assert!(d.pick_log(LogOnlyCategory::Performance));
        assert_one_row(&d, "back to performance");
        assert_eq!(d.build_plan(), BuildPlan::Log(LogOnlyCategory::Performance));
        // The game zip from the earlier pick is never sent as Performance.
        d.built = game_zip;
        assert!(!d.can_send() && !d.can_save(), "stale game zip");
        assert_sent_kind_is_picked(
            &mut d,
            &config,
            PickedKind::Log(LogOnlyCategory::Performance),
            "back to performance",
        );
        // A game cannot be picked onto a Performance report.
        assert!(!d.pick_session("s1".into()));
        assert_eq!(d.game_session, None);
    }

    /// Every start (fresh, crash prompt, each log kind, a game card with
    /// and without a kind) and every pair of picks: the uploaded kind is
    /// the last pick.
    #[test]
    fn uploaded_kind_matches_the_picked_kind_for_every_combination() {
        let (_tmp, config) = pick_fixture();
        let mut starts: Vec<(String, ReportDialog)> =
            vec![("fresh".into(), ReportDialog::new(None, true))];
        for category in [
            LogOnlyCategory::Crash,
            LogOnlyCategory::Performance,
            LogOnlyCategory::Other,
        ] {
            starts.push((
                format!("opened on {category:?}"),
                ReportDialog::new(Some(category), true),
            ));
        }
        let mut card = ReportDialog::for_game(None, true);
        card.game_session = Some("s1".into());
        card.game_choices = choices();
        starts.push(("game card".into(), card));
        for category in GameCategory::ALL {
            let mut d = ReportDialog::for_game(Some(category), true);
            if !category.allows_missing_kit() {
                d.game_session = Some("s1".into());
            }
            d.game_choices = choices();
            starts.push((format!("game card on {category:?}"), d));
        }

        let mut checked = 0;
        for (start, dialog) in &starts {
            for first in PickedKind::ALL {
                for second in std::iter::once(None).chain(PickedKind::ALL.map(Some)) {
                    let ctx = format!("{start}, then {first:?}, then {second:?}");
                    let mut d = dialog.clone();
                    click(&mut d, first);
                    assert_one_row(&d, &ctx);
                    if let Some(second) = second {
                        click(&mut d, second);
                        assert_one_row(&d, &ctx);
                    }
                    let expected = second.unwrap_or(first);
                    assert_sent_kind_is_picked(&mut d, &config, expected, &ctx);
                    checked += 1;
                }
            }
        }
        assert_eq!(checked, starts.len() * 10 * 11);
    }

    /// Crash and Performance never carry a game or pictures, even when the
    /// dialog opened on a game with its pictures already shown.
    #[test]
    fn a_preselected_game_is_cleared_by_crash_or_performance() {
        let (_tmp, config) = pick_fixture();
        for category in [LogOnlyCategory::Crash, LogOnlyCategory::Performance] {
            for opened_on in std::iter::once(None).chain(GameCategory::ALL.map(Some)) {
                let ctx = format!("{category:?} on a card opened with {opened_on:?}");
                let mut d = ReportDialog::for_game(opened_on, true);
                d.game_session = Some("s1".into());
                d.game_choices = choices();
                d.kit_images = kit_images_from_report_images(
                    stat_tracker::report::bundle::kit_preview_images(
                        &stat_tracker::report::kit::load(&config.data_dir, "s1"),
                    ),
                );
                assert!(!d.kit_images.is_empty(), "{ctx}");
                assert_eq!(d.category, None, "{ctx}: top row starts empty");
                assert!(d.pick_log(category), "{ctx}");
                assert_one_row(&d, &ctx);
                assert_eq!(d.game_session, None, "{ctx}");
                assert!(!d.sends_pictures(), "{ctx}");
                assert_sent_kind_is_picked(&mut d, &config, PickedKind::Log(category), &ctx);
            }
        }
    }

    /// Reopening Report a problem starts with nothing picked: no chip in
    /// either row, nothing built, and Send and Save stay off.
    #[test]
    fn reopening_starts_with_nothing_picked() {
        let (_tmp, config) = pick_fixture();
        let mut d = ReportDialog::new(None, true);
        click(&mut d, PickedKind::Game(GameCategory::WrongStats));
        assert_sent_kind_is_picked(
            &mut d,
            &config,
            PickedKind::Game(GameCategory::WrongStats),
            "before close",
        );
        // Close drops the dialog; the menu opens a new one with `Open(None)`.
        let reopened = ReportDialog::new(None, true);
        assert_eq!(reopened.picked(), None);
        assert!(PickedKind::ALL.iter().all(|k| !reopened.is_picked(*k)));
        assert_eq!(reopened.game_session, None);
        assert_eq!(reopened.build_plan(), BuildPlan::Nothing);
        assert!(!reopened.can_send() && !reopened.can_save());
        assert_eq!(reopened.send_label(), "Send");
        // A game card opens with the top row empty too.
        let card = ReportDialog::for_game(None, true);
        assert_eq!(card.picked(), None);
        assert_eq!(card.category, None);
    }

    #[test]
    fn send_label_names_the_kind_and_the_pictures() {
        let mut d = ReportDialog::new(Some(LogOnlyCategory::Performance), true);
        assert_eq!(d.send_label(), "Send Performance report (no pictures)");
        d.send = SendState::Done(SendOutcome::Failed("x".into()));
        assert_eq!(
            d.send_label(),
            "Send again: Performance report (no pictures)"
        );
        d.send = SendState::Sending;
        assert_eq!(d.send_label(), "Sending...");
        let mut d = ReportDialog::new(Some(LogOnlyCategory::Crash), true);
        assert_eq!(d.send_label(), "Send Crash report (no pictures)");
        d.pick_game(GameCategory::WrongStats, Vec::new());
        assert_eq!(
            d.send_label(),
            "Send game report: Wrong stats (with pictures)"
        );
        d.pick_game(GameCategory::MissedGame, Vec::new());
        assert_eq!(
            d.send_label(),
            "Send game report: Missed game (no pictures)"
        );
    }

    /// The note box asks what the member noticed on Crash and Performance
    /// reports, and keeps the general hint on every other kind.
    #[test]
    fn note_hint_follows_the_picked_kind() {
        let d = ReportDialog::new(None, true);
        assert_eq!(d.note_hint(), NOTE_HINT, "nothing picked");
        for kind in PickedKind::ALL {
            let mut d = ReportDialog::new(None, true);
            match kind {
                PickedKind::Log(category) => assert!(d.pick_log(category)),
                PickedKind::Game(category) => assert!(d.pick_game(category, Vec::new())),
            }
            let want = match kind {
                PickedKind::Log(LogOnlyCategory::Crash | LogOnlyCategory::Performance) => {
                    LOG_NOTE_HINT
                }
                _ => NOTE_HINT,
            };
            assert_eq!(d.note_hint(), want, "{kind:?}");
        }
        assert_eq!(
            LOG_NOTE_HINT,
            "What did you notice? Stutter, FPS drops, high CPU, which hero?"
        );
    }

    #[test]
    fn clamp_note_strips_nul_and_caps_at_500_characters() {
        assert_eq!(clamp_note("high\0 CPU\0"), "high CPU");
        assert_eq!(clamp_note(""), "");
        let long = "\u{e9}".repeat(REASON_TEXT_MAX_CHARS + 40);
        let kept = clamp_note(&long);
        assert_eq!(kept.chars().count(), REASON_TEXT_MAX_CHARS);
        assert_eq!(REASON_TEXT_MAX_CHARS, 500);
        // NULs do not count toward the cap.
        let padded = format!("{}{}", "\0".repeat(10), "a".repeat(REASON_TEXT_MAX_CHARS));
        assert_eq!(clamp_note(&padded), "a".repeat(REASON_TEXT_MAX_CHARS));
    }

    /// A Crash or Performance zip carries the note in `reason.text`, with
    /// NULs dropped and at most 500 characters, also when the text reaches
    /// the builder without going through the dialog.
    #[test]
    fn crash_and_performance_notes_go_into_reason_text() {
        let tmp = tempfile::tempdir().unwrap();
        let config = Config {
            data_dir: tmp.path().to_path_buf(),
            ..Default::default()
        };
        let built_at = Utc.with_ymd_and_hms(2026, 10, 18, 12, 0, 0).unwrap();
        let reason = |category: LogOnlyCategory, text: String| {
            let input = LogReportInput::from_config(
                &config,
                category,
                text,
                FALLBACK_RESOLUTION,
                CaptureBackend::None,
                false,
                Vec::new(),
                built_at,
            );
            let zip = stat_tracker::report::build_log_report(&input).unwrap();
            let manifest: serde_json::Value = serde_json::from_str(&zip.manifest_json).unwrap();
            assert_eq!(manifest["reason"]["category"], category.as_str());
            manifest["reason"]["text"].as_str().unwrap().to_string()
        };
        for category in [LogOnlyCategory::Crash, LogOnlyCategory::Performance] {
            let typed = "Stutter on Dorado\0, FPS drops as Lifeweaver";
            assert_eq!(
                reason(category, clamp_note(typed)),
                "Stutter on Dorado, FPS drops as Lifeweaver"
            );
            assert_eq!(
                reason(category, typed.to_string()),
                "Stutter on Dorado, FPS drops as Lifeweaver",
                "the builder strips NUL by itself"
            );
            let long = "fps ".repeat(200);
            assert_eq!(
                reason(category, long.clone()).chars().count(),
                REASON_TEXT_MAX_CHARS
            );
            assert_eq!(
                reason(category, clamp_note(&long)).chars().count(),
                REASON_TEXT_MAX_CHARS
            );
            assert_eq!(reason(category, String::new()), "", "the note is optional");
        }
    }
}
