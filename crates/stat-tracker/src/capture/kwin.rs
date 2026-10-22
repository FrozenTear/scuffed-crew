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
//! and this route share that code path. This route never runs beside the
//! tracker's own screen stream; the one exception, the debug compare pair,
//! and the app's probe while the tracker shares the screen, need Plasma
//! 6.8.1 ([`render_gate`]). Every call in this process goes through one
//! lock ([`FLIGHT`]), so two of ours never overlap here. The app and the
//! daemon are two processes and can call at the same moment; KWin runs
//! those one after the other on its own thread.

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
/// How long a Tab grab (or the probe) waits for the call before it, at
/// most one [`GRAB_TIMEOUT`]. A poll does not wait.
pub const LOCK_WAIT_TAB: Duration = Duration::from_secs(2);

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
    /// KWin answered with no picture. Plasma 6.0 to 6.7 do that when they
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
    /// The call before this one was still running after the lock wait.
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
            KwinError::Busy => super::flight::BUSY.into(),
            KwinError::Other(err) => err.clone(),
        }
    }

    /// A refusal that will not change while this Plasma runs. Every other
    /// failure may pass (KWin not on the bus yet at login, a timeout).
    pub fn is_definitive(&self) -> bool {
        matches!(self, KwinError::NotAuthorized | KwinError::EmptyReply)
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
/// 6.0 to 6.7 answer a caller they cannot identify that way). A type
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
/// 6.7 send 6 (`ARGB32_Premultiplied`); 6.0 to 6.4 send 6 for an area and
/// 5 (`ARGB32`) for a screen.
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
}

/// Checks a KDE direct picture against the output's cached real size.
/// `CaptureScreen` renders at that output's own scale, which is the
/// (rotated) mode size on Plasma 6.5 and later, so a difference means the
/// cached geometry is stale: `reread` drops and reads it again, once. If it
/// still differs, the picture is kept as KWin rendered it. It is never
/// resampled: any resample changes what the readers see.
pub async fn check_native_size<F, Fut>(
    size: (u32, u32),
    real: Option<(u32, u32)>,
    reread: F,
) -> SizeCheck
where
    F: FnOnce() -> Fut,
    Fut: std::future::Future<Output = Option<(u32, u32)>>,
{
    if !size_off(size, real) {
        return SizeCheck::Matches;
    }
    if size_off(size, reread().await) {
        SizeCheck::StillDiffers
    } else {
        SizeCheck::MatchesAfterReread
    }
}

/// The raw pixels as the picture the detectors get: RGB, at the size KWin
/// rendered. Never resampled ([`check_native_size`]).
pub fn picture(meta: &RawMeta, data: &[u8]) -> Result<DynamicImage, KwinError> {
    to_rgb_image(meta, data).map(DynamicImage::ImageRgb8)
}

// -------------------------------------------------------------------- pipe

/// Asks the kernel for a 1 MiB pipe buffer, so KWin's writer needs fewer
/// rounds for a 10 to 30 MB picture. A refusal keeps the default.
fn widen_pipe(fd: &OwnedFd) {
    // SAFETY: fcntl on a live fd; the result is only a hint.
    unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETPIPE_SZ, 1 << 20);
    }
}

/// Closes our copy of the write end, then reads the pixels. KWin holds its
/// own copy and closes it after the last byte; ours must be gone first, or
/// EOF never comes and the read ends at the deadline with an error.
pub fn finish_pipe(
    reader: OwnedFd,
    our_writer: OwnedFd,
    len: usize,
    deadline: Instant,
) -> Result<Vec<u8>, KwinError> {
    drop(our_writer);
    read_exactly(reader, len, deadline)
}

