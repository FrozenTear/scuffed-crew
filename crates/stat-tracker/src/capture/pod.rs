//! SPA pods for the screen-cast stream, built without linking PipeWire.
//!
//! The format pod offers BGRx, RGBx, xBGR, xRGB, BGRA, RGBA, BGR, and RGB.
//! Framerate is the fixed 0/1 KWin and GNOME offer (frames only when the
//! screen changes). maxFramerate is `N/1`, where N is the configured cap
//! (default 2), so the compositor itself stays near that rate. The caller
//! also throttles, in case a backend ignores the cap. The buffers pod asks
//! for one or two buffers and shared memory, not DMA-BUF. A rejected buffer
//! param is logged; the producer keeps the buffers it already chose.

/// Highest frame rate offered to PipeWire when the config does not say.
/// The live cap is [`enum_format_pod`]'s argument.
#[cfg_attr(not(test), allow(dead_code))]
pub const MAX_FPS: u32 = 2;

const SPA_TYPE_ID: u32 = 3;
const SPA_TYPE_INT: u32 = 4;
const SPA_TYPE_RECTANGLE: u32 = 10;
const SPA_TYPE_FRACTION: u32 = 11;
const SPA_TYPE_OBJECT: u32 = 15;
const SPA_TYPE_CHOICE: u32 = 19;

const SPA_CHOICE_NONE: u32 = 0;
const SPA_CHOICE_RANGE: u32 = 1;
const SPA_CHOICE_ENUM: u32 = 3;
const SPA_CHOICE_FLAGS: u32 = 4;

const SPA_PARAM_ENUM_FORMAT: u32 = 3;
const SPA_PARAM_FORMAT: u32 = 4;
const SPA_PARAM_BUFFERS: u32 = 5;

const SPA_MEDIA_TYPE_VIDEO: u32 = 2;
const SPA_MEDIA_SUBTYPE_RAW: u32 = 1;
const SPA_FORMAT_MEDIA_TYPE: u32 = 1;
const SPA_FORMAT_MEDIA_SUBTYPE: u32 = 2;
const SPA_FORMAT_VIDEO_FORMAT: u32 = 0x20001;
const SPA_FORMAT_VIDEO_SIZE: u32 = 0x20003;
const SPA_FORMAT_VIDEO_FRAMERATE: u32 = 0x20004;
const SPA_FORMAT_VIDEO_MAX_FRAMERATE: u32 = 0x20005;

const SPA_TYPE_OBJECT_FORMAT: u32 = 0x40003;
const SPA_TYPE_OBJECT_PARAM_BUFFERS: u32 = 0x40004;

const SPA_PARAM_BUFFERS_BUFFERS: u32 = 1;
const SPA_PARAM_BUFFERS_BLOCKS: u32 = 2;
const SPA_PARAM_BUFFERS_DATA_TYPE: u32 = 6;

/// `(1 << MemPtr) | (1 << MemFd)`. DMA-BUF is left out on purpose.
const MEM_PTR_OR_FD: i32 = (1 << 1) | (1 << 2);

const FMT_RGBX: u32 = 7;
const FMT_BGRX: u32 = 8;
const FMT_XRGB: u32 = 9;
const FMT_XBGR: u32 = 10;
const FMT_RGBA: u32 = 11;
const FMT_BGRA: u32 = 12;
const FMT_RGB: u32 = 15;
const FMT_BGR: u32 = 16;

struct PodBuf {
    bytes: Vec<u8>,
}

impl PodBuf {
    fn new() -> Self {
        Self { bytes: Vec::new() }
    }

    fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn i32(&mut self, v: i32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }

    fn align(&mut self) {
        while !self.bytes.len().is_multiple_of(8) {
            self.bytes.push(0);
        }
    }

    fn begin(&mut self, ty: u32) -> usize {
        let at = self.bytes.len();
        self.u32(0);
        self.u32(ty);
        at
    }

