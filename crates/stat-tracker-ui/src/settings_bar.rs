//! The pinned Settings bar (UX spec 2, #776): a bar across the bottom of
//! the 0.6 Settings tab while typed text is not saved yet, or saved
//! changes wait for a tracker restart.
//!
//! Switches, chips, dropdowns and the session window still apply at once
//! and never count here (Q7 = A): only the three typed groups do
//! ([`crate::settings_save::dirty_drafts`]: timings, website, shortcut),
//! plus [`TrackerApp::changes_waiting`]. Discard puts the typed text back;
//! Save writes it through the same path as Enter and Apply
//! ([`Commit::Drafts`]); Save and restart, Save and start, Restart tracker
//! and Start tracker then run the verb through the status pill, so the
//! mid-game confirm and the busy rules are the pill's. The bar never
//! restarts on its own.
//!
//! Leaving Settings with typed text open (another tab, Run setup again,
//! Uninstall, the app's own restart, Quit, or closing a window with no
//! tray) asks first ([`intercept`]). So does Stop on the pill (spec 1.5).
//!
//! The bar is a slot under the page that is always in the widget tree
//! ([`slot`]), zero height while hidden, so the page pane keeps its place
//! in the tree and its scroll and focus when the bar comes and goes (the
//! #806 rule [`crate::chrome`] keeps for the toast). The page's bottom
//! inset stays fixed; the visible pane is shorter while the bar shows.
//!
//! The points the spec left open (D1 to D10, accepted by FrozenTear on
//! 27 Nov 2026, recorded on #881) each sit in one place here: [`Pending::primary`] (D5, D6),
//! [`Pending::plain_save`] (D6), [`words`] (D4), [`intercept`] (D8),
//! [`before_verb`] (D7), [`BarState::focus`] with [`focus_key`] (D3).

use iced::widget::{
    Column, Row, button, column, container, mouse_area, opaque, space, stack, text,
};
use iced::{Alignment, Background, Element, Fill, Padding, Task, Theme};
use stat_tracker::config::Config;
use std::cell::RefCell;

use crate::app::{Message, TrackerApp};
use crate::daemon::DaemonVerb;
use crate::model::Screen;
use crate::nav::NavStyle;
use crate::settings::SettingsForm;
use crate::settings_model::{
    ReaderChoice, as_form_would_save, columns_for, needs_restart, page_width, row_shown,
};
use crate::settings_rows::Tag;
use crate::settings_save::{Commit, Draft, SaveFrom, dirty_drafts, discard_drafts};
use crate::status_pill::Key;
use crate::theme::{
    self, BORDER, DANGER, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_META, SURFACE,
    TEXT, TEXT_2,
};

/// Space above and below the bar's line (about 56 px tall in all).
const BAR_PAD_Y: f32 = 10.0;
/// Space between the bar's buttons.
const BUTTON_GAP: f32 = 8.0;
/// Narrowest content width that keeps the words and buttons on one line;
/// below it the buttons go on a second line, end-aligned (spec 2.7). A
/// test lays out the longest copy at this width.
pub(crate) const ONE_LINE_MIN: f32 = 640.0;

pub(crate) const DISCARD: &str = "Discard";
pub(crate) const SAVE: &str = "Save";
pub(crate) const SAVE_AND_RESTART: &str = "Save and restart";
pub(crate) const SAVE_AND_START: &str = "Save and start";
pub(crate) const RESTART_TRACKER: &str = "Restart tracker";
pub(crate) const START_TRACKER: &str = "Start tracker";

/// The leave confirm (spec 2.5) and the pill's Stop prompt (spec 1.5).
pub(crate) const LEAVE_TITLE: &str = "You have unsaved changes";
pub(crate) const STOP_TITLE: &str = "Save changes first?";
pub(crate) const DISCARD_AND_LEAVE: &str = "Discard and leave";
pub(crate) const DISCARD_AND_STOP: &str = "Discard and stop";
pub(crate) const SAVE_AND_STOP: &str = "Save and stop";
pub(crate) const CANCEL: &str = "Cancel";

