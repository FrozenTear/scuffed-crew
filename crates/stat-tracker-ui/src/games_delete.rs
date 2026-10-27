//! Delete game in the open Games row (UX Spec, games-delete.md): the
//! outlined button at the end of the actions, the inline confirm strip
//! that replaces them, and the "Game deleted." line over the table. The
//! keys and the focus are [`crate::games_table::update`]'s; iced 0.14
//! buttons take no keyboard focus, so the ring here is drawn from
//! [`Focus`].

use iced::widget::{button, column, container, image, row, space, text};
use iced::{Alignment, Border, Color, Element, Fill, Length, Padding, Theme};

use crate::app::Message;
use crate::games_rows::Focus;
use crate::games_table::Msg;
use crate::icons::{self, Icon};
use crate::theme::{
    self, ACCENT, BG, BORDER, DANGER, FONT_BOLD, FONT_SEMIBOLD, SIZE_META, TEXT, TEXT_2, TEXT_3,
};

/// Every button here is 32px tall (spec).
const BTN_H: f32 = 32.0;
/// The 2px focus ring, 2px off the button.
const RING: f32 = 2.0;
const RING_GAP: f32 = 2.0;
const SMALL: f32 = 12.0;

/// The confirm strip's body for a game that is on the site (`uploaded`)
/// or only on this computer (Waiting to upload, Upload rejected).
pub fn body(uploaded: bool) -> &'static str {
    if uploaded {
        "It will also be removed from the site. Your stats and averages update without it."
    } else {
        "It's only on this computer, so this removes it for good. Your stats and averages update without it."
    }
}

fn games_msg(m: Msg) -> Message {
    Message::Games(m)
}

/// The accent ring around a focused button. Unfocused buttons keep the
/// same room, so focus never moves anything.
fn ring(inner: Element<'static, Message>, on: bool) -> Element<'static, Message> {
    let color = if on { ACCENT } else { Color::TRANSPARENT };
    container(inner)
        .padding(RING + RING_GAP)
        .style(move |_: &Theme| container::Style {
            border: Border {
                color,
                width: RING,
                radius: (8.0 + RING + RING_GAP).into(),
            },
            ..container::Style::default()
        })
        .into()
}

fn label(icon: Option<Icon>, words: &'static str, color: Color) -> Element<'static, Message> {
    let mut r = row![].spacing(8).align_y(Alignment::Center);
    if let Some(icon) = icon {
        r = r.push(image(icons::handle(icon)).width(16).height(16));
    }
    r.push(text(words).size(SIZE_META).font(FONT_BOLD).color(color))
        .into()
}

fn sized(
    content: Element<'static, Message>,
    style: impl Fn(&Theme, button::Status) -> button::Style + 'static,
    msg: Message,
) -> Element<'static, Message> {
    button(container(content).height(Fill).align_y(Alignment::Center))
        .height(BTN_H)
        .padding(Padding::from([0, 12]))
        .style(style)
        .on_press(msg)
        .into()
}

/// Delete game: 1px loss-colour outline, no fill, trash and label in the
/// loss colour.
pub fn button_for(sid: &str, focused: bool) -> Element<'static, Message> {
    let b = sized(
        label(Some(Icon::Trash), "Delete game", DANGER),
        |_: &Theme, status| button::Style {
            background: theme::hover_tint(status),
            text_color: DANGER,
            border: Border {
                color: DANGER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        },
        games_msg(Msg::AskDelete(sid.to_string())),
    );
    ring(b, focused)
}

/// "Delete this game?", the body for the upload state, Cancel (focused
/// first) and the solid Delete game, and the "Esc cancels" hint.
pub fn strip(sid: &str, uploaded: bool, focus: &Focus) -> Element<'static, Message> {
    let cancel = sized(
        label(None, "Cancel", TEXT),
        |_: &Theme, status| button::Style {
            background: theme::hover_tint(status),
            text_color: TEXT,
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        },
        games_msg(Msg::CancelDelete),
    );
    let confirm = sized(
        label(Some(Icon::TrashInk), "Delete game", BG),
        |_: &Theme, _status| button::Style {
            background: Some(DANGER.into()),
            text_color: BG,
            border: Border {
                radius: 8.0.into(),
                ..Border::default()
            },
            ..button::Style::default()
        },
        games_msg(Msg::Delete(sid.to_string())),
    );
    let buttons = row![
        space().width(Fill),
        ring(cancel, *focus == Focus::Cancel),
        ring(confirm, *focus == Focus::ConfirmDelete),
    ]
    // The rings' room makes the spec's 8px gap between the buttons.
    .spacing(0)
    .align_y(Alignment::Center);
    let hint = row![
        space().width(Fill),
        crate::games_panel::key_cap("Esc"),
        text("cancels").size(SMALL).color(TEXT_3),
    ]
    .spacing(6)
    .align_y(Alignment::Center);
    container(
        column![
            text("Delete this game?")
                .size(SIZE_META)
                .font(FONT_SEMIBOLD)
                .color(TEXT),
            text(body(uploaded)).size(SMALL).color(TEXT_2),
            buttons,
            hint,
        ]
        .spacing(8),
    )
    .padding(12)
    .width(Fill)
    .style(|_: &Theme| container::Style {
        border: Border {
            color: DANGER,
            width: 1.0,
            radius: 8.0.into(),
        },
        ..container::Style::default()
    })
    .into()
}

