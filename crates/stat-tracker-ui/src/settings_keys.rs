//! The 0.6 Settings rows that write the config keys added for 0.6: In-game
//! names as a list (`player_name` first, then `player_names`), Upload
//! games after each match (`live_upload`), Read my rank after Comp games
//! (`rank_reader`, under READERS in Capture and reading), the Sharing
//! switch (`share_stat_pictures`), Report pictures
//! (`share_report_pictures`) and Only capture while Overwatch is in front
//! (`capture_only_on_screen`, with a live note while the tracker cannot
//! tell which window is in front). Each applies at once. The tracker reads
//! names, the focus gate and `rank_reader` when it starts, so those rows
//! are tagged After restart.
//!
//! The upload row is not shown ([`crate::settings_model::HIDDEN_KEYS`]);
//! its key still loads and saves as before. The rows whose key nothing
//! reads yet ([`crate::settings_model::NOT_READ_KEYS`]) show
//! [`NOT_READ_NOTE`] and no After restart tag.

use iced::Element;
use iced::widget::Row;

use crate::app::{Message, TrackerApp};
use crate::form_widgets::hint;
use crate::settings::SettingsField;
use crate::settings_model::{ChipList, ConfigEdit, needs_restart, row_shown, same_name};
use crate::settings_rows::{
    Segment, Tag, add_chip, chip, chip_control, segmented, setting_row, switch, title_line,
};
use crate::settings_update::Msg;

/// Under a row whose key nothing reads yet.
const NOT_READ_NOTE: &str = "Not read by this build yet, so this changes nothing for now.";

/// The row's tags for `key`: After restart only when the tracker reads it.
fn read_tags(key: &str) -> &'static [Tag] {
    if needs_restart(key) {
        &[Tag::AfterRestart]
    } else {
        &[]
    }
}

/// Help under In-game names (spec).
const NAMES_HELP: &str = "As shown in-game, without the #1234.";
/// Under Upload games while it is off.
const UPLOAD_OFF_NOTE: &str = "Games stay on this computer and upload once this is on again.";
/// Under Upload games while no website is set, and under "Not connected"
/// while that row is hidden.
pub(crate) const UPLOAD_NO_SITE: &str =
    "Add the website URL and token under Advanced, Website connection.";

/// A switch that writes `edit(on)`, or a disabled one in a demo.
pub(crate) fn edit_switch<'a>(
    on: bool,
    demo: bool,
    edit: fn(bool) -> ConfigEdit,
) -> Element<'a, Message> {
    switch(
        on,
        (!demo).then(|| {
            Box::new(move |v| Message::SettingsPage(Msg::Edit(edit(v))))
                as Box<dyn Fn(bool) -> Message + 'a>
        }),
    )
}

/// The names the In-game names row shows: the form's `player_name`
/// first, then (while that row is shown) each saved `player_names` entry
/// that is not the same name ([`same_name`]: any case, Unicode too).
pub(crate) fn shown_names(app: &TrackerApp) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let primary = app.settings.player_name.trim();
    if !primary.is_empty() {
        out.push(primary.to_string());
    }
    if row_shown("player_names") {
        for name in &app.saved_config.player_names {
            let name = name.trim();
            if !name.is_empty() && !out.iter().any(|n| same_name(n, name)) {
                out.push(name.to_string());
            }
        }
    }
    out
}

/// "+ Add name" took `name`. It fills an empty `player_name` (the name the
/// website lookup and the Tab reader use). Otherwise it joins
/// `player_names`, or replaces `player_name` while extra names are hidden.
pub(crate) fn name_added(app: &mut TrackerApp, name: String) -> Option<ConfigEdit> {
    if app.settings.player_name.trim().is_empty() || !row_shown("player_names") {
        app.settings.set_text(SettingsField::PlayerName, name);
        None
    } else {
        Some(ConfigEdit::AddPlayerName(name))
    }
}

