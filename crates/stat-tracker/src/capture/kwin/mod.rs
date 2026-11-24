//! KDE direct capture: KWin's `org.kde.KWin.ScreenShot2` D-Bus interface.
//!
//! The screenshot portal on KDE ("Older screenshot") builds a dialog,
//! asks KWin for the whole workspace with the cursor drawn in, PNG-encodes
//! it into the Pictures folder, and the tracker then decodes and deletes
//! that file. Asking KWin directly skips the dialog, the cursor, the PNG
//! and the file: KWin renders the one output and writes the raw pixels into
//! a pipe the tracker passed in.
//!
//! Protocol (KWin `src/plugins/screenshot/screenshotdbusinterface2.cpp`):
//! the call takes a pipe fd, KWin duplicates it, sends the D-Bus reply with
//! the picture's metadata, and then writes exactly `stride * height` raw
//! bytes from a worker thread and closes its end. There is no header. The
//! tracker closes its own write end after the call and reads to EOF.
//!
//! Plasma 6.0 to 6.7 refuse callers that no `.desktop` file grants the
//! interface: `Error.NoAuthorized`, or an empty answer when KWin cannot
//! tell who called. Plasma 6.8.0 dropped that check. rc.7 uses this route
//! only when a 1x1 probe succeeds without a grant.
//!
//! KDE bug 524840: KWin could crash on NVIDIA when a screenshot met an
//! active screencast. The fix is verified in 6.6.7 and 6.8.0 (a 6.7.6
//! backport could not be verified), with a further fix in 6.8.1. The portal
//! and this route share that code path, and the screencast may be another
//! app's (a Discord or OBS share), so with the NVIDIA driver loaded this
//! route needs Plasma 6.8.1 or newer: on an older Plasma it is unavailable
//! and renders nothing ([`render_gate`]). Plasma 6.0 to 6.7 refuse the
//! route themselves, so the cut is Plasma 6.8.0 on NVIDIA. Without NVIDIA,
//! and when the Plasma version cannot be read, the probe renders only while
//! the tracker's own share is closed, unless the version is 6.8.1 or later
//! for sure. Every call in
//! this process goes through one lock ([`FLIGHT`]), so two of ours never
//! overlap here. The app and the daemon are two processes and can call at
//! the same moment; KWin runs those one after the other on its own thread.

mod pipe;
mod probe;
mod protocol;

pub use pipe::{finish_pipe, read_exactly};
pub use probe::{
    CRASH_FIX_FULL, CrashFix, NEEDS_FULL_FIX, ProbeAnswer, ProbeBus, Prober, RenderGate,
    STARTUP_RETRY_WAITS, Versions, compare_half_allowed, crash_fix, nvidia_loaded,
    nvidia_loaded_in, parse_kwin_version, probe_at_startup, probe_at_startup_with,
    probe_beside_share, probe_outcome, probe_with_retries, render_gate, reusable,
    status_from_versions, status_without_render, versions,
};
pub use protocol::{
    INTERFACE, KwinError, PATH, PixelOrder, RawMeta, SERVICE, classify_error, parse_meta, picture,
    pixel_order, to_rgb_image,
};

use std::collections::HashMap;
use std::os::fd::OwnedFd;
use std::time::{Duration, Instant};

use ashpd::zbus;
use ashpd::zvariant::{self, OwnedValue, Value};
use image::DynamicImage;

use super::geom::OutputRect;

use pipe::widen_pipe;
use protocol::from_zbus;

/// The whole grab, call and pipe read, ends within this.
pub const GRAB_TIMEOUT: Duration = Duration::from_secs(2);
/// Making the session bus connection ends within this.
pub const CONNECT_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a Tab grab (or the probe) waits for the call before it, at
/// most one [`GRAB_TIMEOUT`]. An ordinary poll does not wait; the end-reel
/// outcome poll waits up to [`super::tab::OUTCOME_POLL_LOCK_WAIT`] (1 s).
pub const LOCK_WAIT_TAB: Duration = Duration::from_secs(2);

static FLIGHT: super::flight::SingleFlight = super::flight::SingleFlight::new();

/// Whether a picture of `size` is off the output's real mode size by more
/// than a pixel in either direction. An unknown real size is not off.
pub fn size_off(size: (u32, u32), real: Option<(u32, u32)>) -> bool {
    match real {
        Some((w, h)) if w > 0 && h > 0 => size.0.abs_diff(w) > 1 || size.1.abs_diff(h) > 1,
        _ => false,
    }
}

/// What [`check_native_size`] found. The picture is never changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SizeCheck {
    /// The cached real size matches, or none is known.
    Matches,
    /// Off against the cached geometry, right after reading it again.
    MatchesAfterReread,
    /// Still off after reading the geometry again: used as KWin sent it.
    StillDiffers,
    /// Off, and the geometry was read again less than
    /// [`NATIVE_REREAD_GAP`] ago: not read again; used as KWin sent it.
    DiffersRereadSkipped,
}

/// A picture that keeps differing from the cached mode size (a monitor
/// whose mode KWin reports differently) re-reads the geometry at most this
/// often, not on every grab. The limit is one clock for the process, not
/// one per output: a re-read for one monitor also holds off another
/// monitor's re-read for this long (the capture monitor rarely changes).
pub const NATIVE_REREAD_GAP: Duration = Duration::from_secs(60);

/// Whether a size mismatch may read the geometry again at `now`.
pub fn native_reread_due(last: Option<Instant>, now: Instant) -> bool {
    last.is_none_or(|at| now.saturating_duration_since(at) >= NATIVE_REREAD_GAP)
}

/// Checks a KDE direct picture against the output's cached real size.
/// `CaptureScreen` renders at that output's own scale, which is the
/// (rotated) mode size on Plasma 6.5 and later, so a difference means the
/// cached geometry is stale: `reread` drops and reads it again, once, when
/// `reread_due` ([`native_reread_due`]). If it still differs, the picture
/// is kept as KWin rendered it. It is never resampled: any resample changes
/// what the readers see.
pub async fn check_native_size<F, Fut>(
    size: (u32, u32),
    real: Option<(u32, u32)>,
    reread_due: bool,
    reread: F,
) -> SizeCheck
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<(u32, u32)>>,
{
    if !size_off(size, real) {
        return SizeCheck::Matches;
    }
    if !reread_due {
        return SizeCheck::DiffersRereadSkipped;
    }
    if size_off(size, reread().await) {
        SizeCheck::StillDiffers
    } else {
        SizeCheck::MatchesAfterReread
    }
}

// ------------------------------------------------------------------- calls

/// What to capture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    Area {
        x: i32,
        y: i32,
        width: u32,
        height: u32,
    },
    Screen(String),
    ActiveScreen,
}

