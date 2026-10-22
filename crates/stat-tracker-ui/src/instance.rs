//! One tracker GUI per user session (#234).
//!
//! The first GUI takes an exclusive `flock` on
//! `$XDG_RUNTIME_DIR/scuffed-stat-tracker-gui.lock` (the data dir when
//! `XDG_RUNTIME_DIR` is unset), writes its pid and version into it, and keeps
//! the file open until it exits.
//!
//! A later launch that finds the lock held:
//! - shows the running GUI: it touches `<data_dir>/show_window` and exits 0;
//! - or, when it replaces the running GUI (`--replace` from "Restart now", or
//!   the holder is an older binary), touches `<data_dir>/quit_window` and
//!   waits a few seconds for the lock.
//!
//! The running GUI polls both files on its Tick. `show_window` runs the tray
//! "Show" path and `quit_window` runs the tray "Quit" path. While an in-app
//! update runs, the GUI leaves `quit_window` unpolled: quitting would drop
//! the update, which stops the installer half way with the tracker stopped.
//!
//! A replacing launch removes `quit_window` once its wait ends, whether it
//! got the lock or not, and a new primary removes both request files left
//! by an earlier session. So a stale request never fires, even after the
//! wall clock steps back past its mtime.
//!
//! The kernel drops a `flock` when the holder dies, including on SIGKILL or
//! a crash, so a stale lock file never blocks a launch. Any error while
//! taking the lock lets the launch go ahead: a second window is better than
//! no window.

use std::ffi::OsString;
use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read, Seek, Write};
use std::os::fd::{AsRawFd as _, FromRawFd as _};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

pub const LOCK_FILE: &str = "scuffed-stat-tracker-gui.lock";
/// The GUI binary under `<prefix>/bin`, as `install.sh` installs it.
pub const GUI_BIN: &str = "stat-tracker-gui";
pub const SHOW_FILE: &str = "show_window";
pub const QUIT_FILE: &str = "quit_window";

/// How long a replacing launch waits for the old GUI to exit.
pub const REPLACE_WAIT: Duration = Duration::from_secs(10);

/// Outcome of [`claim`].
#[derive(Debug)]
pub enum Claim {
    /// This process is the GUI. Keep the file open for the process lifetime.
    Primary(File),
    /// Another GUI holds the lock.
    Secondary,
}

/// The GUI that holds the lock, as written into the lock file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Holder {
    pub pid: u32,
    pub version: String,
}

/// When this process became the primary GUI. Requests older than this were
/// meant for the previous holder.
static ACQUIRED_AT: OnceLock<SystemTime> = OnceLock::new();

/// Directory for the lock file: a non-empty `XDG_RUNTIME_DIR`, else the
/// data dir.
pub fn lock_dir(runtime_dir: Option<OsString>, data_dir: &Path) -> PathBuf {
    match runtime_dir {
        Some(dir) if !dir.is_empty() => PathBuf::from(dir),
        _ => data_dir.to_path_buf(),
    }
}

pub fn lock_path(runtime_dir: Option<OsString>, data_dir: &Path) -> PathBuf {
    lock_dir(runtime_dir, data_dir).join(LOCK_FILE)
}

/// Try to become the one GUI. Never blocks. On success the lock file holds
/// `<pid>\n<version>\n`.
pub fn claim(lock_path: &Path, version: &str) -> io::Result<Claim> {
    let file = open_lock(lock_path)?;
    try_claim(file, version).map(|(claim, _)| claim)
}

fn open_lock(lock_path: &Path) -> io::Result<File> {
    if let Some(parent) = lock_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(lock_path)
}

/// [`claim`] on an open lock file. On [`Claim::Secondary`] the file comes
/// back, so a launch that retries keeps it open between tries: a GUI with
/// the lock file open is in the claim flow, and [`stale_gui_targets`]
/// leaves it alone.
fn try_claim(mut file: File, version: &str) -> io::Result<(Claim, Option<File>)> {
    match file.try_lock() {
        Ok(()) => {
            let _ = ACQUIRED_AT.set(SystemTime::now());
            // Best effort: a holder the next launch cannot read is treated
            // as current, which only means "show it".
            let _ = file
                .set_len(0)
                .and_then(|()| file.rewind())
                .and_then(|()| write!(file, "{}\n{}\n", std::process::id(), version.trim()))
                .and_then(|()| file.flush());
            Ok((Claim::Primary(file), None))
        }
        Err(TryLockError::WouldBlock) => Ok((Claim::Secondary, Some(file))),
        Err(TryLockError::Error(e)) => Err(e),
    }
}

/// The launch decision for `main`: take the lock, or replace an older
/// holder (`replace` is `--replace` from "Restart now"), or report
/// [`Claim::Secondary`] so the caller asks the holder to show. A new primary
/// removes request files left by an earlier session.
pub fn claim_session(
    lock_path: &Path,
    data_dir: &Path,
    version: &str,
    replace: bool,
    proc_root: &Path,
    wait: Duration,
) -> io::Result<Claim> {
    let claimed = match claim(lock_path, version)? {
        Claim::Secondary => {
            // Replace the running GUI on "Restart now" or when it is an
            // older binary (left over from an update). Otherwise the caller
            // just brings its window up.
            let older = read_holder(lock_path)
                .is_some_and(|holder| holder_is_older(&holder, version, proc_root));
            if replace || older {
                tracing::info!(lock = %lock_path.display(), older, "replacing the running tracker GUI");
                claim_replacing(lock_path, data_dir, version, wait)?
            } else {
                Claim::Secondary
            }
        }
        primary => primary,
    };
    if matches!(claimed, Claim::Primary(_)) {
        clear_stale_requests(data_dir);
    }
    Ok(claimed)
}

/// Ask the holder to quit, then retry [`claim`] until `wait` runs out.
/// The quit request is removed when the wait ends, with or without the
/// lock: this launch is the only one that needed it. Left behind, it would
/// make a holder that answers late quit with no GUI to follow it, or make a
/// later GUI quit after the wall clock stepped back past its mtime.
pub fn claim_replacing(
    lock_path: &Path,
    data_dir: &Path,
    version: &str,
    wait: Duration,
) -> io::Result<Claim> {
    // One open file for every try, so this launch has the lock file open
    // for its whole wait (see [`try_claim`]).
    let mut file = open_lock(lock_path)?;
    request_quit(data_dir)?;
    let start = Instant::now();
    let claimed = loop {
        match try_claim(file, version) {
            Ok((Claim::Secondary, Some(open))) if start.elapsed() < wait => {
                file = open;
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok((claim, _)) => break Ok(claim),
            Err(e) => break Err(e),
        }
    };
    let _ = std::fs::remove_file(quit_path(data_dir));
    claimed
}

/// Remove `show_window` and `quit_window`. A new primary calls this right
/// after taking the lock, so any request file it sees later was written for
/// it, whatever the wall clock did since. Best effort: a file that cannot be
/// removed is still filtered by its mtime ([`RequestWatch`]).
pub fn clear_stale_requests(data_dir: &Path) {
    for path in [show_path(data_dir), quit_path(data_dir)] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == io::ErrorKind::NotFound => {}
            Err(e) => {
                tracing::warn!(error = %e, path = %path.display(), "could not remove a stale GUI request");
            }
        }
    }
}

pub fn read_holder(lock_path: &Path) -> Option<Holder> {
    let mut raw = String::new();
    File::open(lock_path).ok()?.read_to_string(&mut raw).ok()?;
    let mut lines = raw.lines();
    let pid = lines.next()?.trim().parse().ok()?;
    let version = lines.next().unwrap_or_default().trim().to_string();
    Some(Holder { pid, version })
}

