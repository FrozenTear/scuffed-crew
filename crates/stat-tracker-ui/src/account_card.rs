//! Pieces of the 0.6 Settings everyday cards after spec p6: the Website
//! account's signed-in line with Sign out, its "What leaves your PC ›"
//! link, and the Updates card's "Up to date. Checked HH:MM." line.

use iced::widget::{Row, button, column, row, space, text};
use iced::{Alignment, Element, Fill, Padding};

use crate::app::{Message, TrackerApp};
use crate::form_widgets::hint;
use crate::settings_open::{OpenMsg, sync_key};
use crate::settings_rows::link;
use crate::settings_update::Msg;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META, TEXT, TEXT_2,
    TEXT_3,
};

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
