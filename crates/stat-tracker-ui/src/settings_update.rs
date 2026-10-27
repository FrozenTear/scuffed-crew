//! State and messages of the 0.6 Settings page ([`crate::settings_page`]).
//!
//! The page has no Save: each change is written to config.toml as it is
//! made, through the same save path the classic page uses
//! ([`crate::settings_save`]). Switches, chips, dropdowns and radios apply
//! at once; typed text applies on Enter, and the Website connection fields
//! on their own Apply. A success is quiet, a refusal shows a toast.
//!
//! The page's own state (open ADVANCED rows, the open chip input, the
//! reader pick waiting for its pack check, and the restart baseline behind
//! "N changes waiting") lives in [`PageState`]. Its messages arrive as
//! `Message::SettingsPage(Msg)`, handled by [`update`]. Demo mode changes
//! nothing: every message that would is refused here.

use std::cell::RefCell;
use std::path::PathBuf;

use iced::Task;
use stat_tracker::config::Config;
use stat_tracker::ocr_switch::PackReadiness;

use crate::app::{Message, TrackerApp};
use crate::settings::{self, SettingsField};
use crate::settings_model::{
    AdvancedOpen, AdvancedSection, ChipList, ConfigEdit, ReaderChoice, name_chip, with_chip,
    without_chip,
};
use crate::settings_save::{Commit, SaveFrom};
use crate::update::UpdateInfo;

/// Toast for a control used in demo mode.
pub(crate) const DEMO_REFUSED: &str = "Demo mode: nothing is changed.";

/// A message from the 0.6 Settings page.
#[derive(Debug, Clone)]
pub enum Msg {
    /// Open or close one ADVANCED row.
    ToggleAdvanced(AdvancedSection),
    /// Run a form edit (a switch, dropdown or chip), then write the form.
    Apply(Box<Message>, Commit),
    /// Write typed text: Enter in a field, or Apply under Website connection.
    Commit(Commit),
    /// A setting with no form field, written at once.
    Edit(ConfigEdit),
    /// Reader radio list.
    PickReader(ReaderChoice),
    /// The pack check a reader pick that turns classic OCR off waits for.
    ReaderChecked {
        seq: u64,
        choice: ReaderChoice,
        packs: PackReadiness,
    },
    /// Open (`Some`), edit, or close (`None`) a chip list's "+ Add" input.
    ChipDraft(ChipList, Option<String>),
    /// Add the open draft as a chip.
    ChipAdd,
    /// Remove chip `index` from a list.
    ChipRemove(ChipList, usize),
    /// "Show Companion during games".
    CompanionDuringGames(bool),
    /// Check now, under Updates.
    CheckUpdates,
    UpdatesChecked(Option<UpdateInfo>),
}

impl Msg {
    /// True for a message that changes a setting or starts something.
    fn acts(&self) -> bool {
        !matches!(
            self,
            Msg::ToggleAdvanced(_) | Msg::ReaderChecked { .. } | Msg::UpdatesChecked(_)
        )
    }
}

/// A control's edit, applied at once.
pub(crate) fn apply(msg: Message) -> Message {
    Message::SettingsPage(Msg::Apply(Box::new(msg), Commit::Control))
}

/// A control's edit that commits a group of typed text with it.
pub(crate) fn apply_with(msg: Message, commit: Commit) -> Message {
    Message::SettingsPage(Msg::Apply(Box::new(msg), commit))
}

