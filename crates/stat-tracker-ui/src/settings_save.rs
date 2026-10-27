//! Writing the Settings form to config.toml, for both pages.
//!
//! The classic page (`SCUFFED_TRACKER_LAYOUT=classic`) saves the whole form
//! with its Save button and toasts the result. The 0.6 page has no Save: it
//! applies each change as it is made ([`SaveFrom::Page`]), stays quiet when
//! that works, and toasts only a refusal. Both go through
//! [`save_settings_form`], which re-reads the file first and never writes
//! over a file that does not parse or that changed on disk.

use iced::Task;
use stat_tracker::config::{Config, EnvOverlay, ShadowRecognizerControl};

use crate::app::{Message, SETTINGS_SAVE_REFUSED, TrackerApp};
use crate::daemon::{self, DaemonVerb};
use crate::hotkey::OverlayHotkey;
use crate::settings::{self, SettingsForm};
use crate::settings_model::ConfigEdit;
use crate::{seasons, update};

/// Shown when the tracker service is already running.
const SETTINGS_SAVED_RESTART: &str =
    "Settings saved. Restart the tracker for changes to take effect.";

/// The file parsed, but it is not the snapshot this form was built from.
const SETTINGS_RELOADED_FROM_DISK: &str = "\
config.toml changed on disk. Settings reloaded from the file, please review and save again.";

/// The same, on the 0.6 page, which has no Save.
const PAGE_RELOADED_FROM_DISK: &str = "\
config.toml changed on disk, so that change was not made. Settings now show the file; make the change again.";

const SETTINGS_SAVED_RESTARTING: &str =
    "Settings saved. Restarting the tracker to use the new screen.";

const SETTINGS_SAVED_ROUTE: &str =
    "Settings saved. Restarting the tracker to use the new capture method.";

const SETTINGS_SAVED_ROUTE_ENV: &str = "Settings saved. STAT_TRACKER_CAPTURE in the environment overrides this choice, so the tracker keeps that route. Edit the file under .config/environment.d, then log out and back in.";

/// Which page saves, and what goes with the change.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum SaveFrom {
    /// Save settings on the classic page: the whole form, with a toast, and
    /// a tracker restart for a new screen or capture method.
    Classic,
    /// The 0.6 page applies one change at once. Typed text that was not
    /// committed stays typed and is not written. Nothing restarts: the
    /// strip counts the change and offers Restart tracker.
    Page(Commit),
}

/// What a 0.6 page apply commits besides the control that was used.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Commit {
    /// A switch, chip, dropdown or radio: no typed text goes with it.
    Control,
    /// Enter in Check every or Wait after detection.
    Timings,
    /// Apply under Website connection: the website URL and token.
    Sync,
    /// The shortcut's box, or Enter in the shortcut field.
    Shortcut,
}

/// Typed text that only an explicit commit writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Draft {
    Timings,
    Sync,
    Shortcut,
}

const DRAFTS: [Draft; 3] = [Draft::Timings, Draft::Sync, Draft::Shortcut];

impl Commit {
    fn commits(self, draft: Draft) -> bool {
        matches!(
            (self, draft),
            (Commit::Timings, Draft::Timings)
                | (Commit::Sync, Draft::Sync)
                | (Commit::Shortcut, Draft::Shortcut)
        )
    }
}

/// Copy one draft group from `from` into `to`.
fn copy_draft(to: &mut SettingsForm, from: &SettingsForm, draft: Draft) {
    match draft {
        Draft::Timings => {
            to.poll_interval_secs.clone_from(&from.poll_interval_secs);
            to.cooldown_secs.clone_from(&from.cooldown_secs);
        }
        Draft::Sync => {
            to.sync_url.clone_from(&from.sync_url);
            to.sync_token.clone_from(&from.sync_token);
        }
        Draft::Shortcut => {
            to.overlay_hotkey.clone_from(&from.overlay_hotkey);
            to.overlay_hotkey_enabled = from.overlay_hotkey_enabled;
        }
    }
}

/// The form a 0.6 page apply writes: `typed`, with every draft group the
/// commit does not cover put back to what is saved (`saved_form`).
pub(crate) fn form_for_commit(
    typed: &SettingsForm,
    saved_form: &SettingsForm,
    commit: Commit,
) -> SettingsForm {
    let mut form = typed.clone();
    for draft in DRAFTS {
        if !commit.commits(draft) {
            copy_draft(&mut form, saved_form, draft);
        }
    }
    form
}

/// After an apply rebuilt the form from the file, put back the text the
/// player had typed but not committed.
pub(crate) fn keep_uncommitted(form: &mut SettingsForm, typed: &SettingsForm, commit: Commit) {
    for draft in DRAFTS {
        if !commit.commits(draft) {
            copy_draft(form, typed, draft);
        }
    }
}

/// After a refused apply rebuilt the form from what is saved, put back all
/// the typed text, the refused group included, so it can be fixed.
fn keep_drafts(form: &mut SettingsForm, typed: &SettingsForm) {
    for draft in DRAFTS {
        copy_draft(form, typed, draft);
    }
}

