//! What the systemd user unit can write under `ProtectSystem=strict`.
//!
//! The unit allow-lists the default data directory, the config directory, and
//! the session runtime directory (`%t`). A `data_dir` anywhere else is
//! read-only when that sandbox is actually applied (it is a no-op on a user
//! manager that cannot set up a mount namespace). `install.sh` writes a
//! drop-in for the path in `config.toml`. If the directory is still not
//! writable at startup, the daemon names the same drop-in and stops.

use std::path::{Component, Path};

/// `$HOME/.local/share/scuffed-stat-tracker` — default data dir, tessdata, store.
pub const DEFAULT_DATA_REL: &str = ".local/share/scuffed-stat-tracker";
/// `$HOME/.config/scuffed-stat-tracker` — `config.toml` and `session.env`.
pub const CONFIG_REL: &str = ".config/scuffed-stat-tracker";

const DROPIN_MARKER: &str = "scuffed-stat-tracker data_dir drop-in";

/// True when `data_dir` is the default data dir, the config dir, the runtime
/// dir, or a child of one of those. Relative paths are not covered: the unit
/// cannot name them.
pub fn data_dir_covered_by_unit(data_dir: &Path, home: &Path, runtime_dir: Option<&Path>) -> bool {
    if !data_dir.is_absolute() {
        return false;
    }
    let data = components_of(data_dir);
    let under_home = |rel: &str| data.starts_with(&components_of(&home.join(rel)));
    if under_home(DEFAULT_DATA_REL) || under_home(CONFIG_REL) {
        return true;
    }
    runtime_dir.is_some_and(|rt| rt.is_absolute() && data.starts_with(&components_of(rt)))
}

/// `ReadWritePaths=` token for one absolute path. `%` is escaped so systemd
/// does not treat it as a specifier. A missing directory is ignored (`-`).
pub fn read_write_paths_token(path: &Path) -> Option<String> {
    if !path.is_absolute() {
        return None;
    }
    let raw = path.to_str()?;
    if raw.contains('\n') || raw.contains('\0') {
        return None;
    }
    let escaped = raw.replace('%', "%%");
    let needs_quote = escaped
        .chars()
        .any(|c| c.is_whitespace() || c == '"' || c == '\\');
    if needs_quote {
        let quoted = escaped.replace('\\', "\\\\").replace('"', "\\\"");
        Some(format!("\"-{quoted}\""))
    } else {
        Some(format!("-{escaped}"))
    }
}

/// Drop-in body install.sh writes for a custom `data_dir`. `None` when the
/// path cannot be expressed as a single systemd token.
pub fn data_dir_dropin(data_dir: &Path) -> Option<String> {
    let token = read_write_paths_token(data_dir)?;
    Some(format!(
        "# {DROPIN_MARKER}\n\
         # ProtectSystem=strict only writes the default data dir, the config dir,\n\
         # and the session runtime dir. This path is outside those.\n\
         [Service]\n\
         ReadWritePaths={token}\n"
    ))
}

/// Text the daemon logs when `data_dir` cannot be created or written.
pub fn unwritable_data_dir_hint(data_dir: &Path) -> String {
    let token = read_write_paths_token(data_dir).unwrap_or_else(|| "-<absolute-data-dir>".into());
    format!(
        "data directory {} is not writable. The systemd user unit sets \
         ProtectSystem=strict, which leaves only ~/.local/share/scuffed-stat-tracker, \
         ~/.config/scuffed-stat-tracker, and the session runtime dir writable. \
         Tessdata stays under the default data dir. The GUI's update staging \
         is /tmp and is not part of this unit. Reinstall so install.sh can \
         write the drop-in, or add it yourself and restart:\n\
         \n\
         mkdir -p ~/.config/systemd/user/scuffed-stat-tracker.service.d\n\
         cat > ~/.config/systemd/user/scuffed-stat-tracker.service.d/data-dir.conf <<'EOF'\n\
         # {DROPIN_MARKER}\n\
         [Service]\n\
         ReadWritePaths={token}\n\
         EOF\n\
         systemctl --user daemon-reload\n\
         systemctl --user restart scuffed-stat-tracker.service",
        data_dir.display()
    )
}