/// The output to grab and its real size. A name is asked for as is (KWin
/// answers `InvalidScreen` when it is gone). Auto picks the output the
/// other portal paths pick ([`super::geom::auto_output_index`]); with no
/// geometry it is KWin's active screen: the focused window's or the
/// pointer's output, as KWin's focus settings decide.
pub(crate) fn pick_target(
    outputs: &[OutputRect],
    output_name: Option<&str>,
) -> (Target, Option<(u32, u32)>) {
    let real = |o: &OutputRect| Some((o.real_width, o.real_height));
    match output_name {
        Some(name) => (
            Target::Screen(name.to_string()),
            outputs.iter().find(|o| o.name == name).and_then(real),
        ),
        None => match super::geom::auto_output_index(
            outputs.iter().map(|o| (o.name.as_str(), o.area())),
        ) {
            Some(index) => (
                Target::Screen(outputs[index].name.clone()),
                real(&outputs[index]),
            ),
            None => (Target::ActiveScreen, None),
        },
    }
}

/// The probe's 1x1 area: the top-left pixel of the output Auto would
/// capture, so it lies on a real output. With no geometry, KWin's active
/// screen at scale 1.
pub(crate) fn probe_target(outputs: &[OutputRect]) -> Target {
    match super::geom::auto_output_index(outputs.iter().map(|o| (o.name.as_str(), o.area()))) {
        Some(index) => Target::Area {
            x: outputs[index].x,
            y: outputs[index].y,
            width: 1,
            height: 1,
        },
        None => Target::ActiveScreen,
    }
}

/// A value made on first use and kept, until an error says it is broken
/// ([`KwinError::NoBus`]); the next use then makes it again. The D-Bus
/// connection lives in one.
pub struct Reconnecting<T> {
    slot: tokio::sync::Mutex<Option<T>>,
}

impl<T: Clone> Reconnecting<T> {
    pub const fn new() -> Self {
        Self {
            slot: tokio::sync::Mutex::const_new(None),
        }
    }

    /// The kept value, or a new one from `make`, which must answer within
    /// [`CONNECT_TIMEOUT`]. zbus has no handshake timeout: a session bus
    /// that accepts and never authenticates would hang every caller,
    /// including the startup probe whose retries then never run.
    pub async fn get_or_make<F, Fut>(&self, make: F) -> Result<T, KwinError>
    where
        F: FnOnce() -> Fut,
        Fut: std::future::Future<Output = Result<T, KwinError>>,
    {
        let mut slot = self.slot.lock().await;
        if let Some(value) = slot.as_ref() {
            return Ok(value.clone());
        }
        let value = tokio::time::timeout(CONNECT_TIMEOUT, make())
            .await
            .unwrap_or(Err(KwinError::Timeout))?;
        *slot = Some(value.clone());
        Ok(value)
    }

    /// Drops the kept value after an error that says it is broken.
    pub async fn forget_after(&self, err: &KwinError) {
        if matches!(err, KwinError::NoBus(_)) {
            *self.slot.lock().await = None;
        }
    }
}

impl<T: Clone> Default for Reconnecting<T> {
    fn default() -> Self {
        Self::new()
    }
}

static CONNECTION: Reconnecting<zbus::Connection> = Reconnecting::new();

/// The session bus connection, made on first use and made again after a
/// call found it gone ([`forget_connection`]).
async fn connection() -> Result<zbus::Connection, KwinError> {
    CONNECTION
        .get_or_make(|| async {
            zbus::Connection::session()
                .await
                .map_err(|err| KwinError::NoBus(err.to_string()))
        })
        .await
}

/// Drops the cached connection after an error that says it is broken.
async fn forget_connection(err: &KwinError) {
    CONNECTION.forget_after(err).await;
}

/// One call with a fresh pipe, then the pixels. `native` is
/// `native-resolution`; the cursor is never drawn.
async fn grab_raw(
    conn: &zbus::Connection,
    target: &Target,
    native: bool,
    deadline: Instant,
) -> Result<(RawMeta, Vec<u8>), KwinError> {
    let (reader, writer) =
        std::io::pipe().map_err(|err| KwinError::Other(format!("pipe: {err}")))?;
    let reader = OwnedFd::from(reader);
    let writer = OwnedFd::from(writer);
    widen_pipe(&reader);
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("include-cursor", Value::Bool(false));
    options.insert("native-resolution", Value::Bool(native));
    let reply = {
        let fd = zvariant::Fd::from(&writer);
        match target {
            Target::Area {
                x,
                y,
                width,
                height,
            } => {
                conn.call_method(
                    Some(SERVICE),
                    PATH,
                    Some(INTERFACE),
                    "CaptureArea",
                    &(*x, *y, *width, *height, options, fd),
                )
                .await
            }
            Target::Screen(name) => {
                conn.call_method(
                    Some(SERVICE),
                    PATH,
                    Some(INTERFACE),
                    "CaptureScreen",
                    &(name.as_str(), options, fd),
                )
                .await
            }
            Target::ActiveScreen => {
                conn.call_method(
                    Some(SERVICE),
                    PATH,
                    Some(INTERFACE),
                    "CaptureActiveScreen",
                    &(options, fd),
                )
                .await
            }
        }
    }
    .map_err(from_zbus)?;
    let map: HashMap<String, OwnedValue> = reply
        .body()
        .deserialize()
        .map_err(|err| KwinError::BadReply(err.to_string()))?;
    let meta = parse_meta(&map)?;
    let len = meta
        .byte_len()
        .ok_or_else(|| KwinError::BadReply("size overflows".into()))?;
    let bytes = tokio::task::spawn_blocking(move || finish_pipe(reader, writer, len, deadline))
        .await
        .map_err(|err| KwinError::Other(format!("pipe read task: {err}")))??;
    Ok((meta, bytes))
}

/// `request`, or [`KwinError::Timeout`] once [`GRAB_TIMEOUT`] has passed.
pub async fn bounded<T>(
    request: impl std::future::Future<Output = Result<T, KwinError>>,
) -> Result<T, KwinError> {
    tokio::time::timeout(GRAB_TIMEOUT, request)
        .await
        .unwrap_or(Err(KwinError::Timeout))
}

/// One grab under [`FLIGHT`], ended by [`GRAB_TIMEOUT`]. `wait` is how long
/// to wait for the lock ([`super::flight::lock_wait`]).
async fn grab_in_flight(
    target: Target,
    native: bool,
    wait: Duration,
) -> Result<(RawMeta, Vec<u8>), KwinError> {
    let result = FLIGHT
        .run(wait, async move {
            let deadline = Instant::now() + GRAB_TIMEOUT;
            bounded(async {
                let conn = connection().await?;
                grab_raw(&conn, &target, native, deadline).await
            })
            .await
        })
        .await
        .map_err(|err| match err {
            super::flight::FlightError::Busy => KwinError::Busy,
            super::flight::FlightError::Task(err) => KwinError::Other(err),
        })
        .and_then(|result| result);
    if let Err(err) = &result {
        forget_connection(err).await;
    }
    result
}

