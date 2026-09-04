//! Wayland-first clipboard for the install-command Copy control.
//!
//! iced `text` is not selectable, so Copy has to write the clipboard itself.
//! On niri / CachyOS / AerynOS the reliable path is `wl-copy` (wl-clipboard),
//! which forks and holds the selection after the click. xclip/xsel cover X11.
//! iced's window clipboard is a last-resort fallback in `app` when no helper
//! is on PATH.

use std::io::Write;
use std::process::{Command, Stdio};

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

fn write_via(backend: ClipboardBackend, text: &str) -> Result<(), String> {
    let (cmd, args) = backend.argv();
    let mut child = Command::new(cmd)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::piped())
        .spawn()
        .map_err(|e| format!("{}: {e}", backend.label()))?;
    {
        let stdin = child
            .stdin
            .as_mut()
            .ok_or_else(|| format!("{}: no stdin", backend.label()))?;
        stdin
            .write_all(text.as_bytes())
            .map_err(|e| format!("{}: {e}", backend.label()))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("{}: {e}", backend.label()))?;
    if out.status.success() {
        Ok(())
    } else {
        let err = String::from_utf8_lossy(&out.stderr);
        let err = err.trim();
        if err.is_empty() {
            Err(format!("{} exited {}", backend.label(), out.status))
        } else {
            Err(format!("{}: {err}", backend.label()))
        }
    }
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

    #[test]
    fn backend_labels_match_binaries() {
        assert_eq!(ClipboardBackend::WlCopy.label(), "wl-copy");
        assert_eq!(ClipboardBackend::Xclip.label(), "xclip");
        assert_eq!(ClipboardBackend::Xsel.label(), "xsel");
    }
}
