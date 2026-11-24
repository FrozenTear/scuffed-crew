mod answer;
mod backoff;

pub use answer::{
    SHOT_GIVE_UP, SHOT_OVERDUE_EVERY, SHOT_OVERDUE_FIRST, SHOT_UNANSWERED, lookup_error_is_absent,
    owner_error_is_gone,
};
pub use backoff::{SHOT_BACKOFF, SHOT_BACKOFF_STEPS, SHOT_UNCLOSED_HOLD};

use std::path::{Path, PathBuf};

use image::DynamicImage;

use super::geom::{OutputRect, crop_rect, native_size};
use super::outputs::output_geometry_for_timed;

use answer::{BusChecks, TakerSlot, await_answer};
use backoff::{BACKOFF, lock_backoff};

pub async fn is_available() -> bool {
    std::env::var("XDG_CURRENT_DESKTOP").is_ok()
}

/// Takes a portal screenshot and, on a multi-monitor desktop, crops it to one
/// output. The portal always returns the whole desktop, so `output_name`
/// picks the monitor. Auto (`None`) prefers an external monitor, then the
/// largest, as wayshot does (see `auto_output_index`).
/// Output geometry is read from Wayland (xdg-output), which KDE exposes even
/// though it has no screencopy. With one output, or when the geometry can't
/// be read, the full image is kept.
///
/// The request runs on its own task. A caller that drops this future (a Tab
/// cancelling a poll, a shutdown race) only detaches that task: it still
/// receives the file URI from the portal and removes the file, so no
/// screenshot is left in the user's Pictures folder. A runtime that is
/// shutting down can still cut the task short.
///
/// One request at a time in this process ([`SHOT_FLIGHT`]).
/// xdg-desktop-portal-kde names the file `Screenshot_yyyyMMdd_hhmmss.png`
/// and overwrites it, so two requests in one second shared one file and one
/// request's cleanup could delete the other's picture. A detached request
/// keeps the lock until its file is gone, and so does a request the portal
/// has not answered yet: it is warned about at [`SHOT_OVERDUE_FIRST`], and
/// grabs meanwhile get "busy".
///
/// An unanswered request is looked at every [`SHOT_GIVE_UP`] (S5-CAP-1,
/// [`await_answer`]):
/// - The portal process changed or is gone (a crash or a restart): the
///   request is dead, and is given up.
/// - The screenshot permission is "yes" in the permission store: no dialog
///   can be what it waits for, so it is given up and polls back off
///   ([`ShotBackoff`]).
/// - Otherwise (no "yes", or the store could not be read) a permission
///   prompt may be up, perhaps behind the game: the request keeps waiting
///   for the player, as in rc.7, and the app says so. Closing it would not
///   dismiss that dialog, and every new request would stack another.
///
/// A request given up is closed (`org.freedesktop.portal.Request.Close`) and
/// its bus connection dropped, so a late answer reaches nothing. A file the
/// portal writes for it later is left where it is, on purpose: the portal
/// names files by the second, so removing it could remove another
/// request's picture. Before rc.8 every unanswered request held the lock
/// until a restart, so a portal that crashed mid-request stopped every
/// capture.
///
/// An ordinary poll that finds a request running is skipped; the end-reel
/// outcome-only poll waits up to [`super::tab::OUTCOME_POLL_LOCK_WAIT`] (1 s)
/// for it ([`super::tab::grab_lock_wait`]). A Tab waits for it (up
/// to [`SHOT_LOCK_WAIT_TAB`]) and then takes its own screenshot: a poll's
/// request was made before the key press, so its picture cannot show the
/// board. The wait mostly overlaps the 400 ms the Tab already sleeps for
/// the overlay to draw.
///
/// [`ShotBackoff`]: backoff::ShotBackoff
pub async fn capture(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let wait = super::tab::grab_lock_wait(SHOT_LOCK_WAIT_TAB);
    // A Tab and the end-reel outcome poll wait for the lock; an ordinary
    // poll does not. The first two may try during a backoff pause.
    let urgent = !wait.is_zero();
    if let Some(why) = lock_backoff().refuses(urgent, tokio::time::Instant::now()) {
        return Err(why.into());
    }
    let output = output_name.map(str::to_owned);
    run_shot(
        &SHOT_FLIGHT,
        wait,
        Box::pin(async move { capture_request(output.as_deref()).await }),
    )
    .await
}

type ShotResult = Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>>;

/// A portal request, boxed: its future is several KB, and every future
/// that awaits a screenshot would otherwise carry copies of it inline.
type ShotRequest = std::pin::Pin<Box<dyn std::future::Future<Output = ShotResult> + Send>>;

/// Runs one portal request under `flight`, holding the lock until the
/// request and its file cleanup end. The wait for the portal's answer is
/// bounded inside the request ([`await_answer`]), the work after it is not.
async fn run_shot(
    flight: &'static super::flight::SingleFlight,
    wait: std::time::Duration,
    request: ShotRequest,
) -> ShotResult {
    flight
        .run(wait, request)
        .await
        .map_err(|err| format!("portal screenshot: {err}"))?
}

fn lock<T>(mutex: &std::sync::Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

/// One screenshot portal request in flight at a time.
static SHOT_FLIGHT: super::flight::SingleFlight = super::flight::SingleFlight::new();

/// How long a Tab grab waits for the request before it (an ordinary poll
/// does not wait; the end-reel outcome poll waits up to 1 s). A KDE
/// screenshot takes about 450 ms, so this covers a poll's request and a
/// slow one after it, while Tab is held.
pub const SHOT_LOCK_WAIT_TAB: std::time::Duration = std::time::Duration::from_secs(4);

/// How long the Close call for a request given up may take.
const SHOT_CLOSE_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

/// How long making the session bus connection for one request may take.
const SHOT_CONNECT_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(2);

const PORTAL_DESKTOP: &str = "org.freedesktop.portal.Desktop";
const PORTAL_PATH: &str = "/org/freedesktop/portal/desktop";
const PORTAL_REQUEST: &str = "org.freedesktop.portal.Request";

/// A `handle_token` no other request of this process has used.
fn next_handle_token() -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
    format!("sst{}_{n}", std::process::id())
}

/// The object path xdg-desktop-portal gives the request with `token` from
/// the connection named `unique_name` (`:1.42` becomes `1_42`).
pub fn request_path(unique_name: &str, token: &str) -> String {
    let sender = unique_name.trim_start_matches(':').replace('.', "_");
    format!("{PORTAL_PATH}/request/{sender}/{token}")
}

