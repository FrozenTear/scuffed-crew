//! Settings and the CLI / env overlay of `Config::load`.
//!
//! The app shows and acts on the config in effect: config.toml with the
//! overlay on top (the sync token and server, debug images, OCR workers),
//! as `Config::load` builds it for the tracker. That is
//! [`TrackerApp::saved_config`]. Saves work on config.toml as written
//! ([`TrackerApp::saved_file`]): the "changed on disk" check compares the
//! file with the file, and a value the overlay put in is written back as
//! the file had it unless the save changed it. So an override is never
//! stored, and the first change after launch is not refused because the
//! environment differs from the file.

use std::path::Path;

use stat_tracker::config::{Config, EnvOverlay};

use crate::app::TrackerApp;

/// What the GUI loads at launch.
pub(crate) struct Loaded {
    /// config.toml as written (the default config when there is no file).
    pub file: Config,
    /// `file` with the overlay on top.
    pub shown: Config,
    /// config.toml did not parse. Both configs are then the default, and
    /// no save may write over the file.
    pub unreadable: bool,
}

/// Read config.toml at `path` for the GUI. A fixture (demo or test) never
/// reads the host's file or environment, so it shows no real token and
/// tests do not depend on the host.
pub(crate) fn load(path: Option<&Path>, fixture: bool) -> Loaded {
    let Some(path) = path.filter(|_| !fixture) else {
        return Loaded {
            file: Config::default(),
            shown: Config::default(),
            unreadable: false,
        };
    };
    match Config::read_at(path) {
        Ok(file) => Loaded {
            shown: shown_from_file(&file, path),
            file,
            unreadable: false,
        },
        Err(e) => {
            tracing::warn!(
                error = %e,
                "config.toml did not parse; Settings will not overwrite it"
            );
            Loaded {
                file: Config::default(),
                shown: Config::default(),
                unreadable: true,
            }
        }
    }
}

/// The config in effect for `file`, which `path` holds.
pub(crate) fn shown_from_file(file: &Config, path: &Path) -> Config {
    file.clone().with_env_overlay(path)
}

/// `next` was built from the shown config. Every value the overlay sets
/// that the save left as shown goes back to the file's value, so the save
/// writes only what the player changed.
pub(crate) fn keep_file_values(next: &mut Config, shown: &Config, file: &Config) {
    if next.debug_ocr == shown.debug_ocr {
        next.debug_ocr = file.debug_ocr;
    }
    if next.ocr_threads == shown.ocr_threads {
        next.ocr_threads = file.ocr_threads;
    }
    if next.sync == shown.sync {
        next.sync.clone_from(&file.sync);
    }
}

/// The website URL and token a save writes, from the form's `url` and
/// `token`. A value the overlay put in (`SCUFFED_TOKEN`, `SCUFFED_SERVER`)
/// that the form still shows goes back to the file's, field by field, so
/// no override reaches config.toml. With no token in the file the token
/// comes back blank, so the save leaves the file's sync as it was (a URL
/// typed next to the outside token cannot be stored without a token).
pub(crate) fn sync_fields_to_write(
    url: &str,
    token: &str,
    shown: &Config,
    file: &Config,
    env: EnvOverlay,
) -> (String, String) {
    let Some(in_effect) = shown.sync.as_ref().filter(|_| shown.sync != file.sync) else {
        return (url.to_string(), token.to_string());
    };
    let from_file = file.sync.as_ref();
    let token = if token.trim() == in_effect.token {
        from_file.map(|s| s.token.clone()).unwrap_or_default()
    } else {
        token.to_string()
    };
    let url = if url.trim() != in_effect.server_url {
        url.to_string()
    } else if let Some(s) = from_file {
        s.server_url.clone()
    } else if env.server {
        String::new()
    } else {
        // The URL of a token-less [sync] table: the file's own.
        url.to_string()
    };
    (url, token)
}

/// The variable that turns debug images on outside the app.
pub(crate) const DEBUG_OCR_ENV: &str = "STAT_TRACKER_DEBUG_OCR";

/// The variables that set the website token (and URL) outside the app,
/// for a lock line. `None` when no token comes from outside: a server
/// without a token changes nothing.
pub(crate) fn sync_env(overlay: EnvOverlay) -> Option<&'static str> {
    match (overlay.token, overlay.server) {
        (true, true) => Some("SCUFFED_TOKEN and SCUFFED_SERVER"),
        (true, false) => Some("SCUFFED_TOKEN"),
        (false, _) => None,
    }
}

impl TrackerApp {
    /// What this process's CLI / env overlay sets, for the lock lines. A
    /// fixture (demo or test) shows none, as [`load`] never reads the
    /// host's environment for one.
    pub(crate) fn env_overlay(&self) -> EnvOverlay {
        if self.fixture.is_some() {
            EnvOverlay::default()
        } else {
            Config::env_overlay()
        }
    }

    /// `SCUFFED_TOKEN` (or `--token`) sets the website token outside the
    /// app: the 0.6 page locks the Website URL, the token and their Apply,
    /// and its saves leave the file's sync as it is.
    pub(crate) fn sync_locked(&self) -> bool {
        sync_env(self.env_overlay()).is_some()
    }

