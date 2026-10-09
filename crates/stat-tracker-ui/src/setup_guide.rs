//! First-run setup guide for the desktop app.
//!
//! Shows when there is no config file yet, or when `setup_completed` is not
//! set. Skip and Finish write that flag by editing `config.toml` in place.
//! Test captures and the Tab check stay in memory. The sign-in device code
//! stays in memory too: it is not written to disk and it is not logged.
//!
//! Server contracts:
//!
//! * `POST /api/link/start` and `POST /api/link/poll` use the shared
//!   `scuffed_types` device-link bodies. Start sends
//!   `{device_label, app_version}` and returns
//!   `{user_code, device_code, interval, expires_in}`. `user_code` is
//!   `XXXX-XXXX`. `device_code` is 64 hex characters. `interval` is the
//!   fixed gap between polls (5 seconds on the current server). `expires_in`
//!   is the code lifetime (600 seconds on the current server). Poll sends
//!   `{device_code}` and returns `{status}` of
//!   `pending`, `slow_down`, `denied`, `expired`, or `approved`.
//!   Neither request sends an `Origin` header.
//!   Each `slow_down` adds 5 seconds to this client's poll interval and
//!   keeps that raised gap for the rest of that device code. `approved`
//!   includes `token` once. The next poll is `expired` and has no token.
//!   A later result does not replace the token already saved.
//! * A non-success body is `{"error":"..."}` except HTTP 429, which is not
//!   parsed, and HTTP 404, which means the route is not on this server yet.
//! * `GET /api/stats/token-check` returns `{display_name}` or
//!   `{"error":"Unauthorized"}` for any failure. One check per button press.
//! * HTTP 429 on `/api/link/*` is JSON
//!   `{"error":"rate_limited","retry_after":N}` plus a `Retry-After` header.
//!   Token-check can return the same kind of 429. The tracker does not parse
//!   that body, and it also accepts a plain-text 429. The wait is the
//!   `Retry-After` header when that header is a number. Otherwise the wait
//!   is 10 seconds. A poll waits that long and does not change its interval.
//! * HTTP 404 on token-check or `POST /api/link/start` means this server
//!   does not have those routes yet. The token is still saved, and site
//!   sign-in is hidden in favour of pasting a token.

use std::path::Path;
use std::time::{Duration, Instant};

use iced::widget::image::Handle;
use iced::widget::{button, column, container, opaque, row, scrollable, space, text, text_input};
use iced::{Alignment, Element, Fill, Length, Padding};

use scuffed_types::{
    DeviceLinkPollRequest, DeviceLinkPollResponse, DeviceLinkStartRequest, DeviceLinkStartResponse,
};
use stat_tracker::config::{Config, SetupDiskPatch, SyncConfig};

use crate::app::Message;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_FEATURED, SIZE_META, SIZE_TITLE,
    TEXT, TEXT_2, TEXT_3,
};

pub const CAPTURE_FAILED: &str =
    "Could not capture the screen. Allow screen capture for this app, then try again.";
pub const REACH_SITE: &str = "Could not reach the site. Check the address and try again.";
pub const TOKEN_REJECTED: &str = "That token wasn't accepted. Check it and try again.";
pub const TOKEN_UNCHECKED: &str =
    "The token is saved. This server doesn't support a token check yet.";
pub const LINK_UNSUPPORTED: &str =
    "This server doesn't support sign-in from the app yet. Paste a token instead.";
pub const SIGNED_IN: &str = "Signed in. The tracker will use this token.";
pub const LINK_DENIED: &str = "The site declined this sign-in. You can paste a token instead.";
pub const LINK_EXPIRED: &str = "That sign-in code expired. Start again to get a new one.";
pub const UNREADABLE_SITE: &str = "The site sent a response the tracker could not read.";
pub const SAVE_FAILED: &str = "Could not save that setting. Check config.toml and try again.";
pub const CAPTURE_INTRO: &str = "\
The tracker needs permission to capture your screen. \
The test picture stays in memory and is not saved.";
pub const CAPTURE_OK: &str = "Screen capture works.";
pub const OW_INTRO: &str = "\
Open Overwatch and press Tab once so the scoreboard is on screen. \
Then capture that screen. The picture stays in memory and is not saved.";
pub const OW_SCALE: &str = "Unsupported UI scale and colorblind filters may hurt reads.";
pub const OW_COLOUR: &str =
    "Win and loss detection does not rely on colour, so custom colour schemes are fine.";
pub const CAPTURE_CONTINUE_WARN: &str =
    "Screen capture did not work. You can continue and still sign in.";
pub const OW_SKIP_WARN: &str =
    "You can skip this step if Overwatch is not running. Sign in is still available.";
pub use stat_tracker::packs::PACK_TOO_BIG;
pub const SITE_TRY_AGAIN: &str = "The site could not finish sign-in. Try again in a moment.";
pub const SITE_BAD_LABEL: &str = "The tracker could not start sign-in. Try again.";
pub const SITE_BAD_VERSION: &str =
    "This app version was not accepted. Update the tracker and try again.";
pub const SITE_INVALID_CODE: &str = "That sign-in code was not accepted. Start again.";
pub const SITE_BODY_REQUIRED: &str = "The site could not read the sign-in request. Try again.";
pub const SITE_BAD_ORIGIN: &str = "The site refused this sign-in request. Try again.";
pub const SITE_INTERNAL: &str = "The site had a problem. Try again in a moment.";
pub const PACK_INTRO: &str = "\
When you are signed in, the tracker can download a reader pack from the site. \
It is saved with the hero templates. You can finish without it.";
pub use stat_tracker::packs::PACK_SAVED;
pub const DEVICE_LABEL: &str = "Scuffed Tracker";

const USER_CODE_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Show the guide on launch when no config file exists, or setup is not done.
pub fn should_auto_open(config_existed: bool, setup_completed: bool) -> bool {
    !config_existed || !setup_completed
}

pub fn reader_pack_configured(url: Option<&str>) -> bool {
    url.is_some_and(|value| !value.trim().is_empty())
}

pub fn connected_as(display_name: &str) -> String {
    format!("Connected as {display_name}")
}

pub use stat_tracker::packs::{
    PACK_MAX_BYTES as READER_PACK_MAX_BYTES, RATE_LIMIT_FALLBACK_SECS, RETRY_AFTER_MAX_SECS,
    RETRY_AFTER_MIN_SECS, clamp_retry_after, rate_limit_message, rate_limit_seconds,
};

/// Added to the poll interval on each `slow_down`. It stays raised for
/// the rest of that device code. A 429 does not use this step.
pub const SLOW_DOWN_STEP_SECS: u64 = 5;

/// Static sentences the guide shows. Tests reject em and en dashes here.
pub fn user_facing_copy() -> &'static [&'static str] {
    &[
        CAPTURE_FAILED,
        REACH_SITE,
        TOKEN_REJECTED,
        TOKEN_UNCHECKED,
        CAPTURE_CONTINUE_WARN,
        OW_SKIP_WARN,
        PACK_TOO_BIG,
        stat_tracker::packs::PACK_CURRENT,
        stat_tracker::packs::PACK_UNAVAILABLE,
        stat_tracker::packs::PACK_UNSUPPORTED,
        stat_tracker::packs::PACK_MISMATCH,
        stat_tracker::packs::PACK_UNSAFE,
        stat_tracker::packs::PACK_FAILED,
        stat_tracker::packs::PACK_AUTH,
        stat_tracker::packs::PACK_SIGN_IN,
        SITE_TRY_AGAIN,
        SITE_BAD_LABEL,
        SITE_BAD_VERSION,
        SITE_INVALID_CODE,
        SITE_BODY_REQUIRED,
        SITE_BAD_ORIGIN,
        SITE_INTERNAL,
        "Skip this step",
        LINK_UNSUPPORTED,
        SIGNED_IN,
        LINK_DENIED,
        LINK_EXPIRED,
        UNREADABLE_SITE,
        SAVE_FAILED,
        CAPTURE_INTRO,
        CAPTURE_OK,
        OW_INTRO,
        OW_SCALE,
        OW_COLOUR,
        PACK_INTRO,
        PACK_SAVED,
        "Allow screen capture",
        "Capture the Tab screen",
        "I can see the scoreboard",
        "Sign in with the site",
        "Open the site",
        "Paste a token instead",
        "Check token",
        "Download reader pack",
        "Open setup guide",
        "Skip",
        "Continue",
        "Finish",
        "Back",
        "Screen capture",
        "Overwatch",
        "Sign in",
        "Reader pack",
        "Setup",
        "Paste a token first.",
        "Scoreboard confirmed.",
        "Waiting for you to approve this code on the site.",
        "No picture size yet. Capture the screen first.",
    ]
}

#[derive(Clone, PartialEq, Eq)]
pub struct MemoryFrame {
    /// Full capture size. The guide shows this as the detected resolution.
    pub width: u32,
    pub height: u32,
    /// In-memory preview. Not written to disk.
    pub preview_width: u32,
    pub preview_height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for MemoryFrame {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MemoryFrame")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.rgba.len())
            .finish()
    }
}

/// Keep a test or Tab frame in memory. `disk` is ignored on purpose.
pub fn retain_memory_frame(frame: MemoryFrame, _disk: &Path) -> MemoryFrame {
    frame
}

pub fn memory_frame_from_image(img: &::image::DynamicImage) -> MemoryFrame {
    let width = img.width();
    let height = img.height();
    let (preview_width, preview_height, rgba) = crate::capture::thumbnail_rgba(img);
    MemoryFrame {
        width,
        height,
        preview_width,
        preview_height,
        rgba,
    }
}

