//! The libpipewire ABI this crate uses: constants, `#[repr(C)]` structs,
//! fn-pointer types, and loading the library and its symbols at runtime.

use std::ffi::{c_char, c_void};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering};

#[cfg_attr(not(test), allow(dead_code))]
pub(super) const PW_ID_ANY: u32 = 0xffff_ffff;
pub(super) const PW_DIRECTION_INPUT: i32 = 0;
pub(super) const PW_STREAM_FLAG_AUTOCONNECT: u32 = 1;
pub(super) const PW_STREAM_FLAG_MAP_BUFFERS: u32 = 4;
pub(super) const PW_STREAM_STATE_ERROR: i32 = -1;
pub(super) const PW_STREAM_STATE_UNCONNECTED: i32 = 0;
pub(super) const PW_STREAM_STATE_CONNECTING: i32 = 1;
pub(super) const PW_STREAM_STATE_PAUSED: i32 = 2;
pub(super) const PW_STREAM_STATE_STREAMING: i32 = 3;
pub(super) const SPA_META_HEADER: u32 = 1;

pub(super) const SPA_DATA_MEM_PTR: u32 = 1;
pub(super) const SPA_DATA_MEM_FD: u32 = 2;
pub(super) const SPA_DATA_DMA_BUF: u32 = 3;
pub(super) const SPA_PARAM_FORMAT: u32 = 4;

#[repr(C)]
pub(super) struct SpaChunk {
    pub(super) offset: u32,
    pub(super) size: u32,
    pub(super) stride: i32,
    pub(super) flags: i32,
}

#[repr(C)]
pub(super) struct SpaData {
    pub(super) type_: u32,
    flags: u32,
    pub(super) fd: i64,
    pub(super) mapoffset: u32,
    pub(super) maxsize: u32,
    pub(super) data: *mut c_void,
    pub(super) chunk: *mut SpaChunk,
}

#[repr(C)]
pub(super) struct SpaBuffer {
    pub(super) n_metas: u32,
    pub(super) n_datas: u32,
    pub(super) metas: *mut c_void,
    pub(super) datas: *mut SpaData,
}

#[repr(C)]
pub(super) struct PwBuffer {
    pub(super) buffer: *mut SpaBuffer,
    user_data: *mut c_void,
    size: u64,
    requested: u64,
}

#[repr(C)]
pub(super) struct SpaMeta {
    pub(super) type_: u32,
    pub(super) size: u32,
    pub(super) data: *mut c_void,
}

/// `spa_meta_header` from libspa. `pts` is at byte 8. A header that ends
/// before `seq` still has a presentation time.
#[repr(C)]
pub(super) struct SpaMetaHeader {
    flags: u32,
    offset: u32,
    pub(super) pts: i64,
    dts_offset: i64,
    seq: u64,
}

#[repr(C)]
pub(super) struct SpaPod {
    pub(super) size: u32,
    type_: u32,
}

/// `spa_dict` from libspa. `pw_properties_new_dict` reads `n_items` at
/// offset 4, so a null pointer segfaults. `n_items == 0` is an empty dict
/// and the items pointer is not followed.
#[repr(C)]
pub(super) struct SpaDict {
    pub(super) flags: u32,
    pub(super) n_items: u32,
    pub(super) items: *const c_void,
}

pub(super) fn empty_spa_dict() -> SpaDict {
    SpaDict {
        flags: 0,
        n_items: 0,
        items: std::ptr::null(),
    }
}

#[repr(C)]
pub(super) struct PwStreamEvents {
    pub(super) version: u32,
    pub(super) destroy: Option<extern "C" fn(*mut c_void)>,
    pub(super) state_changed: Option<extern "C" fn(*mut c_void, i32, i32, *const c_char)>,
    pub(super) control_info: Option<extern "C" fn(*mut c_void, u32, *const c_void)>,
    pub(super) io_changed: Option<extern "C" fn(*mut c_void, u32, *mut c_void, u32)>,
    pub(super) param_changed: Option<extern "C" fn(*mut c_void, u32, *const SpaPod)>,
    pub(super) add_buffer: Option<extern "C" fn(*mut c_void, *mut PwBuffer)>,
    pub(super) remove_buffer: Option<extern "C" fn(*mut c_void, *mut PwBuffer)>,
    pub(super) process: Option<extern "C" fn(*mut c_void)>,
    pub(super) drained: Option<extern "C" fn(*mut c_void)>,
    pub(super) command: Option<extern "C" fn(*mut c_void, *const c_void)>,
    pub(super) trigger_done: Option<extern "C" fn(*mut c_void)>,
}

pub(super) type DequeueFn = unsafe extern "C" fn(*mut c_void) -> *mut PwBuffer;
pub(super) type QueueFn = unsafe extern "C" fn(*mut c_void, *mut PwBuffer) -> i32;
pub(super) type UpdateFn = unsafe extern "C" fn(*mut c_void, *const *const SpaPod, u32) -> i32;
pub(super) type QuitFn = unsafe extern "C" fn(*mut c_void) -> i32;
pub(super) type SetActiveFn = unsafe extern "C" fn(*mut c_void, bool) -> i32;
/// `spa_invoke_func_t`.
pub(super) type SpaInvokeFn =
    unsafe extern "C" fn(*mut c_void, bool, u32, *const c_void, usize, *mut c_void) -> i32;