/// The "×" on name `index` of [`shown_names`]. Removing the first name
/// moves the next one into `player_name`, so it is never left empty while
/// other names remain, and drops every saved spelling of the removed name
/// so none of them comes back as a chip.
pub(crate) fn name_removed(app: &mut TrackerApp, index: usize) -> Option<ConfigEdit> {
    let names = shown_names(app);
    let name = names.get(index)?.clone();
    let primary = !app.settings.player_name.trim().is_empty();
    if index == 0 && primary {
        let next = names.get(1).cloned();
        app.settings
            .set_text(SettingsField::PlayerName, next.clone().unwrap_or_default());
        let mut gone = vec![name];
        gone.extend(next);
        Some(ConfigEdit::RemovePlayerNames(gone))
    } else {
        Some(ConfigEdit::RemovePlayerNames(vec![name]))
    }
}

/// Whether "+ Add name" shows: always while extra names are shown, else
/// only while there is no name (or its input is open).
pub(crate) fn add_name_shown(app: &TrackerApp) -> bool {
    row_shown("player_names")
        || app.settings.player_name.trim().is_empty()
        || matches!(app.page_state.chip_draft(), Some((ChipList::Names, _)))
}

/// In-game names: one chip per name with "×", and "+ Add name" after them
/// (spec p6).
pub(crate) fn names_row(app: &TrackerApp, demo: bool) -> Element<'_, Message> {
    let mut chips = Row::new();
    for (i, name) in shown_names(app).into_iter().enumerate() {
        chips = chips.push(chip(
            name,
            (!demo).then_some(Message::SettingsPage(Msg::ChipRemove(ChipList::Names, i))),
        ));
    }
    if add_name_shown(app) {
        chips = chips.push(add_chip(app, ChipList::Names, "+ Add name", "Name", demo));
    }
    setting_row(
        title_line("In-game names", false, read_tags("player_names")),
        vec![hint(NAMES_HELP)],
        chip_control(chips),
    )
}

/// The note under "Not connected": where to add the website, while the
/// Upload row that says it is hidden. A refused token's line already
/// names the place, so then there is none (one pointer only).
pub(crate) fn not_connected_note(rejected: bool) -> Option<&'static str> {
    (!rejected && !row_shown("live_upload")).then_some(UPLOAD_NO_SITE)
}

/// How Upload games after each match is drawn: on or off, whether the
/// switch can be used, and the note under it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct UploadState {
    pub on: bool,
    pub enabled: bool,
    pub note: Option<&'static str>,
}

/// [`UploadState`]: live while a website and token are saved (and not in a
/// demo), off and disabled without one.
pub(crate) fn upload_state(app: &TrackerApp, demo: bool) -> UploadState {
    let site = app.sync_on();
    let on = site && app.saved_config.live_upload;
    UploadState {
        on,
        enabled: site && !demo,
        note: if !site {
            Some(UPLOAD_NO_SITE)
        } else if !on {
            Some(UPLOAD_OFF_NOTE)
        } else {
            None
        },
    }
}

/// Upload games after each match, or `None` while hidden.
pub(crate) fn upload_row<'a>(
    app: &'a TrackerApp,
    demo: bool,
    help: &'static str,
    show_note: bool,
) -> Option<Element<'a, Message>> {
    if !row_shown("live_upload") {
        return None;
    }
    let state = upload_state(app, demo);
    let mut lines = vec![hint(help)];
    if let Some(note) = state.note.filter(|_| show_note) {
        lines.push(hint(note));
    }
    let control = if state.enabled {
        edit_switch(state.on, false, ConfigEdit::LiveUpload)
    } else {
        switch(state.on, None)
    };
    Some(setting_row(
        title_line("Upload games after each match", false, &[Tag::AfterRestart]),
        lines,
        control,
    ))
}

