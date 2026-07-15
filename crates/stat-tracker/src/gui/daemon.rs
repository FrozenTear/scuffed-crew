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

/// True when a daemon PID file points at a live process. Used by Settings to
/// refuse force-clear of the SurrealKV store while the daemon holds it open.
pub fn is_daemon_running(data_dir: &std::path::Path) -> bool {
    daemon_running(data_dir).is_some()
}

fn find_daemon_binary() -> Option<PathBuf> {
    if let Ok(current_exe) = std::env::current_exe()
        && let Some(dir) = current_exe.parent()
    {
        let sibling = dir.join(DAEMON_BIN);
        if sibling.exists() {
            return Some(sibling);
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

fn spawn_daemon(data_dir: &std::path::Path) -> Result<std::process::Child, String> {
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

    std::process::Command::new(&exe)
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
        .map_err(|e| format!("Failed to spawn daemon: {e}"))
}

/// Spawn the daemon and confirm it actually stayed up. The store is a
/// single-writer SurrealKV database: if another instance (e.g. the systemd
/// service) already holds the lock, the freshly-spawned daemon dies within a
/// few hundred ms. Returning Ok before confirming survival produced a false
/// "started" toast and a zombie child; instead we wait, then either reap the
/// dead child and surface the real reason from the log, or detach a live one.
async fn start_daemon_checked(data_dir: &std::path::Path) -> Result<u32, String> {
    let mut child = spawn_daemon(data_dir)?;
    let pid = child.id();

    // Long enough for config load + the DB-lock acquisition that fails fast on
    // a conflict, short enough that the button still feels responsive.
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;

    match child.try_wait() {
        // Exited already — try_wait reaps it (no zombie) and we report why.
        Ok(Some(status)) => Err(last_log_error(data_dir).unwrap_or_else(|| {
            format!(
                "daemon exited immediately ({status}) — see {}",
                daemon_log_path(data_dir).display()
            )
        })),
        // Still alive — detach so it survives GUI exit (reparented to init)
        // rather than becoming a zombie when the Child drops.
        Ok(None) => {
            std::mem::forget(child);
            Ok(pid)
        }
        // Can't determine — assume it took; the status poll will correct us.
        Err(_) => {
            std::mem::forget(child);
            Ok(pid)
        }
    }
}

/// Read the tail of the daemon log and return the most recent line that looks
/// like an error, so a failed start can show the actual cause (typically the
/// SurrealKV "LOCK is already locked" message) instead of a generic failure.
fn last_log_error(data_dir: &std::path::Path) -> Option<String> {
    use std::io::{Read, Seek, SeekFrom};
    let mut f = std::fs::File::open(daemon_log_path(data_dir)).ok()?;
    let len = f.metadata().ok()?.len();
    let start = len.saturating_sub(8192);
    f.seek(SeekFrom::Start(start)).ok()?;
    let mut buf = Vec::new();
    f.read_to_end(&mut buf).ok()?;
    let text = String::from_utf8_lossy(&buf);
    text.lines()
        .rev()
        .find(|l| l.contains("Error") || l.contains("error") || l.contains("ERROR"))
        .map(|l| l.trim().to_string())
}

fn systemd_unit() -> &'static str {
    "scuffed-stat-tracker.service"
}

async fn systemd_enabled() -> bool {
    tokio::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", systemd_unit()])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

async fn systemd_enable() -> Result<(), String> {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", "enable", "--now", systemd_unit()])
        .output()
        .await
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

async fn systemd_disable() -> Result<(), String> {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", "disable", "--now", systemd_unit()])
        .output()
        .await
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Start/stop the daemon *through systemd* when the service unit is installed.
/// systemd is the single supervisor for the unit, so going through it is
/// idempotent (starting an already-running unit is a no-op) and avoids the GUI
/// spawning a second bare process that fights the service over the DB lock.
async fn systemd_action(verb: &str) -> Result<(), String> {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", verb, systemd_unit()])
        .output()
        .await
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
    // Populated asynchronously — never block the UI thread on systemctl.
    let mut autostart: Signal<bool> = use_signal(|| false);
    let mut toast: Signal<Option<(String, bool)>> = use_signal(|| None);
    let service_installed = service_file_installed();

    use_future(move || async move {
        // Initial autostart read (and every 10s thereafter) via tokio::process.
        let enabled = systemd_enabled().await;
        if enabled != autostart() {
            autostart.set(enabled);
        }
        loop {
            tokio::time::sleep(std::time::Duration::from_secs(10)).await;
            let pid = daemon_running(&config().data_dir);
            if pid != daemon_pid() {
                daemon_pid.set(pid);
            }
            let enabled = systemd_enabled().await;
            if enabled != autostart() {
                autostart.set(enabled);
            }
        }
    });

    let on_start = move |_| {
        let data_dir = config().data_dir.clone();
        spawn(async move {
            // When the unit is installed, systemd is the supervisor — go through
            // it so we don't spawn a competing bare process that loses the DB
            // lock race. Only fall back to a direct spawn if there's no unit.
            let result = if service_installed {
                systemd_action("start").await
            } else {
                start_daemon_checked(&data_dir).await.map(|_| ())
            };
            match result {
                Ok(()) => toast.set(Some(("Daemon started".into(), true))),
                Err(e) => toast.set(Some((e, false))),
            }
            // Give the daemon a moment to write its PID file, then reflect
            // the real state rather than an optimistic guess.
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            daemon_pid.set(daemon_running(&data_dir));
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            toast.set(None);
        });
    };

    let on_stop = move |_| {
        let data_dir = config().data_dir.clone();
        spawn(async move {
            let result = if service_installed {
                systemd_action("stop").await
            } else {
                stop_daemon(&data_dir)
            };
            match result {
                Ok(()) => toast.set(Some(("Daemon stopped".into(), true))),
                Err(e) => toast.set(Some((e, false))),
            }
            tokio::time::sleep(std::time::Duration::from_millis(500)).await;
            daemon_pid.set(daemon_running(&data_dir));
            tokio::time::sleep(std::time::Duration::from_secs(3)).await;
            toast.set(None);
        });
    };

    let on_toggle_autostart = move |_| {
        let enabled = autostart();
        let data_dir = config().data_dir.clone();
        spawn(async move {
            let result = if enabled {
                systemd_disable().await
            } else {
                systemd_enable().await
            };
            match result {
                Ok(()) => {
                    autostart.set(!enabled);
                    daemon_pid.set(daemon_running(&data_dir));
                    let msg = if enabled {
                        "Autostart disabled — daemon will not start on login."
                    } else {
                        "Autostart enabled — daemon will start automatically on login."
                    };
                    toast.set(Some((msg.into(), true)));
                }
                Err(e) => toast.set(Some((format!("systemctl error: {e}"), false))),
            }
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