/// `pw_loop_invoke`, exported from PipeWire 1.4. Before that it is a header
/// macro, the invoke queue has no lock in 0.3 and 1.0, and an invoke before
/// the loop is entered runs on the calling thread. Pausing is only used
/// with this symbol and the `pw_loop_iterate` path.
pub(super) type PwLoopInvokeFn = unsafe extern "C" fn(
    *mut c_void,
    SpaInvokeFn,
    u32,
    *const c_void,
    usize,
    bool,
    *mut c_void,
) -> i32;

/// One process-wide PipeWire library. Unloading it while a stream still
/// holds function pointers is a use-after-free, so a successful load stays.
pub(super) fn pipewire_library() -> Result<&'static libloading::Library, String> {
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

pub(super) struct Symbols {
    pub(super) init: unsafe extern "C" fn(*mut i32, *mut *mut *mut c_char),
    pub(super) main_loop_new: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    pub(super) main_loop_get_loop: unsafe extern "C" fn(*mut c_void) -> *mut c_void,
    pub(super) drive: LoopDrive,
    pub(super) quit: QuitFn,
    pub(super) main_loop_destroy: unsafe extern "C" fn(*mut c_void),
    pub(super) context_new: unsafe extern "C" fn(*mut c_void, *mut c_void, usize) -> *mut c_void,
    pub(super) context_destroy: unsafe extern "C" fn(*mut c_void),
    pub(super) connect_fd:
        unsafe extern "C" fn(*mut c_void, i32, *mut c_void, usize) -> *mut c_void,
    pub(super) properties_new_dict: unsafe extern "C" fn(*const c_void) -> *mut c_void,
    pub(super) properties_set:
        unsafe extern "C" fn(*mut c_void, *const c_char, *const c_char) -> i32,
    pub(super) stream_new:
        unsafe extern "C" fn(*mut c_void, *const c_char, *mut c_void) -> *mut c_void,
    pub(super) add_listener:
        unsafe extern "C" fn(*mut c_void, *mut c_void, *const PwStreamEvents, *mut c_void),
    pub(super) stream_connect:
        unsafe extern "C" fn(*mut c_void, i32, u32, u32, *const *const SpaPod, u32) -> i32,
    pub(super) update_params: UpdateFn,
    pub(super) dequeue: DequeueFn,
    pub(super) queue: QueueFn,
    pub(super) stream_destroy: unsafe extern "C" fn(*mut c_void),
    /// Set only when pausing is safe on this libpipewire ([`pause_symbols`]).
    pub(super) set_active: Option<SetActiveFn>,
    pub(super) loop_invoke: Option<PwLoopInvokeFn>,
}

/// Pausing needs `pw_stream_set_active` called on the loop thread. The
/// tracker queues it there with the exported `pw_loop_invoke` (1.4 on),
/// and only on the `pw_loop_iterate` path, where the loop is entered before
/// it is published. Anything else keeps the stream running as in rc.6.
pub(super) fn pause_symbols(
    path: LoopPath,
    set_active: Option<SetActiveFn>,
    invoke: Option<PwLoopInvokeFn>,
) -> (Option<SetActiveFn>, Option<PwLoopInvokeFn>) {
    match (path, set_active, invoke) {
        (LoopPath::Iterate, Some(set_active), Some(invoke)) => (Some(set_active), Some(invoke)),
        _ => (None, None),
    }
}

pub(super) type LoopEnterFn = unsafe extern "C" fn(*mut c_void);
pub(super) type LoopIterateFn = unsafe extern "C" fn(*mut c_void, i32) -> i32;
pub(super) type LoopLeaveFn = unsafe extern "C" fn(*mut c_void);
type MainLoopRunFn = unsafe extern "C" fn(*mut c_void) -> i32;

/// How the thread runs the PipeWire loop. `pw_loop_enter`, `pw_loop_iterate`,
/// and `pw_loop_leave` are exported from 1.4. On 1.0 and 1.2 they are macros,
/// so the thread uses `pw_main_loop_run` instead.
pub(super) enum LoopDrive {
    Iterate {
        enter: LoopEnterFn,
        iterate: LoopIterateFn,
        leave: LoopLeaveFn,
    },
    MainLoopRun(MainLoopRunFn),
}

/// Which loop path the loaded symbols can drive.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum LoopPath {
    Iterate,
    MainLoopRun,
}

pub(super) fn loop_path(
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

pub(super) fn loop_path_line(path: LoopPath) -> &'static str {
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

pub(super) fn load_symbols(lib: &libloading::Library) -> Result<Symbols, String> {
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

pub(super) unsafe fn load_optional<T>(lib: &libloading::Library, name: &[u8]) -> Option<T>
where
    T: Copy,
{
    let sym: libloading::Symbol<T> = unsafe { lib.get(name) }.ok()?;
    Some(*sym)
}