/// The lines under Read my rank after Comp games: its help, a lock line
/// for each reader an env variable decides (the rank one also locks the
/// switch, which shows the rank reader in effect), and the warning for a
/// reader that is on with no usable pack (games still record).
fn rank_lines<'a>(
    app: &TrackerApp,
    switch: crate::settings_env::RankSwitch,
    help: &'static str,
) -> Vec<Element<'a, Message>> {
    use crate::settings_env::{QUEUE_TITLE_ENV, RANK_ENV};
    let mut lines = vec![hint(help)];
    for (set, env) in [
        (switch.rank_env, RANK_ENV),
        (switch.queue_env, QUEUE_TITLE_ENV),
    ] {
        if set {
            lines.push(crate::capture_reading::lock_line(env));
        }
    }
    if switch.rank_env || switch.queue_env {
        lines.push(hint(crate::capture_reading::SERVICE_MAY_DIFFER));
    }
    if let Some(warning) = app.rank_pack_warning() {
        lines.push(crate::widgets::warn_line(&warning));
    }
    lines
}

/// The rank switch's label.
pub(crate) const RANK_LABEL: &str = "Read my rank after Comp games";

/// Help under [`RANK_LABEL`].
pub(crate) const RANK_HELP: &str = "Reads the rank screen after each Comp game and the queue title before it, and saves your rank, its change and its label with the game. Needs the rank-v1 and queue-title-v1 packs. Starts off.";

/// Read my rank after Comp games, under READERS in Capture and reading
/// with the other reader switches. `None` while hidden.
pub(crate) fn rank_row(app: &TrackerApp, demo: bool) -> Option<Element<'_, Message>> {
    if !row_shown("rank_reader") {
        return None;
    }
    let rank = app.rank_switch();
    Some(setting_row(
        title_line(RANK_LABEL, false, read_tags("rank_reader")),
        rank_lines(app, rank, RANK_HELP),
        edit_switch(rank.rank_on, demo || rank.rank_env, ConfigEdit::RankReader),
    ))
}

/// The Sharing switch under Website account, starting off. Empty while it
/// is hidden.
pub(crate) fn sharing_rows<'a>(
    app: &'a TrackerApp,
    demo: bool,
    pictures_help: &'static str,
) -> Vec<Element<'a, Message>> {
    let c = &app.saved_config;
    let mut rows = Vec::new();
    if row_shown("share_stat_pictures") {
        rows.push(setting_row(
            title_line(
                "Share stat pictures",
                false,
                read_tags("share_stat_pictures"),
            ),
            vec![hint(pictures_help), hint(NOT_READ_NOTE)],
            edit_switch(c.share_stat_pictures, demo, ConfigEdit::ShareStatPictures),
        ));
    }
    rows
}

/// Report pictures under READING (spec p7), or `None` while hidden. Only
/// this app will read it, so it has no After restart tag.
pub(crate) fn report_pictures_row<'a>(
    app: &'a TrackerApp,
    demo: bool,
    help: &'static str,
) -> Option<Element<'a, Message>> {
    row_shown("share_report_pictures").then(|| {
        setting_row(
            title_line("Report pictures", false, &[Tag::New]),
            vec![hint(help), hint(NOT_READ_NOTE)],
            edit_switch(
                app.saved_config.share_report_pictures,
                demo,
                ConfigEdit::ShareReportPictures,
            ),
        )
    })
}

