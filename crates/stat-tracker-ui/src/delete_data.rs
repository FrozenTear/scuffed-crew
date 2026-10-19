//! Delete local tracker data from Settings.
//!
//! The app stays installed. Downloaded hero, letter, word, and map packs stay.
//! Settings go back to defaults unless the member keeps them. The sync token
//! and site link are removed either way.
//!
//! When the member also asks to delete site stats, `DELETE /api/stats/me`
//! runs with the tracker sync token before any local file or token is
//! removed. The account on the site stays.
//!
//! File removal is [`crate::uninstall::purge_saved_games`] and
//! [`crate::uninstall::purge_settings`]. The capture daemon is stopped with
//! the same checks as uninstall ([`crate::daemon::stop_before_file_removal`]).

use std::path::{Path, PathBuf};
use std::time::Duration;

use stat_tracker::config::{Config, SyncConfig};

use crate::daemon::SYSTEMD_UNIT;

/// The member must type this before anything is deleted.
pub const CONFIRM_WORD: &str = "DELETE";

/// Shown after a local wipe that left site stats in place.
pub const WIPED_COPY: &str = "\
Everything on this PC is deleted. Stats already synced to the site stay there until you delete them on the site.";

/// Shared ending of the site success line. Undo is on the website only.
/// This app does not call a restore endpoint.
pub const SITE_DELETE_UNDO: &str =
    "You can undo this from your account page on the website until then.";

/// Used when a 200 body has no usable `stats_delete_scheduled_at`.
pub const SITE_DELETE_FALLBACK_WHEN: &str = "in 7 days";

/// JSON body for `DELETE /api/stats/me`. Exact and case-sensitive.
const SITE_DELETE_BODY: &str = r#"{"confirm":"delete my stats"}"#;

/// Local wipe line plus the site schedule. `when` is a local calendar date.
/// `None` uses [`SITE_DELETE_FALLBACK_WHEN`].
pub fn wiped_with_site(when: Option<&str>) -> String {
    let when = match when {
        Some(date) => format!("on {date}"),
        None => SITE_DELETE_FALLBACK_WHEN.to_string(),
    };
    format!(
        "Everything on this PC is deleted. Your stats on the site will be deleted {when}. {SITE_DELETE_UNDO}"
    )
}

/// Label for the site checkbox. Shown only while a sync token is linked.
pub const SITE_DELETE_LABEL: &str = "Also delete my stats on the site";

/// Shown in the confirm step while the site checkbox is ticked.
pub const SITE_CONFIRM_COPY: &str = "\
Stats on the site from every PC are deleted. Your account stays.";

/// 401 or 403 from `DELETE /api/stats/me`. Local data stays.
pub const SITE_REJECTED: &str = "\
The site did not accept this PC's sign-in. Delete your stats on the website account page instead.";

/// Network error or 5xx from `DELETE /api/stats/me`. Local data stays.
pub const SITE_RETRY: &str = "\
Could not delete stats on the site. Local data was kept. Try again.";

/// The checkbox is shown when saved settings hold a sync token and a site URL.
pub fn site_delete_checkbox_visible(sync: Option<&SyncConfig>) -> bool {
    sync.is_some_and(|sync| !sync.token.trim().is_empty() && !sync.server_url.trim().is_empty())
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
    /// Ticked only while [`site_delete_checkbox_visible`] is true.
    pub also_delete_site: bool,
    pub busy: bool,
    pub notice: Option<String>,
    /// Set after a network error or 5xx. The confirm step offers Try again.
    pub offer_retry: bool,
}

/// Why a wipe stopped. A rejected or unreachable site leaves every local file
/// in place, including the sync token.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WipeFailure {
    /// Network error, 5xx, or a 200 body that was not JSON. Offer retry.
    SiteRetry(String),
    /// 401 or 403. [`SITE_REJECTED`] is the only copy.
    SiteRejected,
    /// Confirmation, lock, or file error. `unlinked` means a 200 already
    /// revoked tokens and the saved sync token was removed.
    Local { message: String, unlinked: bool },
}

impl WipeFailure {
    fn local(message: impl Into<String>) -> Self {
        Self::Local {
            message: message.into(),
            unlinked: false,
        }
    }