/// The game's monitor, full output, cursor hidden, at the size KWin
/// renders it (the output's mode size). The pixels never touch the disk.
/// Our CPU (the geometry read when the cache misses, and the pixel
/// conversion, both on blocking threads) is capture CPU in [`crate::perf`],
/// for a grab that produced a picture, as on the other routes.
pub async fn capture(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let (outputs, geometry_cpu) = super::outputs::output_geometry_timed().await;
    let (target, real) = pick_target(&outputs, output_name);
    let screen = match &target {
        Target::Screen(name) => Some(name.clone()),
        _ => None,
    };
    let wait = super::tab::grab_lock_wait(LOCK_WAIT_TAB);
    let (meta, bytes) = match grab_in_flight(target, true, wait).await {
        Ok(grabbed) => grabbed,
        Err(err) => {
            if err.invalidates_geometry() {
                super::outputs::invalidate_output_geometry();
            }
            return Err(match (&err, output_name) {
                (KwinError::InvalidScreen, Some(name)) => {
                    format!("output '{name}' not found: {err}").into()
                }
                _ => err.into(),
            });
        }
    };
    let (img, convert_cpu) = tokio::task::spawn_blocking(move || {
        let cpu_started = crate::perf::thread_cpu();
        let img = picture(&meta, &bytes);
        (img, crate::perf::thread_cpu().saturating_sub(cpu_started))
    })
    .await
    .map_err(|err| format!("KDE direct convert task failed: {err}"))?;
    let img = img?;
    crate::perf::record_cpu(
        crate::perf::Stage::Capture,
        geometry_cpu.saturating_add(convert_cpu),
    );
    let size = (img.width(), img.height());
    static LAST_REREAD: std::sync::Mutex<Option<Instant>> = std::sync::Mutex::new(None);
    let due = native_reread_due(
        *LAST_REREAD.lock().unwrap_or_else(|err| err.into_inner()),
        Instant::now(),
    );
    let checked = check_native_size(size, real, due, || async {
        *LAST_REREAD.lock().unwrap_or_else(|err| err.into_inner()) = Some(Instant::now());
        super::outputs::invalidate_output_geometry();
        // This read is capture CPU too, as the first one is.
        let (outputs, reread_cpu) = super::outputs::output_geometry_timed().await;
        crate::perf::record_cpu(crate::perf::Stage::Capture, reread_cpu);
        screen.and_then(|name| {
            outputs
                .iter()
                .find(|o| o.name == name)
                .map(|o| (o.real_width, o.real_height))
        })
    })
    .await;
    if matches!(
        checked,
        SizeCheck::StillDiffers | SizeCheck::DiffersRereadSkipped
    ) {
        tracing::debug!(
            width = size.0,
            height = size.1,
            ?real,
            "KDE direct picture differs from the output's mode size; kept as KWin rendered it"
        );
    }
    tracing::trace!(
        width = size.0,
        height = size.1,
        "screen captured via KWin ScreenShot2"
    );
    Ok(img)
}

/// The KDE direct half of a debug compare pair: the same Tab moment as the
/// stream frame, saved as `NNNN-kwin.png`. KDE only. It runs beside the
/// open stream, so it needs Plasma 6.8.1 ([`render_gate`]).
pub(crate) async fn save_compare_half(dir: &std::path::Path, n: u32, output_name: Option<&str>) {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !super::route::is_kde(&desktop) {
        return;
    }
    let probe = probe_beside_share(true).await;
    if let Err(why) = compare_half_allowed(&versions().await, &probe) {
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("KDE direct half of the compare pair skipped: {why}");
        }
        return;
    }
    let path = dir.join(format!("{n:04}-kwin.png"));
    match capture(output_name).await {
        Ok(img) => {
            if img.save(&path).is_ok() {
                crate::fs_mode::tighten_private_file(&path);
                tracing::info!(pair = n, "saved the KDE direct frame of the compare pair");
            } else {
                tracing::warn!("could not save the KDE direct compare frame");
            }
        }
        Err(err) => tracing::warn!(error = %err, "KDE direct half of the compare pair failed"),
    }
}

#[cfg(test)]
mod tests {
    use crate::capture::route::KwinProbe;

    use super::probe::*;
    use super::protocol::*;
    use super::*;

    fn owned(v: Value<'_>) -> OwnedValue {
        OwnedValue::try_from(v).unwrap()
    }

    fn reply(width: u32, height: u32, stride: u32, format: u32) -> HashMap<String, OwnedValue> {
        let mut map = HashMap::new();
        map.insert("type".to_string(), owned(Value::from("raw")));
        map.insert("width".to_string(), owned(Value::U32(width)));
        map.insert("height".to_string(), owned(Value::U32(height)));
        map.insert("stride".to_string(), owned(Value::U32(stride)));
        map.insert("format".to_string(), owned(Value::U32(format)));
        map.insert("scale".to_string(), owned(Value::F64(1.25)));
        map.insert("screen".to_string(), owned(Value::from("DP-1")));
        map
    }

    fn meta(width: u32, height: u32, stride: u32, format: u32) -> RawMeta {
        RawMeta {
            width,
            height,
            stride,
            format,
            scale: None,
            screen: None,
        }
    }

    #[test]
    fn metadata_from_a_6_8_screen_reply() {
        let m = parse_meta(&reply(2560, 1440, 10240, 16)).unwrap();
        assert_eq!(m.width, 2560);
        assert_eq!(m.height, 1440);
        assert_eq!(m.stride, 10240);
        assert_eq!(m.format, 16);
        assert_eq!(m.scale, Some(1.25));
        assert_eq!(m.screen.as_deref(), Some("DP-1"));
        assert_eq!(m.byte_len(), Some(10240 * 1440));
    }

    #[test]
    fn metadata_takes_signed_numbers_and_no_screen() {
        let mut map = HashMap::new();
        map.insert("width".to_string(), owned(Value::I32(4)));
        map.insert("height".to_string(), owned(Value::I32(2)));
        map.insert("stride".to_string(), owned(Value::I32(16)));
        map.insert("format".to_string(), owned(Value::I32(6)));
        let m = parse_meta(&map).unwrap();
        assert_eq!((m.width, m.height, m.stride, m.format), (4, 2, 16, 6));
        assert_eq!(m.screen, None);
        assert_eq!(m.scale, None);
    }

    #[test]
    fn metadata_that_cannot_describe_a_picture_is_refused() {
        assert_eq!(parse_meta(&HashMap::new()), Err(KwinError::EmptyReply));
        let mut png = reply(2, 2, 8, 16);
        png.insert("type".to_string(), owned(Value::from("png")));
        assert!(matches!(parse_meta(&png), Err(KwinError::BadReply(_))));
        let mut no_width = reply(2, 2, 8, 16);
        no_width.remove("width");
        assert!(
            matches!(parse_meta(&no_width), Err(KwinError::BadReply(e)) if e.contains("width"))
        );
        assert!(
            matches!(parse_meta(&reply(4, 2, 12, 16)), Err(KwinError::BadReply(e)) if e.contains("stride"))
        );
        assert!(
            matches!(parse_meta(&reply(2, 2, 8, 13)), Err(KwinError::BadReply(e)) if e.contains("13"))
        );
        assert!(matches!(
            parse_meta(&reply(0, 2, 8, 16)),
            Err(KwinError::BadReply(_))
        ));
        assert!(matches!(
            parse_meta(&reply(70000, 70000, 280000, 16)),
            Err(KwinError::BadReply(_))
        ));
    }

