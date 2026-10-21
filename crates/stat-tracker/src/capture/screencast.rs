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
use std::time::{Duration, Instant};

use ashpd::desktop::PersistMode;
use ashpd::desktop::screencast::{
    CursorMode, Screencast, SelectSourcesOptions, SourceType, Stream,
};
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
    stream: pw::Handle,
    /// Dropping this lets the task that owns the portal session exit, which
    /// drops the session. The session type is not visible outside ashpd.
    _hold: tokio::sync::oneshot::Sender<()>,
}

struct CastState {
    live: Option<Live>,
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

pub async fn capture(
    output_name: Option<&str>,
    kind: CaptureKind,
    data_dir: &Path,
    debug_frames: bool,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let mut state = CAST.lock().await;
    let age = state.last_fail.map(|at| at.elapsed());
    if !should_try_stream(age, START_BACKOFF) {
        drop(state);
        return screenshot(output_name, "screencast backoff").await;
    }
    let restart = state.live.as_ref().is_none_or(|live| !live.stream.alive());
    if restart {
        // Drop a dead session before asking for another, so the portal is
        // not holding two screen shares.
        state.live = None;
        match open_session(data_dir, output_name).await {
            Ok(live) => {
                state.live = Some(live);
                state.last_fail = None;
            }
            Err(err) => {
                state.last_fail = Some(Instant::now());
                state.live = None;
                note_fallback(
                    &mut state,
                    &format!(
                        "screencast session failed ({err}); falling back to the screenshot portal"
                    ),
                );
                drop(state);
                return portal::capture(output_name).await;
            }
        }
    }
    let handle_alive = state.live.as_ref().is_some_and(|live| live.stream.alive());
    if !handle_alive {
        state.last_fail = Some(Instant::now());
        note_fallback(
            &mut state,
            "screencast stream is not running; falling back to the screenshot portal",
        );
        drop(state);
        return portal::capture(output_name).await;
    }
    // Grab without holding the session lock across the wait. The handle is
    // not cloned: take it out, grab, put it back.
    let live = state.live.take().expect("checked above");
    drop(state);
    let grabbed = live.stream.grab(SCREENCAST_FRAME_WAIT);
    let timing = live.stream.timing();
    let alive = live.stream.alive();
    let mut state = CAST.lock().await;
    state.live = Some(live);
    match grabbed {
        Ok(frame) => {
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
                save_compare(data_dir, index, &img).await;
            }
            Ok(img)
        }
        Err(err) => {
            state.last_fail = Some(Instant::now());
            if !alive {
                state.live = None;
            }
            note_fallback(
                &mut state,
                &format!(
                    "screencast produced no frame within {}s ({err}); falling back to the screenshot portal",
                    SCREENCAST_FRAME_WAIT.as_secs()
                ),
            );
            drop(state);
            portal::capture(output_name).await
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

async fn open_session(data_dir: &Path, output_name: Option<&str>) -> Result<Live, String> {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    portal_backend::log_answered(&desktop);
    let token = read_token(data_dir);
    match open_once(
        data_dir,
        output_name,
        token.as_deref(),
        PersistMode::ExplicitlyRevoked,
    )
    .await
    {
        Ok(live) => Ok(live),
        Err(OpenFail::Select(_)) => {
            tracing::info!(
                "portal backend did not accept a persistent restore token; asking again without it"
            );
            let _ = std::fs::remove_file(token_path(data_dir));
            match open_once(data_dir, output_name, None, PersistMode::DoNot).await {
                Ok(live) => Ok(live),
                Err(err) => Err(err.to_string()),
            }
        }
        Err(err) => Err(err.to_string()),
    }
}

async fn open_once(
    data_dir: &Path,
    output_name: Option<&str>,
    token: Option<&str>,
    persist: PersistMode,
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
    let session = proxy
        .create_session(Default::default())
        .await
        .map_err(|err| OpenFail::Other(err.to_string()))?;
    let mut options = SelectSourcesOptions::default()
        .set_sources(enumflags2::BitFlags::from(SourceType::Monitor))
        .set_multiple(true)
        .set_cursor_mode(CursorMode::Hidden)
        .set_persist_mode(persist);
    if let Some(token) = token {
        options = options.set_restore_token(Some(token));
    }
    let selected = proxy
        .select_sources(&session, options)
        .await
        .map_err(|err| select_error(err.to_string(), token.is_some()))?;
    if let Err(err) = selected.response() {
        return Err(select_error(err.to_string(), token.is_some()));
    }
    let started = proxy
        .start(&session, None, Default::default())
        .await
        .map_err(|err| OpenFail::Other(err.to_string()))?;
    let streams = started
        .response()
        .map_err(|err| OpenFail::Other(err.to_string()))?;
    if let Some(restore) = streams.restore_token() {
        write_token(data_dir, restore);
    }
    let chosen = pick_stream(streams.streams(), output_name);
    let Some(chosen) = chosen else {
        return Err(OpenFail::Other(
            "screencast start returned no stream".into(),
        ));
    };
    let node = chosen.pipe_wire_node_id();
    tracing::info!(node, "screencast stream ready");
    let fd = proxy
        .open_pipe_wire_remote(&session, Default::default())
        .await
        .map_err(|err| OpenFail::Other(err.to_string()))?;
    let handle =
        pw::open_stream(std::os::fd::AsRawFd::as_raw_fd(&fd), node).map_err(OpenFail::Other)?;
    let (hold, rx) = tokio::sync::oneshot::channel();
    tokio::spawn(async move {
        let _proxy = proxy;
        let _session = session;
        let _ = rx.await;
    });
    Ok(Live {
        _fd: fd,
        stream: handle,
        _hold: hold,
    })
}

fn select_error(err: String, had_token: bool) -> OpenFail {
    // The token must not land in the log. A select failure is retried once
    // without it, so the text only needs to say that select failed.
    if had_token {
        OpenFail::Select("select_sources failed".into())
    } else {
        OpenFail::Select(format!("select_sources failed: {err}"))
    }
}

fn pick_stream<'a>(streams: &'a [Stream], output_name: Option<&str>) -> Option<&'a Stream> {
    if let Some(name) = output_name
        && let Some(found) = streams
            .iter()
            .find(|stream| stream.id() == Some(name) || stream.mapping_id() == Some(name))
    {
        return Some(found);
    }
    streams
        .iter()
        .find(|stream| stream.source_type() == Some(SourceType::Monitor))
        .or_else(|| streams.first())
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
    if std::fs::write(&path, token).is_ok() {
        crate::fs_mode::tighten_private_file(&path);
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
        let Some((cut_index, rect)) =
            portal::crop_rect(outputs, output_name, img.width(), img.height())?
        else {
            return Ok(img);
        };
        let cut = img.crop_imm(rect.x, rect.y, rect.width, rect.height);
        let output = &outputs[cut_index];
        return Ok(resize_if_larger(cut, rect, output));
    }
    let full = portal::PixelRect {
        x: 0,
        y: 0,
        width: img.width(),
        height: img.height(),
    };
    let target = resize_target(img.width(), img.height(), outputs).unwrap_or(chosen);
    Ok(resize_if_larger(img, full, target))
}

fn resize_if_larger(
    img: DynamicImage,
    rect: portal::PixelRect,
    output: &OutputRect,
) -> DynamicImage {
    match portal::native_size(rect, output.real_width, output.real_height) {
        Some((width, height)) => {
            img.resize_exact(width, height, image::imageops::FilterType::Lanczos3)
        }
        None => img,
    }
}

/// Crop only when the image is clearly the whole desktop: at least two
/// outputs, and the image area is more than 1.2 times the chosen output.
pub fn stream_should_crop(img_w: u32, img_h: u32, chosen_area: u64, outputs: usize) -> bool {
    outputs >= 2
        && chosen_area > 0
        && u64::from(img_w).saturating_mul(u64::from(img_h)) > chosen_area.saturating_mul(6) / 5
}

/// The output a single-monitor frame should be resized against, when one
/// output's area is within 1.2 of the image. A frame that matches none of
/// them is left to the caller.
fn resize_target(img_w: u32, img_h: u32, outputs: &[OutputRect]) -> Option<&OutputRect> {
    let area = u64::from(img_w).saturating_mul(u64::from(img_h));
    outputs.iter().find(|output| {
        let out = output.area();
        out > 0 && area <= out.saturating_mul(6) / 5 && out <= area.saturating_mul(6) / 5
    })
}

async fn save_compare(data_dir: &Path, index: u32, stream: &DynamicImage) {
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
    match portal::capture(None).await {
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
}
