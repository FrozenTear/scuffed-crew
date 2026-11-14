//! Plain ustar reader for reader packs. `read_ustar` is the check the
//! cutter tests run a packed tar through.

use super::{PACK_MISMATCH, PACK_UNSAFE, PackLayout};

pub(crate) fn safe_rel_path(raw: &str) -> Option<String> {
    if raw.is_empty() || raw.starts_with('/') || raw.starts_with('\\') {
        return None;
    }
    if raw.as_bytes().get(1) == Some(&b':') {
        return None;
    }
    let mut parts = Vec::new();
    for part in raw.split(['/', '\\']) {
        if part.is_empty() || part == "." {
            continue;
        }
        if part == ".." || part.contains('\0') {
            return None;
        }
        parts.push(part);
    }
    if parts.is_empty() {
        return None;
    }
    Some(parts.join("/"))
}
pub(crate) fn read_ustar(
    bytes: &[u8],
    layout: PackLayout,
) -> Result<Vec<(String, Vec<u8>)>, &'static str> {
    let mut offset = 0usize;
    let mut files = Vec::new();
    while offset + 512 <= bytes.len() {
        let header = &bytes[offset..offset + 512];
        if header.iter().all(|byte| *byte == 0) {
            break;
        }
        if !checksum_ok(header) {
            return Err(PACK_UNSAFE);
        }
        let typeflag = header[156];
        if header[124] & 0x80 != 0 {
            return Err(PACK_UNSAFE);
        }
        let size = parse_octal(&header[124..136]).map_err(|_| PACK_UNSAFE)?;
        let name = entry_name(header).ok_or(PACK_UNSAFE)?;
        offset += 512;
        let data_end = offset.saturating_add(size as usize);
        if data_end > bytes.len() {
            return Err(PACK_UNSAFE);
        }
        let data = bytes[offset..data_end].to_vec();
        let padded = size.div_ceil(512) * 512;
        offset = offset.saturating_add(padded as usize);
        match typeflag {
            b'0' | b'\0' => {
                let path = layout.file_path(&name)?;
                files.push((path, data));
            }
            b'5' => {
                if !layout.allows_dir(&name) {
                    return Err(PACK_UNSAFE);
                }
            }
            _ => return Err(PACK_UNSAFE),
        }
    }
    if files.is_empty() {
        return Err(PACK_MISMATCH);
    }
    Ok(files)
}

fn entry_name(header: &[u8]) -> Option<String> {
    let name = c_string(&header[..100]);
    let prefix = c_string(&header[345..500]);
    let joined = if prefix.is_empty() {
        name
    } else {
        format!("{prefix}/{name}")
    };
    if joined.is_empty() {
        None
    } else {
        Some(joined)
    }
}

fn c_string(bytes: &[u8]) -> String {
    let end = bytes
        .iter()
        .position(|byte| *byte == 0)
        .unwrap_or(bytes.len());
    String::from_utf8_lossy(&bytes[..end]).trim().to_string()
}

fn parse_octal(field: &[u8]) -> Result<u64, ()> {
    let mut value = 0u64;
    let mut saw = false;
    for &byte in field {
        if byte == 0 || byte == b' ' {
            if saw {
                break;
            }
            continue;
        }
        if !byte.is_ascii_digit() || byte > b'7' {
            return Err(());
        }
        saw = true;
        value = value
            .checked_mul(8)
            .ok_or(())?
            .checked_add(u64::from(byte - b'0'))
            .ok_or(())?;
    }
    Ok(value)
}

fn checksum_ok(header: &[u8]) -> bool {
    let Ok(recorded) = parse_octal(&header[148..156]) else {
        return false;
    };
    let sum = header.iter().enumerate().fold(0u64, |sum, (index, byte)| {
        sum + u64::from(if (148..156).contains(&index) {
            b' '
        } else {
            *byte
        })
    });
    sum == recorded
}