    fn local_unlinked(message: impl Into<String>) -> Self {
        Self::Local {
            message: message.into(),
            unlinked: true,
        }
    }

    pub fn message(&self) -> &str {
        match self {
            Self::SiteRetry(msg) | Self::Local { message: msg, .. } => msg,
            Self::SiteRejected => SITE_REJECTED,
        }
    }

    /// The saved sync token is already gone. The app should look unlinked.
    pub fn unlinked(&self) -> bool {
        matches!(self, Self::Local { unlinked: true, .. })
    }
}

impl std::fmt::Display for WipeFailure {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.message())
    }
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
    /// When set, `DELETE /api/stats/me` runs before any local file changes.
    pub also_delete_site: bool,
    pub server_url: String,
    pub sync_token: String,
}

/// Paths the running app wipes. `data_dir` is the folder this window uses.
///
/// `sync` is the saved link. The site call uses that token, not an unsaved
/// settings field.
pub fn request_for_app(
    data_dir: &Path,
    keep_settings: bool,
    also_delete_site: bool,
    sync: Option<&SyncConfig>,
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
    let (server_url, sync_token) = if also_delete_site {
        let sync = sync
            .filter(|sync| site_delete_checkbox_visible(Some(sync)))
            .ok_or_else(|| {
                "This PC is not signed in to the site. Nothing was removed.".to_string()
            })?;
        (sync.server_url.clone(), sync.token.clone())
    } else {
        (String::new(), String::new())
    };
    Ok(WipeRequest {
        data_dir: data_dir.to_path_buf(),
        home,
        prefix,
        config_dir,
        keep_settings,
        confirmation,
        systemctl: PathBuf::from("systemctl"),
        also_delete_site,
        server_url,
        sync_token,
    })
}

/// Stop the capture daemon, then delete local member data.
///
/// A confirmation other than [`CONFIRM_WORD`] deletes nothing. A store lock
/// held by another open deletes nothing, including logs and the sync token.
///
/// When `also_delete_site` is set, the site call finishes before the daemon
/// is stopped and before any local file or token is removed. A rejected or
/// unreachable site leaves the local data in place.
pub async fn wipe_local_data(req: WipeRequest) -> Result<String, WipeFailure> {
    if !confirmation_accepted(&req.confirmation) {
        return Err(WipeFailure::local(
            "Type DELETE to confirm. Nothing was removed.",
        ));
    }
    if let Err(msg) = preflight(&req) {
        return Err(WipeFailure::local(msg));
    }
    ensure_store_unlocked(&req.data_dir)
        .await
        .map_err(WipeFailure::local)?;
    // `Some` means the site returned 200 and revoked every token. The inner
    // value is the local calendar date, or `None` when the body had no usable
    // timestamp.
    let scheduled = if req.also_delete_site {
        match delete_site_stats(&req.server_url, &req.sync_token).await? {
            SiteCall::Scheduled { when } => {
                clear_sync_token(&req.config_dir).map_err(WipeFailure::local)?;
                Some(when)
            }
            SiteCall::NotFound => None,
        }
    } else {
        None
    };
    let site_scheduled = scheduled.is_some();
    stop_capture(&req.data_dir, &req.systemctl)
        .await
        .map_err(|msg| local_after_site(site_scheduled, msg))?;
    ensure_store_unlocked(&req.data_dir)
        .await
        .map_err(|msg| local_after_site(site_scheduled, msg))?;
    if let Err(still) = crate::uninstall::purge_saved_games(&req.data_dir, &req.home, &req.prefix) {
        return Err(local_after_site(
            site_scheduled,
            format!("Delete stopped. Still there: {}.", still.display()),
        ));
    }
    crate::uninstall::remove_own_name_seed(&req.data_dir, &req.home, &req.prefix);
    apply_settings(&req).map_err(|msg| local_after_site(site_scheduled, msg))?;
    Ok(match scheduled {
        Some(when) => wiped_with_site(when.as_deref()),
        None if req.also_delete_site => wiped_with_site(None),
        None => WIPED_COPY.to_string(),
    })
}

fn local_after_site(site_scheduled: bool, msg: String) -> WipeFailure {
    if site_scheduled {
        let msg = msg.replace("Nothing was removed.", "Local data is still on this PC.");
        WipeFailure::local_unlinked(format!(
            "{msg} Your stats on the site are scheduled for deletion. This PC is no longer linked. A new token is needed to link again."
        ))
    } else {
        WipeFailure::local(msg)
    }
}

