use std::path::{Path, PathBuf};

use image::DynamicImage;

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
/// has not answered: ashpd cannot cancel it, and giving the lock back would
/// let a second request run beside it. Such a request is warned about at
/// [`SHOT_OVERDUE_FIRST`] and every [`SHOT_OVERDUE_EVERY`] after, and grabs
/// meanwhile get "busy".
///
/// A poll that finds a request running is skipped. A Tab waits for it (up
/// to [`SHOT_LOCK_WAIT_TAB`]) and then takes its own screenshot: a poll's
/// request was made before the key press, so its picture cannot show the
/// board. The wait mostly overlaps the 400 ms the Tab already sleeps for
/// the overlay to draw.
pub async fn capture(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let output = output_name.map(str::to_owned);
    let wait = super::flight::lock_wait(super::tab_press().is_some(), SHOT_LOCK_WAIT_TAB);
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

/// Runs one portal request under `flight` until the portal answers, however
/// long that takes: it is never given up, only warned about.
async fn run_shot(
    flight: &'static super::flight::SingleFlight,
    wait: std::time::Duration,
    request: ShotRequest,
) -> ShotResult {
    flight
        .run(wait, async move {
            super::flight::with_overdue_warnings(
                request,
                SHOT_OVERDUE_FIRST,
                SHOT_OVERDUE_EVERY,
                |waited| {
                    tracing::warn!(
                        waited_secs = waited.as_secs(),
                        "portal screenshot request has not answered; screenshots wait for it"
                    );
                },
            )
            .await
        })
        .await
        .map_err(|err| format!("portal screenshot: {err}"))?
}

/// One screenshot portal request in flight at a time.
static SHOT_FLIGHT: super::flight::SingleFlight = super::flight::SingleFlight::new();

/// How long a Tab grab waits for the request before it (a poll does not
/// wait). A KDE screenshot takes about 450 ms, so this covers a poll's
/// request and a slow one after it, while Tab is held.
pub const SHOT_LOCK_WAIT_TAB: std::time::Duration = std::time::Duration::from_secs(4);

/// When an unanswered request is first warned about.
pub const SHOT_OVERDUE_FIRST: std::time::Duration = std::time::Duration::from_secs(20);

/// And how often after that.
pub const SHOT_OVERDUE_EVERY: std::time::Duration = std::time::Duration::from_secs(60);

/// Our CPU and the step times go to [`crate::perf`]. CPU is thread time
/// around the synchronous sections only (PNG decode, crop and resize, and
/// the geometry read on its blocking thread), because an await can move
/// this task to another thread.
async fn capture_request(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let sent = std::time::Instant::now();
    let response = ashpd::desktop::screenshot::ScreenshotRequest::default()
        .interactive(false)
        .modal(false)
        .send()
        .await
        .map_err(|e| format!("portal screenshot request failed: {e}"))?
        .response()
        .map_err(|e| format!("portal screenshot response failed: {e}"))?;
    let round_trip = sent.elapsed();

    let uri = response.uri().to_string();
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

/// The size to resize a portal cut-out to, or `None` to keep it. Only a
/// cut-out larger than the output's real mode is scaled down, and only when
/// it is more than 1px off in either direction. An unknown real size (0)
/// keeps the cut-out.
pub fn native_size(cut: PixelRect, real_width: u32, real_height: u32) -> Option<(u32, u32)> {
    if real_width == 0 || real_height == 0 {
        return None;
    }
    let off = |a: u32, b: u32| a.abs_diff(b) > 1;
    let larger = cut.width >= real_width && cut.height >= real_height;
    (larger && (off(cut.width, real_width) || off(cut.height, real_height)))
        .then_some((real_width, real_height))
}

/// Output names for the monitor picker. Empty when the geometry can't be
/// read, which leaves the picker on Auto and the full image in use. Always
/// read fresh, and the cache takes the answer.
pub async fn list_outputs() -> Vec<String> {
    invalidate_output_geometry();
    output_rects().await.into_iter().map(|o| o.name).collect()
}

/// Output rectangles, from the cache while it is fresh. Stream open uses
/// this; its CPU is not capture CPU.
pub(crate) async fn output_geometry() -> Vec<OutputRect> {
    output_rects().await
}

/// Output rectangles for an image of `img_width` x `img_height` that is one
/// output or the whole desktop, and the thread CPU of any read this made
/// (none when the cache answered), for a grab that counts it as capture
/// CPU. When the cached layout cannot explain that image, or does not have
/// `output_name`, the outputs changed: the cache is dropped and read again
/// once.
pub(crate) async fn output_geometry_for_timed(
    img_width: u32,
    img_height: u32,
    output_name: Option<&str>,
) -> (Vec<OutputRect>, std::time::Duration) {
    let (outputs, cpu) = output_rects_timed().await;
    if !geometry_looks_stale(&outputs, output_name, img_width, img_height) {
        return (outputs, cpu);
    }
    tracing::debug!(
        img_width,
        img_height,
        "output geometry does not match the captured image; reading it again"
    );
    invalidate_output_geometry();
    let (outputs, again) = output_rects_timed().await;
    (outputs, cpu.saturating_add(again))
}

/// [`output_geometry`] and the thread CPU of the read, for a stream grab,
/// which counts that CPU as capture CPU. Zero when the cache answered.
pub(crate) async fn output_geometry_timed() -> (Vec<OutputRect>, std::time::Duration) {
    output_rects_timed().await
}

/// How long read output geometry is reused. `output_rects` opens a Wayland
/// connection, which used to happen on every grab. A change of outputs
/// inside this window is caught by [`geometry_looks_stale`] or an error.
pub const GEOMETRY_TTL: std::time::Duration = std::time::Duration::from_secs(30);

/// An empty answer (no Wayland, or the read failed) is kept this long only.
pub const GEOMETRY_EMPTY_TTL: std::time::Duration = std::time::Duration::from_secs(5);

struct GeometryCache {
    outputs: Vec<OutputRect>,
    read_at: Option<std::time::Instant>,
}

static GEOMETRY: std::sync::Mutex<GeometryCache> = std::sync::Mutex::new(GeometryCache {
    outputs: Vec::new(),
    read_at: None,
});

/// True when outputs read at `read_at` may still be used at `now`.
pub fn geometry_fresh(
    read_at: Option<std::time::Instant>,
    empty: bool,
    now: std::time::Instant,
) -> bool {
    let ttl = if empty {
        GEOMETRY_EMPTY_TTL
    } else {
        GEOMETRY_TTL
    };
    read_at.is_some_and(|at| now.saturating_duration_since(at) < ttl)
}

/// Drop the cached geometry. A grab that failed in a way an output change
/// explains calls this, and so does the monitor picker.
pub(crate) fn invalidate_output_geometry() {
    GEOMETRY
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .read_at = None;
}

/// True when `outputs` cannot be the layout this image came from: a named
/// output is missing, or the image is neither one output's real size nor
/// the whole logical desktop at one scale (2% slack, as compositors round).
/// No outputs is nothing to check.
pub fn geometry_looks_stale(
    outputs: &[OutputRect],
    output_name: Option<&str>,
    img_width: u32,
    img_height: u32,
) -> bool {
    if outputs.is_empty() || img_width == 0 || img_height == 0 {
        return false;
    }
    if let Some(name) = output_name
        && !outputs.iter().any(|o| o.name == name)
    {
        return true;
    }
    let near = |a: u32, b: u32| a.abs_diff(b) <= 2;
    let one_output = outputs.iter().any(|o| {
        (near(img_width, o.real_width) && near(img_height, o.real_height))
            || (near(img_width, o.width) && near(img_height, o.height))
    });
    if one_output {
        return false;
    }
    let min_x = outputs.iter().map(|o| i64::from(o.x)).min().unwrap_or(0);
    let min_y = outputs.iter().map(|o| i64::from(o.y)).min().unwrap_or(0);
    let max_x = outputs
        .iter()
        .map(|o| i64::from(o.x) + i64::from(o.width))
        .max()
        .unwrap_or(0);
    let max_y = outputs
        .iter()
        .map(|o| i64::from(o.y) + i64::from(o.height))
        .max()
        .unwrap_or(0);
    let (w, h) = (max_x - min_x, max_y - min_y);
    if w <= 0 || h <= 0 {
        return false;
    }
    let sx = f64::from(img_width) / w as f64;
    let sy = f64::from(img_height) / h as f64;
    (sx - sy).abs() / sx.max(sy) > 0.02
}

async fn output_rects() -> Vec<OutputRect> {
    output_rects_timed().await.0
}

/// The output rectangles from the cache while it is fresh (no CPU), else
/// read, kept, and the thread CPU the read took on its blocking thread.
async fn output_rects_timed() -> (Vec<OutputRect>, std::time::Duration) {
    let now = std::time::Instant::now();
    {
        let cache = GEOMETRY.lock().unwrap_or_else(|err| err.into_inner());
        if geometry_fresh(cache.read_at, cache.outputs.is_empty(), now) {
            return (cache.outputs.clone(), std::time::Duration::ZERO);
        }
    }
    let (outputs, cpu) = read_output_rects_timed().await;
    let mut cache = GEOMETRY.lock().unwrap_or_else(|err| err.into_inner());
    cache.outputs = outputs.clone();
    cache.read_at = Some(std::time::Instant::now());
    (outputs, cpu)
}

/// Reads the output rectangles, and the thread CPU the read took on its
/// blocking thread.
async fn read_output_rects_timed() -> (Vec<OutputRect>, std::time::Duration) {
    if !super::wayshot::is_available() {
        return (Vec::new(), std::time::Duration::ZERO);
    }
    let read = tokio::task::spawn_blocking(|| {
        let cpu_started = crate::perf::thread_cpu();
        let outputs = super::wayshot::output_rects();
        (
            outputs,
            crate::perf::thread_cpu().saturating_sub(cpu_started),
        )
    })
    .await;
    match read {
        Ok((Ok(outputs), cpu)) => (outputs, cpu),
        Ok((Err(e), cpu)) => {
            tracing::debug!(error = %e, "could not read output geometry for the portal");
            (Vec::new(), cpu)
        }
        Err(e) => {
            tracing::debug!(error = %e, "output geometry task failed");
            (Vec::new(), std::time::Duration::ZERO)
        }
    }
}

/// One output's logical rectangle on the desktop, as xdg-output reports it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OutputRect {
    pub name: String,
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    /// Real pixel size (the mode, rotation applied). 0 when unknown.
    pub real_width: u32,
    pub real_height: u32,
}

impl OutputRect {
    /// Pixel area for the Auto choice: the real size, else the logical one.
    pub(crate) fn area(&self) -> u64 {
        if self.real_width > 0 && self.real_height > 0 {
            u64::from(self.real_width) * u64::from(self.real_height)
        } else {
            u64::from(self.width) * u64::from(self.height)
        }
    }
}

/// A crop rectangle in image pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PixelRect {
    pub x: u32,
    pub y: u32,
    pub width: u32,
    pub height: u32,
}

