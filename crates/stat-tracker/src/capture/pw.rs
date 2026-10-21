//! A PipeWire consumer for one portal ScreenCast node.
//!
//! `libpipewire-0.3.so.0` is opened at runtime. The crate does not link
//! PipeWire, so a host without it still builds and falls back to the
//! screenshot portal. The stream asks for at most `screencast_max_fps`
//! frames per second (default 4). Buffers are requeued immediately. A frame
//! is copied only while a grab is waiting, and only the newest buffer in
//! that callback is copied.

use std::ffi::{c_char, c_void};
use std::sync::atomic::{
    AtomicBool, AtomicI32, AtomicI64, AtomicPtr, AtomicU32, AtomicU64, Ordering,
};
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
const PW_STREAM_STATE_CONNECTING: i32 = 1;
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

/// `spa_meta_header` from libspa. `pts` is at byte 8. A header that ends
/// before `seq` still has a presentation time.
#[repr(C)]
struct SpaMetaHeader {
    flags: u32,
    offset: u32,
    pts: i64,
    dts_offset: i64,
    seq: u64,
}

#[repr(C)]
struct SpaPod {
    size: u32,
    type_: u32,
}

/// `spa_dict` from libspa. `pw_properties_new_dict` reads `n_items` at
/// offset 4, so a null pointer segfaults. `n_items == 0` is an empty dict
/// and the items pointer is not followed.
#[repr(C)]
struct SpaDict {
    flags: u32,
    n_items: u32,
    items: *const c_void,
}

fn empty_spa_dict() -> SpaDict {
    SpaDict {
        flags: 0,
        n_items: 0,
        items: std::ptr::null(),
    }
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
    /// Set by Drop before `quit`. The thread checks this under `loop_lock`
    /// after `pw_loop_enter` and before it publishes the loop, so a quit
    /// that landed early never calls `pw_loop_iterate`.
    quit_requested: AtomicBool,
    /// Wall clock of the last buffer dequeued in `on_process`. This is the
    /// PipeWire delivery time, not the time a grab copied the buffer.
    last_buffer: Mutex<Option<std::time::Instant>>,
    /// Last `pw_stream` state. 0 until the first state callback.
    pw_state: AtomicI32,
    /// When this stream entered Paused after it had reached Streaming.
    /// Cleared on any other state. A short pause is not a dead stream.
    paused_since: Mutex<Option<std::time::Instant>>,
    /// The fd or core dropped the connection. Not set when Drop asked the
    /// thread to quit.
    core_fault: AtomicBool,
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
    /// Stable for this stream object. The next open gets a new id, because
    /// `dequeue_seq` starts at 0 again.
    id: u64,
    dequeue_seq: AtomicU64,
    max_pts: AtomicI64,
    last_copy_ms: AtomicU64,
    /// Arrival the current Tab grab still needs, on the stream clock.
    /// [`NO_TAB_NEED`] means a poll grab: the throttle stands.
    need_arrived_ms: AtomicU64,
    started: std::time::Instant,
    latest: Mutex<Option<Arc<PackedFrame>>>,
    last_good: Mutex<Option<Arc<PackedFrame>>>,
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
#[derive(Debug)]
pub enum GrabOutcome {
    /// `is_new` is false when this is the previous grab's frame. A poll
    /// must not treat that as another read.
    Frame {
        frame: Arc<PackedFrame>,
        is_new: bool,
    },
    TabMiss {
        delay_ms: u64,
    },
}

/// No Tab threshold. Poll grabs leave this in `need_arrived_ms`.
pub const NO_TAB_NEED: u64 = u64::MAX;

/// The PipeWire stream state the capture loop can act on. Numbers stay in
/// this module.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReportedPhase {
    Streaming,
    Error,
    Unconnected,
    Paused,
    Connecting,
    Other,
}

#[derive(Debug, Clone, Copy)]
pub struct StreamReport {
    pub phase: ReportedPhase,
    pub paused_for: Option<Duration>,
    pub core_fault: bool,
    pub has_streamed: bool,
}

