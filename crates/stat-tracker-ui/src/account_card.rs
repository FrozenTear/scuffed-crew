//! Pieces of the 0.6 Settings everyday cards after spec p6: the Website
//! account's signed-in line with Sign out, its "What leaves your PC ›"
//! link, and the Updates card's "Up to date. Checked HH:MM." line.

use iced::widget::{Row, button, column, container, row, space, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::{Message, TrackerApp};
use stat_tracker::sync::StatsVisibility;

use crate::form_widgets::hint;
use crate::settings_open::{OpenMsg, sync_key};
use crate::settings_rows::link;
use crate::settings_update::Msg;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META, TEXT, TEXT_2,
    TEXT_3,
};

/// Size of the stats visibility line (spec: 12 px).
const VISIBILITY_SIZE: f32 = 12.0;
/// The line's widest measure, about 72 characters at 12 px.
const VISIBILITY_MEASURE: f32 = 520.0;

/// The link under the Sharing rows (spec p6).
pub(crate) const WHAT_LEAVES_LINK: &str = "What leaves your PC ›";

fn page(msg: OpenMsg) -> Message {
    Message::SettingsPage(Msg::Open(msg))
}

/// The connected state: "Signed in as <name>" once the website named the
/// saved token, else "Connected to <host>", then the host under it and
/// Sign out on the right. `mark` is the check, or the warning while the
/// website refuses the token. A token set outside the app cannot be
/// signed out here, and a demo signs nothing out.
pub(crate) fn signed_in<'a>(
    app: &'a TrackerApp,
    host: String,
    mark: Element<'a, Message>,
    locked: bool,
) -> Element<'a, Message> {
    let name =
        sync_key(app).and_then(|key| app.page_state.opened.account_name(&key).map(str::to_string));
    let (title, sub) = match name {
        Some(name) => (format!("Signed in as {name}"), Some(host)),
        None => (format!("Connected to {host}"), None),
    };
    let mut lines = column![
        row![
            mark,
            text(title).size(SIZE_BODY).font(FONT_SEMIBOLD).color(TEXT),
        ]
        .spacing(8)
        .align_y(Alignment::Center)
    ]
    .spacing(2);
    if let Some(sub) = sub {
        lines = lines.push(text(sub).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_3));
    }
    let mut sign_out = button(
        text("Sign out")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([7, 14]))
    .style(theme::ghost_btn());
    if app.fixture.is_none() && !locked {
        sign_out = sign_out.on_press(page(OpenMsg::SignOut));
    }
    Row::new()
        .spacing(12)
        .align_y(Alignment::Center)
        .push(lines)
        .push(space().width(Fill))
        .push(sign_out)
        .into()
}

/// What each stats visibility reads as under "Signed in as". The spec's
/// copy, every line ending in the same sentence. Plain text: the site has
/// not said where the switch lives, so there is no link.
pub(crate) fn visibility_copy(visibility: StatsVisibility) -> &'static str {
    match visibility {
        StatsVisibility::Public => {
            "Your stats are shown on your profile and leaderboards, also to visitors who aren't signed in. You can change this in your site settings."
        }
        StatsVisibility::Members => {
            "Your stats are shown to signed-in crew members only. You can change this in your site settings."
        }
        StatsVisibility::Private => {
            "Your stats are hidden from everyone except officers. You can change this in your site settings."
        }
    }
}

/// The visibility to show for the saved website, or `None` to show no line:
/// signed out or uploads off, no answer yet, a failed or unknown answer,
/// and an answer read for another URL or token all hide it.
pub(crate) fn shown_visibility(app: &TrackerApp) -> Option<StatsVisibility> {
    let key = sync_key(app)?;
    app.page_state.opened.stats_visibility(&key)
}

/// The stats visibility line under the account row, or nothing. A grey
/// 12 px read-only line, no icon, no link.
pub(crate) fn visibility_line(app: &TrackerApp) -> Option<Element<'static, Message>> {
    let visibility = shown_visibility(app)?;
    Some(
        container(
            text(visibility_copy(visibility))
                .size(VISIBILITY_SIZE)
                .font(FONT_MEDIUM)
                .color(TEXT_2),
        )
        .max_width(VISIBILITY_MEASURE)
        .into(),
    )
}

/// "What leaves your PC ›": opens [`crate::what_leaves`].
pub(crate) fn what_leaves_link() -> Element<'static, Message> {
    link(WHAT_LEAVES_LINK, Some(page(OpenMsg::WhatLeaves(true))))
}

