//! A PipeWire consumer for one portal ScreenCast node.
//!
//! `libpipewire-0.3.so.0` is opened at runtime. The crate does not link
//! PipeWire, so a host without it still builds and falls back to the
//! screenshot portal. The stream asks for at most `screencast_max_fps`
//! frames per second (default 4). Buffers are requeued immediately. A frame
//! is copied only while a grab is waiting, and only the newest buffer in
//! that callback is copied.
//!
//! Between grabs the stream can be set inactive (`pw_stream_set_active`),
//! so the compositor stops producing frames nobody copies. That call runs
//! on the PipeWire loop thread through the loop's invoke queue. Health
//! timers that used to run on the wall clock run on [`ActiveClock`]
//! instead, which only advances while the tracker wants the stream running,
//! so a pause the tracker asked for never looks like a stalled stream.

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
type SetActiveFn = unsafe extern "C" fn(*mut c_void, bool) -> i32;
/// `spa_invoke_func_t`.
type SpaInvokeFn =
    unsafe extern "C" fn(*mut c_void, bool, u32, *const c_void, usize, *mut c_void) -> i32;
/// `pw_loop_invoke`, exported from PipeWire 1.4. Before that it is a header
/// macro, the invoke queue has no lock in 0.3 and 1.0, and an invoke before
/// the loop is entered runs on the calling thread. Pausing is only used
/// with this symbol and the `pw_loop_iterate` path.
type PwLoopInvokeFn = unsafe extern "C" fn(
    *mut c_void,
    SpaInvokeFn,
    u32,
    *const c_void,
    usize,
    bool,
    *mut c_void,
) -> i32;

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
    /// The pause the tracker asks for, the active clock, and the
    /// resume fail-safe. Paused time is measured on the active clock.
    pause: Mutex<PauseState>,
    /// What the loop thread last passed to `pw_stream_set_active`. Only the
    /// invoke callback reads or writes it.
    applied_active: AtomicBool,
    /// `pw_loop` of the running main loop. Published and cleared with
    /// `loop_`, under `loop_lock`.
    pw_loop: AtomicPtr<c_void>,
    /// Both set, or neither: see [`pause_symbols`].
    set_active: Option<SetActiveFn>,
    loop_invoke: Option<PwLoopInvokeFn>,
    /// [`PAUSE_TRIPS`] outside tests.
    trips: &'static AtomicU32,
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
    /// The info line for Streaming was written. Later resumes log debug.
    streaming_logged: AtomicBool,
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
    /// Resumes from a tracker pause that got a buffer, and the time from
    /// the resume request to that first buffer.
    pub resumes: u64,
    pub avg_resume_ms: u64,
    pub max_resume_ms: u64,
    /// Resumes that were paused again, or tripped the fail-safe, before
    /// any buffer arrived.
    pub resumes_without_buffer: u64,
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
    /// Active-clock time since the stream entered Paused.
    pub paused_for: Option<Duration>,
    /// Paused because the tracker asked, and nothing else is wrong: the
    /// tracker does not want it running and no active time has passed
    /// since the pause began.
    pub self_paused: bool,
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
        let latency = lock(&self.state.pause).latency;
        Timing {
            grabs,
            avg_grab_ms: avg,
            frames_dropped: self.state.dropped.load(Ordering::Relaxed),
            resumes: latency.count,
            avg_resume_ms: latency.total_ms.checked_div(latency.count).unwrap_or(0),
            max_resume_ms: latency.max_ms,
            resumes_without_buffer: latency.unanswered,
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

    /// How long the tracker has wanted this stream running since it was
    /// opened. Time the tracker kept it paused does not count. With pausing
    /// off this is the age of the stream. A session that never reaches
    /// STREAMING is closed after a bound on this clock.
    pub fn running_for(&self) -> Duration {
        lock(&self.state.pause)
            .clock
            .reading(std::time::Instant::now())
    }

    /// Active time since the last PipeWire buffer, or since open when there
    /// was none. A stream the tracker paused is not quiet while paused.
    pub fn quiet_for(&self) -> Duration {
        lock(&self.state.pause).quiet_for(std::time::Instant::now())
    }

    /// True while the tracker wants the stream running (or pausing is off
    /// for it).
    pub fn wants_active(&self) -> bool {
        lock(&self.state.pause).want
    }

    /// Let the tracker pause this stream between grabs. Returns false when
    /// this libpipewire cannot do it safely (older than 1.4).
    /// `streaming_answers`: the compositor draws casts only on damage
    /// (niri), so Streaming again answers a resume without a buffer.
    pub fn enable_pausing(&self, streaming_answers: bool) -> bool {
        if self.state.set_active.is_none() || self.state.loop_invoke.is_none() {
            return false;
        }
        let mut pause = lock(&self.state.pause);
        pause.managed = true;
        pause.streaming_answers = streaming_answers;
        true
    }

    /// Ask for the stream to run (`true`) or pause (`false`). The call is
    /// queued on the PipeWire loop thread and returns at once. A pause is
    /// ignored until this stream has streamed, while pausing is off for it,
    /// and after the resume fail-safe has tripped.
    pub fn set_active(&self, want: bool) {
        let now = std::time::Instant::now();
        let streamed = self.has_streamed();
        let streaming_now =
            self.state.pw_state.load(Ordering::Acquire) == PW_STREAM_STATE_STREAMING;
        let (changed, fail) = {
            let mut pause = lock(&self.state.pause);
            let fail = if want { None } else { pause.check(now) };
            (pause.request(want, streamed, streaming_now, now), fail)
        };
        if let Some(why) = fail {
            self.trip(why);
            return;
        }
        if changed {
            invoke_apply_active(&self.state);
        }
    }

    /// A grab on this stream ended while it was still held active.
    /// `tab_missed` is a Tab grab that found no frame in its window. A Tab
    /// miss while the resume is unanswered (not Streaming again, no buffer)
    /// trips the fail-safe at once, and an unanswered resume that has run
    /// [`RESUME_BUFFER_BOUND`] of active time counts toward
    /// [`RESUME_FAILS_TO_TRIP`].
    pub fn note_grab_end(&self, tab_missed: bool) {
        let fail = lock(&self.state.pause).grab_end(tab_missed, std::time::Instant::now());
        if let Some(why) = fail {
            self.trip(why);
        }
    }

    /// [`Self::note_grab_end`] for what [`Self::grab`] returned.
    pub fn note_grab_result(&self, grabbed: &Result<GrabOutcome, String>) {
        self.note_grab_end(matches!(grabbed, Ok(GrabOutcome::TabMiss { .. })));
    }

    #[cfg(test)]
    pub(super) fn streaming_answers_for_test(&self) -> bool {
        lock(&self.state.pause).streaming_answers
    }

    #[cfg(test)]
    pub(super) fn trip_for_test(&self) {
        self.trip(TRIP_STUCK);
    }

    /// Call before reading health: a resume the compositor never answered
    /// trips here, so the stream runs as rc.6 before the paused rule can
    /// reopen it with pausing still on (and the trip is counted).
    pub fn check_stuck_resume(&self) {
        let phase = pw_phase(
            self.state.pw_state.load(Ordering::Acquire),
            self.has_streamed(),
        );
        let fail = lock(&self.state.pause).stuck(phase, std::time::Instant::now());
        if let Some(why) = fail {
            self.trip(why);
        }
    }

    /// Pausing is on for this stream and has not tripped.
    pub fn pausing(&self) -> bool {
        lock(&self.state.pause).pausing()
    }

    /// Stop pausing this stream and run it as rc.6 did. Counted once per
    /// stream toward [`PAUSE_TRIPS_LIMIT`], with one warning.
    fn trip(&self, why: &'static str) {
        let first = lock(&self.state.pause).trip(why, std::time::Instant::now());
        if first {
            let trips = self.state.trips.fetch_add(1, Ordering::AcqRel) + 1;
            tracing::warn!(
                reason = why,
                trips,
                "screencast pause between grabs turned off for this stream; it stays running"
            );
        }
        invoke_apply_active(&self.state);
    }

    /// True once the fail-safe has turned pausing off for this stream.
    #[cfg_attr(not(test), allow(dead_code))]
    pub fn pause_tripped(&self) -> bool {
        lock(&self.state.pause).tripped.is_some()
    }

    /// The grab may hand back the previous frame while the stream is
    /// resuming from a pause the tracker asked for. Without pausing this is
    /// the old rule: only while Streaming.
    fn reuse_ok(&self) -> bool {
        if self.state.streaming.load(Ordering::Acquire) {
            return true;
        }
        let pausing = lock(&self.state.pause).managed;
        pausing
            && self.has_streamed()
            && self.state.pw_state.load(Ordering::Acquire) == PW_STREAM_STATE_PAUSED
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
        let now = std::time::Instant::now();
        let (paused_for, self_paused) = {
            let pause = lock(&self.state.pause);
            (pause.paused_for(now), pause.self_paused(now))
        };
        StreamReport {
            phase: reported_phase(self.state.pw_state.load(Ordering::Acquire)),
            paused_for,
            self_paused,
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
        let reused = if !fresh && self.reuse_ok() {
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

/// Active time a resume has to reach Streaming, or bring a buffer. PipeWire
/// itself answers a resume in a few ms (Start to the follower, then the
/// driver); KWin 6.8 then waits at most one frame period (250 ms at 4 fps).
/// It is above the 1200 ms Tab window, so one slow Tab alone does not
/// count.
pub const RESUME_BUFFER_BOUND: Duration = Duration::from_millis(1500);

/// Consecutive unanswered resumes ([`RESUME_BUFFER_BOUND`] each) that turn
/// pausing off for one stream. A resume that reaches Streaming or brings a
/// buffer resets the count.
pub const RESUME_FAILS_TO_TRIP: u32 = 2;

/// Streams whose fail-safe has tripped since this process started. A new
/// stream may pause again while this is below [`PAUSE_TRIPS_LIMIT`].
static PAUSE_TRIPS: AtomicU32 = AtomicU32::new(0);

/// After this many tripped streams the tracker stops pausing for the rest
/// of the process: the first trip lets one new share or reopen try again.
pub const PAUSE_TRIPS_LIMIT: u32 = 2;

pub fn pause_trips() -> u32 {
    PAUSE_TRIPS.load(Ordering::Acquire)
}

pub const TRIP_SLOW_RESUMES: &str =
    "two resumes in a row did not reach Streaming or bring a frame within 1.5 s";
pub const TRIP_TAB_MISS: &str =
    "a Tab missed its frame and the resumed stream had not reached Streaming";
pub const TRIP_STUCK: &str =
    "the stream stayed paused (or failed) 1.5 s after the tracker resumed it";
pub const TRIP_RETIRED: &str = "the stream was closed while a resume was still unanswered";
pub const TRIP_FAILED_PAUSED: &str = "the stream failed while the tracker held it paused";

fn lock<T>(mutex: &Mutex<T>) -> std::sync::MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(|err| err.into_inner())
}

/// Time during which the tracker wanted the stream running. It stops while
/// the tracker keeps the stream paused and starts again on resume, so
/// health timers measured on it ignore the tracker's own pauses. With
/// pausing off it never stops and equals the stream's age.
#[derive(Debug, Clone, Copy)]
struct ActiveClock {
    banked: Duration,
    since: Option<std::time::Instant>,
}

impl ActiveClock {
    fn running(at: std::time::Instant) -> Self {
        Self {
            banked: Duration::ZERO,
            since: Some(at),
        }
    }

    fn reading(&self, now: std::time::Instant) -> Duration {
        self.banked
            + self
                .since
                .map_or(Duration::ZERO, |at| now.saturating_duration_since(at))
    }

    fn start(&mut self, now: std::time::Instant) {
        if self.since.is_none() {
            self.since = Some(now);
        }
    }

    fn stop(&mut self, now: std::time::Instant) {
        if let Some(at) = self.since.take() {
            self.banked += now.saturating_duration_since(at);
        }
    }
}

#[derive(Debug, Clone, Copy, Default)]
struct ResumeLatency {
    count: u64,
    total_ms: u64,
    max_ms: u64,
    unanswered: u64,
}

/// The PipeWire state a fail-safe check needs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum PwPhase {
    Streaming,
    Paused,
    /// Error, or Unconnected after it had streamed.
    Failed,
    Other,
}

fn pw_phase(state: i32, seen_streaming: bool) -> PwPhase {
    match state {
        PW_STREAM_STATE_STREAMING => PwPhase::Streaming,
        PW_STREAM_STATE_PAUSED => PwPhase::Paused,
        _ if stream_state_is_dead(state, seen_streaming) => PwPhase::Failed,
        _ => PwPhase::Other,
    }
}

/// Pause bookkeeping for one stream. Plain data so it can be tested
/// without PipeWire. All `Duration` stamps are active-clock readings.
///
/// A resume is *answered* when a buffer arrives. On niri, which draws a
/// cast only on damage (a still screen sends nothing after a resume),
/// reaching Streaming again answers it too. Everywhere else a resume that
/// reaches Streaming and sends no buffer is a broken resume: KWin 6.1+
/// emits a frame on resume and Mutter queues a redraw.
#[derive(Debug)]
struct PauseState {
    /// The tracker wants the stream running. Always true unless pausing is
    /// on for this stream, it has streamed, and no grab holds it.
    want: bool,
    /// Pausing is allowed on this stream (decided at start, symbols present,
    /// and the process has not run out of fail-safe trips).
    managed: bool,
    /// Why the fail-safe turned pausing off for this stream.
    tripped: Option<&'static str>,
    clock: ActiveClock,
    /// When the stream entered Paused, after it had streamed.
    paused_at: Option<Duration>,
    /// That Paused began while the tracker did not want the stream running,
    /// or answered a pause the tracker had asked for: it started as the
    /// tracker's own pause.
    paused_by_us: bool,
    /// A pause was requested and its Paused has not been seen yet. A
    /// resume that arrives first still leaves that Paused marked ours.
    self_pause_pending: bool,
    /// Reaching Streaming answers a resume (niri only).
    streaming_answers: bool,
    /// The stream failed (Error, or Unconnected after streaming) while the
    /// tracker held it paused, with no resume outstanding.
    failed_while_paused: bool,
    /// When the last PipeWire buffer was dequeued.
    last_buffer_at: Option<Duration>,
    /// Start of the oldest resume not answered yet. It survives a pause, so
    /// short grabs add up, and only an answer clears it.
    unanswered_since: Option<Duration>,
    /// Start of the unanswered resume still to be counted toward
    /// [`RESUME_FAILS_TO_TRIP`]. Counting it ends it; the next resume starts
    /// another.
    episode_from: Option<Duration>,
    /// Wall time of the latest resume request, until its first buffer.
    resume_requested: Option<std::time::Instant>,
    /// Consecutive unanswered resumes.
    resume_fails: u32,
    latency: ResumeLatency,
}

impl PauseState {
    fn new(now: std::time::Instant) -> Self {
        Self {
            want: true,
            managed: false,
            tripped: None,
            clock: ActiveClock::running(now),
            paused_at: None,
            paused_by_us: false,
            self_pause_pending: false,
            streaming_answers: false,
            failed_while_paused: false,
            last_buffer_at: None,
            unanswered_since: None,
            episode_from: None,
            resume_requested: None,
            resume_fails: 0,
            latency: ResumeLatency::default(),
        }
    }

    /// Pausing is on and has not tripped.
    fn pausing(&self) -> bool {
        self.managed && self.tripped.is_none()
    }

    /// Record a run or pause request. `streaming_now` is the stream's
    /// current state: a resume of a stream that never left Streaming (the
    /// pause had not been applied yet) has nothing to answer. Returns true
    /// when the wanted state changed and the loop thread has to apply it.
    fn request(
        &mut self,
        want: bool,
        streamed: bool,
        streaming_now: bool,
        now: std::time::Instant,
    ) -> bool {
        let want = want || !self.pausing() || !streamed;
        if want == self.want {
            return false;
        }
        if want {
            self.clock.start(now);
            self.resume_requested = Some(now);
            if streaming_now {
                // The pause was never applied: no Paused will come.
                self.self_pause_pending = false;
            } else {
                let reading = self.clock.reading(now);
                self.unanswered_since.get_or_insert(reading);
                self.episode_from.get_or_insert(reading);
            }
        } else {
            if self.resume_requested.take().is_some() {
                self.latency.unanswered = self.latency.unanswered.saturating_add(1);
            }
            self.clock.stop(now);
            self.self_pause_pending = true;
        }
        self.want = want;
        true
    }

    /// The resume was answered: Streaming again, or a buffer.
    fn answered(&mut self) {
        self.unanswered_since = None;
        self.episode_from = None;
        self.resume_fails = 0;
    }

    /// Count an unanswered resume that has run [`RESUME_BUFFER_BOUND`] of
    /// active time. Returns the trip reason when the count reaches
    /// [`RESUME_FAILS_TO_TRIP`].
    fn check(&mut self, now: std::time::Instant) -> Option<&'static str> {
        if !self.pausing() {
            return None;
        }
        let from = self.episode_from?;
        if self.clock.reading(now).saturating_sub(from) < RESUME_BUFFER_BOUND {
            return None;
        }
        self.episode_from = None;
        self.resume_fails = self.resume_fails.saturating_add(1);
        (self.resume_fails >= RESUME_FAILS_TO_TRIP).then_some(TRIP_SLOW_RESUMES)
    }

    /// A grab ended while the stream was still held. A Tab miss while the
    /// resume is unanswered trips at once.
    fn grab_end(&mut self, tab_missed: bool, now: std::time::Instant) -> Option<&'static str> {
        if !self.pausing() {
            return None;
        }
        if tab_missed && self.unanswered_since.is_some() {
            return Some(TRIP_TAB_MISS);
        }
        self.check(now)
    }

    /// Checked before every health read. A pause that began as the
    /// tracker's own and has not lifted [`RESUME_BUFFER_BOUND`] after the
    /// resume, or a stream that failed while a resume was unanswered, trips
    /// here, before the 2 s paused rule can call it dead and reopen it with
    /// pausing still on.
    fn stuck(&self, phase: PwPhase, now: std::time::Instant) -> Option<&'static str> {
        if !self.pausing() {
            return None;
        }
        if phase == PwPhase::Failed && self.failed_while_paused {
            return Some(TRIP_FAILED_PAUSED);
        }
        let since = self.unanswered_since?;
        match phase {
            PwPhase::Failed => Some(TRIP_STUCK),
            PwPhase::Paused
                if self.paused_by_us
                    && self.clock.reading(now).saturating_sub(since) >= RESUME_BUFFER_BOUND =>
            {
                Some(TRIP_STUCK)
            }
            _ => None,
        }
    }

    /// The stream is being closed. An unanswered resume counts as a trip.
    fn retire(&self) -> Option<&'static str> {
        if !self.pausing() {
            return None;
        }
        if self.failed_while_paused {
            return Some(TRIP_FAILED_PAUSED);
        }
        self.unanswered_since.is_some().then_some(TRIP_RETIRED)
    }

    /// Turn pausing off for this stream. Returns true the first time.
    fn trip(&mut self, why: &'static str, now: std::time::Instant) -> bool {
        if self.tripped.is_some() {
            return false;
        }
        self.tripped = Some(why);
        self.unanswered_since = None;
        self.episode_from = None;
        if !self.want {
            if self.resume_requested.take().is_some() {
                self.latency.unanswered = self.latency.unanswered.saturating_add(1);
            }
            self.clock.start(now);
            self.want = true;
        }
        true
    }

    /// A buffer was dequeued. Returns the resume latency in ms when this is
    /// the first buffer after a resume request.
    fn on_buffer(&mut self, now: std::time::Instant) -> Option<u64> {
        self.last_buffer_at = Some(self.clock.reading(now));
        self.answered();
        let at = self.resume_requested.take()?;
        let ms = now.saturating_duration_since(at).as_millis() as u64;
        let latency = &mut self.latency;
        latency.count = latency.count.saturating_add(1);
        latency.total_ms = latency.total_ms.saturating_add(ms);
        latency.max_ms = latency.max_ms.max(ms);
        Some(ms)
    }

    fn on_streaming(&mut self) {
        self.paused_at = None;
        self.paused_by_us = false;
        self.self_pause_pending = false;
        if self.streaming_answers {
            self.answered();
        }
    }

    fn on_paused(&mut self, now: std::time::Instant) {
        if self.paused_at.is_none() {
            self.paused_at = Some(self.clock.reading(now));
            self.paused_by_us = !self.want || self.self_pause_pending;
        }
        self.self_pause_pending = false;
    }

    /// Error, or Unconnected after streaming.
    fn on_failed(&mut self) {
        if self.pausing() && !self.want && self.unanswered_since.is_none() {
            self.failed_while_paused = true;
        }
    }

    fn on_unpaused(&mut self) {
        self.paused_at = None;
        self.paused_by_us = false;
    }

    fn paused_for(&self, now: std::time::Instant) -> Option<Duration> {
        let at = self.paused_at?;
        Some(self.clock.reading(now).saturating_sub(at))
    }

    /// Paused by the tracker's own request: it does not want the stream
    /// running, and the pause began after the clock stopped. A pause that
    /// began, or lasted, while the tracker wanted the stream running has
    /// active time on it and is not this.
    fn self_paused(&self, now: std::time::Instant) -> bool {
        !self.want && self.paused_for(now) == Some(Duration::ZERO)
    }

    fn quiet_for(&self, now: std::time::Instant) -> Duration {
        self.clock
            .reading(now)
            .saturating_sub(self.last_buffer_at.unwrap_or(Duration::ZERO))
    }
}

