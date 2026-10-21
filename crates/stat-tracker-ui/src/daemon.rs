//! Tracker service control, same systemctl user unit as the Dioxus GUI.
//!
//! Status and stop share the daemon's pid-file identity check
//! (`stat_tracker::proc_id`): `/proc/<pid>/exe` must be the tracker binary,
//! and the start time must match when the pid file recorded one. A reused
//! PID is never treated as the tracker and is never signalled.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// User unit installed by `install.sh`, same name the Dioxus GUI uses.
pub const SYSTEMD_UNIT: &str = "scuffed-stat-tracker.service";

/// Daemon binary name (`/proc/<pid>/comm` truncates to 15 bytes).
pub const DAEMON_BIN: &str = "scuffed-stat-tracker";

/// Kernel-truncated `comm` of [`DAEMON_BIN`]. Exact match only, not a prefix.
pub const TRACKER_COMM: &str = stat_tracker::proc_id::DAEMON_COMM;

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

pub fn read_pid_record(data_dir: &Path) -> Option<stat_tracker::proc_id::PidRecord> {
    let text = std::fs::read_to_string(pid_file(data_dir)).ok()?;
    stat_tracker::proc_id::parse_pid_record(&text)
}

pub fn read_pid(data_dir: &Path) -> Option<u32> {
    read_pid_record(data_dir).map(|rec| rec.pid)
}

/// Exact `comm` of the tracker daemon (full name or the 15-byte truncation).
pub fn comm_is_tracker(comm: &str) -> bool {
    stat_tracker::proc_id::comm_names_daemon(comm)
}

/// True only if `pid` is alive and its executable is the tracker daemon.
///
/// Prefers `/proc/<pid>/exe` over `comm`, so a reused PID whose name only
/// shares a prefix is not the daemon. Never treats this GUI process as it.
pub fn pid_is_live_tracker(pid: u32) -> bool {
    stat_tracker::proc_id::pid_is_live_tracker(pid)
}

/// Live tracker PID from `daemon.pid`, or `None` (stale file is removed).
/// When the file records a start time, a recycled PID of the same binary
/// does not count.
pub fn daemon_running(data_dir: &Path) -> Option<u32> {
    let rec = read_pid_record(data_dir)?;
    if stat_tracker::proc_id::pid_is_live_tracker_started(rec.pid, rec.start_ticks) {
        Some(rec.pid)
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

/// Parsed `systemctl --user show` fields. Synthetic text is enough to test.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UnitShow {
    pub load_state: String,
    pub fragment_path: String,
    pub active_state: String,
    pub result: String,
}

impl UnitShow {
    pub fn installed(&self) -> bool {
        self.load_state == "loaded" && !self.fragment_path.is_empty()
    }

    pub fn needs_reset(&self) -> bool {
        self.active_state == "failed" || self.result == "start-limit-hit"
    }
}

pub fn parse_systemctl_show(text: &str) -> UnitShow {
    let mut show = UnitShow {
        load_state: String::new(),
        fragment_path: String::new(),
        active_state: String::new(),
        result: String::new(),
    };
    for line in text.lines() {
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        match key.trim() {
            "LoadState" => show.load_state = value.trim().to_string(),
            "FragmentPath" => show.fragment_path = value.trim().to_string(),
            "ActiveState" => show.active_state = value.trim().to_string(),
            "Result" => show.result = value.trim().to_string(),
            _ => {}
        }
    }
    show
}

/// Unit directories, user config first, then the packaged locations an AUR
/// or deb install uses.
pub fn user_unit_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();
    if let Some(config) = dirs::config_dir() {
        dirs.push(config.join("systemd").join("user"));
    }
    for path in [
        "/etc/systemd/user",
        "/usr/local/lib/systemd/user",
        "/usr/lib/systemd/user",
        "/lib/systemd/user",
        "/run/systemd/user",
    ] {
        dirs.push(PathBuf::from(path));
    }
    dirs
}

