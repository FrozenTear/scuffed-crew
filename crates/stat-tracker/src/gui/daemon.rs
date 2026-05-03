use std::path::PathBuf;

use dioxus::prelude::*;

use stat_tracker::config::Config;

const DAEMON_BIN: &str = "scuffed-stat-tracker";

fn pid_file(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("daemon.pid")
}

fn read_pid(data_dir: &std::path::Path) -> Option<u32> {
    let path = pid_file(data_dir);
    let text = std::fs::read_to_string(path).ok()?;
    text.trim().parse().ok()
}

fn is_pid_alive(pid: u32) -> bool {
    std::fs::metadata(format!("/proc/{pid}")).is_ok()
}

fn daemon_running(data_dir: &std::path::Path) -> Option<u32> {
    let pid = read_pid(data_dir)?;
    if is_pid_alive(pid) {
        Some(pid)
    } else {
        let _ = std::fs::remove_file(pid_file(data_dir));
        None
    }
}

fn find_daemon_binary() -> Option<PathBuf> {
    if let Ok(current_exe) = std::env::current_exe() {
        if let Some(dir) = current_exe.parent() {
            let sibling = dir.join(DAEMON_BIN);
            if sibling.exists() {
                return Some(sibling);
            }
        }
    }
    for dir in std::env::var("PATH").unwrap_or_default().split(':') {
        let candidate = PathBuf::from(dir).join(DAEMON_BIN);
        if candidate.exists() {
            return Some(candidate);
        }
    }
    None
}

fn start_daemon(data_dir: &std::path::Path) -> Result<u32, String> {
    let exe = find_daemon_binary()
        .ok_or("Cannot find scuffed-stat-tracker binary in PATH or next to GUI binary")?;

    let child = std::process::Command::new(&exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .map_err(|e| format!("Failed to spawn daemon: {e}"))?;

    let pid = child.id();
    let _ = std::fs::create_dir_all(data_dir);
    std::fs::write(pid_file(data_dir), pid.to_string())
        .map_err(|e| format!("Failed to write PID file: {e}"))?;

    Ok(pid)
}

fn stop_daemon(data_dir: &std::path::Path) -> Result<(), String> {
    let pid = read_pid(data_dir).ok_or("No daemon PID found")?;
    if !is_pid_alive(pid) {
        let _ = std::fs::remove_file(pid_file(data_dir));
        return Err("Daemon process already exited".into());
    }
    std::process::Command::new("kill")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("Failed to send SIGTERM: {e}"))?;
    let _ = std::fs::remove_file(pid_file(data_dir));
    Ok(())
}

#[component]
pub fn DaemonCard() -> Element {
    let config = use_signal(|| Config::load().unwrap_or_default());
    let mut daemon_pid: Signal<Option<u32>> = use_signal(|| daemon_running(&config().data_dir));
    let mut toast: Signal<Option<(String, bool)>> = use_signal(|| None);

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            daemon_pid.set(daemon_running(&config().data_dir));
        }
    });

    let on_start = move |_| {
        match start_daemon(&config().data_dir) {
            Ok(pid) => {
                daemon_pid.set(Some(pid));
                toast.set(Some((format!("Daemon started (PID {pid})"), true)));
            }
            Err(e) => toast.set(Some((e, false))),
        }
        spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            toast.set(None);
        });
    };

    let on_stop = move |_| {
        match stop_daemon(&config().data_dir) {
            Ok(()) => {
                daemon_pid.set(None);
                toast.set(Some(("Daemon stopped".into(), true)));
            }
            Err(e) => {
                daemon_pid.set(daemon_running(&config().data_dir));
                toast.set(Some((e, false)));
            }
        }
        spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            toast.set(None);
        });
    };

    let running = daemon_pid().is_some();

    rsx! {
        div { class: "card",
            h3 { "Daemon" }
            div { class: "stat-row",
                span { class: "label", "Status" }
                span { class: "value",
                    span {
                        class: if running { "status-dot ok" } else { "status-dot err" },
                    }
                    if running { "running" } else { "stopped" }
                }
            }
            if let Some(pid) = daemon_pid() {
                div { class: "stat-row",
                    span { class: "label", "PID" }
                    span { class: "value", "{pid}" }
                }
            }
            div { class: "actions",
                if running {
                    button { class: "btn btn-secondary", onclick: on_stop, "Stop Daemon" }
                } else {
                    button { class: "btn btn-primary", onclick: on_start, "Start Daemon" }
                }
            }
        }

        if let Some((msg, ok)) = toast() {
            div { class: if ok { "toast success" } else { "toast error" }, "{msg}" }
        }
    }
}
