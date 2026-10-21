//! A PipeWire consumer for one portal ScreenCast node.
//!
//! `libpipewire-0.3.so.0` is opened at runtime. The crate does not link
//! PipeWire, so a host without it still builds and falls back to the
//! screenshot portal. The stream asks for at most 2 fps. Buffers are
//! requeued immediately. A frame is copied only while a grab is waiting,
//! and only the newest buffer in that callback is copied.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Duration;

use super::framebuf::{self, PackedFrame};
use super::pod;

const PW_ID_ANY: u32 = 0xffff_ffff;
const PW_DIRECTION_INPUT: i32 = 0;
const PW_STREAM_FLAG_AUTOCONNECT: u32 = 1;
const PW_STREAM_FLAG_MAP_BUFFERS: u32 = 4;
const PW_STREAM_STATE_ERROR: i32 = -1;

const SPA_DATA_MEM_PTR: u32 = 1;
const SPA_DATA_MEM_FD: u32 = 2;
const SPA_DATA_DMA_BUF: u32 = 3;
const SPA_PARAM_FORMAT: u32 = 4;

#[repr(C)]
struct SpaChunk {
    offset: u32,
    size: u32,
    stride: i32,
    flags: i32,
}

#[repr(C)]
struct SpaData {
    type_: u32,
    flags: u32,
    fd: i64,
    mapoffset: u32,
    maxsize: u32,
    data: *mut c_void,
    chunk: *mut SpaChunk,
}

#[repr(C)]
struct SpaBuffer {
    n_metas: u32,
    n_datas: u32,
    metas: *mut c_void,
    datas: *mut SpaData,
}

#[repr(C)]
struct PwBuffer {
    buffer: *mut SpaBuffer,
    user_data: *mut c_void,
    size: u64,
    requested: u64,
}

#[repr(C)]
struct SpaPod {
    size: u32,
    type_: u32,
}

#[repr(C)]
struct PwStreamEvents {
    version: u32,
    destroy: Option<extern "C" fn(*mut c_void)>,
    state_changed: Option<extern "C" fn(*mut c_void, i32, i32, *const c_char)>,
    control_info: Option<extern "C" fn(*mut c_void, u32, *const c_void)>,
    io_changed: Option<extern "C" fn(*mut c_void, u32, *mut c_void, u32)>,
    param_changed: Option<extern "C" fn(*mut c_void, u32, *const SpaPod)>,
    add_buffer: Option<extern "C" fn(*mut c_void, *mut PwBuffer)>,
    remove_buffer: Option<extern "C" fn(*mut c_void, *mut PwBuffer)>,
    process: Option<extern "C" fn(*mut c_void)>,
    drained: Option<extern "C" fn(*mut c_void)>,
    command: Option<extern "C" fn(*mut c_void, *const c_void)>,
    trigger_done: Option<extern "C" fn(*mut c_void)>,
}

type DequeueFn = unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer;
type QueueFn = unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> i32;
type UpdateFn = unsafe extern "C" fn(*mut c_void, *const *const SpaPod, u32) -> i32;
type QuitFn = unsafe extern "C" fn(*mut c_void) -> i32;

struct State {
    stream: AtomicPtr<c_void>,
    loop_: AtomicPtr<c_void>,
    dequeue: DequeueFn,
    queue: QueueFn,
    update_params: UpdateFn,
    quit: QuitFn,
    want: AtomicBool,
    alive: AtomicBool,
    dropped: AtomicU64,
    grabs: AtomicU64,
    grab_ns: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
    format: AtomicU32,
    latest: Mutex<Option<PackedFrame>>,
    cv: Condvar,
    buffers_pod: Vec<u8>,
}

/// One running stream. Dropping it stops the PipeWire thread.
pub struct Handle {
    state: Arc<State>,
    join: Option<std::thread::JoinHandle<()>>,
}

pub struct Timing {
    pub grabs: u64,
    pub avg_grab_ms: u64,
    pub frames_dropped: u64,
}

impl Handle {
    pub fn alive(&self) -> bool {
        self.state.alive.load(Ordering::Acquire)
    }

