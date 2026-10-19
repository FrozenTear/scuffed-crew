//! Delete local tracker data from Settings.
//!
//! The app stays installed. Downloaded hero, letter, word, and map packs stay.
//! Settings go back to defaults unless the member keeps them. The sync token
//! and site link are removed either way.
//!
//! File removal is [`crate::uninstall::purge_saved_games`] and
//! [`crate::uninstall::purge_settings`]. The capture daemon is stopped with
//! the same checks as uninstall ([`crate::daemon::stop_before_file_removal`]).

use std::path::{Path, PathBuf};

use stat_tracker::config::Config;

use crate::daemon::SYSTEMD_UNIT;

/// The member must type this before anything is deleted.
pub const CONFIRM_WORD: &str = "DELETE";

/// Shown after a wipe. Stats already on the site are left there.
pub const WIPED_COPY: &str = "\
Everything on this PC is deleted. Stats already synced to the site stay there until you delete them on the site.";

/// Label for the site button. Hidden until [`SITE_STATS_DELETE_READY`].
pub const SITE_DELETE_LABEL: &str = "Also delete my stats on the site";

/// The site button stays hidden until this is true.
///
/// TODO: awaits the server endpoint that deletes synced stats for this
/// account. Do not call any server endpoint from this screen.
pub const SITE_STATS_DELETE_READY: bool = false;

/// Reply when the site button is pressed. No request is sent.
pub const SITE_DELETE_UNAVAILABLE: &str = "Deleting stats on the site is not available yet.";

/// True only while [`SITE_STATS_DELETE_READY`] is set. The button is omitted
/// from Settings until then.
pub fn site_delete_button_visible() -> bool {
    SITE_STATS_DELETE_READY
}

/// The site action. This does not build a client and does not call a server.
///
/// TODO: awaits the server endpoint. Wire that here when it exists, and
/// only then set [`SITE_STATS_DELETE_READY`].
pub fn refuse_site_delete() -> &'static str {
    SITE_DELETE_UNAVAILABLE
}

/// `typed` is the confirmation field, with surrounding spaces ignored.
pub fn confirmation_accepted(typed: &str) -> bool {
    typed.trim() == CONFIRM_WORD
}

/// Settings state for the Delete my data section.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DeleteDataForm {
    pub armed: bool,
    pub typed: String,
    pub keep_settings: bool,
    pub busy: bool,
    pub notice: Option<String>,
}

/// What to wipe. `systemctl` is the binary used to stop the user unit.
/// Tests pass a stand-in script. The app passes `systemctl` on `PATH`.
#[derive(Debug, Clone)]
pub struct WipeRequest {
    pub data_dir: PathBuf,
    pub home: PathBuf,
    pub prefix: PathBuf,
    pub config_dir: PathBuf,
    pub keep_settings: bool,
    pub confirmation: String,
    pub systemctl: PathBuf,
}

/// Paths the running app wipes. `data_dir` is the folder this window uses.
pub fn request_for_app(
    data_dir: &Path,
    keep_settings: bool,
    confirmation: String,
) -> Result<WipeRequest, String> {
    let home = dirs::home_dir()
        .filter(|home| crate::uninstall::home_is_usable(home))
        .ok_or_else(|| "Could not find your home folder. Nothing was removed.".to_string())?;
    let prefix = std::env::current_exe()
        .ok()
        .and_then(|exe| crate::uninstall::prefix_from_gui(&exe))
        .filter(|prefix| prefix.starts_with(&home))
        .unwrap_or_else(|| home.join(".local"));
    let config_dir = Config::config_path()
        .ok()
        .and_then(|path| path.parent().map(Path::to_path_buf))
        .unwrap_or_else(|| home.join(".config").join("scuffed-stat-tracker"));
    Ok(WipeRequest {
        data_dir: data_dir.to_path_buf(),
        home,
        prefix,
        config_dir,
        keep_settings,
        confirmation,
        systemctl: PathBuf::from("systemctl"),
    })
}