/// The file URI of a `Response` signal: `0` with a `uri` is a picture,
/// `1` the user cancelled, anything else the portal failed.
pub fn response_uri(
    code: u32,
    results: &std::collections::HashMap<String, ashpd::zvariant::OwnedValue>,
) -> Result<String, String> {
    match code {
        0 => match results.get("uri").map(|value| &**value) {
            Some(ashpd::zvariant::Value::Str(uri)) => Ok(uri.as_str().to_string()),
            _ => Err("portal screenshot response failed: no file in the answer".into()),
        },
        1 => Err("portal screenshot response failed: cancelled".into()),
        _ => Err("portal screenshot response failed: the portal could not take it".into()),
    }
}

/// Closes a portal request when dropped before [`CloseOnDrop::answered`]:
/// a request [`await_answer`] gave up, or a runtime shutting down. (A
/// caller that drops its grab only detaches the flight task, which keeps
/// the request.) The Close call runs on its own task, bounded by
/// [`SHOT_CLOSE_TIMEOUT`]; the bus connection, which only this request
/// used, goes with it, so an answer that comes later has no one to reach.
struct CloseOnDrop {
    conn: ashpd::zbus::Connection,
    path: String,
    armed: bool,
}

impl CloseOnDrop {
    fn answered(&mut self) {
        self.armed = false;
    }
}

impl Drop for CloseOnDrop {
    fn drop(&mut self) {
        if !self.armed {
            return;
        }
        let Ok(runtime) = tokio::runtime::Handle::try_current() else {
            return;
        };
        let conn = self.conn.clone();
        let path = std::mem::take(&mut self.path);
        runtime.spawn(async move {
            let close = conn.call_method(
                Some(PORTAL_DESKTOP),
                path.as_str(),
                Some(PORTAL_REQUEST),
                "Close",
                &(),
            );
            match tokio::time::timeout(SHOT_CLOSE_TIMEOUT, close).await {
                Ok(Ok(_)) => {
                    lock_backoff().closed();
                    tracing::debug!("closed the unanswered portal screenshot request");
                }
                // A portal that crashed has no such request any more.
                Ok(Err(err)) => {
                    tracing::debug!(error = %err, "portal screenshot request close failed")
                }
                Err(_) => tracing::debug!("portal screenshot request close did not answer"),
            }
        });
    }
}

/// A session bus connection for one request and its checks.
async fn request_connection()
-> Result<ashpd::zbus::Connection, Box<dyn std::error::Error + Send + Sync>> {
    Ok(
        tokio::time::timeout(SHOT_CONNECT_TIMEOUT, ashpd::zbus::Connection::session())
            .await
            .map_err(|_| "portal screenshot request failed: the session bus did not answer")?
            .map_err(|e| format!("portal screenshot request failed: {e}"))?,
    )
}

/// One `Screenshot` call on `conn`, which only this request uses. The
/// outer `Err` is a request the portal never took (no listener, a method
/// error): nothing is closed then. The inner result is the `Response`
/// signal's: the file URI, or why there is none. Dropped before the answer
/// ([`await_answer`] gave up), the request is closed ([`CloseOnDrop`]).
async fn screenshot_uri(
    conn: ashpd::zbus::Connection,
    taker: &TakerSlot,
) -> Result<Result<String, String>, Box<dyn std::error::Error + Send + Sync>> {
    use ashpd::zbus;
    use futures_util::StreamExt as _;
    let unique = conn
        .unique_name()
        .map(|name| name.as_str().to_string())
        .ok_or("portal screenshot request failed: no bus name")?;
    let token = next_handle_token();
    let path = request_path(&unique, &token);
    // Listen before asking, so an answer cannot come first.
    let request = zbus::Proxy::new(&conn, PORTAL_DESKTOP, path.clone(), PORTAL_REQUEST)
        .await
        .map_err(|e| format!("portal screenshot request failed: {e}"))?;
    let mut responses = request
        .receive_signal("Response")
        .await
        .map_err(|e| format!("portal screenshot request failed: {e}"))?;
    let mut close = CloseOnDrop {
        conn: conn.clone(),
        path: path.clone(),
        armed: true,
    };
    let mut options: std::collections::HashMap<&str, ashpd::zvariant::Value<'_>> =
        std::collections::HashMap::new();
    options.insert("handle_token", token.as_str().into());
    options.insert("interactive", false.into());
    options.insert("modal", false.into());
    let reply = match conn
        .call_method(
            Some(PORTAL_DESKTOP),
            PORTAL_PATH,
            Some("org.freedesktop.portal.Screenshot"),
            "Screenshot",
            &("", options),
        )
        .await
    {
        Ok(reply) => reply,
        Err(e) => {
            // The portal did not take the request: nothing to close.
            close.answered();
            return Err(format!("portal screenshot request failed: {e}").into());
        }
    };
    // The frontend that took the request: a later owner that differs is
    // another portal process, and this request died with the first.
    *lock(taker) = reply
        .header()
        .sender()
        .map(|name| name.as_str().to_string());
    if let Ok(handle) = reply
        .body()
        .deserialize::<ashpd::zvariant::OwnedObjectPath>()
        && handle.as_str() != path
    {
        // A portal before 0.9 picks its own path; every current one uses
        // the token's. Its answer would not reach this listener.
        tracing::debug!(handle = %handle.as_str(), "portal screenshot request has an unexpected path");
        close.path = handle.as_str().to_string();
    }
    let Some(message) = responses.next().await else {
        close.answered();
        return Err("portal screenshot response failed: the portal went away".into());
    };
    close.answered();
    Ok(message
        .body()
        .deserialize::<(
            u32,
            std::collections::HashMap<String, ashpd::zvariant::OwnedValue>,
        )>()
        .map_err(|e| format!("portal screenshot response failed: {e}"))
        .and_then(|(code, results)| response_uri(code, &results)))
}