    /// Red (200,10,20), green (30,180,40), blue (50,60,170) and a magenta
    /// (230,0,240) like the VICTORY palette: R and B differ in every pixel,
    /// so a red/blue swap cannot pass.
    const PIXELS: [[u8; 3]; 4] = [[200, 10, 20], [30, 180, 40], [50, 60, 170], [230, 0, 240]];

    fn expected_rgb() -> Vec<u8> {
        PIXELS.iter().flatten().copied().collect()
    }

    #[test]
    fn rgbx_and_rgba_bytes_are_r_g_b_in_order() {
        for format in [16u32, 17, 18] {
            let mut data = Vec::new();
            for [r, g, b] in PIXELS {
                data.extend_from_slice(&[r, g, b, 255]);
            }
            let img = to_rgb_image(&meta(2, 2, 8, format), &data).unwrap();
            assert_eq!(img.into_raw(), expected_rgb(), "format {format}");
        }
    }

    #[cfg(target_endian = "little")]
    #[test]
    fn xrgb_and_argb_words_are_b_g_r_a_bytes_on_little_endian() {
        for format in [4u32, 5, 6] {
            let mut data = Vec::new();
            for [r, g, b] in PIXELS {
                // 0xAARRGGBB stored little-endian.
                let word = 0xff00_0000u32 | u32::from(r) << 16 | u32::from(g) << 8 | u32::from(b);
                data.extend_from_slice(&word.to_le_bytes());
            }
            assert_eq!(&data[..4], &[20, 10, 200, 255], "B, G, R, A in memory");
            let img = to_rgb_image(&meta(2, 2, 8, format), &data).unwrap();
            assert_eq!(img.into_raw(), expected_rgb(), "format {format}");
        }
    }

    #[test]
    fn premultiplied_pixels_keep_their_stored_colour() {
        // Half-transparent red, premultiplied: stored (100, 0, 0, 128).
        // Over black that is (100, 0, 0), which is what is kept.
        let rgba = [100u8, 0, 0, 128];
        let img = to_rgb_image(&meta(1, 1, 4, 18), &rgba).unwrap();
        assert_eq!(img.into_raw(), vec![100, 0, 0]);
        #[cfg(target_endian = "little")]
        {
            let bgra = [0u8, 0, 100, 128];
            let img = to_rgb_image(&meta(1, 1, 4, 6), &bgra).unwrap();
            assert_eq!(img.into_raw(), vec![100, 0, 0]);
        }
        // An opaque premultiplied pixel is unchanged.
        let img = to_rgb_image(&meta(1, 1, 4, 18), &[230, 0, 240, 255]).unwrap();
        assert_eq!(img.into_raw(), vec![230, 0, 240]);
    }

    #[test]
    fn row_padding_is_skipped() {
        // 1 pixel wide, stride 8: 4 bytes of padding per row.
        let data = [1u8, 2, 3, 255, 9, 9, 9, 9, 4, 5, 6, 255, 9, 9, 9, 9];
        let img = to_rgb_image(&meta(1, 2, 8, 16), &data).unwrap();
        assert_eq!(img.into_raw(), vec![1, 2, 3, 4, 5, 6]);
    }

    #[test]
    fn a_length_other_than_stride_times_height_is_refused() {
        let m = meta(2, 2, 8, 16);
        assert!(to_rgb_image(&m, &[0; 15]).is_err());
        assert!(to_rgb_image(&m, &[0; 17]).is_err());
        assert!(to_rgb_image(&m, &[0; 16]).is_ok());
    }

    #[test]
    fn a_picture_keeps_the_size_kwin_rendered() {
        // A 4x2 reply stays 4x2 whatever the cached mode says: no resample.
        let m = meta(4, 2, 16, 16);
        let data: Vec<u8> = (0..8).flat_map(|_| [200u8, 10, 20, 255]).collect();
        let img = picture(&m, &data).unwrap();
        assert_eq!((img.width(), img.height()), (4, 2));
        assert_eq!(img.to_rgb8().into_raw(), [200, 10, 20].repeat(8));
        assert!(
            matches!(img, DynamicImage::ImageRgb8(_)),
            "RGB, as the stream"
        );
        assert!(!size_off((2560, 1440), Some((2560, 1440))));
        assert!(
            !size_off((2561, 1439), Some((2560, 1440))),
            "a pixel of rounding"
        );
        assert!(size_off((2558, 1440), Some((2560, 1440))));
        assert!(size_off((5120, 2880), Some((2560, 1440))));
        assert!(!size_off((5120, 2880), None));
        assert!(!size_off((5120, 2880), Some((0, 0))));
    }

    #[tokio::test]
    async fn a_size_mismatch_rereads_the_geometry_once_and_never_resizes() {
        let calls = std::cell::Cell::new(0);
        let reread = |answer: Option<(u32, u32)>| {
            let calls = &calls;
            move || {
                calls.set(calls.get() + 1);
                async move { answer }
            }
        };
        // Matches: the geometry is not read again.
        let got = check_native_size((2560, 1440), Some((2560, 1440)), true, reread(None)).await;
        assert_eq!((got, calls.get()), (SizeCheck::Matches, 0));
        // Unknown real size: nothing to compare.
        let got = check_native_size((2560, 1440), None, true, reread(None)).await;
        assert_eq!((got, calls.get()), (SizeCheck::Matches, 0));
        // Stale cache (the monitor changed mode): one re-read fixes it.
        let got = check_native_size(
            (2560, 1440),
            Some((1920, 1080)),
            true,
            reread(Some((2560, 1440))),
        )
        .await;
        assert_eq!((got, calls.get()), (SizeCheck::MatchesAfterReread, 1));
        // Still off after the re-read: kept as KWin rendered it.
        let got = check_native_size(
            (3200, 1800),
            Some((2560, 1440)),
            true,
            reread(Some((2560, 1440))),
        )
        .await;
        assert_eq!((got, calls.get()), (SizeCheck::StillDiffers, 2));
    }