/// Stop the capture daemon, then delete local member data.
///
/// A confirmation other than [`CONFIRM_WORD`] deletes nothing. A store lock
/// held by another open deletes nothing, including logs and the sync token.
pub async fn wipe_local_data(req: WipeRequest) -> Result<&'static str, String> {
    if !confirmation_accepted(&req.confirmation) {
        return Err("Type DELETE to confirm. Nothing was removed.".into());
    }
    stop_capture(&req.data_dir, &req.systemctl).await?;
    if !crate::uninstall::confined_path(&req.data_dir, &req.home, &req.prefix) {
        return Err("Saved games are outside your home folder. Nothing was removed.".into());
    }
    ensure_settings_ready(&req)?;
    ensure_store_unlocked(&req.data_dir).await?;
    if let Err(still) = crate::uninstall::purge_saved_games(&req.data_dir, &req.home, &req.prefix) {
        return Err(format!("Delete stopped. Still there: {}.", still.display()));
    }
    crate::uninstall::remove_own_name_seed(&req.data_dir, &req.home, &req.prefix);
    apply_settings(&req)?;
    Ok(WIPED_COPY)
}

async fn stop_capture(data_dir: &Path, systemctl: &Path) -> Result<(), String> {
    // Stop the user unit when it is installed so systemd does not start the
    // daemon again. This is `stop`, not `disable`: the app stays installed.
    if crate::uninstall::systemctl_exists(systemctl) {
        match crate::uninstall::run_systemctl(systemctl, &["--user", "stop", SYSTEMD_UNIT]) {
            Ok(output) => {
                if !output.status.success()
                    && !crate::uninstall::unit_is_missing(&output)
                    && crate::daemon::daemon_running(data_dir).is_some()
                {
                    return Err("Could not stop the tracker. Nothing was removed.".into());
                }
            }
            Err(_) if crate::daemon::daemon_running(data_dir).is_some() => {
                return Err("Could not stop the tracker. Nothing was removed.".into());
            }
            Err(_) => {}
        }
    }
    crate::daemon::stop_before_file_removal(data_dir).await
}

/// `LocalStore::open` takes the same directory lock as the daemon. A busy
/// lock is not a broken store: deleting files under it would half-wipe the
/// database the daemon still has open.
async fn ensure_store_unlocked(data_dir: &Path) -> Result<(), String> {
    let db_path = data_dir.join("stats.surrealkv");
    if !db_path.exists() {
        return Ok(());
    }
    match stat_tracker::storage::LocalStore::open(data_dir).await {
        Ok(store) => {
            drop(store);
            Ok(())
        }
        Err(err) if stat_tracker::storage::is_store_busy(err.as_ref()) => {
            Err("Stop the tracker before deleting stored data. Nothing was removed.".into())
        }
        Err(_) => Ok(()),
    }
}

fn settings_files(config_dir: &Path) -> [PathBuf; 2] {
    [
        config_dir.join("config.toml"),
        config_dir.join("session.env"),
    ]
}

fn ensure_settings_ready(req: &WipeRequest) -> Result<(), String> {
    let present = settings_files(&req.config_dir)
        .into_iter()
        .any(|path| path.exists());
    if !present {
        return Ok(());
    }
    if !crate::uninstall::confined_path(&req.config_dir, &req.home, &req.prefix) {
        return Err("Settings are outside your home folder. Nothing was removed.".into());
    }
    if req.keep_settings {
        let path = req.config_dir.join("config.toml");
        if path.is_file() {
            Config::read_at(&path)
                .map_err(|_| "Could not read settings. Nothing was removed.".to_string())?;
        }
    }
    Ok(())
}

fn apply_settings(req: &WipeRequest) -> Result<(), String> {
    if req.keep_settings {
        clear_sync_token(&req.config_dir)
    } else if let Err(still) =
        crate::uninstall::purge_settings(&req.config_dir, &req.home, &req.prefix)
    {
        Err(format!(
            "Saved games were deleted, but settings are still there: {}.",
            still.display()
        ))
    } else {
        Ok(())
    }
}

