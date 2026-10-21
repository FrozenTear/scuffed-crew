//! Copy and convert one PipeWire video buffer into the RGB image the
//! capture pipeline already consumes.
//!
//! A buffer the capture loop did not ask for is dropped here with no
//! copy. Conversion runs only on bytes that were copied for a grab.

use image::{DynamicImage, RgbImage};

/// What the stream thread does with one delivered buffer.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BufferAction {
    /// Requeue it. Do not read the pixels.
    Drop,
    /// Keep this buffer. Earlier buffers in the same callback are still dropped.
    Copy,
}

/// `capture_requested` is set only while a grab is waiting.
pub fn buffer_action(capture_requested: bool) -> BufferAction {
    if capture_requested {
        BufferAction::Copy
    } else {
        BufferAction::Drop
    }
}

/// SPA video formats this route can turn into RGB.
/// Memory order matches PipeWire: `xBGR` is padding, blue, green, red.
pub const FMT_RGBX: u32 = 7;
pub const FMT_BGRX: u32 = 8;
pub const FMT_XRGB: u32 = 9;
pub const FMT_XBGR: u32 = 10;
pub const FMT_RGBA: u32 = 11;
pub const FMT_BGRA: u32 = 12;
pub const FMT_ARGB: u32 = 13;
pub const FMT_ABGR: u32 = 14;
pub const FMT_RGB: u32 = 15;
pub const FMT_BGR: u32 = 16;

/// Where red, green, and blue sit in one pixel, and how many bytes it uses.
#[derive(Clone, Copy)]
struct RgbOrder {
    r: usize,
    g: usize,
    b: usize,
    bpp: usize,
}

fn rgb_order(format: u32) -> Option<RgbOrder> {
    match format {
        FMT_RGB => Some(RgbOrder {
            r: 0,
            g: 1,
            b: 2,
            bpp: 3,
        }),
        FMT_BGR => Some(RgbOrder {
            r: 2,
            g: 1,
            b: 0,
            bpp: 3,
        }),
        FMT_RGBX | FMT_RGBA => Some(RgbOrder {
            r: 0,
            g: 1,
            b: 2,
            bpp: 4,
        }),
        FMT_BGRX | FMT_BGRA => Some(RgbOrder {
            r: 2,
            g: 1,
            b: 0,
            bpp: 4,
        }),
        FMT_XRGB | FMT_ARGB => Some(RgbOrder {
            r: 1,
            g: 2,
            b: 3,
            bpp: 4,
        }),
        FMT_XBGR | FMT_ABGR => Some(RgbOrder {
            r: 3,
            g: 2,
            b: 1,
            bpp: 4,
        }),
        _ => None,
    }
}

/// Sequence and presentation time of one dequeued buffer.
///
/// `seq` is a monotonic count of dequeues, including ones that were not
/// copied. `pts` is the PipeWire header presentation time, or 0 when the
/// buffer has no header.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct FrameClock {
    pub seq: u64,
    pub pts: i64,
}

/// High-water mark taken at the start of each grab. A frame is fresh only
/// when its sequence (and, when both sides have one, its presentation time)
/// is strictly newer than this.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OpenMarker {
    pub seq: u64,
    pub pts: i64,
}

/// How a packed frame is turned into RGB. Packed strides never walk one
/// pixel at a time through `put_pixel`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(not(test), allow(dead_code))]
pub enum RowCopy {
    /// RGB with `stride == width * 3`: one copy of the whole buffer.
    WholeBuffer,
    /// One `copy_from_slice` per row, then a bulk channel swizzle in that row.
    PerRow,
}

/// Milliseconds after a Tab press before a stream frame can show the
/// scoreboard overlay. The stream path waits for a buffer instead of
/// sleeping a fixed 400 ms.
pub const TAB_OVERLAY_AFTER_PRESS_MS: u64 = 300;

/// Hard stop for that wait, measured from the same press. This is a
/// constant, not a config key. A 2 fps cap (the default is 4, a member can
/// set 2) can leave about half a second between buffers, so the stop sits past one of those gaps. A later check
/// still accepts a buffer that arrived in the gap once its arrival is at or
/// after the overlay delay.
pub const TAB_GRAB_TIMEOUT_AFTER_PRESS_MS: u64 = 1200;

/// What to do with the newest stream buffer for one Tab press.
///
/// Times are milliseconds on one clock. `press_ms` is the key event.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TabFrameChoice {
    /// This buffer arrived at or after `press + 300 ms`. Use it.
    Take,
    /// The deadline has not passed and nothing new enough is held.
    Wait,
    /// The deadline passed and the newest buffer is still too old.
    Miss,
}

