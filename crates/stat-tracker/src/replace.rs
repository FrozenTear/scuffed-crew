//! Replace a leftover tracker, and restart this process when its binary
//! on disk was replaced.
//!
//! Identity is the same rule as the pid file (`proc_id`): the executable
//! name has to be the daemon, and a recorded start time has to match.
//! A foreign process is never signalled. This process is never signalled.

use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::proc_id::{self, PidRecord};

/// `active_game.json` older than this is not a game in progress.
pub const GAME_FILE_STALE: Duration = Duration::from_secs(2 * 60 * 60);

/// How often a waiter rechecks `active_game.json`.
pub const GAME_WAIT_STEP: Duration = Duration::from_secs(10);

/// The longest an automatic replace waits for the open game to be saved.
pub const GAME_WAIT_MAX: Duration = Duration::from_secs(60 * 60);

/// How long a stop waits for the process to leave after SIGTERM.
pub const STOP_WAIT: Duration = Duration::from_secs(90);

/// Exit status the daemon uses when its binary was replaced and systemd
/// should start the new one. Non-zero, so `Restart=on-failure` covers it.
/// `systemctl stop` still exits 0 and stays stopped.
pub const RESTART_EXIT_CODE: i32 = 75;

/// Set on the re-exec'd process so a stamp that still looks replaced
/// cannot restart again in a loop.
const RESTART_ENV: &str = "STAT_TRACKER_BINARY_RESTART";

/// The tracker has a game open. A file not written for [`GAME_FILE_STALE`]
/// is left over and does not count. A clock that moved backwards counts
/// as in progress so we do not cut a game we cannot time.
pub fn game_file_in_progress(data_dir: &Path) -> bool {
    std::fs::metadata(data_dir.join("active_game.json"))
        .and_then(|m| m.modified())
        .is_ok_and(|t| t.elapsed().map_or(true, |age| age < GAME_FILE_STALE))
}

/// One process that might be a leftover tracker.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrayCandidate {
    pub pid: u32,
    pub image_ok: bool,
    pub is_self: bool,
    pub expected_start: Option<u64>,
    pub actual_start: Option<u64>,
}

/// True when this candidate is a live tracker we are allowed to stop.
///
/// Self is never stopped. A reused pid of the same binary name fails the
/// start-time check inside [`proc_id::accept_daemon_process`].
pub fn should_stop_stray(candidate: &StrayCandidate) -> bool {
    if candidate.is_self || candidate.pid == 0 {
        return false;
    }
    proc_id::accept_daemon_process(
        candidate.image_ok,
        candidate.expected_start,
        candidate.actual_start,
    )
}

/// Pids from a synthetic or real `/proc/locks` text that hold a write lock
/// on `inode`. `FLOCK` and `POSIX` advisory write locks both count. Read
/// locks do not.
pub fn flock_pids_for_inode(text: &str, inode: u64) -> Vec<u32> {
    let mut pids = Vec::new();
    for line in text.lines() {
        let Some(holder) = parse_lock_line(line) else {
            continue;
        };
        if holder.write && holder.inode == inode {
            pids.push(holder.pid);
        }
    }
    pids.sort_unstable();
    pids.dedup();
    pids
}

struct LockHolder {
    pid: u32,
    inode: u64,
    write: bool,
}

fn parse_lock_line(line: &str) -> Option<LockHolder> {
    // `1: FLOCK  ADVISORY  WRITE 4242 00:12:999 0 EOF`
    // Field 5 is the pid. Field 6 is major:minor:inode.
    let mut parts = line.split_whitespace();
    let _id = parts.next()?;
    let kind = parts.next()?;
    let _mode = parts.next()?;
    let access = parts.next()?;
    if kind != "FLOCK" && kind != "POSIX" {
        return None;
    }
    let pid: u32 = parts.next()?.parse().ok()?;
    let device = parts.next()?;
    let inode = device.rsplit_once(':')?.1.parse().ok()?;
    Some(LockHolder {
        pid,
        inode,
        write: access == "WRITE",
    })
}

