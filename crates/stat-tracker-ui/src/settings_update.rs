//! State and messages of the 0.6 Settings page ([`crate::settings_page`]).
//!
//! The page's own state (open ADVANCED rows, the open chip input, and the
//! restart baseline behind "N changes waiting") lives in [`PageState`], and
//! its messages arrive as `Message::SettingsPage(Msg)`, handled by
//! [`update`]. Edits still go through the shared form, so the classic page
//! and this one save the same config.

use iced::Task;
use stat_tracker::config::Config;

use crate::app::{Message, TrackerApp};
use crate::settings::SettingsField;
use crate::settings_model::{AdvancedOpen, AdvancedSection, ChipList, ReaderChoice};

/// A message from the 0.6 Settings page.
#[derive(Debug, Clone)]
pub enum Msg {
    /// Open or close one ADVANCED row.
    ToggleAdvanced(AdvancedSection),
    /// Reading radio. Sets both reader switches.
    PickReader(ReaderChoice),
    /// Open (`Some`), edit, or close (`None`) a chip list's "+ Add" input.
    ChipDraft(ChipList, Option<String>),
    /// Add the open draft as a chip.
    ChipAdd,
    /// Remove chip `index` from a list.
    ChipRemove(ChipList, usize),
}

/// What the 0.6 Settings page keeps between frames.
#[derive(Debug, Clone)]
pub struct PageState {
    /// Open ADVANCED rows. All closed at start.
    advanced_open: AdvancedOpen,
    /// The chip input that is open, and its text.
    chip_draft: Option<(ChipList, String)>,
    /// The config the running tracker started with, as far as the GUI
    /// knows: the saved config at launch, then again after each Restart or
    /// Start it ran. Settings counts the changes waiting against it.
    restart_baseline: Config,
    /// A Restart or Start is running; when it succeeds the baseline moves.
    restart_pending: bool,
}

impl PageState {
    /// Fresh page state for a GUI that just loaded `saved`.
    pub fn new(saved: Config) -> Self {
        Self {
            advanced_open: AdvancedOpen::default(),
            chip_draft: None,
            restart_baseline: saved,
            restart_pending: false,
        }
    }

    pub fn is_open(&self, section: AdvancedSection) -> bool {
        self.advanced_open.is_open(section)
    }

    pub fn chip_draft(&self) -> Option<&(ChipList, String)> {
        self.chip_draft.as_ref()
    }

    /// A daemon verb started. Only Restart and Start move the baseline.
    pub fn set_restart_pending(&mut self, pending: bool) {
        self.restart_pending = pending;
    }

    /// A daemon verb finished. After a Restart or Start that worked, the
    /// tracker runs with what is saved.
    pub fn daemon_done(&mut self, ok: bool, saved: &Config) {
        if std::mem::take(&mut self.restart_pending) && ok {
            self.restart_baseline = saved.clone();
        }
    }
}

impl TrackerApp {
    /// Settings that differ from what the running tracker started with,
    /// counting edits not saved yet. Shown as "N changes waiting" next to
    /// Restart tracker.
    pub fn changes_waiting(&self) -> usize {
        crate::settings_model::changes_waiting(
            &self.page_state.restart_baseline,
            &self.settings.to_config(&self.saved_config),
        )
    }
}