/// What the bar keeps between frames, in [`crate::settings_update::PageState`].
#[derive(Debug, Clone, Default)]
pub struct BarState {
    /// The last Save from the bar was refused: its message (spec 2.6), and
    /// the typed text it was refused for. It shows while that text is
    /// still typed, so editing the field clears a stale message.
    error: Option<(String, DraftText)>,
    /// The bar button with the keyboard (D3: after the pill in Tab order).
    pub(crate) focus: Option<BarButton>,
    /// The leave confirm or the Stop prompt, while open.
    pub(crate) leave: Option<LeaveConfirm>,
    /// The waiting keys with the baseline and saved config they came from
    /// ([`waiting_keys`]), so a frame works them out only after a change.
    waiting_memo: RefCell<Option<(Config, Config, Vec<String>)>>,
}

/// The typed text of every draft group, to tell whether an error is
/// still about what is typed.
#[derive(Debug, Clone, PartialEq, Eq)]
struct DraftText([String; 5], bool);

impl DraftText {
    fn of(form: &SettingsForm) -> Self {
        Self(
            [
                form.poll_interval_secs.clone(),
                form.cooldown_secs.clone(),
                form.sync_url.clone(),
                form.sync_token.clone(),
                form.overlay_hotkey.clone(),
            ],
            form.overlay_hotkey_enabled,
        )
    }
}

/// A button on the bar, for the keyboard.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum BarButton {
    Discard,
    Save,
    Primary,
}

/// What a leave confirm does once the typed text is saved or dropped.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Leave {
    /// Another tab.
    Screen(Screen),
    /// Run setup again: the guide writes config.toml.
    SetupGuide,
    /// Uninstall.
    Uninstall,
    /// The app's own restart after an update (Restart now).
    RestartApp,
    /// Quit from the tray.
    Quit,
    /// The window's close button with no tray, which quits the app.
    CloseWindow(iced::window::Id),
    /// Stop on the status pill (spec 1.5).
    StopTracker,
}

/// Buttons of the leave confirm, for the keyboard. Cancel is the default.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub(crate) enum LeaveButton {
    #[default]
    Cancel,
    Discard,
    Save,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct LeaveConfirm {
    pub(crate) after: Leave,
    pub(crate) focus: LeaveButton,
}

/// A message from the bar or the leave confirm.
#[derive(Debug, Clone)]
pub enum Msg {
    Discard,
    /// The plain Save beside Save and start (tracker stopped, D6).
    Save,
    /// The primary button, whatever it says ([`Pending::primary`]).
    Primary,
    LeaveCancel,
    LeaveDiscard,
    LeaveSave,
}

/// The bar's primary button (spec 2.3, 2.4).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Primary {
    Save,
    SaveAndRestart,
    SaveAndStart,
    RestartTracker,
    StartTracker,
}

impl Primary {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Primary::Save => SAVE,
            Primary::SaveAndRestart => SAVE_AND_RESTART,
            Primary::SaveAndStart => SAVE_AND_START,
            Primary::RestartTracker => RESTART_TRACKER,
            Primary::StartTracker => START_TRACKER,
        }
    }

    /// The tracker verb after the save, if any.
    fn verb(self) -> Option<DaemonVerb> {
        match self {
            Primary::Save => None,
            Primary::SaveAndRestart | Primary::RestartTracker => Some(DaemonVerb::Restart),
            Primary::SaveAndStart | Primary::StartTracker => Some(DaemonVerb::Start),
        }
    }
}

/// What is pending, as the bar counts it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Pending {
    /// Typed groups not saved yet ("unsaved").
    pub(crate) drafts: Vec<Draft>,
    /// Saved keys the running tracker has not loaded ("waiting").
    pub(crate) waiting: usize,
    /// The tracker runs.
    pub(crate) running: bool,
}

impl Pending {
    /// Something is pending, so the bar shows (on Settings).
    pub(crate) fn any(&self) -> bool {
        !self.drafts.is_empty() || self.waiting > 0
    }

    /// The primary button (spec 2.3, 2.4), `None` with nothing pending.
    ///
    /// D5 (tracker stopped, changes waiting, nothing typed): Start tracker,
    /// as the strip offered before. D6 (stopped, typed text): Save and
    /// start, with a plain Save beside it ([`Self::plain_save`]).
    pub(crate) fn primary(&self) -> Option<Primary> {
        let unsaved = !self.drafts.is_empty();
        let tracker_reads = self.drafts.iter().any(|draft| draft.tracker_reads());
        Some(match (unsaved, self.running) {
            (true, true) if tracker_reads || self.waiting > 0 => Primary::SaveAndRestart,
            (true, true) => Primary::Save,
            (true, false) => Primary::SaveAndStart,
            (false, true) if self.waiting > 0 => Primary::RestartTracker,
            (false, false) if self.waiting > 0 => Primary::StartTracker,
            (false, _) => return None,
        })
    }