/// Inode of the store directory the daemon flocks, when that directory exists.
pub fn store_dir_inode(data_dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        std::fs::metadata(data_dir.join("stats.surrealkv"))
            .ok()
            .map(|meta| meta.ino())
    }
    #[cfg(not(unix))]
    {
        let _ = data_dir;
        None
    }
}

/// Live tracker pids from the pid file and the store-directory lock.
///
/// The pid file is only a candidate when its record still names a live
/// tracker. Lock holders are checked the same way. This process is omitted.
pub fn live_stray_pids(data_dir: &Path) -> Vec<u32> {
    let mut pids = Vec::new();
    if let Some(rec) = read_pid_record(data_dir) {
        pids.push(rec.pid);
    }
    if let Some(inode) = store_dir_inode(data_dir)
        && let Ok(text) = std::fs::read_to_string("/proc/locks")
    {
        pids.extend(flock_pids_for_inode(&text, inode));
    }
    pids.sort_unstable();
    pids.dedup();
    pids.into_iter()
        .filter(|&pid| {
            let candidate = candidate_for_pid(pid, read_pid_record(data_dir));
            should_stop_stray(&candidate)
        })
        .collect()
}

fn read_pid_record(data_dir: &Path) -> Option<PidRecord> {
    let text = std::fs::read_to_string(data_dir.join("daemon.pid")).ok()?;
    proc_id::parse_pid_record(&text)
}

fn candidate_for_pid(pid: u32, record: Option<PidRecord>) -> StrayCandidate {
    let expected = record
        .filter(|rec| rec.pid == pid)
        .and_then(|rec| rec.start_ticks);
    StrayCandidate {
        pid,
        image_ok: proc_id::image_matches(pid),
        is_self: pid == std::process::id(),
        expected_start: expected,
        actual_start: proc_id::proc_start_ticks(pid),
    }
}

fn pids_to_stop(data_dir: &Path, policy: StrayPolicy) -> Vec<u32> {
    live_stray_pids(data_dir)
        .into_iter()
        .filter(|&pid| !cmdline_is_vacuum(&read_cmdline(pid)))
        .filter(|&pid| match policy {
            StrayPolicy::Any => true,
            StrayPolicy::ReplacedBinary => binary_differs_from_self(pid),
        })
        .collect()
}

fn read_cmdline(pid: u32) -> Vec<u8> {
    std::fs::read(format!("/proc/{pid}/cmdline")).unwrap_or_default()
}

/// True when one argument of a NUL-separated `/proc/<pid>/cmdline` is
/// `--vacuum`. Compacting holds the store lock and must not be signalled.
pub fn cmdline_is_vacuum(bytes: &[u8]) -> bool {
    bytes.split(|byte| *byte == 0).any(|arg| arg == b"--vacuum")
}

/// True when `pid`'s executable inode, mtime, or length differs from this
/// process. Unreadable `/proc` entries are not treated as different.
fn binary_differs_from_self(pid: u32) -> bool {
    let Some(ours) = exe_stamp(Path::new("/proc/self/exe")) else {
        return false;
    };
    let Some(theirs) = exe_stamp(Path::new(&format!("/proc/{pid}/exe"))) else {
        return false;
    };
    ours != theirs
}

fn exe_stamp(path: &Path) -> Option<ExeStamp> {
    stamp_of_meta(&std::fs::metadata(path).ok()?)
}

/// Which leftover trackers a caller is allowed to stop.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StrayPolicy {
    /// Every identity-checked tracker. Explicit Stop and Restart.
    Any,
    /// Only a tracker whose executable stamp differs from this process.
    /// A second start of the same binary is left for the pid guard.
    ReplacedBinary,
}

/// `None` for `--vacuum`: compacting must not signal the live daemon.
/// The service sets `replace_same` so a start replaces an app-started
/// copy of the same binary. Any other start only replaces a different
/// binary, which is what an update leaves behind.
pub fn policy_for_startup(vacuum: bool, replace_same: bool) -> Option<StrayPolicy> {
    if vacuum {
        None
    } else if replace_same {
        Some(StrayPolicy::Any)
    } else {
        Some(StrayPolicy::ReplacedBinary)
    }
}