/// Handle one page message.
pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    match msg {
        Msg::ToggleAdvanced(section) => app.page_state.advanced_open.toggle(section),
        Msg::ChipDraft(list, draft) => {
            app.page_state.chip_draft = draft.map(|text| (list, text));
        }
        Msg::ChipAdd => {
            if let Some((list, draft)) = app.page_state.chip_draft.take() {
                use crate::settings_model::with_chip;
                match list {
                    // Config holds one name, so a new one replaces it.
                    ChipList::Names => {
                        let name = draft.trim();
                        if !name.is_empty() {
                            app.settings
                                .set_text(SettingsField::PlayerName, name.to_string());
                        }
                    }
                    ChipList::Processes => {
                        let next = with_chip(&app.settings.game_process_names, &draft);
                        app.settings.set_text(SettingsField::ProcessNames, next);
                    }
                }
            }
        }
        Msg::ChipRemove(list, index) => {
            use crate::settings_model::without_chip;
            match list {
                ChipList::Names => app
                    .settings
                    .set_text(SettingsField::PlayerName, String::new()),
                ChipList::Processes => {
                    let next = without_chip(&app.settings.game_process_names, index);
                    app.settings.set_text(SettingsField::ProcessNames, next);
                }
            }
        }
        Msg::PickReader(choice) => {
            app.settings.new_reader = choice.new_reader();
            if choice.classic_ocr() != app.settings.classic_ocr {
                return app.toggle_classic_ocr(choice.classic_ocr());
            }
            // Same OCR switch: no pack check, and a refusal from an
            // earlier click no longer applies.
            if choice.classic_ocr() {
                app.ocr_switch.refusal = None;
            }
        }
    }
    Task::none()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn app() -> (tempfile::TempDir, TrackerApp) {
        let dir = tempfile::tempdir().unwrap();
        let cli = crate::cli::Cli {
            data_dir: dir.path().to_path_buf(),
            fixture: Some(crate::cli::FixtureKind::Empty),
            seasons_url: None,
            companion: false,
            help: false,
            version: false,
            replace: false,
        };
        let app = TrackerApp::new(cli).0;
        (dir, app)
    }

    fn page(msg: Msg) -> Message {
        Message::SettingsPage(msg)
    }

    #[test]
    fn reader_pick_sets_both_switches() {
        let (_dir, mut app) = app();
        for choice in ReaderChoice::ALL {
            let _ = app.update(page(Msg::PickReader(choice)));
            assert_eq!(
                ReaderChoice::from_switches(app.settings.new_reader, app.settings.classic_ocr),
                choice
            );
        }
    }

    #[test]
    fn advanced_rows_fold_through_update() {
        let (_dir, mut app) = app();
        assert!(!app.page_state.advanced_open.any_open());
        let _ = app.update(page(Msg::ToggleAdvanced(AdvancedSection::StoredData)));
        assert!(app.page_state.is_open(AdvancedSection::StoredData));
        let _ = app.update(page(Msg::ToggleAdvanced(AdvancedSection::StoredData)));
        assert!(!app.page_state.advanced_open.any_open());
    }

    #[test]
    fn the_baseline_starts_at_the_saved_config() {
        let (_dir, app) = app();
        assert_eq!(app.page_state.restart_baseline, app.saved_config);
        assert!(!app.page_state.restart_pending);
    }

    #[test]
    fn edits_count_as_changes_waiting_until_a_restart() {
        let (_dir, mut app) = app();
        assert_eq!(
            app.changes_waiting(),
            0,
            "an untouched form waits for nothing"
        );
        let _ = app.update(Message::SettingsText(
            SettingsField::SessionWindow,
            "10800".into(),
        ));
        assert_eq!(app.changes_waiting(), 1);
        let _ = app.update(page(Msg::ChipDraft(
            ChipList::Processes,
            Some("ow2.exe".into()),
        )));
        let _ = app.update(page(Msg::ChipAdd));
        assert!(app.settings.game_process_names.contains("ow2.exe"));
        assert!(app.page_state.chip_draft().is_none());
        assert_eq!(app.changes_waiting(), 2);
        let _ = app.update(page(Msg::ChipRemove(ChipList::Names, 0)));
        assert!(app.settings.player_name.is_empty());
        let _ = crate::settings_page::view(&app, 640.0);
        let _ = crate::settings_page::view(&app, 1600.0);
    }

    #[test]
    fn a_restart_clears_the_changes_waiting() {
        let (_dir, mut app) = app();
        app.settings.session_window_secs = "10800".into();
        app.saved_config = app.settings.to_config(&app.saved_config);
        assert_eq!(app.changes_waiting(), 1);
        // A failed restart keeps the change waiting.
        app.page_state.restart_pending = true;
        let _ = app.update(Message::DaemonDone(Err("no".into())));
        assert_eq!(app.changes_waiting(), 1);
        app.page_state.restart_pending = true;
        let _ = app.update(Message::DaemonDone(Ok("restarted".into())));
        assert_eq!(app.changes_waiting(), 0);
        assert!(!app.page_state.restart_pending);
    }
}