fn clear_sync_token(config_dir: &Path) -> Result<(), String> {
    let path = config_dir.join("config.toml");
    if !path.is_file() {
        return Ok(());
    }
    let mut config = Config::read_at(&path)
        .map_err(|_| "Could not read settings. Nothing was removed.".to_string())?;
    config.sync = None;
    config
        .save_at(&path)
        .map_err(|_| "Could not remove the sync token.".to_string())?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use std::os::unix::fs::PermissionsExt;

    struct Fixture {
        home: PathBuf,
        prefix: PathBuf,
        data: PathBuf,
        config: PathBuf,
        systemctl: PathBuf,
        log: PathBuf,
    }

    fn write(path: &Path, body: &str) {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent).unwrap();
        }
        fs::write(path, body).unwrap();
    }

    fn fixture(root: &Path) -> Fixture {
        let home = root.join("home");
        let prefix = home.join(".local");
        let data = home.join(".local/share/scuffed-stat-tracker");
        let config = home.join(".config/scuffed-stat-tracker");
        let log = root.join("systemctl.log");
        let systemctl = root.join("systemctl");
        fs::create_dir_all(&home).unwrap();
        write(
            &systemctl,
            &format!(
                "#!/bin/sh\nprintf '%s\\n' \"$*\" >> '{}'\nexit 0\n",
                log.display()
            ),
        );
        let mut perms = fs::metadata(&systemctl).unwrap().permissions();
        perms.set_mode(0o755);
        fs::set_permissions(&systemctl, perms).unwrap();
        Fixture {
            home,
            prefix,
            data,
            config,
            systemctl,
            log,
        }
    }

    fn seed_member_data(fix: &Fixture) {
        write(&fix.data.join("matches.jsonl"), "{\"session_id\":\"s1\"}\n");
        write(&fix.data.join("live_snapshot.json"), "{}\n");
        write(
            &fix.data.join("stats.surrealkv.pre-vacuum-9/db"),
            "backup\n",
        );
        write(&fix.data.join("debug/accepted/board.png"), "scoreboard\n");
        write(&fix.data.join("debug/mapcrops/crop.png"), "map crop\n");
        write(&fix.data.join("private/own-name/seeds.json"), "crops\n");
        write(&fix.data.join("portraits/row.png"), "crop\n");
        write(&fix.data.join("report-kit/s1/scoreboard.png"), "kit\n");
        write(&fix.data.join("reports/bundle.zip"), "bundle\n");
        write(&fix.data.join("scuffed-report-1.zip"), "bundle\n");
        write(&fix.data.join("perf.log"), "cpu=1\n");
        write(&fix.data.join("perf.log.1"), "cpu=0\n");
        write(&fix.data.join("gui.log"), "read value synthetic\n");
        write(&fix.data.join("gui.log.1"), "read value synthetic old\n");
        write(&fix.data.join("daemon.log"), "read value synthetic\n");
        write(&fix.data.join("daemon.log.1"), "read value synthetic old\n");
        write(&fix.data.join("shadow/digits.jsonl"), "digits\n");
        write(
            &fix.data.join("sync_auth.json"),
            "{\"token\":\"synthetic-token-value\"}\n",
        );
        write(&fix.data.join("daemon.pid"), "not-running\n");
        write(
            &fix.config.join("config.toml"),
            "\
data_dir = \"/tmp/sst-delete-data\"
player_name = \"KeptName\"

[sync]
server_url = \"https://example.test\"
token = \"synthetic-token-value\"
",
        );
        write(
            &fix.config.join("session.env"),
            "WAYLAND_DISPLAY=wayland-1\n",
        );
    }

    fn seed_packs_and_app(fix: &Fixture) {
        write(
            &fix.data.join("templates/heroes/ana.png"),
            "synthetic-hero\n",
        );
        write(
            &fix.data.join("templates/heroes-v2/ana.png"),
            "synthetic-hero-v2\n",
        );
        write(
            &fix.data.join("templates/words-v1/victory.png"),
            "synthetic-word\n",
        );
        write(
            &fix.data.join("templates/maps-v1/atlas.json"),
            "synthetic-map\n",
        );
        write(
            &fix.data.join("templates/letters/a.png"),
            "synthetic-letter\n",
        );
        write(&fix.prefix.join("bin/stat-tracker-gui"), "installed\n");
        write(
            &fix.home
                .join(".local/share/applications/scuffed-stat-tracker.desktop"),
            "installed\n",
        );
        write(&fix.home.join("keep-me"), "keep\n");
    }

    fn request(fix: &Fixture, confirmation: &str, keep_settings: bool) -> WipeRequest {
        WipeRequest {
            data_dir: fix.data.clone(),
            home: fix.home.clone(),
            prefix: fix.prefix.clone(),
            config_dir: fix.config.clone(),
            keep_settings,
            confirmation: confirmation.to_string(),
            systemctl: fix.systemctl.clone(),
        }
    }

    fn assert_packs_and_app(fix: &Fixture) {
        assert_eq!(
            fs::read_to_string(fix.data.join("templates/heroes/ana.png")).unwrap(),
            "synthetic-hero\n"
        );
        assert_eq!(
            fs::read_to_string(fix.data.join("templates/heroes-v2/ana.png")).unwrap(),
            "synthetic-hero-v2\n"
        );
        assert_eq!(
            fs::read_to_string(fix.data.join("templates/words-v1/victory.png")).unwrap(),
            "synthetic-word\n"
        );
        assert_eq!(
            fs::read_to_string(fix.data.join("templates/maps-v1/atlas.json")).unwrap(),
            "synthetic-map\n"
        );
        assert_eq!(
            fs::read_to_string(fix.data.join("templates/letters/a.png")).unwrap(),
            "synthetic-letter\n"
        );
        assert_eq!(
            fs::read_to_string(fix.prefix.join("bin/stat-tracker-gui")).unwrap(),
            "installed\n"
        );
        assert!(
            fix.home
                .join(".local/share/applications/scuffed-stat-tracker.desktop")
                .is_file()
        );
        assert_eq!(
            fs::read_to_string(fix.home.join("keep-me")).unwrap(),
            "keep\n"
        );
    }

    fn assert_member_data_gone(fix: &Fixture) {
        let gone = [
            "stats.surrealkv",
            "stats.surrealkv.pre-vacuum-9",
            "matches.jsonl",
            "live_snapshot.json",
            "debug",
            "private",
            "portraits",
            "report-kit",
            "reports",
            "scuffed-report-1.zip",
            "perf.log",
            "perf.log.1",
            "gui.log",
            "gui.log.1",
            "daemon.log",
            "daemon.log.1",
            "shadow",
            "sync_auth.json",
            "daemon.pid",
        ];
        for name in gone {
            assert!(!fix.data.join(name).exists(), "{name} was not deleted");
        }
    }

    #[test]
    fn confirmation_requires_the_word_delete() {
        assert!(confirmation_accepted("DELETE"));
        assert!(confirmation_accepted("  DELETE  "));
        assert!(!confirmation_accepted(""));
        assert!(!confirmation_accepted("delete"));
        assert!(!confirmation_accepted("DELET"));
    }

    #[test]
    fn site_delete_stays_hidden_and_sends_nothing() {
        const {
            assert!(!SITE_STATS_DELETE_READY);
        }
        assert!(!site_delete_button_visible());
        assert_eq!(refuse_site_delete(), SITE_DELETE_UNAVAILABLE);
        assert!(!WIPED_COPY.contains('\u{2014}'));
        assert!(!SITE_DELETE_LABEL.contains('\u{2014}'));
    }

    #[tokio::test]
    async fn wipe_removes_each_listed_path_and_keeps_packs() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        stat_tracker::storage::LocalStore::open(&fix.data)
            .await
            .expect("open store");
        seed_member_data(&fix);
        seed_packs_and_app(&fix);

        let copy = wipe_local_data(request(&fix, "DELETE", false))
            .await
            .expect("wipe");
        assert_eq!(copy, WIPED_COPY);
        assert_member_data_gone(&fix);
        assert_packs_and_app(&fix);
        assert!(!fix.config.exists(), "settings directory was kept");
        let log = fs::read_to_string(&fix.log).unwrap();
        assert!(
            log.contains("--user stop scuffed-stat-tracker.service"),
            "{log}"
        );
        assert!(!log.contains("disable"), "{log}");
    }

    #[tokio::test]
    async fn wipe_removes_the_token_and_can_keep_other_settings() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        seed_member_data(&fix);
        seed_packs_and_app(&fix);

        let copy = wipe_local_data(request(&fix, "DELETE", true))
            .await
            .expect("wipe");
        assert_eq!(copy, WIPED_COPY);
        assert_member_data_gone(&fix);
        assert_packs_and_app(&fix);
        let config = fs::read_to_string(fix.config.join("config.toml")).unwrap();
        assert!(config.contains("KeptName"), "{config}");
        assert!(!config.contains("synthetic-token-value"), "{config}");
        assert!(!config.contains("example.test"), "{config}");
        assert_eq!(
            fs::read_to_string(fix.config.join("session.env")).unwrap(),
            "WAYLAND_DISPLAY=wayland-1\n"
        );
    }

    #[tokio::test]
    async fn wipe_refuses_without_confirmation() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        seed_member_data(&fix);
        seed_packs_and_app(&fix);

        for typed in ["", "delete", "DELET"] {
            let err = wipe_local_data(request(&fix, typed, false))
                .await
                .expect_err(typed);
            assert!(err.contains("Nothing was removed"), "{typed}: {err}");
        }
        assert!(fix.data.join("debug/accepted/board.png").is_file());
        assert!(fix.data.join("gui.log").is_file());
        assert!(fix.data.join("perf.log").is_file());
        assert!(fix.data.join("report-kit/s1/scoreboard.png").is_file());
        assert!(fix.data.join("templates/heroes/ana.png").is_file());
        let config = fs::read_to_string(fix.config.join("config.toml")).unwrap();
        assert!(config.contains("synthetic-token-value"), "{config}");
        assert!(!fix.log.exists(), "stop ran without confirmation");
    }

    #[tokio::test]
    async fn wipe_does_not_touch_a_locked_store() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        let _held = stat_tracker::storage::LocalStore::open(&fix.data)
            .await
            .expect("open");
        seed_member_data(&fix);
        seed_packs_and_app(&fix);

        let err = wipe_local_data(request(&fix, "DELETE", false))
            .await
            .expect_err("locked store");
        assert!(
            err.contains("Stop the tracker") && err.contains("Nothing was removed"),
            "{err}"
        );
        assert!(
            fix.data.join("stats.surrealkv").exists(),
            "locked store was wiped"
        );
        assert!(fix.data.join("live_snapshot.json").is_file());
        assert!(fix.data.join("matches.jsonl").is_file());
        assert!(fix.data.join("debug/accepted/board.png").is_file());
        assert!(fix.data.join("private/own-name/seeds.json").is_file());
        assert!(fix.data.join("report-kit/s1/scoreboard.png").is_file());
        assert!(fix.data.join("reports/bundle.zip").is_file());
        assert!(fix.data.join("perf.log").is_file());
        assert!(fix.data.join("gui.log").is_file());
        assert!(fix.data.join("daemon.log").is_file());
        assert!(fix.data.join("sync_auth.json").is_file());
        let config = fs::read_to_string(fix.config.join("config.toml")).unwrap();
        assert!(config.contains("synthetic-token-value"), "{config}");
        assert_packs_and_app(&fix);
    }

    #[tokio::test]
    async fn wipe_refuses_a_data_dir_outside_home() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        let outside = dir.path().join("outside-data");
        write(&outside.join("debug/crop.png"), "crop\n");
        let mut req = request(&fix, "DELETE", false);
        req.data_dir = outside.clone();
        let err = wipe_local_data(req).await.expect_err("outside");
        assert!(err.contains("Nothing was removed"), "{err}");
        assert!(outside.join("debug/crop.png").is_file());
    }
}