/// Queue [`apply_active_now`] on the loop thread. Before the loop is
/// published the thread applies the wanted state itself once it starts,
/// and after it is gone there is nothing to apply.
fn invoke_apply_active(state: &State) {
    let user = state as *const State as *mut c_void;
    let rc = {
        let _guard = lock(&state.loop_lock);
        let pw_loop = state.pw_loop.load(Ordering::Acquire);
        if state.loop_.load(Ordering::Acquire).is_null() || pw_loop.is_null() {
            return;
        }
        // SAFETY: `pw_loop` is published only between `pw_loop_enter` and
        // the `unpublish_pw_loop` before `pw_loop_leave`, and both changes
        // happen under `loop_lock`, which this holds. So the loop is entered
        // and alive: the invoke is queued for the loop thread (not run inline
        // here), and that thread holds the `State` while it runs it.
        match state.loop_invoke {
            Some(invoke) => unsafe {
                invoke(
                    pw_loop,
                    apply_active_on_loop,
                    1,
                    std::ptr::null(),
                    0,
                    false,
                    user,
                )
            },
            None => return,
        }
    };
    if rc < 0 {
        tracing::debug!(rc, "could not queue the screencast pause change");
    }
}

unsafe extern "C" fn apply_active_on_loop(
    _loop: *mut c_void,
    _async: bool,
    _seq: u32,
    _data: *const c_void,
    _size: usize,
    user: *mut c_void,
) -> i32 {
    if user.is_null() {
        return 0;
    }
    let state = unsafe { &*(user as *const State) };
    apply_active_now(state);
    0
}

