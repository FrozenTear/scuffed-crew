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
//! interface (`Error.NoAuthorized`). Plasma 6.8.0 dropped that check. rc.7
//! uses this route only when a 1x1 probe succeeds without a grant.
//!
//! KDE bug 524840: KWin could crash on NVIDIA when a screenshot met an
//! active screencast. Fixed in 6.6.7, 6.7.6 and 6.8.0, with a further fix
//! in 6.8.1. The portal and this route share that code path. This route
//! never runs beside the tracker's own screen stream, and every call goes
//! through one lock ([`FLIGHT`]) so two of ours never overlap.

use std::collections::HashMap;
use std::os::fd::{AsRawFd, OwnedFd};
use std::time::{Duration, Instant};

use ashpd::zbus;
use ashpd::zvariant::{self, OwnedValue, Value};
use image::{DynamicImage, RgbImage};

use super::portal::OutputRect;
use super::route::KwinProbe;

pub const SERVICE: &str = "org.kde.KWin.ScreenShot2";
pub const PATH: &str = "/org/kde/KWin/ScreenShot2";
pub const INTERFACE: &str = "org.kde.KWin.ScreenShot2";

/// The whole grab, call and pipe read, ends within this.
pub const GRAB_TIMEOUT: Duration = Duration::from_secs(2);
/// How long a grab waits for the call before it.
pub const LOCK_WAIT: Duration = Duration::from_secs(2);

/// Plasma version with the full fix for KDE bug 524840.
pub const CRASH_FIX_FULL: (u32, u32, u32) = (6, 8, 1);

static FLIGHT: super::flight::SingleFlight = super::flight::SingleFlight::new();

// ------------------------------------------------------------------ errors

/// Why a KWin call gave no picture.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum KwinError {
    /// No session bus.
    NoBus(String),
    /// Nothing answers `org.kde.KWin.ScreenShot2`: not KDE Plasma (or KWin
    /// without its screenshot plugin).
    NotKwin,
    /// KWin refused the caller: Plasma before 6.8 without a `.desktop` grant.
    NotAuthorized,
    /// KWin answered with no picture. Plasma 6.6 and 6.7 do that when they
    /// cannot tell who called, which is a refusal too.
    EmptyReply,
    /// KWin could not render the picture (`Error.Cancelled`).
    Cancelled,
    /// The output name is not one KWin knows (`Error.InvalidScreen`).
    InvalidScreen,
    /// `Error.InvalidArea` or `Error.FileDescriptor`.
    Rejected(String),
    /// The reply or the pixels do not fit together.
    BadReply(String),
    /// No answer within [`GRAB_TIMEOUT`].
    Timeout,
    /// The call before this one was still running after [`LOCK_WAIT`].
    Busy,
    /// Anything else, with the D-Bus error.
    Other(String),
}

impl KwinError {
    /// The reason for the log and the app, in a few words.
    pub fn reason(&self) -> String {
        match self {
            KwinError::NoBus(err) => format!("no D-Bus session bus ({err})"),
            KwinError::NotKwin => {
                "KWin's screenshot interface is not on the session bus (not KDE Plasma)".into()
            }
            KwinError::NotAuthorized | KwinError::EmptyReply => {
                "KWin refused the call; Plasma before 6.8 only lets apps it lists take screenshots"
                    .into()
            }
            KwinError::Cancelled => "KWin could not render the screenshot".into(),
            KwinError::InvalidScreen => "KWin does not know that monitor".into(),
            KwinError::Rejected(err) => format!("KWin rejected the request ({err})"),
            KwinError::BadReply(err) => format!("KWin's answer did not make sense ({err})"),
            KwinError::Timeout => format!("KWin did not answer in {} s", GRAB_TIMEOUT.as_secs()),
            KwinError::Busy => "the previous KWin screenshot was still running".into(),
            KwinError::Other(err) => err.clone(),
        }
    }

    /// An error an output change can explain: the cached geometry is
    /// dropped so the next grab reads it again.
    pub fn invalidates_geometry(&self) -> bool {
        matches!(self, KwinError::InvalidScreen | KwinError::Rejected(_))
    }
}

impl std::fmt::Display for KwinError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "KDE direct capture: {}", self.reason())
    }
}

impl std::error::Error for KwinError {}