/// Stop identity-checked leftover trackers.
///
/// `wait_for_game` waits until `active_game.json` is no longer a game in
/// progress (or [`GAME_WAIT_MAX`]) before signalling, so an update does not
/// cut a game that is not saved yet. An explicit Stop passes `false`.
///
/// Returns an error when a tracker is still alive afterwards. The caller
/// must not start another copy.
pub async fn stop_strays(
    data_dir: &Path,
    wait_for_game: bool,
    policy: StrayPolicy,
) -> Result<(), String> {
    let pids = pids_to_stop(data_dir, policy);
    if pids.is_empty() {
        return Ok(());
    }
    if wait_for_game && game_file_in_progress(data_dir) {
        tracing::info!(
            pids = ?pids,
            "previous tracker is in a game; waiting for it to be saved before replacing it"
        );
        let mut waited = Duration::ZERO;
        while game_file_in_progress(data_dir) && waited < GAME_WAIT_MAX {
            tokio::time::sleep(GAME_WAIT_STEP).await;
            waited += GAME_WAIT_STEP;
            if pids_to_stop(data_dir, policy).is_empty() {
                return Ok(());
            }
        }
        if game_file_in_progress(data_dir) {
            tracing::info!(
                "waited an hour for the game to be saved; stopping the previous tracker"
            );
        }
    }
    let pids = pids_to_stop(data_dir, policy);
    for pid in &pids {
        signal_term(*pid);
    }
    let started = tokio::time::Instant::now();
    loop {
        let still = pids_to_stop(data_dir, policy);
        if still.is_empty() {
            return Ok(());
        }
        if started.elapsed() >= STOP_WAIT {
            let list = still
                .iter()
                .map(|pid| pid.to_string())
                .collect::<Vec<_>>()
                .join(", ");
            return Err(format!(
                "Tracker is still running (PID {list}). It was not stopped, so another copy was not started."
            ));
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
}

fn signal_term(pid: u32) {
    #[cfg(unix)]
    {
        // SAFETY: `kill` sends SIGTERM to one pid. The caller already
        // checked that pid is the tracker binary, not this process.
        let rc = unsafe { libc::kill(pid as i32, libc::SIGTERM) };
        if rc != 0 {
            let err = std::io::Error::last_os_error();
            tracing::warn!(pid, error = %err, "could not signal the previous tracker");
        }
    }
    #[cfg(not(unix))]
    let _ = pid;
}

/// Identity of an executable inode: device inode, mtime (seconds), length.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ExeStamp {
    pub inode: u64,
    pub mtime_secs: u64,
    pub len: u64,
}

/// The file on disk is a different inode, mtime, or length than the one
/// this process is running. `None` for the on-disk stamp means there is
/// nothing to switch to, which is not a replacement.
pub fn binary_replaced(running: ExeStamp, on_disk: Option<ExeStamp>) -> bool {
    match on_disk {
        Some(disk) => running != disk,
        None => false,
    }
}

/// How a replaced binary is picked up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartHow {
    /// Exit [`RESTART_EXIT_CODE`] so systemd `Restart=on-failure` starts
    /// the unit's `ExecStart` again.
    SystemdExit,
    /// `exec` the on-disk binary. Used when this process is not a service.
    Reexec,
}

pub fn restart_how(under_systemd: bool) -> RestartHow {
    if under_systemd {
        RestartHow::SystemdExit
    } else {
        RestartHow::Reexec
    }
}

/// The unit sets `STAT_TRACKER_SYSTEMD=1`. `INVOCATION_ID` is inherited by
/// an app that itself was started under systemd, and must not count.
pub fn under_systemd_env(value: Option<&str>) -> bool {
    value == Some("1")
}

pub fn under_systemd() -> bool {
    under_systemd_env(std::env::var("STAT_TRACKER_SYSTEMD").ok().as_deref())
}

/// How long SIGINT and SIGTERM wait before the process continues the exit
/// anyway. The log names what was still in flight.
pub const SHUTDOWN_DEADLINE: Duration = Duration::from_secs(5);