/// Where in a whole-desktop portal image the chosen output sits. Logical
/// coordinates map to image pixels by the ratio of image size to the
/// logical desktop's bounding box, so a desktop rendered at scale 2 maps
/// each logical pixel to 2 image pixels. Returns the chosen output's index
/// in `outputs` and its rectangle. Auto (`None`) uses `auto_output_index`.
/// `Ok(None)` keeps the full image
/// (one output, no geometry, or a degenerate layout). A named output that
/// isn't there is an error, as it is for wayshot.
pub fn crop_rect(
    outputs: &[OutputRect],
    output_name: Option<&str>,
    img_width: u32,
    img_height: u32,
) -> Result<Option<(usize, PixelRect)>, Box<dyn std::error::Error + Send + Sync>> {
    if outputs.len() < 2 || img_width == 0 || img_height == 0 {
        return Ok(None);
    }
    let index = match output_name {
        Some(name) => outputs.iter().position(|o| o.name == name).ok_or_else(|| {
            let available: Vec<_> = outputs.iter().map(|o| o.name.as_str()).collect();
            format!(
                "output '{}' not found: available: {}",
                name,
                available.join(", ")
            )
        })?,
        None => super::auto_output_index(outputs.iter().map(|o| (o.name.as_str(), o.area())))
            .unwrap_or(0),
    };
    let output = &outputs[index];

    let min_x = outputs.iter().map(|o| i64::from(o.x)).min().unwrap_or(0);
    let min_y = outputs.iter().map(|o| i64::from(o.y)).min().unwrap_or(0);
    let max_x = outputs
        .iter()
        .map(|o| i64::from(o.x) + i64::from(o.width))
        .max()
        .unwrap_or(0);
    let max_y = outputs
        .iter()
        .map(|o| i64::from(o.y) + i64::from(o.height))
        .max()
        .unwrap_or(0);
    let total_w = max_x - min_x;
    let total_h = max_y - min_y;
    if total_w <= 0 || total_h <= 0 || output.width == 0 || output.height == 0 {
        return Ok(None);
    }

    let sx = f64::from(img_width) / total_w as f64;
    let sy = f64::from(img_height) / total_h as f64;
    let to_px = |v: i64, scale: f64, max: u32| -> u32 {
        ((v as f64 * scale).round().max(0.0) as u32).min(max)
    };
    let left = to_px(i64::from(output.x) - min_x, sx, img_width);
    let top = to_px(i64::from(output.y) - min_y, sy, img_height);
    let right = to_px(
        i64::from(output.x) + i64::from(output.width) - min_x,
        sx,
        img_width,
    );
    let bottom = to_px(
        i64::from(output.y) + i64::from(output.height) - min_y,
        sy,
        img_height,
    );
    if right <= left || bottom <= top {
        return Ok(None);
    }
    Ok(Some((
        index,
        PixelRect {
            x: left,
            y: top,
            width: right - left,
            height: bottom - top,
        },
    )))
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
    use super::*;

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
                .run(lock_wait(false, SHOT_LOCK_WAIT_TAB), async {})
                .await;
            assert_eq!(poll, Err(FlightError::Busy));
            let tab = FLIGHT
                .run(lock_wait(true, SHOT_LOCK_WAIT_TAB), async {})
                .await;
            assert_eq!(tab.is_ok(), tab_gets_it, "request held {held_ms} ms");
            held.await.unwrap().unwrap();
        }
        assert_eq!(SHOT_OVERDUE_FIRST, Duration::from_secs(20));
        assert_eq!(SHOT_OVERDUE_EVERY, Duration::from_secs(60));
    }

    /// The portal cannot cancel a request, so one that has not answered
    /// keeps the lock: no second request runs beside it, at 25 s or at
    /// 5 minutes, and the next one runs once it answers.
    #[tokio::test(start_paused = true)]
    async fn an_unanswered_portal_request_keeps_the_lock_until_it_answers() {
        use super::super::flight::SingleFlight;
        use std::time::Duration;
        static FLIGHT: SingleFlight = SingleFlight::new();
        let (answer_tx, answer_rx) = tokio::sync::oneshot::channel::<()>();
        let first = tokio::spawn(run_shot(
            &FLIGHT,
            Duration::ZERO,
            Box::pin(async move {
                let _ = answer_rx.await;
                Ok(DynamicImage::new_rgb8(1, 1))
            }),
        ));
        for at in [25u64, 300] {
            tokio::time::sleep_until(tokio::time::Instant::now() + Duration::from_secs(at)).await;
            assert!(FLIGHT.in_flight(), "held at {at} s");
            let beside = run_shot(
                &FLIGHT,
                Duration::ZERO,
                Box::pin(async { Ok(DynamicImage::new_rgb8(2, 2)) }),
            )
            .await;
            assert!(
                beside.is_err_and(|e| super::super::flight::is_busy(&e.to_string())),
                "nothing runs beside it at {at} s"
            );
        }
        answer_tx.send(()).unwrap();
        let answered = first.await.unwrap().unwrap();
        assert_eq!(answered.width(), 1);
        let next = run_shot(
            &FLIGHT,
            Duration::ZERO,
            Box::pin(async { Ok(DynamicImage::new_rgb8(2, 2)) }),
        )
        .await
        .unwrap();
        assert_eq!(next.width(), 2);
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