/// What the 0.6 Settings page keeps between frames.
#[derive(Debug, Clone)]
pub struct PageState {
    /// Open ADVANCED rows. All closed at start.
    advanced_open: AdvancedOpen,
    /// The chip input that is open, and its text.
    chip_draft: Option<(ChipList, String)>,
    /// The config the running tracker started with, as far as the GUI
    /// knows: the saved config at launch, then again whenever a tracker
    /// starts. Settings counts the changes waiting against it.
    restart_baseline: Config,
    /// The config saved when a Restart or Start began. It becomes the
    /// baseline when that run succeeds, so a change made while it ran
    /// still counts as waiting.
    restart_pending: Option<Config>,
    /// The tracker's pid at the last look. A new pid with no restart of our
    /// own running is a restart from elsewhere (an update, the login
    /// service, a crash restart, systemctl): it loaded what is saved.
    seen_pid: Option<u32>,
    /// A reader pick waiting for its pack check, and the counter that
    /// tells a stale check from the latest one.
    reader_pending: Option<(u64, ReaderChoice)>,
    reader_seq: u64,
    /// "Show Companion during games", stored in ui_state.json. Off keeps
    /// the Companion closed ([`TrackerApp::overlay_showing`]).
    pub companion_during_games: bool,
    /// When Check now last answered (local "HH:MM"), and whether it runs.
    pub update_checked_at: Option<String>,
    pub update_checking: bool,
    /// Where page applies write instead of the user's config.toml (tests).
    pub(crate) config_path: Option<PathBuf>,
    /// Whether the website refused the saved token, as last read
    /// ([`crate::tracker_status::sync_token_rejected`]).
    pub(crate) token_rejected: bool,
    /// The last changes waiting count with the baseline and saved config it
    /// was counted from, so a frame recounts only after one of them moved.
    waiting_memo: RefCell<Option<(Config, Config, usize)>>,
}

impl PageState {
    /// Fresh page state for a GUI that just loaded `saved`, with the
    /// tracker at `pid`.
    pub fn new(saved: Config, pid: Option<u32>, companion_during_games: bool) -> Self {
        Self {
            advanced_open: AdvancedOpen::default(),
            chip_draft: None,
            restart_baseline: saved,
            restart_pending: None,
            seen_pid: pid,
            reader_pending: None,
            reader_seq: 0,
            companion_during_games,
            update_checked_at: None,
            update_checking: false,
            config_path: None,
            token_rejected: false,
            waiting_memo: RefCell::new(None),
        }
    }

    pub fn is_open(&self, section: AdvancedSection) -> bool {
        self.advanced_open.is_open(section)
    }

    pub fn chip_draft(&self) -> Option<&(ChipList, String)> {
        self.chip_draft.as_ref()
    }

    /// The reader the radio list waits to set, while its pack check runs.
    pub fn reader_pending(&self) -> Option<ReaderChoice> {
        self.reader_pending.map(|(_, choice)| choice)
    }

    /// A Restart or Start began with `saved` on disk.
    pub fn restart_started(&mut self, saved: &Config) {
        self.restart_pending = Some(saved.clone());
    }

    /// A daemon verb finished. After a Restart or Start that worked, the
    /// tracker runs with what was saved when it began.
    pub fn daemon_done(&mut self, ok: bool) {
        if let Some(started) = self.restart_pending.take()
            && ok
        {
            self.restart_baseline = started;
        }
    }

    /// The tracker's pid now. A tracker that started without a restart of
    /// ours running loaded the saved config. Called after every refresh.
    pub fn observe_tracker(&mut self, pid: Option<u32>, saved: &Config) {
        if self.restart_pending.is_some() || pid == self.seen_pid {
            return;
        }
        if pid.is_some() {
            self.restart_baseline = saved.clone();
        }
        self.seen_pid = pid;
    }

    /// The player left Settings: an open chip input closes.
    pub fn left_page(&mut self) {
        self.chip_draft = None;
    }
}

impl TrackerApp {
    /// Saved settings that differ from what the running tracker started
    /// with. Shown as "N changes waiting" next to Restart tracker.
    pub fn changes_waiting(&self) -> usize {
        let baseline = &self.page_state.restart_baseline;
        let mut memo = self.page_state.waiting_memo.borrow_mut();
        if let Some((b, s, n)) = memo.as_ref()
            && b == baseline
            && s == &self.saved_config
        {
            return *n;
        }
        let n = crate::settings_model::changes_waiting(baseline, &self.saved_config);
        *memo = Some((baseline.clone(), self.saved_config.clone(), n));
        n
    }

    /// Where the Settings pages write config.toml.
    pub(crate) fn settings_config_path(&self) -> Result<PathBuf, ()> {
        match &self.page_state.config_path {
            Some(path) => Ok(path.clone()),
            None => Config::config_path().map_err(|_| ()),
        }
    }
}