/// First buffer at or after the overlay delay wins, including one that
/// arrives after the hard timeout (a later Tab retry, or the next look).
/// A buffer from before the overlay is never taken.
pub fn select_tab_frame(arrived_ms: Option<u64>, press_ms: u64, now_ms: u64) -> TabFrameChoice {
    let ready = press_ms.saturating_add(TAB_OVERLAY_AFTER_PRESS_MS);
    let deadline = press_ms.saturating_add(TAB_GRAB_TIMEOUT_AFTER_PRESS_MS);
    if arrived_ms.is_some_and(|at| at >= ready) {
        TabFrameChoice::Take
    } else if now_ms >= deadline {
        TabFrameChoice::Miss
    } else {
        TabFrameChoice::Wait
    }
}

/// The press a Tab retry hands the stream grab, given the instant the retry
/// starts. A retry is not a new key press: the overlay delay already passed
/// for the real press, so the grab takes the next buffer that arrives from
/// `now` on instead of waiting another 300 ms on top. The hard stop is then
/// 900 ms after `now`, still past one 2 fps gap.
pub fn tab_retry_press(now: std::time::Instant) -> std::time::Instant {
    now.checked_sub(std::time::Duration::from_millis(TAB_OVERLAY_AFTER_PRESS_MS))
        .unwrap_or(now)
}

/// Delay from the Tab press to the chosen frame, in milliseconds.
pub fn tab_frame_delay_ms(press_ms: u64, arrived_ms: u64) -> u64 {
    arrived_ms.saturating_sub(press_ms)
}

/// One copied frame. The bytes are the packed pixels, including row padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFrame {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bytes: Vec<u8>,
    pub clock: FrameClock,
    /// When this buffer was dequeued, milliseconds on the stream clock.
    /// `0` until the stream thread stamps it.
    pub arrived_ms: u64,
}

/// True when `frame` was produced after `opened`. A non-positive presentation
/// time is ignored, so a buffer with no header is judged on sequence alone.
pub fn frame_is_fresh(frame: FrameClock, opened: OpenMarker) -> bool {
    if frame.seq <= opened.seq {
        return false;
    }
    if frame.pts > 0 && opened.pts > 0 && frame.pts <= opened.pts {
        return false;
    }
    true
}

/// True for a zero-size image or one whose RGB samples are all zero.
/// `rgb` is tightly packed RGB. Padding and alpha are not in this slice.
#[cfg_attr(not(test), allow(dead_code))]
pub fn is_blank_or_empty(width: u32, height: u32, rgb: &[u8]) -> bool {
    if width == 0 || height == 0 {
        return true;
    }
    let samples = (width as usize)
        .saturating_mul(height as usize)
        .saturating_mul(3);
    if samples == 0 || rgb.len() < samples {
        return true;
    }
    rgb[..samples].iter().all(|byte| *byte == 0)
}

/// The row-copy plan for a packed frame. `None` when the stride is shorter
/// than one row.
#[cfg_attr(not(test), allow(dead_code))]
pub fn row_copy_plan(format: u32, width: u32, stride: u32) -> Option<RowCopy> {
    let bpp = bytes_per_pixel(format)?;
    let row = width.checked_mul(bpp)?;
    if width == 0 || stride < row {
        return None;
    }
    if format == FMT_RGB && stride == row {
        Some(RowCopy::WholeBuffer)
    } else {
        Some(RowCopy::PerRow)
    }
}

pub fn bytes_per_pixel(format: u32) -> Option<u32> {
    rgb_order(format).map(|order| order.bpp as u32)
}

/// Copy `src` into an owned frame. `None` when the size or format is not
/// something the rest of the pipeline can read. This is the only copy.
pub fn pack(
    format: u32,
    width: u32,
    height: u32,
    stride: u32,
    src: &[u8],
    clock: FrameClock,
) -> Option<PackedFrame> {
    let bpp = bytes_per_pixel(format)?;
    if width == 0 || height == 0 {
        return None;
    }
    let row = width.checked_mul(bpp)?;
    let stride = if stride == 0 { row } else { stride };
    if stride < row {
        return None;
    }
    let need = (stride as usize).checked_mul(height as usize)?;
    if src.len() < need {
        return None;
    }
    Some(PackedFrame {
        format,
        width,
        height,
        stride,
        bytes: src[..need].to_vec(),
        clock,
        arrived_ms: 0,
    })
}

