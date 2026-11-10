//! The daemon's query channel for the desktop app's page totals.
//!
//! One request per connection over a Unix socket that only the user can
//! open (`tracker.sock` in the data dir, mode 0600, peer uid checked by the
//! daemon): one JSON line in, one JSON line out. The daemon answers from
//! the same rows and the same functions the pages use
//! (`games::rows::games_from_snapshot`, `games::aggregate`), so a total from
//! the daemon equals the total the GUI computes from `live_snapshot.json`.
//! `rev` moves on every stored change, so a client asks again only then.
//! When the daemon is stopped there is no socket and the GUI keeps reading
//! the compact `live_snapshot.json`.

use std::path::{Path, PathBuf};

pub mod client;
pub mod proto;

/// The socket's file name in the data dir.
pub const SOCKET_FILE: &str = "tracker.sock";

/// Longest socket path the kernel takes (`sun_path` is 108 bytes with its
/// terminating zero).
const MAX_SOCKET_PATH: usize = 107;

/// Where the daemon for `data_dir` listens: the data dir, unless that
/// path is too long for a Unix socket; then `$XDG_RUNTIME_DIR` (private to
/// the user), named after a hash of the data dir so two data dirs never
/// share one. `None` when neither works: no socket, and the app reads
/// `live_snapshot.json`. Never a shared directory such as `/tmp`.
pub fn socket_path(data_dir: &Path) -> Option<PathBuf> {
    let direct = data_dir.join(SOCKET_FILE);
    if direct.as_os_str().len() <= MAX_SOCKET_PATH {
        return Some(direct);
    }
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(data_dir.as_os_str().as_encoded_bytes());
    let short: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    let dir = std::env::var_os("XDG_RUNTIME_DIR").filter(|d| !d.is_empty())?;
    let path = PathBuf::from(dir).join(format!("scuffed-stat-tracker-{short}.sock"));
    (path.as_os_str().len() <= MAX_SOCKET_PATH).then_some(path)
}

/// The uid of the process on the other end of a Unix socket (`SO_PEERCRED`).
pub fn peer_uid(fd: std::os::fd::BorrowedFd<'_>) -> std::io::Result<u32> {
    use std::os::fd::AsRawFd as _;
    let mut cred = libc::ucred {
        pid: 0,
        uid: 0,
        gid: 0,
    };
    let mut len = std::mem::size_of::<libc::ucred>() as libc::socklen_t;
    // SAFETY: `cred` and `len` are valid out pointers of the right size.
    let rc = unsafe {
        libc::getsockopt(
            fd.as_raw_fd(),
            libc::SOL_SOCKET,
            libc::SO_PEERCRED,
            (&mut cred as *mut libc::ucred).cast(),
            &mut len,
        )
    };
    if rc == 0 {
        Ok(cred.uid)
    } else {
        Err(std::io::Error::last_os_error())
    }
}

/// This process's effective uid.
pub fn my_uid() -> u32 {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() }
}

#[cfg(test)]
mod tests;
