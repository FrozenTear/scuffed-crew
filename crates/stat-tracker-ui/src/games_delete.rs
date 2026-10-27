//! Delete game in the open Games row (UX Spec, games-delete.md): the
//! outlined button at the end of the actions, the inline confirm strip
//! that replaces them, and the "Game deleted." line over the table. The
//! keys and the focus are [`crate::games_table::update`]'s; iced 0.14
//! buttons take no keyboard focus, so the ring here is drawn from
//! [`Focus`].

use iced::widget::{button, column, container, image, row, space, text};
use iced::{Alignment, Border, Color, Element, Fill, Length, Padding, Theme};

use crate::app::{Message, TrackerApp};
use crate::games_rows::Focus;
use crate::games_table::Msg;
use crate::icons::{self, Icon};
use crate::model::Game;
use crate::theme::{
    self, ACCENT, BG, BORDER, DANGER, FONT_BOLD, FONT_SEMIBOLD, SIZE_META, TEXT, TEXT_2, TEXT_3,
};

/// Every button here is 32px tall (spec).
const BTN_H: f32 = 32.0;
/// The 2px focus ring, 2px off the button.
const RING: f32 = 2.0;
const RING_GAP: f32 = 2.0;
const SMALL: f32 = 12.0;

/// Where a delete removes the game from, as far as the app can tell.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Uploaded: the site copy goes too.
    Site,
    /// Sync is on but this copy is not the uploaded one (never sent, edited
    /// since, or its re-upload refused). The tombstone still removes any
    /// site copy, so the strip does not promise it is local only.
    Maybe,
    /// Sync is off: only this computer has it.
    Local,
}

/// The strip's reach for an upload state and whether sync is on.
pub fn reach(up: &crate::games_detail::Upload, sync_on: bool) -> Reach {
    match (up, sync_on) {
        (crate::games_detail::Upload::Uploaded, _) => Reach::Site,
        (_, true) => Reach::Maybe,
        (_, false) => Reach::Local,
    }
}

/// The confirm strip's body for where the delete reaches.
pub fn body(reach: Reach) -> &'static str {
    match reach {
        Reach::Site => {
            "It will also be removed from the site. Your stats and averages update without it."
        }
        Reach::Maybe => {
            "If a copy reached the site, it's removed there too. Your stats and averages update without it."
        }
        Reach::Local => {
            "It's only on this computer, so this removes it for good. Your stats and averages update without it."
        }
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
pub fn strip(sid: &str, reach: Reach, focus: &Focus) -> Element<'static, Message> {
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
            text(body(reach)).size(SMALL).color(TEXT_2),
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

/// The game the "Game deleted." line is up for, and whether it was on
/// the site when it was deleted.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Deleted {
    pub sid: String,
    pub uploaded: bool,
}

/// The line after a delete in its pending state.
pub const DELETED_PENDING: &str = "Game deleted. It's removed from the site on the next sync.";
pub const DELETED: &str = "Game deleted.";

/// "Game deleted.", with the next-sync note while the site still has the
/// game: it had uploaded and its tombstone has not synced. Until a
/// snapshot is read after the delete, the tombstone counts as pending; a
/// snapshot that still lists the game means the tracker has not applied
/// the delete yet.
pub fn deleted_copy(app: &TrackerApp) -> &'static str {
    let Some(d) = &app.games_tab.deleted_note else {
        return DELETED;
    };
    let pending = match &app.games_tab.pending_deletes {
        None => true,
        Some(list) => list.contains(&d.sid) || app.games.iter().any(|g| g.session_id == d.sid),
    };
    if d.uploaded && pending {
        DELETED_PENDING
    } else {
        DELETED
    }
}

impl crate::games_rows::GamesTab {
    /// The games of a fresh snapshot, noting its pending deletes.
    pub fn games_from(&mut self, snap: &stat_tracker::storage::Snapshot) -> Vec<Game> {
        self.pending_deletes = Some(snap.pending_deletes.clone());
        crate::snapshot::games_from_snapshot(snap)
    }
}