    /// A plain Save sits beside the primary button: only when the primary
    /// would also start a stopped tracker (D6).
    pub(crate) fn plain_save(&self) -> bool {
        self.primary() == Some(Primary::SaveAndStart)
    }

    /// The bar's buttons in order: Discard (only with typed text), the
    /// plain Save, then the primary (spec 2.2).
    pub(crate) fn buttons(&self) -> Vec<BarButton> {
        let mut out = Vec::new();
        if !self.drafts.is_empty() {
            out.push(BarButton::Discard);
        }
        if self.plain_save() {
            out.push(BarButton::Save);
        }
        if self.primary().is_some() {
            out.push(BarButton::Primary);
        }
        out
    }
}

/// The bar's words (spec 2.2). D4: two separate numbers, never a sum.
pub(crate) fn words(unsaved: usize, waiting: usize) -> String {
    let unsaved_words = |n: usize| {
        if n == 1 {
            "1 unsaved change".to_string()
        } else {
            format!("{n} unsaved changes")
        }
    };
    match (unsaved, waiting) {
        (0, 1) => "1 change waiting".to_string(),
        (0, n) => format!("{n} changes waiting"),
        (n, 0) => unsaved_words(n),
        (n, m) => format!("{}, {m} waiting for restart", unsaved_words(n)),
    }
}

/// The bar and the leave confirm work on the 0.6 page only, never in the
/// demo (it cannot write or restart). The classic page keeps its Save.
fn applies(app: &TrackerApp) -> bool {
    app.nav_style == NavStyle::TopTabs && app.fixture.is_none()
}

/// The typed groups not saved yet. A token from outside locks the website
/// fields, so they never count.
pub(crate) fn drafts(app: &TrackerApp) -> Vec<Draft> {
    if !applies(app) {
        return Vec::new();
    }
    let mut drafts = dirty_drafts(&app.settings, &app.form_from_saved());
    if app.sync_locked() {
        drafts.retain(|draft| *draft != Draft::Sync);
    }
    drafts
}

/// What is pending now, on any tab.
pub(crate) fn pending(app: &TrackerApp) -> Pending {
    if !applies(app) {
        return Pending {
            drafts: Vec::new(),
            waiting: 0,
            running: false,
        };
    }
    Pending {
        drafts: drafts(app),
        waiting: app.changes_waiting(),
        running: app.daemon.running(),
    }
}

/// The bar shows: on Settings, with something pending (spec 2.2).
pub(crate) fn shows(app: &TrackerApp) -> bool {
    app.screen == Screen::Settings && pending(app).any()
}

/// The refused save's message, while the text it was refused for is
/// still typed.
pub(crate) fn error(app: &TrackerApp) -> Option<&str> {
    let (message, typed) = app.page_state.bar.error.as_ref()?;
    (*typed == DraftText::of(&app.settings)).then_some(message.as_str())
}

/// The top-level config keys whose saved value the running tracker has not
/// loaded: what [`crate::settings_model::changes_waiting`] counts, by key.
/// A Reader pick waits as `reader` and `ocr`, a name as `player_name` and
/// `player_names`. App-only, hidden and not-read keys never wait.
pub(crate) fn waiting_keys_between(running: &Config, saved: &Config) -> Vec<String> {
    let running = as_form_would_save(running);
    let saved = as_form_would_save(saved);
    let mut out: Vec<String> = Vec::new();
    if ReaderChoice::of(&running) != ReaderChoice::of(&saved) {
        out.extend(["reader".into(), "ocr".into()]);
    }
    if running.own_player_names() != saved.own_player_names() {
        out.extend(["player_name".into(), "player_names".into()]);
    }
    let (Ok(serde_json::Value::Object(a)), Ok(serde_json::Value::Object(b))) =
        (serde_json::to_value(&running), serde_json::to_value(&saved))
    else {
        return out;
    };
    let mut keys: Vec<&String> = a.keys().chain(b.keys()).collect();
    keys.sort();
    keys.dedup();
    for key in keys {
        let grouped = ["reader", "ocr", "player_name", "player_names"];
        if grouped.contains(&key.as_str())
            || !needs_restart(key)
            || !row_shown(key)
            || a.get(key) == b.get(key)
        {
            continue;
        }
        out.push(key.clone());
    }
    out
}