pub async fn capture_test_frame(
    backend: stat_tracker::capture::CaptureBackend,
    output: Option<String>,
) -> Result<MemoryFrame, String> {
    let backend =
        crate::capture::backend_ready(Some(backend)).map_err(|_| CAPTURE_FAILED.to_string())?;
    let img = stat_tracker::capture::capture_screen_output(&backend, output.as_deref())
        .await
        .map_err(|_| CAPTURE_FAILED.to_string())?;
    let frame = tokio::task::spawn_blocking(move || memory_frame_from_image(&img))
        .await
        .map_err(|_| "Could not prepare the test capture.".to_string())?;
    Ok(retain_memory_frame(frame, Path::new("")))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct DisplayGeometry {
    pub width: u32,
    pub height: u32,
    pub aspect_w: u32,
    pub aspect_h: u32,
    pub is_16_9: bool,
}

pub fn display_geometry(width: u32, height: u32) -> DisplayGeometry {
    let is_16_9 = height > 0 && {
        let ratio = f64::from(width) / f64::from(height);
        (ratio - 16.0 / 9.0).abs() <= 0.02
    };
    let (aspect_w, aspect_h) = reduce_ratio(width, height);
    DisplayGeometry {
        width,
        height,
        aspect_w,
        aspect_h,
        is_16_9,
    }
}

pub fn geometry_line(geometry: &DisplayGeometry) -> String {
    if geometry.height == 0 || geometry.width == 0 {
        return "No picture size yet. Capture the screen first.".to_string();
    }
    if geometry.is_16_9 {
        format!("{}x{} is 16:9.", geometry.width, geometry.height)
    } else {
        format!(
            "{}x{} is {}:{}, which is not 16:9. The tracker is built for a 16:9 picture.",
            geometry.width, geometry.height, geometry.aspect_w, geometry.aspect_h
        )
    }
}

fn reduce_ratio(mut width: u32, mut height: u32) -> (u32, u32) {
    if width == 0 || height == 0 {
        return (width, height);
    }
    let divisor = gcd(width, height);
    width /= divisor;
    height /= divisor;
    (width, height)
}

fn gcd(mut a: u32, mut b: u32) -> u32 {
    while b != 0 {
        let next = a % b;
        a = b;
        b = next;
    }
    a
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GuideStep {
    Capture,
    Overwatch,
    Sync,
    ReaderPack,
}

impl GuideStep {
    pub fn title(self) -> &'static str {
        match self {
            Self::Capture => "Screen capture",
            Self::Overwatch => "Overwatch",
            Self::Sync => "Sign in",
            Self::ReaderPack => "Reader pack",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Advance {
    Step(GuideStep),
    Finished,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GuideFlow {
    steps: Vec<GuideStep>,
    index: usize,
}

impl GuideFlow {
    pub fn new(include_reader_pack: bool) -> Self {
        let mut steps = vec![GuideStep::Capture, GuideStep::Overwatch, GuideStep::Sync];
        if include_reader_pack {
            steps.push(GuideStep::ReaderPack);
        }
        Self { steps, index: 0 }
    }

    pub fn current(&self) -> GuideStep {
        self.steps[self.index]
    }

    pub fn position(&self) -> (usize, usize) {
        (self.index + 1, self.steps.len())
    }

    pub fn can_go_back(&self) -> bool {
        self.index > 0
    }

    pub fn is_last(&self) -> bool {
        self.index + 1 == self.steps.len()
    }

    pub fn back(&mut self) {
        self.index = self.index.saturating_sub(1);
    }

    pub fn advance(&mut self) -> Advance {
        if self.is_last() {
            Advance::Finished
        } else {
            self.index += 1;
            Advance::Step(self.current())
        }
    }
}

/// Device code for the sign-in poll. Display and Debug never reveal it.
#[derive(Clone, PartialEq, Eq)]
pub struct DeviceCode(String);

impl DeviceCode {
    pub fn new(raw: impl Into<String>) -> Self {
        Self(raw.into())
    }

    /// Only for the poll request body.
    pub fn for_request(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Debug for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("DeviceCode([redacted])")
    }
}

impl std::fmt::Display for DeviceCode {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("[redacted]")
    }
}

pub fn redact_secret(text: &str, secret: &str) -> String {
    if secret.is_empty() {
        return text.to_string();
    }
    text.replace(secret, "[redacted]")
}

/// Log a link status. The status is a fixed word, never the device code.
pub fn log_link_status(status: &'static str) {
    tracing::info!(status, "site link update");
}

#[derive(Clone, PartialEq, Eq)]
pub enum PollOutcome {
    Pending,
    SlowDown,
    Denied,
    Expired,
    Approved { token: String },
}

impl std::fmt::Debug for PollOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Pending => f.write_str("Pending"),
            Self::SlowDown => f.write_str("SlowDown"),
            Self::Denied => f.write_str("Denied"),
            Self::Expired => f.write_str("Expired"),
            Self::Approved { .. } => f.write_str("Approved { token: [redacted] }"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PollDecision {
    Continue { interval_secs: u64 },
    Stop,
}

#[derive(Clone, PartialEq, Eq)]
enum LinkPhase {
    Polling {
        interval_secs: u64,
        started: Instant,
        expires_in: Duration,
    },
    Approved {
        token: String,
    },
    Denied,
    Expired,
}

#[derive(Clone)]
pub struct LinkMachine {
    device_code: DeviceCode,
    user_code: String,
    server_url: String,
    phase: LinkPhase,
}

impl std::fmt::Debug for LinkMachine {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkMachine")
            .field("user_code", &self.user_code)
            .field("device_code", &self.device_code)
            .field("server_url", &self.server_url)
            .field("phase", &self.phase_name())
            .finish()
    }
}

impl LinkMachine {
    pub fn start(
        device_code: String,
        user_code: String,
        server_url: String,
        interval_secs: u64,
        expires_in_secs: u64,
    ) -> Self {
        Self {
            device_code: DeviceCode::new(device_code),
            user_code,
            server_url,
            phase: LinkPhase::Polling {
                interval_secs: normalize_interval(interval_secs),
                started: Instant::now(),
                expires_in: Duration::from_secs(expires_in_secs),
            },
        }
    }

    pub fn user_code(&self) -> &str {
        &self.user_code
    }

    pub fn server_url(&self) -> &str {
        &self.server_url
    }

    pub fn interval_secs(&self) -> u64 {
        match &self.phase {
            LinkPhase::Polling { interval_secs, .. } => *interval_secs,
            _ => normalize_interval(0),
        }
    }

    pub fn saved_token(&self) -> Option<&str> {
        match &self.phase {
            LinkPhase::Approved { token } => Some(token.as_str()),
            _ => None,
        }
    }

    pub fn is_polling(&self) -> bool {
        matches!(self.phase, LinkPhase::Polling { .. })
    }

    pub fn is_terminal(&self) -> bool {
        !self.is_polling()
    }

    fn phase_name(&self) -> &'static str {
        match &self.phase {
            LinkPhase::Polling { .. } => "pending",
            LinkPhase::Approved { .. } => "approved",
            LinkPhase::Denied => "denied",
            LinkPhase::Expired => "expired",
        }
    }

    pub fn device_code_for_request(&self) -> &str {
        self.device_code.for_request()
    }

    /// Keep the longer gap for later polls of this same device code.
    fn raise_interval(&mut self, step: u64) -> u64 {
        match &mut self.phase {
            LinkPhase::Polling { interval_secs, .. } => {
                *interval_secs = normalize_interval(interval_secs.saturating_add(step));
                *interval_secs
            }
            _ => self.interval_secs(),
        }
    }

    /// A second approved result does not replace the token.
    pub fn apply(&mut self, outcome: PollOutcome) -> PollDecision {
        if self.is_terminal() {
            return PollDecision::Stop;
        }
        match outcome {
            PollOutcome::Pending => {
                log_link_status("pending");
                PollDecision::Continue {
                    interval_secs: self.interval_secs(),
                }
            }
            PollOutcome::SlowDown => {
                log_link_status("slow_down");
                let interval_secs = self.raise_interval(SLOW_DOWN_STEP_SECS);
                PollDecision::Continue { interval_secs }
            }
            PollOutcome::Denied => {
                self.phase = LinkPhase::Denied;
                log_link_status("denied");
                PollDecision::Stop
            }
            PollOutcome::Expired => {
                self.phase = LinkPhase::Expired;
                log_link_status("expired");
                PollDecision::Stop
            }
            PollOutcome::Approved { token } => {
                self.phase = LinkPhase::Approved { token };
                log_link_status("approved");
                PollDecision::Stop
            }
        }
    }

    pub fn mark_approved(&mut self, token: String) {
        if self.is_terminal() {
            return;
        }
        self.phase = LinkPhase::Approved { token };
        log_link_status("approved");
    }

    pub fn expire_if_due(&mut self, now: Instant) -> bool {
        let LinkPhase::Polling {
            started,
            expires_in,
            ..
        } = &self.phase
        else {
            return false;
        };
        if now.saturating_duration_since(*started) >= *expires_in {
            self.phase = LinkPhase::Expired;
            log_link_status("expired");
            true
        } else {
            false
        }
    }
}

pub fn normalize_interval(secs: u64) -> u64 {
    secs.clamp(1, 300)
}

/// `user_code` on the wire: `XXXX-XXXX` from the server alphabet.
pub fn is_user_code(raw: &str) -> bool {
    let bytes = raw.as_bytes();
    bytes.len() == 9
        && bytes[4] == b'-'
        && bytes
            .iter()
            .enumerate()
            .all(|(index, byte)| index == 4 || USER_CODE_ALPHABET.contains(byte))
}

/// `device_code` on the wire: 64 hex characters.
pub fn is_device_code(raw: &str) -> bool {
    raw.len() == 64 && raw.bytes().all(|byte| byte.is_ascii_hexdigit())
}

pub fn parse_start_body(body: &str) -> Result<DeviceLinkStartResponse, String> {
    let started: DeviceLinkStartResponse =
        serde_json::from_str(body).map_err(|_| UNREADABLE_SITE.to_string())?;
    if !is_user_code(&started.user_code)
        || !is_device_code(&started.device_code)
        || started.interval == 0
        || started.expires_in == 0
    {
        return Err(UNREADABLE_SITE.to_string());
    }
    Ok(started)
}

fn site_error_message(body: &str, fallback: &'static str) -> &'static str {
    let logged = redact_logged_body(body);
    tracing::warn!(error = %logged, "site sign-in response");
    if let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(message) = value.get("error").and_then(|item| item.as_str())
    {
        let message = message.trim();
        if !message.is_empty() {
            return mapped_site_error(message);
        }
    }
    fallback
}

fn mapped_site_error(raw: &str) -> &'static str {
    match raw {
        "device_label must be 1-64 characters without control characters" => SITE_BAD_LABEL,
        "app_version must be 1-32 characters from [A-Za-z0-9._+-] and include a letter or digit" => {
            SITE_BAD_VERSION
        }
        "invalid code" => SITE_INVALID_CODE,
        "codes must be sent in the request body" => SITE_BODY_REQUIRED,
        "bad_origin" | "origin not allowed" => SITE_BAD_ORIGIN,
        "Internal error" => SITE_INTERNAL,
        _ => SITE_TRY_AGAIN,
    }
}

/// Log text must not keep device codes or token values.
fn redact_logged_body(raw: &str) -> String {
    redact_hex_runs(&redact_json_string_field(
        &redact_json_string_field(raw, "token"),
        "device_code",
    ))
}

fn redact_json_string_field(raw: &str, field: &str) -> String {
    let key = format!("\"{field}\"");
    let mut rest = raw;
    let mut out = String::new();
    while let Some(pos) = rest.find(&key) {
        out.push_str(&rest[..pos]);
        out.push_str(&key);
        let after_key = &rest[pos + key.len()..];
        let trimmed = after_key.trim_start();
        out.push_str(&after_key[..after_key.len() - trimmed.len()]);
        let Some(after_colon_ws) = trimmed.strip_prefix(':') else {
            rest = after_key;
            continue;
        };
        out.push(':');
        let value_zone = after_colon_ws.trim_start();
        out.push_str(&after_colon_ws[..after_colon_ws.len() - value_zone.len()]);
        let Some(value) = value_zone.strip_prefix('"') else {
            rest = value_zone;
            continue;
        };
        out.push('"');
        let mut end = value.len();
        let mut escaped = false;
        for (index, ch) in value.char_indices() {
            if escaped {
                escaped = false;
                continue;
            }
            if ch == '\\' {
                escaped = true;
                continue;
            }
            if ch == '"' {
                end = index;
                break;
            }
        }
        out.push_str("[redacted]");
        out.push('"');
        rest = &value[end..];
        if rest.starts_with('"') {
            rest = &rest[1..];
        }
    }
    out.push_str(rest);
    out
}

fn redact_hex_runs(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    let mut run = String::new();
    let flush = |run: &mut String, out: &mut String| {
        if run.len() >= 32 {
            out.push_str("[redacted]");
        } else {
            out.push_str(run);
        }
        run.clear();
    };
    for ch in raw.chars() {
        if ch.is_ascii_hexdigit() {
            run.push(ch);
        } else {
            flush(&mut run, &mut out);
            out.push(ch);
        }
    }
    flush(&mut run, &mut out);
    out
}

pub fn parse_poll_body(body: &str) -> Result<PollOutcome, String> {
    let parsed: DeviceLinkPollResponse =
        serde_json::from_str(body).map_err(|_| UNREADABLE_SITE.to_string())?;
    match parsed.status.as_str() {
        "pending" => Ok(PollOutcome::Pending),
        "slow_down" => Ok(PollOutcome::SlowDown),
        "denied" => Ok(PollOutcome::Denied),
        "expired" => Ok(PollOutcome::Expired),
        "approved" => {
            let token = parsed.token.unwrap_or_default();
            let token = token.trim();
            if token.is_empty() {
                Err("The site approved sign-in but did not send a token.".into())
            } else {
                Ok(PollOutcome::Approved {
                    token: token.to_string(),
                })
            }
        }
        _ => Err("The site sent a sign-in update the tracker did not understand.".into()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TokenCheckResult {
    Connected {
        display_name: String,
    },
    Rejected,
    Wait {
        seconds: u64,
    },
    /// HTTP 404: the route is not on this server yet.
    Unchecked,
}

pub fn map_token_check(status: u16, body: &str, retry_after_secs: Option<u64>) -> TokenCheckResult {
    if status == 429 {
        // The body is ignored on purpose. A 429 may be plain text or JSON.
        return TokenCheckResult::Wait {
            seconds: rate_limit_seconds(retry_after_secs),
        };
    }
    if status == 404 {
        return TokenCheckResult::Unchecked;
    }
    if status == 200
        && let Ok(value) = serde_json::from_str::<serde_json::Value>(body)
        && let Some(name) = value.get("display_name").and_then(|item| item.as_str())
    {
        let name = name.trim();
        if !name.is_empty() {
            return TokenCheckResult::Connected {
                display_name: name.to_string(),
            };
        }
    }
    TokenCheckResult::Rejected
}

pub fn token_check_message(result: &TokenCheckResult) -> String {
    match result {
        TokenCheckResult::Connected { display_name } => connected_as(display_name),
        TokenCheckResult::Rejected => TOKEN_REJECTED.to_string(),
        TokenCheckResult::Wait { seconds } => rate_limit_message(*seconds),
        TokenCheckResult::Unchecked => TOKEN_UNCHECKED.to_string(),
    }
}

pub fn normalize_base(url: &str) -> Result<String, String> {
    stat_tracker::config::validate_sync_server_url(url).map_err(|err| err.message().to_string())?;
    Ok(url.trim().trim_end_matches('/').to_string())
}

pub fn site_link_url(server_url: &str) -> Result<String, String> {
    let base = normalize_base(server_url)?;
    Ok(format!("{base}/link"))
}

fn http_client() -> Result<reqwest::Client, String> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| REACH_SITE.to_string())
}

fn retry_after_secs(headers: &reqwest::header::HeaderMap) -> Option<u64> {
    headers
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|raw| raw.trim().parse().ok())
}

#[derive(Clone)]
pub struct LinkStart {
    pub machine: LinkMachine,
}

/// `Unsupported` is HTTP 404: this server does not have `/api/link/start` yet.
#[derive(Debug, Clone)]
pub enum LinkStartOutcome {
    Ready(LinkStart),
    Unsupported,
}

impl std::fmt::Debug for LinkStart {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("LinkStart")
            .field("machine", &self.machine)
            .finish()
    }
}

pub async fn start_link(
    base: String,
    device_label: String,
    app_version: String,
) -> Result<LinkStartOutcome, String> {
    let client = http_client()?;
    let url = format!("{base}/api/link/start");
    // Start does not check Origin. The body is the shared request type.
    let response = client
        .post(&url)
        .json(&DeviceLinkStartRequest {
            device_label,
            app_version,
        })
        .send()
        .await
        .map_err(|_| REACH_SITE.to_string())?;
    let status = response.status();
    let retry = retry_after_secs(response.headers());
    if status.as_u16() == 429 {
        return Err(rate_limit_message(rate_limit_seconds(retry)));
    }
    let text = response.text().await.unwrap_or_default();
    if status.as_u16() == 404 {
        return Ok(LinkStartOutcome::Unsupported);
    }
    if !status.is_success() {
        return Err(site_error_message(&text, SITE_TRY_AGAIN).to_string());
    }
    let started = parse_start_body(&text)?;
    log_link_status("started");
    Ok(LinkStartOutcome::Ready(LinkStart {
        machine: LinkMachine::start(
            started.device_code,
            started.user_code,
            base,
            started.interval,
            started.expires_in,
        ),
    }))
}

/// A poll response. `RateLimited` does not parse the body.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PollUpdate {
    Outcome(PollOutcome),
    RateLimited { seconds: u64 },
}

pub async fn poll_link(base: String, device_code: String) -> Result<PollUpdate, String> {
    let client = http_client()?;
    let url = format!("{base}/api/link/poll");
    // Poll does not check Origin. The body is the shared request type.
    let response = client
        .post(&url)
        .json(&DeviceLinkPollRequest {
            device_code: device_code.clone(),
        })
        .send()
        .await
        .map_err(|_| REACH_SITE.to_string())?;
    let status = response.status();
    let retry = retry_after_secs(response.headers());
    if status.as_u16() == 429 {
        return Ok(PollUpdate::RateLimited {
            seconds: rate_limit_seconds(retry),
        });
    }
    let text = response.text().await.unwrap_or_default();
    let text = redact_secret(&text, &device_code);
    if !status.is_success() {
        return Err(site_error_message(&text, SITE_TRY_AGAIN).to_string());
    }
    parse_poll_body(&text)
        .map(PollUpdate::Outcome)
        .map_err(|err| redact_secret(&err, &device_code))
}

/// One GET. Callers must not loop on the result.
pub async fn check_token(base: String, token: String) -> Result<TokenCheckResult, String> {
    let client = http_client()?;
    let url = format!("{base}/api/stats/token-check");
    let response = client
        .get(&url)
        .bearer_auth(&token)
        .send()
        .await
        .map_err(|_| REACH_SITE.to_string())?;
    let status = response.status().as_u16();
    let retry = retry_after_secs(response.headers());
    if status == 429 {
        return Ok(TokenCheckResult::Wait {
            seconds: rate_limit_seconds(retry),
        });
    }
    let body = response.text().await.unwrap_or_default();
    Ok(map_token_check(status, &body, retry))
}

#[derive(Clone)]
pub enum SetupMessage {
    Open,
    Skip,
    SkipStep,
    Back,
    Next,
    SyncUrl(String),
    PasteToken(String),
    TogglePaste,
    RequestCapture,
    CaptureReady(Result<MemoryFrame, String>),
    RequestTab,
    TabReady(Result<MemoryFrame, String>),
    ConfirmScoreboard,
    SignIn,
    LinkStarted(Result<LinkStartOutcome, String>),
    PollReady(Result<PollUpdate, String>),
    CheckToken,
    TokenChecked(Result<TokenCheckResult, String>),
    OpenSite,
    DownloadPack,
    PackReady(Result<String, String>),
}

impl std::fmt::Debug for SetupMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Open => f.write_str("Open"),
            Self::Skip => f.write_str("Skip"),
            Self::SkipStep => f.write_str("SkipStep"),
            Self::Back => f.write_str("Back"),
            Self::Next => f.write_str("Next"),
            Self::SyncUrl(url) => f.debug_tuple("SyncUrl").field(url).finish(),
            Self::PasteToken(_) => f.write_str("PasteToken([redacted])"),
            Self::TogglePaste => f.write_str("TogglePaste"),
            Self::RequestCapture => f.write_str("RequestCapture"),
            Self::CaptureReady(Ok(_)) => f.write_str("CaptureReady(Ok([frame]))"),
            Self::CaptureReady(Err(_)) => f.write_str("CaptureReady(Err([redacted]))"),
            Self::RequestTab => f.write_str("RequestTab"),
            Self::TabReady(Ok(_)) => f.write_str("TabReady(Ok([frame]))"),
            Self::TabReady(Err(_)) => f.write_str("TabReady(Err([redacted]))"),
            Self::ConfirmScoreboard => f.write_str("ConfirmScoreboard"),
            Self::SignIn => f.write_str("SignIn"),
            Self::LinkStarted(Ok(outcome)) => f.debug_tuple("LinkStarted").field(outcome).finish(),
            Self::LinkStarted(Err(_)) => f.write_str("LinkStarted(Err([redacted]))"),
            Self::PollReady(Ok(update)) => f.debug_tuple("PollReady").field(update).finish(),
            Self::PollReady(Err(_)) => f.write_str("PollReady(Err([redacted]))"),
            Self::CheckToken => f.write_str("CheckToken"),
            Self::TokenChecked(Ok(result)) => f.debug_tuple("TokenChecked").field(result).finish(),
            Self::TokenChecked(Err(_)) => f.write_str("TokenChecked(Err([redacted]))"),
            Self::OpenSite => f.write_str("OpenSite"),
            Self::DownloadPack => f.write_str("DownloadPack"),
            Self::PackReady(Ok(message)) => f.debug_tuple("PackReady").field(message).finish(),
            Self::PackReady(Err(_)) => f.write_str("PackReady(Err([redacted]))"),
        }
    }
}

