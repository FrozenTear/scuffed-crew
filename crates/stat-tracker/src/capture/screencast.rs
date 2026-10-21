//! One long-lived xdg-desktop-portal ScreenCast session.
//!
//! The cursor mode is Hidden. Metadata is never requested: that is the
//! KWin crash when a screenshot and a voice-app share run together. The
//! restore token is stored in the data dir so the prompt is once per
//! machine, not once per boot. A backend that rejects the token is asked
//! again without it. If the session cannot start, or a grab sees no frame
//! within [`SCREENCAST_FRAME_WAIT`], the screenshot portal is used and a
//! warning is logged. A failed start is not retried for [`START_BACKOFF`].

use std::path::{Path, PathBuf};
use std::sync::LazyLock;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream,
};
use ashpd::desktop::{PersistMode, Session};
use image::DynamicImage;

use super::CaptureKind;
use super::framebuf;
use super::portal::{self, OutputRect};
use super::portal_backend;
use super::pw;

/// How long a grab waits after the session is up. The permission dialog
/// is not on this timer.
pub const SCREENCAST_FRAME_WAIT: Duration = Duration::from_secs(2);

/// After a failed start or a grab with no frame, use the screenshot portal
/// until this elapses, then try the stream again.
pub const START_BACKOFF: Duration = Duration::from_secs(30);

const TOKEN_FILE: &str = "screencast-restore-token";
const COMPARE_LIMIT: u32 = 4;

struct Live {
    _fd: std::os::fd::OwnedFd,
    stream: std::sync::Arc<pw::Handle>,
    /// Dropping this asks the task that owns the portal session to call
    /// `Session::close`. Drop alone does not close it.
    close_session: Option<tokio::sync::oneshot::Sender<()>>,
}

impl Drop for Live {
    fn drop(&mut self) {
        if let Some(tx) = self.close_session.take() {
            let _ = tx.send(());
        }
    }
}

static SESSION_OPEN: AtomicBool = AtomicBool::new(false);

pub fn session_is_open() -> bool {
    SESSION_OPEN.load(Ordering::Acquire)
}

struct CastState {
    live: Option<Live>,
    /// Bumped when an open starts and when the share is closed. An open
    /// that finishes for an older generation drops its session.
    open_gen: u64,
    /// True while `open_session` is in progress. The permission dialog runs
    /// in that task, not under this mutex.
    opening: bool,
    last_fail: Option<Instant>,
    warned: bool,
    compare_saved: u32,
    compare_warned: bool,
    last_timing_log: Instant,
}

impl Default for CastState {
    fn default() -> Self {
        Self {
            live: None,
            open_gen: 0,
            opening: false,
            last_fail: None,
            warned: false,
            compare_saved: 0,
            compare_warned: false,
            last_timing_log: Instant::now(),
        }
    }
}

static CAST: LazyLock<tokio::sync::Mutex<CastState>> =
    LazyLock::new(|| tokio::sync::Mutex::new(CastState::default()));

/// True when a new session attempt is allowed. `age` is how long ago the
/// last failure was.
pub fn should_try_stream(last_fail_age: Option<Duration>, backoff: Duration) -> bool {
    match last_fail_age {
        None => true,
        Some(age) => age >= backoff,
    }
}

/// True on the running to not-running edge, and only when this process is
/// on the screen stream and a restore token can reopen it without a prompt.
/// That includes a forced `screencast` or Settings stream on niri, not only
/// the KDE or GNOME default. Without a token the share stays up, so the
/// next game does not ask again.
pub fn should_close_share(
    stream_route: bool,
    was_running: bool,
    running: bool,
    restorable: bool,
) -> bool {
    stream_route && was_running && !running && restorable
}

/// True when a restore token is on disk. The bytes are not logged.
pub fn restore_token_saved(data_dir: &Path) -> bool {
    read_token(data_dir).is_some()
}

/// A deliberate close is not a failed start. The failure clock is left as
/// it was, which is unset while a session was up, so the next capture
/// opens immediately.
pub fn last_fail_after_deliberate_close(last_fail: Option<Duration>) -> Option<Duration> {
    last_fail
}