    fn end(&mut self, at: usize) {
        let body = (self.bytes.len() - at - 8) as u32;
        self.bytes[at..at + 4].copy_from_slice(&body.to_le_bytes());
        self.align();
    }

    fn id(&mut self, value: u32) {
        let at = self.begin(SPA_TYPE_ID);
        self.u32(value);
        self.end(at);
    }

    fn int(&mut self, value: i32) {
        let at = self.begin(SPA_TYPE_INT);
        self.i32(value);
        self.end(at);
    }

    fn fraction(&mut self, num: u32, den: u32) {
        let at = self.begin(SPA_TYPE_FRACTION);
        self.u32(num);
        self.u32(den);
        self.end(at);
    }

    fn prop_pod(&mut self, key: u32, write: impl FnOnce(&mut Self)) {
        self.u32(key);
        self.u32(0);
        write(self);
    }

    /// Choice whose child values are raw `words` of `child_size` bytes.
    fn choice(&mut self, kind: u32, child_type: u32, child_size: u32, words: &[u32]) {
        let at = self.begin(SPA_TYPE_CHOICE);
        self.u32(kind);
        self.u32(0);
        self.u32(child_size);
        self.u32(child_type);
        for word in words {
            self.u32(*word);
        }
        self.end(at);
    }
}

/// EnumFormat pod offered at connect time. `max_fps` is the max-framerate
/// numerator (denominator 1). Framerate itself is the fixed fraction 0/1.
pub fn enum_format_pod(max_fps: u32) -> Vec<u8> {
    let mut pod = PodBuf::new();
    let at = pod.begin(SPA_TYPE_OBJECT);
    pod.u32(SPA_TYPE_OBJECT_FORMAT);
    pod.u32(SPA_PARAM_ENUM_FORMAT);
    pod.prop_pod(SPA_FORMAT_MEDIA_TYPE, |p| p.id(SPA_MEDIA_TYPE_VIDEO));
    pod.prop_pod(SPA_FORMAT_MEDIA_SUBTYPE, |p| p.id(SPA_MEDIA_SUBTYPE_RAW));
    pod.prop_pod(SPA_FORMAT_VIDEO_FORMAT, |p| {
        p.choice(
            SPA_CHOICE_ENUM,
            SPA_TYPE_ID,
            4,
            &[
                FMT_BGRX, FMT_BGRX, FMT_RGBX, FMT_XBGR, FMT_XRGB, FMT_BGRA, FMT_RGBA, FMT_BGR,
                FMT_RGB,
            ],
        );
    });
    // Fixed 0/1 matches the variable rate KWin and GNOME offer. A range
    // that starts at 1/1 does not intersect that offer, so negotiation fails.
    pod.prop_pod(SPA_FORMAT_VIDEO_FRAMERATE, |p| p.fraction(0, 1));
    pod.prop_pod(SPA_FORMAT_VIDEO_MAX_FRAMERATE, |p| {
        p.fraction(max_fps.max(1), 1)
    });
    pod.end(at);
    pod.bytes
}

