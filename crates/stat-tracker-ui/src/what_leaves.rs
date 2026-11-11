//! "What leaves your PC": the dialog the Website account's link opens
//! (spec p6). It lists every time this app or the tracker sends anything
//! off this computer, and what goes with it. Each line follows the code:
//! the upload body (`StatsUploadEntry` in the types crate), the opt-in
//! training crops (`stat_tracker::sync::training`), the update check
//! (GitHub releases), the season list, the token check, and Report a
//! problem.

use iced::widget::{Column, button, column, container, opaque, row, scrollable, space, text};
use iced::{Element, Fill, Padding};

use crate::app::{Message, TrackerApp};
use crate::settings_open::OpenMsg;
use crate::settings_update::Msg;
use crate::theme::{
    self, FONT_BOLD, FONT_MEDIUM, FONT_SEMIBOLD, SIZE_BODY, SIZE_LABEL, SIZE_META, TEXT, TEXT_2,
};

const TITLE: &str = "What leaves your PC";

/// Heading and lines for each thing that is sent.
pub(crate) const SECTIONS: [(&str, &[&str]); 6] = [
    (
        "Game uploads, while Upload games is on",
        &[
            "After each game, your own row only: hero, role, map, mode, result, eliminations, deaths, assists, damage, healing, mitigation and when it was played.",
            "With it: whether you edited the game, which reader read it and which numbers it was not sure of.",
            "Games you delete here are deleted on the website too.",
            "No pictures, and no other player's name or stats. It goes only to the website you connected, with your token.",
        ],
    ),
    (
        "Training crops, only the kinds you turn on at the website",
        &[
            "Small pictures of what the tracker read from a Tab screen it accepted, at their own size: the map name banner, your hero portrait from the career panel, and the number cells of your own row.",
            "With each: a random number for the game, the screen size and which screen it came from. With a hero portrait, the hero, the name of the hero picture pack it was matched with, and how sure the reader was. With a number cell, its column and your hero. No names, chat, other players' rows or whole screens.",
            "Sent with your token while Upload games is on. Delete my training crops, under Website account, removes them from the website.",
        ],
    ),
    (
        "Website account",
        &[
            "Opening Settings asks the website which account the saved token belongs to and which training crops you allow. The app also reads the website's season list. None of these sends your games.",
        ],
    ),
    (
        "Update check",
        &[
            "At start and on Check now the app asks GitHub for the list of releases. Nothing about you or your games is sent.",
        ],
    ),
    (
        "Report a problem",
        &["Only when you send a report from its window, with what that window lists."],
    ),
    (
        "Not sent",
        &[
            "Screen captures, debug images and the stored games stay on this computer, apart from the training crops above. Read my rank after Comp games, Share stat pictures and Report pictures are not read by this version and start off when they come.",
        ],
    ),
];

/// The dialog as a layer over the window, while it is open.
pub(crate) fn layer(app: &TrackerApp) -> Option<Element<'_, Message>> {
    app.page_state.opened.what_leaves_open.then(dialog)
}

fn close() -> Message {
    Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(false)))
}

fn dialog<'a>() -> Element<'a, Message> {
    let mut body = Column::new().spacing(14);
    for (heading, lines) in SECTIONS {
        let mut part =
            column![text(heading).size(SIZE_META).font(FONT_BOLD).color(TEXT)].spacing(4);
        for line in lines {
            part = part.push(text(*line).size(SIZE_LABEL).font(FONT_MEDIUM).color(TEXT_2));
        }
        body = body.push(part);
    }
    let done = button(
        text("Close")
            .size(SIZE_META)
            .font(FONT_SEMIBOLD)
            .color(TEXT),
    )
    .padding(Padding::from([7, 14]))
    .style(theme::ghost_btn())
    .on_press(close());
    let card = container(
        column![
            text(TITLE)
                .size(SIZE_BODY + 4.0)
                .font(FONT_BOLD)
                .color(TEXT),
            scrollable(body).height(iced::Length::Shrink),
            row![space().width(Fill), done],
        ]
        .spacing(16),
    )
    .padding(24)
    .max_width(560)
    .style(theme::surface_panel);
    let mut backdrop = theme::BG;
    backdrop.a = 0.88;
    opaque(
        container(card)
            .padding(28)
            .center(Fill)
            .style(move |_theme| container::Style {
                background: Some(iced::Background::Color(backdrop)),
                ..container::Style::default()
            }),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_dialog_opens_and_closes_from_its_messages() {
        let (_dir, mut app) = crate::fixtures::test_app();
        assert!(layer(&app).is_none());
        let _ = app.update(Message::SettingsPage(Msg::Open(OpenMsg::WhatLeaves(true))));
        assert!(
            layer(&app).is_some(),
            "a demo may open it: it changes nothing"
        );
        let _ = app.update(close());
        assert!(layer(&app).is_none());
    }

    #[test]
    fn the_upload_lines_name_every_stat_field_sent() {
        let upload = SECTIONS[0].1.join(" ");
        for field in [
            "hero",
            "role",
            "map",
            "mode",
            "result",
            "eliminations",
            "deaths",
            "assists",
            "damage",
            "healing",
            "mitigation",
            "played",
        ] {
            assert!(upload.contains(field), "{field}");
        }
    }
}