/// The waiting keys now, memoised like the count.
fn waiting_keys(app: &TrackerApp) -> std::cell::Ref<'_, Vec<String>> {
    let baseline = app.page_state.restart_baseline();
    let saved = &app.saved_config;
    let memo = &app.page_state.bar.waiting_memo;
    let fresh = memo
        .borrow()
        .as_ref()
        .is_some_and(|(b, s, _)| b == baseline && s == saved);
    if !fresh {
        let keys = waiting_keys_between(baseline, saved);
        *memo.borrow_mut() = Some((baseline.clone(), saved.clone(), keys));
    }
    std::cell::Ref::map(memo.borrow(), |m| &m.as_ref().expect("filled above").2)
}

/// The saved value of `key` waits for a tracker restart (D9: the After
/// restart tag shows only then; the bar carries the count).
pub(crate) fn key_waiting(app: &TrackerApp, key: &str) -> bool {
    waiting_keys(app).iter().any(|k| k == key)
}

/// A row's tags for `key`: After restart while its change waits.
pub(crate) fn restart_tag(app: &TrackerApp, key: &str) -> &'static [Tag] {
    if key_waiting(app, key) {
        &[Tag::AfterRestart]
    } else {
        &[]
    }
}

/// A row's tags for one value inside a key (`auto_detect.enabled`, say):
/// After restart while that value, as the running tracker loaded it,
/// differs from what is saved. The key alone would tag every row it holds.
pub(crate) fn part_tag<T: PartialEq>(
    app: &TrackerApp,
    part: impl Fn(&Config) -> T,
) -> &'static [Tag] {
    if part(app.page_state.restart_baseline()) != part(&app.saved_config) {
        &[Tag::AfterRestart]
    } else {
        &[]
    }
}

/// A fold's After restart tag: one of its shown keys waits.
pub(crate) fn section_waiting(
    app: &TrackerApp,
    section: crate::settings_model::AdvancedSection,
) -> bool {
    section.keys().iter().any(|key| key_waiting(app, key))
}

/// Put `message` on the bar as a refused save would, for layout tests
/// that must not write a config.toml.
#[cfg(test)]
pub(crate) fn show_error(app: &mut TrackerApp, message: &str) {
    app.page_state.bar.error = Some((message.to_string(), DraftText::of(&app.settings)));
}

/// Save on the bar, and the save before a verb or a leave: every typed
/// group in one write. `true` when the file now holds them (or nothing
/// was typed). A refusal keeps the text typed, restarts nothing, and puts
/// its message on the bar instead of a toast while the text is still
/// typed (spec 2.6); a reload from disk that left nothing typed keeps
/// today's toast.
pub(crate) fn save_drafts(app: &mut TrackerApp) -> (Task<Message>, bool) {
    app.page_state.bar.error = None;
    if drafts(app).is_empty() {
        return (Task::none(), true);
    }
    // Clear the toast first, so a refusal shows up even when the same
    // words already toast (say after Enter was refused the same way).
    let before = (app.toast.take(), app.toast_action.take());
    let (task, ok) = app.save_settings_checked(SaveFrom::Page(Commit::Drafts), None);
    if app.toast.is_none() {
        // Nothing new to say: the earlier toast stays.
        (app.toast, app.toast_action) = before;
    } else if !ok && !drafts(app).is_empty() {
        let message = app.toast.take().unwrap_or_default();
        app.toast_action = None;
        app.page_state.bar.error = Some((message, DraftText::of(&app.settings)));
    }
    (task, ok)
}

/// Discard (spec 2.3): the typed groups back to what is saved. Switches
/// and chips are already written and stay. Nothing restarts.
pub(crate) fn discard(app: &mut TrackerApp) {
    let saved = app.form_from_saved();
    discard_drafts(&mut app.settings, &saved);
    app.page_state.bar.error = None;
}