/// Escape closes an open chip input without adding it. The app listens only
/// while one is open; the input itself takes Escape too, so the status is
/// not looked at.
pub(crate) fn chip_escape(
    event: iced::Event,
    _status: iced::event::Status,
    _window: iced::window::Id,
) -> Option<Message> {
    let iced::Event::Keyboard(iced::keyboard::Event::KeyPressed { key, .. }) = event else {
        return None;
    };
    escape_closes_chip(&key)
}

/// `ChipDraft(_, None)` closes whichever input is open.
fn escape_closes_chip(key: &iced::keyboard::Key) -> Option<Message> {
    use iced::keyboard::{Key, key::Named};
    matches!(key, Key::Named(Named::Escape))
        .then_some(Message::SettingsPage(Msg::ChipDraft(ChipList::Names, None)))
}

/// Id of a chip list's "+ Add" input, so opening it can focus it.
pub(crate) fn chip_input_id(list: ChipList) -> iced::widget::Id {
    match list {
        ChipList::Names => iced::widget::Id::new("settings-chip-names"),
        ChipList::Processes => iced::widget::Id::new("settings-chip-processes"),
    }
}

fn page_save(app: &mut TrackerApp, commit: Commit, edit: Option<ConfigEdit>) -> Task<Message> {
    app.save_settings(SaveFrom::Page(commit), edit)
}

/// Handle one page message.
pub fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    if app.fixture.is_some() && msg.acts() {
        app.replace_toast(DEMO_REFUSED);
        return Task::none();
    }
    match msg {
        Msg::ToggleAdvanced(section) => app.page_state.advanced_open.toggle(section),
        Msg::Apply(inner, commit) => {
            let first = app.update(*inner);
            let save = page_save(app, commit, None);
            return Task::batch([first, save]);
        }
        Msg::Commit(commit) => return page_save(app, commit, None),
        Msg::Edit(edit) => return page_save(app, Commit::Control, Some(edit)),
        Msg::PickReader(choice) => return pick_reader(app, choice),
        Msg::ReaderChecked { seq, choice, packs } => {
            app.ocr_switch.packs = Some(packs);
            if app.page_state.reader_pending != Some((seq, choice)) {
                // A later pick (or one that needed no check) won.
                return Task::none();
            }
            app.page_state.reader_pending = None;
            let needing = stat_tracker::ocr_switch::Read::needing_ocr();
            match settings::classic_ocr_toggle(false, &packs, &needing) {
                Ok(_) => return set_reader(app, choice),
                // The old choice stays, with the reason next to it.
                Err(reason) => app.ocr_switch.refusal = Some(reason),
            }
        }
        Msg::ChipDraft(list, draft) => {
            let opening = draft.is_some()
                && app
                    .page_state
                    .chip_draft
                    .as_ref()
                    .is_none_or(|(open, _)| *open != list);
            app.page_state.chip_draft = draft.map(|text| (list, text));
            if opening {
                return iced::widget::operation::focus(chip_input_id(list));
            }
        }
        Msg::ChipAdd => {
            let Some((list, draft)) = app.page_state.chip_draft.take() else {
                return Task::none();
            };
            let changed = match list {
                // Config holds one name, so a new one replaces it.
                ChipList::Names => match name_chip(&draft) {
                    Some(name) => {
                        app.settings.set_text(SettingsField::PlayerName, name);
                        true
                    }
                    None => false,
                },
                ChipList::Processes => {
                    let next = with_chip(&app.settings.game_process_names, &draft);
                    let changed = next != app.settings.game_process_names;
                    app.settings.set_text(SettingsField::ProcessNames, next);
                    changed
                }
            };
            if changed {
                return page_save(app, Commit::Control, None);
            }
        }
        Msg::ChipRemove(list, index) => {
            match list {
                ChipList::Names => app
                    .settings
                    .set_text(SettingsField::PlayerName, String::new()),
                ChipList::Processes => {
                    let next = without_chip(&app.settings.game_process_names, index);
                    app.settings.set_text(SettingsField::ProcessNames, next);
                }
            }
            return page_save(app, Commit::Control, None);
        }
        Msg::CompanionDuringGames(on) => {
            app.page_state.companion_during_games = on;
            if let Err(e) = crate::seasons::save_companion_during_games(&app.data_dir, on) {
                app.replace_toast(format!("Could not save the Companion setting: {e}"));
            }
        }
        Msg::CheckUpdates => {
            if app.page_state.update_checking {
                return Task::none();
            }
            app.page_state.update_checking = true;
            app.last_update_check = Some(std::time::Instant::now());
            return Task::perform(crate::update::check_for_update(), |info| {
                Message::SettingsPage(Msg::UpdatesChecked(info))
            });
        }
        Msg::UpdatesChecked(info) => {
            app.page_state.update_checking = false;
            app.page_state.update_checked_at =
                Some(chrono::Local::now().format("%H:%M").to_string());
            return app.update(Message::UpdateChecked(info));
        }
    }
    Task::none()
}

