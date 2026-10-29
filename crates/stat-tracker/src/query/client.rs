//! The blocking client the desktop app uses. Call it off the UI thread.

use std::io::{Read as _, Write as _};
use std::path::Path;
use std::time::Duration;

use super::proto::{PROTO, Request, RequestLine, Response};

/// Short reads: `rev` and `hello`.
pub const QUICK: Duration = Duration::from_millis(300);
/// A page of totals or the games list.
pub const SLOW: Duration = Duration::from_secs(5);

/// Longest response read.
const MAX_RESPONSE: u64 = 64 * 1024 * 1024;

#[derive(Debug)]
pub enum QueryError {
    /// No daemon listening (no socket, or nobody behind it).
    NotRunning,
    Io(std::io::Error),
    /// An unreadable line, or an `Err` answer.
    Bad(String),
}

impl std::fmt::Display for QueryError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            QueryError::NotRunning => write!(f, "tracker not running"),
            QueryError::Io(e) => write!(f, "{e}"),
            QueryError::Bad(m) => write!(f, "{m}"),
        }
    }
}

impl std::error::Error for QueryError {}

/// Ask the daemon for `data_dir`. A local connect does not wait (it fails
/// at once with no listener); `timeout` bounds each write and read.
pub fn ask(data_dir: &Path, request: &Request, timeout: Duration) -> Result<Response, QueryError> {
    let path = super::socket_path(data_dir);
    let mut stream = match std::os::unix::net::UnixStream::connect(&path) {
        Ok(s) => s,
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(QueryError::NotRunning);
        }
        Err(e) => return Err(QueryError::Io(e)),
    };
    stream
        .set_read_timeout(Some(timeout))
        .map_err(QueryError::Io)?;
    stream
        .set_write_timeout(Some(timeout))
        .map_err(QueryError::Io)?;
    let mut line = serde_json::to_vec(&RequestLine {
        v: PROTO,
        request: request.clone(),
    })
    .map_err(|e| QueryError::Bad(e.to_string()))?;
    line.push(b'\n');
    stream.write_all(&line).map_err(QueryError::Io)?;
    let mut body = Vec::new();
    stream
        .take(MAX_RESPONSE)
        .read_to_end(&mut body)
        .map_err(QueryError::Io)?;
    let response: Response =
        serde_json::from_slice(&body).map_err(|e| QueryError::Bad(e.to_string()))?;
    match response {
        Response::Err { code, msg } => Err(QueryError::Bad(format!("{code}: {msg}"))),
        ok => Ok(ok),
    }
}
