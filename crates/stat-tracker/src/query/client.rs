//! The blocking client the desktop app uses. Call it off the UI thread.

use std::io::{Read as _, Write as _};
use std::os::fd::FromRawFd as _;
use std::path::Path;
use std::time::Duration;

use super::proto::{PROTO, Request, RequestLine, Response};

/// Short reads: `rev` and `hello`.
pub const QUICK: Duration = Duration::from_millis(300);
/// A page of totals or the games list.
pub const SLOW: Duration = Duration::from_secs(5);

/// Longest response read.
const MAX_RESPONSE: u64 = 16 * 1024 * 1024;

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
    let deadline = std::time::Instant::now() + timeout;
    let mut stream = match connect_within(&path, timeout) {
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
    // One deadline for the whole answer: a server that trickles bytes
    // cannot hold the call past it.
    let mut body = Vec::new();
    let mut chunk = [0u8; 64 * 1024];
    loop {
        let left = deadline.saturating_duration_since(std::time::Instant::now());
        if left.is_zero() {
            return Err(QueryError::Io(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "the tracker did not answer in time",
            )));
        }
        stream
            .set_read_timeout(Some(left))
            .map_err(QueryError::Io)?;
        match stream.read(&mut chunk) {
            Ok(0) => break,
            Ok(n) => {
                body.extend_from_slice(&chunk[..n]);
                if body.len() as u64 > MAX_RESPONSE {
                    return Err(QueryError::Bad("answer too large".into()));
                }
            }
            Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
            Err(e) => return Err(QueryError::Io(e)),
        }
    }
    let response: Response =
        serde_json::from_slice(&body).map_err(|e| QueryError::Bad(e.to_string()))?;
    match response {
        Response::Err { code, msg } => Err(QueryError::Bad(format!("{code}: {msg}"))),
        ok => Ok(ok),
    }
}

/// Connect without waiting past `timeout` and without a helper thread: a
/// non-blocking connect, then a wait for it to finish. A daemon whose
/// backlog is full answers `EAGAIN` at once, which fails here as a timeout.
fn connect_within(
    path: &Path,
    timeout: Duration,
) -> std::io::Result<std::os::unix::net::UnixStream> {
    use std::os::unix::ffi::OsStrExt as _;
    let bytes = path.as_os_str().as_bytes();
    // SAFETY: sockaddr_un is a plain C struct; all-zero is valid.
    let mut addr: libc::sockaddr_un = unsafe { std::mem::zeroed() };
    addr.sun_family = libc::AF_UNIX as libc::sa_family_t;
    if bytes.len() >= addr.sun_path.len() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "socket path too long",
        ));
    }
    for (d, b) in addr.sun_path.iter_mut().zip(bytes) {
        *d = *b as libc::c_char;
    }
    // SAFETY: plain socket(2) call.
    let fd = unsafe {
        libc::socket(
            libc::AF_UNIX,
            libc::SOCK_STREAM | libc::SOCK_NONBLOCK | libc::SOCK_CLOEXEC,
            0,
        )
    };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    // SAFETY: `fd` is a socket this call just opened and nothing else owns.
    let stream = unsafe { std::os::unix::net::UnixStream::from_raw_fd(fd) };
    // SAFETY: `addr` is a valid sockaddr_un of the given size.
    let rc = unsafe {
        libc::connect(
            fd,
            (&addr as *const libc::sockaddr_un).cast(),
            std::mem::size_of::<libc::sockaddr_un>() as libc::socklen_t,
        )
    };
    if rc != 0 {
        let err = std::io::Error::last_os_error();
        match err.raw_os_error() {
            Some(libc::EINPROGRESS) => {
                let mut pfd = libc::pollfd {
                    fd,
                    events: libc::POLLOUT,
                    revents: 0,
                };
                let ms = timeout.as_millis().min(i32::MAX as u128) as i32;
                // SAFETY: one valid pollfd.
                let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
                if ready <= 0 {
                    return Err(std::io::Error::new(
                        std::io::ErrorKind::TimedOut,
                        "the tracker did not accept in time",
                    ));
                }
                if let Some(e) = stream.take_error()? {
                    return Err(e);
                }
            }
            Some(libc::EAGAIN) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "the tracker is not accepting",
                ));
            }
            _ => return Err(err),
        }
    }
    stream.set_nonblocking(false)?;
    stream.set_write_timeout(Some(timeout))?;
    Ok(stream)
}
