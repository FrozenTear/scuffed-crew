//! Tracker service control — same systemctl user unit as the Dioxus GUI.
//!
//! Status and stop share one PID/`comm` identity check (the daemon's
//! `pid_is_live_tracker` rule) so a reused PID is never treated as the
//! tracker and is never signalled.

use std::path::{Path, PathBuf};

/// User unit installed by `install.sh` — same name the Dioxus GUI uses.
pub const SYSTEMD_UNIT: &str = "scuffed-stat-tracker.service";

/// Daemon binary name (`/proc/<pid>/comm` truncates to 15 bytes).
pub const DAEMON_BIN: &str = "scuffed-stat-tracker";

/// Shared with the daemon binary's `pid_is_live_tracker`:
/// `comm` is `scuffed-stat-tracker` or the kernel-truncated `scuffed-stat-tr`.
pub const TRACKER_COMM_PREFIX: &str = "scuffed-stat";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DaemonVerb {
    Start,
    Stop,
    Restart,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DaemonView {
    pub pid: Option<u32>,
    pub service_installed: bool,
    pub autostart: bool,
}

impl Default for DaemonView {
    fn default() -> Self {
        Self {
            pid: None,
            service_installed: service_file_installed(),
            autostart: false,
        }
    }
}

impl DaemonView {
    pub fn running(&self) -> bool {
        self.pid.is_some()
    }

    pub fn status_label(&self) -> &'static str {
        if self.running() { "running" } else { "stopped" }
    }
}

pub fn pid_file(data_dir: &Path) -> PathBuf {
    data_dir.join("daemon.pid")
}

pub fn daemon_log_path(data_dir: &Path) -> PathBuf {
    data_dir.join("daemon.log")
}

pub fn read_pid(data_dir: &Path) -> Option<u32> {
    let text = std::fs::read_to_string(pid_file(data_dir)).ok()?;
    text.trim().parse().ok()
}

/// True when `/proc/<pid>/comm` is the tracker daemon (not a reused PID).
pub fn comm_is_tracker(comm: &str) -> bool {
    comm.trim().starts_with(TRACKER_COMM_PREFIX)
}

/// True only if `pid` is alive **and** is actually a scuffed-stat-tracker process.
///
/// A bare `/proc/{pid}` existence check false-positives on PID reuse. Matching
/// `/proc/{pid}/comm` (world-readable) against our binary name rejects that
/// case. Never treats this GUI process as the daemon.
pub fn pid_is_live_tracker(pid: u32) -> bool {
    if pid == std::process::id() {
        return false;
    }
    match std::fs::read_to_string(format!("/proc/{pid}/comm")) {
        Ok(comm) => comm_is_tracker(&comm),
        Err(_) => false,
    }
}

/// Live tracker PID from `daemon.pid`, or `None` (stale file is removed).
pub fn daemon_running(data_dir: &Path) -> Option<u32> {
    let pid = read_pid(data_dir)?;
    if pid_is_live_tracker(pid) {
        Some(pid)
    } else {
        let _ = std::fs::remove_file(pid_file(data_dir));
        None
    }
}

pub fn is_daemon_running(data_dir: &Path) -> bool {
    daemon_running(data_dir).is_some()
}

pub fn refresh_view(data_dir: &Path, current: &DaemonView) -> DaemonView {
    DaemonView {
        pid: daemon_running(data_dir),
        service_installed: service_file_installed(),
        autostart: current.autostart,
    }
}

pub fn service_unit_path() -> Option<PathBuf> {
    dirs::config_dir().map(|d| d.join("systemd").join("user").join(SYSTEMD_UNIT))
}

pub fn service_file_installed() -> bool {
    service_unit_path().is_some_and(|p| p.exists())
}

pub(crate) fn find_daemon_binary() -> Option<PathBuf> {
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

fn spawn_daemon(data_dir: &Path) -> Result<std::process::Child, String> {
    let exe = find_daemon_binary().ok_or(
        "Cannot find the tracker service next to this app or on PATH (scuffed-stat-tracker)",
    )?;

    let _ = std::fs::create_dir_all(data_dir);
    let log_path = daemon_log_path(data_dir);
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&log_path)
        .map_err(|e| format!("Could not open the tracker log {}: {e}", log_path.display()))?;
    let stderr_file = log_file
        .try_clone()
        .map_err(|e| format!("Could not open the tracker log: {e}"))?;

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
        .map_err(|e| format!("Could not start the tracker: {e}"))
}