/// Escape skips the guide. Enter continues. Other keys are ignored.
pub fn guide_key_action(key: &iced::keyboard::Key) -> Option<SetupMessage> {
    match key {
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Escape) => Some(SetupMessage::Skip),
        iced::keyboard::Key::Named(iced::keyboard::key::Named::Enter) => Some(SetupMessage::Next),
        _ => None,
    }
}

pub struct GuideUi {
    pub open: bool,
    flow: GuideFlow,
    capture_busy: bool,
    test_frame: Option<MemoryFrame>,
    capture_error: Option<String>,
    tab_busy: bool,
    tab_frame: Option<MemoryFrame>,
    tab_error: Option<String>,
    scoreboard_confirmed: bool,
    sync_url: String,
    paste_open: bool,
    paste_token: String,
    link_busy: bool,
    /// Set after `POST /api/link/start` returns 404. Site sign-in stays hidden.
    link_unsupported: bool,
    link: Option<LinkMachine>,
    link_message: Option<String>,
    poll_inflight: bool,
    next_poll_at: Option<Instant>,
    check_inflight: bool,
    token_message: Option<String>,
    pack_busy: bool,
    pack_message: Option<String>,
    app_version: String,
    saved_token: bool,
}

impl GuideUi {
    pub fn startup(open: bool, config: &Config, app_version: Option<&str>) -> Self {
        let include_pack = true;
        Self {
            open,
            flow: GuideFlow::new(include_pack),
            capture_busy: false,
            test_frame: None,
            capture_error: None,
            tab_busy: false,
            tab_frame: None,
            tab_error: None,
            scoreboard_confirmed: false,
            sync_url: config
                .sync
                .as_ref()
                .map(|sync| sync.server_url.clone())
                .unwrap_or_default(),
            paste_open: false,
            paste_token: String::new(),
            link_busy: false,
            link_unsupported: false,
            link: None,
            link_message: None,
            poll_inflight: false,
            next_poll_at: None,
            check_inflight: false,
            token_message: None,
            pack_busy: false,
            pack_message: None,
            app_version: app_version
                .filter(|value| !value.trim().is_empty())
                .unwrap_or("unknown")
                .to_string(),
            saved_token: config
                .sync
                .as_ref()
                .is_some_and(|sync| !sync.token.is_empty()),
        }
    }

    pub fn reopen(&mut self, config: &Config) {
        let version = self.app_version.clone();
        *self = Self::startup(true, config, Some(&version));
    }

    pub fn skip(&mut self) -> SetupDiskPatch {
        self.close();
        SetupDiskPatch {
            setup_completed: Some(true),
            sync: None,
        }
    }

    pub fn back(&mut self) {
        self.flow.back();
    }

    /// `Some` when the guide finished and should persist `setup_completed`.
    pub fn continue_step(&mut self) -> Option<SetupDiskPatch> {
        if !self.can_continue() {
            return None;
        }
        match self.flow.advance() {
            Advance::Finished => {
                self.close();
                Some(SetupDiskPatch {
                    setup_completed: Some(true),
                    sync: None,
                })
            }
            Advance::Step(_) => None,
        }
    }

    pub fn can_continue(&self) -> bool {
        match self.flow.current() {
            GuideStep::Capture => {
                !self.capture_busy && (self.test_frame.is_some() || self.capture_error.is_some())
            }
            GuideStep::Overwatch => {
                !self.tab_busy && (self.scoreboard_confirmed || self.tab_error.is_some())
            }
            GuideStep::Sync => !self.link_busy && !self.check_inflight,
            GuideStep::ReaderPack => !self.pack_busy,
        }
    }