/// What a second caller may do while a session open is in progress.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenPermit {
    Open,
    Reuse,
    Busy,
}

pub fn open_permit(opening: bool, live: bool) -> OpenPermit {
    if opening {
        OpenPermit::Busy
    } else if live {
        OpenPermit::Reuse
    } else {
        OpenPermit::Open
    }
}

/// A portal session that was created and then failed or timed out must be
/// closed. A success hands the session to the hold task.
pub fn must_close_session(created: bool, succeeded: bool) -> bool {
    created && !succeeded
}

/// How long a later grab waits for a new buffer before reusing the last
/// converted frame. One poll converts at most one frame. The new-reader
/// poll is 1.5 s, so this stays well under that.
pub fn reuse_wait(max_fps: u32) -> Duration {
    Duration::from_millis(pw::throttle_gap_ms(max_fps).min(400))
}

/// What a Tab stream timeout does. KDE/KWin must not open the screenshot
/// portal while a share is up. niri and other wlroots compositors can, and
/// so can GNOME when the stream was forced rather than chosen automatically.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabTimeoutAction {
    Skip,
    Screenshot,
}

pub fn is_user_cancel(err: &str) -> bool {
    let lower = err.to_ascii_lowercase();
    lower.contains("cancel") || lower.contains("dismiss")
}

fn desktop_has(desktop: &str, names: &[&str]) -> bool {
    desktop.split(':').any(|token| {
        let token = token.trim().to_ascii_lowercase();
        names
            .iter()
            .any(|name| token == *name || token.contains(name))
    })
}

pub fn screenshot_fallback_is_safe(desktop: &str, stream_forced: bool) -> bool {
    if desktop_has(desktop, &["kde", "plasma", "kwin"]) {
        return false;
    }
    if desktop_has(
        desktop,
        &[
            "niri", "sway", "hyprland", "wayfire", "river", "labwc", "wlroots",
        ],
    ) {
        return true;
    }
    desktop_has(desktop, &["gnome"]) && stream_forced
}

pub fn tab_timeout_action(desktop: &str, stream_forced: bool) -> TabTimeoutAction {
    if screenshot_fallback_is_safe(desktop, stream_forced) {
        TabTimeoutAction::Screenshot
    } else {
        TabTimeoutAction::Skip
    }
}

static TAB_MISS_COUNT: AtomicU64 = AtomicU64::new(0);
static TAB_SHOT_COUNT: AtomicU64 = AtomicU64::new(0);

/// `(misses, screenshot fallbacks)` since this process started.
#[cfg_attr(not(test), allow(dead_code))]
pub fn tab_timeout_counts() -> (u64, u64) {
    (
        TAB_MISS_COUNT.load(Ordering::Relaxed),
        TAB_SHOT_COUNT.load(Ordering::Relaxed),
    )
}

/// Close the live session when one exists and this process is on the stream
/// route. A no-op otherwise. Does not start the failure backoff.
pub async fn close_if_open() {
    if !super::stream_route_active() {
        return;
    }
    let mut state = CAST.lock().await;
    let had = state.live.is_some() || state.opening;
    state.open_gen = state.open_gen.wrapping_add(1);
    state.opening = false;
    let live = state.live.take();
    let _ = last_fail_after_deliberate_close(state.last_fail.map(|at| at.elapsed()));
    drop(state);
    drop(live);
    SESSION_OPEN.store(false, Ordering::Release);
    if had {
        tracing::info!("screencast session closed");
    }
}