pub fn unit_in_dirs(dirs: &[PathBuf], unit: &str) -> Option<PathBuf> {
    dirs.iter()
        .map(|dir| dir.join(unit))
        .find(|path| path.is_file())
}

pub fn service_unit_path() -> Option<PathBuf> {
    if let Some(show) = fresh_show(Duration::from_secs(10))
        && show.installed()
    {
        return Some(PathBuf::from(show.fragment_path));
    }
    unit_in_dirs(&user_unit_dirs(), SYSTEMD_UNIT)
}

pub fn service_file_installed() -> bool {
    if let Some(show) = fresh_show(Duration::from_secs(10)) {
        return show.installed();
    }
    if let Some(show) = systemctl_show_sync() {
        remember_show(show.clone());
        return show.installed();
    }
    unit_in_dirs(&user_unit_dirs(), SYSTEMD_UNIT).is_some()
}

struct ShowCache {
    at: std::time::Instant,
    show: UnitShow,
}

fn cache() -> &'static std::sync::Mutex<Option<ShowCache>> {
    static CACHE: std::sync::OnceLock<std::sync::Mutex<Option<ShowCache>>> =
        std::sync::OnceLock::new();
    CACHE.get_or_init(|| std::sync::Mutex::new(None))
}

fn fresh_show(max_age: Duration) -> Option<UnitShow> {
    let guard = cache().lock().unwrap_or_else(|err| err.into_inner());
    guard
        .as_ref()
        .and_then(|cached| (cached.at.elapsed() <= max_age).then(|| cached.show.clone()))
}

fn remember_show(show: UnitShow) {
    let mut guard = cache().lock().unwrap_or_else(|err| err.into_inner());
    *guard = Some(ShowCache {
        at: std::time::Instant::now(),
        show,
    });
}

fn systemctl_show_sync() -> Option<UnitShow> {
    let mut child = std::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "LoadState",
            "-p",
            "FragmentPath",
            "-p",
            "ActiveState",
            "-p",
            "Result",
            SYSTEMD_UNIT,
        ])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let started = std::time::Instant::now();
    let status = loop {
        if let Ok(Some(status)) = child.try_wait() {
            break status;
        }
        if started.elapsed() > Duration::from_millis(800) {
            let _ = child.kill();
            let _ = child.wait();
            return None;
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        return None;
    }
    let mut stdout = child.stdout?;
    let mut buf = String::new();
    use std::io::Read;
    stdout.read_to_string(&mut buf).ok()?;
    Some(parse_systemctl_show(&buf))
}

async fn unit_show_now() -> Option<UnitShow> {
    let out = tokio::process::Command::new("systemctl")
        .args([
            "--user",
            "show",
            "-p",
            "LoadState",
            "-p",
            "FragmentPath",
            "-p",
            "ActiveState",
            "-p",
            "Result",
            SYSTEMD_UNIT,
        ])
        .output()
        .await
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let show = parse_systemctl_show(&text);
    remember_show(show.clone());
    Some(show)
}

/// Ask systemd. If that fails, scan the unit directories. The bool the
/// caller remembered is not trusted on its own.
async fn service_installed_now(remembered: bool) -> bool {
    if let Some(show) = unit_show_now().await {
        return show.installed();
    }
    unit_in_dirs(&user_unit_dirs(), SYSTEMD_UNIT).is_some() || remembered
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
                "Tracker exited immediately ({status}). See {}",
                daemon_log_path(data_dir).display()
            )
        })),
        Ok(None) | Err(_) => {
            std::mem::forget(child);
            Ok(pid)
        }
    }
}

/// How long a GUI stop waits for the tracker process to leave `/proc`.
///
/// Shutdown drains in-flight Tab OCR before the process exits and releases
/// the SurrealKV store. systemd's default `TimeoutStopSec` is 90s; match
/// that so Stop/Restart does not open a second writer early and does not
/// hang forever if the process never exits.
pub const STOP_WAIT_TIMEOUT: Duration = Duration::from_secs(90);