/// A D-Bus error name (and its message) as a [`KwinError`].
pub fn classify_error(name: &str, detail: Option<&str>) -> KwinError {
    let detail = detail.unwrap_or("").trim();
    match name {
        "org.freedesktop.DBus.Error.ServiceUnknown"
        | "org.freedesktop.DBus.Error.NameHasNoOwner"
        | "org.freedesktop.DBus.Error.UnknownObject"
        | "org.freedesktop.DBus.Error.UnknownInterface"
        | "org.freedesktop.DBus.Error.UnknownMethod"
        | "org.freedesktop.DBus.Error.UnknownProperty" => KwinError::NotKwin,
        "org.kde.KWin.ScreenShot2.Error.NoAuthorized"
        | "org.freedesktop.DBus.Error.AccessDenied" => KwinError::NotAuthorized,
        "org.kde.KWin.ScreenShot2.Error.Cancelled" => KwinError::Cancelled,
        "org.kde.KWin.ScreenShot2.Error.InvalidScreen" => KwinError::InvalidScreen,
        "org.kde.KWin.ScreenShot2.Error.InvalidArea"
        | "org.kde.KWin.ScreenShot2.Error.FileDescriptor" => {
            KwinError::Rejected(name.rsplit('.').next().unwrap_or(name).to_string())
        }
        "org.freedesktop.DBus.Error.NoReply" | "org.freedesktop.DBus.Error.Timeout" => {
            KwinError::Timeout
        }
        _ if detail.is_empty() => KwinError::Other(name.to_string()),
        _ => KwinError::Other(format!("{name}: {detail}")),
    }
}

fn from_zbus(err: zbus::Error) -> KwinError {
    match err {
        zbus::Error::MethodError(name, detail, _) => {
            classify_error(name.as_str(), detail.as_deref())
        }
        zbus::Error::InputOutput(err) => KwinError::NoBus(err.to_string()),
        zbus::Error::Address(err) => KwinError::NoBus(err),
        other => KwinError::Other(other.to_string()),
    }
}

/// What the probe's result means for the route.
pub fn probe_outcome(result: &Result<(), KwinError>) -> KwinProbe {
    match result {
        Ok(()) => KwinProbe::Available,
        Err(err) => KwinProbe::Unavailable(err.reason()),
    }
}

// ---------------------------------------------------------------- metadata

/// The reply's description of the raw pixels.
#[derive(Debug, Clone, PartialEq)]
pub struct RawMeta {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    /// A `QImage::Format` value.
    pub format: u32,
    pub scale: Option<f64>,
    /// The output, for a screen capture.
    pub screen: Option<String>,
}

impl RawMeta {
    /// Bytes KWin writes into the pipe.
    pub fn byte_len(&self) -> Option<usize> {
        (self.stride as usize).checked_mul(self.height as usize)
    }
}

fn value_u32(value: &Value<'_>) -> Option<u32> {
    match value {
        Value::U32(v) => Some(*v),
        Value::I32(v) => u32::try_from(*v).ok(),
        Value::U64(v) => u32::try_from(*v).ok(),
        Value::I64(v) => u32::try_from(*v).ok(),
        Value::U16(v) => Some(u32::from(*v)),
        Value::Value(inner) => value_u32(inner),
        _ => None,
    }
}

fn value_f64(value: &Value<'_>) -> Option<f64> {
    match value {
        Value::F64(v) => Some(*v),
        Value::Value(inner) => value_f64(inner),
        other => value_u32(other).map(f64::from),
    }
}

fn value_str(value: &Value<'_>) -> Option<String> {
    match value {
        Value::Str(s) => Some(s.as_str().to_string()),
        Value::Value(inner) => value_str(inner),
        _ => None,
    }
}