pub async fn capture(
    output_name: Option<&str>,
    kind: CaptureKind,
    data_dir: &Path,
    debug_frames: bool,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let max_fps = super::prefs().max_fps.max(1);
    let mut state = CAST.lock().await;
    let age = state.last_fail.map(|at| at.elapsed());
    if !should_try_stream(age, START_BACKOFF) {
        drop(state);
        return screenshot(output_name, "screencast backoff").await;
    }
    let restart = match open_permit(state.opening, state.live.is_some()) {
        OpenPermit::Busy => {
            drop(state);
            return while_opening(output_name).await;
        }
        OpenPermit::Reuse => state.live.as_ref().is_some_and(|live| !live.stream.alive()),
        OpenPermit::Open => true,
    };
    if restart {
        // Drop a dead session before asking for another, so the portal is
        // not holding two screen shares. The dialog runs in its own task.
        // This grab does not wait on it, and a second grab sees `opening`.
        state.live.take();
        SESSION_OPEN.store(false, Ordering::Release);
        state.open_gen = state.open_gen.wrapping_add(1);
        let generation = state.open_gen;
        state.opening = true;
        drop(state);
        let data_dir = data_dir.to_path_buf();
        let output = output_name.map(str::to_string);
        tokio::spawn(async move {
            let result = open_session(&data_dir, output.as_deref(), max_fps).await;
            let mut state = CAST.lock().await;
            if state.open_gen != generation {
                drop(result);
                return;
            }
            state.opening = false;
            match result {
                Ok(live) => {
                    if state.live.is_some() {
                        drop(live);
                    } else {
                        state.live = Some(live);
                        state.last_fail = None;
                        SESSION_OPEN.store(true, Ordering::Release);
                    }
                }
                Err(err) => {
                    state.last_fail = Some(Instant::now());
                    state.live = None;
                    SESSION_OPEN.store(false, Ordering::Release);
                    note_fallback(
                        &mut state,
                        &format!(
                            "screencast session failed ({err}); falling back to the screenshot portal"
                        ),
                    );
                }
            }
        });
        return while_opening(output_name).await;
    }
    let Some(stream) = state
        .live
        .as_ref()
        .map(|live| std::sync::Arc::clone(&live.stream))
    else {
        state.last_fail = Some(Instant::now());
        note_fallback(
            &mut state,
            "screencast stream is not running; falling back to the screenshot portal",
        );
        drop(state);
        return portal::capture(output_name).await;
    };
    if !stream.alive() {
        state.last_fail = Some(Instant::now());
        state.live = None;
        SESSION_OPEN.store(false, Ordering::Release);
        note_fallback(
            &mut state,
            "screencast stream is not running; falling back to the screenshot portal",
        );
        drop(state);
        return portal::capture(output_name).await;
    }
    let tab_press = match kind {
        CaptureKind::Tab => Some(super::tab_press().unwrap_or_else(Instant::now)),
        CaptureKind::Poll => None,
    };
    let fresh = stream.needs_fresh_frame();
    let timeout = if tab_press.is_some() {
        Duration::from_millis(framebuf::TAB_GRAB_TIMEOUT_AFTER_PRESS_MS)
    } else if fresh {
        SCREENCAST_FRAME_WAIT
    } else {
        reuse_wait(max_fps)
    };
    // The mutex stays held so a second caller cannot open another share.
    let grabbed = stream.grab(timeout, tab_press).await;
    let timing = stream.timing();
    let alive = stream.alive();
    match grabbed {
        Ok(pw::GrabOutcome::TabMiss { delay_ms }) => {
            drop(state);
            tab_timeout(output_name, delay_ms).await
        }
        Ok(pw::GrabOutcome::Frame(frame)) => {
            state.last_fail = None;
            state.warned = false;
            let grab_ms = timing.avg_grab_ms;
            let log_summary = state.last_timing_log.elapsed() >= Duration::from_secs(60);
            if log_summary {
                state.last_timing_log = Instant::now();
            }
            let compare =
                debug_frames && kind == CaptureKind::Tab && state.compare_saved < COMPARE_LIMIT;
            let index = state.compare_saved;
            if compare {
                state.compare_saved += 1;
                if !state.compare_warned {
                    state.compare_warned = true;
                    tracing::warn!(
                        "debug portal frames also takes a screenshot; a voice app sharing the screen can hit the KWin cursor crash while this is on"
                    );
                }
            }
            drop(state);
            tracing::debug!(
                grab_ms,
                frames_dropped = timing.frames_dropped,
                "screencast grab"
            );
            if log_summary {
                tracing::info!("{}", pw::timing_line(&timing));
            }
            let img = framebuf::to_dynamic(&frame).ok_or("screencast frame format is not RGB")?;
            let img = fit_stream(img, output_name).await?;
            if compare {
                save_compare(data_dir, index, &img, output_name).await;
            }
            Ok(img)
        }
        Err(err) => {
            if !alive {
                state.last_fail = Some(Instant::now());
                state.live = None;
                SESSION_OPEN.store(false, Ordering::Release);
                note_fallback(
                    &mut state,
                    &format!(
                        "screencast stream ended ({err}); falling back to the screenshot portal"
                    ),
                );
                drop(state);
                return portal::capture(output_name).await;
            }
            drop(state);
            Err(err.into())
        }
    }
}