const STOP_POLL: Duration = Duration::from_millis(50);

#[derive(Debug)]
pub enum StopError {
    NotRunning,
    Refused(&'static str),
    Failed(String),
}

impl StopError {
    /// Nothing live was signalled: there was no pid, or the pid is not the
    /// tracker. Restart and uninstall may continue. A [`StopError::Failed`]
    /// means a live tracker is still up.
    pub(crate) fn allows_restart(&self) -> bool {
        matches!(self, StopError::NotRunning | StopError::Refused(_))
    }
}

impl std::fmt::Display for StopError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            StopError::NotRunning => f.write_str("Tracker is not running"),
            StopError::Refused(msg) => f.write_str(msg),
            StopError::Failed(msg) => f.write_str(msg),
        }
    }
}

/// Stop only a live tracker PID. Never signals a reused / foreign process.
///
/// Does not unlink `daemon.pid`. The daemon removes that file on the way out,
/// and only when it still names that process. Unlinking here raced a restart:
/// the new process wrote its pid, then the exiting process deleted it.
///
/// Stale pid files are left in place. [`daemon_running`] still clears them
/// when it refreshes status; this path must not delete a file that a newer
/// tracker may already own.
pub async fn stop_daemon(data_dir: &Path) -> Result<(), StopError> {
    stop_daemon_result(data_dir).await
}

async fn stop_daemon_result(data_dir: &Path) -> Result<(), StopError> {
    let pid = read_pid(data_dir).ok_or(StopError::NotRunning)?;
    if pid == std::process::id() {
        return Err(StopError::Refused("Refusing to stop this window's process"));
    }
    if !pid_is_live_tracker(pid) {
        return Err(StopError::Refused(
            "Saved process id is not the tracker, not stopping it",
        ));
    }
    signal_term(pid)?;
    wait_until_tracker_gone(pid, STOP_WAIT_TIMEOUT, STOP_POLL).await
}

fn signal_term(pid: u32) -> Result<(), StopError> {
    let out = std::process::Command::new("kill")
        .arg(pid.to_string())
        .output()
        .map_err(|e| StopError::Failed(format!("Could not stop the tracker: {e}")))?;
    if out.status.success() || !pid_is_live_tracker(pid) {
        return Ok(());
    }
    let detail = String::from_utf8_lossy(&out.stderr).trim().to_string();
    if detail.is_empty() {
        Err(StopError::Failed(format!(
            "Could not stop the tracker (kill status {})",
            out.status
        )))
    } else {
        Err(StopError::Failed(format!(
            "Could not stop the tracker: {detail}"
        )))
    }
}

/// Poll until `pid` is gone or `/proc/<pid>/comm` is no longer the tracker.
async fn wait_until_tracker_gone(
    pid: u32,
    timeout: Duration,
    poll: Duration,
) -> Result<(), StopError> {
    wait_while(|| pid_is_live_tracker(pid), timeout, poll).await
}

