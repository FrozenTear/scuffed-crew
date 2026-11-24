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

/// Where a delete removes the game from, as far as the app can tell. It
/// follows what is known to have been sent, never the current sync
/// setting: signing out leaves an uploaded game on the site.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Reach {
    /// Uploaded: the site copy goes too.
    Site,
    /// Not known either way: not the uploaded copy, but a copy may have
    /// reached the site (edited since an upload, waiting, or its upload
    /// refused). The tombstone still removes any site copy, so the strip
    /// does not promise it is local only.
    Maybe,
    /// Never sent: only this computer has it.
    Local,
}

/// Where deleting `g` reaches. `in_progress` is the open session, kept so
/// the strip and the "Game deleted." line share one call shape; it does
/// not change the result. A live game with no result is Local: the
/// uploader never sends `outcome = unknown`.
///
/// - Site: the row is synced and has a result (the daemon marks a
///   no-result row synced without sending it).
/// - Local: the uploader never sends it as it is (no result yet, or a map
///   or hero missing), and no edit or refused upload says an earlier copy
///   was sent.
/// - Maybe: anything else.
pub fn reach(g: &Game, _in_progress: bool) -> Reach {
    let no_result = g.outcome == crate::model::Outcome::Unknown;
    if g.synced && !no_result {
        return Reach::Site;
    }
    let never_sent =
        (no_result || g.needs_review()) && g.edited_fields.is_empty() && g.upload_reject.is_none();
    if never_sent {
        Reach::Local
    } else {
        Reach::Maybe
    }
}

/// Maybe, with no website linked: do not assume a site copy exists.
pub const MAYBE_SYNC_OFF: &str =
    "This game is not known to have been sent. Your stats and averages update without it.";