/// The neutral "Game deleted." line at the top of the table (no undo).
pub fn deleted_line() -> Element<'static, Message> {
    container(
        row![
            image(icons::handle(Icon::Tick)).width(16).height(16),
            text("Game deleted.").size(SMALL).color(TEXT_2),
        ]
        .spacing(8)
        .align_y(Alignment::Center),
    )
    .padding(Padding::from([8, 16]))
    .width(Length::Fill)
    .into()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_strip_says_where_the_game_is_removed_from() {
        assert_eq!(
            body(false),
            "It's only on this computer, so this removes it for good. Your stats and averages update without it."
        );
        assert_eq!(
            body(true),
            "It will also be removed from the site. Your stats and averages update without it."
        );
    }

    fn app() -> (tempfile::TempDir, crate::app::TrackerApp) {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = crate::fixtures::test_cli(dir.path());
        cli.fixture = Some(crate::cli::FixtureKind::Games);
        let mut app = crate::app::TrackerApp::new(cli).0;
        app.nav_style = crate::nav::NavStyle::TopTabs;
        app.expanded = Some(crate::fixtures_games::NORMAL.into());
        (dir, app)
    }

    fn send(app: &mut crate::app::TrackerApp, msg: Msg) {
        let _ = crate::games_table::update(app, msg);
    }

    /// Tab in an open row reaches Delete game, Enter on it opens the strip
    /// on Cancel, and Tab then reaches the solid Delete game. Esc closes
    /// the strip back onto Delete game, and only a second Esc closes the
    /// row.
    #[test]
    fn the_strip_takes_esc_before_the_row_does() {
        let (_dir, mut app) = app();
        let sid = crate::fixtures_games::NORMAL.to_string();
        send(&mut app, Msg::Tab);
        assert_eq!(
            app.games_tab.focus,
            Focus::DeleteButton,
            "Tab reaches Delete"
        );
        send(&mut app, Msg::Enter);
        assert_eq!(app.confirm_delete.as_deref(), Some(sid.as_str()));
        assert_eq!(app.games_tab.focus, Focus::Cancel);
        send(&mut app, Msg::Tab);
        assert_eq!(app.games_tab.focus, Focus::ConfirmDelete);
        send(&mut app, Msg::Tab);
        assert_eq!(app.games_tab.focus, Focus::Cancel);
        send(&mut app, Msg::Escape);
        assert_eq!(app.confirm_delete, None);
        assert_eq!(app.games_tab.focus, Focus::DeleteButton);
        assert_eq!(app.expanded.as_deref(), Some(sid.as_str()));
        send(&mut app, Msg::Space);
        assert_eq!(app.games_tab.focus, Focus::Cancel, "Space opens it too");
        send(&mut app, Msg::Enter);
        assert_eq!(app.confirm_delete, None, "Enter on Cancel cancels");
        send(&mut app, Msg::Escape);
        assert_eq!(app.expanded, None);
        assert_eq!(app.games_tab.focus, Focus::None);
    }

    /// Delete game queues the delete (its tombstone goes on the next
    /// sync), drops the row, puts the focus on a neighbour row and the
    /// "Game deleted." line up; opening a row clears both.
    #[test]
    fn delete_drops_the_row_and_moves_the_focus() {
        let (_dir, mut app) = app();
        let sid = crate::fixtures_games::NORMAL.to_string();
        let before = app.games.len();
        send(&mut app, Msg::AskDelete(sid.clone()));
        send(&mut app, Msg::Tab);
        send(&mut app, Msg::Enter);
        assert_eq!(app.games.len(), before - 1);
        assert!(app.games.iter().all(|g| g.session_id != sid));
        assert!(
            crate::commands::queued(&app.data_dir).iter().any(|c| matches!(
                c,
                stat_tracker::storage::StoreCommand::DeleteSession { session_id } if *session_id == sid
            ))
        );
        assert_eq!(
            (app.expanded.clone(), app.confirm_delete.clone()),
            (None, None)
        );
        assert!(app.games_tab.deleted_note);
        let Focus::Row(next) = app.games_tab.focus.clone() else {
            panic!("focus on a row, got {:?}", app.games_tab.focus);
        };
        assert!(app.games.iter().any(|g| g.session_id == next));
        assert!(
            crate::games_panel::listens(&app),
            "Enter opens the focused row"
        );
        send(&mut app, Msg::Enter);
        assert_eq!(app.expanded.as_deref(), Some(next.as_str()));
        assert!(!app.games_tab.deleted_note);
        assert_eq!(app.games_tab.focus, Focus::None);
    }
}