/// Handle a bar or leave confirm message.
pub(crate) fn update(app: &mut TrackerApp, msg: Msg) -> Task<Message> {
    match msg {
        Msg::Discard => {
            discard(app);
            Task::none()
        }
        Msg::Save => save_drafts(app).0,
        Msg::Primary => primary(app),
        Msg::LeaveCancel => {
            app.page_state.bar.leave = None;
            Task::none()
        }
        Msg::LeaveDiscard => {
            let Some(confirm) = app.page_state.bar.leave.take() else {
                return Task::none();
            };
            discard(app);
            go_on(app, confirm.after)
        }
        Msg::LeaveSave => {
            let Some(confirm) = app.page_state.bar.leave.take() else {
                return Task::none();
            };
            let restart = leave_saves_and_restarts(&pending(app), confirm.after);
            let (saved, ok) = save_drafts(app);
            if !ok {
                // Stay on Settings with the message on the bar.
                return saved;
            }
            let restart = if restart {
                crate::status_pill::request(app, DaemonVerb::Restart)
            } else {
                Task::none()
            };
            Task::batch([saved, restart, go_on(app, confirm.after)])
        }
    }
}

/// The primary button: save, then the verb through the pill (the
/// mid-game confirm is the pill's), or the verb alone.
fn primary(app: &mut TrackerApp) -> Task<Message> {
    let pending = pending(app);
    let Some(primary) = pending.primary() else {
        return Task::none();
    };
    if primary.verb().is_some() && app.daemon_busy {
        return Task::none();
    }
    let (saved, ok) = save_drafts(app);
    if !ok {
        return saved;
    }
    match primary.verb() {
        Some(verb) => Task::batch([saved, crate::status_pill::request(app, verb)]),
        None => saved,
    }
}

/// The leave confirm's Save also restarts the tracker: spec 2.5 "Save
/// and restart if 2.4 applies", but never on the way out of the app (a
/// quit would drop the restart) or into the app's own restart, which
/// restarts the tracker itself.
fn leave_saves_and_restarts(pending: &Pending, after: Leave) -> bool {
    pending.primary() == Some(Primary::SaveAndRestart)
        && matches!(
            after,
            Leave::Screen(_) | Leave::SetupGuide | Leave::Uninstall
        )
}

/// The words of the leave confirm: title, Discard button, Save button.
pub(crate) fn leave_words(
    pending: &Pending,
    after: Leave,
) -> (&'static str, &'static str, &'static str) {
    if after == Leave::StopTracker {
        return (STOP_TITLE, DISCARD_AND_STOP, SAVE_AND_STOP);
    }
    let save = if leave_saves_and_restarts(pending, after) {
        SAVE_AND_RESTART
    } else {
        SAVE
    };
    (LEAVE_TITLE, DISCARD_AND_LEAVE, save)
}

/// Carry on with what the leave confirm held up. The typed text is saved
/// or dropped by now, so the replayed message passes [`intercept`].
fn go_on(app: &mut TrackerApp, after: Leave) -> Task<Message> {
    match after {
        Leave::Screen(screen) => app.update(Message::Navigate(screen)),
        Leave::SetupGuide => app.update(Message::Setup(crate::setup_guide::SetupMessage::Open)),
        Leave::Uninstall => app.update(Message::OpenUninstall),
        Leave::RestartApp => app.update(Message::RestartGui),
        Leave::Quit => app.update(Message::Tray(crate::tray::TrayAction::Quit)),
        Leave::CloseWindow(id) => iced::window::close(id),
        Leave::StopTracker => crate::status_pill::request(app, DaemonVerb::Stop),
    }
}

/// Run before [`TrackerApp::update`] handles `message`. A message that
/// leaves Settings while typed text is not saved opens the leave confirm
/// instead (spec 2.5, D8) and is replayed once the text is saved or
/// dropped. `None` lets the message through.
///
/// D8: a tab change, Run setup again, Uninstall, the app's own Restart
/// now, tray Quit (the window is shown to ask in) and a window close with
/// no tray ask. Hide window and a close with a tray do not: the app keeps
/// running and the typed text stays, with the bar, when the window comes
/// back. Read-only dialogs (About, What leaves your PC, Report a problem,
/// What's new) do not ask either; the text waits under them.
pub(crate) fn intercept(app: &mut TrackerApp, message: &Message) -> Option<Task<Message>> {
    use crate::setup_guide::SetupMessage;
    use crate::tray::TrayAction;
    let after = match message {
        Message::Navigate(screen) if *screen != app.screen => Leave::Screen(*screen),
        Message::Setup(SetupMessage::Open) => Leave::SetupGuide,
        Message::OpenUninstall => Leave::Uninstall,
        Message::RestartGui => Leave::RestartApp,
        Message::Tray(TrayAction::Quit) => Leave::Quit,
        Message::WindowCloseRequested(id) if app.tray.is_none() => Leave::CloseWindow(*id),
        _ => return None,
    };
    if drafts(app).is_empty() {
        return None;
    }
    // B2 (#921 QA, FrozenTear 27 Nov): a Quit or a close is answered in
    // front of the user, so a read-only dialog closes first.
    let closed = if matches!(after, Leave::Quit | Leave::CloseWindow(_)) {
        close_read_only_dialogs(app)
    } else {
        Task::none()
    };
    let asked = Task::batch([closed, ask(app, after)]);
    if after == Leave::Quit {
        return Some(Task::batch([
            app.update(Message::Tray(TrayAction::Show)),
            asked,
        ]));
    }
    Some(asked)
}