/// Reads exactly `len` bytes from `fd`, then EOF, all before `deadline`.
/// Fewer bytes before EOF, more bytes than `len`, or no EOF by the deadline
/// is an error: KWin writes `stride * height` bytes and closes its end.
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
        let full = filled == len;
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
        let wait = deadline.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return Err(if full {
                KwinError::BadReply("the pipe stayed open after the picture".into())
            } else {
                KwinError::Timeout
            });
        }
        let mut pfd = libc::pollfd {
            fd: raw,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(wait.as_millis().max(1)).unwrap_or(i32::MAX);
        // SAFETY: one pollfd for an fd this function owns.
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
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

/// The probe's 1x1 area: the top-left pixel of the output Auto would
/// capture, so it lies on a real output. With no geometry, KWin's active
/// screen at scale 1.
pub fn probe_target(outputs: &[OutputRect]) -> Target {
    match super::auto_output_index(outputs.iter().map(|o| (o.name.as_str(), o.area()))) {
        Some(index) => Target::Area {
            x: outputs[index].x,
            y: outputs[index].y,
            width: 1,
            height: 1,
        },
        None => Target::ActiveScreen,
    }
}

static CONNECTION: tokio::sync::Mutex<Option<zbus::Connection>> =
    tokio::sync::Mutex::const_new(None);

/// The session bus connection, made on first use and made again after a
/// call found it gone ([`forget_connection`]).
async fn connection() -> Result<zbus::Connection, KwinError> {
    let mut slot = CONNECTION.lock().await;
    if let Some(conn) = slot.as_ref() {
        return Ok(conn.clone());
    }
    let conn = zbus::Connection::session()
        .await
        .map_err(|err| KwinError::NoBus(err.to_string()))?;
    *slot = Some(conn.clone());
    Ok(conn)
}

/// Drops the cached connection after an error that says it is broken.
async fn forget_connection(err: &KwinError) {
    if matches!(err, KwinError::NoBus(_)) {
        *CONNECTION.lock().await = None;
    }
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
pub async fn capture(
    output_name: Option<&str>,
) -> Result<DynamicImage, Box<dyn std::error::Error + Send + Sync>> {
    let outputs = super::portal::output_geometry().await;
    let (target, real) = pick_target(&outputs, output_name);
    let screen = match &target {
        Target::Screen(name) => Some(name.clone()),
        _ => None,
    };
    let wait = super::flight::lock_wait(super::tab_press().is_some(), LOCK_WAIT_TAB);
    let (meta, bytes) = match grab_in_flight(target, true, wait).await {
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
    let img = tokio::task::spawn_blocking(move || picture(&meta, &bytes))
        .await
        .map_err(|err| format!("KDE direct convert task failed: {err}"))??;
    let size = (img.width(), img.height());
    let checked = check_native_size(size, real, || async {
        super::portal::invalidate_output_geometry();
        let outputs = super::portal::output_geometry().await;
        screen.and_then(|name| {
            outputs
                .iter()
                .find(|o| o.name == name)
                .map(|o| (o.real_width, o.real_height))
        })
    })
    .await;
    if checked == SizeCheck::StillDiffers {
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
    /// 6.6.7+, 6.7.6+ (not verified), 6.8.0: the main fix, not the 6.8.1
    /// follow-up.
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

fn version_label(v: Option<(u32, u32, u32)>) -> String {
    v.map_or_else(|| "unknown".to_string(), |(a, b, c)| format!("{a}.{b}.{c}"))
}

/// What a render probe may do now.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RenderGate {
    /// A probe result is cached: use it, render nothing.
    Cached,
    /// The interface did not answer: unavailable, render nothing.
    Refused(String),
    /// A screen share may be open and this Plasma lacks the full KDE bug
    /// 524840 fix: not checked, render nothing.
    NotChecked(String),
    /// Render the 1x1 probe.
    Render,
}

/// The one rule for a render probe. `share_open` is whether the tracker's
/// screen share may be open beside this call. Rendering beside it needs
/// Plasma 6.8.1 or later, known for sure.
pub fn render_gate(
    share_open: bool,
    interface: &Result<u32, KwinError>,
    plasma: Option<(u32, u32, u32)>,
    probe_done: bool,
) -> RenderGate {
    if probe_done {
        return RenderGate::Cached;
    }
    if let Err(err) = interface {
        return RenderGate::Refused(err.reason());
    }
    if share_open && plasma.is_none_or(|v| crash_fix(v) != CrashFix::Full) {
        return RenderGate::NotChecked(format!(
            "it is not tested while the tracker shares the screen on Plasma {} (needs 6.8.1, KDE bug 524840)",
            version_label(plasma)
        ));
    }
    RenderGate::Render
}

/// A probe failure that holds for the life of the process: KWin refused
/// the caller. It is kept for [`FAILURE_TTL`] only, all the same. Other
/// failures (a timeout, no bus, KWin not on the bus yet at login) are not
/// kept at all.
const FAILURE_TTL: Duration = Duration::from_secs(60);

/// Whether a probe result may be reused at `now`.
pub fn reusable(
    result: &Result<(), KwinError>,
    at: tokio::time::Instant,
    now: tokio::time::Instant,
) -> bool {
    match result {
        Ok(()) => true,
        Err(err) => err.is_definitive() && now.saturating_duration_since(at) < FAILURE_TTL,
    }
}

/// What a probe needs from KWin; a fake in tests.
pub trait ProbeBus {
    fn versions(&self) -> impl std::future::Future<Output = Versions> + Send;
    fn render(&self) -> impl std::future::Future<Output = Result<(), KwinError>> + Send;
}

/// A probe's answer and whether trying again soon may change it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProbeAnswer {
    pub probe: KwinProbe,
    pub transient: bool,
}

/// One probe at a time, with its last result.
pub struct Prober {
    last: tokio::sync::Mutex<Option<(Result<(), KwinError>, tokio::time::Instant)>>,
}

impl Prober {
    pub const fn new() -> Self {
        Self {
            last: tokio::sync::Mutex::const_new(None),
        }
    }

    /// The probe through [`render_gate`]: renders only when allowed, and
    /// keeps the result as [`reusable`] says.
    pub async fn probe<B: ProbeBus>(&self, bus: &B, share_open: bool) -> ProbeAnswer {
        let mut last = self.last.lock().await;
        let now = tokio::time::Instant::now();
        let cached = last
            .as_ref()
            .filter(|(result, at)| reusable(result, *at, now))
            .map(|(result, _)| result.clone());
        let found = bus.versions().await;
        match render_gate(share_open, &found.interface, found.plasma, cached.is_some()) {
            RenderGate::Cached => ProbeAnswer {
                probe: match &cached {
                    Some(result) => probe_outcome(result),
                    None => KwinProbe::NotProbed,
                },
                transient: false,
            },
            RenderGate::Refused(reason) => ProbeAnswer {
                probe: KwinProbe::Unavailable(reason),
                transient: found.interface.as_ref().is_err_and(|e| !e.is_definitive()),
            },
            RenderGate::NotChecked(reason) => ProbeAnswer {
                probe: KwinProbe::NotChecked(reason),
                transient: false,
            },
            RenderGate::Render => {
                let result = bus.render().await;
                log_probe(&result, &found);
                let transient = result.as_ref().is_err_and(|e| !e.is_definitive());
                let probe = probe_outcome(&result);
                *last = Some((result, now));
                ProbeAnswer { probe, transient }
            }
        }
    }

    /// The last result while it may be reused, without asking KWin.
    pub async fn last(&self) -> Option<KwinProbe> {
        let last = self.last.lock().await;
        let now = tokio::time::Instant::now();
        last.as_ref()
            .filter(|(result, at)| reusable(result, *at, now))
            .map(|(result, _)| probe_outcome(result))
    }
}

impl Default for Prober {
    fn default() -> Self {
        Self::new()
    }
}

fn log_probe(result: &Result<(), KwinError>, found: &Versions) {
    let plasma = version_label(found.plasma);
    match result {
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
}

/// Waits before each startup retry of a probe that failed in a way that
/// can pass (KWin not on the bus yet at login, a timeout): 15 s in all.
pub const STARTUP_RETRY_WAITS: [Duration; 4] = [
    Duration::from_secs(1),
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
];

/// [`Prober::probe`], tried again after each of `waits` while the answer
/// is transient.
pub async fn probe_with_retries<B: ProbeBus>(
    prober: &Prober,
    bus: &B,
    share_open: bool,
    waits: &[Duration],
) -> KwinProbe {
    let mut answer = prober.probe(bus, share_open).await;
    for wait in waits {
        if !answer.transient {
            break;
        }
        tracing::debug!(
            wait_ms = wait.as_millis() as u64,
            "KDE direct probe will try again"
        );
        tokio::time::sleep(*wait).await;
        answer = prober.probe(bus, share_open).await;
    }
    answer.probe
}

/// The real bus. Versions are kept once the interface answered.
struct RealBus;

static VERSIONS: tokio::sync::Mutex<Option<Versions>> = tokio::sync::Mutex::const_new(None);

impl ProbeBus for RealBus {
    async fn versions(&self) -> Versions {
        let mut slot = VERSIONS.lock().await;
        if let Some(found) = slot.as_ref() {
            return found.clone();
        }
        let found = read_versions().await;
        if let Err(err) = &found.interface {
            forget_connection(err).await;
        } else {
            *slot = Some(found.clone());
        }
        found
    }

    async fn render(&self) -> Result<(), KwinError> {
        let target = probe_target(&super::portal::output_geometry().await);
        let (meta, bytes) = grab_in_flight(target, false, LOCK_WAIT_TAB).await?;
        to_rgb_image(&meta, &bytes).map(|_| ())
    }
}

static PROBER: Prober = Prober::new();

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
    let interface = bounded(async {
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
    })
    .await;
    // KWin has no version property. Its support information (the text
    // `qdbus org.kde.KWin /KWin supportInformation` prints) starts with
    // `KWin version: X.Y.Z`. Best effort.
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

/// What KWin says about itself, without rendering anything.
pub async fn versions() -> Versions {
    RealBus.versions().await
}

/// The probe for a process that may run beside the tracker's screen
/// stream: with `share_open`, the 1x1 render runs only on Plasma 6.8.1 or
/// later ([`render_gate`]).
pub async fn probe_beside_share(share_open: bool) -> KwinProbe {
    PROBER.probe(&RealBus, share_open).await.probe
}

/// The probe at daemon or app startup. On KDE, a failure that can pass
/// (KWin not on the bus yet at login, a timeout, no bus) is tried again
/// for up to 15 s ([`STARTUP_RETRY_WAITS`]).
pub async fn probe_at_startup(share_open: bool, on_kde: bool) -> KwinProbe {
    let waits: &[Duration] = if on_kde { &STARTUP_RETRY_WAITS } else { &[] };
    probe_with_retries(&PROBER, &RealBus, share_open, waits).await
}

/// KDE direct for Settings without rendering anything: the last probe
/// result while it holds, else what the version read says (not available,
/// or not checked yet).
pub async fn status_without_render() -> KwinProbe {
    if let Some(last) = PROBER.last().await {
        return last;
    }
    status_from_versions(&versions().await.interface)
}

/// [`status_without_render`] when no probe result is kept.
pub fn status_from_versions(interface: &Result<u32, KwinError>) -> KwinProbe {
    match interface {
        Ok(_) => KwinProbe::NotProbed,
        Err(err) => KwinProbe::Unavailable(err.reason()),
    }
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
    if probe != KwinProbe::Available {
        static SAID: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
        if !SAID.swap(true, std::sync::atomic::Ordering::Relaxed) {
            tracing::info!(?probe, "KDE direct half of the compare pair skipped");
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
        let got = check_native_size((2560, 1440), Some((2560, 1440)), reread(None)).await;
        assert_eq!((got, calls.get()), (SizeCheck::Matches, 0));
        // Unknown real size: nothing to compare.
        let got = check_native_size((2560, 1440), None, reread(None)).await;
        assert_eq!((got, calls.get()), (SizeCheck::Matches, 0));
        // Stale cache (the monitor changed mode): one re-read fixes it.
        let got =
            check_native_size((2560, 1440), Some((1920, 1080)), reread(Some((2560, 1440)))).await;
        assert_eq!((got, calls.get()), (SizeCheck::MatchesAfterReread, 1));
        // Still off after the re-read: kept as KWin rendered it.
        let got =
            check_native_size((3200, 1800), Some((2560, 1440)), reread(Some((2560, 1440)))).await;
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

    #[test]
    fn the_render_gate_has_one_answer_per_case() {
        let up: Result<u32, KwinError> = Ok(5);
        let down: Result<u32, KwinError> = Err(KwinError::NotKwin);
        let full = Some((6, 8, 1));
        let partial = Some((6, 8, 0));
        // A kept result wins over everything, and renders nothing.
        assert_eq!(render_gate(true, &down, None, true), RenderGate::Cached);
        assert_eq!(render_gate(true, &up, partial, true), RenderGate::Cached);
        // The interface did not answer.
        assert!(
            matches!(render_gate(false, &down, full, false), RenderGate::Refused(r) if r.contains("not KDE"))
        );
        assert!(matches!(
            render_gate(true, &down, full, false),
            RenderGate::Refused(_)
        ));
        // A share may be open: only a known 6.8.1 renders.
        assert_eq!(render_gate(true, &up, full, false), RenderGate::Render);
        assert_eq!(
            render_gate(true, &up, Some((6, 9, 0)), false),
            RenderGate::Render
        );
        assert!(
            matches!(render_gate(true, &up, partial, false), RenderGate::NotChecked(r) if r.contains("6.8.0"))
        );
        assert!(matches!(
            render_gate(true, &up, Some((6, 7, 6)), false),
            RenderGate::NotChecked(_)
        ));
        assert!(
            matches!(render_gate(true, &up, None, false), RenderGate::NotChecked(r) if r.contains("unknown"))
        );
        // No share: any Plasma renders.
        assert_eq!(render_gate(false, &up, partial, false), RenderGate::Render);
        assert_eq!(render_gate(false, &up, None, false), RenderGate::Render);
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
                versions: Versions { interface, plasma },
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
        let bus = FakeBus::new(Ok(5), Some((6, 8, 0)), vec![]);
        let answer = prober.probe(&bus, true).await;
        assert!(
            matches!(answer.probe, KwinProbe::NotChecked(_)),
            "{answer:?}"
        );
        assert_eq!(bus.rendered(), 0, "no render beside the share on 6.8.0");
        assert_eq!(prober.last().await, None, "nothing kept");
        // With the share closed it renders, and the answer is kept.
        assert_eq!(prober.probe(&bus, false).await.probe, KwinProbe::Available);
        assert_eq!(bus.rendered(), 1);
        assert_eq!(prober.probe(&bus, true).await.probe, KwinProbe::Available);
        assert_eq!(bus.rendered(), 1, "the kept answer renders nothing");
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
        // A refusal is kept for a minute, then asked again.
        let prober = Prober::new();
        let bus = FakeBus::new(
            Ok(4),
            Some((6, 7, 5)),
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
        let got = probe_with_retries(&prober, &bus, false, &STARTUP_RETRY_WAITS).await;
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
        let got = probe_with_retries(&prober, &bus, false, &STARTUP_RETRY_WAITS).await;
        assert!(matches!(got, KwinProbe::Unavailable(_)));
        assert_eq!(started.elapsed(), Duration::ZERO);
        // Off KDE there are no waits.
        let prober = Prober::new();
        let bus = FakeBus::new(Err(KwinError::NotKwin), None, vec![]);
        let started = tokio::time::Instant::now();
        probe_with_retries(&prober, &bus, false, &[]).await;
        assert_eq!(started.elapsed(), Duration::ZERO);
    }

    #[test]
    fn settings_status_without_a_render() {
        assert_eq!(status_from_versions(&Ok(5)), KwinProbe::NotProbed);
        assert!(matches!(
            status_from_versions(&Err(KwinError::NotKwin)),
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
}
