//! `OMP_THREAD_LIMIT` on the real daemon binary (alpha.7 C1).
//!
//! libgomp reads the variable when the binary loads, so the daemon cannot
//! set it for itself: a start without it is replaced by the same binary with
//! it set. `--print-omp-limit` prints what the process ended up with and
//! exits before any daemon start-up, so these runs read no config and open
//! no store.

use std::collections::HashMap;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

const LIMIT: &str = "OMP_THREAD_LIMIT";
const GUARD: &str = "SCUFFED_OMP_REEXEC";
const OPT_OUT: &str = "SCUFFED_OMP_NO_REEXEC";

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

/// Run the daemon with `--print-omp-limit` and only `env` set. A restart
/// loop would never print, so the run is killed after a timeout.
///
/// HOME and the XDG dirs are an empty temp directory: if a restart ever lost
/// the flag, what starts is a daemon with no config and its own data dir,
/// not one that reads this machine's.
fn print_limit(env: &[(&str, &str)]) -> Printed {
    let root = tempfile::tempdir().unwrap();
    let mut cmd = Command::new(env!("CARGO_BIN_EXE_scuffed-stat-tracker"));
    cmd.arg("--print-omp-limit")
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", root.path().join("home"))
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("XDG_CACHE_HOME", root.path().join("cache"))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    for (key, value) in env {
        cmd.env(key, value);
    }
    let mut child = cmd.spawn().expect("spawn daemon");
    let pid = child.id();
    let start = Instant::now();
    while child.try_wait().expect("wait").is_none() {
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon did not exit with {env:?}: a restart loop?");
        }
        std::thread::sleep(Duration::from_millis(20));
    }
    let out = child.wait_with_output().expect("output");
    assert!(out.status.success(), "{env:?}: {out:?}");
    let lines = String::from_utf8_lossy(&out.stdout)
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(key, value)| (key.to_string(), value.to_string()))
        .collect();
    Printed { pid, lines }
}

/// What libgomp reports. A Tesseract built without OpenMP loads no libgomp,
/// and then there is nothing to limit.
fn assert_libgomp(printed: &Printed, want: &str) {
    let got = printed.get("libgomp_thread_limit");
    assert!(
        got == want || got == "no-openmp",
        "libgomp reports {got}, want {want}"
    );
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