/// Close What leaves your PC, Report a problem, and What's new or About
/// (one dialog), each through its own close, so the leave confirm under
/// the dialogs is what the user sees. A report that is sending stays
/// open, as its own close leaves it.
fn close_read_only_dialogs(app: &mut TrackerApp) -> Task<Message> {
    let mut tasks = Vec::new();
    if app.page_state.opened.what_leaves_open {
        let close = crate::settings_open::OpenMsg::WhatLeaves(false);
        tasks.push(
            app.update(Message::SettingsPage(crate::settings_update::Msg::Open(
                close,
            ))),
        );
    }
    if app.report.is_some() {
        tasks.push(app.update(Message::Report(crate::report::ReportMessage::Close)));
    }
    if app.notes_dialog.is_some() {
        tasks.push(app.update(Message::DismissNotes));
    }
    Task::batch(tasks)
}

/// R5 (#921 QA, FrozenTear 27 Nov): a typed group whose field a switch
/// just hid is dropped, back to what is saved. It leaves the unsaved
/// count, Save never writes it, and showing the field again shows the
/// saved value. Today that is the timings, hidden while Watch for match
/// start and end is off.
pub(crate) fn drop_hidden_drafts(app: &mut TrackerApp) {
    if !app.settings.auto_detect_enabled {
        let saved = app.form_from_saved();
        crate::settings_save::discard_draft(&mut app.settings, &saved, Draft::Timings);
    }
}

/// Open the leave confirm with Cancel marked, over everything but the
/// dialogs. The pill's menu closes, the bar lets go of the keyboard, and
/// the returned task takes the keyboard from a field under the confirm,
/// so typing cannot change the text the confirm is about.
fn ask(app: &mut TrackerApp, after: Leave) -> Task<Message> {
    app.pill.menu = None;
    app.pill.focused = false;
    app.page_state.bar.focus = None;
    app.page_state.bar.leave = Some(LeaveConfirm {
        after,
        focus: LeaveButton::Cancel,
    });
    iced::advanced::widget::operate(iced::advanced::widget::operation::focusable::unfocus())
}

/// What a pill verb does with typed text open (spec 1.5, D7).
pub(crate) enum Gate {
    /// Go on with the verb, after this task (the save, if any).
    Go(Task<Message>),
    /// Do not run the verb now: the save was refused (its message is on
    /// the bar), or the Stop prompt is open.
    Held(Task<Message>),
}

/// Before a pill verb: Start, Retry and Restart save the typed text first
/// (whatever the group); a refused save stops the verb. Stop asks "Save
/// changes first?", before any mid-game confirm (D7 A: the same order as
/// Restart, save then confirm).
pub(crate) fn before_verb(app: &mut TrackerApp, verb: DaemonVerb) -> Gate {
    if drafts(app).is_empty() {
        return Gate::Go(Task::none());
    }
    if verb == DaemonVerb::Stop {
        return Gate::Held(ask(app, Leave::StopTracker));
    }
    match save_drafts(app) {
        (task, true) => Gate::Go(task),
        (task, false) => Gate::Held(task),
    }
}

/// The bar or the leave confirm has the keys.
pub(crate) fn owns_keys(app: &TrackerApp) -> bool {
    app.page_state.bar.leave.is_some() || app.page_state.bar.focus.is_some()
}