/// Create `data_dir` and confirm this process can write a file there.
/// Permission and read-only failures include [`unwritable_data_dir_hint`].
pub fn ensure_data_dir_writable(data_dir: &Path) -> std::io::Result<()> {
    if let Err(err) = std::fs::create_dir_all(data_dir) {
        return Err(with_sandbox_hint(data_dir, err));
    }
    let probe = data_dir.join(".write-probe");
    if let Err(err) = std::fs::write(&probe, b"ok") {
        return Err(with_sandbox_hint(data_dir, err));
    }
    let _ = std::fs::remove_file(&probe);
    Ok(())
}

fn with_sandbox_hint(data_dir: &Path, err: std::io::Error) -> std::io::Error {
    match err.kind() {
        std::io::ErrorKind::PermissionDenied | std::io::ErrorKind::ReadOnlyFilesystem => {
            let msg = format!("{err}\n\n{}", unwritable_data_dir_hint(data_dir));
            std::io::Error::new(err.kind(), msg)
        }
        _ => err,
    }
}

fn components_of(path: &Path) -> Vec<Component<'_>> {
    path.components().collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_dirs_are_covered_and_a_custom_path_is_not() {
        let home = Path::new("/home/player");
        let runtime = Path::new("/run/user/1000");
        assert!(data_dir_covered_by_unit(
            Path::new("/home/player/.local/share/scuffed-stat-tracker"),
            home,
            Some(runtime),
        ));
        assert!(data_dir_covered_by_unit(
            Path::new("/home/player/.local/share/scuffed-stat-tracker/debug"),
            home,
            Some(runtime),
        ));
        assert!(data_dir_covered_by_unit(
            Path::new("/home/player/.config/scuffed-stat-tracker"),
            home,
            Some(runtime),
        ));
        assert!(data_dir_covered_by_unit(
            Path::new("/run/user/1000/scuffed"),
            home,
            Some(runtime),
        ));
        assert!(
            !data_dir_covered_by_unit(Path::new("/var/lib/scuffed-outside"), home, Some(runtime)),
            "outside the three unit paths"
        );
        assert!(
            !data_dir_covered_by_unit(Path::new("/home/player/games/stats"), home, Some(runtime),),
            "under $HOME but not in ReadWritePaths"
        );
        assert!(
            !data_dir_covered_by_unit(
                Path::new("/home/player/.local/share/scuffed-stat-tracker-extra"),
                home,
                Some(runtime),
            ),
            "a longer directory name is not the default data dir"
        );
        assert!(!data_dir_covered_by_unit(
            Path::new("relative/stats"),
            home,
            Some(runtime),
        ));
    }

    #[test]
    fn dropin_token_quotes_spaces_and_escapes_percent() {
        assert_eq!(
            read_write_paths_token(Path::new("/var/lib/scuffed-outside")).as_deref(),
            Some("-/var/lib/scuffed-outside")
        );
        assert_eq!(
            read_write_paths_token(Path::new("/var/lib/scuffed stats")).as_deref(),
            Some("\"-/var/lib/scuffed stats\"")
        );
        assert_eq!(
            read_write_paths_token(Path::new("/var/lib/scuffed%stats")).as_deref(),
            Some("-/var/lib/scuffed%%stats")
        );
        assert!(read_write_paths_token(Path::new("relative")).is_none());
        let body = data_dir_dropin(Path::new("/var/lib/scuffed-outside")).unwrap();
        assert!(body.contains(DROPIN_MARKER));
        assert!(body.contains("ReadWritePaths=-/var/lib/scuffed-outside\n"));
    }

    #[test]
    fn readonly_hint_names_the_dropin_and_a_real_dir_is_writable() {
        let hint = unwritable_data_dir_hint(Path::new("/var/lib/scuffed-outside"));
        assert!(hint.contains("ProtectSystem=strict"));
        assert!(hint.contains("ReadWritePaths=-/var/lib/scuffed-outside"));
        assert!(hint.contains("systemctl --user daemon-reload"));
        assert!(hint.contains("/tmp"));

        let err = std::io::Error::new(
            std::io::ErrorKind::ReadOnlyFilesystem,
            "read-only file system",
        );
        let wrapped = with_sandbox_hint(Path::new("/var/lib/scuffed-outside"), err);
        assert_eq!(wrapped.kind(), std::io::ErrorKind::ReadOnlyFilesystem);
        assert!(wrapped.to_string().contains("daemon-reload"));

        let dir = tempfile::tempdir().unwrap();
        ensure_data_dir_writable(&dir.path().join("nested")).unwrap();
        assert!(!dir.path().join("nested").join(".write-probe").exists());
    }
}