/// Under Version when no newer one is out: "Up to date. Checked HH:MM."
/// with the time in bold, from the last check that reached GitHub (at
/// start or Check now). Before any answer it says no check has answered.
pub(crate) fn up_to_date_line(checked_at: Option<&str>) -> Element<'static, Message> {
    match checked_at {
        Some(at) => row![
            text("Up to date. Checked")
                .size(SIZE_LABEL)
                .font(FONT_MEDIUM)
                .color(TEXT_3),
            text(format!("{at}."))
                .size(SIZE_LABEL)
                .font(FONT_BOLD)
                .color(TEXT_2),
        ]
        .spacing(4)
        .into(),
        None => hint("Not checked yet."),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use stat_tracker::config::SyncConfig;

    const ALL: [StatsVisibility; 3] = [
        StatsVisibility::Public,
        StatsVisibility::Members,
        StatsVisibility::Private,
    ];

    fn signed_in_app() -> (tempfile::TempDir, TrackerApp) {
        let (dir, mut app) = crate::fixtures::test_app();
        app.fixture = None;
        app.saved_config.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "tok".into(),
        });
        (dir, app)
    }

    fn key() -> (String, String) {
        ("https://crew.example".into(), "tok".into())
    }

    fn remember(app: &mut TrackerApp, value: Option<StatsVisibility>) {
        let _ = crate::settings_open::update(
            app,
            OpenMsg::VisibilityChecked { key: key(), value },
            |_| iced::Task::none(),
        );
    }

    /// The copy is the spec's: each value its own line, all ending in the
    /// same sentence, with no dashes and no link.
    #[test]
    fn each_value_has_its_spec_line() {
        assert_eq!(
            visibility_copy(StatsVisibility::Public),
            "Your stats are shown on your profile and leaderboards, also to visitors who aren't signed in. You can change this in your site settings."
        );
        assert_eq!(
            visibility_copy(StatsVisibility::Members),
            "Your stats are shown to signed-in crew members only. You can change this in your site settings."
        );
        assert_eq!(
            visibility_copy(StatsVisibility::Private),
            "Your stats are hidden from everyone except officers. You can change this in your site settings."
        );
        for v in ALL {
            let line = visibility_copy(v);
            assert!(line.ends_with(" You can change this in your site settings."));
            assert!(
                !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                "{line}"
            );
        }
    }

    #[test]
    fn a_read_value_shows_for_a_signed_in_website() {
        for v in ALL {
            let (_dir, mut app) = signed_in_app();
            app.page_state.opened.mark_asking_visibility(key());
            remember(&mut app, Some(v));
            assert_eq!(shown_visibility(&app), Some(v));
            assert!(visibility_line(&app).is_some());
        }
    }

    #[test]
    fn nothing_shows_before_an_answer() {
        let (_dir, app) = signed_in_app();
        assert_eq!(shown_visibility(&app), None);
        assert!(visibility_line(&app).is_none());
    }

    #[test]
    fn an_unknown_value_or_a_failed_read_hides_the_line() {
        let (_dir, mut app) = signed_in_app();
        app.page_state.opened.mark_asking_visibility(key());
        remember(&mut app, Some(StatsVisibility::Members));
        assert!(visibility_line(&app).is_some());
        // The parse maps an unknown value and a failed read to the same `None`.
        app.page_state.opened.mark_asking_visibility(key());
        remember(&mut app, None);
        assert_eq!(shown_visibility(&app), None);
        assert!(visibility_line(&app).is_none());
    }

    #[test]
    fn signed_out_or_uploads_off_hides_a_cached_value() {
        let (_dir, mut app) = signed_in_app();
        app.page_state.opened.mark_asking_visibility(key());
        remember(&mut app, Some(StatsVisibility::Public));
        assert!(shown_visibility(&app).is_some());

        app.saved_config.sync = None;
        assert_eq!(shown_visibility(&app), None, "signed out");

        app.saved_config.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "".into(),
        });
        assert_eq!(shown_visibility(&app), None, "blank token");
    }

    #[test]
    fn a_value_read_for_another_token_is_not_shown() {
        let (_dir, mut app) = signed_in_app();
        app.page_state.opened.mark_asking_visibility(key());
        remember(&mut app, Some(StatsVisibility::Private));
        app.saved_config.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "other".into(),
        });
        assert_eq!(shown_visibility(&app), None);
    }

    #[test]
    fn sign_out_forgets_the_value() {
        let (_dir, mut app) = signed_in_app();
        app.page_state.opened.mark_asking_visibility(key());
        remember(&mut app, Some(StatsVisibility::Public));
        let _ = crate::settings_open::update(&mut app, OpenMsg::SignOut, |_| iced::Task::none());
        assert_eq!(app.page_state.opened.stats_visibility(&key()), None);
    }

    #[test]
    fn a_demo_asks_nothing() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert!(app.fixture.is_some());
        app.saved_config.sync = Some(SyncConfig {
            server_url: "https://crew.example".into(),
            token: "tok".into(),
        });
        let _ = crate::settings_open::on_open(&mut app);
        assert_eq!(shown_visibility(&app), None);
    }
}
