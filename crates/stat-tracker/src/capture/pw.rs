//! A PipeWire consumer for one portal ScreenCast node.
//!
//! `libpipewire-0.3.so.0` is opened at runtime. The crate does not link
//! PipeWire, so a host without it still builds and falls back to the
//! screenshot portal. The stream asks for at most 2 fps. Buffers are
//! requeued immediately. A frame is copied only while a grab is waiting,
//! and only the newest buffer in that callback is copied.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{AtomicBool, AtomicI64, AtomicPtr, AtomicU32, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use super::framebuf::{self, PackedFrame};
use super::pod;

#[cfg_attr(not(test), allow(dead_code))]
const PW_ID_ANY: u32 = 0xffff_ffff;
const PW_DIRECTION_INPUT: i32 = 0;
const PW_STREAM_FLAG_AUTOCONNECT: u32 = 1;
const PW_STREAM_FLAG_MAP_BUFFERS: u32 = 4;
const PW_STREAM_STATE_ERROR: i32 = -1;
const PW_STREAM_STATE_UNCONNECTED: i32 = 0;
const PW_STREAM_STATE_PAUSED: i32 = 2;
const PW_STREAM_STATE_STREAMING: i32 = 3;
const SPA_META_HEADER: u32 = 1;

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
struct SpaMeta {
    type_: u32,
    size: u32,
    data: *mut c_void,
}

#[repr(C)]
struct SpaMetaHeader {
    seq: u32,
    flags: u32,
    pts: i64,
    dts_offset: i64,
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

struct FpsWindow {
    at: std::time::Instant,
    info_at: std::time::Instant,
    count: u64,
    info_frames: u64,
    info_secs: u64,
}

struct State {
    stream: AtomicPtr<c_void>,
    loop_: AtomicPtr<c_void>,
    loop_lock: Mutex<()>,
    dequeue: DequeueFn,
    queue: QueueFn,
    update_params: UpdateFn,
    quit: QuitFn,
    want: AtomicBool,
    alive: AtomicBool,
    streaming: AtomicBool,
    seen_streaming: AtomicBool,
    fresh_done: AtomicBool,
    dropped: AtomicU64,
    received: AtomicU64,
    grabs: AtomicU64,
    grab_ns: AtomicU64,
    width: AtomicU32,
    height: AtomicU32,
    format: AtomicU32,
    max_fps: u32,
    dequeue_seq: AtomicU64,
    max_pts: AtomicI64,
    last_copy_ms: AtomicU64,
    /// Arrival the current Tab grab still needs, on the stream clock.
    /// [`NO_TAB_NEED`] means a poll grab: the throttle stands.
    need_arrived_ms: AtomicU64,
    started: std::time::Instant,
    latest: Mutex<Option<PackedFrame>>,
    last_good: Mutex<Option<PackedFrame>>,
    notify: tokio::sync::Notify,
    buffers_pod: Vec<u8>,
    rate_logged: AtomicBool,
    fps_window: Mutex<FpsWindow>,
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

/// A finished wait. `TabMiss` means the hard timeout passed with no buffer
/// at or after the overlay delay. The caller decides whether a screenshot
/// is safe.
pub enum GrabOutcome {
    Frame(PackedFrame),
    TabMiss { delay_ms: u64 },
}

/// No Tab threshold. Poll grabs leave this in `need_arrived_ms`.
pub const NO_TAB_NEED: u64 = u64::MAX;

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

    pub fn needs_fresh_frame(&self) -> bool {
        !self.state.fresh_done.load(Ordering::Acquire)
    }

    pub fn open_marker(&self) -> framebuf::OpenMarker {
        framebuf::OpenMarker {
            seq: self.state.dequeue_seq.load(Ordering::Acquire),
            pts: self.state.max_pts.load(Ordering::Acquire),
        }
    }