fn preflight(req: &WipeRequest) -> Result<(), String> {
    if !crate::uninstall::confined_path(&req.data_dir, &req.home, &req.prefix) {
        return Err("Saved games are outside your home folder. Nothing was removed.".into());
    }
    ensure_settings_ready(req)
}

/// Result of `DELETE /api/stats/me` when the local wipe should continue.
#[derive(Debug, Clone, PartialEq, Eq)]
enum SiteCall {
    /// 200. Every token is revoked. `when` is the local date to show.
    Scheduled { when: Option<String> },
    /// 404. Nothing was scheduled, and tokens were not revoked.
    NotFound,
}

/// `DELETE {base}/api/stats/me` with the tracker sync token and
/// [`SITE_DELETE_BODY`].
///
/// 200 with JSON, and 404, continue the local wipe. The caller removes the
/// saved token only after 200. Any other result returns before local files
/// change. The token is not logged. Undo stays on the website.
async fn delete_site_stats(server_url: &str, token: &str) -> Result<SiteCall, WipeFailure> {
    if token.trim().is_empty() || server_url.trim().is_empty() {
        return Err(WipeFailure::local(
            "This PC is not signed in to the site. Nothing was removed.",
        ));
    }
    let base = crate::setup_guide::normalize_base(server_url)
        .map_err(|err| WipeFailure::local(format!("{err} Nothing was removed.")))?;
    let url = format!("{base}/api/stats/me");
    let client = reqwest::Client::builder()
        .timeout(Duration::from_secs(30))
        .build()
        .map_err(|_| WipeFailure::SiteRetry(SITE_RETRY.into()))?;
    let response = client
        .delete(&url)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, "application/json")
        .body(SITE_DELETE_BODY)
        .send()
        .await
        .map_err(|_| WipeFailure::SiteRetry(SITE_RETRY.into()))?;
    let status = response.status().as_u16();
    let body = response.text().await.unwrap_or_default();
    interpret_site_response(status, &body)
}

/// Contract for `DELETE /api/stats/me`.
///
/// 200 must be JSON. Unknown fields are ignored. `stats_delete_scheduled_at`
/// is an RFC3339 timestamp shown as a local date. A missing or unparseable
/// timestamp uses [`SITE_DELETE_FALLBACK_WHEN`]. 404 means there was nothing
/// to delete. 401 and 403 are [`WipeFailure::SiteRejected`]. 5xx, other
/// statuses, and a non-JSON 200 are [`WipeFailure::SiteRetry`].
fn interpret_site_response(status: u16, body: &str) -> Result<SiteCall, WipeFailure> {
    match status {
        200 => match serde_json::from_str::<serde_json::Value>(body) {
            Ok(value) => Ok(SiteCall::Scheduled {
                when: scheduled_local_date(&value),
            }),
            Err(_) => Err(WipeFailure::SiteRetry(SITE_RETRY.into())),
        },
        404 => Ok(SiteCall::NotFound),
        401 | 403 => Err(WipeFailure::SiteRejected),
        _ => Err(WipeFailure::SiteRetry(SITE_RETRY.into())),
    }
}