/// Reads the reply map. An empty map is [`KwinError::EmptyReply`] (Plasma
/// 6.6 and 6.7 answer a caller they cannot identify that way). A type
/// other than `raw`, or a size that does not add up, is a bad reply.
pub fn parse_meta(map: &HashMap<String, OwnedValue>) -> Result<RawMeta, KwinError> {
    if map.is_empty() {
        return Err(KwinError::EmptyReply);
    }
    let get = |key: &str| map.get(key).map(|v| &**v);
    if let Some(kind) = get("type").and_then(value_str)
        && kind != "raw"
    {
        return Err(KwinError::BadReply(format!("type {kind}, not raw")));
    }
    let need = |key: &str| {
        get(key)
            .and_then(value_u32)
            .ok_or_else(|| KwinError::BadReply(format!("no {key}")))
    };
    let meta = RawMeta {
        width: need("width")?,
        height: need("height")?,
        stride: need("stride")?,
        format: need("format")?,
        scale: get("scale").and_then(value_f64),
        screen: get("screen").and_then(value_str),
    };
    if meta.width == 0 || meta.height == 0 {
        return Err(KwinError::BadReply("empty picture".into()));
    }
    let Some(row) = meta.width.checked_mul(4) else {
        return Err(KwinError::BadReply("width too large".into()));
    };
    if meta.stride < row {
        return Err(KwinError::BadReply(format!(
            "stride {} is shorter than a {}-pixel row",
            meta.stride, meta.width
        )));
    }
    if pixel_order(meta.format).is_none() {
        return Err(KwinError::BadReply(format!(
            "pixel format {} is not handled",
            meta.format
        )));
    }
    if meta.byte_len().is_none_or(|len| len > MAX_PIXEL_BYTES) {
        return Err(KwinError::BadReply("picture too large".into()));
    }
    Ok(meta)
}

/// 16384 x 8192 at 4 bytes a pixel. Far above any monitor; a bound on what
/// a reply can make the tracker allocate.
const MAX_PIXEL_BYTES: usize = 16384 * 8192 * 4;

// ------------------------------------------------------------------ pixels

/// Byte order of one 4-byte pixel in memory.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PixelOrder {
    /// B, G, R, A/X: `QImage::Format_RGB32`, `ARGB32` and
    /// `ARGB32_Premultiplied` on a little-endian machine.
    Bgra,
    /// A/X, R, G, B: the same formats on a big-endian machine.
    Argb,
    /// R, G, B, A/X: `Format_RGBX8888`, `RGBA8888` and
    /// `RGBA8888_Premultiplied`, on any machine.
    Rgba,
}

/// The byte order of a `QImage::Format`, or `None` for one not handled.
/// Plasma 6.8 sends 16 (`RGBX8888`) for area and screen captures; 6.5 to
/// 6.7 send 6 (`ARGB32_Premultiplied`); older ones 5 (`ARGB32`).
pub fn pixel_order(format: u32) -> Option<PixelOrder> {
    match format {
        // RGB32, ARGB32, ARGB32_Premultiplied: 32-bit 0xAARRGGBB words.
        4..=6 => Some(if cfg!(target_endian = "big") {
            PixelOrder::Argb
        } else {
            PixelOrder::Bgra
        }),
        // RGBX8888, RGBA8888, RGBA8888_Premultiplied: bytes R, G, B, A.
        16..=18 => Some(PixelOrder::Rgba),
        _ => None,
    }
}

/// The raw pixels as an RGB image, the type the screen stream hands the
/// detectors. Alpha is dropped: a screen is opaque, and a premultiplied
/// pixel's colour is already what it looks like over black. Padding past
/// each row's pixels (`stride`) is skipped.
pub fn to_rgb_image(meta: &RawMeta, data: &[u8]) -> Result<RgbImage, KwinError> {
    let order = pixel_order(meta.format)
        .ok_or_else(|| KwinError::BadReply(format!("pixel format {}", meta.format)))?;
    let expected = meta
        .byte_len()
        .ok_or_else(|| KwinError::BadReply("size overflows".into()))?;
    if data.len() != expected {
        return Err(KwinError::BadReply(format!(
            "{} bytes for a {}x{} picture with stride {} ({expected} expected)",
            data.len(),
            meta.width,
            meta.height,
            meta.stride
        )));
    }
    let (r, g, b) = match order {
        PixelOrder::Bgra => (2, 1, 0),
        PixelOrder::Argb => (1, 2, 3),
        PixelOrder::Rgba => (0, 1, 2),
    };
    let width = meta.width as usize;
    let row_bytes = width * 4;
    let mut out = Vec::with_capacity(width * meta.height as usize * 3);
    for row in data.chunks_exact(meta.stride as usize) {
        for px in row[..row_bytes].as_chunks::<4>().0 {
            out.extend_from_slice(&[px[r], px[g], px[b]]);
        }
    }
    RgbImage::from_raw(meta.width, meta.height, out)
        .ok_or_else(|| KwinError::BadReply("pixel buffer size".into()))
}