/// Why typed Check every or Wait after detection text is refused on the
/// 0.6 page, instead of being written as the default. Blank is the default.
pub(crate) fn timings_problem(poll: &str, cooldown: &str) -> Option<&'static str> {
    let poll = poll.trim();
    let cooldown = cooldown.trim();
    if !poll.is_empty() && poll.parse::<u64>().ok().filter(|&s| s > 0).is_none() {
        return Some(POLL_NOT_A_NUMBER);
    }
    if !cooldown.is_empty() && cooldown.parse::<u64>().is_err() {
        return Some(COOLDOWN_NOT_A_NUMBER);
    }
    None
}

const POLL_NOT_A_NUMBER: &str =
    "Check every must be a whole number of seconds, 1 or more. Nothing was changed.";
const COOLDOWN_NOT_A_NUMBER: &str =
    "Wait after detection must be a whole number of seconds. Nothing was changed.";

/// Every Save re-reads the file. `Err` means it still does not parse.
fn reread_config_for_save(path: &std::path::Path) -> Result<Config, &'static str> {
    Config::read_at(path).map_err(|_| SETTINGS_SAVE_REFUSED)
}

/// Result of a save against the config file.
#[derive(Debug, PartialEq)]
pub(crate) enum SettingsFileSave {
    /// The file does not parse. Nothing was written.
    Unreadable,
    /// The form was built while the file was unreadable, or the parsed file
    /// differs from that snapshot. Nothing was written.
    Reloaded(Config),
    /// The on-disk parse matched the snapshot, and the form was written.
    Saved(Config),
}

#[derive(Debug)]
pub(crate) enum SettingsSaveError {
    /// Sync URL or shortcut check failed. The file was not written.
    Blocked(String),
    /// The in-place write failed.
    Write(String),
}

/// Re-read `path`, then either refuse, reload, or write the form in place.
///
/// A write happens only when the file parses as `snapshot`, the file as
/// written when the form was built. The form shows `shown`, that file with
/// the env overlay `env` on top (`settings_env`). The website URL and token
/// the form still shows from the overlay go back to the file's first
/// ([`crate::settings_env::sync_fields_to_write`]). The cleartext URL check
/// then runs on the URL that will be written, so an outside URL never
/// blocks a change, and the other checks run in `before_write`. Both come
/// after the snapshot match, so a reload is not blocked by the empty
/// defaults a broken startup file leaves in the form. The write is
/// `form.to_config(shown)` with `edit` on top, and every overlaid value the
/// form left as shown put back to the file's, then [`Config::save_at`],
/// which edits the file in place.
#[allow(clippy::too_many_arguments)]
pub(crate) fn save_settings_form(
    path: &std::path::Path,
    form_built_unreadable: bool,
    snapshot: &Config,
    shown: &Config,
    env: EnvOverlay,
    form: &SettingsForm,
    edit: Option<ConfigEdit>,
    before_write: impl FnOnce() -> Result<(), String>,
) -> Result<SettingsFileSave, SettingsSaveError> {
    let fresh = match reread_config_for_save(path) {
        Err(_) => return Ok(SettingsFileSave::Unreadable),
        Ok(fresh) => fresh,
    };
    if form_built_unreadable || fresh != *snapshot {
        return Ok(SettingsFileSave::Reloaded(fresh));
    }
    let mut form = form.clone();
    (form.sync_url, form.sync_token) = crate::settings_env::sync_fields_to_write(
        &form.sync_url,
        &form.sync_token,
        shown,
        snapshot,
        env,
    );
    // Block the whole save. Writing the form would either store the
    // cleartext URL or drop the sync block; the file on disk stays as it
    // is until the URL is https, loopback http, or blank.
    if let Some(problem) = settings::sync_url_problem(&form.sync_url) {
        return Err(SettingsSaveError::Blocked(problem.to_string()));
    }
    before_write().map_err(SettingsSaveError::Blocked)?;
    let mut next = form.to_config(shown);
    if let Some(edit) = edit {
        edit.apply(&mut next);
    }
    crate::settings_env::keep_file_values(&mut next, shown, snapshot);
    next.save_at(path)
        .map_err(|err| SettingsSaveError::Write(err.to_string()))?;
    Ok(SettingsFileSave::Saved(next))
}

/// Form fields from `fresh`, keeping the shortcut the member was editing.
fn form_after_disk_reload(
    previous: &SettingsForm,
    fresh: &Config,
    shadow: ShadowRecognizerControl,
) -> SettingsForm {
    let mut next = SettingsForm::from_config_and_shadow(fresh, shadow);
    next.overlay_hotkey = previous.overlay_hotkey.clone();
    next.overlay_hotkey_enabled = previous.overlay_hotkey_enabled;
    next
}

/// True when `after` differs from `before` only in `update_channel`.
///
/// Only this app reads that key, so the save does not need a tracker restart.
fn only_update_channel_changed(before: &Config, after: &Config) -> bool {
    if before.update_channel == after.update_channel {
        return false;
    }
    let mut same = before.clone();
    same.update_channel.clone_from(&after.update_channel);
    same == *after
}