fn last_log_error(data_dir: &Path) -> Option<String> {
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

async fn start_daemon_checked(data_dir: &Path) -> Result<u32, String> {
    let mut child = spawn_daemon(data_dir)?;
    let pid = child.id();
    tokio::time::sleep(std::time::Duration::from_millis(1500)).await;
    match child.try_wait() {
        Ok(Some(status)) => Err(last_log_error(data_dir).unwrap_or_else(|| {
            format!(
                "Tracker exited immediately ({status}) — see {}",
                daemon_log_path(data_dir).display()
            )
        })),
        Ok(None) | Err(_) => {
            std::mem::forget(child);
            Ok(pid)
        }
    }
}

/// Stop only a live tracker PID. Never signals a reused / foreign process.
pub fn stop_daemon(data_dir: &Path) -> Result<(), String> {
    let pid = read_pid(data_dir).ok_or("Tracker is not running")?;
    if pid == std::process::id() {
        return Err("Refusing to stop this window's process".into());
    }
    if !pid_is_live_tracker(pid) {
        let _ = std::fs::remove_file(pid_file(data_dir));
        return Err("Saved process id is not the tracker — not stopping it".into());
    }
    std::process::Command::new("kill")
        .arg(pid.to_string())
        .output()
        .map_err(|e| format!("Could not stop the tracker: {e}"))?;
    let _ = std::fs::remove_file(pid_file(data_dir));
    Ok(())
}

async fn systemd_action(verb: &str) -> Result<(), String> {
    let out = tokio::process::Command::new("systemctl")
        .args(["--user", verb, SYSTEMD_UNIT])
        .output()
        .await
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

pub async fn systemd_enabled() -> bool {
    tokio::process::Command::new("systemctl")
        .args(["--user", "is-enabled", "--quiet", SYSTEMD_UNIT])
        .status()
        .await
        .map(|s| s.success())
        .unwrap_or(false)
}

pub async fn systemd_enable() -> Result<(), String> {
    systemd_action_full(&["enable", "--now"]).await
}

pub async fn systemd_disable() -> Result<(), String> {
    systemd_action_full(&["disable", "--now"]).await
}

async fn systemd_action_full(args: &[&str]) -> Result<(), String> {
    let out = tokio::process::Command::new("systemctl")
        .arg("--user")
        .args(args)
        .arg(SYSTEMD_UNIT)
        .output()
        .await
        .map_err(|e| format!("systemctl: {e}"))?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_string())
    }
}

/// Start / stop / restart through the user unit when installed; otherwise
/// spawn or identity-checked stop. Restart without a unit is stop-then-start.
pub async fn run_verb(
    data_dir: PathBuf,
    verb: DaemonVerb,
    service_installed: bool,
) -> Result<String, String> {
    match verb {
        DaemonVerb::Start => {
            if service_installed {
                systemd_action("start").await?;
            } else {
                start_daemon_checked(&data_dir).await?;
            }
            Ok("Tracker started".into())
        }
        DaemonVerb::Stop => {
            if service_installed {
                systemd_action("stop").await?;
            } else {
                stop_daemon(&data_dir)?;
            }
            Ok("Tracker stopped".into())
        }
        DaemonVerb::Restart => {
            if service_installed {
                systemd_action("restart").await?;
            } else {
                let _ = stop_daemon(&data_dir);
                tokio::time::sleep(std::time::Duration::from_millis(400)).await;
                start_daemon_checked(&data_dir).await?;
            }
            Ok("Tracker restarted".into())
        }
    }
}

pub async fn toggle_autostart(currently_enabled: bool) -> Result<String, String> {
    if currently_enabled {
        systemd_disable().await?;
        Ok("Start on login is off".into())
    } else {
        systemd_enable().await?;
        Ok("Tracker will start when you log in".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn comm_identity_matches_daemon_rule() {
        assert!(comm_is_tracker("scuffed-stat-tr"));
        assert!(comm_is_tracker("scuffed-stat-tracker"));
        assert!(comm_is_tracker("scuffed-stat-tr\n"));
        assert!(!comm_is_tracker("stat-tracker-gu"));
        assert!(!comm_is_tracker("firefox"));
        assert!(!comm_is_tracker(""));
        assert!(!comm_is_tracker("scuffed"));
    }

    #[test]
    fn self_pid_is_never_the_daemon() {
        assert!(!pid_is_live_tracker(std::process::id()));
        assert!(!pid_is_live_tracker(u32::MAX));
    }

    #[test]
    fn stale_or_foreign_pid_file_is_not_running() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(pid_file(dir.path()), format!("{}", std::process::id())).unwrap();
        assert!(
            daemon_running(dir.path()).is_none(),
            "GUI pid in daemon.pid must not count as the tracker"
        );
        std::fs::write(pid_file(dir.path()), "4294967295").unwrap();
        assert!(daemon_running(dir.path()).is_none());
    }

    #[test]
    fn stop_refuses_foreign_pid() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(pid_file(dir.path()), format!("{}", std::process::id())).unwrap();
        let err = stop_daemon(dir.path()).expect_err("must refuse");
        assert!(
            err.contains("not the tracker") || err.contains("this window"),
            "unexpected refusal: {err}"
        );
    }

    #[test]
    fn stop_without_pid_file_is_a_clean_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = stop_daemon(dir.path()).expect_err("no pid");
        assert!(err.contains("not running"), "{err}");
    }
}