    #[test]
    fn error_names_map_to_route_outcomes() {
        let outcome = |name: &str| probe_outcome(&Err(classify_error(name, Some("x"))));
        for name in [
            "org.freedesktop.DBus.Error.ServiceUnknown",
            "org.freedesktop.DBus.Error.UnknownObject",
            "org.freedesktop.DBus.Error.UnknownMethod",
        ] {
            assert_eq!(classify_error(name, None), KwinError::NotKwin);
            assert!(matches!(outcome(name), KwinProbe::Unavailable(r) if r.contains("not KDE")));
        }
        let refused = outcome("org.kde.KWin.ScreenShot2.Error.NoAuthorized");
        assert!(
            matches!(&refused, KwinProbe::Unavailable(r) if r.contains("6.8")),
            "{refused:?}"
        );
        assert_eq!(
            probe_outcome(&Err(KwinError::EmptyReply)),
            refused,
            "an empty 6.7 answer is the same refusal"
        );
        assert_eq!(
            classify_error("org.kde.KWin.ScreenShot2.Error.Cancelled", None),
            KwinError::Cancelled
        );
        assert_eq!(
            classify_error("org.kde.KWin.ScreenShot2.Error.InvalidScreen", None),
            KwinError::InvalidScreen
        );
        assert!(KwinError::InvalidScreen.invalidates_geometry());
        assert!(!KwinError::Cancelled.invalidates_geometry());
        assert_eq!(
            classify_error("org.kde.KWin.ScreenShot2.Error.FileDescriptor", None),
            KwinError::Rejected("FileDescriptor".into())
        );
        assert_eq!(
            classify_error("org.freedesktop.DBus.Error.NoReply", None),
            KwinError::Timeout
        );
        assert_eq!(
            classify_error("org.example.Weird", Some("boom")),
            KwinError::Other("org.example.Weird: boom".into())
        );
        assert_eq!(probe_outcome(&Ok(())), KwinProbe::Available);
    }

    #[test]
    fn plasma_version_from_support_information() {
        let text = "KWin Support Information:\n...\nVersion\n=======\nKWin version: 6.8.1\nQt Version: 6.10.0\n";
        assert_eq!(parse_kwin_version(text), Some((6, 8, 1)));
        assert_eq!(
            parse_kwin_version("KWin version: 6.7.90\n"),
            Some((6, 7, 90))
        );
        assert_eq!(parse_kwin_version("KWin version: 6.8\n"), Some((6, 8, 0)));
        assert_eq!(parse_kwin_version("nothing here"), None);
        assert_eq!(crash_fix((6, 8, 1)), CrashFix::Full);
        assert_eq!(crash_fix((6, 9, 0)), CrashFix::Full);
        assert_eq!(crash_fix((6, 8, 0)), CrashFix::Partial);
        assert_eq!(crash_fix((6, 7, 6)), CrashFix::Partial);
        assert_eq!(crash_fix((6, 6, 7)), CrashFix::Partial);
        assert_eq!(crash_fix((6, 7, 5)), CrashFix::Missing);
        assert_eq!(crash_fix((6, 6, 6)), CrashFix::Missing);
        assert_eq!(crash_fix((6, 5, 9)), CrashFix::Missing);
    }

    #[test]
    fn auto_target_is_the_external_monitor_at_its_real_size() {
        let outputs = [
            OutputRect {
                name: "eDP-1".into(),
                x: 0,
                y: 0,
                width: 1536,
                height: 960,
                real_width: 1920,
                real_height: 1200,
            },
            OutputRect {
                name: "DP-1".into(),
                x: 1536,
                y: 0,
                width: 2048,
                height: 1152,
                real_width: 2560,
                real_height: 1440,
            },
        ];
        assert_eq!(
            pick_target(&outputs, None),
            (Target::Screen("DP-1".into()), Some((2560, 1440)))
        );
        assert_eq!(
            pick_target(&outputs, Some("eDP-1")),
            (Target::Screen("eDP-1".into()), Some((1920, 1200)))
        );
        assert_eq!(
            pick_target(&outputs, Some("HDMI-A-1")),
            (Target::Screen("HDMI-A-1".into()), None)
        );
        assert_eq!(pick_target(&[], None), (Target::ActiveScreen, None));
    }

    fn pipe_with(bytes: Vec<u8>, close: bool) -> (OwnedFd, std::thread::JoinHandle<()>) {
        let (reader, mut writer) = std::io::pipe().unwrap();
        let handle = std::thread::spawn(move || {
            use std::io::Write as _;
            // Several writes, as KWin's worker does for a big picture.
            for chunk in bytes.chunks(7) {
                writer.write_all(chunk).unwrap();
            }
            if !close {
                std::thread::sleep(Duration::from_millis(300));
            }
        });
        (OwnedFd::from(reader), handle)
    }

    #[test]
    fn the_pipe_is_read_to_exactly_the_announced_length() {
        let data: Vec<u8> = (0..100u8).collect();
        let (fd, writer) = pipe_with(data.clone(), true);
        let got = read_exactly(fd, 100, Instant::now() + Duration::from_secs(2)).unwrap();
        writer.join().unwrap();
        assert_eq!(got, data);
    }

    #[test]
    fn a_short_or_long_pipe_is_an_error() {
        let (fd, writer) = pipe_with(vec![1; 50], true);
        let short = read_exactly(fd, 100, Instant::now() + Duration::from_secs(2));
        writer.join().unwrap();
        assert!(matches!(short, Err(KwinError::BadReply(e)) if e.contains("50 of 100")));
        let (fd, writer) = pipe_with(vec![1; 101], true);
        let long = read_exactly(fd, 100, Instant::now() + Duration::from_secs(2));
        writer.join().unwrap();
        assert!(matches!(long, Err(KwinError::BadReply(e)) if e.contains("more than")));
    }

    #[test]
    fn a_silent_pipe_times_out_and_a_full_one_needs_its_eof() {
        // Nothing written, writer still open: the deadline ends the read.
        let (reader, writer) = std::io::pipe().unwrap();
        let started = Instant::now();
        let got = read_exactly(
            OwnedFd::from(reader),
            10,
            Instant::now() + Duration::from_millis(100),
        );
        assert_eq!(got, Err(KwinError::Timeout));
        assert!(started.elapsed() < Duration::from_secs(1));
        drop(writer);
        // All bytes in, EOF 300 ms later: kept.
        let (fd, writer) = pipe_with(vec![3; 10], false);
        let got = read_exactly(fd, 10, Instant::now() + Duration::from_secs(2)).unwrap();
        assert_eq!(got, vec![3; 10]);
        writer.join().unwrap();
        // All bytes in, the pipe never closed: an error at the deadline.
        let (reader, mut writer) = std::io::pipe().unwrap();
        std::io::Write::write_all(&mut writer, &[5; 10]).unwrap();
        let got = read_exactly(
            OwnedFd::from(reader),
            10,
            Instant::now() + Duration::from_millis(100),
        );
        assert!(matches!(got, Err(KwinError::BadReply(e)) if e.contains("stayed open")));
        drop(writer);
    }