/// A key while the leave confirm is open: Tab moves Cancel, Discard,
/// Save; Enter or Space presses; Escape is Cancel (spec 2.5).
pub(crate) fn leave_key(app: &mut TrackerApp, key: Key) -> Task<Message> {
    let Some(confirm) = app.page_state.bar.leave.as_mut() else {
        return Task::none();
    };
    let pressed = match key {
        Key::Tab => {
            confirm.focus = match confirm.focus {
                LeaveButton::Cancel => LeaveButton::Discard,
                LeaveButton::Discard => LeaveButton::Save,
                LeaveButton::Save => LeaveButton::Cancel,
            };
            return Task::none();
        }
        Key::Escape => LeaveButton::Cancel,
        Key::Enter | Key::Space => confirm.focus,
        Key::Up | Key::Down => return Task::none(),
    };
    update(
        app,
        match pressed {
            LeaveButton::Cancel => Msg::LeaveCancel,
            LeaveButton::Discard => Msg::LeaveDiscard,
            LeaveButton::Save => Msg::LeaveSave,
        },
    )
}

/// Tab from the pill (D3): the bar's first button, when the bar shows.
/// `true` when the bar took the keyboard.
pub(crate) fn focus_first(app: &mut TrackerApp) -> bool {
    if !shows(app) {
        return false;
    }
    app.page_state.bar.focus = pending(app).buttons().first().copied();
    app.page_state.bar.focus.is_some()
}

/// A key while a bar button has the keyboard. Tab moves to the next
/// button, then off the bar; Escape lets go and never discards (spec
/// 2.8); Enter or Space presses. `None` when no bar button has it (or it
/// is gone), so the key goes to the pill.
pub(crate) fn focus_key(app: &mut TrackerApp, key: Key) -> Option<Task<Message>> {
    let at = app.page_state.bar.focus?;
    let buttons = if shows(app) {
        pending(app).buttons()
    } else {
        Vec::new()
    };
    let Some(index) = buttons.iter().position(|b| *b == at) else {
        app.page_state.bar.focus = None;
        return None;
    };
    match key {
        Key::Tab => app.page_state.bar.focus = buttons.get(index + 1).copied(),
        Key::Escape => app.page_state.bar.focus = None,
        Key::Enter | Key::Space => {
            return Some(update(
                app,
                match at {
                    BarButton::Discard => Msg::Discard,
                    BarButton::Save => Msg::Save,
                    BarButton::Primary => Msg::Primary,
                },
            ));
        }
        Key::Up | Key::Down => {}
    }
    Some(Task::none())
}

fn bar_msg(msg: Msg) -> Message {
    Message::SettingsPage(crate::settings_update::Msg::Bar(msg))
}

/// The slot under the page in the top tabs window (D1): the bar while it
/// shows, else a zero-size space, so the tree keeps its shape.
pub(crate) fn slot(app: &TrackerApp, window_width: f32) -> Element<'_, Message> {
    if !shows(app) {
        return space().into();
    }
    view(app, window_width)
}

/// The bar for a `window_width` px window: full width, SURFACE fill, a
/// 1 px BORDER line along the top, the page gutter at the sides, the
/// content as wide as the Settings page and aligned with it (spec 2.7).
pub(crate) fn view(app: &TrackerApp, window_width: f32) -> Element<'_, Message> {
    let pending = pending(app);
    let gutter = theme::page_gutter(window_width);
    let content = (window_width - 2.0 * gutter).max(0.0);
    let mut copy = Column::new().spacing(4).width(Fill).push(
        text(words(pending.drafts.len(), pending.waiting))
            .size(SIZE_BODY)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    );
    if let Some(message) = error(app) {
        copy = copy.push(
            text(message)
                .size(SIZE_META)
                .font(FONT_MEDIUM)
                .color(DANGER),
        );
    }
    let focus = app.page_state.bar.focus;
    let busy = app.daemon_busy;
    let mut buttons = Row::new().spacing(BUTTON_GAP).align_y(Alignment::Center);
    for which in pending.buttons() {
        let (label, msg, accent, enabled) = match which {
            BarButton::Discard => (DISCARD, Msg::Discard, false, true),
            BarButton::Save => (SAVE, Msg::Save, false, true),
            BarButton::Primary => {
                let primary = pending.primary().expect("buttons() lists it");
                let enabled = !(busy && primary.verb().is_some());
                (primary.label(), Msg::Primary, true, enabled)
            }
        };
        buttons = buttons.push(bar_button(
            label,
            msg,
            accent,
            enabled,
            focus == Some(which),
        ));
    }
    let body: Element<'_, Message> = if content >= ONE_LINE_MIN {
        row_body(copy, buttons)
    } else {
        column![copy, container(buttons).width(Fill).align_right(Fill)]
            .spacing(8)
            .into()
    };
    let inner = container(body).max_width(page_width(content)).width(Fill);
    let aligned = if columns_for(content) == 2 {
        container(inner).width(Fill).align_left(Fill)
    } else {
        container(inner).width(Fill).center_x(Fill)
    };
    let line = container(space().height(1))
        .width(Fill)
        .height(1)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(BORDER)),
            ..container::Style::default()
        });
    let fill = container(aligned)
        .padding(Padding::from([BAR_PAD_Y, gutter]))
        .width(Fill)
        .style(|_: &Theme| container::Style {
            background: Some(Background::Color(SURFACE)),
            ..container::Style::default()
        });
    column![line, fill].width(Fill).into()
}