/// True when the holder runs an older binary than `my_version`: its
/// executable was replaced on disk (`/proc/<pid>/exe` ends in
/// ` (deleted)`), or its recorded version is lower. An unreadable or equal
/// holder is not older, so the launch only shows it.
pub fn holder_is_older(holder: &Holder, my_version: &str, proc_root: &Path) -> bool {
    let exe = proc_root.join(holder.pid.to_string()).join("exe");
    if let Ok(target) = std::fs::read_link(&exe)
        && target.to_string_lossy().ends_with(" (deleted)")
    {
        return true;
    }
    match (
        crate::update::parse_semver(&holder.version),
        crate::update::parse_semver(my_version),
    ) {
        (Some(theirs), Some(mine)) => theirs < mine,
        _ => false,
    }
}

/// Path to start the new binary from. After an update `/proc/self/exe`
/// reads `<path> (deleted)`; the new file sits at `<path>`.
pub fn relaunch_path(current_exe: &Path) -> PathBuf {
    let raw = current_exe.to_string_lossy();
    match raw.strip_suffix(" (deleted)") {
        Some(path) => PathBuf::from(path),
        None => current_exe.to_path_buf(),
    }
}

/// Binary for "Restart now": `<install_prefix>/bin/stat-tracker-gui`, the
/// file the update just installed, when it exists. Otherwise (no prefix, no
/// file there) the running binary's path, as [`relaunch_path`] gives it.
/// The two differ when `STAT_TRACKER_PREFIX` points elsewhere or this GUI
/// runs from outside a `bin/` directory.
pub fn relaunch_target(install_prefix: Option<&Path>, current_exe: &Path) -> PathBuf {
    let running = relaunch_path(current_exe);
    if let Some(prefix) = install_prefix {
        let installed = prefix.join("bin").join(GUI_BIN);
        if installed != running && installed.is_file() {
            return installed;
        }
    }
    running
}

/// Start the installed GUI binary with this process's arguments plus
/// `--replace`, so it waits for this process to exit and takes the lock.
/// `install_prefix` is the prefix the in-app update installed to.
pub fn spawn_replacement(install_prefix: Option<&Path>) -> io::Result<()> {
    let exe = relaunch_target(install_prefix, &std::env::current_exe()?);
    let mut args: Vec<OsString> = std::env::args_os().skip(1).collect();
    if !args.iter().any(|a| a == "--replace") {
        args.push("--replace".into());
    }
    std::process::Command::new(exe).args(args).spawn()?;
    Ok(())
}

pub fn show_path(data_dir: &Path) -> PathBuf {
    data_dir.join(SHOW_FILE)
}

pub fn quit_path(data_dir: &Path) -> PathBuf {
    data_dir.join(QUIT_FILE)
}

fn touch(path: &Path) -> io::Result<()> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)?;
    file.set_modified(SystemTime::now())
}

/// Ask the running GUI to show its window.
pub fn request_show(data_dir: &Path) -> io::Result<()> {
    touch(&show_path(data_dir))
}

/// Ask the running GUI to quit (tray Quit path).
pub fn request_quit(data_dir: &Path) -> io::Result<()> {
    touch(&quit_path(data_dir))
}

/// Watches one request file. The first poll fires when the file's mtime is
/// newer than `since` (a request that came in after this GUI took the lock
/// but before its first Tick). After that, a file that appears or whose
/// mtime moves fires once. A file left by an earlier session never fires.
#[derive(Debug)]
pub struct RequestWatch {
    path: PathBuf,
    since: SystemTime,
    /// `None` until the first poll; then the last seen mtime (`None` = no file).
    seen: Option<Option<SystemTime>>,
}

impl RequestWatch {
    pub fn new(path: PathBuf, since: SystemTime) -> Self {
        Self {
            path,
            since,
            seen: None,
        }
    }

    pub fn poll(&mut self) -> bool {
        let mtime = std::fs::metadata(&self.path)
            .and_then(|m| m.modified())
            .ok();
        let fire = match self.seen {
            None => mtime.is_some_and(|t| t > self.since),
            Some(prev) => mtime.is_some() && mtime != prev,
        };
        self.seen = Some(mtime);
        fire
    }
}

static SHOW_WATCH: Mutex<Option<RequestWatch>> = Mutex::new(None);
static QUIT_WATCH: Mutex<Option<RequestWatch>> = Mutex::new(None);

fn consume(slot: &Mutex<Option<RequestWatch>>, path: PathBuf) -> bool {
    // Only the GUI that holds the lock answers requests. A fixture run or a
    // launch that could not open the lock leaves them alone.
    let Some(since) = ACQUIRED_AT.get().copied() else {
        return false;
    };
    let mut guard = slot.lock().unwrap_or_else(|e| e.into_inner());
    guard
        .get_or_insert_with(|| RequestWatch::new(path, since))
        .poll()
}

/// For the GUI Tick: true when a later launch asked to show the window.
pub fn consume_show(data_dir: &Path) -> bool {
    consume(&SHOW_WATCH, show_path(data_dir))
}

/// For the GUI Tick: true when a newer GUI asked this one to quit.
pub fn consume_quit(data_dir: &Path) -> bool {
    consume(&QUIT_WATCH, quit_path(data_dir))
}

/// A process running the GUI binary, as [`scan_gui_processes`] read it
/// from `/proc`.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct GuiProcess {
    pub pid: u32,
    pub ppid: u32,
    /// The `/proc/<pid>/exe` target. Ends in ` (deleted)` once an update
    /// replaced the file.
    pub exe: String,
    /// Started with `--companion`: an overlay child, not a GUI.
    pub companion: bool,
    /// Started with `--fixture`: a demo run, which never takes the lock.
    pub fixture: bool,
    /// Started with `--replace`: a launch in the claim flow.
    pub replace: bool,
    /// Has the lock file open (also when that file was unlinked since).
    /// `None` when its open files could not be read.
    pub has_lock_open: Option<bool>,
    /// Start time in clock ticks after boot (`/proc/<pid>/stat` field 22).
    pub start_ticks: Option<u64>,
    /// Has a live installer child of a 0.4.x in-app update: a command line
    /// naming the update work dir (`bash <work dir>/bootstrap.sh`), or
    /// systemctl. Stopping the GUI would kill that install half way.
    /// Zombies, unreadable children and other children (its companion, a
    /// tracker it started, `xdg-open` and a browser) do not count.
    pub busy_child: bool,
    /// A 0.4.x in-app update work dir (`$TMPDIR/sst-gui-update-<pid>-*`)
    /// of this pid exists: an update is in progress.
    pub update_dir: bool,
}

/// Who this process is, for [`stale_gui_targets`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Me {
    pub pid: u32,
    pub ppid: u32,
    pub uid: u32,
    /// This process's start, in clock ticks after boot.
    pub start_ticks: u64,
    /// Clock ticks per second (`sysconf(_SC_CLK_TCK)`).
    pub clk_tck: u64,
}

impl Me {
    /// This process, from libc and `/proc/self/stat`. `None` when its start
    /// time cannot be read: then no process is stopped.
    pub fn current() -> Option<Self> {
        // SAFETY: getppid, getuid and sysconf take no pointers and cannot
        // fail in a way that matters here.
        let (ppid, uid, clk_tck) = unsafe {
            (
                libc::getppid(),
                libc::getuid(),
                libc::sysconf(libc::_SC_CLK_TCK),
            )
        };
        let stat = std::fs::read_to_string("/proc/self/stat").ok()?;
        Some(Self {
            pid: std::process::id(),
            ppid: u32::try_from(ppid).unwrap_or(0),
            uid,
            start_ticks: parse_start_ticks(&stat)?,
            clk_tck: u64::try_from(clk_tck).ok().filter(|t| *t > 0)?,
        })
    }
}