/// Our CPU and the step times go to [`crate::perf`]. CPU is thread time
/// around the synchronous sections only (PNG decode, crop and resize, and
/// the geometry read on its blocking thread), because an await can move
/// this task to another thread.
async fn capture_request(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let sent = std::time::Instant::now();
    let conn = request_connection().await?;
    let checks = BusChecks { conn: conn.clone() };
    // Only the wait for the answer is bounded. The file read, decode and
    // crop after it run to the end under the lock, so the lock covers the
    // file until it is gone.
    let taker = TakerSlot::new(None);
    let uri = await_answer(screenshot_uri(conn, &taker), &checks, &BACKOFF, &taker).await?;
    drop(checks);
    let round_trip = sent.elapsed();

    let path = file_uri_path(&uri).ok_or("unexpected screenshot URI: not a local file")?;
    // The portal wrote this file for us. It goes on every path out of here,
    // a decode error included, so no screenshot is left behind.
    let file = RemoveOnDrop(path);

    // Read, remove and decode on the blocking pool: the read is file I/O
    // and a 1440p PNG takes tens of milliseconds to decode, neither of
    // which may hold a runtime worker. The guard moves into the closure, so
    // the file goes even if the closure never runs. The closure is
    // synchronous on its blocking thread, so thread CPU time measures it.
    let (img, png_bytes, decode, decode_cpu) = tokio::task::spawn_blocking(move || {
        let decode_started = std::time::Instant::now();
        let cpu_started = crate::perf::thread_cpu();
        let bytes = std::fs::read(&file.0);
        drop(file);
        let bytes = bytes.map_err(|e| format!("failed to open portal screenshot: {e}"))?;
        let png_bytes = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
        let img = image::load_from_memory(&bytes)
            .map_err(|e| format!("failed to open portal screenshot: {e}"))?;
        Ok::<_, String>((
            img,
            png_bytes,
            decode_started.elapsed(),
            crate::perf::thread_cpu().saturating_sub(cpu_started),
        ))
    })
    .await
    .map_err(|e| format!("portal screenshot decode task failed: {e}"))??;
    let mut cpu = decode_cpu;

    tracing::debug!(
        width = img.width(),
        height = img.height(),
        "screen captured via XDG Desktop Portal"
    );

    let geometry_started = std::time::Instant::now();
    let (outputs, geometry_cpu) =
        output_geometry_for_timed(img.width(), img.height(), output_name).await;
    let geometry = geometry_started.elapsed();
    cpu = cpu.saturating_add(geometry_cpu);

    let crop_started = std::time::Instant::now();
    let crop_cpu_started = crate::perf::thread_cpu();
    let fitted = crop_to_output(img, &outputs, output_name);
    cpu = cpu.saturating_add(crate::perf::thread_cpu().saturating_sub(crop_cpu_started));
    let crop = crop_started.elapsed();
    if fitted.is_ok() {
        crate::perf::record_cpu(crate::perf::Stage::Capture, cpu);
        crate::perf::record_portal_shot(crate::perf::PortalShot {
            round_trip,
            png_bytes,
            decode,
            geometry,
            crop,
        });
    }
    fitted
}

/// Crop a whole-desktop screenshot to the chosen output and scale it to
/// the output's real size. One output, or no geometry, keeps `img`.
fn crop_to_output(
    img: DynamicImage,
    outputs: &[OutputRect],
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let Some((index, rect)) = crop_rect(outputs, output_name, img.width(), img.height())? else {
        return Ok(img);
    };
    let output = &outputs[index];
    tracing::debug!(output = %output.name, ?rect, "cropped portal screenshot to one output");
    let cut = img.crop_imm(rect.x, rect.y, rect.width, rect.height);
    match native_size(rect, output.real_width, output.real_height) {
        Some((width, height)) => {
            // The portal draws the whole desktop at one scale, so an output
            // at a lower scale than its neighbour comes out larger than its
            // real mode. The recognizers expect the size the game renders.
            tracing::debug!(
                width,
                height,
                "resized the cut-out to the output's real size"
            );
            Ok(cut.resize_exact(width, height, image::imageops::FilterType::Lanczos3))
        }
        None => Ok(cut),
    }
}

/// Deletes the file when dropped.
struct RemoveOnDrop(PathBuf);

impl Drop for RemoveOnDrop {
    fn drop(&mut self) {
        remove_quietly(&self.0);
    }
}

fn remove_quietly(path: &Path) {
    if let Err(e) = std::fs::remove_file(path)
        && e.kind() != std::io::ErrorKind::NotFound
    {
        tracing::debug!(error = %e, "could not remove the portal screenshot");
    }
}

/// The local path of a `file://` URI, percent-decoded. The portal encodes
/// spaces and other bytes (GNOME names the file "Screenshot from ...png",
/// sent as `Screenshot%20from%20...`), and before alpha.12 the encoded
/// path was opened as is, failed, and left the screenshot on disk. `None`
/// for any other scheme, a remote host, or a bad escape.
pub fn file_uri_path(uri: &str) -> Option<PathBuf> {
    use std::os::unix::ffi::OsStringExt as _;
    let rest = uri.strip_prefix("file://")?;
    let rest = rest.strip_prefix("localhost").unwrap_or(rest);
    if !rest.starts_with('/') {
        return None;
    }
    let raw = rest.split(['?', '#']).next()?.as_bytes();
    let mut out = Vec::with_capacity(raw.len());
    let mut i = 0;
    while i < raw.len() {
        if raw[i] == b'%' {
            let hex = raw.get(i + 1..i + 3)?;
            let byte = u8::from_str_radix(std::str::from_utf8(hex).ok()?, 16).ok()?;
            out.push(byte);
            i += 3;
        } else {
            out.push(raw[i]);
            i += 1;
        }
    }
    if out.contains(&0) {
        return None;
    }
    Some(PathBuf::from(std::ffi::OsString::from_vec(out)))
}

#[cfg(test)]
mod tests {
    use super::answer::*;
    use super::backoff::*;
    use super::*;
    use crate::capture::geom::PixelRect;
    use crate::capture::outputs::{
        GEOMETRY_EMPTY_TTL, GEOMETRY_TTL, geometry_fresh, geometry_looks_stale,
    };

    #[test]
    fn file_uri_path_decodes_percent_escapes() {
        assert_eq!(
            file_uri_path("file:///home/a/Pictures/Screenshot%20from%202026-10-18%2014-02-11.png"),
            Some(PathBuf::from(
                "/home/a/Pictures/Screenshot from 2026-10-18 14-02-11.png"
            ))
        );
        assert_eq!(
            file_uri_path("file:///tmp/Screenshot_20261018_140211.png"),
            Some(PathBuf::from("/tmp/Screenshot_20261018_140211.png"))
        );
        assert_eq!(
            file_uri_path("file://localhost/tmp/a%C3%A9.png"),
            Some(PathBuf::from("/tmp/a\u{e9}.png"))
        );
    }

    #[test]
    fn file_uri_path_refuses_what_is_not_a_local_file() {
        for uri in [
            "https://example.com/a.png",
            "file://otherhost/tmp/a.png",
            "file:///tmp/a%2.png",
            "file:///tmp/a%zz.png",
            "file:///tmp/a%00.png",
            "/tmp/a.png",
            "",
        ] {
            assert_eq!(file_uri_path(uri), None, "{uri}");
        }
    }