async fn tab_timeout(
    output_name: Option<&str>,
    delay_ms: u64,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    match tab_timeout_action(&desktop, super::stream_was_forced()) {
        TabTimeoutAction::Screenshot => {
            let count = TAB_SHOT_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::info!(
                delay_ms,
                count,
                "Tab grab: stream timeout, screenshot fallback"
            );
            portal::capture(output_name).await
        }
        TabTimeoutAction::Skip => {
            let count = TAB_MISS_COUNT.fetch_add(1, Ordering::Relaxed) + 1;
            tracing::info!(count, "Tab grab: no fresh stream frame (timeout)");
            Err("no fresh stream frame".into())
        }
    }
}

fn note_fallback(state: &mut CastState, message: &str) {
    if state.warned {
        tracing::debug!("{message}");
    } else {
        tracing::warn!("{message}");
        state.warned = true;
    }
}

/// A grab that arrived while the permission dialog is still up. KDE must
/// not open the screenshot portal beside a share that is about to start.
async fn while_opening(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if screenshot_fallback_is_safe(&desktop, super::stream_was_forced()) {
        return screenshot(output_name, "screencast open in progress").await;
    }
    Err("screencast open in progress".into())
}

async fn screenshot(
    output_name: Option<&str>,
    why: &str,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    tracing::debug!(why, "screencast skipped; using the screenshot portal");
    portal::capture(output_name).await
}

enum OpenFail {
    Select(String),
    Other(String),
}

impl std::fmt::Display for OpenFail {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            OpenFail::Select(msg) | OpenFail::Other(msg) => f.write_str(msg),
        }
    }
}

