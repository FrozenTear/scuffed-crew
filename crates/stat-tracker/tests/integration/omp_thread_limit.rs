//! `OMP_THREAD_LIMIT` on the real daemon binary (alpha.7 C1).
//!
//! libgomp reads the variable when the binary loads, so the daemon cannot
//! set it for itself: a start without it is replaced by the same binary with
//! it set. `--print-omp-limit` prints what the process ended up with and
//! exits before any daemon start-up, so those runs read no config and open
//! no store.

use crate::common;

use std::collections::HashMap;
use std::io::Read as _;
use std::os::fd::{AsRawFd as _, FromRawFd as _, OwnedFd};
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::Path;
use std::process::{Command, ExitStatus, Stdio};
use std::sync::OnceLock;
use std::time::{Duration, Instant};

const DAEMON: &str = env!("CARGO_BIN_EXE_scuffed-stat-tracker");
const LIMIT: &str = "OMP_THREAD_LIMIT";
const GUARD: &str = "SCUFFED_OMP_REEXEC";
const OPT_OUT: &str = "SCUFFED_OMP_NO_REEXEC";

/// What one short daemon run left behind.
struct Finished {
    pid: u32,
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

/// Start `cmd` and wait for it to exit. A restart loop would never exit, so
/// the run is killed after a timeout. Whatever was piped is read after the
/// exit; these runs print a few lines at most.
fn run_to_exit(mut cmd: Command) -> Finished {
    cmd.stdin(Stdio::null());
    // Killed and reaped when `daemon` drops, also when the timeout panics.
    let mut daemon = common::spawn_daemon(cmd);
    let pid = daemon.child.id();
    let start = Instant::now();
    let status = loop {
        if let Some(status) = daemon.child.try_wait().expect("wait") {
            break status;
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "the daemon did not exit: a restart loop?"
        );
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = String::new();
    if let Some(mut pipe) = daemon.child.stdout.take() {
        pipe.read_to_string(&mut stdout).expect("read stdout");
    }
    let mut stderr = String::new();
    if let Some(mut pipe) = daemon.child.stderr.take() {
        pipe.read_to_string(&mut stderr).expect("read stderr");
    }
    Finished {
        pid,
        status,
        stdout,
        stderr,
    }
}

/// Only `env` set, plus HOME and the XDG dirs in an empty temp directory:
/// if a restart ever lost the flag, what starts is a daemon with no config
/// and its own data dir, not one that reads this machine's.
fn isolate(cmd: &mut Command, root: &Path, env: &[(&str, &str)]) {
    cmd.env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", root.join("home"))
        .env("XDG_CONFIG_HOME", root.join("config"))
        .env("XDG_DATA_HOME", root.join("data"))
        .env("XDG_CACHE_HOME", root.join("cache"))
        .env("NO_COLOR", "1");
    for (key, value) in env {
        cmd.env(key, value);
    }
}

struct Printed {
    pid: u32,
    lines: HashMap<String, String>,
}

impl Printed {
    fn get(&self, key: &str) -> &str {
        self.lines
            .get(key)
            .unwrap_or_else(|| panic!("no {key} line in {:?}", self.lines))
    }
}

/// Run `cmd` (a daemon start with `--print-omp-limit`) and parse its lines.
fn printed_by(mut cmd: Command, env: &[(&str, &str)]) -> Printed {
    let root = tempfile::tempdir().unwrap();
    isolate(&mut cmd, root.path(), env);
    cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
    let run = run_to_exit(cmd);
    assert!(
        run.status.success(),
        "{env:?}: {:?}\n{}\n{}",
        run.status,
        run.stdout,
        run.stderr
    );
    let lines = run
        .stdout
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    Printed {
        pid: run.pid,
        lines,
    }
}

/// The daemon with `args` before `--print-omp-limit`, and only `env` set.
fn print_limit_with(args: &[&str], env: &[(&str, &str)]) -> Printed {
    let mut cmd = Command::new(DAEMON);
    cmd.args(args).arg("--print-omp-limit");
    printed_by(cmd, env)
}

fn print_limit(env: &[(&str, &str)]) -> Printed {
    print_limit_with(&[], env)
}

/// Whether the daemon binary loads an OpenMP runtime. The answer comes from
/// the dynamic loader (this is what `ldd` runs), not from the daemon's own
/// code: with `LD_TRACE_LOADED_OBJECTS` the loader lists the libraries and
/// exits without starting the program.
fn daemon_loads_openmp() -> bool {
    static LOADS: OnceLock<bool> = OnceLock::new();
    *LOADS.get_or_init(|| {
        let root = tempfile::tempdir().unwrap();
        let mut cmd = Command::new(DAEMON);
        isolate(&mut cmd, root.path(), &[("LD_TRACE_LOADED_OBJECTS", "1")]);
        cmd.stdout(Stdio::piped()).stderr(Stdio::piped());
        let run = run_to_exit(cmd);
        assert!(
            run.status.success() && run.stdout.contains("libc.so"),
            "not a loader listing: {:?}\n{}\n{}",
            run.status,
            run.stdout,
            run.stderr
        );
        ["libgomp.so", "libomp.so", "libiomp5.so"]
            .iter()
            .any(|lib| run.stdout.contains(lib))
    })
}

/// What libgomp reports. When the loader maps an OpenMP runtime into the
/// daemon (every build we ship: libtesseract needs libgomp), the daemon must
/// report that runtime's number; `no-openmp` there would mean the lookup is
/// broken. Only a daemon with no OpenMP runtime at all may say `no-openmp`,
/// and then it must.
fn assert_libgomp(printed: &Printed, want: &str) {
    let want = if daemon_loads_openmp() {
        want
    } else {
        "no-openmp"
    };
    assert_eq!(printed.get("libgomp_thread_limit"), want);
}

#[test]
fn a_start_without_the_limit_restarts_once_with_it() {
    let printed = print_limit(&[]);
    assert_eq!(printed.get(LIMIT), "1");
    assert_eq!(printed.get(GUARD), "1");
    // libgomp saw the variable when it loaded. A value set after the start
    // (`set_var` in `main`) would print 1 above and `none` here.
    assert_libgomp(&printed, "1");
    // Same process: the pid the starter holds (systemd, the app) is the
    // daemon's, and the kernel still names it as the tracker.
    assert_eq!(printed.get("pid"), printed.pid.to_string());
    assert_eq!(printed.get("comm"), "scuffed-stat-tr");
}

#[test]
fn a_value_the_user_set_is_kept_and_there_is_no_restart() {
    for chosen in ["4", "1"] {
        let printed = print_limit(&[(LIMIT, chosen)]);
        assert_eq!(printed.get(LIMIT), chosen);
        assert_eq!(printed.get(GUARD), "unset", "restarted with {chosen}");
        assert_libgomp(&printed, chosen);
    }
}

#[test]
fn the_guard_stops_a_second_restart() {
    // As if a restart had lost the limit: the process must run on as it is,
    // not start itself again.
    let printed = print_limit(&[(GUARD, "1")]);
    assert_eq!(printed.get(LIMIT), "unset");
    assert_eq!(printed.get(GUARD), "1");
    assert_libgomp(&printed, "none");
    assert_eq!(printed.get("pid"), printed.pid.to_string());
}

#[test]
fn the_opt_out_keeps_the_process_as_started() {
    let printed = print_limit(&[(OPT_OUT, "1")]);
    assert_eq!(printed.get(LIMIT), "unset");
    assert_eq!(printed.get(GUARD), "unset");
    assert_libgomp(&printed, "none");
}

#[test]
fn a_training_run_does_not_restart() {
    // The release job runs `--generate-tessdata`: its training tools must
    // not inherit the limit. The print flag exits before any training.
    let printed = print_limit_with(&["--generate-tessdata"], &[]);
    assert_eq!(printed.get(LIMIT), "unset");
    assert_eq!(printed.get(GUARD), "unset");
    assert_libgomp(&printed, "none");
}

// ── a restart that cannot happen ────────────────────────────────────────────

/// A daemon command whose own path is gone by the time it looks.
///
/// The binary is hard-linked next to itself, the link is opened and then
/// removed, and the daemon is started from the open descriptor. Its
/// `/proc/self/exe` then reads `<link> (deleted)`, which is not a file, so
/// the restart fails every time and with no race.
fn daemon_with_no_path() -> Command {
    let dir = tempfile::tempdir_in(Path::new(DAEMON).parent().unwrap()).unwrap();
    let link = dir.path().join("scuffed-stat-tracker");
    std::fs::hard_link(DAEMON, &link).unwrap();
    let file = std::fs::File::open(&link).unwrap();
    // SAFETY: `dup` on a descriptor this function owns. The copy has
    // close-on-exec off, so the child still has it when it execs. It stays
    // open in this test process until the process ends.
    let fd = unsafe { libc::dup(file.as_raw_fd()) };
    assert!(fd >= 3);
    drop(dir);
    assert!(!link.exists());
    Command::new(format!("/proc/self/fd/{fd}"))
}

#[test]
fn a_failed_restart_carries_on_without_the_limit() {
    let mut cmd = daemon_with_no_path();
    cmd.arg("--print-omp-limit");
    let printed = printed_by(cmd, &[]);
    // It tried (nothing told it not to), could not, and ran on as started.
    assert_eq!(printed.get(LIMIT), "unset");
    assert_eq!(printed.get(GUARD), "unset");
    assert_libgomp(&printed, "none");
    assert_eq!(printed.get("pid"), printed.pid.to_string());
}

/// Run `cmd` with a stdout nobody reads: the read end of the pipe is closed
/// before the daemon starts, so its first print gets EPIPE or SIGPIPE.
fn run_with_closed_stdout(mut cmd: Command) -> Finished {
    let root = tempfile::tempdir().unwrap();
    isolate(&mut cmd, root.path(), &[]);
    let mut ends = [0; 2];
    // SAFETY: `pipe2` writes two descriptors into the array.
    assert_eq!(
        unsafe { libc::pipe2(ends.as_mut_ptr(), libc::O_CLOEXEC) },
        0
    );
    // SAFETY: both descriptors were just created and are owned here only.
    let (read, write) = unsafe { (OwnedFd::from_raw_fd(ends[0]), OwnedFd::from_raw_fd(ends[1])) };
    drop(read);
    cmd.arg("--print-omp-limit")
        .stdout(Stdio::from(write))
        .stderr(Stdio::piped());
    run_to_exit(cmd)
}

#[test]
fn a_failed_restart_keeps_sigpipe_ignored() {
    // What a healthy daemon does when its stdout is gone: the write fails,
    // `print!` panics, exit code 101. Rust ignores SIGPIPE at start-up.
    let healthy = run_with_closed_stdout(Command::new(DAEMON));
    assert_eq!(
        (healthy.status.signal(), healthy.status.code()),
        (None, Some(101)),
        "{}",
        healthy.stderr
    );
    assert!(healthy.stderr.contains("Broken pipe"), "{}", healthy.stderr);

    // `Command::exec` resets SIGPIPE before execve. After a restart that
    // failed, the daemon must be back to ignoring it, not die of the signal.
    let failed = run_with_closed_stdout(daemon_with_no_path());
    assert_eq!(
        (failed.status.signal(), failed.status.code()),
        (None, Some(101)),
        "{}",
        failed.stderr
    );
    assert!(failed.stderr.contains("Broken pipe"), "{}", failed.stderr);
}

// ── a started daemon never outlives the process that started it ─────────────

const STARTER_ROOT: &str = "SCUFFED_TEST_DAEMON_STARTER_ROOT";
const STARTER_FN: &str = "starter_half_starts_a_daemon_and_waits";
const STARTER_ARGV0: &str = "tracker-under-test";
const HELD_FILE: &str = "held.lock";

fn wait_for(what: &str, timeout: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// `pid` is a live process running the daemon binary. A zombie has no
/// `exe` link, so a killed daemon that is not reaped yet reads as gone.
fn is_live_daemon(pid: u32) -> bool {
    let daemon = std::fs::canonicalize(DAEMON).unwrap();
    std::fs::read_link(format!("/proc/{pid}/exe")).is_ok_and(|exe| exe == daemon)
}

/// Kills the daemon if it is still there when the test ends, pass or fail.
struct Sweep(u32);

impl Drop for Sweep {
    fn drop(&mut self) {
        if is_live_daemon(self.0) {
            // SAFETY: plain kill(2) on a pid just checked to be the daemon.
            unsafe { libc::kill(self.0 as i32, libc::SIGKILL) };
        }
    }
}

/// Not a test on its own: the test below runs this test binary again with
/// [`STARTER_ROOT`] set, as the process that starts a daemon and is then
/// killed. It starts a full daemon (no flag) in the empty directory it was
/// given, writes the pid there, and waits to be killed.
#[test]
#[ignore = "the starter half of a_daemon_dies_with_the_process_that_started_it"]
fn starter_half_starts_a_daemon_and_waits() {
    let Some(root) = std::env::var_os(STARTER_ROOT) else {
        return;
    };
    let root = Path::new(&root);
    let log = std::fs::File::create(root.join("daemon.out")).unwrap();
    let mut cmd = Command::new(DAEMON);
    isolate(&mut cmd, root, &[]);
    // A name of its own, to see that the restart passes argv[0] on.
    cmd.arg0(STARTER_ARGV0)
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log));
    // A descriptor a child would inherit, like the lock a build wrapper
    // holds open (`dup` leaves close-on-exec off). The daemon must not get it.
    let held = std::fs::File::create(root.join(HELD_FILE)).unwrap();
    // SAFETY: `dup` on a descriptor this function owns.
    assert!(unsafe { libc::dup(held.as_raw_fd()) } >= 3);
    let daemon = common::spawn_daemon(cmd);
    // Written under another name first, so the reader never sees half a pid.
    std::fs::write(root.join("daemon.pid.tmp"), daemon.child.id().to_string()).unwrap();
    std::fs::rename(root.join("daemon.pid.tmp"), root.join("daemon.pid")).unwrap();
    // Until the other half kills this process. If it never does, this ends
    // on its own and dropping `daemon` kills the daemon.
    std::thread::sleep(Duration::from_secs(120));
}

/// A test binary that is killed runs no destructor, so [`common::Daemon`]
/// cannot clean up. The daemon was started with a parent-death signal for
/// that case. This checks the signal still holds after the daemon has
/// replaced itself to pick up `OMP_THREAD_LIMIT`.
#[test]
fn a_daemon_dies_with_the_process_that_started_it() {
    let root = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(std::env::current_exe().unwrap());
    let starter_test = common::test_name(module_path!(), STARTER_FN);
    cmd.args(["--exact", &starter_test, "--ignored", "--nocapture"])
        .env(STARTER_ROOT, root.path())
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null());
    // The starter gets the same protections, so it cannot outlive this test.
    let mut starter = common::spawn_daemon(cmd);

