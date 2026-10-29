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

/// Ask the daemon for `data_dir`. The connect, each write and the read are
/// bounded by `timeout` (a connect to a live but stuck daemon would
/// otherwise wait), and the answer counts only from a process of this user.
pub fn ask(data_dir: &Path, request: &Request, timeout: Duration) -> Result<Response, QueryError> {
    let Some(path) = super::socket_path(data_dir) else {
        return Err(QueryError::NotRunning);
    };
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = tx.send(std::os::unix::net::UnixStream::connect(&path));
    });
    let mut stream = match rx.recv_timeout(timeout) {
        Ok(Ok(s)) => s,
        Ok(Err(e))
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::ConnectionRefused
            ) =>
        {
            return Err(QueryError::NotRunning);
        }
        Ok(Err(e)) => return Err(QueryError::Io(e)),
        Err(_) => {
            return Err(QueryError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the tracker did not accept in time",
            )));
        }
    };
    {
        use std::os::fd::AsFd as _;
        let peer = super::peer_uid(stream.as_fd()).map_err(QueryError::Io)?;
        if peer != super::my_uid() {
            return Err(QueryError::Bad("the socket belongs to another user".into()));
        }
    }
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
