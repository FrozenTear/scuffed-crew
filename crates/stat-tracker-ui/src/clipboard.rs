//! Wayland-first clipboard for the install-command Copy control.
//!
//! iced `text` is not selectable, so Copy has to write the clipboard itself.
//! On niri / CachyOS / AerynOS the reliable path is `wl-copy` (wl-clipboard),
//! which forks and holds the selection after the click. xclip/xsel cover X11.
//! iced's window clipboard is a last-resort fallback in `app` when no helper
//! is on PATH.

use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{Read, Seek, SeekFrom, Write};
use std::process::{Child, Command, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardBackend {
    WlCopy,
    Xclip,
    Xsel,
}

impl ClipboardBackend {
    pub fn label(self) -> &'static str {
        match self {
            Self::WlCopy => "wl-copy",
            Self::Xclip => "xclip",
            Self::Xsel => "xsel",
        }
    }

    fn argv(self) -> (&'static str, &'static [&'static str]) {
        match self {
            Self::WlCopy => ("wl-copy", &["--type", "text/plain"]),
            Self::Xclip => ("xclip", &["-selection", "clipboard"]),
            Self::Xsel => ("xsel", &["--clipboard", "--input"]),
        }
    }
}

pub fn wayland_session() -> bool {
    std::env::var_os("WAYLAND_DISPLAY").is_some()
        || std::env::var_os("XDG_SESSION_TYPE").is_some_and(|v| v == "wayland")
}

pub fn tool_on_path(name: &str) -> bool {
    std::env::var_os("PATH")
        .is_some_and(|paths| std::env::split_paths(&paths).any(|dir| dir.join(name).is_file()))
}

/// Wayland prefers `wl-copy` even when xclip exists (XWayland).
pub fn preferred_backend(wayland: bool, has: impl Fn(&str) -> bool) -> Option<ClipboardBackend> {
    if wayland && has("wl-copy") {
        return Some(ClipboardBackend::WlCopy);
    }
    if !wayland && has("xclip") {
        return Some(ClipboardBackend::Xclip);
    }
    if !wayland && has("xsel") {
        return Some(ClipboardBackend::Xsel);
    }
    if has("wl-copy") {
        return Some(ClipboardBackend::WlCopy);
    }
    if has("xclip") {
        return Some(ClipboardBackend::Xclip);
    }
    if has("xsel") {
        return Some(ClipboardBackend::Xsel);
    }
    None
}

pub fn copy_error_message(wayland: bool, has_wl_copy: bool) -> String {
    if wayland && !has_wl_copy {
        "Could not copy. This is a Wayland session and wl-copy is not on PATH. \
         Install wl-clipboard (CachyOS/Arch: pacman -S wl-clipboard; \
         Debian: apt install wl-clipboard) and press Copy again."
            .into()
    } else if wayland {
        "Could not copy with wl-copy. Check WAYLAND_DISPLAY and try again, \
         or run the command in a terminal."
            .into()
    } else {
        "Could not copy (no wl-copy, xclip, or xsel). Install wl-clipboard \
         or xclip, or run the command in a terminal."
            .into()
    }
}

pub fn copy_text(text: &str) -> Result<ClipboardBackend, String> {
    let wayland = wayland_session();
    let Some(backend) = preferred_backend(wayland, tool_on_path) else {
        return Err(copy_error_message(wayland, tool_on_path("wl-copy")));
    };
    write_via(backend, text).map(|()| backend).map_err(|e| {
        if wayland {
            format!("{e} On Wayland, install or repair wl-clipboard (wl-copy) if paste is empty.")
        } else {
            e
        }
    })
}

/// How long the UI thread waits for the copy tool's own process to exit.
///
/// xclip, xsel and wl-copy read stdin, fork a background owner that keeps
/// serving the selection, and the parent exits at once. Anything slower
/// than this is stuck (a dead X server, a compositor that never answers),
/// and the click must not freeze the window waiting for it.
const TOOL_EXIT_TIMEOUT: Duration = Duration::from_secs(3);

fn write_via(backend: ClipboardBackend, text: &str) -> Result<(), String> {
    let (cmd, args) = backend.argv();
    run_copy_tool(
        OsStr::new(cmd),
        args,
        backend.label(),
        text,
        TOOL_EXIT_TIMEOUT,
    )
}

/// Feed `text` to a clipboard tool and wait (bounded) for its parent process.
///
/// GUI-5: the background owner the tool forks inherits our stdio. With
/// stderr on a pipe, `wait_with_output` reads until EOF, and EOF only comes
/// when that owner exits (when another app takes the clipboard), so the
/// window hung. stderr therefore goes to an unlinked temp file, read only
/// after the parent has exited, and the wait itself is capped at `timeout`.
fn run_copy_tool(
    program: &OsStr,
    args: &[&str],
    label: &str,
    text: &str,
    timeout: Duration,
) -> Result<(), String> {
    let mut stderr_file = stderr_sink().map_err(|e| format!("{label}: {e}"))?;
    let stderr_for_child = stderr_file
        .try_clone()
        .map_err(|e| format!("{label}: {e}"))?;
    let mut child = Command::new(program)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::from(stderr_for_child))
        .spawn()
        .map_err(|e| format!("{label}: {e}"))?;
    // `stdin` is dropped right after the write: the tool sees EOF and forks.
    // A write error (say EPIPE because the tool died on a missing display)
    // is kept until the exit status is known, so its stderr wins.
    let write_err = match child.stdin.take() {
        Some(mut stdin) => stdin
            .write_all(text.as_bytes())
            .err()
            .map(|e| e.to_string()),
        None => Some("no stdin".to_string()),
    };

    let deadline = Instant::now() + timeout;
    let status = loop {
        match child.try_wait() {
            Ok(Some(status)) => break status,
            Ok(None) if Instant::now() >= deadline => {
                // Not killed: if it is a slow but working owner, killing it
                // would empty the clipboard. Reap it whenever it exits.
                reap_in_background(child);
                return Err(format!(
                    "{label} did not finish within {} s, so the copy is unconfirmed.",
                    timeout.as_secs_f32()
                ));
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(10)),
            Err(e) => return Err(format!("{label}: {e}")),
        }
    };
    if status.success() {
        return match write_err {
            None => Ok(()),
            Some(e) => Err(format!("{label}: {e}")),
        };
    }
    let mut err = String::new();
    let _ = stderr_file.seek(SeekFrom::Start(0));
    let _ = (&mut stderr_file).take(4096).read_to_string(&mut err);
    let err = err.trim();
    if err.is_empty() {
        Err(format!("{label} exited {status}"))
    } else {
        Err(format!("{label}: {err}"))
    }
}