/// The size to resample a KWin picture of the chosen output to, or `None`
/// to keep it. Detectors are tuned on the output's real mode size, which
/// is what the screen stream delivers. `CaptureScreen` with
/// `native-resolution` renders at that output's own scale, which is the
/// mode size up to rounding. A fractional scale or a stale mode can still
/// be off by more than a pixel; then the picture goes back to the real
/// size, either way. An unknown real size (0) keeps the picture.
pub fn fit_size(width: u32, height: u32, real_width: u32, real_height: u32) -> Option<(u32, u32)> {
    if real_width == 0 || real_height == 0 {
        return None;
    }
    let off = |a: u32, b: u32| a.abs_diff(b) > 1;
    (off(width, real_width) || off(height, real_height)).then_some((real_width, real_height))
}

/// Converts and, when [`fit_size`] says so, resamples with the same
/// Lanczos3 filter the portal path uses for its cut-outs.
pub fn picture(
    meta: &RawMeta,
    data: &[u8],
    real: Option<(u32, u32)>,
) -> Result<DynamicImage, KwinError> {
    let rgb = to_rgb_image(meta, data)?;
    let (real_w, real_h) = real.unwrap_or((0, 0));
    match fit_size(rgb.width(), rgb.height(), real_w, real_h) {
        Some((w, h)) => {
            tracing::debug!(
                from_width = rgb.width(),
                from_height = rgb.height(),
                width = w,
                height = h,
                "KDE direct picture resized to the output's real size"
            );
            Ok(DynamicImage::ImageRgb8(rgb).resize_exact(
                w,
                h,
                image::imageops::FilterType::Lanczos3,
            ))
        }
        None => Ok(DynamicImage::ImageRgb8(rgb)),
    }
}

// -------------------------------------------------------------------- pipe

/// Reads exactly `len` bytes from `fd` and then expects EOF, all before
/// `deadline`. More bytes than that is an error. When all `len` bytes are
/// in but KWin has not closed its end yet, a short wait for EOF follows and
/// the picture is kept either way.
pub fn read_exactly(fd: OwnedFd, len: usize, deadline: Instant) -> Result<Vec<u8>, KwinError> {
    let raw = fd.as_raw_fd();
    // SAFETY: fcntl on an fd this function owns.
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        if flags < 0 || libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(KwinError::Other(format!(
                "pipe: {}",
                std::io::Error::last_os_error()
            )));
        }
    }
    let mut buf = vec![0u8; len];
    let mut filled = 0usize;
    let mut extra = [0u8; 1];
    loop {
        let now = Instant::now();
        let full = filled == len;
        let wait = if full {
            deadline
                .saturating_duration_since(now)
                .min(Duration::from_millis(50))
        } else {
            deadline.saturating_duration_since(now)
        };
        let target: &mut [u8] = if full { &mut extra } else { &mut buf[filled..] };
        // SAFETY: `target` is a live, writable buffer of the given length.
        let n = unsafe { libc::read(raw, target.as_mut_ptr().cast(), target.len()) };
        if n > 0 {
            if full {
                return Err(KwinError::BadReply(format!(
                    "more than the {len} bytes the reply announced"
                )));
            }
            filled += n as usize;
            continue;
        }
        if n == 0 {
            return if full {
                Ok(buf)
            } else {
                Err(KwinError::BadReply(format!(
                    "{filled} of {len} bytes before the pipe closed"
                )))
            };
        }
        let err = std::io::Error::last_os_error();
        match err.kind() {
            std::io::ErrorKind::Interrupted => continue,
            std::io::ErrorKind::WouldBlock => {}
            _ => return Err(KwinError::Other(format!("pipe read: {err}"))),
        }
        if wait.is_zero() {
            return if full {
                Ok(buf)
            } else {
                Err(KwinError::Timeout)
            };
        }
        let mut pfd = libc::pollfd {
            fd: raw,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(wait.as_millis().max(1)).unwrap_or(i32::MAX);
        // SAFETY: one pollfd for an fd this function owns.
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready == 0 && full {
            return Ok(buf);
        }
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(KwinError::Other(format!("pipe poll: {err}")));
            }
        }
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
/// other portal paths pick ([`super::auto_output_index`]); with no
/// geometry it is KWin's active screen, the one with the focused window.
pub fn pick_target(
    outputs: &[OutputRect],
    output_name: Option<&str>,
) -> (Target, Option<(u32, u32)>) {
    let real = |o: &OutputRect| Some((o.real_width, o.real_height));
    match output_name {
        Some(name) => (
            Target::Screen(name.to_string()),
            outputs.iter().find(|o| o.name == name).and_then(real),
        ),
        None => match super::auto_output_index(outputs.iter().map(|o| (o.name.as_str(), o.area())))
        {
            Some(index) => (
                Target::Screen(outputs[index].name.clone()),
                real(&outputs[index]),
            ),
            None => (Target::ActiveScreen, None),
        },
    }
}

