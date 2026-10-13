//! One OpenMP thread per Tesseract call.
//!
//! libtesseract is built with OpenMP. With no limit, each OCR call fans out
//! onto a libgomp team that spins while it waits, on top of the `ocr-N`
//! workers that already run the fields in parallel (`ocr::OCR_POOL`). With
//! `OMP_THREAD_LIMIT=1` the reads are the same, text and confidence, and the
//! CPU time is about a third.
//!
//! libgomp reads `OMP_*` in its library constructor, before `main`. Setting
//! the variable from inside the process is too late: it has to be in the
//! environment at exec. The systemd unit sets it. For every other start (a
//! shell, the app's Start button, a dev run) [`ensure_thread_limit`] runs
//! first in `main` and starts the same binary again with the variable set.

use std::ffi::{OsStr, OsString};

/// The variable libgomp reads.
pub const LIMIT_VAR: &str = "OMP_THREAD_LIMIT";
/// What the unit and the restart set it to.
pub const LIMIT_VALUE: &str = "1";
/// Set on the restarted process. A process that sees it never restarts again.
pub const REEXEC_VAR: &str = "SCUFFED_OMP_REEXEC";
/// Developer opt-out: any value but empty or `0` keeps the process as started.
pub const OPT_OUT_VAR: &str = "SCUFFED_OMP_NO_REEXEC";
/// Prints the limit this process runs with and exits.
pub const PRINT_FLAG: &str = "--print-omp-limit";
/// Trains the game font with Tesseract's own tools and does no OCR here.
const TRAINING_FLAG: &str = "--generate-tessdata";

/// What the start-up check reads.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct StartEnv<'a> {
    /// [`LIMIT_VAR`].
    pub limit: Option<&'a OsStr>,
    /// [`REEXEC_VAR`].
    pub reexec: Option<&'a OsStr>,
    /// [`OPT_OUT_VAR`].
    pub opt_out: Option<&'a OsStr>,
    /// The command line holds [`TRAINING_FLAG`].
    pub training_only: bool,
}

/// Why the process keeps the environment it was started with.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kept {
    /// [`LIMIT_VAR`] already has a value: the unit's, this binary's own
    /// restart, or one the user chose. It is never replaced.
    LimitSet,
    /// [`REEXEC_VAR`] is set and the limit is not. This process is already a
    /// restart, so it does not try again.
    AlreadyRestarted,
    /// [`OPT_OUT_VAR`] asked for no restart.
    OptedOut,
    /// A training run. The training tools it starts would inherit the limit
    /// and train on one thread.
    TrainingOnly,
}

/// Outcome of [`decide`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Decision {
    /// Start the binary again with [`LIMIT_VAR`] and [`REEXEC_VAR`] set.
    Reexec,
    Keep(Kept),
}

fn has_value(var: Option<&OsStr>) -> bool {
    var.is_some_and(|v| !v.is_empty())
}

/// Whether to start the binary again. An empty [`LIMIT_VAR`] counts as unset
/// (libgomp ignores it too). [`REEXEC_VAR`] counts when it is present at all,
/// so nothing a caller puts in it can make a loop.
pub fn decide(env: StartEnv<'_>) -> Decision {
    if has_value(env.limit) {
        return Decision::Keep(Kept::LimitSet);
    }
    if env.reexec.is_some() {
        return Decision::Keep(Kept::AlreadyRestarted);
    }
    if has_value(env.opt_out) && env.opt_out != Some(OsStr::new("0")) {
        return Decision::Keep(Kept::OptedOut);
    }
    if env.training_only {
        return Decision::Keep(Kept::TrainingOnly);
    }
    Decision::Reexec
}

/// What [`ensure_thread_limit`] did, for the start-up log line.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Startup {
    Kept(Kept),
    /// The restart was tried and did not happen. The process runs on with no
    /// limit.
    ReexecFailed(String),
}

/// Call first in `main`, before any thread, log line or Tesseract use.
///
/// When [`decide`] says so, this replaces the process with the same binary,
/// same arguments and same pid, plus [`LIMIT_VAR`]`=1` and [`REEXEC_VAR`]. It
/// then does not return. In every other case, and when the restart fails, it
/// returns and the caller carries on.
pub fn ensure_thread_limit() -> Startup {
    let args: Vec<OsString> = std::env::args_os().collect();
    let limit = std::env::var_os(LIMIT_VAR);
    let reexec = std::env::var_os(REEXEC_VAR);
    let opt_out = std::env::var_os(OPT_OUT_VAR);
    let decision = decide(StartEnv {
        limit: limit.as_deref(),
        reexec: reexec.as_deref(),
        opt_out: opt_out.as_deref(),
        training_only: args.iter().skip(1).any(|a| a == TRAINING_FLAG),
    });
    match decision {
        Decision::Keep(kept) => Startup::Kept(kept),
        Decision::Reexec => Startup::ReexecFailed(reexec_self(&args)),
    }
}