/// Where [`scan_gui_processes`] looks: `/proc` and the temp dir a process
/// without `TMPDIR` in its environment uses (`/tmp`). Tests point both at
/// temp dirs.
#[derive(Debug, Clone, Copy)]
pub struct ScanRoots<'a> {
    pub proc_root: &'a Path,
    pub default_tmp: &'a Path,
}

impl ScanRoots<'static> {
    pub fn live() -> Self {
        Self {
            proc_root: Path::new("/proc"),
            default_tmp: Path::new("/tmp"),
        }
    }
}

/// How long a GUI must have run before this one started to count as stale.
/// A launch that just started may be about to claim the lock, show the
/// running GUI or read `--build-id`, with no lock file open yet.
pub const MIN_STALE_AGE: Duration = Duration::from_secs(5);

fn strip_deleted(path: &str) -> &str {
    path.strip_suffix(" (deleted)").unwrap_or(path)
}

fn exe_named(exe: &str, name: &str) -> bool {
    Path::new(strip_deleted(exe))
        .file_name()
        .is_some_and(|n| n == name)
}

/// True when `exe` (a `/proc/<pid>/exe` target) is the GUI binary, replaced
/// on disk or not. The tracker daemon (`scuffed-stat-tracker`) never is.
pub fn is_gui_exe(exe: &str) -> bool {
    exe_named(exe, GUI_BIN)
}

/// Real uid, parent pid and whether the process is a zombie or dead
/// (`State:` Z or X), from `/proc/<pid>/status`.
fn read_status(dir: &Path) -> Option<(u32, u32, bool)> {
    let status = std::fs::read_to_string(dir.join("status")).ok()?;
    let mut uid = None;
    let mut ppid = None;
    let mut gone = false;
    for line in status.lines() {
        if let Some(rest) = line.strip_prefix("Uid:") {
            uid = rest.split_whitespace().next()?.parse().ok();
        } else if let Some(rest) = line.strip_prefix("PPid:") {
            ppid = rest.trim().parse().ok();
        } else if let Some(rest) = line.strip_prefix("State:") {
            gone = matches!(rest.trim_start().chars().next(), Some('Z' | 'X'));
        }
    }
    Some((uid?, ppid?, gone))
}

/// True when a child of a GUI is the installer of a 0.4.x in-app update:
/// alive, with a readable exe, and either its command line names the update
/// work dir (`bash <dir>/sst-gui-update-<pid>-<ms>/bootstrap.sh`) or it is
/// `systemctl` (0.4.x stops and starts the tracker around the install).
///
/// Nothing else counts: not a zombie (an unreaped tracker or `xdg-open`),
/// not a live `xdg-open` (a shell script, which stays in the foreground on
/// some desktops while the browser runs), not a browser, and not a child
/// whose exe cannot be read. The work-dir check ([`GuiProcess::update_dir`])
/// covers the download before the installer starts.
fn is_update_child(child: &ProcEntry, proc_root: &Path) -> bool {
    let Some(exe) = child.exe.as_deref() else {
        return false;
    };
    if child.gone {
        return false;
    }
    if exe_named(exe, "systemctl") {
        return true;
    }
    std::fs::read(proc_root.join(child.pid.to_string()).join("cmdline")).is_ok_and(|cmdline| {
        cmdline
            .windows(UPDATE_DIR_PREFIX.len())
            .any(|w| w == UPDATE_DIR_PREFIX.as_bytes())
    })
}

/// The work dir name a 0.4.x in-app update uses, `sst-gui-update-<pid>-<ms>`.
const UPDATE_DIR_PREFIX: &str = "sst-gui-update-";

/// Field 22 (`starttime`) of a `/proc/<pid>/stat` line. The command name in
/// field 2 may hold spaces and parentheses, so fields are counted after the
/// last `)`.
pub fn parse_start_ticks(stat: &str) -> Option<u64> {
    let rest = &stat[stat.rfind(')')? + 1..];
    // After `)`: field 3 (state) is the first.
    rest.split_whitespace().nth(22 - 3)?.parse().ok()
}

fn read_exe(dir: &Path) -> Option<String> {
    let target = std::fs::read_link(dir.join("exe")).ok()?;
    Some(target.to_string_lossy().into_owned())
}

fn has_lock_open(dir: &Path) -> Option<bool> {
    let fds = std::fs::read_dir(dir.join("fd")).ok()?;
    Some(fds.flatten().any(|fd| {
        std::fs::read_link(fd.path()).is_ok_and(|target| {
            let target = target.to_string_lossy();
            exe_named(&target, LOCK_FILE)
        })
    }))
}

/// `TMPDIR` from `/proc/<pid>/environ` when set, else `default_tmp`.
fn process_tmp(dir: &Path, default_tmp: &Path) -> PathBuf {
    std::fs::read(dir.join("environ"))
        .ok()
        .and_then(|env| {
            env.split(|b| *b == 0)
                .find_map(|var| var.strip_prefix(b"TMPDIR="))
                .filter(|v| !v.is_empty())
                .map(|v| PathBuf::from(String::from_utf8_lossy(v).into_owned()))
        })
        .unwrap_or_else(|| default_tmp.to_path_buf())
}

fn has_update_dir(tmp: &Path, pid: u32) -> bool {
    let prefix = format!("{UPDATE_DIR_PREFIX}{pid}-");
    std::fs::read_dir(tmp).is_ok_and(|entries| {
        entries
            .flatten()
            .any(|e| e.file_name().to_string_lossy().starts_with(&prefix))
    })
}

/// One `/proc` entry, any user.
struct ProcEntry {
    pid: u32,
    ppid: u32,
    uid: u32,
    /// A zombie or dead process (`State:` Z or X).
    gone: bool,
    exe: Option<String>,
    args: Vec<Vec<u8>>,
}

impl ProcEntry {
    fn has_arg(&self, arg: &[u8]) -> bool {
        self.args.iter().skip(1).any(|a| a == arg)
    }
}

/// The processes of user `uid` that run the GUI binary, read from
/// `roots.proc_root` (`/proc` in the app; a temp dir in tests). Processes
/// of other users, and entries that vanish mid-scan, are left out. Children
/// of every user are read for [`GuiProcess::busy_child`].
pub fn scan_gui_processes(roots: ScanRoots<'_>, uid: u32) -> Vec<GuiProcess> {
    let Ok(entries) = std::fs::read_dir(roots.proc_root) else {
        return Vec::new();
    };
    let mut all = Vec::new();
    for entry in entries.flatten() {
        let Some(pid) = entry
            .file_name()
            .to_str()
            .and_then(|n| n.parse::<u32>().ok())
        else {
            continue;
        };
        let dir = entry.path();
        let Some((owner, ppid, gone)) = read_status(&dir) else {
            continue;
        };
        let exe = read_exe(&dir);
        let args = if owner == uid && exe.as_deref().is_some_and(is_gui_exe) {
            std::fs::read(dir.join("cmdline"))
                .unwrap_or_default()
                .split(|b| *b == 0)
                .map(<[u8]>::to_vec)
                .collect()
        } else {
            Vec::new()
        };
        all.push(ProcEntry {
            pid,
            ppid,
            uid: owner,
            gone,
            exe,
            args,
        });
    }
    let mut found = Vec::new();
    for p in &all {
        let Some(exe) = p.exe.as_deref() else {
            continue;
        };
        if p.uid != uid || !is_gui_exe(exe) {
            continue;
        }
        let dir = roots.proc_root.join(p.pid.to_string());
        let busy_child = all
            .iter()
            .any(|child| child.ppid == p.pid && is_update_child(child, roots.proc_root));
        found.push(GuiProcess {
            pid: p.pid,
            ppid: p.ppid,
            exe: exe.to_string(),
            companion: p.has_arg(b"--companion"),
            fixture: p.has_arg(b"--fixture"),
            replace: p.has_arg(b"--replace"),
            has_lock_open: has_lock_open(&dir),
            start_ticks: std::fs::read_to_string(dir.join("stat"))
                .ok()
                .as_deref()
                .and_then(parse_start_ticks),
            busy_child,
            update_dir: has_update_dir(&process_tmp(&dir, roots.default_tmp), p.pid),
        });
    }
    found.sort_by_key(|p| p.pid);
    found
}