/// True when the save changed the channel the update check uses, so it must
/// run again on the new channel. `installed` matters: on an alpha install an
/// unset channel is already prerelease, so writing `stable` is a switch.
fn update_channel_switched(before: &Config, after: &Config, installed: Option<&str>) -> bool {
    update::prerelease_updates_effective(&before.update_channel, installed)
        != update::prerelease_updates_effective(&after.update_channel, installed)
}

/// True when the save changed which screen the tracker captures. The
/// tracker reads `capture_output` once at start, so the classic page
/// restarts a running tracker after such a save instead of asking the
/// player to.
fn capture_output_changed(before: &Config, after: &Config) -> bool {
    before.capture_output != after.capture_output
}

fn capture_route_changed(before: &Config, after: &Config) -> bool {
    before.capture_route != after.capture_route
}

pub(crate) fn toast_after_successful_save(daemon_up: bool) -> &'static str {
    if daemon_up {
        SETTINGS_SAVED_RESTART
    } else {
        "Settings saved"
    }
}

impl TrackerApp {
    /// Write the form (and `edit`) to config.toml. Demo mode never writes.
    pub(crate) fn save_settings(
        &mut self,
        from: SaveFrom,
        edit: Option<ConfigEdit>,
    ) -> Task<Message> {
        if self.fixture.is_some() {
            return Task::none();
        }
        let config_path = match self.settings_config_path() {
            Ok(path) => path,
            Err(_) => {
                self.config_unreadable = true;
                self.show_refused_toast();
                return Task::none();
            }
        };
        let typed = self.settings.clone();
        let env = self.env_overlay();
        let form = match from {
            SaveFrom::Classic => typed.clone(),
            SaveFrom::Page(commit) => {
                let mut saved_form = SettingsForm::from_config(&self.saved_config);
                saved_form.overlay_hotkey = self.overlay_hotkey.bind.clone();
                saved_form.overlay_hotkey_enabled = self.overlay_hotkey.enabled;
                // A token from outside locks the website fields, so the
                // page never commits them and the file's sync stays.
                let commit = if commit == Commit::Sync && self.sync_locked() {
                    Commit::Control
                } else {
                    commit
                };
                form_for_commit(&typed, &saved_form, commit)
            }
        };
        let saved_ocr = self.saved_config.ocr;
        let form_classic_ocr = form.classic_ocr;
        let known_packs = self.ocr_switch.packs;
        let hotkey_enabled = form.overlay_hotkey_enabled;
        let hotkey_bind = form.overlay_hotkey.clone();
        let timings = (from == SaveFrom::Page(Commit::Timings))
            .then(|| (form.poll_interval_secs.clone(), form.cooldown_secs.clone()));
        let mut stored_hotkey = None;
        let outcome = save_settings_form(
            &config_path,
            self.config_unreadable,
            &self.saved_file,
            &self.saved_config,
            env,
            &form,
            edit,
            || {
                // The page has no Save to back out of, so typed timings
                // that do not parse are refused, not written as defaults.
                if let Some((poll, cooldown)) = &timings
                    && let Some(problem) = timings_problem(poll, cooldown)
                {
                    return Err(problem.to_string());
                }
                // Turning classic OCR off needs a tracker version
                // that can, and the reader packs. The click looked;
                // the save checks what that look found. The tracker
                // itself decides again when it starts.
                if let Some(reason) = settings::ocr_save_problem(
                    saved_ocr,
                    form_classic_ocr,
                    known_packs.as_ref(),
                    &stat_tracker::ocr_switch::Read::needing_ocr(),
                ) {
                    return Err(reason);
                }
                let hotkey = OverlayHotkey::normalized(hotkey_enabled, &hotkey_bind);
                stored_hotkey = Some(hotkey.validate_for_save()?);
                Ok(())
            },
        );
        match outcome {
            Ok(SettingsFileSave::Unreadable) => {
                self.config_unreadable = true;
                self.show_refused_toast();
                self.page_refused(from, &typed);
            }
            Ok(SettingsFileSave::Reloaded(fresh)) => {
                let previous = self.settings.clone();
                self.take_saved_file(fresh, &config_path);
                self.settings = form_after_disk_reload(
                    &previous,
                    &self.saved_config,
                    self.saved_config.shadow_recognizer_control(),
                )
                .with_installed_version(&self.saved_config, self.installed_version.as_deref());
                if let SaveFrom::Page(commit) = from {
                    keep_uncommitted(&mut self.settings, &typed, commit);
                }
                self.config_unreadable = false;
                self.page_state.file_read(&self.saved_config);
                self.refresh_health();
                self.replace_toast(match from {
                    SaveFrom::Classic => SETTINGS_RELOADED_FROM_DISK,
                    SaveFrom::Page(_) => PAGE_RELOADED_FROM_DISK,
                });
                self.toast_shown_at = None;
            }
            Ok(SettingsFileSave::Saved(config)) => {
                let Some(hotkey) = stored_hotkey else {
                    self.replace_toast("Could not save settings: shortcut was not checked");
                    return Task::none();
                };
                if let SaveFrom::Page(commit) = from {
                    return self.after_page_save(commit, config, hotkey, &typed, &config_path);
                }
                if let Err(e) = seasons::save_overlay_hotkey(&self.data_dir, &hotkey) {
                    self.replace_toast(format!("Could not save companion shortcut: {e}"));
                    return Task::none();
                }
                return self.after_save(from, config, hotkey, &typed, &config_path);
            }
            Err(SettingsSaveError::Blocked(message)) => {
                self.replace_toast(message);
                self.page_refused(from, &typed);
            }
            Err(SettingsSaveError::Write(message)) => {
                self.replace_toast(format!("Could not save settings: {message}"));
                self.page_refused(from, &typed);
            }
        }
        Task::none()
    }