/// A pick on the Reader radio list. A pick that keeps classic OCR (or
/// turns it on) applies at once. One that turns it off waits for a look at
/// the reader packs; only the latest pick counts when its look comes back.
fn pick_reader(app: &mut TrackerApp, choice: ReaderChoice) -> Task<Message> {
    let current = ReaderChoice::from_switches(app.settings.new_reader, app.settings.classic_ocr);
    app.page_state.reader_seq += 1;
    if choice == current {
        app.page_state.reader_pending = None;
        return Task::none();
    }
    if choice.classic_ocr() || !current.classic_ocr() {
        app.page_state.reader_pending = None;
        return set_reader(app, choice);
    }
    let seq = app.page_state.reader_seq;
    app.page_state.reader_pending = Some((seq, choice));
    let data_dir = app.data_dir.clone();
    Task::perform(
        async move { stat_tracker::ocr_switch::pack_readiness_on_disk(&data_dir) },
        move |packs| Message::SettingsPage(Msg::ReaderChecked { seq, choice, packs }),
    )
}

/// Set both reader switches to `choice` and write them.
fn set_reader(app: &mut TrackerApp, choice: ReaderChoice) -> Task<Message> {
    app.settings.new_reader = choice.new_reader();
    app.settings.classic_ocr = choice.classic_ocr();
    app.ocr_switch.refusal = None;
    page_save(app, Commit::Control, None)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings::SettingsToggle;

    fn demo_app() -> (tempfile::TempDir, TrackerApp) {
        crate::fixtures::test_app()
    }

    /// A live (not demo) app whose page writes a config.toml in a temp dir.
    fn live_app() -> (tempfile::TempDir, TrackerApp) {
        let (dir, mut app) = demo_app();
        app.fixture = None;
        let path = dir.path().join("config.toml");
        app.saved_config.save_at(&path).unwrap();
        app.saved_config = Config::read_at(&path).unwrap();
        app.settings = crate::settings::SettingsForm::from_config(&app.saved_config);
        app.settings.overlay_hotkey = app.overlay_hotkey.bind.clone();
        app.settings.overlay_hotkey_enabled = app.overlay_hotkey.enabled;
        app.page_state = PageState::new(app.saved_config.clone(), None, true);
        app.page_state.config_path = Some(path);
        app.toast = None;
        (dir, app)
    }

    fn on_disk(dir: &tempfile::TempDir) -> Config {
        Config::read_at(&dir.path().join("config.toml")).unwrap()
    }

    fn page(msg: Msg) -> Message {
        Message::SettingsPage(msg)
    }

    #[test]
    fn advanced_rows_fold_through_update() {
        let (_dir, mut app) = demo_app();
        assert!(!app.page_state.advanced_open.any_open());
        let _ = app.update(page(Msg::ToggleAdvanced(AdvancedSection::StoredData)));
        assert!(app.page_state.is_open(AdvancedSection::StoredData));
        let _ = app.update(page(Msg::ToggleAdvanced(AdvancedSection::StoredData)));
        assert!(!app.page_state.advanced_open.any_open());
    }

    #[test]
    fn demo_mode_changes_nothing() {
        let (_dir, mut app) = demo_app();
        let form = app.settings.clone();
        let saved = app.saved_config.clone();
        for msg in [
            Msg::Apply(
                Box::new(Message::SettingsToggle(SettingsToggle::DebugOcr, true)),
                Commit::Control,
            ),
            Msg::Edit(ConfigEdit::FrameRate(Some(2))),
            Msg::PickReader(ReaderChoice::ClassicOnly),
            Msg::ChipDraft(ChipList::Processes, Some("x".into())),
            Msg::ChipRemove(ChipList::Processes, 0),
            Msg::Commit(Commit::Sync),
            Msg::CompanionDuringGames(false),
            Msg::CheckUpdates,
        ] {
            let _ = app.update(page(msg));
        }
        assert_eq!(app.settings, form);
        assert_eq!(app.saved_config, saved);
        assert!(app.page_state.companion_during_games);
        assert!(app.page_state.chip_draft().is_none());
        assert_eq!(app.toast.as_deref(), Some(DEMO_REFUSED));
    }

    #[test]
    fn demo_mode_never_opens_or_runs_uninstall() {
        let (_dir, mut app) = demo_app();
        let _ = app.update(Message::OpenUninstall);
        assert!(app.uninstall_dialog.is_none());
        let _ = app.update(Message::ConfirmUninstall);
        assert!(app.uninstall_dialog.is_none());
    }

    #[test]
    fn a_switch_applies_at_once_and_quietly() {
        let (dir, mut app) = live_app();
        let _ = app.update(apply(Message::SettingsToggle(
            SettingsToggle::DebugOcr,
            true,
        )));
        assert!(app.saved_config.debug_ocr);
        assert!(on_disk(&dir).debug_ocr);
        assert!(app.toast.is_none(), "{:?}", app.toast);
    }

    #[test]
    fn typed_text_waits_for_its_own_commit() {
        let (dir, mut app) = live_app();
        let before = app.saved_config.auto_detect.cooldown_secs;
        let _ = app.update(Message::SettingsText(SettingsField::Cooldown, "45".into()));
        let _ = app.update(Message::SettingsText(
            SettingsField::SyncUrl,
            "https://typed.example".into(),
        ));
        // A switch elsewhere writes neither field, and both stay typed.
        let _ = app.update(apply(Message::SettingsToggle(
            SettingsToggle::DebugOcr,
            true,
        )));
        assert_eq!(on_disk(&dir).auto_detect.cooldown_secs, before);
        assert_eq!(app.settings.cooldown_secs, "45");
        assert_eq!(app.settings.sync_url, "https://typed.example");
        // Enter commits the timings only.
        let _ = app.update(page(Msg::Commit(Commit::Timings)));
        assert_eq!(on_disk(&dir).auto_detect.cooldown_secs, 45);
        assert!(on_disk(&dir).sync.is_none());
        assert_eq!(app.settings.sync_url, "https://typed.example");
    }

    #[test]
    fn a_config_edit_is_written_and_counted() {
        let (dir, mut app) = live_app();
        assert_eq!(app.changes_waiting(), 0);
        let _ = app.update(page(Msg::Edit(ConfigEdit::FrameRate(Some(2)))));
        assert_eq!(on_disk(&dir).screencast_max_fps, Some(2));
        assert_eq!(app.changes_waiting(), 1);
        let _ = app.update(page(Msg::Edit(ConfigEdit::FrameRate(None))));
        assert_eq!(on_disk(&dir).screencast_max_fps, None);
        assert_eq!(app.changes_waiting(), 0);
    }

    #[test]
    fn edits_wait_for_a_restart_but_app_only_ones_do_not() {
        let (_dir, mut app) = live_app();
        let _ = app.update(apply(Message::SettingsText(
            SettingsField::SessionWindow,
            "10800".into(),
        )));
        assert_eq!(app.saved_config.session_window_secs, 10_800);
        assert_eq!(app.changes_waiting(), 0, "only this app reads it");
        let _ = app.update(page(Msg::ChipDraft(
            ChipList::Processes,
            Some("ow2.exe".into()),
        )));
        let _ = app.update(page(Msg::ChipAdd));
        assert!(
            app.saved_config
                .game_process_names
                .contains(&"ow2.exe".into())
        );
        assert!(app.page_state.chip_draft().is_none());
        assert_eq!(app.changes_waiting(), 1);
        // A restart of ours that worked clears it.
        app.page_state.restart_started(&app.saved_config);
        app.page_state.daemon_done(true);
        assert_eq!(app.changes_waiting(), 0);
    }

    /// Through the real messages, in a live (not demo) app: the verb's task
    /// is returned, not run, so nothing touches the host's tracker.
    #[test]
    fn a_restart_that_works_clears_the_changes_waiting() {
        use crate::daemon::DaemonVerb;
        let (_dir, mut app) = live_app();
        let _ = app.update(page(Msg::Edit(ConfigEdit::FrameRate(Some(2)))));
        assert_eq!(app.changes_waiting(), 1);
        let _ = app.update(Message::Daemon(DaemonVerb::Stop));
        let _ = app.update(Message::DaemonDone(Ok("Tracker stopped".into())));
        assert_eq!(app.changes_waiting(), 1, "a stop loads nothing");
        let _ = app.update(Message::Daemon(DaemonVerb::Restart));
        assert!(app.daemon_busy);
        let _ = app.update(Message::DaemonDone(Err("failed".into())));
        assert_eq!(app.changes_waiting(), 1, "a failed restart loads nothing");
        let _ = app.update(Message::Daemon(DaemonVerb::Restart));
        let _ = app.update(Message::DaemonDone(Ok("Tracker restarted".into())));
        assert!(!app.daemon_busy);
        assert_eq!(app.changes_waiting(), 0);
    }

    #[test]
    fn a_change_during_a_restart_still_waits() {
        let (_dir, mut app) = live_app();
        let started = app.saved_config.clone();
        app.page_state.restart_started(&started);
        let mut later = started.clone();
        later.auto_detect.cooldown_secs += 5;
        app.saved_config = later;
        // The tick sees the new pid while our restart runs: no move yet.
        app.page_state.observe_tracker(Some(42), &app.saved_config);
        app.page_state.daemon_done(true);
        assert_eq!(app.changes_waiting(), 1);
        // A failed restart moves nothing.
        app.page_state.restart_started(&app.saved_config);
        app.page_state.daemon_done(false);
        assert_eq!(app.changes_waiting(), 1);
    }

    #[test]
    fn a_restart_from_elsewhere_moves_the_baseline() {
        let (_dir, mut app) = live_app();
        let mut later = app.saved_config.clone();
        later.debug_ocr = !later.debug_ocr;
        app.saved_config = later;
        assert_eq!(app.changes_waiting(), 1);
        app.page_state.observe_tracker(None, &app.saved_config);
        assert_eq!(app.changes_waiting(), 1, "no tracker, nothing loaded");
        app.page_state.observe_tracker(Some(7), &app.saved_config);
        assert_eq!(app.changes_waiting(), 0);
    }

    #[test]
    fn a_reader_pick_that_turns_ocr_off_waits_for_the_packs() {
        let (dir, mut app) = live_app();
        let start = ReaderChoice::of(&app.saved_config);
        let _ = app.update(page(Msg::PickReader(ReaderChoice::NewWithOcrBackup)));
        let _ = app.update(page(Msg::PickReader(ReaderChoice::NewOnly)));
        assert_eq!(app.page_state.reader_pending(), Some(ReaderChoice::NewOnly));
        let backup = ReaderChoice::NewWithOcrBackup;
        let now = ReaderChoice::from_switches(app.settings.new_reader, app.settings.classic_ocr);
        assert!(now == backup || now == start);
        let seq = app.page_state.reader_seq;
        // A stale look is dropped.
        let _ = app.update(page(Msg::ReaderChecked {
            seq: seq - 1,
            choice: ReaderChoice::NewOnly,
            packs: PackReadiness::default(),
        }));
        assert_eq!(app.page_state.reader_pending(), Some(ReaderChoice::NewOnly));
        // No packs: refused, the old choice stays, nothing is written.
        let before = on_disk(&dir);
        let _ = app.update(page(Msg::ReaderChecked {
            seq,
            choice: ReaderChoice::NewOnly,
            packs: PackReadiness::default(),
        }));
        assert!(app.page_state.reader_pending().is_none());
        assert!(app.settings.classic_ocr);
        assert!(app.ocr_switch.refusal.is_some());
        assert_eq!(on_disk(&dir).ocr, before.ocr);
    }

    #[test]
    fn a_reader_pick_that_keeps_ocr_applies_at_once() {
        let (dir, mut app) = live_app();
        let _ = app.update(page(Msg::PickReader(ReaderChoice::ClassicOnly)));
        assert!(app.page_state.reader_pending().is_none());
        assert_eq!(ReaderChoice::of(&on_disk(&dir)), ReaderChoice::ClassicOnly);
        let _ = app.update(page(Msg::PickReader(ReaderChoice::NewWithOcrBackup)));
        assert_eq!(
            ReaderChoice::of(&on_disk(&dir)),
            ReaderChoice::NewWithOcrBackup
        );
    }

    #[test]
    fn a_name_chip_drops_the_tag_and_leaving_closes_the_input() {
        let (dir, mut app) = live_app();
        let _ = app.update(page(Msg::ChipDraft(
            ChipList::Names,
            Some("FrozenTear#2112".into()),
        )));
        let _ = app.update(page(Msg::ChipAdd));
        assert_eq!(on_disk(&dir).player_name.as_deref(), Some("FrozenTear"));
        let _ = app.update(page(Msg::ChipDraft(ChipList::Processes, Some("a".into()))));
        let _ = app.update(Message::Navigate(crate::model::Screen::Overview));
        assert!(app.page_state.chip_draft().is_none());
    }

    #[test]
    fn a_chip_is_removed_from_either_list() {
        let (dir, mut app) = live_app();
        let _ = app.update(page(Msg::ChipDraft(
            ChipList::Names,
            Some("FrozenTear".into()),
        )));
        let _ = app.update(page(Msg::ChipAdd));
        assert_eq!(on_disk(&dir).player_name.as_deref(), Some("FrozenTear"));
        let _ = app.update(page(Msg::ChipRemove(ChipList::Names, 0)));
        assert_eq!(on_disk(&dir).player_name, None);

        let _ = app.update(page(Msg::ChipDraft(
            ChipList::Processes,
            Some("a.exe, b.exe".into()),
        )));
        let _ = app.update(page(Msg::ChipAdd));
        let names = on_disk(&dir).game_process_names;
        assert!(names.contains(&"a.exe".into()) && names.contains(&"b.exe".into()));
        let index = names.iter().position(|n| n == "a.exe").unwrap();
        let _ = app.update(page(Msg::ChipRemove(ChipList::Processes, index)));
        let names = on_disk(&dir).game_process_names;
        assert!(!names.contains(&"a.exe".into()));
        assert!(names.contains(&"b.exe".into()));
    }

    #[test]
    fn cancel_and_escape_close_the_input_and_add_nothing() {
        use iced::keyboard::{Key, key::Named};
        let (dir, mut app) = live_app();
        let before = on_disk(&dir).game_process_names;
        let _ = app.update(page(Msg::ChipDraft(
            ChipList::Processes,
            Some("typed.exe".into()),
        )));
        let _ = app.update(page(Msg::ChipDraft(ChipList::Processes, None)));
        assert!(app.page_state.chip_draft().is_none());
        assert_eq!(on_disk(&dir).game_process_names, before);

        assert!(escape_closes_chip(&Key::Named(Named::Enter)).is_none());
        let _ = app.update(page(Msg::ChipDraft(ChipList::Names, Some("x".into()))));
        let escape = escape_closes_chip(&Key::Named(Named::Escape)).expect("Escape closes");
        let _ = app.update(escape);
        assert!(app.page_state.chip_draft().is_none());
        assert_eq!(on_disk(&dir).player_name, None);
    }

    #[test]
    fn the_companion_switch_is_stored_and_keeps_it_closed() {
        let (dir, mut app) = live_app();
        app.game_running = true;
        let _ = app.update(page(Msg::CompanionDuringGames(false)));
        assert!(!crate::seasons::load_companion_during_games(dir.path()));
        assert!(!app.overlay_showing());
        let _ = app.update(page(Msg::CompanionDuringGames(true)));
        assert!(crate::seasons::load_companion_during_games(dir.path()));
    }
}
