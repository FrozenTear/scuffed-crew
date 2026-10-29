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

/// Where the daemon for `data_dir` listens. The data dir, unless that path
/// is too long for a Unix socket; then `$XDG_RUNTIME_DIR`, named after a
/// hash of the data dir so two data dirs never share one.
pub fn socket_path(data_dir: &Path) -> PathBuf {
    let direct = data_dir.join(SOCKET_FILE);
    if direct.as_os_str().len() <= MAX_SOCKET_PATH {
        return direct;
    }
    use sha2::Digest as _;
    let hash = sha2::Sha256::digest(data_dir.as_os_str().as_encoded_bytes());
    let short: String = hash[..8].iter().map(|b| format!("{b:02x}")).collect();
    let dir = std::env::var_os("XDG_RUNTIME_DIR")
        .filter(|d| !d.is_empty())
        .map(PathBuf::from)
        .unwrap_or_else(std::env::temp_dir);
    dir.join(format!("scuffed-stat-tracker-{short}.sock"))
}

#[cfg(test)]
mod tests;