    /// A 0.6 page apply was refused or failed, so nothing was written. The
    /// controls go back to what is saved, so the page never shows a value
    /// that is not on disk and a later apply cannot write it unseen. Typed
    /// text stays typed. The classic form keeps everything until Save.
    fn page_refused(&mut self, from: SaveFrom, typed: &SettingsForm) {
        if from == SaveFrom::Classic {
            return;
        }
        self.settings = self.form_from_saved();
        keep_drafts(&mut self.settings, typed);
    }

    /// The form for what is saved, with the stored shortcut.
    fn form_from_saved(&self) -> SettingsForm {
        let mut form = SettingsForm::from_config(&self.saved_config)
            .with_installed_version(&self.saved_config, self.installed_version.as_deref());
        form.overlay_hotkey = self.overlay_hotkey.bind.clone();
        form.overlay_hotkey_enabled = self.overlay_hotkey.enabled;
        form
    }

    /// A 0.6 page apply wrote config.toml. ui_state.json is written only
    /// when the shortcut changed. If that write fails, config.toml is still
    /// taken in (it holds `config` now) and the shortcut error shows alone.
    fn after_page_save(
        &mut self,
        commit: Commit,
        config: Config,
        hotkey: OverlayHotkey,
        typed: &SettingsForm,
        path: &std::path::Path,
    ) -> Task<Message> {
        let from = SaveFrom::Page(commit);
        if commit != Commit::Shortcut && hotkey == self.overlay_hotkey {
            return self.after_save(from, config, hotkey, typed, path);
        }
        match seasons::save_overlay_hotkey(&self.data_dir, &hotkey) {
            Ok(()) => self.after_save(from, config, hotkey, typed, path),
            Err(e) => {
                let kept = self.overlay_hotkey.clone();
                let task = self.after_save(from, config, kept, typed, path);
                copy_draft(&mut self.settings, typed, Draft::Shortcut);
                self.replace_toast(format!("Could not save companion shortcut: {e}"));
                task
            }
        }
    }

