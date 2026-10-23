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
    let tmp = data_dir.join(format!("{TOKEN_FILE}.tmp"));
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