/// Only capture while Overwatch is in front: Auto or Off (spec p7, UX Spec
/// focus status), or `None` while hidden. While the tracker cannot tell
/// which window is in front, a note says so under the help. The row has
/// the id the top bar's unreadable pill scrolls to, and is marked for a
/// moment after that pill opened it.
pub(crate) fn focus_gate_row<'a>(
    app: &'a TrackerApp,
    demo: bool,
    help: &'static str,
) -> Option<Element<'a, Message>> {
    if !row_shown("capture_only_on_screen") {
        return None;
    }
    let auto = app.saved_config.capture_only_on_screen;
    let pick = |on: bool| focus_gate_pick(demo, on);
    let mut lines = vec![hint(help)];
    if focus_note_shown(app) {
        lines.push(focus_note());
    }
    let row = setting_row(
        title_line(
            crate::focus_status::ROW_TITLE,
            !auto,
            &[Tag::New, Tag::AfterRestart],
        ),
        lines,
        demo_tooltip(
            segmented(vec![
                Segment {
                    label: "Auto",
                    selected: auto,
                    msg: pick(true),
                },
                Segment {
                    label: "Off",
                    selected: !auto,
                    msg: pick(false),
                },
            ]),
            demo,
        ),
    );
    // Marked: a 3 px accent edge on the left only, as the open Advanced
    // section's bar; otherwise the row sits flush with the others.
    let body: Element<'a, Message> = if app.page_state.focus_row_marked(std::time::Instant::now()) {
        let edge = iced::widget::container(iced::widget::space().width(3))
            .width(3)
            .height(iced::Fill)
            .style(|_: &iced::Theme| iced::widget::container::Style {
                background: Some(iced::Background::Color(crate::theme::ACCENT)),
                ..iced::widget::container::Style::default()
            });
        iced::widget::row![edge, row]
            .spacing(8)
            .height(iced::Length::Shrink)
            .into()
    } else {
        row
    };
    Some(
        iced::widget::container(body)
            .id(crate::focus_status::ROW_ID)
            .into(),
    )
}

/// What picking Auto (`true`) or Off in the focus row does: nothing in a
/// demo, where the control is shown but never changes a setting.
fn focus_gate_pick(demo: bool, on: bool) -> Option<Message> {
    (!demo).then_some(Message::SettingsPage(Msg::Edit(
        ConfigEdit::CaptureOnlyOnScreen(on),
    )))
}

