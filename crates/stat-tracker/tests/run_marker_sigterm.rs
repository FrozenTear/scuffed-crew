//! Crash marker on the real daemon binary (report-bundle spec section 9):
//! a stale `run.marker` (dead pid) becomes `last_crash.marker` with one WARN
//! line, the running daemon writes its own `run.marker`, and SIGTERM removes
//! it (and `daemon.pid`).
//!
//! The daemon runs with an empty HOME and XDG dirs in a temp directory and
//! no display, so it reads no real config, holds no sync token, and captures
//! nothing.

use std::path::Path;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

fn wait_for(what: &str, timeout: Duration, mut ok: impl FnMut() -> bool) {
    let start = Instant::now();
    while !ok() {
        assert!(start.elapsed() < timeout, "timed out waiting for {what}");
        std::thread::sleep(Duration::from_millis(50));
    }
}

fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_default()
}

#[test]
fn daemon_moves_a_stale_marker_and_sigterm_removes_its_own() {
    let root = tempfile::tempdir().unwrap();
    let home = root.path().join("home");
    let data_dir = root.path().join("data").join("scuffed-stat-tracker");
    std::fs::create_dir_all(&home).unwrap();
    std::fs::create_dir_all(&data_dir).unwrap();
    let stale = "pid=4294967295\nstarted_at=2026-10-11T10:00:00Z\nlast_seen=2026-10-11T10:41:00Z\n";
    std::fs::write(data_dir.join("run.marker"), stale).unwrap();

    let log_path = root.path().join("daemon.out");
    let log = std::fs::File::create(&log_path).unwrap();
    let mut child = Command::new(env!("CARGO_BIN_EXE_scuffed-stat-tracker"))
        .env_clear()
        .env("PATH", std::env::var("PATH").unwrap_or_default())
        .env("HOME", &home)
        .env("XDG_CONFIG_HOME", root.path().join("config"))
        .env("XDG_DATA_HOME", root.path().join("data"))
        .env("XDG_CACHE_HOME", root.path().join("cache"))
        .env("NO_COLOR", "1")
        .stdin(Stdio::null())
        .stdout(Stdio::from(log.try_clone().unwrap()))
        .stderr(Stdio::from(log))
        .spawn()
        .expect("spawn daemon");
    let pid = child.id();

    let marker = data_dir.join("run.marker");
    let ours = format!("pid={pid}\n");
    let waited = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for("this run's run.marker", Duration::from_secs(60), || {
            read(&marker).starts_with(&ours)
        });
    }));
    if waited.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        panic!(
            "daemon never wrote run.marker; output:\n{}",
            read(&log_path)
        );
    }

    assert_eq!(
        read(&data_dir.join("last_crash.marker")),
        stale,
        "the stale marker is moved, not rewritten"
    );
    let out = read(&log_path);
    let warns: Vec<&str> = out
        .lines()
        .filter(|l| l.contains("WARN") && l.contains("did not exit cleanly"))
        .collect();
    assert_eq!(warns.len(), 1, "one WARN line; output:\n{out}");

    // run.marker is written early in startup, before run_loop installs the
    // SIGTERM handler. A signal in that gap takes the default action (killed
    // by signal 15), so wait until the daemon says the handlers are in place.
    let ready = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        wait_for("signal handlers ready", Duration::from_secs(60), || {
            read(&log_path).contains("signal handlers ready")
        });
    }));
    if ready.is_err() {
        let _ = child.kill();
        let _ = child.wait();
        panic!(
            "daemon never reached the run loop; output:
{}",
            read(&log_path)
        );
    }

    // SAFETY: plain kill(2) on the child we spawned.
    assert_eq!(unsafe { libc::kill(pid as i32, libc::SIGTERM) }, 0);
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().unwrap() {
            break status;
        }
        if start.elapsed() > Duration::from_secs(60) {
            let _ = child.kill();
            let _ = child.wait();
            panic!("daemon ignored SIGTERM; output:\n{}", read(&log_path));
        }
        std::thread::sleep(Duration::from_millis(50));
    };
    let out = read(&log_path);
    assert!(status.success(), "exit {status:?}; output:\n{out}");
    assert!(
        !marker.exists(),
        "SIGTERM removes run.marker; output:\n{out}"
    );
    assert!(
        !data_dir.join("daemon.pid").exists(),
        "clean shutdown removes daemon.pid; output:\n{out}"
    );
    assert!(
        !out.contains("not a pid"),
        "pid file unlink must parse the two-line record; output:\n{out}"
    );
    assert!(data_dir.join("last_crash.marker").exists());
}