/// True when every RGB sample is zero, or the frame has no area. Alpha and
/// row padding are ignored.
pub fn frame_is_blank(frame: &PackedFrame) -> bool {
    if frame.width == 0 || frame.height == 0 || frame.bytes.is_empty() {
        return true;
    }
    let Some(order) = rgb_order(frame.format) else {
        return true;
    };
    let stride = frame.stride as usize;
    let width = frame.width as usize;
    let bpp = order.bpp;
    for y in 0..frame.height as usize {
        let row = y * stride;
        for x in 0..width {
            let i = row + x * bpp;
            if frame.bytes.get(i + order.r).copied().unwrap_or(1) != 0
                || frame.bytes.get(i + order.g).copied().unwrap_or(1) != 0
                || frame.bytes.get(i + order.b).copied().unwrap_or(1) != 0
            {
                return false;
            }
        }
    }
    true
}

/// RGB image, top-left origin, no padding. Rows are copied with
/// `copy_from_slice`. Channel swizzle runs across the row, not through
/// `put_pixel`.
pub fn to_rgb(frame: &PackedFrame) -> Option<RgbImage> {
    let order = rgb_order(frame.format)?;
    let bpp = order.bpp;
    let width = frame.width as usize;
    let height = frame.height as usize;
    let stride = frame.stride as usize;
    let row = width * bpp;
    if width == 0 || height == 0 || stride < row {
        return None;
    }
    let mut buf = vec![0u8; width * height * 3];
    let tight_rgb = order.bpp == 3 && order.r == 0 && order.g == 1 && order.b == 2;
    if tight_rgb && stride == row {
        let need = row * height;
        buf.copy_from_slice(&frame.bytes[..need]);
    } else {
        copy_rows(&frame.bytes, &mut buf, height, stride, width, order);
    }
    RgbImage::from_raw(frame.width, frame.height, buf)
}

fn copy_rows(
    src: &[u8],
    dst: &mut [u8],
    height: usize,
    stride: usize,
    width: usize,
    order: RgbOrder,
) {
    let bpp = order.bpp;
    for y in 0..height {
        let row_src = &src[y * stride..y * stride + width * bpp];
        let row_dst = &mut dst[y * width * 3..(y + 1) * width * 3];
        if bpp == 3 && order.r == 0 && order.g == 1 && order.b == 2 {
            row_dst.copy_from_slice(row_src);
            continue;
        }
        let (pixels, _) = row_dst.as_chunks_mut::<3>();
        for (chunk, px) in row_src.chunks_exact(bpp).zip(pixels) {
            px[0] = chunk[order.r];
            px[1] = chunk[order.g];
            px[2] = chunk[order.b];
        }
    }
}

