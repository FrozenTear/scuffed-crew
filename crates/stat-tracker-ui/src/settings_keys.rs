//! The 0.6 Settings rows that write the config keys added for 0.6 (#554):
//! In-game names as a list (`player_name` first, then `player_names`),
//! Upload games after each match (`live_upload`), the two Sharing
//! switches (`rank_reader`, `share_stat_pictures`), Report pictures
//! (`share_report_pictures`) and Only capture while Overwatch is on screen
//! (`capture_only_on_screen`). Each applies at once and is tagged After
//! restart, since the tracker reads them when it starts.

use iced::Element;
use iced::widget::Row;

use crate::app::{Message, TrackerApp};
use crate::form_widgets::hint;
use crate::settings_model::{ChipList, ConfigEdit};
use crate::settings_rows::{
    Segment, Tag, add_chip, chip, chip_control, segmented, setting_row, switch, title_line,
};
use crate::settings_update::Msg;

/// Help under In-game names (spec).
const NAMES_HELP: &str = "As shown in-game, without the #1234.";
/// Under Upload games while it is off.
const UPLOAD_OFF_NOTE: &str = "Games stay on this computer and upload once this is on again.";
/// Under Upload games while no website is set.
const UPLOAD_NO_SITE: &str = "Add the website URL and token under Advanced, Website connection.";

/// A switch that writes `edit(on)`, or a disabled one in a demo.
fn edit_switch<'a>(on: bool, demo: bool, edit: fn(bool) -> ConfigEdit) -> Element<'a, Message> {
    switch(
        on,
        (!demo).then(|| {
            Box::new(move |v| Message::SettingsPage(Msg::Edit(edit(v))))
                as Box<dyn Fn(bool) -> Message + 'a>
        }),
    )
}

/// The names the In-game names row shows: the form's `player_name`
/// first, then each saved `player_names` entry it does not repeat.
pub(crate) fn shown_names(app: &TrackerApp) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let primary = app.settings.player_name.trim();
    if !primary.is_empty() {
        out.push(primary.to_string());
    }
    for name in &app.saved_config.player_names {
        let name = name.trim();
        if !name.is_empty() && !out.iter().any(|n| n.eq_ignore_ascii_case(name)) {
            out.push(name.to_string());
        }
    }
    out
}

/// "+ Add name" took `name`: it fills an empty `player_name` (the name the
/// website lookup and the Tab reader use), else it joins `player_names`.
pub(crate) fn name_added(app: &mut TrackerApp, name: String) -> Option<ConfigEdit> {
    if app.settings.player_name.trim().is_empty() {
        app.settings
            .set_text(crate::settings::SettingsField::PlayerName, name);
        None
    } else {
        Some(ConfigEdit::AddPlayerName(name))
    }
}

/// The "×" on name `index` of [`shown_names`]. Removing the first name
/// moves the next one into `player_name`, so it is never left empty while
/// other names remain.
pub(crate) fn name_removed(app: &mut TrackerApp, index: usize) -> Option<ConfigEdit> {
    use crate::settings::SettingsField::PlayerName;
    let names = shown_names(app);
    let name = names.get(index)?.clone();
    let primary = !app.settings.player_name.trim().is_empty();
    if index == 0 && primary {
        let next = names.get(1).cloned();
        app.settings
            .set_text(PlayerName, next.clone().unwrap_or_default());
        next.map(ConfigEdit::RemovePlayerName)
    } else {
        Some(ConfigEdit::RemovePlayerName(name))
    }
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
    chips = chips.push(add_chip(app, ChipList::Names, "+ Add name", "Name", demo));
    setting_row(
        title_line("In-game names", false, &[Tag::AfterRestart]),
        vec![hint(NAMES_HELP)],
        chip_control(chips),
    )
}

/// Upload games after each match: live while a website and token are
/// saved, off and disabled without one.
pub(crate) fn upload_row<'a>(
    app: &'a TrackerApp,
    demo: bool,
    help: &'static str,
    show_note: bool,
) -> Element<'a, Message> {
    let site = app.sync_on();
    let on = site && app.saved_config.live_upload;
    let mut lines = vec![hint(help)];
    if show_note {
        if !site {
            lines.push(hint(UPLOAD_NO_SITE));
        } else if !on {
            lines.push(hint(UPLOAD_OFF_NOTE));
        }
    }
    let control = if site {
        edit_switch(on, demo, ConfigEdit::LiveUpload)
    } else {
        switch(false, None)
    };
    setting_row(
        title_line("Upload games after each match", false, &[Tag::AfterRestart]),
        lines,
        control,
    )
}

