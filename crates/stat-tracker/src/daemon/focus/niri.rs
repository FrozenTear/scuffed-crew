//! niri's focused window over its IPC socket (`NIRI_SOCKET`).
//!
//! One request per read on a fresh connection: the JSON string
//! `"FocusedWindow"` and a newline, answered by one line,
//! `{"Ok":{"FocusedWindow":{...}}}`, `{"Ok":{"FocusedWindow":null}}` or
//! `{"Err":"..."}`. The socket is used rather than spawning `niri msg`
//! every poll: the same JSON, no process per read, and the read and write
//! deadlines bound it.

use super::{FOCUS_READ_TIMEOUT, FocusRead, FocusSource};
use std::io::{BufRead as _, Read as _, Write as _};

/// Longest reply line taken; a window's JSON is far smaller.
const MAX_REPLY: u64 = 64 * 1024;

pub(crate) struct NiriSource {
    socket: std::path::PathBuf,
}

impl NiriSource {
    pub(crate) fn new(socket: std::path::PathBuf) -> Self {
        Self { socket }
    }

    fn ask(&self) -> std::io::Result<String> {
        let mut stream = std::os::unix::net::UnixStream::connect(&self.socket)?;
        stream.set_read_timeout(Some(FOCUS_READ_TIMEOUT))?;
        stream.set_write_timeout(Some(FOCUS_READ_TIMEOUT))?;
        stream.write_all(b"\"FocusedWindow\"\n")?;
        stream.flush()?;
        let mut line = String::new();
        std::io::BufReader::new(stream.take(MAX_REPLY)).read_line(&mut line)?;
        Ok(line)
    }
}

impl FocusSource for NiriSource {
    fn name(&self) -> &'static str {
        "niri"
    }

    fn read(&self) -> FocusRead {
        match self.ask() {
            Ok(line) => parse_niri_reply(&line),
            Err(err) => FocusRead::Unreadable(format!("niri socket: {err}")),
        }
    }
}

/// One niri reply line. A window with no app id is a window that is not
/// Overwatch; `null` is no focused window, which niri reports reliably
/// (its XWayland windows are its own windows too).
pub(crate) fn parse_niri_reply(line: &str) -> FocusRead {
    let Ok(reply) = serde_json::from_str::<serde_json::Value>(line.trim()) else {
        return FocusRead::Unreadable("niri reply is not JSON".into());
    };
    if let Some(err) = reply.get("Err") {
        let why = err.as_str().unwrap_or("an error");
        return FocusRead::Unreadable(format!("niri answered: {why}"));
    }
    let Some(window) = reply.get("Ok").and_then(|ok| ok.get("FocusedWindow")) else {
        return FocusRead::Unreadable("niri reply has no focused window field".into());
    };
    if window.is_null() {
        return FocusRead::NoWindow;
    }
    if !window.is_object() {
        return FocusRead::Unreadable("niri focused window is not an object".into());
    }
    let ids = window
        .get("app_id")
        .and_then(serde_json::Value::as_str)
        .map(|id| vec![id.to_string()])
        .unwrap_or_default();
    FocusRead::Window(ids)
}
