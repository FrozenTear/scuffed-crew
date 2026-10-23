//! The KWin ScreenShot2 protocol: the D-Bus names, the errors KWin
//! answers with, the reply's picture metadata, and turning the raw pixels
//! into an RGB image.

use std::collections::HashMap;

use ashpd::zbus;
use ashpd::zvariant::{OwnedValue, Value};
use image::{DynamicImage, RgbImage};

use super::GRAB_TIMEOUT;

pub const SERVICE: &str = "org.kde.KWin.ScreenShot2";
pub const PATH: &str = "/org/kde/KWin/ScreenShot2";
pub const INTERFACE: &str = "org.kde.KWin.ScreenShot2";

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
            KwinError::Busy => crate::capture::flight::BUSY.into(),
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

pub(super) fn from_zbus(err: zbus::Error) -> KwinError {
    match err {
        zbus::Error::MethodError(name, detail, _) => {
            classify_error(name.as_str(), detail.as_deref())
        }
        zbus::Error::InputOutput(err) => KwinError::NoBus(err.to_string()),
        zbus::Error::Address(err) => KwinError::NoBus(err),
        other => KwinError::Other(other.to_string()),
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

pub(super) fn value_u32(value: &Value<'_>) -> Option<u32> {
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

/// The raw pixels as the picture the detectors get: RGB, at the size KWin
/// rendered. Never resampled ([`check_native_size`]).
///
/// [`check_native_size`]: super::check_native_size
pub fn picture(meta: &RawMeta, data: &[u8]) -> Result<DynamicImage, KwinError> {
    to_rgb_image(meta, data).map(DynamicImage::ImageRgb8)
}