    /// Take in `file`, now what config.toml holds at `path`.
    pub(crate) fn take_saved_file(&mut self, file: Config, path: &Path) {
        self.saved_config = shown_from_file(&file, path);
        self.saved_file = file;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stat_tracker::config::SyncConfig;

    fn sync(token: &str) -> Option<SyncConfig> {
        Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: token.into(),
        })
    }

    #[test]
    fn an_override_left_as_shown_is_not_written() {
        let file = Config::default();
        let mut shown = file.clone();
        shown.debug_ocr = true;
        shown.ocr_threads = Some(3);
        shown.sync = sync("from-env");
        let mut next = shown.clone();
        next.player_name = Some("FrozenTear".into());
        keep_file_values(&mut next, &shown, &file);
        assert!(!next.debug_ocr);
        assert_eq!(next.ocr_threads, None);
        assert_eq!(next.sync, None);
        assert_eq!(next.player_name.as_deref(), Some("FrozenTear"));
    }

    #[test]
    fn a_change_the_player_made_is_written() {
        let file = Config::default();
        let mut shown = file.clone();
        shown.sync = sync("from-env");
        let mut next = shown.clone();
        next.sync = sync("pasted");
        next.ocr_threads = Some(2);
        keep_file_values(&mut next, &shown, &file);
        assert_eq!(next.sync, sync("pasted"));
        assert_eq!(next.ocr_threads, Some(2));
    }

    fn file_and_shown(file_token: Option<&str>, env: (&str, &str)) -> (Config, Config) {
        let mut file = Config::default();
        file.sync = file_token.and_then(sync);
        let mut shown = file.clone();
        shown.sync = Some(SyncConfig {
            server_url: env.0.into(),
            token: env.1.into(),
        });
        (file, shown)
    }

    const TOKEN_ONLY: EnvOverlay = EnvOverlay {
        token: true,
        server: false,
        debug_ocr: false,
        ocr_threads: None,
    };

    #[test]
    fn an_outside_token_left_as_shown_goes_back_to_the_file_token() {
        let (file, shown) = file_and_shown(Some("T_file"), ("https://crew.example", "T_env"));
        // Only the URL changed: the file's token goes with it.
        let (url, token) =
            sync_fields_to_write("https://new.example", "T_env", &shown, &file, TOKEN_ONLY);
        assert_eq!(
            (url.as_str(), token.as_str()),
            ("https://new.example", "T_file")
        );
        // A token the player typed is theirs to write.
        let (_, token) =
            sync_fields_to_write("https://crew.example", "pasted", &shown, &file, TOKEN_ONLY);
        assert_eq!(token, "pasted");
    }

    #[test]
    fn an_outside_server_left_as_shown_goes_back_to_the_file_url() {
        let env = EnvOverlay {
            server: true,
            ..TOKEN_ONLY
        };
        let (file, shown) = file_and_shown(Some("T_file"), ("http://env.example", "T_env"));
        let (url, token) = sync_fields_to_write("http://env.example", "pasted", &shown, &file, env);
        assert_eq!(
            (url.as_str(), token.as_str()),
            ("https://crew.example", "pasted")
        );
        // No sync in the file: neither outside value is written.
        let (file, shown) = file_and_shown(None, ("http://env.example", "T_env"));
        let (url, token) = sync_fields_to_write("http://env.example", "T_env", &shown, &file, env);
        assert_eq!((url.as_str(), token.as_str()), ("", ""));
    }

    #[test]
    fn without_an_outside_sync_the_form_is_written_as_typed() {
        let (file, _) = file_and_shown(Some("T_file"), ("", ""));
        let shown = file.clone();
        let (url, token) =
            sync_fields_to_write(" https://typed ", "typed", &shown, &file, TOKEN_ONLY);
        assert_eq!((url.as_str(), token.as_str()), (" https://typed ", "typed"));
    }

    #[test]
    fn a_fixture_shows_no_outside_values() {
        let (_dir, app) = crate::fixtures::test_app();
        assert!(app.fixture.is_some());
        assert_eq!(app.env_overlay(), EnvOverlay::default());
        assert!(!app.sync_locked());
    }

    #[test]
    fn a_fixture_never_reads_the_host() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        std::fs::write(&path, "data_dir = \"/x\"\nplayer_name = \"host\"\n").expect("seed");
        let loaded = load(Some(&path), true);
        assert_eq!(loaded.file, Config::default());
        assert_eq!(loaded.shown, Config::default());
        let live = load(Some(&path), false);
        assert_eq!(live.file.player_name.as_deref(), Some("host"));
        assert!(!live.unreadable);
    }

    /// No config.toml: the file snapshot is the default config, the same
    /// parse every save re-reads, so the first change applies.
    #[test]
    fn a_missing_file_is_the_default_snapshot() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let loaded = load(Some(&path), false);
        assert_eq!(loaded.file, Config::read_at(&path).expect("missing file"));
        assert!(!path.exists(), "loading never writes the file");
    }

    #[test]
    fn a_token_from_outside_names_its_variables() {
        let none = EnvOverlay::default();
        assert_eq!(sync_env(none), None);
        let server_only = EnvOverlay {
            server: true,
            ..none
        };
        assert_eq!(sync_env(server_only), None);
        let token = EnvOverlay {
            token: true,
            ..none
        };
        assert_eq!(sync_env(token), Some("SCUFFED_TOKEN"));
        let both = EnvOverlay {
            token: true,
            server: true,
            ..none
        };
        assert_eq!(sync_env(both), Some("SCUFFED_TOKEN and SCUFFED_SERVER"));
    }
}