/// The pids to stop once this process holds the lock (S5-UP-1): GUIs of
/// this user that run outside the lock protocol, then those GUIs'
/// `--companion` children. That is a GUI from before the lock (0.4.x), or
/// one whose binary was replaced and that lost the lock: either can save an
/// older config over this one's settings.
///
/// A GUI is left alone when it:
/// - is this process, or its parent (an old GUI that started this one
///   through "Restart now" and is on its way out);
/// - is a `--companion`, `--fixture` or `--replace` run;
/// - has the lock file open (it holds the lock, or is in the claim flow),
///   or its open files could not be read;
/// - started less than [`MIN_STALE_AGE`] before this one, or its start
///   could not be read (a launch on its way into the claim flow);
/// - has a live update child ([`GuiProcess::busy_child`]) or an update
///   work dir: a 0.4.x GUI in the middle of its in-app update.
///   Stopping it would kill the installer half way with the tracker
///   stopped.
///
/// The GUIs go first; their companions were picked from the same scan.
pub fn stale_gui_targets(procs: &[GuiProcess], me: Me) -> Vec<u32> {
    let min_age = MIN_STALE_AGE.as_secs() * me.clk_tck;
    let stale: Vec<u32> = procs
        .iter()
        .filter(|p| {
            p.pid != me.pid
                && p.pid != me.ppid
                && !p.companion
                && !p.fixture
                && !p.replace
                && p.has_lock_open == Some(false)
                && p.start_ticks
                    .is_some_and(|start| start.saturating_add(min_age) <= me.start_ticks)
                && !p.busy_child
                && !p.update_dir
        })
        .map(|p| p.pid)
        .collect();
    let companions = procs
        .iter()
        .filter(|p| p.companion && p.pid != me.pid && stale.contains(&p.ppid))
        .map(|p| p.pid);
    let mut targets = stale.clone();
    targets.extend(companions);
    targets
}

/// Stop the stale GUIs [`stale_gui_targets`] picks, with SIGTERM through
/// `kill(pid, exe)`. Each pid's `exe` is read again right before its signal
/// and a pid that no longer runs the same binary is skipped; the live
/// `kill` ([`sigterm_if_exe`]) checks once more through a pidfd. Returns
/// the pids signalled.
pub fn stop_stale_guis(
    roots: ScanRoots<'_>,
    me: Me,
    kill: &mut dyn FnMut(u32, &str) -> io::Result<()>,
) -> Vec<u32> {
    let procs = scan_gui_processes(roots, me.uid);
    let mut stopped = Vec::new();
    for pid in stale_gui_targets(&procs, me) {
        let Some(seen) = procs.iter().find(|p| p.pid == pid) else {
            continue;
        };
        if read_exe(&roots.proc_root.join(pid.to_string())).as_deref() != Some(seen.exe.as_str()) {
            continue;
        }
        match kill(pid, &seen.exe) {
            Ok(()) => {
                tracing::info!(pid, exe = %seen.exe, companion = seen.companion, "stopped an older tracker GUI that runs without the lock");
                stopped.push(pid);
            }
            Err(e) => {
                tracing::warn!(pid, error = %e, "could not stop an older tracker GUI");
            }
        }
    }
    stopped
}