/// `stats_delete_scheduled_at` as a local calendar date. Missing, non-string,
/// and unparseable values return `None` so the caller can use the fallback.
fn scheduled_local_date(value: &serde_json::Value) -> Option<String> {
    let raw = value.get("stats_delete_scheduled_at")?.as_str()?;
    let parsed = chrono::DateTime::parse_from_rfc3339(raw).ok()?;
    Some(
        parsed
            .with_timezone(&chrono::Local)
            .format("%d %b %Y")
            .to_string(),
    )
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
            also_delete_site: false,
            server_url: String::new(),
            sync_token: String::new(),
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
    fn site_checkbox_needs_a_linked_token_and_copy_has_no_em_dash() {
        assert!(!site_delete_checkbox_visible(None));
        assert!(!site_delete_checkbox_visible(Some(&SyncConfig {
            server_url: "https://example.test".into(),
            token: "  ".into(),
        })));
        assert!(!site_delete_checkbox_visible(Some(&SyncConfig {
            server_url: String::new(),
            token: "synthetic-token-value".into(),
        })));
        assert!(site_delete_checkbox_visible(Some(&SyncConfig {
            server_url: "https://example.test".into(),
            token: "synthetic-token-value".into(),
        })));
        assert_eq!(WipeFailure::SiteRejected.message(), SITE_REJECTED);
        let dated = wiped_with_site(Some("26 Oct 2026"));
        let fallback = wiped_with_site(None);
        for copy in [
            WIPED_COPY,
            dated.as_str(),
            fallback.as_str(),
            SITE_DELETE_UNDO,
            SITE_DELETE_FALLBACK_WHEN,
            SITE_DELETE_LABEL,
            SITE_CONFIRM_COPY,
            SITE_REJECTED,
            SITE_RETRY,
        ] {
            assert!(!copy.contains('\u{2014}'), "{copy}");
        }
        assert!(SITE_CONFIRM_COPY.contains("every PC"));
        assert!(SITE_CONFIRM_COPY.contains("account stays"));
        assert_eq!(
            dated,
            "Everything on this PC is deleted. Your stats on the site will be deleted on 26 Oct 2026. You can undo this from your account page on the website until then."
        );
        assert!(fallback.contains("will be deleted in 7 days"));
        assert!(fallback.contains(SITE_DELETE_UNDO));
        assert!(!dated.contains("stay there"));
        assert_eq!(SITE_DELETE_BODY, r#"{"confirm":"delete my stats"}"#);
    }

    #[test]
    fn site_response_reads_the_schedule_and_keeps_data_otherwise() {
        let body = r#"{"scheduled":true,"stats_delete_scheduled_at":"2026-10-26T15:04:05Z","tokens_revoked":2,"unexpected":true}"#;
        let SiteCall::Scheduled { when } = interpret_site_response(200, body).unwrap() else {
            panic!("200 should schedule");
        };
        let expected = scheduled_local_date(
            &serde_json::json!({"stats_delete_scheduled_at": "2026-10-26T15:04:05Z"}),
        );
        assert_eq!(when, expected);
        assert!(when.unwrap().contains("2026"));

        let SiteCall::Scheduled { when } =
            interpret_site_response(200, r#"{"scheduled":true,"tokens_revoked":0,"extra":1}"#)
                .unwrap()
        else {
            panic!("missing date");
        };
        assert!(when.is_none());

        let SiteCall::Scheduled { when } = interpret_site_response(
            200,
            r#"{"scheduled":true,"stats_delete_scheduled_at":"not-a-date","tokens_revoked":1}"#,
        )
        .unwrap() else {
            panic!("bad date");
        };
        assert!(when.is_none());

        assert!(matches!(
            interpret_site_response(404, "missing"),
            Ok(SiteCall::NotFound)
        ));
        assert_eq!(
            interpret_site_response(401, "no"),
            Err(WipeFailure::SiteRejected)
        );
        assert_eq!(
            interpret_site_response(403, "no"),
            Err(WipeFailure::SiteRejected)
        );
        assert_eq!(
            interpret_site_response(200, "not-json"),
            Err(WipeFailure::SiteRetry(SITE_RETRY.into()))
        );
        assert_eq!(
            interpret_site_response(500, "down"),
            Err(WipeFailure::SiteRetry(SITE_RETRY.into()))
        );
        assert_eq!(
            interpret_site_response(503, "down"),
            Err(WipeFailure::SiteRetry(SITE_RETRY.into()))
        );
        assert_eq!(
            interpret_site_response(400, "bad"),
            Err(WipeFailure::SiteRetry(SITE_RETRY.into()))
        );
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
            assert!(
                err.message().contains("Nothing was removed"),
                "{typed}: {err}"
            );
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
            err.message().contains("Stop the tracker")
                && err.message().contains("Nothing was removed"),
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
        assert!(err.message().contains("Nothing was removed"), "{err}");
        assert!(outside.join("debug/crop.png").is_file());
    }

    fn spawn_mock(handler: impl Fn(&str) -> (u16, &'static str) + Send + Sync + 'static) -> String {
        use std::io::{Read, Write};
        use std::sync::Arc;

        let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("bind");
        let port = listener.local_addr().expect("addr").port();
        let handler = Arc::new(handler);
        std::thread::spawn(move || {
            for _ in 0..4 {
                let Ok((mut stream, _)) = listener.accept() else {
                    break;
                };
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(2)));
                let mut buf = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    match stream.read(&mut tmp) {
                        Ok(0) => break,
                        Ok(n) => {
                            buf.extend_from_slice(&tmp[..n]);
                            if request_complete(&buf) {
                                break;
                            }
                        }
                        Err(_) => break,
                    }
                }
                let text = String::from_utf8_lossy(&buf).to_string();
                let (status, body) = handler(&text);
                let resp = format!(
                    "HTTP/1.1 {status} OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
                let _ = stream.write_all(resp.as_bytes());
            }
        });
        format!("http://127.0.0.1:{port}")
    }

    fn request_complete(buf: &[u8]) -> bool {
        let text = String::from_utf8_lossy(buf);
        let Some(split) = text.find("\r\n\r\n") else {
            return false;
        };
        let headers = &text[..split];
        let body = &text[split + 4..];
        let length = headers
            .lines()
            .find_map(|line| {
                let rest = line
                    .strip_prefix("Content-Length:")
                    .or_else(|| line.strip_prefix("content-length:"))?;
                rest.trim().parse::<usize>().ok()
            })
            .unwrap_or(0);
        body.len() >= length
    }

    fn assert_site_delete(raw: &str) {
        let lower = raw.to_ascii_lowercase();
        assert!(
            lower.contains("authorization: bearer synthetic-token-value"),
            "{raw}"
        );
        assert!(raw.starts_with("DELETE /api/stats/me "), "{raw}");
        assert!(lower.contains("content-type: application/json"), "{raw}");
        let body = raw.split("\r\n\r\n").nth(1).unwrap_or("");
        assert_eq!(body, SITE_DELETE_BODY);
    }

    fn assert_local_kept(fix: &Fixture) {
        assert!(fix.data.join("debug/accepted/board.png").is_file());
        assert!(fix.data.join("gui.log").is_file());
        assert!(fix.data.join("sync_auth.json").is_file());
        assert!(fix.data.join("stats.surrealkv.pre-vacuum-9").exists());
        let config = fs::read_to_string(fix.config.join("config.toml")).unwrap();
        assert!(config.contains("synthetic-token-value"), "{config}");
        assert!(!fix.log.exists(), "daemon was stopped");
    }

    #[tokio::test]
    async fn site_delete_runs_before_the_local_token_is_removed() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        seed_member_data(&fix);
        seed_packs_and_app(&fix);
        let config_path = fix.config.join("config.toml");
        let base = spawn_mock(move |raw| {
            assert_site_delete(raw);
            let on_disk = fs::read_to_string(&config_path).unwrap();
            assert!(
                on_disk.contains("synthetic-token-value"),
                "token was wiped before the site call: {on_disk}"
            );
            (
                200,
                r#"{"scheduled":true,"stats_delete_scheduled_at":"2026-10-26T15:04:05Z","tokens_revoked":2,"unexpected":true}"#,
            )
        });
        let mut req = request(&fix, "DELETE", true);
        req.also_delete_site = true;
        req.server_url = base;
        req.sync_token = "synthetic-token-value".into();

        let copy = wipe_local_data(req).await.expect("wipe");
        let when = scheduled_local_date(
            &serde_json::json!({"stats_delete_scheduled_at": "2026-10-26T15:04:05Z"}),
        );
        assert_eq!(copy, wiped_with_site(when.as_deref()));
        assert_member_data_gone(&fix);
        assert_packs_and_app(&fix);
        let config = fs::read_to_string(fix.config.join("config.toml")).unwrap();
        assert!(config.contains("KeptName"), "{config}");
        assert!(!config.contains("synthetic-token-value"), "{config}");
    }

    #[tokio::test]
    async fn site_not_found_still_wipes_this_pc() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        seed_member_data(&fix);
        seed_packs_and_app(&fix);
        let base = spawn_mock(|raw| {
            assert_site_delete(raw);
            (404, "nothing")
        });
        let mut req = request(&fix, "DELETE", false);
        req.also_delete_site = true;
        req.server_url = base;
        req.sync_token = "synthetic-token-value".into();

        let copy = wipe_local_data(req).await.expect("wipe");
        assert_eq!(copy, wiped_with_site(None));
        assert_member_data_gone(&fix);
        assert!(!fix.config.exists());
        assert_packs_and_app(&fix);
    }

    #[tokio::test]
    async fn site_rejection_keeps_local_data() {
        for status in [401, 403] {
            let dir = tempfile::tempdir().unwrap();
            let fix = fixture(dir.path());
            seed_member_data(&fix);
            seed_packs_and_app(&fix);
            let base = spawn_mock(move |raw| {
                assert_site_delete(raw);
                (status, "rejected")
            });
            let mut req = request(&fix, "DELETE", false);
            req.also_delete_site = true;
            req.server_url = base;
            req.sync_token = "synthetic-token-value".into();

            let err = wipe_local_data(req).await.expect_err("rejected");
            assert_eq!(err, WipeFailure::SiteRejected, "{status}");
            assert_eq!(err.message(), SITE_REJECTED);
            assert_local_kept(&fix);
            assert_packs_and_app(&fix);
        }
    }

    #[tokio::test]
    async fn site_server_error_keeps_local_data_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        seed_member_data(&fix);
        seed_packs_and_app(&fix);
        let base = spawn_mock(|raw| {
            assert_site_delete(raw);
            (500, "down")
        });
        let mut req = request(&fix, "DELETE", false);
        req.also_delete_site = true;
        req.server_url = base;
        req.sync_token = "synthetic-token-value".into();

        let err = wipe_local_data(req).await.expect_err("retry");
        assert_eq!(err, WipeFailure::SiteRetry(SITE_RETRY.into()));
        assert_local_kept(&fix);
        assert_packs_and_app(&fix);
    }

    #[tokio::test]
    async fn site_network_error_keeps_local_data_for_retry() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        seed_member_data(&fix);
        seed_packs_and_app(&fix);
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let port = listener.local_addr().unwrap().port();
        drop(listener);
        let mut req = request(&fix, "DELETE", false);
        req.also_delete_site = true;
        req.server_url = format!("http://127.0.0.1:{port}");
        req.sync_token = "synthetic-token-value".into();

        let err = wipe_local_data(req).await.expect_err("network");
        assert_eq!(err, WipeFailure::SiteRetry(SITE_RETRY.into()));
        assert_local_kept(&fix);
        assert_packs_and_app(&fix);
    }

    #[tokio::test]
    async fn site_call_waits_for_confirmation_and_a_token() {
        let dir = tempfile::tempdir().unwrap();
        let fix = fixture(dir.path());
        seed_member_data(&fix);
        let hits = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
        let hits_handler = hits.clone();
        let base = spawn_mock(move |_| {
            hits_handler.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            (200, "{}")
        });

        let mut unconfirmed = request(&fix, "delete", false);
        unconfirmed.also_delete_site = true;
        unconfirmed.server_url = base.clone();
        unconfirmed.sync_token = "synthetic-token-value".into();
        let err = wipe_local_data(unconfirmed).await.expect_err("confirm");
        assert!(err.message().contains("Nothing was removed"), "{err}");

        let mut unsigned = request(&fix, "DELETE", false);
        unsigned.also_delete_site = true;
        unsigned.server_url = base;
        unsigned.sync_token.clear();
        let err = wipe_local_data(unsigned).await.expect_err("token");
        assert!(err.message().contains("Nothing was removed"), "{err}");
        assert_eq!(hits.load(std::sync::atomic::Ordering::SeqCst), 0);
        let config = fs::read_to_string(fix.config.join("config.toml")).unwrap();
        assert!(config.contains("synthetic-token-value"), "{config}");
    }

    #[test]
    fn request_for_app_refuses_site_delete_without_a_saved_token() {
        let dir = tempfile::tempdir().unwrap();
        let err =
            request_for_app(dir.path(), false, true, None, "DELETE".into()).expect_err("unlinked");
        assert!(err.contains("not signed in"), "{err}");
        let err = request_for_app(
            dir.path(),
            false,
            true,
            Some(&SyncConfig {
                server_url: "https://example.test".into(),
                token: String::new(),
            }),
            "DELETE".into(),
        )
        .expect_err("empty token");
        assert!(err.contains("not signed in"), "{err}");
    }
}