    /// Wait for a frame without blocking a runtime worker. The PipeWire
    /// thread wakes `notify`. A fresh wait (the first grab after open)
    /// discards anything older than the open marker, and anything black or
    /// empty. A later poll grab returns the last converted frame when no
    /// new buffer arrives.
    ///
    /// `tab_press` is the evdev Tab instant. The grab then takes the first
    /// buffer that arrived at or after press + 300 ms, and gives up at
    /// press + 1000 ms. Poll grabs pass `None` and keep the newest frame.
    pub async fn grab(
        &self,
        timeout: Duration,
        tab_press: Option<std::time::Instant>,
    ) -> Result<GrabOutcome, String> {
        if !self.alive() {
            return Err("screencast stream is not running".into());
        }
        let fresh = self.needs_fresh_frame();
        let marker = self.open_marker();
        if fresh {
            let mut slot = self
                .state
                .latest
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            *slot = None;
        }
        let _need = tab_press.map(|press| {
            let press_ms = rel_ms(self.state.started, press);
            let ready = press_ms.saturating_add(framebuf::TAB_OVERLAY_AFTER_PRESS_MS);
            self.state.need_arrived_ms.store(ready, Ordering::Release);
            NeedReset(&self.state.need_arrived_ms)
        });
        self.state.want.store(true, Ordering::Release);
        let started = std::time::Instant::now();
        let poll_deadline = started + timeout;
        loop {
            if let Some(press) = tab_press {
                let press_ms = rel_ms(self.state.started, press);
                let now_ms = rel_ms(self.state.started, std::time::Instant::now());
                let arrived = held_arrival(&self.state, fresh, marker);
                match framebuf::select_tab_frame(arrived, press_ms, now_ms) {
                    framebuf::TabFrameChoice::Wait => {}
                    framebuf::TabFrameChoice::Take => {
                        let Some(frame) = take_frame(&self.state, fresh, marker) else {
                            continue;
                        };
                        let delay_ms = framebuf::tab_frame_delay_ms(press_ms, frame.arrived_ms);
                        tracing::info!(delay_ms, "screencast tab frame delay");
                        return Ok(self.finish_frame(frame, fresh, started));
                    }
                    framebuf::TabFrameChoice::Miss => {
                        self.state.want.store(false, Ordering::Release);
                        let delay_ms = now_ms.saturating_sub(press_ms);
                        return Ok(GrabOutcome::TabMiss { delay_ms });
                    }
                }
            } else if let Some(frame) = take_frame(&self.state, fresh, marker) {
                return Ok(self.finish_frame(frame, fresh, started));
            }
            if !self.alive() || stream_died(&self.state) {
                self.state.want.store(false, Ordering::Release);
                return Err("screencast stream ended".into());
            }
            let now = std::time::Instant::now();
            let deadline = match tab_press {
                Some(press) => {
                    press + Duration::from_millis(framebuf::TAB_GRAB_TIMEOUT_AFTER_PRESS_MS)
                }
                None => poll_deadline,
            };
            if now >= deadline {
                break;
            }
            let left = deadline - now;
            tokio::select! {
                _ = self.state.notify.notified() => {}
                _ = tokio::time::sleep(left) => {}
            }
        }
        self.state.want.store(false, Ordering::Release);
        if let Some(press) = tab_press {
            let press_ms = rel_ms(self.state.started, press);
            let now_ms = rel_ms(self.state.started, std::time::Instant::now());
            let arrived = held_arrival(&self.state, fresh, marker);
            if framebuf::select_tab_frame(arrived, press_ms, now_ms)
                == framebuf::TabFrameChoice::Take
                && let Some(frame) = take_frame(&self.state, fresh, marker)
            {
                let delay_ms = framebuf::tab_frame_delay_ms(press_ms, frame.arrived_ms);
                tracing::info!(delay_ms, "screencast tab frame delay");
                return Ok(self.finish_frame(frame, fresh, started));
            }
            return Ok(GrabOutcome::TabMiss {
                delay_ms: now_ms.saturating_sub(press_ms),
            });
        }
        if !fresh
            && self.state.streaming.load(Ordering::Acquire)
            && let Some(frame) = self
                .state
                .last_good
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        {
            return Ok(self.finish_frame(frame, fresh, started));
        }
        Err("no screencast frame".into())
    }

    fn finish_frame(
        &self,
        frame: PackedFrame,
        fresh: bool,
        started: std::time::Instant,
    ) -> GrabOutcome {
        remember_good(&self.state, &frame);
        if fresh {
            self.state.fresh_done.store(true, Ordering::Release);
        }
        let ns = started.elapsed().as_nanos() as u64;
        self.state.grabs.fetch_add(1, Ordering::Relaxed);
        self.state.grab_ns.fetch_add(ns, Ordering::Relaxed);
        self.state.want.store(false, Ordering::Release);
        GrabOutcome::Frame(frame)
    }
}

struct NeedReset<'a>(&'a AtomicU64);