    let pid_file = root.path().join("daemon.pid");
    wait_for("the daemon's pid", Duration::from_secs(60), || {
        pid_file.exists()
    });
    let pid: u32 = std::fs::read_to_string(&pid_file).unwrap().parse().unwrap();
    let _sweep = Sweep(pid);

    // run.marker is written during start-up, after the restart. Waiting for
    // it means the daemon killed below has already replaced itself.
    let marker = root
        .path()
        .join("data")
        .join("scuffed-stat-tracker")
        .join("run.marker");
    let ours = format!("pid={pid}\n");
    wait_for("the daemon's run.marker", Duration::from_secs(60), || {
        std::fs::read_to_string(&marker).is_ok_and(|m| m.starts_with(&ours))
    });
    let environ = std::fs::read(format!("/proc/{pid}/environ")).unwrap();
    assert!(
        environ
            .split(|b| *b == 0)
            .any(|var| var == b"SCUFFED_OMP_REEXEC=1"),
        "the daemon did not restart itself"
    );
    assert!(is_live_daemon(pid));

    // The restart kept the name the daemon was started under.
    let cmdline = std::fs::read(format!("/proc/{pid}/cmdline")).unwrap();
    assert_eq!(
        String::from_utf8_lossy(cmdline.split(|b| *b == 0).next().unwrap()),
        STARTER_ARGV0
    );

    // The starter held a file open on a descriptor children inherit. The
    // daemon has only what it was given: its log on stdout and stderr.
    // The kernel lists resolved paths.
    let real_root = root.path().canonicalize().unwrap();
    let held = real_root.join(HELD_FILE);
    assert!(held.exists(), "the starter did not open {held:?}");
    let open: Vec<_> = std::fs::read_dir(format!("/proc/{pid}/fd"))
        .unwrap()
        .filter_map(|entry| std::fs::read_link(entry.ok()?.path()).ok())
        .collect();
    assert!(
        open.contains(&real_root.join("daemon.out")),
        "the fd listing does not show the daemon's own log: {open:?}"
    );
    assert!(
        !open.contains(&held),
        "the daemon inherited a descriptor it was not given: {open:?}"
    );

    // SIGKILL: nothing in the starter gets to run.
    starter.child.kill().unwrap();
    starter.child.wait().unwrap();
    let start = Instant::now();
    while is_live_daemon(pid) && start.elapsed() < Duration::from_secs(10) {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert!(
        !is_live_daemon(pid),
        "the daemon outlived the process that started it"
    );
}