pub fn shutdown_overrun_line(capture: bool, sync: bool, screencast: bool) -> String {
    let mut parts = Vec::new();
    if capture {
        parts.push("in-flight capture");
    }
    if sync {
        parts.push("in-flight sync");
    }
    if screencast {
        parts.push("screencast session");
    }
    let what = if parts.is_empty() {
        "shutdown".to_string()
    } else {
        parts.join(", ")
    };
    format!("shutdown deadline exceeded; exiting while still running: {what}")
}

/// A failed re-exec keeps this process. The systemd exit does not.
pub fn keeps_running_after_restart(how: RestartHow, exec_failed: bool) -> bool {
    match how {
        RestartHow::SystemdExit => false,
        RestartHow::Reexec => exec_failed,
    }
}

/// Remove `daemon.pid` when it names this process. `process::exit` and
/// `exec` skip the guard's destructor.
pub fn release_own_pid_file(data_dir: &Path) -> bool {
    let path = data_dir.join("daemon.pid");
    let Ok(text) = std::fs::read_to_string(&path) else {
        return false;
    };
    let Some(rec) = proc_id::parse_pid_record(&text) else {
        return false;
    };
    if rec.pid != std::process::id() {
        return false;
    }
    std::fs::remove_file(&path).is_ok()
}

pub fn restore_own_pid_file(data_dir: &Path) {
    let path = data_dir.join("daemon.pid");
    let pid = std::process::id();
    let body = proc_id::format_pid_record(pid, proc_id::proc_start_ticks(pid));
    if std::fs::write(&path, body).is_ok() {
        crate::fs_mode::tighten_private_file(&path);
    }
}

/// What Restart now in the app should do.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartGuiPlan {
    Wait,
    RestartTracker,
    RelaunchGui,
}

pub fn plan_restart_gui(game_open: bool, binary_replaced: bool) -> RestartGuiPlan {
    if game_open {
        RestartGuiPlan::Wait
    } else if binary_replaced {
        RestartGuiPlan::RestartTracker
    } else {
        RestartGuiPlan::RelaunchGui
    }
}

/// `1`, `true`, or `yes`. `0`, empty, and unset are off.
pub fn flag_enabled(value: Option<&str>) -> bool {
    matches!(
        value.map(str::trim),
        Some("1" | "true" | "TRUE" | "yes" | "YES")
    )
}

/// `Some(path)` when `/proc/self/exe` and the file at that path differ.
/// The path is the on-disk binary to exec, with a kernel ` (deleted)`
/// suffix removed. A process that already exec'd this exact on-disk stamp
/// returns `None`. A later stamp is picked up.
pub fn replaced_binary_path() -> Option<PathBuf> {
    let link = std::fs::read_link("/proc/self/exe").ok()?;
    let running = stamp_of_meta(&std::fs::metadata("/proc/self/exe").ok()?)?;
    let path = on_disk_path(&link);
    let disk = std::fs::metadata(&path)
        .ok()
        .and_then(|meta| stamp_of_meta(&meta))?;
    if !binary_replaced(running, Some(disk)) {
        return None;
    }
    let remembered = std::env::var(RESTART_ENV).ok();
    if same_exec_stamp(remembered.as_deref(), &disk) {
        return None;
    }
    Some(path)
}

pub fn exe_stamp_token(stamp: &ExeStamp) -> String {
    format!("{}:{}:{}", stamp.inode, stamp.mtime_secs, stamp.len)
}

pub fn same_exec_stamp(remembered: Option<&str>, disk: &ExeStamp) -> bool {
    remembered.is_some_and(|raw| raw == exe_stamp_token(disk))
}

fn on_disk_path(link: &Path) -> PathBuf {
    let lossy = link.to_string_lossy();
    let trimmed = lossy.strip_suffix(" (deleted)").unwrap_or(lossy.as_ref());
    PathBuf::from(trimmed)
}