    /// KWin keeps its own copy of the write end and closes it after the
    /// picture. Ours is closed before the read, so EOF comes at once.
    #[test]
    fn our_write_end_is_closed_before_the_read_so_eof_comes() {
        let (reader, writer) = std::io::pipe().unwrap();
        let ours = OwnedFd::from(writer);
        let kwin_copy = ours.try_clone().unwrap();
        let kwin = std::thread::spawn(move || {
            let mut file = std::fs::File::from(kwin_copy);
            std::io::Write::write_all(&mut file, &[9; 64]).unwrap();
            // KWin's FileDescriptor closes here.
        });
        let started = Instant::now();
        let got = finish_pipe(
            OwnedFd::from(reader),
            ours,
            64,
            Instant::now() + Duration::from_secs(2),
        );
        kwin.join().unwrap();
        assert_eq!(got, Ok(vec![9; 64]));
        assert!(
            started.elapsed() < Duration::from_secs(1),
            "EOF came without waiting for the deadline"
        );
    }

    /// Pins [`GRAB_TIMEOUT`] at 2 s: a call that never answers ends then.
    #[tokio::test(start_paused = true)]
    async fn a_call_that_never_answers_ends_at_the_grab_timeout() {
        let started = tokio::time::Instant::now();
        let got = bounded(std::future::pending::<Result<(), KwinError>>()).await;
        assert_eq!(got, Err(KwinError::Timeout));
        assert_eq!(started.elapsed(), Duration::from_secs(2));
        let quick = bounded(async { Ok::<_, KwinError>(3) }).await;
        assert_eq!(quick, Ok(3));
        assert_eq!(LOCK_WAIT_TAB, GRAB_TIMEOUT, "a Tab waits at most one grab");
    }

    #[test]
    fn the_probe_pixel_lies_on_the_auto_output() {
        let outputs = [
            OutputRect {
                name: "eDP-1".into(),
                x: -1920,
                y: 200,
                width: 1920,
                height: 1080,
                real_width: 1920,
                real_height: 1080,
            },
            OutputRect {
                name: "DP-1".into(),
                x: 0,
                y: 0,
                width: 2560,
                height: 1440,
                real_width: 2560,
                real_height: 1440,
            },
        ];
        assert_eq!(
            probe_target(&outputs),
            Target::Area {
                x: 0,
                y: 0,
                width: 1,
                height: 1
            }
        );
        assert_eq!(
            probe_target(&outputs[..1]),
            Target::Area {
                x: -1920,
                y: 200,
                width: 1,
                height: 1
            },
            "(0,0) is on no output here"
        );
        assert_eq!(probe_target(&[]), Target::ActiveScreen);
    }

    fn found(
        interface: Result<u32, KwinError>,
        plasma: Option<(u32, u32, u32)>,
        nvidia: bool,
    ) -> Versions {
        Versions {
            interface,
            plasma,
            nvidia,
        }
    }

    #[test]
    fn the_render_gate_has_one_answer_per_case() {
        let up = |plasma, nvidia| found(Ok(5), plasma, nvidia);
        let down = |plasma| found(Err(KwinError::NotKwin), plasma, true);
        let full = Some((6, 8, 1));
        let partial = Some((6, 8, 0));
        // A kept result wins over everything, and renders nothing.
        assert_eq!(render_gate(true, &down(None), true), RenderGate::Cached);
        assert_eq!(
            render_gate(true, &up(partial, true), true),
            RenderGate::Cached
        );
        // The interface did not answer.
        assert!(
            matches!(render_gate(false, &down(full), false), RenderGate::Refused(r) if r.contains("not KDE"))
        );
        assert!(matches!(
            render_gate(true, &down(full), false),
            RenderGate::Refused(_)
        ));
        // 6.8.1 and later render, share or not, NVIDIA or not.
        for nvidia in [true, false] {
            for share in [true, false] {
                assert_eq!(
                    render_gate(share, &up(full, nvidia), false),
                    RenderGate::Render
                );
                assert_eq!(
                    render_gate(share, &up(Some((6, 9, 0)), nvidia), false),
                    RenderGate::Render
                );
            }
        }
        // S5-CAP-3: with NVIDIA a known Plasma before 6.8.1 is unavailable,
        // share or not (another app's share can crash KWin there).
        for share in [true, false] {
            for old in [partial, Some((6, 7, 6)), Some((6, 6, 7)), Some((6, 5, 0))] {
                let got = render_gate(share, &up(old, true), false);
                assert!(
                    matches!(&got, RenderGate::Refused(r) if r.starts_with(NEEDS_FULL_FIX)),
                    "{old:?} share={share}: {got:?}"
                );
            }
        }
        let why = match render_gate(false, &up(partial, true), false) {
            RenderGate::Refused(why) => why,
            other => panic!("{other:?}"),
        };
        assert_eq!(
            why,
            "it needs Plasma 6.8.1 or newer on NVIDIA (found 6.8.0, KDE bug 524840)"
        );
        // Without NVIDIA the rc.7 rule: not beside the tracker's share
        // unless 6.8.1 is known, any Plasma without one.
        assert!(
            matches!(render_gate(true, &up(partial, false), false), RenderGate::NotChecked(r) if r.contains("6.8.0"))
        );
        assert_eq!(
            render_gate(false, &up(partial, false), false),
            RenderGate::Render
        );
        // An unknown version: not beside the tracker's share, NVIDIA or not.
        for nvidia in [true, false] {
            assert!(
                matches!(render_gate(true, &up(None, nvidia), false), RenderGate::NotChecked(r) if r.contains("unknown"))
            );
            assert_eq!(
                render_gate(false, &up(None, nvidia), false),
                RenderGate::Render
            );
        }
    }

