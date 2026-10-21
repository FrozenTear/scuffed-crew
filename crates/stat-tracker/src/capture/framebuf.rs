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
pub const FMT_RGBX: u32 = 7;
pub const FMT_BGRX: u32 = 8;
pub const FMT_RGBA: u32 = 11;
pub const FMT_BGRA: u32 = 12;
pub const FMT_RGB: u32 = 15;
pub const FMT_BGR: u32 = 16;

/// One copied frame. The bytes are the packed pixels, including row padding.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFrame {
    pub format: u32,
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub bytes: Vec<u8>,
}

pub fn bytes_per_pixel(format: u32) -> Option<u32> {
    match format {
        FMT_RGBX | FMT_BGRX | FMT_RGBA | FMT_BGRA => Some(4),
        FMT_RGB | FMT_BGR => Some(3),
        _ => None,
    }
}

/// Copy `src` into an owned frame. `None` when the size or format is not
/// something the rest of the pipeline can read. This is the only copy.
pub fn pack(format: u32, width: u32, height: u32, stride: u32, src: &[u8]) -> Option<PackedFrame> {
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
    })
}

/// RGB image, top-left origin, no padding. `None` for a format we did not pack.
pub fn to_rgb(frame: &PackedFrame) -> Option<RgbImage> {
    let bpp = bytes_per_pixel(frame.format)?;
    let mut img = RgbImage::new(frame.width, frame.height);
    for y in 0..frame.height {
        let row_off = y as usize * frame.stride as usize;
        for x in 0..frame.width {
            let i = row_off + x as usize * bpp as usize;
            let px = &frame.bytes[i..i + bpp as usize];
            let rgb = match frame.format {
                FMT_RGBX | FMT_RGBA | FMT_RGB => [px[0], px[1], px[2]],
                FMT_BGRX | FMT_BGRA | FMT_BGR => [px[2], px[1], px[0]],
                _ => return None,
            };
            img.put_pixel(x, y, image::Rgb(rgb));
        }
    }
    Some(img)
}

pub fn to_dynamic(frame: &PackedFrame) -> Option<DynamicImage> {
    to_rgb(frame).map(DynamicImage::ImageRgb8)
}

/// Among buffers delivered together, only the last one is copied when a
/// grab is waiting. The rest are dropped. When no grab is waiting, all of
/// them are dropped and nothing is copied.
#[cfg_attr(not(test), allow(dead_code))]
pub fn keep_latest(capture_requested: bool, buffers: usize) -> usize {
    match buffer_action(capture_requested) {
        BufferAction::Drop => 0,
        BufferAction::Copy => usize::from(buffers > 0),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unrequested_buffer_is_not_copied() {
        assert_eq!(buffer_action(false), BufferAction::Drop);
        assert_eq!(buffer_action(true), BufferAction::Copy);
        assert_eq!(keep_latest(false, 4), 0);
        assert_eq!(keep_latest(true, 0), 0);
        assert_eq!(keep_latest(true, 3), 1);
    }

    #[test]
    fn bgrx_with_row_padding_becomes_rgb() {
        // Two pixels, stride 12, padding after each row. BGRx blue then red.
        let src = [
            0xff, 0x00, 0x00, 0x00, 0x00, 0x00, 0xff, 0x00, 0xaa, 0xbb, 0xcc, 0xdd,
        ];
        let frame = pack(FMT_BGRX, 2, 1, 12, &src).unwrap();
        assert_eq!(frame.bytes.len(), 12);
        let rgb = to_rgb(&frame).unwrap();
        assert_eq!(rgb.get_pixel(0, 0).0, [0x00, 0x00, 0xff]);
        assert_eq!(rgb.get_pixel(1, 0).0, [0xff, 0x00, 0x00]);
    }

    #[test]
    fn rgbx_rgba_and_bgr_map_channels() {
        let rgbx = pack(FMT_RGBX, 1, 1, 0, &[1, 2, 3, 4]).unwrap();
        assert_eq!(to_rgb(&rgbx).unwrap().get_pixel(0, 0).0, [1, 2, 3]);
        let rgba = pack(FMT_RGBA, 1, 1, 4, &[9, 8, 7, 6]).unwrap();
        assert_eq!(to_rgb(&rgba).unwrap().get_pixel(0, 0).0, [9, 8, 7]);
        let bgr = pack(FMT_BGR, 1, 1, 3, &[1, 2, 3]).unwrap();
        assert_eq!(to_rgb(&bgr).unwrap().get_pixel(0, 0).0, [3, 2, 1]);
        let rgb = pack(FMT_RGB, 1, 1, 3, &[4, 5, 6]).unwrap();
        assert_eq!(to_rgb(&rgb).unwrap().get_pixel(0, 0).0, [4, 5, 6]);
    }

    #[test]
    fn a_short_or_unknown_buffer_is_refused() {
        assert!(pack(FMT_BGRA, 2, 2, 8, &[0; 8]).is_none());
        assert!(pack(1, 2, 2, 8, &[0; 64]).is_none());
        assert!(pack(FMT_RGB, 0, 1, 0, &[]).is_none());
    }
}