    /// The file holds `config` now: take it in, rebuild the form, and run
    /// what the save needs (a restart on the classic page, an update check
    /// after a channel switch).
    fn after_save(
        &mut self,
        from: SaveFrom,
        config: Config,
        hotkey: OverlayHotkey,
        typed: &SettingsForm,
        path: &std::path::Path,
    ) -> Task<Message> {
        let classic = from == SaveFrom::Classic;
        let daemon_up = daemon::is_daemon_running(&self.data_dir);
        let gui_only = only_update_channel_changed(&self.saved_file, &config);
        let route_changed = capture_route_changed(&self.saved_file, &config);
        let restart_for_screen = classic
            && daemon_up
            && !self.daemon_busy
            && capture_output_changed(&self.saved_file, &config);
        let restart_for_route = classic && daemon_up && !self.daemon_busy && route_changed;
        let env_overrides_route = std::env::var(stat_tracker::config::CAPTURE_ENV)
            .ok()
            .is_some_and(|value| !value.trim().is_empty());
        let recheck =
            update_channel_switched(&self.saved_file, &config, self.installed_version.as_deref());
        self.take_saved_file(config, path);
        self.config_unreadable = false;
        self.refresh_health();
        self.overlay_hotkey = hotkey;
        self.settings = self.form_from_saved();
        if let SaveFrom::Page(commit) = from {
            keep_uncommitted(&mut self.settings, typed, commit);
        } else {
            self.replace_toast(if restart_for_route && env_overrides_route {
                SETTINGS_SAVED_ROUTE_ENV
            } else if restart_for_screen {
                SETTINGS_SAVED_RESTARTING
            } else if restart_for_route {
                SETTINGS_SAVED_ROUTE
            } else {
                toast_after_successful_save(daemon_up && !gui_only)
            });
            self.toast_shown_at = None;
        }
        let mut tasks = Vec::new();
        if restart_for_screen || restart_for_route {
            // Same path as the Restart button, so the unit or the spawned
            // tracker is restarted the usual way.
            self.daemon_busy = true;
            self.page_state.restart_started(&self.saved_config);
            tasks.push(Task::perform(
                daemon::run_verb(
                    self.data_dir.clone(),
                    DaemonVerb::Restart,
                    self.daemon.service_installed,
                ),
                Message::DaemonDone,
            ));
        }
        if recheck {
            // `check_for_update` reads the channel from config.toml, which
            // now holds the new value.
            tasks.push(Task::perform(
                update::check_for_update(),
                Message::UpdateChecked,
            ));
            self.last_update_check = Some(std::time::Instant::now());
        }
        Task::batch(tasks)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsToggle;

    #[test]
    fn a_new_capture_screen_restarts_the_tracker_and_other_saves_do_not() {
        let before = Config::default();
        let mut picked = before.clone();
        picked.capture_output = Some("DP-1".into());
        assert!(capture_output_changed(&before, &picked));
        assert!(capture_output_changed(&picked, &before));
        let mut other = picked.clone();
        other.capture_output = Some("eDP-1".into());
        assert!(capture_output_changed(&picked, &other));
        let mut same = picked.clone();
        same.update_channel = "prerelease".into();
        assert!(!capture_output_changed(&picked, &same));
        assert!(!capture_output_changed(&picked, &picked));
    }

    #[test]
    fn save_toasts_have_no_em_or_en_dash() {
        for copy in [
            SETTINGS_SAVED_RESTART,
            SETTINGS_RELOADED_FROM_DISK,
            PAGE_RELOADED_FROM_DISK,
            SETTINGS_SAVED_RESTARTING,
            SETTINGS_SAVED_ROUTE,
            SETTINGS_SAVED_ROUTE_ENV,
            "Settings saved",
        ] {
            assert!(!copy.contains('\u{2014}'), "{copy}");
            assert!(!copy.contains('\u{2013}'), "{copy}");
        }
        assert!(SETTINGS_SAVED_RESTART.contains("Restart the tracker"));
        assert_ne!(toast_after_successful_save(true), SETTINGS_SAVE_REFUSED);
        assert!(!PAGE_RELOADED_FROM_DISK.contains("save"));
    }

    #[test]
    fn alpha_updates_save_rechecks_and_needs_no_tracker_restart() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let raw = "\
# keep this comment
data_dir = \"/tmp/sst-alpha-save\"
player_name = \"FrozenTear\"
";
        std::fs::write(&path, raw).expect("seed");
        let snapshot = Config::parse_file_contents(raw).expect("parse");
        let mut form = SettingsForm::from_config_and_shadow(
            &snapshot,
            Config::shadow_control(snapshot.shadow_recognizer, None),
        );
        form.set_toggle(SettingsToggle::PrereleaseUpdates, true);
        let saved = save_settings_form(
            &path,
            false,
            &snapshot,
            &snapshot,
            EnvOverlay::default(),
            &form,
            None,
            || Ok(()),
        )
        .expect("save writes");
        let SettingsFileSave::Saved(written) = saved else {
            panic!("expected a write, got {saved:?}");
        };
        let on_disk = std::fs::read_to_string(&path).expect("file");
        assert!(on_disk.contains("# keep this comment"), "{on_disk}");
        assert!(
            on_disk.contains("update_channel = \"prerelease\""),
            "{on_disk}"
        );
        assert!(
            Config::parse_file_contents(&on_disk)
                .expect("reparse")
                .wants_prerelease_updates(),
            "the next update check reads prerelease from disk"
        );
        assert!(update_channel_switched(&snapshot, &written, None));
        assert!(only_update_channel_changed(&snapshot, &written));
        assert_eq!(toast_after_successful_save(false), "Settings saved");

        let mut other = written.clone();
        other.debug_ocr = !other.debug_ocr;
        assert!(!only_update_channel_changed(&written, &other));
        assert!(!update_channel_switched(&written, &other, None));
        assert!(!only_update_channel_changed(&written, &written));

        let mut respelled = written.clone();
        respelled.update_channel = "PRERELEASE".into();
        assert!(!update_channel_switched(&written, &respelled, None));

        // On an alpha install an unset channel is already prerelease, so
        // writing `stable` switches and writing `prerelease` does not.
        let alpha = Some("0.5.0-alpha.3");
        let mut stable = snapshot.clone();
        stable.update_channel = "stable".into();
        assert!(update_channel_switched(&snapshot, &stable, alpha));
        assert!(!update_channel_switched(&snapshot, &written, alpha));
    }

