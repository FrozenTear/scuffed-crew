//! glibc allocator limits for the long-running daemon.
//!
//! The daemon's resident memory used to climb for days with nothing leaking:
//! freed memory stayed in glibc arenas. Every thread can get its own arena
//! (up to 8 per core), and a 10 MB snapshot export or a 15 MB frame copy
//! leaves its pages behind in whichever arena served it. Measured on a copy
//! of a real store (300 snapshot exports, no play): default glibc grew from
//! 306 MB to 1,065 MB; the same binary with the three limits below stayed at
//! 345 to 363 MB.
//!
//! [`apply`] runs once at the top of `main`, before the async runtime starts
//! any thread. [`trim_loop`] hands free pages back every [`TRIM_EVERY`] and
//! logs the live heap, for the long-run memory sampler.
//!
//! A limit the user already set (the `MALLOC_*` variable or the matching
//! `GLIBC_TUNABLES` entry) wins and is left alone. `STAT_TRACKER_MEMTUNE=0`
//! turns all of this off, for before/after measurements.

use std::time::Duration;

/// At most this many malloc arenas for all threads.
pub(crate) const ARENA_MAX: i32 = 2;
/// Blocks this size and larger get their own mapping, given back on free.
/// Setting it also turns off glibc's dynamic threshold, which otherwise
/// rises to 32 MiB after the first large free.
pub(crate) const MMAP_THRESHOLD: i32 = 1024 * 1024;
/// Free space at the top of the heap above this goes back to the system.
pub(crate) const TRIM_THRESHOLD: i32 = 4 * 1024 * 1024;
/// How often [`trim_loop`] runs `malloc_trim`.
pub(crate) const TRIM_EVERY: Duration = Duration::from_secs(5 * 60);

/// One allocator limit: the `mallopt` parameter and the two ways a user can
/// set the same limit from the environment.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct Limit {
    pub(crate) name: &'static str,
    pub(crate) env: &'static str,
    pub(crate) tunable: &'static str,
    pub(crate) value: i32,
}

pub(crate) const LIMITS: [Limit; 3] = [
    Limit {
        name: "arena_max",
        env: "MALLOC_ARENA_MAX",
        tunable: "glibc.malloc.arena_max",
        value: ARENA_MAX,
    },
    Limit {
        name: "mmap_threshold",
        env: "MALLOC_MMAP_THRESHOLD_",
        tunable: "glibc.malloc.mmap_threshold",
        value: MMAP_THRESHOLD,
    },
    Limit {
        name: "trim_threshold",
        env: "MALLOC_TRIM_THRESHOLD_",
        tunable: "glibc.malloc.trim_threshold",
        value: TRIM_THRESHOLD,
    },
];

/// What [`apply`] did, for the startup log line.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct Applied {
    /// Off by `STAT_TRACKER_MEMTUNE=0`, or not a glibc build.
    pub(crate) disabled: bool,
    pub(crate) set: Vec<&'static str>,
    /// Left to the user's own setting.
    pub(crate) user: Vec<&'static str>,
}

/// Limits to set, given the environment. Pure, for the tests.
pub(crate) fn plan(get: impl Fn(&str) -> Option<String>) -> Applied {
    if get("STAT_TRACKER_MEMTUNE").is_some_and(|v| v.trim() == "0") {
        return Applied {
            disabled: true,
            ..Applied::default()
        };
    }
    let tunables = get("GLIBC_TUNABLES").unwrap_or_default();
    let mut out = Applied::default();
    for limit in LIMITS {
        let by_env = get(limit.env).is_some_and(|v| !v.trim().is_empty());
        let by_tunable = tunables
            .split(':')
            .any(|t| t.split('=').next() == Some(limit.tunable));
        if by_env || by_tunable {
            out.user.push(limit.name);
        } else {
            out.set.push(limit.name);
        }
    }
    out
}

static APPLIED: std::sync::OnceLock<Applied> = std::sync::OnceLock::new();

/// Set the limits. Call before any thread starts.
pub(crate) fn apply() -> Applied {
    let applied = apply_limits();
    let _ = APPLIED.set(applied.clone());
    applied
}

/// The startup log line for what [`apply`] did.
pub(crate) fn log_startup() {
    match APPLIED.get() {
        Some(a) if !a.disabled => tracing::info!(
            target: "scuffed_stat_tracker",
            set = ?a.set,
            left_to_user = ?a.user,
            arena_max = ARENA_MAX,
            mmap_threshold = MMAP_THRESHOLD,
            trim_threshold = TRIM_THRESHOLD,
            "allocator limits"
        ),
        _ => tracing::info!(target: "scuffed_stat_tracker", "allocator limits off"),
    }
}

#[cfg(all(target_os = "linux", target_env = "gnu"))]
fn apply_limits() -> Applied {
    let applied = plan(|k| std::env::var(k).ok());
    if !applied.disabled {
        for limit in LIMITS.iter().filter(|l| applied.set.contains(&l.name)) {
            let param = match limit.name {
                "arena_max" => libc::M_ARENA_MAX,
                "mmap_threshold" => libc::M_MMAP_THRESHOLD,
                _ => libc::M_TRIM_THRESHOLD,
            };
            // SAFETY: mallopt only changes allocator parameters; no thread
            // other than this one exists yet.
            unsafe { libc::mallopt(param, limit.value) };
        }
    }
    applied
}

#[cfg(not(all(target_os = "linux", target_env = "gnu")))]
fn apply_limits() -> Applied {
    Applied {
        disabled: true,
        ..Applied::default()
    }
}

/// Bytes in use on the malloc heap (`mallinfo2`: arena plus mmapped blocks).
pub(crate) fn live_heap_bytes() -> Option<u64> {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    {
        // SAFETY: mallinfo2 only reads allocator statistics.
        let m = unsafe { libc::mallinfo2() };
        Some((m.uordblks + m.hblkhd) as u64)
    }
    #[cfg(not(all(target_os = "linux", target_env = "gnu")))]
    None
}

/// Resident set size from `/proc/self/statm`.
pub(crate) fn resident_bytes() -> Option<u64> {
    let statm = std::fs::read_to_string("/proc/self/statm").ok()?;
    let pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    Some(pages * 4096)
}

/// Give free heap pages back to the system.
pub(crate) fn trim() {
    #[cfg(all(target_os = "linux", target_env = "gnu"))]
    // SAFETY: malloc_trim is thread safe.
    unsafe {
        libc::malloc_trim(0);
    }
}

/// Every [`TRIM_EVERY`]: trim, then log the live heap and RSS (info, one
/// line per 5 minutes: the long-run memory sampler reads it).
/// Runs for the daemon's lifetime.
pub(crate) async fn trim_loop() {
    let mut tick = tokio::time::interval(TRIM_EVERY);
    tick.tick().await;
    loop {
        tick.tick().await;
        let _ = tokio::task::spawn_blocking(trim).await;
        tracing::info!(
            target: "scuffed_stat_tracker",
            live_heap_bytes = live_heap_bytes(),
            resident_bytes = resident_bytes(),
            "memory"
        );
    }
}
