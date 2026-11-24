//! The PipeWire loop thread: the loop and its `loop_lock` publish and quit
//! protocol, the fd hand-off, the C callbacks, and the pause change that
//! runs on the loop.

use std::ffi::{c_char, c_void};
use std::sync::Arc;
use std::sync::atomic::Ordering;

use super::buffer::{buffer_info, copy_buffer, header_pts};
use super::ffi::{
    LoopDrive, LoopLeaveFn, PW_DIRECTION_INPUT, PW_STREAM_FLAG_AUTOCONNECT,
    PW_STREAM_FLAG_MAP_BUFFERS, PW_STREAM_STATE_PAUSED, PW_STREAM_STATE_STREAMING, PwBuffer,
    PwStreamEvents, SPA_PARAM_FORMAT, SpaDict, SpaPod, Symbols, empty_spa_dict,
};
use super::{
    State, copy_ends_request, copy_for_wait, framebuf, lock, note_buffer_for_pause, note_fps, pod,
    pw_state_name, record_dequeued_buffer, state_is_core_fault, stream_cap, stream_link_plan,
    stream_state_is_dead, throttle_gap_ms, update_max_pts,
};

/// Queue [`apply_active_now`] on the loop thread. Before the loop is
/// published the thread applies the wanted state itself once it starts,
/// and after it is gone there is nothing to apply.
pub(super) fn invoke_apply_active(state: &State) {
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
pub(super) fn apply_active_now(state: &State) {
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

pub(super) fn request_quit(state: &State) {
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
pub(super) unsafe fn leave_loop(state: &State, leave: LoopLeaveFn, pw_loop: *mut c_void) {
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

pub(super) fn run_loop(symbols: Symbols, state: Arc<State>, fd: i32, node_id: u32) {
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

pub(super) extern "C" fn on_state(data: *mut c_void, _old: i32, state: i32, error: *const c_char) {
    if data.is_null() {
        return;
    }
    let state_ref = unsafe { &*(data as *const State) };
    // The iterate loop keeps running after a core error, so the thread's
    // own exit (which also sets this) never comes while the stream is up.
    if state_is_core_fault(state, !error.is_null()) {
        state_ref.core_fault.store(true, Ordering::Release);
    }
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
        lock(&state_ref.pause).on_failed(std::time::Instant::now());
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