    /// Advance Capture or Overwatch without a successful check.
    pub fn skip_step(&mut self) -> Option<SetupDiskPatch> {
        match self.flow.current() {
            GuideStep::Capture if self.capture_busy => return None,
            GuideStep::Overwatch if self.tab_busy => return None,
            GuideStep::Sync | GuideStep::ReaderPack => return self.continue_step(),
            GuideStep::Capture | GuideStep::Overwatch => {}
        }
        match self.flow.advance() {
            Advance::Finished => {
                self.close();
                Some(SetupDiskPatch {
                    setup_completed: Some(true),
                    sync: None,
                })
            }
            Advance::Step(_) => None,
        }
    }

    pub fn capture_warning(&self) -> Option<&'static str> {
        if self.flow.current() == GuideStep::Capture
            && self.capture_error.is_some()
            && self.test_frame.is_none()
        {
            Some(CAPTURE_CONTINUE_WARN)
        } else {
            None
        }
    }

    pub fn set_sync_url(&mut self, value: String) {
        if value != self.sync_url {
            self.link_unsupported = false;
        }
        self.sync_url = value;
    }

    pub fn set_paste_token(&mut self, value: String) {
        self.paste_token = value;
    }

    pub fn toggle_paste(&mut self) {
        self.paste_open = !self.paste_open;
    }

    pub fn begin_capture(&mut self) -> bool {
        if self.capture_busy {
            return false;
        }
        self.capture_busy = true;
        self.capture_error = None;
        true
    }

    pub fn capture_ready(&mut self, result: Result<MemoryFrame, String>) {
        self.capture_busy = false;
        match result {
            Ok(frame) => {
                self.test_frame = Some(retain_memory_frame(frame, Path::new("")));
                self.capture_error = None;
            }
            Err(message) => self.capture_error = Some(message),
        }
    }

    pub fn begin_tab(&mut self) -> bool {
        if self.tab_busy {
            return false;
        }
        self.tab_busy = true;
        self.tab_error = None;
        self.scoreboard_confirmed = false;
        true
    }

    pub fn tab_ready(&mut self, result: Result<MemoryFrame, String>) {
        self.tab_busy = false;
        match result {
            Ok(frame) => {
                self.tab_frame = Some(retain_memory_frame(frame, Path::new("")));
                self.tab_error = None;
            }
            Err(message) => self.tab_error = Some(message),
        }
    }

    pub fn confirm_scoreboard(&mut self) {
        if self.tab_frame.is_some() {
            self.scoreboard_confirmed = true;
        }
    }

    pub fn begin_sign_in(&mut self) -> Option<(String, String, String)> {
        if self.link_unsupported || self.link_busy || self.poll_inflight {
            return None;
        }
        let base = match normalize_base(&self.sync_url) {
            Ok(base) => base,
            Err(message) => {
                self.link_message = Some(message);
                return None;
            }
        };
        self.link_busy = true;
        self.link_message = None;
        self.link = None;
        self.next_poll_at = None;
        Some((base, DEVICE_LABEL.to_string(), self.app_version.clone()))
    }

    pub fn link_started(&mut self, result: Result<LinkStartOutcome, String>) {
        self.link_busy = false;
        match result {
            Ok(LinkStartOutcome::Ready(start)) => {
                let interval = start.machine.interval_secs();
                self.link_unsupported = false;
                self.link_message = Some(format!(
                    "Enter this code on the site: {}",
                    start.machine.user_code()
                ));
                self.link = Some(start.machine);
                self.next_poll_at = Some(Instant::now() + Duration::from_secs(interval));
            }
            Ok(LinkStartOutcome::Unsupported) => {
                self.link_unsupported = true;
                self.paste_open = true;
                self.link = None;
                self.next_poll_at = None;
                self.link_message = Some(LINK_UNSUPPORTED.to_string());
            }
            Err(message) => self.link_message = Some(message),
        }
    }

    /// `(server_url, device_code)` for one poll request. The code is only
    /// for that request.
    pub fn poll_request_if_due(&mut self, now: Instant) -> Option<(String, String)> {
        if !self.open || self.poll_inflight {
            return None;
        }
        let machine = self.link.as_mut()?;
        if machine.expire_if_due(now) {
            self.link_message = Some(LINK_EXPIRED.to_string());
            self.next_poll_at = None;
            return None;
        }
        if !machine.is_polling() {
            return None;
        }
        let due = self.next_poll_at.is_some_and(|at| now >= at);
        if !due {
            return None;
        }
        let url = machine.server_url().to_string();
        let code = machine.device_code_for_request().to_string();
        self.poll_inflight = true;
        Some((url, code))
    }

    pub fn poll_ready(&mut self, result: Result<PollUpdate, String>) -> Option<SetupDiskPatch> {
        self.poll_inflight = false;
        if !self.open {
            return None;
        }
        let machine = self.link.as_mut()?;
        match result {
            Err(message) => {
                let secret = machine.device_code_for_request().to_string();
                self.link_message = Some(redact_secret(&message, &secret));
                if machine.is_polling() {
                    let interval = machine.interval_secs();
                    self.next_poll_at = Some(Instant::now() + Duration::from_secs(interval));
                }
                None
            }
            Ok(PollUpdate::RateLimited { seconds }) => {
                // A 429 waits out Retry-After (or the 10 second fallback).
                // It does not change the interval slow_down may have raised.
                let seconds = clamp_retry_after(seconds);
                self.link_message = Some(rate_limit_message(seconds));
                if machine.is_polling() {
                    self.next_poll_at = Some(Instant::now() + Duration::from_secs(seconds));
                }
                None
            }
            Ok(PollUpdate::Outcome(outcome)) => {
                let decision = machine.apply(outcome);
                match decision {
                    PollDecision::Continue { interval_secs } => {
                        self.next_poll_at =
                            Some(Instant::now() + Duration::from_secs(interval_secs));
                        self.link_message =
                            Some("Waiting for you to approve this code on the site.".to_string());
                        None
                    }
                    PollDecision::Stop => {
                        self.next_poll_at = None;
                        match machine.phase_name() {
                            "denied" => {
                                self.link_message = Some(LINK_DENIED.to_string());
                                None
                            }
                            "expired" => {
                                self.link_message = Some(LINK_EXPIRED.to_string());
                                None
                            }
                            "approved" => {
                                let token = machine.saved_token()?.to_string();
                                let server_url = machine.server_url().to_string();
                                self.link_message = Some(SIGNED_IN.to_string());
                                self.saved_token = true;
                                Some(SetupDiskPatch {
                                    setup_completed: None,
                                    sync: Some(SyncConfig { server_url, token }),
                                })
                            }
                            _ => None,
                        }
                    }
                }
            }
        }
    }

    /// One check per successful call. A second call while one is running
    /// returns `None` and does not schedule another request.
    pub fn begin_check(&mut self) -> Option<(String, String)> {
        if self.check_inflight {
            return None;
        }
        let base = match normalize_base(&self.sync_url) {
            Ok(base) => base,
            Err(message) => {
                self.token_message = Some(message);
                return None;
            }
        };
        let token = self.paste_token.trim().to_string();
        if token.is_empty() {
            self.token_message = Some("Paste a token first.".to_string());
            return None;
        }
        self.check_inflight = true;
        self.token_message = None;
        Some((base, token))
    }

    pub fn token_checked(
        &mut self,
        result: Result<TokenCheckResult, String>,
    ) -> Option<SetupDiskPatch> {
        self.check_inflight = false;
        match result {
            Err(message) => {
                self.token_message = Some(message);
                None
            }
            Ok(result) => {
                self.token_message = Some(token_check_message(&result));
                match result {
                    TokenCheckResult::Connected { .. } | TokenCheckResult::Unchecked => {
                        self.save_pasted_token()
                    }
                    TokenCheckResult::Rejected | TokenCheckResult::Wait { .. } => None,
                }
            }
        }
    }

    fn save_pasted_token(&mut self) -> Option<SetupDiskPatch> {
        let server_url = normalize_base(&self.sync_url).ok()?;
        let token = self.paste_token.trim().to_string();
        if token.is_empty() {
            return None;
        }
        if let Some(machine) = self.link.as_mut() {
            machine.mark_approved(token.clone());
        }
        self.next_poll_at = None;
        self.saved_token = true;
        Some(SetupDiskPatch {
            setup_completed: None,
            sync: Some(SyncConfig { server_url, token }),
        })
    }

    pub fn prepare_site_url(&mut self) -> Option<String> {
        match site_link_url(&self.sync_url) {
            Ok(url) => Some(url),
            Err(message) => {
                self.link_message = Some(message);
                None
            }
        }
    }

    pub fn begin_pack_fetch(&mut self) -> bool {
        if self.pack_busy {
            return false;
        }
        self.pack_busy = true;
        self.pack_message = None;
        true
    }

    pub fn pack_needs_sign_in(&mut self) {
        self.pack_message = Some(stat_tracker::packs::PACK_SIGN_IN.to_string());
    }

    pub fn download_ready(&mut self, result: Result<String, String>) {
        self.pack_busy = false;
        self.pack_message = Some(result.unwrap_or_else(|message| message));
    }

    fn close(&mut self) {
        self.open = false;
        self.link = None;
        self.poll_inflight = false;
        self.next_poll_at = None;
        self.link_busy = false;
        self.link_unsupported = false;
        self.test_frame = None;
        self.tab_frame = None;
    }

    /// Text a log or debug dump of this guide is allowed to contain.
    pub fn redacted_debug(&self) -> String {
        format!("{:?}", self.link)
    }
}

pub fn view(guide: &GuideUi) -> Element<'_, Message> {
    let (step_n, step_total) = guide.flow.position();
    let step = guide.flow.current();
    let header = column![
        text("Setup")
            .size(SIZE_FEATURED)
            .font(theme::FONT_EXTRABOLD)
            .color(TEXT),
        text(format!("Step {step_n} of {step_total}"))
            .size(SIZE_META)
            .font(FONT_MEDIUM)
            .color(TEXT_3),
        text(step.title())
            .size(SIZE_TITLE)
            .font(FONT_BOLD)
            .color(TEXT),
    ]
    .spacing(4);

    let body = match step {
        GuideStep::Capture => capture_body(guide),
        GuideStep::Overwatch => overwatch_body(guide),
        GuideStep::Sync => sync_body(guide),
        GuideStep::ReaderPack => pack_body(guide),
    };

    let mut footer = row![].spacing(8).align_y(Alignment::Center);
    if guide.flow.can_go_back() {
        footer = footer.push(guide_button(
            "Back",
            false,
            Some(Message::Setup(SetupMessage::Back)),
        ));
    }
    footer = footer.push(guide_button(
        "Skip",
        false,
        Some(Message::Setup(SetupMessage::Skip)),
    ));
    if matches!(step, GuideStep::Capture | GuideStep::Overwatch) {
        let busy = match step {
            GuideStep::Capture => guide.capture_busy,
            GuideStep::Overwatch => guide.tab_busy,
            _ => false,
        };
        footer = footer.push(guide_button(
            "Skip this step",
            false,
            (!busy).then_some(Message::Setup(SetupMessage::SkipStep)),
        ));
    }
    footer = footer.push(space().width(Fill));
    let forward = if guide.flow.is_last() {
        "Finish"
    } else {
        "Continue"
    };
    let forward_msg = guide
        .can_continue()
        .then_some(Message::Setup(SetupMessage::Next));
    footer = footer.push(guide_button(forward, true, forward_msg));

    let card = container(
        column![header, scrollable(body).height(Fill).width(Fill), footer,]
            .spacing(16)
            .height(Fill),
    )
    .padding(24)
    .width(Length::Fixed(720.0))
    .height(Fill)
    .style(theme::surface_panel);

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