impl Drop for NeedReset<'_> {
    fn drop(&mut self) {
        self.0.store(NO_TAB_NEED, Ordering::Release);
    }
}

fn rel_ms(base: std::time::Instant, at: std::time::Instant) -> u64 {
    at.saturating_duration_since(base).as_millis() as u64
}

/// Arrival of the held frame when it is usable. Blank and pre-open frames
/// do not count, so a Tab wait keeps looking.
fn held_arrival(state: &State, fresh: bool, marker: framebuf::OpenMarker) -> Option<u64> {
    let slot = state.latest.lock().unwrap_or_else(|err| err.into_inner());
    let frame = slot.as_ref()?;
    if framebuf::frame_is_blank(frame) {
        return None;
    }
    if fresh && !framebuf::frame_is_fresh(frame.clock, marker) {
        return None;
    }
    Some(frame.arrived_ms)
}

fn take_frame(state: &State, fresh: bool, marker: framebuf::OpenMarker) -> Option<PackedFrame> {
    let mut slot = state.latest.lock().unwrap_or_else(|err| err.into_inner());
    let frame = slot.take()?;
    if framebuf::frame_is_blank(&frame) {
        return None;
    }
    if fresh && !framebuf::frame_is_fresh(frame.clock, marker) {
        return None;
    }
    Some(frame)
}

fn remember_good(state: &State, frame: &PackedFrame) {
    *state
        .last_good
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = Some(frame.clone());
}

fn stream_died(state: &State) -> bool {
    state.seen_streaming.load(Ordering::Acquire) && !state.streaming.load(Ordering::Acquire)
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.state.alive.store(false, Ordering::Release);
        self.state.want.store(false, Ordering::Release);
        let loop_ = take_loop(&self.state);
        if !loop_.is_null() {
            // SAFETY: `loop_` was stored by the stream thread and cleared
            // here before that thread destroys it. `pw_main_loop_quit` only
            // wakes the loop.
            unsafe { (self.state.quit)(loop_) };
        }
        self.state.notify.notify_waiters();
        if let Some(join) = self.join.take() {
            // Bound the join so a stuck PipeWire loop cannot hold process
            // exit. The thread is signaled to quit above.
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let _ = join.join();
                let _ = tx.send(());
            });
            let _ = rx.recv_timeout(Duration::from_secs(2));
        }
    }
}

fn take_loop(state: &State) -> *mut c_void {
    let _guard = state
        .loop_lock
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    state.loop_.swap(std::ptr::null_mut(), Ordering::AcqRel)
}

fn clear_loop_ptr(state: &State, main: *mut c_void) {
    let _guard = state
        .loop_lock
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if state.loop_.load(Ordering::Acquire) == main {
        state.loop_.store(std::ptr::null_mut(), Ordering::Release);
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

/// How the stream is linked. `target_id` is the portal node id passed to
/// `pw_stream_connect`. `target_object` is an object serial when one is
/// known. The node id is never written into `target.object`: WirePlumber
/// matches that against `object.serial`, not the portal's node id.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StreamLink {
    pub target_id: u32,
    pub dont_fallback: bool,
    pub target_object: Option<u32>,
}

pub fn stream_link_plan(node_id: u32, serial: Option<u32>) -> StreamLink {
    StreamLink {
        target_id: node_id,
        dont_fallback: true,
        target_object: serial,
    }
}

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

/// One process-wide PipeWire library. Unloading it while a stream still
/// holds function pointers is a use-after-free, so a successful load stays.
fn pipewire_library() -> Result<&'static libloading::Library, String> {
    static LIB: Mutex<Option<&'static libloading::Library>> = Mutex::new(None);
    let mut slot = LIB.lock().unwrap_or_else(|err| err.into_inner());
    if let Some(lib) = *slot {
        return Ok(lib);
    }
    let loaded = unsafe { libloading::Library::new("libpipewire-0.3.so.0") }
        .map_err(|_| "libpipewire-0.3.so.0 is not available".to_string())?;
    let leaked: &'static libloading::Library = Box::leak(Box::new(loaded));
    *slot = Some(leaked);
    Ok(leaked)
}