fn reported_phase(state: i32) -> ReportedPhase {
    match state {
        PW_STREAM_STATE_STREAMING => ReportedPhase::Streaming,
        PW_STREAM_STATE_ERROR => ReportedPhase::Error,
        PW_STREAM_STATE_UNCONNECTED => ReportedPhase::Unconnected,
        PW_STREAM_STATE_PAUSED => ReportedPhase::Paused,
        PW_STREAM_STATE_CONNECTING => ReportedPhase::Connecting,
        _ => ReportedPhase::Other,
    }
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

    pub fn needs_fresh_frame(&self) -> bool {
        !self.state.fresh_done.load(Ordering::Acquire)
    }

    /// True once PipeWire has reported STREAMING. A pause before that is
    /// negotiation, not a user stop.
    pub fn has_streamed(&self) -> bool {
        self.state.seen_streaming.load(Ordering::Acquire)
    }

    /// How long this stream object has existed. A session that never
    /// reaches STREAMING is closed after a bound on this clock.
    pub fn running_for(&self) -> Duration {
        self.state.started.elapsed()
    }

    /// When PipeWire last dequeued a buffer on this stream. `None` until
    /// the first one. A grab that reused an older frame does not move it.
    pub fn last_buffer_at(&self) -> Option<std::time::Instant> {
        *self
            .state
            .last_buffer
            .lock()
            .unwrap_or_else(|err| err.into_inner())
    }

    pub fn stream_report(&self) -> StreamReport {
        let paused_for = self
            .state
            .paused_since
            .lock()
            .unwrap_or_else(|err| err.into_inner())
            .map(|at| at.elapsed());
        StreamReport {
            phase: reported_phase(self.state.pw_state.load(Ordering::Acquire)),
            paused_for,
            core_fault: self.state.core_fault.load(Ordering::Acquire),
            has_streamed: self.has_streamed(),
        }
    }

    pub fn stream_id(&self) -> u64 {
        self.state.id
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
    /// `tab_press` is the instant the loop read the Tab key. The grab then
    /// takes the first buffer that arrived at or after press + 300 ms, and
    /// gives up at press + 1200 ms. Poll grabs pass `None`. A poll that
    /// sees no new buffer returns the previous frame with `is_new` false.
    /// A wait that ends with nothing returns an error, which is no read.
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
            // Arm the waiter before reading the slot. A frame that arrives
            // between the check and the wait is then still observed.
            let notified = self.state.notify.notified();
            tokio::pin!(notified);
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
                        return Ok(self.finish_frame(frame, fresh, started, true));
                    }
                    framebuf::TabFrameChoice::Miss => {
                        self.state.want.store(false, Ordering::Release);
                        let delay_ms = now_ms.saturating_sub(press_ms);
                        return Ok(GrabOutcome::TabMiss { delay_ms });
                    }
                }
            } else if let Some(frame) = take_frame(&self.state, fresh, marker) {
                return Ok(self.finish_frame(frame, fresh, started, true));
            }
            // A pause is not fatal: niri and KWin leave a stream Streaming
            // with no buffers, and a short Paused is negotiation. Error,
            // unconnected, and a dropped fd clear `alive` or `core_fault`.
            if !self.alive()
                || self.state.core_fault.load(Ordering::Acquire)
                || crate::replace::shutdown_requested()
            {
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
                _ = &mut notified => {}
                _ = tokio::time::sleep(left) => {}
                _ = crate::replace::shutdown_notified() => {
                    self.state.want.store(false, Ordering::Release);
                    return Err("shutting down".into());
                }
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
                return Ok(self.finish_frame(frame, fresh, started, true));
            }
            return Ok(GrabOutcome::TabMiss {
                delay_ms: now_ms.saturating_sub(press_ms),
            });
        }
        // A frame that landed as the deadline passed is newer than the
        // reused one. Take it first.
        if let Some(frame) = take_frame(&self.state, fresh, marker) {
            return Ok(self.finish_frame(frame, fresh, started, true));
        }
        // Clone into a local before `finish_frame`. An `if let` on the
        // lock guard would keep that guard alive through the then-block
        // (edition 2024), and `remember_good` locks `last_good` again.
        // Other capture locks are statement-scoped, or they live inside a
        // helper that returns an owned value before the caller locks again.
        // `take_frame` above is that helper: it locks `latest`, not this mutex.
        let reused = if !fresh && self.state.streaming.load(Ordering::Acquire) {
            self.state
                .last_good
                .lock()
                .unwrap_or_else(|err| err.into_inner())
                .clone()
        } else {
            None
        };
        if let Some(frame) = reused {
            return Ok(self.finish_frame(frame, fresh, started, false));
        }
        Err(NO_STREAM_FRAME.into())
    }

    fn finish_frame(
        &self,
        frame: Arc<PackedFrame>,
        fresh: bool,
        started: std::time::Instant,
        is_new: bool,
    ) -> GrabOutcome {
        remember_good(&self.state, &frame);
        if fresh {
            self.state.fresh_done.store(true, Ordering::Release);
        }
        let ns = started.elapsed().as_nanos() as u64;
        self.state.grabs.fetch_add(1, Ordering::Relaxed);
        self.state.grab_ns.fetch_add(ns, Ordering::Relaxed);
        self.state.want.store(false, Ordering::Release);
        GrabOutcome::Frame { frame, is_new }
    }
}

/// A poll wait that ended with no buffer. Not a frame, and not a confirm.
pub const NO_STREAM_FRAME: &str = "no screencast frame";

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
    if fresh && !framebuf::frame_is_fresh(frame.clock, marker) {
        return None;
    }
    Some(frame.arrived_ms)
}