/// SIGTERM `pid` when `/proc/<pid>/exe` still reads `exe`. The check and
/// the signal go through one pidfd, so a pid that exits and is reused
/// after the check is not signalled. Without pidfds (kernel before 5.3) it
/// falls back to a check and `kill`.
pub fn sigterm_if_exe(pid: u32, exe: &str) -> io::Result<()> {
    let raw =
        libc::pid_t::try_from(pid).map_err(|_| io::Error::from(io::ErrorKind::InvalidInput))?;
    if raw <= 0 {
        return Err(io::Error::from(io::ErrorKind::InvalidInput));
    }
    let same_exe = || read_exe(&Path::new("/proc").join(pid.to_string())).as_deref() == Some(exe);
    // SAFETY: pidfd_open takes a pid and flags and returns a new fd or -1.
    let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, raw, 0) };
    if fd < 0 {
        let err = io::Error::last_os_error();
        if err.raw_os_error() != Some(libc::ENOSYS) {
            return Err(err);
        }
        if !same_exe() {
            return Err(io::Error::other("the process changed before the signal"));
        }
        // SAFETY: kill only sends a signal to one positive pid.
        return if unsafe { libc::kill(raw, libc::SIGTERM) } == 0 {
            Ok(())
        } else {
            Err(io::Error::last_os_error())
        };
    }
    // SAFETY: `fd` is a pidfd this function owns from here on.
    let fd = unsafe { std::os::fd::OwnedFd::from_raw_fd(fd as std::os::fd::RawFd) };
    // The signal goes only to the process the pidfd refers to. If that
    // process died and the pid was reused before this check, the check
    // reads the new process, but the signal then fails with ESRCH.
    if !same_exe() {
        return Err(io::Error::other("the process changed before the signal"));
    }
    // SAFETY: pidfd_send_signal on an owned pidfd with no siginfo.
    let sent = unsafe {
        libc::syscall(
            libc::SYS_pidfd_send_signal,
            fd.as_raw_fd(),
            libc::SIGTERM,
            std::ptr::null::<libc::siginfo_t>(),
            0,
        )
    };
    if sent == 0 {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::{Child, Command, Stdio};

    const CHILD_ENV: &str = "STAT_TRACKER_UI_TEST_LOCK_HOLDER";
    const CHILD_MODE_ENV: &str = "STAT_TRACKER_UI_TEST_LOCK_MODE";

    fn spawn_holder(path: &Path, mode: &str) -> Child {
        let ready = path.with_extension("ready");
        let child = Command::new(std::env::current_exe().unwrap())
            .args([
                "instance::tests::lock_holder_child",
                "--exact",
                "--ignored",
                "--nocapture",
            ])
            .env(CHILD_ENV, path)
            .env(CHILD_MODE_ENV, mode)
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let start = Instant::now();
        while !ready.exists() {
            assert!(
                start.elapsed() < Duration::from_secs(30),
                "lock holder never started"
            );
            std::thread::sleep(Duration::from_millis(20));
        }
        child
    }

    /// Helper process for the tests below. Does nothing unless the parent
    /// test sets the env var. Mode `sleep` holds the lock for 60 s. Mode
    /// `quit` behaves like the GUI: it exits when a quit request arrives.
    #[test]
    #[ignore]
    fn lock_holder_child() {
        let Some(path) = std::env::var_os(CHILD_ENV) else {
            return;
        };
        let path = PathBuf::from(path);
        let data_dir = path.parent().unwrap().to_path_buf();
        let claim = claim(&path, "0.5.0-alpha.5.1").unwrap();
        assert!(matches!(claim, Claim::Primary(_)));
        std::fs::write(path.with_extension("ready"), b"").unwrap();
        let quit_mode = std::env::var(CHILD_MODE_ENV).as_deref() == Ok("quit");
        let start = Instant::now();
        while start.elapsed() < Duration::from_secs(60) {
            if quit_mode && consume_quit(&data_dir) {
                break;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
        drop(claim);
    }

    #[test]
    fn lock_dir_prefers_runtime_dir() {
        let data = Path::new("/data");
        assert_eq!(
            lock_path(Some("/run/user/7".into()), data),
            Path::new("/run/user/7").join(LOCK_FILE)
        );
        assert_eq!(lock_path(Some("".into()), data), data.join(LOCK_FILE));
        assert_eq!(lock_path(None, data), data.join(LOCK_FILE));
    }

    #[test]
    fn second_claim_is_secondary_until_first_drops() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("sub").join(LOCK_FILE);
        let first = claim(&path, "1.0.0").unwrap();
        assert!(matches!(first, Claim::Primary(_)));
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary));
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary));
        drop(first);
        // Other tests spawn child processes in parallel. Between fork and
        // exec a child shares this process's open files, so the flock can
        // outlive `drop` by a moment. Retry briefly.
        let start = Instant::now();
        let mut again = claim(&path, "1.0.0").unwrap();
        while matches!(again, Claim::Secondary) && start.elapsed() < Duration::from_secs(2) {
            std::thread::sleep(Duration::from_millis(10));
            again = claim(&path, "1.0.0").unwrap();
        }
        assert!(matches!(again, Claim::Primary(_)));
    }

    #[test]
    fn primary_records_pid_and_version() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        std::fs::write(&path, b"999999\n9.9.9-long-stale-version\n").unwrap();
        let _held = claim(&path, " 0.5.0-alpha.5.1 ").unwrap();
        assert_eq!(
            read_holder(&path),
            Some(Holder {
                pid: std::process::id(),
                version: "0.5.0-alpha.5.1".into()
            })
        );
    }

    #[test]
    fn stale_lock_file_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        std::fs::write(&path, b"12345\n0.1.0\n").unwrap();
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Primary(_)));
    }

    #[test]
    fn killed_holder_does_not_block_launch() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "sleep");
        assert!(
            matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary),
            "a live holder must keep the lock"
        );
        child.kill().unwrap(); // SIGKILL: no destructors, no unlock call.
        child.wait().unwrap();
        assert!(
            matches!(claim(&path, "1.0.0").unwrap(), Claim::Primary(_)),
            "a killed holder must not block the next launch"
        );
    }

    #[test]
    fn replacing_launch_quits_the_holder_and_takes_the_lock() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "quit");
        let holder = read_holder(&path).expect("holder wrote its pid");
        assert_eq!(holder.pid, child.id());
        assert!(matches!(claim(&path, "1.0.0").unwrap(), Claim::Secondary));
        let start = Instant::now();
        let got = claim_replacing(&path, dir.path(), "1.0.0", Duration::from_secs(20)).unwrap();
        assert!(
            matches!(got, Claim::Primary(_)),
            "the holder must quit on request and free the lock"
        );
        assert!(start.elapsed() < Duration::from_secs(15));
        let status = child.wait().unwrap();
        assert!(status.success(), "holder exited cleanly: {status:?}");
        assert_eq!(read_holder(&path).unwrap().pid, std::process::id());
        assert!(
            !quit_path(dir.path()).exists(),
            "R2-2: the request is removed once the replace succeeded"
        );
    }

    #[test]
    fn replacing_launch_gives_up_on_a_stuck_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "sleep");
        let got = claim_replacing(&path, dir.path(), "1.0.0", Duration::from_millis(300)).unwrap();
        assert!(matches!(got, Claim::Secondary));
        assert!(
            !quit_path(dir.path()).exists(),
            "R2-2: a launch that gave up takes its request back, so the holder does not quit later with no GUI to follow"
        );
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn a_new_primary_ignores_requests_from_before_a_clock_step() {
        // R2-2: request files left by an earlier session, with mtimes an
        // hour "ahead" because the wall clock stepped back since.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let ahead = SystemTime::now() + Duration::from_secs(3600);
        for stale in [quit_path(dir.path()), show_path(dir.path())] {
            std::fs::write(&stale, b"").unwrap();
            File::options()
                .write(true)
                .open(&stale)
                .unwrap()
                .set_modified(ahead)
                .unwrap();
        }
        let now = SystemTime::now();
        assert!(
            RequestWatch::new(quit_path(dir.path()), now).poll(),
            "precondition: by mtime alone the stale request would fire"
        );
        let got = claim_session(
            &path,
            dir.path(),
            "1.0.0",
            false,
            Path::new("/nonexistent-proc"),
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(matches!(got, Claim::Primary(_)));
        let mut quit = RequestWatch::new(quit_path(dir.path()), now);
        let mut show = RequestWatch::new(show_path(dir.path()), now);
        assert!(!quit.poll(), "a stale quit request must not fire");
        assert!(!show.poll(), "a stale show request must not fire");
        std::thread::sleep(Duration::from_millis(20));
        request_quit(dir.path()).unwrap();
        assert!(quit.poll(), "a request written after the claim still fires");
    }

    #[test]
    fn a_same_version_launch_only_shows_the_holder() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "quit");
        let got = claim_session(
            &path,
            dir.path(),
            "0.5.0-alpha.5.1",
            false,
            Path::new("/nonexistent-proc"),
            Duration::from_millis(300),
        )
        .unwrap();
        assert!(matches!(got, Claim::Secondary));
        assert!(!quit_path(dir.path()).exists(), "no quit request was sent");
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn restart_starts_the_binary_the_update_installed() {
        // R2-3: the update installs to `<prefix>/bin/stat-tracker-gui`.
        let prefix = tempfile::tempdir().unwrap();
        let installed = prefix.path().join("bin").join(GUI_BIN);
        let elsewhere = Path::new("/opt/dev/target/release/stat-tracker-gui (deleted)");
        assert_eq!(
            relaunch_target(Some(prefix.path()), elsewhere),
            Path::new("/opt/dev/target/release/stat-tracker-gui"),
            "no file at the prefix: fall back to the running binary"
        );
        std::fs::create_dir_all(installed.parent().unwrap()).unwrap();
        std::fs::write(&installed, b"#!/bin/sh\n").unwrap();
        assert_eq!(relaunch_target(Some(prefix.path()), elsewhere), installed);
        let running = PathBuf::from(format!("{} (deleted)", installed.display()));
        assert_eq!(
            relaunch_target(Some(prefix.path()), &running),
            installed,
            "same file: the running path, without the suffix"
        );
        assert_eq!(
            relaunch_target(None, elsewhere),
            Path::new("/opt/dev/target/release/stat-tracker-gui")
        );
    }

    #[test]
    fn holder_is_older_by_deleted_exe_or_version() {
        let proc_root = tempfile::tempdir().unwrap();
        let pid_dir = proc_root.path().join("42");
        std::fs::create_dir_all(&pid_dir).unwrap();
        let same = Holder {
            pid: 42,
            version: "0.5.0-alpha.5.1".into(),
        };

        std::os::unix::fs::symlink("/home/u/.local/bin/stat-tracker-gui", pid_dir.join("exe"))
            .unwrap();
        assert!(!holder_is_older(&same, "0.5.0-alpha.5.1", proc_root.path()));
        assert!(
            !holder_is_older(&same, "0.5.0-alpha.5", proc_root.path()),
            "a newer holder is shown, not replaced"
        );
        assert!(holder_is_older(&same, "0.5.0-alpha.6", proc_root.path()));

        std::fs::remove_file(pid_dir.join("exe")).unwrap();
        std::os::unix::fs::symlink(
            "/home/u/.local/bin/stat-tracker-gui (deleted)",
            pid_dir.join("exe"),
        )
        .unwrap();
        assert!(
            holder_is_older(&same, "0.5.0-alpha.5.1", proc_root.path()),
            "a replaced binary is older even at the same version"
        );

        let unknown = Holder {
            pid: 7,
            version: String::new(),
        };
        assert!(!holder_is_older(&unknown, "0.5.0", proc_root.path()));
    }

    #[test]
    fn relaunch_path_strips_deleted_suffix() {
        assert_eq!(
            relaunch_path(Path::new("/home/u/.local/bin/stat-tracker-gui (deleted)")),
            Path::new("/home/u/.local/bin/stat-tracker-gui")
        );
        assert_eq!(
            relaunch_path(Path::new("/usr/bin/stat-tracker-gui")),
            Path::new("/usr/bin/stat-tracker-gui")
        );
    }

    #[test]
    fn request_watch_ignores_startup_then_fires_on_request() {
        let dir = tempfile::tempdir().unwrap();
        let since = SystemTime::now();
        let mut watch = RequestWatch::new(show_path(dir.path()), since);
        assert!(!watch.poll(), "no file yet");
        assert!(!watch.poll());
        std::thread::sleep(Duration::from_millis(20));
        request_show(dir.path()).unwrap();
        assert!(watch.poll(), "a new file fires");
        assert!(!watch.poll(), "fires once per request");
        std::thread::sleep(Duration::from_millis(20));
        request_show(dir.path()).unwrap();
        assert!(watch.poll(), "a moved mtime fires");
        assert!(!watch.poll());
    }

    #[test]
    fn request_watch_ignores_file_left_by_earlier_session() {
        let dir = tempfile::tempdir().unwrap();
        request_show(dir.path()).unwrap();
        std::thread::sleep(Duration::from_millis(20));
        let mut watch = RequestWatch::new(show_path(dir.path()), SystemTime::now());
        assert!(!watch.poll());
        assert!(!watch.poll());
    }

    #[test]
    fn request_before_first_poll_fires() {
        // R241-B: a launch between taking the lock and the first Tick.
        let dir = tempfile::tempdir().unwrap();
        let since = SystemTime::now();
        std::thread::sleep(Duration::from_millis(20));
        request_show(dir.path()).unwrap();
        let mut watch = RequestWatch::new(show_path(dir.path()), since);
        assert!(watch.poll(), "a request newer than the lock fires");
        assert!(!watch.poll());
    }

    #[test]
    fn quit_request_is_consumed_once() {
        let dir = tempfile::tempdir().unwrap();
        let mut watch = RequestWatch::new(quit_path(dir.path()), SystemTime::now());
        assert!(!watch.poll());
        std::thread::sleep(Duration::from_millis(20));
        request_quit(dir.path()).unwrap();
        assert!(watch.poll());
        assert!(!watch.poll());
        assert!(!watch.poll());
    }

    /// A fake `/proc/<pid>` under `root`: `exe` and the `fd/*` links are
    /// dangling symlinks, which is all the scan reads. Never the real /proc.
    struct FakeProc<'a> {
        pid: u32,
        ppid: u32,
        uid: u32,
        exe: &'a str,
        args: &'a [&'a str],
        /// `None`: no `fd` directory (its open files cannot be read).
        fds: Option<&'a [&'a str]>,
        /// Start in clock ticks; `None` writes no `stat` file.
        start: Option<u64>,
        /// `TMPDIR` in its environment.
        tmpdir: Option<&'a Path>,
        /// An unreaped child: `State: Z`, no `exe`, empty `cmdline`.
        zombie: bool,
    }

    const UID: u32 = 1000;
    const TCK: u64 = 100;
    /// This GUI started 1000 s after boot; old GUIs at 100 s.
    const MY_START: u64 = 1000 * TCK;
    const OLD: Option<u64> = Some(100 * TCK);
    const GUI: &str = "/home/u/.local/bin/stat-tracker-gui";
    const GUI_DELETED: &str = "/home/u/.local/bin/stat-tracker-gui (deleted)";
    const LOCK: &str = "/run/user/1000/scuffed-stat-tracker-gui.lock";
    const ME: Me = Me {
        pid: 500,
        ppid: 1,
        uid: UID,
        start_ticks: MY_START,
        clk_tck: TCK,
    };

    fn base(pid: u32, exe: &str) -> FakeProc<'_> {
        FakeProc {
            pid,
            ppid: 1,
            uid: UID,
            exe,
            args: &[],
            fds: Some(&[]),
            start: OLD,
            tmpdir: None,
            zombie: false,
        }
    }

    fn fake_proc(root: &Path, p: FakeProc<'_>) {
        let dir = root.join(p.pid.to_string());
        std::fs::create_dir_all(&dir).unwrap();
        if !p.zombie {
            std::os::unix::fs::symlink(p.exe, dir.join("exe")).unwrap();
        }
        let state = if p.zombie {
            "Z (zombie)"
        } else {
            "S (sleeping)"
        };
        std::fs::write(
            dir.join("status"),
            format!(
                "Name:\tstat-tracker-gu\nState:\t{state}\nPPid:\t{}\nUid:\t{}\t{}\t{}\t{}\n",
                p.ppid, p.uid, p.uid, p.uid, p.uid
            ),
        )
        .unwrap();
        if let Some(start) = p.start {
            // Field 22 is starttime; the name has a space and a ')'.
            std::fs::write(
                dir.join("stat"),
                format!(
                    "{} (stat tracker) gui) S {} 1 1 0 -1 4194560 0 0 0 0 0 0 0 0 20 0 1 0 {start} 0 0\n",
                    p.pid, p.ppid
                ),
            )
            .unwrap();
        }
        let mut cmdline = Vec::new();
        if !p.zombie {
            for arg in std::iter::once(p.exe).chain(p.args.iter().copied()) {
                cmdline.extend_from_slice(arg.as_bytes());
                cmdline.push(0);
            }
        }
        std::fs::write(dir.join("cmdline"), cmdline).unwrap();
        let mut environ = b"HOME=/home/u\0".to_vec();
        if let Some(tmp) = p.tmpdir {
            environ.extend_from_slice(format!("TMPDIR={}\0", tmp.display()).as_bytes());
        }
        std::fs::write(dir.join("environ"), environ).unwrap();
        if let Some(fds) = p.fds {
            let fd_dir = dir.join("fd");
            std::fs::create_dir_all(&fd_dir).unwrap();
            for (i, target) in fds.iter().enumerate() {
                std::os::unix::fs::symlink(target, fd_dir.join(i.to_string())).unwrap();
            }
        }
    }

    /// A fake `/proc` and an empty default temp dir.
    struct Tree {
        proc: tempfile::TempDir,
        tmp: tempfile::TempDir,
    }

    impl Tree {
        fn new() -> Self {
            Self {
                proc: tempfile::tempdir().unwrap(),
                tmp: tempfile::tempdir().unwrap(),
            }
        }

        fn roots(&self) -> ScanRoots<'_> {
            ScanRoots {
                proc_root: self.proc.path(),
                default_tmp: self.tmp.path(),
            }
        }

        fn add(&self, p: FakeProc<'_>) {
            fake_proc(self.proc.path(), p);
        }

        fn targets(&self, me: Me) -> Vec<u32> {
            stale_gui_targets(&scan_gui_processes(self.roots(), UID), me)
        }
    }

    fn proc_tree() -> Tree {
        let t = Tree::new();
        // This GUI, holding the lock, and its companion.
        t.add(FakeProc {
            fds: Some(&[LOCK]),
            start: Some(MY_START),
            ..base(500, GUI)
        });
        t.add(FakeProc {
            ppid: 500,
            args: &["--companion"],
            start: Some(MY_START),
            ..base(501, GUI)
        });
        // A 0.4.24 GUI left running by its in-app update: binary replaced,
        // no lock. Its companion overlay and the daemon it started.
        t.add(FakeProc {
            fds: Some(&["/dev/null", "socket:[123]"]),
            ..base(300, GUI_DELETED)
        });
        t.add(FakeProc {
            ppid: 300,
            args: &["--companion"],
            ..base(301, GUI_DELETED)
        });
        t.add(FakeProc {
            ppid: 300,
            ..base(302, "/home/u/.local/bin/scuffed-stat-tracker (deleted)")
        });
        // A lockless legacy GUI whose binary was not replaced.
        t.add(base(310, "/usr/bin/stat-tracker-gui"));
        // A replacing launch waiting for the lock with the file open.
        t.add(FakeProc {
            args: &["--replace"],
            fds: Some(&[LOCK]),
            ..base(320, GUI)
        });
        // A demo run.
        t.add(FakeProc {
            args: &["--fixture", "sample"],
            ..base(330, GUI_DELETED)
        });
        // The tracker daemon, a stale GUI of another user, a GUI whose open
        // files cannot be read, and an orphan companion.
        t.add(base(
            340,
            "/home/u/.local/bin/scuffed-stat-tracker (deleted)",
        ));
        t.add(FakeProc {
            uid: 1001,
            ..base(350, GUI_DELETED)
        });
        t.add(FakeProc {
            fds: None,
            ..base(360, GUI_DELETED)
        });
        t.add(FakeProc {
            args: &["--companion"],
            ..base(370, GUI_DELETED)
        });
        // Not a pid, and an unrelated process.
        std::fs::create_dir_all(t.proc.path().join("self")).unwrap();
        t.add(base(380, "/usr/bin/bash"));
        t
    }

    #[test]
    fn gui_exe_matches_the_gui_binary_only() {
        assert!(is_gui_exe(GUI));
        assert!(is_gui_exe(GUI_DELETED));
        assert!(is_gui_exe("/tmp/.mount_abc/usr/bin/stat-tracker-gui"));
        assert!(!is_gui_exe("/home/u/.local/bin/scuffed-stat-tracker"));
        assert!(!is_gui_exe(
            "/home/u/.local/bin/scuffed-stat-tracker (deleted)"
        ));
        assert!(!is_gui_exe("/usr/bin/stat-tracker-gui-old"));
        assert!(!is_gui_exe(""));
    }

    #[test]
    fn start_ticks_are_field_22_after_the_name() {
        let line = "42 (a) b (c)) S 1 42 42 0 -1 4194560 1 2 3 4 5 6 7 8 20 0 3 0 123456 999 77\n";
        assert_eq!(parse_start_ticks(line), Some(123456));
        assert_eq!(parse_start_ticks("42 (x) S 1"), None);
        assert_eq!(parse_start_ticks("no name"), None);
    }

    #[test]
    fn scan_reads_gui_processes_of_this_user_only() {
        let t = proc_tree();
        let procs = scan_gui_processes(t.roots(), UID);
        let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![300, 301, 310, 320, 330, 360, 370, 500, 501]);
        let legacy = &procs[0];
        assert_eq!(legacy.exe, GUI_DELETED);
        assert_eq!(legacy.has_lock_open, Some(false));
        assert_eq!(legacy.start_ticks, OLD);
        // Its companion and the daemon it started are not busy children.
        assert!(!legacy.busy_child && !legacy.update_dir);
        assert!(!legacy.companion && !legacy.fixture && !legacy.replace);
        assert!(procs[1].companion);
        assert_eq!(procs[1].ppid, 300);
        assert!(procs[3].replace);
        assert_eq!(procs[3].has_lock_open, Some(true));
        assert!(procs[4].fixture);
        assert_eq!(procs[5].has_lock_open, None);
        let missing = t.proc.path().join("missing");
        let roots = ScanRoots {
            proc_root: &missing,
            ..t.roots()
        };
        assert!(scan_gui_processes(roots, UID).is_empty());
    }

    #[test]
    fn stale_targets_are_lockless_guis_then_their_companions() {
        let t = proc_tree();
        // GUIs first, then companions. Never this GUI or its companion,
        // the replacing launch, the demo run, the daemon, another user's
        // GUI, an unreadable one, or an orphan companion.
        assert_eq!(t.targets(ME), vec![300, 310, 301]);
    }

    #[test]
    fn stale_targets_skip_the_parent_that_restarted_this_gui() {
        let t = proc_tree();
        assert_eq!(t.targets(Me { ppid: 300, ..ME }), vec![310]);
    }

    #[test]
    fn a_gui_mid_update_is_left_alone() {
        // 0.4.24 runs `bash <work dir>/bootstrap.sh` as its child, then
        // `systemctl --user start`.
        let t = Tree::new();
        t.add(base(300, GUI_DELETED));
        t.add(FakeProc {
            ppid: 300,
            args: &["/tmp/sst-gui-update-300-1729600000000/bootstrap.sh"],
            ..base(303, "/usr/bin/bash")
        });
        t.add(base(310, GUI_DELETED));
        t.add(FakeProc {
            ppid: 310,
            args: &["--user", "start", "scuffed-stat-tracker.service"],
            ..base(311, "/usr/bin/systemctl")
        });
        // A child whose exe cannot be read does not count, even naming the
        // work dir: the installer is this user's own bash.
        t.add(base(315, GUI_DELETED));
        t.add(FakeProc {
            ppid: 315,
            uid: 0,
            args: &["/tmp/sst-gui-update-315-1/payload"],
            ..base(316, "/usr/bin/install")
        });
        std::fs::remove_file(t.proc.path().join("316").join("exe")).unwrap();
        // Between download and install: only the work dir, in its TMPDIR.
        let its_tmp = tempfile::tempdir().unwrap();
        std::fs::create_dir(its_tmp.path().join("sst-gui-update-320-1729600000000")).unwrap();
        t.add(FakeProc {
            tmpdir: Some(its_tmp.path()),
            ..base(320, GUI_DELETED)
        });
        // No TMPDIR: the default temp dir.
        std::fs::create_dir(t.tmp.path().join("sst-gui-update-330-1")).unwrap();
        t.add(base(330, GUI_DELETED));
        // Another pid's work dir does not count.
        std::fs::create_dir(t.tmp.path().join("sst-gui-update-3400-1")).unwrap();
        t.add(base(340, GUI_DELETED));
        let procs = scan_gui_processes(t.roots(), UID);
        let pids: Vec<u32> = procs.iter().map(|p| p.pid).collect();
        assert_eq!(pids, vec![300, 310, 315, 320, 330, 340]);
        assert!(procs[0].busy_child && procs[1].busy_child);
        assert!(!procs[2].busy_child);
        assert!(procs[3].update_dir && procs[4].update_dir);
        assert!(!procs[5].busy_child && !procs[5].update_dir);
        assert_eq!(stale_gui_targets(&procs, ME), vec![315, 340]);
    }

    #[test]
    fn zombies_and_other_children_do_not_shield_a_stale_gui() {
        // 0.4.24 forgets a tracker it started itself (a zombie once it
        // stops) and never waits on xdg-open; a browser may outlive it.
        let t = Tree::new();
        t.add(base(300, GUI_DELETED));
        t.add(FakeProc {
            ppid: 300,
            zombie: true,
            ..base(301, "/home/u/.local/bin/scuffed-stat-tracker")
        });
        t.add(FakeProc {
            ppid: 300,
            zombie: true,
            ..base(302, "/usr/bin/xdg-open")
        });
        t.add(FakeProc {
            ppid: 300,
            args: &["https://github.com/FrozenTear/scuffed-crew/releases"],
            ..base(303, "/usr/lib/firefox/firefox")
        });
        t.add(FakeProc {
            ppid: 300,
            ..base(304, "/home/u/.local/bin/scuffed-stat-tracker (deleted)")
        });
        // A zombie shell is finished, not an update in progress.
        t.add(FakeProc {
            ppid: 300,
            zombie: true,
            ..base(305, "/usr/bin/bash")
        });
        // A live xdg-open: a shell script that stays in the foreground
        // while the browser runs on niri, sway or Hyprland.
        t.add(FakeProc {
            ppid: 300,
            args: &[
                "/usr/bin/xdg-open",
                "https://github.com/FrozenTear/scuffed-crew/releases",
            ],
            ..base(306, "/usr/bin/bash")
        });
        // A live shell running something else.
        t.add(FakeProc {
            ppid: 300,
            args: &["-c", "notify-send done"],
            ..base(307, "/usr/bin/sh")
        });
        let procs = scan_gui_processes(t.roots(), UID);
        assert_eq!(procs.len(), 1);
        assert!(!procs[0].busy_child);
        assert_eq!(stale_gui_targets(&procs, ME), vec![300]);
    }

    #[test]
    fn a_young_or_undated_gui_is_left_alone() {
        let t = Tree::new();
        // Started 4 s before this one, after it, or with no readable start.
        t.add(FakeProc {
            start: Some(MY_START - 4 * TCK),
            ..base(300, GUI)
        });
        t.add(FakeProc {
            start: Some(MY_START + TCK),
            ..base(310, GUI)
        });
        t.add(FakeProc {
            start: None,
            ..base(320, GUI)
        });
        // Exactly the minimum age counts as stale.
        t.add(FakeProc {
            start: Some(MY_START - 5 * TCK),
            ..base(330, GUI)
        });
        assert_eq!(t.targets(ME), vec![330]);
    }

    #[test]
    fn a_holder_of_an_unlinked_lock_file_is_left_alone() {
        let t = Tree::new();
        let unlinked = format!("{LOCK} (deleted)");
        t.add(FakeProc {
            fds: Some(&[unlinked.as_str()]),
            ..base(300, GUI_DELETED)
        });
        t.add(FakeProc {
            fds: Some(&["/run/user/1000/other.lock (deleted)"]),
            ..base(310, GUI_DELETED)
        });
        let procs = scan_gui_processes(t.roots(), UID);
        assert_eq!(procs[0].has_lock_open, Some(true));
        assert_eq!(procs[1].has_lock_open, Some(false));
        assert_eq!(stale_gui_targets(&procs, ME), vec![310]);
    }

    #[test]
    fn stop_signals_only_the_targets_through_the_injected_kill() {
        let t = proc_tree();
        let mut sent = Vec::new();
        let stopped = stop_stale_guis(t.roots(), ME, &mut |pid, exe| {
            sent.push((pid, exe.to_string()));
            if pid == 310 {
                Err(io::Error::from_raw_os_error(libc::EPERM))
            } else {
                Ok(())
            }
        });
        assert_eq!(
            sent,
            vec![
                (300, GUI_DELETED.to_string()),
                (310, "/usr/bin/stat-tracker-gui".to_string()),
                (301, GUI_DELETED.to_string()),
            ]
        );
        assert_eq!(stopped, vec![300, 301]);
    }

    #[test]
    fn stop_skips_a_pid_reused_since_the_scan() {
        let t = proc_tree();
        let mut sent = Vec::new();
        let mut first = true;
        let stopped = stop_stale_guis(t.roots(), ME, &mut |pid, _| {
            if first {
                first = false;
                // Pid 310 exits and is reused before its turn.
                let exe = t.proc.path().join("310").join("exe");
                std::fs::remove_file(&exe).unwrap();
                std::os::unix::fs::symlink("/usr/bin/vim", &exe).unwrap();
            }
            sent.push(pid);
            Ok(())
        });
        assert_eq!(sent, vec![300, 301]);
        assert_eq!(stopped, vec![300, 301]);
    }

    #[test]
    fn nothing_to_stop_when_this_is_the_only_gui() {
        let t = Tree::new();
        t.add(FakeProc {
            fds: Some(&[LOCK]),
            start: Some(MY_START),
            ..base(500, GUI)
        });
        let stopped = stop_stale_guis(t.roots(), ME, &mut |_, _| panic!("no signal"));
        assert!(stopped.is_empty());
    }

    /// L1: a launch waiting in `claim_replacing` keeps the lock file open
    /// the whole time, so the stale scan of a new primary never takes it
    /// for a lockless GUI. The holder is a child process; this test reads
    /// only its own `/proc/self/fd`.
    #[test]
    fn claim_replacing_keeps_the_lock_file_open_while_it_waits() {
        use std::sync::Arc;
        use std::sync::atomic::{AtomicBool, Ordering};
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let mut child = spawn_holder(&path, "sleep");
        let lock = path.canonicalize().unwrap();
        let data_dir = dir.path().to_path_buf();
        let waiter_path = path.clone();
        let done = Arc::new(AtomicBool::new(false));
        let waiter_done = Arc::clone(&done);
        let waiter = std::thread::spawn(move || {
            let got = claim_replacing(&waiter_path, &data_dir, "1.0.0", Duration::from_secs(3));
            waiter_done.store(true, Ordering::SeqCst);
            got
        });
        let open_on_lock = || {
            std::fs::read_dir("/proc/self/fd")
                .map(|fds| {
                    fds.flatten()
                        .filter(|fd| std::fs::read_link(fd.path()).is_ok_and(|t| t == lock))
                        .count()
                })
                .unwrap_or(0)
        };
        // Until the waiter has the file open the first time.
        let start = Instant::now();
        while open_on_lock() == 0 {
            assert!(
                start.elapsed() < Duration::from_secs(10),
                "the waiter never had the lock file open: it closes it between tries"
            );
            std::thread::sleep(Duration::from_millis(5));
        }
        // Then it must stay open on every sample until the wait ends. A
        // sample taken as the wait ended is not counted.
        let mut samples = 0;
        let mut closed = 0;
        while samples < 60 {
            let open = open_on_lock();
            if done.load(Ordering::SeqCst) {
                break;
            }
            samples += 1;
            if open == 0 {
                closed += 1;
            }
            std::thread::sleep(Duration::from_millis(13));
        }
        let got = waiter.join().unwrap().unwrap();
        child.kill().unwrap();
        child.wait().unwrap();
        assert!(matches!(got, Claim::Secondary), "the holder never let go");
        assert!(samples >= 10, "too few samples ({samples}) to tell");
        assert_eq!(
            closed, 0,
            "the waiting launch closed the lock file between tries ({closed} of {samples})"
        );
    }

    #[test]
    fn a_launch_that_must_wait_gets_its_lock_file_back() {
        // claim_replacing retries on this one file, so the waiting launch
        // has the lock file open the whole time and the stale scan leaves
        // it alone.
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join(LOCK_FILE);
        let holder = claim(&path, "1.0.0").unwrap();
        assert!(matches!(holder, Claim::Primary(_)));
        let (claimed, kept) = try_claim(open_lock(&path).unwrap(), "1.0.1").unwrap();
        assert!(matches!(claimed, Claim::Secondary));
        let kept = kept.expect("the file comes back on Secondary");
        let (claimed, kept) = try_claim(kept, "1.0.1").unwrap();
        assert!(matches!(claimed, Claim::Secondary));
        assert!(kept.is_some());
        drop(holder);
        // Other tests fork children that briefly share this process's open
        // files, so the flock can outlive `drop` by a moment.
        let mut file = kept.unwrap();
        let start = Instant::now();
        loop {
            match try_claim(file, "1.0.1").unwrap() {
                (Claim::Primary(_), None) => break,
                (Claim::Secondary, Some(open)) if start.elapsed() < Duration::from_secs(5) => {
                    file = open;
                    std::thread::sleep(Duration::from_millis(20));
                }
                other => panic!("unexpected {other:?}"),
            }
        }
    }
}
