//! PNG chunk filter for report crops.
//!
//! Allowed chunks are `IHDR`, `IDAT`, `IEND`, and `PLTE` only on a palette
//! image. Ancillary chunks (text, EXIF, time, profiles, physical pixels) are
//! removed. Any other critical chunk is rejected.

use std::io::Read;

use flate2::read::ZlibDecoder;

const PNG_MAGIC: &[u8] = b"\x89PNG\r\n\x1a\n";
const JPEG_MAGIC: &[u8] = &[0xFF, 0xD8, 0xFF];

/// Decoded scanlines larger than this are rejected (inflate bomb guard).
const MAX_DECODED_BYTES: u64 = 160 * 1024 * 1024;
const MAX_CHUNKS: usize = 64;

/// Strip disallowed chunks and check the image is a real PNG.
pub fn prepare_png(input: &[u8]) -> Result<Vec<u8>, String> {
    if input.starts_with(JPEG_MAGIC) {
        return Err("jpeg is not allowed".into());
    }
    if input.len() < 8 || &input[..8] != PNG_MAGIC {
        return Err("not a png".into());
    }

    let mut i = 8usize;
    let mut out = Vec::with_capacity(input.len());
    out.extend_from_slice(PNG_MAGIC);

    let mut saw_ihdr = false;
    let mut saw_idat = false;
    let mut idat_finished = false;
    let mut saw_iend = false;
    let mut saw_plte = false;
    let mut chunks = 0usize;
    let mut width = 0u32;
    let mut height = 0u32;
    let mut color_type = 0u8;
    let mut idat = Vec::new();

    while i < input.len() {
        if saw_iend {
            return Err("bytes after iend".into());
        }
        if i + 12 > input.len() {
            return Err("truncated png chunk".into());
        }
        let len = u32::from_be_bytes(input[i..i + 4].try_into().unwrap()) as usize;
        if len > input.len() || i + 12 + len > input.len() {
            return Err("truncated png chunk".into());
        }
        chunks += 1;
        if chunks > MAX_CHUNKS {
            return Err("too many png chunks".into());
        }
        let ctype = &input[i + 4..i + 8];
        let data = &input[i + 8..i + 8 + len];
        let crc_got = u32::from_be_bytes(input[i + 8 + len..i + 12 + len].try_into().unwrap());
        let crc_expect = crc32(&input[i + 4..i + 8 + len]);
        if crc_got != crc_expect {
            return Err("png chunk crc mismatch".into());
        }
        if !ctype.iter().all(|b| b.is_ascii_alphabetic()) {
            return Err("png chunk type is not letters".into());
        }

        let keep = if ctype == b"IHDR" {
            if saw_ihdr || chunks != 1 || len != 13 {
                return Err("bad ihdr".into());
            }
            width = u32::from_be_bytes(data[0..4].try_into().unwrap());
            height = u32::from_be_bytes(data[4..8].try_into().unwrap());
            let bit_depth = data[8];
            color_type = data[9];
            let compression = data[10];
            let filter = data[11];
            let interlace = data[12];
            if width == 0
                || height == 0
                || width > scuffed_types::MAX_IMAGE_EDGE
                || height > scuffed_types::MAX_IMAGE_EDGE
            {
                return Err("image edge is out of range".into());
            }
            if bit_depth != 8 {
                return Err("png bit depth must be 8".into());
            }
            if color_type != 2 && color_type != 6 && color_type != 3 {
                return Err("png color type must be rgb, rgba, or palette".into());
            }
            if compression != 0 || filter != 0 || interlace != 0 {
                return Err("png compression, filter, or interlace is not allowed".into());
            }
            saw_ihdr = true;
            true
        } else if ctype == b"PLTE" {
            if color_type != 3
                || saw_idat
                || saw_plte
                || len == 0
                || !len.is_multiple_of(3)
                || len > 256 * 3
            {
                return Err("plte is only allowed on a palette image".into());
            }
            saw_plte = true;
            true
        } else if ctype == b"IDAT" {
            if !saw_ihdr || idat_finished || len == 0 {
                return Err("bad idat".into());
            }
            if color_type == 3 && !saw_plte {
                return Err("palette image is missing plte".into());
            }
            saw_idat = true;
            idat.extend_from_slice(data);
            if idat.len() as u64 > scuffed_types::MAX_BUNDLE_BYTES {
                return Err("idat is too large".into());
            }
            true
        } else if ctype == b"IEND" {
            if !saw_idat || len != 0 {
                return Err("bad iend".into());
            }
            saw_iend = true;
            true
        } else if is_ancillary(ctype) {
            if saw_idat && !idat_finished {
                idat_finished = true;
            }
            false
        } else {
            return Err("png has a critical chunk that is not allowed".into());
        };

        if ctype != b"IDAT" && saw_idat {
            idat_finished = true;
        }
        if keep {
            out.extend_from_slice(&input[i..i + 12 + len]);
        }
        i += 12 + len;
    }

    if !saw_iend {
        return Err("png is missing iend".into());
    }
    let bpp: u64 = match color_type {
        2 => 3,
        6 => 4,
        3 => 1,
        _ => return Err("png color type must be rgb, rgba, or palette".into()),
    };
    let expected = u64::from(height) * (1 + u64::from(width) * bpp);
    if expected == 0 || expected > MAX_DECODED_BYTES {
        return Err("decoded image is too large".into());
    }
    inflate_exact(&idat, expected)?;
    Ok(out)
}