/// The confirm strip's body for where the delete reaches. `sync_on` is
/// whether a website is linked: with it off, Maybe does not talk about
/// a site copy.
pub fn body(reach: Reach, sync_on: bool) -> &'static str {
    match (reach, sync_on) {
        (Reach::Site, _) => {
            "It will also be removed from the site. Your stats and averages update without it."
        }
        (Reach::Maybe, false) => MAYBE_SYNC_OFF,
        (Reach::Maybe, true) => {
            "If a copy reached the site, it's removed there too. Your stats and averages update without it."
        }
        (Reach::Local, _) => {
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
pub fn strip(sid: &str, reach: Reach, sync_on: bool, focus: &Focus) -> Element<'static, Message> {
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
            text(body(reach, sync_on)).size(SMALL).color(TEXT_2),
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

/// The filters the Games tab lists under: the season, role, hero, map and
/// result filters, and the Needs a look filter.
pub type Filters = (crate::aggregate::GameFilter, bool);

pub fn filters_now(app: &TrackerApp) -> Filters {
    (app.games_filter(), app.needs_look_filter)
}

/// Whether the "Game deleted." line is up: a delete left a note and the
/// filters are the ones it was made under. Another filter is another
/// list, where the line would top a table it says nothing about.
pub fn note_shows(app: &TrackerApp) -> bool {
    app.games_tab.deleted_note.is_some()
        && app.games_tab.deleted_filters.as_ref() == Some(&filters_now(app))
}

/// The line over the empty state, when the delete left no row to show.
pub fn empty_note(app: &TrackerApp) -> Option<&'static str> {
    note_shows(app).then(|| deleted_copy(app))
}

/// Take a note down once the filters changed, so it never comes back over
/// the first table of a later list. Run before each app message. Also
/// drops a note that stood over the empty state once rows are back.
pub fn drop_stale_note(app: &mut TrackerApp) {
    if app.games_tab.deleted_note.is_some() && !note_shows(app) {
        app.games_tab.deleted_note = None;
        app.games_tab.deleted_filters = None;
        app.games_tab.deleted_empty_shown = false;
    }
    if app.games_tab.deleted_empty_shown
        && app.games_tab.deleted_note.is_some()
        && !crate::games::shown_games(app).is_empty()
    {
        app.games_tab.deleted_note = None;
        app.games_tab.deleted_filters = None;
        app.games_tab.deleted_empty_shown = false;
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

    fn fixture_game(sid: &str) -> Game {
        crate::snapshot::games_from_snapshot(&crate::fixtures_games::snapshot())
            .into_iter()
            .find(|g| g.session_id == sid)
            .unwrap()
    }

    #[test]
    fn the_strip_says_where_the_game_is_removed_from() {
        use crate::fixtures_games::{HELD, NORMAL, UNDECIDED, UNSURE};
        let mut held = fixture_game(HELD);
        held.synced = false;
        assert_eq!(
            body(reach(&held, false), true),
            "It's only on this computer, so this removes it for good. Your stats and averages update without it."
        );
        assert_eq!(
            body(reach(&fixture_game(NORMAL), false), true),
            "It will also be removed from the site. Your stats and averages update without it."
        );
        // Not the uploaded copy, and maybe sent: no "for good" promise.
        for sid in [UNSURE, UNDECIDED] {
            assert_eq!(
                body(reach(&fixture_game(sid), false), true),
                "If a copy reached the site, it's removed there too. Your stats and averages update without it.",
                "{sid}"
            );
        }
    }

    /// Where a delete reaches follows what was sent, not the sync setting:
    /// a no-result row the daemon marked synced without sending it is
    /// local, and a game uploaded and then edited (re-queued, so no longer
    /// synced) may still be on the site.
    #[test]
    fn the_reach_follows_what_was_sent_not_the_sync_setting() {
        let mut g = fixture_game(crate::fixtures_games::NORMAL);
        g.edited_fields.clear();
        g.outcome = crate::model::Outcome::Unknown;
        g.synced = true;
        assert_eq!(reach(&g, false), Reach::Local, "never sent");
        assert_eq!(reach(&g, true), Reach::Local, "live: never sent");
        let mut g = fixture_game(crate::fixtures_games::NORMAL);
        g.synced = false;
        assert!(!g.edited_fields.is_empty());
        assert_eq!(reach(&g, false), Reach::Maybe, "edited after an upload");
        g.edited_fields.clear();
        assert_eq!(reach(&g, false), Reach::Maybe, "waiting: may be sent");
        assert_eq!(body(Reach::Maybe, false), MAYBE_SYNC_OFF);
        assert!(
            !body(Reach::Maybe, false).contains("site"),
            "sync off does not assume a site"
        );
    }

    /// The "Game deleted." line comes from the same reach as the strip
    /// over the fixture states, a live game, and a synced row.
    #[test]
    fn the_deleted_line_follows_the_strip() {
        use crate::fixtures_games::{HELD, NORMAL, REJECTED_2, UNDECIDED, UNSURE};
        for (sid, live) in [
            (NORMAL, false),
            (UNSURE, false),
            (UNDECIDED, false),
            (HELD, false),
            (REJECTED_2, false),
            (NORMAL, true),
            (UNDECIDED, true),
        ] {
            let (_dir, mut app) = app();
            if live {
                app.live_game_session = Some(sid.into());
            }
            let g = app.games.iter().find(|g| g.session_id == sid).unwrap();
            let want = reach(
                g,
                crate::model::game_card_in_progress(
                    &g.session_id,
                    g.outcome,
                    app.live_game_session.as_deref(),
                ),
            ) == Reach::Site;
            send(&mut app, Msg::Delete(sid.into()));
            assert_eq!(
                app.games_tab.deleted_note.as_ref().map(|d| d.uploaded),
                Some(want),
                "{sid} live={live}"
            );
        }
    }

    /// Deleting the only game the filters leave still says "Game
    /// deleted." above the empty state, and a filter change takes the
    /// line down so it never pops up over a table later.
    #[test]
    fn deleting_the_last_shown_row_says_so_above_the_empty_state() {
        let (_dir, mut app) = app();
        let sid = crate::fixtures_games::UNSURE;
        app.filter_map = Some("Suravasa".into());
        app.filter_outcome = Some(crate::model::Outcome::Win);
        let shown: Vec<String> = crate::games::shown_games(&app)
            .iter()
            .map(|g| g.session_id.clone())
            .collect();
        assert_eq!(shown, [sid]);
        app.expanded = Some(sid.into());
        send(&mut app, Msg::Delete(sid.into()));
        assert!(crate::games::shown_games(&app).is_empty());
        assert_eq!(app.games_tab.focus, Focus::None);
        assert_eq!(empty_note(&app), Some(DELETED));
        let _ = crate::games_table::view(&app, 1366.0, &[], &[]);
        // Another filter: the line goes at once, and the note with it.
        app.filter_outcome = None;
        assert_eq!(empty_note(&app), None);
        assert!(!note_shows(&app));
        let _ = app.update(crate::app::Message::Navigate(crate::model::Screen::Games));
        assert!(app.games_tab.deleted_note.is_none());
        // The only game of all: the line over "No games yet." too.
        let (_dir, mut app) = super::tests::app();
        app.games.retain(|g| g.session_id == sid);
        send(&mut app, Msg::Delete(sid.into()));
        assert!(app.games.is_empty());
        assert_eq!(empty_note(&app), Some(DELETED));
        // The delete marks it, not the view: drawing changes nothing.
        assert!(app.games_tab.deleted_empty_shown);
        let _ = crate::games_table::view(&app, 1366.0, &[], &[]);
        assert!(app.games_tab.deleted_empty_shown);
        // A new game under the same filters: the note does not return at row 0.
        app.games = crate::snapshot::games_from_snapshot(&crate::fixtures_games::snapshot());
        drop_stale_note(&mut app);
        assert!(app.games_tab.deleted_note.is_none());
        assert!(!note_shows(&app));
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

    /// Leaving the Games tab takes the "Game deleted." line down.
    #[test]
    fn leaving_the_tab_clears_the_deleted_line() {
        let (_dir, mut app) = app();
        send(&mut app, Msg::Delete(crate::fixtures_games::NORMAL.into()));
        assert!(app.games_tab.deleted_note.is_some());
        let _ = app.update(crate::app::Message::Navigate(crate::model::Screen::Games));
        assert!(app.games_tab.deleted_note.is_some(), "staying keeps it");
        let _ = app.update(crate::app::Message::Navigate(
            crate::model::Screen::Overview,
        ));
        assert!(app.games_tab.deleted_note.is_none());
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

    /// Opening a row with Enter or the Delete key refreshes the kit set
    /// as a click does: a stale entry goes.
    #[test]
    fn opening_a_row_by_key_refreshes_the_kits() {
        for key in [Msg::Enter, Msg::DeleteKey] {
            let (_dir, mut app) = app();
            let sid = crate::fixtures_games::UNSURE.to_string();
            app.expanded = None;
            app.games_tab.focus = Focus::Row(sid.clone());
            app.sessions_with_kit.insert("stale".into());
            send(&mut app, key);
            assert_eq!(app.expanded.as_deref(), Some(sid.as_str()));
            assert!(!app.sessions_with_kit.contains("stale"));
        }
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

    /// A waiting or rejected game is not known to be on the site (its
    /// reach is [`Reach::Maybe`]: a copy may have reached it), so the line
    /// makes no next-sync promise: plain line.
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