fn take_frame(
    state: &State,
    fresh: bool,
    marker: framebuf::OpenMarker,
) -> Option<Arc<PackedFrame>> {
    let mut slot = state.latest.lock().unwrap_or_else(|err| err.into_inner());
    let frame = slot.take()?;
    if fresh && !framebuf::frame_is_fresh(frame.clock, marker) {
        return None;
    }
    Some(frame)
}

fn remember_good(state: &State, frame: &Arc<PackedFrame>) {
    *state
        .last_good
        .lock()
        .unwrap_or_else(|err| err.into_inner()) = Some(Arc::clone(frame));
}

fn record_dequeued_buffer(slot: &Mutex<Option<std::time::Instant>>, now: std::time::Instant) {
    *slot.lock().unwrap_or_else(|err| err.into_inner()) = Some(now);
}

/// How long Drop waits for the stream thread. Quit is repeated on this
/// interval so a `pw_main_loop_quit` that landed before `pw_main_loop_run`
/// (PipeWire 1.0 and 1.2) is sent again once the loop is inside run.
const QUIT_JOIN: Duration = Duration::from_secs(2);
const QUIT_RETRY: Duration = Duration::from_millis(50);

impl Drop for Handle {
    fn drop(&mut self) {
        self.state.alive.store(false, Ordering::Release);
        self.state.want.store(false, Ordering::Release);
        // Quit while the thread still owns the loop. Do not drop the
        // pointer here: the thread nulls it and destroys the loop under
        // the same lock, after the iterate loop has returned.
        request_quit(&self.state);
        self.state.notify.notify_waiters();
        if let Some(join) = self.join.take() {
            // Bound the join so a stuck PipeWire loop cannot hold process
            // exit. Quit is retried until the thread leaves or the cap
            // passes, because PipeWire 1.0 and 1.2 drop a quit that arrives
            // before `pw_main_loop_run`.
            let (tx, rx) = std::sync::mpsc::sync_channel(1);
            std::thread::spawn(move || {
                let _ = join.join();
                let _ = tx.send(());
            });
            let deadline = std::time::Instant::now() + QUIT_JOIN;
            loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                if left.is_zero() {
                    break;
                }
                match rx.recv_timeout(left.min(QUIT_RETRY)) {
                    Ok(()) | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(std::sync::mpsc::RecvTimeoutError::Timeout) => request_quit(&self.state),
                }
            }
        }
    }
}

fn request_quit(state: &State) {
    let _guard = state
        .loop_lock
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    state.quit_requested.store(true, Ordering::Release);
    let loop_ = state.loop_.load(Ordering::Acquire);
    if !loop_.is_null() {
        // SAFETY: the pointer is still published. The thread nulls it and
        // destroys the loop under this same lock, so quit cannot run after
        // the loop is freed. `pw_main_loop_quit` only wakes the loop.
        unsafe { (state.quit)(loop_) };
    }
}

/// Publish `main` unless Drop already asked the thread to stop.
/// The check and the store share `loop_lock`, so a quit that arrived
/// before the pointer was visible cannot be missed.
fn begin_loop(state: &State, main: *mut c_void) -> bool {
    let _guard = state
        .loop_lock
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if state.quit_requested.load(Ordering::Acquire) {
        return false;
    }
    state.loop_.store(main, Ordering::Release);
    true
}

/// Null the published pointer and destroy `main` while holding `loop_lock`.
fn destroy_loop(state: &State, destroy: unsafe extern "C" fn(*mut c_void), main: *mut c_void) {
    if main.is_null() {
        return;
    }
    let _guard = state
        .loop_lock
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if state.loop_.load(Ordering::Acquire) == main {
        state.loop_.store(std::ptr::null_mut(), Ordering::Release);
    }
    // SAFETY: `main` was created on this thread and is destroyed once, under
    // the lock that `request_quit` needs before it may call quit.
    unsafe { destroy(main) };
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

fn next_stream_id() -> u64 {
    static NEXT: AtomicU64 = AtomicU64::new(1);
    NEXT.fetch_add(1, Ordering::Relaxed)
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
        id: next_stream_id(),
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
        quit_requested: AtomicBool::new(false),
        last_buffer: Mutex::new(None),
        pw_state: AtomicI32::new(PW_STREAM_STATE_UNCONNECTED),
        paused_since: Mutex::new(None),
        core_fault: AtomicBool::new(false),
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
    drive: LoopDrive,
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

type LoopEnterFn = unsafe extern "C" fn(*mut c_void);
type LoopIterateFn = unsafe extern "C" fn(*mut c_void, i32) -> i32;
type LoopLeaveFn = unsafe extern "C" fn(*mut c_void);
type MainLoopRunFn = unsafe extern "C" fn(*mut c_void) -> i32;

/// How the thread runs the PipeWire loop. `pw_loop_enter`, `pw_loop_iterate`,
/// and `pw_loop_leave` are exported from 1.4. On 1.0 and 1.2 they are macros,
/// so the thread uses `pw_main_loop_run` instead.
enum LoopDrive {
    Iterate {
        enter: LoopEnterFn,
        iterate: LoopIterateFn,
        leave: LoopLeaveFn,
    },
    MainLoopRun(MainLoopRunFn),
}

/// Which loop path the loaded symbols can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum LoopPath {
    Iterate,
    MainLoopRun,
}

fn loop_path(
    has_enter: bool,
    has_iterate: bool,
    has_leave: bool,
    has_run: bool,
) -> Result<LoopPath, &'static str> {
    if has_enter && has_iterate && has_leave {
        Ok(LoopPath::Iterate)
    } else if has_run {
        Ok(LoopPath::MainLoopRun)
    } else {
        Err("pipewire is missing pw_main_loop_run")
    }
}