/// The neutral "Game deleted." line at the top of the table (no undo).
pub fn deleted_line(copy: &'static str) -> Element<'static, Message> {
    container(
        row![
            image(icons::handle(Icon::Tick)).width(16).height(16),
            text(copy).size(SMALL).color(TEXT_2),
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
        use crate::games_detail::Upload;
        assert_eq!(
            body(reach(&Upload::SyncOff, false)),
            "It's only on this computer, so this removes it for good. Your stats and averages update without it."
        );
        assert_eq!(
            body(reach(&Upload::Uploaded, true)),
            "It will also be removed from the site. Your stats and averages update without it."
        );
        // Sync on and not the uploaded copy: no "for good" promise.
        for up in [
            Upload::Waiting,
            Upload::Undecided,
            Upload::Rejected("x".into()),
        ] {
            assert_eq!(
                body(reach(&up, true)),
                "If a copy reached the site, it's removed there too. Your stats and averages update without it."
            );
        }
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
        assert!(app.games_tab.deleted_note.is_some());
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
        assert!(app.games_tab.deleted_note.is_none());
        assert_eq!(app.games_tab.focus, Focus::None);
    }

    /// Delete on the open row opens the confirm on Cancel and deletes
    /// nothing, however often it is pressed.
    #[test]
    fn the_delete_key_opens_the_confirm_on_cancel() {
        let (_dir, mut app) = app();
        let sid = crate::fixtures_games::NORMAL;
        let before = app.games.len();
        for _ in 0..2 {
            send(&mut app, Msg::DeleteKey);
            assert_eq!(app.confirm_delete.as_deref(), Some(sid));
            assert_eq!(app.games_tab.focus, Focus::Cancel);
        }
        assert_eq!(app.games.len(), before);
        assert!(crate::commands::queued(&app.data_dir).is_empty());
        send(&mut app, Msg::Enter);
        assert_eq!(app.confirm_delete, None, "Cancel had the focus");
        assert_eq!(app.games.len(), before);
    }

    /// On a focused row Delete opens the row first.
    #[test]
    fn the_delete_key_opens_a_focused_row() {
        let (_dir, mut app) = app();
        let sid = crate::fixtures_games::UNSURE.to_string();
        app.expanded = None;
        app.games_tab.focus = Focus::Row(sid.clone());
        app.games_tab.deleted_note = Some(Deleted {
            sid: crate::fixtures_games::NORMAL.into(),
            uploaded: true,
        });
        assert!(crate::games_panel::listens(&app));
        send(&mut app, Msg::DeleteKey);
        assert_eq!(app.expanded.as_deref(), Some(sid.as_str()));
        assert_eq!(app.confirm_delete.as_deref(), Some(sid.as_str()));
        assert_eq!(app.games_tab.focus, Focus::Cancel);
        assert!(app.games_tab.deleted_note.is_none());
    }

    /// Nothing open or focused, or the edit form up: Delete does nothing.
    #[test]
    fn the_delete_key_does_nothing_without_a_row_or_over_the_form() {
        let (_dir, mut app) = app();
        app.editing = true;
        send(&mut app, Msg::DeleteKey);
        assert_eq!(app.confirm_delete, None);
        app.editing = false;
        app.expanded = None;
        send(&mut app, Msg::DeleteKey);
        assert_eq!(
            (app.confirm_delete.clone(), app.expanded.clone()),
            (None, None)
        );
        assert_eq!(app.games_tab.focus, Focus::None);
    }

    /// An uploaded game reads as pending until a snapshot shows its
    /// tombstone synced; a game only on this computer reads plain.
    #[test]
    fn the_deleted_line_waits_for_the_tombstone_of_an_uploaded_game() {
        let (_dir, mut app) = app();
        let sid = crate::fixtures_games::NORMAL;
        send(&mut app, Msg::Delete(sid.into()));
        assert!(
            app.games.iter().all(|g| g.session_id != sid),
            "row removed at once"
        );
        assert_eq!(deleted_copy(&app), DELETED_PENDING);
        let mut snap = crate::fixtures_games::snapshot();
        snap.matches.retain(|m| m.session_id != sid);
        snap.pending_deletes = vec![sid.into()];
        app.games = app.games_tab.games_from(&snap);
        assert_eq!(deleted_copy(&app), DELETED_PENDING, "tombstone not synced");
        snap.pending_deletes.clear();
        app.games = app.games_tab.games_from(&snap);
        assert_eq!(deleted_copy(&app), DELETED, "tombstone synced");
    }

    /// A snapshot that still lists the game keeps the line pending.
    #[test]
    fn the_deleted_line_stays_pending_while_the_snapshot_has_the_game() {
        let (_dir, mut app) = app();
        let sid = crate::fixtures_games::NORMAL;
        send(&mut app, Msg::Delete(sid.into()));
        app.games = app.games_tab.games_from(&crate::fixtures_games::snapshot());
        assert_eq!(deleted_copy(&app), DELETED_PENDING);
    }

    /// Waiting or rejected games never reached the site: plain line.
    #[test]
    fn the_deleted_line_is_plain_for_a_game_not_on_the_site() {
        for sid in [
            crate::fixtures_games::UNSURE,
            crate::fixtures_games::UNDECIDED,
        ] {
            let (_dir, mut app) = app();
            send(&mut app, Msg::Delete(sid.into()));
            assert_eq!(
                app.games_tab.deleted_note.as_ref().map(|d| d.uploaded),
                Some(false)
            );
            assert_eq!(deleted_copy(&app), DELETED, "{sid}");
        }
    }
}
