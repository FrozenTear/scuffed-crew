//! The pipe KWin writes the pixels into: buffer size, closing our write
//! end, and reading exactly the announced bytes, then EOF, by a deadline.

use std::os::fd::{AsRawFd, OwnedFd};
use std::time::Instant;

use super::protocol::KwinError;

// -------------------------------------------------------------------- pipe

/// Asks the kernel for a 1 MiB pipe buffer, so KWin's writer needs fewer
/// rounds for a 10 to 30 MB picture. A refusal keeps the default.
pub(super) fn widen_pipe(fd: &OwnedFd) {
    // SAFETY: fcntl on a live fd; the result is only a hint.
    unsafe {
        libc::fcntl(fd.as_raw_fd(), libc::F_SETPIPE_SZ, 1 << 20);
    }
}

/// Closes our copy of the write end, then reads the pixels. KWin holds its
/// own copy and closes it after the last byte; ours must be gone first, or
/// EOF never comes and the read ends at the deadline with an error.
pub fn finish_pipe(
    reader: OwnedFd,
    our_writer: OwnedFd,
    len: usize,
    deadline: Instant,
) -> Result<Vec<u8>, KwinError> {
    drop(our_writer);
    read_exactly(reader, len, deadline)
}

/// Reads exactly `len` bytes from `fd`, then EOF, all before `deadline`.
/// Fewer bytes before EOF, more bytes than `len`, or no EOF by the deadline
/// is an error: KWin writes `stride * height` bytes and closes its end.
pub fn read_exactly(fd: OwnedFd, len: usize, deadline: Instant) -> Result<Vec<u8>, KwinError> {
    let raw = fd.as_raw_fd();
    // SAFETY: fcntl on an fd this function owns.
    unsafe {
        let flags = libc::fcntl(raw, libc::F_GETFL);
        if flags < 0 || libc::fcntl(raw, libc::F_SETFL, flags | libc::O_NONBLOCK) < 0 {
            return Err(KwinError::Other(format!(
                "pipe: {}",
                std::io::Error::last_os_error()
            )));
        }
    }
    let mut buf = vec![0u8; len];
    let mut filled = 0usize;
    let mut extra = [0u8; 1];
    loop {
        let full = filled == len;
        let target: &mut [u8] = if full { &mut extra } else { &mut buf[filled..] };
        // SAFETY: `target` is a live, writable buffer of the given length.
        let n = unsafe { libc::read(raw, target.as_mut_ptr().cast(), target.len()) };
        if n > 0 {
            if full {
                return Err(KwinError::BadReply(format!(
                    "more than the {len} bytes the reply announced"
                )));
            }
            filled += n as usize;
            continue;
        }
        if n == 0 {
            return if full {
                Ok(buf)
            } else {
                Err(KwinError::BadReply(format!(
                    "{filled} of {len} bytes before the pipe closed"
                )))
            };
        }
        let err = std::io::Error::last_os_error();
        match err.kind() {
            std::io::ErrorKind::Interrupted => continue,
            std::io::ErrorKind::WouldBlock => {}
            _ => return Err(KwinError::Other(format!("pipe read: {err}"))),
        }
        let wait = deadline.saturating_duration_since(Instant::now());
        if wait.is_zero() {
            return Err(if full {
                KwinError::BadReply("the pipe stayed open after the picture".into())
            } else {
                KwinError::Timeout
            });
        }
        let mut pfd = libc::pollfd {
            fd: raw,
            events: libc::POLLIN,
            revents: 0,
        };
        let ms = i32::try_from(wait.as_millis().max(1)).unwrap_or(i32::MAX);
        // SAFETY: one pollfd for an fd this function owns.
        let ready = unsafe { libc::poll(&mut pfd, 1, ms) };
        if ready < 0 {
            let err = std::io::Error::last_os_error();
            if err.kind() != std::io::ErrorKind::Interrupted {
                return Err(KwinError::Other(format!("pipe poll: {err}")));
            }
        }
    }
}