static CONNECTION: tokio::sync::OnceCell<zbus::Connection> = tokio::sync::OnceCell::const_new();

async fn connection() -> Result<zbus::Connection, KwinError> {
    CONNECTION
        .get_or_try_init(zbus::Connection::session)
        .await
        .cloned()
        .map_err(|err| KwinError::NoBus(err.to_string()))
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
    let writer = OwnedFd::from(writer);
    let mut options: HashMap<&str, Value<'_>> = HashMap::new();
    options.insert("include-cursor", Value::Bool(false));
    options.insert("native-resolution", Value::Bool(native));
    let fd = zvariant::Fd::from(&writer);
    let reply = match target {
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
    .map_err(from_zbus)?;
    // KWin holds its own copy. Ours must go, or the read never sees EOF.
    drop(writer);
    let map: HashMap<String, OwnedValue> = reply
        .body()
        .deserialize()
        .map_err(|err| KwinError::BadReply(err.to_string()))?;
    let meta = parse_meta(&map)?;
    let len = meta
        .byte_len()
        .ok_or_else(|| KwinError::BadReply("size overflows".into()))?;
    let reader = OwnedFd::from(reader);
    let bytes = tokio::task::spawn_blocking(move || read_exactly(reader, len, deadline))
        .await
        .map_err(|err| KwinError::Other(format!("pipe read task: {err}")))??;
    Ok((meta, bytes))
}

/// One grab under [`FLIGHT`], ended by [`GRAB_TIMEOUT`].
async fn grab_in_flight(target: Target, native: bool) -> Result<(RawMeta, Vec<u8>), KwinError> {
    FLIGHT
        .run(LOCK_WAIT, async move {
            let deadline = Instant::now() + GRAB_TIMEOUT;
            let grab = async {
                let conn = connection().await?;
                grab_raw(&conn, &target, native, deadline).await
            };
            tokio::time::timeout(GRAB_TIMEOUT, grab)
                .await
                .unwrap_or(Err(KwinError::Timeout))
        })
        .await
        .map_err(|err| match err {
            super::flight::FlightError::Busy => KwinError::Busy,
            super::flight::FlightError::Task(err) => KwinError::Other(err),
        })?
}

/// The game's monitor, full output, cursor hidden, at the output's real
/// size. The pixels never touch the disk.
pub async fn capture(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let outputs = super::portal::output_geometry().await;
    let (target, real) = pick_target(&outputs, output_name);
    let (meta, bytes) = match grab_in_flight(target, true).await {
        Ok(grabbed) => grabbed,
        Err(err) => {
            if err.invalidates_geometry() {
                super::portal::invalidate_output_geometry();
            }
            return Err(match (&err, output_name) {
                (KwinError::InvalidScreen, Some(name)) => {
                    format!("output '{name}' not found: {err}").into()
                }
                _ => err.into(),
            });
        }
    };
    let img = tokio::task::spawn_blocking(move || picture(&meta, &bytes, real))
        .await
        .map_err(|err| format!("KDE direct convert task failed: {err}"))??;
    tracing::trace!(
        width = img.width(),
        height = img.height(),
        "screen captured via KWin ScreenShot2"
    );
    Ok(img)
}

// ------------------------------------------------------------------- probe

/// `KWin version: 6.8.1` from KWin's support information.
pub fn parse_kwin_version(text: &str) -> Option<(u32, u32, u32)> {
    let line = text
        .lines()
        .find_map(|line| line.trim().strip_prefix("KWin version:"))?;
    let mut parts = line.trim().split(['.', ' ', '-']);
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().and_then(|p| p.parse().ok()).unwrap_or(0);
    Some((major, minor, patch))
}

/// How far a Plasma version has the KDE bug 524840 fix.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CrashFix {
    /// Before 6.6.7, 6.7.0 to 6.7.5.
    Missing,
    /// 6.6.7+, 6.7.6+, 6.8.0: the main fix, not the 6.8.1 follow-up.
    Partial,
    /// 6.8.1 and later.
    Full,
}