/// Poll `still_present` until it is false, or `timeout` elapses.
async fn wait_while(
    mut still_present: impl FnMut() -> bool,
    timeout: Duration,
    poll: Duration,
) -> Result<(), StopError> {
    let started = tokio::time::Instant::now();
    loop {
        if !still_present() {
            return Ok(());
        }
        if started.elapsed() >= timeout {
            return Err(StopError::Failed(
                "Tracker is still shutting down, not starting another copy".into(),
            ));
        }
        let slice = poll.min(timeout.saturating_sub(started.elapsed()));
        if slice.is_zero() {
            return Err(StopError::Failed(
                "Tracker is still shutting down, not starting another copy".into(),
            ));
        }
        tokio::time::sleep(slice).await;
    }
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
    let _ = systemd_action("reset-failed").await;
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

/// Start / stop / restart through the user unit when systemd has it,
/// otherwise spawn or identity-checked stop.
///
/// `service_installed` is not trusted: the unit is looked up again, so a
/// stale flag from a 1s refresh cannot skip an AUR unit or keep using one
/// that was removed. Stop and Restart signal the process that actually
/// holds the pid file or the store lock, after an identity check, whether
/// or not systemd started it. Start, when the unit exists, only starts
/// that unit.
pub async fn run_verb(
    data_dir: PathBuf,
    verb: DaemonVerb,
    service_installed: bool,
) -> Result<String, String> {
    let installed = service_installed_now(service_installed).await;
    match verb {
        DaemonVerb::Start => {
            if installed {
                let _ = systemd_action("reset-failed").await;
                systemd_action("start").await?;
            } else {
                // No unit: the app is the only starter, so a leftover
                // tracker has to be stopped here. The unit path does this
                // inside the new process instead.
                stat_tracker::replace::stop_strays(
                    &data_dir,
                    false,
                    stat_tracker::replace::StrayPolicy::Any,
                )
                .await?;
                start_daemon_checked(&data_dir).await?;
            }
            Ok("Tracker started".into())
        }
        DaemonVerb::Stop => {
            stop_running(&data_dir, installed).await?;
            Ok("Tracker stopped".into())
        }
        DaemonVerb::Restart => {
            stop_running(&data_dir, installed).await?;
            if installed {
                let _ = systemd_action("reset-failed").await;
                systemd_action("start").await?;
            } else {
                start_daemon_checked(&data_dir).await?;
            }
            Ok("Tracker restarted".into())
        }
    }
}

/// Stop the unit when it exists, then any identity-checked tracker that is
/// still alive. An explicit Stop does not wait for a game to finish.
async fn stop_running(data_dir: &Path, installed: bool) -> Result<(), String> {
    if installed {
        let _ = systemd_action("stop").await;
    }
    let before = stat_tracker::replace::live_stray_pids(data_dir);
    if before.is_empty() && !installed {
        return Err("Tracker is not running".into());
    }
    stat_tracker::replace::stop_strays(data_dir, false, stat_tracker::replace::StrayPolicy::Any)
        .await
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
        assert!(!comm_is_tracker("scuffed-station"));
        assert!(!comm_is_tracker("scuffed-stat-tracker-helper"));
        assert!(!comm_is_tracker("stat-tracker-gu"));
        assert!(!comm_is_tracker("firefox"));
        assert!(!comm_is_tracker(""));
        assert!(!comm_is_tracker("scuffed"));
        assert_eq!(TRACKER_COMM, "scuffed-stat-tr");
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
        assert!(
            !pid_file(dir.path()).exists(),
            "status refresh still clears a stale pid file; stop does not"
        );
        std::fs::write(pid_file(dir.path()), "4294967295").unwrap();
        assert!(daemon_running(dir.path()).is_none());
        assert!(!pid_file(dir.path()).exists());
    }

    #[tokio::test]
    async fn stop_refuses_foreign_pid_and_leaves_the_pid_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let contents = format!("{}", std::process::id());
        std::fs::write(pid_file(dir.path()), &contents).unwrap();
        let err = stop_daemon(dir.path()).await.expect_err("must refuse");
        assert!(
            matches!(err, StopError::Refused(_)),
            "unexpected refusal: {err}"
        );
        let err = err.to_string();
        assert!(
            err.contains("not the tracker") || err.contains("this window"),
            "unexpected refusal: {err}"
        );
        assert_eq!(
            std::fs::read_to_string(pid_file(dir.path())).unwrap(),
            contents,
            "stop must not unlink a pid file it did not take down"
        );
    }

    #[tokio::test]
    async fn stop_of_stale_pid_does_not_unlink() {
        let dir = tempfile::tempdir().expect("tempdir");
        std::fs::write(pid_file(dir.path()), "4294967295").unwrap();
        let err = stop_daemon(dir.path()).await.expect_err("stale");
        assert!(matches!(err, StopError::Refused(_)), "{err}");
        let err = err.to_string();
        assert!(err.contains("not the tracker"), "{err}");
        assert_eq!(
            std::fs::read_to_string(pid_file(dir.path()))
                .unwrap()
                .trim(),
            "4294967295"
        );
    }

    #[tokio::test]
    async fn stop_without_pid_file_is_a_clean_error() {
        let dir = tempfile::tempdir().expect("tempdir");
        let err = stop_daemon(dir.path()).await.expect_err("no pid");
        assert!(matches!(err, StopError::NotRunning), "{err}");
        let err = err.to_string();
        assert!(err.contains("not running"), "{err}");
    }

    #[tokio::test]
    async fn wait_returns_immediately_when_pid_is_already_gone() {
        let started = std::time::Instant::now();
        wait_until_tracker_gone(u32::MAX, STOP_WAIT_TIMEOUT, STOP_POLL)
            .await
            .expect("dead pid is not the tracker");
        assert!(
            started.elapsed() < Duration::from_secs(2),
            "a dead pid must not sit out the shutdown timeout"
        );
    }

    #[tokio::test]
    async fn wait_returns_once_tracker_predicate_clears() {
        let mut checks = 0u32;
        wait_while(
            || {
                checks += 1;
                checks < 3
            },
            Duration::from_secs(2),
            Duration::from_millis(10),
        )
        .await
        .expect("cleared");
        assert!(checks >= 3, "checks={checks}");
    }

    #[tokio::test]
    async fn wait_times_out_while_tracker_predicate_stays_set() {
        let err = wait_while(
            || true,
            Duration::from_millis(40),
            Duration::from_millis(10),
        )
        .await
        .expect_err("must time out");
        let msg = err.to_string();
        assert!(
            msg.contains("still shutting down"),
            "timeout must refuse to start another copy: {msg}"
        );
    }

    #[test]
    fn systemctl_show_covers_a_packaged_unit_and_a_missing_one() {
        let loaded = parse_systemctl_show(
            "\
LoadState=loaded
FragmentPath=/usr/lib/systemd/user/scuffed-stat-tracker.service
ActiveState=failed
Result=start-limit-hit
",
        );
        assert!(loaded.installed());
        assert!(loaded.needs_reset());
        assert_eq!(
            loaded.fragment_path,
            "/usr/lib/systemd/user/scuffed-stat-tracker.service"
        );
        let missing =
            parse_systemctl_show("LoadState=not-found\nFragmentPath=\nActiveState=inactive\n");
        assert!(!missing.installed());
        assert!(!missing.needs_reset());
        let config = parse_systemctl_show(
            "LoadState=loaded\nFragmentPath=/home/player/.config/systemd/user/scuffed-stat-tracker.service\n",
        );
        assert!(config.installed());
        assert!(!config.fragment_path.contains("session.env"));
    }

    #[test]
    fn unit_scan_finds_a_packaged_file_and_ignores_other_names() {
        let dir = tempfile::tempdir().unwrap();
        let unit = dir.path().join(SYSTEMD_UNIT);
        std::fs::write(
            &unit,
            "[Service]\nExecStart=/usr/bin/scuffed-stat-tracker\n",
        )
        .unwrap();
        let other = dir.path().join("other.service");
        std::fs::write(&other, "").unwrap();
        assert_eq!(
            unit_in_dirs(&[dir.path().to_path_buf()], SYSTEMD_UNIT),
            Some(unit)
        );
        assert!(unit_in_dirs(&[dir.path().join("missing")], SYSTEMD_UNIT).is_none());
        assert!(
            user_unit_dirs()
                .iter()
                .any(|path| { path.ends_with("systemd/user") || path.ends_with("systemd/user/") })
        );
    }

    #[test]
    fn restart_proceeds_only_when_nothing_live_was_signalled() {
        assert!(StopError::NotRunning.allows_restart());
        assert!(
            StopError::Refused("Saved process id is not the tracker, not stopping it")
                .allows_restart()
        );
        assert!(
            !StopError::Failed("Tracker is still shutting down, not starting another copy".into())
                .allows_restart()
        );
    }
}