fn row_body<'a>(copy: Column<'a, Message>, buttons: Row<'a, Message>) -> Element<'a, Message> {
    iced::widget::row![copy, buttons]
        .spacing(16)
        .align_y(Alignment::Center)
        .into()
}

/// A bar or confirm button: accent fill for the primary, ghost otherwise,
/// a 2 px ACCENT ring when it has the keyboard.
fn bar_button(
    label: &'static str,
    msg: Msg,
    accent: bool,
    enabled: bool,
    ring: bool,
) -> Element<'static, Message> {
    let b = button(text(label).size(SIZE_META).font(FONT_BOLD).color(TEXT))
        .padding(Padding::from([9, 18]))
        .style(button_style(accent, enabled, ring));
    if enabled {
        b.on_press(bar_msg(msg)).into()
    } else {
        b.into()
    }
}

/// A bar or confirm button's look. The keyboard ring is 2 px ACCENT on a
/// ghost button; on the ACCENT-filled primary an ACCENT ring would not
/// show, so it is TEXT there (R3 of the #921 QA).
pub(crate) fn button_style(
    accent: bool,
    enabled: bool,
    ring: bool,
) -> impl Fn(&Theme, button::Status) -> button::Style {
    move |theme_, status| {
        let mut out = if accent {
            crate::settings_page::primary_btn(enabled)(theme_, status)
        } else {
            crate::settings_page::ghost_btn(enabled)(theme_, status)
        };
        if ring {
            out.border.color = if accent { TEXT } else { theme::ACCENT };
            out.border.width = 2.0;
        }
        out
    }
}

/// The leave confirm or the Stop prompt as a layer over the window, under
/// the dialogs ([`crate::chrome::with_layers`]); `None` while closed.
pub(crate) fn leave_layer(app: &TrackerApp) -> Option<Element<'_, Message>> {
    let confirm = app.page_state.bar.leave?;
    let pending = pending(app);
    let (title, discard_label, save_label) = leave_words(&pending, confirm.after);
    let ring = |b: LeaveButton| confirm.focus == b;
    let buttons = iced::widget::row![
        space().width(Fill),
        bar_button(
            CANCEL,
            Msg::LeaveCancel,
            false,
            true,
            ring(LeaveButton::Cancel)
        ),
        bar_button(
            discard_label,
            Msg::LeaveDiscard,
            false,
            true,
            ring(LeaveButton::Discard)
        ),
        bar_button(
            save_label,
            Msg::LeaveSave,
            true,
            true,
            ring(LeaveButton::Save)
        ),
    ]
    .spacing(BUTTON_GAP);
    let card = container(
        column![
            text(title)
                .size(SIZE_BODY + 4.0)
                .font(FONT_BOLD)
                .color(TEXT),
            text(words(pending.drafts.len(), 0))
                .size(SIZE_BODY)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
            buttons,
        ]
        .spacing(12),
    )
    .padding(24)
    .max_width(520)
    .style(theme::surface_panel);
    let mut backdrop = theme::BG;
    backdrop.a = 0.88;
    let scrim = mouse_area(
        container(space())
            .width(Fill)
            .height(Fill)
            .style(move |_: &Theme| container::Style {
                background: Some(Background::Color(backdrop)),
                ..container::Style::default()
            }),
    )
    .on_press(bar_msg(Msg::LeaveCancel));
    let card = container(opaque(card)).padding(28).center(Fill);
    Some(stack![scrim, card].width(Fill).height(Fill).into())
}

#[cfg(test)]
#[path = "settings_bar_tests.rs"]
mod tests;
