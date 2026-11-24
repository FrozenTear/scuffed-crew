//! The ScreenCast restore token file (`screencast-restore-token`, 0600,
//! tmp + fsync + rename).

use std::path::{Path, PathBuf};

pub(super) const TOKEN_FILE: &str = "screencast-restore-token";

/// True when a restore token is on disk. The bytes are not logged.
pub fn restore_token_saved(data_dir: &Path) -> bool {
    read_token(data_dir).is_some()
}

pub(super) fn token_path(data_dir: &Path) -> PathBuf {
    data_dir.join(TOKEN_FILE)
}

/// The temp file [`write_token`] renames over [`token_path`].
fn token_tmp_path(data_dir: &Path) -> PathBuf {
    data_dir.join(format!("{TOKEN_FILE}.tmp"))
}

/// Delete the saved restore token and its temp file, so the next share asks
/// for permission again. A file that is not there is not an error. Both
/// files are tried even when the first fails; the first error is returned.
///
/// Only call this while the daemon is stopped: a running share would write
/// a new token when the portal hands one out.
pub fn remove_restore_token(data_dir: &Path) -> std::io::Result<()> {
    let mut first_err = None;
    for path in [token_path(data_dir), token_tmp_path(data_dir)] {
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => {
                first_err.get_or_insert(e);
            }
        }
    }
    first_err.map_or(Ok(()), Err)
}

pub(super) fn read_token(data_dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(token_path(data_dir)).ok()?;
    let token = text.trim();
    if token.is_empty() {
        None
    } else {
        Some(token.to_string())
    }
}

pub(super) fn write_token(data_dir: &Path, token: &str) {
    if token.is_empty() {
        return;
    }
    let path = token_path(data_dir);
    let tmp = token_tmp_path(data_dir);
    let _ = std::fs::remove_file(&tmp);
    use std::io::Write;
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let Ok(mut file) = options.open(&tmp) else {
        return;
    };
    if file.write_all(token.as_bytes()).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return;
    }
    let _ = file.sync_all();
    drop(file);
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn remove_restore_token_deletes_the_token_and_its_tmp() {
        let dir = tempfile::tempdir().expect("tempdir");
        write_token(dir.path(), "abc");
        std::fs::write(token_tmp_path(dir.path()), "half").unwrap();
        assert!(restore_token_saved(dir.path()));

        remove_restore_token(dir.path()).expect("remove");
        assert!(!token_path(dir.path()).exists());
        assert!(!token_tmp_path(dir.path()).exists());
        // Nothing left to remove is fine.
        remove_restore_token(dir.path()).expect("second remove");
    }

    #[test]
    fn remove_restore_token_still_removes_the_tmp_when_the_token_fails() {
        let dir = tempfile::tempdir().expect("tempdir");
        // A directory at the token path makes remove_file fail.
        std::fs::create_dir(token_path(dir.path())).unwrap();
        std::fs::write(token_tmp_path(dir.path()), "half").unwrap();

        remove_restore_token(dir.path()).expect_err("token path is a directory");
        assert!(!token_tmp_path(dir.path()).exists());
    }
}