    pub fn timing(&self) -> Timing {
        let grabs = self.state.grabs.load(Ordering::Relaxed);
        let ns = self.state.grab_ns.load(Ordering::Relaxed);
        let avg = ns.checked_div(grabs).unwrap_or(0) / 1_000_000;
        Timing {
            grabs,
            avg_grab_ms: avg,
            frames_dropped: self.state.dropped.load(Ordering::Relaxed),
        }
    }

    /// Block until the stream copies one frame, or `timeout` elapses.
    /// The copy happens on the PipeWire thread only because `want` is set.
    pub fn grab(&self, timeout: Duration) -> Result<PackedFrame, String> {
        if !self.alive() {
            return Err("screencast stream is not running".into());
        }
        let started = std::time::Instant::now();
        {
            let mut slot = self.state.latest.lock().unwrap_or_else(|e| e.into_inner());
            *slot = None;
        }
        self.state.want.store(true, Ordering::Release);
        let mut slot = self.state.latest.lock().unwrap_or_else(|e| e.into_inner());
        let deadline = std::time::Instant::now() + timeout;
        loop {
            if let Some(frame) = slot.take() {
                let ns = started.elapsed().as_nanos() as u64;
                self.state.grabs.fetch_add(1, Ordering::Relaxed);
                self.state.grab_ns.fetch_add(ns, Ordering::Relaxed);
                return Ok(frame);
            }
            if !self.alive() {
                self.state.want.store(false, Ordering::Release);
                return Err("screencast stream ended".into());
            }
            let now = std::time::Instant::now();
            if now >= deadline {
                self.state.want.store(false, Ordering::Release);
                return Err("no screencast frame".into());
            }
            let (guard, status) = self
                .state
                .cv
                .wait_timeout(slot, deadline - now)
                .unwrap_or_else(|e| e.into_inner());
            slot = guard;
            if status.timed_out() && slot.is_none() {
                self.state.want.store(false, Ordering::Release);
                return Err("no screencast frame".into());
            }
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.state.alive.store(false, Ordering::Release);
        self.state.want.store(false, Ordering::Release);
        let loop_ = self.state.loop_.load(Ordering::Acquire);
        if !loop_.is_null() {
            // SAFETY: `loop_` is the pw_main_loop created on the stream thread
            // and left set until this drop. `pw_main_loop_quit` is safe to
            // call from another thread and only wakes that loop.
            unsafe { (self.state.quit)(loop_) };
        }
        self.state.cv.notify_all();
        if let Some(join) = self.join.take() {
            let _ = join.join();
        }
    }
}

/// Closes the duplicated portal fd unless the stream thread takes it.
/// PipeWire closes the fd after `pw_context_connect_fd`, so the thread
/// must disarm this before that call. A failed `spawn` drops the closure
/// and this closes the fd.
struct DupFd(i32);

impl Drop for DupFd {
    fn drop(&mut self) {
        if self.0 >= 0 {
            unsafe { libc::close(self.0) };
        }
    }
}

impl DupFd {
    fn into_raw(mut self) -> i32 {
        let fd = self.0;
        self.0 = -1;
        fd
    }
}

/// Connect an input stream to `node_id` on a duplicated `fd`.
///
/// The duplicate is handed to PipeWire. `fd` itself is not closed here.
pub fn open_stream(fd: std::os::fd::RawFd, node_id: u32) -> Result<Handle, String> {
    let duped = DupFd(unsafe { libc::dup(fd) });
    if duped.0 < 0 {
        return Err("could not duplicate the pipewire fd".into());
    }
    let lib = unsafe { libloading::Library::new("libpipewire-0.3.so.0") }
        .map_err(|_| "libpipewire-0.3.so.0 is not available".to_string())?;
    let symbols = load_symbols(&lib)?;
    let state = Arc::new(State {
        stream: AtomicPtr::new(std::ptr::null_mut()),
        loop_: AtomicPtr::new(std::ptr::null_mut()),
        dequeue: symbols.dequeue,
        queue: symbols.queue,
        update_params: symbols.update_params,
        quit: symbols.quit,
        want: AtomicBool::new(false),
        alive: AtomicBool::new(true),
        dropped: AtomicU64::new(0),
        grabs: AtomicU64::new(0),
        grab_ns: AtomicU64::new(0),
        width: AtomicU32::new(0),
        height: AtomicU32::new(0),
        format: AtomicU32::new(0),
        latest: Mutex::new(None),
        cv: Condvar::new(),
        buffers_pod: pod::buffers_pod(),
    });
    let thread_state = Arc::clone(&state);
    let join = std::thread::Builder::new()
        .name("screencast-pw".into())
        .spawn(move || {
            let fd = duped.into_raw();
            run_loop(lib, symbols, thread_state, fd, node_id);
        })
        .map_err(|e| format!("could not start the pipewire thread: {e}"))?;
    Ok(Handle {
        state,
        join: Some(join),
    })
}

struct Symbols {
    init: unsafe extern "C" fn(*mut i32, *mut *mut *mut c_char),
    main_loop_new: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    main_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    main_loop_run: unsafe extern "C" fn(*mut c_void) -> i32,
    quit: QuitFn,
    main_loop_destroy: unsafe extern "C" fn(*mut c_void),
    context_new: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    context_destroy: unsafe extern "C" fn(*mut c_void),
    connect_fd: unsafe extern "C" fn(*mut c_void, i32, *mut c_void, usize) -> *mut c_void,
    properties_new: unsafe extern "C" fn(*const c_char) -> *mut c_void,
    properties_set: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32,
    stream_new_simple: unsafe extern "C" fn(
        *mut c_void,
        *const c_char,
        *mut c_void,
        *const PwStreamEvents,
        *mut c_void,
    ) -> *mut c_void,
    stream_connect:
        unsafe extern "C" fn(*mut c_void, i32, u32, u32, *const *const SpaPod, u32) -> i32,
    update_params: UpdateFn,
    dequeue: DequeueFn,
    queue: QueueFn,
    stream_destroy: unsafe extern "C" fn(*mut c_void),
}

fn load_symbols(lib: &libloading::Library) -> Result<Symbols, String> {
    unsafe {
        Ok(Symbols {
            init: load(lib, b"pw_init\0")?,
            main_loop_new: load(lib, b"pw_main_loop_new\0")?,
            main_loop_get_loop: load(lib, b"pw_main_loop_get_loop\0")?,
            main_loop_run: load(lib, b"pw_main_loop_run\0")?,
            quit: load(lib, b"pw_main_loop_quit\0")?,
            main_loop_destroy: load(lib, b"pw_main_loop_destroy\0")?,
            context_new: load(lib, b"pw_context_new\0")?,
            context_destroy: load(lib, b"pw_context_destroy\0")?,
            connect_fd: load(lib, b"pw_context_connect_fd\0")?,
            properties_new: load(lib, b"pw_properties_new\0")?,
            properties_set: load(lib, b"pw_properties_set\0")?,
            stream_new_simple: load(lib, b"pw_stream_new_simple\0")?,
            stream_connect: load(lib, b"pw_stream_connect\0")?,
            update_params: load(lib, b"pw_stream_update_params\0")?,
            dequeue: load(lib, b"pw_stream_dequeue_buffer\0")?,
            queue: load(lib, b"pw_stream_queue_buffer\0")?,
            stream_destroy: load(lib, b"pw_stream_destroy\0")?,
        })
    }
}

unsafe fn load<T>(lib: &libloading::Library, name: &[u8]) -> Result<T, String>
where
    T: Copy,
{
    let sym: libloading::Symbol<T> = unsafe { lib.get(name) }
        .map_err(|_| format!("pipewire is missing {}", String::from_utf8_lossy(name)))?;
    Ok(*sym)
}

fn run_loop(lib: libloading::Library, symbols: Symbols, state: Arc<State>, fd: i32, node_id: u32) {
    let _lib = lib;
    let format = pod::enum_format_pod();
    unsafe {
        (symbols.init)(std::ptr::null_mut(), std::ptr::null_mut());
        let main = (symbols.main_loop_new)(std::ptr::null());
        if main.is_null() {
            state.alive.store(false, Ordering::Release);
            state.cv.notify_all();
            return;
        }
        state.loop_.store(main, Ordering::Release);
        let pw_loop = (symbols.main_loop_get_loop)(main);
        let context = (symbols.context_new)(pw_loop, std::ptr::null_mut(), 0);
        let core = if context.is_null() {
            std::ptr::null_mut()
        } else {
            (symbols.connect_fd)(context, fd, std::ptr::null_mut(), 0)
        };
        if core.is_null() {
            state.alive.store(false, Ordering::Release);
            state.cv.notify_all();
            if !context.is_null() {
                (symbols.context_destroy)(context);
            }
            (symbols.main_loop_destroy)(main);
            state.loop_.store(std::ptr::null_mut(), Ordering::Release);
            return;
        }
        let props = (symbols.properties_new)(std::ptr::null());
        set_prop(&symbols, props, "media.type", "Video");
        set_prop(&symbols, props, "media.category", "Capture");
        set_prop(&symbols, props, "media.role", "Screen");
        let node = format!("{node_id}");
        let node_c =
            std::ffi::CString::new(node).unwrap_or_else(|_| std::ffi::CString::new("0").unwrap());
        if !props.is_null() {
            (symbols.properties_set)(props, c"target.object".as_ptr(), node_c.as_ptr());
        }
        let events = events();
        let data = Arc::as_ptr(&state) as *mut c_void;
        let stream = (symbols.stream_new_simple)(
            pw_loop,
            c"scuffed-stat-tracker".as_ptr(),
            props,
            &events,
            data,
        );
        state.stream.store(stream, Ordering::Release);
        if stream.is_null() {
            state.alive.store(false, Ordering::Release);
            state.cv.notify_all();
        } else {
            let pod_ptr = format.as_ptr() as *const SpaPod;
            let params = [pod_ptr];
            let rc = (symbols.stream_connect)(
                stream,
                PW_DIRECTION_INPUT,
                PW_ID_ANY,
                PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS,
                params.as_ptr(),
                1,
            );
            if rc < 0 {
                state.alive.store(false, Ordering::Release);
                state.cv.notify_all();
            } else {
                (symbols.main_loop_run)(main);
            }
            (symbols.stream_destroy)(stream);
        }
        (symbols.context_destroy)(context);
        (symbols.main_loop_destroy)(main);
        state.loop_.store(std::ptr::null_mut(), Ordering::Release);
        state.alive.store(false, Ordering::Release);
        state.cv.notify_all();
    }
}

fn set_prop(symbols: &Symbols, props: *mut c_void, key: &str, value: &str) {
    if props.is_null() {
        return;
    }
    let Ok(key) = std::ffi::CString::new(key) else {
        return;
    };
    let Ok(value) = std::ffi::CString::new(value) else {
        return;
    };
    unsafe { (symbols.properties_set)(props, key.as_ptr(), value.as_ptr()) };
}

fn events() -> PwStreamEvents {
    PwStreamEvents {
        version: 2,
        destroy: None,
        state_changed: Some(on_state),
        control_info: None,
        io_changed: None,
        param_changed: Some(on_param),
        add_buffer: None,
        remove_buffer: None,
        process: Some(on_process),
        drained: None,
        command: None,
        trigger_done: None,
    }
}

extern "C" fn on_state(data: *mut c_void, _old: i32, state: i32, _error: *const c_char) {
    if data.is_null() {
        return;
    }
    let state_ref = unsafe { &*(data as *const State) };
    if state == PW_STREAM_STATE_ERROR {
        state_ref.alive.store(false, Ordering::Release);
        state_ref.cv.notify_all();
    }
}

extern "C" fn on_param(data: *mut c_void, id: u32, param: *const SpaPod) {
    if data.is_null() || id != SPA_PARAM_FORMAT || param.is_null() {
        return;
    }
    let state = unsafe { &*(data as *const State) };
    let header = unsafe { &*param };
    let bytes = unsafe {
        std::slice::from_raw_parts(
            param as *const u8,
            (header.size as usize).saturating_add(8).min(65_536),
        )
    };
    if let Some(info) = pod::parse_format_pod(bytes) {
        state.format.store(info.format, Ordering::Release);
        if info.width > 0 {
            state.width.store(info.width, Ordering::Release);
        }
        if info.height > 0 {
            state.height.store(info.height, Ordering::Release);
        }
    }
    let stream = state.stream.load(Ordering::Acquire);
    if stream.is_null() || state.buffers_pod.is_empty() {
        return;
    }
    let pod_ptr = state.buffers_pod.as_ptr() as *const SpaPod;
    let params = [pod_ptr];
    let rc = unsafe { (state.update_params)(stream, params.as_ptr(), 1) };
    if rc < 0 {
        // Shared-memory buffers were refused. Leave the format in place so a
        // DMA-BUF that still arrives can be mapped in the process callback.
        tracing::debug!("pipewire rejected the shared-memory buffer param");
    }
}

extern "C" fn on_process(data: *mut c_void) {
    if data.is_null() {
        return;
    }
    let state = unsafe { &*(data as *const State) };
    let stream = state.stream.load(Ordering::Acquire);
    if stream.is_null() {
        return;
    }
    let requested = state.want.swap(false, Ordering::AcqRel);
    let copy = framebuf::buffer_action(requested) == framebuf::BufferAction::Copy;
    let mut newest: *mut PwBuffer = std::ptr::null_mut();
    loop {
        let buf = unsafe { (state.dequeue)(stream) };
        if buf.is_null() {
            break;
        }
        if copy {
            if !newest.is_null() {
                unsafe { (state.queue)(stream, newest) };
                state.dropped.fetch_add(1, Ordering::Relaxed);
            }
            newest = buf;
        } else {
            unsafe { (state.queue)(stream, buf) };
            state.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    if !copy {
        return;
    }
    if newest.is_null() {
        state.want.store(true, Ordering::Release);
        return;
    }
    let copied = copy_buffer(state, newest);
    unsafe { (state.queue)(stream, newest) };
    match copied {
        Some(frame) => {
            let mut slot = state.latest.lock().unwrap_or_else(|err| err.into_inner());
            *slot = Some(frame);
            drop(slot);
            state.cv.notify_one();
        }
        None => {
            state.dropped.fetch_add(1, Ordering::Relaxed);
            state.want.store(true, Ordering::Release);
        }
    }
}

fn copy_buffer(state: &State, buf: *mut PwBuffer) -> Option<PackedFrame> {
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
    let need = (stride as usize).saturating_mul(height as usize);
    // One copy, inside pack. The mapping is gone before this returns.
    read_bytes(data, chunk, need, |bytes| {
        framebuf::pack(format, width, height, stride, bytes)
    })
}

fn read_bytes<T>(
    data: &SpaData,
    chunk: &SpaChunk,
    need: usize,
    use_bytes: impl FnOnce(&[u8]) -> Option<T>,
) -> Option<T> {
    let offset = chunk.offset as usize;
    if !data.data.is_null() && matches!(data.type_, SPA_DATA_MEM_PTR | SPA_DATA_MEM_FD | 0) {
        let available = data.maxsize as usize;
        if offset.saturating_add(need) > available && data.maxsize != 0 {
            return None;
        }
        let ptr = unsafe { (data.data as *const u8).add(offset) };
        let bytes = unsafe { std::slice::from_raw_parts(ptr, need) };
        return use_bytes(bytes);
    }
    if data.fd < 0 || !matches!(data.type_, SPA_DATA_MEM_FD | SPA_DATA_DMA_BUF | 0) {
        return None;
    }
    let map_len = (data.maxsize as usize)
        .max(offset.saturating_add(need))
        .max(need);
    if map_len == 0 {
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

/// Counts only. No paths and no frame bytes.
pub fn timing_line(timing: &Timing) -> String {
    format!(
        "screencast timing: grabs={} avg_grab_ms={} frames_dropped={}",
        timing.grabs, timing.avg_grab_ms, timing.frames_dropped
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn timing_line_prints_counts_only() {
        let line = timing_line(&Timing {
            grabs: 4,
            avg_grab_ms: 12,
            frames_dropped: 80,
        });
        assert_eq!(
            line,
            "screencast timing: grabs=4 avg_grab_ms=12 frames_dropped=80"
        );
        assert!(!line.contains('/'));
    }
}