/// Buffers param sent once the format is fixed. One or two buffers, memfd or memptr.
pub fn buffers_pod() -> Vec<u8> {
    let mut pod = PodBuf::new();
    let at = pod.begin(SPA_TYPE_OBJECT);
    pod.u32(SPA_TYPE_OBJECT_PARAM_BUFFERS);
    pod.u32(SPA_PARAM_BUFFERS);
    pod.prop_pod(SPA_PARAM_BUFFERS_BUFFERS, |p| {
        p.choice(SPA_CHOICE_RANGE, SPA_TYPE_INT, 4, &[2, 1, 2]);
    });
    pod.prop_pod(SPA_PARAM_BUFFERS_BLOCKS, |p| p.int(1));
    pod.prop_pod(SPA_PARAM_BUFFERS_DATA_TYPE, |p| {
        p.choice(SPA_CHOICE_FLAGS, SPA_TYPE_INT, 4, &[MEM_PTR_OR_FD as u32]);
    });
    pod.end(at);
    pod.bytes
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VideoInfo {
    pub format: u32,
    pub width: u32,
    pub height: u32,
}

/// Read a negotiated Format pod.
///
/// PipeWire wraps every property both sides offered in a Choice of type
/// None. The first child value is the fixated one. A plain Id or Rectangle
/// is accepted too, so a hand-built pod still parses.
pub fn parse_format_pod(bytes: &[u8]) -> Option<VideoInfo> {
    if bytes.len() < 16 {
        return None;
    }
    let size = read_u32(bytes, 0)? as usize;
    let ty = read_u32(bytes, 4)?;
    if ty != SPA_TYPE_OBJECT || bytes.len() < 8 + size {
        return None;
    }
    let obj_ty = read_u32(bytes, 8)?;
    let id = read_u32(bytes, 12)?;
    if obj_ty != SPA_TYPE_OBJECT_FORMAT || (id != SPA_PARAM_FORMAT && id != SPA_PARAM_ENUM_FORMAT) {
        return None;
    }
    let mut format = None;
    let mut width = None;
    let mut height = None;
    let mut off = 16;
    let end = 8 + size;
    while off + 8 <= end {
        let key = read_u32(bytes, off)?;
        off += 8; // key + flags
        if off + 8 > end {
            break;
        }
        let pod_size = read_u32(bytes, off)? as usize;
        let pod_ty = read_u32(bytes, off + 4)?;
        let body = off + 8;
        let next = align8(body + pod_size);
        if next > bytes.len() {
            break;
        }
        if let Some((value_ty, value_at)) = prop_value(bytes, body, pod_size, pod_ty) {
            if value_ty == SPA_TYPE_ID && key == SPA_FORMAT_VIDEO_FORMAT {
                format = read_u32(bytes, value_at);
            }
            if value_ty == SPA_TYPE_RECTANGLE && key == SPA_FORMAT_VIDEO_SIZE {
                width = read_u32(bytes, value_at);
                height = read_u32(bytes, value_at + 4);
            }
        }
        off = next;
    }
    Some(VideoInfo {
        format: format?,
        width: width.unwrap_or(0),
        height: height.unwrap_or(0),
    })
}

/// First child of a Choice, or the pod itself when it is already a value.
fn prop_value(bytes: &[u8], body: usize, pod_size: usize, pod_ty: u32) -> Option<(u32, usize)> {
    if pod_ty != SPA_TYPE_CHOICE {
        return Some((pod_ty, body));
    }
    if pod_size < 16 {
        return None;
    }
    let child_size = read_u32(bytes, body + 8)? as usize;
    let child_ty = read_u32(bytes, body + 12)?;
    let value_at = body + 16;
    if child_size == 0 || value_at + child_size > bytes.len() {
        return None;
    }
    Some((child_ty, value_at))
}

fn align8(n: usize) -> usize {
    n.div_ceil(8) * 8
}

fn read_u32(bytes: &[u8], at: usize) -> Option<u32> {
    let b = bytes.get(at..at + 4)?;
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

/// Every fraction stored in `pod`. Each fraction is `(numerator, denominator)`.
/// Objects are opened so a fraction nested in an EnumFormat pod is seen.
#[cfg_attr(not(test), allow(dead_code))]
pub fn fractions(bytes: &[u8]) -> Vec<(u32, u32)> {
    let mut out = Vec::new();
    walk_pods(bytes, 0, bytes.len(), &mut out);
    out
}

#[cfg_attr(not(test), allow(dead_code))]
fn walk_pods(bytes: &[u8], start: usize, end: usize, out: &mut Vec<(u32, u32)>) {
    let end = end.min(bytes.len());
    let mut off = start;
    while off + 8 <= end {
        let size = read_u32(bytes, off).unwrap_or(0) as usize;
        let ty = read_u32(bytes, off + 4).unwrap_or(0);
        let body = off + 8;
        let pod_end = body.saturating_add(size);
        if pod_end > end {
            break;
        }
        match ty {
            SPA_TYPE_FRACTION if size >= 8 => {
                if let (Some(n), Some(d)) = (read_u32(bytes, body), read_u32(bytes, body + 4)) {
                    out.push((n, d));
                }
            }
            SPA_TYPE_CHOICE if size >= 16 => push_choice_fractions(bytes, body, pod_end, out),
            SPA_TYPE_OBJECT if size >= 8 => walk_object_props(bytes, body, pod_end, out),
            _ => {}
        }
        let next = align8(pod_end);
        if next <= off {
            break;
        }
        off = next;
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn walk_object_props(bytes: &[u8], body: usize, pod_end: usize, out: &mut Vec<(u32, u32)>) {
    // Object body starts with type and id. Each prop is key, flags, then a pod.
    let mut prop = body + 8;
    while prop + 16 <= pod_end {
        prop += 8;
        let vsize = read_u32(bytes, prop).unwrap_or(0) as usize;
        let value_end = prop + 8 + vsize;
        if value_end > pod_end {
            break;
        }
        walk_pods(bytes, prop, value_end, out);
        let next = align8(value_end);
        if next <= prop {
            break;
        }
        prop = next;
    }
}

#[cfg_attr(not(test), allow(dead_code))]
fn push_choice_fractions(bytes: &[u8], body: usize, pod_end: usize, out: &mut Vec<(u32, u32)>) {
    // Choice body: kind, flags, then a child pod header whose size is one
    // value. The values themselves follow that header as raw words.
    let Some(child_size) = read_u32(bytes, body + 8) else {
        return;
    };
    let Some(child_ty) = read_u32(bytes, body + 12) else {
        return;
    };
    if child_ty != SPA_TYPE_FRACTION || child_size != 8 {
        return;
    }
    let mut cursor = body + 16;
    while cursor + 8 <= pod_end {
        if let (Some(n), Some(d)) = (read_u32(bytes, cursor), read_u32(bytes, cursor + 4)) {
            out.push((n, d));
        }
        cursor += 8;
    }
}

/// True when every fraction is at most `max_fps` frames per second.
/// 0/1 is the variable rate and is always inside the cap.
#[cfg_attr(not(test), allow(dead_code))]
pub fn framerate_within_cap(bytes: &[u8], max_fps: u32) -> bool {
    let found = fractions(bytes);
    !found.is_empty()
        && found.iter().all(|(num, den)| {
            *den > 0 && u64::from(*num) <= u64::from(max_fps.max(1)) * u64::from(*den)
        })
}

/// One line for the session-start log. `configured` is the cap from config.
/// The frame-rate cap in a negotiated Format's fractions (see
/// [`negotiated_rate_line`] for the order): the max-framerate when it is
/// not 0, else a fixed framerate that is not 0, else `None` (variable rate
/// with no max).
pub fn negotiated_cap(rates: &[(u32, u32)]) -> Option<(u32, u32)> {
    let usable = |r: &(u32, u32)| r.0 > 0 && r.1 > 0;
    match rates {
        [_, max, ..] if usable(max) => Some(*max),
        [rate, ..] if usable(rate) => Some(*rate),
        _ => None,
    }
}

pub fn negotiated_rate_line(rates: &[(u32, u32)], configured: u32) -> String {
    match rates {
        [rate, max, ..] => format!(
            "screencast negotiated framerate {}/{} max-framerate {}/{} (configured {configured} fps)",
            rate.0, rate.1, max.0, max.1
        ),
        [rate] => format!(
            "screencast negotiated framerate {}/{} (configured {configured} fps)",
            rate.0, rate.1
        ),
        _ => format!("screencast negotiated framerate unknown (configured {configured} fps)"),
    }
}

/// Negotiated Format the way KWin leaves it: every property is a Choice.
/// Format and size are Choice None. Framerate is the fixed 0/1. maxFramerate
/// is a range whose default is `max_fps`/1, inside a 0/1..60/1 offer.
#[cfg_attr(not(test), allow(dead_code))]
pub fn kwin_negotiated_format_pod(format: u32, width: u32, height: u32, max_fps: u32) -> Vec<u8> {
    let mut pod = PodBuf::new();
    let at = pod.begin(SPA_TYPE_OBJECT);
    pod.u32(SPA_TYPE_OBJECT_FORMAT);
    pod.u32(SPA_PARAM_FORMAT);
    pod.prop_pod(SPA_FORMAT_VIDEO_FORMAT, |p| {
        p.choice(SPA_CHOICE_NONE, SPA_TYPE_ID, 4, &[format]);
    });
    pod.prop_pod(SPA_FORMAT_VIDEO_SIZE, |p| {
        p.choice(SPA_CHOICE_NONE, SPA_TYPE_RECTANGLE, 8, &[width, height]);
    });
    pod.prop_pod(SPA_FORMAT_VIDEO_FRAMERATE, |p| {
        p.choice(SPA_CHOICE_NONE, SPA_TYPE_FRACTION, 8, &[0, 1]);
    });
    pod.prop_pod(SPA_FORMAT_VIDEO_MAX_FRAMERATE, |p| {
        p.choice(
            SPA_CHOICE_RANGE,
            SPA_TYPE_FRACTION,
            8,
            &[max_fps, 1, 0, 1, 60, 1],
        );
    });
    pod.end(at);
    pod.bytes
}

/// EnumFormat the way KWin offers it: framerate fixed at 0/1, maxFramerate
/// a range from 0/1 up to `refresh`/1. Not sent to PipeWire.
#[cfg_attr(not(test), allow(dead_code))]
pub fn kwin_enum_format_offer(refresh: u32) -> Vec<u8> {
    let refresh = refresh.max(1);
    let mut pod = PodBuf::new();
    let at = pod.begin(SPA_TYPE_OBJECT);
    pod.u32(SPA_TYPE_OBJECT_FORMAT);
    pod.u32(SPA_PARAM_ENUM_FORMAT);
    pod.prop_pod(SPA_FORMAT_VIDEO_FORMAT, |p| {
        p.choice(
            SPA_CHOICE_ENUM,
            SPA_TYPE_ID,
            4,
            &[FMT_BGRX, FMT_BGRX, FMT_RGBX],
        );
    });
    pod.prop_pod(SPA_FORMAT_VIDEO_FRAMERATE, |p| {
        p.choice(SPA_CHOICE_NONE, SPA_TYPE_FRACTION, 8, &[0, 1]);
    });
    pod.prop_pod(SPA_FORMAT_VIDEO_MAX_FRAMERATE, |p| {
        p.choice(
            SPA_CHOICE_RANGE,
            SPA_TYPE_FRACTION,
            8,
            &[refresh, 1, 0, 1, refresh, 1],
        );
    });
    pod.end(at);
    pod.bytes
}

/// A fixed Format pod, for the parser test. Not sent to PipeWire.
#[cfg_attr(not(test), allow(dead_code))]
pub fn fixed_format_pod(format: u32, width: u32, height: u32) -> Vec<u8> {
    let mut pod = PodBuf::new();
    let at = pod.begin(SPA_TYPE_OBJECT);
    pod.u32(SPA_TYPE_OBJECT_FORMAT);
    pod.u32(SPA_PARAM_FORMAT);
    pod.prop_pod(SPA_FORMAT_VIDEO_FORMAT, |p| p.id(format));
    pod.prop_pod(SPA_FORMAT_VIDEO_SIZE, |p| {
        let inner = p.begin(SPA_TYPE_RECTANGLE);
        p.u32(width);
        p.u32(height);
        p.end(inner);
    });
    pod.end(at);
    pod.bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn offered_framerate_is_fixed_zero_and_capped() {
        let pod = enum_format_pod(MAX_FPS);
        assert!(
            framerate_within_cap(&pod, MAX_FPS),
            "fractions: {:?}",
            fractions(&pod)
        );
        let rates = fractions(&pod);
        assert!(
            rates.contains(&(0, 1)),
            "KWin's fixed 0/1 must be in the offer: {rates:?}"
        );
        assert!(rates.contains(&(2, 1)), "{rates:?}");
        assert!(!rates.contains(&(1, 1)), "{rates:?}");
        assert!(!rates.iter().any(|(n, _)| *n >= 30));
        let four = enum_format_pod(4);
        let four_rates = fractions(&four);
        assert!(four_rates.contains(&(0, 1)));
        assert!(four_rates.contains(&(4, 1)), "{four_rates:?}");
        assert!(framerate_within_cap(&four, 4));
        assert_eq!(
            negotiated_rate_line(&[(0, 1), (4, 1)], 4),
            "screencast negotiated framerate 0/1 max-framerate 4/1 (configured 4 fps)"
        );
    }

    #[test]
    fn buffers_pod_asks_for_one_or_two_shared_buffers() {
        let pod = buffers_pod();
        assert!(pod.len() > 16);
        assert_eq!(read_u32(&pod, 4), Some(SPA_TYPE_OBJECT));
        // The memfd|memptr mask is in the pod, and DMA-BUF's bit (1<<3) is not
        // the only type offered. The mask value itself is present.
        let mask = MEM_PTR_OR_FD as u32;
        let mut found = false;
        for off in (0..pod.len().saturating_sub(4)).step_by(4) {
            if read_u32(&pod, off) == Some(mask) {
                found = true;
            }
        }
        assert!(found, "shared-memory mask missing");
        assert_ne!(mask & (1 << 3), mask);
    }

    #[test]
    fn a_kwin_choice_pod_unwraps_format_and_size() {
        let pod = kwin_negotiated_format_pod(FMT_BGRX, 2560, 1440, 2);
        let info = parse_format_pod(&pod).expect("choice-wrapped format");
        assert_eq!(
            info,
            VideoInfo {
                format: FMT_BGRX,
                width: 2560,
                height: 1440
            }
        );
        let rates = fractions(&pod);
        assert!(rates.contains(&(0, 1)), "{rates:?}");
        assert!(rates.contains(&(2, 1)), "{rates:?}");
        let rgbx = kwin_negotiated_format_pod(FMT_RGBX, 1920, 1080, 4);
        assert_eq!(parse_format_pod(&rgbx).unwrap().format, FMT_RGBX);
        let xbgr = kwin_negotiated_format_pod(FMT_XBGR, 1920, 1080, 2);
        assert_eq!(parse_format_pod(&xbgr).unwrap().format, FMT_XBGR);
    }

    #[test]
    fn our_enum_format_intersects_a_kwin_enum_offer() {
        let ours = fractions(&enum_format_pod(2));
        let kwin = fractions(&kwin_enum_format_offer(60));
        assert!(
            ours.contains(&(0, 1)) && kwin.contains(&(0, 1)),
            "both offers include KWin's fixed 0/1: ours {ours:?} kwin {kwin:?}"
        );
        assert!(ours.contains(&(2, 1)), "{ours:?}");
        assert!(
            !ours.contains(&(1, 1)),
            "a range that starts at 1/1 does not meet KWin's fixed 0/1: {ours:?}"
        );
        assert!(framerate_within_cap(&enum_format_pod(2), 2));
        assert!(
            !framerate_within_cap(&enum_format_pod(2), 1),
            "2/1 is over a 1 fps cap"
        );
        assert!(
            negotiated_rate_line(&fractions(&enum_format_pod(4)), 4).contains("max-framerate 4/1"),
            "the negotiated line must name the cap"
        );
    }

    #[test]
    fn negotiated_format_pod_round_trips_size_and_format() {
        let pod = fixed_format_pod(FMT_BGRA, 1920, 1080);
        let info = parse_format_pod(&pod).unwrap();
        assert_eq!(
            info,
            VideoInfo {
                format: FMT_BGRA,
                width: 1920,
                height: 1080
            }
        );
    }
}