async fn open_session(
    data_dir: &Path,
    output_name: Option<&str>,
    max_fps: u32,
) -> Result<Live, String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    portal_backend::log_answered(&desktop);
    let token = read_token(data_dir);
    match open_once(
        data_dir,
        output_name,
        token.as_deref(),
        PersistMode::ExplicitlyRevoked,
        max_fps,
    )
    .await
    {
        Ok(live) => Ok(live),
        Err(OpenFail::Select(_)) if token.is_some() => {
            tracing::info!(
                "portal backend did not accept a persistent restore token; asking again without it"
            );
            let _ = std::fs::remove_file(token_path(data_dir));
            match open_once(
                data_dir,
                output_name,
                None,
                PersistMode::ExplicitlyRevoked,
                max_fps,
            )
            .await
            {
                Ok(live) => Ok(live),
                Err(err) => Err(err.to_string()),
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

struct SessionClose {
    session: Option<Session<Screencast>>,
}

impl SessionClose {
    fn arm(session: Session<Screencast>) -> Self {
        Self {
            session: Some(session),
        }
    }

    fn session(&self) -> &Session<Screencast> {
        self.session.as_ref().expect("portal session")
    }

    async fn fail(mut self, err: OpenFail) -> OpenFail {
        if must_close_session(self.session.is_some(), false)
            && let Some(session) = self.session.take()
        {
            let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
        }
        err
    }

    fn disarm(mut self) -> Session<Screencast> {
        self.session.take().expect("portal session")
    }
}

impl Drop for SessionClose {
    fn drop(&mut self) {
        if let Some(session) = self.session.take() {
            tokio::spawn(async move {
                let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
            });
        }
    }
}

async fn open_once(
    data_dir: &Path,
    output_name: Option<&str>,
    token: Option<&str>,
    persist: PersistMode,
    max_fps: u32,
) -> Result<Live, OpenFail> {
    let proxy = Screencast::new()
        .await
        .map_err(|err| OpenFail::Other(err.to_string()))?;
    match proxy.available_cursor_modes().await {
        Ok(modes) if !modes.contains(CursorMode::Hidden) => {
            return Err(OpenFail::Other(
                "portal does not offer a hidden cursor, so cursor metadata was not requested"
                    .into(),
            ));
        }
        Ok(_) => {}
        Err(err) => {
            tracing::debug!(error = %err, "portal did not report cursor modes; asking for hidden");
        }
    }
    let session = match proxy.create_session(Default::default()).await {
        Ok(session) => session,
        Err(err) => return Err(OpenFail::Other(err.to_string())),
    };
    let guard = SessionClose::arm(session);
    let mut options = SelectSourcesOptions::default()
        .set_sources(enumflags2::BitFlags::from(SourceType::Monitor))
        .set_multiple(false)
        .set_cursor_mode(CursorMode::Hidden)
        .set_persist_mode(persist);
    if let Some(token) = token {
        options = options.set_restore_token(Some(token));
    }
    let selected = match proxy.select_sources(guard.session(), options).await {
        Ok(selected) => selected,
        Err(err) => {
            return Err(guard
                .fail(select_error(err.to_string(), token.is_some()))
                .await);
        }
    };
    if let Err(err) = selected.response() {
        return Err(guard
            .fail(select_error(err.to_string(), token.is_some()))
            .await);
    }
    let started = match proxy.start(guard.session(), None, Default::default()).await {
        Ok(started) => started,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    let streams = match started.response() {
        Ok(streams) => streams,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    if let Some(restore) = streams.restore_token() {
        write_token(data_dir, restore);
    }
    let outputs = portal::output_geometry().await;
    let Some(chosen) = pick_stream(streams.streams(), output_name, &outputs) else {
        return Err(guard
            .fail(OpenFail::Other(
                "screencast start returned no stream".into(),
            ))
            .await);
    };
    let node = chosen.pipe_wire_node_id();
    tracing::info!(node, "screencast stream ready");
    let fd = match proxy
        .open_pipe_wire_remote(guard.session(), Default::default())
        .await
    {
        Ok(fd) => fd,
        Err(err) => return Err(guard.fail(OpenFail::Other(err.to_string())).await),
    };
    let handle = match pw::open_stream(std::os::fd::AsRawFd::as_raw_fd(&fd), node, max_fps) {
        Ok(handle) => handle,
        Err(err) => return Err(guard.fail(OpenFail::Other(err)).await),
    };
    let session = guard.disarm();
    let (hold, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _proxy = proxy;
        let _ = rx.await;
        let _ = tokio::time::timeout(Duration::from_secs(2), session.close()).await;
    });
    Ok(Live {
        _fd: fd,
        stream: std::sync::Arc::new(handle),
        close_session: Some(hold),
    })
}

fn select_error(err: String, had_token: bool) -> OpenFail {
    // A cancel waits out the backoff. It is not asked again immediately.
    // The token must not land in the log. A select failure that had a token
    // is retried once without it.
    if is_user_cancel(&err) {
        return OpenFail::Other("select_sources cancelled".into());
    }
    if had_token {
        OpenFail::Select("select_sources failed".into())
    } else {
        OpenFail::Other(format!("select_sources failed: {err}"))
    }
}

fn pick_stream<'a>(
    streams: &'a [Stream],
    output_name: Option<&str>,
    outputs: &[OutputRect],
) -> Option<&'a Stream> {
    let monitors: Vec<&Stream> = streams
        .iter()
        .filter(|stream| stream.source_type() == Some(SourceType::Monitor))
        .collect();
    let pool: Vec<&Stream> = if monitors.is_empty() {
        streams.iter().collect()
    } else {
        monitors
    };
    if let Some(name) = output_name
        && let Some(output) = outputs.iter().find(|output| output.name == name)
    {
        if let Some(found) = pool
            .iter()
            .copied()
            .find(|stream| stream_covers_output(stream, output))
        {
            return Some(found);
        }
        tracing::warn!(
            output = name,
            "configured monitor was not the shared stream; using the stream the portal returned"
        );
    }
    if let Some(name) = output_name
        && let Some(found) = pool
            .iter()
            .copied()
            .find(|stream| stream.id() == Some(name) || stream.mapping_id() == Some(name))
    {
        return Some(found);
    }
    pool.first().copied().or_else(|| streams.first())
}

/// Position and size in compositor space, against the monitor the user set.
pub fn stream_covers_output(stream: &Stream, output: &OutputRect) -> bool {
    monitor_matches(stream.position(), stream.size(), output)
}

pub fn monitor_matches(
    position: Option<(i32, i32)>,
    size: Option<(i32, i32)>,
    output: &OutputRect,
) -> bool {
    match (position, size) {
        (Some((x, y)), Some((w, h))) => {
            x == output.x
                && y == output.y
                && w == i32::try_from(output.width).unwrap_or(i32::MAX)
                && h == i32::try_from(output.height).unwrap_or(i32::MAX)
        }
        _ => false,
    }
}

fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

fn read_token(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(token_path(data_dir)).ok()?;
    let token = text.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

fn write_token(data_dir: &Path, token: &str) {
    if token.is_empty() {
        return;
    }
    let path = token_path(data_dir);
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    if let Ok(mut file) = options.open(&path) {
        let _ = file.write_all(token.as_bytes());
    }
}

async fn fit_stream(
    img: DynamicImage,
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let outputs = portal::output_geometry().await;
    fit_stream_image(img, &outputs, output_name)
}

pub fn fit_stream_image(
    img: DynamicImage,
    outputs: &[OutputRect],
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    if outputs.is_empty() {
        return Ok(img);
    }
    let index = match output_name {
        Some(name) => outputs
            .iter()
            .position(|output| output.name == name)
            .ok_or_else(|| format!("output '{name}' not found"))?,
        None => super::auto_output_index(
            outputs
                .iter()
                .map(|output| (output.name.as_str(), output.area())),
        )
        .unwrap_or(0),
    };
    let chosen = &outputs[index];
    if stream_should_crop(img.width(), img.height(), chosen.area(), outputs.len()) {
        let Some((_, rect)) = portal::crop_rect(outputs, output_name, img.width(), img.height())?
        else {
            return Ok(img);
        };
        return Ok(img.crop_imm(rect.x, rect.y, rect.width, rect.height));
    }
    Ok(img)
}

/// Crop only when the image is clearly the whole desktop: at least two
/// outputs, and the image area is more than 1.2 times the chosen output.
pub fn stream_should_crop(img_w: u32, img_h: u32, chosen_area: u64, outputs: usize) -> bool {
    outputs >= 2
        && chosen_area > 0
        && u64::from(img_w).saturating_mul(u64::from(img_h)) > chosen_area.saturating_mul(6) / 5
}

async fn save_compare(
    data_dir: &Path,
    index: u32,
    stream: &DynamicImage,
    output_name: Option<&str>,
) {
    let dir = data_dir.join("debug").join("portal-compare");
    if std::fs::create_dir_all(&dir).is_err() {
        tracing::warn!("could not create debug/portal-compare");
        return;
    }
    crate::fs_mode::tighten_private_dir(&dir);
    let n = index + 1;
    let stream_path = dir.join(format!("{n:04}-stream.png"));
    let shot_path = dir.join(format!("{n:04}-screenshot.png"));
    if stream.save(&stream_path).is_err() {
        tracing::warn!("could not save a screencast debug frame");
        return;
    }
    crate::fs_mode::tighten_private_file(&stream_path);
    match portal::capture(output_name).await {
        Ok(shot) => {
            if shot.save(&shot_path).is_ok() {
                crate::fs_mode::tighten_private_file(&shot_path);
                tracing::info!(
                    pair = n,
                    "saved a portal compare pair under debug/portal-compare"
                );
            }
        }
        Err(err) => {
            tracing::warn!(error = %err, "screenshot half of the portal compare pair failed");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn output(name: &str, w: u32, h: u32) -> OutputRect {
        OutputRect {
            name: name.to_string(),
            x: 0,
            y: 0,
            width: w,
            height: h,
            real_width: w,
            real_height: h,
        }
    }

    #[test]
    fn backoff_skips_a_recent_failure_and_allows_a_later_try() {
        assert!(should_try_stream(None, START_BACKOFF));
        assert!(!should_try_stream(
            Some(Duration::from_secs(1)),
            START_BACKOFF
        ));
        assert!(should_try_stream(Some(START_BACKOFF), START_BACKOFF));
        assert_eq!(SCREENCAST_FRAME_WAIT, Duration::from_secs(2));
    }

    #[test]
    fn a_single_monitor_frame_is_not_cropped() {
        let outputs = [output("eDP-1", 1920, 1200), output("DP-1", 1920, 1080)];
        assert!(!stream_should_crop(
            1920,
            1080,
            outputs[1].area(),
            outputs.len()
        ));
        assert!(stream_should_crop(
            3840,
            1200,
            outputs[1].area(),
            outputs.len()
        ));
        assert!(!stream_should_crop(1920, 1080, outputs[0].area(), 1));
    }

    #[test]
    fn a_desktop_sized_stream_crops_and_a_monitor_sized_one_does_not() {
        let mut dp = output("DP-1", 1920, 1080);
        dp.x = 1920;
        let outputs = [output("eDP-1", 1920, 1080), dp];
        let desktop = image::DynamicImage::ImageRgb8(image::RgbImage::new(3840, 1080));
        let cropped = fit_stream_image(desktop, &outputs, Some("DP-1")).unwrap();
        assert_eq!((cropped.width(), cropped.height()), (1920, 1080));
        let one = image::DynamicImage::ImageRgb8(image::RgbImage::new(1920, 1080));
        let kept = fit_stream_image(one, &outputs, Some("eDP-1")).unwrap();
        assert_eq!((kept.width(), kept.height()), (1920, 1080));
    }

    #[test]
    fn the_share_closes_on_the_running_edge_and_only_on_the_stream() {
        assert!(should_close_share(true, true, false, true));
        assert!(!should_close_share(true, true, false, false));
        assert!(!should_close_share(true, false, false, true));
        assert!(!should_close_share(true, true, true, true));
        assert!(!should_close_share(false, true, false, true));
        assert!(last_fail_after_deliberate_close(None).is_none());
        assert!(should_try_stream(
            last_fail_after_deliberate_close(None),
            START_BACKOFF
        ));
        assert_eq!(open_permit(true, false), OpenPermit::Busy);
        assert_eq!(open_permit(false, true), OpenPermit::Reuse);
        assert_eq!(open_permit(false, false), OpenPermit::Open);
        let mut placed = output("DP-1", 1920, 1080);
        placed.x = 1920;
        assert!(monitor_matches(
            Some((1920, 0)),
            Some((1920, 1080)),
            &placed
        ));
        assert!(!monitor_matches(Some((0, 0)), Some((1920, 1080)), &placed));
        assert!(!monitor_matches(None, Some((1920, 1080)), &placed));
        assert!(must_close_session(true, false));
        assert!(!must_close_session(true, true));
        assert!(!must_close_session(false, false));
    }

    #[test]
    fn a_cancel_is_not_retried_and_kde_does_not_screenshot_a_late_tab() {
        assert!(is_user_cancel("org.freedesktop.portal.Error.Cancelled"));
        assert!(is_user_cancel("Dialog dismissed by the user"));
        assert!(!is_user_cancel("select_sources failed"));
        assert_eq!(tab_timeout_action("KDE", true), TabTimeoutAction::Skip);
        assert_eq!(tab_timeout_action("KDE", false), TabTimeoutAction::Skip);
        assert_eq!(tab_timeout_action("GNOME", false), TabTimeoutAction::Skip);
        assert_eq!(
            tab_timeout_action("GNOME", true),
            TabTimeoutAction::Screenshot
        );
        assert_eq!(
            tab_timeout_action("niri", true),
            TabTimeoutAction::Screenshot
        );
        assert_eq!(
            tab_timeout_action("Hyprland", false),
            TabTimeoutAction::Screenshot
        );
        assert!(screenshot_fallback_is_safe("sway", true));
        assert!(!screenshot_fallback_is_safe("ubuntu:GNOME", false));
        let _ = tab_timeout_counts();
    }
}