pub fn crash_fix((major, minor, patch): (u32, u32, u32)) -> CrashFix {
    let v = (major, minor, patch);
    if v >= CRASH_FIX_FULL {
        CrashFix::Full
    } else if v >= (6, 8, 0)
        || (major == 6 && minor == 7 && patch >= 6)
        || (major == 6 && minor == 6 && patch >= 7)
    {
        CrashFix::Partial
    } else {
        CrashFix::Missing
    }
}

/// What KWin says about itself, read without rendering anything.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Versions {
    /// The interface's `Version` property, or why it could not be read.
    pub interface: Result<u32, KwinError>,
    /// The Plasma (KWin) version, when KWin's support information has it.
    pub plasma: Option<(u32, u32, u32)>,
}

static VERSIONS: tokio::sync::OnceCell<Versions> = tokio::sync::OnceCell::const_new();
static RENDER_PROBE: tokio::sync::OnceCell<Result<(), KwinError>> =
    tokio::sync::OnceCell::const_new();

async fn read_versions() -> Versions {
    let conn = match connection().await {
        Ok(conn) => conn,
        Err(err) => {
            return Versions {
                interface: Err(err),
                plasma: None,
            };
        }
    };
    let interface = async {
        let reply = conn
            .call_method(
                Some(SERVICE),
                PATH,
                Some("org.freedesktop.DBus.Properties"),
                "Get",
                &(INTERFACE, "Version"),
            )
            .await
            .map_err(from_zbus)?;
        let value: OwnedValue = reply
            .body()
            .deserialize()
            .map_err(|err| KwinError::BadReply(err.to_string()))?;
        value_u32(&value).ok_or_else(|| KwinError::BadReply("Version is not a number".into()))
    };
    let interface = tokio::time::timeout(GRAB_TIMEOUT, interface)
        .await
        .unwrap_or(Err(KwinError::Timeout));
    // KWin has no version property. Its support information (the text
    // `qdbus org.kde.KWin /KWin supportInformation` prints) starts with
    // `KWin version: X.Y.Z`. Read once, at startup, best effort.
    let plasma = if interface.is_ok() {
        let info = async {
            let reply = conn
                .call_method(
                    Some("org.kde.KWin"),
                    "/KWin",
                    Some("org.kde.KWin"),
                    "supportInformation",
                    &(),
                )
                .await
                .ok()?;
            let text: String = reply.body().deserialize().ok()?;
            parse_kwin_version(&text)
        };
        tokio::time::timeout(GRAB_TIMEOUT, info)
            .await
            .ok()
            .flatten()
    } else {
        None
    };
    Versions { interface, plasma }
}

/// [`Versions`], read once per process.
pub async fn versions() -> Versions {
    VERSIONS.get_or_init(read_versions).await.clone()
}

fn version_label(v: Option<(u32, u32, u32)>) -> String {
    v.map_or_else(|| "unknown".to_string(), |(a, b, c)| format!("{a}.{b}.{c}"))
}

async fn render_probe() -> Result<(), KwinError> {
    versions().await.interface?;
    let (meta, bytes) = grab_in_flight(
        Target::Area {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        },
        false,
    )
    .await?;
    to_rgb_image(&meta, &bytes).map(|_| ())
}