/// An anonymous (already unlinked) file for the tool's stderr.
fn stderr_sink() -> std::io::Result<File> {
    static SEQ: AtomicU64 = AtomicU64::new(0);
    let nanos = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    let path = std::env::temp_dir().join(format!(
        "stat-tracker-gui-clip-{}-{}-{nanos}.err",
        std::process::id(),
        SEQ.fetch_add(1, Ordering::Relaxed)
    ));
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)?;
    let _ = std::fs::remove_file(&path);
    Ok(file)
}

fn reap_in_background(mut child: Child) {
    std::thread::spawn(move || {
        let _ = child.wait();
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wayland_prefers_wl_copy_over_xclip() {
        let has = |n: &str| n == "wl-copy" || n == "xclip";
        assert_eq!(preferred_backend(true, has), Some(ClipboardBackend::WlCopy));
    }

    #[test]
    fn x11_prefers_xclip_when_wl_copy_also_exists() {
        let has = |n: &str| n == "wl-copy" || n == "xclip";
        assert_eq!(preferred_backend(false, has), Some(ClipboardBackend::Xclip));
    }

    #[test]
    fn x11_uses_xsel_when_that_is_all_there_is() {
        assert_eq!(
            preferred_backend(false, |n| n == "xsel"),
            Some(ClipboardBackend::Xsel)
        );
    }

    #[test]
    fn no_helper_is_none() {
        assert_eq!(preferred_backend(true, |_| false), None);
        assert_eq!(preferred_backend(false, |_| false), None);
    }

    #[test]
    fn wayland_error_is_actionable_without_wl_copy() {
        let msg = copy_error_message(true, false);
        assert!(msg.contains("Wayland"), "{msg}");
        assert!(msg.contains("wl-clipboard"), "{msg}");
        assert!(msg.contains("wl-copy"), "{msg}");
        assert!(msg.contains("pacman") || msg.contains("apt"), "{msg}");
    }

    /// Run `sh -c <script> sh <arg>` through the copy path. Never touches
    /// the real clipboard: the "tool" is a shell script.
    fn run_sh(script: &str, arg: &str, timeout: Duration) -> (Result<(), String>, Duration) {
        let started = Instant::now();
        let result = run_copy_tool(
            OsStr::new("/bin/sh"),
            &["-c", script, "sh", arg],
            "fake-clip",
            "ABCD-EFGH",
            timeout,
        );
        (result, started.elapsed())
    }

    fn read_pid(path: &std::path::Path) -> u32 {
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            if let Some(pid) = std::fs::read_to_string(path)
                .ok()
                .and_then(|s| s.trim().parse().ok())
            {
                return pid;
            }
            assert!(Instant::now() < deadline, "script never wrote its pid");
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    fn alive(pid: u32) -> bool {
        std::path::Path::new(&format!("/proc/{pid}")).exists()
    }

    fn kill(pid: u32) {
        let _ = Command::new("kill").arg(pid.to_string()).status();
    }

    /// GUI-5 regression: xclip / xsel / wl-copy fork a background owner that
    /// inherits stderr and lives until another app takes the clipboard. The
    /// copy must return when the tool's own process exits, not when the
    /// owner does, and must leave the owner running.
    #[test]
    fn background_owner_holding_stderr_does_not_block() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("owner.pid");
        let (result, elapsed) = run_sh(
            r#"cat >/dev/null; (sleep 30) & echo $! > "$1"; exit 0"#,
            pidfile.to_str().unwrap(),
            Duration::from_secs(10),
        );
        let owner = read_pid(&pidfile);
        let owner_alive = alive(owner);
        kill(owner);
        assert_eq!(result, Ok(()));
        assert!(
            elapsed < Duration::from_secs(2),
            "copy blocked for {elapsed:?}"
        );
        assert!(owner_alive, "the selection owner must not be killed");
    }

    #[test]
    fn tool_receives_text_and_success_is_ok() {
        let dir = tempfile::tempdir().unwrap();
        let out = dir.path().join("clip.txt");
        let (result, _) = run_sh(r#"cat > "$1""#, out.to_str().unwrap(), TOOL_EXIT_TIMEOUT);
        assert_eq!(result, Ok(()));
        assert_eq!(std::fs::read_to_string(&out).unwrap(), "ABCD-EFGH");
    }

    #[test]
    fn failed_tool_reports_its_stderr() {
        let (result, _) = run_sh(
            "cat >/dev/null; echo 'Error: Can not open display' >&2; exit 3",
            "",
            TOOL_EXIT_TIMEOUT,
        );
        let err = result.unwrap_err();
        assert!(err.starts_with("fake-clip: "), "{err}");
        assert!(err.contains("Can not open display"), "{err}");
    }

    #[test]
    fn stuck_tool_times_out_unconfirmed_and_is_not_killed() {
        let dir = tempfile::tempdir().unwrap();
        let pidfile = dir.path().join("tool.pid");
        let (result, elapsed) = run_sh(
            r#"cat >/dev/null; echo $$ > "$1"; exec sleep 30"#,
            pidfile.to_str().unwrap(),
            Duration::from_millis(300),
        );
        let tool = read_pid(&pidfile);
        let tool_alive = alive(tool);
        kill(tool);
        let err = result.unwrap_err();
        assert!(err.contains("unconfirmed"), "{err}");
        assert!(
            elapsed < Duration::from_secs(2),
            "timeout overran: {elapsed:?}"
        );
        assert!(tool_alive, "a slow tool may still own the clipboard");
    }

    #[test]
    fn backend_labels_match_binaries() {
        assert_eq!(ClipboardBackend::WlCopy.label(), "wl-copy");
        assert_eq!(ClipboardBackend::Xclip.label(), "xclip");
        assert_eq!(ClipboardBackend::Xsel.label(), "xsel");
    }
}
