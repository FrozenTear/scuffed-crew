//! The 0.6 Settings rows that write the config keys added for 0.6: In-game
//! names as a list (`player_name` first, then `player_names`), Upload
//! games after each match (`live_upload`), the two Sharing switches
//! (`rank_reader`, `share_stat_pictures`), Report pictures
//! (`share_report_pictures`) and Only capture while Overwatch is on screen
//! (`capture_only_on_screen`). Each applies at once. The tracker will read
//! them when it starts, so each is tagged After restart, except Report
//! pictures, which only this app will read.
//!
//! Rows whose key nothing reads yet are not shown
//! ([`crate::settings_model::HIDDEN_KEYS`]): their keys still load and save
//! as before, and In-game names keeps one name while `player_names` is
//! hidden.

use iced::Element;
use iced::widget::Row;

use crate::app::{Message, TrackerApp};
use crate::form_widgets::hint;
use crate::settings::SettingsField;
use crate::settings_model::{ChipList, ConfigEdit, row_shown, same_name};
use crate::settings_rows::{
    Segment, Tag, add_chip, chip, chip_control, segmented, setting_row, switch, title_line,
};
use crate::settings_update::Msg;

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
        title_line("In-game names", false, &[Tag::AfterRestart]),
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

/// The two Sharing switches under Website account, both starting off,
/// less the hidden ones. Empty while both are hidden.
pub(crate) fn sharing_rows<'a>(
    app: &'a TrackerApp,
    demo: bool,
    rank_help: &'static str,
    pictures_help: &'static str,
) -> Vec<Element<'a, Message>> {
    let c = &app.saved_config;
    let mut rows = Vec::new();
    if row_shown("rank_reader") {
        rows.push(setting_row(
            title_line("Read my rank after Comp games", false, &[Tag::AfterRestart]),
            vec![hint(rank_help)],
            edit_switch(c.rank_reader, demo, ConfigEdit::RankReader),
        ));
    }
    if row_shown("share_stat_pictures") {
        rows.push(setting_row(
            title_line("Share stat pictures", false, &[Tag::AfterRestart]),
            vec![hint(pictures_help)],
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
            vec![hint(help)],
            edit_switch(
                app.saved_config.share_report_pictures,
                demo,
                ConfigEdit::ShareReportPictures,
            ),
        )
    })
}

/// Only capture while Overwatch is on screen: Auto or Off (spec p7), or
/// `None` while hidden.
pub(crate) fn focus_gate_row<'a>(
    app: &'a TrackerApp,
    demo: bool,
    help: &'static str,
) -> Option<Element<'a, Message>> {
    if !row_shown("capture_only_on_screen") {
        return None;
    }
    let auto = app.saved_config.capture_only_on_screen;
    let pick = |on: bool| {
        (!demo).then_some(Message::SettingsPage(Msg::Edit(
            ConfigEdit::CaptureOnlyOnScreen(on),
        )))
    };
    Some(setting_row(
        title_line(
            "Only capture while Overwatch is on screen",
            !auto,
            &[Tag::New, Tag::AfterRestart],
        ),
        vec![hint(help)],
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
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::settings_model::HIDDEN_KEYS;
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

    #[test]
    fn rows_nothing_reads_yet_are_hidden() {
        for key in [
            "rank_reader",
            "share_stat_pictures",
            "share_report_pictures",
            "capture_only_on_screen",
            "live_upload",
            "player_names",
        ] {
            assert!(HIDDEN_KEYS.contains(&key), "{key}");
            assert!(!row_shown(key), "{key}");
        }
        let (_dir, app) = crate::fixtures::test_app();
        assert!(upload_row(&app, false, "help", true).is_none());
        assert!(sharing_rows(&app, false, "a", "b").is_empty());
        assert!(report_pictures_row(&app, false, "c").is_none());
        assert!(focus_gate_row(&app, false, "d").is_none());
    }

    #[test]
    fn hidden_extra_names_never_show_and_one_name_is_kept() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.settings.player_name = "Main".into();
        app.saved_config.player_names = vec!["Alt".into()];
        assert_eq!(shown_names(&app), ["Main"], "player_names is hidden");
        assert!(
            !add_name_shown(&app),
            "one name: + Add name waits for an empty slot"
        );
        assert_eq!(name_added(&mut app, "Other".into()), None);
        assert_eq!(app.settings.player_name, "Other", "a new name replaces it");
        app.settings.player_name.clear();
        assert!(add_name_shown(&app));
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
        // Extra names are hidden, so another name replaces it.
        let _ = app.update(page(Msg::ChipDraft(ChipList::Names, Some("Other".into()))));
        let _ = app.update(page(Msg::ChipAdd));
        let saved = on_disk(&dir);
        assert_eq!(saved.player_name.as_deref(), Some("Other"));
        assert!(saved.player_names.is_empty());
        // Removing the only name clears it.
        let _ = app.update(page(Msg::ChipRemove(ChipList::Names, 0)));
        assert_eq!(on_disk(&dir).player_name, None);
    }
}