/// Connect an input stream to `node_id` on a duplicated `fd`.
///
/// The duplicate is close-on-exec and is handed to the core opened on that
/// fd. `fd` itself is not closed here.
pub fn open_stream(fd: std::os::fd::RawFd, node_id: u32, max_fps: u32) -> Result<Handle, String> {
    let duped = DupFd(unsafe { libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 0) });
    if duped.0 < 0 {
        return Err("could not duplicate the pipewire fd".into());
    }
    let lib = pipewire_library()?;
    let symbols = load_symbols(lib)?;
    let state = Arc::new(State {
        stream: AtomicPtr::new(std::ptr::null_mut()),
        loop_: AtomicPtr::new(std::ptr::null_mut()),
        dequeue: symbols.dequeue,
        queue: symbols.queue,
        update_params: symbols.update_params,
        quit: symbols.quit,
        want: AtomicBool::new(false),
        alive: AtomicBool::new(true),
        streaming: AtomicBool::new(false),
        seen_streaming: AtomicBool::new(false),
        fresh_done: AtomicBool::new(false),
        dropped: AtomicU64::new(0),
        received: AtomicU64::new(0),
        grabs: AtomicU64::new(0),
        grab_ns: AtomicU64::new(0),
        width: AtomicU32::new(0),
        height: AtomicU32::new(0),
        format: AtomicU32::new(0),
        max_fps: max_fps.max(1),
        dequeue_seq: AtomicU64::new(0),
        max_pts: AtomicI64::new(0),
        last_copy_ms: AtomicU64::new(0),
        need_arrived_ms: AtomicU64::new(NO_TAB_NEED),
        started: std::time::Instant::now(),
        latest: Mutex::new(None),
        last_good: Mutex::new(None),
        notify: tokio::sync::Notify::new(),
        buffers_pod: pod::buffers_pod(),
        rate_logged: AtomicBool::new(false),
        fps_window: Mutex::new(FpsWindow {
            at: std::time::Instant::now(),
            info_at: std::time::Instant::now(),
            count: 0,
            info_frames: 0,
            info_secs: 0,
        }),
        loop_lock: Mutex::new(()),
    });
    let thread_state = Arc::clone(&state);
    let join = std::thread::Builder::new()
        .name("screencast-pw".into())
        .spawn(move || {
            let fd = duped.into_raw();
            run_loop(symbols, thread_state, fd, node_id);
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
    properties_new_dict: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    properties_set: unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32,
    stream_new: unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> *mut c_void,
    add_listener:
        unsafe extern "C" fn(*mut c_void, *mut c_void, *const PwStreamEvents, *mut c_void),
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
            properties_new_dict: load(lib, b"pw_properties_new_dict\0")?,
            properties_set: load(lib, b"pw_properties_set\0")?,
            stream_new: load(lib, b"pw_stream_new\0")?,
            add_listener: load(lib, b"pw_stream_add_listener\0")?,
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

fn run_loop(symbols: Symbols, state: Arc<State>, fd: i32, node_id: u32) {
    let format = pod::enum_format_pod(state.max_fps);
    unsafe {
        (symbols.init)(std::ptr::null_mut(), std::ptr::null_mut());
        let main = (symbols.main_loop_new)(std::ptr::null());
        if main.is_null() {
            state.alive.store(false, Ordering::Release);
            state.notify.notify_waiters();
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
            state.notify.notify_waiters();
            if !context.is_null() {
                (symbols.context_destroy)(context);
            }
            clear_loop_ptr(&state, main);
            (symbols.main_loop_destroy)(main);
            return;
        }
        let link = stream_link_plan(node_id, None);
        let props = (symbols.properties_new_dict)(std::ptr::null());
        set_prop(&symbols, props, "media.type", "Video");
        set_prop(&symbols, props, "media.category", "Capture");
        set_prop(&symbols, props, "media.role", "Screen");
        if link.dont_fallback {
            set_prop(&symbols, props, "node.dont-fallback", "true");
        }
        if let Some(serial) = link.target_object {
            set_prop(&symbols, props, "target.object", &serial.to_string());
        }
        let events = events();
        let data = Arc::as_ptr(&state) as *mut c_void;
        let stream = (symbols.stream_new)(core, c"scuffed-stat-tracker".as_ptr(), props);
        state.stream.store(stream, Ordering::Release);
        if stream.is_null() {
            state.alive.store(false, Ordering::Release);
            state.notify.notify_waiters();
        } else {
            // The hook must outlive the loop. It sits on this stack until
            // the stream is destroyed below.
            let mut hook = [0u8; 64];
            (symbols.add_listener)(stream, hook.as_mut_ptr() as *mut c_void, &events, data);
            let pod_ptr = format.as_ptr() as *const SpaPod;
            let params = [pod_ptr];
            let rc = (symbols.stream_connect)(
                stream,
                PW_DIRECTION_INPUT,
                link.target_id,
                PW_STREAM_FLAG_AUTOCONNECT | PW_STREAM_FLAG_MAP_BUFFERS,
                params.as_ptr(),
                1,
            );
            if rc < 0 {
                state.alive.store(false, Ordering::Release);
                state.notify.notify_waiters();
            } else {
                (symbols.main_loop_run)(main);
            }
            (symbols.stream_destroy)(stream);
        }
        clear_loop_ptr(&state, main);
        (symbols.context_destroy)(context);
        (symbols.main_loop_destroy)(main);
        state.alive.store(false, Ordering::Release);
        state.notify.notify_waiters();
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
    if state == PW_STREAM_STATE_STREAMING {
        state_ref.seen_streaming.store(true, Ordering::Release);
        state_ref.streaming.store(true, Ordering::Release);
        tracing::info!("screencast pipewire state streaming");
    } else {
        state_ref.streaming.store(false, Ordering::Release);
        tracing::debug!(state = pw_state_name(state), "screencast pipewire state");
    }
    if stream_state_is_dead(state, state_ref.seen_streaming.load(Ordering::Acquire)) {
        state_ref.alive.store(false, Ordering::Release);
        tracing::info!(
            state = pw_state_name(state),
            "screencast stream ended; the next open waits out the backoff"
        );
    }
    state_ref.notify.notify_waiters();
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
    if !state.rate_logged.swap(true, Ordering::AcqRel) {
        let rates = pod::fractions(bytes);
        tracing::info!("{}", pod::negotiated_rate_line(&rates, state.max_fps));
    }
    let stream = state.stream.load(Ordering::Acquire);
    if stream.is_null() || state.buffers_pod.is_empty() {
        return;
    }
    let pod_ptr = state.buffers_pod.as_ptr() as *const SpaPod;
    let params = [pod_ptr];
    let rc = unsafe { (state.update_params)(stream, params.as_ptr(), 1) };
    if rc < 0 {
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
    let requested = state.want.load(Ordering::Acquire);
    let now_ms = state.started.elapsed().as_millis() as u64;
    let gap = throttle_gap_ms(state.max_fps);
    let latest_arrived = state
        .latest
        .lock()
        .unwrap_or_else(|err| err.into_inner())
        .as_ref()
        .map(|frame| frame.arrived_ms)
        .unwrap_or(0);
    let copy = requested
        && framebuf::buffer_action(true) == framebuf::BufferAction::Copy
        && copy_for_wait(
            state.last_copy_ms.load(Ordering::Acquire),
            now_ms,
            gap,
            latest_arrived,
            state.need_arrived_ms.load(Ordering::Acquire),
        );
    let mut newest: *mut PwBuffer = std::ptr::null_mut();
    let mut newest_seq = 0u64;
    let mut newest_pts = 0i64;
    loop {
        let buf = unsafe { (state.dequeue)(stream) };
        if buf.is_null() {
            break;
        }
        let seq = state.dequeue_seq.fetch_add(1, Ordering::AcqRel) + 1;
        state.received.fetch_add(1, Ordering::Relaxed);
        let pts = header_pts(buf);
        if let Some(pts) = pts.filter(|pts| *pts > 0) {
            update_max_pts(&state.max_pts, pts);
        }
        note_fps(state);
        if copy {
            if !newest.is_null() {
                unsafe { (state.queue)(stream, newest) };
                state.dropped.fetch_add(1, Ordering::Relaxed);
            }
            newest = buf;
            newest_seq = seq;
            newest_pts = pts.unwrap_or(0);
        } else {
            unsafe { (state.queue)(stream, buf) };
            state.dropped.fetch_add(1, Ordering::Relaxed);
        }
    }
    if !requested {
        return;
    }
    if !copy || newest.is_null() {
        if newest.is_null() {
            state.want.store(true, Ordering::Release);
        }
        state.notify.notify_waiters();
        return;
    }
    // Clear want only after we accept a frame, so a black buffer does not
    // end the grab. A second callback inside the throttle gap is dropped.
    state.want.store(false, Ordering::Release);
    let copied = copy_buffer(state, newest, newest_seq, newest_pts, now_ms);
    unsafe { (state.queue)(stream, newest) };
    match copied {
        Some(frame) if !framebuf::frame_is_blank(&frame) => {
            state.last_copy_ms.store(now_ms, Ordering::Release);
            let mut slot = state.latest.lock().unwrap_or_else(|err| err.into_inner());
            *slot = Some(frame);
            drop(slot);
            state.notify.notify_waiters();
        }
        _ => {
            state.dropped.fetch_add(1, Ordering::Relaxed);
            state.want.store(true, Ordering::Release);
            state.notify.notify_waiters();
        }
    }
}

fn header_pts(buf: *mut PwBuffer) -> Option<i64> {
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
        if meta.type_ == SPA_META_HEADER
            && !meta.data.is_null()
            && meta.size as usize >= std::mem::size_of::<SpaMetaHeader>()
        {
            let header = unsafe { &*(meta.data as *const SpaMetaHeader) };
            return Some(header.pts);
        }
    }
    None
}

fn update_max_pts(slot: &AtomicI64, pts: i64) {
    let mut cur = slot.load(Ordering::Acquire);
    while pts > cur {
        match slot.compare_exchange_weak(cur, pts, Ordering::AcqRel, Ordering::Acquire) {
            Ok(_) => break,
            Err(seen) => cur = seen,
        }
    }
}

fn note_fps(state: &State) {
    let mut window = state
        .fps_window
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    window.count = window.count.saturating_add(1);
    window.info_frames = window.info_frames.saturating_add(1);
    if window.at.elapsed() >= Duration::from_secs(1) {
        let secs = window.at.elapsed().as_secs().max(1);
        let line = fps_line(window.count, secs, state.max_fps);
        window.info_secs = window.info_secs.saturating_add(secs);
        window.count = 0;
        window.at = std::time::Instant::now();
        let info = window.info_at.elapsed() >= Duration::from_secs(180);
        if info {
            window.info_at = std::time::Instant::now();
        }
        let info_line = fps_line(window.info_frames, window.info_secs.max(1), state.max_fps);
        if info {
            window.info_frames = 0;
            window.info_secs = 0;
        }
        drop(window);
        tracing::debug!("{line}");
        if info {
            tracing::info!("{info_line}");
        }
    }
}

fn copy_buffer(
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

/// Milliseconds between copies at `max_fps`. The compositor is also asked
/// to stay at this rate. One grab converts at most one frame.
pub fn throttle_gap_ms(max_fps: u32) -> u64 {
    1000 / u64::from(max_fps.max(1))
}

/// True when a grab may copy. `last_ms == 0` is the first frame after open.
pub fn copy_due(last_ms: u64, now_ms: u64, gap_ms: u64) -> bool {
    last_ms == 0 || now_ms.saturating_sub(last_ms) >= gap_ms
}

/// Poll grabs stay on the throttle. A Tab grab that still needs a buffer
/// after the overlay delay copies the next one even inside that gap, so a
/// single frame during a 2 second hole is not dropped.
pub fn copy_for_wait(
    last_ms: u64,
    now_ms: u64,
    gap_ms: u64,
    latest_arrived_ms: u64,
    need_arrived_ms: u64,
) -> bool {
    if need_arrived_ms != NO_TAB_NEED && latest_arrived_ms < need_arrived_ms {
        return true;
    }
    copy_due(last_ms, now_ms, gap_ms)
}

/// ERROR is always dead. UNCONNECTED and PAUSED count only after the stream
/// has reached STREAMING, so negotiation's pause is not a user stop.
pub fn stream_state_is_dead(state: i32, seen_streaming: bool) -> bool {
    match state {
        PW_STREAM_STATE_ERROR => true,
        PW_STREAM_STATE_UNCONNECTED | PW_STREAM_STATE_PAUSED => seen_streaming,
        _ => false,
    }
}

pub fn pw_state_name(state: i32) -> &'static str {
    match state {
        PW_STREAM_STATE_ERROR => "error",
        PW_STREAM_STATE_UNCONNECTED => "unconnected",
        1 => "connecting",
        PW_STREAM_STATE_PAUSED => "paused",
        PW_STREAM_STATE_STREAMING => "streaming",
        _ => "other",
    }
}

pub fn fps_line(frames: u64, secs: u64, configured: u32) -> String {
    let per = frames / secs.max(1);
    format!("screencast frames received per second: {per} (configured {configured} fps)")
}

/// What a grab returns. A fresh wait never reuses the previous frame.
#[cfg_attr(not(test), allow(dead_code))]
pub fn grab_choice(new_frame: bool, has_last: bool, fresh_wait: bool) -> &'static str {
    if new_frame {
        "new"
    } else if !fresh_wait && has_last {
        "reuse"
    } else {
        "timeout"
    }
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
    fn the_stream_targets_the_portal_node_and_does_not_fall_back() {
        let plan = stream_link_plan(42, None);
        assert_eq!(plan.target_id, 42);
        assert_ne!(plan.target_id, PW_ID_ANY);
        assert!(plan.dont_fallback);
        assert!(plan.target_object.is_none());
        let with_serial = stream_link_plan(42, Some(1001));
        assert_eq!(with_serial.target_object, Some(1001));
        assert_eq!(with_serial.target_id, 42);
    }

    #[test]
    fn a_red_pixel_uses_the_chunk_offset_and_padded_stride() {
        // BGRx red sits at byte 2. The chunk starts 8 bytes into a 64-byte
        // mapping, and the row is padded to stride 16.
        let mut raw = [0u8; 64];
        raw[10] = 255;
        let (offset, need) = plane_window(8, 64, 16, 16, 1).unwrap();
        assert_eq!((offset, need), (8, 16));
        let frame = framebuf::pack(
            framebuf::FMT_BGRX,
            1,
            1,
            16,
            &raw[offset..offset + need],
            framebuf::FrameClock { seq: 1, pts: 1 },
        )
        .unwrap();
        assert_eq!(
            framebuf::to_rgb(&frame).unwrap().get_pixel(0, 0).0,
            [255, 0, 0]
        );
        let mut rgbx = [0u8; 32];
        rgbx[4] = 255;
        let (offset, need) = plane_window(4, 32, 16, 16, 1).unwrap();
        let frame = framebuf::pack(
            framebuf::FMT_RGBX,
            1,
            1,
            16,
            &rgbx[offset..offset + need],
            framebuf::FrameClock { seq: 2, pts: 1 },
        )
        .unwrap();
        assert_eq!(
            framebuf::to_rgb(&frame).unwrap().get_pixel(0, 0).0,
            [255, 0, 0]
        );
        assert!(plane_window(0, 0, 16, 16, 1).is_none());
        assert!(plane_window(0, 8, 4, 16, 1).is_none());
        assert!(plane_window(8, 16, 16, 16, 1).is_none());
    }

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

    #[test]
    fn unconnected_after_streaming_ends_the_session() {
        assert!(!stream_state_is_dead(PW_STREAM_STATE_UNCONNECTED, false));
        assert!(!stream_state_is_dead(PW_STREAM_STATE_PAUSED, false));
        assert!(stream_state_is_dead(PW_STREAM_STATE_ERROR, false));
        assert!(stream_state_is_dead(PW_STREAM_STATE_UNCONNECTED, true));
        assert!(stream_state_is_dead(PW_STREAM_STATE_PAUSED, true));
        assert!(!stream_state_is_dead(PW_STREAM_STATE_STREAMING, true));
        assert_eq!(pw_state_name(PW_STREAM_STATE_STREAMING), "streaming");
    }

    #[test]
    fn a_still_screen_reuses_the_last_frame_and_a_fresh_wait_does_not() {
        assert_eq!(grab_choice(false, true, false), "reuse");
        assert_eq!(grab_choice(true, true, false), "new");
        assert_eq!(grab_choice(false, true, true), "timeout");
        assert_eq!(grab_choice(false, false, false), "timeout");
    }

    #[test]
    fn throttle_matches_the_configured_cap() {
        assert_eq!(throttle_gap_ms(2), 500);
        assert_eq!(throttle_gap_ms(4), 250);
        assert!(copy_due(0, 10, 500));
        assert!(!copy_due(1000, 1400, 500));
        assert!(copy_due(1000, 1500, 500));
        assert!(copy_for_wait(1000, 1200, 500, 1000, 1500));
        assert!(!copy_for_wait(1000, 1200, 500, 1500, 1500));
        assert!(!copy_for_wait(1000, 1200, 500, 0, NO_TAB_NEED));
        assert!(copy_for_wait(0, 10, 500, 0, NO_TAB_NEED));
        let line = fps_line(8, 4, 4);
        assert_eq!(
            line,
            "screencast frames received per second: 2 (configured 4 fps)"
        );
    }
}