/// Replace this process. Returns only on failure, with the reason.
///
/// The path is the resolved binary, not `/proc/self/exe`: the kernel names
/// the process after the last part of the path it was given, and a process
/// called `exe` is not one the app or `proc_id` would take for the tracker.
#[cfg(unix)]
fn reexec_self(args: &[OsString]) -> String {
    use std::os::unix::process::CommandExt;
    let exe = match std::env::current_exe() {
        Ok(exe) => exe,
        Err(e) => return format!("could not find this binary: {e}"),
    };
    let mut cmd = std::process::Command::new(&exe);
    if let Some(arg0) = args.first() {
        cmd.arg0(arg0);
    }
    cmd.args(args.iter().skip(1))
        .env(LIMIT_VAR, LIMIT_VALUE)
        .env(REEXEC_VAR, "1");
    cmd.exec().to_string()
}

#[cfg(not(unix))]
fn reexec_self(_args: &[OsString]) -> String {
    "not supported on this platform".to_string()
}

/// [`LIMIT_VAR`] as this process has it, or `None` when it is unset or empty.
pub fn env_limit() -> Option<String> {
    std::env::var_os(LIMIT_VAR)
        .filter(|v| !v.is_empty())
        .map(|v| v.to_string_lossy().into_owned())
}

/// [`env_limit`] for a log field or a printed line.
pub fn env_limit_label() -> String {
    env_limit().unwrap_or_else(|| "unset".to_string())
}

/// The thread limit libgomp itself reports, or `None` when no OpenMP runtime
/// is loaded (a Tesseract built without OpenMP). With no limit set, libgomp
/// reports `i32::MAX`.
pub fn runtime_limit() -> Option<i32> {
    #[cfg(unix)]
    {
        // SAFETY: `dlsym` only looks the name up in the libraries already
        // loaded; the name is a NUL-terminated literal.
        let sym = unsafe { libc::dlsym(libc::RTLD_DEFAULT, c"omp_get_thread_limit".as_ptr()) };
        if sym.is_null() {
            return None;
        }
        // SAFETY: the OpenMP API declares `int omp_get_thread_limit(void)`.
        // It takes no argument and only reads a value fixed at load time.
        let get: unsafe extern "C" fn() -> libc::c_int = unsafe { std::mem::transmute(sym) };
        // SAFETY: see above; the symbol is that function.
        Some(unsafe { get() })
    }
    #[cfg(not(unix))]
    None
}

/// [`runtime_limit`] for a log field or a printed line.
pub fn runtime_limit_label() -> String {
    match runtime_limit() {
        Some(i32::MAX) => "none".to_string(),
        Some(n) => n.to_string(),
        None => "no-openmp".to_string(),
    }
}

/// `Threads:` from the text of `/proc/<pid>/status`.
pub fn parse_thread_count(status: &str) -> Option<u32> {
    status
        .lines()
        .find_map(|line| line.strip_prefix("Threads:"))
        .and_then(|rest| rest.trim().parse().ok())
}

/// OS threads of this process right now.
pub fn os_threads() -> Option<u32> {
    parse_thread_count(&std::fs::read_to_string("/proc/self/status").ok()?)
}

/// [`os_threads`] for a log field.
pub fn os_threads_label() -> String {
    os_threads().map_or_else(|| "unknown".to_string(), |n| n.to_string())
}

/// The lines [`PRINT_FLAG`] prints.
pub fn print_report() -> String {
    let comm = std::fs::read_to_string("/proc/self/comm").unwrap_or_default();
    format!(
        "{LIMIT_VAR}={}\n{REEXEC_VAR}={}\nlibgomp_thread_limit={}\ncomm={}\npid={}\n",
        env_limit_label(),
        std::env::var_os(REEXEC_VAR)
            .map_or_else(|| "unset".to_string(), |v| v.to_string_lossy().into_owned()),
        runtime_limit_label(),
        comm.trim_end(),
        std::process::id(),
    )
}

