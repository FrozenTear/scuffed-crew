//! `stat-tracker-gui --build-id` prints the id and exits before the
//! single-instance lock (G3). A running GUI probes the installed binary
//! with it; a probe that took the lock path would show, or even replace,
//! that running GUI.

use std::fs::File;
use std::io::Write;
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

use stat_tracker_ui::{instance, restart_prompt};

#[test]
fn build_id_prints_and_exits_before_the_instance_lock() {
    let scratch = tempfile::tempdir().expect("tempdir");
    let runtime = scratch.path().join("runtime");
    let data = scratch.path().join("data");
    let home = scratch.path().join("home");
    for dir in [&runtime, &data, &home] {
        std::fs::create_dir_all(dir).unwrap();
    }

    // Hold the lock like a running GUI of a newer version, so a probe that
    // reached the lock would ask it to show (touch `show_window`).
    let lock_path = instance::lock_path(Some(runtime.clone().into_os_string()), &data);
    let mut lock = File::create(&lock_path).expect("lock file");
    lock.try_lock().expect("take the lock");
    let holder = format!("{}\n999.0.0\n", std::process::id());
    lock.write_all(holder.as_bytes()).unwrap();
    lock.flush().unwrap();

    let mut child = Command::new(env!("CARGO_BIN_EXE_stat-tracker-gui"))
        .arg("--build-id")
        .arg("--data-dir")
        .arg(&data)
        .env("XDG_RUNTIME_DIR", &runtime)
        .env("XDG_CONFIG_HOME", home.join(".config"))
        .env("XDG_DATA_HOME", home.join(".local/share"))
        .env("HOME", &home)
        .env_remove("WAYLAND_DISPLAY")
        .env_remove("DISPLAY")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .expect("start stat-tracker-gui");
    let start = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait().expect("wait") {
            break status;
        }
        if start.elapsed() > Duration::from_secs(30) {
            let _ = child.kill();
            panic!("--build-id did not exit within 30 s");
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    let mut stdout = String::new();
    std::io::Read::read_to_string(&mut child.stdout.take().unwrap(), &mut stdout).unwrap();

    assert!(status.success(), "{status}");
    assert_eq!(stdout, format!("{}\n", restart_prompt::BUILD_ID));
    assert!(restart_prompt::is_build_id(stdout.trim()), "{stdout:?}");
    assert!(
        !data.join(instance::SHOW_FILE).exists(),
        "the probe asked the running GUI to show"
    );
    assert!(
        !data.join(instance::QUIT_FILE).exists(),
        "the probe asked the running GUI to quit"
    );
    assert_eq!(
        std::fs::read_to_string(&lock_path).unwrap(),
        holder,
        "the probe rewrote the lock file"
    );
}