/// The two Sharing switches under Website account. Both start off.
pub(crate) fn sharing_rows<'a>(
    app: &'a TrackerApp,
    demo: bool,
    rank_help: &'static str,
    pictures_help: &'static str,
) -> Vec<Element<'a, Message>> {
    let c = &app.saved_config;
    vec![
        setting_row(
            title_line("Read my rank after Comp games", false, &[Tag::AfterRestart]),
            vec![hint(rank_help)],
            edit_switch(c.rank_reader, demo, ConfigEdit::RankReader),
        ),
        setting_row(
            title_line("Share stat pictures", false, &[Tag::AfterRestart]),
            vec![hint(pictures_help)],
            edit_switch(c.share_stat_pictures, demo, ConfigEdit::ShareStatPictures),
        ),
    ]
}

/// Report pictures under READING (spec p7).
pub(crate) fn report_pictures_row<'a>(
    app: &'a TrackerApp,
    demo: bool,
    help: &'static str,
) -> Element<'a, Message> {
    setting_row(
        title_line("Report pictures", false, &[Tag::New, Tag::AfterRestart]),
        vec![hint(help)],
        edit_switch(
            app.saved_config.share_report_pictures,
            demo,
            ConfigEdit::ShareReportPictures,
        ),
    )
}

/// Only capture while Overwatch is on screen: Auto or Off (spec p7).
pub(crate) fn focus_gate_row<'a>(
    app: &'a TrackerApp,
    demo: bool,
    help: &'static str,
) -> Element<'a, Message> {
    let auto = app.saved_config.capture_only_on_screen;
    let pick = |on: bool| {
        (!demo).then_some(Message::SettingsPage(Msg::Edit(
            ConfigEdit::CaptureOnlyOnScreen(on),
        )))
    };
    setting_row(
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
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn names_list_the_form_name_first_without_repeats() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.settings.player_name = "FrozenTear".into();
        app.saved_config.player_names = vec!["frozentear".into(), "Alt".into(), " ".into()];
        assert_eq!(shown_names(&app), ["FrozenTear", "Alt"]);
        app.settings.player_name.clear();
        assert_eq!(shown_names(&app), ["frozentear", "Alt"]);
    }

    #[test]
    fn adding_fills_the_first_name_then_the_list() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.settings.player_name.clear();
        assert_eq!(name_added(&mut app, "Main".into()), None);
        assert_eq!(app.settings.player_name, "Main");
        assert_eq!(
            name_added(&mut app, "Alt".into()),
            Some(ConfigEdit::AddPlayerName("Alt".into()))
        );
    }

    #[test]
    fn removing_the_first_name_promotes_the_next() {
        let (_dir, mut app) = crate::fixtures::test_app();
        app.settings.player_name = "Main".into();
        app.saved_config.player_names = vec!["Alt".into(), "Third".into()];
        assert_eq!(
            name_removed(&mut app, 0),
            Some(ConfigEdit::RemovePlayerName("Alt".into()))
        );
        assert_eq!(app.settings.player_name, "Alt");
        assert_eq!(
            name_removed(&mut app, 2),
            Some(ConfigEdit::RemovePlayerName("Third".into()))
        );
        assert_eq!(name_removed(&mut app, 9), None);
    }

    #[test]
    fn new_key_edits_write_their_fields() {
        let mut c = stat_tracker::config::Config::default();
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
        ConfigEdit::RemovePlayerName("ALT".into()).apply(&mut c);
        assert!(c.player_names.is_empty());
    }

    #[test]
    fn the_rows_draw_in_a_demo() {
        let (_dir, app) = crate::fixtures::test_app();
        let _ = names_row(&app, true);
        let _ = upload_row(&app, true, "help", true);
        let _ = sharing_rows(&app, true, "a", "b");
        let _ = report_pictures_row(&app, true, "c");
        let _ = focus_gate_row(&app, true, "d");
    }
}