fn capture_body(guide: &GuideUi) -> Element<'_, Message> {
    let mut col = column![body_text(CAPTURE_INTRO)].spacing(8).width(Fill);
    let label = if guide.capture_busy {
        "Capturing..."
    } else {
        "Allow screen capture"
    };
    let press = (!guide.capture_busy).then_some(Message::Setup(SetupMessage::RequestCapture));
    col = col.push(guide_button(label, true, press));
    if let Some(err) = &guide.capture_error {
        col = col.push(error_text(err));
    }
    if let Some(warning) = guide.capture_warning() {
        col = col.push(warn_text(warning));
    }
    if let Some(frame) = &guide.test_frame {
        col = col.push(body_text(CAPTURE_OK));
        col = col.push(frame_view(frame));
    }
    col.into()
}

fn overwatch_body(guide: &GuideUi) -> Element<'_, Message> {
    let line = guide
        .test_frame
        .as_ref()
        .map(|frame| geometry_line(&display_geometry(frame.width, frame.height)))
        .unwrap_or_else(|| "No picture size yet. Capture the screen first.".to_string());
    let mut col = column![
        body_text(&line),
        body_text(OW_INTRO),
        body_text(OW_SCALE),
        body_text(OW_COLOUR),
        body_text(OW_SKIP_WARN),
    ]
    .spacing(8)
    .width(Fill);
    if !display_geometry(
        guide
            .test_frame
            .as_ref()
            .map(|frame| frame.width)
            .unwrap_or(0),
        guide
            .test_frame
            .as_ref()
            .map(|frame| frame.height)
            .unwrap_or(0),
    )
    .is_16_9
        && guide.test_frame.is_some()
    {
        col = col.push(warn_text(
            "This picture is not 16:9. Set Overwatch to a 16:9 resolution if you can.",
        ));
    }
    let label = if guide.tab_busy {
        "Capturing..."
    } else {
        "Capture the Tab screen"
    };
    let press = (!guide.tab_busy).then_some(Message::Setup(SetupMessage::RequestTab));
    col = col.push(guide_button(label, true, press));
    if let Some(err) = &guide.tab_error {
        col = col.push(error_text(err));
    }
    if let Some(frame) = &guide.tab_frame {
        col = col.push(frame_view(frame));
        let confirm = if guide.scoreboard_confirmed {
            None
        } else {
            Some(Message::Setup(SetupMessage::ConfirmScoreboard))
        };
        col = col.push(guide_button("I can see the scoreboard", true, confirm));
    }
    if guide.scoreboard_confirmed {
        col = col.push(ok_text("Scoreboard confirmed."));
    }
    col.into()
}

fn sync_body(guide: &GuideUi) -> Element<'_, Message> {
    let mut col = column![
        body_text("Connect this computer to the site, or paste a token instead."),
        text("Website URL")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT_2),
        text_input("https://your-site.com", &guide.sync_url)
            .on_input(|value| Message::Setup(SetupMessage::SyncUrl(value)))
            .padding(Padding::from([8, 10]))
            .size(SIZE_BODY)
            .width(Fill)
            .style(theme::text_input_style),
    ]
    .spacing(8)
    .width(Fill);
    if guide.saved_token {
        col = col.push(body_text("A token is already saved. You can replace it."));
    }
    if sign_in_offered(guide) {
        let sign_in = if guide.link_busy {
            None
        } else {
            Some(Message::Setup(SetupMessage::SignIn))
        };
        col = col.push(guide_button("Sign in with the site", true, sign_in));
    }
    if let Some(machine) = &guide.link {
        col = col.push(
            text(machine.user_code())
                .size(SIZE_FEATURED)
                .font(theme::FONT_EXTRABOLD)
                .color(TEXT),
        );
        col = col.push(guide_button(
            "Open the site",
            false,
            Some(Message::Setup(SetupMessage::OpenSite)),
        ));
    }
    if let Some(message) = &guide.link_message {
        col = col.push(body_text(message));
    }
    col = col.push(guide_button(
        "Paste a token instead",
        false,
        Some(Message::Setup(SetupMessage::TogglePaste)),
    ));
    if guide.paste_open {
        col = col.push(
            text_input("paste the token from the website", &guide.paste_token)
                .on_input(|value| Message::Setup(SetupMessage::PasteToken(value)))
                .secure(true)
                .padding(Padding::from([8, 10]))
                .size(SIZE_BODY)
                .width(Fill)
                .style(theme::text_input_style),
        );
        let check = (!guide.check_inflight).then_some(Message::Setup(SetupMessage::CheckToken));
        col = col.push(guide_button("Check token", true, check));
    }
    if let Some(message) = &guide.token_message {
        let warn = message == TOKEN_REJECTED;
        if warn {
            col = col.push(error_text(message));
        } else {
            col = col.push(body_text(message));
        }
    }
    col.into()
}

fn sign_in_offered(guide: &GuideUi) -> bool {
    !guide.link_unsupported
}

fn pack_body(guide: &GuideUi) -> Element<'_, Message> {
    let mut col = column![body_text(PACK_INTRO)].spacing(8).width(Fill);
    let press = (!guide.pack_busy).then_some(Message::Setup(SetupMessage::DownloadPack));
    let label = if guide.pack_busy {
        "Downloading..."
    } else {
        "Download reader pack"
    };
    col = col.push(guide_button(label, true, press));
    if let Some(message) = &guide.pack_message {
        col = col.push(body_text(message));
    }
    col.into()
}

fn frame_view(frame: &MemoryFrame) -> Element<'static, Message> {
    iced::widget::image(Handle::from_rgba(
        frame.preview_width,
        frame.preview_height,
        frame.rgba.clone(),
    ))
    .width(Fill)
    .height(Length::Fixed(220.0))
    .into()
}

fn body_text(value: &str) -> Element<'static, Message> {
    text(value.to_string())
        .size(SIZE_BODY)
        .font(FONT_MEDIUM)
        .color(TEXT_2)
        .into()
}

fn error_text(value: &str) -> Element<'static, Message> {
    text(value.to_string())
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(theme::DANGER)
        .into()
}

fn warn_text(value: &str) -> Element<'static, Message> {
    text(value.to_string())
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(theme::WARN)
        .into()
}

fn ok_text(value: &str) -> Element<'static, Message> {
    text(value.to_string())
        .size(SIZE_META)
        .font(FONT_MEDIUM)
        .color(theme::OK)
        .into()
}

