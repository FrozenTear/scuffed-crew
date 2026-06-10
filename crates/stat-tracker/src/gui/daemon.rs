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

fn daemon_log_path(data_dir: &std::path::Path) -> PathBuf {
    data_dir.join("daemon.log")
}

fn start_daemon(data_dir: &std::path::Path) -> Result<u32, String> {
    let exe = find_daemon_binary()
        .ok_or("Cannot find scuffed-stat-tracker binary in PATH or next to GUI binary")?;

    let _ = std::fs::create_dir_all(data_dir);
    let log_path = daemon_log_path(data_dir);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("Failed to open daemon log {}: {e}", log_path.display()))?;
    let stderr_file = log_file
        .try_clone()
        .map_err(|e| format!("Failed to clone log file handle: {e}"))?;

    let child = std::process::Command::new(&exe)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::from(log_file))
        .stderr(std::process::Stdio::from(stderr_file))
        .env(
            "RUST_LOG",
            std::env::var("RUST_LOG").unwrap_or_else(|_| {
                "scuffed_stat_tracker=info,stat_tracker=info,surrealdb=warn".into()
            }),
        )
        .spawn()
        .map_err(|e| format!("Failed to spawn daemon: {e}"))?;

    let pid = child.id();
    // Forget the Child handle so it gets reparented to init on GUI exit
    // rather than becoming a zombie when dropped.
    std::mem::forget(child);

    // The daemon writes its own PID file on startup — don't write it here
    // or the daemon will see its own PID as "already running" and refuse to start.
    Ok(pid)
}

fn systemd_unit() -> &'static str {
    "scuffed-stat-tracker.service"
}

fn systemd_enabled() -> bool {
    std::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", systemd_unit()])
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

fn systemd_enable() -> Result<(), String> {
    let out = std::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", systemd_unit()])
        .output()
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn systemd_disable() -> Result<(), String> {
    let out = std::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", systemd_unit()])
        .output()
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

fn service_file_installed() -> bool {
    dirs::config_dir()
        .map(|d| d.join("systemd").join("user").join(systemd_unit()).exists())
        .unwrap_or(false)
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
    let mut autostart: Signal<bool> = use_signal(systemd_enabled);
    let mut toast: Signal<Option<(String, bool)>> = use_signal(|| None);
    let service_installed = service_file_installed();

    use_future(move || async move {
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            daemon_pid.set(daemon_running(&config().data_dir));
            autostart.set(systemd_enabled());
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

    let on_toggle_autostart = move |_| {
        let enabled = autostart();
        let result = if enabled {
            systemd_disable()
        } else {
            systemd_enable()
        };
        match result {
            Ok(()) => {
                autostart.set(!enabled);
                daemon_pid.set(daemon_running(&config().data_dir));
                let msg = if enabled {
                    "Autostart disabled — daemon will not start on login."
                } else {
                    "Autostart enabled — daemon will start automatically on login."
                };
                toast.set(Some((msg.into(), true)));
            }
            Err(e) => toast.set(Some((format!("systemctl error: {e}"), false))),
        }
        spawn(async move {
            tokio::time::sleep(std::time::Duration::from_secs(4)).await;
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
                div { class: "stat-row",
                    span { class: "label", "Log" }
                    span { class: "value text-dim", "{daemon_log_path(&config().data_dir).display()}" }
                }
            }
            div { class: "stat-row",
                span { class: "label", "Autostart" }
                span { class: "value",
                    if !service_installed {
                        span { class: "text-dim", "service not installed" }
                    } else if autostart() {
                        span { class: "status-dot ok" }
                        "enabled"
                    } else {
                        span { class: "status-dot err" }
                        "disabled"
                    }
                }
            }
            div { class: "actions",
                if running {
                    button { class: "btn btn-secondary", onclick: on_stop, "Stop" }
                } else {
                    button { class: "btn btn-primary", onclick: on_start, "Start" }
                }
                if service_installed {
                    button {
                        class: if autostart() { "btn btn-secondary" } else { "btn btn-outline" },
                        onclick: on_toggle_autostart,
                        if autostart() { "Disable Autostart" } else { "Enable Autostart" }
                    }
                }
            }
        }

        if let Some((msg, ok)) = toast() {
            div { class: if ok { "toast success" } else { "toast error" }, "{msg}" }
        }
    }
}