/// Pass the wanted state to `pw_stream_set_active`. Loop thread only. The
/// pause lock is released first: `set_active` can emit a state change.
fn apply_active_now(state: &State) {
    let Some(set_active) = state.set_active else {
        return;
    };
    let stream = state.stream.load(Ordering::Acquire);
    if stream.is_null() {
        return;
    }
    let want = lock(&state.pause).want;
    if state.applied_active.load(Ordering::Acquire) == want {
        return;
    }
    // SAFETY: loop thread, live stream (nulled before `pw_stream_destroy`).
    let rc = unsafe { set_active(stream, want) };
    if rc < 0 {
        tracing::debug!(rc, active = want, "pw_stream_set_active failed");
        return;
    }
    state.applied_active.store(want, Ordering::Release);
    tracing::trace!(active = want, "screencast stream set active");
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

/// Stamp the active clock, and log the resume latency for the first buffer
/// after a resume.
fn note_buffer_for_pause(state: &State) {
    let resumed = lock(&state.pause).on_buffer(std::time::Instant::now());
    if let Some(resume_ms) = resumed {
        tracing::debug!(resume_ms, "screencast resume: first buffer");
    }
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
        // A stream closed while a resume is still unanswered counts as a
        // failed resume (a reopen for a stall, a dead stream, or a close).
        // Bound first: an `if let` on the guard would hold the lock through
        // `trip`, which takes it again.
        let retired = lock(&self.state.pause).retire();
        if let Some(why) = retired {
            self.trip(why);
        }
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
fn begin_loop(state: &State, main: *mut c_void, pw_loop: *mut c_void) -> bool {
    let _guard = state
        .loop_lock
        .lock()
        .unwrap_or_else(|err| err.into_inner());
    if state.quit_requested.load(Ordering::Acquire) {
        return false;
    }
    state.pw_loop.store(pw_loop, Ordering::Release);
    state.loop_.store(main, Ordering::Release);
    true
}

/// Stop handing `pw_loop` to [`invoke_apply_active`]. Called before
/// `pw_loop_leave`, under `loop_lock`, so no invoke can start after it.
fn unpublish_pw_loop(state: &State) {
    let _guard = lock(&state.loop_lock);
    state.pw_loop.store(std::ptr::null_mut(), Ordering::Release);
}

/// `pw_loop_leave`, after [`unpublish_pw_loop`]: once the loop is left,
/// an invoke from another thread runs inline on that thread, where
/// `pw_stream_set_active` must not run.
///
/// # Safety
/// `pw_loop` is the loop this thread entered.
unsafe fn leave_loop(state: &State, leave: LoopLeaveFn, pw_loop: *mut c_void) {
    unpublish_pw_loop(state);
    unsafe { leave(pw_loop) };
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
        state.pw_loop.store(std::ptr::null_mut(), Ordering::Release);
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
    // Until PipeWire negotiates, the cap is the configured one.
    crate::perf::set_stream_cap(configured_cap(max_fps));
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
        streaming_logged: AtomicBool::new(false),
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
        pause: Mutex::new(PauseState::new(std::time::Instant::now())),
        applied_active: AtomicBool::new(true),
        pw_loop: AtomicPtr::new(std::ptr::null_mut()),
        set_active: symbols.set_active,
        loop_invoke: symbols.loop_invoke,
        trips: &PAUSE_TRIPS,
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
    /// Set only when pausing is safe on this libpipewire ([`pause_symbols`]).
    set_active: Option<SetActiveFn>,
    loop_invoke: Option<PwLoopInvokeFn>,
}

/// Pausing needs `pw_stream_set_active` called on the loop thread. The
/// tracker queues it there with the exported `pw_loop_invoke` (1.4 on),
/// and only on the `pw_loop_iterate` path, where the loop is entered before
/// it is published. Anything else keeps the stream running as in rc.6.
fn pause_symbols(
    path: LoopPath,
    set_active: Option<SetActiveFn>,
    invoke: Option<PwLoopInvokeFn>,
) -> (Option<SetActiveFn>, Option<PwLoopInvokeFn>) {
    match (path, set_active, invoke) {
        (LoopPath::Iterate, Some(set_active), Some(invoke)) => (Some(set_active), Some(invoke)),
        _ => (None, None),
    }
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
        let (set_active, loop_invoke) = pause_symbols(
            path,
            load_optional::<SetActiveFn>(lib, b"pw_stream_set_active\0"),
            load_optional::<PwLoopInvokeFn>(lib, b"pw_loop_invoke\0"),
        );
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
            set_active,
            loop_invoke,
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
                            if begin_loop(&state, main, pw_loop) {
                                // A pause asked for before the loop was
                                // published could not be queued.
                                apply_active_now(&state);
                                while !state.quit_requested.load(Ordering::Acquire) {
                                    iterate(pw_loop, 200);
                                }
                            }
                            leave_loop(&state, leave, pw_loop);
                        }
                    }
                    LoopDrive::MainLoopRun(run) => {
                        // PipeWire 1.0 and 1.2. `pw_main_loop_quit` returns
                        // this call. `begin_loop` still refuses a quit that
                        // landed before the pointer was published.
                        if begin_loop(&state, main, pw_loop) {
                            apply_active_now(&state);
                            run(main);
                        }
                    }
                }
            }
            // A pause change still queued must not reach a destroyed stream.
            state.stream.store(std::ptr::null_mut(), Ordering::Release);
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
        lock(&state_ref.pause).on_streaming();
        // Every resume reaches Streaming again. Info once per stream.
        if !state_ref.streaming_logged.swap(true, Ordering::AcqRel) {
            tracing::info!("screencast pipewire state streaming");
        } else {
            tracing::debug!("screencast pipewire state streaming");
        }
    } else {
        state_ref.streaming.store(false, Ordering::Release);
        if state == PW_STREAM_STATE_PAUSED && state_ref.seen_streaming.load(Ordering::Acquire) {
            lock(&state_ref.pause).on_paused(std::time::Instant::now());
        } else {
            lock(&state_ref.pause).on_unpaused();
        }
        tracing::debug!(state = pw_state_name(state), "screencast pipewire state");
    }
    // Error and unconnected end the stream. A pause is recorded above and
    // only counts once it persists. Connecting is negotiation.
    if stream_state_is_dead(state, state_ref.seen_streaming.load(Ordering::Acquire)) {
        lock(&state_ref.pause).on_failed();
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
    let rates = pod::fractions(bytes);
    crate::perf::set_stream_cap(stream_cap(&rates, state.max_fps));
    if !state.rate_logged.swap(true, Ordering::AcqRel) {
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
        crate::perf::note_stream_delivered();
        // Every dequeued buffer, including one the throttle drops. This is
        // the PipeWire clock. There is no way to ask a consumer stream for
        // a frame, so a poll does not move this.
        record_dequeued_buffer(&state.last_buffer, std::time::Instant::now());
        note_buffer_for_pause(state);
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
    // This callback runs on the PipeWire thread, start to end, so thread
    // CPU time around the copy is the copy's CPU.
    let cpu_started = crate::perf::thread_cpu();
    let info = buffer_info(state, newest);
    let copied = copy_buffer(state, newest, newest_seq, newest_pts, now_ms);
    let copy_cpu = crate::perf::thread_cpu().saturating_sub(cpu_started);
    unsafe { (state.queue)(stream, newest) };
    // After the requeue, so the log never holds the buffer back.
    crate::perf::record_stream_copy(info, copied.is_some(), copy_cpu);
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

/// What `buf` is, for the performance log: the first data plane's memory
/// type and chunk size, and the negotiated format and size.
fn buffer_info(state: &State, buf: *mut PwBuffer) -> Option<crate::perf::StreamBuffer> {
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

fn configured_cap(max_fps: u32) -> crate::perf::StreamCap {
    crate::perf::StreamCap {
        num: max_fps,
        den: 1,
        source: "configured",
    }
}

/// The negotiated max-framerate from a Format's fractions, else the
/// configured `max_fps`.
pub fn stream_cap(rates: &[(u32, u32)], max_fps: u32) -> crate::perf::StreamCap {
    match pod::negotiated_cap(rates) {
        Some((num, den)) => crate::perf::StreamCap {
            num,
            den,
            source: "negotiated",
        },
        None => configured_cap(max_fps),
    }
}

/// `spa_data_type` as the performance log names it.
pub fn data_type_name(type_: u32) -> &'static str {
    match type_ {
        SPA_DATA_MEM_PTR => "memptr",
        SPA_DATA_MEM_FD => "memfd",
        SPA_DATA_DMA_BUF => "dmabuf",
        _ => "other",
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
/// lives with `PauseState::paused_at`. CONNECTING is negotiation.
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
        "screencast timing: grabs={} avg_grab_ms={} frames_dropped={} resumes={} avg_resume_ms={} max_resume_ms={} resumes_without_buffer={}",
        timing.grabs,
        timing.avg_grab_ms,
        timing.frames_dropped,
        timing.resumes,
        timing.avg_resume_ms,
        timing.max_resume_ms,
        timing.resumes_without_buffer
    )
}

/// A stream with no PipeWire behind it, for other modules' tests. Requests
/// are recorded; nothing is queued, because no loop is published.
#[cfg(test)]
pub(super) fn test_stream(streamed: bool) -> Handle {
    let handle = tests::test_handle(None, None, true);
    handle
        .state
        .seen_streaming
        .store(streamed, Ordering::Release);
    handle.state.streaming.store(streamed, Ordering::Release);
    handle
}

/// A KWin stream the tracker paused, resumed 10 s of active time ago, and
/// still Paused: [`Handle::check_stuck_resume`] trips it.
#[cfg(test)]
pub(super) fn test_stream_stuck_after_our_pause() -> Handle {
    let handle = test_stream(true);
    assert!(handle.enable_pausing(false));
    handle
        .state
        .pw_state
        .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
    handle.state.streaming.store(false, Ordering::Release);
    {
        let mut pause = lock(&handle.state.pause);
        let now = std::time::Instant::now();
        let ago = now.checked_sub(Duration::from_secs(10)).unwrap_or(now);
        pause.clock = ActiveClock::running(ago);
        pause.unanswered_since = Some(Duration::ZERO);
        pause.paused_at = Some(Duration::ZERO);
        pause.paused_by_us = true;
    }
    handle
}

/// [`test_stream`] on a libpipewire without `pw_loop_invoke` (before 1.4).
#[cfg(test)]
pub(super) fn test_stream_without_invoke(streamed: bool) -> Handle {
    let mut handle = test_stream(streamed);
    std::sync::Arc::get_mut(&mut handle.state)
        .expect("fresh test stream")
        .loop_invoke = None;
    handle
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
    fn the_stream_cap_is_the_negotiated_max_framerate() {
        // KWin's negotiated Format: framerate 0/1, maxFramerate 4/1 in a
        // 0/1..60/1 range.
        let pod = pod::kwin_negotiated_format_pod(framebuf::FMT_BGRX, 2560, 1440, 4);
        let cap = stream_cap(&pod::fractions(&pod), 2);
        assert_eq!((cap.num, cap.den, cap.source), (4, 1, "negotiated"));
        assert_eq!(cap.label(), "4");
        // A fixed rate alone is the cap.
        let fixed = stream_cap(&[(60_000, 1_001)], 4);
        assert_eq!(
            (fixed.label(), fixed.source),
            ("59.94".into(), "negotiated")
        );
        // Variable rate and no max: the configured cap.
        for rates in [&[][..], &[(0, 1)][..], &[(0, 1), (0, 1)][..]] {
            let c = stream_cap(rates, 3);
            assert_eq!((c.num, c.den, c.source), (3, 1, "configured"), "{rates:?}");
        }
    }

    #[test]
    fn data_types_are_named() {
        assert_eq!(data_type_name(SPA_DATA_MEM_PTR), "memptr");
        assert_eq!(data_type_name(SPA_DATA_MEM_FD), "memfd");
        assert_eq!(data_type_name(SPA_DATA_DMA_BUF), "dmabuf");
        assert_eq!(data_type_name(0), "other");
    }

    #[test]
    fn timing_line_prints_counts_only() {
        let line = timing_line(&Timing {
            grabs: 4,
            avg_grab_ms: 12,
            frames_dropped: 80,
            resumes: 3,
            avg_resume_ms: 41,
            max_resume_ms: 90,
            resumes_without_buffer: 1,
        });
        assert_eq!(
            line,
            "screencast timing: grabs=4 avg_grab_ms=12 frames_dropped=80 resumes=3 avg_resume_ms=41 max_resume_ms=90 resumes_without_buffer=1"
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

    unsafe extern "C" fn noop_set_active(_: *mut c_void, _: bool) -> i32 {
        0
    }

    unsafe extern "C" fn noop_invoke(
        _: *mut c_void,
        _: SpaInvokeFn,
        _: u32,
        _: *const c_void,
        _: usize,
        _: bool,
        _: *mut c_void,
    ) -> i32 {
        0
    }

    pub(super) fn test_handle(
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
                pause: Mutex::new(PauseState::new(std::time::Instant::now())),
                applied_active: AtomicBool::new(true),
                pw_loop: AtomicPtr::new(std::ptr::null_mut()),
                set_active: Some(noop_set_active),
                loop_invoke: Some(noop_invoke),
                trips: Box::leak(Box::new(AtomicU32::new(0))),
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
                streaming_logged: AtomicBool::new(false),
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

    fn ms(n: u64) -> Duration {
        Duration::from_millis(n)
    }

    /// niri: Streaming again answers a resume.
    fn niri_state(at: std::time::Instant) -> PauseState {
        let mut pause = managed_state(at);
        pause.streaming_answers = true;
        pause
    }

    /// KWin and everything but niri: only a buffer answers a resume.
    fn managed_state(at: std::time::Instant) -> PauseState {
        let mut pause = PauseState::new(at);
        pause.managed = true;
        pause
    }

    #[test]
    fn the_active_clock_stops_while_paused() {
        let t0 = std::time::Instant::now();
        let mut clock = ActiveClock::running(t0);
        assert_eq!(clock.reading(t0 + ms(300)), ms(300));
        clock.stop(t0 + ms(300));
        assert_eq!(
            clock.reading(t0 + ms(5_000)),
            ms(300),
            "paused time is not counted"
        );
        clock.start(t0 + ms(5_000));
        clock.start(t0 + ms(5_100)); // a second start does not move it
        assert_eq!(clock.reading(t0 + ms(5_200)), ms(500));
        clock.stop(t0 + ms(5_200));
        clock.stop(t0 + ms(6_000));
        assert_eq!(clock.reading(t0 + ms(9_000)), ms(500));
    }

    #[test]
    fn a_pause_is_ignored_unless_managed_and_streamed_and_not_tripped() {
        let t0 = std::time::Instant::now();
        let mut off = PauseState::new(t0);
        assert!(
            !off.request(false, true, false, t0),
            "pausing off: no change"
        );
        assert!(off.want);
        let mut fresh = managed_state(t0);
        assert!(
            !fresh.request(false, false, false, t0),
            "a stream that has not streamed keeps running"
        );
        assert!(fresh.want);
        assert!(fresh.request(false, true, false, t0 + ms(10)));
        assert!(!fresh.want);
        assert!(fresh.request(true, true, false, t0 + ms(20)));
        assert!(
            !fresh.request(true, true, false, t0 + ms(30)),
            "already running"
        );
        assert!(fresh.trip(TRIP_TAB_MISS, t0 + ms(35)));
        assert!(
            !fresh.trip(TRIP_TAB_MISS, t0 + ms(36)),
            "one trip per stream"
        );
        assert!(
            !fresh.request(false, true, false, t0 + ms(40)),
            "tripped: runs on"
        );
        assert!(fresh.want);
    }

    #[test]
    fn the_trackers_own_pause_is_not_a_paused_stream_but_a_foreign_one_is() {
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        // Our pause: the clock stops, then PipeWire reports Paused.
        pause.request(false, true, false, t0 + ms(1_000));
        pause.on_paused(t0 + ms(1_010));
        let later = t0 + ms(60_000);
        assert_eq!(pause.paused_for(later), Some(Duration::ZERO));
        assert!(
            pause.self_paused(later),
            "a minute of our own pause is healthy"
        );
        // Resume, Streaming comes back: no pause left.
        pause.request(true, true, false, later);
        assert!(!pause.self_paused(later));
        pause.on_unpaused();
        assert_eq!(pause.paused_for(later + ms(50)), None);

        // A pause nobody asked for, while the tracker wants the stream.
        let start = later + ms(100);
        pause.on_paused(start);
        assert!(!pause.self_paused(start + ms(500)));
        assert_eq!(pause.paused_for(start + ms(500)), Some(ms(500)));
        // The grab ends and the tracker pauses: the clock stops, the
        // foreign pause keeps its 500 ms and still is not ours.
        pause.request(false, true, false, start + ms(500));
        assert!(!pause.self_paused(start + ms(30_000)));
        assert_eq!(pause.paused_for(start + ms(30_000)), Some(ms(500)));
        // The next poll resumes and the stream stays Paused: active time
        // adds up to the 2 s that marks it dead.
        let poll = start + ms(31_500);
        pause.request(true, true, false, poll);
        assert_eq!(pause.paused_for(poll + ms(1_500)), Some(ms(2_000)));
    }

    #[test]
    fn quiet_and_running_time_skip_the_trackers_pauses() {
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        pause.on_buffer(t0 + ms(400));
        pause.request(false, true, false, t0 + ms(500));
        let resume = t0 + ms(60_000);
        assert_eq!(
            pause.quiet_for(resume),
            ms(100),
            "a paused minute is not quiet"
        );
        assert_eq!(pause.clock.reading(resume), ms(500));
        pause.request(true, true, false, resume);
        assert_eq!(pause.quiet_for(resume + ms(8_000)), ms(8_100));
        // Without pausing the clocks are wall time, as before.
        let mut off = PauseState::new(t0);
        off.on_buffer(t0 + ms(400));
        off.request(false, true, false, t0 + ms(500));
        assert_eq!(off.quiet_for(resume), ms(59_600));
    }

    #[test]
    fn a_resume_logs_its_latency_and_a_pause_before_a_buffer_counts_as_unanswered() {
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        pause.request(false, true, false, t0);
        pause.request(true, true, false, t0 + ms(1_000));
        assert_eq!(pause.on_buffer(t0 + ms(1_040)), Some(40));
        assert_eq!(
            pause.on_buffer(t0 + ms(1_290)),
            None,
            "only the first buffer"
        );
        pause.request(false, true, false, t0 + ms(1_300));
        pause.request(true, true, false, t0 + ms(2_000));
        assert_eq!(pause.on_buffer(t0 + ms(2_090)), Some(90));
        pause.request(false, true, false, t0 + ms(2_100));
        pause.request(true, true, false, t0 + ms(3_000));
        pause.request(false, true, false, t0 + ms(3_200));
        let latency = pause.latency;
        assert_eq!(
            (
                latency.count,
                latency.total_ms,
                latency.max_ms,
                latency.unanswered
            ),
            (2, 130, 90, 1)
        );
    }

    /// One poll as `capture()` runs it on a resumed stream that stays
    /// Paused: the pre-grab health read, the resume, the grab window, the
    /// post-grab health read, the pause. Returns the trip reason if a health
    /// read tripped, and the paused-for that read saw.
    fn stuck_poll(
        pause: &mut PauseState,
        at: std::time::Instant,
        window: Duration,
    ) -> Option<(&'static str, Duration)> {
        // `capture()` runs the stuck check, then reads health. A read whose
        // pause has reached PAUSED_PERSIST (dead, reopen) must trip first.
        let read = |pause: &mut PauseState, now| {
            let seen = pause.paused_for(now).unwrap_or_default();
            let why = pause.stuck(PwPhase::Paused, now);
            assert!(
                why.is_some() || seen < crate::capture::screencast::PAUSED_PERSIST,
                "a dead read at {seen:?} without the trip"
            );
            pause.trip(why?, now);
            Some((why?, seen))
        };
        if let Some(hit) = read(pause, at) {
            return Some(hit);
        }
        pause.request(true, true, false, at);
        let end = at + window;
        if let Some(why) = pause.grab_end(false, end) {
            pause.trip(why, end);
            return Some((why, pause.paused_for(end).unwrap_or_default()));
        }
        if let Some(hit) = read(pause, end) {
            return Some(hit);
        }
        pause.request(false, true, false, end);
        None
    }

    #[test]
    fn a_resume_that_stays_paused_trips_before_the_paused_rule_reopens() {
        let persist = crate::capture::screencast::PAUSED_PERSIST;
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        // Our own pause: PipeWire reports Paused after the request.
        pause.request(false, true, false, t0);
        pause.on_paused(t0 + ms(5));
        assert!(pause.paused_by_us);
        let mut at = t0 + ms(1_500);
        let mut hit = None;
        for _ in 0..10 {
            hit = stuck_poll(&mut pause, at, ms(750));
            if hit.is_some() {
                break;
            }
            at += ms(1_500);
        }
        let (why, seen) = hit.expect("the fail-safe trips");
        assert_eq!(why, TRIP_STUCK);
        assert!(
            seen < persist,
            "tripped at {seen:?} of pause, before the {persist:?} reopen"
        );
        assert!(pause.tripped.is_some() && pause.want);
        // Tab windows (1.15 s): the trip lands on the read that would first
        // see the pause as dead, before that read's reopen decision.
        let mut tab = managed_state(t0);
        tab.request(false, true, false, t0);
        tab.on_paused(t0);
        let (why, _) = stuck_poll(&mut tab, t0 + ms(2_000), ms(1_150))
            .or_else(|| stuck_poll(&mut tab, t0 + ms(4_000), ms(1_150)))
            .expect("trips");
        assert_eq!(why, TRIP_STUCK);
        assert!(tab.tripped.is_some());
    }

    #[test]
    fn a_resume_that_fails_trips_at_once_and_a_foreign_pause_does_not() {
        let t0 = std::time::Instant::now();
        let mut failed = managed_state(t0);
        failed.request(false, true, false, t0);
        failed.request(true, true, false, t0 + ms(1_000));
        assert_eq!(
            failed.stuck(PwPhase::Failed, t0 + ms(1_010)),
            Some(TRIP_STUCK)
        );
        // A pause that began after the resumed stream was Streaming again
        // is not the tracker's: the 2 s paused rule handles it, as in rc.6.
        let mut foreign = managed_state(t0);
        foreign.request(false, true, false, t0);
        foreign.on_paused(t0 + ms(1));
        foreign.request(true, true, false, t0 + ms(1_000));
        foreign.on_streaming();
        foreign.on_paused(t0 + ms(1_100));
        assert!(!foreign.paused_by_us);
        assert_eq!(foreign.stuck(PwPhase::Paused, t0 + ms(5_000)), None);
        // An answered resume (a buffer) is never stuck.
        let mut ok = managed_state(t0);
        ok.request(false, true, false, t0);
        ok.on_paused(t0);
        ok.request(true, true, false, t0 + ms(1_000));
        ok.on_buffer(t0 + ms(1_010));
        assert_eq!(ok.stuck(PwPhase::Failed, t0 + ms(5_000)), None);
        // Pausing off: never.
        let mut off = PauseState::new(t0);
        off.unanswered_since = Some(Duration::ZERO);
        assert_eq!(off.stuck(PwPhase::Failed, t0 + ms(5_000)), None);
    }

    #[test]
    fn a_still_niri_screen_reaches_streaming_and_never_counts() {
        let t0 = std::time::Instant::now();
        let mut pause = niri_state(t0);
        let mut at = t0;
        for _ in 0..20 {
            pause.request(false, true, false, at);
            pause.on_paused(at + ms(2));
            at += ms(1_500);
            pause.request(true, true, false, at);
            // niri: Streaming again, but no damage, so no buffer.
            pause.on_streaming();
            assert_eq!(
                pause.grab_end(true, at + ms(1_150)),
                None,
                "a Tab over a still window"
            );
            assert_eq!(pause.grab_end(false, at + ms(1_150)), None);
            assert_eq!(pause.stuck(PwPhase::Streaming, at + ms(1_150)), None);
            // The grab held it running for 1.15 s of active time.
            at += ms(1_150);
        }
        assert!(
            pause.clock.reading(at) >= ms(20 * 1_150),
            "active time accrued"
        );
        assert_eq!(pause.resume_fails, 0);
        assert!(pause.tripped.is_none());
        assert_eq!(pause.retire(), None);
    }

    /// Review round 2, V4: on KWin a resume always sends a frame, so one that
    /// reaches Streaming and sends none is broken. Polls (750 ms) and Tabs
    /// (1.15 s) must trip it, count it, and a close must count it too.
    #[test]
    fn kwin_streaming_without_a_buffer_trips_and_counts() {
        let t0 = std::time::Instant::now();
        let broken = |grab: Duration, tab: bool| {
            let mut pause = managed_state(t0);
            let mut at = t0;
            for n in 1..=20u32 {
                pause.request(false, true, false, at);
                pause.on_paused(at + ms(2));
                at += ms(1_500);
                pause.request(true, true, false, at);
                pause.on_streaming();
                assert_eq!(pause.stuck(PwPhase::Streaming, at + grab), None);
                if let Some(why) = pause.grab_end(tab, at + grab) {
                    return (why, n);
                }
                // The grab is over: the next pause starts when it ended.
                at += grab;
            }
            panic!("never tripped");
        };
        // Polls: two resumes of 1.5 s active time each, in a row.
        assert_eq!(broken(ms(750), false), (TRIP_SLOW_RESUMES, 4));
        // A Tab that misses on the first broken resume.
        assert_eq!(broken(ms(1_150), true), (TRIP_TAB_MISS, 1));
        // Closed after a resume that reached Streaming with no buffer.
        let mut closed = managed_state(t0);
        closed.request(false, true, false, t0);
        closed.request(true, true, false, t0 + ms(1_000));
        closed.on_streaming();
        assert_eq!(closed.retire(), Some(TRIP_RETIRED));
        // Through the handle: the trip is counted.
        let handle = test_stream(true);
        assert!(handle.enable_pausing(false));
        handle
            .state
            .pw_state
            .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
        handle.set_active(false);
        handle.set_active(true);
        lock(&handle.state.pause).on_streaming();
        handle.note_grab_result(&Ok(GrabOutcome::TabMiss { delay_ms: 1_200 }));
        assert!(handle.pause_tripped());
        assert_eq!(handle.state.trips.load(Ordering::Acquire), 1);
        // The same on niri never trips.
        let niri = test_stream(true);
        assert!(niri.enable_pausing(true));
        niri.state
            .pw_state
            .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
        niri.set_active(false);
        niri.set_active(true);
        lock(&niri.state.pause).on_streaming();
        niri.note_grab_result(&Ok(GrabOutcome::TabMiss { delay_ms: 1_200 }));
        assert!(!niri.pause_tripped());
        let trips = niri.state.trips;
        drop(niri);
        assert_eq!(trips.load(Ordering::Acquire), 0, "retired answered");
    }

    #[test]
    fn a_stream_that_fails_while_we_hold_it_paused_counts() {
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        pause.request(false, true, false, t0);
        pause.on_paused(t0 + ms(1));
        pause.on_failed();
        assert!(pause.failed_while_paused);
        assert_eq!(
            pause.stuck(PwPhase::Failed, t0 + ms(5)),
            Some(TRIP_FAILED_PAUSED)
        );
        assert_eq!(pause.retire(), Some(TRIP_FAILED_PAUSED));
        // Failing while the tracker wants it running is not this.
        let mut running = managed_state(t0);
        running.on_failed();
        assert!(!running.failed_while_paused);
        assert_eq!(running.retire(), None);
        // Nor with pausing off.
        let mut off = PauseState::new(t0);
        off.want = false;
        off.on_failed();
        assert_eq!(off.retire(), None);
        // A failure with a resume outstanding is the stuck rule's.
        let mut resumed = managed_state(t0);
        resumed.request(false, true, false, t0);
        resumed.request(true, true, false, t0 + ms(10));
        resumed.request(false, true, false, t0 + ms(20));
        resumed.on_failed();
        assert!(!resumed.failed_while_paused);
        assert_eq!(
            resumed.stuck(PwPhase::Failed, t0 + ms(30)),
            Some(TRIP_STUCK)
        );
    }

    #[test]
    fn a_resume_before_our_pauses_paused_still_marks_it_ours() {
        let t0 = std::time::Instant::now();
        let mut race = managed_state(t0);
        race.request(false, true, false, t0);
        race.request(true, true, false, t0 + ms(5));
        race.on_paused(t0 + ms(6));
        assert!(race.paused_by_us, "the Paused answers our pause request");
        // A pause that was never applied leaves nothing pending.
        let mut flap = managed_state(t0);
        flap.request(false, true, false, t0);
        flap.request(true, true, true, t0 + ms(5));
        flap.on_paused(t0 + ms(500));
        assert!(!flap.paused_by_us);
    }

    #[test]
    fn two_consecutive_unanswered_resumes_trip_and_an_answer_resets_the_count() {
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        pause.request(false, true, false, t0);
        // Unanswered, state not Paused (so the stuck rule is not the one
        // that fires): 2 polls of 750 ms make one failed resume.
        let poll = |pause: &mut PauseState, at: std::time::Instant| {
            pause.request(true, true, false, at);
            let why = pause.grab_end(false, at + ms(750));
            pause.request(false, true, false, at + ms(750));
            why
        };
        assert_eq!(poll(&mut pause, t0 + ms(1_000)), None);
        assert_eq!(poll(&mut pause, t0 + ms(2_500)), None);
        assert_eq!(pause.resume_fails, 1);
        // The next resume is answered: the count starts again.
        pause.request(true, true, false, t0 + ms(4_000));
        pause.on_buffer(t0 + ms(4_050));
        pause.request(false, true, false, t0 + ms(4_100));
        assert_eq!(pause.resume_fails, 0);
        let mut at = t0 + ms(5_000);
        let mut polls = 0;
        let mut why = None;
        while why.is_none() && polls < 20 {
            why = poll(&mut pause, at);
            at += ms(1_500);
            polls += 1;
        }
        assert_eq!(why, Some(TRIP_SLOW_RESUMES));
        assert_eq!(polls, 4, "two resumes of 1.5 s each, in a row");
        assert_eq!(RESUME_FAILS_TO_TRIP, 2);
        assert_eq!(RESUME_BUFFER_BOUND, ms(1_500));
        // A buffer answers too.
        let mut buf = managed_state(t0);
        buf.request(false, true, false, t0);
        assert_eq!(poll(&mut buf, t0 + ms(1_000)), None);
        assert_eq!(poll(&mut buf, t0 + ms(2_500)), None);
        buf.request(true, true, false, t0 + ms(4_000));
        buf.on_buffer(t0 + ms(4_050));
        assert_eq!(buf.resume_fails, 0);
    }

    #[test]
    fn a_tab_miss_trips_only_while_the_resume_is_unanswered() {
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        pause.request(false, true, false, t0);
        pause.request(true, true, false, t0 + ms(1_000));
        assert_eq!(pause.grab_end(true, t0 + ms(2_200)), Some(TRIP_TAB_MISS));
        // Early buffers answered the resume: a pacing miss, not a dead one.
        let mut early = managed_state(t0);
        early.request(false, true, false, t0);
        early.request(true, true, false, t0 + ms(1_000));
        early.on_buffer(t0 + ms(1_050));
        assert_eq!(early.grab_end(true, t0 + ms(2_200)), None);
        // Streaming again with no buffer (a still window on niri).
        let mut still = niri_state(t0);
        still.request(false, true, false, t0);
        still.request(true, true, false, t0 + ms(1_000));
        still.on_streaming();
        assert_eq!(still.grab_end(true, t0 + ms(2_200)), None);
        // A resume of a stream that never left Streaming has nothing to answer.
        let mut flap = managed_state(t0);
        flap.request(false, true, false, t0);
        flap.request(true, true, true, t0 + ms(5));
        assert_eq!(flap.grab_end(true, t0 + ms(1_200)), None);
        // Never paused, or pausing off.
        let mut never = managed_state(t0);
        assert_eq!(never.grab_end(true, t0 + ms(2_200)), None);
        let mut off = PauseState::new(t0);
        off.unanswered_since = Some(Duration::ZERO);
        assert_eq!(off.grab_end(true, t0 + ms(2_200)), None);
    }

    #[test]
    fn closing_a_stream_with_an_unanswered_resume_is_a_trip() {
        let t0 = std::time::Instant::now();
        let mut pause = managed_state(t0);
        assert_eq!(pause.retire(), None);
        pause.request(false, true, false, t0);
        pause.request(true, true, false, t0 + ms(1_000));
        assert_eq!(pause.retire(), Some(TRIP_RETIRED));
        pause.on_buffer(t0 + ms(1_050));
        assert_eq!(pause.retire(), None);

        let handle = test_stream(true);
        assert!(handle.enable_pausing(false));
        handle
            .state
            .pw_state
            .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
        handle.set_active(false);
        handle.set_active(true);
        let trips = handle.state.trips;
        drop(handle);
        assert_eq!(trips.load(Ordering::Acquire), 1, "retired unanswered");
        let calm = test_stream(true);
        assert!(calm.enable_pausing(false));
        let trips = calm.state.trips;
        drop(calm);
        assert_eq!(trips.load(Ordering::Acquire), 0);
    }

    #[test]
    fn a_tripped_stream_runs_and_counts_once() {
        let handle = test_stream(true);
        assert!(handle.enable_pausing(false));
        assert!(handle.pausing());
        handle
            .state
            .pw_state
            .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
        handle.set_active(false);
        assert!(!handle.wants_active());
        handle.set_active(true);
        handle.note_grab_end(true);
        assert!(handle.pause_tripped());
        assert!(!handle.pausing());
        assert!(handle.wants_active());
        assert_eq!(handle.state.trips.load(Ordering::Acquire), 1);
        handle.set_active(false);
        assert!(handle.wants_active(), "no more pauses on this stream");
        handle.trip(TRIP_STUCK);
        handle.trip(TRIP_RETIRED);
        assert_eq!(
            handle.state.trips.load(Ordering::Acquire),
            1,
            "counted once"
        );
        assert_eq!(PAUSE_TRIPS_LIMIT, 2, "one retry after the first trip");
    }

    #[test]
    fn a_grab_result_tells_the_fail_safe_whether_a_tab_missed() {
        let resumed = || {
            let handle = test_stream(true);
            assert!(handle.enable_pausing(false));
            handle
                .state
                .pw_state
                .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
            handle.set_active(false);
            handle.set_active(true);
            handle
        };
        let tab_miss = resumed();
        tab_miss.note_grab_result(&Ok(GrabOutcome::TabMiss { delay_ms: 1_200 }));
        assert!(
            tab_miss.pause_tripped(),
            "a Tab miss on an unanswered resume"
        );
        let frame = resumed();
        frame.note_grab_result(&Ok(GrabOutcome::Frame {
            frame: Arc::new(red_frame(1)),
            is_new: true,
        }));
        assert!(!frame.pause_tripped());
        let ended = resumed();
        ended.note_grab_result(&Err(NO_STREAM_FRAME.into()));
        assert!(
            !ended.pause_tripped(),
            "a poll with nothing is not a Tab miss"
        );
    }

    #[test]
    fn a_stuck_resume_trips_through_the_handle_before_health_is_read() {
        let handle = test_stream(true);
        assert!(handle.enable_pausing(false));
        handle
            .state
            .pw_state
            .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
        {
            let mut pause = lock(&handle.state.pause);
            let ago = std::time::Instant::now() - ms(10_000);
            pause.clock = ActiveClock::running(ago);
            pause.unanswered_since = Some(Duration::ZERO);
            pause.paused_at = Some(Duration::ZERO);
            pause.paused_by_us = true;
        }
        handle.check_stuck_resume();
        assert!(handle.pause_tripped());
        assert_eq!(handle.state.trips.load(Ordering::Acquire), 1);
    }

    static LEAVE_STATE: AtomicPtr<State> = AtomicPtr::new(std::ptr::null_mut());
    static LEAVE_SAW_PUBLISHED: AtomicBool = AtomicBool::new(false);
    static LEAVE_INVOKES: AtomicU32 = AtomicU32::new(0);

    unsafe extern "C" fn fake_leave(_: *mut c_void) {
        let state = unsafe { &*LEAVE_STATE.load(Ordering::Acquire) };
        LEAVE_SAW_PUBLISHED.store(
            !state.pw_loop.load(Ordering::Acquire).is_null(),
            Ordering::Release,
        );
    }

    unsafe extern "C" fn counting_invoke(
        _: *mut c_void,
        _: SpaInvokeFn,
        _: u32,
        _: *const c_void,
        _: usize,
        _: bool,
        _: *mut c_void,
    ) -> i32 {
        LEAVE_INVOKES.fetch_add(1, Ordering::AcqRel);
        0
    }

    /// After `pw_loop_leave` an invoke from another thread runs inline on
    /// that thread. `leave_loop` stops publishing the loop first, so a pause
    /// change after it queues nothing at all.
    #[test]
    fn no_invoke_reaches_the_loop_once_it_is_left() {
        let mut handle = test_stream(true);
        Arc::get_mut(&mut handle.state).unwrap().loop_invoke = Some(counting_invoke);
        let fake = std::ptr::NonNull::<u8>::dangling().as_ptr() as *mut c_void;
        handle.state.loop_.store(fake, Ordering::Release);
        handle.state.pw_loop.store(fake, Ordering::Release);
        // Published: a change is queued on the loop.
        invoke_apply_active(&handle.state);
        assert_eq!(LEAVE_INVOKES.load(Ordering::Acquire), 1);
        LEAVE_STATE.store(Arc::as_ptr(&handle.state) as *mut State, Ordering::Release);
        unsafe { leave_loop(&handle.state, fake_leave, fake) };
        assert!(
            !LEAVE_SAW_PUBLISHED.load(Ordering::Acquire),
            "unpublished before pw_loop_leave"
        );
        invoke_apply_active(&handle.state);
        assert_eq!(
            LEAVE_INVOKES.load(Ordering::Acquire),
            1,
            "nothing invoked after leave"
        );
        handle
            .state
            .loop_
            .store(std::ptr::null_mut(), Ordering::Release);
    }

    static SET_ACTIVE_CALLS: Mutex<Vec<bool>> = Mutex::new(Vec::new());
    static SET_ACTIVE_RC: AtomicI32 = AtomicI32::new(0);

    unsafe extern "C" fn recording_set_active(stream: *mut c_void, active: bool) -> i32 {
        assert!(!stream.is_null());
        SET_ACTIVE_CALLS.lock().unwrap().push(active);
        SET_ACTIVE_RC.load(Ordering::Acquire)
    }

    #[test]
    fn apply_active_now_calls_pipewire_once_per_change_and_retries_a_failure() {
        let mut handle = test_stream(true);
        Arc::get_mut(&mut handle.state).unwrap().set_active = Some(recording_set_active);
        let fake = std::ptr::NonNull::<u8>::dangling().as_ptr() as *mut c_void;
        handle.state.stream.store(fake, Ordering::Release);
        let calls = || SET_ACTIVE_CALLS.lock().unwrap().clone();
        assert!(handle.enable_pausing(false));
        // Running and applied running: nothing to do.
        apply_active_now(&handle.state);
        assert_eq!(calls(), Vec::<bool>::new());
        lock(&handle.state.pause).want = false;
        apply_active_now(&handle.state);
        apply_active_now(&handle.state);
        assert_eq!(calls(), vec![false], "coalesced: one call per change");
        assert!(!handle.state.applied_active.load(Ordering::Acquire));
        // PipeWire refuses: the applied state stays, the next try calls again.
        SET_ACTIVE_RC.store(-22, Ordering::Release);
        lock(&handle.state.pause).want = true;
        apply_active_now(&handle.state);
        assert!(!handle.state.applied_active.load(Ordering::Acquire));
        SET_ACTIVE_RC.store(0, Ordering::Release);
        apply_active_now(&handle.state);
        assert!(handle.state.applied_active.load(Ordering::Acquire));
        assert_eq!(calls(), vec![false, true, true]);
        // A destroyed stream (nulled before pw_stream_destroy) is skipped.
        handle
            .state
            .stream
            .store(std::ptr::null_mut(), Ordering::Release);
        lock(&handle.state.pause).want = false;
        apply_active_now(&handle.state);
        assert_eq!(calls(), vec![false, true, true]);
        // Without a published loop nothing is queued either.
        invoke_apply_active(&handle.state);
        assert_eq!(calls(), vec![false, true, true]);
    }

    #[test]
    fn pausing_needs_the_iterate_path_and_both_symbols() {
        unsafe extern "C" fn invoke(
            _: *mut c_void,
            _: SpaInvokeFn,
            _: u32,
            _: *const c_void,
            _: usize,
            _: bool,
            _: *mut c_void,
        ) -> i32 {
            0
        }
        let set: SetActiveFn = noop_set_active;
        let inv: PwLoopInvokeFn = invoke;
        let both = pause_symbols(LoopPath::Iterate, Some(set), Some(inv));
        assert!(both.0.is_some() && both.1.is_some());
        for (path, set, inv) in [
            (LoopPath::MainLoopRun, Some(set), Some(inv)),
            (LoopPath::Iterate, None, Some(inv)),
            (LoopPath::Iterate, Some(set), None),
        ] {
            let got = pause_symbols(path, set, inv);
            assert!(got.0.is_none() && got.1.is_none());
        }
        let handle = test_stream(true);
        assert!(handle.enable_pausing(false));
        let mut old = test_stream(true);
        Arc::get_mut(&mut old.state).unwrap().loop_invoke = None;
        assert!(!old.enable_pausing(false), "PipeWire before 1.4");
        old.set_active(false);
        assert!(old.wants_active());
    }

    #[test]
    fn a_poll_reuses_the_last_frame_while_resuming_from_our_pause_only() {
        let resuming = test_handle(None, Some(red_frame(3)), true);
        resuming
            .state
            .pw_state
            .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
        resuming.state.streaming.store(false, Ordering::Release);
        let unmanaged = test_handle(None, Some(red_frame(3)), true);
        unmanaged
            .state
            .pw_state
            .store(PW_STREAM_STATE_PAUSED, Ordering::Release);
        unmanaged.state.streaming.store(false, Ordering::Release);
        assert!(resuming.enable_pausing(false));
        match grab_now(resuming).expect("reused frame") {
            GrabOutcome::Frame { is_new, .. } => assert!(!is_new),
            GrabOutcome::TabMiss { .. } => panic!("poll returned a tab miss"),
        }
        assert_eq!(
            grab_now(unmanaged).expect_err("pausing off: a Paused stream reuses nothing"),
            NO_STREAM_FRAME
        );
    }

    /// Print a skip past the test harness's output capture, so it shows in
    /// a normal `cargo test` run (and in CI logs).
    fn skipped(why: &str) {
        use std::io::Write as _;
        let _ = writeln!(
            std::io::stderr().lock(),
            "SKIPPED pw_loop_invoke_from_another_thread_runs_on_the_loop_thread: {why}"
        );
    }

    /// The real libpipewire: a call queued with `pw_loop_invoke` from
    /// another thread runs on the loop's own thread, without blocking the
    /// caller. No PipeWire daemon is contacted. Skipped where
    /// libpipewire 1.4 or newer is not installed.
    #[test]
    fn pw_loop_invoke_from_another_thread_runs_on_the_loop_thread() {
        let Ok(lib) = pipewire_library() else {
            skipped("no libpipewire-0.3.so.0");
            return;
        };
        let syms = unsafe {
            (
                load_optional::<unsafe extern "C" fn(*mut i32, *mut *mut *mut c_char)>(
                    lib,
                    b"pw_init\0",
                ),
                load_optional::<unsafe extern "C" fn(*const c_void) -> *mut c_void>(
                    lib,
                    b"pw_main_loop_new\0",
                ),
                load_optional::<unsafe extern "C" fn(*mut c_void) -> *mut c_void>(
                    lib,
                    b"pw_main_loop_get_loop\0",
                ),
                load_optional::<LoopEnterFn>(lib, b"pw_loop_enter\0"),
                load_optional::<LoopIterateFn>(lib, b"pw_loop_iterate\0"),
                load_optional::<LoopLeaveFn>(lib, b"pw_loop_leave\0"),
                load_optional::<unsafe extern "C" fn(*mut c_void)>(lib, b"pw_main_loop_destroy\0"),
                load_optional::<PwLoopInvokeFn>(lib, b"pw_loop_invoke\0"),
            )
        };
        let (
            Some(init),
            Some(new),
            Some(get_loop),
            Some(enter),
            Some(iterate),
            Some(leave),
            Some(destroy),
            Some(invoke),
        ) = syms
        else {
            skipped("libpipewire is older than 1.4");
            return;
        };

        struct Seen {
            ran_on: Mutex<Option<std::thread::ThreadId>>,
        }
        unsafe extern "C" fn record(
            _: *mut c_void,
            _: bool,
            _: u32,
            _: *const c_void,
            _: usize,
            user: *mut c_void,
        ) -> i32 {
            let seen = unsafe { &*(user as *const Seen) };
            *seen.ran_on.lock().unwrap() = Some(std::thread::current().id());
            0
        }
        let seen = Arc::new(Seen {
            ran_on: Mutex::new(None),
        });
        let stop = Arc::new(AtomicBool::new(false));
        let (tx, rx) = std::sync::mpsc::channel::<(usize, std::thread::ThreadId)>();
        let stop_loop = Arc::clone(&stop);
        let thread = std::thread::spawn(move || unsafe {
            init(std::ptr::null_mut(), std::ptr::null_mut());
            let main = new(std::ptr::null());
            assert!(!main.is_null());
            let pw_loop = get_loop(main);
            enter(pw_loop);
            tx.send((pw_loop as usize, std::thread::current().id()))
                .unwrap();
            while !stop_loop.load(Ordering::Acquire) {
                iterate(pw_loop, 20);
            }
            leave(pw_loop);
            destroy(main);
        });
        let (pw_loop, loop_thread) = rx.recv_timeout(Duration::from_secs(5)).unwrap();
        let rc = unsafe {
            invoke(
                pw_loop as *mut c_void,
                record,
                1,
                std::ptr::null(),
                0,
                false,
                Arc::as_ptr(&seen) as *mut c_void,
            )
        };
        assert!(rc >= 0, "pw_loop_invoke failed: {rc}");
        let deadline = std::time::Instant::now() + Duration::from_secs(5);
        while seen.ran_on.lock().unwrap().is_none() && std::time::Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(5));
        }
        stop.store(true, Ordering::Release);
        thread.join().unwrap();
        let ran_on = *seen.ran_on.lock().unwrap();
        assert_eq!(ran_on, Some(loop_thread), "the call ran on the loop thread");
        assert_ne!(ran_on, Some(std::thread::current().id()));
    }
}