fn guide_button(
    label: &'static str,
    primary: bool,
    msg: Option<Message>,
) -> Element<'static, Message> {
    let label = text(label).size(SIZE_META).font(FONT_SEMIBOLD).color(TEXT);
    if primary {
        let mut btn = button(label)
            .padding(Padding::from([8, 16]))
            .style(theme::chip(true));
        if let Some(msg) = msg {
            btn = btn.on_press(msg);
        }
        btn.into()
    } else {
        let mut btn = button(label)
            .padding(Padding::from([8, 16]))
            .style(theme::ghost_btn());
        if let Some(msg) = msg {
            btn = btn.on_press(msg);
        }
        btn.into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};
    use std::sync::{Arc, Mutex};

    fn no_dash(text: &str) -> bool {
        !text.contains('\u{2014}') && !text.contains('\u{2013}')
    }

    fn assert_plain(text: &str) {
        assert!(no_dash(text), "{text}");
    }

    #[test]
    fn copy_has_no_em_or_en_dash() {
        for line in user_facing_copy() {
            assert_plain(line);
        }
        assert_plain(&geometry_line(&display_geometry(2560, 1080)));
        assert_plain(&geometry_line(&display_geometry(1920, 1080)));
        assert_plain(&connected_as("FrozenTear"));
        assert_plain(&rate_limit_message(12));
        assert_plain(&rate_limit_message(RATE_LIMIT_FALLBACK_SECS));
        assert_plain(&token_check_message(&TokenCheckResult::Rejected));
    }

    #[test]
    fn guide_opens_when_config_is_missing_or_setup_is_not_done() {
        assert!(should_auto_open(false, false));
        assert!(should_auto_open(false, true));
        assert!(should_auto_open(true, false));
        assert!(!should_auto_open(true, true));
    }

    #[test]
    fn steps_skip_the_reader_pack_unless_one_is_configured() {
        let mut plain = GuideFlow::new(false);
        assert_eq!(
            plain.position(),
            (1, 3),
            "reader pack is not a step without a url"
        );
        assert_eq!(plain.current(), GuideStep::Capture);
        assert!(!plain.can_go_back());
        plain.back();
        assert_eq!(plain.current(), GuideStep::Capture);
        assert_eq!(plain.advance(), Advance::Step(GuideStep::Overwatch));
        assert_eq!(plain.advance(), Advance::Step(GuideStep::Sync));
        assert!(plain.is_last());
        assert_eq!(plain.advance(), Advance::Finished);

        let mut with_pack = GuideFlow::new(true);
        assert_eq!(with_pack.position(), (1, 4));
        assert!(!reader_pack_configured(None));
        assert!(!reader_pack_configured(Some("  ")));
        assert!(reader_pack_configured(Some(
            "https://crew.example/pack.zip"
        )));
        assert_eq!(with_pack.advance(), Advance::Step(GuideStep::Overwatch));
        assert_eq!(with_pack.advance(), Advance::Step(GuideStep::Sync));
        assert_eq!(with_pack.advance(), Advance::Step(GuideStep::ReaderPack));
        assert_eq!(with_pack.advance(), Advance::Finished);
    }

    #[test]
    fn skip_closes_from_every_step_and_finish_marks_setup_done() {
        let config = Config::default();
        for include_pack in [false, true] {
            let mut guide = GuideUi::startup(true, &config, Some("0.4.24"));
            guide.flow = GuideFlow::new(include_pack);
            let steps = if include_pack { 4 } else { 3 };
            for _ in 0..steps {
                let mut skipped = GuideUi::startup(true, &config, Some("0.4.24"));
                skipped.flow = guide.flow.clone();
                let patch = skipped.skip();
                assert!(!skipped.open);
                assert!(skipped.link.is_none());
                assert!(skipped.test_frame.is_none());
                assert_eq!(patch.setup_completed, Some(true));
                assert!(patch.sync.is_none());
                guide.flow.advance();
            }
        }

        let mut guide = GuideUi::startup(true, &config, Some("0.4.24"));
        assert!(guide.continue_step().is_none(), "capture blocks continue");
        guide.test_frame = Some(MemoryFrame {
            width: 2,
            height: 1,
            preview_width: 2,
            preview_height: 1,
            rgba: vec![0, 0, 0, 255, 0, 0, 0, 255],
        });
        assert!(guide.continue_step().is_none());
        assert_eq!(guide.flow.current(), GuideStep::Overwatch);
        assert!(
            guide.continue_step().is_none(),
            "scoreboard confirm is required"
        );
        guide.tab_frame = Some(MemoryFrame {
            width: 1,
            height: 1,
            preview_width: 1,
            preview_height: 1,
            rgba: vec![1, 2, 3, 255],
        });
        guide.confirm_scoreboard();
        assert!(guide.continue_step().is_none());
        assert_eq!(guide.flow.current(), GuideStep::Sync);
        assert!(guide.continue_step().is_none());
        assert_eq!(guide.flow.current(), GuideStep::ReaderPack);
        let done = guide
            .continue_step()
            .expect("reader pack is the last step");
        assert!(!guide.open);
        assert_eq!(done.setup_completed, Some(true));
    }

    #[test]
    fn aspect_ratio_flags_anything_other_than_16_9() {
        let hd = display_geometry(1920, 1080);
        assert!(hd.is_16_9);
        assert_eq!((hd.aspect_w, hd.aspect_h), (16, 9));
        let qhd = display_geometry(2560, 1440);
        assert!(qhd.is_16_9);
        let ultrawide = display_geometry(2560, 1080);
        assert!(!ultrawide.is_16_9);
        assert!(geometry_line(&ultrawide).contains("not 16:9"));
        let laptop = display_geometry(1366, 768);
        assert!(laptop.is_16_9);
    }

    #[test]
    fn test_and_tab_frames_are_not_written() {
        let dir = tempfile::tempdir().expect("tempdir");
        let frame = MemoryFrame {
            width: 1,
            height: 1,
            preview_width: 1,
            preview_height: 1,
            rgba: vec![9, 8, 7, 255],
        };
        let kept = retain_memory_frame(frame.clone(), dir.path());
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
        assert_eq!(kept, frame);
        let img = ::image::DynamicImage::ImageRgba8(::image::ImageBuffer::from_pixel(
            4,
            4,
            ::image::Rgba([1, 2, 3, 255]),
        ));
        let shot = memory_frame_from_image(&img);
        let kept = retain_memory_frame(shot, dir.path());
        assert!(kept.width > 0 && kept.height > 0);
        assert!(std::fs::read_dir(dir.path()).unwrap().next().is_none());
    }

    #[test]
    fn poll_machine_pending_slow_down_denied_expired_and_approved_once() {
        let mut machine = LinkMachine::start(
            "dc-SECRET-9f3a-not-for-disk".into(),
            "ABCD-EFGH".into(),
            "http://127.0.0.1:9".into(),
            5,
            600,
        );
        assert_eq!(
            machine.apply(PollOutcome::Pending),
            PollDecision::Continue { interval_secs: 5 }
        );
        assert_eq!(
            machine.apply(PollOutcome::SlowDown),
            PollDecision::Continue { interval_secs: 10 }
        );
        assert_eq!(
            machine.apply(PollOutcome::SlowDown),
            PollDecision::Continue { interval_secs: 15 }
        );
        assert_eq!(machine.interval_secs(), 15);
        assert_eq!(
            machine.apply(PollOutcome::Pending),
            PollDecision::Continue { interval_secs: 15 },
            "the raised interval stays for the rest of this code"
        );

        let mut denied = LinkMachine::start(
            "code".into(),
            "CODE".into(),
            "http://127.0.0.1".into(),
            5,
            60,
        );
        assert_eq!(denied.apply(PollOutcome::Denied), PollDecision::Stop);
        assert_eq!(
            denied.apply(PollOutcome::Approved {
                token: "later".into()
            }),
            PollDecision::Stop
        );
        assert!(denied.saved_token().is_none());

        let mut expired = LinkMachine::start(
            "code".into(),
            "CODE".into(),
            "http://127.0.0.1".into(),
            5,
            60,
        );
        assert_eq!(expired.apply(PollOutcome::Expired), PollDecision::Stop);
        assert!(expired.is_terminal());

        let mut approved = LinkMachine::start(
            "dc-SECRET-9f3a-not-for-disk".into(),
            "ABCD-EFGH".into(),
            "http://127.0.0.1".into(),
            5,
            60,
        );
        assert_eq!(
            approved.apply(PollOutcome::Approved {
                token: "tok-1".into()
            }),
            PollDecision::Stop
        );
        assert_eq!(
            approved.apply(PollOutcome::Approved {
                token: "tok-2".into()
            }),
            PollDecision::Stop,
            "a second approved result is ignored"
        );
        assert_eq!(approved.saved_token(), Some("tok-1"));
        assert_eq!(
            approved.apply(PollOutcome::Expired),
            PollDecision::Stop,
            "the server poll after handover is expired and does not clear the token"
        );
        assert_eq!(approved.saved_token(), Some("tok-1"));
        let debug = format!("{approved:?}");
        assert!(!debug.contains("dc-SECRET-9f3a-not-for-disk"));
        assert!(!debug.contains("tok-1"));
        assert_eq!(format!("{}", approved.device_code), "[redacted]");
    }

    #[test]
    fn start_and_poll_bodies_match_the_device_link_contract() {
        let device = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let token = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        let started = parse_start_body(&format!(
            r#"{{"user_code":"ABCD-EFGH","device_code":"{device}","interval":5,"expires_in":600}}"#
        ))
        .expect("start");
        assert_eq!(
            started,
            DeviceLinkStartResponse {
                user_code: "ABCD-EFGH".into(),
                device_code: device.into(),
                interval: 5,
                expires_in: 600,
            }
        );
        let start_request = DeviceLinkStartRequest {
            device_label: DEVICE_LABEL.into(),
            app_version: "0.4.24".into(),
        };
        let encoded = serde_json::to_string(&start_request).expect("encode start");
        assert_eq!(
            serde_json::from_str::<DeviceLinkStartRequest>(&encoded).expect("decode start"),
            start_request
        );
        let poll_request = DeviceLinkPollRequest {
            device_code: device.into(),
        };
        let encoded = serde_json::to_string(&poll_request).expect("encode poll");
        assert_eq!(
            serde_json::from_str::<DeviceLinkPollRequest>(&encoded).expect("decode poll"),
            poll_request
        );
        assert!(
            parse_start_body(
                r#"{"user_code":"ABCD-234O","device_code":"aa","interval":5,"expires_in":600}"#
            )
            .is_err()
        );
        assert_eq!(
            parse_poll_body(r#"{"status":"pending"}"#).unwrap(),
            PollOutcome::Pending
        );
        assert_eq!(
            parse_poll_body(r#"{"status":"slow_down"}"#).unwrap(),
            PollOutcome::SlowDown
        );
        assert_eq!(
            parse_poll_body(r#"{"status":"denied"}"#).unwrap(),
            PollOutcome::Denied
        );
        assert_eq!(
            parse_poll_body(r#"{"status":"expired"}"#).unwrap(),
            PollOutcome::Expired
        );
        assert_eq!(
            parse_poll_body(&format!(r#"{{"status":"approved","token":"{token}"}}"#)).unwrap(),
            PollOutcome::Approved {
                token: token.into()
            }
        );
        assert!(parse_poll_body(r#"{"status":"approved"}"#).is_err());
        assert_eq!(
            site_error_message(
                r#"{"error":"device_label must be 1-64 characters without control characters"}"#,
                SITE_TRY_AGAIN,
            ),
            SITE_BAD_LABEL
        );
    }

    #[test]
    fn token_check_maps_name_unauthorized_and_429() {
        assert_eq!(
            map_token_check(200, r#"{"display_name":"FrozenTear"}"#, None),
            TokenCheckResult::Connected {
                display_name: "FrozenTear".into()
            }
        );
        assert_eq!(
            token_check_message(&map_token_check(
                200,
                r#"{"display_name":"FrozenTear"}"#,
                None
            )),
            "Connected as FrozenTear"
        );
        for (status, body) in [
            (401, r#"{"error":"Unauthorized"}"#),
            (403, r#"{"error":"Unauthorized"}"#),
            (500, "nope"),
            (200, r#"{"error":"Unauthorized"}"#),
            (200, r#"{"display_name":"  "}"#),
        ] {
            assert_eq!(
                map_token_check(status, body, None),
                TokenCheckResult::Rejected,
                "{status} {body}"
            );
            assert_eq!(
                token_check_message(&TokenCheckResult::Rejected),
                TOKEN_REJECTED
            );
        }
        let waited = map_token_check(429, r#"{"error":"Unauthorized"}"#, Some(8));
        assert_eq!(waited, TokenCheckResult::Wait { seconds: 8 });
        assert_eq!(token_check_message(&waited), rate_limit_message(8));
        assert_ne!(token_check_message(&waited), TOKEN_REJECTED);
        let json_header =
            map_token_check(429, r#"{"error":"rate_limited","retry_after":99}"#, Some(8));
        assert_eq!(json_header, TokenCheckResult::Wait { seconds: 8 });
        assert!(!token_check_message(&json_header).contains("99"));
        let json_body_only =
            map_token_check(429, r#"{"error":"rate_limited","retry_after":99}"#, None);
        assert_eq!(
            json_body_only,
            TokenCheckResult::Wait {
                seconds: RATE_LIMIT_FALLBACK_SECS
            }
        );
        assert!(!token_check_message(&json_body_only).contains("99"));
        let plain = map_token_check(429, "not json", None);
        assert_eq!(
            plain,
            TokenCheckResult::Wait {
                seconds: RATE_LIMIT_FALLBACK_SECS
            }
        );
        assert_eq!(
            token_check_message(&plain),
            "Too many tries, wait 10 seconds and try again"
        );
        assert!(!token_check_message(&plain).contains("not json"));
        assert!(!token_check_message(&waited).contains("Unauthorized"));
    }

    #[test]
    fn check_gate_sends_one_request_per_press() {
        let mut guide = GuideUi::startup(true, &Config::default(), Some("0.4.24"));
        guide.flow = GuideFlow::new(false);
        guide.flow.advance();
        guide.flow.advance();
        guide.sync_url = "http://127.0.0.1:9".into();
        guide.paste_token = "secret-token".into();
        assert!(guide.begin_check().is_some());
        assert!(
            guide.begin_check().is_none(),
            "a second press while the check is running does not schedule another request"
        );
        let _ = guide.token_checked(Ok(TokenCheckResult::Wait { seconds: 3 }));
        assert_eq!(
            guide.token_message.as_deref(),
            Some(rate_limit_message(3).as_str())
        );
        assert!(
            guide.begin_check().is_some(),
            "the next press can check once"
        );
    }

    #[test]
    fn device_code_never_reaches_disk_or_logs() {
        let secret = "dc-SECRET-9f3a-not-for-disk";
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer_buf = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || SharedWriter(writer_buf.clone()))
            .finish();
        tracing::subscriber::with_default(subscriber, || {
            let mut machine = LinkMachine::start(
                secret.into(),
                "ABCD-EFGH".into(),
                "http://127.0.0.1:9".into(),
                5,
                600,
            );
            let _ = machine.apply(PollOutcome::Pending);
            let _ = machine.apply(PollOutcome::SlowDown);
            let _ = machine.apply(PollOutcome::Approved {
                token: "tok-1".into(),
            });
            let _ = machine.apply(PollOutcome::Approved {
                token: "tok-2".into(),
            });
            let leaked = format!("server said {secret} in the body");
            let redacted = redact_secret(&leaked, secret);
            tracing::info!(error = %redacted, "site link failed");
            assert!(!format!("{machine:?}").contains(secret));
            assert_eq!(machine.saved_token(), Some("tok-1"));
        });
        let logs = String::from_utf8(buf.lock().expect("log").clone()).expect("utf8");
        assert!(logs.contains("pending"), "{logs}");
        assert!(logs.contains("slow_down"), "{logs}");
        assert!(logs.contains("approved"), "{logs}");
        assert!(
            !logs.contains(secret),
            "device code reached the log:\n{logs}"
        );

        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(
            &path,
            "data_dir = \"/tmp/sst-guide\"\nshadow_recognizer = true\n",
        )
        .expect("seed");
        let patch = SetupDiskPatch {
            setup_completed: Some(true),
            sync: Some(SyncConfig {
                server_url: "http://127.0.0.1:9".into(),
                token: "tok-1".into(),
            }),
        };
        Config::apply_setup_patch(&path, &patch).expect("patch");
        let disk = std::fs::read_to_string(&path).expect("read");
        assert!(!disk.contains(secret));
        assert!(disk.contains("tok-1"));
        assert!(disk.contains("shadow_recognizer = true"));
        assert!(disk.contains("setup_completed = true"));
        for entry in std::fs::read_dir(dir.path()).expect("dir") {
            let entry = entry.expect("entry");
            let text = std::fs::read_to_string(entry.path()).unwrap_or_default();
            assert!(!text.contains(secret), "{}", entry.path().display());
        }
    }

    struct SharedWriter(Arc<Mutex<Vec<u8>>>);

    impl std::io::Write for SharedWriter {
        fn write(&mut self, data: &[u8]) -> std::io::Result<usize> {
            self.0.lock().expect("log").extend_from_slice(data);
            Ok(data.len())
        }

        fn flush(&mut self) -> std::io::Result<()> {
            Ok(())
        }
    }

    fn request_complete(buf: &[u8]) -> bool {
        let text = String::from_utf8_lossy(buf);
        let Some(split) = text.find("\r\n\r\n") else {
            return false;
        };
        let headers = &text[..split];
        let body = &text[split + 4..];
        let length = headers
            .lines()
            .find_map(|line| {
                let rest = line
                    .strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))?;
                rest.trim().parse::<usize>().ok()
            })
            .unwrap_or(0);
        body.len() >= length
    }

    fn assert_no_browser_headers(raw: &str) {
        let headers = raw.split("\r\n\r\n").next().unwrap_or(raw);
        for line in headers.lines() {
            let name = line.split(':').next().unwrap_or("").trim();
            assert!(
                !name.eq_ignore_ascii_case("origin")
                    && !name.eq_ignore_ascii_case("sec-fetch-site")
                    && !name.eq_ignore_ascii_case("sec-fetch-mode")
                    && !name.eq_ignore_ascii_case("sec-fetch-dest"),
                "start and poll must not send browser headers:\n{headers}"
            );
        }
    }

    fn spawn_mock(
        handler: impl Fn(&str, &str) -> (u16, Vec<(&'static str, String)>, String)
        + Send
        + Sync
        + 'static,
    ) -> String {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handler = Arc::new(handler);
        std::thread::spawn(move || {
            for _ in 0..12 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                stream
                    .set_read_timeout(Some(Duration::from_secs(2)))
                    .expect("timeout");
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if request_complete(&buf) {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let (status, extra, body) = handler.as_ref()(&text, &text);
                let mut resp = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n",
                    body.len()
                );
                for (key, value) in extra {
                    resp.push_str(key);
                    resp.push_str(": ");
                    resp.push_str(&value);
                    resp.push_str("\r\n");
                }
                resp.push_str("\r\n");
                resp.push_str(&body);
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    #[tokio::test]
    async fn mock_server_link_flow_and_single_token_check() {
        let secret = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let handover = "0123456789abcdef0123456789abcdef0123456789abcdef0123456789abcdef";
        assert!(is_device_code(secret));
        assert!(is_device_code(handover));
        assert!(is_user_code("ABCD-EFGH"));
        let polls = Arc::new(Mutex::new(0u32));
        let checks = Arc::new(Mutex::new(0u32));
        let polls_handler = polls.clone();
        let checks_handler = checks.clone();
        let base = spawn_mock(move |head, body| {
            if head.contains("POST /api/link/start") {
                assert_no_browser_headers(head);
                let json = body.split("\r\n\r\n").nth(1).unwrap_or("");
                let request: DeviceLinkStartRequest =
                    serde_json::from_str(json).expect("shared start request");
                assert_eq!(request.device_label, DEVICE_LABEL);
                assert_eq!(request.app_version, "0.4.24");
                assert!(!body.contains(secret));
                return (
                    200,
                    vec![],
                    format!(
                        r#"{{"user_code":"ABCD-EFGH","device_code":"{secret}","interval":5,"expires_in":600}}"#
                    ),
                );
            }
            if head.contains("POST /api/link/poll") {
                assert_no_browser_headers(head);
                let json = body.split("\r\n\r\n").nth(1).unwrap_or("");
                let request: DeviceLinkPollRequest =
                    serde_json::from_str(json).expect("shared poll request");
                assert_eq!(request.device_code, secret);
                let mut n = polls_handler.lock().expect("polls");
                *n += 1;
                let body = match *n {
                    1 => r#"{"status":"pending"}"#.to_string(),
                    2 => r#"{"status":"slow_down"}"#.to_string(),
                    3 => format!(r#"{{"status":"approved","token":"{handover}"}}"#),
                    _ => r#"{"status":"expired"}"#.to_string(),
                };
                return (200, vec![], body);
            }
            if head.contains("GET /api/stats/token-check") {
                let mut n = checks_handler.lock().expect("checks");
                *n += 1;
                if *n == 1 {
                    return (
                        429,
                        vec![("Retry-After", "4".into())],
                        r#"{"error":"Unauthorized"}"#.to_string(),
                    );
                }
                return (200, vec![], r#"{"display_name":"FrozenTear"}"#.to_string());
            }
            (404, vec![], r#"{"error":"Unauthorized"}"#.to_string())
        });

        let started = match start_link(base.clone(), DEVICE_LABEL.into(), "0.4.24".into())
            .await
            .expect("start")
        {
            LinkStartOutcome::Ready(start) => start,
            LinkStartOutcome::Unsupported => panic!("this mock implements link start"),
        };
        assert_eq!(started.machine.user_code(), "ABCD-EFGH");
        assert_eq!(started.machine.interval_secs(), 5);
        assert_eq!(started.machine.device_code_for_request(), secret);
        assert!(!format!("{:?}", started.machine).contains(secret));

        let mut machine = started.machine;
        let first = expect_outcome(
            poll_link(base.clone(), machine.device_code_for_request().to_string())
                .await
                .expect("pending"),
        );
        assert_eq!(
            machine.apply(first),
            PollDecision::Continue { interval_secs: 5 }
        );
        let second = expect_outcome(
            poll_link(base.clone(), machine.device_code_for_request().to_string())
                .await
                .expect("slow"),
        );
        assert_eq!(
            machine.apply(second),
            PollDecision::Continue { interval_secs: 10 }
        );
        assert_eq!(machine.interval_secs(), 10);
        let third = expect_outcome(
            poll_link(base.clone(), machine.device_code_for_request().to_string())
                .await
                .expect("approved"),
        );
        assert_eq!(machine.apply(third), PollDecision::Stop);
        assert_eq!(machine.saved_token(), Some(handover));
        let again = expect_outcome(
            poll_link(base.clone(), machine.device_code_for_request().to_string())
                .await
                .expect("expired after handover"),
        );
        assert_eq!(again, PollOutcome::Expired);
        assert_eq!(machine.apply(again), PollDecision::Stop);
        assert_eq!(machine.saved_token(), Some(handover));
        assert_eq!(
            machine.apply(PollOutcome::Approved {
                token: "tok-2".into()
            }),
            PollDecision::Stop
        );
        assert_eq!(machine.saved_token(), Some(handover));

        let limited = check_token(base.clone(), "tok-1".into())
            .await
            .expect("429");
        assert_eq!(limited, TokenCheckResult::Wait { seconds: 4 });
        assert_eq!(token_check_message(&limited), rate_limit_message(4));
        assert_ne!(token_check_message(&limited), TOKEN_REJECTED);
        let ok = check_token(base, "tok-1".into()).await.expect("200");
        assert_eq!(token_check_message(&ok), "Connected as FrozenTear");
        assert_eq!(*checks.lock().expect("checks"), 2);
    }

    #[tokio::test]
    async fn missing_routes_save_the_token_and_hide_site_sign_in() {
        let base = spawn_mock(|head, _body| {
            if head.contains("POST /api/link/start") || head.contains("GET /api/stats/token-check")
            {
                return (404, vec![], r#"{"error":"Unauthorized"}"#.to_string());
            }
            (500, vec![], "nope".to_string())
        });

        let started = start_link(base.clone(), DEVICE_LABEL.into(), "0.4.24".into())
            .await
            .expect("404 is a result, not a transport error");
        assert!(matches!(started, LinkStartOutcome::Unsupported));

        let mut guide = GuideUi::startup(true, &Config::default(), Some("0.4.24"));
        guide.sync_url = base.clone();
        guide.paste_token = "tok-1".into();
        assert!(sign_in_offered(&guide));
        guide.link_started(Ok(LinkStartOutcome::Unsupported));
        assert!(guide.link_unsupported);
        assert!(guide.paste_open);
        assert!(!guide.link_busy);
        assert!(guide.link.is_none());
        assert!(!sign_in_offered(&guide));
        assert!(guide.begin_sign_in().is_none());
        assert_eq!(guide.link_message.as_deref(), Some(LINK_UNSUPPORTED));
        assert_ne!(guide.link_message.as_deref(), Some(TOKEN_REJECTED));

        let checked = check_token(base, "tok-1".into()).await.expect("404 check");
        assert_eq!(checked, TokenCheckResult::Unchecked);
        assert_eq!(
            map_token_check(404, r#"{"error":"Unauthorized"}"#, None),
            TokenCheckResult::Unchecked
        );
        assert_eq!(token_check_message(&checked), TOKEN_UNCHECKED);
        assert_ne!(token_check_message(&checked), TOKEN_REJECTED);
        let patch = guide.token_checked(Ok(checked)).expect("token is saved");
        let sync = patch.sync.expect("sync");
        assert_eq!(sync.token, "tok-1");
        assert_eq!(sync.server_url, guide.sync_url);
        assert_eq!(guide.token_message.as_deref(), Some(TOKEN_UNCHECKED));

        guide.set_sync_url(format!("{}/other", guide.sync_url.trim_end_matches('/')));
        assert!(sign_in_offered(&guide));
    }

    fn expect_outcome(update: PollUpdate) -> PollOutcome {
        match update {
            PollUpdate::Outcome(outcome) => outcome,
            PollUpdate::RateLimited { seconds } => panic!("rate limited for {seconds}s"),
        }
    }

    #[tokio::test]
    async fn rate_limit_ignores_plain_text_and_json_bodies() {
        let hits = Arc::new(Mutex::new(Vec::<String>::new()));
        let hits_handler = hits.clone();
        let base = spawn_mock(move |head, _body| {
            let kind = if head.contains("POST /api/link/start") {
                "start"
            } else if head.contains("POST /api/link/poll") {
                "poll"
            } else if head.contains("GET /api/stats/token-check") {
                "check"
            } else {
                "other"
            };
            let mut log = hits_handler.lock().expect("hits");
            let seen = log.iter().filter(|item| item.as_str() == kind).count();
            log.push(kind.to_string());
            drop(log);
            let link_json = r#"{"error":"rate_limited","retry_after":99}"#;
            match (kind, seen) {
                ("start", 0) => (
                    429,
                    vec![("Retry-After", "4".into())],
                    "plain text not json".into(),
                ),
                ("start", 1) => (429, vec![], link_json.into()),
                ("start", _) => (429, vec![("Retry-After", "7".into())], link_json.into()),
                ("poll", 0) => (
                    429,
                    vec![("Retry-After", "6".into())],
                    "dc-SECRET-9f3a-not-for-disk".into(),
                ),
                ("poll", 1) => (429, vec![], link_json.into()),
                ("poll", _) => (429, vec![("Retry-After", "2".into())], link_json.into()),
                ("check", 0) => (429, vec![("Retry-After", "3".into())], "hold on".into()),
                ("check", 1) => (429, vec![], link_json.into()),
                ("check", _) => (429, vec![("Retry-After", "8".into())], link_json.into()),
                _ => (500, vec![], "nope".into()),
            }
        });

        let start_plain = start_link(base.clone(), DEVICE_LABEL.into(), "0.4.24".into())
            .await
            .expect_err("plain 429");
        assert_eq!(start_plain, rate_limit_message(4));
        assert!(!start_plain.contains("plain text"));

        let start_json = start_link(base.clone(), DEVICE_LABEL.into(), "0.4.24".into())
            .await
            .expect_err("json 429");
        assert_eq!(start_json, rate_limit_message(RATE_LIMIT_FALLBACK_SECS));
        assert!(!start_json.contains("rate_limited"));
        assert!(!start_json.contains("99"));
        let start_header = start_link(base.clone(), DEVICE_LABEL.into(), "0.4.24".into())
            .await
            .expect_err("json 429 with header");
        assert_eq!(start_header, rate_limit_message(7));
        assert!(!start_header.contains("99"));

        let secret = "dc-SECRET-9f3a-not-for-disk";
        let poll_plain = poll_link(base.clone(), secret.into())
            .await
            .expect("plain poll 429");
        assert_eq!(poll_plain, PollUpdate::RateLimited { seconds: 6 });
        let poll_json = poll_link(base.clone(), secret.into())
            .await
            .expect("json poll 429");
        assert_eq!(
            poll_json,
            PollUpdate::RateLimited {
                seconds: RATE_LIMIT_FALLBACK_SECS
            }
        );
        let poll_header = poll_link(base.clone(), secret.into())
            .await
            .expect("json poll 429 with header");
        assert_eq!(poll_header, PollUpdate::RateLimited { seconds: 2 });

        let mut guide = GuideUi::startup(true, &Config::default(), Some("0.4.24"));
        guide.link = Some(LinkMachine::start(
            secret.into(),
            "ABCD-EFGH".into(),
            base.clone(),
            5,
            600,
        ));
        let before = Instant::now();
        assert!(guide.poll_ready(Ok(poll_plain)).is_none());
        assert_eq!(
            guide.link_message.as_deref(),
            Some(rate_limit_message(6).as_str())
        );
        assert!(!guide.link_message.as_deref().unwrap_or("").contains(secret));
        assert!(!guide.link_message.as_deref().unwrap_or("").contains("99"));
        assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 5);
        let scheduled = guide.next_poll_at.expect("next poll");
        let wait = scheduled.saturating_duration_since(before);
        assert!(wait >= Duration::from_secs(6), "{wait:?}");
        assert!(wait < Duration::from_secs(7), "{wait:?}");
        assert!(
            guide
                .poll_request_if_due(scheduled - Duration::from_millis(1))
                .is_none()
        );
        assert!(guide.poll_request_if_due(scheduled).is_some());
        guide.poll_inflight = false;
        let before = Instant::now();
        assert!(guide.poll_ready(Ok(poll_header)).is_none());
        assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 5);
        assert_eq!(
            guide.link_message.as_deref(),
            Some("Too many tries, wait 2 seconds and try again")
        );
        assert!(!guide.link_message.as_deref().unwrap_or("").contains("99"));
        let wait = guide
            .next_poll_at
            .expect("next poll")
            .saturating_duration_since(before);
        assert!(wait >= Duration::from_secs(2), "{wait:?}");
        assert!(wait < Duration::from_secs(3), "{wait:?}");

        let check_plain = check_token(base.clone(), "tok-1".into())
            .await
            .expect("plain check 429");
        assert_eq!(check_plain, TokenCheckResult::Wait { seconds: 3 });
        assert_eq!(token_check_message(&check_plain), rate_limit_message(3));
        assert!(!token_check_message(&check_plain).contains("hold"));

        let check_json = check_token(base.clone(), "tok-1".into())
            .await
            .expect("json check 429");
        assert_eq!(
            check_json,
            TokenCheckResult::Wait {
                seconds: RATE_LIMIT_FALLBACK_SECS
            }
        );
        assert_eq!(
            token_check_message(&check_json),
            "Too many tries, wait 10 seconds and try again"
        );
        assert!(!token_check_message(&check_json).contains("99"));
        assert!(!token_check_message(&check_json).contains("rate_limited"));
        let check_header = check_token(base, "tok-1".into())
            .await
            .expect("json check 429 with header");
        assert_eq!(check_header, TokenCheckResult::Wait { seconds: 8 });
        assert!(!token_check_message(&check_header).contains("99"));
        guide.paste_token = "tok-1".into();
        guide.sync_url = guide
            .link
            .as_ref()
            .expect("machine")
            .server_url()
            .to_string();
        assert!(guide.token_checked(Ok(check_json)).is_none());
    }

    #[test]
    fn slow_down_raises_the_interval_twice_and_a_429_leaves_it() {
        let mut guide = GuideUi::startup(true, &Config::default(), Some("0.4.24"));
        guide.link = Some(LinkMachine::start(
            "dc-SECRET-9f3a-not-for-disk".into(),
            "ABCD-EFGH".into(),
            "http://127.0.0.1:9".into(),
            5,
            600,
        ));

        let before = Instant::now();
        assert!(
            guide
                .poll_ready(Ok(PollUpdate::Outcome(PollOutcome::SlowDown)))
                .is_none()
        );
        assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 10);
        let wait = guide
            .next_poll_at
            .expect("next poll")
            .saturating_duration_since(before);
        assert!(wait >= Duration::from_secs(10), "{wait:?}");
        assert!(wait < Duration::from_secs(11), "{wait:?}");

        let before = Instant::now();
        assert!(
            guide
                .poll_ready(Ok(PollUpdate::Outcome(PollOutcome::SlowDown)))
                .is_none()
        );
        assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 15);
        let wait = guide
            .next_poll_at
            .expect("next poll")
            .saturating_duration_since(before);
        assert!(wait >= Duration::from_secs(15), "{wait:?}");
        assert!(wait < Duration::from_secs(16), "{wait:?}");

        let before = Instant::now();
        assert!(
            guide
                .poll_ready(Ok(PollUpdate::Outcome(PollOutcome::Pending)))
                .is_none()
        );
        assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 15);
        let wait = guide
            .next_poll_at
            .expect("next poll")
            .saturating_duration_since(before);
        assert!(wait >= Duration::from_secs(15), "{wait:?}");
        assert!(wait < Duration::from_secs(16), "{wait:?}");

        let before = Instant::now();
        assert!(
            guide
                .poll_ready(Ok(PollUpdate::RateLimited { seconds: 6 }))
                .is_none()
        );
        assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 15);
        assert_eq!(
            guide.link_message.as_deref(),
            Some(rate_limit_message(6).as_str())
        );
        let wait = guide
            .next_poll_at
            .expect("next poll")
            .saturating_duration_since(before);
        assert!(wait >= Duration::from_secs(6), "{wait:?}");
        assert!(wait < Duration::from_secs(7), "{wait:?}");

        let before = Instant::now();
        assert!(
            guide
                .poll_ready(Ok(PollUpdate::RateLimited {
                    seconds: RATE_LIMIT_FALLBACK_SECS
                }))
                .is_none()
        );
        assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 15);
        assert_eq!(
            guide.link_message.as_deref(),
            Some("Too many tries, wait 10 seconds and try again")
        );
        let wait = guide
            .next_poll_at
            .expect("next poll")
            .saturating_duration_since(before);
        assert!(wait >= Duration::from_secs(10), "{wait:?}");
        assert!(wait < Duration::from_secs(11), "{wait:?}");

        let fresh = LinkMachine::start(
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".into(),
            "WXYZ-2345".into(),
            "http://127.0.0.1:9".into(),
            5,
            600,
        );
        assert_eq!(fresh.interval_secs(), 5);
    }

    #[test]
    fn retry_after_is_clamped_before_it_is_added_to_the_clock() {
        assert_eq!(clamp_retry_after(0), 1);
        assert_eq!(clamp_retry_after(3601), 3600);
        assert_eq!(clamp_retry_after(u64::MAX), 3600);
        assert_eq!(rate_limit_seconds(Some(0)), 1);
        assert_eq!(rate_limit_seconds(Some(3601)), 3600);
        assert_eq!(rate_limit_seconds(Some(u64::MAX)), 3600);

        let mut guide = GuideUi::startup(true, &Config::default(), Some("0.4.24"));
        guide.link = Some(LinkMachine::start(
            "dc-SECRET-9f3a-not-for-disk".into(),
            "ABCD-EFGH".into(),
            "http://127.0.0.1:9".into(),
            5,
            600,
        ));
        for (raw, expected) in [(0u64, 1u64), (3601, 3600), (u64::MAX, 3600)] {
            let before = Instant::now();
            assert!(
                guide
                    .poll_ready(Ok(PollUpdate::RateLimited { seconds: raw }))
                    .is_none()
            );
            let wait = guide
                .next_poll_at
                .expect("next poll")
                .saturating_duration_since(before);
            assert!(wait >= Duration::from_secs(expected), "{raw} {wait:?}");
            assert!(wait < Duration::from_secs(expected + 1), "{raw} {wait:?}");
            assert_eq!(
                guide.link_message.as_deref(),
                Some(rate_limit_message(expected).as_str())
            );
            assert_eq!(guide.link.as_ref().expect("machine").interval_secs(), 5);
        }
    }

    #[test]
    fn sign_in_is_reachable_after_a_capture_failure() {
        let mut guide = GuideUi::startup(true, &Config::default(), Some("0.4.24"));
        assert!(!guide.can_continue());
        guide.capture_ready(Err(CAPTURE_FAILED.into()));
        assert!(guide.can_continue());
        assert_eq!(guide.capture_warning(), Some(CAPTURE_CONTINUE_WARN));
        assert!(guide.continue_step().is_none());
        assert_eq!(guide.flow.current(), GuideStep::Overwatch);
        assert!(guide.skip_step().is_none());
        assert_eq!(guide.flow.current(), GuideStep::Sync);
        assert!(sign_in_offered(&guide));
        guide.sync_url = "http://127.0.0.1:9".into();
        assert!(guide.begin_sign_in().is_some());
    }

    #[test]
    fn guide_keys_skip_on_escape_and_continue_on_enter() {
        use iced::keyboard::Key;
        use iced::keyboard::key::Named;
        assert!(matches!(
            guide_key_action(&Key::Named(Named::Escape)),
            Some(SetupMessage::Skip)
        ));
        assert!(matches!(
            guide_key_action(&Key::Named(Named::Enter)),
            Some(SetupMessage::Next)
        ));
        assert!(guide_key_action(&Key::Named(Named::Tab)).is_none());
    }

    #[test]
    fn server_errors_shown_to_members_are_plain_sentences() {
        let secret = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let token = "tok-SECRET-value";
        let body =
            format!(r#"{{"error":"invalid code","device_code":"{secret}","token":"{token}"}}"#);
        let buf = Arc::new(Mutex::new(Vec::<u8>::new()));
        let writer_buf = buf.clone();
        let subscriber = tracing_subscriber::fmt()
            .with_max_level(tracing::Level::TRACE)
            .with_ansi(false)
            .with_writer(move || SharedWriter(writer_buf.clone()))
            .finish();
        let shown = tracing::subscriber::with_default(subscriber, || {
            site_error_message(&body, SITE_TRY_AGAIN)
        });
        assert_eq!(shown, SITE_INVALID_CODE);
        assert!(!shown.contains(secret));
        assert!(!shown.contains(token));
        assert!(!shown.contains("invalid code"));
        let logs = String::from_utf8(buf.lock().expect("log").clone()).expect("utf8");
        assert!(logs.contains("invalid code"), "{logs}");
        assert!(!logs.contains(secret), "{logs}");
        assert!(!logs.contains(token), "{logs}");
    }

    #[test]
    fn debug_redacts_tokens_and_device_codes() {
        let token = "tok-SECRET-value";
        let secret = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
        let outcome = PollOutcome::Approved {
            token: token.into(),
        };
        let outcome_debug = format!("{outcome:?}");
        assert!(!outcome_debug.contains(token));
        assert!(outcome_debug.contains("[redacted]"));

        let paste = SetupMessage::PasteToken(token.into());
        assert!(!format!("{paste:?}").contains(token));

        let poll = SetupMessage::PollReady(Ok(PollUpdate::Outcome(PollOutcome::Approved {
            token: token.into(),
        })));
        assert!(!format!("{poll:?}").contains(token));

        let machine = LinkMachine::start(
            secret.into(),
            "ABCD-EFGH".into(),
            "http://127.0.0.1".into(),
            5,
            60,
        );
        let started = SetupMessage::LinkStarted(Ok(LinkStartOutcome::Ready(LinkStart { machine })));
        let started_debug = format!("{started:?}");
        assert!(!started_debug.contains(secret), "{started_debug}");
        assert!(started_debug.contains("[redacted]"), "{started_debug}");
    }
}