    #[test]
    fn the_screenshot_is_removed_even_when_it_does_not_decode() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Screenshot from 2026-10-18.png");
        std::fs::write(&path, b"not a png").unwrap();
        let uri = format!("file://{}", path.to_str().unwrap().replace(' ', "%20"));
        let resolved = file_uri_path(&uri).unwrap();
        assert_eq!(resolved, path);
        let result = {
            let file = RemoveOnDrop(resolved);
            image::open(&file.0)
        };
        assert!(result.is_err());
        assert!(!path.exists(), "a failed decode left the screenshot behind");
        // A file that is already gone is not an error.
        drop(RemoveOnDrop(path));
    }

    /// An output at scale 1: the real size is the logical size.
    fn rect(name: &str, x: i32, y: i32, width: u32, height: u32) -> OutputRect {
        OutputRect {
            name: name.to_string(),
            x,
            y,
            width,
            height,
            real_width: width,
            real_height: height,
        }
    }

    fn scaled(name: &str, x: i32, y: i32, real: (u32, u32), scale: f64) -> OutputRect {
        OutputRect {
            name: name.to_string(),
            x,
            y,
            width: (f64::from(real.0) / scale).round() as u32,
            height: (f64::from(real.1) / scale).round() as u32,
            real_width: real.0,
            real_height: real.1,
        }
    }

    /// `crop_rect` without the index, for the geometry tests.
    fn crop(
        outputs: &[OutputRect],
        name: Option<&str>,
        width: u32,
        height: u32,
    ) -> Option<PixelRect> {
        crop_rect(outputs, name, width, height)
            .unwrap()
            .map(|(_, rect)| rect)
    }

    #[test]
    fn two_side_by_side_outputs_crop_to_the_chosen_one() {
        let outputs = [
            rect("eDP-1", 0, 0, 1920, 1080),
            rect("DP-1", 1920, 0, 1920, 1080),
        ];
        let px = |x, y, width, height| PixelRect {
            x,
            y,
            width,
            height,
        };
        assert_eq!(
            crop(&outputs, Some("DP-1"), 3840, 1080),
            Some(px(1920, 0, 1920, 1080))
        );
        assert_eq!(
            crop(&outputs, Some("eDP-1"), 3840, 1080),
            Some(px(0, 0, 1920, 1080))
        );
        // Auto takes the external monitor, as wayshot does.
        assert_eq!(
            crop_rect(&outputs, None, 3840, 1080).unwrap(),
            Some((1, px(1920, 0, 1920, 1080)))
        );
    }

    #[test]
    fn mixed_scale_maps_logical_to_image_pixels() {
        // eDP-1 is a 2560x1440 panel at scale 2 (1280x720 logical), DP-1 a
        // 1920x1080 monitor at scale 1 placed to its right. The portal
        // renders the whole desktop at scale 2, so the image is twice the
        // 3200x1080 logical desktop.
        let outputs = [
            rect("eDP-1", 0, 0, 1280, 720),
            rect("DP-1", 1280, 0, 1920, 1080),
        ];
        assert_eq!(
            crop(&outputs, Some("DP-1"), 6400, 2160),
            Some(PixelRect {
                x: 2560,
                y: 0,
                width: 3840,
                height: 2160
            })
        );
        assert_eq!(
            crop(&outputs, Some("eDP-1"), 6400, 2160),
            Some(PixelRect {
                x: 0,
                y: 0,
                width: 2560,
                height: 1440
            })
        );
    }

    #[test]
    fn negative_origins_are_measured_from_the_desktop_corner() {
        let outputs = [
            rect("DP-1", 0, 0, 1920, 1080),
            rect("HDMI-A-1", -1920, 200, 1920, 1080),
        ];
        assert_eq!(
            crop(&outputs, Some("DP-1"), 3840, 1280),
            Some(PixelRect {
                x: 1920,
                y: 0,
                width: 1920,
                height: 1080
            })
        );
    }

    #[test]
    fn one_output_or_no_geometry_keeps_the_full_image() {
        let one = [rect("eDP-1", 0, 0, 1920, 1080)];
        assert_eq!(crop(&one, None, 1920, 1080), None);
        assert_eq!(crop(&one, Some("eDP-1"), 1920, 1080), None);
        assert_eq!(crop(&[], None, 1920, 1080), None);
        assert_eq!(crop(&[], Some("DP-1"), 1920, 1080), None);
    }

    #[test]
    fn an_unknown_output_name_is_an_error() {
        let outputs = [
            rect("eDP-1", 0, 0, 1920, 1080),
            rect("DP-1", 1920, 0, 1920, 1080),
        ];
        let err = crop_rect(&outputs, Some("HDMI-A-1"), 3840, 1080)
            .unwrap_err()
            .to_string();
        assert!(err.contains("eDP-1, DP-1"), "{err}");
    }

    #[test]
    fn outputs_of_different_heights_and_y_offsets() {
        // Laptop 1920x1200 at scale 1.25 (1536x960 logical) sits below and
        // left of a 2560x1440 monitor at scale 1, offset 300 down.
        let outputs = [
            rect("eDP-1", 0, 300, 1536, 960),
            rect("DP-1", 1536, 0, 2560, 1440),
        ];
        // Logical desktop 4096x1440; the portal image at scale 1.25 is
        // 5120x1800.
        assert_eq!(
            crop(&outputs, Some("eDP-1"), 5120, 1800),
            Some(PixelRect {
                x: 0,
                y: 375,
                width: 1920,
                height: 1200
            })
        );
        assert_eq!(
            crop(&outputs, Some("DP-1"), 5120, 1800),
            Some(PixelRect {
                x: 1920,
                y: 0,
                width: 3200,
                height: 1800
            })
        );
        // Same layout, image at logical size (scale 1): no stretch.
        assert_eq!(
            crop(&outputs, Some("eDP-1"), 4096, 1440),
            Some(PixelRect {
                x: 0,
                y: 300,
                width: 1536,
                height: 960
            })
        );
    }

    #[test]
    fn fractional_scale_rounds_and_clamps_to_the_image() {
        // 1.25 scale on odd sizes: 1229x691 logical side by side. The image
        // is one pixel short of the exact ratio, so the right edge clamps.
        let outputs = [
            rect("eDP-1", 0, 0, 1229, 691),
            rect("DP-1", 1229, 0, 1229, 691),
        ];
        let r = crop(&outputs, Some("DP-1"), 3072, 864).unwrap();
        assert!(r.x + r.width <= 3072, "{r:?}");
        assert!(r.y + r.height <= 864, "{r:?}");
        assert_eq!(r.x, 1536);
        assert_eq!(r.width, 1536);
        assert_eq!(r.height, 864);
    }

    #[test]
    fn auto_prefers_the_external_monitor_and_one_output_keeps_the_image() {
        let outputs = [
            rect("eDP-1", 0, 0, 1920, 1080),
            rect("DP-1", 1920, 0, 1920, 1080),
        ];
        let (index, _) = crop_rect(&outputs, None, 3840, 1080).unwrap().unwrap();
        assert_eq!(outputs[index].name, "DP-1");
        // eDP-1 alone: no crop, the full image is that screen.
        assert_eq!(
            crop_rect(&[rect("eDP-1", 0, 0, 1920, 1080)], None, 1920, 1080).unwrap(),
            None
        );
        // Two external monitors: the bigger one.
        let two = [
            rect("DP-1", 0, 0, 1920, 1080),
            rect("HDMI-A-1", 1920, 0, 2560, 1440),
        ];
        let (index, rect) = crop_rect(&two, None, 4480, 1440).unwrap().unwrap();
        assert_eq!(two[index].name, "HDMI-A-1");
        assert_eq!(
            rect,
            PixelRect {
                x: 1920,
                y: 0,
                width: 2560,
                height: 1440
            }
        );
    }

    #[test]
    fn a_monitor_drawn_above_its_real_size_is_resized_back() {
        // Laptop 1920x1200 at 1.25 (1536x960 logical) next to a 1920x1080
        // monitor at 1.0. The portal draws the desktop at 1.25, so DP-1
        // comes out 2400x1350 and goes back to 1920x1080.
        let outputs = [
            scaled("eDP-1", 0, 0, (1920, 1200), 1.25),
            scaled("DP-1", 1536, 0, (1920, 1080), 1.0),
        ];
        let (index, cut) = crop_rect(&outputs, Some("DP-1"), 4320, 1350)
            .unwrap()
            .unwrap();
        assert_eq!(
            cut,
            PixelRect {
                x: 1920,
                y: 0,
                width: 2400,
                height: 1350
            }
        );
        let o = &outputs[index];
        assert_eq!(
            native_size(cut, o.real_width, o.real_height),
            Some((1920, 1080))
        );
        // The laptop itself is drawn at its own scale: no resize.
        let (index, cut) = crop_rect(&outputs, Some("eDP-1"), 4320, 1350)
            .unwrap()
            .unwrap();
        let o = &outputs[index];
        assert_eq!((cut.width, cut.height), (1920, 1200));
        assert_eq!(native_size(cut, o.real_width, o.real_height), None);
    }

    /// Pins [`SHOT_LOCK_WAIT_TAB`]: a Tab gets the lock behind a slow
    /// request of 3.5 s and gives up behind one of 4.5 s; a poll never waits.
    #[tokio::test(start_paused = true)]
    async fn a_tab_waits_up_to_four_seconds_for_the_portal_and_a_poll_not_at_all() {
        use super::super::flight::{FlightError, SingleFlight, lock_wait};
        use std::time::Duration;
        static FLIGHT: SingleFlight = SingleFlight::new();
        for (held_ms, tab_gets_it) in [(3500u64, true), (4500, false)] {
            let held = tokio::spawn(FLIGHT.run(Duration::ZERO, async move {
                tokio::time::sleep(Duration::from_millis(held_ms)).await;
            }));
            tokio::task::yield_now().await;
            let poll = FLIGHT
                .run(
                    lock_wait(false, SHOT_LOCK_WAIT_TAB, Duration::ZERO),
                    async {},
                )
                .await;
            assert_eq!(poll, Err(FlightError::Busy));
            let tab = FLIGHT
                .run(lock_wait(true, SHOT_LOCK_WAIT_TAB, Duration::ZERO), async {
                })
                .await;
            assert_eq!(tab.is_ok(), tab_gets_it, "request held {held_ms} ms");
            held.await.unwrap().unwrap();
        }
        assert_eq!(SHOT_OVERDUE_FIRST, Duration::from_secs(20));
        assert_eq!(SHOT_OVERDUE_EVERY, Duration::from_secs(60));
    }

    /// A fake desktop for [`await_answer`]: who owns the portal, and the
    /// permission store's answer.
    struct FakeChecks {
        owner: std::sync::Mutex<OwnerLookup>,
        permission: std::sync::Mutex<Permission>,
        /// Checks made, to time a Response against them.
        checks: std::sync::atomic::AtomicUsize,
        /// The notice states published, in order.
        notes: std::sync::Mutex<Vec<crate::capture::PortalState>>,
    }

    impl FakeChecks {
        const fn new(permission: Permission) -> Self {
            Self {
                owner: std::sync::Mutex::new(OwnerLookup::Unknown),
                permission: std::sync::Mutex::new(permission),
                checks: std::sync::atomic::AtomicUsize::new(0),
                notes: std::sync::Mutex::new(Vec::new()),
            }
        }

        fn set_owner(&self, owner: OwnerLookup) {
            *self.owner.lock().unwrap() = owner;
        }

        fn set_permission(&self, permission: Permission) {
            *self.permission.lock().unwrap() = permission;
        }
    }

    impl ShotChecks for FakeChecks {
        async fn portal_owner(&self) -> OwnerLookup {
            self.checks
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            self.owner.lock().unwrap().clone()
        }

        async fn screenshot_permission(&self) -> Permission {
            *self.permission.lock().unwrap()
        }

        fn note(&self, state: crate::capture::PortalState) {
            self.notes.lock().unwrap().push(state);
        }
    }

    fn owner(name: &str) -> OwnerLookup {
        OwnerLookup::Owner(name.to_string())
    }

    type Answer = Result<Result<String, String>, Box<dyn std::error::Error + Send + Sync>>;

    /// A request taken by `:1.5`, answered when `answer_rx` gets a value.
    async fn pending_request(
        answer_rx: tokio::sync::oneshot::Receiver<()>,
        taker: &TakerSlot,
    ) -> Answer {
        *taker.lock().unwrap() = Some(":1.5".to_string());
        let _ = answer_rx.await;
        Ok(Ok("file:///tmp/Screenshot_1.png".to_string()))
    }

    /// S5-CAP-1, permission granted: a request with no answer holds the
    /// lock (nothing beside it at 25 s), is given up at [`SHOT_GIVE_UP`],
    /// frees the lock, is dropped (a late answer reaches nothing), and
    /// polls back off 30 s, then 2, 5, 10 minutes. A Tab or the end-reel
    /// poll may try during the pause once the Close of the request given
    /// up was answered.
    #[tokio::test(start_paused = true)]
    async fn a_stalled_request_with_permission_is_given_up_and_polls_back_off() {
        use super::super::flight::SingleFlight;
        use std::time::Duration;
        static FLIGHT: SingleFlight = SingleFlight::new();
        static CHECKS: FakeChecks = FakeChecks::new(Permission::Yes);
        static STATE: std::sync::Mutex<ShotBackoff> = std::sync::Mutex::new(ShotBackoff {
            until: None,
            gave_up_at: None,
            stalls: 0,
            abandoned_open: false,
            answered_ok: false,
        });
        static TAKER: TakerSlot = TakerSlot::new(None);
        CHECKS.set_owner(owner(":1.5"));
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<()>();
        let started = tokio::time::Instant::now();
        let first = tokio::spawn(run_shot(
            &FLIGHT,
            Duration::ZERO,
            Box::pin(async move {
                await_answer(pending_request(answer_rx, &TAKER), &CHECKS, &STATE, &TAKER).await?;
                Ok(DynamicImage::new_rgb8(1, 1))
            }),
        ));
        tokio::time::sleep_until(started + Duration::from_secs(25)).await;
        assert!(FLIGHT.in_flight(), "held at 25 s");
        let beside = run_shot(
            &FLIGHT,
            Duration::ZERO,
            Box::pin(async { Ok(DynamicImage::new_rgb8(2, 2)) }),
        )
        .await;
        assert!(
            beside.is_err_and(|e| super::super::flight::is_busy(&e.to_string())),
            "nothing runs beside it at 25 s"
        );
        let given_up = tokio::time::timeout(Duration::from_secs(600), first)
            .await
            .expect("the request was given up, not left to hang")
            .unwrap()
            .unwrap_err()
            .to_string();
        assert!(given_up.contains(SHOT_UNANSWERED), "{given_up}");
        assert_eq!(started.elapsed(), SHOT_GIVE_UP, "given up at the bound");
        assert!(!FLIGHT.in_flight(), "the lock is released");
        assert!(
            answer_tx.send(()).is_err(),
            "the request was dropped: a late answer reaches nothing"
        );
        // Polls pause 30 s first; a Tab waits until the Close is answered.
        let now = tokio::time::Instant::now();
        {
            let mut state = STATE.lock().unwrap();
            assert_eq!(state.refuses(false, now), Some(SHOT_BACKOFF));
            assert_eq!(
                state.refuses(true, now),
                Some(SHOT_BACKOFF),
                "one outstanding"
            );
            state.closed();
            assert_eq!(
                state.refuses(true, now),
                None,
                "a Tab or the reel poll may try"
            );
            assert_eq!(
                state.refuses(false, now + Duration::from_secs(29)),
                Some(SHOT_BACKOFF)
            );
            assert_eq!(state.refuses(false, now + Duration::from_secs(30)), None);
            // The next stalls pause longer: 2, 5, then 10 minutes.
            assert_eq!(state.gave_up(now), Duration::from_secs(120));
            assert_eq!(state.gave_up(now), Duration::from_secs(300));
            assert_eq!(state.gave_up(now), Duration::from_secs(600));
            assert_eq!(state.gave_up(now), Duration::from_secs(600));
            // An answered request ends the pause.
            state.answered(true);
            assert_eq!(state.refuses(false, now), None);
            assert_eq!(state.gave_up(now), Duration::from_secs(30));
        }
        assert_eq!(
            CHECKS.notes.lock().unwrap().last(),
            Some(&crate::capture::PortalState::Stalled)
        );
        assert!(SHOT_GIVE_UP >= Duration::from_secs(60));
        assert!(SHOT_OVERDUE_FIRST < SHOT_GIVE_UP);
    }

    /// Permission not granted, absent before any picture, or unreadable:
    /// a prompt may be up, so the request keeps waiting for the player, as
    /// in rc.7, and its answer is used whenever it comes. No pause follows.
    #[tokio::test(start_paused = true)]
    async fn a_request_that_may_wait_on_a_prompt_is_not_given_up() {
        use std::time::Duration;
        for permission in [Permission::Other, Permission::Absent] {
            let checks = FakeChecks::new(permission);
            checks.set_owner(owner(":1.5"));
            let state = std::sync::Mutex::new(ShotBackoff::default());
            let taker = TakerSlot::new(None);
            let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<()>();
            let started = tokio::time::Instant::now();
            let answering = async {
                tokio::time::sleep(Duration::from_secs(150)).await;
                answer_tx.send(()).unwrap();
            };
            let (got, ()) = tokio::join!(
                await_answer(pending_request(answer_rx, &taker), &checks, &state, &taker),
                answering
            );
            assert_eq!(got.unwrap(), "file:///tmp/Screenshot_1.png");
            assert_eq!(started.elapsed(), Duration::from_secs(150));
            let now = tokio::time::Instant::now();
            let state = state.lock().unwrap();
            assert_eq!(state.refuses(false, now), None);
            assert!(state.answered_ok, "a picture was answered");
        }
    }

    /// The portal that took the request went away (no owner, or a new
    /// one): the request is dead, given up whatever the permission says,
    /// with no pause. An owner read that fails is not "gone".
    #[tokio::test(start_paused = true)]
    async fn a_request_to_a_portal_that_went_away_is_given_up() {
        use std::time::Duration;
        for gone in [OwnerLookup::NoOwner, owner(":1.9")] {
            let checks = FakeChecks::new(Permission::Other);
            checks.set_owner(owner(":1.5"));
            let state = std::sync::Mutex::new(ShotBackoff::default());
            let taker = TakerSlot::new(None);
            let (_answer_tx, answer_rx) = tokio::sync::oneshot::channel::<()>();
            let request = await_answer(pending_request(answer_rx, &taker), &checks, &state, &taker);
            tokio::pin!(request);
            assert!(
                tokio::time::timeout(Duration::from_secs(61), &mut request)
                    .await
                    .is_err(),
                "a prompt may be up: waiting"
            );
            checks.set_owner(gone.clone());
            let err = tokio::time::timeout(Duration::from_secs(600), request)
                .await
                .expect("given up")
                .unwrap_err()
                .to_string();
            assert!(err.contains("went away"), "{gone:?}: {err}");
            let now = tokio::time::Instant::now();
            assert_eq!(state.lock().unwrap().refuses(false, now), None, "no pause");
        }
        // A failed owner read changes nothing, whatever the permission.
        let checks = FakeChecks::new(Permission::Yes);
        assert_eq!(
            judge_unanswered(&checks, Some(":1.5"), true).await,
            Unanswered::Unknown
        );
        // Before the method reply named the taker, a live owner is fine.
        checks.set_owner(owner(":1.7"));
        assert_eq!(
            judge_unanswered(&checks, None, false).await,
            Unanswered::Stalled
        );
    }

    /// #442 review B-1: while the Close of the request given up is
    /// unanswered, nothing makes a request for 2 minutes after the give-up,
    /// even past the 30 s first step; once the Close is answered the 30 s
    /// step holds.
    #[test]
    fn an_unclosed_request_holds_new_requests_for_two_minutes() {
        use std::time::Duration;
        let t0 = tokio::time::Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        let mut state = ShotBackoff::default();
        assert_eq!(state.gave_up(t0), Duration::from_secs(30));
        // Close unanswered: refused at 30 s, by polls and Tabs alike.
        assert_eq!(state.refuses(false, at(30)), Some(SHOT_BACKOFF));
        assert_eq!(state.refuses(true, at(30)), Some(SHOT_BACKOFF));
        assert_eq!(state.refuses(false, at(119)), Some(SHOT_BACKOFF));
        // From 2 minutes on, requests run again.
        assert_eq!(state.refuses(false, at(120)), None);
        assert_eq!(state.refuses(true, at(120)), None);
        assert_eq!(SHOT_UNCLOSED_HOLD, Duration::from_secs(120));
        // Close answered: the 30 s step is the pause.
        let mut state = ShotBackoff::default();
        state.gave_up(t0);
        state.closed();
        assert_eq!(state.refuses(false, at(29)), Some(SHOT_BACKOFF));
        assert_eq!(state.refuses(false, at(30)), None);
        assert_eq!(state.refuses(true, at(1)), None);
    }

    /// #442 review A-R1: an unanswered Close blocks urgent grabs for the
    /// whole pause, not only the 2 minute hold; an answered Close lets a
    /// Tab try at once.
    #[test]
    fn an_unclosed_request_blocks_tabs_for_the_whole_pause() {
        use std::time::Duration;
        let t0 = tokio::time::Instant::now();
        let at = |secs| t0 + Duration::from_secs(secs);
        let mut state = ShotBackoff::default();
        state.gave_up(t0);
        state.gave_up(t0);
        // The third stall: a 5 minute pause.
        assert_eq!(state.gave_up(t0), Duration::from_secs(300));
        assert_eq!(state.refuses(true, at(121)), Some(SHOT_BACKOFF));
        assert_eq!(state.refuses(true, at(299)), Some(SHOT_BACKOFF));
        assert_eq!(state.refuses(true, at(300)), None);
        assert_eq!(state.refuses(false, at(300)), None);
        state.closed();
        assert_eq!(state.refuses(true, at(0)), None, "a Tab may try at once");
        assert_eq!(state.refuses(false, at(299)), Some(SHOT_BACKOFF));
    }

    /// Review L1 (N4): a request that showed the prompt notice and then
    /// ends without an answer (the portal went away, or the request turned
    /// out never taken) leaves no prompt notice behind.
    #[tokio::test(start_paused = true)]
    async fn a_prompt_notice_goes_when_its_request_ends_unanswered() {
        use crate::capture::PortalState::{Answering, MayBePrompt};
        use std::time::Duration;
        // The portal went away after the prompt notice.
        let checks = FakeChecks::new(Permission::Other);
        checks.set_owner(owner(":1.5"));
        let state = std::sync::Mutex::new(ShotBackoff::default());
        let taker = TakerSlot::new(None);
        let (_answer_tx, answer_rx) = tokio::sync::oneshot::channel::<()>();
        let request = await_answer(pending_request(answer_rx, &taker), &checks, &state, &taker);
        tokio::pin!(request);
        assert!(
            tokio::time::timeout(Duration::from_secs(61), &mut request)
                .await
                .is_err()
        );
        assert_eq!(*checks.notes.lock().unwrap(), vec![MayBePrompt]);
        checks.set_owner(OwnerLookup::NoOwner);
        assert!(
            tokio::time::timeout(Duration::from_secs(600), request)
                .await
                .expect("given up")
                .is_err()
        );
        assert_eq!(*checks.notes.lock().unwrap(), vec![MayBePrompt, Answering]);

        // The request fails after the prompt notice (the outer Err).
        let checks = FakeChecks::new(Permission::Other);
        checks.set_owner(owner(":1.5"));
        let failing = async {
            tokio::time::sleep(Duration::from_secs(90)).await;
            Err::<Result<String, String>, _>("portal screenshot response failed: x".into())
        };
        assert!(
            await_answer(failing, &checks, &state, &taker)
                .await
                .is_err()
        );
        assert_eq!(*checks.notes.lock().unwrap(), vec![MayBePrompt, Answering]);

        // No prompt notice, nothing to clear.
        let checks = FakeChecks::new(Permission::Other);
        let failing = async {
            Err::<Result<String, String>, _>("portal screenshot request failed: x".into())
        };
        assert!(
            await_answer(failing, &checks, &state, &taker)
                .await
                .is_err()
        );
        assert!(checks.notes.lock().unwrap().is_empty());
    }

    /// No permission entry: a prompt may be up, unless a request of this
    /// process already got a picture (that frontend asks no permission).
    #[tokio::test]
    async fn an_absent_entry_is_a_stall_only_after_a_picture() {
        let checks = FakeChecks::new(Permission::Absent);
        checks.set_owner(owner(":1.5"));
        assert_eq!(
            judge_unanswered(&checks, Some(":1.5"), false).await,
            Unanswered::MayBePrompt
        );
        assert_eq!(
            judge_unanswered(&checks, Some(":1.5"), true).await,
            Unanswered::Stalled
        );
        checks.set_permission(Permission::Other);
        assert_eq!(
            judge_unanswered(&checks, Some(":1.5"), true).await,
            Unanswered::MayBePrompt
        );
        // Only a picture counts as answered before.
        let mut state = ShotBackoff::default();
        state.answered(false);
        assert!(!state.answered_ok);
        state.answered(true);
        state.answered(false);
        assert!(state.answered_ok, "kept for the process");
    }

    /// N3: a Response that lands while a check runs is used, not dropped.
    #[tokio::test(start_paused = true)]
    async fn an_answer_during_the_check_is_not_dropped() {
        struct SlowChecks {
            answer: std::sync::Mutex<Option<tokio::sync::oneshot::Sender<()>>>,
        }
        impl ShotChecks for SlowChecks {
            async fn portal_owner(&self) -> OwnerLookup {
                // The portal answers while the owner is read.
                if let Some(tx) = self.answer.lock().unwrap().take() {
                    let _ = tx.send(());
                }
                tokio::task::yield_now().await;
                OwnerLookup::NoOwner
            }
            async fn screenshot_permission(&self) -> Permission {
                Permission::Yes
            }
        }
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<()>();
        let checks = SlowChecks {
            answer: std::sync::Mutex::new(Some(answer_tx)),
        };
        let state = std::sync::Mutex::new(ShotBackoff::default());
        let taker = TakerSlot::new(None);
        let got = await_answer(pending_request(answer_rx, &taker), &checks, &state, &taker).await;
        assert_eq!(got.unwrap(), "file:///tmp/Screenshot_1.png");
    }

    /// A request the portal never took (a method error) is not an answer.
    #[tokio::test(start_paused = true)]
    async fn a_request_the_portal_never_took_fails_at_once() {
        let checks = FakeChecks::new(Permission::Yes);
        checks.set_owner(owner(":1.5"));
        let state = std::sync::Mutex::new(ShotBackoff::default());
        let taker = TakerSlot::new(None);
        let err = await_answer(
            async {
                Err::<Result<String, String>, _>("portal screenshot request failed: x".into())
            },
            &checks,
            &state,
            &taker,
        )
        .await
        .unwrap_err()
        .to_string();
        assert!(err.contains("request failed"), "{err}");
        assert_eq!(checks.checks.load(std::sync::atomic::Ordering::SeqCst), 0);
    }

    #[test]
    fn the_permission_entry_reads_yes_absent_or_other() {
        let table = |entries: &[(&str, &str)]| {
            entries
                .iter()
                .map(|(app, value)| (app.to_string(), vec![value.to_string()]))
                .collect::<std::collections::HashMap<_, _>>()
        };
        assert_eq!(
            screenshot_permission(&table(&[("", "yes")])),
            Permission::Yes
        );
        assert_eq!(
            screenshot_permission(&table(&[("", "no")])),
            Permission::Other
        );
        assert_eq!(
            screenshot_permission(&table(&[("", "ask")])),
            Permission::Other
        );
        assert_eq!(
            screenshot_permission(&table(&[("org.x.App", "yes")])),
            Permission::Absent
        );
        assert_eq!(screenshot_permission(&table(&[])), Permission::Absent);
        // "No entry for screenshot" is absent; a missing owner is gone.
        assert!(lookup_error_is_absent(
            "org.freedesktop.portal.Error.NotFound"
        ));
        assert!(!lookup_error_is_absent(
            "org.freedesktop.DBus.Error.NoReply"
        ));
        assert!(owner_error_is_gone(
            "org.freedesktop.DBus.Error.NameHasNoOwner"
        ));
        assert!(!owner_error_is_gone("org.freedesktop.DBus.Error.NoReply"));
    }

    /// The reply seen on a desktop after Allow was clicked:
    /// `a{sas}v 1 "" 1 "yes" y 0`. It decodes as the store's answer and
    /// reads "yes".
    #[test]
    fn the_permission_store_reply_after_allow_decodes_as_yes() {
        use ashpd::zvariant::{self, OwnedValue, Value, serialized::Context};
        let mut permissions = std::collections::HashMap::new();
        permissions.insert(String::new(), vec!["yes".to_string()]);
        let ctxt = Context::new_dbus(zvariant::LE, 0);
        let data = zvariant::to_bytes(ctxt, &(permissions, Value::from(0u8))).unwrap();
        let ((got, _), _): (
            (std::collections::HashMap<String, Vec<String>>, OwnedValue),
            _,
        ) = data.deserialize().unwrap();
        assert_eq!(screenshot_permission(&got), Permission::Yes);
    }

    #[test]
    fn the_request_path_follows_the_portal_rule() {
        assert_eq!(
            request_path(":1.42", "sst7_0"),
            "/org/freedesktop/portal/desktop/request/1_42/sst7_0"
        );
        let (a, b) = (next_handle_token(), next_handle_token());
        assert_ne!(a, b, "every request has its own token");
        assert!(
            a.chars().all(|c| c.is_ascii_alphanumeric() || c == '_'),
            "{a} is a valid handle token"
        );
    }

    #[test]
    fn a_response_names_its_file_or_why_not() {
        use ashpd::zvariant::{OwnedValue, Str};
        let mut results = std::collections::HashMap::new();
        assert!(response_uri(0, &results).is_err(), "no uri");
        results.insert(
            "uri".to_string(),
            OwnedValue::from(Str::from("file:///tmp/Screenshot_1.png")),
        );
        assert_eq!(
            response_uri(0, &results).unwrap(),
            "file:///tmp/Screenshot_1.png"
        );
        assert!(response_uri(1, &results).unwrap_err().contains("cancelled"));
        assert!(response_uri(2, &results).is_err());
    }

    #[test]
    fn cached_geometry_expires_and_an_empty_read_expires_sooner() {
        let t0 = std::time::Instant::now();
        assert!(!geometry_fresh(None, false, t0));
        assert!(geometry_fresh(Some(t0), false, t0 + GEOMETRY_TTL / 2));
        assert!(!geometry_fresh(Some(t0), false, t0 + GEOMETRY_TTL));
        assert!(geometry_fresh(Some(t0), true, t0 + GEOMETRY_EMPTY_TTL / 2));
        assert!(!geometry_fresh(Some(t0), true, t0 + GEOMETRY_EMPTY_TTL));
        assert!(GEOMETRY_EMPTY_TTL < GEOMETRY_TTL);
    }

    #[test]
    fn geometry_that_explains_the_image_is_kept() {
        let two = [
            rect("eDP-1", 0, 0, 1920, 1080),
            rect("DP-1", 1920, 0, 2560, 1440),
        ];
        // One output's real size (a stream or KWin grab of that monitor).
        assert!(!geometry_looks_stale(&two, Some("DP-1"), 2560, 1440));
        assert!(!geometry_looks_stale(&two, None, 1920, 1080));
        // The whole desktop at scale 1 and at scale 2.
        assert!(!geometry_looks_stale(&two, None, 4480, 1440));
        assert!(!geometry_looks_stale(&two, None, 8960, 2880));
        // A scaled panel: the real size, not the logical one.
        let scaled_panel = [scaled("eDP-1", 0, 0, (2560, 1600), 1.25)];
        assert!(!geometry_looks_stale(&scaled_panel, None, 2560, 1600));
        // Nothing to compare against.
        assert!(!geometry_looks_stale(&[], Some("DP-1"), 1920, 1080));
    }

    #[test]
    fn geometry_from_before_an_output_change_is_stale() {
        let two = [
            rect("eDP-1", 0, 0, 1920, 1080),
            rect("DP-1", 1920, 0, 1920, 1080),
        ];
        // The named monitor is gone from the cached list.
        assert!(geometry_looks_stale(&two, Some("HDMI-A-1"), 1920, 1080));
        // A monitor was added below: the image is taller than the cached
        // desktop at any one scale.
        assert!(geometry_looks_stale(&two, None, 3840, 2160));
        // The mode changed from 16:9 to 16:10. (A change that keeps the
        // aspect reads as a scale change; the TTL bounds that case.)
        let one = [rect("DP-1", 0, 0, 1920, 1080)];
        assert!(geometry_looks_stale(&one, None, 2560, 1600));
    }

    #[test]
    fn same_scale_or_unknown_size_is_not_resized() {
        let outputs = [
            rect("eDP-1", 0, 0, 1920, 1080),
            rect("DP-1", 1920, 0, 2560, 1440),
        ];
        let (index, cut) = crop_rect(&outputs, Some("DP-1"), 4480, 1440)
            .unwrap()
            .unwrap();
        let o = &outputs[index];
        assert_eq!(native_size(cut, o.real_width, o.real_height), None);
        let px = |width, height| PixelRect {
            x: 0,
            y: 0,
            width,
            height,
        };
        // Within 1px is the same size.
        assert_eq!(native_size(px(1921, 1081), 1920, 1080), None);
        // Unknown real size, or a cut-out smaller than the mode, is kept.
        assert_eq!(native_size(px(2400, 1350), 0, 0), None);
        assert_eq!(native_size(px(1536, 864), 1920, 1080), None);
    }
}
