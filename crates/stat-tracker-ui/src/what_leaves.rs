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

/// The bans and picks in the game upload: values only, never a picture.
/// Read at 2560x1440, and the switch starts on.
pub(crate) const BANS_PICKS_LINE: &str = "With Read bans and picks on (it starts on): the bans and picks the tracker read at 2560x1440, sent with the game as values only, never a picture.";

/// The rank fields in the game upload: read values only, never a picture.
/// Rank screen pictures go only as training crops, when that kind is on.
pub(crate) const RANK_LINE: &str = "With Read my rank after Comp games on: what the tracker read on the rank screen after the game (role, rank, division, progress and its change, its labels, the share of players you rank above, hero SR and its change, the next mode banner) and on the queue title before it, each with how sure the reader was. Only the values it read are sent with the game, never a picture of either screen. Parts of the rank screen go only as training crops (below), when you turn that kind on.";

/// Heading and lines for each thing that is sent.
pub(crate) const SECTIONS: [(&str, &[&str]); 6] = [
    (
        "Game uploads, while Upload games is on",
        &[
            "After each game, your own row: hero, role, map, mode, result, eliminations, deaths, assists, damage, healing, mitigation and when it was played.",
            "With it: whether you edited the game, which reader read it and which numbers it was not sure of.",
            BANS_PICKS_LINE,
            RANK_LINE,
            "Games you delete here are deleted on the website too.",
            "No pictures, and no other player's name or stats. It goes only to the website you connected, with your token.",
        ],
    ),
    (
        "Training crops, only the kinds you turn on at the website",
        &[
            "Small pictures of what the tracker read, at their own size. From a Tab it keeps for its game: the map name banner, and with the New reader your hero portrait from the career panel and the number cells of your own row. From the rank screen after a competitive game, up to three parts: your role icon and name when you queued for a role, the rank emblem and tier, and the progress bar with your progress, the \"Higher ranked than N%\" percentile and its change.",
            "With each: a random number for the game, the screen size and which screen it came from. The website also keeps the tracker version (its User-Agent) with each one. With a hero portrait, the hero, the name of the hero picture pack it was matched with, and how sure the reader was. With a number cell, its column and your hero. With a rank screen part, which part it is and where on the screen it was cut. Never whole screens or other players' rows, and never a part where the game shows names or chat, but text another program draws over the game can still show in one.",
            "Sent with your token while Upload games is on. Delete my training crops, under Website account, removes them from the website.",
            "While Upload games is on, the tracker itself asks the website which kinds you allow, with your token, when it starts and every 10 minutes, even with every kind off, and before each send. That question sends no pictures and no games.",
        ],
    ),
    (
        "Website account",
        &[
            "Opening Settings asks the website which account the saved token belongs to and which training crops you allow. Saving a website or token while Settings is open asks again which training crops you allow. The app also reads the website's season list. None of these sends your games.",
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
            "Screen captures, debug images and the stored games stay on this computer, apart from the training crops above (parts of a Tab and of the rank screen). Share stat pictures and Report pictures are not read by this version and start off when they come.",
        ],
    ),
];

/// The dialog as a layer over the window, while it is open. Esc closes it.
pub(crate) fn layer(app: &TrackerApp) -> Option<crate::dialog_focus::Dialog<'_>> {
    let open = app.page_state.opened.what_leaves_open;
    open.then(|| crate::dialog_focus::Dialog::new(dialog(), Some(close())))
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
    use stat_tracker::sync::UploadKind;

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

    /// The bans and picks line sits under Game uploads, says they start on,
    /// are read at 2560x1440, and go with the game as values only.
    #[test]
    fn the_bans_picks_line_says_values_go_with_uploads_at_2560x1440() {
        assert!(SECTIONS[0].0.starts_with("Game uploads"));
        assert_eq!(SECTIONS[0].1[2], BANS_PICKS_LINE);
        assert!(
            BANS_PICKS_LINE.contains("it starts on")
                && BANS_PICKS_LINE.contains("2560x1440")
                && BANS_PICKS_LINE.contains("values only")
                && BANS_PICKS_LINE.contains("never a picture"),
            "{BANS_PICKS_LINE}"
        );
        assert!(SECTIONS[0].1[0].starts_with("After each game, your own row:"));
        assert!(!SECTIONS[0].1[0].contains("own row only"));
    }

    /// The rank line sits under Game uploads, says the rank fields are what
    /// was read (not a picture), and matches the switch's help.
    #[test]
    fn the_rank_line_says_the_read_values_go_with_uploads_not_a_picture() {
        assert!(SECTIONS[0].0.starts_with("Game uploads"));
        assert_eq!(SECTIONS[0].1[3], RANK_LINE);
        assert_eq!(
            RANK_LINE,
            "With Read my rank after Comp games on: what the tracker read on the rank screen \
             after the game (role, rank, division, progress and its change, its labels, the \
             share of players you rank above, hero SR and its change, the next mode banner) \
             and on the queue title before it, each with how sure the reader was. Only the \
             values it read are sent with the game, never a picture of either screen. Parts \
             of the rank screen go only as training crops (below), when you turn that kind on."
        );
        let help = crate::settings_keys::RANK_HELP;
        assert!(
            help.contains("uploads") && help.contains("never a picture"),
            "{help}"
        );
        assert!(SECTIONS[0].1[5].starts_with("No pictures"));
    }

    /// What the training crop lines must say for `kind`. An exhaustive
    /// match, so a new `UploadKind` does not build until it has copy here.
    fn kind_copy(kind: UploadKind) -> &'static [&'static str] {
        match kind {
            UploadKind::HeroIcon => &["hero portrait"],
            UploadKind::StatDigits => &["number cells of your own row"],
            UploadKind::MapName => &["map name banner"],
            UploadKind::Rank => &[
                "rank screen after a competitive game",
                "role icon",
                "rank emblem and tier",
                "progress bar",
                "\"Higher ranked than N%\" percentile",
                "With a rank screen part, which part it is and where on the screen it was cut.",
            ],
        }
    }

    /// The training crop lines name every kind the tracker can send, and
    /// Not sent does not say rank screen pictures stay on this computer.
    #[test]
    fn the_training_lines_name_every_upload_kind() {
        assert!(SECTIONS[1].0.starts_with("Training crops"));
        let training = SECTIONS[1].1.join(" ");
        for kind in UploadKind::ALL {
            for said in kind_copy(kind) {
                assert!(training.contains(said), "{kind:?}: {said}");
            }
        }
        assert!(SECTIONS[5].0 == "Not sent" && SECTIONS[5].1[0].contains("rank screen"));
    }

    /// The training crop lines say what goes with each crop, and that the
    /// tracker reads the consent on its own, not only when Settings opens.
    #[test]
    fn the_training_lines_name_the_trackers_own_consent_read_and_version() {
        let training = SECTIONS[1].1.join(" ");
        assert!(SECTIONS[1].0.starts_with("Training crops"));
        for said in [
            "User-Agent",
            "tracker version",
            "when it starts and every 10 minutes",
            "even with every kind off",
        ] {
            assert!(training.contains(said), "{said}");
        }
        for (_, lines) in SECTIONS {
            for line in lines {
                assert!(
                    !line.contains('\u{2014}') && !line.contains('\u{2013}'),
                    "{line}"
                );
            }
        }
    }
}