    #[test]
    fn nvidia_is_seen_from_either_kernel_module() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!nvidia_loaded_in(dir.path()), "no module: AMD or Intel");
        std::fs::create_dir(dir.path().join("amdgpu")).unwrap();
        assert!(!nvidia_loaded_in(dir.path()));
        std::fs::create_dir(dir.path().join("nvidia_drm")).unwrap();
        assert!(nvidia_loaded_in(dir.path()));
        let dir = tempfile::tempdir().unwrap();
        std::fs::create_dir(dir.path().join("nvidia")).unwrap();
        assert!(nvidia_loaded_in(dir.path()));
    }

    /// A fake KWin for the prober: fixed versions, scripted render results.
    struct FakeBus {
        versions: Versions,
        renders: std::sync::Mutex<std::collections::VecDeque<Result<(), KwinError>>>,
        rendered: std::sync::atomic::AtomicUsize,
    }

    impl FakeBus {
        fn new(
            interface: Result<u32, KwinError>,
            plasma: Option<(u32, u32, u32)>,
            renders: Vec<Result<(), KwinError>>,
        ) -> Self {
            Self {
                // NVIDIA, where the KDE bug 524840 rule bites.
                versions: found(interface, plasma, true),
                renders: std::sync::Mutex::new(renders.into()),
                rendered: std::sync::atomic::AtomicUsize::new(0),
            }
        }

        fn rendered(&self) -> usize {
            self.rendered.load(std::sync::atomic::Ordering::SeqCst)
        }
    }

    impl ProbeBus for FakeBus {
        fn versions(&self) -> impl std::future::Future<Output = Versions> + Send {
            let found = self.versions.clone();
            async move { found }
        }

        fn render(&self) -> impl std::future::Future<Output = Result<(), KwinError>> + Send {
            self.rendered
                .fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            let next = self.renders.lock().unwrap().pop_front().unwrap_or(Ok(()));
            async move { next }
        }
    }

    #[tokio::test]
    async fn the_prober_never_renders_beside_a_share_without_the_full_fix() {
        let prober = Prober::new();
        let bus = FakeBus::new(Ok(5), None, vec![]);
        let answer = prober.probe(&bus, true).await;
        assert!(
            matches!(answer.probe, KwinProbe::NotChecked(_)),
            "{answer:?}"
        );
        assert_eq!(
            bus.rendered(),
            0,
            "no render beside the share, version unknown"
        );
        assert_eq!(prober.last().await, None, "nothing kept");
        // With the share closed it renders, and the answer is kept.
        assert_eq!(prober.probe(&bus, false).await.probe, KwinProbe::Available);
        assert_eq!(bus.rendered(), 1);
        assert_eq!(prober.probe(&bus, true).await.probe, KwinProbe::Available);
        assert_eq!(bus.rendered(), 1, "the kept answer renders nothing");
        // S5-CAP-3: 6.8.0 is unavailable, share or not, and never renders.
        let prober = Prober::new();
        let bus = FakeBus::new(Ok(5), Some((6, 8, 0)), vec![]);
        for share in [false, true] {
            let answer = prober.probe(&bus, share).await;
            assert!(
                matches!(&answer.probe, KwinProbe::Unavailable(r) if r.starts_with(NEEDS_FULL_FIX)),
                "{answer:?}"
            );
            assert!(!answer.transient, "a Plasma version does not pass");
        }
        assert_eq!(bus.rendered(), 0, "6.8.0 never renders");
        // Without NVIDIA 6.8.0 keeps the rc.7 rule: not beside the share,
        // and it renders with the share closed.
        let prober = Prober::new();
        let mut bus = FakeBus::new(Ok(5), Some((6, 8, 0)), vec![]);
        bus.versions.nvidia = false;
        assert!(matches!(
            prober.probe(&bus, true).await.probe,
            KwinProbe::NotChecked(_)
        ));
        assert_eq!(bus.rendered(), 0);
        assert_eq!(prober.probe(&bus, false).await.probe, KwinProbe::Available);
        assert_eq!(bus.rendered(), 1);
        // 6.8.1 renders beside the share.
        let prober = Prober::new();
        let bus = FakeBus::new(Ok(5), Some((6, 8, 1)), vec![]);
        assert_eq!(prober.probe(&bus, true).await.probe, KwinProbe::Available);
        assert_eq!(bus.rendered(), 1);
        // No interface: unavailable, no render, may pass.
        let prober = Prober::new();
        let bus = FakeBus::new(Err(KwinError::NotKwin), None, vec![]);
        let answer = prober.probe(&bus, false).await;
        assert!(matches!(answer.probe, KwinProbe::Unavailable(_)));
        assert!(answer.transient);
        assert_eq!(bus.rendered(), 0);
    }

    #[tokio::test(start_paused = true)]
    async fn failures_that_can_pass_are_not_kept_and_refusals_expire() {
        // A timeout is not kept: the next probe renders again.
        let prober = Prober::new();
        let bus = FakeBus::new(
            Ok(5),
            Some((6, 8, 1)),
            vec![Err(KwinError::Timeout), Ok(())],
        );
        let first = prober.probe(&bus, false).await;
        assert!(first.transient);
        assert_eq!(prober.last().await, None);
        assert_eq!(prober.probe(&bus, false).await.probe, KwinProbe::Available);
        assert_eq!(bus.rendered(), 2);
        // A refusal is kept for a minute, then asked again. (The version
        // is unread here: a known Plasma before 6.8.1 renders nothing.)
        let prober = Prober::new();
        let bus = FakeBus::new(
            Ok(4),
            None,
            vec![Err(KwinError::NotAuthorized), Err(KwinError::NotAuthorized)],
        );
        let refused = prober.probe(&bus, false).await;
        assert!(!refused.transient);
        assert!(matches!(refused.probe, KwinProbe::Unavailable(_)));
        tokio::time::advance(Duration::from_secs(59)).await;
        prober.probe(&bus, false).await;
        assert_eq!(bus.rendered(), 1, "kept inside the minute");
        tokio::time::advance(Duration::from_secs(2)).await;
        prober.probe(&bus, false).await;
        assert_eq!(bus.rendered(), 2, "asked again after it");
    }

    #[tokio::test(start_paused = true)]
    async fn the_startup_probe_tries_again_while_kwin_is_not_up_yet() {
        let prober = Prober::new();
        let bus = FakeBus::new(
            Ok(5),
            Some((6, 8, 1)),
            vec![
                Err(KwinError::Timeout),
                Err(KwinError::NoBus("x".into())),
                Ok(()),
            ],
        );
        let started = tokio::time::Instant::now();
        let got = probe_with_retries(&prober, &bus, || false, &STARTUP_RETRY_WAITS).await;
        assert_eq!(got, KwinProbe::Available);
        assert_eq!(bus.rendered(), 3);
        assert_eq!(
            started.elapsed(),
            Duration::from_secs(3),
            "after the 1 s and 2 s waits"
        );
        // A refusal is final: no retry.
        let prober = Prober::new();
        let bus = FakeBus::new(Ok(4), None, vec![Err(KwinError::NotAuthorized)]);
        let started = tokio::time::Instant::now();
        let got = probe_with_retries(&prober, &bus, || false, &STARTUP_RETRY_WAITS).await;
        assert!(matches!(got, KwinProbe::Unavailable(_)));
        assert_eq!(started.elapsed(), Duration::ZERO);
        // Off KDE there are no waits.
        let prober = Prober::new();
        let bus = FakeBus::new(Err(KwinError::NotKwin), None, vec![]);
        let started = tokio::time::Instant::now();
        probe_with_retries(&prober, &bus, || false, &[]).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[test]
    fn settings_status_without_a_render() {
        let at = |interface, plasma| found(interface, plasma, true);
        assert_eq!(status_from_versions(&at(Ok(5), None)), KwinProbe::NotProbed);
        assert_eq!(
            status_from_versions(&at(Ok(5), Some((6, 8, 1)))),
            KwinProbe::NotProbed
        );
        assert!(matches!(
            status_from_versions(&at(Ok(5), Some((6, 8, 0)))),
            KwinProbe::Unavailable(r) if r.starts_with(NEEDS_FULL_FIX)
        ));
        assert!(matches!(
            status_from_versions(&at(Err(KwinError::NotKwin), None)),
            KwinProbe::Unavailable(r) if r.contains("not KDE")
        ));
        assert!(KwinError::NotAuthorized.is_definitive());
        assert!(KwinError::EmptyReply.is_definitive());
        assert!(
            !KwinError::NotKwin.is_definitive(),
            "KWin may not be up yet"
        );
        assert!(!KwinError::Timeout.is_definitive());
        assert!(super::super::flight::is_busy(&KwinError::Busy.to_string()));
    }

    #[tokio::test]
    async fn a_broken_connection_is_made_again_and_others_are_kept() {
        let cache: Reconnecting<u32> = Reconnecting::new();
        let made = std::sync::atomic::AtomicU32::new(0);
        let make = || async {
            Ok::<_, KwinError>(made.fetch_add(1, std::sync::atomic::Ordering::SeqCst) + 1)
        };
        assert_eq!(cache.get_or_make(make).await, Ok(1));
        assert_eq!(cache.get_or_make(make).await, Ok(1), "kept");
        // A timeout or a refusal says nothing about the connection.
        cache.forget_after(&KwinError::Timeout).await;
        cache.forget_after(&KwinError::NotAuthorized).await;
        assert_eq!(cache.get_or_make(make).await, Ok(1), "still kept");
        // No bus: the next use connects again.
        cache.forget_after(&KwinError::NoBus("closed".into())).await;
        assert_eq!(cache.get_or_make(make).await, Ok(2), "made again");
        // A failed make keeps nothing.
        let empty: Reconnecting<u32> = Reconnecting::new();
        let failed = empty
            .get_or_make(|| async { Err::<u32, _>(KwinError::NoBus("x".into())) })
            .await;
        assert!(failed.is_err());
        assert_eq!(empty.get_or_make(make).await, Ok(3));
    }

    #[tokio::test(start_paused = true)]
    async fn off_kde_the_startup_probe_does_not_try_again() {
        let prober = Prober::new();
        let bus = FakeBus::new(
            Ok(5),
            Some((6, 8, 1)),
            vec![Err(KwinError::Timeout), Ok(())],
        );
        let started = tokio::time::Instant::now();
        let got = probe_at_startup_with(&prober, &bus, || false, false).await;
        assert!(matches!(got, KwinProbe::Unavailable(_)), "{got:?}");
        assert_eq!(bus.rendered(), 1, "one try");
        assert_eq!(started.elapsed(), Duration::ZERO, "no wait");
        // On KDE the same failure is tried again.
        let prober = Prober::new();
        let bus = FakeBus::new(
            Ok(5),
            Some((6, 8, 1)),
            vec![Err(KwinError::Timeout), Ok(())],
        );
        assert_eq!(
            probe_at_startup_with(&prober, &bus, || false, true).await,
            KwinProbe::Available
        );
        assert_eq!(bus.rendered(), 2);
    }

    #[tokio::test]
    async fn a_lasting_size_mismatch_rereads_at_most_once_a_minute() {
        let t0 = Instant::now();
        assert!(native_reread_due(None, t0));
        assert!(!native_reread_due(Some(t0), t0 + Duration::from_secs(59)));
        assert!(native_reread_due(Some(t0), t0 + NATIVE_REREAD_GAP));
        let calls = std::cell::Cell::new(0);
        let got = check_native_size((3200, 1800), Some((2560, 1440)), false, || {
            calls.set(calls.get() + 1);
            async { Some((2560, 1440)) }
        })
        .await;
        assert_eq!(got, SizeCheck::DiffersRereadSkipped);
        assert_eq!(calls.get(), 0, "not read again inside the gap");
        // A matching size never reads, due or not.
        let got = check_native_size((2560, 1440), Some((2560, 1440)), false, || {
            calls.set(calls.get() + 1);
            async { None }
        })
        .await;
        assert_eq!((got, calls.get()), (SizeCheck::Matches, 0));
    }

    #[test]
    fn the_compare_half_needs_the_full_fix_even_after_an_available_probe() {
        let at = |plasma| found(Ok(5), plasma, true);
        // A kept "available" does not let 6.8.0 through, NVIDIA or not.
        let why = compare_half_allowed(&at(Some((6, 8, 0))), &KwinProbe::Available).unwrap_err();
        assert!(why.contains("6.8.1"), "{why}");
        let amd = found(Ok(5), Some((6, 8, 0)), false);
        assert!(compare_half_allowed(&amd, &KwinProbe::Available).is_err());
        assert!(compare_half_allowed(&at(None), &KwinProbe::Available).is_err());
        assert_eq!(
            compare_half_allowed(&at(Some((6, 8, 1))), &KwinProbe::Available),
            Ok(())
        );
        assert!(
            compare_half_allowed(&at(Some((6, 8, 1))), &KwinProbe::NotChecked("x".into())).is_err()
        );
        let down = found(Err(KwinError::NotKwin), None, true);
        assert!(compare_half_allowed(&down, &KwinProbe::Available).is_err());
    }

    /// A share that opens during a retry wait is seen by the next attempt:
    /// with the Plasma version unknown that attempt renders nothing.
    #[tokio::test(start_paused = true)]
    async fn each_startup_retry_looks_at_the_share_again() {
        let prober = Prober::new();
        let bus = FakeBus::new(Ok(5), None, vec![Err(KwinError::Timeout), Ok(())]);
        let looks = std::sync::atomic::AtomicUsize::new(0);
        // Closed for the first attempt, open from the second.
        let share_open = || looks.fetch_add(1, std::sync::atomic::Ordering::SeqCst) > 0;
        let got = probe_at_startup_with(&prober, &bus, share_open, true).await;
        assert!(matches!(got, KwinProbe::NotChecked(_)), "{got:?}");
        assert_eq!(
            bus.rendered(),
            1,
            "only the attempt with the share closed rendered"
        );
        assert_eq!(looks.load(std::sync::atomic::Ordering::SeqCst), 2);
    }

    /// A bus that never finishes the handshake ends the connect after
    /// [`CONNECT_TIMEOUT`]; nothing is kept, and the next use tries again.
    #[tokio::test(start_paused = true)]
    async fn a_connect_that_never_answers_times_out_and_is_tried_again() {
        let cache: Reconnecting<u32> = Reconnecting::new();
        let started = tokio::time::Instant::now();
        // The outer timeout only keeps a broken build from hanging here.
        let hung = tokio::time::timeout(
            Duration::from_secs(60),
            cache.get_or_make(std::future::pending::<Result<u32, KwinError>>),
        )
        .await
        .expect("the connect gave up on its own");
        assert_eq!(hung, Err(KwinError::Timeout));
        assert_eq!(started.elapsed(), CONNECT_TIMEOUT);
        assert_eq!(CONNECT_TIMEOUT, Duration::from_secs(2));
        assert_eq!(cache.get_or_make(|| async { Ok(9) }).await, Ok(9));
    }
}
