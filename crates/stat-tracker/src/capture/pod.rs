//! SPA pods for the screen-cast stream, built without linking PipeWire.
//!
//! The format pod offers BGRx, RGBx, BGRA, RGBA, BGR, and RGB. Framerate is
//! a 1/1 to 2/1 range and maxFramerate is 2/1, so the compositor is not
//! asked for the display rate. The buffers pod asks for one or two buffers
//! and shared memory, not DMA-BUF. If that negotiation fails the caller
//! retries with the format pod only.

/// Highest frame rate offered to PipeWire, numerator over a denominator of 1.
pub const MAX_FPS: u32 = 2;

const SPA_TYPE_ID: u32 = 3;
const SPA_TYPE_INT: u32 = 4;
const SPA_TYPE_RECTANGLE: u32 = 10;
const SPA_TYPE_FRACTION: u32 = 11;
const SPA_TYPE_OBJECT: u32 = 15;
const SPA_TYPE_CHOICE: u32 = 19;

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

/// EnumFormat pod offered at connect time.
pub fn enum_format_pod() -> Vec<u8> {
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
                FMT_BGRX, FMT_BGRX, FMT_RGBX, FMT_BGRA, FMT_RGBA, FMT_BGR, FMT_RGB,
            ],
        );
    });
    // default 1/1, min 1/1, max 2/1. Never the display rate.
    pod.prop_pod(SPA_FORMAT_VIDEO_FRAMERATE, |p| {
        p.choice(
            SPA_CHOICE_RANGE,
            SPA_TYPE_FRACTION,
            8,
            &[1, 1, 1, 1, MAX_FPS, 1],
        );
    });
    pod.prop_pod(SPA_FORMAT_VIDEO_MAX_FRAMERATE, |p| p.fraction(MAX_FPS, 1));
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

/// Read a negotiated Format pod (fixed values, not a choice).
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
        if pod_ty == SPA_TYPE_ID && key == SPA_FORMAT_VIDEO_FORMAT && pod_size >= 4 {
            format = read_u32(bytes, body);
        }
        if pod_ty == SPA_TYPE_RECTANGLE && key == SPA_FORMAT_VIDEO_SIZE && pod_size >= 8 {
            width = read_u32(bytes, body);
            height = read_u32(bytes, body + 4);
        }
        off = next;
    }
    Some(VideoInfo {
        format: format?,
        width: width.unwrap_or(0),
        height: height.unwrap_or(0),
    })
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

/// True when every fraction is at most [`MAX_FPS`] frames per second.
#[cfg_attr(not(test), allow(dead_code))]
pub fn framerate_within_cap(bytes: &[u8]) -> bool {
    let found = fractions(bytes);
    !found.is_empty()
        && found
            .iter()
            .all(|(num, den)| *den > 0 && u64::from(*num) <= u64::from(MAX_FPS) * u64::from(*den))
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
    fn offered_framerate_never_exceeds_two() {
        let pod = enum_format_pod();
        assert!(
            framerate_within_cap(&pod),
            "fractions: {:?}",
            fractions(&pod)
        );
        let rates = fractions(&pod);
        assert!(rates.contains(&(1, 1)));
        assert!(rates.contains(&(MAX_FPS, 1)));
        assert!(!rates.iter().any(|(n, _)| *n >= 30));
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