/// In a demo `control` carries the tooltip that says why it does nothing.
/// The help text and the live note stay as they are.
fn demo_tooltip(control: Element<'static, Message>, demo: bool) -> Element<'static, Message> {
    use iced::widget::{container, text, tooltip};
    if !demo {
        return control;
    }
    let tip = container(
        text(crate::focus_status::DEMO_TOOLTIP)
            .size(crate::theme::SIZE_LABEL)
            .font(crate::theme::FONT_MEDIUM)
            .color(crate::theme::TEXT_2),
    )
    .padding(iced::Padding::from([8, 12]))
    .style(|_: &iced::Theme| container::Style {
        background: Some(iced::Background::Color(crate::theme::SURFACE_2)),
        border: iced::Border {
            color: crate::theme::BORDER,
            width: 1.0,
            radius: crate::theme::RADIUS_INNER.into(),
        },
        ..container::Style::default()
    });
    tooltip(control, tip, tooltip::Position::Bottom)
        .gap(6)
        .into()
}

/// The live note shows while the running tracker cannot tell which
/// window is in front (as the top bar shows it, debounced), and only while
/// a game runs, as the top bar's focus words: a value left from the last
/// game is not shown. A rejected token does not hide it.
pub(crate) fn focus_note_shown(app: &TrackerApp) -> bool {
    app.fixture.is_none()
        && app.daemon.running()
        && app.game_running
        && app.page_state.focus.shown() == stat_tracker::ocr_status::FocusGateStatus::Unreadable
}

/// The live note: neutral words with an amber dot, as the capture method
/// notes.
fn focus_note() -> Element<'static, Message> {
    use iced::widget::{container, row, space, text};
    let dot = container(space().width(6).height(6))
        .width(6)
        .height(6)
        .style(|_: &iced::Theme| container::Style {
            background: Some(iced::Background::Color(crate::theme::WARN)),
            border: iced::Border {
                radius: 999.0.into(),
                ..iced::Border::default()
            },
            ..container::Style::default()
        });
    row![
        dot,
        text(crate::focus_status::ROW_UNREADABLE_NOTE)
            .size(crate::theme::SIZE_LABEL)
            .font(crate::theme::FONT_MEDIUM)
            .color(crate::theme::TEXT_3),
    ]
    .spacing(6)
    .align_y(iced::Alignment::Center)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_model::{HIDDEN_KEYS, NOT_READ_KEYS};
    use stat_tracker::config::{Config, SyncConfig};

    /// A live (not demo) app whose page writes `config.toml` in a temp
    /// dir, as `settings_update`'s tests set it up.
    fn live_app() -> (tempfile::TempDir, TrackerApp) {
        let (dir, mut app) = crate::fixtures::test_app();
        app.fixture = None;
        let path = dir.path().join("config.toml");
        app.saved_config.save_at(&path).unwrap();
        app.saved_config = Config::read_at(&path).unwrap();
        app.saved_file = app.saved_config.clone();
        app.settings = crate::settings::SettingsForm::from_config(&app.saved_config);
        app.page_state =
            crate::settings_update::PageState::new(app.saved_config.clone(), None, true);
        app.page_state.config_path = Some(path);
        (dir, app)
    }

    fn on_disk(dir: &tempfile::TempDir) -> Config {
        Config::read_at(&dir.path().join("config.toml")).unwrap()
    }

    fn page(msg: Msg) -> Message {
        Message::SettingsPage(msg)
    }

    /// Only the upload switch stays hidden (PR 560 wires it). The rows
    /// nothing reads yet show, each saying so, with no After restart tag.
    #[test]
    fn only_the_upload_row_is_hidden_and_unread_rows_say_so() {
        assert_eq!(HIDDEN_KEYS, ["live_upload"]);
        for key in ["player_names", "capture_only_on_screen"]
            .into_iter()
            .chain(NOT_READ_KEYS)
        {
            assert!(row_shown(key), "{key}");
        }
        let (_dir, app) = crate::fixtures::test_app();
        assert!(upload_row(&app, false, "help", true).is_none());
        assert_eq!(not_connected_note(false), Some(UPLOAD_NO_SITE));
        assert_eq!(sharing_rows(&app, false, "b").len(), 1);
        assert!(rank_row(&app, false).is_some());
        assert!(!RANK_HELP.contains('\u{2014}') && RANK_HELP.contains("Starts off"));
        assert!(report_pictures_row(&app, false, "c").is_some());
        assert!(focus_gate_row(&app, false, "d").is_some());
        assert_eq!(read_tags("rank_reader"), &[Tag::AfterRestart]);
        assert_eq!(read_tags("share_stat_pictures"), &[] as &[Tag]);
        assert_eq!(read_tags("player_names"), &[Tag::AfterRestart]);
        assert!(NOT_READ_NOTE.starts_with("Not read by this build yet"));
    }

    #[test]
    fn the_focus_row_in_a_demo_changes_nothing_and_keeps_its_help_and_note() {
        for on in [true, false] {
            assert!(focus_gate_pick(true, on).is_none(), "demo: no message");
            assert!(focus_gate_pick(false, on).is_some(), "live: a message");
        }
        // The row still builds in a demo (help, note and the control), and
        // the tooltip says why it does nothing.
        let (_dir, app) = crate::fixtures::test_app();
        assert!(app.fixture.is_some());
        assert!(focus_gate_row(&app, true, "help").is_some());
        assert_eq!(
            crate::focus_status::DEMO_TOOLTIP,
            "Settings can't be changed in demo mode."
        );
    }

    #[test]
    fn the_focus_row_notes_unreadable_and_is_marked_after_the_pill() {
        use stat_tracker::ocr_status::FocusGateStatus;
        let (_dir, mut app) = crate::fixtures::test_app();
        app.fixture = None;
        app.daemon.pid = Some(41);
        app.game_running = true;
        let t0 = std::time::Instant::now();
        assert!(!focus_note_shown(&app));
        // The note follows the debounced top bar value: 5 s of unreadable.
        app.page_state
            .focus
            .observe(Some(FocusGateStatus::Unreadable), t0);
        assert!(!focus_note_shown(&app));
        app.page_state.focus.observe(
            Some(FocusGateStatus::Unreadable),
            t0 + crate::focus_status::UNREADABLE_AFTER,
        );
        assert!(focus_note_shown(&app));
        // Only while a game runs: a value left from the last game is not shown.
        app.game_running = false;
        assert!(!focus_note_shown(&app));
        app.game_running = true;
        app.daemon.pid = None;
        assert!(!focus_note_shown(&app), "no tracker running");
        app.daemon.pid = Some(41);
        app.page_state.focus.observe(Some(FocusGateStatus::Off), t0);
        assert!(!focus_note_shown(&app), "Off stops the warning");
        // The pill marks the row for a moment.
        assert!(!app.page_state.focus_row_marked(t0));
        app.page_state.show_focus_row(t0);
        assert!(app.page_state.focus_row_marked(t0));
        assert!(
            !app.page_state
                .focus_row_marked(t0 + crate::focus_status::HIGHLIGHT_FOR)
        );
    }

    /// Extra names show after the main one, "öster" beside "Öster" is one
    /// name, and "+ Add name" adds another instead of replacing.
    #[test]
    fn extra_names_show_and_a_new_name_joins_them() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.settings.player_name = "Öster".into();
        app.saved_config.player_names = vec!["öster".into(), "Alt".into()];
        assert_eq!(shown_names(&app), ["Öster", "Alt"]);
        assert!(add_name_shown(&app));
        assert_eq!(
            name_added(&mut app, "Other".into()),
            Some(ConfigEdit::AddPlayerName("Other".into()))
        );
        assert_eq!(app.settings.player_name, "Öster", "the main name stays");
        app.settings.player_name.clear();
        assert_eq!(name_added(&mut app, "New".into()), None);
        assert_eq!(
            app.settings.player_name, "New",
            "an empty main name is filled"
        );
    }

    #[test]
    fn names_match_in_any_case_unicode_too() {
        assert!(same_name("Öster", "öster"));
        assert!(same_name("FrozenTear", "frozentear#1234"));
        assert!(!same_name("Main", "Alt"));
        let mut c = Config {
            player_name: Some("Öster".into()),
            ..Config::default()
        };
        ConfigEdit::AddPlayerName("öster".into()).apply(&mut c);
        ConfigEdit::AddPlayerName(" ÖSTER ".into()).apply(&mut c);
        assert!(c.player_names.is_empty(), "{:?}", c.player_names);
        c.player_names = vec!["ÄLT".into(), "Third".into()];
        ConfigEdit::RemovePlayerNames(vec!["ält".into()]).apply(&mut c);
        assert_eq!(c.player_names, ["Third"]);
    }

    #[test]
    fn removing_the_first_name_also_drops_its_other_spellings() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.settings.player_name = "Main".into();
        app.saved_config.player_names = vec!["main".into()];
        // The only name: cleared, and "main" goes with it.
        assert_eq!(
            name_removed(&mut app, 0),
            Some(ConfigEdit::RemovePlayerNames(vec!["Main".into()]))
        );
        assert_eq!(app.settings.player_name, "");
        let mut c = app.saved_config.clone();
        ConfigEdit::RemovePlayerNames(vec!["Main".into()]).apply(&mut c);
        assert!(c.player_names.is_empty());
        app.saved_config = c;
        assert_eq!(name_removed(&mut app, 0), None, "no names left to remove");
    }

    #[test]
    fn new_key_edits_write_their_fields() {
        let mut c = Config::default();
        ConfigEdit::LiveUpload(false).apply(&mut c);
        ConfigEdit::RankReader(true).apply(&mut c);
        ConfigEdit::ShareStatPictures(true).apply(&mut c);
        ConfigEdit::ShareReportPictures(true).apply(&mut c);
        ConfigEdit::CaptureOnlyOnScreen(false).apply(&mut c);
        assert!(!c.live_upload && c.rank_reader && c.share_stat_pictures);
        assert!(c.share_report_pictures && !c.capture_only_on_screen);
        c.player_name = Some("Main".into());
        ConfigEdit::AddPlayerName("Alt".into()).apply(&mut c);
        ConfigEdit::AddPlayerName("main".into()).apply(&mut c);
        ConfigEdit::AddPlayerName(" alt ".into()).apply(&mut c);
        assert_eq!(c.player_names, ["Alt"]);
    }

    #[test]
    fn an_added_name_is_cleaned_by_the_core_rule() {
        let mut c = Config {
            player_name: Some("Main".into()),
            ..Config::default()
        };
        // Under 3 characters, or only a tag: not a name.
        ConfigEdit::AddPlayerName("Ab".into()).apply(&mut c);
        ConfigEdit::AddPlayerName(" #2112 ".into()).apply(&mut c);
        assert!(c.player_names.is_empty(), "{:?}", c.player_names);
        // A BattleTag suffix is dropped before it is saved.
        ConfigEdit::AddPlayerName("Alt#1234".into()).apply(&mut c);
        assert_eq!(c.player_names, ["Alt"]);
        // The same name with another tag or case is a repeat.
        ConfigEdit::AddPlayerName("ALT#99".into()).apply(&mut c);
        ConfigEdit::AddPlayerName("main#4321".into()).apply(&mut c);
        assert_eq!(c.player_names, ["Alt"]);
    }

    #[test]
    fn not_connected_points_to_website_connection_once() {
        assert_eq!(not_connected_note(false), Some(UPLOAD_NO_SITE));
        assert_eq!(
            not_connected_note(true),
            None,
            "the refusal line points there"
        );
    }

    #[test]
    fn upload_needs_a_site_and_a_live_app() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.saved_config.sync = None;
        assert_eq!(
            upload_state(&app, false),
            UploadState {
                on: false,
                enabled: false,
                note: Some(UPLOAD_NO_SITE),
            }
        );
        app.saved_config.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "tok".into(),
        });
        assert_eq!(upload_state(&app, false).note, None);
        assert!(upload_state(&app, false).enabled);
        assert!(!upload_state(&app, true).enabled, "a demo changes nothing");
        app.saved_config.live_upload = false;
        assert_eq!(upload_state(&app, false).note, Some(UPLOAD_OFF_NOTE));
    }

    #[test]
    fn adding_and_removing_names_writes_the_file() {
        let (dir, mut app) = live_app();
        app.settings.player_name.clear();
        let _ = app.update(page(Msg::ChipDraft(
            ChipList::Names,
            Some("Main#2112".into()),
        )));
        let _ = app.update(page(Msg::ChipAdd));
        assert_eq!(on_disk(&dir).player_name.as_deref(), Some("Main"));
        // Another name joins the extra names; the same name in other case
        // is not added twice.
        for typed in ["Other", "OTHER"] {
            let _ = app.update(page(Msg::ChipDraft(ChipList::Names, Some(typed.into()))));
            let _ = app.update(page(Msg::ChipAdd));
        }
        let saved = on_disk(&dir);
        assert_eq!(saved.player_name.as_deref(), Some("Main"));
        assert_eq!(saved.player_names, ["Other"]);
        assert_eq!(saved.own_player_names(), ["Main", "Other"]);
        // Removing the main name moves the next one up.
        let _ = app.update(page(Msg::ChipRemove(ChipList::Names, 0)));
        let saved = on_disk(&dir);
        assert_eq!(saved.player_name.as_deref(), Some("Other"));
        assert!(saved.player_names.is_empty());
    }
}