    #[test]
    fn a_config_edit_is_written_with_the_form() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let raw = "# keep\ndata_dir = \"/tmp/sst-edit\"\n";
        std::fs::write(&path, raw).expect("seed");
        let snapshot = Config::parse_file_contents(raw).expect("parse");
        let form = SettingsForm::from_config_and_shadow(
            &snapshot,
            Config::shadow_control(snapshot.shadow_recognizer, None),
        );
        let saved = save_settings_form(
            &path,
            false,
            &snapshot,
            &snapshot,
            EnvOverlay::default(),
            &form,
            Some(ConfigEdit::FrameRate(Some(2))),
            || Ok(()),
        )
        .expect("writes");
        let SettingsFileSave::Saved(written) = saved else {
            panic!("expected a write, got {saved:?}");
        };
        assert_eq!(written.screencast_max_fps, Some(2));
        let on_disk = Config::read_at(&path).expect("reread");
        assert_eq!(on_disk.screencast_max_fps, Some(2));
        assert!(
            std::fs::read_to_string(&path)
                .expect("file")
                .contains("# keep")
        );
    }

    /// Every instant change keeps the hand-edited file: its comments, a
    /// key this build does not know, and a token-less [sync] URL.
    #[test]
    fn every_instant_change_edits_the_file_in_place() {
        let raw = "\
# keep
data_dir = \"/tmp/sst-edit\"
from_a_newer_build = 1

[sync]
# token goes here
server_url = \"https://crew.example\"
";
        let mut edits: Vec<(Option<ConfigEdit>, Option<SettingsToggle>)> = vec![
            (Some(ConfigEdit::FrameRate(Some(2))), None),
            (Some(ConfigEdit::PauseBetweenGrabs(false)), None),
            (Some(ConfigEdit::NameReader(false)), None),
            (Some(ConfigEdit::OcrWorkers(Some(2))), None),
        ];
        for toggle in [SettingsToggle::DebugOcr, SettingsToggle::AutoDetect] {
            edits.push((None, Some(toggle)));
        }
        for (edit, toggle) in edits {
            let dir = tempfile::tempdir().expect("tempdir");
            let path = dir.path().join("config.toml");
            std::fs::write(&path, raw).expect("seed");
            let snapshot = Config::parse_file_contents(raw).expect("parse");
            let mut form = SettingsForm::from_config_and_shadow(
                &snapshot,
                Config::shadow_control(snapshot.shadow_recognizer, None),
            );
            if let Some(toggle) = toggle {
                let on = match toggle {
                    SettingsToggle::AutoDetect => !form.auto_detect_enabled,
                    _ => true,
                };
                form.set_toggle(toggle, on);
            }
            let saved = save_settings_form(
                &path,
                false,
                &snapshot,
                &snapshot,
                EnvOverlay::default(),
                &form,
                edit,
                || Ok(()),
            )
            .expect("writes");
            let SettingsFileSave::Saved(written) = saved else {
                panic!("{edit:?} {toggle:?}: expected a write, got {saved:?}");
            };
            assert_ne!(written, snapshot, "{edit:?} {toggle:?} changes something");
            let text = std::fs::read_to_string(&path).expect("file");
            for kept in [
                "# keep\n",
                "from_a_newer_build = 1",
                "# token goes here\nserver_url = \"https://crew.example\"",
            ] {
                assert!(text.contains(kept), "{edit:?} {toggle:?}: {kept:?}\n{text}");
            }
            assert_eq!(Config::read_at(&path).expect("reread"), written);
        }
    }

    #[test]
    fn a_page_apply_leaves_uncommitted_text_out_and_keeps_it_typed() {
        let saved = Config::default();
        let saved_form = SettingsForm::from_config_and_shadow(
            &saved,
            Config::shadow_control(saved.shadow_recognizer, None),
        );
        let mut typed = saved_form.clone();
        typed.debug_ocr = true;
        typed.cooldown_secs = "45".into();
        typed.sync_url = "https://half.typed".into();
        typed.overlay_hotkey = "Super+".into();

        let control = form_for_commit(&typed, &saved_form, Commit::Control);
        assert!(control.debug_ocr, "the clicked switch goes in");
        assert_eq!(control.cooldown_secs, saved_form.cooldown_secs);
        assert_eq!(control.sync_url, saved_form.sync_url);
        assert_eq!(control.overlay_hotkey, saved_form.overlay_hotkey);

        let timings = form_for_commit(&typed, &saved_form, Commit::Timings);
        assert_eq!(timings.cooldown_secs, "45");
        assert_eq!(timings.sync_url, saved_form.sync_url);

        let sync = form_for_commit(&typed, &saved_form, Commit::Sync);
        assert_eq!(sync.sync_url, "https://half.typed");
        assert_eq!(sync.overlay_hotkey, saved_form.overlay_hotkey);

        let shortcut = form_for_commit(&typed, &saved_form, Commit::Shortcut);
        assert_eq!(shortcut.overlay_hotkey, "Super+");

        // After the write the form is rebuilt; the typed drafts come back.
        let mut rebuilt = saved_form.clone();
        keep_uncommitted(&mut rebuilt, &typed, Commit::Timings);
        assert_eq!(rebuilt.sync_url, "https://half.typed");
        assert_eq!(rebuilt.overlay_hotkey, "Super+");
        assert_eq!(rebuilt.cooldown_secs, saved_form.cooldown_secs);
    }

    #[test]
    fn save_reloads_a_fixed_or_hand_edited_config_instead_of_overwriting_it() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let broken = "\
this is not toml
token = \"secret-token-must-stay\"
";
        std::fs::write(&path, broken).expect("seed broken file");

        // Startup: the file did not parse, so the form is defaults.
        let mut snapshot = Config::default();
        let mut form = SettingsForm::from_config_and_shadow(
            &snapshot,
            Config::shadow_control(snapshot.shadow_recognizer, None),
        );
        let mut form_built_unreadable = true;
        assert!(form.player_name.is_empty());
        assert!(form.sync_token.is_empty());

        let mut checked_before_reload = false;
        let refused = save_settings_form(
            &path,
            form_built_unreadable,
            &snapshot,
            &snapshot,
            EnvOverlay::default(),
            &form,
            None,
            || {
                checked_before_reload = true;
                Ok(())
            },
        )
        .expect("refuse is not a write error");
        assert!(!checked_before_reload);
        assert_eq!(refused, SettingsFileSave::Unreadable);
        assert_eq!(std::fs::read_to_string(&path).expect("unchanged"), broken);

        let fixed = "\
# scoreboard name
data_dir = \"/tmp/sst-fixed-on-disk\"
player_name = \"FrozenTear\"
session_window_secs = 1800
finished_game_close_secs = 180
game_process_names = [\"Overwatch.exe\"]
debug_ocr = false

[auto_detect]
enabled = true
poll_interval_secs = 4
cooldown_secs = 120

[sync]
server_url = \"https://crew.example\"
token = \"secret-token-must-stay\"
";
        std::fs::write(&path, fixed).expect("fix the file");

        // (a) First Save reloads the fixed file and does not write the defaults.
        let mut checked_before_reload = false;
        let reloaded = save_settings_form(
            &path,
            form_built_unreadable,
            &snapshot,
            &snapshot,
            EnvOverlay::default(),
            &form,
            None,
            || {
                checked_before_reload = true;
                Err("sync check must not run before a reload".into())
            },
        )
        .expect("reload is not a write error");
        assert!(!checked_before_reload);
        let SettingsFileSave::Reloaded(fresh) = reloaded else {
            panic!("first save after the fix must reload, got {reloaded:?}");
        };
        assert_eq!(std::fs::read_to_string(&path).expect("file"), fixed);
        form = form_after_disk_reload(
            &form,
            &fresh,
            Config::shadow_control(fresh.shadow_recognizer, None),
        );
        snapshot = fresh;
        form_built_unreadable = false;
        assert_eq!(form.player_name, "FrozenTear");
        assert_eq!(form.sync_token, "secret-token-must-stay");
        assert_eq!(form.sync_url, "https://crew.example");
        assert!(!snapshot.shadow_recognizer);

        // Second Save, after the member toggles the reader, writes only that key.
        form.set_toggle(SettingsToggle::ShadowRecognizer, true);
        let saved = save_settings_form(
            &path,
            form_built_unreadable,
            &snapshot,
            &snapshot,
            EnvOverlay::default(),
            &form,
            None,
            || Ok(()),
        )
        .expect("second save writes");
        let SettingsFileSave::Saved(written) = saved else {
            panic!("second save must write the toggle, got {saved:?}");
        };
        assert!(written.shadow_recognizer);
        let text = std::fs::read_to_string(&path).expect("saved");
        let loaded = Config::read_at(&path).expect("load after save");
        assert!(loaded.shadow_recognizer);
        assert_eq!(
            loaded.player_name.as_deref(),
            Some("FrozenTear"),
            "the reloaded name must survive the toggle write"
        );
        assert_eq!(
            loaded.sync.as_ref().map(|sync| sync.token.as_str()),
            Some("secret-token-must-stay")
        );
        let header = text.find("[auto_detect]").expect("tables");
        assert_eq!(
            &text[header..],
            &fixed[fixed.find("[auto_detect]").expect("tables")..]
        );
        assert_eq!(text[..header].matches("shadow_recognizer").count(), 1);
        assert!(text[..header].contains("shadow_recognizer = true"));
        assert!(text.contains("# scoreboard name"));
        assert!(text.contains("player_name = \"FrozenTear\""));

        // (b) A hand edit while the form is open is reloaded, not overwritten.
        let hand_edited = fixed.replace(
            "player_name = \"FrozenTear\"",
            "player_name = \"the other account\"",
        );
        let hand_edited = hand_edited.replace(
            "token = \"secret-token-must-stay\"",
            "token = \"token-edited-on-disk\"",
        );
        std::fs::write(&path, &hand_edited).expect("hand edit");
        form.set_toggle(SettingsToggle::ShadowRecognizer, false);
        let outcome = save_settings_form(
            &path,
            form_built_unreadable,
            &snapshot,
            &snapshot,
            EnvOverlay::default(),
            &form,
            None,
            || Err("must not write over a hand edit".into()),
        )
        .expect("hand edit reloads");
        let SettingsFileSave::Reloaded(fresh) = outcome else {
            panic!("a hand edit must reload, got {outcome:?}");
        };
        assert_eq!(
            std::fs::read_to_string(&path).expect("hand edit kept"),
            hand_edited
        );
        form = form_after_disk_reload(
            &form,
            &fresh,
            Config::shadow_control(fresh.shadow_recognizer, None),
        );
        assert_eq!(form.player_name, "the other account");
        assert_eq!(form.sync_token, "token-edited-on-disk");

        // The startup flag reloads even when the parsed values already match.
        let defaults_path = dir.path().join("defaults.toml");
        let defaults = Config::default();
        let defaults_text = toml::to_string_pretty(&defaults).expect("pretty");
        std::fs::write(&defaults_path, &defaults_text).expect("write defaults");
        let round_trip = Config::read_at(&defaults_path).expect("read defaults");
        assert_eq!(round_trip, defaults);
        let mut dirty = SettingsForm::from_config_and_shadow(
            &defaults,
            Config::shadow_control(defaults.shadow_recognizer, None),
        );
        dirty.set_toggle(SettingsToggle::ShadowRecognizer, true);
        let flagged = save_settings_form(
            &defaults_path,
            true,
            &defaults,
            &defaults,
            EnvOverlay::default(),
            &dirty,
            None,
            || Err("must not write while the form was built unreadable".into()),
        )
        .expect("flagged reload");
        assert!(matches!(flagged, SettingsFileSave::Reloaded(_)));
        assert_eq!(
            std::fs::read_to_string(&defaults_path).expect("defaults"),
            defaults_text
        );
    }

    /// The file at `path` with `[sync]` (https://crew.example, T_file), and
    /// the config shown with `env` on top of it.
    fn file_with_env_sync(path: &std::path::Path, env_url: &str) -> (Config, Config) {
        let raw = "\
data_dir = \"/tmp/sst-env-sync\"

[sync]
server_url = \"https://crew.example\"
token = \"T_file\"
";
        std::fs::write(path, raw).expect("seed");
        let file = Config::read_at(path).expect("parse");
        let mut shown = file.clone();
        shown.sync = Some(stat_tracker::config::SyncConfig {
            server_url: env_url.into(),
            token: "T_env".into(),
        });
        (file, shown)
    }

    /// SCUFFED_TOKEN in the GUI's environment: changing only the Website
    /// URL on the classic page writes the URL with the file's token, never
    /// the outside one.
    #[test]
    fn an_outside_token_never_reaches_the_file() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let (file, shown) = file_with_env_sync(&path, "https://crew.example");
        let env = EnvOverlay {
            token: true,
            ..EnvOverlay::default()
        };
        let mut form = SettingsForm::from_config(&shown);
        assert_eq!(form.sync_token, "T_env");
        form.sync_url = "https://new.example".into();
        let saved = save_settings_form(&path, false, &file, &shown, env, &form, None, || Ok(()))
            .expect("writes");
        assert!(matches!(saved, SettingsFileSave::Saved(_)), "{saved:?}");
        let text = std::fs::read_to_string(&path).expect("file");
        assert!(!text.contains("T_env"), "{text}");
        let on_disk = Config::read_at(&path).expect("reread").sync.expect("sync");
        assert_eq!(on_disk.token, "T_file");
        assert_eq!(on_disk.server_url, "https://new.example");

        // An unrelated change leaves the file's sync as it was.
        let (file, shown) = file_with_env_sync(&path, "https://crew.example");
        let mut form = SettingsForm::from_config(&shown);
        form.debug_ocr = true;
        save_settings_form(&path, false, &file, &shown, env, &form, None, || Ok(()))
            .expect("writes");
        let on_disk = Config::read_at(&path).expect("reread");
        assert!(on_disk.debug_ocr);
        assert_eq!(on_disk.sync, file.sync);
    }

    /// A cleartext SCUFFED_SERVER is never written, so it does not block
    /// other changes; a cleartext URL the player typed still does.
    #[test]
    fn a_cleartext_outside_server_does_not_block_other_changes() {
        let dir = tempfile::tempdir().expect("tempdir");
        let path = dir.path().join("config.toml");
        let (file, shown) = file_with_env_sync(&path, "http://env.example");
        let env = EnvOverlay {
            token: true,
            server: true,
            ..EnvOverlay::default()
        };
        let mut form = SettingsForm::from_config(&shown);
        assert_eq!(form.sync_url, "http://env.example");
        form.debug_ocr = true;
        let saved = save_settings_form(&path, false, &file, &shown, env, &form, None, || Ok(()))
            .expect("not blocked");
        assert!(matches!(saved, SettingsFileSave::Saved(_)), "{saved:?}");
        let on_disk = Config::read_at(&path).expect("reread");
        assert!(on_disk.debug_ocr);
        assert_eq!(on_disk.sync, file.sync);

        let file = on_disk;
        let mut typed = SettingsForm::from_config(&shown);
        typed.sync_url = "http://typed.example".into();
        let before = std::fs::read_to_string(&path).expect("file");
        let blocked = save_settings_form(&path, false, &file, &shown, env, &typed, None, || Ok(()));
        assert!(
            matches!(blocked, Err(SettingsSaveError::Blocked(_))),
            "{blocked:?}"
        );
        assert_eq!(std::fs::read_to_string(&path).expect("file"), before);
    }
}