fn stamp_of_meta(meta: &std::fs::Metadata) -> Option<ExeStamp> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::MetadataExt;
        let mtime_secs = meta
            .modified()
            .ok()?
            .duration_since(std::time::UNIX_EPOCH)
            .ok()?
            .as_secs();
        Some(ExeStamp {
            inode: meta.ino(),
            mtime_secs,
            len: meta.len(),
        })
    }
    #[cfg(not(unix))]
    {
        let _ = meta;
        None
    }
}

/// Stamp of a running pid's executable, and of the path it names on disk.
/// `None` when `/proc/<pid>/exe` cannot be read.
pub fn pid_binary_replaced(pid: u32) -> Option<bool> {
    let link = std::fs::read_link(format!("/proc/{pid}/exe")).ok()?;
    let running = stamp_of_meta(&std::fs::metadata(format!("/proc/{pid}/exe")).ok()?)?;
    let disk = std::fs::metadata(on_disk_path(&link))
        .ok()
        .and_then(|meta| stamp_of_meta(&meta));
    Some(binary_replaced(running, disk))
}

/// Log one line and either exit for systemd or re-exec the on-disk binary.
/// Does not return on success.
pub fn restart_for_replaced_binary(path: &Path) -> anyhow::Result<()> {
    match restart_how(under_systemd()) {
        RestartHow::SystemdExit => {
            tracing::info!(
                "tracker binary on disk was replaced; exiting so systemd starts the new version"
            );
            std::process::exit(RESTART_EXIT_CODE);
        }
        RestartHow::Reexec => {
            tracing::info!(
                path = %path.display(),
                "tracker binary on disk was replaced; re-executing the new version"
            );
            #[cfg(unix)]
            {
                use std::os::unix::process::CommandExt;
                let mut cmd = std::process::Command::new(path);
                cmd.args(std::env::args_os().skip(1));
                if let Some(stamp) = std::fs::metadata(path)
                    .ok()
                    .and_then(|meta| stamp_of_meta(&meta))
                {
                    cmd.env(RESTART_ENV, exe_stamp_token(&stamp));
                }
                let err = cmd.exec();
                anyhow::bail!("could not re-exec the new tracker: {err}");
            }
            #[cfg(not(unix))]
            {
                anyhow::bail!("re-exec is only supported on unix");
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn candidate(
        image_ok: bool,
        is_self: bool,
        expected: Option<u64>,
        actual: Option<u64>,
    ) -> StrayCandidate {
        StrayCandidate {
            pid: 4242,
            image_ok,
            is_self,
            expected_start: expected,
            actual_start: actual,
        }
    }

    #[test]
    fn startup_policy_spares_vacuum_and_a_same_binary_second_start() {
        assert_eq!(policy_for_startup(true, true), None);
        assert_eq!(policy_for_startup(true, false), None);
        assert_eq!(
            policy_for_startup(false, false),
            Some(StrayPolicy::ReplacedBinary)
        );
        assert_eq!(policy_for_startup(false, true), Some(StrayPolicy::Any));
    }

    #[test]
    fn stray_identity_refuses_self_and_a_foreign_or_reused_pid() {
        assert!(should_stop_stray(&candidate(true, false, None, None)));
        assert!(should_stop_stray(&candidate(
            true,
            false,
            Some(10),
            Some(10)
        )));
        assert!(
            !should_stop_stray(&candidate(true, true, None, None)),
            "this process is never stopped"
        );
        assert!(
            !should_stop_stray(&candidate(false, false, None, Some(10))),
            "a different executable is not the tracker"
        );
        assert!(
            !should_stop_stray(&candidate(true, false, Some(10), Some(11))),
            "same binary name, different start time, is a reused pid"
        );
        assert!(!should_stop_stray(&candidate(true, false, Some(10), None)));
        let mut zero = candidate(true, false, None, None);
        zero.pid = 0;
        assert!(!should_stop_stray(&zero));
    }

    #[test]
    fn flock_parser_keeps_write_locks_on_the_store_inode_only() {
        let text = "\
1: FLOCK  ADVISORY  WRITE 4242 08:01:999 0 EOF
2: FLOCK  ADVISORY  READ  1111 08:01:999 0 EOF
3: POSIX  ADVISORY  WRITE 4242 08:01:999 0 EOF
4: FLOCK  ADVISORY  WRITE 5555 08:01:1000 0 EOF
5: LEASE  ADVISORY  WRITE 6666 08:01:999 0 EOF
not a lock line
";
        assert_eq!(flock_pids_for_inode(text, 999), vec![4242]);
        assert!(flock_pids_for_inode(text, 7).is_empty());
        assert_eq!(flock_pids_for_inode("", 999).len(), 0);
    }

    #[test]
    fn replaced_binary_compares_inode_mtime_and_length() {
        let running = ExeStamp {
            inode: 10,
            mtime_secs: 20,
            len: 30,
        };
        assert!(!binary_replaced(running, Some(running)));
        assert!(binary_replaced(
            running,
            Some(ExeStamp {
                inode: 11,
                ..running
            })
        ));
        assert!(binary_replaced(
            running,
            Some(ExeStamp {
                mtime_secs: 21,
                ..running
            })
        ));
        assert!(binary_replaced(
            running,
            Some(ExeStamp { len: 31, ..running })
        ));
        assert!(
            !binary_replaced(running, None),
            "a missing file is not a new binary to switch to"
        );
    }

    #[test]
    fn systemd_exit_and_reexec_are_chosen_from_the_service_flag() {
        assert_eq!(restart_how(true), RestartHow::SystemdExit);
        assert_eq!(restart_how(false), RestartHow::Reexec);
        assert_eq!(RESTART_EXIT_CODE, 75);
        assert!(under_systemd_env(Some("1")));
        assert!(!under_systemd_env(Some("0")));
        assert!(flag_enabled(Some("1")));
        assert!(flag_enabled(Some("true")));
        assert!(!flag_enabled(Some("0")));
        assert!(!flag_enabled(Some("")));
        assert!(!flag_enabled(None));
        let stamp = ExeStamp {
            inode: 7,
            mtime_secs: 8,
            len: 9,
        };
        let token = exe_stamp_token(&stamp);
        assert!(same_exec_stamp(Some(&token), &stamp));
        assert!(!same_exec_stamp(Some("1"), &stamp));
        let newer = ExeStamp {
            mtime_secs: 10,
            ..stamp
        };
        assert!(!same_exec_stamp(Some(&token), &newer));
        assert!(!under_systemd_env(None));
        assert!(!keeps_running_after_restart(RestartHow::SystemdExit, true));
        assert!(keeps_running_after_restart(RestartHow::Reexec, true));
        assert!(!keeps_running_after_restart(RestartHow::Reexec, false));
        assert_eq!(SHUTDOWN_DEADLINE, Duration::from_secs(5));
        assert!(shutdown_overrun_line(true, false, true).contains("in-flight capture"));
        assert!(cmdline_is_vacuum(b"scuffed-stat-tracker\0--vacuum\0"));
        assert!(!cmdline_is_vacuum(b"scuffed-stat-tracker\0"));
        assert_eq!(plan_restart_gui(true, true), RestartGuiPlan::Wait);
        assert_eq!(
            plan_restart_gui(false, true),
            RestartGuiPlan::RestartTracker
        );
        assert_eq!(plan_restart_gui(false, false), RestartGuiPlan::RelaunchGui);
    }

    #[test]
    fn game_file_staleness_ignores_an_old_or_missing_file() {
        let dir = tempfile::tempdir().unwrap();
        assert!(!game_file_in_progress(dir.path()));
        let path = dir.path().join("active_game.json");
        std::fs::write(&path, b"{}").unwrap();
        assert!(game_file_in_progress(dir.path()));
        let old = std::time::SystemTime::now() - GAME_FILE_STALE - Duration::from_secs(60);
        filetime_set(&path, old);
        assert!(!game_file_in_progress(dir.path()));
    }

    fn filetime_set(path: &Path, when: std::time::SystemTime) {
        let secs = when
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64;
        let rc = std::process::Command::new("touch")
            .arg("-d")
            .arg(format!("@{secs}"))
            .arg(path)
            .status();
        assert!(rc.unwrap().success());
    }
}