/// The startup probe: the `Version` property, then a 1x1 `CaptureArea`.
/// Run once per process and logged once. Must not run beside the
/// tracker's own screen stream on a Plasma without the full crash fix;
/// [`probe_beside_share`] checks that.
pub async fn probe() -> KwinProbe {
    let result = RENDER_PROBE
        .get_or_init(|| async {
            let result = render_probe().await;
            let found = versions().await;
            let plasma = version_label(found.plasma);
            match &result {
                Ok(()) => {
                    tracing::info!(
                        interface_version = found.interface.as_ref().ok().copied().unwrap_or(0),
                        plasma = %plasma,
                        "KDE direct capture available (KWin ScreenShot2)"
                    );
                    if let Some(v) = found.plasma
                        && crash_fix(v) != CrashFix::Full
                    {
                        tracing::warn!(
                            plasma = %plasma,
                            "KWin before 6.8.1 can crash on NVIDIA when a screenshot meets another app's screen share (KDE bug 524840)"
                        );
                    }
                }
                // The route logs the one warning a user sees (`resolve`'s
                // fallback line); this is for a debug log.
                Err(err) => {
                    tracing::debug!(plasma = %plasma, "KDE direct probe failed: {}", err.reason());
                }
            }
            result
        })
        .await;
    probe_outcome(result)
}

/// [`probe`] for a process that may run beside the tracker's screen
/// stream (the app's Settings, the debug compare pair). With
/// `share_open`, the 1x1 render runs only on Plasma 6.8.1 or later: the
/// version is read first, which renders nothing.
pub async fn probe_beside_share(share_open: bool) -> KwinProbe {
    if share_open && RENDER_PROBE.get().is_none() {
        let found = versions().await;
        if let Err(err) = &found.interface {
            return KwinProbe::Unavailable(err.reason());
        }
        if found.plasma.is_none_or(|v| crash_fix(v) != CrashFix::Full) {
            return KwinProbe::Unavailable(format!(
                "not checked while the screen stream is open on Plasma {} (needs 6.8.1, KDE bug 524840)",
                version_label(found.plasma)
            ));
        }
    }
    probe().await
}

/// The KDE direct half of a debug compare pair: the same Tab moment as the
/// stream frame, saved as `NNNN-kwin.png`. KDE only. It runs beside the
/// open stream, so it needs Plasma 6.8.1 (see [`probe_beside_share`]).
pub(crate) async fn save_compare_half(dir: &std::path::Path, n: u32, output_name: Option<&str>) {
    let desktop = std::env::var("XDG_CURRENT_DESKTOP").unwrap_or_default();
    if !super::route::is_kde(&desktop) {
        return;
    }
    if let KwinProbe::Unavailable(reason) = probe_beside_share(true).await {
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::info!("KDE direct half of the compare pair skipped: {reason}");
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
    fn a_picture_off_the_real_mode_is_resampled_to_it() {
        // Same size, or within a pixel: kept.
        assert_eq!(fit_size(2560, 1440, 2560, 1440), None);
        assert_eq!(fit_size(2561, 1440, 2560, 1440), None);
        // Unknown real size: kept.
        assert_eq!(fit_size(3200, 1800, 0, 0), None);
        // 1440p panel next to a 2x 4K monitor, CaptureArea-style max scale:
        // too large, back to the mode.
        assert_eq!(fit_size(5120, 2880, 2560, 1440), Some((2560, 1440)));
        // Fractional rounding the other way: up to the mode.
        assert_eq!(fit_size(2558, 1438, 2560, 1440), Some((2560, 1440)));
    }

    #[test]
    fn a_synthetic_mixed_scale_reply_reaches_the_detectors_at_the_mode_size() {
        // A 4x2 reply for an output whose real mode is 2x1.
        let m = meta(4, 2, 16, 16);
        let data: Vec<u8> = (0..8).flat_map(|_| [200u8, 10, 20, 255]).collect();
        let img = picture(&m, &data, Some((2, 1))).unwrap();
        assert_eq!((img.width(), img.height()), (2, 1));
        let px = img.to_rgb8().get_pixel(0, 0).0;
        assert_eq!(px, [200, 10, 20], "a flat colour survives the resample");
        // At the right size: not touched, same pixels.
        let img = picture(&m, &data, Some((4, 2))).unwrap();
        assert_eq!(img.to_rgb8().into_raw(), [200, 10, 20].repeat(8));
        assert!(
            matches!(img, DynamicImage::ImageRgb8(_)),
            "RGB, as the stream"
        );
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
    fn a_pipe_that_stays_silent_times_out_and_a_full_one_is_kept() {
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
        // All bytes in, EOF late: the picture is kept after a short wait.
        let (fd, writer) = pipe_with(vec![3; 10], false);
        let got = read_exactly(fd, 10, Instant::now() + Duration::from_secs(2)).unwrap();
        assert_eq!(got, vec![3; 10]);
        writer.join().unwrap();
    }
}
