//! Reading one dequeued PipeWire buffer: the plane window, the header
//! presentation time, and the copy out of mapped memory.

use std::sync::atomic::Ordering;

use super::ffi::{
    PwBuffer, SPA_DATA_DMA_BUF, SPA_DATA_MEM_FD, SPA_DATA_MEM_PTR, SPA_META_HEADER, SpaChunk,
    SpaData, SpaMeta, SpaMetaHeader,
};
use super::{PackedFrame, State, data_type_name, framebuf};

/// Bytes to copy from a mapped buffer. `None` rejects DMA-sized holes:
/// `maxsize == 0`, a chunk shorter than `stride * height`, or a window that
/// would run past the mapping. `offset` is taken modulo `maxsize`.
pub fn plane_window(
    offset: u32,
    maxsize: u32,
    chunk_size: u32,
    stride: u32,
    height: u32,
) -> Option<(usize, usize)> {
    if maxsize == 0 || stride == 0 || height == 0 {
        return None;
    }
    let need = (stride as usize).checked_mul(height as usize)?;
    if (chunk_size as usize) < need {
        return None;
    }
    let offset = (offset as usize) % (maxsize as usize);
    if offset.saturating_add(need) > maxsize as usize {
        return None;
    }
    Some((offset, need))
}

pub(super) fn pts_from_header(bytes: &[u8]) -> Option<i64> {
    let pts_at = std::mem::offset_of!(SpaMetaHeader, pts);
    let pts_end = pts_at + std::mem::size_of::<i64>();
    if bytes.len() < pts_end {
        return None;
    }
    let mut pts = [0u8; 8];
    pts.copy_from_slice(&bytes[pts_at..pts_end]);
    Some(i64::from_ne_bytes(pts))
}

pub(super) fn header_pts(buf: *mut PwBuffer) -> Option<i64> {
    if buf.is_null() {
        return None;
    }
    let pw = unsafe { &*buf };
    if pw.buffer.is_null() {
        return None;
    }
    let spa = unsafe { &*pw.buffer };
    if spa.n_metas == 0 || spa.metas.is_null() || spa.n_metas > 16 {
        return None;
    }
    let metas =
        unsafe { std::slice::from_raw_parts(spa.metas as *const SpaMeta, spa.n_metas as usize) };
    for meta in metas {
        if meta.type_ == SPA_META_HEADER && !meta.data.is_null() {
            let len = meta.size as usize;
            let bytes = unsafe { std::slice::from_raw_parts(meta.data as *const u8, len) };
            if let Some(pts) = pts_from_header(bytes) {
                return Some(pts);
            }
        }
    }
    None
}

/// What `buf` is, for the performance log: the first data plane's memory
/// type and chunk size, and the negotiated format and size.
pub(super) fn buffer_info(state: &State, buf: *mut PwBuffer) -> Option<crate::perf::StreamBuffer> {
    if buf.is_null() {
        return None;
    }
    let pw = unsafe { &*buf };
    if pw.buffer.is_null() {
        return None;
    }
    let spa = unsafe { &*pw.buffer };
    if spa.n_datas == 0 || spa.datas.is_null() {
        return None;
    }
    let data = unsafe { &*spa.datas };
    let bytes = unsafe { data.chunk.as_ref() }.map_or(0, |c| c.size);
    Some(crate::perf::StreamBuffer {
        kind: data_type_name(data.type_),
        bytes,
        format: framebuf::format_name(state.format.load(Ordering::Acquire)),
        width: state.width.load(Ordering::Acquire),
        height: state.height.load(Ordering::Acquire),
    })
}

pub(super) fn copy_buffer(
    state: &State,
    buf: *mut PwBuffer,
    seq: u64,
    pts: i64,
    now_ms: u64,
) -> Option<PackedFrame> {
    if buf.is_null() {
        return None;
    }
    let pw = unsafe { &*buf };
    if pw.buffer.is_null() {
        return None;
    }
    let spa = unsafe { &*pw.buffer };
    if spa.n_datas == 0 || spa.datas.is_null() {
        return None;
    }
    let data = unsafe { &*spa.datas };
    if data.type_ == SPA_DATA_DMA_BUF {
        return None;
    }
    let chunk = unsafe { data.chunk.as_ref()? };
    // SPA_CHUNK_FLAG_CORRUPTED is 1, SPA_CHUNK_FLAG_EMPTY is 2.
    if chunk.flags & 0b11 != 0 {
        return None;
    }
    let format = state.format.load(Ordering::Acquire);
    let bpp = framebuf::bytes_per_pixel(format)?;
    let width = state.width.load(Ordering::Acquire);
    let mut height = state.height.load(Ordering::Acquire);
    let stride = if chunk.stride > 0 {
        chunk.stride as u32
    } else if width > 0 {
        width.saturating_mul(bpp)
    } else {
        return None;
    };
    if height == 0 && stride > 0 {
        height = chunk.size / stride;
    }
    if width == 0 || height == 0 {
        return None;
    }
    let (_, need) = plane_window(chunk.offset, data.maxsize, chunk.size, stride, height)?;
    // One copy, inside pack. The mapping is gone before this returns.
    read_bytes(data, chunk, need, |bytes| {
        let mut frame = framebuf::pack(
            format,
            width,
            height,
            stride,
            bytes,
            framebuf::FrameClock { seq, pts },
        )?;
        frame.arrived_ms = now_ms;
        Some(frame)
    })
}

fn read_bytes<T>(
    data: &SpaData,
    chunk: &SpaChunk,
    need: usize,
    use_bytes: impl FnOnce(&[u8]) -> Option<T>,
) -> Option<T> {
    if data.type_ == SPA_DATA_DMA_BUF || data.maxsize == 0 || need == 0 {
        return None;
    }
    let (offset, window) = plane_window(
        chunk.offset,
        data.maxsize,
        chunk.size,
        u32::try_from(need).ok()?,
        1,
    )?;
    if window < need {
        return None;
    }
    if !data.data.is_null() && matches!(data.type_, SPA_DATA_MEM_PTR | SPA_DATA_MEM_FD) {
        let ptr = unsafe { (data.data as *const u8).add(offset) };
        let bytes = unsafe { std::slice::from_raw_parts(ptr, need) };
        return use_bytes(bytes);
    }
    if data.fd < 0 || data.type_ != SPA_DATA_MEM_FD {
        return None;
    }
    // Never map past maxsize. A longer map of a memfd faults on read.
    let map_len = data.maxsize as usize;
    if offset.saturating_add(need) > map_len {
        return None;
    }
    // SAFETY: the fd belongs to this buffer until it is queued back, which
    // happens after this function returns. The mapping is read-only, shared,
    // and removed before return. The offset is the buffer's map offset.
    let mapped = unsafe {
        libc::mmap(
            std::ptr::null_mut(),
            map_len,
            libc::PROT_READ,
            libc::MAP_SHARED,
            data.fd as i32,
            data.mapoffset as i64,
        )
    };
    if mapped == libc::MAP_FAILED {
        return None;
    }
    let packed = unsafe {
        let ptr = (mapped as *const u8).add(offset.min(map_len.saturating_sub(1)));
        let len = need.min(map_len.saturating_sub(offset));
        let bytes = std::slice::from_raw_parts(ptr, len);
        if bytes.len() < need {
            None
        } else {
            use_bytes(bytes)
        }
    };
    unsafe { libc::munmap(mapped, map_len) };
    packed
}