/// One line at start-up: the limit in force and how it got there.
pub fn log_startup(startup: &Startup) {
    let limit = env_limit_label();
    let libgomp = runtime_limit_label();
    match startup {
        Startup::Kept(Kept::LimitSet) => {
            let restarted = std::env::var_os(REEXEC_VAR).is_some();
            tracing::info!(
                omp_thread_limit = %limit,
                libgomp_thread_limit = %libgomp,
                restarted,
                "OpenMP thread limit for OCR"
            );
        }
        Startup::Kept(kept) => {
            tracing::info!(
                omp_thread_limit = %limit,
                libgomp_thread_limit = %libgomp,
                reason = ?kept,
                "no OpenMP thread limit for OCR; started without {LIMIT_VAR}"
            );
        }
        Startup::ReexecFailed(error) => {
            tracing::warn!(
                omp_thread_limit = %limit,
                libgomp_thread_limit = %libgomp,
                error = %error,
                "could not restart with {LIMIT_VAR}={LIMIT_VALUE}; OCR will use more CPU for the same reads"
            );
        }
    }
}

/// Log the thread count once, after the first scoreboard OCR: by then the
/// `ocr-N` workers exist and any OpenMP team would too.
pub fn log_threads_after_first_ocr() {
    static LOGGED: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);
    if LOGGED.swap(true, std::sync::atomic::Ordering::Relaxed) {
        return;
    }
    tracing::info!(
        os_threads = %os_threads_label(),
        omp_thread_limit = %env_limit_label(),
        "threads after the first scoreboard OCR"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    fn env<'a>(
        limit: Option<&'a str>,
        reexec: Option<&'a str>,
        opt_out: Option<&'a str>,
    ) -> StartEnv<'a> {
        StartEnv {
            limit: limit.map(OsStr::new),
            reexec: reexec.map(OsStr::new),
            opt_out: opt_out.map(OsStr::new),
            training_only: false,
        }
    }

    #[test]
    fn a_bare_start_restarts_with_the_limit() {
        assert_eq!(decide(env(None, None, None)), Decision::Reexec);
        // libgomp ignores an empty value, so it is not a choice to keep.
        assert_eq!(decide(env(Some(""), None, None)), Decision::Reexec);
        // "0" and an empty opt-out do not opt out.
        assert_eq!(decide(env(None, None, Some("0"))), Decision::Reexec);
        assert_eq!(decide(env(None, None, Some(""))), Decision::Reexec);
    }

    #[test]
    fn a_limit_that_is_already_set_is_kept() {
        // The unit's value, and this binary's own restart.
        assert_eq!(
            decide(env(Some("1"), None, None)),
            Decision::Keep(Kept::LimitSet)
        );
        assert_eq!(
            decide(env(Some("1"), Some("1"), None)),
            Decision::Keep(Kept::LimitSet)
        );
        // A value the user chose is not replaced, whatever it is.
        for chosen in ["4", "16", "0", "many"] {
            assert_eq!(
                decide(env(Some(chosen), None, None)),
                Decision::Keep(Kept::LimitSet),
                "{chosen}"
            );
        }
    }

    #[test]
    fn the_guard_stops_a_second_restart() {
        // The limit did not survive the restart: do not try again.
        assert_eq!(
            decide(env(None, Some("1"), None)),
            Decision::Keep(Kept::AlreadyRestarted)
        );
        // Any content counts, also empty and "0".
        for guard in ["", "0", "no"] {
            assert_eq!(
                decide(env(None, Some(guard), None)),
                Decision::Keep(Kept::AlreadyRestarted),
                "{guard:?}"
            );
            assert_eq!(
                decide(env(Some(""), Some(guard), None)),
                Decision::Keep(Kept::AlreadyRestarted),
                "{guard:?}"
            );
        }
    }

    #[test]
    fn the_opt_out_and_a_training_run_keep_the_process() {
        assert_eq!(
            decide(env(None, None, Some("1"))),
            Decision::Keep(Kept::OptedOut)
        );
        let training = StartEnv {
            training_only: true,
            ..env(None, None, None)
        };
        assert_eq!(decide(training), Decision::Keep(Kept::TrainingOnly));
    }

    #[test]
    fn thread_count_comes_from_the_threads_line() {
        let status =
            "Name:\tscuffed-stat-tr\nTgid:\t4242\nThreads:\t5\nvoluntary_ctxt_switches:\t9\n";
        assert_eq!(parse_thread_count(status), Some(5));
        assert_eq!(parse_thread_count("Name:\tx\nTgid:\t1\n"), None);
        assert_eq!(parse_thread_count("Threads:\tmany\n"), None);
        assert!(os_threads().is_some_and(|n| n >= 1));
    }
}