fn is_ancillary(ctype: &[u8]) -> bool {
    ctype.first().is_some_and(|b| b.is_ascii_lowercase())
}

fn inflate_exact(idat: &[u8], expected: u64) -> Result<(), String> {
    let mut dec = ZlibDecoder::new(idat);
    let mut buf = [0u8; 8192];
    let mut total = 0u64;
    loop {
        let n = dec
            .read(&mut buf)
            .map_err(|_| "png idat is not valid zlib".to_string())?;
        if n == 0 {
            break;
        }
        total += n as u64;
        if total > expected {
            return Err("png idat inflates past the image size".into());
        }
    }
    if total != expected {
        return Err("png idat does not match the image size".into());
    }
    if dec.total_in() != idat.len() as u64 {
        return Err("png has bytes after the image data".into());
    }
    Ok(())
}

fn crc32(data: &[u8]) -> u32 {
    let mut crc: u32 = 0xFFFF_FFFF;
    for &byte in data {
        crc ^= u32::from(byte);
        for _ in 0..8 {
            let mask = 0u32.wrapping_sub(crc & 1);
            crc = (crc >> 1) ^ (0xEDB8_8320 & mask);
        }
    }
    !crc
}

#[cfg(test)]
mod tests {
    use super::*;
    use flate2::Compression;
    use flate2::write::ZlibEncoder;
    use std::io::Write;

    fn chunk(ctype: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(&(data.len() as u32).to_be_bytes());
        out.extend_from_slice(ctype);
        out.extend_from_slice(data);
        let crc = crc32(&out[4..]);
        out.extend_from_slice(&crc.to_be_bytes());
        out
    }

    fn tiny_rgb() -> Vec<u8> {
        let mut ihdr = [0u8; 13];
        ihdr[0..4].copy_from_slice(&1u32.to_be_bytes());
        ihdr[4..8].copy_from_slice(&1u32.to_be_bytes());
        ihdr[8] = 8;
        ihdr[9] = 2;
        let mut enc = ZlibEncoder::new(Vec::new(), Compression::default());
        enc.write_all(&[0, 255, 0, 0]).unwrap();
        let idat = enc.finish().unwrap();
        let mut png = Vec::new();
        png.extend_from_slice(PNG_MAGIC);
        png.extend(chunk(b"IHDR", &ihdr));
        png.extend(chunk(b"IDAT", &idat));
        png.extend(chunk(b"IEND", &[]));
        png
    }

    #[test]
    fn strips_text_chunk() {
        let base = tiny_rgb();
        let text = chunk(b"tEXt", b"Comment\0SECRETMETA");
        let iend_at = base.len() - 12;
        let mut with_text = base[..iend_at].to_vec();
        with_text.extend(text);
        with_text.extend_from_slice(&base[iend_at..]);
        assert!(with_text.windows(10).any(|w| w == b"SECRETMETA"));
        let stripped = prepare_png(&with_text).unwrap();
        assert!(!stripped.windows(10).any(|w| w == b"SECRETMETA"));
        assert!(!stripped.windows(4).any(|w| w == b"tEXt"));
        assert_eq!(&stripped[..8], PNG_MAGIC);
    }

    #[test]
    fn rejects_jpeg_magic() {
        let err = prepare_png(&[0xFF, 0xD8, 0xFF, 0x00]).unwrap_err();
        assert!(err.contains("jpeg"), "{err}");
    }

    #[test]
    fn rejects_text_after_iend_and_after_image_data() {
        let mut after_iend = tiny_rgb();
        after_iend.extend_from_slice(b"HIDDENTEXT");
        let err = prepare_png(&after_iend).unwrap_err();
        assert!(err.contains("iend") || err.contains("after"), "{err}");

        let hidden = append_idat_bytes(&tiny_rgb(), b"HIDDENTEXT");
        assert!(hidden.windows(10).any(|w| w == b"HIDDENTEXT"));
        let err = prepare_png(&hidden).unwrap_err();
        assert!(err.contains("after the image data"), "{err}");
    }

    fn append_idat_bytes(png: &[u8], extra: &[u8]) -> Vec<u8> {
        let mut i = 8usize;
        let mut out = png[..8].to_vec();
        while i + 12 <= png.len() {
            let len = u32::from_be_bytes(png[i..i + 4].try_into().unwrap()) as usize;
            let ctype = &png[i + 4..i + 8];
            let end = i + 12 + len;
            if ctype == b"IDAT" {
                let mut data = png[i + 8..i + 8 + len].to_vec();
                data.extend_from_slice(extra);
                out.extend(chunk(b"IDAT", &data));
            } else {
                out.extend_from_slice(&png[i..end]);
            }
            i = end;
            if ctype == b"IEND" {
                break;
            }
        }
        out
    }
}