pub fn to_dynamic(frame: &PackedFrame) -> Option<DynamicImage> {
    to_rgb(frame).map(DynamicImage::ImageRgb8)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn clock() -> FrameClock {
        FrameClock { seq: 1, pts: 1 }
    }

    #[test]
    fn an_unrequested_buffer_is_not_copied() {
        assert_eq!(buffer_action(false), BufferAction::Drop);
        assert_eq!(buffer_action(true), BufferAction::Copy);
    }

    #[test]
    fn a_red_pixel_stays_red_for_bgrx_and_rgbx_with_padded_stride() {
        // One pixel, stride 16: four live bytes and twelve of padding.
        // A red/blue swap here paints the wrong hero and the wrong result word.
        let mut bgrx = [0x11u8; 16];
        bgrx[0] = 0;
        bgrx[1] = 0;
        bgrx[2] = 255;
        bgrx[3] = 0;
        let frame = pack(FMT_BGRX, 1, 1, 16, &bgrx, clock()).unwrap();
        assert_eq!(to_rgb(&frame).unwrap().get_pixel(0, 0).0, [255, 0, 0]);

        let mut rgbx = [0x22u8; 16];
        rgbx[0] = 255;
        rgbx[1] = 0;
        rgbx[2] = 0;
        rgbx[3] = 0;
        let frame = pack(FMT_RGBX, 1, 1, 16, &rgbx, clock()).unwrap();
        assert_eq!(to_rgb(&frame).unwrap().get_pixel(0, 0).0, [255, 0, 0]);

        let mut xbgr = [0x33u8; 16];
        xbgr[0] = 0;
        xbgr[1] = 0;
        xbgr[2] = 0;
        xbgr[3] = 255;
        let frame = pack(FMT_XBGR, 1, 1, 16, &xbgr, clock()).unwrap();
        assert_eq!(to_rgb(&frame).unwrap().get_pixel(0, 0).0, [255, 0, 0]);

        let mut bgra = [0u8; 8];
        bgra[2] = 255;
        bgra[3] = 255;
        let frame = pack(FMT_BGRA, 1, 1, 8, &bgra, clock()).unwrap();
        assert_eq!(to_rgb(&frame).unwrap().get_pixel(0, 0).0, [255, 0, 0]);

        let mut rgba = [0u8; 8];
        rgba[0] = 255;
        rgba[3] = 255;
        let frame = pack(FMT_RGBA, 1, 1, 8, &rgba, clock()).unwrap();
        assert_eq!(to_rgb(&frame).unwrap().get_pixel(0, 0).0, [255, 0, 0]);
    }

    #[test]
    fn bgrx_with_row_padding_becomes_rgb() {
        // Two pixels, stride 12, padding after each row. BGRx blue then red.
        let src = [
            0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00, 0xaa, 0xbb, 0xcc, 0xdd,
        ];
        let frame = pack(FMT_BGRX, 2, 1, 12, &src, clock()).unwrap();
        assert_eq!(frame.bytes.len(), 12);
        let rgb = to_rgb(&frame).unwrap();
        assert_eq!(rgb.get_pixel(0, 0).0, [0x00, 0x00, 0xff]);
        assert_eq!(rgb.get_pixel(1, 0).0, [0xff, 0x00, 0x00]);
    }

    #[test]
    fn rgbx_rgba_and_bgr_map_channels() {
        let rgbx = pack(FMT_RGBX, 1, 1, 0, &[1, 2, 3, 4], clock()).unwrap();
        assert_eq!(to_rgb(&rgbx).unwrap().get_pixel(0, 0).0, [1, 2, 3]);
        let rgba = pack(FMT_RGBA, 1, 1, 4, &[9, 8, 7, 6], clock()).unwrap();
        assert_eq!(to_rgb(&rgba).unwrap().get_pixel(0, 0).0, [9, 8, 7]);
        let bgr = pack(FMT_BGR, 1, 1, 3, &[1, 2, 3], clock()).unwrap();
        assert_eq!(to_rgb(&bgr).unwrap().get_pixel(0, 0).0, [3, 2, 1]);
        let rgb = pack(FMT_RGB, 1, 1, 3, &[4, 5, 6], clock()).unwrap();
        assert_eq!(to_rgb(&rgb).unwrap().get_pixel(0, 0).0, [4, 5, 6]);
    }

    #[test]
    fn a_short_or_unknown_buffer_is_refused() {
        assert!(pack(FMT_BGRA, 2, 2, 8, &[0; 8], clock()).is_none());
        assert!(pack(1, 2, 2, 8, &[0; 64], clock()).is_none());
        assert!(pack(FMT_RGB, 0, 1, 0, &[], clock()).is_none());
    }

    #[test]
    fn a_frame_must_be_newer_than_the_open_marker() {
        let opened = OpenMarker {
            seq: 10,
            pts: 1_000,
        };
        assert!(!frame_is_fresh(
            FrameClock {
                seq: 10,
                pts: 1_001
            },
            opened
        ));
        assert!(!frame_is_fresh(
            FrameClock {
                seq: 11,
                pts: 1_000
            },
            opened
        ));
        assert!(frame_is_fresh(
            FrameClock {
                seq: 11,
                pts: 1_001
            },
            opened
        ));
        assert!(frame_is_fresh(
            FrameClock { seq: 11, pts: 0 },
            OpenMarker { seq: 10, pts: 0 }
        ));
        assert!(!frame_is_fresh(
            FrameClock { seq: 4, pts: 50 },
            OpenMarker { seq: 4, pts: 40 }
        ));
    }

    #[test]
    fn a_black_or_empty_frame_is_rejected() {
        assert!(is_blank_or_empty(0, 10, &[1, 2, 3]));
        assert!(is_blank_or_empty(1, 1, &[0, 0, 0]));
        assert!(!is_blank_or_empty(1, 1, &[0, 0, 1]));
        let black = pack(FMT_RGB, 2, 1, 6, &[0; 6], clock()).unwrap();
        assert!(frame_is_blank(&black));
        let pixel = pack(FMT_BGRX, 1, 1, 4, &[0, 0, 1, 0], clock()).unwrap();
        assert!(!frame_is_blank(&pixel));
    }

    #[test]
    fn packed_strides_use_the_row_copy_path() {
        assert_eq!(
            row_copy_plan(FMT_RGB, 1920, 1920 * 3),
            Some(RowCopy::WholeBuffer)
        );
        assert_eq!(
            row_copy_plan(FMT_BGRX, 1920, 1920 * 4),
            Some(RowCopy::PerRow)
        );
        assert_eq!(row_copy_plan(FMT_RGB, 8, 8 * 3 + 4), Some(RowCopy::PerRow));
        let width = 64u32;
        let height = 32u32;
        let mut bytes = vec![0u8; (width * height * 4) as usize];
        bytes[2] = 9;
        let frame = pack(FMT_BGRX, width, height, width * 4, &bytes, clock()).unwrap();
        assert_eq!(
            row_copy_plan(frame.format, frame.width, frame.stride),
            Some(RowCopy::PerRow)
        );
        let started = std::time::Instant::now();
        let rgb = to_rgb(&frame).unwrap();
        assert!(
            started.elapsed() < std::time::Duration::from_millis(200),
            "row copy took {:?}",
            started.elapsed()
        );
        assert_eq!(rgb.get_pixel(0, 0).0, [9, 0, 0]);
        let packed = pack(FMT_RGB, 4, 2, 12, &[7u8; 24], clock()).unwrap();
        assert_eq!(
            row_copy_plan(packed.format, packed.width, packed.stride),
            Some(RowCopy::WholeBuffer)
        );
        assert_eq!(to_rgb(&packed).unwrap().as_raw(), &[7u8; 24]);
    }

    #[test]
    fn a_tab_frame_must_arrive_at_or_after_the_overlay_delay() {
        let press = 5_000u64;
        let early = press + 200;
        let ready = press + TAB_OVERLAY_AFTER_PRESS_MS;
        let late = press + 2_000;
        let before_deadline = press + 700;
        let deadline = press + TAB_GRAB_TIMEOUT_AFTER_PRESS_MS;
        assert_eq!(TAB_OVERLAY_AFTER_PRESS_MS, 300);
        assert_eq!(TAB_GRAB_TIMEOUT_AFTER_PRESS_MS, 1200);
        assert_eq!(
            select_tab_frame(None, press, before_deadline),
            TabFrameChoice::Wait
        );
        assert_eq!(
            select_tab_frame(Some(early), press, before_deadline),
            TabFrameChoice::Wait
        );
        assert_eq!(
            select_tab_frame(Some(ready), press, before_deadline),
            TabFrameChoice::Take
        );
        assert_eq!(
            select_tab_frame(Some(early), press, deadline),
            TabFrameChoice::Miss
        );
        assert_eq!(
            select_tab_frame(None, press, deadline),
            TabFrameChoice::Miss
        );
        // Past the hard timeout, a buffer that did land after the overlay
        // is still the one to use. A 2 fps gap of about 2 s hits this path
        // on the retry, instead of a frame from before the scoreboard.
        assert_eq!(
            select_tab_frame(Some(late), press, late),
            TabFrameChoice::Take
        );
        assert_eq!(tab_frame_delay_ms(press, late), 2_000);
        assert_eq!(tab_frame_delay_ms(press, ready), 300);
    }
    /// 2026-10-21 (rc.5, niri, 2 fps): each retry faked a new press at
    /// `now`, so it waited another 300 ms plus the 2 fps grid and its
    /// frames landed a full 1 s and 2 s after the first. A retry now takes
    /// the first buffer that arrives after it starts.
    #[test]
    fn a_retry_takes_the_next_buffer_without_another_overlay_delay() {
        let base = std::time::Instant::now();
        let now = base + std::time::Duration::from_secs(10);
        let ms = |at: std::time::Instant| at.duration_since(base).as_millis() as u64;
        let press = ms(tab_retry_press(now));
        let now_ms = ms(now);
        assert_eq!(now_ms - press, TAB_OVERLAY_AFTER_PRESS_MS);
        // A buffer 1 ms after the retry starts is taken.
        assert_eq!(
            select_tab_frame(Some(now_ms + 1), press, now_ms + 1),
            TabFrameChoice::Take
        );
        // A buffer from before the retry (the rejected frame) is not.
        assert_eq!(
            select_tab_frame(Some(now_ms - 1), press, now_ms + 1),
            TabFrameChoice::Wait
        );
        // The grab still waits across one 2 fps gap (500 ms) for it.
        assert_eq!(
            select_tab_frame(None, press, now_ms + 600),
            TabFrameChoice::Wait
        );
        // Right after process start the subtraction cannot underflow.
        let _ = tab_retry_press(std::time::Instant::now());
    }
}
