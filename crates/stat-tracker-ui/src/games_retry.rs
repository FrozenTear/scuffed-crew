//! Retry for rejected uploads (UX Spec's answer to open question 5): an
//! outlined "Retry upload" beside the rejected message in the open row's
//! status box, and one "N games couldn't upload · Retry all" line above
//! the table, only when two or more games are rejected. With one rejected
//! game nothing shows above the table: its row's mark leads to the panel.
//! A queued retry clears the rejection here at once; the next snapshot
//! brings it back if the server says no again.

use iced::widget::{button, row, text};
use iced::{Alignment, Border, Element, Padding, Theme};

use crate::app::{Message, TrackerApp};
use crate::games_table::Msg;
use crate::theme::{self, BORDER, FONT_SEMIBOLD, TEXT, TEXT_2};

/// Games whose upload the server rejected.
pub fn rejected_count(app: &TrackerApp) -> usize {
    app.games
        .iter()
        .filter(|g| g.upload_reject.is_some())
        .count()
}

/// "3 games couldn't upload", only for two or more.
pub fn retry_all_line(rejected: usize) -> Option<String> {
    (rejected >= 2).then(|| format!("{rejected} games couldn't upload"))
}

/// The line above the table, when [`retry_all_line`] has one.
pub fn above_table(app: &TrackerApp) -> Option<Element<'static, Message>> {
    let line = retry_all_line(rejected_count(app))?;
    let retry_all = outlined("Retry all", Message::Games(Msg::RetryAll));
    // One neutral line (spec): the count, a dot, an outlined Retry all.
    Some(
        row![text(format!("{line} ·")).size(12).color(TEXT_2), retry_all]
            .spacing(8)
            .align_y(Alignment::Center)
            .into(),
    )
}

/// The status box's outlined "Retry upload".
pub fn retry_button(sid: &str) -> Element<'static, Message> {
    outlined("Retry upload", Message::Games(Msg::Retry(sid.to_string())))
}

/// A small outlined button: 12px semibold, 1px border, no fill.
fn outlined(label: &'static str, msg: Message) -> Element<'static, Message> {
    button(text(label).size(12).font(FONT_SEMIBOLD).color(TEXT))
        .padding(Padding::from([4, 10]))
        .style(|_: &Theme, status| button::Style {
            background: theme::hover_tint(status),
            text_color: TEXT,
            border: Border {
                color: BORDER,
                width: 1.0,
                radius: 8.0.into(),
            },
            ..button::Style::default()
        })
        .on_press(msg)
        .into()
}

/// Queue a retry of one game, or of every rejected one (`None`).
pub fn retry(app: &mut TrackerApp, sid: Option<&str>) {
    let queued = match sid {
        Some(sid) => crate::commands::retry_upload(&app.data_dir, sid),
        None => crate::commands::retry_rejected_uploads(&app.data_dir),
    };
    match queued {
        Ok(()) => {
            for g in &mut app.games {
                if sid.is_none_or(|sid| g.session_id == sid) {
                    g.upload_reject = None;
                }
            }
            app.replace_toast(if sid.is_some() {
                "Upload queued again"
            } else {
                "Rejected uploads queued again"
            });
        }
        Err(e) => app.replace_toast(format!("Could not retry: {e}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fixtures_games::{NORMAL, UNDECIDED, UNSURE};
    use stat_tracker::storage::StoreCommand;

    fn app_with(rejected: &[&str]) -> (tempfile::TempDir, TrackerApp) {
        let dir = tempfile::tempdir().unwrap();
        let mut cli = crate::fixtures::test_cli(dir.path());
        cli.fixture = Some(crate::cli::FixtureKind::Games);
        let mut app = TrackerApp::new(cli).0;
        app.nav_style = crate::nav::NavStyle::TopTabs;
        for g in &mut app.games {
            g.upload_reject = rejected
                .contains(&g.session_id.as_str())
                .then(|| "no".to_string());
        }
        (dir, app)
    }

    #[test]
    fn the_line_shows_for_two_or_more_rejected_games() {
        assert_eq!(retry_all_line(0), None);
        assert_eq!(retry_all_line(1), None);
        assert_eq!(
            retry_all_line(2).as_deref(),
            Some("2 games couldn't upload")
        );
    }

    #[test]
    fn no_rejected_game_shows_nothing_above_the_table() {
        let (_dir, app) = app_with(&[]);
        assert_eq!(rejected_count(&app), 0);
        assert!(above_table(&app).is_none());
    }

    /// One rejected game: nothing above the table; its panel's Retry
    /// upload queues it and clears the rejection.
    #[test]
    fn one_rejected_game_retries_from_its_panel_only() {
        let (_dir, mut app) = app_with(&[UNDECIDED]);
        assert_eq!(rejected_count(&app), 1);
        assert!(above_table(&app).is_none());
        let _ = crate::games_table::update(&mut app, Msg::Retry(UNDECIDED.into()));
        assert_eq!(rejected_count(&app), 0);
        assert!(matches!(
            crate::commands::queued(&app.data_dir).as_slice(),
            [StoreCommand::RetryUpload { session_id }] if session_id == UNDECIDED
        ));
    }

    /// Two rejected: "2 games couldn't upload · Retry all" queues them
    /// all; one game's Retry upload leaves the other rejected.
    #[test]
    fn two_rejected_games_get_retry_all() {
        let (_dir, mut app) = app_with(&[UNDECIDED, UNSURE]);
        assert_eq!(rejected_count(&app), 2);
        assert!(above_table(&app).is_some());
        let _ = crate::games_table::update(&mut app, Msg::Retry(UNSURE.into()));
        assert_eq!(rejected_count(&app), 1);
        assert!(above_table(&app).is_none());
        let (_dir, mut app) = app_with(&[UNDECIDED, UNSURE, NORMAL]);
        assert_eq!(
            retry_all_line(rejected_count(&app)).as_deref(),
            Some("3 games couldn't upload")
        );
        let _ = crate::games_table::update(&mut app, Msg::RetryAll);
        assert_eq!(rejected_count(&app), 0);
        assert!(matches!(
            crate::commands::queued(&app.data_dir).as_slice(),
            [StoreCommand::RetryRejectedUploads]
        ));
    }
}