fn loop_path_line(path: LoopPath) -> &'static str {
    match path {
        LoopPath::Iterate => "pipewire loop path: pw_loop_iterate",
        LoopPath::MainLoopRun => "pipewire loop path: pw_main_loop_run",
    }
}

fn log_loop_path(path: LoopPath) {
    static LOGGED: AtomicBool = AtomicBool::new(false);
    if LOGGED.swap(true, Ordering::Relaxed) {
        return;
    }
    tracing::info!("{}", loop_path_line(path));
}

fn load_symbols(lib: &libloading::Library) -> Result<Symbols, String> {
    unsafe {
        let enter = load_optional::<LoopEnterFn>(lib, b"pw_loop_enter\0");
        let iterate = load_optional::<LoopIterateFn>(lib, b"pw_loop_iterate\0");
        let leave = load_optional::<LoopLeaveFn>(lib, b"pw_loop_leave\0");
        let run = load_optional::<MainLoopRunFn>(lib, b"pw_main_loop_run\0");
        let path = loop_path(
            enter.is_some(),
            iterate.is_some(),
            leave.is_some(),
            run.is_some(),
        )
        .map_err(str::to_string)?;
        let drive = match (path, enter, iterate, leave, run) {
            (LoopPath::Iterate, Some(enter), Some(iterate), Some(leave), _) => LoopDrive::Iterate {
                enter,
                iterate,
                leave,
            },
            (LoopPath::MainLoopRun, _, _, _, Some(run)) => LoopDrive::MainLoopRun(run),
            _ => return Err("pipewire is missing pw_main_loop_run".into()),
        };
        log_loop_path(path);
        Ok(Symbols {
            init: load(lib, b"pw_init\0")?,
            main_loop_new: load(lib, b"pw_main_loop_new\0")?,
            main_loop_get_loop: load(lib, b"pw_main_loop_get_loop\0")?,
            drive,
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

unsafe fn load_optional<T>(lib: &libloading::Library, name: &[u8]) -> Option<T>
where
    T: Copy,
{
    let sym: libloading::Symbol<T> = unsafe { lib.get(name) }.ok()?;
    Some(*sym)
}

fn run_loop(symbols: Symbols, state: Arc<State>, fd: i32, node_id: u32) {
    let format = pod::enum_format_pod(state.max_fps);
    unsafe {
        // PipeWire 1.6 `pw_init` does not read argc or argv. NULL is the
        // no-argv init.
        (symbols.init)(std::ptr::null_mut(), std::ptr::null_mut());
        // `pw_loop_new` only looks at this dict when the pointer is set.
        let main = (symbols.main_loop_new)(std::ptr::null());
        if main.is_null() {
            state.alive.store(false, Ordering::Release);
            state.notify.notify_waiters();
            return;
        }
        let pw_loop = (symbols.main_loop_get_loop)(main);
        // `pw_context_new` replaces a null properties pointer with an empty set.
        let context = (symbols.context_new)(pw_loop, std::ptr::null_mut(), 0);
        let core = if context.is_null() {
            std::ptr::null_mut()
        } else {
            // `core_new` does the same for a null properties pointer.
            (symbols.connect_fd)(context, fd, std::ptr::null_mut(), 0)
        };
        if core.is_null() {
            state.alive.store(false, Ordering::Release);
            state.notify.notify_waiters();
            if !context.is_null() {
                (symbols.context_destroy)(context);
            }
            destroy_loop(&state, symbols.main_loop_destroy, main);
            return;
        }
        let link = stream_link_plan(node_id, None);
        // A null dict is dereferenced (`dict->n_items`). An empty spa_dict
        // is the same bytes QA ran on PipeWire 1.6.9. `pw_stream_new` takes
        // ownership of the result. `set_prop` skips a null result from an
        // allocation failure, and `pw_stream_new` accepts null props.
        let empty = empty_spa_dict();
        let props = (symbols.properties_new_dict)(&empty as *const SpaDict as *const c_void);
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
            // spa_hook is pointer-aligned. A byte array is only align 1.
            let mut hook = [0u64; 8];
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
                match symbols.drive {
                    LoopDrive::Iterate {
                        enter,
                        iterate,
                        leave,
                    } => {
                        if !pw_loop.is_null() {
                            // Enter before the pointer is published. A quit
                            // that lands in that gap sees `quit_requested`
                            // and iterate is never called, so an early
                            // `pw_main_loop_quit` cannot be missed by a later
                            // iterate. The short timeout covers a quit whose
                            // wake already ran.
                            enter(pw_loop);
                            if begin_loop(&state, main) {
                                while !state.quit_requested.load(Ordering::Acquire) {
                                    iterate(pw_loop, 200);
                                }
                            }
                            leave(pw_loop);
                        }
                    }
                    LoopDrive::MainLoopRun(run) => {
                        // PipeWire 1.0 and 1.2. `pw_main_loop_quit` returns
                        // this call. `begin_loop` still refuses a quit that
                        // landed before the pointer was published.
                        if begin_loop(&state, main) {
                            run(main);
                        }
                    }
                }
            }
            (symbols.stream_destroy)(stream);
        }
        (symbols.context_destroy)(context);
        destroy_loop(&state, symbols.main_loop_destroy, main);
        // Drop sets `quit_requested` before the thread leaves. Any other
        // exit is the fd or core going away.
        if !state.quit_requested.load(Ordering::Acquire) {
            state.core_fault.store(true, Ordering::Release);
        }
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
    state_ref.pw_state.store(state, Ordering::Release);
    if state == PW_STREAM_STATE_STREAMING {
        state_ref.seen_streaming.store(true, Ordering::Release);
        state_ref.streaming.store(true, Ordering::Release);
        *state_ref
            .paused_since
            .lock()
            .unwrap_or_else(|err| err.into_inner()) = None;
        tracing::info!("screencast pipewire state streaming");
    } else {
        state_ref.streaming.store(false, Ordering::Release);
        if state == PW_STREAM_STATE_PAUSED && state_ref.seen_streaming.load(Ordering::Acquire) {
            let mut since = state_ref
                .paused_since
                .lock()
                .unwrap_or_else(|err| err.into_inner());
            if since.is_none() {
                *since = Some(std::time::Instant::now());
            }
        } else {
            *state_ref
                .paused_since
                .lock()
                .unwrap_or_else(|err| err.into_inner()) = None;
        }
        tracing::debug!(state = pw_state_name(state), "screencast pipewire state");
    }
    // Error and unconnected end the stream. A pause is recorded above and
    // only counts once it persists. Connecting is negotiation.
    if stream_state_is_dead(state, state_ref.seen_streaming.load(Ordering::Acquire)) {
        state_ref.alive.store(false, Ordering::Release);
        tracing::info!(
            state = pw_state_name(state),
            "screencast pipewire stream left streaming"
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
        // Every dequeued buffer, including one the throttle drops. This is
        // the PipeWire clock. There is no way to ask a consumer stream for
        // a frame, so a poll does not move this.
        record_dequeued_buffer(&state.last_buffer, std::time::Instant::now());
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
    // A black buffer does not end the grab. An early Tab buffer does not
    // either: the request stays up until a frame at or after the overlay
    // delay is stored, so the stream keeps delivering the next one.
    let copied = copy_buffer(state, newest, newest_seq, newest_pts, now_ms);
    unsafe { (state.queue)(stream, newest) };
    match copied {
        Some(frame) if !framebuf::frame_is_blank(&frame) => {
            let frame = Arc::new(frame);
            let need = state.need_arrived_ms.load(Ordering::Acquire);
            let ends = copy_ends_request(need, frame.arrived_ms);
            if ends {
                state.last_copy_ms.store(now_ms, Ordering::Release);
            }
            let mut slot = state.latest.lock().unwrap_or_else(|err| err.into_inner());
            *slot = Some(frame);
            drop(slot);
            state.want.store(!ends, Ordering::Release);
            state.notify.notify_waiters();
        }
        _ => {
            state.dropped.fetch_add(1, Ordering::Relaxed);
            state.want.store(true, Ordering::Release);
            state.notify.notify_waiters();
        }
    }
}

fn pts_from_header(bytes: &[u8]) -> Option<i64> {
    let pts_at = std::mem::offset_of!(SpaMetaHeader, pts);
    let pts_end = pts_at + std::mem::size_of::<i64>();
    if bytes.len() < pts_end {
        return None;
    }
    let mut pts = [0u8; 8];
    pts.copy_from_slice(&bytes[pts_at..pts_end]);
    Some(i64::from_ne_bytes(pts))
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
    // A Tab buffer from before the overlay delay can never be taken.
    // Requeue it without copying.
    if need_arrived_ms != NO_TAB_NEED && now_ms < need_arrived_ms {
        return false;
    }
    if need_arrived_ms != NO_TAB_NEED && latest_arrived_ms < need_arrived_ms {
        return true;
    }
    copy_due(last_ms, now_ms, gap_ms)
}

/// A stored frame ends the grab's request when this is a poll, or when the
/// buffer arrived at or after the Tab overlay. An earlier Tab frame is kept
/// and the stream is asked for another.
pub fn copy_ends_request(need_arrived_ms: u64, arrived_ms: u64) -> bool {
    need_arrived_ms == NO_TAB_NEED || arrived_ms >= need_arrived_ms
}

/// ERROR is always dead. UNCONNECTED counts only after the stream has
/// reached STREAMING, so the state before the first callback is not a
/// stop. PAUSED is not dead here: a pause has to persist, and that clock
/// lives with `paused_since`. CONNECTING is negotiation.
pub fn stream_state_is_dead(state: i32, seen_streaming: bool) -> bool {
    match state {
        PW_STREAM_STATE_ERROR => true,
        PW_STREAM_STATE_UNCONNECTED => seen_streaming,
        _ => false,
    }
}

pub fn pw_state_name(state: i32) -> &'static str {
    match state {
        PW_STREAM_STATE_ERROR => "error",
        PW_STREAM_STATE_UNCONNECTED => "unconnected",
        PW_STREAM_STATE_CONNECTING => "connecting",
        PW_STREAM_STATE_PAUSED => "paused",
        PW_STREAM_STATE_STREAMING => "streaming",
        _ => "other",
    }
}

pub fn fps_line(frames: u64, secs: u64, configured: u32) -> String {
    let per = frames as f64 / f64::from(u32::try_from(secs.max(1)).unwrap_or(u32::MAX));
    format!("screencast frames received per second: {per:.1} (configured {configured} fps)")
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
    fn a_dequeued_buffer_stamps_the_pipewire_clock() {
        let now = std::time::Instant::now();
        let held = Mutex::new(None);
        record_dequeued_buffer(&held, now);
        assert_eq!(*held.lock().unwrap(), Some(now));
        let later = now + Duration::from_secs(60);
        record_dequeued_buffer(&held, later);
        assert_eq!(*held.lock().unwrap(), Some(later));
    }

    #[test]
    fn unconnected_after_streaming_ends_the_session() {
        assert!(!stream_state_is_dead(PW_STREAM_STATE_UNCONNECTED, false));
        assert!(!stream_state_is_dead(PW_STREAM_STATE_PAUSED, false));
        assert!(stream_state_is_dead(PW_STREAM_STATE_ERROR, false));
        assert!(stream_state_is_dead(PW_STREAM_STATE_UNCONNECTED, true));
        // A pause is not an immediate end. The capture loop reopens only
        // once that pause has persisted.
        assert!(!stream_state_is_dead(PW_STREAM_STATE_PAUSED, true));
        assert!(!stream_state_is_dead(PW_STREAM_STATE_CONNECTING, false));
        assert!(!stream_state_is_dead(PW_STREAM_STATE_CONNECTING, true));
        assert!(!stream_state_is_dead(PW_STREAM_STATE_STREAMING, true));
        assert_eq!(pw_state_name(PW_STREAM_STATE_STREAMING), "streaming");
        assert_eq!(pw_state_name(PW_STREAM_STATE_CONNECTING), "connecting");
    }

    #[test]
    fn throttle_matches_the_configured_cap() {
        assert_eq!(throttle_gap_ms(2), 500);
        assert_eq!(throttle_gap_ms(4), 250);
        assert!(copy_due(0, 10, 500));
        assert!(!copy_due(1000, 1400, 500));
        assert!(copy_due(1000, 1500, 500));
        assert!(!copy_for_wait(1000, 1200, 500, 1000, 1500));
        assert!(copy_for_wait(1000, 1600, 500, 1000, 1500));
        assert!(!copy_for_wait(1000, 1200, 500, 1500, 1500));
        assert!(!copy_for_wait(1000, 1200, 500, 0, NO_TAB_NEED));
        assert!(copy_for_wait(0, 10, 500, 0, NO_TAB_NEED));
        assert!(copy_ends_request(NO_TAB_NEED, 10));
        assert!(!copy_ends_request(300, 100));
        assert!(copy_ends_request(300, 300));
        let line = fps_line(8, 4, 4);
        assert_eq!(
            line,
            "screencast frames received per second: 2.0 (configured 4 fps)"
        );
        // One frame across two seconds. Integer division printed 0.
        assert_eq!(
            fps_line(1, 2, 2),
            "screencast frames received per second: 0.5 (configured 2 fps)"
        );
    }

    #[test]
    fn an_early_frame_then_a_two_fps_frame_is_captured() {
        let press = 1_000u64;
        let need = press + framebuf::TAB_OVERLAY_AFTER_PRESS_MS;
        let early = press + 100;
        let gap = throttle_gap_ms(2);
        assert_eq!(gap, 500);
        // The next buffer at 2 fps lands after the overlay delay, and
        // before the hard stop.
        let fresh = early + gap;
        assert!(fresh >= need);
        assert!(fresh < press + framebuf::TAB_GRAB_TIMEOUT_AFTER_PRESS_MS);

        assert!(
            !copy_for_wait(0, early, gap, 0, need),
            "a buffer before the overlay delay is not copied"
        );
        assert!(!copy_ends_request(need, early));
        assert_eq!(
            framebuf::select_tab_frame(Some(early), press, early),
            framebuf::TabFrameChoice::Wait
        );

        assert!(
            copy_for_wait(early, fresh, gap, early, need),
            "the next 2 fps frame is copied"
        );
        assert!(copy_ends_request(need, fresh));
        assert_eq!(
            framebuf::select_tab_frame(Some(fresh), press, fresh),
            framebuf::TabFrameChoice::Take
        );

        assert_eq!(
            framebuf::select_tab_frame(
                Some(early),
                press,
                press + framebuf::TAB_GRAB_TIMEOUT_AFTER_PRESS_MS
            ),
            framebuf::TabFrameChoice::Miss
        );
    }

    #[test]
    fn empty_spa_dict_is_the_pipewire_zero_dict() {
        let dict = empty_spa_dict();
        assert_eq!(std::mem::offset_of!(SpaDict, n_items), 4);
        assert_eq!(dict.flags, 0);
        assert_eq!(dict.n_items, 0);
        assert!(dict.items.is_null());
        let size = std::mem::size_of::<SpaDict>();
        assert_eq!(size, 8 + std::mem::size_of::<*const c_void>());
        let bytes = unsafe { std::slice::from_raw_parts(&raw const dict as *const u8, size) };
        assert!(bytes.iter().all(|byte| *byte == 0));
    }

    unsafe extern "C" fn noop_dequeue(_: *mut c_void) -> *mut PwBuffer {
        std::ptr::null_mut()
    }

    unsafe extern "C" fn noop_queue(_: *mut c_void, _: *mut PwBuffer) -> i32 {
        0
    }

    unsafe extern "C" fn noop_update(_: *mut c_void, _: *const *const SpaPod, _: u32) -> i32 {
        0
    }

    unsafe extern "C" fn noop_quit(_: *mut c_void) -> i32 {
        0
    }

    fn test_handle(
        latest: Option<PackedFrame>,
        last_good: Option<PackedFrame>,
        fresh_done: bool,
    ) -> Handle {
        test_handle_for(next_stream_id(), 0, latest, last_good, fresh_done)
    }

    fn test_handle_for(
        id: u64,
        dequeue_seq: u64,
        latest: Option<PackedFrame>,
        last_good: Option<PackedFrame>,
        fresh_done: bool,
    ) -> Handle {
        Handle {
            state: Arc::new(State {
                stream: AtomicPtr::new(std::ptr::null_mut()),
                loop_: AtomicPtr::new(std::ptr::null_mut()),
                loop_lock: Mutex::new(()),
                quit_requested: AtomicBool::new(false),
                last_buffer: Mutex::new(None),
                pw_state: AtomicI32::new(PW_STREAM_STATE_STREAMING),
                paused_since: Mutex::new(None),
                core_fault: AtomicBool::new(false),
                dequeue: noop_dequeue,
                queue: noop_queue,
                update_params: noop_update,
                quit: noop_quit,
                want: AtomicBool::new(false),
                alive: AtomicBool::new(true),
                streaming: AtomicBool::new(true),
                seen_streaming: AtomicBool::new(true),
                fresh_done: AtomicBool::new(fresh_done),
                dropped: AtomicU64::new(0),
                received: AtomicU64::new(0),
                grabs: AtomicU64::new(0),
                grab_ns: AtomicU64::new(0),
                width: AtomicU32::new(1),
                height: AtomicU32::new(1),
                format: AtomicU32::new(framebuf::FMT_BGRX),
                max_fps: 2,
                id,
                dequeue_seq: AtomicU64::new(dequeue_seq),
                max_pts: AtomicI64::new(0),
                last_copy_ms: AtomicU64::new(0),
                need_arrived_ms: AtomicU64::new(NO_TAB_NEED),
                started: std::time::Instant::now(),
                latest: Mutex::new(latest.map(Arc::new)),
                last_good: Mutex::new(last_good.map(Arc::new)),
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
            }),
            join: None,
        }
    }

    fn grab_now(handle: Handle) -> Result<GrabOutcome, String> {
        let (tx, rx) = std::sync::mpsc::channel();
        let worker = std::thread::spawn(move || {
            let runtime = tokio::runtime::Builder::new_current_thread()
                .enable_time()
                .build()
                .unwrap();
            let outcome = runtime.block_on(handle.grab(Duration::ZERO, None));
            let _ = tx.send(outcome);
        });
        let outcome = match rx.recv_timeout(Duration::from_secs(2)) {
            Ok(outcome) => outcome,
            Err(_) => panic!("poll grab deadlocked"),
        };
        let _ = worker.join();
        outcome
    }

    fn red_frame(seq: u64) -> PackedFrame {
        framebuf::pack(
            framebuf::FMT_BGRX,
            1,
            1,
            4,
            &[0, 0, 255, 0],
            framebuf::FrameClock { seq, pts: 1 },
        )
        .unwrap()
    }

    #[test]
    fn poll_grab_reuses_last_good_without_relocking() {
        let frame = red_frame(3);
        let handle = test_handle(None, Some(frame), true);
        match grab_now(handle).expect("poll grab") {
            GrabOutcome::Frame { frame, is_new } => {
                assert!(!is_new, "a reused frame is not a new read");
                assert_eq!(frame.clock.seq, 3);
                assert_eq!(
                    framebuf::to_rgb(&frame).unwrap().get_pixel(0, 0).0,
                    [255, 0, 0]
                );
            }
            GrabOutcome::TabMiss { .. } => panic!("poll grab returned a tab miss"),
        }
    }

    #[test]
    fn a_reopened_stream_starts_at_zero_and_its_first_frame_is_new() {
        let n = 40;
        let a = test_handle_for(11, n, Some(red_frame(n)), None, true);
        assert_eq!(a.state.dequeue_seq.load(Ordering::Acquire), n);
        let seen = crate::capture::SeenStream {
            id: a.stream_id(),
            seq: n,
        };
        let b = test_handle_for(22, 0, Some(red_frame(1)), None, true);
        assert_eq!(b.state.dequeue_seq.load(Ordering::Acquire), 0);
        let id_b = b.stream_id();
        match grab_now(b).expect("first frame of the new stream") {
            GrabOutcome::Frame { frame, is_new } => {
                assert!(is_new, "the first frame of a new stream is a new read");
                assert!(frame.clock.seq < n);
                assert!(
                    !crate::capture::poll_uses_frame(
                        true,
                        Some(frame.clock.seq),
                        Some(seen.id),
                        Some(seen)
                    ),
                    "the same stream would still treat a restarted sequence as already seen"
                );
                assert!(crate::capture::poll_uses_frame(
                    is_new,
                    Some(frame.clock.seq),
                    Some(id_b),
                    Some(seen),
                ));
            }
            GrabOutcome::TabMiss { .. } => panic!("the first frame was a tab miss"),
        }
    }

    #[test]
    fn pipewire_before_1_4_uses_main_loop_run() {
        assert_eq!(loop_path(true, true, true, true), Ok(LoopPath::Iterate));
        assert_eq!(
            loop_path(false, false, false, true),
            Ok(LoopPath::MainLoopRun)
        );
        assert_eq!(
            loop_path(true, false, true, true),
            Ok(LoopPath::MainLoopRun)
        );
        assert_eq!(
            loop_path(false, false, false, false),
            Err("pipewire is missing pw_main_loop_run")
        );
        assert_eq!(
            loop_path_line(LoopPath::Iterate),
            "pipewire loop path: pw_loop_iterate"
        );
        assert_eq!(
            loop_path_line(LoopPath::MainLoopRun),
            "pipewire loop path: pw_main_loop_run"
        );
    }

    #[test]
    fn a_new_buffer_is_a_new_frame_and_a_fresh_wait_times_out() {
        let fresh = red_frame(8);
        let handle = test_handle(Some(fresh), Some(red_frame(3)), true);
        match grab_now(handle).expect("new frame") {
            GrabOutcome::Frame { frame, is_new } => {
                assert!(is_new);
                assert_eq!(frame.clock.seq, 8);
            }
            GrabOutcome::TabMiss { .. } => panic!("poll grab returned a tab miss"),
        }
        let handle = test_handle(None, Some(red_frame(3)), false);
        let err = grab_now(handle).expect_err("fresh wait with no buffer");
        assert_eq!(err, NO_STREAM_FRAME);
    }

    #[test]
    fn a_short_header_still_has_pts() {
        assert_eq!(std::mem::offset_of!(SpaMetaHeader, pts), 8);
        assert!(std::mem::size_of::<SpaMetaHeader>() >= 32);
        let mut bytes = [0u8; 24];
        bytes[8..16].copy_from_slice(&42i64.to_ne_bytes());
        assert_eq!(pts_from_header(&bytes), Some(42));
        assert_eq!(pts_from_header(&bytes[..8]), None);
    }
}
